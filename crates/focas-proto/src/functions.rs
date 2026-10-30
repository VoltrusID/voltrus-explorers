//! The calls the reference documents: function numbers, request builders and payload codecs.
//!
//! The client uses the `req_*` builders and `parse_*` functions; the simulator answers with the
//! matching `encode_*` functions, so both sides share one description of every payload.

use crate::codec::{Command, SCALED_INVALID, SCALED_LEN, SubRequest, decode_scaled, encode_scaled};
use crate::error::FocasError;
use crate::types::*;

/// Function numbers (pyfanuc `protocol.txt`, `README.md` and the sample files).
pub mod func {
    use crate::codec::Command;

    /// `cnc_rdparam`: read parameters.
    pub const PARAM: Command = Command::cnc(0x0e);
    /// Program directory listing.
    pub const PROGRAM_LIST: Command = Command::cnc(0x06);
    /// `cnc_rdmacro`: read macro variables.
    pub const MACRO_READ: Command = Command::cnc(0x15);
    /// `cnc_sysinfo`.
    pub const SYSINFO: Command = Command::cnc(0x18);
    /// `cnc_statinfo`.
    pub const STATINFO: Command = Command::cnc(0x19);
    /// `cnc_alarm`: alarm bit field.
    pub const ALARM: Command = Command::cnc(0x1a);
    /// Running / main program number.
    pub const PROGRAM_NUMBER: Command = Command::cnc(0x1c);
    /// Active sequence number.
    pub const SEQUENCE_NUMBER: Command = Command::cnc(0x1d);
    /// Currently executing block text.
    pub const EXEC_BLOCK: Command = Command::cnc(0x20);
    /// `cnc_rdalmmsg`: alarm number and message.
    pub const ALARM_MESSAGE: Command = Command::cnc(0x23);
    /// `cnc_actf`: actual feed rate.
    pub const ACTUAL_FEED: Command = Command::cnc(0x24);
    /// `cnc_acts`: actual spindle speed.
    pub const ACTUAL_SPINDLE: Command = Command::cnc(0x25);
    /// Axis positions (absolute / machine / relative / distance to go / skip).
    pub const POSITION: Command = Command::cnc(0x26);
    /// `cnc_diagnoss`: diagnosis values.
    pub const DIAG: Command = Command::cnc(0x30);
    /// Servo load per axis (from the multi-packet sample).
    pub const SERVO_LOAD: Command = Command::cnc(0x56);
    /// Axis names (from the multi-packet sample).
    pub const AXIS_NAMES: Command = Command::cnc(0x89);
    /// Macro variable write with a double (from the pyfocas package).
    pub const MACRO_WRITE_DOUBLE: Command = Command::cnc(0xa8);
    /// Main program name with path (series 30i and later).
    pub const PROGRAM_NAME: Command = Command::cnc(0xb9);
    /// PMC read.
    pub const PMC_READ: Command = Command::pmc(0x8001);
}

/// Parameter storage type codes (pyfanuc `readparam`).
pub mod param_type {
    pub const BYTE: u16 = 0;
    pub const BIT: u16 = 1;
    pub const SHORT: u16 = 2;
    pub const INT: u16 = 3;
}

/// Diagnosis storage type codes (pyfanuc `readdiag`).
pub mod diag_type {
    pub const BYTE: u16 = 0;
    pub const SHORT: u16 = 1;
    pub const INT: u16 = 2;
    pub const BIT: u16 = 3;
    pub const BYTE_ALT: u16 = 4;
}

/// Which numbered table a [`NumberedValues`] payload belongs to; the two use different type codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberedKind {
    Param,
    Diag,
}

/// Axis slots in a position payload (the samples show eight).
pub const POSITION_SLOTS: usize = 8;
/// Axis slots in a servo-load payload (the multi-packet sample shows 32).
pub const SERVO_SLOTS: usize = 32;
/// Programs per directory request (pyfanuc asks for `0x13`).
pub const PROGRAM_LIST_BATCH: i32 = 0x13;
/// Bytes per directory entry: number, size, 64-byte comment.
pub const PROGRAM_ENTRY_LEN: usize = 72;
/// Axis argument meaning "all axes".
pub const ALL_AXES: i32 = -1;

fn bad(call: &'static str, detail: impl Into<String>) -> FocasError {
    FocasError::UnexpectedPayload {
        call,
        detail: detail.into(),
    }
}

