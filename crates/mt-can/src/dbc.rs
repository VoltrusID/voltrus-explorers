//! DBC files: messages, signals (Intel and Motorola byte order, signed, IEEE float, multiplexed),
//! value tables, comments and cycle times. Decoding turns a frame into physical values; encoding
//! builds a frame's data from them.

use std::collections::HashMap;

use serde::Serialize;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum Multiplex {
    /// The multiplexor switch (`M`).
    Switch,
    /// Present only when the switch has this value (`m3`).
    Value(u64),
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
    pub comment: String,
    /// Raw value to label.
    pub values: Vec<(i64, String)>,
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
    pub signals: Vec<Signal>,
    pub comment: String,
    /// `GenMsgCycleTime`, when the file has one.
    pub cycle_ms: Option<u32>,
}

impl Message {
    pub fn key(&self) -> u32 {
        frame_key(self.id, self.extended)
    }

    pub fn signal(&self, name: &str) -> Option<&Signal> {
        self.signals.iter().find(|s| s.name == name)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dbc {
    pub version: String,
    pub nodes: Vec<String>,
    pub messages: Vec<Message>,
    /// Statements the parser skipped, kept for the log.
    pub skipped: usize,
    #[serde(skip)]
    by_key: HashMap<u32, usize>,
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
}

impl Dbc {
    pub fn parse(text: &str) -> Result<Dbc, CanError> {
        Parser::new(text).parse()
    }

    pub fn message(&self, id: u32, extended: bool) -> Option<&Message> {
        self.by_key.get(&frame_key(id, extended)).map(|&i| &self.messages[i])
    }

    pub fn message_by_name(&self, name: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.name == name)
    }

    pub fn signal_count(&self) -> usize {
        self.messages.iter().map(|m| m.signals.len()).sum()
    }

    /// The message and signal values carried by a frame, or `None` when the file does not describe it.
    pub fn decode(&self, frame: &CanFrame) -> Option<(&Message, Vec<SignalValue>)> {
        if frame.rtr {
            return None;
        }
        let msg = self.message(frame.id, frame.extended)?;
        Some((msg, decode_message(msg, &frame.data)))
    }

    fn index(&mut self) {
        self.by_key = self.messages.iter().enumerate().map(|(i, m)| (m.key(), i)).collect();
    }
}

/// Decodes every signal present in `data`. Signals that do not fit in the data, and multiplexed
/// signals whose switch value does not match, are left out.
pub fn decode_message(msg: &Message, data: &[u8]) -> Vec<SignalValue> {
    let switch = msg
        .signals
        .iter()
        .find(|s| s.multiplex == Some(Multiplex::Switch))
        .and_then(|s| extract(data, s));
    msg.signals
        .iter()
        .filter(|s| match &s.multiplex {
            Some(Multiplex::Value(v)) => switch == Some(*v),
            _ => true,
        })
        .filter_map(|s| decode_signal(s, data))
        .collect()
}

pub fn decode_signal(s: &Signal, data: &[u8]) -> Option<SignalValue> {
    let bits = extract(data, s)?;
    let raw = match s.value_type {
        ValueType::Unsigned => bits as f64,
        ValueType::Signed => sign_extend(bits, s.length) as f64,
        ValueType::Float => f32::from_bits(bits as u32) as f64,
        ValueType::Double => f64::from_bits(bits),
    };
    let label = match s.value_type {
        ValueType::Unsigned | ValueType::Signed => {
            let r = if s.value_type == ValueType::Signed { sign_extend(bits, s.length) } else { bits as i64 };
            s.values.iter().find(|(v, _)| *v == r).map(|(_, l)| l.clone())
        }
        _ => None,
    };
    Some(SignalValue { name: s.name.clone(), raw, value: raw * s.factor + s.offset, unit: s.unit.clone(), label })
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
fn bit_positions(s: &Signal) -> Vec<u32> {
    let len = s.length as u32;
    let start = s.start_bit as u32;
    match s.byte_order {
        // Intel: start is the LSB, bits go up.
        ByteOrder::Intel => (0..len).rev().map(|i| start + i).collect(),
        // Motorola: start is the MSB; walk down within a byte, then to bit 7 of the next byte.
        ByteOrder::Motorola => {
            let mut out = Vec::with_capacity(len as usize);
            let mut p = start;
            for _ in 0..len {
                out.push(p);
                if p.is_multiple_of(8) { p += 15 } else { p -= 1 }
            }
            out
        }
    }
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
    let positions = bit_positions(s);
    let n = positions.len();
    for (i, p) in positions.into_iter().enumerate() {
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
            scaled.round().clamp(0.0, max) as u64
        }
        ValueType::Signed => {
            let half = if s.length >= 64 { i64::MAX as f64 } else { (1i128 << (s.length - 1)) as f64 };
            let r = scaled.round().clamp(-half, half - 1.0) as i64;
            if s.length >= 64 { r as u64 } else { (r as u64) & ((1u64 << s.length) - 1) }
        }
    }
}

/// Builds the data of `msg` from physical values. Signals not named keep their raw value from `base`
/// (or zero). Multiplexed signals are only written when the switch value selects them.
pub fn encode_message(msg: &Message, values: &HashMap<String, f64>, base: Option<&[u8]>) -> Result<Vec<u8>, CanError> {
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
    let switch_sig = msg.signals.iter().find(|s| s.multiplex == Some(Multiplex::Switch));
    if let Some(sw) = switch_sig
        && let Some(v) = values.get(&sw.name) {
            insert(&mut data, sw, raw_for(sw, *v)).map_err(|_| too_long(msg, sw))?;
        }
    let switch = switch_sig.and_then(|s| extract(&data, s));
    for s in &msg.signals {
        if s.multiplex == Some(Multiplex::Switch) {
            continue;
        }
        let Some(v) = values.get(&s.name) else { continue };
        if let Some(Multiplex::Value(m)) = &s.multiplex
            && switch != Some(*m) {
                continue;
            }
        insert(&mut data, s, raw_for(s, *v)).map_err(|_| too_long(msg, s))?;
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

// ---- parser -------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(String),
    Str(String),
    Punct(char),
}

struct Token {
    tok: Tok,
    line: usize,
}

fn tokenize(text: &str) -> Result<Vec<Token>, CanError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '"' {
            let start_line = line;
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err(CanError::Dbc { line: start_line, msg: "unterminated string".into() }),
                    Some('\\') if chars.get(i + 1) == Some(&'"') => {
                        s.push('"');
                        i += 2;
                    }
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some(&ch) => {
                        if ch == '\n' {
                            line += 1;
                        }
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Token { tok: Tok::Str(s), line: start_line });
        } else if c.is_ascii_digit()
            || ((c == '-' || c == '+' || c == '.') && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == '.'))
        {
            let mut s = String::new();
            s.push(c);
            i += 1;
            while let Some(&n) = chars.get(i) {
                let prev = s.chars().last().unwrap_or(' ');
                if n.is_ascii_digit() || n == '.' || n == 'e' || n == 'E' || ((n == '-' || n == '+') && (prev == 'e' || prev == 'E')) {
                    s.push(n);
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token { tok: Tok::Num(s), line });
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(&n) = chars.get(i) {
                if n.is_ascii_alphanumeric() || n == '_' {
                    s.push(n);
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token { tok: Tok::Ident(s), line });
        } else {
            out.push(Token { tok: Tok::Punct(c), line });
            i += 1;
        }
    }
    Ok(out)
}

/// Drops the `NS_ :` block (a list of keywords, one per indented line), which would otherwise look
/// like statements.
fn strip_ns_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ns = false;
    for l in text.split_inclusive('\n') {
        if l.starts_with("NS_ ") || l.starts_with("NS_:") || l.trim_end() == "NS_" {
            in_ns = true;
            out.push('\n');
            continue;
        }
        if in_ns {
            let indented = l.starts_with(' ') || l.starts_with('\t') || l.trim().is_empty();
            if indented {
                out.push('\n');
                continue;
            }
            in_ns = false;
        }
        out.push_str(l);
    }
    out
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    text: String,
}

