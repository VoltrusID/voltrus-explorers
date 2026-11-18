//! DBC files: messages, signals (Intel and Motorola byte order, signed, IEEE float, multiplexed,
//! including extended multiplexing), value tables, comments and attributes. Decoding turns a frame
//! into physical values; encoding builds a frame's data from them.
//!
//! The parser is tolerant: a statement it cannot read is skipped and reported in
//! [`Dbc::warnings`] (line number and text) instead of rejecting the whole file.

use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::CanError;
use crate::frame::{CanFrame, frame_key};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ByteOrder {
    /// `@1`: little-endian, the start bit is the least significant bit.
    Intel,
    /// `@0`: big-endian, the start bit is the most significant bit (DBC sawtooth numbering).
    Motorola,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    Unsigned,
    Signed,
    Float,
    Double,
}

/// The multiplex indicator written on the `SG_` line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Multiplex {
    /// The multiplexor switch (`M`).
    Switch,
    /// Present only when its switch has this value (`m3`); `SG_MUL_VAL_` can widen it to ranges.
    Value(u64),
    /// Extended multiplexing: multiplexed (`m3`) and itself the switch of other signals (`m3M`).
    ValueSwitch(u64),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    pub name: String,
    pub start_bit: u16,
    pub length: u16,
    pub byte_order: ByteOrder,
    pub value_type: ValueType,
    pub factor: f64,
    pub offset: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub receivers: Vec<String>,
    pub multiplex: Option<Multiplex>,
    /// The switch that selects this signal, when it is multiplexed.
    pub mux_switch: Option<String>,
    /// Switch values (inclusive ranges) for which this signal is present.
    pub mux_values: Vec<(u64, u64)>,
    pub comment: String,
    /// Raw value to label.
    pub values: Vec<(i64, String)>,
    #[serde(skip)]
    mux_idx: Option<usize>,
}

impl Signal {
    /// The multiplex column as DBC tools show it: `M`, `m3`, `m3M`, or `m0,3-5` with extended
    /// multiplexing (and `@Switch` when the switch is not the message's only one).
    pub fn mux_label(&self) -> Option<String> {
        let ranges = || {
            self.mux_values
                .iter()
                .map(|&(a, b)| if a == b { a.to_string() } else { format!("{a}-{b}") })
                .collect::<Vec<_>>()
                .join(",")
        };
        match &self.multiplex {
            None => None,
            Some(Multiplex::Switch) => Some("M".into()),
            Some(Multiplex::Value(v)) | Some(Multiplex::ValueSwitch(v)) => {
                let suffix = if matches!(self.multiplex, Some(Multiplex::ValueSwitch(_))) { "M" } else { "" };
                if self.mux_switch.is_none() {
                    return Some(format!("m{v}{suffix}"));
                }
                Some(format!("m{}{suffix}", ranges()))
            }
        }
    }

    /// Whether this signal is a switch for other signals.
    pub fn is_switch(&self) -> bool {
        matches!(self.multiplex, Some(Multiplex::Switch) | Some(Multiplex::ValueSwitch(_)))
    }

    fn is_integer(&self) -> bool {
        matches!(self.value_type, ValueType::Unsigned | ValueType::Signed)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Identifier without the 29-bit flag.
    pub id: u32,
    pub extended: bool,
    pub name: String,
    pub size: u16,
    pub transmitter: String,
    /// Further senders from `BO_TX_BU_`.
    #[serde(default)]
    pub transmitters: Vec<String>,
    pub signals: Vec<Signal>,
    pub comment: String,
    /// `GenMsgCycleTime` (or its `BA_DEF_DEF_` default), when the file has one.
    pub cycle_ms: Option<u32>,
    /// Sent as CAN FD: `VFrameFormat` is `StandardCAN_FD` / `ExtendedCAN_FD`, or the message is longer than 8 bytes.
    #[serde(default)]
    pub fd: bool,
    /// CAN FD with bit rate switch (`CANFD_BRS`, or its `BA_DEF_DEF_` default).
    #[serde(default)]
    pub brs: bool,
    #[serde(skip)]
    by_name: HashMap<String, usize>,
}

impl Message {
    pub fn key(&self) -> u32 {
        frame_key(self.id, self.extended)
    }

    pub fn signal(&self, name: &str) -> Option<&Signal> {
        self.by_name.get(name).map(|&i| &self.signals[i]).or_else(|| self.signals.iter().find(|s| s.name == name))
    }
}

/// A statement the parser skipped or read in a way the user should know about.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcWarning {
    pub line: usize,
    pub message: String,
    /// The text of that line in the file.
    pub source: String,
}

/// Warnings kept per file at most; the rest are counted in the last one.
const MAX_WARNINGS: usize = 200;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dbc {
    pub version: String,
    pub nodes: Vec<String>,
    pub messages: Vec<Message>,
    /// Statements the parser does not use (environment variables, signal groups, ...).
    pub skipped: usize,
    /// Statements that were skipped because they could not be read, and other problems.
    pub warnings: Vec<DbcWarning>,
    /// SAE J1939 database (`ProtocolType` "J1939", `VFrameFormat` J1939PG, or 29-bit J1939 identifiers):
    /// frames are matched by parameter group number when their identifier is not listed.
    pub j1939: bool,
    #[serde(skip)]
    by_key: HashMap<u32, usize>,
    #[serde(skip)]
    by_name: HashMap<String, usize>,
    #[serde(skip)]
    by_pgn: HashMap<u32, usize>,
    #[serde(skip)]
    by_pgn_sa: HashMap<(u32, u8), usize>,
}

/// One decoded signal.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalValue {
    pub name: String,
    pub raw: f64,
    pub value: f64,
    pub unit: String,
    /// Value table label for the raw value.
    pub label: Option<String>,
    /// The value as text when a JSON number can not carry it: `NaN`, `Inf`, `-Inf`, or the exact
    /// integer of a signal wider than 53 bits (factor 1, offset 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A physical value to encode: a number, or text for exact integers of wide signals.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum PhysValue {
    Number(f64),
    Text(String),
}

impl From<f64> for PhysValue {
    fn from(v: f64) -> Self {
        PhysValue::Number(v)
    }
}

impl Dbc {
    pub fn parse(text: &str) -> Result<Dbc, CanError> {
        Parser::new(text).parse()
    }

    pub fn message(&self, id: u32, extended: bool) -> Option<&Message> {
        self.by_key.get(&frame_key(id, extended)).map(|&i| &self.messages[i])
    }

    pub fn message_by_name(&self, name: &str) -> Option<&Message> {
        self.by_name.get(name).map(|&i| &self.messages[i])
    }

    pub fn signal_count(&self) -> usize {
        self.messages.iter().map(|m| m.signals.len()).sum()
    }

    /// The message describing a frame: by identifier, or for J1939 databases by parameter group
    /// (ignoring priority and source address, and the destination of PDU1 groups), preferring a
    /// message from the same source address.
    pub fn message_for(&self, frame: &CanFrame) -> Option<&Message> {
        if let Some(m) = self.message(frame.id, frame.extended) {
            return Some(m);
        }
        if !self.j1939 || !frame.extended {
            return None;
        }
        let j = crate::protocols::j1939(frame.id);
        self.by_pgn_sa.get(&(j.pgn, j.source)).or_else(|| self.by_pgn.get(&j.pgn)).map(|&i| &self.messages[i])
    }

    /// The message and signal values carried by a frame, or `None` when the file does not describe it.
    pub fn decode(&self, frame: &CanFrame) -> Option<(&Message, Vec<SignalValue>)> {
        if frame.rtr {
            return None;
        }
        let msg = self.message_for(frame)?;
        Some((msg, decode_message(msg, &frame.data)))
    }

    fn index(&mut self) {
        self.by_key = HashMap::with_capacity(self.messages.len());
        self.by_name = HashMap::with_capacity(self.messages.len());
        self.by_pgn.clear();
        self.by_pgn_sa.clear();
        for (i, m) in self.messages.iter_mut().enumerate() {
            self.by_key.entry(m.key()).or_insert(i);
            self.by_name.entry(m.name.clone()).or_insert(i);
            m.by_name = HashMap::with_capacity(m.signals.len());
            for (j, s) in m.signals.iter().enumerate() {
                m.by_name.entry(s.name.clone()).or_insert(j);
            }
            if m.extended {
                let j = crate::protocols::j1939(m.id);
                self.by_pgn.entry(j.pgn).or_insert(i);
                self.by_pgn_sa.entry((j.pgn, j.source)).or_insert(i);
            }
        }
    }
}

/// The raw value of a switch signal in `data`, when it fits.
fn switch_value(s: &Signal, data: &[u8]) -> Option<i128> {
    let bits = extract(data, s)?;
    Some(match s.value_type {
        ValueType::Signed => sign_extend(bits, s.length) as i128,
        _ => bits as i128,
    })
}

/// Whether signal `i` is present in `data`: not multiplexed, or its switch is present and has one
/// of its values (cascaded multiplexing follows the chain of switches).
fn active(msg: &Message, data: &[u8], i: usize, memo: &mut [Option<bool>], depth: usize) -> bool {
    if let Some(v) = memo[i] {
        return v;
    }
    let s = &msg.signals[i];
    let v = match s.mux_idx {
        None => true,
        Some(_) if depth > 16 => false,
        Some(sw) => {
            active(msg, data, sw, memo, depth + 1)
                && switch_value(&msg.signals[sw], data)
                    .is_some_and(|v| s.mux_values.iter().any(|&(a, b)| v >= a as i128 && v <= b as i128))
        }
    };
    memo[i] = Some(v);
    v
}

/// Decodes every signal present in `data`. Signals that do not fit in the data, and multiplexed
/// signals whose switch value does not select them, are left out.
pub fn decode_message(msg: &Message, data: &[u8]) -> Vec<SignalValue> {
    let mut memo = vec![None; msg.signals.len()];
    (0..msg.signals.len())
        .filter(|&i| active(msg, data, i, &mut memo, 0))
        .filter_map(|i| decode_signal(&msg.signals[i], data))
        .collect()
}

pub fn decode_signal(s: &Signal, data: &[u8]) -> Option<SignalValue> {
    let bits = extract(data, s)?;
    let (raw, int) = match s.value_type {
        ValueType::Unsigned => (bits as f64, Some(bits as i128)),
        ValueType::Signed => {
            let v = sign_extend(bits, s.length);
            (v as f64, Some(v as i128))
        }
        ValueType::Float => (f32::from_bits(bits as u32) as f64, None),
        ValueType::Double => (f64::from_bits(bits), None),
    };
    let label = int.and_then(|r| {
        s.values
            .iter()
            .rev()
            .find(|(v, _)| *v as i128 == r || (r > i64::MAX as i128 && *v as u64 as i128 == r))
            .map(|(_, l)| l.clone())
    });
    let value = raw * s.factor + s.offset;
    let text = if value.is_nan() {
        Some("NaN".to_string())
    } else if value.is_infinite() {
        Some(if value > 0.0 { "Inf" } else { "-Inf" }.to_string())
    } else {
        match int {
            Some(r) if s.length > 53 && s.factor == 1.0 && s.offset == 0.0 => Some(r.to_string()),
            _ => None,
        }
    };
    Some(SignalValue { name: s.name.clone(), raw, value, unit: s.unit.clone(), label, text })
}

fn sign_extend(bits: u64, len: u16) -> i64 {
    if len == 0 || len >= 64 {
        return bits as i64;
    }
    let shift = 64 - len as u32;
    ((bits << shift) as i64) >> shift
}

/// Positions of the signal's bits in the frame, most significant first. Bit `p` is bit `p % 8` of
/// byte `p / 8`.
fn bit_positions(s: &Signal) -> impl Iterator<Item = u32> {
    let len = s.length as u32;
    let start = s.start_bit as u32;
    let intel = s.byte_order == ByteOrder::Intel;
    // Motorola: start is the MSB; walk down within a byte, then to bit 7 of the next byte.
    let mut p = start;
    (0..len).map(move |i| {
        if intel {
            // Intel: start is the LSB, bits go up.
            start + (len - 1 - i)
        } else {
            let out = p;
            if p.is_multiple_of(8) { p += 15 } else { p = p.wrapping_sub(1) }
            out
        }
    })
}

