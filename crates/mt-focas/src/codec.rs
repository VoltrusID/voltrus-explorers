//! FOCAS/Ethernet framing, as reverse-engineered by pyfanuc (Unlicense).
//!
//! Every packet, request or response, is
//!
//! ```text
//! a0 a0 a0 a0 | version u16 | frame type u16 | payload length u16 | payload
//! ```
//!
//! all big endian. The frame types seen in the reference are open (`0101`/`0102`), close
//! (`0201`/`0202`) and variable data (`2101`/`2102`). A variable-data payload is a sub-packet
//! count (`u16`) followed by the sub-packets, each led by its own length (`u16`, counting itself):
//!
//! ```text
//! request  sub-packet: length | device u16 | class u16 | function u16 | 5 x i32 | optional extra bytes
//! response sub-packet: length | device u16 | class u16 | function u16 | error u16 | 4 fill | payload length u16 | payload
//! ```
//!
//! The device is 1 for the CNC and 2 for the PMC; the class and function together are the four
//! byte "func" of the reference (`0001 0018` for system info). A failed call carries a non-zero
//! error (an `EW_*` code) and an empty payload.

use thiserror::Error;

/// Start of every frame.
pub const SYNC: [u8; 4] = [0xa0; 4];
/// Sync, version, frame type and payload length.
pub const HEADER_LEN: usize = 10;
/// Version field of request frames (pyfanuc `fvers=1`).
pub const VERSION_REQUEST: u16 = 1;
/// Version field this crate's simulator puts in responses. The reference samples show `0002` and
/// `0004` from real controllers; clients (pyfanuc, this crate) do not look at it.
pub const VERSION_RESPONSE: u16 = 2;
/// Open-request payload for the control (data) stream, pyfanuc `FRAME_DST`.
pub const CHANNEL_CONTROL: u16 = 2;
/// Open-request payload for the program-transfer stream, pyfanuc `FRAME_DST2`.
pub const CHANNEL_PROGRAM: u16 = 1;
/// Device number of the CNC in a sub-packet.
pub const DEVICE_CNC: u16 = 1;
/// Device number of the PMC in a sub-packet.
pub const DEVICE_PMC: u16 = 2;
/// Request sub-packet body: command (6) plus five `i32` arguments (20).
pub const REQUEST_BODY_LEN: usize = 26;
/// Response sub-packet body before the payload: command (6), error (2), fill (4), length (2).
pub const RESPONSE_HEAD_LEN: usize = 14;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    #[error("frame shorter than its header")]
    Truncated,
    #[error("frame does not start with a0 a0 a0 a0")]
    BadSync,
    #[error("payload length {declared} does not match the {actual} bytes received")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("payload of {0} bytes does not fit the 16-bit frame length")]
    PayloadTooLarge(usize),
    #[error("malformed sub-packet: {0}")]
    Malformed(&'static str),
    #[error("expected frame type {expected:04x}, got {actual:04x}")]
    UnexpectedFrame { expected: u16, actual: u16 },
}

/// The frame types of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    OpenRequest,
    OpenResponse,
    CloseRequest,
    CloseResponse,
    VarRequest,
    VarResponse,
    /// Anything else, e.g. the program-transfer stream types (`0x1501`, `0x1604`, ...).
    Other(u16),
}

impl FrameType {
    pub fn code(self) -> u16 {
        match self {
            FrameType::OpenRequest => 0x0101,
            FrameType::OpenResponse => 0x0102,
            FrameType::CloseRequest => 0x0201,
            FrameType::CloseResponse => 0x0202,
            FrameType::VarRequest => 0x2101,
            FrameType::VarResponse => 0x2102,
            FrameType::Other(c) => c,
        }
    }

    pub fn from_code(code: u16) -> Self {
        match code {
            0x0101 => FrameType::OpenRequest,
            0x0102 => FrameType::OpenResponse,
            0x0201 => FrameType::CloseRequest,
            0x0202 => FrameType::CloseResponse,
            0x2101 => FrameType::VarRequest,
            0x2102 => FrameType::VarResponse,
            other => FrameType::Other(other),
        }
    }
}

