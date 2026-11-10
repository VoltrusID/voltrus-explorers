//! Modbus RTU on top of the UART decoder.
//!
//! Frames are split on silence: more than 3.5 character times between characters (a fixed
//! 1.75 ms above 19200 baud, as the Modbus serial line specification sets). A gap above 1.5
//! characters inside a frame is a spec violation and is flagged but kept. When a "frame" fails its
//! CRC, the decoder also tries to split it where a prefix has a valid CRC, because many devices
//! answer faster than 3.5 characters after a request.
//!
//! The request channel (the master's TX, or the RS-485 bus seen through one receiver) and the
//! response channel (the slave's TX) can be separate; with one shared channel, frames are told
//! apart by pairing: after a request, the next frame from the same slave with the same function
//! (or its exception) is the response.

use serde::{Deserialize, Serialize};

use super::uart::{Parity, UartConfig, decode as uart_decode};
use super::{Annotation, DecodeOutput, Level, Table, TableRow, hex, time_cell};
use crate::capture::{Capture, fmt_time};

/// CRC-16/MODBUS (poly 0xA001 reflected, init 0xFFFF). Sent low byte first.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModbusConfig {
    /// Master to slave (requests). With one channel only, set this one and leave the other empty.
    pub request_channel: Option<u8>,
    /// Slave to master (responses).
    pub response_channel: Option<u8>,
    pub baud: f64,
    pub parity: Parity,
    pub stop_bits: f32,
    #[serde(default)]
    pub inverted: bool,
    /// Response timeout for "no response" (ms).
    #[serde(default = "default_timeout")]
    pub timeout_ms: f64,
}

fn default_timeout() -> f64 {
    1000.0
}

impl Default for ModbusConfig {
    fn default() -> Self {
        Self { request_channel: Some(0), response_channel: None, baud: 19200.0, parity: Parity::Even, stop_bits: 1.0, inverted: false, timeout_ms: 1000.0 }
    }
}

impl ModbusConfig {
    pub fn uart(&self, ch: u8) -> UartConfig {
        UartConfig { channel: ch, baud: self.baud, data_bits: 8, parity: self.parity, stop_bits: self.stop_bits, inverted: self.inverted, msb_first: false }
    }

    pub fn char_time(&self) -> f64 {
        self.uart(0).char_bits() / self.baud
    }

    /// Frame separation silence (t3.5), seconds.
    pub fn t35(&self) -> f64 {
        if self.baud > 19200.0 { 0.00175 } else { 3.5 * self.char_time() }
    }

