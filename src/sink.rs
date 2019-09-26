// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

use futures::{prelude::*, ready};
use std::{pin::Pin, task::{Context, Poll}};

pub fn unfold<S, F, T, A, E>(init: S, f: F) -> Unfold<S, F, T, A, E>
where
    F: FnMut(S, Command<A>) -> T,
    T: Future<Output = Result<S, E>>,
{
    Unfold {
        lambda: f,
        future: None,
        param: Some(init),
        state: State::Empty,
        _mark: std::marker::PhantomData
    }
}

#[derive(Debug)]
pub enum Command<A> {
    Send(A),
    Flush,
    Close
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Empty,
    Sending,
    Flushing,
    Closing,
    Closed
}

#[derive(Debug)]
pub struct Unfold<S, F, T, A, E> {
    lambda: F,
    future: Option<T>,
    param: Option<S>,
    state: State,
    _mark: std::marker::PhantomData<(A, E)>
}

impl<S, F, T, A, E> Unfold<S, F, T, A, E> {
    fn lambda(self: Pin<&mut Self>) -> &mut F {
        unsafe {
            &mut self.get_unchecked_mut().lambda
        }
    }

    fn future(self: Pin<&mut Self>) -> Pin<&mut Option<T>> {
        unsafe {
            self.map_unchecked_mut(|s| &mut s.future)
        }
    }

    fn param(self: Pin<&mut Self>) -> &mut Option<S> {
        unsafe {
            &mut self.get_unchecked_mut().param
        }
    }

    fn state(self: Pin<&mut Self>) -> &mut State {
        unsafe {
            &mut self.get_unchecked_mut().state
        }
    }
}

impl<S, F, T: Unpin, A, E> Unpin for Unfold<S, F, T, A, E> {}

impl<S, F, T, A, E> Sink<A> for Unfold<S, F, T, A, E>
where
    F: FnMut(S, Command<A>) -> T,
    T: Future<Output = Result<S, E>>
{
    type Error = E;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        match self.as_mut().state() {
            State::Sending | State::Flushing => {
                match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                    Ok(p) => {
                        *self.as_mut().param() = Some(p);
                        *self.as_mut().state() = State::Empty;
                        Poll::Ready(Ok(()))
                    }
                    Err(e) => Poll::Ready(Err(e))
                }
            }
            State::Closing => {
                match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                    Ok(p) => {
                        *self.as_mut().param() = Some(p);
                        *self.as_mut().state() = State::Closed;
                        Poll::Ready(Ok(()))
                    }
                    Err(e) => Poll::Ready(Err(e))
                }
            }
            State::Empty | State::Closed => Poll::Ready(Ok(()))
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: A) -> Result<(), Self::Error> {
        assert_eq!(State::Empty, *self.as_mut().state());
        if let Some(p) = self.as_mut().param().take() {
            let future = (self.as_mut().lambda())(p, Command::Send(item));
            self.as_mut().future().set(Some(future));
            *self.as_mut().state() = State::Sending
        }
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        loop {
            match self.as_mut().state() {
                State::Empty =>
                    if let Some(p) = self.as_mut().param().take() {
                        let future = (self.as_mut().lambda())(p, Command::Flush);
                        self.as_mut().future().set(Some(future));
                        *self.as_mut().state() = State::Flushing
                    } else {
                        return Poll::Ready(Ok(()))
                    }
                State::Sending =>
                    match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                        Ok(p) => {
                            *self.as_mut().param() = Some(p);
                            *self.as_mut().state() = State::Empty
                        }
                        Err(e) => return Poll::Ready(Err(e))
                    }
                State::Flushing =>
                    match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                        Ok(p) => {
                            *self.as_mut().param() = Some(p);
                            *self.as_mut().state() = State::Empty;
                            return Poll::Ready(Ok(()))
                        }
                        Err(e) => return Poll::Ready(Err(e))
                    }
                State::Closing =>
                    match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                        Ok(p) => {
                            *self.as_mut().param() = Some(p);
                            *self.as_mut().state() = State::Closed;
                            return Poll::Ready(Ok(()))
                        }
                        Err(e) => return Poll::Ready(Err(e))
                    }
                State::Closed => return Poll::Ready(Ok(()))
            }
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        loop {
            match self.as_mut().state() {
                State::Empty =>
                    if let Some(p) = self.as_mut().param().take() {
                        let future = (self.as_mut().lambda())(p, Command::Close);
                        self.as_mut().future().set(Some(future));
                        *self.as_mut().state() = State::Closing;
                    } else {
                        return Poll::Ready(Ok(()))
                    }
                State::Sending =>
                    match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                        Ok(p) => {
                            *self.as_mut().param() = Some(p);
                            *self.as_mut().state() = State::Empty
                        }
                        Err(e) => return Poll::Ready(Err(e))
                    }
                State::Flushing =>
                    match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                        Ok(p) => {
                            *self.as_mut().param() = Some(p);
                            *self.as_mut().state() = State::Empty
                        }
                        Err(e) => return Poll::Ready(Err(e))
                    }
                State::Closing =>
                    match ready!(self.as_mut().future().as_pin_mut().unwrap().poll(cx)) {
                        Ok(p) => {
                            *self.as_mut().param() = Some(p);
                            *self.as_mut().state() = State::Closed;
                            return Poll::Ready(Ok(()))
                        }
                        Err(e) => return Poll::Ready(Err(e))
                    }
                State::Closed => return Poll::Ready(Ok(()))
            }
        }
    }
}

