//! The SLCAN (Lawicel) ASCII protocol spoken by USB-CAN adapters that show up as a serial port:
//! CANable and CANable 2 (slcan firmware), Lawicel CANUSB and CAN232, USBtin and their clones.
//!
//! Frames are `t<iii><l><dd..>` (11-bit), `T<iiiiiiii><l><dd..>` (29-bit), `r`/`R` for remote frames
//! and, on adapters with CAN FD, `d`/`D` (FD) and `b`/`B` (FD with bit rate switch), each ended by a
//! carriage return.
//!
//! Two families of firmware answer differently, so the driver asks `V` first and keeps the answer:
//! - Lawicel style (CANUSB, CAN232, USBtin and clones) answers `V<hhhh>`, acknowledges every command
//!   with `\r` (BEL 0x07 refuses it), confirms queued frames with `z`/`Z`, opens listen-only with `L`
//!   and reports status flags on `F`. `S7` is 800 kbit/s.
//! - CANable and CANable 2 slcan firmware answers `<git hash> <repository url>` and acknowledges
//!   nothing: a refused command is silently ignored. It has no `L` and no `F` (its `M1` silent mode is undone when `O` opens the channel),
//!   `S7` is 750 kbit/s, CANable 2 adds `S9` (83.3 kbit/s) and only `Y2` / `Y5` as CAN FD data rates.
//!   A bare `\r` re-runs its last command, so the driver never sends one.

use crate::error::CanError;
use crate::frame::{BusState, CanFrame, dlc_to_len, len_to_dlc};

/// Which SLCAN dialect an adapter speaks, from its answer to `V`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Firmware {
    /// CANable (STM32F042, classic CAN) slcan firmware.
    Canable,
    /// CANable 2 (STM32G431, CAN FD) slcan firmware.
    Canable2,
    /// Lawicel CANUSB / CAN232, USBtin and other adapters that acknowledge commands.
    Lawicel,
}

impl Firmware {
    pub fn name(self) -> &'static str {
        match self {
            Firmware::Canable => "CANable slcan firmware (CANable 2 not confirmed)",
            Firmware::Canable2 => "CANable 2 slcan firmware",
            Firmware::Lawicel => "Lawicel-compatible SLCAN adapter",
        }
    }

    /// Whether the firmware answers commands with `\r` / BEL.
    pub fn acknowledges(self) -> bool {
        self == Firmware::Lawicel
    }

    /// Nominal bit rates and their `S` digit.
    pub fn bitrates(self) -> &'static [(u32, char)] {
        match self {
            Firmware::Canable => &CANABLE_BITRATES[..9],
            Firmware::Canable2 => &CANABLE_BITRATES,
            Firmware::Lawicel => &LAWICEL_BITRATES,
        }
    }

    /// CAN FD data bit rates and their `Y` digit. Lawicel-style adapters that do CAN FD acknowledge
    /// `Y`, so the driver can try these and report a refusal. A CANable whose version answer does
    /// not name the CANable 2 firmware may still be one (a fork, a local build): `Y` is allowed and
    /// harmless on a CANable 1 (ignored, as are FD frames), and the driver warns.
    pub fn data_bitrates(self) -> &'static [(u32, char)] {
        &DATA_BITRATES
    }
}

/// `S0`..`S8` on Lawicel, USBtin and the Linux slcan driver.
pub const LAWICEL_BITRATES: [(u32, char); 9] = [
    (10_000, '0'),
    (20_000, '1'),
    (50_000, '2'),
    (100_000, '3'),
    (125_000, '4'),
    (250_000, '5'),
    (500_000, '6'),
    (800_000, '7'),
    (1_000_000, '8'),
];

/// CANable firmware (and python-can): `S7` is 750 kbit/s; CANable 2 adds `S9`, 83.3 kbit/s.
pub const CANABLE_BITRATES: [(u32, char); 10] = [
    (10_000, '0'),
    (20_000, '1'),
    (50_000, '2'),
    (100_000, '3'),
    (125_000, '4'),
    (250_000, '5'),
    (500_000, '6'),
    (750_000, '7'),
    (1_000_000, '8'),
    (83_333, '9'),
];

/// The only CAN FD data rates CANable 2 firmware has (`Y2`, `Y5`).
pub const DATA_BITRATES: [(u32, char); 2] = [(2_000_000, '2'), (5_000_000, '5')];

fn rate_list(t: &[(u32, char)]) -> String {
    let mut v: Vec<u32> = t.iter().map(|(b, _)| *b).collect();
    v.sort_unstable();
    v.iter().map(|b| crate::bus::rate_text(*b)).collect::<Vec<_>>().join(", ")
}

