//! A SECS-II message: stream, function, W-bit and an optional item.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CodecError;
use crate::item::Item;

/// One SECS-II message as it travels in an HSMS data message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecsMessage {
    /// 0..=127.
    pub stream: u8,
    pub function: u8,
    /// The sender wants a reply.
    pub w_bit: bool,
    /// The message text; `None` for a header-only message such as `S1F1`.
    pub item: Option<Item>,
}

impl SecsMessage {
    pub fn new(stream: u8, function: u8, w_bit: bool, item: Option<Item>) -> Self {
        SecsMessage {
            stream: stream & 0x7f,
            function,
            w_bit,
            item,
        }
    }

    /// A primary message that expects a reply (W-bit set).
    pub fn request(stream: u8, function: u8, item: Option<Item>) -> Self {
        Self::new(stream, function, true, item)
    }

    /// A message that wants no reply (W-bit clear).
    pub fn notify(stream: u8, function: u8, item: Option<Item>) -> Self {
        Self::new(stream, function, false, item)
    }

    /// A primary message is odd-numbered; function 0 is the abort reply of its stream.
    pub fn is_primary(&self) -> bool {
        !self.function.is_multiple_of(2)
    }

    /// `S1F13`.
    pub fn name(&self) -> String {
        format!("S{}F{}", self.stream, self.function)
    }

    /// The secondary message answering this one (function + 1, no W-bit).
    pub fn reply(&self, item: Option<Item>) -> SecsMessage {
        SecsMessage::new(self.stream, self.function.wrapping_add(1), false, item)
    }

    /// The `S{n}F0` abort answering this one.
    pub fn abort(&self) -> SecsMessage {
        SecsMessage::new(self.stream, 0, false, None)
    }

    /// Header byte 2: W-bit and stream.
    pub(crate) fn byte2(&self) -> u8 {
        (u8::from(self.w_bit) << 7) | (self.stream & 0x7f)
    }

    /// The encoded message text (empty when there is no item).
    pub fn encode_text(&self) -> Result<Vec<u8>, CodecError> {
        match &self.item {
            Some(item) => item.try_encode(),
            None => Ok(Vec::new()),
        }
    }

    /// Rebuild a message from header bytes 2 and 3 and the text.
    pub fn from_parts(byte2: u8, function: u8, text: &[u8]) -> Result<Self, CodecError> {
        let item = if text.is_empty() {
            None
        } else {
            Some(Item::decode(text)?)
        };
        Ok(SecsMessage {
            stream: byte2 & 0x7f,
            function,
            w_bit: byte2 & 0x80 != 0,
            item,
        })
    }
}

impl fmt::Display for SecsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_sml())
    }
}
