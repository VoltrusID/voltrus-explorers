//! 3E and 4E frames, binary and ASCII: encoding of requests and responses, and stream framing.
//!
//! Layout (binary 3E request): `50 00 | net | pc | io(LE16) | station | len(LE16) | timer(LE16) | cmd(LE16) |
//! sub(LE16) | body`, where `len` counts the bytes from the timer to the end. The 4E frame inserts a serial number
//! (`54 00 | serial(LE16) | 00 00`). Responses start with `D0 00` / `D4 00` and carry an end code instead of the
//! timer. ASCII frames carry every field as upper-case hex text, most significant digit first.

use serde::{Deserialize, Serialize};

use crate::device::{McAddress, McDevice};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FrameKind {
    #[default]
    #[serde(rename = "3E")]
    E3,
    #[serde(rename = "4E")]
    E4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Binary,
    Ascii,
}

/// Access route: network number, PC number, request destination module I/O number and station number.
/// `0, 0xFF, 0x03FF, 0` addresses the CPU the Ethernet port belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub network: u8,
    pub pc: u8,
    pub io: u16,
    pub station: u8,
}

impl Default for Route {
    fn default() -> Self {
        Self { network: 0, pc: 0xFF, io: 0x03FF, station: 0 }
    }
}

pub mod cmd {
    pub const BATCH_READ: u16 = 0x0401;
    pub const BATCH_WRITE: u16 = 0x1401;
    pub const READ_CPU_MODEL: u16 = 0x0101;
    pub const REMOTE_RUN: u16 = 0x1001;
    pub const REMOTE_STOP: u16 = 0x1002;
    pub const SUB_WORD: u16 = 0x0000;
    pub const SUB_BIT: u16 = 0x0001;
}

/// The commands this crate speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    ReadWords { head: McAddress, count: u16 },
    ReadBits { head: McAddress, count: u16 },
    WriteWords { head: McAddress, values: Vec<u16> },
    WriteBits { head: McAddress, values: Vec<bool> },
    ReadCpuModel,
    /// `force`: run even if another device holds the CPU stopped. `clear`: 0 keeps devices, 1 clears all but latch,
    /// 2 clears everything.
    RemoteRun { force: bool, clear: u8 },
    RemoteStop,
}

impl Command {
    pub fn code(&self) -> (u16, u16) {
        use cmd::*;
        match self {
            Command::ReadWords { .. } => (BATCH_READ, SUB_WORD),
            Command::ReadBits { .. } => (BATCH_READ, SUB_BIT),
            Command::WriteWords { .. } => (BATCH_WRITE, SUB_WORD),
            Command::WriteBits { .. } => (BATCH_WRITE, SUB_BIT),
            Command::ReadCpuModel => (READ_CPU_MODEL, 0),
            Command::RemoteRun { .. } => (REMOTE_RUN, 0),
            Command::RemoteStop => (REMOTE_STOP, 0),
        }
    }

