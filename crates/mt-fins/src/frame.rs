//! FINS command and response frames, and the FINS/TCP envelope.
//!
//! FINS frame: `ICF RSV GCT DNA DA1 DA2 SNA SA1 SA2 SID | MRC SRC | parameters` for a command; a response has the
//! same header (ICF 0xC0, source and destination swapped), the command code, then `MRES SRES` and data. All
//! multi-byte numbers are big-endian.

use serde::{Deserialize, Serialize};

use crate::area::{FinsAddress, FinsArea};

pub const HEADER_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FinsHeader {
    pub icf: u8,
    pub rsv: u8,
    pub gct: u8,
    pub dna: u8,
    pub da1: u8,
    pub da2: u8,
    pub sna: u8,
    pub sa1: u8,
    pub sa2: u8,
    pub sid: u8,
}

impl FinsHeader {
    /// A command header that asks for a response.
    pub fn command(dna: u8, da1: u8, da2: u8, sna: u8, sa1: u8, sa2: u8, sid: u8) -> Self {
        Self { icf: 0x80, rsv: 0, gct: 0x02, dna, da1, da2, sna, sa1, sa2, sid }
    }

    /// The response header for a command header.
    pub fn reply(&self) -> Self {
        Self {
            icf: 0xC0,
            rsv: 0,
            gct: 0x02,
            dna: self.sna,
            da1: self.sa1,
            da2: self.sa2,
            sna: self.dna,
            sa1: self.da1,
            sa2: self.da2,
            sid: self.sid,
        }
    }

    pub fn is_response(&self) -> bool {
        self.icf & 0x40 != 0
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[self.icf, self.rsv, self.gct, self.dna, self.da1, self.da2, self.sna, self.sa1, self.sa2, self.sid]);
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER_LEN {
            return None;
        }
        Some(Self { icf: b[0], rsv: b[1], gct: b[2], dna: b[3], da1: b[4], da2: b[5], sna: b[6], sa1: b[7], sa2: b[8], sid: b[9] })
    }
}

pub mod code {
    pub const MEMORY_READ: u16 = 0x0101;
    pub const MEMORY_WRITE: u16 = 0x0102;
    pub const RUN: u16 = 0x0401;
    pub const STOP: u16 = 0x0402;
    pub const CONTROLLER_DATA_READ: u16 = 0x0501;
    pub const CONTROLLER_STATUS_READ: u16 = 0x0601;
}

/// End codes (MRES SRES with the flag bits masked off).
pub mod end {
    pub const NORMAL: u16 = 0x0000;
    pub const UNDEFINED_COMMAND: u16 = 0x0401;
    pub const COMMAND_TOO_LONG: u16 = 0x1001;
    pub const COMMAND_TOO_SHORT: u16 = 0x1002;
    pub const ELEMENTS_DATA_MISMATCH: u16 = 0x1003;
    pub const NO_AREA_TYPE: u16 = 0x1101;
    pub const ADDRESS_RANGE: u16 = 0x1103;
    pub const ADDRESS_RANGE_EXCEEDED: u16 = 0x1104;
    pub const PARAMETER_ERROR: u16 = 0x110C;
    pub const READ_ONLY: u16 = 0x2101;
}

pub fn end_code_text(code: u16) -> Option<&'static str> {
    Some(match code {
        0x0000 => "normal completion",
        0x0001 => "service canceled",
        0x0101 => "local node is not in the network",
        0x0102 => "token timeout",
        0x0103 => "retries failed",
        0x0105 => "node address setting error (check the FINS node numbers)",
        0x0106 => "the same node address is used twice on the network",
        0x0201 => "destination node is not in the network",
        0x0202 => "no such unit (check DA2, 0 is the CPU)",
        0x0204 => "destination node is busy",
        0x0205 => "response timeout from the destination",
        0x0301 => "communications controller error",
        0x0401 => "undefined command",
        0x0402 => "not supported by this model or version",
        0x0501 => "destination address setting error (check DNA, DA1 and the routing tables)",
        0x0502 => "no routing tables",
        0x0503 => "routing table error",
        0x1001 => "command too long",
        0x1002 => "command too short",
        0x1003 => "the number of elements does not match the data",
        0x1004 => "command format error",
        0x1005 => "header error",
        0x1101 => "no such memory area on this CPU",
        0x1102 => "access size error",
        0x1103 => "address out of range",
        0x1104 => "address range exceeded (the read or write runs past the end of the area)",
        0x1106 => "program number does not exist",
        0x110B => "the response would be too long (read fewer words)",
        0x110C => "parameter error",
        0x2002 => "the program area is protected",
        0x2101 => "the area is read-only",
        0x2102 => "the area is write-protected",
        0x2201 => "not possible in the current operating mode",
        0x2202 => "not possible while the CPU is running",
        0x2203 => "the PLC is in PROGRAM mode",
        0x2205 => "the PLC is in MONITOR mode",
        0x2206 => "the PLC is in RUN mode",
        0x2502 => "memory error",
        0x2601 => "no protection",
        0x2602 => "wrong password",
        0x3001 => "no access right (another device holds it)",
        _ => return None,
    })
}