/// `S<n>\r` for a nominal bit rate on this firmware.
pub fn bitrate_command(fw: Firmware, bitrate: u32) -> Result<String, CanError> {
    fw.bitrates().iter().find(|(b, _)| *b == bitrate).map(|(_, c)| format!("S{c}\r")).ok_or_else(|| {
        CanError::Unsupported(format!(
            "the adapter ({}) has no {} setting; it offers {}",
            fw.name(),
            crate::bus::rate_text(bitrate),
            rate_list(fw.bitrates())
        ))
    })
}

/// `Y<n>\r` for a CAN FD data bit rate on this firmware.
pub fn data_bitrate_command(fw: Firmware, bitrate: u32) -> Result<String, CanError> {
    fw.data_bitrates().iter().find(|(b, _)| *b == bitrate).map(|(_, c)| format!("Y{c}\r")).ok_or_else(|| {
        CanError::Unsupported(format!(
            "SLCAN has no {} CAN FD data rate; it offers {}",
            crate::bus::rate_text(bitrate),
            rate_list(fw.data_bitrates())
        ))
    })
}

/// Recognises the answer to `V`. `None` when the line is not a version answer.
pub fn parse_version(line: &str) -> Option<Firmware> {
    let line = line.trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ' || c == '\0');
    let lower = line.to_ascii_lowercase();
    // Lawicel / USBtin: `V1013`, `v0107`.
    if line.len() == 5 && line.starts_with(['V', 'v']) && line[1..].chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(Firmware::Lawicel);
    }
    // CANable: `<git hash> <remote url>`, e.g. `a1b2c3d https://github.com/normaldotcom/canable2-fw.git`.
    let mut words = line.split_whitespace();
    let hash = words.next().unwrap_or("");
    let looks_like_hash = (7..=40).contains(&hash.len()) && hash.chars().all(|c| c.is_ascii_hexdigit());
    // `git describe` output can also be `v1.0-3-ga1b2c3d-dirty`; the second word is the remote.
    let remote = words.next().unwrap_or("").to_ascii_lowercase();
    let looks_like_remote = remote.contains("://") || remote.ends_with(".git") || remote.starts_with("git@");
    if (!hash.is_empty() && (looks_like_hash && !remote.is_empty() || looks_like_remote)) || lower.contains("canable") {
        let fd = lower.contains("canable2") || lower.contains("canable-2") || lower.contains("canable_2");
        return Some(if fd { Firmware::Canable2 } else { Firmware::Canable });
    }
    None
}

/// The SLCAN line for a frame, with its `\r`.
pub fn encode(frame: &CanFrame) -> Result<String, CanError> {
    frame.validate()?;
    let kind = match (frame.fd, frame.brs, frame.rtr, frame.extended) {
        (false, _, false, false) => 't',
        (false, _, false, true) => 'T',
        (false, _, true, false) => 'r',
        (false, _, true, true) => 'R',
        (true, false, _, false) => 'd',
        (true, false, _, true) => 'D',
        (true, true, _, false) => 'b',
        (true, true, _, true) => 'B',
    };
    let mut s = String::with_capacity(12 + frame.data.len() * 2);
    s.push(kind);
    if frame.extended {
        s.push_str(&format!("{:08X}", frame.id));
    } else {
        s.push_str(&format!("{:03X}", frame.id));
    }
    let dlc = frame.dlc();
    s.push(char::from_digit(dlc as u32, 16).unwrap_or('0').to_ascii_uppercase());
    for b in &frame.data {
        s.push_str(&format!("{b:02X}"));
    }
    s.push('\r');
    Ok(s)
}

/// What one line from the adapter means.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Frame(CanFrame),
    /// `\r` on its own: the last command was accepted.
    Ok,
    /// BEL: the last command was refused.
    Refused,
    /// `z` or `Z` after a transmit (Lawicel firmware).
    TxAck,
    /// `F<xx>`: status flags.
    Status(u8),
    /// `V<hhhh>` / `v<hhhh>` / `N<xxxx>`: version or serial number.
    Info(String),
    /// Anything else, kept for the log.
    Other(String),
}

