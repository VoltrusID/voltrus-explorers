//! CAN and CAN FD frames.

use serde::{Deserialize, Serialize};

use crate::error::CanError;

/// Largest 11-bit identifier.
pub const MAX_STD_ID: u32 = 0x7FF;
/// Largest 29-bit identifier.
pub const MAX_EXT_ID: u32 = 0x1FFF_FFFF;

/// The data lengths a CAN FD frame can carry; a classic frame carries 0 to 8 bytes.
pub const FD_LENGTHS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// One frame on the bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanFrame {
    pub id: u32,
    /// 29-bit identifier.
    pub extended: bool,
    /// Remote transmission request (classic CAN only); `data` is empty and `rtr_len` holds the DLC.
    #[serde(default)]
    pub rtr: bool,
    /// CAN FD frame.
    #[serde(default)]
    pub fd: bool,
    /// CAN FD bit rate switch.
    #[serde(default)]
    pub brs: bool,
    #[serde(default)]
    pub data: Vec<u8>,
    /// Requested length of a remote frame.
    #[serde(default)]
    pub rtr_len: u8,
}

impl CanFrame {
    /// A classic data frame. The identifier is 29-bit when `id > 0x7FF`.
    pub fn new(id: u32, data: &[u8]) -> Result<Self, CanError> {
        let f = CanFrame {
            id,
            extended: id > MAX_STD_ID,
            rtr: false,
            fd: false,
            brs: false,
            data: data.to_vec(),
            rtr_len: 0,
        };
        f.validate()?;
        Ok(f)
    }

    /// A classic data frame with an explicit identifier length.
    pub fn with_ext(id: u32, extended: bool, data: &[u8]) -> Result<Self, CanError> {
        let f = CanFrame { id, extended, rtr: false, fd: false, brs: false, data: data.to_vec(), rtr_len: 0 };
        f.validate()?;
        Ok(f)
    }

    pub fn remote(id: u32, extended: bool, len: u8) -> Result<Self, CanError> {
        let f = CanFrame { id, extended, rtr: true, fd: false, brs: false, data: Vec::new(), rtr_len: len };
        f.validate()?;
        Ok(f)
    }

    pub fn fd(id: u32, extended: bool, brs: bool, data: &[u8]) -> Result<Self, CanError> {
        let f = CanFrame { id, extended, rtr: false, fd: true, brs, data: data.to_vec(), rtr_len: 0 };
        f.validate()?;
        Ok(f)
    }

    pub fn validate(&self) -> Result<(), CanError> {
        let max = if self.extended { MAX_EXT_ID } else { MAX_STD_ID };
        if self.id > max {
            return Err(CanError::Frame(format!(
                "identifier 0x{:X} does not fit in {} bits",
                self.id,
                if self.extended { 29 } else { 11 }
            )));
        }
        if self.fd {
            if self.rtr {
                return Err(CanError::Frame("CAN FD has no remote frames".into()));
            }
            if !FD_LENGTHS.contains(&self.data.len()) {
                return Err(CanError::Frame(format!(
                    "a CAN FD frame carries 0-8, 12, 16, 20, 24, 32, 48 or 64 bytes, not {}",
                    self.data.len()
                )));
            }
        } else {
            if self.brs {
                return Err(CanError::Frame("bit rate switch needs a CAN FD frame".into()));
            }
            if self.data.len() > 8 {
                return Err(CanError::Frame(format!("a classic CAN frame carries at most 8 bytes, not {}", self.data.len())));
            }
            if self.rtr && (!self.data.is_empty() || self.rtr_len > 8) {
                return Err(CanError::Frame("a remote frame has no data and a length of 0 to 8".into()));
            }
        }
        Ok(())
    }

    /// The data length code on the wire.
    pub fn dlc(&self) -> u8 {
        if self.rtr {
            return self.rtr_len;
        }
        len_to_dlc(self.data.len()).unwrap_or(15)
    }

    /// `123` or `12345678`, as candump writes it.
    pub fn id_text(&self) -> String {
        if self.extended { format!("{:08X}", self.id) } else { format!("{:03X}", self.id) }
    }

    /// Lookup key that keeps 11-bit and 29-bit identifiers apart (bit 31 set for 29-bit, as in DBC files).
    pub fn key(&self) -> u32 {
        frame_key(self.id, self.extended)
    }