    /// Request data after the subcommand.
    pub fn encode_body(&self, enc: Encoding) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Command::ReadWords { head, count } | Command::ReadBits { head, count } => {
                put_device(&mut out, enc, head);
                put_u16(&mut out, enc, *count);
            }
            Command::WriteWords { head, values } => {
                put_device(&mut out, enc, head);
                put_u16(&mut out, enc, values.len() as u16);
                put_words(&mut out, enc, values);
            }
            Command::WriteBits { head, values } => {
                put_device(&mut out, enc, head);
                put_u16(&mut out, enc, values.len() as u16);
                put_bits(&mut out, enc, values);
            }
            Command::ReadCpuModel => {}
            Command::RemoteRun { force, clear } => {
                put_u16(&mut out, enc, if *force { 0x0003 } else { 0x0001 });
                put_u8(&mut out, enc, *clear);
                put_u8(&mut out, enc, 0);
            }
            Command::RemoteStop => put_u16(&mut out, enc, 0x0001),
        }
        out
    }

    /// Server side: decode a request body.
    pub fn decode(command: u16, sub: u16, body: &[u8], enc: Encoding) -> Result<Command, u16> {
        let mut r = Reader { buf: body, pos: 0, enc };
        let c = match (command, sub) {
            (cmd::BATCH_READ, cmd::SUB_WORD) => Command::ReadWords { head: r.device()?, count: r.u16()? },
            (cmd::BATCH_READ, cmd::SUB_BIT) => Command::ReadBits { head: r.device()?, count: r.u16()? },
            (cmd::BATCH_WRITE, cmd::SUB_WORD) => {
                let head = r.device()?;
                let n = r.u16()? as usize;
                let values = (0..n).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
                Command::WriteWords { head, values }
            }
            (cmd::BATCH_WRITE, cmd::SUB_BIT) => {
                let head = r.device()?;
                let n = r.u16()? as usize;
                let values = r.bits(n)?;
                Command::WriteBits { head, values }
            }
            (cmd::READ_CPU_MODEL, 0) => Command::ReadCpuModel,
            (cmd::REMOTE_RUN, 0) => {
                let mode = r.u16()?;
                let clear = r.u8()?;
                r.u8()?; // fixed 00
                Command::RemoteRun { force: mode == 0x0003, clear }
            }
            (cmd::REMOTE_STOP, 0) => {
                r.u16()?; // fixed 0001
                Command::RemoteStop
            }
            _ => return Err(end::UNSUPPORTED_COMMAND),
        };
        if r.pos != body.len() {
            return Err(end::LENGTH_MISMATCH);
        }
        Ok(c)
    }
}

/// End codes the simulator answers with (and the client explains).
pub mod end {
    pub const ASCII_CONVERSION: u16 = 0xC050;
    pub const POINTS_OUT_OF_RANGE: u16 = 0xC051;
    pub const DEVICE_OUT_OF_RANGE: u16 = 0xC056;
    pub const LENGTH_MISMATCH: u16 = 0xC058;
    pub const UNSUPPORTED_COMMAND: u16 = 0xC059;
    pub const WRONG_DEVICE: u16 = 0xC05B;
    pub const REQUEST_CONTENT: u16 = 0xC05C;
    pub const DATA_LENGTH: u16 = 0xC061;
}

/// What an end code means, in plain words. Codes not listed come back as "end code 0xXXXX" from the caller.
pub fn end_code_text(code: u16) -> Option<&'static str> {
    Some(match code {
        0x0000 => "normal completion",
        0xC050 => "the module received ASCII data it cannot convert to binary (check binary/ASCII matches the PLC's open setting)",
        0xC051..=0xC054 => "the number of points is out of range for this command",
        0xC056 => "the request goes past the last device number of the CPU",
        0xC058 => "the request data length does not match the data that followed",
        0xC059 => "the command or subcommand is not supported by this CPU or module",
        0xC05B => "the CPU cannot read or write the specified device",
        0xC05C => "request content error (for example bit units on a word device)",
        0xC05F => "the request cannot be executed on the target CPU",
        0xC061 => "the request data length does not match the number of data points",
        0xC06F => "the frame's code (binary or ASCII) does not match the module's communication data code setting",
        0xC070 => "the device memory extension cannot be specified for this station",
        0xC0B5 => "the CPU cannot handle the specified data",
        0x4030 => "the specified device cannot be used on this CPU",
        0x4031 => "the device number is outside the CPU's device allocation",
        0x4013 => "the CPU is running and the request is only allowed in STOP",
        _ => return None,
    })
}

// ── Field encoding ──

fn put_u8(out: &mut Vec<u8>, enc: Encoding, v: u8) {
    match enc {
        Encoding::Binary => out.push(v),
        Encoding::Ascii => out.extend_from_slice(format!("{v:02X}").as_bytes()),
    }
}

