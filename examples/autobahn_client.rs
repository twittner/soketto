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
// Once started, the tests can be executed with: wstest -m fuzzingserver
//
// See https://github.com/crossbario/autobahn-testsuite for details.

use assert_matches::assert_matches;
use async_std::{net::TcpStream, task};
use soketto::{BoxedError, handshake};
use std::str::FromStr;

const SOKETTO_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<(), BoxedError> {
    env_logger::init();
    task::block_on(async {
        let mut buf = Vec::new();
        let n = num_of_cases(&mut buf).await?;
        for i in 1 ..= n {
            if let Err(e) = run_case(i, &mut buf).await {
                log::debug!("case {}: {:?}", i, e)
            }
        }
        update_report(&mut buf).await?;
        Ok(())
    })
}

async fn num_of_cases(buf: &mut Vec<u8>) -> Result<usize, BoxedError> {
    let s = TcpStream::connect("127.0.0.1:9001").await?;
    let mut c = new_client(s, "/getCaseCount");
    assert_matches!(c.handshake(buf).await?, handshake::ServerResponse::Accepted(_));
    let mut c = c.into_connection(true);
    assert!(c.receive(buf).await?);
    Ok(usize::from_str(std::str::from_utf8(buf)?)?)
}

async fn run_case(n: usize, buf: &mut Vec<u8>) -> Result<(), BoxedError> {
    let resource = format!("/runCase?case={}&agent=soketto-{}", n, SOKETTO_VERSION);
    let s = TcpStream::connect("127.0.0.1:9001").await?;
    let mut c = new_client(s, &resource);
    assert_matches!(c.handshake(buf).await?, handshake::ServerResponse::Accepted(_));
    let mut c = c.into_connection(true);
    c.validate_utf8(true);
    loop {
        buf.clear();
        let is_text = c.receive(buf).await?;
        if buf.is_empty() {
            break
        }
        if is_text {
            c.send_text(buf).await?
        } else {
            c.send_binary(buf).await?
        }
    }
    Ok(())
}

async fn update_report(buf: &mut Vec<u8>) -> Result<(), BoxedError> {
    let resource = format!("/updateReports?agent=soketto-{}", SOKETTO_VERSION);
    let s = TcpStream::connect("127.0.0.1:9001").await?;
    let mut c = new_client(s, &resource);
    assert_matches!(c.handshake(buf).await?, handshake::ServerResponse::Accepted(_));
    c.into_connection(true).close().await?;
    Ok(())
}

#[cfg(not(feature = "deflate"))]
fn new_client(socket: TcpStream, path: &str) -> handshake::Client<'_, TcpStream> {
    handshake::Client::new(socket, "127.0.0.1:9001", path)
}

#[cfg(feature = "deflate")]
fn new_client(socket: TcpStream, path: &str) -> handshake::Client<'_, TcpStream> {
    let mut client = handshake::Client::new(socket, "127.0.0.1:9001", path);
    let deflate = soketto::extension::deflate::Deflate::new(soketto::connection::Mode::Client);
    client.add_extension(Box::new(deflate));
    client
}