fn extract(data: &[u8], s: &Signal) -> Option<u64> {
    if s.length == 0 || s.length > 64 {
        return None;
    }
    let mut v: u64 = 0;
    for p in bit_positions(s) {
        let byte = *data.get((p / 8) as usize)?;
        v = (v << 1) | ((byte >> (p % 8)) & 1) as u64;
    }
    Some(v)
}

fn insert(data: &mut [u8], s: &Signal, value: u64) -> Result<(), CanError> {
    let n = s.length as usize;
    for (i, p) in bit_positions(s).enumerate() {
        let bit = (value >> (n - 1 - i)) & 1;
        let byte = data
            .get_mut((p / 8) as usize)
            .ok_or_else(|| CanError::Frame(format!("signal {} runs past the end of its message", s.name)))?;
        if bit == 1 {
            *byte |= 1 << (p % 8);
        } else {
            *byte &= !(1 << (p % 8));
        }
    }
    Ok(())
}

/// Raw bits for a physical value: scaled, rounded and clamped to what the signal can hold.
pub fn raw_for(s: &Signal, physical: f64) -> u64 {
    let scaled = if s.factor != 0.0 { (physical - s.offset) / s.factor } else { 0.0 };
    match s.value_type {
        ValueType::Float => (scaled as f32).to_bits() as u64,
        ValueType::Double => scaled.to_bits(),
        ValueType::Unsigned => {
            let max = if s.length >= 64 { u64::MAX as f64 } else { ((1u128 << s.length) - 1) as f64 };
            // `as u64` saturates, so the 64-bit maximum (which rounds up in f64) stays u64::MAX.
            scaled.round().clamp(0.0, max) as u64
        }
        ValueType::Signed => {
            let half = if s.length >= 64 { i64::MAX as f64 } else { (1i128 << (s.length - 1)) as f64 };
            let r = scaled.round().clamp(-half, half - 1.0) as i64;
            if s.length >= 64 { r as u64 } else { (r as u64) & ((1u64 << s.length) - 1) }
        }
    }
}

/// Raw bits for an exact integer of an integer signal with factor 1 and offset 0 (clamped).
fn raw_for_int(s: &Signal, v: i128) -> u64 {
    let len = s.length.min(64) as u32;
    match s.value_type {
        ValueType::Unsigned => {
            let max = if len >= 64 { u64::MAX as i128 } else { (1i128 << len) - 1 };
            v.clamp(0, max) as u64
        }
        ValueType::Signed => {
            let half = 1i128 << (len - 1);
            let r = v.clamp(-half, half - 1) as i64;
            if len >= 64 { r as u64 } else { (r as u64) & ((1u64 << len) - 1) }
        }
        _ => raw_for(s, v as f64),
    }
}

/// Raw bits for a value given as a number or text. Text is read exactly as an integer for integer
/// signals with factor 1 and offset 0 (so 64-bit values survive), as a number otherwise.
pub fn raw_for_value(s: &Signal, v: &PhysValue) -> Result<u64, CanError> {
    match v {
        PhysValue::Number(n) => Ok(raw_for(s, *n)),
        PhysValue::Text(t) => {
            let t = t.trim();
            if s.is_integer()
                && s.factor == 1.0
                && s.offset == 0.0
                && let Ok(i) = t.parse::<i128>()
            {
                return Ok(raw_for_int(s, i));
            }
            let n = match t.to_ascii_lowercase().as_str() {
                "nan" => f64::NAN,
                "inf" | "+inf" | "infinity" => f64::INFINITY,
                "-inf" | "-infinity" => f64::NEG_INFINITY,
                _ => t.parse::<f64>().map_err(|_| CanError::Frame(format!("signal {}: \"{t}\" is not a number", s.name)))?,
            };
            Ok(raw_for(s, n))
        }
    }
}

/// Builds the data of `msg` from physical values. Signals not named keep their raw value from `base`
/// (or zero). Multiplexed signals are only written when their switch values select them.
pub fn encode_message(msg: &Message, values: &HashMap<String, f64>, base: Option<&[u8]>) -> Result<Vec<u8>, CanError> {
    let values: HashMap<String, PhysValue> = values.iter().map(|(k, v)| (k.clone(), PhysValue::Number(*v))).collect();
    encode_message_values(msg, &values, base)
}

/// [`encode_message`] with values that may be text (exact integers of wide signals).
pub fn encode_message_values(msg: &Message, values: &HashMap<String, PhysValue>, base: Option<&[u8]>) -> Result<Vec<u8>, CanError> {
    let mut data = vec![0u8; msg.size as usize];
    if let Some(b) = base {
        let n = b.len().min(data.len());
        data[..n].copy_from_slice(&b[..n]);
    }
    for name in values.keys() {
        if msg.signal(name).is_none() {
            return Err(CanError::Frame(format!("{} has no signal {name}", msg.name)));
        }
    }
    // Write switches before what they select: by depth in the multiplexing tree.
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(sw) = msg.signals[i].mux_idx {
            d += 1;
            i = sw;
            if d > 16 {
                break;
            }
        }
        d
    };
    let mut order: Vec<(usize, usize)> = (0..msg.signals.len()).map(|i| (depth(i), i)).collect();
    order.sort();
    for (_, i) in order {
        let s = &msg.signals[i];
        let Some(v) = values.get(&s.name) else { continue };
        let mut memo = vec![None; msg.signals.len()];
        if !active(msg, &data, i, &mut memo, 0) {
            continue;
        }
        insert(&mut data, s, raw_for_value(s, v)?).map_err(|_| too_long(msg, s))?;
    }
    Ok(data)
}

fn too_long(msg: &Message, s: &Signal) -> CanError {
    CanError::Frame(format!("signal {} runs past the end of {} ({} bytes)", s.name, msg.name, msg.size))
}

impl PartialEq for Signal {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.start_bit == other.start_bit && self.length == other.length
    }
}

// ---- tokenizer ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Num(&'a str),
    Str(Cow<'a, str>),
    Punct(char),
}

struct Token<'a> {
    tok: Tok<'a>,
    line: usize,
    /// Last line of the token (strings can span lines).
    end_line: usize,
}

/// Splits the text into tokens. `\n`, `\r\n` and a lone `\r` all end a line. Line breaks inside
/// strings become `\n`. An unterminated string ends at the end of its line (with a warning).
fn tokenize<'a>(text: &'a str, warnings: &mut Vec<(usize, String)>) -> Vec<Token<'a>> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 5);
    let mut i = 0;
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'\r' => {
                if b.get(i + 1) != Some(&b'\n') {
                    line += 1;
                }
                i += 1;
            }
            b' ' | b'\t' | 0x0B | 0x0C => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
            }
            b'"' => {
                let start = i;
                let start_line = line;
                i += 1;
                // Owned only when an escape or a \r had to be rewritten.
                let mut s: Option<String> = None;
                let mut seg = i;
                let mut closed = false;
                while i < b.len() {
                    match b[i] {
                        b'\\' if b.get(i + 1) == Some(&b'"') => {
                            let o = s.get_or_insert_with(String::new);
                            o.push_str(&text[seg..i]);
                            o.push('"');
                            i += 2;
                            seg = i;
                        }
                        b'"' => {
                            closed = true;
                            break;
                        }
                        b'\r' => {
                            let o = s.get_or_insert_with(String::new);
                            o.push_str(&text[seg..i]);
                            o.push('\n');
                            line += 1;
                            i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                            seg = i;
                        }
                        b'\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
                let value: Cow<'a, str> = if closed {
                    let v = match s {
                        Some(mut o) => {
                            o.push_str(&text[seg..i]);
                            Cow::Owned(o)
                        }
                        None => Cow::Borrowed(&text[seg..i]),
                    };
                    i += 1;
                    v
                } else {
                    // Unterminated: take the rest of the line as the string and carry on after it.
                    warnings.push((start_line, "unterminated string; read up to the end of the line".into()));
                    line = start_line;
                    let mut j = start + 1;
                    while j < b.len() && b[j] != b'\n' && b[j] != b'\r' {
                        j += 1;
                    }
                    i = j;
                    Cow::Borrowed(&text[start + 1..j])
                };
                out.push(Token { tok: Tok::Str(value), line: start_line, end_line: line });
            }
            _ if c.is_ascii_digit()
                || ((c == b'-' || c == b'+' || c == b'.')
                    && b.get(i + 1).is_some_and(|n| n.is_ascii_digit() || (*n == b'.' && b.get(i + 2).is_some_and(u8::is_ascii_digit)))) =>
            {
                let start = i;
                i += 1;
                while let Some(&n) = b.get(i) {
                    let prev = b[i - 1];
                    if n.is_ascii_digit()
                        || n == b'.'
                        || ((n == b'e' || n == b'E') && b.get(i + 1).is_some_and(|x| x.is_ascii_digit() || *x == b'-' || *x == b'+'))
                        || ((n == b'-' || n == b'+') && (prev == b'e' || prev == b'E'))
                    {
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Token { tok: Tok::Num(&text[start..i]), line, end_line: line });
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                out.push(Token { tok: Tok::Ident(&text[start..i]), line, end_line: line });
            }
            _ => {
                let ch = text[i..].chars().next().unwrap_or('?');
                out.push(Token { tok: Tok::Punct(ch), line, end_line: line });
                i += ch.len_utf8().max(1);
            }
        }
    }
    out
}

/// Keywords that start a statement.
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "VERSION"
            | "NS_"
            | "BS_"
            | "BU_"
            | "BO_"
            | "SG_"
            | "CM_"
            | "BA_DEF_"
            | "BA_"
            | "BA_DEF_DEF_"
            | "VAL_"
            | "VAL_TABLE_"
            | "EV_"
            | "ENVVAR_DATA_"
            | "EV_DATA_"
            | "SIG_VALTYPE_"
            | "BO_TX_BU_"
            | "SG_MUL_VAL_"
            | "SIG_GROUP_"
            | "SGTYPE_"
            | "SGTYPE_VAL_"
            | "SIG_TYPE_REF_"
            | "SIGTYPE_VALTYPE_"
            | "BA_DEF_SGTYPE_"
            | "BA_SGTYPE_"
            | "CAT_DEF_"
            | "CAT_"
            | "FILTER"
            | "BA_DEF_REL_"
            | "BA_REL_"
            | "BA_DEF_DEF_REL_"
            | "BU_SG_REL_"
            | "BU_EV_REL_"
            | "BU_BO_REL_"
            | "NS_DESC_"
    )
}

/// Names that appear in the `NS_ :` list (new symbols), which is not made of statements.
fn is_ns_symbol(s: &str) -> bool {
    is_keyword(s) && !matches!(s, "VERSION" | "NS_" | "BS_" | "BU_" | "BO_" | "SG_")
}

// ---- parser -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct AttrValue {
    number: Option<f64>,
    text: Option<String>,
}

impl AttrValue {
    /// An enum attribute's value as its name: a text value, or an index into the definition's list.
    fn enum_text(&self, choices: Option<&Vec<String>>) -> Option<String> {
        if let Some(t) = &self.text {
            return Some(t.clone());
        }
        let n = self.number?;
        match choices {
            Some(c) if !c.is_empty() => c.get(n as usize).cloned(),
            _ => None,
        }
    }
}

/// Where the statement being parsed puts its signals.
#[derive(Clone, Copy, PartialEq)]
enum Current {
    None,
    Message(usize),
    /// A message that was left out (with a warning, or the pseudo message of unassigned signals).
    Skipped,
}

#[derive(Default)]
struct Pending {
    msg_comments: Vec<(u64, String)>,
    sig_comments: Vec<(u64, String, String)>,
    tables: Vec<(u64, String, Vec<(i64, String)>)>,
    valtypes: Vec<(u64, String, u64)>,
    /// `SG_MUL_VAL_`: message, signal, switch, ranges.
    mux_vals: Vec<(u64, String, String, Vec<(u64, u64)>)>,
    tx_bu: Vec<(u64, Vec<String>)>,
    /// `BA_DEF_` enum choices (empty for other types).
    attr_defs: HashMap<String, Vec<String>>,
    attr_defaults: HashMap<String, AttrValue>,
    msg_attrs: Vec<(u64, String, AttrValue)>,
    sig_attrs: Vec<(u64, String, String, AttrValue)>,
    node_attrs: Vec<(String, String, AttrValue)>,
    net_attrs: HashMap<String, AttrValue>,
}

