//! DNP3 application layer (IEEE 1815-2012 clause 4): fragment headers, internal indications and
//! the request builders the master uses.
//!
//! A request fragment starts with two octets, application control then function code. A response
//! fragment has four: application control, function code, IIN1 and IIN2.

use anyhow::Result;

/// DNP3 application function codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FunctionCode {
    Confirm = 0x00,
    Read = 0x01,
    Write = 0x02,
    Select = 0x03,
    Operate = 0x04,
    DirectOperate = 0x05,
    DirectOperateNoAck = 0x06,
    Freeze = 0x07,
    FreezeNoAck = 0x08,
    FreezeClear = 0x09,
    FreezeClearNoAck = 0x0A,
    FreezeAtTime = 0x0B,
    FreezeAtTimeNoAck = 0x0C,
    ColdRestart = 0x0D,
    WarmRestart = 0x0E,
    InitializeData = 0x0F,
    EnableUnsolicited = 0x14,
    DisableUnsolicited = 0x15,
    DelayMeasure = 0x17,
    Response = 0x81,
    UnsolicitedResponse = 0x82,
}

impl FunctionCode {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0x00 => Self::Confirm,
            0x01 => Self::Read,
            0x02 => Self::Write,
            0x03 => Self::Select,
            0x04 => Self::Operate,
            0x05 => Self::DirectOperate,
            0x06 => Self::DirectOperateNoAck,
            0x07 => Self::Freeze,
            0x08 => Self::FreezeNoAck,
            0x09 => Self::FreezeClear,
            0x0A => Self::FreezeClearNoAck,
            0x0B => Self::FreezeAtTime,
            0x0C => Self::FreezeAtTimeNoAck,
            0x0D => Self::ColdRestart,
            0x0E => Self::WarmRestart,
            0x0F => Self::InitializeData,
            0x14 => Self::EnableUnsolicited,
            0x15 => Self::DisableUnsolicited,
            0x17 => Self::DelayMeasure,
            0x81 => Self::Response,
            0x82 => Self::UnsolicitedResponse,
            _ => anyhow::bail!("Unknown function code: 0x{:02X}", v),
        })
    }
}

/// Application control octet: FIR | FIN | CON | UNS | SEQ (4 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppControl {
    /// First fragment of a message.
    pub fir: bool,
    /// Final fragment of a message.
    pub fin: bool,
    /// The receiver must answer this fragment with a CONFIRM.
    pub con: bool,
    /// Unsolicited response.
    pub uns: bool,
    pub seq: u8,
}

impl AppControl {
    /// A complete single-fragment message with sequence number `seq`.
    pub const fn single(seq: u8) -> Self {
        Self { fir: true, fin: true, con: false, uns: false, seq: seq & 0x0F }
    }

    pub fn to_byte(self) -> u8 {
        (if self.fir { 0x80 } else { 0 })
            | (if self.fin { 0x40 } else { 0 })
            | (if self.con { 0x20 } else { 0 })
            | (if self.uns { 0x10 } else { 0 })
            | (self.seq & 0x0F)
    }

    pub fn from_byte(byte: u8) -> Self {
        Self {
            fir: byte & 0x80 != 0,
            fin: byte & 0x40 != 0,
            con: byte & 0x20 != 0,
            uns: byte & 0x10 != 0,
            seq: byte & 0x0F,
        }
    }
}

/// Internal indications (IIN1 and IIN2), carried in every response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Iin {
    // IIN1
    pub all_stations: bool,
    pub class_1_events: bool,
    pub class_2_events: bool,
    pub class_3_events: bool,
    pub need_time: bool,
    pub local_control: bool,
    pub device_trouble: bool,
    pub device_restart: bool,
    // IIN2
    pub no_func_code_support: bool,
    pub object_unknown: bool,
    pub parameter_error: bool,
    pub event_buffer_overflow: bool,
    pub already_executing: bool,
    pub config_corrupt: bool,
}

impl Iin {
    pub fn to_bytes(&self) -> [u8; 2] {
        let bit = |set: bool, mask: u8| if set { mask } else { 0 };
        [
            bit(self.all_stations, 0x01)
                | bit(self.class_1_events, 0x02)
                | bit(self.class_2_events, 0x04)
                | bit(self.class_3_events, 0x08)
                | bit(self.need_time, 0x10)
                | bit(self.local_control, 0x20)
                | bit(self.device_trouble, 0x40)
                | bit(self.device_restart, 0x80),
            bit(self.no_func_code_support, 0x01)
                | bit(self.object_unknown, 0x02)
                | bit(self.parameter_error, 0x04)
                | bit(self.event_buffer_overflow, 0x08)
                | bit(self.already_executing, 0x10)
                | bit(self.config_corrupt, 0x20),
        ]
    }