fn put_u16(out: &mut Vec<u8>, enc: Encoding, v: u16) {
    match enc {
        Encoding::Binary => out.extend_from_slice(&v.to_le_bytes()),
        Encoding::Ascii => out.extend_from_slice(format!("{v:04X}").as_bytes()),
    }
}

fn put_device(out: &mut Vec<u8>, enc: Encoding, a: &McAddress) {
    match enc {
        Encoding::Binary => {
            out.extend_from_slice(&a.number.to_le_bytes()[..3]);
            out.push(a.device.binary_code());
        }
        Encoding::Ascii => {
            out.extend_from_slice(a.device.ascii_code().as_bytes());
            let n = if a.device.hex_numbered() { format!("{:06X}", a.number) } else { format!("{:06}", a.number) };
            out.extend_from_slice(n.as_bytes());
        }
    }
}

pub(crate) fn put_words(out: &mut Vec<u8>, enc: Encoding, values: &[u16]) {
    for v in values {
        put_u16(out, enc, *v);
    }
}

/// Bit units: binary packs two points per byte, the first in the high nibble; ASCII sends one '0'/'1' per point.
pub(crate) fn put_bits(out: &mut Vec<u8>, enc: Encoding, values: &[bool]) {
    match enc {
        Encoding::Binary => {
            for pair in values.chunks(2) {
                let hi = if pair[0] { 0x10 } else { 0 };
                let lo = if pair.get(1).copied().unwrap_or(false) { 0x01 } else { 0 };
                out.push(hi | lo);
            }
        }
        Encoding::Ascii => out.extend(values.iter().map(|b| if *b { b'1' } else { b'0' })),
    }
}

/// Reads fields from a body in either encoding. Errors are MC end codes.
pub struct Reader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
    pub enc: Encoding,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], u16> {
        if self.pos + n > self.buf.len() {
            return Err(end::LENGTH_MISMATCH);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn hex(&mut self, chars: usize) -> Result<u32, u16> {
        let s = self.take(chars)?;
        let t = std::str::from_utf8(s).map_err(|_| end::ASCII_CONVERSION)?;
        u32::from_str_radix(t, 16).map_err(|_| end::ASCII_CONVERSION)
    }

    pub fn u8(&mut self) -> Result<u8, u16> {
        match self.enc {
            Encoding::Binary => Ok(self.take(1)?[0]),
            Encoding::Ascii => Ok(self.hex(2)? as u8),
        }
    }

    pub fn u16(&mut self) -> Result<u16, u16> {
        match self.enc {
            Encoding::Binary => {
                let s = self.take(2)?;
                Ok(u16::from_le_bytes([s[0], s[1]]))
            }
            Encoding::Ascii => Ok(self.hex(4)? as u16),
        }
    }

    pub fn device(&mut self) -> Result<McAddress, u16> {
        match self.enc {
            Encoding::Binary => {
                let s = self.take(4)?;
                let number = u32::from_le_bytes([s[0], s[1], s[2], 0]);
                let device = McDevice::from_binary_code(s[3]).ok_or(end::WRONG_DEVICE)?;
                Ok(McAddress::new(device, number))
            }
            Encoding::Ascii => {
                let code = std::str::from_utf8(self.take(2)?).map_err(|_| end::ASCII_CONVERSION)?.to_string();
                let device = McDevice::from_ascii_code(&code).ok_or(end::WRONG_DEVICE)?;
                let digits = std::str::from_utf8(self.take(6)?).map_err(|_| end::ASCII_CONVERSION)?;
                let radix = if device.hex_numbered() { 16 } else { 10 };
                let number = u32::from_str_radix(digits, radix).map_err(|_| end::ASCII_CONVERSION)?;
                Ok(McAddress::new(device, number))
            }
        }
    }

    pub fn bits(&mut self, n: usize) -> Result<Vec<bool>, u16> {
        match self.enc {
            Encoding::Binary => {
                let s = self.take(n.div_ceil(2))?;
                Ok((0..n).map(|i| {
                    let b = s[i / 2];
                    if i % 2 == 0 { b & 0x10 != 0 } else { b & 0x01 != 0 }
                }).collect())
            }
            Encoding::Ascii => {
                let s = self.take(n)?;
                s.iter()
                    .map(|c| match c {
                        b'0' => Ok(false),
                        b'1' => Ok(true),
                        _ => Err(end::ASCII_CONVERSION),
                    })
                    .collect()
            }
        }
    }

    pub fn words(&mut self, n: usize) -> Result<Vec<u16>, u16> {
        (0..n).map(|_| self.u16()).collect()
    }

    pub fn rest(&mut self) -> &[u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
}

// ── Frames ──

/// A request frame (client to PLC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: FrameKind,
    pub encoding: Encoding,
    pub serial: u16,
    pub route: Route,
    /// Monitoring timer in units of 250 ms (0 waits forever).
    pub timer: u16,
    pub command: u16,
    pub subcommand: u16,
    /// Request data in the frame's encoding.
    pub body: Vec<u8>,
}

/// A response frame (PLC to client).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub kind: FrameKind,
    pub encoding: Encoding,
    pub serial: u16,
    pub route: Route,
    pub end_code: u16,
    /// Response data in the frame's encoding (for an error, the error information block).
    pub data: Vec<u8>,
}

