//! SECS-II (SEMI E5) data items: encode and decode.
//!
//! An item is a format byte, 1 to 3 length bytes and the data. The format byte holds the format
//! code in its upper six bits and the number of length bytes (1-3) in its lower two. All numbers
//! are big endian. A list's length is its element count; every other item's length is in bytes.
//!
//! The encoder always uses the fewest length bytes; the decoder accepts any of the three forms.

use serde::{Deserialize, Serialize};

use crate::error::CodecError;

/// Deepest list nesting the decoder follows (a bound against hostile input).
pub const MAX_DEPTH: usize = 64;

/// Largest length a 3-byte length field can carry.
pub const MAX_ITEM_LEN: usize = 0x00FF_FFFF;

/// Format codes (the upper six bits of the format byte, shifted down).
pub mod format {
    pub const LIST: u8 = 0o00;
    pub const BINARY: u8 = 0o10;
    pub const BOOLEAN: u8 = 0o11;
    pub const ASCII: u8 = 0o20;
    pub const JIS8: u8 = 0o21;
    pub const I8: u8 = 0o30;
    pub const I1: u8 = 0o31;
    pub const I2: u8 = 0o32;
    pub const I4: u8 = 0o34;
    pub const F8: u8 = 0o40;
    pub const F4: u8 = 0o44;
    pub const U8: u8 = 0o50;
    pub const U1: u8 = 0o51;
    pub const U2: u8 = 0o52;
    pub const U4: u8 = 0o54;
}

/// One SECS-II data item. Numeric, binary and boolean items are arrays; a "single value" is an
/// array of one.
///
/// `Ascii` holds one char per byte (bytes above 0x7f map to U+0080..U+00FF), so any byte string
/// survives a decode/encode round trip. `Jis8` keeps the raw JIS-X-0201 bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Item {
    List(Vec<Item>),
    Binary(Vec<u8>),
    Boolean(Vec<bool>),
    Ascii(String),
    Jis8(Vec<u8>),
    I1(Vec<i8>),
    I2(Vec<i16>),
    I4(Vec<i32>),
    I8(Vec<i64>),
    U1(Vec<u8>),
    U2(Vec<u16>),
    U4(Vec<u32>),
    U8(Vec<u64>),
    F4(Vec<f32>),
    F8(Vec<f64>),
}

macro_rules! scalar_ctor {
    ($($name:ident, $variant:ident, $ty:ty;)*) => {
        $(
            /// A one-element array item.
            pub fn $name(v: $ty) -> Item {
                Item::$variant(vec![v])
            }
        )*
    };
}

impl Item {
    pub fn list(items: impl IntoIterator<Item = Item>) -> Item {
        Item::List(items.into_iter().collect())
    }

    pub fn ascii(s: impl Into<String>) -> Item {
        Item::Ascii(s.into())
    }

    pub fn binary(bytes: impl Into<Vec<u8>>) -> Item {
        Item::Binary(bytes.into())
    }

    /// An empty list, `<L [0]>`.
    pub fn empty_list() -> Item {
        Item::List(Vec::new())
    }

    scalar_ctor! {
        boolean, Boolean, bool;
        i1, I1, i8;
        i2, I2, i16;
        i4, I4, i32;
        i8, I8, i64;
        u1, U1, u8;
        u2, U2, u16;
        u4, U4, u32;
        u8, U8, u64;
        f4, F4, f32;
        f8, F8, f64;
    }

    /// A one-byte binary item.
    pub fn byte(v: u8) -> Item {
        Item::Binary(vec![v])
    }

    /// The format code (see [`format`]).
    pub fn format_code(&self) -> u8 {
        match self {
            Item::List(_) => format::LIST,
            Item::Binary(_) => format::BINARY,
            Item::Boolean(_) => format::BOOLEAN,
            Item::Ascii(_) => format::ASCII,
            Item::Jis8(_) => format::JIS8,
            Item::I1(_) => format::I1,
            Item::I2(_) => format::I2,
            Item::I4(_) => format::I4,
            Item::I8(_) => format::I8,
            Item::U1(_) => format::U1,
            Item::U2(_) => format::U2,
            Item::U4(_) => format::U4,
            Item::U8(_) => format::U8,
            Item::F4(_) => format::F4,
            Item::F8(_) => format::F8,
        }
    }