/// One frame: header fields and the raw payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: u16,
    pub frame_type: FrameType,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(version: u16, frame_type: FrameType, payload: Vec<u8>) -> Self {
        Frame {
            version,
            frame_type,
            payload,
        }
    }

    /// Header plus payload as sent on the wire.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let len = u16::try_from(self.payload.len())
            .map_err(|_| CodecError::PayloadTooLarge(self.payload.len()))?;
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&SYNC);
        out.extend_from_slice(&self.version.to_be_bytes());
        out.extend_from_slice(&self.frame_type.code().to_be_bytes());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    /// Parse one frame from the start of `buf`.
    ///
    /// Returns `Ok(None)` while `buf` holds less than a whole frame, otherwise the frame and the
    /// number of bytes it took.
    pub fn parse_prefix(buf: &[u8]) -> Result<Option<(Frame, usize)>, CodecError> {
        if buf.len() >= SYNC.len() && buf[..SYNC.len()] != SYNC {
            return Err(CodecError::BadSync);
        }
        if buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let len = usize::from(u16::from_be_bytes([buf[8], buf[9]]));
        let total = HEADER_LEN + len;
        if buf.len() < total {
            return Ok(None);
        }
        let frame = Frame {
            version: u16::from_be_bytes([buf[4], buf[5]]),
            frame_type: FrameType::from_code(u16::from_be_bytes([buf[6], buf[7]])),
            payload: buf[HEADER_LEN..total].to_vec(),
        };
        Ok(Some((frame, total)))
    }

    /// Parse `buf` as exactly one frame (pyfanuc `_decap`: the length must match).
    pub fn decode(buf: &[u8]) -> Result<Frame, CodecError> {
        if buf.len() < HEADER_LEN {
            return Err(if buf.len() >= SYNC.len() && buf[..SYNC.len()] != SYNC {
                CodecError::BadSync
            } else {
                CodecError::Truncated
            });
        }
        match Frame::parse_prefix(buf)? {
            Some((frame, used)) if used == buf.len() => Ok(frame),
            Some((_, used)) => Err(CodecError::LengthMismatch {
                declared: used - HEADER_LEN,
                actual: buf.len() - HEADER_LEN,
            }),
            None => Err(CodecError::LengthMismatch {
                declared: usize::from(u16::from_be_bytes([buf[8], buf[9]])),
                actual: buf.len() - HEADER_LEN,
            }),
        }
    }

    /// Open request for `channel` ([`CHANNEL_CONTROL`] or [`CHANNEL_PROGRAM`]).
    pub fn open_request(channel: u16) -> Frame {
        Frame::new(
            VERSION_REQUEST,
            FrameType::OpenRequest,
            channel.to_be_bytes().to_vec(),
        )
    }

    /// Open response. The payload of a real controller's answer is not documented in the
    /// reference and no client looks at it, so the simulator sends none.
    pub fn open_response() -> Frame {
        Frame::new(VERSION_RESPONSE, FrameType::OpenResponse, Vec::new())
    }

    pub fn close_request() -> Frame {
        Frame::new(VERSION_REQUEST, FrameType::CloseRequest, Vec::new())
    }

    pub fn close_response() -> Frame {
        Frame::new(VERSION_RESPONSE, FrameType::CloseResponse, Vec::new())
    }

    /// The channel an open request asks for.
    pub fn open_channel(&self) -> Option<u16> {
        match self.payload[..] {
            [a, b] => Some(u16::from_be_bytes([a, b])),
            _ => None,
        }
    }
}

/// Device, class and function of a sub-packet: the first six bytes of both its request and its
/// response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Command {
    pub device: u16,
    /// Class in the high 16 bits, function in the low 16 (the reference's "func").
    pub function: u32,
}

impl Command {
    /// A CNC command (device 1, class 1) with the given function number.
    pub const fn cnc(function: u16) -> Self {
        Command {
            device: DEVICE_CNC,
            function: 0x0001_0000 | function as u32,
        }
    }