    pub fn from_bytes(low: u8, high: u8) -> Self {
        Self {
            all_stations: low & 0x01 != 0,
            class_1_events: low & 0x02 != 0,
            class_2_events: low & 0x04 != 0,
            class_3_events: low & 0x08 != 0,
            need_time: low & 0x10 != 0,
            local_control: low & 0x20 != 0,
            device_trouble: low & 0x40 != 0,
            device_restart: low & 0x80 != 0,
            no_func_code_support: high & 0x01 != 0,
            object_unknown: high & 0x02 != 0,
            parameter_error: high & 0x04 != 0,
            event_buffer_overflow: high & 0x08 != 0,
            already_executing: high & 0x10 != 0,
            config_corrupt: high & 0x20 != 0,
        }
    }

    /// Combine the indications of several fragments of one response.
    pub fn merge(&mut self, other: &Iin) {
        let [a1, a2] = self.to_bytes();
        let [b1, b2] = other.to_bytes();
        *self = Self::from_bytes(a1 | b1, a2 | b2);
    }

    /// The IIN2 conditions that mean the outstation refused or could not parse the request.
    /// `object_unknown` is only counted when `strict_objects` is set, because an outstation that
    /// lacks one of several requested classes still answers with the data it has.
    pub fn error(&self, strict_objects: bool) -> Option<String> {
        let mut problems = Vec::new();
        if self.no_func_code_support {
            problems.push("function code not supported (IIN2.0)");
        }
        if self.object_unknown && strict_objects {
            problems.push("object unknown (IIN2.1)");
        }
        if self.parameter_error {
            problems.push("parameter error (IIN2.2)");
        }
        if self.already_executing {
            problems.push("already executing (IIN2.4)");
        }
        if self.config_corrupt {
            problems.push("configuration corrupt (IIN2.5)");
        }
        if problems.is_empty() { None } else { Some(problems.join(", ")) }
    }
}

/// Parsed application header of a request or a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsduHeader {
    pub control: AppControl,
    pub function: FunctionCode,
    /// Present in responses; default (all clear) for requests.
    pub iin: Iin,
}

impl AsduHeader {
    pub fn sequence(&self) -> u8 {
        self.control.seq
    }

    /// Decode a response fragment: returns the header and the object bytes that follow it.
    pub fn decode_response(asdu: &[u8]) -> Result<(Self, &[u8])> {
        if asdu.len() < 4 {
            anyhow::bail!("Response fragment too short: {} bytes", asdu.len());
        }
        let function = FunctionCode::from_u8(asdu[1])?;
        Ok((
            Self {
                control: AppControl::from_byte(asdu[0]),
                function,
                iin: Iin::from_bytes(asdu[2], asdu[3]),
            },
            &asdu[4..],
        ))
    }
}

/// Split a request fragment into its application control, raw function code and object bytes.
/// The function code stays raw so an outstation can still answer one it does not know.
pub fn split_request(asdu: &[u8]) -> Option<(AppControl, u8, &[u8])> {
    if asdu.len() < 2 {
        return None;
    }
    Some((AppControl::from_byte(asdu[0]), asdu[1], &asdu[2..]))
}

/// Qualifier 0x06: no range field, all objects.
pub const QUALIFIER_ALL: u8 = 0x06;

/// Build a single-fragment request: 2-octet header followed by `objects`.
pub fn build_request(function: FunctionCode, sequence: u8, objects: &[u8]) -> Vec<u8> {
    let mut asdu = Vec::with_capacity(2 + objects.len());
    asdu.push(AppControl::single(sequence).to_byte());
    asdu.push(function as u8);
    asdu.extend_from_slice(objects);
    asdu
}

/// Build a single-fragment, solicited response: 4-octet header followed by `objects`.
pub fn build_response(sequence: u8, iin: &Iin, objects: &[u8]) -> Vec<u8> {
    build_response_fragment(AppControl::single(sequence), iin, objects)
}

/// Build one response fragment with explicit application control.
pub fn build_response_fragment(control: AppControl, iin: &Iin, objects: &[u8]) -> Vec<u8> {
    let mut asdu = Vec::with_capacity(4 + objects.len());
    asdu.push(control.to_byte());
    asdu.push(FunctionCode::Response as u8);
    asdu.extend_from_slice(&iin.to_bytes());
    asdu.extend_from_slice(objects);
    asdu
}

/// Build a READ for the given (group, variation) pairs, each with qualifier 0x06 (all objects).
pub fn build_read_request(sequence: u8, objects: &[(u8, u8)]) -> Vec<u8> {
    let mut body = Vec::with_capacity(3 * objects.len());
    for &(group, variation) in objects {
        body.extend_from_slice(&[group, variation, QUALIFIER_ALL]);
    }
    build_request(FunctionCode::Read, sequence, &body)
}