    /// The SML name of the format (`L`, `B`, `BOOLEAN`, `A`, `J`, `I1`...).
    pub fn type_name(&self) -> &'static str {
        match self {
            Item::List(_) => "L",
            Item::Binary(_) => "B",
            Item::Boolean(_) => "BOOLEAN",
            Item::Ascii(_) => "A",
            Item::Jis8(_) => "J",
            Item::I1(_) => "I1",
            Item::I2(_) => "I2",
            Item::I4(_) => "I4",
            Item::I8(_) => "I8",
            Item::U1(_) => "U1",
            Item::U2(_) => "U2",
            Item::U4(_) => "U4",
            Item::U8(_) => "U8",
            Item::F4(_) => "F4",
            Item::F8(_) => "F8",
        }
    }

    /// Number of elements (list entries, bytes of text, or array length).
    pub fn len(&self) -> usize {
        match self {
            Item::List(v) => v.len(),
            Item::Binary(v) | Item::Jis8(v) | Item::U1(v) => v.len(),
            Item::Boolean(v) => v.len(),
            Item::Ascii(s) => s.chars().count(),
            Item::I1(v) => v.len(),
            Item::I2(v) => v.len(),
            Item::I4(v) => v.len(),
            Item::I8(v) => v.len(),
            Item::U2(v) => v.len(),
            Item::U4(v) => v.len(),
            Item::U8(v) => v.len(),
            Item::F4(v) => v.len(),
            Item::F8(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn element_size(&self) -> usize {
        match self {
            Item::I2(_) | Item::U2(_) => 2,
            Item::I4(_) | Item::U4(_) | Item::F4(_) => 4,
            Item::I8(_) | Item::U8(_) | Item::F8(_) => 8,
            _ => 1,
        }
    }

    /// Bytes the item occupies on the wire.
    pub fn encoded_len(&self) -> usize {
        let body = match self {
            Item::List(items) => items.iter().map(Item::encoded_len).sum(),
            other => other.len() * other.element_size(),
        };
        1 + length_bytes(self.wire_length()) + body
    }

    fn wire_length(&self) -> usize {
        match self {
            Item::List(v) => v.len(),
            other => other.len() * other.element_size(),
        }
    }

    /// Encode to bytes. An item above 16 MiB - 1 gets a wrapped length field here; use
    /// [`Item::try_encode`] when the size is not under your control.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        self.encode_into(&mut out);
        out
    }

    /// Encode, failing when any item exceeds 16 MiB - 1.
    pub fn try_encode(&self) -> Result<Vec<u8>, CodecError> {
        self.check_lengths()?;
        Ok(self.encode())
    }

    fn check_lengths(&self) -> Result<(), CodecError> {
        let len = self.wire_length();
        if len > MAX_ITEM_LEN {
            return Err(CodecError::TooLong(len));
        }
        if let Item::List(items) = self {
            for i in items {
                i.check_lengths()?;
            }
        }
        Ok(())
    }

    /// Append the encoding to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let len = self.wire_length();
        let n = length_bytes(len);
        out.push((self.format_code() << 2) | n as u8);
        out.extend_from_slice(&(len as u32).to_be_bytes()[4 - n..]);
        match self {
            Item::List(items) => {
                for i in items {
                    i.encode_into(out);
                }
            }
            Item::Binary(v) | Item::Jis8(v) | Item::U1(v) => out.extend_from_slice(v),
            Item::Boolean(v) => out.extend(v.iter().map(|b| u8::from(*b))),
            Item::Ascii(s) => out.extend(
                s.chars()
                    .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')),
            ),
            Item::I1(v) => out.extend(v.iter().map(|x| *x as u8)),
            Item::I2(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::I4(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::I8(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::U2(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::U4(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::U8(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::F4(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            Item::F8(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
        }
    }

    /// Decode exactly one item that fills `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Item, CodecError> {
        let (item, used) = Item::decode_prefix(bytes)?;
        if used != bytes.len() {
            return Err(CodecError::TrailingBytes(bytes.len() - used));
        }
        Ok(item)
    }

    /// Decode the item at the start of `bytes`; returns it and the bytes it used.
    pub fn decode_prefix(bytes: &[u8]) -> Result<(Item, usize), CodecError> {
        decode_item(bytes, 0)
    }

    // ---- accessors -------------------------------------------------------------------------

    pub fn as_list(&self) -> Option<&[Item]> {
        match self {
            Item::List(v) => Some(v),
            _ => None,
        }
    }

    /// The text of an ASCII item (or of a JIS8 item that is plain ASCII).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Item::Ascii(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Item::Binary(v) | Item::Jis8(v) => Some(v),
            _ => None,
        }
    }

    /// First element of a boolean item.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Item::Boolean(v) => v.first().copied(),
            _ => None,
        }
    }

    /// First element of a binary item (the usual shape of an ACK code).
    pub fn as_byte(&self) -> Option<u8> {
        match self {
            Item::Binary(v) => v.first().copied(),
            _ => None,
        }
    }

    /// First element of any integer or binary item that is not negative and fits `u64`.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Item::Binary(v) | Item::U1(v) => v.first().map(|x| u64::from(*x)),
            Item::U2(v) => v.first().map(|x| u64::from(*x)),
            Item::U4(v) => v.first().map(|x| u64::from(*x)),
            Item::U8(v) => v.first().copied(),
            Item::I1(v) => v.first().and_then(|x| u64::try_from(*x).ok()),
            Item::I2(v) => v.first().and_then(|x| u64::try_from(*x).ok()),
            Item::I4(v) => v.first().and_then(|x| u64::try_from(*x).ok()),
            Item::I8(v) => v.first().and_then(|x| u64::try_from(*x).ok()),
            _ => None,
        }
    }

    /// First element of any integer or binary item that fits `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Item::Binary(v) | Item::U1(v) => v.first().map(|x| i64::from(*x)),
            Item::U2(v) => v.first().map(|x| i64::from(*x)),
            Item::U4(v) => v.first().map(|x| i64::from(*x)),
            Item::U8(v) => v.first().and_then(|x| i64::try_from(*x).ok()),
            Item::I1(v) => v.first().map(|x| i64::from(*x)),
            Item::I2(v) => v.first().map(|x| i64::from(*x)),
            Item::I4(v) => v.first().map(|x| i64::from(*x)),
            Item::I8(v) => v.first().copied(),
            _ => None,
        }
    }

    /// First element of any numeric item (integer or float) as `f64`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Item::F4(v) => v.first().map(|x| f64::from(*x)),
            Item::F8(v) => v.first().copied(),
            other => other
                .as_i64()
                .map(|x| x as f64)
                .or_else(|| other.as_u64().map(|x| x as f64)),
        }
    }

    /// True for every numeric (integer or float) item.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Item::I1(_)
                | Item::I2(_)
                | Item::I4(_)
                | Item::I8(_)
                | Item::U1(_)
                | Item::U2(_)
                | Item::U4(_)
                | Item::U8(_)
                | Item::F4(_)
                | Item::F8(_)
        )
    }
}