    /// A PMC command (device 2, class 1) with the given function number.
    pub const fn pmc(function: u16) -> Self {
        Command {
            device: DEVICE_PMC,
            function: 0x0001_0000 | function as u32,
        }
    }

    pub fn to_bytes(self) -> [u8; 6] {
        let d = self.device.to_be_bytes();
        let f = self.function.to_be_bytes();
        [d[0], d[1], f[0], f[1], f[2], f[3]]
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 6 {
            return None;
        }
        Some(Command {
            device: u16::from_be_bytes([b[0], b[1]]),
            function: u32::from_be_bytes([b[2], b[3], b[4], b[5]]),
        })
    }
}

/// One request sub-packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRequest {
    pub command: Command,
    /// The five `i32` arguments every request carries; unused ones are 0.
    pub args: [i32; 5],
    /// Bytes after the arguments (macro write, program-directory buffers).
    pub extra: Vec<u8>,
}

impl SubRequest {
    pub fn new(command: Command, args: [i32; 5]) -> Self {
        SubRequest {
            command,
            args,
            extra: Vec::new(),
        }
    }

    pub fn with_extra(mut self, extra: Vec<u8>) -> Self {
        self.extra = extra;
        self
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(REQUEST_BODY_LEN + self.extra.len());
        out.extend_from_slice(&self.command.to_bytes());
        for a in self.args {
            out.extend_from_slice(&a.to_be_bytes());
        }
        out.extend_from_slice(&self.extra);
        out
    }

    fn decode_body(body: &[u8]) -> Result<Self, CodecError> {
        if body.len() < REQUEST_BODY_LEN {
            return Err(CodecError::Malformed(
                "request sub-packet shorter than 26 bytes",
            ));
        }
        let command = Command::from_bytes(body).ok_or(CodecError::Malformed("no command"))?;
        let mut args = [0i32; 5];
        for (i, a) in args.iter_mut().enumerate() {
            let o = 6 + i * 4;
            *a = i32::from_be_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        }
        Ok(SubRequest {
            command,
            args,
            extra: body[REQUEST_BODY_LEN..].to_vec(),
        })
    }
}

/// One response sub-packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubResponse {
    pub command: Command,
    /// `EW_*` return code; 0 is success.
    pub error: i16,
    /// Empty when `error` is not 0.
    pub payload: Vec<u8>,
}

impl SubResponse {
    pub fn ok(command: Command, payload: Vec<u8>) -> Self {
        SubResponse {
            command,
            error: 0,
            payload,
        }
    }

    pub fn failed(command: Command, error: i16) -> Self {
        SubResponse {
            command,
            error,
            payload: Vec::new(),
        }
    }