fn put_head(out: &mut Vec<u8>, kind: FrameKind, enc: Encoding, response: bool, serial: u16, route: &Route) {
    let sub = match (kind, response) {
        (FrameKind::E3, false) => 0x5000u16,
        (FrameKind::E4, false) => 0x5400,
        (FrameKind::E3, true) => 0xD000,
        (FrameKind::E4, true) => 0xD400,
    };
    match enc {
        Encoding::Binary => out.extend_from_slice(&sub.to_be_bytes()),
        Encoding::Ascii => out.extend_from_slice(format!("{sub:04X}").as_bytes()),
    }
    if kind == FrameKind::E4 {
        put_u16(out, enc, serial);
        put_u16(out, enc, 0);
    }
    put_u8(out, enc, route.network);
    put_u8(out, enc, route.pc);
    put_u16(out, enc, route.io);
    put_u8(out, enc, route.station);
}

impl Request {
    pub fn new(kind: FrameKind, encoding: Encoding, serial: u16, route: Route, timer: u16, command: &Command) -> Self {
        let (c, s) = command.code();
        Self { kind, encoding, serial, route, timer, command: c, subcommand: s, body: command.encode_body(encoding) }
    }

    pub fn encode(&self) -> Vec<u8> {
        let enc = self.encoding;
        let mut out = Vec::with_capacity(32 + self.body.len());
        put_head(&mut out, self.kind, enc, false, self.serial, &self.route);
        let fixed = match enc {
            Encoding::Binary => 6,
            Encoding::Ascii => 12,
        };
        put_u16(&mut out, enc, (fixed + self.body.len()) as u16);
        put_u16(&mut out, enc, self.timer);
        put_u16(&mut out, enc, self.command);
        put_u16(&mut out, enc, self.subcommand);
        out.extend_from_slice(&self.body);
        out
    }

    /// Decode one complete request frame (the server side).
    pub fn decode(buf: &[u8]) -> Result<Request, String> {
        let (kind, encoding) = detect(buf).ok_or("not an MC protocol 3E/4E request")?;
        if is_response(buf) {
            return Err("this is a response, not a request".into());
        }
        let mut r = Reader { buf, pos: if encoding == Encoding::Binary { 2 } else { 4 }, enc: encoding };
        let e = |c: u16| format!("malformed request (0x{c:04X})");
        let serial = if kind == FrameKind::E4 {
            let s = r.u16().map_err(e)?;
            r.u16().map_err(e)?;
            s
        } else {
            0
        };
        let route = Route { network: r.u8().map_err(e)?, pc: r.u8().map_err(e)?, io: r.u16().map_err(e)?, station: r.u8().map_err(e)? };
        let len = r.u16().map_err(e)? as usize;
        if r.pos + len != buf.len() {
            return Err("request data length does not match the frame".into());
        }
        let timer = r.u16().map_err(e)?;
        let command = r.u16().map_err(e)?;
        let subcommand = r.u16().map_err(e)?;
        let body = r.rest().to_vec();
        Ok(Request { kind, encoding, serial, route, timer, command, subcommand, body })
    }
}