    /// Longest allowed gap inside a frame (t1.5), seconds.
    pub fn t15(&self) -> f64 {
        if self.baud > 19200.0 { 0.00075 } else { 1.5 * self.char_time() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Request,
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawFrame {
    pub start: u64,
    pub end: u64,
    pub bytes: Vec<u8>,
    /// Start and end sample of every byte.
    pub spans: Vec<(u64, u64)>,
    /// Which channel it came from; `None` on a shared channel.
    pub role: Option<Role>,
    /// A gap over 1.5 characters inside the frame.
    pub t15_violation: bool,
    /// UART parity or framing errors inside the frame.
    pub uart_errors: usize,
}

impl RawFrame {
    pub fn crc_ok(&self) -> bool {
        self.bytes.len() >= 4 && {
            let n = self.bytes.len();
            crc16(&self.bytes[..n - 2]) == u16::from_le_bytes([self.bytes[n - 2], self.bytes[n - 1]])
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub raw: RawFrame,
    pub role: Role,
    pub address: u8,
    pub function: u8,
    pub crc_ok: bool,
    /// What it means: "Read holding registers 0–9 (40001–40010)", "Exception 02 Illegal data address".
    pub detail: String,
    /// Values carried, e.g. registers `[230, 0, 1500]` or coils as 0/1.
    pub values: Vec<u16>,
    pub exception: Option<u8>,
    /// Length doesn't fit the function code.
    pub malformed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub request: Option<usize>,
    pub response: Option<usize>,
    /// From the end of the request to the start of the response, seconds.
    pub response_time: Option<f64>,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Status {
    Ok,
    Exception { code: u8 },
    CrcError,
    NoResponse,
    Broadcast,
    /// A response with no request in the capture (e.g. the capture starts mid-exchange).
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct ModbusResult {
    pub frames: Vec<Frame>,
    pub transactions: Vec<Transaction>,
}

pub fn function_name(fc: u8) -> &'static str {
    match fc & 0x7F {
        1 => "Read coils",
        2 => "Read discrete inputs",
        3 => "Read holding registers",
        4 => "Read input registers",
        5 => "Write single coil",
        6 => "Write single register",
        7 => "Read exception status",
        8 => "Diagnostics",
        11 => "Get comm event counter",
        12 => "Get comm event log",
        15 => "Write multiple coils",
        16 => "Write multiple registers",
        17 => "Report server ID",
        20 => "Read file record",
        21 => "Write file record",
        22 => "Mask write register",
        23 => "Read/write multiple registers",
        24 => "Read FIFO queue",
        43 => "Encapsulated interface (MEI)",
        _ => "Unknown function",
    }
}

pub fn exception_name(code: u8) -> &'static str {
    match code {
        1 => "Illegal function",
        2 => "Illegal data address",
        3 => "Illegal data value",
        4 => "Server device failure",
        5 => "Acknowledge",
        6 => "Server device busy",
        8 => "Memory parity error",
        10 => "Gateway path unavailable",
        11 => "Gateway target failed to respond",
        _ => "Unknown exception",
    }
}

/// Classic 5-digit reference of a zero-based address for a table (0xxxx coils, 1xxxx inputs,
/// 3xxxx input registers, 4xxxx holding registers).
fn reference(fc: u8, addr: u16) -> Option<u32> {
    let base = match fc {
        1 | 5 | 15 => 1,
        2 => 10_001,
        4 => 30_001,
        3 | 6 | 16 | 22 | 23 => 40_001,
        _ => return None,
    };
    Some(base + addr as u32)
}

fn range_text(fc: u8, start: u16, qty: u16) -> String {
    let last = start as u32 + qty.max(1) as u32 - 1;
    let r = match (reference(fc, start), reference(fc, last.min(0xFFFF) as u16)) {
        (Some(a), Some(b)) if qty > 1 => format!(" ({a:05}–{b:05})"),
        (Some(a), _) => format!(" ({a:05})"),
        _ => String::new(),
    };
    if qty > 1 { format!("{start}–{last}{r}") } else { format!("{start}{r}") }
}

fn be16(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn regs(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect()
}

fn bits(b: &[u8], n: usize) -> Vec<u16> {
    (0..n.min(b.len() * 8)).map(|i| (b[i / 8] >> (i % 8) & 1) as u16).collect()
}

/// Interprets a PDU (function + data, no address or CRC) as a request.
/// Returns `None` when the length doesn't fit a request of that function.
fn parse_request(fc: u8, d: &[u8]) -> Option<(String, Vec<u16>)> {
    let name = function_name(fc);
    Some(match fc {
        1..=4 if d.len() == 4 => {
            let (a, q) = (be16(d, 0), be16(d, 2));
            (format!("{name} {} ×{q}", range_text(fc, a, q)), vec![])
        }
        5 if d.len() == 4 => {
            let v = be16(d, 2);
            let s = match v {
                0xFF00 => "ON",
                0x0000 => "OFF",
                _ => "invalid value",
            };
            (format!("{name} {} = {s}", range_text(fc, be16(d, 0), 1)), vec![u16::from(v == 0xFF00)])
        }
        6 if d.len() == 4 => {
            let v = be16(d, 2);
            (format!("{name} {} = {v}", range_text(fc, be16(d, 0), 1)), vec![v])
        }
        15 if d.len() >= 5 && d.len() == 5 + d[4] as usize => {
            let (a, q) = (be16(d, 0), be16(d, 2));
            let v = bits(&d[5..], q as usize);
            (format!("{name} {} ×{q}", range_text(fc, a, q)), v)
        }
        16 if d.len() >= 5 && d.len() == 5 + d[4] as usize => {
            let (a, q) = (be16(d, 0), be16(d, 2));
            (format!("{name} {} ×{q}", range_text(fc, a, q)), regs(&d[5..]))
        }
        22 if d.len() == 6 => (format!("{name} {} AND {:04X} OR {:04X}", range_text(fc, be16(d, 0), 1), be16(d, 2), be16(d, 4)), vec![]),
        23 if d.len() >= 9 && d.len() == 9 + d[8] as usize => {
            let (ra, rq, wa, wq) = (be16(d, 0), be16(d, 2), be16(d, 4), be16(d, 6));
            (format!("{name}: read {} ×{rq}, write {} ×{wq}", range_text(3, ra, rq), range_text(3, wa, wq)), regs(&d[9..]))
        }
        7 | 11 | 12 | 17 if d.is_empty() => (name.to_string(), vec![]),
        8 if d.len() >= 2 => (format!("{name} sub-function {}", be16(d, 0)), regs(&d[2..])),
        43 if !d.is_empty() => (format!("{name} type {}", d[0]), vec![]),
        _ if !matches!(fc, 1..=8 | 11 | 12 | 15..=17 | 22 | 23 | 43) && fc < 0x80 => (format!("Function {fc} ({} data bytes)", d.len()), vec![]),
        _ => return None,
    })
}

fn parse_response(fc: u8, d: &[u8], req: Option<&Frame>) -> Option<(String, Vec<u16>, Option<u8>)> {
    if fc & 0x80 != 0 {
        return (d.len() == 1).then(|| (format!("Exception {:02X} {}", d[0], exception_name(d[0])), vec![], Some(d[0])));
    }
    let name = function_name(fc);
    Some(match fc {
        1 | 2 if !d.is_empty() && d.len() == 1 + d[0] as usize => {
            let n = req.and_then(|r| r.raw.bytes.get(4..6).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize)).unwrap_or(d[0] as usize * 8);
            let v = bits(&d[1..], n);
            (format!("{name}: {} bits", v.len()), v, None)
        }
        3 | 4 | 23 if !d.is_empty() && d.len() == 1 + d[0] as usize && d[0].is_multiple_of(2) => {
            let v = regs(&d[1..]);
            (format!("{name}: {} registers", v.len()), v, None)
        }
        5 | 6 if d.len() == 4 => {
            let v = be16(d, 2);
            (format!("{name} {} = {} (echo)", range_text(fc, be16(d, 0), 1), if fc == 5 { if v == 0xFF00 { "ON".into() } else { "OFF".into() } } else { v.to_string() }), vec![v], None)
        }
        15 | 16 if d.len() == 4 => (format!("{name} {} ×{} done", range_text(fc, be16(d, 0), be16(d, 2)), be16(d, 2)), vec![], None),
        22 if d.len() == 6 => (format!("{name} done"), vec![], None),
        7 if d.len() == 1 => (format!("{name}: {:08b}", d[0]), vec![d[0] as u16], None),
        8 if d.len() >= 2 => (format!("{name} sub-function {} (echo)", be16(d, 0)), regs(&d[2..]), None),
        11 if d.len() == 4 => (format!("{name}: status {:04X}, count {}", be16(d, 0), be16(d, 2)), vec![be16(d, 2)], None),
        12 | 17 | 20 | 21 | 24 | 43 if !d.is_empty() => (format!("{name} ({} bytes)", d.len()), vec![], None),
        _ if !matches!(fc, 1..=8 | 11 | 12 | 15..=17 | 20..=24 | 43) => (format!("Function {fc} ({} data bytes)", d.len()), vec![], None),
        _ => return None,
    })
}

/// Splits the characters of one channel into frames on silence.
pub fn raw_frames(cap: &Capture, cfg: &ModbusConfig, ch: u8, role: Option<Role>) -> Vec<RawFrame> {
    let r = uart_decode(cap, &cfg.uart(ch));
    let rate = cap.sample_rate as f64;
    let t35 = (cfg.t35() * rate) as u64;
    let t15 = (cfg.t15() * rate) as u64;
    let mut frames = Vec::new();
    let mut cur: Option<RawFrame> = None;
    for c in r.chars.iter().filter(|c| !c.brk) {
        let err = usize::from(c.framing_error || c.parity_ok == Some(false));
        if let Some(f) = cur.as_mut() {
            let gap = c.start.saturating_sub(f.end);
            if gap <= t35 {
                if gap > t15 {
                    f.t15_violation = true;
                }
                f.bytes.push(c.value as u8);
                f.spans.push((c.start, c.end));
                f.end = c.end;
                f.uart_errors += err;
                continue;
            }
            frames.push(cur.take().unwrap());
        }
        cur = Some(RawFrame { start: c.start, end: c.end, bytes: vec![c.value as u8], spans: vec![(c.start, c.end)], role, t15_violation: false, uart_errors: err });
    }
    frames.extend(cur);
    // Split frames that only check out as two back-to-back frames (no 3.5-char gap between them).
    let mut out = Vec::new();
    for f in frames {
        split_on_crc(f, &mut out);
    }
    out
}

fn split_on_crc(f: RawFrame, out: &mut Vec<RawFrame>) {
    if f.crc_ok() || f.bytes.len() < 8 {
        out.push(f);
        return;
    }
    for n in 4..=f.bytes.len() - 4 {
        let head = &f.bytes[..n];
        if crc16(&head[..n - 2]) == u16::from_le_bytes([head[n - 2], head[n - 1]]) && head[0] == f.bytes[n] {
            let tail = RawFrame {
                start: f.spans[n].0,
                end: f.end,
                bytes: f.bytes[n..].to_vec(),
                spans: f.spans[n..].to_vec(),
                role: f.role,
                t15_violation: f.t15_violation,
                uart_errors: 0,
            };
            out.push(RawFrame { start: f.start, end: f.spans[n - 1].1, bytes: head.to_vec(), spans: f.spans[..n].to_vec(), role: f.role, t15_violation: f.t15_violation, uart_errors: f.uart_errors });
            split_on_crc(tail, out);
            return;
        }
    }
    out.push(f);
}

pub fn decode(cap: &Capture, cfg: &ModbusConfig) -> Result<ModbusResult, String> {
    let (req_ch, resp_ch) = match (cfg.request_channel, cfg.response_channel) {
        (None, None) => return Err("Modbus RTU needs at least one channel".into()),
        (None, Some(r)) => (Some(r), None),
        (a, b) => (a, b),
    };
    cfg.uart(req_ch.unwrap_or(0)).validate(cap)?;
    let two = req_ch.is_some() && resp_ch.is_some() && req_ch != resp_ch;
    let mut raws = Vec::new();
    if let Some(c) = req_ch {
        raws.extend(raw_frames(cap, cfg, c, two.then_some(Role::Request)));
    }
    if two && let Some(c) = resp_ch {
        raws.extend(raw_frames(cap, cfg, c, Some(Role::Response)));
    }
    raws.sort_by_key(|f| f.start);
    Ok(interpret(cap, cfg, raws))
}

fn interpret(cap: &Capture, cfg: &ModbusConfig, raws: Vec<RawFrame>) -> ModbusResult {
    let mut res = ModbusResult::default();
    // Index of the request waiting for its response.
    let mut pending: Option<usize> = None;
    let timeout = (cfg.timeout_ms / 1000.0 * cap.sample_rate as f64) as u64;
    for raw in raws {
        let crc_ok = raw.crc_ok();
        let b = &raw.bytes;
        if b.len() < 2 {
            // A lone character: noise or a cut frame.
            let f = Frame { role: raw.role.unwrap_or(Role::Request), address: b[0], function: 0, crc_ok: false, detail: "Too short for a frame".into(), values: vec![], exception: None, malformed: true, raw };
            res.frames.push(f);
            continue;
        }
        let (addr, fc) = (b[0], b[1]);
        let pdu = if b.len() >= 4 { &b[2..b.len() - 2] } else { &b[2..] };
        // Close a request that timed out before this frame.
        if let Some(pi) = pending
            && raw.start.saturating_sub(res.frames[pi].raw.end) > timeout
        {
            close_no_response(&mut res, pi);
            pending = None;
        }
        let pend_frame = pending.map(|i| res.frames[i].clone());
        let pend = pend_frame.as_ref();
        let fits_pending = pend.is_some_and(|p| p.address == addr && (p.function == fc || p.function | 0x80 == fc));
        let role = match raw.role {
            Some(r) => r,
            None if fits_pending && parse_response(fc, pdu, pend).is_some() => Role::Response,
            None if fits_pending && fc & 0x80 != 0 => Role::Response,
            None => Role::Request,
        };
        let (detail, values, exception, malformed) = match role {
            Role::Request => match parse_request(fc, pdu) {
                Some((d, v)) => (d, v, None, false),
                None => (format!("{} (unexpected length {})", function_name(fc), b.len()), vec![], None, true),
            },
            Role::Response => match parse_response(fc, pdu, pend.filter(|_| fits_pending)) {
                Some((d, v, e)) => (d, v, e, false),
                None => (format!("{} (unexpected length {})", function_name(fc), b.len()), vec![], None, true),
            },
        };
        let detail = if crc_ok { detail } else { format!("{detail} — CRC error") };
        let idx = res.frames.len();
        res.frames.push(Frame { raw, role, address: addr, function: fc, crc_ok, detail, values, exception, malformed });
        match role {
            Role::Request => {
                if let Some(pi) = pending.take() {
                    close_no_response(&mut res, pi);
                }
                if addr == 0 {
                    res.transactions.push(Transaction { request: Some(idx), response: None, response_time: None, status: Status::Broadcast });
                } else if !crc_ok {
                    res.transactions.push(Transaction { request: Some(idx), response: None, response_time: None, status: Status::CrcError });
                } else {
                    pending = Some(idx);
                }
            }
            Role::Response => {
                let req = pending.take().filter(|&pi| {
                    let p = &res.frames[pi];
                    p.address == addr && (p.function == fc || p.function | 0x80 == fc)
                });
                let f = &res.frames[idx];
                let status = if !f.crc_ok {
                    Status::CrcError
                } else if let Some(code) = f.exception {
                    Status::Exception { code }
                } else if req.is_none() {
                    Status::Unmatched
                } else {
                    Status::Ok
                };
                let response_time = req.map(|pi| (f.raw.start.saturating_sub(res.frames[pi].raw.end)) as f64 / cap.sample_rate as f64);
                res.transactions.push(Transaction { request: req, response: Some(idx), response_time, status });
            }
        }
    }
    if let Some(pi) = pending {
        close_no_response(&mut res, pi);
    }
    res.transactions.sort_by_key(|t| t.request.or(t.response).map(|i| res.frames[i].raw.start).unwrap_or(0));
    res
}

fn close_no_response(res: &mut ModbusResult, pi: usize) {
    res.transactions.push(Transaction { request: Some(pi), response: None, response_time: None, status: Status::NoResponse });
}

fn status_text(s: &Status) -> (Level, String) {
    match s {
        Status::Ok => (Level::Data, "OK".into()),
        Status::Exception { code } => (Level::Warning, format!("Exception {code:02X} {}", exception_name(*code))),
        Status::CrcError => (Level::Error, "CRC error".into()),
        Status::NoResponse => (Level::Error, "No response".into()),
        Status::Broadcast => (Level::Info, "Broadcast".into()),
        Status::Unmatched => (Level::Warning, "Response without request".into()),
    }
}

fn values_text(v: &[u16]) -> String {
    const MAX: usize = 16;
    let mut s = v.iter().take(MAX).map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    if v.len() > MAX {
        s.push_str(&format!(", … ({} more)", v.len() - MAX));
    }
    s
}

pub fn output(cap: &Capture, cfg: &ModbusConfig, r: &ModbusResult) -> DecodeOutput {
    let mut ann = Vec::new();
    // Rows: 0 bytes, 1 fields, 2 frames, 3 warnings.
    for f in &r.frames {
        let b = &f.raw.bytes;
        for (i, &(s, e)) in f.raw.spans.iter().enumerate() {
            ann.push(Annotation::new(0, s, e, Level::Data, format!("0x{:02X}", b[i]), format!("{:02X}", b[i])));
        }
        let sp = &f.raw.spans;
        if b.len() >= 4 {
            let n = b.len();
            ann.push(Annotation::new(1, sp[0].0, sp[0].1, Level::Control, format!("Slave {}", f.address), f.address.to_string()));
            ann.push(Annotation::new(1, sp[1].0, sp[1].1, Level::Control, format!("FC {} {}", f.function, function_name(f.function)), format!("FC{}", f.function)));
            if n > 4 {
                ann.push(Annotation::new(1, sp[2].0, sp[n - 3].1, Level::Data, values_or_hex(f), hex(&b[2..n - 2])));
            }
            let crc = u16::from_le_bytes([b[n - 2], b[n - 1]]);
            let (lvl, txt) = if f.crc_ok { (Level::Control, format!("CRC {crc:04X} OK")) } else { (Level::Error, format!("CRC {crc:04X} wrong (expected {:04X})", crc16(&b[..n - 2]))) };
            ann.push(Annotation::new(1, sp[n - 2].0, sp[n - 1].1, lvl, txt, "CRC"));
        }
        let lvl = if !f.crc_ok || f.malformed {
            Level::Error
        } else if f.exception.is_some() {
            Level::Warning
        } else {
            Level::Info
        };
        let dir = if f.role == Role::Request { "→" } else { "←" };
        ann.push(Annotation::new(2, f.raw.start, f.raw.end, lvl, format!("{dir} {} {}", f.address, f.detail), format!("{dir}{}", f.address)));
        if f.raw.t15_violation {
            ann.push(Annotation::new(3, f.raw.start, f.raw.end, Level::Warning, "Gap over 1.5 characters inside the frame", "t1.5"));
        }
        if f.raw.uart_errors > 0 {
            ann.push(Annotation::new(3, f.raw.start, f.raw.end, Level::Error, format!("{} UART parity/framing errors", f.raw.uart_errors), "UART"));
        }
    }

    let mut rows = Vec::new();
    for t in &r.transactions {
        let req = t.request.map(|i| &r.frames[i]);
        let resp = t.response.map(|i| &r.frames[i]);
        let first = req.or(resp).unwrap();
        let start = first.raw.start;
        let end = resp.or(req).unwrap().raw.end;
        let (level, status) = status_text(&t.status);
        let values = resp.filter(|f| !f.values.is_empty()).or(req).map(|f| values_text(&f.values)).unwrap_or_default();
        rows.push(TableRow {
            start,
            end,
            level,
            cells: vec![
                time_cell(cap, start),
                first.address.to_string(),
                format!("{} {}", first.function & 0x7F, function_name(first.function)),
                req.map(|f| f.detail.clone()).unwrap_or_else(|| "—".into()),
                values,
                t.response_time.map(fmt_time).unwrap_or_else(|| "—".into()),
                status,
            ],
        });
    }
    let n_err = r.transactions.iter().filter(|t| matches!(t.status, Status::CrcError | Status::NoResponse)).count();
    let n_exc = r.transactions.iter().filter(|t| matches!(t.status, Status::Exception { .. })).count();
    let times: Vec<f64> = r.transactions.iter().filter_map(|t| t.response_time).collect();
    let avg = if times.is_empty() { String::new() } else { format!(", response {} avg", fmt_time(times.iter().sum::<f64>() / times.len() as f64)) };
    let summary = format!(
        "{} frames, {} transactions at {} baud {}{}{}{}",
        r.frames.len(),
        r.transactions.len(),
        cfg.baud,
        parity_text(cfg),
        avg,
        if n_exc > 0 { format!(", {n_exc} exceptions") } else { String::new() },
        if n_err > 0 { format!(", {n_err} errors") } else { String::new() },
    );
    DecodeOutput {
        rows: vec!["Bytes".into(), "Fields".into(), "Frames".into(), "Warnings".into()],
        annotations: ann,
        table: Some(Table {
            columns: ["Time", "Slave", "Function", "Request", "Values", "Response time", "Status"].map(String::from).to_vec(),
            rows,
        }),
        summary,
    }
}

fn parity_text(cfg: &ModbusConfig) -> String {
    let p = match cfg.parity {
        Parity::None => 'N',
        Parity::Even => 'E',
        Parity::Odd => 'O',
        Parity::Mark => 'M',
        Parity::Space => 'S',
    };
    format!("8{p}{}", cfg.stop_bits)
}

fn values_or_hex(f: &Frame) -> String {
    if f.values.is_empty() { hex(&f.raw.bytes[2..f.raw.bytes.len() - 2]) } else { values_text(&f.values) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{Synth, UartFault, modbus_frame, modbus_uart, uart_tx};

    const RATE: u64 = 1_000_000;

    #[test]
    fn crc_reference_values() {
        // Read holding registers, slave 1, address 0, 10 registers: 01 03 00 00 00 0A C5 CD.
        assert_eq!(modbus_frame(&[1, 3, 0, 0, 0, 10]), vec![1, 3, 0, 0, 0, 10, 0xC5, 0xCD]);
        assert_eq!(crc16(b"123456789"), 0x4B37);
    }

    struct Bus {
        s: Synth,
        t: f64,
        cfg: ModbusConfig,
    }

    impl Bus {
        fn new(cfg: ModbusConfig) -> Self {
            Self { s: Synth::new(RATE, [true; 8]), t: 0.002, cfg }
        }

        /// Sends a frame on `ch`, then waits `after` seconds.
        fn send(&mut self, ch: u8, bytes: &[u8], after: f64) {
            let u = modbus_uart(self.cfg.baud, ch);
            let u = UartConfig { parity: self.cfg.parity, stop_bits: self.cfg.stop_bits, ..u };
            self.t = uart_tx(&mut self.s, ch, self.t, bytes, &u, self.cfg.baud, 0.0, UartFault::default()) + after;
        }

        fn cap(mut self) -> Capture {
            self.s.extend_to(self.s.at(self.t + 0.01));
            self.s.capture(&["TX", "RX"], 0)
        }
    }

    fn two_ch() -> ModbusConfig {
        ModbusConfig { request_channel: Some(0), response_channel: Some(1), ..ModbusConfig::default() }
    }

    #[test]
    fn pairs_requests_and_responses_on_two_channels() {
        let cfg = two_ch();
        let mut b = Bus::new(cfg.clone());
        b.send(0, &modbus_frame(&[17, 3, 0, 107, 0, 3]), 0.004);
        b.send(1, &modbus_frame(&[17, 3, 6, 0x02, 0x2B, 0, 0, 0, 0x64]), 0.02);
        b.send(0, &modbus_frame(&[17, 6, 0, 1, 0, 3]), 0.003);
        b.send(1, &modbus_frame(&[17, 6, 0, 1, 0, 3]), 0.02);
        b.send(0, &modbus_frame(&[17, 16, 0, 1, 0, 2, 4, 0, 10, 1, 2]), 0.003);
        b.send(1, &modbus_frame(&[17, 16, 0, 1, 0, 2]), 0.02);
        let r = decode(&b.cap(), &cfg).unwrap();
        assert_eq!(r.frames.len(), 6);
        assert_eq!(r.transactions.len(), 3);
        assert!(r.transactions.iter().all(|t| t.status == Status::Ok));
        assert_eq!(r.frames[0].detail, "Read holding registers 107–109 (40108–40110) ×3");
        assert_eq!(r.frames[1].values, vec![555, 0, 100]);
        assert_eq!(r.frames[4].values, vec![10, 258]);
        let rt = r.transactions[0].response_time.unwrap();
        assert!((rt - 0.004).abs() < 0.0002, "response time {rt}");
    }

    #[test]
    fn single_channel_bus_pairs_by_context() {
        let cfg = ModbusConfig::default();
        let mut b = Bus::new(cfg.clone());
        // FC 6 request and echo are byte-identical: only the order tells them apart.
        b.send(0, &modbus_frame(&[5, 6, 0, 9, 0x12, 0x34]), 0.005);
        b.send(0, &modbus_frame(&[5, 6, 0, 9, 0x12, 0x34]), 0.02);
        b.send(0, &modbus_frame(&[5, 1, 0, 0, 0, 10]), 0.005);
        b.send(0, &modbus_frame(&[5, 1, 2, 0b1010_0101, 0b10]), 0.02);
        let r = decode(&b.cap(), &cfg).unwrap();
        let roles: Vec<Role> = r.frames.iter().map(|f| f.role).collect();
        assert_eq!(roles, vec![Role::Request, Role::Response, Role::Request, Role::Response]);
        assert_eq!(r.frames[3].values, vec![1, 0, 1, 0, 0, 1, 0, 1, 0, 1]);
        assert!(r.transactions.iter().all(|t| t.status == Status::Ok));
    }

    #[test]
    fn exception_crc_error_timeout_and_broadcast() {
        let cfg = two_ch();
        let mut b = Bus::new(cfg.clone());
        b.send(0, &modbus_frame(&[2, 3, 0x27, 0x10, 0, 1]), 0.003);
        b.send(1, &modbus_frame(&[2, 0x83, 2]), 0.02);
        let mut bad = modbus_frame(&[2, 4, 0, 0, 0, 2]);
        b.send(0, &bad, 0.003);
        bad = modbus_frame(&[2, 4, 4, 0, 1, 0, 2]);
        bad[4] ^= 0x10;
        b.send(1, &bad, 0.02);
        // No answer for 1.2 s.
        b.send(0, &modbus_frame(&[9, 3, 0, 0, 0, 1]), 1.2);
        b.send(0, &modbus_frame(&[0, 6, 0, 0, 0, 1]), 0.02);
        let r = decode(&b.cap(), &cfg).unwrap();
        let st: Vec<_> = r.transactions.iter().map(|t| t.status.clone()).collect();
        assert_eq!(st, vec![Status::Exception { code: 2 }, Status::CrcError, Status::NoResponse, Status::Broadcast]);
        assert_eq!(r.frames[1].detail, "Exception 02 Illegal data address");
        let out = output(&b_cap_dummy(), &cfg, &r);
        let table = out.table.unwrap();
        assert_eq!(table.rows.len(), 4);
        assert_eq!(table.rows[0].cells[6], "Exception 02 Illegal data address");
        assert!(out.annotations.iter().any(|a| a.level == Level::Error && a.text.starts_with("CRC")));
    }

    fn b_cap_dummy() -> Capture {
        Capture::new(RATE, crate::capture::default_channels(8), vec![0xFF; 10])
    }

    #[test]
    fn splits_back_to_back_frames_without_silence() {
        let cfg = ModbusConfig::default();
        let mut b = Bus::new(cfg.clone());
        // The slave answers 1 character time after the request: no 3.5-char gap.
        b.send(0, &modbus_frame(&[1, 4, 0, 0, 0, 1]), 0.0006);
        b.send(0, &modbus_frame(&[1, 4, 2, 0x01, 0xF4]), 0.02);
        let r = decode(&b.cap(), &cfg).unwrap();
        assert_eq!(r.frames.len(), 2);
        assert!(r.frames.iter().all(|f| f.crc_ok));
        assert_eq!(r.frames[1].values, vec![500]);
        assert_eq!(r.transactions[0].status, Status::Ok);
    }

    #[test]
    fn flags_inter_character_gap() {
        let cfg = ModbusConfig::default();
        let mut s = Synth::new(RATE, [true; 8]);
        let u = modbus_uart(cfg.baud, 0);
        let f = modbus_frame(&[1, 3, 0, 0, 0, 1]);
        let t = uart_tx(&mut s, 0, 0.001, &f[..3], &u, cfg.baud, 0.0, UartFault::default());
        // 2.5 characters of silence: over t1.5, under t3.5.
        let t = uart_tx(&mut s, 0, t + 2.5 * cfg.char_time(), &f[3..], &u, cfg.baud, 0.0, UartFault::default());
        s.extend_to(s.at(t + 0.01));
        let r = decode(&s.capture(&[], 0), &cfg).unwrap();
        assert_eq!(r.frames.len(), 1);
        assert!(r.frames[0].crc_ok);
        assert!(r.frames[0].raw.t15_violation);
    }

    #[test]
    fn wrong_baud_shows_errors() {
        let cfg = two_ch();
        let mut b = Bus::new(cfg.clone());
        b.send(0, &modbus_frame(&[1, 3, 0, 0, 0, 2]), 0.003);
        b.send(1, &modbus_frame(&[1, 3, 4, 0, 1, 0, 2]), 0.02);
        let cap = b.cap();
        let wrong = ModbusConfig { baud: 9600.0, ..cfg };
        let r = decode(&cap, &wrong).unwrap();
        assert!(r.frames.iter().all(|f| !f.crc_ok));
        assert!(r.transactions.iter().all(|t| t.status != Status::Ok));
    }

    #[test]
    fn high_baud_uses_fixed_silence() {
        let cfg = ModbusConfig { baud: 115200.0, parity: Parity::None, stop_bits: 2.0, ..two_ch() };
        assert_eq!(cfg.t35(), 0.00175);
        let mut b = Bus::new(cfg.clone());
        b.send(0, &modbus_frame(&[1, 3, 0, 0, 0, 2]), 0.002);
        b.send(1, &modbus_frame(&[1, 3, 4, 0, 1, 0, 2]), 0.002);
        let r = decode(&b.cap(), &cfg).unwrap();
        assert_eq!(r.transactions.len(), 1);
        assert_eq!(r.transactions[0].status, Status::Ok);
    }
}
