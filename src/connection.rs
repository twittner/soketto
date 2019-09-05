// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

use crate::{base::{self, Header, OpCode}, extension::Extension};
use log::{debug, trace};
use futures::prelude::*;
use smallvec::SmallVec;
use std::{fmt, io};

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
    extensions: SmallVec<[Box<dyn Extension + Send>; 4]>
}

impl<T: AsyncRead + AsyncWrite + Unpin> Connection<T> {
    /// Create a new `Connection` from the given socket.
    pub fn new(socket: T, mode: Mode) -> Self {
        Connection {
            mode,
            socket,
            codec: base::Codec::default(),
            extensions: SmallVec::new()
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

    /// Send some data over this connection.
    ///
    /// If `as_text` is `true`, the websocket frame will use [`OpCode::Text`]
    /// and [`OpCode::Binary`] otherwise.
    ///
    /// The `data` is borrowed mutably because the masking (if any) is applied
    /// in-place and extensions (if any) get mutable access to it.
    pub async fn send<D>(&mut self, data: &mut D, as_text: bool) -> Result<(), Error>
    where
        D: AsMut<[u8]>
    {
        let mut header = Header::new(if as_text { OpCode::Text } else { OpCode::Binary });
        let data = data.as_mut();
        for e in &mut self.extensions {
            trace!("encoding with extension: {}", e.name());
            e.encode(&mut header, data).map_err(Error::Extension)?
        }
        if self.mode.is_client() {
            header.set_masked(true);
            header.set_mask(rand::random());
            self.codec.apply_mask(&header, data)
        }
        header.set_payload_len(data.len());
        let header_bytes = self.codec.encode_header(&header);
        self.socket.write_all(header_bytes).await?;
        self.socket.write_all(data).await?;
        Ok(())
    }
}

//impl<T: AsyncRead> AsyncRead for Connection<T> {
//    fn poll_read
//        ( self: Pin<&mut Self>
//        , ctx: &mut Context
//        , buf: &mut [u8]
//        ) -> Poll<Result<usize, io::Error>>
//    {
//    }
//}

//////impl<T: AsyncRead + AsyncWrite> Connection<T> {
//////    fn answer_ping(&mut self, frame: Frame, buf: Option<base::Data>) -> Poll<(), Error> {
//////        trace!("answering ping: {:?}", frame.header());
//////        if let AsyncSink::NotReady(frame) = self.framed.start_send(frame)? {
//////            self.state = Some(State::AnswerPing(frame, buf));
//////            return Ok(Async::NotReady)
//////        }
//////        self.flush(buf)
//////    }
//////
//////    fn answer_close(&mut self, frame: Frame) -> Poll<(), Error> {
//////        trace!("answering close: {:?}", frame.header());
//////        if let AsyncSink::NotReady(frame) = self.framed.start_send(frame)? {
//////            self.state = Some(State::AnswerClose(frame));
//////            return Ok(Async::NotReady)
//////        }
//////        self.closing()
//////    }
//////
//////    fn send_close(&mut self, frame: Frame) -> Poll<(), Error> {
//////        trace!("sending close: {:?}", frame.header());
//////        if let AsyncSink::NotReady(frame) = self.framed.start_send(frame)? {
//////            self.state = Some(State::SendClose(frame));
//////            return Ok(Async::NotReady)
//////        }
//////        self.flush_close()
//////    }
//////
//////    fn flush_close(&mut self) -> Poll<(), Error> {
//////        trace!("flushing close");
//////        if self.framed.poll_complete()?.is_not_ready() {
//////            self.state = Some(State::FlushClose);
//////            return Ok(Async::NotReady)
//////        }
//////        self.state = Some(State::AwaitClose);
//////        Ok(Async::Ready(()))
//////    }
//////
//////    fn flush(&mut self, buf: Option<base::Data>) -> Poll<(), Error> {
//////        trace!("flushing");
//////        if self.framed.poll_complete()?.is_not_ready() {
//////            self.state = Some(State::Flush(buf));
//////            return Ok(Async::NotReady)
//////        }
//////        self.state = Some(State::Open(buf));
//////        Ok(Async::Ready(()))
//////    }
//////
//////    fn closing(&mut self) -> Poll<(), Error> {
//////        trace!("closing");
//////        if self.framed.poll_complete()?.is_not_ready() {
//////            self.state = Some(State::Closing);
//////            return Ok(Async::NotReady)
//////        }
//////        self.state = Some(State::Closed);
//////        Ok(Async::Ready(()))
//////    }
//////
//////    fn await_close(&mut self) -> Poll<(), Error> {
//////        trace!("awaiting close");
//////        match self.framed.poll()? {
//////            Async::Ready(Some(frame)) =>
//////                if let OpCode::Close = frame.header().opcode() {
//////                    self.state = Some(State::Closed);
//////                    return Ok(Async::Ready(()))
//////                }
//////            Async::Ready(None) => self.state = Some(State::Closed),
//////            Async::NotReady => self.state = Some(State::AwaitClose)
//////        }
//////        Ok(Async::NotReady)
//////    }
//////}
//////
//////#[derive(Debug)]
//////enum State {
//////    /// Default state.
//////    /// Possible transitions: `Open`, `AnswerPing`, `AnswerClose`, `Closed`.
//////    Open(Option<base::Data>),
//////
//////    /// Send a PONG frame as answer to a PING we have received.
//////    /// Possible transitions: `AnswerPing`, `Open`.
//////    AnswerPing(Frame, Option<base::Data>),
//////
//////    /// Flush some frame we started sending.
//////    /// Possible transitions: `Flush`, `Open`.
//////    Flush(Option<base::Data>),
//////
//////    /// We want to send a close frame.
//////    /// Possible transitions: `SendClose`, `FlushClose`.
//////    SendClose(Frame),
//////
//////    /// We have sent a close frame and need to flush it.
//////    /// Possible transitions: `FlushClose`, `AwaitClose`.
//////    FlushClose,
//////
//////    /// We have sent a close frame and awaiting a close response.
//////    /// Possible transitions: `AwaitClose`, `Closed`.
//////    AwaitClose,
//////
//////    /// We have received a close frame and want to send a close response.
//////    /// Possible transitions: `AnswerClose`, `Closing`.
//////    AnswerClose(Frame),
//////
//////    /// We have begun sending a close answer frame and need to flush it.
//////    /// Possible transitions: `Closing`, `Closed`.
//////    Closing,
//////
//////    /// We are closed (terminal state).
//////    /// Possible transitions: none.
//////    Closed
//////}
//////
//////impl<T: AsyncRead + AsyncWrite> Stream for Connection<T> {
//////    type Item = base::Data;
//////    type Error = Error;
//////
//////    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
//////        loop {
//////            match self.state.take() {
//////                Some(State::Open(None)) => match self.framed.poll()? {
//////                    Async::Ready(Some(mut frame)) => {
//////                        trace!("received: {:?}", frame.header());
//////                        match frame.header().opcode() {
//////                            OpCode::Text | OpCode::Binary if frame.header().is_fin() => {
//////                                self.state = Some(State::Open(None));
//////                                let (mut h, mut d) = frame.into_parts();
//////                                decode_with_extensions(&mut h, &mut d, &mut self.extensions)?;
//////                                return Ok(Async::Ready(d))
//////                            }
//////                            OpCode::Text | OpCode::Binary => {
//////                                let (mut h, mut d) = frame.into_parts();
//////                                decode_with_extensions(&mut h, &mut d, &mut self.extensions)?;
//////                                self.state = Some(State::Open(d))
//////                            }
//////                            OpCode::Ping => {
//////                                let mut answer = Frame::new(OpCode::Pong);
//////                                answer.set_payload_data(frame.take_payload_data());
//////                                self.set_mask(&mut answer);
//////                                try_ready!(self.answer_ping(answer, None))
//////                            }
//////                            OpCode::Close => {
//////                                let mut answer = close_answer(frame)?;
//////                                self.set_mask(&mut answer);
//////                                try_ready!(self.answer_close(answer))
//////                            }
//////                            OpCode::Pong => {
//////                                trace!("unexpected Pong; ignoring");
//////                                self.state = Some(State::Open(None))
//////                            }
//////                            OpCode::Continue => {
//////                                debug!("unexpected Continue opcode");
//////                                return Err(Error::UnexpectedOpCode(OpCode::Continue))
//////                            }
//////                            reserved => {
//////                                debug_assert!(reserved.is_reserved());
//////                                debug!("unexpected opcode: {}", reserved);
//////                                return Err(Error::UnexpectedOpCode(reserved))
//////                            }
//////                        }
//////                    }
//////                    Async::Ready(None) => {
//////                        self.state = Some(State::Closed);
//////                        return Ok(Async::Ready(None))
//////                    }
//////                    Async::NotReady => {
//////                        self.state = Some(State::Open(None));
//////                        return Ok(Async::NotReady)
//////                    }
//////                }
//////                // We have buffered some data => we are processing a fragmented message
//////                // and expect either control frames or a CONTINUE frame.
//////                Some(State::Open(Some(mut data))) => match self.framed.poll()? {
//////                    Async::Ready(Some(mut frame)) => {
//////                        trace!("received: {:?}", frame.header());
//////                        match frame.header().opcode() {
//////                            OpCode::Continue if frame.header().is_fin() => {
//////                                let (mut hdr, dat) = frame.into_parts();
//////                                if let Some(d) = dat {
//////                                    ensure_max_buffer_size(self.max_buffer_size, &data, &d)?;
//////                                    data.bytes_mut().unsplit(d.into_bytes())
//////                                }
//////                                let mut data = Some(data);
//////                                decode_with_extensions(&mut hdr, &mut data, &mut self.extensions)?;
//////                                self.state = Some(State::Open(None));
//////                                return Ok(Async::Ready(data))
//////                            }
//////                            OpCode::Continue => {
//////                                let (mut hdr, dat) = frame.into_parts();
//////                                if let Some(d) = dat {
//////                                    ensure_max_buffer_size(self.max_buffer_size, &data, &d)?;
//////                                    data.bytes_mut().unsplit(d.into_bytes())
//////                                }
//////                                let mut data = Some(data);
//////                                decode_with_extensions(&mut hdr, &mut data, &mut self.extensions)?;
//////                                self.state = Some(State::Open(data))
//////                            }
//////                            OpCode::Ping => {
//////                                let mut answer = Frame::new(OpCode::Pong);
//////                                answer.set_payload_data(frame.take_payload_data());
//////                                self.set_mask(&mut answer);
//////                                try_ready!(self.answer_ping(answer, Some(data)))
//////                            }
//////                            OpCode::Close => {
//////                                let mut answer = close_answer(frame)?;
//////                                self.set_mask(&mut answer);
//////                                try_ready!(self.answer_close(answer))
//////                            }
//////                            OpCode::Pong => {
//////                                trace!("unexpected Pong; ignoring");
//////                                self.state = Some(State::Open(Some(data)))
//////                            }
//////                            OpCode::Text | OpCode::Binary => {
//////                                debug!("unexpected opcode {}", frame.header().opcode());
//////                                return Err(Error::UnexpectedOpCode(frame.header().opcode()))
//////                            }
//////                            reserved => {
//////                                debug_assert!(reserved.is_reserved());
//////                                debug!("unexpected opcode: {}", reserved);
//////                                return Err(Error::UnexpectedOpCode(reserved))
//////                            }
//////                        }
//////                    }
//////                    Async::Ready(None) => {
//////                        self.state = Some(State::Closed);
//////                        return Ok(Async::Ready(None))
//////                    }
//////                    Async::NotReady => {
//////                        self.state = Some(State::Open(Some(data)));
//////                        return Ok(Async::NotReady)
//////                    }
//////                }
//////                Some(State::AnswerPing(frame, buf)) => try_ready!(self.answer_ping(frame, buf)),
//////                Some(State::SendClose(frame)) => try_ready!(self.send_close(frame)),
//////                Some(State::AnswerClose(frame)) => try_ready!(self.answer_close(frame)),
//////                Some(State::Flush(buf)) => try_ready!(self.flush(buf)),
//////                Some(State::FlushClose) => try_ready!(self.flush_close()),
//////                Some(State::AwaitClose) => try_ready!(self.await_close()),
//////                Some(State::Closing) => try_ready!(self.closing()),
//////                Some(State::Closed) | None => return Ok(Async::Ready(None)),
//////            }
//////        }
//////    }
//////}
//////
//////impl<T: AsyncRead + AsyncWrite> Sink for Connection<T> {
//////    type SinkItem = base::Data;
//////    type SinkError = Error;
//////
//////    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
//////        loop {
//////            match self.state.take() {
//////                Some(State::Open(buf)) => {
//////                    let mut header = if item.is_text() {
//////                        Header::new(OpCode::Text)
//////                    } else {
//////                        Header::new(OpCode::Binary)
//////                    };
//////                    let mut data = Some(item);
//////                    encode_with_extensions(&mut header, &mut data, &mut self.extensions)?;
//////                    let mut frame = Frame::from(header);
//////                    frame.set_payload_data(data);
//////                    self.set_mask(&mut frame);
//////                    self.state = Some(State::Open(buf));
//////                    trace!("send: {:?}", frame.header());
//////                    if let AsyncSink::NotReady(mut frame) = self.framed.start_send(frame)? {
//////                        let data = frame.take_payload_data().expect("frame was constructed with Some");
//////                        return Ok(AsyncSink::NotReady(data))
//////                    } else {
//////                        return Ok(AsyncSink::Ready)
//////                    }
//////                }
//////                Some(State::AnswerPing(frame, buf)) =>
//////                    if self.answer_ping(frame, buf)?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::AnswerClose(frame)) =>
//////                    if self.answer_close(frame)?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::Flush(buf)) =>
//////                    if self.flush(buf)?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::Closing) =>
//////                    if self.closing()?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::AwaitClose) =>
//////                    if self.await_close()?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::SendClose(frame)) =>
//////                    if self.send_close(frame)?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::FlushClose) =>
//////                    if self.flush_close()?.is_not_ready() {
//////                        return Ok(AsyncSink::NotReady(item))
//////                    }
//////                Some(State::Closed) | None => return Err(Error::Closed)
//////            }
//////        }
//////    }
//////
//////    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
//////        match self.state.take() {
//////            Some(State::Open(buf)) => {
//////                self.state = Some(State::Open(buf));
//////                try_ready!(self.framed.poll_complete())
//////            }
//////            Some(State::AnswerPing(frame, buf)) => try_ready!(self.answer_ping(frame, buf)),
//////            Some(State::AnswerClose(frame)) => try_ready!(self.answer_close(frame)),
//////            Some(State::Flush(buf)) => try_ready!(self.flush(buf)),
//////            Some(State::Closing) => try_ready!(self.closing()),
//////            Some(State::AwaitClose) => try_ready!(self.await_close()),
//////            Some(State::SendClose(frame)) => try_ready!(self.send_close(frame)),
//////            Some(State::FlushClose) => try_ready!(self.flush_close()),
//////            Some(State::Closed) | None => ()
//////        }
//////        Ok(Async::Ready(()))
//////    }
//////
//////    fn close(&mut self) -> Poll<(), Self::SinkError> {
//////        try_ready!(self.poll_complete());
//////        if let Some(State::Open(_)) = self.state.take() {
//////            let mut frame = Frame::new(OpCode::Close);
//////            // code 1000 means normal closure
//////            let code = base::Data::Binary(1000_u16.to_be_bytes()[..].into());
//////            frame.set_payload_data(Some(code));
//////            self.set_mask(&mut frame);
//////            try_ready!(self.send_close(frame))
//////        }
//////        Ok(Async::Ready(()))
//////    }
//////}
//////
//////fn close_answer(mut frame: Frame) -> Result<Frame, Error> {
//////    if let Some(mut data) = frame.take_payload_data() {
//////        if data.as_ref().len() >= 2 {
//////            let slice = data.as_ref();
//////            let code = u16::from_be_bytes([slice[0], slice[1]]);
//////            let reason = std::str::from_utf8(&slice[2 ..])?;
//////            debug!("received close frame; code = {}; reason = {}", code, reason);
//////            let mut answer = Frame::new(OpCode::Close);
//////            let data = match code {
//////                1000 ..= 1003 | 1007 ..= 1011 | 1015 | 3000 ..= 4999 => { // acceptable codes
//////                    data.bytes_mut().truncate(2);
//////                    data
//////                }
//////                _ => {
//////                    // Other codes are invalid => reply with protocol error (1002).
//////                    base::Data::Binary(1002_u16.to_be_bytes()[..].into())
//////                }
//////            };
//////            answer.set_payload_data(Some(data));
//////            return Ok(answer)
//////        }
//////    }
//////    debug!("received close frame");
//////    Ok(Frame::new(OpCode::Close))
//////}
//////
//////fn decode_with_extensions<'a, I>(h: &mut Header, d: &mut Option<Data>, exts: I) -> Result<(), Error>
//////where
//////    I: IntoIterator<Item = &'a mut Box<dyn Extension + Send>>
//////{
//////    for e in exts {
//////        trace!("decoding with extension: {}", e.name());
//////        e.decode(h, d).map_err(Error::Extension)?
//////    }
//////    Ok(())
//////}
//////
//////fn encode_with_extensions<'a, I>(h: &mut Header, d: &mut Option<Data>, exts: I) -> Result<(), Error>
//////where
//////    I: IntoIterator<Item = &'a mut Box<dyn Extension + Send>>
//////{
//////    for e in exts {
//////        trace!("encoding with extension: {}", e.name());
//////        e.encode(h, d).map_err(Error::Extension)?
//////    }
//////    Ok(())
//////}
//////
//////fn ensure_max_buffer_size(maximum: usize, current: &Data, new: &Data) -> Result<(), Error> {
//////    let size = current.as_ref().len() + new.as_ref().len();
//////    if size > maximum {
//////        let e = Error::MessageTooLarge { actual: size, maximum };
//////        warn!("{}", e);
//////        return Err(e)
//////    }
//////    Ok(())
//////}

// Connection error type //////////////////////////////////////////////////////////////////////////

/// Connection error cases.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The base codec errored.
    Codec(base::Error),
    /// An extension produced an error while encoding or decoding.
    Extension(crate::BoxError),
    /// An unexpected opcode as encountered.
    UnexpectedOpCode(OpCode),
    /// A close reason was not correctly UTF-8 encoded.
    Utf8(std::str::Utf8Error),
    /// The total message payload data size exceeds the configured maximum.
    MessageTooLarge { actual: usize, maximum: usize },
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
            Error::MessageTooLarge { actual, maximum } =>
                write!(f, "message to large: len >= {}, maximum = {}", actual, maximum),
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
