// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

// Example to be used with the autobahn test suite, a fully automated test
// suite to verify client and server implementations of websocket
// implementation.
//
// Once started, the tests can be executed with: wstest -m fuzzingclient
//
// See https://github.com/crossbario/autobahn-testsuite for details.

use async_std::{net::{TcpListener, TcpStream}, prelude::*, task};
use bytes::BytesMut;
use soketto::{BoxedError, handshake};

fn main() -> Result<(), BoxedError> {
    env_logger::init();
    task::block_on(async {
        let mut buf = BytesMut::new();
        let listener = TcpListener::bind("127.0.0.1:9001").await?;
        let mut incoming = listener.incoming();
        while let Some(s) = incoming.next().await {
            let mut s = new_server(s?);
            let key = {
                let req = s.receive_request(&mut buf).await?;
                req.into_key()
            };
            let accept = handshake::server::Response::Accept { key: &key, protocol: None };
            s.send_response(&mut buf, &accept).await?;
            let mut c = s.into_connection(true);
            c.validate_utf8(true);
            loop {
                let is_text = c.receive(&mut buf).await?;
                if buf.is_empty() {
                    break
                }
                if is_text {
                    c.send_text(&mut buf).await?
                } else {
                    c.send_binary(&mut buf).await?
                }
            }
        }
        Ok(())
    })
}

#[cfg(not(feature = "deflate"))]
fn new_server<'a>(socket: TcpStream) -> handshake::Server<'a, TcpStream> {
    handshake::Server::new(socket)
}

#[cfg(feature = "deflate")]
fn new_server<'a>(socket: TcpStream) -> handshake::Server<'a, TcpStream> {
    let mut server = handshake::Server::new(socket);
    let deflate = soketto::extension::deflate::Deflate::new(soketto::connection::Mode::Server);
    server.add_extension(Box::new(deflate));
    server

}