/// End code from MRES/SRES: bit 7 of MRES flags a relay error and bits 6 and 7 of SRES the PLC's own non-fatal and
/// fatal errors; none of them belong to the code itself.
pub fn end_code(mres: u8, sres: u8) -> u16 {
    (((mres & 0x7F) as u16) << 8) | (sres & 0x3F) as u16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Monitor,
    Run,
}

/// The commands this crate speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    ReadWords { at: FinsAddress, count: u16 },
    ReadBits { at: FinsAddress, count: u16 },
    WriteWords { at: FinsAddress, values: Vec<u16> },
    WriteBits { at: FinsAddress, values: Vec<bool> },
    Run(RunMode),
    Stop,
    ControllerData,
    ControllerStatus,
}

fn put_area(out: &mut Vec<u8>, at: &FinsAddress, bit_access: bool) -> Result<(), String> {
    let area_code = if bit_access {
        at.area.bit_code().ok_or_else(|| format!("{} has no bit access", at.area.name()))?
    } else {
        at.area.word_code()
    };
    out.push(area_code);
    out.extend_from_slice(&(at.word.wrapping_add(at.area.wire_offset())).to_be_bytes());
    out.push(if bit_access { at.bit.unwrap_or(0) } else { 0 });
    Ok(())
}

impl Command {
    pub fn code(&self) -> u16 {
        match self {
            Command::ReadWords { .. } | Command::ReadBits { .. } => code::MEMORY_READ,
            Command::WriteWords { .. } | Command::WriteBits { .. } => code::MEMORY_WRITE,
            Command::Run(_) => code::RUN,
            Command::Stop => code::STOP,
            Command::ControllerData => code::CONTROLLER_DATA_READ,
            Command::ControllerStatus => code::CONTROLLER_STATUS_READ,
        }
    }

    /// Command code and parameters.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut out = self.code().to_be_bytes().to_vec();
        match self {
            Command::ReadWords { at, count } => {
                put_area(&mut out, at, false)?;
                out.extend_from_slice(&count.to_be_bytes());
            }
            Command::ReadBits { at, count } => {
                put_area(&mut out, at, true)?;
                out.extend_from_slice(&count.to_be_bytes());
            }
            Command::WriteWords { at, values } => {
                put_area(&mut out, at, false)?;
                out.extend_from_slice(&(values.len() as u16).to_be_bytes());
                for v in values {
                    out.extend_from_slice(&v.to_be_bytes());
                }
            }
            Command::WriteBits { at, values } => {
                put_area(&mut out, at, true)?;
                out.extend_from_slice(&(values.len() as u16).to_be_bytes());
                out.extend(values.iter().map(|b| *b as u8));
            }
            Command::Run(mode) => {
                out.extend_from_slice(&[0xFF, 0xFF]);
                out.push(match mode {
                    RunMode::Monitor => 0x02,
                    RunMode::Run => 0x04,
                });
            }
            Command::Stop => out.extend_from_slice(&[0xFF, 0xFF]),
            Command::ControllerData | Command::ControllerStatus => {}
        }
        Ok(out)
    }

    /// Server side: decode command code and parameters. Errors are end codes.
    pub fn decode(b: &[u8]) -> Result<Command, u16> {
        if b.len() < 2 {
            return Err(end::COMMAND_TOO_SHORT);
        }
        let code = u16::from_be_bytes([b[0], b[1]]);
        let p = &b[2..];
        let area = |p: &[u8]| -> Result<(FinsAddress, bool, usize), u16> {
            if p.len() < 6 {
                return Err(end::COMMAND_TOO_SHORT);
            }
            let wire = u16::from_be_bytes([p[1], p[2]]);
            let (area, bit, word) = FinsArea::from_code(p[0], wire).ok_or(end::NO_AREA_TYPE)?;
            if bit && p[3] > 15 {
                return Err(end::ADDRESS_RANGE);
            }
            let count = u16::from_be_bytes([p[4], p[5]]) as usize;
            Ok((FinsAddress { area, word, bit: bit.then_some(p[3]) }, bit, count))
        };
        Ok(match code {
            code::MEMORY_READ => {
                let (at, bit, count) = area(p)?;
                if p.len() > 6 {
                    return Err(end::COMMAND_TOO_LONG);
                }
                if bit { Command::ReadBits { at, count: count as u16 } } else { Command::ReadWords { at, count: count as u16 } }
            }
            code::MEMORY_WRITE => {
                let (at, bit, count) = area(p)?;
                let data = &p[6..];
                if bit {
                    if data.len() != count {
                        return Err(end::ELEMENTS_DATA_MISMATCH);
                    }
                    Command::WriteBits { at, values: data.iter().map(|b| *b & 1 != 0).collect() }
                } else {
                    if data.len() != count * 2 {
                        return Err(end::ELEMENTS_DATA_MISMATCH);
                    }
                    Command::WriteWords { at, values: data.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect() }
                }
            }
            code::RUN => Command::Run(match p.get(2) {
                Some(0x02) => RunMode::Monitor,
                _ => RunMode::Run,
            }),
            code::STOP => Command::Stop,
            code::CONTROLLER_DATA_READ => Command::ControllerData,
            code::CONTROLLER_STATUS_READ => Command::ControllerStatus,
            _ => return Err(end::UNDEFINED_COMMAND),
        })
    }
}

