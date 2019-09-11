// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

use crate::{
    BoxedError,
    Parsing,
    connection::{Connection, Mode},
    handshake::{self, server::ClientRequest, client::ServerResponse}
};
use futures::prelude::*;

const BLOCK_SIZE: usize = 4096;

#[derive(Debug)] pub enum Client {}
#[derive(Debug)] pub enum Server {}

#[derive(Debug)]
pub struct WebSocket<T, M> {
    buffer: Vec<u8>,
    socket: T,
    _maker: std::marker::PhantomData<M>
}

impl<T: AsyncRead + AsyncWrite + Unpin> WebSocket<T, Client> {
    /// Create a new client websocket based on the given async I/O resource.
    pub fn client(socket: T) -> Self {
        WebSocket {
            buffer: Vec::new(),
            socket,
            _maker: std::marker::PhantomData
        }
    }

    /// Initiate a client to server handshake and get back the server response.
    pub async fn handshake<'a>
        ( &'a mut self
        , client: &mut handshake::Client<'_>
        ) -> Result<ServerResponse<'a>, BoxedError>
    {
        self.buffer.clear();
        client.encode_request(&mut self.buffer);
        self.socket.write_all(&self.buffer).await?;

        self.buffer.clear();
        self.buffer.resize(BLOCK_SIZE, 0);
        let mut offset = 0;
        loop {
            let buf_slice = {
                let p = self.buffer.as_mut_ptr();
                let n = self.buffer.len();
                unsafe {
                    std::slice::from_raw_parts_mut(p, n)
                }
            };
            offset += self.socket.read(&mut buf_slice[offset ..]).await?;
            match client.decode_response(&buf_slice[.. offset])? {
                Parsing::NeedMore(()) =>
                    if self.buffer.len() == self.buffer.capacity() {
                        self.buffer.resize(self.buffer.len() + BLOCK_SIZE, 0)
                    }
                Parsing::Done { value, .. } => return Ok(value)
            }
        }
    }

    pub fn into_connection(self) -> Connection<T> {
        Connection::new(self.socket, Mode::Client)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> WebSocket<T, Server> {
    /// Create a new client websocket based on the given async I/O resource.
    pub fn server(socket: T) -> Self {
        WebSocket {
            buffer: Vec::new(),
            socket,
            _maker: std::marker::PhantomData
        }
    }

    /// Await a client initiated handshake request.
    pub async fn handshake_request<'a>
        ( &'a mut self
        , server: &mut handshake::Server<'_>
        ) -> Result<ClientRequest<'a>, BoxedError>
    {
        self.buffer.clear();
        self.buffer.resize(BLOCK_SIZE, 0);
        let mut offset = 0;
        loop {
            let buf_slice = {
                let p = self.buffer.as_mut_ptr();
                let n = self.buffer.len();
                unsafe {
                    std::slice::from_raw_parts_mut(p, n)
                }
            };
            offset += self.socket.read(&mut buf_slice[offset ..]).await?;
            match server.decode_request(&buf_slice[.. offset])? {
                Parsing::NeedMore(()) =>
                    if self.buffer.len() == self.buffer.capacity() {
                        self.buffer.resize(self.buffer.len() + BLOCK_SIZE, 0)
                    }
                Parsing::Done { value, .. } => return Ok(value)
            }
        }
    }

    /// Send back a handshake response.
    pub async fn handshake_response
        ( &mut self
        , server: &mut handshake::Server<'_>
        , response: &handshake::server::Response<'_>
        ) -> Result<(), BoxedError>
    {
        self.buffer.clear();
        server.encode_response(response, &mut self.buffer);
        self.socket.write_all(&self.buffer).await?;
        Ok(())
    }

    pub fn into_connection(self) -> Connection<T> {
        Connection::new(self.socket, Mode::Server)
    }
}
