// Copyright (c) 2019 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! Types describing various forms of payload data.

use std::{convert::TryFrom, fmt};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Incoming<'a> {
    /// Text or binary data.
    Data(Data),
    Pong(&'a [u8])
}

impl Incoming<'_> {
    /// Is this text or binary data?
    pub fn is_data(&self) -> bool {
        if let Incoming::Data(_) = self { true } else { false }
    }

    /// Is this a PONG?
    pub fn is_pong(&self) -> bool {
        if let Incoming::Pong(_) = self { true } else { false }
    }

    /// Is this text data?
    pub fn is_text(&self) -> bool {
        if let Incoming::Data(d) = self {
            d.is_text()
        } else {
            false
        }
    }

    /// Is this binary data?
    pub fn is_binary(&self) -> bool {
        if let Incoming::Data(d) = self {
            d.is_binary()
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Data {
    /// Binary data.
    Binary,
    /// UTF-8 encoded data.
    Text
}

impl Data {
    /// Is this binary data?
    pub fn is_binary(&self) -> bool {
        if let Data::Binary = self { true } else { false }
    }

    /// Is this UTF-8 encoded textual data?
    pub fn is_text(&self) -> bool {
        if let Data::Text = self { true } else { false }
    }
}

/// Wrapper which restricts the length of its byte slice to 125 bytes.
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