fn be_i32(b: &[u8]) -> i32 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be_i16(b: &[u8]) -> i16 {
    i16::from_be_bytes([b[0], b[1]])
}

fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn zero_args(command: Command) -> SubRequest {
    SubRequest::new(command, [0; 5])
}

// ---- requests -------------------------------------------------------------------------------

pub fn req_sysinfo() -> SubRequest {
    zero_args(func::SYSINFO)
}

pub fn req_statinfo() -> SubRequest {
    zero_args(func::STATINFO)
}

pub fn req_alarm_bits() -> SubRequest {
    zero_args(func::ALARM)
}

pub fn req_program_numbers() -> SubRequest {
    zero_args(func::PROGRAM_NUMBER)
}

pub fn req_sequence_number() -> SubRequest {
    zero_args(func::SEQUENCE_NUMBER)
}

pub fn req_actual_feed() -> SubRequest {
    zero_args(func::ACTUAL_FEED)
}

pub fn req_actual_spindle() -> SubRequest {
    zero_args(func::ACTUAL_SPINDLE)
}

pub fn req_axis_names() -> SubRequest {
    zero_args(func::AXIS_NAMES)
}

pub fn req_servo_load() -> SubRequest {
    SubRequest::new(func::SERVO_LOAD, [1, 0, 0, 0, 0])
}

pub fn req_program_name() -> SubRequest {
    zero_args(func::PROGRAM_NAME)
}

pub fn req_position(kind: PositionKind, axis: i32) -> SubRequest {
    SubRequest::new(func::POSITION, [kind.code(), axis, 0, 0, 0])
}

pub fn req_macro_read(first: u32, last: u32) -> SubRequest {
    SubRequest::new(func::MACRO_READ, [first as i32, last as i32, 0, 0, 0])
}

/// Macro write: arguments are the number and the byte length (8) of the double that follows.
pub fn req_macro_write(number: u32, value: f64) -> SubRequest {
    SubRequest::new(func::MACRO_WRITE_DOUBLE, [number as i32, 0, 0, 0, 8])
        .with_extra(value.to_be_bytes().to_vec())
}

pub fn req_numbered(kind: NumberedKind, first: u32, last: u32, axis: i32) -> SubRequest {
    let command = match kind {
        NumberedKind::Param => func::PARAM,
        NumberedKind::Diag => func::DIAG,
    };
    SubRequest::new(command, [first as i32, last as i32, axis, 0, 0])
}

pub fn req_pmc_read(area: PmcArea, data_type: PmcDataType, first: u32, count: u32) -> SubRequest {
    let last = first + (data_type.size() as u32) * count - 1;
    SubRequest::new(
        func::PMC_READ,
        [first as i32, last as i32, area.code(), data_type.code(), 0],
    )
}

pub fn req_alarm_messages(q: &AlarmQuery) -> SubRequest {
    SubRequest::new(
        func::ALARM_MESSAGE,
        [
            q.alarm_type,
            i32::from(q.max_messages),
            i32::from(q.with_text),
            i32::from(q.text_length),
            0,
        ],
    )
}

pub fn req_exec_block(chars: u16) -> SubRequest {
    SubRequest::new(func::EXEC_BLOCK, [i32::from(chars), 0, 0, 0, 0])
}

pub fn req_program_list(start: u32) -> SubRequest {
    SubRequest::new(
        func::PROGRAM_LIST,
        [start as i32, PROGRAM_LIST_BATCH, 2, 0, 0],
    )
}

// ---- payload parsers (client) ---------------------------------------------------------------

fn ascii(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string()
}

pub fn parse_sysinfo(p: &[u8]) -> Result<SysInfo, FocasError> {
    if p.len() < 18 {
        return Err(bad("sysinfo", format!("{} bytes, expected 18", p.len())));
    }
    Ok(SysInfo {
        addinfo: be_u16(&p[0..2]),
        max_axis: be_u16(&p[2..4]),
        cnc_type: ascii(&p[4..6]),
        mt_type: ascii(&p[6..8]),
        series: ascii(&p[8..12]),
        version: ascii(&p[12..16]),
        axes: ascii(&p[16..18]),
    })
}

fn pad(s: &str, width: usize, right_aligned: bool) -> Vec<u8> {
    let mut b: Vec<u8> = s.bytes().take(width).collect();
    let fill = vec![b' '; width - b.len()];
    if right_aligned {
        let mut out = fill;
        out.append(&mut b);
        out
    } else {
        b.extend(fill);
        b
    }
}