impl Parser {
    fn new(text: &str) -> Self {
        let text = strip_ns_block(text);
        Parser { toks: Vec::new(), pos: 0, text }
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).or_else(|| self.toks.last()).map(|t| t.line).unwrap_or(1)
    }

    fn fail<T>(&self, msg: impl Into<String>) -> Result<T, CanError> {
        Err(CanError::Dbc { line: self.line(), msg: msg.into() })
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.tok.clone());
        self.pos += 1;
        t
    }

    fn punct(&mut self, c: char) -> Result<(), CanError> {
        match self.next() {
            Some(Tok::Punct(p)) if p == c => Ok(()),
            other => {
                self.pos -= 1;
                self.fail(format!("expected '{c}', found {}", show(other.as_ref())))
            }
        }
    }

    fn ident(&mut self) -> Result<String, CanError> {
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            other => {
                self.pos -= 1;
                self.fail(format!("expected a name, found {}", show(other.as_ref())))
            }
        }
    }

    fn string(&mut self) -> Result<String, CanError> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(s),
            other => {
                self.pos -= 1;
                self.fail(format!("expected a quoted string, found {}", show(other.as_ref())))
            }
        }
    }

    fn number(&mut self) -> Result<f64, CanError> {
        match self.next() {
            Some(Tok::Num(s)) => match s.parse::<f64>() {
                Ok(v) => Ok(v),
                Err(_) => {
                    self.pos -= 1;
                    self.fail(format!("bad number {s}"))
                }
            },
            other => {
                self.pos -= 1;
                self.fail(format!("expected a number, found {}", show(other.as_ref())))
            }
        }
    }

    fn integer(&mut self) -> Result<u64, CanError> {
        match self.next() {
            Some(Tok::Num(s)) => match s.parse::<u64>() {
                Ok(v) => Ok(v),
                Err(_) => {
                    self.pos -= 1;
                    self.fail(format!("expected a whole number, found {s}"))
                }
            },
            other => {
                self.pos -= 1;
                self.fail(format!("expected a whole number, found {}", show(other.as_ref())))
            }
        }
    }

    fn signed_integer(&mut self) -> Result<i64, CanError> {
        match self.next() {
            Some(Tok::Num(s)) => match s.parse::<i64>().or_else(|_| s.parse::<f64>().map(|f| f as i64)) {
                Ok(v) => Ok(v),
                Err(_) => {
                    self.pos -= 1;
                    self.fail(format!("expected a whole number, found {s}"))
                }
            },
            other => {
                self.pos -= 1;
                self.fail(format!("expected a whole number, found {}", show(other.as_ref())))
            }
        }
    }

    /// Skips to just after the next `;`.
    fn skip_statement(&mut self) {
        while let Some(t) = self.next() {
            if t == Tok::Punct(';') {
                return;
            }
        }
    }

    /// Skips the rest of the current line.
    fn skip_line(&mut self, line: usize) {
        while self.toks.get(self.pos).is_some_and(|t| t.line == line) {
            self.pos += 1;
        }
    }

    fn parse(mut self) -> Result<Dbc, CanError> {
        self.toks = tokenize(&self.text)?;
        let mut dbc = Dbc::default();
        let mut comments: Vec<(u32, Option<String>, String)> = Vec::new();
        #[allow(clippy::type_complexity)]
        let mut tables: Vec<(u32, String, Vec<(i64, String)>)> = Vec::new();
        let mut valtypes: Vec<(u32, String, u64)> = Vec::new();
        let mut cycles: Vec<(u32, u32)> = Vec::new();
        let mut current: Option<usize> = None;

        while let Some(tok) = self.peek().cloned() {
            let line = self.line();
            let Tok::Ident(kw) = tok else {
                // Stray punctuation or numbers between statements.
                self.pos += 1;
                continue;
            };
            match kw.as_str() {
                "VERSION" => {
                    self.pos += 1;
                    dbc.version = self.string()?;
                }
                "BU_" => {
                    self.pos += 1;
                    self.punct(':')?;
                    while let Some(t) = self.toks.get(self.pos) {
                        if t.line != line {
                            break;
                        }
                        if let Tok::Ident(n) = &t.tok {
                            dbc.nodes.push(n.clone());
                        }
                        self.pos += 1;
                    }
                }
                "BO_" => {
                    self.pos += 1;
                    let raw = self.integer()?;
                    let name = self.ident()?;
                    self.punct(':')?;
                    let size = self.integer()?;
                    let transmitter = match self.peek() {
                        Some(Tok::Ident(_)) if self.toks[self.pos].line == line => self.ident()?,
                        _ => String::new(),
                    };
                    let extended = raw & 0x8000_0000 != 0;
                    let id = (raw & 0x1FFF_FFFF) as u32;
                    if size > 64 {
                        return Err(CanError::Dbc { line, msg: format!("message {name} is {size} bytes long (64 at most)") });
                    }
                    dbc.messages.push(Message {
                        id,
                        extended,
                        name,
                        size: size as u16,
                        transmitter,
                        signals: Vec::new(),
                        comment: String::new(),
                        cycle_ms: None,
                    });
                    current = Some(dbc.messages.len() - 1);
                }
                "SG_" => {
                    self.pos += 1;
                    let sig = self.signal(line)?;
                    let Some(m) = current else {
                        return Err(CanError::Dbc { line, msg: format!("signal {} outside a message", sig.name) });
                    };
                    dbc.messages[m].signals.push(sig);
                }
                "CM_" => {
                    self.pos += 1;
                    match self.peek().cloned() {
                        Some(Tok::Ident(k)) if k == "BO_" => {
                            self.pos += 1;
                            let id = self.integer()? as u32;
                            let text = self.string()?;
                            comments.push((id, None, text));
                            self.punct(';')?;
                        }
                        Some(Tok::Ident(k)) if k == "SG_" => {
                            self.pos += 1;
                            let id = self.integer()? as u32;
                            let sig = self.ident()?;
                            let text = self.string()?;
                            comments.push((id, Some(sig), text));
                            self.punct(';')?;
                        }
                        _ => self.skip_statement(),
                    }
                }
                "VAL_" => {
                    self.pos += 1;
                    if !matches!(self.peek(), Some(Tok::Num(_))) {
                        // `VAL_ <env var> ...`: environment variables are not used here.
                        self.skip_statement();
                        continue;
                    }
                    let id = self.integer()? as u32;
                    let sig = self.ident()?;
                    let mut entries = Vec::new();
                    while !matches!(self.peek(), Some(Tok::Punct(';')) | None) {
                        let v = self.signed_integer()?;
                        let l = self.string()?;
                        entries.push((v, l));
                    }
                    self.punct(';')?;
                    tables.push((id, sig, entries));
                }
                "SIG_VALTYPE_" => {
                    self.pos += 1;
                    let id = self.integer()? as u32;
                    let sig = self.ident()?;
                    if matches!(self.peek(), Some(Tok::Punct(':'))) {
                        self.pos += 1;
                    }
                    let t = self.integer()?;
                    self.punct(';')?;
                    valtypes.push((id, sig, t));
                }
                "BA_" => {
                    self.pos += 1;
                    let attr = self.string()?;
                    let on_msg = matches!(self.peek(), Some(Tok::Ident(k)) if k == "BO_");
                    if attr == "GenMsgCycleTime" && on_msg {
                        self.pos += 1;
                        let id = self.integer()? as u32;
                        let v = self.number()?;
                        self.punct(';')?;
                        if v > 0.0 {
                            cycles.push((id, v as u32));
                        }
                    } else {
                        self.skip_statement();
                    }
                }
                "BS_" => {
                    self.pos += 1;
                    self.skip_line(line);
                }
                "VERSION_" | "NS_" => self.skip_line(line),
                k if k.ends_with('_') => {
                    // BA_DEF_, BA_DEF_DEF_, VAL_TABLE_, EV_, BO_TX_BU_, SG_MUL_VAL_, SIG_GROUP_, ...
                    self.pos += 1;
                    dbc.skipped += 1;
                    self.skip_statement();
                }
                _ => {
                    dbc.skipped += 1;
                    self.pos += 1;
                    self.skip_line(line);
                }
            }
        }

        let find = |dbc: &mut Dbc, raw: u32| -> Option<usize> {
            let extended = raw & 0x8000_0000 != 0;
            let id = raw & 0x1FFF_FFFF;
            dbc.messages.iter().position(|m| m.id == id && m.extended == extended)
        };
        for (raw, sig, text) in comments {
            if let Some(m) = find(&mut dbc, raw) {
                match sig {
                    None => dbc.messages[m].comment = text,
                    Some(s) => {
                        if let Some(sg) = dbc.messages[m].signals.iter_mut().find(|x| x.name == s) {
                            sg.comment = text;
                        }
                    }
                }
            }
        }
        for (raw, sig, entries) in tables {
            if let Some(m) = find(&mut dbc, raw)
                && let Some(sg) = dbc.messages[m].signals.iter_mut().find(|x| x.name == sig) {
                    sg.values = entries;
                }
        }
        for (raw, sig, t) in valtypes {
            if let Some(m) = find(&mut dbc, raw)
                && let Some(sg) = dbc.messages[m].signals.iter_mut().find(|x| x.name == sig) {
                    sg.value_type = match t {
                        1 => ValueType::Float,
                        2 => ValueType::Double,
                        _ => sg.value_type,
                    };
                }
        }
        for (raw, ms) in cycles {
            if let Some(m) = find(&mut dbc, raw) {
                dbc.messages[m].cycle_ms = Some(ms);
            }
        }
        dbc.index();
        Ok(dbc)
    }

    fn signal(&mut self, line: usize) -> Result<Signal, CanError> {
        let name = self.ident()?;
        let mut multiplex = None;
        if let Some(Tok::Ident(m)) = self.peek().cloned() {
            self.pos += 1;
            multiplex = if m == "M" {
                Some(Multiplex::Switch)
            } else if let Some(rest) = m.strip_prefix('m') {
                // `m3` or `m3M` (extended multiplexing: both multiplexed and a switch; treated as `m3`).
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                match digits.parse::<u64>() {
                    Ok(v) => Some(Multiplex::Value(v)),
                    Err(_) => return self.fail(format!("bad multiplex indicator {m}")),
                }
            } else {
                return self.fail(format!("bad multiplex indicator {m}"));
            };
        }
        self.punct(':')?;
        let start = self.integer()?;
        self.punct('|')?;
        let length = self.integer()?;
        self.punct('@')?;
        // `1+` tokenizes as the number `1` and `+`; `0-` as `0` and `-`.
        let order = self.integer()?;
        let sign = match self.next() {
            Some(Tok::Punct('+')) => ValueType::Unsigned,
            Some(Tok::Punct('-')) => ValueType::Signed,
            other => {
                self.pos -= 1;
                return self.fail(format!("expected + or -, found {}", show(other.as_ref())));
            }
        };
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
        while let Some(t) = self.toks.get(self.pos) {
            if t.line != line {
                break;
            }
            match &t.tok {
                Tok::Ident(r) => receivers.push(r.clone()),
                Tok::Punct(',') => {}
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
            comment: String::new(),
            values: Vec::new(),
        })
    }
}

