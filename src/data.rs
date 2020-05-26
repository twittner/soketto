// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! Types describing various forms of payload data.

use std::{convert::TryFrom, fmt};

/// The various types of incoming data.
///
/// A PONG may be received unsolicited or as an answer to a PING.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Incoming<'a> {
    /// Textual data.
    Text,
    /// Binary data.
    Binary,
    /// PONG payload data.
    Pong(&'a [u8])
}

impl Incoming<'_> {
    /// Is this a PONG?
    pub fn is_pong(&self) -> bool {
        if let Incoming::Pong(_) = self { true } else { false }
    }

    /// Is this text data?
    pub fn is_text(&self) -> bool {
        if let Incoming::Text = self { true } else { false }
    }

    /// Is this binary data?
    pub fn is_binary(&self) -> bool {
        if let Incoming::Binary = self { true } else { false }
    }
}

/// The types of payload data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataType {
    /// Binary data.
    Binary,
    /// Textual data.
    Text
}

impl DataType {
    /// Is this binary data?
    pub fn is_binary(&self) -> bool {
        if let DataType::Binary = self { true } else { false }
    }

    /// Is this UTF-8 encoded textual data?
    pub fn is_text(&self) -> bool {
        if let DataType::Text = self { true } else { false }
    }
}

/// Payload data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Data {
    /// Binary data.
    Binary(Vec<u8>),
    /// UTF-8 encoded data.
    Text(Vec<u8>)
}

impl Data {
    /// Is this binary data?
    pub fn is_binary(&self) -> bool {
        if let Data::Binary(_) = self { true } else { false }
    }

    /// Is this UTF-8 encoded textual data?
    pub fn is_text(&self) -> bool {
        if let Data::Text(_) = self { true } else { false }
    }
}

/// Wrapper type which restricts the length of its byte slice to 125 bytes.
#[derive(Debug)]
pub struct ByteSlice125<'a>(&'a [u8]);

/// Error, if converting to [`ByteSlice125`] fails.
#[derive(Clone, Debug)]
pub struct SliceTooLarge(());

impl fmt::Display for SliceTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Slice larger than 125 bytes")
    }
}

impl std::error::Error for SliceTooLarge {}

impl<'a> TryFrom<&'a [u8]> for ByteSlice125<'a> {
    type Error = SliceTooLarge;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        if value.len() > 125 {
            Err(SliceTooLarge(()))
        } else {
            Ok(ByteSlice125(value))
        }
    }
}

impl AsRef<[u8]> for ByteSlice125<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

