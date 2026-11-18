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
    /// CAN FD error state indicator: the sender was error passive. Set by the controller on received
    /// frames; this crate never asks a controller to transmit it.
    #[serde(default)]
    pub esi: bool,
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
            esi: false,
            data: data.to_vec(),
            rtr_len: 0,
        };
        f.validate()?;
        Ok(f)
    }

    /// A classic data frame with an explicit identifier length.
    pub fn with_ext(id: u32, extended: bool, data: &[u8]) -> Result<Self, CanError> {
        let f = CanFrame { id, extended, rtr: false, fd: false, brs: false, esi: false, data: data.to_vec(), rtr_len: 0 };
        f.validate()?;
        Ok(f)
    }

    pub fn remote(id: u32, extended: bool, len: u8) -> Result<Self, CanError> {
        let f = CanFrame { id, extended, rtr: true, fd: false, brs: false, esi: false, data: Vec::new(), rtr_len: len };
        f.validate()?;
        Ok(f)
    }

    pub fn fd(id: u32, extended: bool, brs: bool, data: &[u8]) -> Result<Self, CanError> {
        let f = CanFrame { id, extended, rtr: false, fd: true, brs, esi: false, data: data.to_vec(), rtr_len: 0 };
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
            if self.esi {
                return Err(CanError::Frame("the error state indicator needs a CAN FD frame".into()));
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

/// Error state of a CAN controller (ISO 11898-1 fault confinement).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BusState {
    ErrorActive,
    ErrorWarning,
    ErrorPassive,
    BusOff,
}

impl BusState {
    /// The state the error counters put a controller in (bus off is only ever reported, not derived).
    pub fn from_counters(tx_errors: u8, rx_errors: u8) -> Self {
        match tx_errors.max(rx_errors) {
            0..=95 => BusState::ErrorActive,
            96..=127 => BusState::ErrorWarning,
            _ => BusState::ErrorPassive,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            BusState::ErrorActive => "error active",
            BusState::ErrorWarning => "error warning",
            BusState::ErrorPassive => "error passive",
            BusState::BusOff => "bus off",
        }
    }
}

/// The controller's state and, when the adapter reports them, its error counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusStatus {
    pub state: BusState,
    pub tx_errors: Option<u8>,
    pub rx_errors: Option<u8>,
}

impl Default for BusStatus {
    fn default() -> Self {
        BusStatus { state: BusState::ErrorActive, tx_errors: None, rx_errors: None }
    }
}

impl BusStatus {
    /// Folds an error frame in. Returns true when the state or the counters changed.
    pub fn update(&mut self, e: &ErrorFrame) -> bool {
        let before = *self;
        if let Some((tx, rx)) = e.counters {
            self.tx_errors = Some(tx);
            self.rx_errors = Some(rx);
        }
        match (e.state, e.counters) {
            (Some(s), _) => self.state = s,
            // A bus error with counters and no state report: the counters tell the state, except
            // that only an explicit report ends bus off.
            (None, Some((tx, rx))) if self.state != BusState::BusOff => self.state = BusState::from_counters(tx, rx),
            _ => {}
        }
        *self != before
    }
}

// Linux CAN error frame classes (include/uapi/linux/can/error.h), also what gs_usb adapters send.
pub const CAN_ERR_TX_TIMEOUT: u32 = 0x001;
pub const CAN_ERR_LOSTARB: u32 = 0x002;
pub const CAN_ERR_CRTL: u32 = 0x004;
pub const CAN_ERR_PROT: u32 = 0x008;
pub const CAN_ERR_TRX: u32 = 0x010;
pub const CAN_ERR_ACK: u32 = 0x020;
pub const CAN_ERR_BUSOFF: u32 = 0x040;
pub const CAN_ERR_BUSERROR: u32 = 0x080;
pub const CAN_ERR_RESTARTED: u32 = 0x100;
pub const CAN_ERR_CNT: u32 = 0x200;

const CRTL_TX_WARNING: u8 = 0x08;
const CRTL_TX_PASSIVE: u8 = 0x20;
const PROT_LOC_ACK: u8 = 0x19;

