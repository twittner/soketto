// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! Websocket client [handshake].
//!
//! [handshake]: https://tools.ietf.org/html/rfc6455#section-4

use bytes::BytesMut;
use crate::{Parsing, connection::{Connection, Mode}, extension::Extension};
use futures::prelude::*;
use sha1::Sha1;
use smallvec::SmallVec;
use std::{fmt, str};
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

/// Websocket client handshake.
#[derive(Debug)]
pub struct Client<'a, T> {
    /// The underlying async I/O resource.
    socket: T,
    /// The HTTP host to send the handshake to.
    host: &'a str,
    /// The HTTP host ressource.
    resource: &'a str,
    /// The HTTP origin header.
    origin: Option<&'a str>,
    /// A buffer holding the base-64 encoded request nonce.
    nonce: [u8; 32],
    /// The offset into the nonce buffer.
    nonce_offset: usize,
    /// The protocols to include in the handshake.
    protocols: SmallVec<[&'a str; 4]>,
    /// The extensions the client wishes to include in the request.
    extensions: SmallVec<[Box<dyn Extension + Send>; 4]>
}

impl<'a, T: AsyncRead + AsyncWrite + Unpin> Client<'a, T> {
    /// Create a new client handshake for some host and resource.
    pub fn new(socket: T, host: &'a str, resource: &'a str) -> Self {
        Client {
            socket,
            host,
            resource,
            origin: None,
            nonce: [0; 32],
            nonce_offset: 0,
            protocols: SmallVec::new(),
            extensions: SmallVec::new()
        }
    }

    /// Set the handshake origin header.
    pub fn set_origin(&mut self, o: &'a str) -> &mut Self {
        self.origin = Some(o);
        self
    }

    /// Add a protocol to be included in the handshake.
    pub fn add_protocol(&mut self, p: &'a str) -> &mut Self {
        self.protocols.push(p);
        self
    }

    /// Add an extension to be included in the handshake.
    pub fn add_extension(&mut self, e: Box<dyn Extension + Send>) -> &mut Self {
        self.extensions.push(e);
        self
    }