/// Parses one line (without its `\r`).
pub fn decode_line(line: &str) -> Result<Reply, CanError> {
    let line = line.trim_matches(|c| c == '\n' || c == '\r');
    let Some(first) = line.chars().next() else {
        return Ok(Reply::Ok);
    };
    let bad = |m: &str| CanError::Adapter(format!("{m}: \"{line}\""));
    match first {
        't' | 'T' | 'r' | 'R' | 'd' | 'D' | 'b' | 'B' => {
            let extended = first.is_ascii_uppercase();
            let rtr = matches!(first, 'r' | 'R');
            let fd = matches!(first, 'd' | 'D' | 'b' | 'B');
            let brs = matches!(first, 'b' | 'B');
            let id_len = if extended { 8 } else { 3 };
            if line.len() < 1 + id_len + 1 || !line.is_ascii() {
                return Err(bad("short frame"));
            }
            let id = u32::from_str_radix(&line[1..1 + id_len], 16).map_err(|_| bad("bad identifier"))?;
            let dlc = u8::from_str_radix(&line[1 + id_len..2 + id_len], 16).map_err(|_| bad("bad length"))?;
            if !fd && dlc > 8 {
                return Err(bad("length over 8 in a classic frame"));
            }
            let rest = &line[2 + id_len..];
            if rtr {
                // Some firmware appends a timestamp (4 hex digits) to every frame.
                let f = CanFrame { id, extended, rtr: true, fd: false, brs: false, esi: false, data: Vec::new(), rtr_len: dlc };
                f.validate()?;
                return Ok(Reply::Frame(f));
            }
            let len = dlc_to_len(dlc);
            if rest.len() < len * 2 {
                return Err(bad("data shorter than its length"));
            }
            let mut data = Vec::with_capacity(len);
            for i in 0..len {
                data.push(u8::from_str_radix(&rest[i * 2..i * 2 + 2], 16).map_err(|_| bad("bad data byte"))?);
            }
            let f = CanFrame { id, extended, rtr: false, fd, brs, esi: false, data, rtr_len: 0 };
            f.validate()?;
            Ok(Reply::Frame(f))
        }
        'z' | 'Z' if line.len() == 1 => Ok(Reply::TxAck),
        'F' if line.len() == 3 => u8::from_str_radix(&line[1..], 16).map(Reply::Status).map_err(|_| bad("bad status")),
        'V' | 'v' | 'N' => Ok(Reply::Info(line.to_string())),
        _ => Ok(Reply::Other(line.to_string())),
    }
}

/// Splits the adapter's byte stream into replies. Bytes may arrive in any chunking.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes and returns every complete reply. Lines that cannot be parsed come back as `Err`.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Reply, CanError>> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                b'\r' => {
                    let line = String::from_utf8_lossy(&self.buf).into_owned();
                    self.buf.clear();
                    out.push(decode_line(&line));
                }
                0x07 => {
                    self.buf.clear();
                    out.push(Ok(Reply::Refused));
                }
                b'\n' => {}
                _ => {
                    // A line never gets longer than a 64-byte FD frame; drop garbage that would.
                    if self.buf.len() < 200 {
                        self.buf.push(b);
                    } else {
                        self.buf.clear();
                    }
                }
            }
        }
        out
    }
}

/// Meaning of the `F` status flag bits.
pub fn status_text(flags: u8) -> String {
    const NAMES: [&str; 8] = [
        "receive FIFO full",
        "transmit FIFO full",
        "error warning",
        "data overrun",
        "",
        "error passive",
        "arbitration lost",
        "bus error",
    ];
    let parts: Vec<&str> = (0..8).filter(|b| flags & (1 << b) != 0 && !NAMES[*b].is_empty()).map(|b| NAMES[b]).collect();
    if parts.is_empty() { "no errors".into() } else { parts.join(", ") }
}

/// The controller state the `F` flags report (they have no bus-off bit).
pub fn status_state(flags: u8) -> BusState {
    if flags & 0x20 != 0 {
        BusState::ErrorPassive
    } else if flags & 0x04 != 0 {
        BusState::ErrorWarning
    } else {
        BusState::ErrorActive
    }
}