/// A decoded error frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorFrame {
    /// What happened, in words (the details `candump -e` prints).
    pub text: String,
    /// The controller state this frame reports, if it reports one.
    pub state: Option<BusState>,
    /// Transmit and receive error counters, when the frame carries them.
    pub counters: Option<(u8, u8)>,
    /// A frame we sent was not acknowledged by any other node.
    pub no_ack: bool,
}

pub const NO_ACK_HINT: &str =
    "no other node acknowledged: check the wiring, the termination (120 ohm at each end) and that every node uses the same bit rate";

/// Decodes the class bits (`can_id` without the error flag) and the data of a Linux CAN error frame.
pub fn decode_error_frame(class: u32, d: &[u8; 8]) -> ErrorFrame {
    let mut parts: Vec<String> = Vec::new();
    let mut state = None;
    if class & CAN_ERR_TX_TIMEOUT != 0 {
        parts.push("transmit timeout".into());
    }
    if class & CAN_ERR_LOSTARB != 0 {
        parts.push(if d[0] != 0 { format!("arbitration lost at bit {}", d[0]) } else { "arbitration lost".into() });
    }
    if class & CAN_ERR_CRTL != 0 {
        const CRTL: [(u8, &str); 7] = [
            (0x01, "receive buffer overflow"),
            (0x02, "transmit buffer overflow"),
            (0x04, "RX error warning"),
            (0x08, "TX error warning"),
            (0x10, "RX error passive"),
            (0x20, "TX error passive"),
            (0x40, "back to error active"),
        ];
        let names: Vec<&str> = CRTL.iter().filter(|(b, _)| d[1] & b != 0).map(|(_, n)| *n).collect();
        parts.push(if names.is_empty() { "controller problem".into() } else { format!("controller: {}", names.join(", ")) });
        state = if d[1] & 0x30 != 0 {
            Some(BusState::ErrorPassive)
        } else if d[1] & 0x0C != 0 {
            Some(BusState::ErrorWarning)
        } else if d[1] & 0x40 != 0 {
            Some(BusState::ErrorActive)
        } else {
            None
        };
    }
    if class & CAN_ERR_PROT != 0 {
        const KIND: [(u8, &str); 8] = [
            (0x01, "single bit error"),
            (0x02, "frame format error"),
            (0x04, "bit stuffing error"),
            (0x08, "unable to send a dominant bit"),
            (0x10, "unable to send a recessive bit"),
            (0x20, "bus overload"),
            (0x40, "active error announcement"),
            (0x80, "while transmitting"),
        ];
        let kinds: Vec<&str> = KIND.iter().filter(|(b, _)| d[2] & b != 0).map(|(_, n)| *n).collect();
        let mut s = if kinds.is_empty() { "protocol violation".to_string() } else { format!("protocol violation: {}", kinds.join(", ")) };
        if let Some(loc) = prot_location(d[3]) {
            s.push_str(&format!(" at {loc}"));
        }
        parts.push(s);
    }
    if class & CAN_ERR_TRX != 0 {
        let t = match d[4] {
            0x04 => "CAN H wire missing",
            0x05 => "CAN H shorted to battery",
            0x06 => "CAN H shorted to VCC",
            0x07 => "CAN H shorted to ground",
            0x40 => "CAN L wire missing",
            0x50 => "CAN L shorted to battery",
            0x60 => "CAN L shorted to VCC",
            0x70 => "CAN L shorted to ground",
            0x80 => "CAN L shorted to CAN H",
            _ => "transceiver problem",
        };
        parts.push(format!("transceiver: {t}"));
    }
    let no_ack = class & CAN_ERR_ACK != 0 || (class & CAN_ERR_PROT != 0 && d[3] == PROT_LOC_ACK);
    if class & CAN_ERR_ACK != 0 {
        parts.push("no acknowledgement".into());
    }
    if class & CAN_ERR_BUSOFF != 0 {
        parts.push("bus off".into());
        state = Some(BusState::BusOff);
    }
    if class & CAN_ERR_BUSERROR != 0 && class & CAN_ERR_PROT == 0 {
        parts.push("bus error".into());
    }
    if class & CAN_ERR_RESTARTED != 0 {
        parts.push("controller restarted".into());
        state = Some(BusState::ErrorActive);
    }
    let counters = (class & CAN_ERR_CNT != 0).then_some((d[6], d[7]));
    if let Some((tx, rx)) = counters {
        parts.push(format!("TX errors {tx}, RX errors {rx}"));
    }
    let mut text = if parts.is_empty() { "error frame".to_string() } else { format!("error frame: {}", parts.join("; ")) };
    if no_ack {
        text.push_str(&format!(" ({NO_ACK_HINT})"));
    } else if class & CAN_ERR_CRTL != 0 && d[1] & (CRTL_TX_WARNING | CRTL_TX_PASSIVE) != 0 {
        text.push_str(&format!(" (transmit errors are piling up; if nothing else is on the bus, {NO_ACK_HINT})"));
    }
    ErrorFrame { text, state, counters, no_ack }
}