    /// Get back all extensions.
    pub fn drain_extensions(&mut self) -> impl Iterator<Item = Box<dyn Extension + Send>> + '_ {
        self.extensions.drain()
    }

    /// Initiate client handshake request to server and get back the response.
    pub async fn handshake(&mut self, buf: &mut BytesMut) -> Result<ServerResponse, Error> {
        buf.clear();
        self.encode_request(buf);
        self.socket.write_all(buf).await?;
        self.socket.flush().await?;

        buf.clear();
        let mut offset = 0;
        loop {
            if buf.len() == offset {
                buf.resize(offset + BLOCK_SIZE, 0)
            }
            offset += self.socket.read(&mut buf[offset ..]).await?;
            if let Parsing::Done { value, offset } = self.decode_response(buf)? {
                buf.split_to(offset);
                return Ok(value)
            }
        }
    }

    /// Turn this handshake into a [`Connection`].
    ///
    /// If `take_over_extensions` is true, the extensions from this
    /// handshake will be set on the `Connection` returned.
    pub fn into_connection(mut self, take_over_extensions: bool) -> Connection<T> {
        let mut c = Connection::new(self.socket, Mode::Client);
        if take_over_extensions {
            c.add_extensions(self.extensions.drain());
        }
        c
    }

    /// Encode the client handshake as a request, ready to be sent to the server.
    fn encode_request(&mut self, buf: &mut BytesMut) {
        let nonce: [u8; 16] = rand::random();
        self.nonce_offset = base64::encode_config_slice(&nonce, base64::STANDARD, &mut self.nonce);
        buf.extend_from_slice(b"GET ");
        buf.extend_from_slice(self.resource.as_bytes());
        buf.extend_from_slice(b" HTTP/1.1");
        buf.extend_from_slice(b"\r\nHost: ");
        buf.extend_from_slice(self.host.as_bytes());
        buf.extend_from_slice(b"\r\nUpgrade: websocket\r\nConnection: upgrade");
        buf.extend_from_slice(b"\r\nSec-WebSocket-Key: ");
        buf.extend_from_slice(&self.nonce[.. self.nonce_offset]);
        if let Some(o) = &self.origin {
            buf.extend_from_slice(b"\r\nOrigin: ");
            buf.extend_from_slice(o.as_bytes())
        }
        if let Some((last, prefix)) = self.protocols.split_last() {
            buf.extend_from_slice(b"\r\nSec-WebSocket-Protocol: ");
            for p in prefix {
                buf.extend_from_slice(p.as_bytes());
                buf.extend_from_slice(b",")
            }
            buf.extend_from_slice(last.as_bytes())
        }
        append_extensions(&self.extensions, buf);
        buf.extend_from_slice(b"\r\nSec-WebSocket-Version: 13\r\n\r\n")
    }

    /// Decode the server response to this client request.
    fn decode_response(&mut self, buf: &mut BytesMut) -> Result<Parsing<ServerResponse>, Error> {
        let mut header_buf = [httparse::EMPTY_HEADER; MAX_NUM_HEADERS];
        let mut response = httparse::Response::new(&mut header_buf);

        let offset = match response.parse(buf) {
            Ok(httparse::Status::Complete(off)) => off,
            Ok(httparse::Status::Partial) => return Ok(Parsing::NeedMore(())),
            Err(e) => return Err(Error::Http(Box::new(e)))
        };

        if response.version != Some(1) {
            return Err(Error::UnsupportedHttpVersion)
        }

        match response.code {
            Some(101) => (),
            Some(code@(301 ..= 303)) | Some(code@307) | Some(code@308) => { // redirect response
                let location = with_first_header(response.headers, "Location", |loc| {
                    Ok(String::from(std::str::from_utf8(loc)?))
                })?;
                buf.split_to(offset); // chop off the HTTP part we have processed
                let response = Redirect { status_code: code, location };
                return Ok(Parsing::Done { value: ServerResponse::Redirect(response), offset })
            }
            other => {
                buf.split_to(offset); // chop off the HTTP part we have processed
                let response = Rejected { code: other.unwrap_or(0) };
                return Ok(Parsing::Done { value: ServerResponse::Rejected(response), offset })
            }
        }

        expect_ascii_header(response.headers, "Upgrade", "websocket")?;
        expect_ascii_header(response.headers, "Connection", "upgrade")?;

        let nonce = &self.nonce[.. self.nonce_offset];
        with_first_header(&response.headers, "Sec-WebSocket-Accept", |theirs| {
            let mut digest = Sha1::new();
            digest.update(nonce);
            digest.update(KEY);
            let ours = base64::encode(&digest.digest().bytes());
            if ours.as_bytes() != theirs {
                return Err(Error::InvalidSecWebSocketAccept)
            }
            Ok(())
        })?;

        // Parse `Sec-WebSocket-Extensions` headers.

        for h in response.headers.iter()
            .filter(|h| h.name.eq_ignore_ascii_case(SEC_WEBSOCKET_EXTENSIONS))
        {
            configure_extensions(&mut self.extensions, std::str::from_utf8(h.value)?)?
        }

        // Match `Sec-WebSocket-Protocol` header.

        let mut selected_proto = None;
        if let Some(tp) = response.headers.iter()
            .find(|h| h.name.eq_ignore_ascii_case(SEC_WEBSOCKET_PROTOCOL))
        {
            if let Some(&p) = self.protocols.iter().find(|x| x.as_bytes() == tp.value) {
                selected_proto = Some(String::from(p))
            } else {
                return Err(Error::UnsolicitedProtocol)
            }
        }

        buf.split_to(offset); // chop off the HTTP part we have processed

        let response = Accepted { protocol: selected_proto };
        Ok(Parsing::Done { value: ServerResponse::Accepted(response), offset: 0 })
    }
}

/// Handshake response received from the server.
#[derive(Debug)]
pub enum ServerResponse {
    /// The server has accepted our request.
    Accepted(Accepted),
    /// The server is redirecting us to some other location.
    Redirect(Redirect),
    /// The server rejected our request.
    Rejected(Rejected)
}

/// The server accepted the handshake request.
#[derive(Debug)]
pub struct Accepted {
    /// The protocol (if any) the server has selected.
    protocol: Option<String>
}

impl Accepted {
    /// The protocol the server has selected from the proposed ones.
    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_ref().map(|s| &**s)
    }

    pub fn into_protocol(self) -> Option<String> {
        self.protocol
    }
}

/// Error handshake response received from the server.
#[derive(Debug)]
pub struct Rejected {
    /// HTTP response status code.
    code: u16
}

impl Rejected {
    /// The response code from the server.
    pub fn code(&self) -> u16 {
        self.code
    }
}

/// The server is redirecting us to another location.
#[derive(Debug)]
pub struct Redirect {
    /// The HTTP response status code.
    status_code: u16,
    /// The location URL we should go to.
    location: String
}

impl fmt::Display for Redirect {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "redirect: code = {}, location = \"{}\"", self.status_code, self.location)
    }
}

impl Redirect {
    /// The HTTP response status code.
    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    /// The HTTP response location header.
    pub fn location(&self) -> &str {
        &self.location
    }

    pub fn into_location(self) -> String {
        self.location
    }
}