    fn encode_body(&self) -> Result<Vec<u8>, CodecError> {
        let len = u16::try_from(self.payload.len())
            .map_err(|_| CodecError::PayloadTooLarge(self.payload.len()))?;
        let mut out = Vec::with_capacity(RESPONSE_HEAD_LEN + self.payload.len());
        out.extend_from_slice(&self.command.to_bytes());
        out.extend_from_slice(&self.error.to_be_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    fn decode_body(body: &[u8]) -> Result<Self, CodecError> {
        if body.len() < RESPONSE_HEAD_LEN {
            return Err(CodecError::Malformed(
                "response sub-packet shorter than 14 bytes",
            ));
        }
        let command = Command::from_bytes(body).ok_or(CodecError::Malformed("no command"))?;
        let error = i16::from_be_bytes([body[6], body[7]]);
        if error != 0 {
            return Ok(SubResponse::failed(command, error));
        }
        let len = usize::from(u16::from_be_bytes([body[12], body[13]]));
        let end = RESPONSE_HEAD_LEN + len;
        if body.len() < end {
            return Err(CodecError::Malformed("payload longer than its sub-packet"));
        }
        Ok(SubResponse::ok(
            command,
            body[RESPONSE_HEAD_LEN..end].to_vec(),
        ))
    }
}

fn encode_subpackets(bodies: Vec<Vec<u8>>) -> Result<Vec<u8>, CodecError> {
    let count = u16::try_from(bodies.len())
        .map_err(|_| CodecError::Malformed("more than 65535 sub-packets"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_be_bytes());
    for body in bodies {
        let len =
            u16::try_from(body.len() + 2).map_err(|_| CodecError::PayloadTooLarge(body.len()))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&body);
    }
    Ok(out)
}

fn split_subpackets(payload: &[u8]) -> Result<Vec<&[u8]>, CodecError> {
    if payload.len() < 2 {
        return Err(CodecError::Malformed("no sub-packet count"));
    }
    let count = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
    let mut pos = 2;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if payload.len() < pos + 2 {
            return Err(CodecError::Malformed("sub-packet length missing"));
        }
        let len = usize::from(u16::from_be_bytes([payload[pos], payload[pos + 1]]));
        if len < 2 || payload.len() < pos + len {
            return Err(CodecError::Malformed("sub-packet longer than the frame"));
        }
        out.push(&payload[pos + 2..pos + len]);
        pos += len;
    }
    Ok(out)
}

/// Variable-data request frame carrying `subs` (one or several, in one packet).
pub fn encode_var_request(subs: &[SubRequest]) -> Result<Frame, CodecError> {
    let payload = encode_subpackets(subs.iter().map(SubRequest::encode_body).collect())?;
    Ok(Frame::new(VERSION_REQUEST, FrameType::VarRequest, payload))
}

/// The sub-packets of a variable-data request frame.
pub fn decode_var_request(frame: &Frame) -> Result<Vec<SubRequest>, CodecError> {
    expect_type(frame, FrameType::VarRequest)?;
    split_subpackets(&frame.payload)?
        .into_iter()
        .map(SubRequest::decode_body)
        .collect()
}

/// Variable-data response frame carrying `subs`.
pub fn encode_var_response(subs: &[SubResponse]) -> Result<Frame, CodecError> {
    let bodies = subs
        .iter()
        .map(SubResponse::encode_body)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Frame::new(
        VERSION_RESPONSE,
        FrameType::VarResponse,
        encode_subpackets(bodies)?,
    ))
}

/// The sub-packets of a variable-data response frame.
pub fn decode_var_response(frame: &Frame) -> Result<Vec<SubResponse>, CodecError> {
    expect_type(frame, FrameType::VarResponse)?;
    split_subpackets(&frame.payload)?
        .into_iter()
        .map(SubResponse::decode_body)
        .collect()
}

fn expect_type(frame: &Frame, expected: FrameType) -> Result<(), CodecError> {
    if frame.frame_type == expected {
        Ok(())
    } else {
        Err(CodecError::UnexpectedFrame {
            expected: expected.code(),
            actual: frame.frame_type.code(),
        })
    }
}

/// Size of one scaled number on the wire.
pub const SCALED_LEN: usize = 8;

/// Decode an 8-byte scaled number: `i32` mantissa, a zero byte, the base (2 or 10), a flag byte
/// and the exponent. The value is `mantissa / base^exponent`.
///
/// Returns `None` when the CNC marks the slot invalid (flag and exponent bytes both `ff`, as
/// pyfanuc tests) or the base is neither 2 nor 10.
pub fn decode_scaled(b: &[u8]) -> Option<f64> {
    if b.len() != SCALED_LEN || (b[6] == 0xff && b[7] == 0xff) {
        return None;
    }
    let base = match b[5] {
        2 => 2.0f64,
        10 => 10.0f64,
        _ => return None,
    };
    let mantissa = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Some(f64::from(mantissa) / base.powi(i32::from(b[7])))
}

/// Encode `value` as a base-10 scaled number with `decimals` decimal places, rounding and
/// saturating the mantissa to `i32`.
pub fn encode_scaled(value: f64, decimals: u8) -> [u8; SCALED_LEN] {
    let scaled = (value * 10f64.powi(i32::from(decimals))).round();
    let mantissa = if scaled.is_nan() {
        0
    } else {
        scaled.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    };
    let m = mantissa.to_be_bytes();
    [m[0], m[1], m[2], m[3], 0, 10, 0, decimals]
}