    /// Bits on the wire for a classic frame, with the worst-case bit stuffing estimate; FD frames are
    /// counted at the arbitration rate for the header and the data rate for the rest when `brs` is set
    /// (callers pass the data rate ratio).
    pub fn wire_bits(&self, data_rate_ratio: f64) -> f64 {
        let n = if self.rtr { 0 } else { self.data.len() } as f64;
        if !self.fd {
            // SOF + id + control + data + CRC + delimiters + ACK + EOF + IFS, plus stuff bits on the
            // stuffed part (everything up to the CRC delimiter).
            let (header, stuffed) = if self.extended { (67.0, 54.0) } else { (47.0, 34.0) };
            let stuffed_bits = stuffed + 8.0 * n;
            return header + 8.0 * n + ((stuffed_bits - 1.0) / 4.0).floor();
        }
        let arb = if self.extended { 32.0 } else { 13.0 };
        let crc = if n > 16.0 { 21.0 } else { 17.0 };
        let data_phase = 8.0 + 8.0 * n + crc + 4.0 + (8.0 * n + 20.0) / 5.0;
        let tail = 12.0;
        let ratio = if self.brs && data_rate_ratio > 0.0 { data_rate_ratio } else { 1.0 };
        arb + tail + data_phase / ratio
    }
}

/// Lookup key for an identifier: bit 31 set for 29-bit identifiers, as DBC files write them.
pub fn frame_key(id: u32, extended: bool) -> u32 {
    if extended { id | 0x8000_0000 } else { id }
}

/// Data length to DLC (0-15). `None` for lengths a frame cannot carry.
pub fn len_to_dlc(len: usize) -> Option<u8> {
    FD_LENGTHS.iter().position(|&l| l == len).map(|p| p as u8)
}

/// DLC (0-15) to data length.
pub fn dlc_to_len(dlc: u8) -> usize {
    FD_LENGTHS[(dlc & 0x0F) as usize]
}

/// Parses hex bytes written as `DE AD BE EF`, `DEADBEEF` or `de:ad:be:ef`.
pub fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, CanError> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace() && *c != ':' && *c != ',' && *c != '.').collect();
    let clean = clean.strip_prefix("0x").or_else(|| clean.strip_prefix("0X")).unwrap_or(&clean);
    if !clean.len().is_multiple_of(2) {
        return Err(CanError::Frame(format!("odd number of hex digits in \"{}\"", text.trim())));
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&clean[i..i + 2], 16)
                .map_err(|_| CanError::Frame(format!("not hex: \"{}\"", &clean[i..i + 2])))
        })
        .collect()
}

pub fn hex_bytes(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{b:02X}"));
    }
    s
}

/// Where a frame came from, from this node's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Rx,
    Tx,
}

/// A frame with the host time it was seen (microseconds since the Unix epoch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedFrame {
    pub ts_us: u64,
    pub dir: Direction,
    #[serde(flatten)]
    pub frame: CanFrame,
}

pub fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ids_and_lengths() {
        assert!(!CanFrame::new(0x7FF, &[1, 2]).unwrap().extended);
        assert!(CanFrame::new(0x800, &[]).unwrap().extended);
        assert!(CanFrame::with_ext(0x800, false, &[]).is_err());
        assert!(CanFrame::new(0x2000_0000, &[]).is_err());
        assert!(CanFrame::new(1, &[0; 9]).is_err());
        assert!(CanFrame::fd(1, false, true, &[0; 12]).is_ok());
        assert!(CanFrame::fd(1, false, true, &[0; 13]).is_err());
        assert!(CanFrame::remote(1, false, 9).is_err());
    }

    #[test]
    fn dlc_mapping() {
        assert_eq!(len_to_dlc(8), Some(8));
        assert_eq!(len_to_dlc(12), Some(9));
        assert_eq!(len_to_dlc(64), Some(15));
        assert_eq!(len_to_dlc(9), None);
        assert_eq!(dlc_to_len(13), 32);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex_bytes("de ad:BE,ef").unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(parse_hex_bytes("0x0102").unwrap(), vec![1, 2]);
        assert_eq!(parse_hex_bytes("").unwrap(), Vec::<u8>::new());
        assert!(parse_hex_bytes("123").is_err());
        assert!(parse_hex_bytes("zz").is_err());
        assert_eq!(hex_bytes(&[1, 0xAB]), "01 AB");
    }

    #[test]
    fn wire_bits_classic() {
        // 8-byte standard frame: 111 bits without stuffing, at most 24 stuff bits.
        let f = CanFrame::new(0x123, &[0; 8]).unwrap();
        let b = f.wire_bits(1.0);
        assert!((111.0..=135.0).contains(&b), "{b}");
        let e = CanFrame::new(0x1234_5678, &[0; 8]).unwrap();
        assert!(e.wire_bits(1.0) > b);
    }
}
