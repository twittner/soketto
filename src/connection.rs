// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! A persistent websocket connection after the handshake phase.

use bytes::{BufMut, BytesMut};
use crate::{Parsing, base::{self, Header, OpCode}, extension::Extension};
use log::{debug, trace};
use futures::prelude::*;
use smallvec::SmallVec;
use static_assertions::const_assert;
use std::{fmt, io};

const BLOCK_SIZE: usize = 4096;

/// Is the [`Connection`] used by a client or server?
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Client-side of a connection (implies masking of payload data).
    Client,
    /// Server-side of a connection.
    Server
}

impl Mode {
    pub fn is_client(self) -> bool {
        if let Mode::Client = self {
            true
        } else {
            false
        }
    }

    pub fn is_server(self) -> bool {
        !self.is_client()
    }
}

/// A persistent websocket connection.
#[derive(Debug)]
pub struct Connection<T> {
    mode: Mode,
    socket: T,
    codec: base::Codec,
    extensions: SmallVec<[Box<dyn Extension + Send>; 4]>,
    max_buffer_size: usize,
    validate_utf8: bool,
    is_closed: bool
}

impl<T: AsyncRead + AsyncWrite + Unpin> Connection<T> {
    /// Create a new `Connection` from the given socket.
    pub fn new(socket: T, mode: Mode) -> Self {
        Connection {
            mode,
            socket,
            codec: base::Codec::default(),
            extensions: SmallVec::new(),
            max_buffer_size: 256 * 1024 * 1024,
            validate_utf8: false,
            is_closed: false
        }
    }

    /// Add extensions to this connection.
    ///
    /// Only enabled extensions will be considered.
    pub fn add_extensions<I>(&mut self, extensions: I) -> &mut Self
    where
        I: IntoIterator<Item = Box<dyn Extension + Send>>
    {
        for e in extensions.into_iter().filter(|e| e.is_enabled()) {
            debug!("using extension: {}", e.name());
            self.codec.add_reserved_bits(e.reserved_bits());
            self.extensions.push(e)
        }
        self
    }

    /// Set the maximum buffer size in bytes.
    ///
    /// Messages fragments will be buffered and concatenated up to this value.
    pub fn set_max_buffer_size(&mut self, max: usize) -> &mut Self {
        self.max_buffer_size = max;
        self
    }

    /// Toggle UTF-8 check for incoming text messages.
    pub fn validate_utf8(&mut self, value: bool) -> &mut Self {
        self.validate_utf8 = value;
        self
    }

    /// Send some binary data over this connection.
    pub async fn send_binary(&mut self, data: &mut BytesMut) -> Result<(), Error> {
        let mut header = Header::new(OpCode::Binary);
        self.send(&mut header, data).await?;
        Ok(())
    }

    /// Send some text data over this connection.
    pub async fn send_text(&mut self, data: &mut BytesMut) -> Result<(), Error> {
        debug_assert!(std::str::from_utf8(&data).is_ok());
        let mut header = Header::new(OpCode::Text);
        self.send(&mut header, data).await?;
        Ok(())
    }

    /// Send arbitrary websocket frames.
    async fn send(&mut self, header: &mut Header, data: &mut BytesMut) -> Result<(), Error> {
        if self.is_closed {
            debug!("can not send, connection is closed");
            return Err(Error::Closed)
        }
        for e in &mut self.extensions {
            trace!("encoding with extension: {}", e.name());
            e.encode(header, data).map_err(Error::Extension)?
        }
        write(self.mode, &mut self.codec, &mut self.socket, header, data, false).await?;
        Ok(())
    }

