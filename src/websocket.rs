// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

use crate::{BoxedError, handshake::{self, Client}, Parsing};
use futures::prelude::*;

const BLOCK_SIZE: usize = 4096;

#[derive(Debug)]
pub struct WebSocket<T> {
    socket: T,
    buffer: Vec<u8>,
    roffset: usize, // read offset
    woffset: usize, // write offset
}

impl<T: AsyncRead + AsyncWrite + Unpin> WebSocket<T> {
    pub fn new(socket: T) -> Self {
        WebSocket {
            socket,
            buffer: Vec::new(),
            roffset: 0,
            woffset: 0
        }
    }

    pub async fn client_handshake<'a>
        ( &'a mut self
        , client: &mut Client<'_>
        ) -> Result<handshake::client::Response<'a>, BoxedError>
    {
        self.buffer.clear();
        client.encode_request(&mut self.buffer);
        self.socket.write_all(&self.buffer).await?;
        self.buffer.clear();
        self.buffer.resize(BLOCK_SIZE, 0);

        self.roffset = 0;
        self.woffset = 0;
        loop {
            let buf_slice = {
                let p = self.buffer.as_mut_ptr();
                let n = self.buffer.len();
                unsafe {
                    std::slice::from_raw_parts_mut(p, n)
                }
            };
            self.woffset += self.socket.read(&mut buf_slice[self.woffset ..]).await?;
            match client.decode_response(&buf_slice[.. self.woffset])? {
                Parsing::NeedMore(()) =>
                    if self.buffer.len() == self.buffer.capacity() {
                        self.buffer.resize(self.buffer.len() + BLOCK_SIZE, 0)
                    }
                Parsing::Done { value, offset } => {
                    self.roffset = offset;
                    return Ok(value)
                }
            }
        }
    }
}