fn length_bytes(len: usize) -> usize {
    match len {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        _ => 3,
    }
}

fn take(bytes: &[u8], at: usize, n: usize) -> Result<&[u8], CodecError> {
    match bytes.get(at..at + n) {
        Some(s) => Ok(s),
        None => Err(CodecError::Truncated {
            needed: (at + n).saturating_sub(bytes.len()),
            have: bytes.len().saturating_sub(at),
        }),
    }
}

fn decode_item(bytes: &[u8], depth: usize) -> Result<(Item, usize), CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError::TooDeep(MAX_DEPTH));
    }
    let head = take(bytes, 0, 1)?[0];
    let n = usize::from(head & 0x03);
    if n == 0 {
        return Err(CodecError::NoLengthBytes(head));
    }
    let len = take(bytes, 1, n)?
        .iter()
        .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
    let code = head >> 2;
    let start = 1 + n;

    if code == format::LIST {
        // An element needs at least 2 bytes, so a claimed count beyond that is a lie; this also
        // keeps the allocation proportional to the input.
        let mut items = Vec::with_capacity(len.min(bytes.len().saturating_sub(start) / 2));
        let mut at = start;
        for _ in 0..len {
            let (item, used) = decode_item(
                bytes
                    .get(at..)
                    .ok_or(CodecError::Truncated { needed: 2, have: 0 })?,
                depth + 1,
            )?;
            items.push(item);
            at += used;
        }
        return Ok((Item::List(items), at));
    }

    let data = take(bytes, start, len)?;
    let end = start + len;

    fn array<const N: usize, T>(
        kind: &'static str,
        data: &[u8],
        f: impl Fn([u8; N]) -> T,
    ) -> Result<Vec<T>, CodecError> {
        let (chunks, rest) = data.as_chunks::<N>();
        if !rest.is_empty() {
            return Err(CodecError::BadLength {
                kind,
                len: data.len(),
                size: N,
            });
        }
        Ok(chunks.iter().map(|c| f(*c)).collect())
    }

    let item = match code {
        format::BINARY => Item::Binary(data.to_vec()),
        format::BOOLEAN => Item::Boolean(data.iter().map(|b| *b != 0).collect()),
        format::ASCII => Item::Ascii(data.iter().map(|b| char::from(*b)).collect()),
        format::JIS8 => Item::Jis8(data.to_vec()),
        format::I1 => Item::I1(data.iter().map(|b| *b as i8).collect()),
        format::I2 => Item::I2(array("I2", data, i16::from_be_bytes)?),
        format::I4 => Item::I4(array("I4", data, i32::from_be_bytes)?),
        format::I8 => Item::I8(array("I8", data, i64::from_be_bytes)?),
        format::U1 => Item::U1(data.to_vec()),
        format::U2 => Item::U2(array("U2", data, u16::from_be_bytes)?),
        format::U4 => Item::U4(array("U4", data, u32::from_be_bytes)?),
        format::U8 => Item::U8(array("U8", data, u64::from_be_bytes)?),
        format::F4 => Item::F4(array("F4", data, f32::from_be_bytes)?),
        format::F8 => Item::F8(array("F8", data, f64::from_be_bytes)?),
        _ => return Err(CodecError::UnknownFormat(head)),
    };
    Ok((item, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(item: Item) -> Vec<u8> {
        let bytes = item.encode();
        assert_eq!(bytes.len(), item.encoded_len());
        assert_eq!(Item::decode(&bytes).unwrap(), item, "{bytes:02x?}");
        bytes
    }

    #[test]
    fn every_type_round_trips() {
        round(Item::empty_list());
        round(Item::list([Item::u1(1), Item::ascii("x")]));
        round(Item::Binary(vec![0, 1, 0xff]));
        round(Item::Boolean(vec![true, false, true]));
        round(Item::ascii("MDLN"));
        round(Item::ascii(""));
        round(Item::Jis8(vec![0x41, 0xb1, 0x5c]));
        round(Item::I1(vec![-128, 0, 127]));
        round(Item::I2(vec![-32768, 0, 32767]));
        round(Item::I4(vec![i32::MIN, -1, i32::MAX]));
        round(Item::I8(vec![i64::MIN, -1, i64::MAX]));
        round(Item::U1(vec![0, 255]));
        round(Item::U2(vec![0, 65535]));
        round(Item::U4(vec![0, u32::MAX]));
        round(Item::U8(vec![0, u64::MAX]));
        round(Item::F4(vec![0.0, -1.5, 3.25e10]));
        round(Item::F8(vec![0.0, -1.5, 3.25e300]));
    }

    #[test]
    fn known_byte_layouts() {
        assert_eq!(Item::ascii("AB").encode(), [0x41, 0x02, b'A', b'B']);
        assert_eq!(Item::u4(1).encode(), [0xb1, 0x04, 0, 0, 0, 1]);
        assert_eq!(Item::u2(0x0102).encode(), [0xa9, 0x02, 1, 2]);
        assert_eq!(Item::byte(0).encode(), [0x21, 0x01, 0x00]);
        assert_eq!(Item::boolean(true).encode(), [0x25, 0x01, 0x01]);
        assert_eq!(Item::i2(-2).encode(), [0x69, 0x02, 0xff, 0xfe]);
        assert_eq!(Item::f4(1.0).encode(), [0x91, 0x04, 0x3f, 0x80, 0, 0]);
        assert_eq!(
            Item::f8(1.0).encode(),
            [0x81, 0x08, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(Item::empty_list().encode(), [0x01, 0x00]);
        assert_eq!(
            Item::list([Item::ascii("MDLN"), Item::ascii("1")]).encode(),
            [
                0x01, 0x02, 0x41, 0x04, b'M', b'D', b'L', b'N', 0x41, 0x01, b'1'
            ]
        );
        assert_eq!(Item::Jis8(vec![0xb1]).encode(), [0x45, 0x01, 0xb1]);
    }

    #[test]
    fn length_byte_variants() {
        // 300 bytes need two length bytes, 70000 need three.
        let two = round(Item::Binary(vec![7; 300]));
        assert_eq!(two[0], 0x22);
        assert_eq!(&two[1..3], &[0x01, 0x2c]);
        let three = round(Item::Binary(vec![7; 70_000]));
        assert_eq!(three[0], 0x23);
        assert_eq!(&three[1..4], &[0x01, 0x11, 0x70]);
        // a list of 300 entries too
        let list = round(Item::List(vec![Item::byte(1); 300]));
        assert_eq!(list[0], 0x02);
        // the decoder accepts a longer length field than needed
        assert_eq!(
            Item::decode(&[0x43, 0x00, 0x00, 0x02, b'h', b'i']).unwrap(),
            Item::ascii("hi")
        );
        assert_eq!(Item::decode(&[0x42, 0x00, 0x00]).unwrap(), Item::ascii(""));
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(matches!(
            Item::decode(&[]),
            Err(CodecError::Truncated { .. })
        ));
        assert!(matches!(
            Item::decode(&[0x41, 0x05, b'a']),
            Err(CodecError::Truncated { .. })
        ));
        assert!(matches!(
            Item::decode(&[0x40, 0x00]),
            Err(CodecError::NoLengthBytes(_))
        ));
        assert!(matches!(
            Item::decode(&[0xb1, 0x03, 0, 0, 0]),
            Err(CodecError::BadLength { kind: "U4", .. })
        ));
        assert!(matches!(
            Item::decode(&[0x0d, 0x00]),
            Err(CodecError::UnknownFormat(_))
        ));
        assert!(matches!(
            Item::decode(&[0x21, 0x01, 0, 0]),
            Err(CodecError::TrailingBytes(1))
        ));
        // a list that claims more entries than there are bytes
        assert!(matches!(
            Item::decode(&[0x01, 0x03, 0x21, 0x01, 0x00]),
            Err(CodecError::Truncated { .. })
        ));
        // absurd length claims do not allocate
        assert!(Item::decode(&[0x03, 0xff, 0xff, 0xff]).is_err());
        // too deep
        let mut deep = [0x01, 0x01].repeat(MAX_DEPTH + 2);
        deep.extend_from_slice(&[0x21, 0x01, 0x00]);
        assert!(matches!(Item::decode(&deep), Err(CodecError::TooDeep(_))));
    }

    #[test]
    fn prefix_decoding_reports_used_bytes() {
        let mut bytes = Item::u1(9).encode();
        bytes.extend_from_slice(&[0xde, 0xad]);
        let (item, used) = Item::decode_prefix(&bytes).unwrap();
        assert_eq!(item, Item::u1(9));
        assert_eq!(used, 3);
    }

    #[test]
    fn accessors_convert_between_integer_types() {
        assert_eq!(Item::u2(300).as_u64(), Some(300));
        assert_eq!(Item::i4(-5).as_i64(), Some(-5));
        assert_eq!(Item::i4(-5).as_u64(), None);
        assert_eq!(Item::byte(7).as_u64(), Some(7));
        assert_eq!(Item::f4(1.5).as_f64(), Some(1.5));
        assert_eq!(Item::u1(3).as_f64(), Some(3.0));
        assert_eq!(Item::ascii("x").as_str(), Some("x"));
        assert_eq!(Item::boolean(true).as_bool(), Some(true));
        assert_eq!(Item::ascii("x").as_u64(), None);
        assert!(Item::f8(1.0).is_numeric());
        assert!(!Item::byte(1).is_numeric());
    }

    #[test]
    fn oversized_items_are_refused_by_try_encode() {
        let big = Item::Binary(vec![0; MAX_ITEM_LEN + 1]);
        assert_eq!(big.try_encode(), Err(CodecError::TooLong(MAX_ITEM_LEN + 1)));
    }

    #[test]
    fn ascii_keeps_high_bytes() {
        let bytes = [0x41, 0x03, b'a', 0xe9, 0x80];
        let item = Item::decode(&bytes).unwrap();
        assert_eq!(item.encode(), bytes);
    }
}