pub fn encode_sysinfo(s: &SysInfo) -> Vec<u8> {
    let mut out = Vec::with_capacity(18);
    out.extend_from_slice(&s.addinfo.to_be_bytes());
    out.extend_from_slice(&s.max_axis.to_be_bytes());
    out.extend(pad(&s.cnc_type, 2, true));
    out.extend(pad(&s.mt_type, 2, true));
    out.extend(pad(&s.series, 4, false));
    out.extend(pad(&s.version, 4, false));
    out.extend(pad(&s.axes, 2, true));
    out
}

/// The CNC sends seven big-endian words (`aut, run, motion, mstb, emergency, alarm, edit`); the
/// reference decodes statinfo only from payloads of exactly that size.
///
/// The `ODBST` of `fwlib32.h` has nine words: two leading ones (`hdck`/`tmmode`, "dummy" in
/// older headers) and then the same seven. A payload of 18 bytes is therefore read as that
/// header layout, skipping the two leading words. No capture shows a controller sending 18
/// bytes; the branch only exists so the wire call and the FOCAS library agree on the meaning of
/// the seven words if one ever does. Any other length is an error.
pub fn parse_statinfo(p: &[u8]) -> Result<StatInfo, FocasError> {
    let words = match p.len() {
        14 => p,
        18 => &p[4..],
        n => return Err(bad("statinfo", format!("{n} bytes, expected 14 (or 18)"))),
    };
    let w = |i: usize| be_u16(&words[i * 2..i * 2 + 2]);
    Ok(StatInfo {
        aut: AutoMode::from_code(w(0)),
        run: RunState::from_code(w(1)),
        motion: MotionState::from_code(w(2)),
        mstb: MstbState::from_code(w(3)),
        emergency: EmergencyState::from_code(w(4)),
        alarm: AlarmState::from_code(w(5)),
        edit: EditState::from_code(w(6)),
    })
}

pub fn encode_statinfo(s: &StatInfo) -> Vec<u8> {
    [
        s.aut.code(),
        s.run.code(),
        s.motion.code(),
        s.mstb.code(),
        s.emergency.code(),
        s.alarm.code(),
        s.edit.code(),
    ]
    .iter()
    .flat_map(|w| w.to_be_bytes())
    .collect()
}

pub fn parse_alarm_bits(p: &[u8]) -> Result<AlarmBits, FocasError> {
    if p.len() != 4 {
        return Err(bad("alarm", format!("{} bytes, expected 4", p.len())));
    }
    Ok(AlarmBits(be_u32(p)))
}

pub fn encode_alarm_bits(bits: AlarmBits) -> Vec<u8> {
    bits.0.to_be_bytes().to_vec()
}

pub fn parse_program_numbers(p: &[u8]) -> Result<ProgramNumbers, FocasError> {
    if p.len() < 8 {
        return Err(bad(
            "program number",
            format!("{} bytes, expected 8", p.len()),
        ));
    }
    Ok(ProgramNumbers {
        running: be_i32(&p[0..4]),
        main: be_i32(&p[4..8]),
    })
}

pub fn encode_program_numbers(n: ProgramNumbers) -> Vec<u8> {
    let mut out = n.running.to_be_bytes().to_vec();
    out.extend_from_slice(&n.main.to_be_bytes());
    out
}

pub fn parse_sequence_number(p: &[u8]) -> Result<i32, FocasError> {
    if p.len() < 4 {
        return Err(bad(
            "sequence number",
            format!("{} bytes, expected 4", p.len()),
        ));
    }
    Ok(be_i32(&p[0..4]))
}

/// One scaled number (actual feed, actual spindle speed).
pub fn parse_scaled_single(call: &'static str, p: &[u8]) -> Result<Option<f64>, FocasError> {
    if p.len() != SCALED_LEN {
        return Err(bad(call, format!("{} bytes, expected 8", p.len())));
    }
    Ok(decode_scaled(p))
}

/// A run of scaled numbers (positions, servo load); empty axis slots are included.
pub fn parse_scaled_list(call: &'static str, p: &[u8]) -> Result<Vec<Option<f64>>, FocasError> {
    if !p.len().is_multiple_of(SCALED_LEN) {
        return Err(bad(
            call,
            format!("{} bytes is not a multiple of 8", p.len()),
        ));
    }
    Ok(p.as_chunks::<SCALED_LEN>()
        .0
        .iter()
        .map(|c| decode_scaled(c))
        .collect())
}