/// A full FINS command frame.
pub fn encode_command(h: &FinsHeader, c: &Command) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(32);
    h.encode(&mut out);
    out.extend(c.encode()?);
    Ok(out)
}

/// A full FINS response frame.
pub fn encode_response(h: &FinsHeader, command_code: u16, end_code: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + data.len());
    h.encode(&mut out);
    out.extend_from_slice(&command_code.to_be_bytes());
    out.extend_from_slice(&end_code.to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// A decoded response: header, command code, MRES/SRES and data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinsResponse {
    pub header: FinsHeader,
    pub command: u16,
    pub mres: u8,
    pub sres: u8,
    pub data: Vec<u8>,
}

impl FinsResponse {
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        let header = FinsHeader::decode(b).ok_or("FINS reply shorter than its header")?;
        if !header.is_response() {
            return Err("FINS frame is a command, not a response".into());
        }
        if b.len() < HEADER_LEN + 4 {
            return Err("FINS reply has no end code".into());
        }
        Ok(Self {
            header,
            command: u16::from_be_bytes([b[10], b[11]]),
            mres: b[12],
            sres: b[13],
            data: b[14..].to_vec(),
        })
    }

    pub fn end_code(&self) -> u16 {
        end_code(self.mres, self.sres)
    }
}

/// The bytes at the start of a buffer are not a FINS/TCP frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAFrame;

// ── FINS/TCP ──

pub mod tcp {
    pub use super::NotAFrame;

    pub const MAGIC: &[u8; 4] = b"FINS";
    pub const NODE_REQUEST: u32 = 0;
    pub const NODE_RESPONSE: u32 = 1;
    pub const FRAME: u32 = 2;
    pub const ERROR: u32 = 3;

    /// `FINS | length | command | error code | data`; length counts from the command field.
    pub fn envelope(command: u32, error: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + data.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
        out.extend_from_slice(&command.to_be_bytes());
        out.extend_from_slice(&error.to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    /// Total length of the envelope at the start of `buf` once its header is in; `Err` if it is not FINS/TCP.
    pub fn frame_len(buf: &[u8]) -> Result<Option<usize>, NotAFrame> {
        if buf.len() < 8 {
            return if MAGIC.starts_with(&buf[..buf.len().min(4)]) { Ok(None) } else { Err(NotAFrame) };
        }
        if &buf[..4] != MAGIC {
            return Err(NotAFrame);
        }
        let len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        if !(8..=65536).contains(&len) {
            return Err(NotAFrame);
        }
        Ok(Some(8 + len))
    }

    /// (command, error code, data) of a complete envelope.
    pub fn parse(frame: &[u8]) -> (u32, u32, &[u8]) {
        let command = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]);
        let error = u32::from_be_bytes([frame[12], frame[13], frame[14], frame[15]]);
        (command, error, &frame[16..])
    }