struct Parser<'a> {
    text: &'a str,
    toks: Vec<Token<'a>>,
    pos: usize,
    warnings: Vec<(usize, String)>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        Parser { text, toks: Vec::new(), pos: 0, warnings: Vec::new() }
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).or_else(|| self.toks.last()).map(|t| t.line).unwrap_or(1)
    }

    fn fail<T>(&self, msg: impl Into<String>) -> Result<T, CanError> {
        Err(CanError::Dbc { line: self.line(), msg: msg.into() })
    }

    fn warn(&mut self, line: usize, msg: impl Into<String>) {
        self.warnings.push((line, msg.into()));
    }

    fn peek(&self) -> Option<&Tok<'a>> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    /// Whether the token at `pos` is a statement keyword at the start of its line.
    fn stmt_start(&self, pos: usize) -> bool {
        let Some(t) = self.toks.get(pos) else { return false };
        let Tok::Ident(k) = &t.tok else { return false };
        is_keyword(k) && (pos == 0 || self.toks[pos - 1].end_line < t.line)
    }

    /// Whether the token at `pos` is on the same line as the one before it.
    fn same_line(&self, pos: usize) -> bool {
        pos > 0 && self.toks.get(pos).is_some_and(|t| t.line == self.toks[pos - 1].end_line)
    }

    fn punct(&mut self, c: char) -> Result<(), CanError> {
        match self.peek() {
            Some(Tok::Punct(p)) if *p == c => {
                self.pos += 1;
                Ok(())
            }
            other => self.fail(format!("expected '{c}', found {}", show(other))),
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if matches!(self.peek(), Some(Tok::Punct(p)) if *p == c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Result<String, CanError> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.to_string();
                self.pos += 1;
                Ok(s)
            }
            other => self.fail(format!("expected a name, found {}", show(other))),
        }
    }

    fn string(&mut self) -> Result<String, CanError> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let s = s.to_string();
                self.pos += 1;
                Ok(s)
            }
            other => self.fail(format!("expected a quoted string, found {}", show(other))),
        }
    }

    fn number(&mut self) -> Result<f64, CanError> {
        match self.peek() {
            Some(Tok::Num(s)) => match s.parse::<f64>() {
                Ok(v) => {
                    self.pos += 1;
                    Ok(v)
                }
                Err(_) => self.fail(format!("bad number {s}")),
            },
            other => self.fail(format!("expected a number, found {}", show(other))),
        }
    }

    fn integer(&mut self) -> Result<u64, CanError> {
        match self.peek() {
            Some(Tok::Num(s)) => match s.strip_prefix('+').unwrap_or(s).parse::<u64>() {
                Ok(v) => {
                    self.pos += 1;
                    Ok(v)
                }
                Err(_) => self.fail(format!("expected a whole number, found {s}")),
            },
            other => self.fail(format!("expected a whole number, found {}", show(other))),
        }
    }

    fn signed_integer(&mut self) -> Result<i64, CanError> {
        match self.peek() {
            Some(Tok::Num(s)) => {
                let v = s
                    .parse::<i64>()
                    .ok()
                    .or_else(|| s.parse::<u64>().ok().map(|u| u as i64))
                    .or_else(|| s.parse::<f64>().ok().filter(|f| f.fract() == 0.0).map(|f| f as i64));
                match v {
                    Some(v) => {
                        self.pos += 1;
                        Ok(v)
                    }
                    None => self.fail(format!("expected a whole number, found {s}")),
                }
            }
            other => self.fail(format!("expected a whole number, found {}", show(other))),
        }
    }

    /// An attribute value: a number or a quoted string.
    fn attr_value(&mut self) -> Result<AttrValue, CanError> {
        match self.peek() {
            Some(Tok::Num(n)) => {
                let v = AttrValue { number: n.parse().ok(), text: None };
                self.pos += 1;
                Ok(v)
            }
            Some(Tok::Str(t)) => {
                let v = AttrValue { number: None, text: Some(t.to_string()) };
                self.pos += 1;
                Ok(v)
            }
            other => self.fail(format!("expected an attribute value, found {}", show(other))),
        }
    }

    /// The `;` that ends a statement. A missing one is accepted at the end of the file and before
    /// the next statement (with a warning).
    fn end(&mut self) -> Result<(), CanError> {
        if self.eat(';') {
            return Ok(());
        }
        if self.pos >= self.toks.len() {
            return Ok(());
        }
        if self.stmt_start(self.pos) {
            let line = self.toks[self.pos - 1].end_line;
            self.warn(line, "missing ';' at the end of the statement");
            return Ok(());
        }
        self.fail(format!("expected ';', found {}", show(self.peek())))
    }

    /// Skips to the end of the current statement: just after its `;`, or up to the next statement.
    fn skip_statement(&mut self) {
        while self.pos < self.toks.len() {
            if self.stmt_start(self.pos) {
                return;
            }
            let semi = self.toks[self.pos].tok == Tok::Punct(';');
            self.pos += 1;
            if semi {
                return;
            }
        }
    }

    /// After an error in the statement that started at `start`: on to the next statement.
    fn recover(&mut self, start: usize) {
        self.pos = self.pos.max(start + 1);
        self.skip_statement();
    }

    fn parse(mut self) -> Result<Dbc, CanError> {
        let mut tok_warnings = Vec::new();
        self.toks = tokenize(self.text, &mut tok_warnings);
        self.warnings = tok_warnings;
        let mut dbc = Dbc::default();
        let mut p = Pending::default();
        let mut raw_to_idx: HashMap<u64, usize> = HashMap::new();
        let mut key_to_idx: HashMap<u32, usize> = HashMap::new();
        let mut current = Current::None;
        let mut statements = 0usize;
        let mut outside_warned = false;

        while let Some(tok) = self.peek().cloned() {
            let start = self.pos;
            let line = self.line();
            let Tok::Ident(kw) = tok else {
                // Stray punctuation or numbers between statements.
                self.warn(line, format!("unexpected {} between statements", show(Some(&tok))));
                self.recover(start);
                continue;
            };
            self.pos += 1;
            if is_keyword(&kw) {
                statements += 1;
            }
            let r: Result<(), CanError> = match kw {
                "VERSION" => self.string().map(|v| dbc.version = v),
                "NS_" => {
                    self.eat(':');
                    // The list of new symbols: bare keywords, one per line (indented or not) or
                    // several on a line. A keyword followed by something else on its line is a statement.
                    while let Some(Tok::Ident(k)) = self.peek() {
                        let keyword = is_keyword(k);
                        if keyword && !is_ns_symbol(k) {
                            break;
                        }
                        let next_same_line = self.same_line(self.pos + 1);
                        let next_is_symbol = matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Ident(n)) if is_ns_symbol(n));
                        if keyword && next_same_line && !next_is_symbol {
                            break;
                        }
                        self.pos += 1;
                    }
                    Ok(())
                }
                "BS_" => {
                    // `BS_: baudrate : btr1 , btr2 ;` (obsolete; usually empty)
                    self.eat(':');
                    while self.pos < self.toks.len() && self.same_line(self.pos) && !self.stmt_start(self.pos) {
                        self.pos += 1;
                    }
                    self.eat(';');
                    Ok(())
                }
                "BU_" => {
                    self.eat(':');
                    // Node names, possibly continued on the following lines.
                    while let Some(Tok::Ident(n)) = self.peek() {
                        if is_keyword(n) {
                            break;
                        }
                        dbc.nodes.push(n.to_string());
                        self.pos += 1;
                    }
                    self.eat(';');
                    Ok(())
                }
                "BO_" => {
                    current = Current::Skipped;
                    self.message(line, &mut dbc, &mut raw_to_idx, &mut key_to_idx).map(|c| current = c)
                }
                "SG_" => match self.signal(line) {
                    Ok(sig) => {
                        match current {
                            Current::Message(m) => dbc.messages[m].signals.push(sig),
                            Current::Skipped => {}
                            Current::None => {
                                if !outside_warned {
                                    self.warn(line, format!("signal {} is not inside a message; skipped", sig.name));
                                    outside_warned = true;
                                }
                            }
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                "CM_" => {
                    current = Current::None;
                    self.comment(&mut p)
                }
                "VAL_" => {
                    current = Current::None;
                    if !matches!(self.peek(), Some(Tok::Num(_))) {
                        // `VAL_ <env var> ...`: environment variables are not used here.
                        dbc.skipped += 1;
                        self.skip_statement();
                        Ok(())
                    } else {
                        self.value_table(&mut p)
                    }
                }
                "SIG_VALTYPE_" => {
                    current = Current::None;
                    (|| {
                        let id = self.integer()?;
                        let sig = self.ident()?;
                        self.eat(':');
                        let t = self.integer()?;
                        self.end()?;
                        p.valtypes.push((id, sig, t));
                        Ok(())
                    })()
                }
                "SG_MUL_VAL_" => {
                    current = Current::None;
                    self.mux_values(&mut p)
                }
                "BO_TX_BU_" => {
                    current = Current::None;
                    (|| {
                        let id = self.integer()?;
                        self.eat(':');
                        let mut nodes = Vec::new();
                        while let Some(Tok::Ident(n)) = self.peek() {
                            if self.stmt_start(self.pos) {
                                break;
                            }
                            nodes.push(n.to_string());
                            self.pos += 1;
                            self.eat(',');
                        }
                        self.end()?;
                        p.tx_bu.push((id, nodes));
                        Ok(())
                    })()
                }
                "BA_DEF_" => {
                    current = Current::None;
                    self.attr_def(&mut p)
                }
                "BA_DEF_DEF_" => {
                    current = Current::None;
                    (|| {
                        let attr = self.string()?;
                        let v = self.attr_value()?;
                        self.end()?;
                        p.attr_defaults.insert(attr, v);
                        Ok(())
                    })()
                }
                "BA_" => {
                    current = Current::None;
                    self.attribute(&mut p)
                }
                k if is_keyword(k) => {
                    // VAL_TABLE_, EV_, SIG_GROUP_, BA_DEF_REL_, ...: not used here.
                    current = Current::None;
                    dbc.skipped += 1;
                    self.skip_statement();
                    Ok(())
                }
                _ => {
                    self.fail(format!("unknown statement {kw}"))
                }
            };
            if let Err(e) = r {
                let msg = match e {
                    CanError::Dbc { msg, .. } => msg,
                    other => other.to_string(),
                };
                let what = match kw {
                    "SG_" => "signal skipped",
                    "BO_" => "message skipped",
                    k if is_keyword(k) => "statement skipped",
                    _ => "skipped",
                };
                self.warn(line, format!("{msg}; {what}"));
                self.recover(start);
            }
        }

        self.apply(&mut dbc, p, &raw_to_idx);
        dbc.index();

        if dbc.messages.is_empty() && !self.text.trim().is_empty() {
            if statements == 0 {
                return Err(CanError::Dbc { line: 1, msg: "no messages found — is this a DBC file?".into() });
            }
            self.warn(1, "no messages found — is this a DBC file?");
        }
        dbc.warnings = self.finish_warnings();
        Ok(dbc)
    }

    /// `BO_ <id> <name>: <size> <transmitter>`; returns where its signals go.
    fn message(
        &mut self,
        line: usize,
        dbc: &mut Dbc,
        raw_to_idx: &mut HashMap<u64, usize>,
        key_to_idx: &mut HashMap<u32, usize>,
    ) -> Result<Current, CanError> {
        let raw = self.integer()?;
        let name = self.ident()?;
        self.punct(':')?;
        let size = self.integer()?;
        let transmitter = match self.peek() {
            Some(Tok::Ident(t)) if self.same_line(self.pos) && !is_keyword(t) => self.ident()?,
            _ => String::new(),
        };
        // Signals that belong to no message (CANdb++), and its identifiers 0xC0000000 / 0x40000000.
        if name == "VECTOR__INDEPENDENT_SIG_MSG" || raw == 0xC000_0000 || raw == 0x4000_0000 {
            return Ok(Current::Skipped);
        }
        let (id, extended) = if raw & 0x8000_0000 != 0 {
            let id = raw & 0x7FFF_FFFF;
            if id > 0x1FFF_FFFF {
                self.warn(line, format!("message {name}: identifier {id:#X} is longer than 29 bits; message skipped"));
                return Ok(Current::Skipped);
            }
            (id as u32, true)
        } else if raw > 0x1FFF_FFFF {
            self.warn(line, format!("message {name}: identifier {raw:#X} is longer than 29 bits; message skipped"));
            return Ok(Current::Skipped);
        } else if raw > 0x7FF {
            self.warn(
                line,
                format!("message {name}: identifier {raw:#X} needs 29 bits but has no extended flag (0x80000000); read as extended"),
            );
            (raw as u32, true)
        } else {
            (raw as u32, false)
        };
        if size > 64 {
            self.warn(line, format!("message {name} is {size} bytes long (64 at most); message skipped"));
            return Ok(Current::Skipped);
        }
        let key = frame_key(id, extended);
        if let Some(&first) = key_to_idx.get(&key) {
            let first = dbc.messages[first].name.clone();
            self.warn(line, format!("message {name} has the same identifier as {first}; {name} skipped (the first one is used)"));
            return Ok(Current::Skipped);
        }
        dbc.messages.push(Message {
            id,
            extended,
            name,
            size: size as u16,
            transmitter,
            transmitters: Vec::new(),
            signals: Vec::new(),
            comment: String::new(),
            cycle_ms: None,
            fd: false,
            brs: false,
            by_name: HashMap::new(),
        });
        let idx = dbc.messages.len() - 1;
        key_to_idx.insert(key, idx);
        raw_to_idx.entry(raw).or_insert(idx);
        Ok(Current::Message(idx))
    }

    fn signal(&mut self, line: usize) -> Result<Signal, CanError> {
        let name = self.ident()?;
        let mut multiplex = None;
        if let Some(Tok::Ident(m)) = self.peek().cloned() {
            self.pos += 1;
            multiplex = if m == "M" {
                Some(Multiplex::Switch)
            } else if m == "m" {
                self.warn(line, format!("signal {name}: multiplex indicator 'm' without a value, read as the switch 'M'"));
                Some(Multiplex::Switch)
            } else if let Some(rest) = m.strip_prefix('m') {
                let (digits, sw) = match rest.strip_suffix('M') {
                    Some(d) => (d, true),
                    None => (rest, false),
                };
                match digits.parse::<u64>() {
                    Ok(v) if sw => Some(Multiplex::ValueSwitch(v)),
                    Ok(v) => Some(Multiplex::Value(v)),
                    Err(_) => return self.fail(format!("bad multiplex indicator {m} on signal {name}")),
                }
            } else {
                return self.fail(format!("bad multiplex indicator {m} on signal {name}"));
            };
        }
        self.punct(':')?;
        let start = self.integer()?;
        self.punct('|')?;
        let length = self.integer()?;
        self.punct('@')?;
        // `1+` tokenizes as the number `1` and `+`; `0-` as `0` and `-`.
        let order = self.integer()?;
        let sign = match self.peek() {
            Some(Tok::Punct('+')) => ValueType::Unsigned,
            Some(Tok::Punct('-')) => ValueType::Signed,
            other => return self.fail(format!("expected + or -, found {}", show(other))),
        };
        self.pos += 1;
        self.punct('(')?;
        let factor = self.number()?;
        self.punct(',')?;
        let offset = self.number()?;
        self.punct(')')?;
        self.punct('[')?;
        let min = self.number()?;
        self.punct('|')?;
        let max = self.number()?;
        self.punct(']')?;
        let unit = self.string()?;
        let mut receivers = Vec::new();
        while self.same_line(self.pos) {
            match self.peek() {
                Some(Tok::Ident(r)) if !is_keyword(r) => receivers.push(r.to_string()),
                Some(Tok::Punct(',')) => {}
                _ => break,
            }
            self.pos += 1;
        }
        if length == 0 || length > 64 {
            return self.fail(format!("signal {name} is {length} bits long (1 to 64)"));
        }
        if start > 511 {
            return self.fail(format!("signal {name} starts at bit {start} (511 at most)"));
        }
        Ok(Signal {
            name,
            start_bit: start as u16,
            length: length as u16,
            byte_order: if order == 1 { ByteOrder::Intel } else { ByteOrder::Motorola },
            value_type: sign,
            factor,
            offset,
            min,
            max,
            unit,
            receivers,
            multiplex,
            mux_switch: None,
            mux_values: Vec::new(),
            comment: String::new(),
            values: Vec::new(),
            mux_idx: None,
        })
    }

    fn comment(&mut self, p: &mut Pending) -> Result<(), CanError> {
        match self.peek().cloned() {
            Some(Tok::Ident(k)) if k == "BO_" => {
                self.pos += 1;
                let id = self.integer()?;
                let text = self.string()?;
                self.end()?;
                p.msg_comments.push((id, text));
            }
            Some(Tok::Ident(k)) if k == "SG_" => {
                self.pos += 1;
                let id = self.integer()?;
                let sig = self.ident()?;
                let text = self.string()?;
                self.end()?;
                p.sig_comments.push((id, sig, text));
            }
            Some(Tok::Ident(k)) if k == "BU_" || k == "EV_" => {
                self.pos += 1;
                self.ident()?;
                self.string()?;
                self.end()?;
            }
            Some(Tok::Str(_)) => {
                self.pos += 1;
                self.end()?;
            }
            other => return self.fail(format!("expected BO_, SG_, BU_, EV_ or a string after CM_, found {}", show(other.as_ref()))),
        }
        Ok(())
    }

    fn value_table(&mut self, p: &mut Pending) -> Result<(), CanError> {
        let id = self.integer()?;
        let sig = self.ident()?;
        let mut entries = Vec::new();
        while matches!(self.peek(), Some(Tok::Num(_))) {
            let v = self.signed_integer()?;
            let l = self.string()?;
            entries.push((v, l));
        }
        self.end()?;
        p.tables.push((id, sig, entries));
        Ok(())
    }

    /// `SG_MUL_VAL_ <msg> <signal> <switch> 0-0, 3-5;`
    fn mux_values(&mut self, p: &mut Pending) -> Result<(), CanError> {
        let id = self.integer()?;
        let sig = self.ident()?;
        let switch = self.ident()?;
        let mut ranges = Vec::new();
        while matches!(self.peek(), Some(Tok::Num(_))) {
            let a = self.integer()?;
            // `3-5` tokenizes as `3` and `-5`; `3 - 5` as `3`, `-`, `5`.
            let b = match self.peek().cloned() {
                Some(Tok::Num(n)) if n.starts_with('-') => {
                    self.pos += 1;
                    n[1..].parse::<u64>().map_err(|_| CanError::Dbc { line: self.line(), msg: format!("bad range end {n}") })?
                }
                Some(Tok::Punct('-')) => {
                    self.pos += 1;
                    self.integer()?
                }
                _ => a,
            };
            ranges.push((a.min(b), a.max(b)));
            self.eat(',');
        }
        self.end()?;
        p.mux_vals.push((id, sig, switch, ranges));
        Ok(())
    }

    /// `BA_DEF_ [BU_|BO_|SG_|EV_] "name" INT|HEX|FLOAT|STRING|ENUM ...;` (enum choices are kept).
    fn attr_def(&mut self, p: &mut Pending) -> Result<(), CanError> {
        if matches!(self.peek(), Some(Tok::Ident(_))) {
            self.pos += 1;
        }
        let name = self.string()?;
        let kind = self.ident()?;
        let mut choices = Vec::new();
        if kind == "ENUM" {
            while let Some(Tok::Str(s)) = self.peek() {
                choices.push(s.to_string());
                self.pos += 1;
                self.eat(',');
            }
        }
        self.skip_statement();
        p.attr_defs.insert(name, choices);
        Ok(())
    }

    /// `BA_ "name" [BU_ node | BO_ id | SG_ id signal | EV_ var] value;`
    fn attribute(&mut self, p: &mut Pending) -> Result<(), CanError> {
        let name = self.string()?;
        match self.peek().cloned() {
            Some(Tok::Ident(k)) if k == "BO_" => {
                self.pos += 1;
                let id = self.integer()?;
                let v = self.attr_value()?;
                self.end()?;
                p.msg_attrs.push((id, name, v));
            }
            Some(Tok::Ident(k)) if k == "SG_" => {
                self.pos += 1;
                let id = self.integer()?;
                let sig = self.ident()?;
                let v = self.attr_value()?;
                self.end()?;
                p.sig_attrs.push((id, sig, name, v));
            }
            Some(Tok::Ident(k)) if k == "BU_" => {
                self.pos += 1;
                let node = self.ident()?;
                let v = self.attr_value()?;
                self.end()?;
                p.node_attrs.push((node, name, v));
            }
            Some(Tok::Ident(k)) if k == "EV_" => {
                self.skip_statement();
            }
            _ => {
                let v = self.attr_value()?;
                self.end()?;
                p.net_attrs.insert(name, v);
            }
        }
        Ok(())
    }

    /// Attaches comments, value tables, value types, multiplexing and attributes to the messages and
    /// signals they name (by the identifier as written in the file), then applies long names.
    fn apply(&mut self, dbc: &mut Dbc, p: Pending, raw_to_idx: &HashMap<u64, usize>) {
        // Signals by their (short) name, per message.
        let sig_maps: Vec<HashMap<String, usize>> = dbc
            .messages
            .iter()
            .map(|m| {
                let mut h = HashMap::with_capacity(m.signals.len());
                for (i, s) in m.signals.iter().enumerate() {
                    h.entry(s.name.clone()).or_insert(i);
                }
                h
            })
            .collect();
        let find = |raw: u64, sig: &str| -> Option<(usize, usize)> {
            let m = *raw_to_idx.get(&raw)?;
            Some((m, *sig_maps[m].get(sig)?))
        };

        for (raw, text) in p.msg_comments {
            if let Some(&m) = raw_to_idx.get(&raw) {
                dbc.messages[m].comment = text;
            }
        }
        for (raw, sig, text) in p.sig_comments {
            if let Some((m, s)) = find(raw, &sig) {
                dbc.messages[m].signals[s].comment = text;
            }
        }
        for (raw, sig, entries) in p.tables {
            if let Some((m, s)) = find(raw, &sig) {
                dbc.messages[m].signals[s].values = entries;
            }
        }
        for (raw, sig, t) in p.valtypes {
            if let Some((m, s)) = find(raw, &sig) {
                let sg = &mut dbc.messages[m].signals[s];
                sg.value_type = match t {
                    1 => ValueType::Float,
                    2 => ValueType::Double,
                    _ => sg.value_type,
                };
            }
        }
        for (raw, nodes) in p.tx_bu {
            if let Some(&m) = raw_to_idx.get(&raw) {
                let msg = &mut dbc.messages[m];
                for n in nodes {
                    if n != msg.transmitter && !msg.transmitters.contains(&n) {
                        msg.transmitters.push(n);
                    }
                }
            }
        }

        // Multiplexing. `SG_MUL_VAL_` names the switch and the values of a signal; without it a
        // multiplexed signal belongs to the message's switch.
        let mut mux_vals: HashMap<(usize, usize), (usize, Vec<(u64, u64)>)> = HashMap::new();
        for (raw, sig, switch, ranges) in p.mux_vals {
            if let (Some((m, s)), Some((_, sw))) = (find(raw, &sig), find(raw, &switch)) {
                mux_vals.insert((m, s), (sw, ranges));
            }
        }
        let mut mux_warnings = Vec::new();
        for (mi, msg) in dbc.messages.iter_mut().enumerate() {
            // A switch written as plain `M` that `SG_MUL_VAL_` hangs off another switch is a cascaded
            // switch (`m2M`): some writers drop the value from the indicator.
            for i in 0..msg.signals.len() {
                if msg.signals[i].multiplex == Some(Multiplex::Switch)
                    && let Some((sw, ranges)) = mux_vals.get(&(mi, i)).filter(|(sw, r)| *sw != i && !r.is_empty())
                    && msg.signals[*sw].is_switch()
                {
                    msg.signals[i].multiplex = Some(Multiplex::ValueSwitch(ranges[0].0));
                    msg.signals[i].mux_idx = Some(*sw);
                    msg.signals[i].mux_values = ranges.clone();
                }
            }
            let switches: Vec<usize> = (0..msg.signals.len()).filter(|&i| msg.signals[i].is_switch()).collect();
            let top: Vec<usize> = switches.iter().copied().filter(|&i| msg.signals[i].multiplex == Some(Multiplex::Switch)).collect();
            let default = if switches.len() == 1 { Some(switches[0]) } else if top.len() == 1 { Some(top[0]) } else { None };
            for i in 0..msg.signals.len() {
                let n = match msg.signals[i].multiplex {
                    Some(Multiplex::Value(n)) | Some(Multiplex::ValueSwitch(n)) => n,
                    _ => continue,
                };
                if let Some((sw, ranges)) = mux_vals.get(&(mi, i)).filter(|(sw, _)| *sw != i) {
                    let mut ranges = ranges.clone();
                    if Some(*sw) == default && !ranges.iter().any(|&(a, b)| n >= a && n <= b) && switches.len() == 1 {
                        ranges.push((n, n));
                    }
                    msg.signals[i].mux_idx = Some(*sw);
                    msg.signals[i].mux_values = ranges;
                } else if let Some(d) = default.filter(|&d| d != i) {
                    msg.signals[i].mux_idx = Some(d);
                    msg.signals[i].mux_values = vec![(n, n)];
                } else {
                    mux_warnings.push(format!(
                        "signal {}.{} is multiplexed (m{n}) but the message has no switch it can belong to; shown always",
                        msg.name, msg.signals[i].name
                    ));
                }
            }
        }
        for w in mux_warnings {
            self.warn(0, w);
        }

        // Attributes.
        let choices = |name: &str| p.attr_defs.get(name);
        let default = |name: &str| p.attr_defaults.get(name);
        let cycle_default = default("GenMsgCycleTime").and_then(|v| v.number).filter(|&v| v > 0.0);
        let format_text = |v: &AttrValue| -> Option<String> {
            match v.enum_text(choices("VFrameFormat")) {
                Some(t) => Some(t),
                // No definition to look the index up in: Vector's numbering.
                None => v.number.map(|n| match n as u32 {
                    1 => "ExtendedCAN".to_string(),
                    3 => "J1939PG".to_string(),
                    14 => "StandardCAN_FD".to_string(),
                    15 => "ExtendedCAN_FD".to_string(),
                    _ => "StandardCAN".to_string(),
                }),
            }
        };
        let on = |v: &AttrValue, name: &str| -> bool {
            match v.enum_text(choices(name)) {
                Some(t) => t == "1" || t.eq_ignore_ascii_case("yes") || t.eq_ignore_ascii_case("true"),
                None => v.number.is_some_and(|n| n != 0.0),
            }
        };
        let default_format = default("VFrameFormat").and_then(format_text);
        let default_brs = default("CANFD_BRS").is_some_and(|v| on(v, "CANFD_BRS"));
        let mut formats: Vec<Option<String>> = vec![default_format.clone(); dbc.messages.len()];
        for m in &mut dbc.messages {
            m.cycle_ms = cycle_default.map(|v| v as u32);
            m.brs = default_brs;
        }
        let mut msg_names: Vec<(usize, String)> = Vec::new();
        for (raw, name, v) in &p.msg_attrs {
            let Some(&m) = raw_to_idx.get(raw) else { continue };
            match name.as_str() {
                "GenMsgCycleTime" => {
                    dbc.messages[m].cycle_ms = v.number.filter(|&n| n > 0.0).map(|n| n as u32);
                }
                "VFrameFormat" => formats[m] = format_text(v),
                "CANFD_BRS" => dbc.messages[m].brs = on(v, "CANFD_BRS"),
                "SystemMessageLongSymbol" => {
                    if let Some(t) = v.text.as_ref().filter(|t| !t.is_empty()) {
                        msg_names.push((m, t.clone()));
                    }
                }
                _ => {}
            }
        }
        let mut j1939_format = false;
        for (m, f) in dbc.messages.iter_mut().zip(&formats) {
            let f = f.as_deref().unwrap_or("");
            m.fd = f.ends_with("CAN_FD") || m.size > 8;
            m.brs &= m.fd;
            j1939_format |= f == "J1939PG";
        }
        for (raw, sig, name, v) in &p.sig_attrs {
            if name == "SystemSignalLongSymbol"
                && let Some(t) = v.text.as_ref().filter(|t| !t.is_empty())
                && let Some((m, s)) = find(*raw, sig)
            {
                dbc.messages[m].signals[s].name = t.clone();
            }
        }
        for (m, name) in msg_names {
            dbc.messages[m].name = name;
        }
        let node_names: HashMap<String, String> = p
            .node_attrs
            .iter()
            .filter(|(_, a, v)| a == "SystemNodeLongSymbol" && v.text.as_ref().is_some_and(|t| !t.is_empty()))
            .map(|(n, _, v)| (n.clone(), v.text.clone().unwrap_or_default()))
            .collect();
        if !node_names.is_empty() {
            let rename = |n: &mut String| {
                if let Some(l) = node_names.get(n.as_str()) {
                    *n = l.clone();
                }
            };
            dbc.nodes.iter_mut().for_each(rename);
            for m in &mut dbc.messages {
                rename(&mut m.transmitter);
                m.transmitters.iter_mut().for_each(rename);
                for s in &mut m.signals {
                    s.receivers.iter_mut().for_each(rename);
                }
            }
        }
        for m in &mut dbc.messages {
            for i in 0..m.signals.len() {
                if let Some(sw) = m.signals[i].mux_idx {
                    m.signals[i].mux_switch = Some(m.signals[sw].name.clone());
                }
            }
        }

        // J1939: declared, or every message is 29-bit with the usual J1939 priorities (3 and 6).
        let protocol = p.net_attrs.get("ProtocolType").or_else(|| default("ProtocolType")).and_then(|v| v.text.clone());
        let declared = protocol.is_some_and(|t| t.to_ascii_uppercase().contains("J1939"));
        let n = dbc.messages.len();
        let typical = dbc.messages.iter().filter(|m| m.extended && matches!((m.id >> 26) & 7, 3 | 6)).count();
        let heuristic = n > 0 && dbc.messages.iter().all(|m| m.extended) && typical * 4 >= n * 3;
        dbc.j1939 = declared || j1939_format || heuristic;
    }

    /// Warnings in line order, with the text of their line (capped).
    fn finish_warnings(&mut self) -> Vec<DbcWarning> {
        let mut ws = std::mem::take(&mut self.warnings);
        ws.sort_by_key(|(l, _)| *l);
        let total = ws.len();
        let wanted: Vec<usize> = ws.iter().take(MAX_WARNINGS).map(|(l, _)| *l).collect();
        let mut sources: HashMap<usize, String> = HashMap::new();
        if !wanted.is_empty() {
            let set: std::collections::HashSet<usize> = wanted.iter().copied().collect();
            for (i, l) in split_lines(self.text).enumerate() {
                if set.contains(&(i + 1)) {
                    let t = l.trim();
                    let t = if t.chars().count() > 160 { format!("{}…", t.chars().take(160).collect::<String>()) } else { t.to_string() };
                    sources.insert(i + 1, t);
                }
            }
        }
        let mut out: Vec<DbcWarning> = ws
            .into_iter()
            .take(MAX_WARNINGS)
            .map(|(line, message)| DbcWarning { line, source: sources.get(&line).cloned().unwrap_or_default(), message })
            .collect();
        if total > MAX_WARNINGS {
            out.push(DbcWarning { line: 0, message: format!("{} more warnings not shown", total - MAX_WARNINGS), source: String::new() });
        }
        out
    }
}