/// `values` padded with zero slots to `slots`, each with `decimals` decimal places. A `None`
/// value becomes the invalid marker.
pub fn encode_scaled_list(values: &[Option<f64>], slots: usize, decimals: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(slots * SCALED_LEN);
    for i in 0..slots {
        match values.get(i) {
            Some(Some(v)) => out.extend_from_slice(&encode_scaled(*v, decimals)),
            Some(None) => out.extend_from_slice(&SCALED_INVALID),
            None => out.extend_from_slice(&encode_scaled(0.0, decimals)),
        }
    }
    out
}

/// Axis names: four bytes per controlled axis, the name character first. The suffix character is
/// taken from the first non-zero byte after it; the reference samples only show suffix-less names.
pub fn parse_axis_names(p: &[u8]) -> Result<Vec<AxisName>, FocasError> {
    if !p.len().is_multiple_of(4) {
        return Err(bad(
            "axis names",
            format!("{} bytes is not a multiple of 4", p.len()),
        ));
    }
    Ok(p.as_chunks::<4>()
        .0
        .iter()
        .map(|c| AxisName {
            name: char::from(c[0]),
            suffix: c[1..].iter().find(|b| **b != 0).map(|b| char::from(*b)),
        })
        .collect())
}

pub fn encode_axis_names(names: &[AxisName]) -> Vec<u8> {
    names
        .iter()
        .flat_map(|n| {
            let mut b = [0u8; 4];
            b[0] = n.name as u32 as u8;
            if let Some(s) = n.suffix {
                b[1] = s as u32 as u8;
            }
            b
        })
        .collect()
}

pub fn parse_macro_values(first: u32, p: &[u8]) -> Result<Vec<MacroValue>, FocasError> {
    if p.is_empty() || !p.len().is_multiple_of(SCALED_LEN) {
        return Err(bad(
            "macro read",
            format!("{} bytes is not a positive multiple of 8", p.len()),
        ));
    }
    Ok(p.as_chunks::<SCALED_LEN>()
        .0
        .iter()
        .enumerate()
        .map(|(i, c)| MacroValue {
            number: first + i as u32,
            value: decode_scaled(c),
        })
        .collect())
}

/// Macro values, each with as many decimal places as it needs (up to eight, as far as the
/// 32-bit mantissa allows); a vacant variable is the invalid marker.
pub fn encode_macro_values(values: &[Option<f64>]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| match v {
            Some(v) => crate::codec::encode_scaled_auto(*v),
            None => SCALED_INVALID,
        })
        .collect()
}

fn numbered_value(kind: NumberedKind, data_type: u16, slot: &[u8]) -> i32 {
    let short = |s: &[u8]| i32::from(i16::from_be_bytes([s[2], s[3]]));
    match (kind, data_type) {
        (NumberedKind::Param, param_type::BYTE | param_type::BIT) => i32::from(slot[3]),
        (NumberedKind::Param, param_type::SHORT) => short(slot),
        (NumberedKind::Param, param_type::INT) => be_i32(slot),
        (NumberedKind::Diag, diag_type::BYTE | diag_type::BYTE_ALT | diag_type::BIT) => {
            i32::from(slot[3])
        }
        (NumberedKind::Diag, diag_type::SHORT) => short(slot),
        (NumberedKind::Diag, diag_type::INT) => be_i32(slot),
        // A type code the reference does not describe: give the whole slot.
        _ => be_i32(slot),
    }
}

/// Parameter or diagnosis payload: per number `number u32, axis i16, type u16` followed by
/// `max_axis` four-byte slots. A request for one axis fills only the first slot.
pub fn parse_numbered(
    kind: NumberedKind,
    max_axis: usize,
    p: &[u8],
) -> Result<Vec<NumberedValues>, FocasError> {
    let call = match kind {
        NumberedKind::Param => "parameter read",
        NumberedKind::Diag => "diagnosis read",
    };
    let stride = max_axis * 4 + 8;
    if p.is_empty() || !p.len().is_multiple_of(stride) {
        return Err(bad(
            call,
            format!(
                "{} bytes is not a multiple of {stride} (max_axis {max_axis})",
                p.len()
            ),
        ));
    }
    Ok(p.chunks_exact(stride)
        .map(|c| {
            let number = be_u32(&c[0..4]);
            let axis = be_i16(&c[4..6]);
            let data_type = be_u16(&c[6..8]);
            let slots = c[8..].as_chunks::<4>().0.iter();
            let values = if axis == -1 {
                slots.map(|s| numbered_value(kind, data_type, s)).collect()
            } else {
                slots
                    .take(1)
                    .map(|s| numbered_value(kind, data_type, s))
                    .collect()
            };
            NumberedValues {
                number,
                axis,
                data_type,
                values,
            }
        })
        .collect())
}