fn show(t: Option<&Tok>) -> String {
    match t {
        None => "the end of the file".into(),
        Some(Tok::Ident(s)) => s.clone(),
        Some(Tok::Num(s)) => s.clone(),
        Some(Tok::Str(s)) => format!("\"{s}\""),
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

    #[test]
    fn parses_structure() {
        let d = dbc();
        assert_eq!(d.version, "1.0");
        assert_eq!(d.nodes, vec!["ECU", "Tester"]);
        assert_eq!(d.messages.len(), 4);
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
        let names: Vec<&str> = v.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Page", "Odometer"]);
        assert_eq!(v[1].value, 1000.0);
        let (_, v) = d.decode(&CanFrame::new(1232, &[1, 150, 0, 0, 0, 0, 0, 0]).unwrap()).unwrap();
        assert_eq!(v.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Page", "Fuel"]);
        assert_eq!(v[1].value, 75.0);
    }

    #[test]
    fn encodes_round_trip() {
        let d = dbc();
        let m = d.message_by_name("Engine").unwrap();
        let vals: HashMap<String, f64> =
            [("Rpm", 3500.25), ("Coolant", -12.0), ("Torque", -300.5), ("Running", 1.0)].iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let data = encode_message(m, &vals, None).unwrap();
        let got = decode_message(m, &data);
        for s in got {
            assert!((s.value - vals[&s.name]).abs() < 1e-9, "{} {}", s.name, s.value);
        }
        let b = d.message_by_name("Battery").unwrap();
        let vals: HashMap<String, f64> = [("Voltage".to_string(), 27.6), ("Current".to_string(), -100.3)].into_iter().collect();
        let data = encode_message(b, &vals, None).unwrap();
        let got = decode_message(b, &data);
        assert!((got[0].value - 27.6).abs() < 1e-9 && (got[1].value + 100.3).abs() < 1e-9);
        // Clamped to the raw range.
        let vals: HashMap<String, f64> = [("Coolant".to_string(), 1000.0)].into_iter().collect();
        let data = encode_message(m, &vals, None).unwrap();
        assert_eq!(data[2], 255);
        // Multiplexed: only the selected page is written.
        let dg = d.message_by_name("Diag").unwrap();
        let vals: HashMap<String, f64> = [("Page".to_string(), 1.0), ("Fuel".to_string(), 40.0), ("Odometer".to_string(), 5.0)].into_iter().collect();
        let data = encode_message(dg, &vals, None).unwrap();
        assert_eq!(&data[..2], &[1, 80]);
        assert!(encode_message(m, &[("Nope".to_string(), 1.0)].into_iter().collect(), None).is_err());
    }

    #[test]
    fn reports_errors_with_lines() {
        let e = Dbc::parse("BO_ 1 A: 8 X\n SG_ S : 0|8@1+ (1,0 [0|1] \"\" X\n").unwrap_err();
        assert!(matches!(e, CanError::Dbc { line: 2, .. }), "{e}");
        assert!(Dbc::parse(" SG_ S : 0|8@1+ (1,0) [0|1] \"\" X\n").is_err());
        assert!(Dbc::parse("CM_ \"open").is_err());
    }
}