    pub fn error_text(code: u32) -> &'static str {
        match code {
            0x01 => "the header is not 'FINS'",
            0x02 => "the data length is too long",
            0x03 => "the command is not supported",
            0x20 => "all connections are in use",
            0x21 => "the specified node is already connected",
            0x22 => "attempt to access a protected node from an unspecified IP address",
            0x23 => "the client FINS node address is out of range",
            0x24 => "the same FINS node address is being used by the client and server",
            0x25 => "all the node addresses available for allocation have been used",
            _ => "unknown FINS/TCP error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Memory area read of D100, 2 words, from node 0x22 to PLC node 0x0A (the shape of the FINS manual's example).
    #[test]
    fn memory_area_read_frame() {
        let h = FinsHeader::command(0, 0x0A, 0, 0, 0x22, 0, 0x01);
        let f = encode_command(&h, &Command::ReadWords { at: FinsAddress::new(FinsArea::Dm, 100), count: 2 }).unwrap();
        assert_eq!(f, hex("80 00 02 00 0A 00 00 22 00 01 01 01 82 00 64 00 00 02"));
        assert_eq!(Command::decode(&f[10..]).unwrap(), Command::ReadWords { at: FinsAddress::new(FinsArea::Dm, 100), count: 2 });
    }

    #[test]
    fn bit_write_and_counter_addressing() {
        let at = FinsAddress { area: FinsArea::Cio, word: 0, bit: Some(5) };
        let c = Command::WriteBits { at, values: vec![true] };
        assert_eq!(c.encode().unwrap(), hex("01 02 30 00 00 05 00 01 01"));
        assert_eq!(Command::decode(&c.encode().unwrap()).unwrap(), c);
        let cnt = Command::ReadWords { at: FinsAddress::new(FinsArea::Cnt, 3), count: 1 };
        assert_eq!(cnt.encode().unwrap(), hex("01 01 89 80 03 00 00 01"));
        assert_eq!(Command::decode(&cnt.encode().unwrap()).unwrap(), cnt);
    }

    #[test]
    fn word_write_is_big_endian() {
        let c = Command::WriteWords { at: FinsAddress::new(FinsArea::Hr, 2), values: vec![0x1234, 0xABCD] };
        assert_eq!(c.encode().unwrap(), hex("01 02 B2 00 02 00 00 02 12 34 AB CD"));
        assert_eq!(Command::decode(&c.encode().unwrap()).unwrap(), c);
    }

    #[test]
    fn response_header_swaps_source_and_destination() {
        let h = FinsHeader::command(1, 0x0A, 0, 2, 0x22, 0, 0x33);
        let r = h.reply();
        assert_eq!((r.icf, r.dna, r.da1, r.sna, r.sa1, r.sid), (0xC0, 2, 0x22, 1, 0x0A, 0x33));
        let bytes = encode_response(&r, code::MEMORY_READ, 0, &[0x00, 0x2A]);
        let back = FinsResponse::decode(&bytes).unwrap();
        assert_eq!((back.command, back.end_code(), back.data.as_slice()), (0x0101, 0, &[0x00, 0x2A][..]));
    }

    #[test]
    fn end_code_masks_the_flag_bits() {
        // Relay error flag on MRES and a fatal-error flag on SRES around "address range exceeded".
        assert_eq!(end_code(0x91, 0x84), 0x1104);
        assert_eq!(end_code(0x00, 0x40), 0x0000);
        assert!(end_code_text(0x1104).unwrap().contains("past the end"));
    }

    #[test]
    fn malformed_commands_are_end_codes() {
        assert_eq!(Command::decode(&hex("01")), Err(end::COMMAND_TOO_SHORT));
        assert_eq!(Command::decode(&hex("07 01")), Err(end::UNDEFINED_COMMAND));
        assert_eq!(Command::decode(&hex("01 01 FF 00 00 00 00 01")), Err(end::NO_AREA_TYPE));
        assert_eq!(Command::decode(&hex("01 02 82 00 00 00 00 02 00 01")), Err(end::ELEMENTS_DATA_MISMATCH));
    }

    #[test]
    fn tcp_envelopes() {
        let req = tcp::envelope(tcp::NODE_REQUEST, 0, &[0, 0, 0, 0]);
        assert_eq!(req, hex("46 49 4E 53 00 00 00 0C 00 00 00 00 00 00 00 00 00 00 00 00"));
        assert_eq!(tcp::frame_len(&req).unwrap(), Some(20));
        assert_eq!(tcp::frame_len(b"FIN").unwrap(), None);
        assert!(tcp::frame_len(b"GET / HTTP/1.1").is_err());
        let resp = tcp::envelope(tcp::NODE_RESPONSE, 0, &[0, 0, 0, 0x22, 0, 0, 0, 0x0A]);
        assert_eq!(tcp::parse(&resp), (1, 0, &[0, 0, 0, 0x22, 0, 0, 0, 0x0A][..]));
    }
}
