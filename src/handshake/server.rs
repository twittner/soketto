// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! Websocket server [handshake].
//!
//! [handshake]: https://tools.ietf.org/html/rfc6455#section-4

use crate::{Parsing, connection::{Connection, Mode}, extension::Extension};
use futures::prelude::*;
use http::StatusCode;
use sha1::Sha1;
use smallvec::SmallVec;
use std::str;
use super::{
    Error,
    KEY,
    MAX_NUM_HEADERS,
    SEC_WEBSOCKET_EXTENSIONS,
    SEC_WEBSOCKET_PROTOCOL,
    append_extensions,
    configure_extensions,
    expect_ascii_header,
    with_first_header
};

const BLOCK_SIZE: usize = 4096;
const SOKETTO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Websocket handshake client.
#[derive(Debug)]
pub struct Server<'a, T> {
    socket: T,
    /// Protocols the server supports.
    protocols: SmallVec<[&'a str; 4]>,
    /// Extensions the server supports.
    extensions: SmallVec<[Box<dyn Extension + Send>; 4]>
}

impl<'a, T: AsyncRead + AsyncWrite + Unpin> Server<'a, T> {
    /// Create a new server handshake.
    pub fn new(socket: T) -> Self {
        Server {
            socket,
            protocols: SmallVec::new(),
            extensions: SmallVec::new()
        }
    }

    /// Add a protocol the server supports.
    pub fn add_protocol(&mut self, p: &'a str) -> &mut Self {
        self.protocols.push(p);
        self
    }

    /// Add an extension the server supports.
    pub fn add_extension(&mut self, e: Box<dyn Extension + Send>) -> &mut Self {
        self.extensions.push(e);
        self
    }

    /// Get back all extensions.
    pub fn drain_extensions(&mut self) -> impl Iterator<Item = Box<dyn Extension + Send>> + '_ {
        self.extensions.drain()
    }

    /// Await an incoming client handshake request.
    pub async fn receive_request<'b>(&mut self, buf: &'b mut Vec<u8>) -> Result<ClientRequest<'b>, Error> {
        buf.clear();
        let mut offset = 0;
        loop {
            // Here is what we would like to write:
            //
            //   if buf.len() == offset {
            //       buf.resize(offset + BLOCK_SIZE, 0)
            //   }
            //   offset += self.socket.read(&mut buf[offset ..]).await?;
            //   if let Parsing::Done { value, .. } = self.decode_request(&buf[.. offset])? {
            //       return Ok(value)
            //   }
            //
            // But because `&buf[.. offset]` has lifetime 'b (it backs the
            // `ClientRequest<'b>`), the borrow checker will not allow us
            // to `resize` `buf` because it means a second mutable borrow
            // during the lifetime of 'b (on the next loop iteration).
            // Note that this is safe because we will only modify `buf` if
            // we do not return a borrow.
            if buf.len() == offset {
                buf.resize(offset + BLOCK_SIZE, 0)
            }
            let buf_slice = {
                let p = buf.as_mut_ptr();
                let n = buf.len();
                unsafe { // NOTE
                    std::slice::from_raw_parts_mut(p, n)
                }
            };
            offset += self.socket.read(&mut buf_slice[offset ..]).await?;
            if let Parsing::Done { value, .. } = self.decode_request(&buf_slice[.. offset])? {
                return Ok(value)
            }
        }
    }

    /// Respond to the client.
    pub async fn send_response(&mut self, buf: &mut Vec<u8>, r: &Response<'_>) -> Result<(), Error> {
        buf.clear();
        self.encode_response(buf, r);
        self.socket.write_all(buf).await?;
        self.socket.flush().await?;
        Ok(())
    }

    /// Turn this handshake into a [`Connection`].
    ///
    /// If `take_over_extensions` is true, the extensions from this
    /// handshake will be set on the `Connection` returned.
    pub fn into_connection(mut self, take_over_extensions: bool) -> Connection<T> {
        let mut c = Connection::new(self.socket, Mode::Server);
        if take_over_extensions {
            c.add_extensions(self.extensions.drain());
        }
        c
    }

    // Decode client handshake request.
    fn decode_request<'b>(&mut self, buf: &'b [u8]) -> Result<Parsing<ClientRequest<'b>>, Error> {
        let mut header_buf = [httparse::EMPTY_HEADER; MAX_NUM_HEADERS];
        let mut request = httparse::Request::new(&mut header_buf);

        let offset = match request.parse(buf) {
            Ok(httparse::Status::Complete(off)) => off,
            Ok(httparse::Status::Partial) => return Ok(Parsing::NeedMore(())),
            Err(e) => return Err(Error::Http(Box::new(e)))
        };

        if request.method != Some("GET") {
            return Err(Error::InvalidRequestMethod)
        }
        if request.version != Some(1) {
            return Err(Error::UnsupportedHttpVersion)
        }

        // TODO: Host Validation
        with_first_header(&request.headers, "Host", |_h| Ok(()))?;

        expect_ascii_header(request.headers, "Upgrade", "websocket")?;
        expect_ascii_header(request.headers, "Connection", "upgrade")?;
        expect_ascii_header(request.headers, "Sec-WebSocket-Version", "13")?;

        let ws_key = with_first_header(&request.headers, "Sec-WebSocket-Key", |k| {
            Ok(k)
        })?;

        for h in request.headers.iter()
            .filter(|h| h.name.eq_ignore_ascii_case(SEC_WEBSOCKET_EXTENSIONS))
        {
            configure_extensions(&mut self.extensions, std::str::from_utf8(h.value)?)?
        }

        let mut protocols = SmallVec::new();
        for p in request.headers.iter()
            .filter(|h| h.name.eq_ignore_ascii_case(SEC_WEBSOCKET_PROTOCOL))
        {
            if self.protocols.iter().find(|x| x.as_bytes() == p.value).is_some() {
                protocols.push(std::str::from_utf8(p.value)?)
            }
        }

        Ok(Parsing::Done { value: ClientRequest { ws_key, protocols }, offset })
    }

    // Encode server handshake response.
    fn encode_response(&mut self, buf: &mut Vec<u8>, response: &Response) {
        match response {
            Response::Accept(accept) => {
                let mut key_buf = [0; 32];
                let accept_value = {
                    let mut digest = Sha1::new();
                    digest.update(&accept.key);
                    digest.update(KEY);
                    let d = digest.digest().bytes();
                    let n = base64::encode_config_slice(&d, base64::STANDARD, &mut key_buf);
                    &key_buf[.. n]
                };
                buf.extend_from_slice(b"HTTP/1.1 101 Switching Protocols");
                buf.extend_from_slice(b"\r\nServer: soketto-");
                buf.extend_from_slice(SOKETTO_VERSION.as_bytes());
                buf.extend_from_slice(b"\r\nUpgrade: websocket\r\nConnection: upgrade");
                buf.extend_from_slice(b"\r\nSec-WebSocket-Accept: ");
                buf.extend_from_slice(accept_value);
                if let Some(p) = accept.protocol {
                    buf.extend_from_slice(b"\r\nSec-WebSocket-Protocol: ");
                    buf.extend_from_slice(p.as_bytes())
                }
                append_extensions(self.extensions.iter().filter(|e| e.is_enabled()), buf);
                buf.extend_from_slice(b"\r\n\r\n")
            }
            Response::Reject(rej) => {
                buf.extend_from_slice(b"HTTP/1.1 ");
                let s = StatusCode::from_u16(rej.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                buf.extend_from_slice(s.as_str().as_bytes());
                buf.extend_from_slice(b" ");
                buf.extend_from_slice(s.canonical_reason().unwrap_or("N/A").as_bytes());
                buf.extend_from_slice(b"\r\n\r\n")
            }
        }
    }
}