pub fn encode_numbered(entries: &[NumberedValues], kind: NumberedKind, max_axis: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        out.extend_from_slice(&e.number.to_be_bytes());
        out.extend_from_slice(&e.axis.to_be_bytes());
        out.extend_from_slice(&e.data_type.to_be_bytes());
        let used = if e.axis == -1 { max_axis } else { 1 };
        for i in 0..max_axis {
            let v = if i < used {
                e.values.get(i).copied().unwrap_or(0)
            } else {
                0
            };
            let word = match (kind, e.data_type) {
                (NumberedKind::Param, param_type::BYTE | param_type::BIT)
                | (NumberedKind::Diag, diag_type::BYTE | diag_type::BYTE_ALT | diag_type::BIT) => {
                    i32::from(v as u8)
                }
                (_, _) => v,
            };
            out.extend_from_slice(&word.to_be_bytes());
        }
    }
    out
}

pub fn parse_pmc(
    data_type: PmcDataType,
    first: u32,
    p: &[u8],
) -> Result<Vec<PmcValue>, FocasError> {
    let size = data_type.size();
    if p.is_empty() || !p.len().is_multiple_of(size) {
        return Err(bad(
            "pmc read",
            format!("{} bytes is not a positive multiple of {size}", p.len()),
        ));
    }
    Ok(p.chunks_exact(size)
        .enumerate()
        .map(|(i, c)| PmcValue {
            address: first + (i * size) as u32,
            value: c.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)),
        })
        .collect())
}

pub fn encode_pmc(data_type: PmcDataType, values: &[u32]) -> Vec<u8> {
    let size = data_type.size();
    values
        .iter()
        .flat_map(|v| v.to_be_bytes()[4 - size..].to_vec())
        .collect()
}