fn prot_location(loc: u8) -> Option<&'static str> {
    Some(match loc {
        0x03 => "start of frame",
        0x02 => "ID bits 28-21",
        0x06 => "ID bits 20-18",
        0x04 => "substitute RTR",
        0x05 => "identifier extension",
        0x07 => "ID bits 17-13",
        0x0F => "ID bits 12-5",
        0x0E => "ID bits 4-0",
        0x0C => "RTR bit",
        0x0D => "reserved bit 1",
        0x09 => "reserved bit 0",
        0x0B => "data length code",
        0x0A => "data field",
        0x08 => "CRC sequence",
        0x18 => "CRC delimiter",
        0x19 => "ACK slot",
        0x1B => "ACK delimiter",
        0x1A => "end of frame",
        0x12 => "intermission",
        _ => return None,
    })
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
        let lens = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
        for (dlc, len) in lens.into_iter().enumerate() {
            assert_eq!(dlc_to_len(dlc as u8), len);
            assert_eq!(len_to_dlc(len), Some(dlc as u8));
            assert_eq!(CanFrame::fd(0x200, false, true, &vec![0; len]).unwrap().dlc(), dlc as u8);
        }
        for len in (9..64).filter(|l| !lens.contains(l)) {
            assert_eq!(len_to_dlc(len), None, "{len}");
            assert!(CanFrame::fd(0x200, false, false, &vec![0; len]).is_err(), "{len}");
        }
        assert_eq!(len_to_dlc(65), None);
        // Only the low nibble is a DLC.
        assert_eq!(dlc_to_len(0x1F), 64);
    }

    #[test]
    fn fd_flags() {
        let mut f = CanFrame::fd(0x123, false, false, &[0; 64]).unwrap();
        f.esi = true;
        assert!(f.validate().is_ok());
        let mut c = CanFrame::new(0x123, &[1]).unwrap();
        c.esi = true;
        assert!(c.validate().is_err());
        assert!(CanFrame { brs: true, ..CanFrame::new(0x123, &[1]).unwrap() }.validate().is_err());
        assert!(CanFrame { rtr: true, ..CanFrame::fd(0x123, false, false, &[]).unwrap() }.validate().is_err());
        // Frames serialised before `esi` existed still read back.
        let old: CanFrame = serde_json::from_str(r#"{"id":1,"extended":false,"fd":true,"brs":true,"data":[1,2]}"#).unwrap();
        assert!(old.fd && old.brs && !old.esi);
    }

    #[test]
    fn wire_bits_fd() {
        let slow = CanFrame::fd(0x200, false, false, &[0; 64]).unwrap();
        let fast = CanFrame::fd(0x200, false, true, &[0; 64]).unwrap();
        assert!(fast.wire_bits(4.0) < slow.wire_bits(4.0) / 2.0);
        assert_eq!(slow.wire_bits(4.0), slow.wire_bits(1.0));
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