/// Integrity poll: class 1, 2 and 3 events, then class 0 static data
/// (g60v2, g60v3, g60v4, g60v1, all with qualifier 0x06).
pub fn build_integrity_poll_request(sequence: u8) -> Vec<u8> {
    build_read_request(sequence, &[(60, 2), (60, 3), (60, 4), (60, 1)])
}

/// Event scan for one class (g60v2 / g60v3 / g60v4); any other value polls class 0.
pub fn build_event_scan_request(sequence: u8, class: u8) -> Vec<u8> {
    let variation = match class {
        1 => 2,
        2 => 3,
        3 => 4,
        _ => 1,
    };
    build_read_request(sequence, &[(60, variation)])
}

/// FREEZE of all counters: g20v0 with qualifier 0x06.
pub fn build_freeze_request(sequence: u8) -> Vec<u8> {
    build_request(FunctionCode::Freeze, sequence, &[20, 0, QUALIFIER_ALL])
}

/// DNP3 timestamp: 48-bit unsigned milliseconds since the Unix epoch, little-endian.
pub fn dnp3_time_bytes(unix_ms: u64) -> [u8; 6] {
    let b = unix_ms.to_le_bytes();
    [b[0], b[1], b[2], b[3], b[4], b[5]]
}

/// WRITE of the current time: g50v1 (time and date), qualifier 0x07, one object of 6 octets.
pub fn build_time_sync_request(sequence: u8) -> Vec<u8> {
    build_time_sync_request_at(sequence, now_ms())
}

/// WRITE of `unix_ms` as g50v1.
pub fn build_time_sync_request_at(sequence: u8, unix_ms: u64) -> Vec<u8> {
    let mut body = vec![50, 1, 0x07, 0x01];
    body.extend_from_slice(&dnp3_time_bytes(unix_ms));
    build_request(FunctionCode::Write, sequence, &body)
}

/// COLD RESTART (0x0D). No objects.
pub fn build_cold_restart_request(sequence: u8) -> Vec<u8> {
    build_request(FunctionCode::ColdRestart, sequence, &[])
}

/// WARM RESTART (0x0E). No objects.
pub fn build_warm_restart_request(sequence: u8) -> Vec<u8> {
    build_request(FunctionCode::WarmRestart, sequence, &[])
}

/// WRITE of 0 to IIN1.7 DEVICE_RESTART: g80v1, qualifier 0x00, index 7..7, one packed bit.
pub fn build_clear_restart_request(sequence: u8) -> Vec<u8> {
    build_request(FunctionCode::Write, sequence, &[80, 1, 0x00, 7, 7, 0x00])
}