/// Lines split at `\n`, `\r\n` or a lone `\r` (the tokenizer's line numbering).
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let b = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    std::iter::from_fn(move || {
        if start > b.len() {
            return None;
        }
        while i < b.len() {
            match b[i] {
                b'\n' => {
                    let l = &text[start..i];
                    i += 1;
                    start = i;
                    return Some(l);
                }
                b'\r' => {
                    let l = &text[start..i];
                    i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                    start = i;
                    return Some(l);
                }
                _ => i += 1,
            }
        }
        let l = &text[start..];
        start = b.len() + 1;
        Some(l)
    })
}

fn show(t: Option<&Tok>) -> String {
    match t {
        None => "the end of the file".into(),
        Some(Tok::Ident(s)) => s.to_string(),
        Some(Tok::Num(s)) => s.to_string(),
        Some(Tok::Str(s)) => format!("\"{}\"", s.chars().take(40).collect::<String>()),
        Some(Tok::Punct(c)) => format!("'{c}'"),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"VERSION "1.0"

NS_ :
	NS_DESC_
	CM_
	BA_DEF_
	VAL_

BS_:

BU_: ECU Tester

BO_ 256 Engine: 8 ECU
 SG_ Rpm : 0|16@1+ (0.25,0) [0|16383.75] "rpm" Tester
 SG_ Coolant : 16|8@1+ (1,-40) [-40|215] "degC" Tester
 SG_ Torque : 24|12@1- (0.5,0) [-1024|1023.5] "Nm" Tester
 SG_ Running : 36|1@1+ (1,0) [0|1] "" Tester

BO_ 512 Battery: 8 ECU
 SG_ Voltage : 7|16@0+ (0.01,0) [0|655.35] "V"  Tester
 SG_ Current : 23|16@0- (0.1,0) [-3276.8|3276.7] "A" Tester,ECU

BO_ 2566844158 Ext: 8 ECU
 SG_ Temp : 0|32@1- (1,0) [0|0] "" Tester

BO_ 1232 Diag: 8 ECU
 SG_ Page M : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ Odometer m0 : 8|32@1+ (0.1,0) [0|0] "km" Tester
 SG_ Fuel m1 : 8|8@1+ (0.5,0) [0|100] "%" Tester

CM_ "file comment";
CM_ BO_ 256 "Engine data,
two lines";
CM_ SG_ 256 Rpm "Engine speed";
BA_DEF_ BO_ "GenMsgCycleTime" INT 0 10000;
BA_ "GenMsgCycleTime" BO_ 256 20;
VAL_ 256 Running 0 "Off" 1 "On" ;
SIG_VALTYPE_ 2566844158 Temp : 1;
"#;

    fn dbc() -> Dbc {
        Dbc::parse(SAMPLE).unwrap()
    }

    fn names(v: &[SignalValue]) -> Vec<&str> {
        v.iter().map(|s| s.name.as_str()).collect()
    }

    fn vals(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn parses_structure() {
        let d = dbc();
        assert_eq!(d.version, "1.0");
        assert_eq!(d.nodes, vec!["ECU", "Tester"]);
        assert_eq!(d.messages.len(), 4);
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let e = d.message(256, false).unwrap();
        assert_eq!(e.name, "Engine");
        assert_eq!(e.comment, "Engine data,\ntwo lines");
        assert_eq!(e.cycle_ms, Some(20));
        assert_eq!(e.signal("Rpm").unwrap().comment, "Engine speed");
        assert_eq!(e.signal("Running").unwrap().values, vec![(0, "Off".into()), (1, "On".into())]);
        let x = d.message(0x18FEEEFE, true).unwrap();
        assert_eq!(x.name, "Ext");
        assert_eq!(x.signals[0].value_type, ValueType::Float);
        assert!(d.message(0x18FEEEFE, false).is_none());
        assert_eq!(d.message(512, false).unwrap().signal("Current").unwrap().receivers, vec!["Tester", "ECU"]);
        let diag = d.message_by_name("Diag").unwrap();
        assert_eq!(diag.signal("Fuel").unwrap().mux_switch.as_deref(), Some("Page"));
        assert_eq!(diag.signal("Fuel").unwrap().mux_label().as_deref(), Some("m1"));
        assert_eq!(diag.signal("Page").unwrap().mux_label().as_deref(), Some("M"));
    }

    #[test]
    fn decodes_intel_and_motorola() {
        let d = dbc();
        // Rpm raw 0x1F40 = 8000 -> 2000 rpm; coolant 130 -> 90 degC; torque raw -20 -> -10 Nm; running.
        let torque = (-20i16 as u16) & 0x0FFF;
        let data = [0x40, 0x1F, 130, (torque & 0xFF) as u8, ((torque >> 8) as u8) | 0x10, 0, 0, 0];
        let (m, v) = d.decode(&CanFrame::new(256, &data).unwrap()).unwrap();
        assert_eq!(m.name, "Engine");
        let get = |n: &str| v.iter().find(|s| s.name == n).unwrap().clone();
        assert_eq!(get("Rpm").value, 2000.0);
        assert_eq!(get("Coolant").value, 90.0);
        assert_eq!(get("Torque").value, -10.0);
        assert_eq!(get("Running").label.as_deref(), Some("On"));

        // Motorola: Voltage 0x04D2 = 1234 -> 12.34 V in bytes 0-1 big-endian; Current -15 -> -1.5 A.
        let cur = (-15i16) as u16;
        let data = [0x04, 0xD2, (cur >> 8) as u8, cur as u8, 0, 0, 0, 0];
        let (_, v) = d.decode(&CanFrame::new(512, &data).unwrap()).unwrap();
        assert!((v[0].value - 12.34).abs() < 1e-9);
        assert!((v[1].value + 1.5).abs() < 1e-9);
    }

    #[test]
    fn decodes_float_and_multiplex() {
        let d = dbc();
        let mut data = [0u8; 8];
        data[..4].copy_from_slice(&21.5f32.to_le_bytes());
        let (_, v) = d.decode(&CanFrame::new(0x18FEEEFE, &data).unwrap()).unwrap();
        assert_eq!(v[0].value, 21.5);

        let (_, v) = d.decode(&CanFrame::new(1232, &[0, 0x10, 0x27, 0, 0, 0, 0, 0]).unwrap()).unwrap();
        assert_eq!(names(&v), vec!["Page", "Odometer"]);
        assert_eq!(v[1].value, 1000.0);
        let (_, v) = d.decode(&CanFrame::new(1232, &[1, 150, 0, 0, 0, 0, 0, 0]).unwrap()).unwrap();
        assert_eq!(names(&v), vec!["Page", "Fuel"]);
        assert_eq!(v[1].value, 75.0);
        // A page nobody uses: only the switch.
        let (_, v) = d.decode(&CanFrame::new(1232, &[7, 150, 0, 0, 0, 0, 0, 0]).unwrap()).unwrap();
        assert_eq!(names(&v), vec!["Page"]);
    }

    #[test]
    fn encodes_round_trip() {
        let d = dbc();
        let m = d.message_by_name("Engine").unwrap();
        let v = vals(&[("Rpm", 3500.25), ("Coolant", -12.0), ("Torque", -300.5), ("Running", 1.0)]);
        let data = encode_message(m, &v, None).unwrap();
        for s in decode_message(m, &data) {
            assert!((s.value - v[&s.name]).abs() < 1e-9, "{} {}", s.name, s.value);
        }
        let b = d.message_by_name("Battery").unwrap();
        let data = encode_message(b, &vals(&[("Voltage", 27.6), ("Current", -100.3)]), None).unwrap();
        let got = decode_message(b, &data);
        assert!((got[0].value - 27.6).abs() < 1e-9 && (got[1].value + 100.3).abs() < 1e-9);
        // Clamped to the raw range.
        let data = encode_message(m, &vals(&[("Coolant", 1000.0)]), None).unwrap();
        assert_eq!(data[2], 255);
        // Multiplexed: only the selected page is written.
        let dg = d.message_by_name("Diag").unwrap();
        let data = encode_message(dg, &vals(&[("Page", 1.0), ("Fuel", 40.0), ("Odometer", 5.0)]), None).unwrap();
        assert_eq!(&data[..2], &[1, 80]);
        assert!(encode_message(m, &vals(&[("Nope", 1.0)]), None).is_err());
    }

    /// CAN FD layouts: multiplexed and Motorola signals past byte 8, and the frame format attributes.
    const FD_DBC: &str = r#"
BO_ 2147484160 FdMux: 64 BMS
 SG_ Page M : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ CellV12 m0 : 384|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ CellT40 m1 : 504|8@1- (1,-40) [-168|87] "degC" Tester
 SG_ PackCurrent : 79|16@0- (0.1,0) [-3276.8|3276.7] "A" Tester
BO_ 300 ShortFd: 8 BMS
 SG_ X : 0|8@1+ (1,0) [0|255] "" Tester
BO_ 301 SlowFd: 12 BMS
 SG_ Y : 88|8@1+ (1,0) [0|255] "" Tester
BO_ 302 Classic: 8 BMS
 SG_ Z : 0|8@1+ (1,0) [0|255] "" Tester
BA_DEF_ BO_ "VFrameFormat" ENUM "StandardCAN","ExtendedCAN","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","StandardCAN_FD","ExtendedCAN_FD";
BA_DEF_ BO_ "CANFD_BRS" ENUM "0","1";
BA_DEF_DEF_ "VFrameFormat" "StandardCAN";
BA_DEF_DEF_ "CANFD_BRS" "0";
BA_DEF_DEF_ "BusType" "CAN FD";
BA_ "VFrameFormat" BO_ 2147484160 15;
BA_ "CANFD_BRS" BO_ 2147484160 1;
BA_ "VFrameFormat" BO_ 300 14;
BA_ "CANFD_BRS" BO_ 302 1;
"#;

    #[test]
    fn fd_frame_formats() {
        let d = Dbc::parse(FD_DBC).unwrap();
        let m = |n: &str| d.message_by_name(n).unwrap();
        assert!(m("FdMux").fd && m("FdMux").brs && m("FdMux").extended);
        // VFrameFormat makes an 8-byte message FD; a 12-byte one is FD without it.
        assert!(m("ShortFd").fd && !m("ShortFd").brs);
        assert!(m("SlowFd").fd && !m("SlowFd").brs);
        // BRS on a classic message means nothing.
        assert!(!m("Classic").fd && !m("Classic").brs);
        assert_eq!(d.skipped, 0);
        // A file with no attributes at all: classic unless longer than 8 bytes.
        let plain = Dbc::parse("BO_ 1 A: 8 X\nBO_ 2 B: 64 X\n").unwrap();
        assert!(!plain.messages[0].fd && plain.messages[1].fd && !plain.messages[1].brs);
    }

    #[test]
    fn fd_signals_past_byte_8() {
        let d = Dbc::parse(FD_DBC).unwrap();
        let mut data = vec![0u8; 64];
        data[48..50].copy_from_slice(&3456u16.to_le_bytes()); // CellV12 = 3.456 V
        data[63] = 65; // CellT40 = 25 degC (page 1 only)
        let cur: i16 = -1234; // Motorola, MSB at bit 79 (byte 9): bytes 9-10 big-endian.
        data[9..11].copy_from_slice(&cur.to_be_bytes());
        let f = CanFrame::fd(0x200, true, true, &data).unwrap();
        let (m, v) = d.decode(&f).unwrap();
        assert_eq!(m.name, "FdMux");
        let get = |v: &[SignalValue], n: &str| v.iter().find(|s| s.name == n).map(|s| s.value);
        assert_eq!(get(&v, "CellV12"), Some(3.456));
        assert_eq!(get(&v, "CellT40"), None);
        assert!((get(&v, "PackCurrent").unwrap() + 123.4).abs() < 1e-9);
        data[0] = 1;
        let (_, v) = d.decode(&CanFrame::fd(0x200, true, true, &data).unwrap()).unwrap();
        assert_eq!((get(&v, "CellV12"), get(&v, "CellT40")), (None, Some(25.0)));
        // Encoding the page-1 signals writes byte 63 and leaves the page-0 cell alone.
        let msg = d.message_by_name("FdMux").unwrap();
        let enc = encode_message(msg, &vals(&[("Page", 1.0), ("CellT40", -10.0), ("CellV12", 4.0)]), None).unwrap();
        assert_eq!(enc.len(), 64);
        assert_eq!((enc[0], enc[63], enc[48], enc[49]), (1, 30, 0, 0));
    }

    #[test]
    fn bad_statements_become_warnings() {
        // A broken signal is skipped; the message and the next signal stay.
        let d = Dbc::parse("BO_ 1 A: 8 X\n SG_ S : 0|8@1+ (1,0 [0|1] \"\" X\n SG_ T : 8|8@1+ (1,0) [0|1] \"\" X\n").unwrap();
        assert_eq!(d.messages[0].signals.len(), 1);
        assert_eq!(d.messages[0].signals[0].name, "T");
        assert_eq!(d.warnings.len(), 1);
        assert_eq!(d.warnings[0].line, 2);
        assert!(d.warnings[0].message.contains("signal skipped"), "{:?}", d.warnings);
        assert!(d.warnings[0].source.starts_with("SG_ S : 0|8@1+"));
        // A signal outside a message, an unterminated string: warnings, not errors.
        let d = Dbc::parse(" SG_ S : 0|8@1+ (1,0) [0|1] \"\" X\nBO_ 1 A: 8 X\n").unwrap();
        assert_eq!(d.messages.len(), 1);
        assert_eq!(d.warnings.len(), 1);
        let d = Dbc::parse("CM_ \"open").unwrap();
        assert!(d.warnings.iter().any(|w| w.message.contains("unterminated")));
        assert!(d.warnings.iter().any(|w| w.message.contains("no messages found")));
        // Not a DBC file at all.
        let e = Dbc::parse("hello, world\nthis is a text file\n").unwrap_err();
        assert!(e.to_string().contains("no messages found"), "{e}");
        assert!(Dbc::parse("").unwrap().messages.is_empty());
    }

    #[test]
    fn tolerates_opendbc_quirks() {
        // vw_pq.dbc: a bare `m` is the switch; gm_global_a_powertrain.dbc: no `;` at the end.
        let text = "BO_ 648 Motor_2: 8 Motor\n SG_ Code m : 6|2@1+ (1,0) [0|3] \"\" GW\n SG_ A m2 : 0|6@1+ (1,0) [0|63] \"\" GW\n SG_ B m3 : 0|6@1+ (10,0) [0|630] \"Nm\" GW\nVAL_ 648 Code 1 \"one\" 0 \"zero\"";
        let d = Dbc::parse(text).unwrap();
        let m = d.message_by_name("Motor_2").unwrap();
        assert_eq!(m.signals.len(), 3);
        assert_eq!(m.signal("Code").unwrap().values.len(), 2);
        let (_, v) = d.decode(&CanFrame::new(648, &[0x80 | 5, 0, 0, 0, 0, 0, 0, 0]).unwrap()).unwrap();
        assert_eq!(names(&v), vec!["Code", "A"]);
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        // A missing `;` before the next statement.
        let d = Dbc::parse("BO_ 1 A: 8 X\n SG_ S : 0|8@1+ (1,0) [0|1] \"\" X\nCM_ BO_ 1 \"x\"\nCM_ SG_ 1 S \"y\";\n").unwrap();
        assert_eq!(d.messages[0].comment, "x");
        assert_eq!(d.messages[0].signals[0].comment, "y");
        assert_eq!(d.warnings.len(), 1);
        // Signals over 64 bits are skipped, the rest of the message stays.
        let d = Dbc::parse("BO_ 100 M1: 64 A\n SG_ Big : 0|128@1+ (1,0) [0|0] \"\" B\n SG_ S : 200|8@1+ (1,0) [0|255] \"\" B\nBO_ 101 M2: 8 A\n").unwrap();
        assert_eq!(d.messages.len(), 2);
        assert_eq!(names(&decode_message(&d.messages[0], &[0; 64])), vec!["S"]);
        assert!(d.warnings[0].message.contains("128 bits"));
    }

    #[test]
    fn ns_block_variants_and_line_endings() {
        let body = "BS_:\nBU_: A B\nBO_ 100 M1: 8 A\n SG_ S : 0|8@1+ (1,0) [0|255] \"\" B\nBO_ 101 M2: 8 A\n SG_ T : 0|8@1+ (1,0) [0|255] \"\" B\nCM_ BO_ 100 \"x\";\n";
        for ns in [
            "NS_ :\n\tNS_DESC_\n\tCM_\n\tBA_DEF_\n\n",
            "NS_ :\nNS_DESC_\nCM_\nBA_DEF_\nBA_\nVAL_\n\n",
            " NS_ :\n\tNS_DESC_\n\tCM_\n\n",
            "NS_: NS_DESC_ CM_ BA_DEF_\n",
            "NS_ :\n",
        ] {
            let text = format!("VERSION \"\"\n{ns}{body}");
            for t in [text.clone(), text.replace('\n', "\r\n"), text.replace('\n', "\r")] {
                let d = Dbc::parse(&t).unwrap();
                assert_eq!(d.messages.len(), 2, "{t:?}");
                assert_eq!(d.nodes, vec!["A", "B"], "{t:?}");
                assert_eq!(d.messages[0].comment, "x");
                assert!(d.warnings.is_empty(), "{t:?} {:?}", d.warnings);
            }
        }
        // CR line endings: line numbers still count, and comments lose their \r.
        let d = Dbc::parse("BO_ 1 A: 8 X\r SG_ S : 0|8@1+ (1,0 [0|1] \"\" X\rCM_ BO_ 1 \"a\r\nb\";").unwrap();
        assert_eq!(d.warnings[0].line, 2);
        assert_eq!(d.messages[0].comment, "a\nb");
    }

    #[test]
    fn nodes_on_several_lines_and_tx_bu() {
        let d = Dbc::parse("BU_: A B\n C D\nBO_ 100 M1: 8 A\n SG_ S : 0|8@1+ (1,0) [0|255] \"\" B, C\nBO_TX_BU_ 100 : A,B;\n").unwrap();
        assert_eq!(d.nodes, vec!["A", "B", "C", "D"]);
        assert_eq!(d.messages[0].transmitter, "A");
        assert_eq!(d.messages[0].transmitters, vec!["B"]);
        assert_eq!(d.messages[0].signals[0].receivers, vec!["B", "C"]);
    }

    #[test]
    fn identifiers() {
        let text = "BO_ 274923520 NoFlag: 8 A\n SG_ S : 0|8@1+ (1,0) [0|255] \"\" B\n\
BO_ 2147483939 Ext123: 8 A\n SG_ E : 0|8@1+ (1,0) [0|255] \"\" B\n\
BO_ 291 Std123: 8 A\n SG_ D : 0|8@1+ (1,0) [0|255] \"\" B\n\
BO_ 1073741824 VECTOR__INDEPENDENT_SIG_MSG: 0 Vector__XXX\n SG_ Orphan : 0|8@1+ (1,-50) [0|0] \"\" Vector__XXX\n\
BO_ 3221225472 VECTOR__INDEPENDENT_SIG_MSG: 0 Vector__XXX\n SG_ Orphan2 : 0|8@1+ (1,-50) [0|0] \"\" Vector__XXX\n\
BO_ 4294967295 TooBig: 8 A\n SG_ X : 0|8@1+ (1,0) [0|255] \"\" B\n\
BO_ 1610612736 Bits29And30: 8 A\n\
CM_ BO_ 274923520 \"no flag\";\n";
        let d = Dbc::parse(text).unwrap();
        let names: Vec<&str> = d.messages.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["NoFlag", "Ext123", "Std123"]);
        // 0x10630000 without the flag: a 29-bit identifier, with a warning; attributes still attach.
        let m = d.message(0x1063_0000, true).unwrap();
        assert_eq!(m.comment, "no flag");
        assert!(d.message(0x0063_0000, false).is_none());
        assert_eq!(d.message(0x123, true).unwrap().name, "Ext123");
        assert_eq!(d.message(0x123, false).unwrap().name, "Std123");
        assert_eq!(d.warnings.len(), 3, "{:?}", d.warnings);
        assert!(d.warnings[0].message.contains("read as extended"));
        assert!(d.warnings[1].message.contains("longer than 29 bits"));
    }

    #[test]
    fn duplicate_identifiers_keep_the_first() {
        let text = "BO_ 100 First: 8 A\n SG_ S : 0|8@1+ (1,0) [0|255] \"\" B\nBO_ 100 Second: 8 A\n SG_ T : 8|8@1+ (2,0) [0|255] \"\" B\n\
CM_ BO_ 100 \"comment\";\nBA_DEF_ BO_ \"GenMsgCycleTime\" INT 0 1000;\nBA_ \"GenMsgCycleTime\" BO_ 100 50;\nVAL_ 100 S 1 \"one\";\n";
        let d = Dbc::parse(text).unwrap();
        assert_eq!(d.messages.len(), 1);
        let m = d.message(100, false).unwrap();
        assert_eq!((m.name.as_str(), m.comment.as_str(), m.cycle_ms), ("First", "comment", Some(50)));
        assert_eq!(m.signal("S").unwrap().values, vec![(1, "one".to_string())]);
        assert!(m.signal("T").is_none());
        assert!(d.warnings[0].message.contains("same identifier as First"));
    }

    #[test]
    fn attributes_defaults_enums_and_long_names() {
        let text = r#"BU_: ECU1 ECU2_SHORT
BO_ 100 M1: 8 ECU2_SHORT
 SG_ S : 0|8@1+ (1,0) [0|255] "" ECU1
 SG_ Long_sig_shor : 8|8@1+ (1,0) [0|255] "" ECU1
BO_ 101 M2: 8 ECU1
 SG_ U : 0|8@1+ (1,0) [0|255] "" ECU2_SHORT
BO_ 102 M3: 8 ECU1
 SG_ V m1 : 8|8@1+ (1,0) [0|255] "" ECU2_SHORT
 SG_ W M : 0|8@1+ (1,0) [0|255] "" ECU2_SHORT
BA_DEF_ BO_ "GenMsgCycleTime" INT 0 1000;
BA_DEF_ BO_ "VFrameFormat" ENUM "StandardCAN","ExtendedCAN","StandardCAN_FD","ExtendedCAN_FD";
BA_DEF_ BO_ "SystemMessageLongSymbol" STRING;
BA_DEF_ SG_ "SystemSignalLongSymbol" STRING;
BA_DEF_ BU_ "SystemNodeLongSymbol" STRING;
BA_DEF_DEF_ "GenMsgCycleTime" 100;
BA_DEF_DEF_ "VFrameFormat" "StandardCAN";
BA_ "VFrameFormat" BO_ 101 2;
BA_ "GenMsgCycleTime" BO_ 102 0;
BA_ "SystemMessageLongSymbol" BO_ 100 "Message_with_a_very_long_name_1234567890";
BA_ "SystemSignalLongSymbol" SG_ 100 Long_sig_shor "Long_signal_name_over_32_characters";
BA_ "SystemSignalLongSymbol" SG_ 102 W "Switch_with_a_long_name_0123456789";
BA_ "SystemNodeLongSymbol" BU_ ECU2_SHORT "ECU2_with_a_long_name_0123456789";
VAL_ 100 Long_sig_shor 1 "one";
CM_ SG_ 100 Long_sig_shor "comment";
"#;
        let d = Dbc::parse(text).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let m1 = d.message(100, false).unwrap();
        assert_eq!(m1.name, "Message_with_a_very_long_name_1234567890");
        assert_eq!(d.message_by_name("Message_with_a_very_long_name_1234567890").unwrap().id, 100);
        assert_eq!(m1.cycle_ms, Some(100));
        assert_eq!(m1.transmitter, "ECU2_with_a_long_name_0123456789");
        assert_eq!(d.nodes[1], "ECU2_with_a_long_name_0123456789");
        let s = m1.signal("Long_signal_name_over_32_characters").unwrap();
        assert_eq!((s.comment.as_str(), s.values.len()), ("comment", 1));
        // Enum index 2 of this file's own VFrameFormat list is StandardCAN_FD.
        assert!(d.message(101, false).unwrap().fd);
        assert!(!m1.fd);
        // An explicit 0 means no cycle time.
        let m3 = d.message(102, false).unwrap();
        assert_eq!(m3.cycle_ms, None);
        assert_eq!(m3.signal("V").unwrap().mux_switch.as_deref(), Some("Switch_with_a_long_name_0123456789"));
        let (_, v) = d.decode(&CanFrame::new(102, &[1, 9, 0, 0, 0, 0, 0, 0]).unwrap()).unwrap();
        assert_eq!(names(&v), vec!["V", "Switch_with_a_long_name_0123456789"]);
    }

    const EXT_MUX: &str = r#"
BO_ 100 Multi: 8 A
 SG_ Mux M : 0|8@1+ (1,0) [0|255] "" B
 SG_ Val m0 : 8|8@1+ (1,0) [0|255] "" B
 SG_ Other m1 : 8|8@1+ (1,0) [0|255] "" B
BO_ 101 Cascade: 8 A
 SG_ MuxA M : 0|8@1+ (1,0) [0|255] "" B
 SG_ MuxB m1M : 8|8@1+ (1,0) [0|255] "" B
 SG_ X m2 : 16|8@1+ (1,0) [0|255] "" B
 SG_ Y m2 : 24|8@1+ (1,0) [0|255] "" B
 SG_ Z m3 : 32|8@1+ (1,0) [0|255] "" B
SG_MUL_VAL_ 100 Val Mux 0-0, 3-5;
SG_MUL_VAL_ 101 MuxB MuxA 1-1;
SG_MUL_VAL_ 101 X MuxB 2-2;
SG_MUL_VAL_ 101 Y MuxA 2-2;
SG_MUL_VAL_ 101 Z MuxB 3 - 4;
"#;

    #[test]
    fn extended_multiplexing_decode() {
        let d = Dbc::parse(EXT_MUX).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let m = d.message_by_name("Multi").unwrap();
        assert_eq!(m.signal("Val").unwrap().mux_values, vec![(0, 0), (3, 5)]);
        assert_eq!(m.signal("Val").unwrap().mux_label().as_deref(), Some("m0,3-5"));
        for (mux, want) in [(0u8, vec!["Mux", "Val"]), (1, vec!["Mux", "Other"]), (2, vec!["Mux"]), (4, vec!["Mux", "Val"]), (6, vec!["Mux"])] {
            assert_eq!(names(&decode_message(m, &[mux, 42, 0, 0, 0, 0, 0, 0])), want, "mux {mux}");
        }
        let c = d.message_by_name("Cascade").unwrap();
        assert_eq!(c.signal("MuxB").unwrap().mux_label().as_deref(), Some("m1M"));
        // MuxA 1 selects MuxB; MuxB 2 selects X, 3 or 4 selects Z. Y hangs off MuxA 2.
        assert_eq!(names(&decode_message(c, &[1, 2, 0, 0, 0, 0, 0, 0])), vec!["MuxA", "MuxB", "X"]);
        assert_eq!(names(&decode_message(c, &[1, 4, 0, 0, 0, 0, 0, 0])), vec!["MuxA", "MuxB", "Z"]);
        assert_eq!(names(&decode_message(c, &[2, 2, 0, 0, 0, 0, 0, 0])), vec!["MuxA", "Y"]);
        assert_eq!(names(&decode_message(c, &[0, 2, 0, 0, 0, 0, 0, 0])), vec!["MuxA"]);
    }

    #[test]
    fn extended_multiplexing_encode() {
        let d = Dbc::parse(EXT_MUX).unwrap();
        let m = d.message_by_name("Multi").unwrap();
        // Val is present for 0 and 3-5: writing it with Mux 4 must write it.
        let data = encode_message(m, &vals(&[("Mux", 4.0), ("Val", 42.0)]), None).unwrap();
        assert_eq!(&data[..2], &[4, 42]);
        let data = encode_message(m, &vals(&[("Mux", 2.0), ("Val", 42.0)]), None).unwrap();
        assert_eq!(&data[..2], &[2, 0]);
        // Cascaded: the inner switch is written before what it selects, whatever the map order.
        let c = d.message_by_name("Cascade").unwrap();
        let data = encode_message(c, &vals(&[("X", 7.0), ("MuxB", 2.0), ("MuxA", 1.0), ("Y", 9.0)]), None).unwrap();
        assert_eq!(&data[..4], &[1, 2, 7, 0]);
        let data = encode_message(c, &vals(&[("Z", 5.0), ("MuxB", 3.0), ("MuxA", 1.0)]), None).unwrap();
        assert_eq!(&data[..5], &[1, 3, 0, 0, 5]);
        // MuxB is not selected by MuxA 2, so neither is X.
        let data = encode_message(c, &vals(&[("X", 7.0), ("MuxB", 2.0), ("MuxA", 2.0), ("Y", 9.0)]), None).unwrap();
        assert_eq!(&data[..4], &[2, 0, 0, 9]);
    }

    #[test]
    fn cascaded_switch_written_as_plain_m() {
        // cantools' writer drops the value from `m2M`; SG_MUL_VAL_ still hangs it off MUX_A.
        let text = r#"BO_ 2147483649 ext_MUX_cascaded: 8 Vector__XXX
 SG_ muxed_B_1 m1 : 24|8@1- (1,0) [0|0] "" Vector__XXX
 SG_ muxed_B_0 m0 : 16|8@1- (1,0) [0|0] "" Vector__XXX
 SG_ muxed_A_1 m1 : 8|8@1- (1,0) [0|0] "" Vector__XXX
 SG_ muxed_A_2_MUX_B M : 8|8@1- (1,0) [0|0] "" Vector__XXX
 SG_ MUX_A M : 0|8@1- (1,0) [0|0] "" Vector__XXX
SG_MUL_VAL_ 2147483649 muxed_B_1 muxed_A_2_MUX_B 1-1;
SG_MUL_VAL_ 2147483649 muxed_B_0 muxed_A_2_MUX_B 0-0;
SG_MUL_VAL_ 2147483649 muxed_A_2_MUX_B MUX_A 2-2;
SG_MUL_VAL_ 2147483649 muxed_A_1 MUX_A 1-1;
"#;
        let d = Dbc::parse(text).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let m = &d.messages[0];
        let b = m.signal("muxed_A_2_MUX_B").unwrap();
        assert_eq!((b.mux_switch.as_deref(), b.mux_label().as_deref()), (Some("MUX_A"), Some("m2M")));
        assert_eq!(names(&decode_message(m, &[2, 1, 0, 0x33, 0, 0, 0, 0])), vec!["muxed_B_1", "muxed_A_2_MUX_B", "MUX_A"]);
        assert_eq!(names(&decode_message(m, &[2, 0, 0, 0, 0, 0, 0, 0])), vec!["muxed_B_0", "muxed_A_2_MUX_B", "MUX_A"]);
        assert_eq!(names(&decode_message(m, &[1, 0, 0, 0, 0, 0, 0, 0])), vec!["muxed_A_1", "MUX_A"]);
        let data = encode_message(m, &vals(&[("muxed_B_1", 5.0), ("muxed_A_2_MUX_B", 1.0), ("MUX_A", 2.0)]), None).unwrap();
        assert_eq!(&data[..4], &[2, 1, 0, 5]);
    }

    #[test]
    fn several_switches_without_mul_val() {
        // Two switches and no SG_MUL_VAL_: plain mN signals belong to the top-level `M`.
        let d = Dbc::parse("BO_ 5 E: 8 A\n SG_ S0 M : 0|4@1+ (1,0) [0|0] \"\" B\n SG_ S1 m0M : 4|4@1+ (1,0) [0|0] \"\" B\n SG_ S2 m1 : 8|8@1+ (1,0) [0|0] \"\" B\n").unwrap();
        let m = &d.messages[0];
        assert_eq!(m.signal("S1").unwrap().mux_switch.as_deref(), Some("S0"));
        assert_eq!(m.signal("S2").unwrap().mux_switch.as_deref(), Some("S0"));
        assert_eq!(names(&decode_message(m, &[0x10, 0, 0, 0, 0, 0, 0, 0])), vec!["S0", "S1"]);
        assert_eq!(names(&decode_message(m, &[0x01, 0, 0, 0, 0, 0, 0, 0])), vec!["S0", "S2"]);
    }

    #[test]
    fn wide_signals_are_exact() {
        let d = Dbc::parse("BO_ 101 W: 16 A\n SG_ U64 : 0|64@1+ (1,0) [0|0] \"\" B\n SG_ S64 : 64|64@1- (1,0) [0|0] \"\" B\nBO_ 102 X: 8 A\n SG_ Scaled : 0|60@1+ (2,0) [0|0] \"\" B\n").unwrap();
        let m = d.message_by_name("W").unwrap();
        let big = 0xFFFF_FFFF_FFFF_FFF1u64;
        let neg = -9_007_199_254_740_993i64; // -(2^53 + 1): not representable in f64
        let mut data = big.to_le_bytes().to_vec();
        data.extend_from_slice(&neg.to_le_bytes());
        let v = decode_message(m, &data);
        assert_eq!(v[0].text.as_deref(), Some("18446744073709551601"));
        assert_eq!(v[1].text.as_deref(), Some("-9007199254740993"));
        // Encoding from the text keeps every bit; a number would round.
        let values: HashMap<String, PhysValue> = [
            ("U64".to_string(), PhysValue::Text("18446744073709551601".into())),
            ("S64".to_string(), PhysValue::Text(" -9007199254740993 ".into())),
        ]
        .into_iter()
        .collect();
        assert_eq!(encode_message_values(m, &values, None).unwrap(), data);
        let values: HashMap<String, PhysValue> = [("U64".to_string(), PhysValue::Text("99999999999999999999999".into()))].into_iter().collect();
        assert_eq!(&encode_message_values(m, &values, None).unwrap()[..8], &[0xFF; 8]);
        let values: HashMap<String, PhysValue> = [("U64".to_string(), PhysValue::Text("12abc".into()))].into_iter().collect();
        assert!(encode_message_values(m, &values, None).is_err());
        let values: HashMap<String, PhysValue> = [("U64".to_string(), PhysValue::Number(5.0))].into_iter().collect();
        assert_eq!(encode_message_values(m, &values, None).unwrap()[0], 5);
        // Narrow or scaled signals: no text.
        let x = d.message_by_name("X").unwrap();
        assert_eq!(decode_message(x, &[1, 0, 0, 0, 0, 0, 0, 0])[0].text, None);
        // Value tables match the full 64-bit raw value.
        let d = Dbc::parse("BO_ 1 W: 8 A\n SG_ U : 0|64@1+ (1,0) [0|0] \"\" B\nVAL_ 1 U 18446744073709551615 \"all ones\" 1 \"one\";").unwrap();
        assert_eq!(decode_message(&d.messages[0], &[0xFF; 8])[0].label.as_deref(), Some("all ones"));
    }

    #[test]
    fn non_finite_floats_have_text() {
        let d = Dbc::parse("BO_ 1 F: 8 A\n SG_ D : 0|64@1- (1,0) [0|0] \"\" B\nSIG_VALTYPE_ 1 D : 2;").unwrap();
        let m = &d.messages[0];
        let get = |x: f64| decode_message(m, &x.to_le_bytes())[0].text.clone();
        assert_eq!(get(f64::NAN).as_deref(), Some("NaN"));
        assert_eq!(get(f64::INFINITY).as_deref(), Some("Inf"));
        assert_eq!(get(f64::NEG_INFINITY).as_deref(), Some("-Inf"));
        assert_eq!(get(1.5), None);
    }

    #[test]
    fn j1939_matches_by_pgn() {
        let text = r#"BA_DEF_ "ProtocolType" STRING;
BA_ "ProtocolType" "J1939";
BO_ 2364540158 EEC1: 8 Vector__XXX
 SG_ EngineSpeed : 24|16@1+ (0.125,0) [0|8031.875] "rpm" Vector__XXX
BO_ 2566844158 ET1_FromEngine: 8 Vector__XXX
 SG_ Coolant : 0|8@1+ (1,-40) [-40|210] "degC" Vector__XXX
BO_ 2566843937 ET1_FromRetarder: 8 Vector__XXX
 SG_ Coolant : 0|8@1+ (1,-40) [-40|210] "degC" Vector__XXX
BO_ 2348810494 TSC1: 8 Vector__XXX
 SG_ Requested : 8|16@1+ (0.125,0) [0|8031.875] "rpm" Vector__XXX
"#;
        let d = Dbc::parse(text).unwrap();
        assert!(d.j1939);
        // EEC1 is 0x8CF004FE in the file (SA FE); a frame from SA 00 at priority 6 still matches.
        let f = CanFrame::new(0x18F0_0400, &[0, 0, 0, 0x40, 0x1F, 0, 0, 0]).unwrap();
        let (m, v) = d.decode(&f).unwrap();
        assert_eq!(m.name, "EEC1");
        assert_eq!(v[0].value, 1000.0);
        // Same PGN from two sources: the source address picks the message.
        assert_eq!(d.decode(&CanFrame::new(0x18FE_EE21, &[100; 8]).unwrap()).unwrap().0.name, "ET1_FromRetarder");
        assert_eq!(d.decode(&CanFrame::new(0x18FE_EE00, &[100; 8]).unwrap()).unwrap().0.name, "ET1_FromEngine");
        // PDU1: the destination address is not part of the PGN.
        assert_eq!(d.decode(&CanFrame::new(0x0C00_0317, &[0; 8]).unwrap()).unwrap().0.name, "TSC1");
        assert!(d.decode(&CanFrame::new(0x18FE_EF00, &[0; 8]).unwrap()).is_none());
        // 11-bit frames are never matched by PGN.
        assert!(d.decode(&CanFrame::new(0x400, &[0; 8]).unwrap()).is_none());

        // Without the attribute, 29-bit identifiers at J1939 priorities are enough.
        let plain = text.replace("BA_ \"ProtocolType\" \"J1939\";", "");
        assert!(Dbc::parse(&plain).unwrap().j1939);
        // Not J1939: exact identifiers only.
        let d = Dbc::parse("BO_ 2147484160 A: 8 X\nBO_ 2147484161 B: 8 X\n").unwrap();
        assert!(!d.j1939);
        assert!(d.decode(&CanFrame::with_ext(0x1_0200, true, &[0; 8]).unwrap()).is_none());
    }

    #[test]
    fn large_files_parse_quickly() {
        use std::fmt::Write;
        let mut t = String::from("VERSION \"\"\nNS_ :\n\tCM_\nBS_:\nBU_: A B\n");
        for i in 0..10_000u32 {
            let _ = writeln!(t, "BO_ {} M{i}: 8 A", i | 0x8000_0000);
            for j in 0..8 {
                let _ = writeln!(t, " SG_ S{j} : {}|8@1+ (0.5,0) [0|127.5] \"u\" B", j * 8);
            }
        }
        for i in 0..10_000u32 {
            let _ = writeln!(t, "CM_ BO_ {} \"message {i}\";", i | 0x8000_0000);
            for j in 0..8 {
                let _ = writeln!(t, "CM_ SG_ {} S{j} \"signal {j}\";", i | 0x8000_0000);
                let _ = writeln!(t, "VAL_ {} S{j} 0 \"off\" 1 \"on\";", i | 0x8000_0000);
            }
            let _ = writeln!(t, "BA_ \"GenMsgCycleTime\" BO_ {} 100;", i | 0x8000_0000);
        }
        let t0 = std::time::Instant::now();
        let d = Dbc::parse(&t).unwrap();
        let ms = t0.elapsed().as_millis();
        assert_eq!(d.messages.len(), 10_000);
        assert_eq!(d.messages[9999].signals[7].comment, "signal 7");
        // Timing means nothing in an unoptimized build; optimized, this takes well under a second
        // (the bound is loose for loaded CI machines).
        if !cfg!(debug_assertions) {
            assert!(ms < 2_000, "{ms} ms");
        }
    }
}