    /// Receive the next websocket message.
    ///
    /// Fragmented messages will be concatenated into `data`.
    /// The `bool` indicates if the data is textual (when `true`) or binary
    /// (when `false`). If `Connection::validate_utf8` is `true` and the
    /// return value is `Ok(true)`, `data` will be valid UTF-8.
    pub async fn receive(&mut self, data: &mut BytesMut) -> Result<(BytesMut, bool), Error> {
        let mut code = None;
        let mut offset = 0;
        loop {
            if self.is_closed {
                debug!("can not receive, connection is closed");
                return Err(Error::Closed)
            }

            let mut header = self.receive_header(data).await?;
            trace!("recv: {:?}", header);

            // Handle control frames.
            if header.opcode().is_control() {
                debug_assert!(header.payload_len() < 126); // ensured by `base::Codec`
                if data.len() < header.payload_len() {
                    const_assert!(min_block_size; BLOCK_SIZE > 125);
                    data.reserve(BLOCK_SIZE)
                }
                while data.len() < header.payload_len() {
                    unsafe {
                        let n = self.socket.read(data.bytes_mut()).await?;
                        data.advance_mut(n)
                    }
                }
                self.on_control(&header, data).await?;
                let mut continuation = data.split_off(offset);
                continuation.split_to(header.payload_len());
                data.unsplit(continuation);
                continue
            }

            if data.len() + header.payload_len() > self.max_buffer_size {
                return Err(Error::MessageTooLarge {
                    current: data.len() + header.payload_len(),
                    maximum: self.max_buffer_size
                })
            }

            while data.len() < header.payload_len() {
                data.reserve(std::cmp::max(BLOCK_SIZE, header.payload_len()));
                unsafe {
                    let n = self.socket.read(data.bytes_mut()).await?;
                    data.advance_mut(n)
                }
            }

            self.codec.apply_mask(&header, &mut data[offset .. header.payload_len()]);
            offset += header.payload_len();

            if !header.is_fin() {
                if header.opcode() != OpCode::Continue { // first fragment
                    if code.is_some() {
                        // We received a new first fragment while the current
                        // fragments are not finished yet.
                        return Err(Error::UnexpectedOpCode(header.opcode()))
                    } else {
                        // Save opcode so we can restore it at the end.
                        code = Some(header.opcode())
                    }
                }
                continue
            } else if header.opcode() == OpCode::Continue { // last fragment
                if let Some(c) = code.take() {
                    // Restore opcode and set the payload length to the
                    // accumulated length of all fragments.
                    //
                    // We do this so that extensions only operate on
                    // non-fragmented messages.
                    header.set_opcode(c);
                    header.set_payload_len(offset);
                } else {
                    // Malformed fragment
                    return Err(Error::UnexpectedOpCode(header.opcode()))
                }
            }

            let mut payload = data.split_to(offset);

            for e in &mut self.extensions {
                trace!("decoding with extension: {}", e.name());
                e.decode(&mut header, &mut payload).map_err(Error::Extension)?
            }

            let is_text = header.opcode() == OpCode::Text;

            if is_text && self.validate_utf8 {
                std::str::from_utf8(&payload)?;
            }

            return Ok((payload, is_text))
        }
    }

    /// Answer incoming control frames.
    async fn on_control(&mut self, header: &Header, data: &mut BytesMut) -> Result<(), Error> {
        debug_assert!(data.len() >= header.payload_len());
        match header.opcode() {
            OpCode::Ping => {
                let mut answer = Header::new(OpCode::Pong);
                let codec = &mut self.codec;
                let sockt = &mut self.socket;
                let payload = &mut data[.. header.payload_len()];
                write(self.mode, codec, sockt, &mut answer, payload, true).await?;
                Ok(())
            }
            OpCode::Pong => Ok(()),
            OpCode::Close => {
                let codec = &mut self.codec;
                let sockt = &mut self.socket;
                let (mut header, code) = close_answer(&data[.. header.payload_len()])?;
                if let Some(c) = code {
                    let mut data = c.to_be_bytes();
                    write(self.mode, codec, sockt, &mut header, &mut data[..], true).await?
                } else {
                    write(self.mode, codec, sockt, &mut header, &mut [], true).await?
                }
                self.is_closed = true;
                Ok(())
            }
            OpCode::Binary
            | OpCode::Text
            | OpCode::Continue
            | OpCode::Reserved3
            | OpCode::Reserved4
            | OpCode::Reserved5
            | OpCode::Reserved6
            | OpCode::Reserved7
            | OpCode::Reserved11
            | OpCode::Reserved12
            | OpCode::Reserved13
            | OpCode::Reserved14
            | OpCode::Reserved15 => Err(Error::UnexpectedOpCode(header.opcode()))
        }
    }