/// Application-layer CONFIRM for a received fragment with CON set. Echoes that fragment's
/// sequence number; `uns` is set when confirming an unsolicited response.
pub fn build_confirm(sequence: u8, uns: bool) -> Vec<u8> {
    let control = AppControl { uns, ..AppControl::single(sequence) };
    vec![control.to_byte(), FunctionCode::Confirm as u8]
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_control_bits() {
        let c = AppControl::from_byte(0xE3);
        assert!(c.fir && c.fin && c.con && !c.uns);
        assert_eq!(c.seq, 3);
        assert_eq!(c.to_byte(), 0xE3);
        assert_eq!(AppControl::single(5).to_byte(), 0xC5);
        assert_eq!(AppControl { uns: true, ..AppControl::single(0) }.to_byte(), 0xD0);
        // Sequence is four bits.
        assert_eq!(AppControl::single(17).seq, 1);
    }

    #[test]
    fn iin_bit_positions_follow_the_standard() {
        // IIN1: bit 0 all stations, 1-3 class 1-3 events, 4 need time, 5 local control,
        // 6 device trouble, 7 device restart. IIN2: bit 0 function code not supported,
        // 1 object unknown, 2 parameter error, 3 event buffer overflow, 4 already executing,
        // 5 configuration corrupt.
        let iin = Iin::from_bytes(0x96, 0x00);
        assert!(iin.class_1_events && iin.class_2_events && !iin.class_3_events);
        assert!(iin.need_time && iin.device_restart);
        assert!(!iin.device_trouble && !iin.local_control && !iin.all_stations);
        assert_eq!(iin.to_bytes(), [0x96, 0x00]);

        let errors = Iin::from_bytes(0x00, 0x07);
        assert!(errors.no_func_code_support && errors.object_unknown && errors.parameter_error);
        assert_eq!(errors.to_bytes(), [0x00, 0x07]);
        for byte in 0..=255u8 {
            assert_eq!(Iin::from_bytes(byte, byte & 0x3F).to_bytes(), [byte, byte & 0x3F]);
        }
    }

    #[test]
    fn iin_errors_ignore_unknown_object_unless_strict() {
        let iin = Iin::from_bytes(0, 0x02);
        assert_eq!(iin.error(false), None);
        assert!(iin.error(true).unwrap().contains("IIN2.1"));
        assert!(Iin::from_bytes(0, 0x01).error(false).unwrap().contains("IIN2.0"));
        assert!(Iin::from_bytes(0, 0x04).error(false).unwrap().contains("IIN2.2"));
    }

    #[test]
    fn iin_merge_combines_fragments() {
        let mut a = Iin::from_bytes(0x02, 0x00);
        a.merge(&Iin::from_bytes(0x10, 0x08));
        assert!(a.class_1_events && a.need_time && a.event_buffer_overflow);
    }

    #[test]
    fn response_header_is_four_octets() {
        let (header, rest) = AsduHeader::decode_response(&[0xE3, 0x81, 0x96, 0x00, 0xAA]).unwrap();
        assert_eq!(header.function, FunctionCode::Response);
        assert_eq!(header.sequence(), 3);
        assert!(header.control.con);
        assert!(header.iin.device_restart);
        assert_eq!(rest, [0xAA]);
        assert!(AsduHeader::decode_response(&[0xC0, 0x81, 0x00]).is_err());
        assert!(AsduHeader::decode_response(&[0xC0, 0x42, 0x00, 0x00]).is_err());
    }

    #[test]
    fn request_header_is_two_octets() {
        let req = build_request(FunctionCode::Read, 7, &[60, 1, 6]);
        assert_eq!(req, [0xC7, 0x01, 60, 1, 6]);
        let (control, function, objects) = split_request(&req).unwrap();
        assert_eq!(control.seq, 7);
        assert_eq!(function, 0x01);
        assert_eq!(objects, [60, 1, 6]);
        assert!(split_request(&[0xC0]).is_none());
    }

    #[test]
    fn function_code_values() {
        assert_eq!(FunctionCode::Confirm as u8, 0x00);
        assert_eq!(FunctionCode::Read as u8, 0x01);
        assert_eq!(FunctionCode::Write as u8, 0x02);
        assert_eq!(FunctionCode::Select as u8, 0x03);
        assert_eq!(FunctionCode::Operate as u8, 0x04);
        assert_eq!(FunctionCode::DirectOperate as u8, 0x05);
        assert_eq!(FunctionCode::Freeze as u8, 0x07);
        assert_eq!(FunctionCode::FreezeClear as u8, 0x09);
        assert_eq!(FunctionCode::ColdRestart as u8, 0x0D);
        assert_eq!(FunctionCode::WarmRestart as u8, 0x0E);
        assert_eq!(FunctionCode::EnableUnsolicited as u8, 0x14);
        assert_eq!(FunctionCode::DisableUnsolicited as u8, 0x15);
        assert_eq!(FunctionCode::Response as u8, 0x81);
        assert_eq!(FunctionCode::UnsolicitedResponse as u8, 0x82);
        for code in 0..=255u8 {
            if let Ok(f) = FunctionCode::from_u8(code) {
                assert_eq!(f as u8, code);
            }
        }
    }

    #[test]
    fn dnp3_time_is_48_bits() {
        assert_eq!(dnp3_time_bytes(0x0000_AABB_CCDD_EEFF), [0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]);
        let req = build_time_sync_request_at(1, 0x0102_0304_0506);
        assert_eq!(req.len(), 2 + 4 + 6);
        assert_eq!(req[..6], [0xC1, 0x02, 50, 1, 0x07, 0x01]);
    }

    #[test]
    fn class_polls_use_qualifier_06() {
        assert_eq!(build_event_scan_request(0, 1), [0xC0, 0x01, 60, 2, 0x06]);
        assert_eq!(build_event_scan_request(0, 2), [0xC0, 0x01, 60, 3, 0x06]);
        assert_eq!(build_event_scan_request(0, 3), [0xC0, 0x01, 60, 4, 0x06]);
        assert_eq!(build_freeze_request(2), [0xC2, 0x07, 20, 0, 0x06]);
        assert_eq!(build_cold_restart_request(3), [0xC3, 0x0D]);
        assert_eq!(build_warm_restart_request(4), [0xC4, 0x0E]);
    }

    #[test]
    fn clear_restart_writes_iin_bit_7_as_zero() {
        assert_eq!(build_clear_restart_request(5), [0xC5, 0x02, 80, 1, 0x00, 7, 7, 0x00]);
    }

    #[test]
    fn confirm_echoes_sequence_and_unsolicited_bit() {
        assert_eq!(build_confirm(3, false), [0xC3, 0x00]);
        assert_eq!(build_confirm(3, true), [0xD3, 0x00]);
    }
}