/// Handshake request received from the client.
#[derive(Debug)]
pub struct ClientRequest<'a> {
    ws_key: &'a [u8],
    protocols: SmallVec<[&'a str; 4]>
}

impl<'a> ClientRequest<'a> {
    /// A reference to the nonce.
    pub fn key(&self) -> &[u8] {
        self.ws_key
    }

    /// The protocols the client is proposing.
    pub fn protocols(&self) -> impl Iterator<Item = &str> {
        self.protocols.iter().cloned()
    }
}

/// Handshake response the server sends back to the client.
#[derive(Debug)]
pub enum Response<'a> {
    /// The server accepts the handshake request.
    Accept(Accept<'a>),
    /// The server rejects the handshake request.
    Reject(Reject)
}

/// Successful handshake response the server wants to send to the client.
#[derive(Debug)]
pub struct Accept<'a> {
    key: &'a [u8],
    protocol: Option<&'a str>
}

impl<'a> Accept<'a> {
    /// Create a new accept response.
    ///
    /// The `key` corresponds to the websocket key (nonce) the client has
    /// sent in its handshake request.
    pub fn new(key: &'a [u8]) -> Self {
        Accept {
            key: key,
            protocol: None
        }
    }

    /// Set the protocol the server selected from the proposed ones.
    pub fn set_protocol(&mut self, p: &'a str) -> &mut Self {
        self.protocol = Some(p);
        self
    }
}

/// Error handshake response the server wants to send to the client.
#[derive(Debug)]
pub struct Reject {
    /// HTTP response status code.
    code: u16
}

impl Reject {
    /// Create a new reject response with the given HTTP status code.
    pub fn new(code: u16) -> Self {
        Reject { code }
    }
}