/// Picks the DLC for a length, for callers that build frames from free text.
pub fn dlc_for(len: usize) -> Option<u8> {
    len_to_dlc(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_frames() {
        assert_eq!(encode(&CanFrame::new(0x123, &[0xDE, 0xAD]).unwrap()).unwrap(), "t1232DEAD\r");
        assert_eq!(encode(&CanFrame::new(0x1234_5678, &[]).unwrap()).unwrap(), "T123456780\r");
        assert_eq!(encode(&CanFrame::remote(0x7DF, false, 8).unwrap()).unwrap(), "r7DF8\r");
        let fd = CanFrame::fd(0x10, false, true, &[0xAA; 12]).unwrap();
        assert_eq!(encode(&fd).unwrap(), format!("b0109{}\r", "AA".repeat(12)));
    }

    #[test]
    fn decodes_frames_and_replies() {
        let mut d = Decoder::new();
        let r = d.push(b"t12");
        assert!(r.is_empty());
        let r = d.push(b"32DEAD\r\x07T1ABCDEF0111\rz\rF04\rV1013\r\r");
        let r: Vec<Reply> = r.into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(r[0], Reply::Frame(CanFrame::new(0x123, &[0xDE, 0xAD]).unwrap()));
        assert_eq!(r[1], Reply::Refused);
        assert_eq!(r[2], Reply::Frame(CanFrame::with_ext(0x1ABC_DEF0, true, &[0x11]).unwrap()));
        assert_eq!(r[3], Reply::TxAck);
        assert_eq!(r[4], Reply::Status(4));
        assert_eq!(r[5], Reply::Info("V1013".into()));
        assert_eq!(r[6], Reply::Ok);
    }

    #[test]
    fn tolerates_timestamps_and_rejects_garbage() {
        // Timestamp (Z1) appends 4 hex digits after the data.
        let r = decode_line("t1230EA5F").unwrap();
        assert_eq!(r, Reply::Frame(CanFrame::new(0x123, &[]).unwrap()));
        assert!(decode_line("t12").is_err());
        assert!(decode_line("t1239").is_err());
        assert!(decode_line("t1232DE").is_err());
        assert!(decode_line("tXYZ0").is_err());
    }

    #[test]
    fn round_trip() {
        for f in [
            CanFrame::new(0, &[]).unwrap(),
            CanFrame::new(0x7FF, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap(),
            CanFrame::remote(0x1FFF_FFFF, true, 3).unwrap(),
            CanFrame::fd(0x55, true, false, &(0..64).collect::<Vec<u8>>()).unwrap(),
        ] {
            let line = encode(&f).unwrap();
            assert_eq!(decode_line(&line).unwrap(), Reply::Frame(f));
        }
    }

    #[test]
    fn bitrates() {
        use Firmware::*;
        assert_eq!(bitrate_command(Lawicel, 500_000).unwrap(), "S6\r");
        // S7 means different rates on the two families.
        assert_eq!(bitrate_command(Lawicel, 800_000).unwrap(), "S7\r");
        assert!(bitrate_command(Lawicel, 750_000).is_err());
        assert_eq!(bitrate_command(Canable, 750_000).unwrap(), "S7\r");
        assert!(bitrate_command(Canable, 800_000).unwrap_err().to_string().contains("750 kbit/s"));
        assert!(bitrate_command(Canable, 83_333).is_err());
        assert_eq!(bitrate_command(Canable2, 83_333).unwrap(), "S9\r");
        assert!(bitrate_command(Lawicel, 83_333).is_err());
        assert_eq!(data_bitrate_command(Canable2, 2_000_000).unwrap(), "Y2\r");
        assert_eq!(data_bitrate_command(Canable2, 5_000_000).unwrap(), "Y5\r");
        for bad in [1_000_000, 4_000_000, 8_000_000] {
            assert!(data_bitrate_command(Canable2, bad).unwrap_err().to_string().contains("2 Mbit/s, 5 Mbit/s"));
        }
        assert_eq!(data_bitrate_command(Canable, 5_000_000).unwrap(), "Y5\r");
        assert!(data_bitrate_command(Canable, 4_000_000).is_err());
        assert_eq!(status_text(0x24), "error warning, error passive");
        assert_eq!(status_state(0x24), BusState::ErrorPassive);
        assert_eq!(status_state(0x04), BusState::ErrorWarning);
        assert_eq!(status_state(0x80), BusState::ErrorActive);
    }

    #[test]
    fn recognises_firmware_from_the_version_answer() {
        assert_eq!(parse_version("V1013"), Some(Firmware::Lawicel));
        assert_eq!(parse_version("v0107\r"), Some(Firmware::Lawicel));
        assert_eq!(parse_version("a1b2c3d https://github.com/normaldotcom/canable-fw.git"), Some(Firmware::Canable));
        assert_eq!(parse_version("bd4f1a2 https://github.com/normaldotcom/canable2-fw.git"), Some(Firmware::Canable2));
        assert_eq!(parse_version("v1.0-3-gdeadbee-dirty git@github.com:me/fork.git"), Some(Firmware::Canable));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("$GPGGA,123519,4807.038,N,01131.000,E"), None);
        assert_eq!(parse_version("t1230"), None);
    }
}