fn c_string(raw: &[u8]) -> String {
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

/// Alarm messages: per entry `number i32, type i32, axis i32, text length i32` and `text_length`
/// bytes of text.
pub fn parse_alarm_messages(q: &AlarmQuery, p: &[u8]) -> Result<Vec<AlarmMessage>, FocasError> {
    let text_len = usize::from(q.text_length);
    let stride = 16 + text_len;
    if !p.len().is_multiple_of(stride) {
        return Err(bad(
            "alarm message",
            format!("{} bytes is not a multiple of {stride}", p.len()),
        ));
    }
    Ok(p.chunks_exact(stride)
        .map(|c| {
            let msg_len = be_i32(&c[12..16]);
            let body = &c[16..];
            let text = (q.with_text && msg_len > 0)
                .then(|| c_string(&body[..(msg_len as usize).min(text_len)]));
            AlarmMessage {
                number: be_i32(&c[0..4]),
                alarm_type: AlarmType::from_code(be_i32(&c[4..8])),
                axis: be_i32(&c[8..12]),
                text,
            }
        })
        .collect())
}

pub fn encode_alarm_messages(q: &AlarmQuery, alarms: &[AlarmMessage]) -> Vec<u8> {
    let text_len = usize::from(q.text_length);
    let mut out = Vec::new();
    for a in alarms {
        out.extend_from_slice(&a.number.to_be_bytes());
        out.extend_from_slice(&a.alarm_type.code().to_be_bytes());
        out.extend_from_slice(&a.axis.to_be_bytes());
        let body: Vec<u8> = match (&a.text, q.with_text) {
            (Some(t), true) => t.bytes().take(text_len).collect(),
            _ => Vec::new(),
        };
        out.extend_from_slice(&(body.len() as i32).to_be_bytes());
        out.extend_from_slice(&body);
        out.extend(std::iter::repeat_n(0u8, text_len - body.len()));
    }
    out
}

pub fn parse_exec_block(p: &[u8]) -> Result<ExecBlock, FocasError> {
    if p.len() < 4 {
        return Err(bad(
            "executing block",
            format!("{} bytes, expected at least 4", p.len()),
        ));
    }
    Ok(ExecBlock {
        block: be_i32(&p[0..4]),
        text: c_string(&p[4..]),
    })
}

pub fn encode_exec_block(b: &ExecBlock) -> Vec<u8> {
    let mut out = b.block.to_be_bytes().to_vec();
    out.extend_from_slice(b.text.as_bytes());
    out
}

pub fn parse_program_list(p: &[u8]) -> Result<Vec<ProgramEntry>, FocasError> {
    if !p.len().is_multiple_of(PROGRAM_ENTRY_LEN) {
        return Err(bad(
            "program list",
            format!("{} bytes is not a multiple of {PROGRAM_ENTRY_LEN}", p.len()),
        ));
    }
    Ok(p.as_chunks::<PROGRAM_ENTRY_LEN>()
        .0
        .iter()
        .map(|c| ProgramEntry {
            number: be_u32(&c[0..4]),
            size: be_u32(&c[4..8]),
            comment: c_string(&c[8..]),
        })
        .collect())
}

pub fn encode_program_list(entries: &[ProgramEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * PROGRAM_ENTRY_LEN);
    for e in entries {
        out.extend_from_slice(&e.number.to_be_bytes());
        out.extend_from_slice(&e.size.to_be_bytes());
        let mut comment = [0u8; 64];
        // keep the last byte 0 so the comment stays NUL terminated
        for (dst, src) in comment.iter_mut().take(63).zip(e.comment.bytes()) {
            *dst = src;
        }
        out.extend_from_slice(&comment);
    }
    out
}

/// The main program name: a NUL-terminated path.
pub fn parse_program_name(p: &[u8]) -> String {
    c_string(p)
}

pub fn encode_program_name(name: &str) -> Vec<u8> {
    let mut out = name.as_bytes().to_vec();
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tests::hex;

    #[test]
    fn sysinfo_round_trip_and_trim() {
        let s = SysInfo {
            addinfo: 0x0002,
            max_axis: 32,
            cnc_type: "31".into(),
            mt_type: "M".into(),
            series: "G401".into(),
            version: "42.0".into(),
            axes: "3".into(),
        };
        let bytes = encode_sysinfo(&s);
        assert_eq!(bytes.len(), 18);
        assert_eq!(&bytes[4..8], b"31 M");
        assert_eq!(&bytes[16..18], b" 3");
        assert_eq!(parse_sysinfo(&bytes).unwrap(), s);
        assert_eq!(s.axis_count(), Some(3));
        assert!(s.is_i_series());
        assert!(parse_sysinfo(&bytes[..17]).is_err());
    }

    #[test]
    fn statinfo_round_trip() {
        // aut=1 MEM, run=3 STRT, motion=1, mstb=0, emergency=0, alarm=0, edit=0
        let p = hex("00 01 00 03 00 01 00 00 00 00 00 00 00 00");
        let s = parse_statinfo(&p).unwrap();
        assert_eq!(s.aut, AutoMode::Memory);
        assert_eq!(s.run, RunState::Start);
        assert_eq!(s.motion, MotionState::Motion);
        assert_eq!(s.alarm, AlarmState::None);
        assert_eq!(encode_statinfo(&s), p);
        let odd = parse_statinfo(&hex("00 63 00 00 00 00 00 00 00 00 00 00 00 00")).unwrap();
        assert_eq!(odd.aut, AutoMode::Unknown(0x63));
        assert!(parse_statinfo(&p[..12]).is_err());
        // 9-word ODBST layout of fwlib32.h: hdck, tmmode, then the same seven words.
        let mut odbst = hex("00 00 00 01");
        odbst.extend_from_slice(&p);
        assert_eq!(parse_statinfo(&odbst).unwrap(), s);
        assert!(parse_statinfo(&odbst[..16]).is_err());
    }

    /// `sample_with_error.md`: running and main program O3000, sequence 0, feed 0.
    #[test]
    fn sample_program_feed_alarm() {
        let n = parse_program_numbers(&hex("00 00 0b b8 00 00 0b b8")).unwrap();
        assert_eq!((n.running, n.main), (3000, 3000));
        assert_eq!(parse_sequence_number(&hex("00 00 00 00")).unwrap(), 0);
        assert_eq!(
            parse_scaled_single("actf", &hex("00 00 00 00 00 0a 00 04")).unwrap(),
            Some(0.0)
        );
        assert_eq!(parse_alarm_bits(&hex("00 00 00 00")).unwrap(), AlarmBits(0));
        assert!(parse_alarm_bits(&hex("00 00 00")).is_err());
    }

    /// First position payload of `sample_with_error.md` (absolute positions, four decimals).
    #[test]
    fn sample_absolute_positions() {
        let p = hex("00 2f 7a e9 00 0a 00 04 00 02 bc 02 00 0a 00 04
             00 00 00 00 00 0a 00 04 00 00 00 00 00 0a 00 04
             ff ec ed 30 00 0a 00 04 00 36 ee 80 00 0a 00 04
             00 00 00 00 00 0a 00 04 00 00 00 00 00 0a 00 04");
        let v = parse_scaled_list("position", &p).unwrap();
        assert_eq!(v.len(), POSITION_SLOTS);
        assert!((v[0].unwrap() - 311.1657).abs() < 1e-9);
        assert!((v[1].unwrap() - 17.9202).abs() < 1e-9); // 0x0002bc02 = 179202
        assert_eq!(v[2], Some(0.0));
        assert!((v[4].unwrap() + 125.0).abs() < 1e-9);
        assert!((v[5].unwrap() - 360.0).abs() < 1e-9); // 0x0036ee80 = 3600000
    }

    /// `sample_multipacket.md`: names `X Y Z B` as 4-byte slots.
    #[test]
    fn sample_axis_names_and_servo_load() {
        let names =
            parse_axis_names(&hex("58 00 00 00 59 00 00 00 5a 00 00 00 42 00 00 00")).unwrap();
        assert_eq!(
            names.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            ["X", "Y", "Z", "B"]
        );
        assert_eq!(
            encode_axis_names(&names),
            hex("58 00 00 00 59 00 00 00 5a 00 00 00 42 00 00 00")
        );
        // servo load: 0, -3, 0, 33, ... all with zero decimals
        let load = parse_scaled_list(
            "servo load",
            &hex("00 00 00 00 00 0a 00 00 ff ff ff fd 00 0a 00 00
                 00 00 00 21 00 0a 00 00 00 00 00 00 00 0a 00 00"),
        )
        .unwrap();
        assert_eq!(load, vec![Some(0.0), Some(-3.0), Some(33.0), Some(0.0)]);
        // a suffix after the name
        let n = parse_axis_names(&hex("58 31 00 00")).unwrap();
        assert_eq!(n[0].to_string(), "X1");
    }

    /// pyfanuc README: PMC D2204 as a 16-bit value is 600.
    #[test]
    fn sample_pmc_word() {
        let v = parse_pmc(PmcDataType::Word, 2204, &hex("02 58")).unwrap();
        assert_eq!(
            v,
            vec![PmcValue {
                address: 2204,
                value: 600
            }]
        );
        let req = req_pmc_read(PmcArea::D, PmcDataType::Word, 2204, 1);
        assert_eq!(req.args, [2204, 2205, 9, 1, 0]);
        // a dword read steps four bytes
        let v = parse_pmc(PmcDataType::DWord, 1870, &hex("00 00 01 00 00 00 02 00")).unwrap();
        assert_eq!(
            v,
            vec![
                PmcValue {
                    address: 1870,
                    value: 256
                },
                PmcValue {
                    address: 1874,
                    value: 512
                }
            ]
        );
        assert_eq!(
            encode_pmc(PmcDataType::DWord, &[256, 512]),
            hex("00 00 01 00 00 00 02 00")
        );
        assert_eq!(encode_pmc(PmcDataType::Byte, &[0xab]), vec![0xab]);
    }

    /// pyfocas README: macro read response with two scaled numbers.
    #[test]
    fn sample_macro_values() {
        let v = parse_macro_values(502, &hex("29 7c 1e 00 00 0a 00 06 05 f5 e1 00 00 0a 00 08"))
            .unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].number, 502);
        assert!((v[0].value.unwrap() - 696.0).abs() < 1e-9); // 0x297c1e00 = 696000000, six decimals
        assert_eq!(v[1].number, 503);
        assert!((v[1].value.unwrap() - 1.0).abs() < 1e-12);
        let req = req_macro_write(502, 100.0);
        assert_eq!(req.args, [502, 0, 0, 0, 8]);
        assert_eq!(req.extra, hex("40 59 00 00 00 00 00 00"));
    }

    /// pyfanuc README "GETDIAG 980-981 for first Axis" on a control with MAX_AXIS 8.
    #[test]
    fn sample_diag_two_values() {
        // each value: name, axis, type, the value and seven empty slots (MAX_AXIS 8)
        let mut p = hex("00 00 03 d4 00 01 00 02 ff f6 de aa");
        p.extend([0u8; 28]);
        p.extend(hex("00 00 03 d5 00 01 00 02 ff e2 62 6c"));
        p.extend([0u8; 28]);
        let v = parse_numbered(NumberedKind::Diag, 8, &p).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].number, v[0].axis, v[0].data_type), (980, 1, 2));
        assert_eq!(v[0].values, vec![-598_358]);
        assert_eq!(v[1].values, vec![-1_940_884]);
        assert_eq!(encode_numbered(&v, NumberedKind::Diag, 8), p);
        // wrong max_axis is reported, not mis-parsed
        assert!(parse_numbered(NumberedKind::Diag, 7, &p).is_err());
    }

    #[test]
    fn param_types_decode() {
        let mk = |data_type, axis, slot: [u8; 4]| {
            let mut p = Vec::new();
            p.extend_from_slice(&1020u32.to_be_bytes());
            p.extend_from_slice(&(axis as i16).to_be_bytes());
            p.extend_from_slice(&(data_type as u16).to_be_bytes());
            p.extend_from_slice(&slot);
            p.extend_from_slice(&[0u8; 4]);
            p
        };
        let one = |data_type, axis, slot| {
            parse_numbered(NumberedKind::Param, 2, &mk(data_type, axis, slot)).unwrap()
        };
        assert_eq!(one(0, 1, [0, 0, 0, 88])[0].values, vec![88]);
        assert_eq!(
            one(1, 0, [0, 0, 0, 0b1010_0001])[0].values,
            vec![0b1010_0001]
        );
        assert_eq!(one(2, 1, [0, 0, 0xff, 0xfe])[0].values, vec![-2]);
        assert_eq!(one(3, 0, [0, 1, 0, 0])[0].values, vec![65536]);
        // all axes: every slot
        let all = one(3, -1, [0, 0, 0, 5]);
        assert_eq!(all[0].values, vec![5, 0]);
    }

    #[test]
    fn alarm_messages_round_trip() {
        let q = AlarmQuery::default();
        let alarms = vec![
            AlarmMessage {
                number: 100,
                alarm_type: AlarmType::ParameterSwitchOn,
                axis: 0,
                text: Some("PARAMETER WRITE ENABLE".into()),
            },
            AlarmMessage {
                number: 401,
                alarm_type: AlarmType::Servo,
                axis: 2,
                text: Some("SERVO ALARM: VRDY OFF".into()),
            },
        ];
        let bytes = encode_alarm_messages(&q, &alarms);
        assert_eq!(bytes.len(), 2 * (16 + 32));
        assert_eq!(parse_alarm_messages(&q, &bytes).unwrap(), alarms);
        // without text the number, type and axis are still there
        let q0 = AlarmQuery {
            with_text: false,
            ..q
        };
        let parsed = parse_alarm_messages(&q0, &encode_alarm_messages(&q0, &alarms)).unwrap();
        assert!(parsed.iter().all(|a| a.text.is_none()));
        assert_eq!(parsed[1].number, 401);
        assert!(parse_alarm_messages(&q, &bytes[..47]).is_err());
    }

    #[test]
    fn program_list_and_name() {
        let entries = vec![
            ProgramEntry {
                number: 1,
                size: 120,
                comment: "DEMO".into(),
            },
            ProgramEntry {
                number: 3000,
                size: 4096,
                comment: String::new(),
            },
        ];
        let bytes = encode_program_list(&entries);
        assert_eq!(bytes.len(), 144);
        assert_eq!(parse_program_list(&bytes).unwrap(), entries);
        assert!(parse_program_list(&bytes[..100]).is_err());
        assert_eq!(
            parse_program_name(&encode_program_name("//CNC_MEM/USER/PATH1/O3000")),
            "//CNC_MEM/USER/PATH1/O3000"
        );
    }

    #[test]
    fn exec_block_and_position_kinds() {
        let b = ExecBlock {
            block: 12,
            text: "G01 X10. Y5. F300".into(),
        };
        assert_eq!(parse_exec_block(&encode_exec_block(&b)).unwrap(), b);
        assert!(parse_exec_block(&[0, 0]).is_err());
        for k in [
            PositionKind::Absolute,
            PositionKind::Machine,
            PositionKind::Relative,
            PositionKind::DistanceToGo,
            PositionKind::Skip,
        ] {
            assert_eq!(PositionKind::from_code(k.code()), Some(k));
        }
        assert_eq!(PositionKind::from_code(2), None);
    }
}
