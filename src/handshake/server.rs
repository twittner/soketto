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

use crate::{Parsing, extension::Extension};
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

const SOKETTO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Websocket handshake client.
#[derive(Debug, Default)]
pub struct Server<'a> {
    /// Protocols the server supports.
    protocols: SmallVec<[&'a str; 4]>,
    /// Extensions the server supports.
    extensions: SmallVec<[Box<dyn Extension + Send>; 4]>
}

impl<'a> Server<'a> {
    /// Create a new server handshake.
    pub fn new() -> Self {
        Server::default()
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

    // Decode client handshake request.
    pub fn decode_request(&mut self, bytes: &'a [u8]) -> Result<Parsing<Request<'a>>, Error> {
        let mut header_buf = [httparse::EMPTY_HEADER; MAX_NUM_HEADERS];
        let mut request = httparse::Request::new(&mut header_buf);

        let offset = match request.parse(bytes) {
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
            if let Some(x) = self.protocols.iter().find(|x| x.as_bytes() == p.value) {
                protocols.push(x.clone())
            }
        }

        Ok(Parsing::Done { value: Request { ws_key, protocols }, offset })
    }

    // Encode server handshake response.
    pub fn encode_response(&mut self, response: &Response, bytes: &mut Vec<u8>) {
        match response {
            Response::Accept(accept) => {
                let mut buffer = [0; 32];
                let accept_value = {
                    let mut digest = Sha1::new();
                    digest.update(&accept.key);
                    digest.update(KEY);
                    let d = digest.digest().bytes();
                    let n = base64::encode_config_slice(&d, base64::STANDARD, &mut buffer);
                    &buffer[.. n]
                };
                bytes.extend_from_slice(b"HTTP/1.1 101 Switching Protocols");
                bytes.extend_from_slice(b"\r\nServer: soketto-");
                bytes.extend_from_slice(SOKETTO_VERSION.as_bytes());
                bytes.extend_from_slice(b"\r\nUpgrade: websocket\r\nConnection: upgrade");
                bytes.extend_from_slice(b"\r\nSec-WebSocket-Accept: ");
                bytes.extend_from_slice(accept_value);
                if let Some(p) = accept.protocol {
                    bytes.extend_from_slice(b"\r\nSec-WebSocket-Protocol: ");
                    bytes.extend_from_slice(p.as_bytes())
                }
                append_extensions(self.extensions.iter().filter(|e| e.is_enabled()), bytes);
                bytes.extend_from_slice(b"\r\n\r\n")
            }
            Response::Reject(reject) => {
                bytes.extend_from_slice(b"HTTP/1.1 ");
                let s = StatusCode::from_u16(reject.code)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                bytes.extend_from_slice(s.as_str().as_bytes());
                bytes.extend_from_slice(b" ");
                bytes.extend_from_slice(s.canonical_reason().unwrap_or("N/A").as_bytes());
                bytes.extend_from_slice(b"\r\n\r\n")
            }
        }
    }
}

/// Handshake request received from the client.
#[derive(Debug)]
pub struct Request<'a> {
    ws_key: &'a [u8],
    protocols: SmallVec<[&'a str; 4]>
}

impl<'a> Request<'a> {
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