impl Response {
    /// A response that echoes the request's frame, encoding, serial and route.
    pub fn to(req: &Request, end_code: u16, data: Vec<u8>) -> Self {
        let data = if end_code == 0 {
            data
        } else {
            // Error information: the request's route, command and subcommand.
            let mut d = Vec::new();
            let enc = req.encoding;
            put_u8(&mut d, enc, req.route.network);
            put_u8(&mut d, enc, req.route.pc);
            put_u16(&mut d, enc, req.route.io);
            put_u8(&mut d, enc, req.route.station);
            put_u16(&mut d, enc, req.command);
            put_u16(&mut d, enc, req.subcommand);
            d
        };
        Self { kind: req.kind, encoding: req.encoding, serial: req.serial, route: req.route, end_code, data }
    }

    pub fn encode(&self) -> Vec<u8> {
        let enc = self.encoding;
        let mut out = Vec::with_capacity(24 + self.data.len());
        put_head(&mut out, self.kind, enc, true, self.serial, &self.route);
        let fixed = match enc {
            Encoding::Binary => 2,
            Encoding::Ascii => 4,
        };
        put_u16(&mut out, enc, (fixed + self.data.len()) as u16);
        put_u16(&mut out, enc, self.end_code);
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(buf: &[u8]) -> Result<Response, String> {
        let (kind, encoding) = detect(buf).ok_or("not an MC protocol 3E/4E response")?;
        if !is_response(buf) {
            return Err("this is a request, not a response".into());
        }
        let mut r = Reader { buf, pos: if encoding == Encoding::Binary { 2 } else { 4 }, enc: encoding };
        let e = |c: u16| format!("malformed response (0x{c:04X})");
        let serial = if kind == FrameKind::E4 {
            let s = r.u16().map_err(e)?;
            r.u16().map_err(e)?;
            s
        } else {
            0
        };
        let route = Route { network: r.u8().map_err(e)?, pc: r.u8().map_err(e)?, io: r.u16().map_err(e)?, station: r.u8().map_err(e)? };
        let len = r.u16().map_err(e)? as usize;
        if r.pos + len != buf.len() {
            return Err(format!("response length field says {len}, frame has {}", buf.len() - r.pos));
        }
        let end_code = r.u16().map_err(e)?;
        let data = r.rest().to_vec();
        Ok(Response { kind, encoding, serial, route, end_code, data })
    }

    pub fn reader(&self) -> Reader<'_> {
        Reader { buf: &self.data, pos: 0, enc: self.encoding }
    }
}

/// Frame kind and encoding from the subheader (requests `50 00`/`54 00`, responses `D0 00`/`D4 00`, or their ASCII text).
pub fn detect(buf: &[u8]) -> Option<(FrameKind, Encoding)> {
    if buf.len() >= 2 && buf[1] == 0x00 {
        match buf[0] {
            0x50 | 0xD0 => return Some((FrameKind::E3, Encoding::Binary)),
            0x54 | 0xD4 => return Some((FrameKind::E4, Encoding::Binary)),
            _ => {}
        }
    }
    if buf.len() >= 4 {
        match &buf[..4] {
            b"5000" | b"D000" => return Some((FrameKind::E3, Encoding::Ascii)),
            b"5400" | b"D400" => return Some((FrameKind::E4, Encoding::Ascii)),
            _ => {}
        }
    }
    None
}

/// The bytes at the start of a buffer are not an MC protocol frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAFrame;

fn is_response(buf: &[u8]) -> bool {
    matches!(buf.first(), Some(0xD0 | 0xD4 | b'D'))
}

