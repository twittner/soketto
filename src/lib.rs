// Copyright (c) 2019 Parity Technologies (UK) Ltd.
// Copyright (c) 2016 twist developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! An implementation of the [RFC 6455][rfc6455] websocket protocol.
//!
//! To begin a websocket connection one first needs to perform a [handshake],
//! either as [client] or [server], in order to upgrade from HTTP.
//! Once successful, the client or server can transition to a connection,
//! i.e. a [Sender]/[Receiver] pair and send and receive textual or
//! binary data.
//!
//! **Note**: While it is possible to only receive websocket messages it is
//! not possible to only send websocket messages. Receiving data is required
//! in order to react to control frames such as PING or CLOSE. While those will be
//! answered transparently they have to be received in the first place, so
//! calling [`connection::Receiver::receive`] is imperative.
//!
//! **Note**: None of the `async` methods are safe to cancel so their `Future`s
//! must not be dropped unless they return `Poll::Ready`.
//!
//! # Client example
//!
//! ```no_run
//! # use async_std::net::TcpStream;
//! # let _: Result<(), soketto::BoxedError> = async_std::task::block_on(async {
//! use soketto::handshake::{Client, ServerResponse};
//!
//! // First, we need to establish a TCP connection.
//! let socket = TcpStream::connect("...").await?;
//!
//! // Then we configure the client handshake.
//! let mut client = Client::new(socket, "...", "/");
//!
//! // And finally we perform the handshake and handle the result.
//! let (mut sender, mut receiver) = match client.handshake().await? {
//!     ServerResponse::Accepted { .. } => client.into_builder().finish(),
//!     ServerResponse::Redirect { status_code, location } => unimplemented!("follow location URL"),
//!     ServerResponse::Rejected { status_code } => unimplemented!("handle failure")
//! };
//!
//! // Over the established websocket connection we can send
//! sender.send_data("some text").await?;
//! sender.send_data("some more text").await?;
//! sender.flush().await?;
//!
//! // ... and receive data.
//! let data = receiver.receive_data().await?;
//!
//! # Ok(())
//! # });
//!
//! ```
//!
//! # Server example
//!
//! ```no_run
//! # use async_std::{net::TcpListener, prelude::*};
//! # let _: Result<(), soketto::BoxedError> = async_std::task::block_on(async {
//! use soketto::handshake::{Server, ClientRequest, server::Response};
//!
//! // First, we listen for incoming connections.
//! let listener = TcpListener::bind("...").await?;
//! let mut incoming = listener.incoming();
//!
//! while let Some(socket) = incoming.next().await {
//!     // For each incoming connection we perform a handshake.
//!     let mut server = Server::new(socket?);
//!
//!     let websocket_key = {
//!         let req = server.receive_request().await?;
//!         req.into_key()
//!     };
//!
//!     // Here we accept the client unconditionally.
//!     let accept = Response::Accept { key: &websocket_key, protocol: None };
//!     server.send_response(&accept).await?;
//!
//!     // And we can finally transition to a websocket connection.
//!     let (mut sender, mut receiver) = server.into_builder().finish();
//!     let message = receiver.receive_data().await?;
//!     sender.send_data(message).await?;
//!     sender.close().await?;
//! }
//!
//! # Ok(())
//! # });
//!
//! ```
//! [client]: handshake::Client
//! [server]: handshake::Server
//! [Sender]: connection::Sender
//! [Receiver]: connection::Receiver
//! [rfc6455]: https://tools.ietf.org/html/rfc6455
//! [handshake]: https://tools.ietf.org/html/rfc6455#section-4

pub mod base;
pub mod data;
pub mod extension;
pub mod handshake;
pub mod connection;

use bytes::{BufMut, BytesMut};
use futures::io::{AsyncRead, AsyncReadExt};
pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// A parsing result.
#[derive(Debug, Clone)]
pub enum Parsing<T, N = ()> {
    /// Parsing completed.
    Done {
        /// The parsed value.
        value: T,
        /// The offset into the byte slice that has been consumed.
        offset: usize
    },
    /// Parsing is incomplete and needs more data.
    NeedMore(N)
}

/// Helper function to allow casts from `usize` to `u64` only on platforms
/// where the sizes are guaranteed to fit.
#[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]
const fn as_u64(a: usize) -> u64 {
    a as u64
}

/// Reserve (and initialise) additional bytes.
pub(crate) fn reserve(bytes: &mut BytesMut, additional: usize) {
    let n = bytes.len();
    bytes.resize(n + additional, 0);
    unsafe { bytes.set_len(n) }
}

/// Helper to read from an `AsyncRead` resource into some buffer.
pub(crate) async fn read<R>(r: &mut R, b: &mut BytesMut) -> Result<(), std::io::Error>
where
    R: AsyncRead + Unpin
{
    unsafe {
        let n = r.read(b.bytes_mut()).await?;
        if n == 0 && b.has_remaining_mut() {
            return Err(std::io::ErrorKind::UnexpectedEof.into())
        }
        b.advance_mut(n);
        log::trace!("read {} bytes", n)
    }
    Ok(())
}
