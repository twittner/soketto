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
use soketto::{BoxedError, WebSocket, handshake};
use std::str::FromStr;

const SOKETTO_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<(), BoxedError> {
    env_logger::init();
    task::block_on(async {
        let n = num_of_cases().await?;
        for i in 1 ..= n {
            if let Err(e) = run_case(i).await {
                log::debug!("case {}: {:?}", i, e)
            }
        }
//        update_report()?;
        Ok(())
    })
}

async fn num_of_cases() -> Result<usize, BoxedError> {
    let s = TcpStream::connect("127.0.0.1:9001").await?;
    let mut ws = WebSocket::client(s);
    let mut hs = handshake::Client::new("127.0.0.1:9001", "/getCaseCount");
    assert_matches!(ws.handshake(&mut hs).await?, handshake::ServerResponse::Accepted(_));
    let mut c = ws.into_connection();
    let mut v = Vec::new();
    assert!(c.receive(&mut v).await?);
    Ok(usize::from_str(std::str::from_utf8(&v)?)?)
}

async fn run_case(n: usize) -> Result<(), BoxedError> {
    let resource = format!("/runCase?case={}&agent=soketto-{}", n, SOKETTO_VERSION);
    let s = TcpStream::connect("127.0.0.1:9001").await?;
    let mut ws = WebSocket::client(s);
    let mut hs = handshake::Client::new("127.0.0.1:9001", &resource);
    assert_matches!(ws.handshake(&mut hs).await?, handshake::ServerResponse::Accepted(_));
    let mut c = ws.into_connection();
    let mut v = Vec::new();
    loop {
        v.clear();
        let is_text = c.receive(&mut v).await?;
        if v.is_empty() {
            break
        }
        if is_text {
            c.send_text(&mut v).await?
        } else {
            c.send_binary(&mut v).await?
        }
    }
    Ok(())
}
//
//fn update_report() -> Result<(), Box<dyn error::Error>> {
//    let addr = "127.0.0.1:9001".parse().unwrap();
//    TcpStream::connect(&addr)
//        .map_err(|e| Box::new(e) as Box<dyn error::Error>)
//        .and_then(|socket| {
//            let resource = format!("/updateReports?agent=soketto-{}", SOKETTO_VERSION);
//            let client = handshake::Client::new("127.0.0.1:9001", resource);
//            tokio::codec::Framed::new(socket, client)
//                .send(())
//                .map_err(|e| Box::new(e) as Box<dyn error::Error>)
//                .and_then(|framed| {
//                    framed.into_future().map_err(|(e, _)| Box::new(e) as Box<dyn error::Error>)
//                })
//                .and_then(|(response, framed)| {
//                    if response.is_none() {
//                        let e: io::Error = io::ErrorKind::ConnectionAborted.into();
//                        return Either::A(future::err(Box::new(e) as Box<dyn error::Error>))
//                    }
//                    let mut framed = {
//                        let codec = base::Codec::new();
//                        let old = framed.into_parts();
//                        let mut new = FramedParts::new(old.io, codec);
//                        new.read_buf = old.read_buf;
//                        new.write_buf = old.write_buf;
//                        let framed = Framed::from_parts(new);
//                        connection::Connection::from_framed(framed, connection::Mode::Client)
//                    };
//                    Either::B(future::poll_fn(move || {
//                        framed.close().map_err(|e| Box::new(e) as Box<dyn error::Error>)
//                    }))
//                })
//        })
//        .wait()
//}
//
//#[cfg(not(feature = "deflate"))]
//fn new_client<'a>(path: impl Into<Cow<'a, str>>) -> handshake::Client<'a> {
//    handshake::Client::new("127.0.0.1:9001", path)
//}
//
//#[cfg(feature = "deflate")]
//fn new_client<'a>(path: impl Into<Cow<'a, str>>) -> handshake::Client<'a> {
//    let mut client = handshake::Client::new("127.0.0.1:9001", path);
//    let deflate = soketto::extension::deflate::Deflate::new(connection::Mode::Client);
//    client.add_extension(Box::new(deflate));
//    client
//}