/// Like [`encode_scaled`], with the fewest decimal places (0 to 8) that represent `value`
/// exactly, or the most the 32-bit mantissa allows.
pub fn encode_scaled_auto(value: f64) -> [u8; SCALED_LEN] {
    let mut decimals = 0u8;
    for d in 0..=8u8 {
        let scaled = value * 10f64.powi(i32::from(d));
        if scaled.is_nan() || scaled.abs() > f64::from(i32::MAX) {
            break;
        }
        decimals = d;
        if (scaled - scaled.round()).abs() <= 1e-9 * scaled.abs().max(1.0) {
            break;
        }
    }
    encode_scaled(value, decimals)
}

/// The slot a CNC sends for a value it cannot provide.
pub const SCALED_INVALID: [u8; SCALED_LEN] = [0, 0, 0, 0, 0, 10, 0xff, 0xff];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Parse "a0 a0 ..." (whitespace separated hex bytes).
    pub(crate) fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|t| u8::from_str_radix(t, 16).expect("hex byte"))
            .collect()
    }

    #[test]
    fn encodes_open_and_close_like_pyfanuc() {
        assert_eq!(
            Frame::open_request(CHANNEL_CONTROL).encode().unwrap(),
            hex("a0 a0 a0 a0 00 01 01 01 00 02 00 02")
        );
        // README: the program-transfer stream opens with the same packet and a trailing 00 01.
        assert_eq!(
            Frame::open_request(CHANNEL_PROGRAM).encode().unwrap(),
            hex("a0 a0 a0 a0 00 01 01 01 00 02 00 01")
        );
        assert_eq!(
            Frame::close_request().encode().unwrap(),
            hex("a0 a0 a0 a0 00 01 02 01 00 00")
        );
    }

    #[test]
    fn decode_requires_exact_length_and_sync() {
        let ok = hex("a0 a0 a0 a0 00 02 02 02 00 00");
        let f = Frame::decode(&ok).unwrap();
        assert_eq!(f.frame_type, FrameType::CloseResponse);
        assert!(f.payload.is_empty());

        assert_eq!(Frame::decode(&ok[..9]), Err(CodecError::Truncated));
        assert_eq!(
            Frame::decode(&hex("a0 a0 a0 a1 00 02 02 02 00 00")),
            Err(CodecError::BadSync)
        );
        let mut long = ok.clone();
        long.push(0);
        assert!(matches!(
            Frame::decode(&long),
            Err(CodecError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn parse_prefix_streams() {
        let mut two = hex("a0 a0 a0 a0 00 01 01 01 00 02 00 02");
        two.extend(hex("a0 a0 a0 a0 00 01 02 01 00 00"));
        assert_eq!(Frame::parse_prefix(&two[..11]).unwrap(), None);
        let (f1, n1) = Frame::parse_prefix(&two).unwrap().unwrap();
        assert_eq!(f1.frame_type, FrameType::OpenRequest);
        assert_eq!(f1.open_channel(), Some(2));
        let (f2, n2) = Frame::parse_prefix(&two[n1..]).unwrap().unwrap();
        assert_eq!(f2.frame_type, FrameType::CloseRequest);
        assert_eq!(n1 + n2, two.len());
    }

    /// pyfanuc `sample_multipacket.md`: four sub-packets in one request.
    #[test]
    fn multipacket_request_matches_sample() {
        let sample = hex("a0 a0 a0 a0 00 01 21 01 00 72 00 04
             00 1c 00 01 00 01 00 a4 00 00 00 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 89 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 56 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 a4 00 00 00 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        let subs = vec![
            SubRequest::new(Command::cnc(0xa4), [2, 0, 0, 0, 0]),
            SubRequest::new(Command::cnc(0x89), [0, 0, 0, 0, 0]),
            SubRequest::new(Command::cnc(0x56), [1, 0, 0, 0, 0]),
            SubRequest::new(Command::cnc(0xa4), [2, 0, 0, 0, 0]),
        ];
        assert_eq!(encode_var_request(&subs).unwrap().encode().unwrap(), sample);
        // and back
        let frame = Frame::decode(&sample).unwrap();
        assert_eq!(decode_var_request(&frame).unwrap(), subs);
    }

    /// pyfanuc `sample_with_error.md`: nine sub-packets, positions and alarm/program reads.
    #[test]
    fn nine_subpacket_request_matches_sample() {
        let sample = hex("a0 a0 a0 a0 00 01 21 01 00 fe 00 09
             00 1c 00 01 00 01 00 1a 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 1c 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 1d 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 24 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 25 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 26 00 00 00 04 ff ff ff ff 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 26 00 00 00 01 ff ff ff ff 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 26 00 00 00 06 ff ff ff ff 00 00 00 00 00 00 00 00 00 00 00 00
             00 1c 00 01 00 01 00 26 00 00 00 07 ff ff ff ff 00 00 00 00 00 00 00 00 00 00 00 00");
        let pos = |kind| SubRequest::new(Command::cnc(0x26), [kind, -1, 0, 0, 0]);
        let subs = vec![
            SubRequest::new(Command::cnc(0x1a), [0; 5]),
            SubRequest::new(Command::cnc(0x1c), [0; 5]),
            SubRequest::new(Command::cnc(0x1d), [0; 5]),
            SubRequest::new(Command::cnc(0x24), [0; 5]),
            SubRequest::new(Command::cnc(0x25), [0; 5]),
            pos(4),
            pos(1),
            pos(6),
            pos(7),
        ];
        assert_eq!(encode_var_request(&subs).unwrap().encode().unwrap(), sample);
        let frame = Frame::decode(&sample).unwrap();
        assert_eq!(decode_var_request(&frame).unwrap(), subs);
    }

    /// pyfanuc README: PMC read of D2204 as a 16-bit value.
    #[test]
    fn pmc_request_and_response_match_readme() {
        let req = hex("a0 a0 a0 a0 00 01 21 01 00 1e 00 01 00 1c
             00 02 00 01 80 01 00 00 08 9c 00 00 08 9d 00 00 00 09 00 00 00 01 00 00 00 00");
        let sub = SubRequest::new(Command::pmc(0x8001), [2204, 2205, 9, 1, 0]);
        assert_eq!(
            encode_var_request(std::slice::from_ref(&sub))
                .unwrap()
                .encode()
                .unwrap(),
            req
        );
        assert_eq!(
            decode_var_request(&Frame::decode(&req).unwrap()).unwrap(),
            vec![sub]
        );

        let resp = hex("a0 a0 a0 a0 00 02 21 02 00 14 00 01 00 12
             00 02 00 01 80 01 00 00 00 00 00 00 00 02 02 58");
        let got = decode_var_response(&Frame::decode(&resp).unwrap()).unwrap();
        assert_eq!(
            got,
            vec![SubResponse::ok(Command::pmc(0x8001), vec![0x02, 0x58])]
        );
        assert_eq!(encode_var_response(&got).unwrap().encode().unwrap(), resp);
    }

    /// pyfanuc `sample_with_error.md` response: errors (1 and 6) are sub-packets of their own.
    #[test]
    fn nine_subpacket_response_matches_sample() {
        let sample = hex("a0 a0 a0 a0 00 02 21 02 01 6a 00 09
             00 14 00 01 00 01 00 1a 00 00 00 00 00 00 00 04 00 00 00 00
             00 18 00 01 00 01 00 1c 00 00 00 00 00 00 00 08 00 00 0b b8 00 00 0b b8
             00 14 00 01 00 01 00 1d 00 00 00 00 00 00 00 04 00 00 00 00
             00 18 00 01 00 01 00 24 00 00 00 00 00 00 00 08 00 00 00 00 00 0a 00 04
             00 10 00 01 00 01 00 25 00 01 00 00 00 00 00 00
             00 50 00 01 00 01 00 26 00 00 00 00 00 00 00 40
                00 2f 7a e9 00 0a 00 04 00 02 bc 02 00 0a 00 04
                00 00 00 00 00 0a 00 04 00 00 00 00 00 0a 00 04
                ff ec ed 30 00 0a 00 04 00 36 ee 80 00 0a 00 04
                00 00 00 00 00 0a 00 04 00 00 00 00 00 0a 00 04
             00 50 00 01 00 01 00 26 00 00 00 00 00 00 00 40
                ff fd b0 bd 00 0a 00 04 ff ed 51 11 00 0a 00 04
                ff ff f7 ad 00 0a 00 04 ff ff ff 9d 00 0a 00 04
                ff ec ed 30 00 0a 00 04 00 32 2a 47 00 0a 00 04
                00 00 00 00 00 0a 00 04 00 00 00 00 00 0a 00 04
             00 50 00 01 00 01 00 26 00 00 00 00 00 00 00 40
                00 16 d1 05 00 0a 00 04 00 04 78 0b 00 0a 00 04
                ff ff fe 62 00 0a 00 04 00 00 10 af 00 0a 00 04
                00 0a e5 67 00 0a 00 04 e4 18 b6 0e 00 0a 00 04
                00 00 00 00 00 0a 00 04 00 00 00 00 00 0a 00 04
             00 10 00 01 00 01 00 26 00 06 00 00 00 00 00 00");
        let frame = Frame::decode(&sample).unwrap();
        let subs = decode_var_response(&frame).unwrap();
        assert_eq!(subs.len(), 9);
        assert_eq!(subs[0].payload, vec![0, 0, 0, 0]);
        assert_eq!(subs[1].payload, vec![0, 0, 0x0b, 0xb8, 0, 0, 0x0b, 0xb8]);
        assert_eq!(subs[4], SubResponse::failed(Command::cnc(0x25), 1));
        assert_eq!(subs[8], SubResponse::failed(Command::cnc(0x26), 6));
        assert_eq!(subs[5].payload.len(), 64);
        // The codec reproduces the capture byte for byte (the sample's version field is 2).
        assert_eq!(
            encode_var_response(&subs).unwrap().encode().unwrap(),
            sample
        );
    }

    #[test]
    fn scaled_numbers() {
        // 0x002f7ae9 = 3111657, four decimals (first X position of the sample).
        let v = decode_scaled(&hex("00 2f 7a e9 00 0a 00 04")).unwrap();
        assert!((v - 311.1657).abs() < 1e-9);
        // negative, from the same sample
        let v = decode_scaled(&hex("ff ec ed 30 00 0a 00 04")).unwrap();
        assert!((v + 125.0).abs() < 1e-9, "{v}");
        // pyfocas doctest: 0x0bc77120 with six decimals
        let v = decode_scaled(&hex("0b c7 71 20 00 0a 00 06")).unwrap();
        assert!((v - 197.62).abs() < 1e-9);
        // base 2 is accepted, other bases and the ff ff flag are not
        assert_eq!(decode_scaled(&hex("00 00 00 08 00 02 00 02")), Some(2.0));
        assert_eq!(decode_scaled(&hex("00 00 00 01 00 03 00 01")), None);
        assert_eq!(decode_scaled(&hex("00 00 00 01 00 0a ff ff")), None);
        assert_eq!(decode_scaled(&hex("00 00 00 01 00 0a")), None);
        // round trip
        let b = encode_scaled(-12.3456, 4);
        assert_eq!(b, [0xff, 0xfe, 0x1d, 0xc0, 0, 10, 0, 4]);
    }

    #[test]
    fn scaled_round_trip() {
        for (v, d) in [
            (0.0, 0u8),
            (311.1657, 4),
            (-0.001, 3),
            (1.0e12, 3),
            (42.0, 0),
        ] {
            let back = decode_scaled(&encode_scaled(v, d)).unwrap();
            if v.abs() < 1.0e5 {
                assert!(
                    (back - v).abs() < 10f64.powi(-i32::from(d)),
                    "{v} {d} -> {back}"
                );
            } else {
                // saturates to the i32 range
                assert!(back > 0.0);
            }
        }
        assert_eq!(decode_scaled(&SCALED_INVALID), None);
    }
}