/// Total length of the frame at the start of `buf`, once enough bytes for the length field have arrived.
/// `Err` when the start is not an MC frame.
pub fn frame_len(buf: &[u8]) -> Result<Option<usize>, NotAFrame> {
    let binary_start = buf.len() >= 2 && buf[1] == 0 && matches!(buf[0], 0x50 | 0x54 | 0xD0 | 0xD4);
    if buf.len() < 4 && !binary_start {
        return Ok(None);
    }
    let (kind, enc) = detect(buf).ok_or(NotAFrame)?;
    // Offset and size of the length field.
    let (at, size) = match (kind, enc) {
        (FrameKind::E3, Encoding::Binary) => (7, 2),
        (FrameKind::E4, Encoding::Binary) => (11, 2),
        (FrameKind::E3, Encoding::Ascii) => (14, 4),
        (FrameKind::E4, Encoding::Ascii) => (22, 4),
    };
    if buf.len() < at + size {
        return Ok(None);
    }
    let len = match enc {
        Encoding::Binary => u16::from_le_bytes([buf[at], buf[at + 1]]) as usize,
        Encoding::Ascii => {
            let t = std::str::from_utf8(&buf[at..at + 4]).map_err(|_| NotAFrame)?;
            u16::from_str_radix(t, 16).map_err(|_| NotAFrame)? as usize
        }
    };
    Ok(Some(at + size + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The manual's example: batch read of D100..D102 in word units, binary 3E.
    #[test]
    fn binary_3e_batch_read_matches_the_manual() {
        let req = Request::new(
            FrameKind::E3,
            Encoding::Binary,
            0,
            Route::default(),
            0x0010,
            &Command::ReadWords { head: McAddress::new(McDevice::D, 100), count: 3 },
        );
        assert_eq!(req.encode(), hex("50 00 00 FF FF 03 00 0C 00 10 00 01 04 00 00 64 00 00 A8 03 00"));
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);
        assert_eq!(frame_len(&req.encode()).unwrap(), Some(21));
    }

    #[test]
    fn ascii_3e_batch_read_matches_the_manual() {
        let req = Request::new(
            FrameKind::E3,
            Encoding::Ascii,
            0,
            Route::default(),
            0x0010,
            &Command::ReadWords { head: McAddress::new(McDevice::D, 100), count: 3 },
        );
        assert_eq!(String::from_utf8(req.encode()).unwrap(), "500000FF03FF000018001004010000D*0001000003");
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);
    }

    #[test]
    fn ascii_hex_numbered_device() {
        let body = Command::ReadBits { head: McAddress::new(McDevice::X, 0x1F), count: 8 }.encode_body(Encoding::Ascii);
        assert_eq!(String::from_utf8(body).unwrap(), "X*00001F0008");
    }

    #[test]
    fn binary_4e_carries_the_serial() {
        let req = Request::new(
            FrameKind::E4,
            Encoding::Binary,
            0x1234,
            Route::default(),
            0x0010,
            &Command::ReadBits { head: McAddress::new(McDevice::M, 100), count: 8 },
        );
        assert_eq!(req.encode(), hex("54 00 34 12 00 00 00 FF FF 03 00 0C 00 10 00 01 04 01 00 64 00 00 90 08 00"));
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);
        assert_eq!(frame_len(&req.encode()).unwrap(), Some(25));
    }

    #[test]
    fn binary_bit_write_packs_nibbles() {
        let req = Request::new(
            FrameKind::E3,
            Encoding::Binary,
            0,
            Route::default(),
            0x0010,
            &Command::WriteBits { head: McAddress::new(McDevice::M, 100), values: vec![true, false, true] },
        );
        // M100 = ON, M101 = OFF, M102 = ON: 10 10.
        assert_eq!(req.encode(), hex("50 00 00 FF FF 03 00 0E 00 10 00 01 14 01 00 64 00 00 90 03 00 10 10"));
    }

    #[test]
    fn binary_word_write() {
        let req = Request::new(
            FrameKind::E3,
            Encoding::Binary,
            0,
            Route::default(),
            0x0010,
            &Command::WriteWords { head: McAddress::new(McDevice::D, 100), values: vec![0x1995, 0x1202, 0x1130] },
        );
        assert_eq!(
            req.encode(),
            hex("50 00 00 FF FF 03 00 12 00 10 00 01 14 00 00 64 00 00 A8 03 00 95 19 02 12 30 11")
        );
        let back = Request::decode(&req.encode()).unwrap();
        assert_eq!(
            Command::decode(back.command, back.subcommand, &back.body, back.encoding).unwrap(),
            Command::WriteWords { head: McAddress::new(McDevice::D, 100), values: vec![0x1995, 0x1202, 0x1130] }
        );
    }

    #[test]
    fn binary_response_and_error_response() {
        let req = Request::new(FrameKind::E3, Encoding::Binary, 0, Route::default(), 16, &Command::ReadCpuModel);
        let ok = Response::to(&req, 0, hex("95 19 02 12 30 11"));
        assert_eq!(ok.encode(), hex("D0 00 00 FF FF 03 00 08 00 00 00 95 19 02 12 30 11"));
        assert_eq!(Response::decode(&ok.encode()).unwrap(), ok);
        let err = Response::to(&req, 0xC059, vec![]);
        assert_eq!(err.encode(), hex("D0 00 00 FF FF 03 00 0B 00 59 C0 00 FF FF 03 00 01 01 00 00"));
        assert_eq!(frame_len(&err.encode()).unwrap(), Some(20));
    }

    #[test]
    fn ascii_response_round_trip_and_framing() {
        let req = Request::new(FrameKind::E4, Encoding::Ascii, 7, Route::default(), 16, &Command::ReadCpuModel);
        let resp = Response::to(&req, 0, b"0102".to_vec());
        let bytes = resp.encode();
        assert_eq!(String::from_utf8(bytes.clone()).unwrap(), "D40000070000" .to_string() + "00FF03FF00" + "0008" + "0000" + "0102");
        assert_eq!(frame_len(&bytes).unwrap(), Some(bytes.len()));
        assert_eq!(frame_len(&bytes[..20]).unwrap(), None);
        assert_eq!(Response::decode(&bytes).unwrap(), resp);
    }

    #[test]
    fn ascii_bits_and_remote_run() {
        let body = Command::WriteBits { head: McAddress::new(McDevice::M, 0), values: vec![true, true, false] }.encode_body(Encoding::Ascii);
        assert_eq!(String::from_utf8(body).unwrap(), "M*0000000003110");
        let run = Command::RemoteRun { force: false, clear: 0 };
        assert_eq!(run.encode_body(Encoding::Binary), hex("01 00 00 00"));
        assert_eq!(Command::decode(0x1001, 0, &hex("03 00 02 00"), Encoding::Binary).unwrap(), Command::RemoteRun { force: true, clear: 2 });
    }

    #[test]
    fn unknown_commands_and_bad_lengths_are_end_codes() {
        assert_eq!(Command::decode(0x0613, 0, &[], Encoding::Binary), Err(end::UNSUPPORTED_COMMAND));
        assert_eq!(Command::decode(0x0401, 0, &hex("64 00 00 A8"), Encoding::Binary), Err(end::LENGTH_MISMATCH));
        assert_eq!(Command::decode(0x0401, 0, &hex("64 00 00 FE 01 00"), Encoding::Binary), Err(end::WRONG_DEVICE));
        assert!(frame_len(b"HTTP/1.1").is_err());
        let req = Request::new(FrameKind::E3, Encoding::Binary, 0, Route::default(), 16, &Command::ReadCpuModel);
        assert!(Response::decode(&req.encode()).is_err());
        assert!(Request::decode(&Response::to(&req, 0, vec![]).encode()).is_err());
    }
}