    /// Read the next frame header from the socket.
    async fn receive_header(&mut self, data: &mut BytesMut) -> Result<Header, Error> {
        loop {
            match self.codec.decode_header(&data)? {
                Parsing::Done { value: header, offset } => {
                    data.split_to(offset);
                    return Ok(header)
                }
                Parsing::NeedMore(_) => {
                    if !data.has_remaining_mut() {
                        data.reserve(BLOCK_SIZE)
                    }
                    unsafe {
                        let n = self.socket.read(data.bytes_mut()).await?;
                        data.advance_mut(n)
                    }
                }
            }
        }
    }

    /// Send a close message and close the connection.
    pub async fn close(&mut self) -> Result<(), Error> {
        if self.is_closed {
            return Ok(())
        }

        let mut header = Header::new(OpCode::Close);
        let mut code = 1000_u16.to_be_bytes(); // 1000 = normal closure
        let codec = &mut self.codec;
        let sockt = &mut self.socket;
        write(self.mode, codec, sockt, &mut header, &mut code[..], true).await?;
        self.is_closed = true;
        Ok(())
    }
}

/// Write header and data to socket.
///
/// Not a method due to borrowing issues in relation to the
/// `control_buffer` field.
async fn write<T>
    ( mode: Mode
    , codec: &mut base::Codec
    , socket: &mut T
    , header: &mut Header
    , data: &mut [u8]
    , flush: bool
    ) -> Result<(), Error>
where
    T: AsyncWrite + Unpin
{
    if mode.is_client() {
        header.set_masked(true);
        header.set_mask(rand::random());
        codec.apply_mask(&header, data)
    }
    header.set_payload_len(data.len());
    let header_bytes = codec.encode_header(&header);
    trace!("send: {:?}", header);
    socket.write_all(header_bytes).await?;
    if !data.is_empty() {
        socket.write_all(data).await?;
    }
    if flush {
        socket.flush().await?
    }
    Ok(())
}

/// Derive a response to an incoming close frame.
fn close_answer(data: &[u8]) -> Result<(Header, Option<u16>), Error> {
    let answer = Header::new(OpCode::Close);
    if data.len() < 2 {
        return Ok((answer, None))
    }
    std::str::from_utf8(&data[2 ..])?; // check reason is properly encoded
    let code = u16::from_be_bytes([data[0], data[1]]);
    match code {
        | 1000 ..= 1003
        | 1007 ..= 1011
        | 1015
        | 3000 ..= 4999 => Ok((answer, Some(code))), // acceptable codes
        _               => Ok((answer, Some(1002))) // invalid code => protocol error (1002)
    }
}

// Connection error type //////////////////////////////////////////////////////////////////////////

/// Connection error cases.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The base codec errored.
    Codec(base::Error),
    /// An extension produced an error while encoding or decoding.
    Extension(crate::BoxedError),
    /// An unexpected opcode was encountered.
    UnexpectedOpCode(OpCode),
    /// A close reason was not correctly UTF-8 encoded.
    Utf8(std::str::Utf8Error),
    /// The total message payload data size exceeds the configured maximum.
    MessageTooLarge { current: usize, maximum: usize },
    /// The connection is closed.
    Closed,

    #[doc(hidden)]
    __Nonexhaustive

}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o error: {}", e),
            Error::Codec(e) => write!(f, "codec error: {}", e),
            Error::Extension(e) => write!(f, "extension error: {}", e),
            Error::UnexpectedOpCode(c) => write!(f, "unexpected opcode: {}", c),
            Error::Utf8(e) => write!(f, "utf-8 error: {}", e),
            Error::MessageTooLarge { current, maximum } =>
                write!(f, "message to large: len >= {}, maximum = {}", current, maximum),
            Error::Closed => f.write_str("connection closed"),
            Error::__Nonexhaustive => f.write_str("__Nonexhaustive")
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Codec(e) => Some(e),
            Error::Extension(e) => Some(&**e),
            Error::Utf8(e) => Some(e),
            Error::UnexpectedOpCode(_)
            | Error::MessageTooLarge {..}
            | Error::Closed
            | Error::__Nonexhaustive => None
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<base::Error> for Error {
    fn from(e: base::Error) -> Self {
        Error::Codec(e)
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(e: std::str::Utf8Error) -> Self {
        Error::Utf8(e)
    }
}
