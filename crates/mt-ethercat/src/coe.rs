//! CANopen over EtherCAT (ETG.1000.6 section 5.6): the mailbox header, SDO upload and download (expedited, normal,
//! segmented), SDO Information (object dictionary lists and descriptions), abort codes and data types. Requests are
//! built and responses parsed for the master; requests parsed and responses built for the simulated devices.

use serde::Serialize;

/// Mailbox header length: length, address, channel/priority, type/counter.
pub const MBX_HEADER: usize = 6;
pub const MBX_TYPE_ERROR: u8 = 0x00;
pub const MBX_TYPE_AOE: u8 = 0x01;
pub const MBX_TYPE_EOE: u8 = 0x02;
pub const MBX_TYPE_COE: u8 = 0x03;
pub const MBX_TYPE_FOE: u8 = 0x04;
pub const MBX_TYPE_SOE: u8 = 0x05;

/// CoE service numbers (upper 4 bits of the CoE header).
pub const SVC_EMERGENCY: u8 = 1;
pub const SVC_SDO_REQ: u8 = 2;
pub const SVC_SDO_RESP: u8 = 3;
pub const SVC_SDO_INFO: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbxHeader {
    /// Length of the data after this header.
    pub len: u16,
    pub address: u16,
    pub priority: u8,
    pub kind: u8,
    pub counter: u8,
}

impl MbxHeader {
    pub fn parse(b: &[u8]) -> Option<MbxHeader> {
        if b.len() < MBX_HEADER {
            return None;
        }
        Some(MbxHeader {
            len: u16::from_le_bytes([b[0], b[1]]),
            address: u16::from_le_bytes([b[2], b[3]]),
            priority: b[4] >> 6,
            kind: b[5] & 0x0F,
            counter: (b[5] >> 4) & 0x07,
        })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.len.to_le_bytes());
        out.extend_from_slice(&self.address.to_le_bytes());
        out.push(self.priority << 6);
        out.push(self.kind & 0x0F | (self.counter & 0x07) << 4);
    }
}

/// A whole mailbox message: header plus `body`.
pub fn mailbox(kind: u8, counter: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MBX_HEADER + body.len());
    MbxHeader { len: body.len() as u16, address: 0, priority: 0, kind, counter }.write(&mut out);
    out.extend_from_slice(body);
    out
}

/// The CoE header: a 9-bit number (0 for SDO) and the service.
pub fn coe_header(service: u8) -> [u8; 2] {
    ((service as u16) << 12).to_le_bytes()
}

pub fn coe_service(body: &[u8]) -> Option<u8> {
    (body.len() >= 2).then(|| body[1] >> 4)
}

/// SDO abort codes (ETG.1000.6 table 41, CiA 301).
pub fn abort_text(code: u32) -> &'static str {
    match code {
        0x0503_0000 => "Toggle bit not changed",
        0x0504_0000 => "SDO protocol timeout",
        0x0504_0001 => "Client/server command specifier not valid or unknown",
        0x0504_0005 => "Out of memory",
        0x0601_0000 => "Unsupported access to an object",
        0x0601_0001 => "Attempt to read a write-only object",
        0x0601_0002 => "Attempt to write a read-only object",
        0x0601_0003 => "Subindex cannot be written, SI0 must be 0 for write access",
        0x0601_0004 => "SDO complete access not supported for variable-length objects",
        0x0601_0005 => "Object length exceeds mailbox size",
        0x0601_0006 => "Object mapped to RxPDO, SDO download blocked",
        0x0602_0000 => "The object does not exist in the object dictionary",
        0x0604_0041 => "The object cannot be mapped into the PDO",
        0x0604_0042 => "The number and length of the objects to be mapped would exceed the PDO length",
        0x0604_0043 => "General parameter incompatibility",
        0x0604_0047 => "General internal incompatibility in the device",
        0x0606_0000 => "Access failed due to a hardware error",
        0x0607_0010 => "Data type does not match, length of service parameter does not match",
        0x0607_0012 => "Data type does not match, length of service parameter too high",
        0x0607_0013 => "Data type does not match, length of service parameter too low",
        0x0609_0011 => "Subindex does not exist",
        0x0609_0030 => "Value range of parameter exceeded",
        0x0609_0031 => "Value of parameter written too high",
        0x0609_0032 => "Value of parameter written too low",
        0x0609_0036 => "Maximum value is less than minimum value",
        0x0800_0000 => "General error",
        0x0800_0020 => "Data cannot be transferred or stored to the application",
        0x0800_0021 => "Data cannot be transferred or stored because of local control",
        0x0800_0022 => "Data cannot be transferred or stored in the present device state",
        0x0800_0023 => "Object dictionary dynamic generation failed or no object dictionary present",
        _ => "Unknown abort code",
    }
}

/// Mailbox error codes (mailbox type 0, ETG.1000.4 table 30).
pub fn mailbox_error_text(code: u16) -> &'static str {
    match code {
        0x0001 => "Syntax error in the mailbox header",
        0x0002 => "Mailbox protocol not supported",
        0x0003 => "Channel field invalid",
        0x0004 => "Service not supported",
        0x0005 => "Invalid mailbox header",
        0x0006 => "Mailbox data too short",
        0x0007 => "No more memory",
        0x0008 => "Inconsistent data length",
        _ => "Unknown mailbox error",
    }
}

/// CoE data types (ETG.1000.6 table 63 and ETG.1020 base types).
pub fn data_type_name(dt: u16) -> String {
    let s = match dt {
        0x0001 => "BOOL",
        0x0002 => "SINT",
        0x0003 => "INT",
        0x0004 => "DINT",
        0x0005 => "USINT",
        0x0006 => "UINT",
        0x0007 => "UDINT",
        0x0008 => "REAL",
        0x0009 => "STRING",
        0x000A => "OCTET_STRING",
        0x000B => "UNICODE_STRING",
        0x000C => "TIME_OF_DAY",
        0x000D => "TIME_DIFFERENCE",
        0x000F => "DOMAIN",
        0x0010 => "INT24",
        0x0011 => "LREAL",
        0x0012 => "INT40",
        0x0013 => "INT48",
        0x0014 => "INT56",
        0x0015 => "LINT",
        0x0016 => "UINT24",
        0x0018 => "UINT40",
        0x0019 => "UINT48",
        0x001A => "UINT56",
        0x001B => "ULINT",
        0x001E => "BYTE",
        0x001F => "WORD",
        0x0020 => "DWORD",
        0x0021 => "PDO_MAPPING",
        0x0023 => "IDENTITY",
        0x0025 => "COMMAND_PAR",
        0x0029 => "SYNC_PAR",
        0x0030..=0x0037 => return format!("BIT{}", dt - 0x002F),
        0x0000 => "NULL",
        _ => return format!("0x{dt:04X}"),
    };
    s.to_string()
}

/// Bit length of a basic data type, `None` for strings and structured types.
pub fn data_type_bits(dt: u16) -> Option<u16> {
    Some(match dt {
        0x0001 => 1,
        0x0002 | 0x0005 | 0x001E => 8,
        0x0003 | 0x0006 | 0x001F => 16,
        0x0010 | 0x0016 => 24,
        0x0004 | 0x0007 | 0x0008 | 0x0020 => 32,
        0x0012 | 0x0018 => 40,
        0x0013 | 0x0019 => 48,
        0x0014 | 0x001A => 56,
        0x0011 | 0x0015 | 0x001B => 64,
        0x0030..=0x0037 => dt - 0x002F,
        _ => return None,
    })
}

/// Formats a little-endian value of type `dt` for display.
pub fn format_value(dt: u16, b: &[u8]) -> String {
    fn le(b: &[u8]) -> u64 {
        b.iter().take(8).enumerate().fold(0u64, |a, (i, &x)| a | (x as u64) << (8 * i))
    }
    fn sext(v: u64, bits: u32) -> i64 {
        let s = 64 - bits;
        ((v << s) as i64) >> s
    }
    if b.is_empty() {
        return String::new();
    }
    match dt {
        0x0001 => (if b[0] & 1 != 0 { "TRUE" } else { "FALSE" }).to_string(),
        0x0002 | 0x0003 | 0x0004 | 0x0010 | 0x0012 | 0x0013 | 0x0014 | 0x0015 => {
            let bits = data_type_bits(dt).unwrap_or(64) as u32;
            sext(le(b), bits.min(8 * b.len() as u32)).to_string()
        }
        0x0005 | 0x0006 | 0x0007 | 0x0016 | 0x0018 | 0x0019 | 0x001A | 0x001B => le(b).to_string(),
        0x001E | 0x001F | 0x0020 | 0x0030..=0x0037 => format!("0x{:0width$X}", le(b), width = 2 * b.len().min(8)),
        0x0008 if b.len() >= 4 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
        0x0011 if b.len() >= 8 => f64::from_le_bytes(b[..8].try_into().unwrap()).to_string(),
        0x0009 => String::from_utf8_lossy(b).trim_end_matches('\0').to_string(),
        _ => b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" "),
    }
}

/// Parses `text` as a value of type `dt` for an SDO download or a process data write. Integers take decimal or
/// `0x` hex; BOOL takes true/false/1/0; strings are sent as typed; other types take hex bytes (`01 02 0A`).
pub fn parse_value(dt: u16, bits: u16, text: &str) -> Result<Vec<u8>, String> {
    let t = text.trim();
    let bytes = (bits as usize).div_ceil(8).max(1);
    let int = |t: &str| -> Result<i128, String> {
        let (neg, body) = t.strip_prefix('-').map(|r| (true, r)).unwrap_or((false, t));
        let v = if let Some(h) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            i128::from_str_radix(&h.replace('_', ""), 16)
        } else {
            body.replace('_', "").parse::<i128>()
        }
        .map_err(|_| format!("Not a number: {t}"))?;
        Ok(if neg { -v } else { v })
    };
    match dt {
        0x0001 => match t.to_ascii_lowercase().as_str() {
            "1" | "true" | "on" => Ok(vec![1]),
            "0" | "false" | "off" => Ok(vec![0]),
            _ => Err(format!("Not a BOOL: {t}")),
        },
        0x0008 => t.parse::<f32>().map(|v| v.to_le_bytes().to_vec()).map_err(|_| format!("Not a REAL: {t}")),
        0x0011 => t.parse::<f64>().map(|v| v.to_le_bytes().to_vec()).map_err(|_| format!("Not an LREAL: {t}")),
        0x0009 => Ok(t.as_bytes().to_vec()),
        _ if data_type_bits(dt).is_some() => {
            let v = int(t)?;
            let signed = matches!(dt, 0x0002 | 0x0003 | 0x0004 | 0x0010 | 0x0012 | 0x0013 | 0x0014 | 0x0015);
            let b = bits.clamp(1, 64) as u32;
            let (min, max) = if signed {
                (-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1)
            } else {
                (0, (1i128 << b) - 1)
            };
            if v < min || v > max {
                return Err(format!("{t} is outside {min}..{max}"));
            }
            Ok((v as u64).to_le_bytes()[..bytes].to_vec())
        }
        _ => {
            let hex: String = t.chars().filter(|c| !c.is_whitespace()).collect();
            let hex = hex.strip_prefix("0x").unwrap_or(&hex);
            if hex.len() % 2 != 0 {
                return Err("Hex bytes need an even number of digits".into());
            }
            (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| format!("Not hex: {t}"))).collect()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// SDO, master side

/// Upload (read) request: `[CoE header][cmd][index][sub][4 reserved]`.
pub fn upload_request(index: u16, sub: u8) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_REQ).to_vec();
    b.push(2 << 5);
    b.extend_from_slice(&index.to_le_bytes());
    b.push(sub);
    b.extend_from_slice(&[0; 4]);
    b
}

/// Upload segment request with the toggle bit.
pub fn upload_segment_request(toggle: bool) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_REQ).to_vec();
    b.push(3 << 5 | (toggle as u8) << 4);
    b.extend_from_slice(&[0; 7]);
    b
}

/// Download (write) request: expedited for up to 4 bytes, else normal with the complete size and as much data as
/// fits in `mbx_size`; the rest goes in segments ([`download_segment_request`]). Returns the request and how many
/// data bytes it carries.
pub fn download_request(index: u16, sub: u8, data: &[u8], mbx_size: usize) -> (Vec<u8>, usize) {
    let mut b = coe_header(SVC_SDO_REQ).to_vec();
    if data.len() <= 4 {
        let n = 4 - data.len() as u8;
        b.push(1 << 5 | n << 2 | 0b11);
        b.extend_from_slice(&index.to_le_bytes());
        b.push(sub);
        let mut d = [0u8; 4];
        d[..data.len()].copy_from_slice(data);
        b.extend_from_slice(&d);
        return (b, data.len());
    }
    b.push(1 << 5 | 0b01);
    b.extend_from_slice(&index.to_le_bytes());
    b.push(sub);
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    let room = mbx_size.saturating_sub(MBX_HEADER + 10);
    let n = data.len().min(room);
    b.extend_from_slice(&data[..n]);
    (b, n)
}

/// Download segment: `last` marks the final one. Segments shorter than 7 bytes are padded and the unused count set.
pub fn download_segment_request(toggle: bool, last: bool, data: &[u8]) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_REQ).to_vec();
    let unused = 7usize.saturating_sub(data.len()) as u8;
    b.push((toggle as u8) << 4 | unused << 1 | last as u8);
    b.extend_from_slice(data);
    if data.len() < 7 {
        b.resize(b.len() + unused as usize, 0);
    }
    b
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdoResponse {
    /// Upload response: the data it holds, the complete size (normal transfers) and whether more segments follow.
    Upload { data: Vec<u8>, complete_size: Option<u32>, more: bool },
    UploadSegment { data: Vec<u8>, last: bool, toggle: bool },
    Download,
    DownloadSegment { toggle: bool },
    Abort { index: u16, sub: u8, code: u32 },
}

/// Parses a CoE SDO response body (after the mailbox header).
pub fn parse_sdo_response(body: &[u8]) -> Result<SdoResponse, String> {
    if body.len() < 3 {
        return Err("SDO response too short".into());
    }
    let svc = body[1] >> 4;
    if svc != SVC_SDO_RESP && svc != SVC_SDO_REQ {
        return Err(format!("Not an SDO response (CoE service {svc})"));
    }
    let cmd = body[2];
    let scs = cmd >> 5;
    let at = |i: usize| body.get(i).copied().unwrap_or(0);
    let index = u16::from_le_bytes([at(3), at(4)]);
    let sub = at(5);
    match scs {
        4 => Ok(SdoResponse::Abort { index, sub, code: u32::from_le_bytes([at(6), at(7), at(8), at(9)]) }),
        2 => {
            let expedited = cmd & 0b10 != 0;
            let size_set = cmd & 0b01 != 0;
            if expedited {
                let n = if size_set { 4 - ((cmd >> 2) & 0b11) as usize } else { 4 };
                Ok(SdoResponse::Upload { data: body.get(6..6 + n).unwrap_or(&[]).to_vec(), complete_size: Some(n as u32), more: false })
            } else {
                let size = u32::from_le_bytes([at(6), at(7), at(8), at(9)]);
                let data = body.get(10..).unwrap_or(&[]);
                let n = data.len().min(size as usize);
                Ok(SdoResponse::Upload { data: data[..n].to_vec(), complete_size: Some(size), more: n < size as usize })
            }
        }
        0 => {
            let last = cmd & 1 != 0;
            let unused = ((cmd >> 1) & 0b111) as usize;
            let data = body.get(3..).unwrap_or(&[]);
            // In EtherCAT the segment fills the mailbox; the unused count only matters for the 7-byte minimum.
            let n = if data.len() <= 7 { 7 - unused } else { data.len() };
            Ok(SdoResponse::UploadSegment { data: data[..n.min(data.len())].to_vec(), last, toggle: cmd & 0x10 != 0 })
        }
        3 => Ok(SdoResponse::Download),
        1 => Ok(SdoResponse::DownloadSegment { toggle: cmd & 0x10 != 0 }),
        _ => Err(format!("Unknown SDO server command 0x{cmd:02X}")),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// SDO Information

pub const INFO_OD_LIST_REQ: u8 = 1;
pub const INFO_OD_LIST_RESP: u8 = 2;
pub const INFO_OBJ_DESC_REQ: u8 = 3;
pub const INFO_OBJ_DESC_RESP: u8 = 4;
pub const INFO_ENTRY_DESC_REQ: u8 = 5;
pub const INFO_ENTRY_DESC_RESP: u8 = 6;
pub const INFO_ERROR: u8 = 7;

/// OD list types.
pub const LIST_ALL: u16 = 1;
pub const LIST_RXPDO: u16 = 2;
pub const LIST_TXPDO: u16 = 3;

fn info_request(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_INFO).to_vec();
    b.push(opcode);
    b.push(0);
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(payload);
    b
}

pub fn od_list_request(list: u16) -> Vec<u8> {
    info_request(INFO_OD_LIST_REQ, &list.to_le_bytes())
}

pub fn object_desc_request(index: u16) -> Vec<u8> {
    info_request(INFO_OBJ_DESC_REQ, &index.to_le_bytes())
}

/// Entry description request asking for the access rights and name (value info 0: no unit, default or limits).
pub fn entry_desc_request(index: u16, sub: u8) -> Vec<u8> {
    let mut p = index.to_le_bytes().to_vec();
    p.push(sub);
    p.push(0);
    info_request(INFO_ENTRY_DESC_REQ, &p)
}

/// An SDO Info response fragment: opcode, whether more fragments follow, and the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoFragment {
    pub opcode: u8,
    pub incomplete: bool,
    pub fragments_left: u16,
    pub payload: Vec<u8>,
}

pub fn parse_info(body: &[u8]) -> Result<InfoFragment, String> {
    if body.len() < 6 || body[1] >> 4 != SVC_SDO_INFO {
        return Err("Not an SDO Info response".into());
    }
    Ok(InfoFragment {
        opcode: body[2] & 0x7F,
        incomplete: body[2] & 0x80 != 0,
        fragments_left: u16::from_le_bytes([body[4], body[5]]),
        payload: body[6..].to_vec(),
    })
}

/// Object codes.
pub const OBJ_VAR: u8 = 7;
pub const OBJ_ARRAY: u8 = 8;
pub const OBJ_RECORD: u8 = 9;

pub fn object_code_name(c: u8) -> &'static str {
    match c {
        OBJ_VAR => "VAR",
        OBJ_ARRAY => "ARRAY",
        OBJ_RECORD => "RECORD",
        _ => "?",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDesc {
    pub index: u16,
    pub data_type: u16,
    pub max_sub: u8,
    pub object_code: u8,
    pub name: String,
}

pub fn parse_object_desc(p: &[u8]) -> Result<ObjectDesc, String> {
    if p.len() < 6 {
        return Err("Object description too short".into());
    }
    Ok(ObjectDesc {
        index: u16::from_le_bytes([p[0], p[1]]),
        data_type: u16::from_le_bytes([p[2], p[3]]),
        max_sub: p[4],
        object_code: p[5],
        name: String::from_utf8_lossy(&p[6..]).trim_end_matches('\0').to_string(),
    })
}

/// Object access bits of an entry description.
pub const ACC_READ_PREOP: u16 = 0x0001;
pub const ACC_READ_SAFEOP: u16 = 0x0002;
pub const ACC_READ_OP: u16 = 0x0004;
pub const ACC_WRITE_PREOP: u16 = 0x0008;
pub const ACC_WRITE_SAFEOP: u16 = 0x0010;
pub const ACC_WRITE_OP: u16 = 0x0020;
pub const ACC_RXPDO: u16 = 0x0040;
pub const ACC_TXPDO: u16 = 0x0080;
pub const ACC_BACKUP: u16 = 0x0100;
pub const ACC_SETTINGS: u16 = 0x0200;
pub const ACC_RO: u16 = ACC_READ_PREOP | ACC_READ_SAFEOP | ACC_READ_OP;
pub const ACC_RW: u16 = ACC_RO | ACC_WRITE_PREOP | ACC_WRITE_SAFEOP | ACC_WRITE_OP;
/// Readable everywhere, writable in PRE-OP only (PDO mapping and assignment).
pub const ACC_RW_PREOP: u16 = ACC_RO | ACC_WRITE_PREOP;

/// `rw`, `ro`, `wo` or `rw (PRE-OP)` style text for an access mask.
pub fn access_text(a: u16) -> String {
    let r = a & ACC_RO != 0;
    let wr = a & (ACC_WRITE_PREOP | ACC_WRITE_SAFEOP | ACC_WRITE_OP);
    let mut s = match (r, wr != 0) {
        (true, true) => "rw",
        (true, false) => "ro",
        (false, true) => "wo",
        (false, false) => "-",
    }
    .to_string();
    if wr != 0 && wr != ACC_WRITE_PREOP | ACC_WRITE_SAFEOP | ACC_WRITE_OP {
        let states: Vec<&str> = [(ACC_WRITE_PREOP, "PRE-OP"), (ACC_WRITE_SAFEOP, "SAFE-OP"), (ACC_WRITE_OP, "OP")]
            .into_iter()
            .filter(|(b, _)| wr & b != 0)
            .map(|(_, n)| n)
            .collect();
        s.push_str(&format!(" ({})", states.join(", ")));
    }
    if a & ACC_RXPDO != 0 {
        s.push_str(", RxPDO");
    }
    if a & ACC_TXPDO != 0 {
        s.push_str(", TxPDO");
    }
    s
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDesc {
    pub index: u16,
    pub sub: u8,
    pub data_type: u16,
    pub bits: u16,
    pub access: u16,
    pub name: String,
}

pub fn parse_entry_desc(p: &[u8]) -> Result<EntryDesc, String> {
    if p.len() < 10 {
        return Err("Entry description too short".into());
    }
    Ok(EntryDesc {
        index: u16::from_le_bytes([p[0], p[1]]),
        sub: p[2],
        data_type: u16::from_le_bytes([p[4], p[5]]),
        bits: u16::from_le_bytes([p[6], p[7]]),
        access: u16::from_le_bytes([p[8], p[9]]),
        // With value info 0 the name follows the access word directly.
        name: String::from_utf8_lossy(&p[10..]).trim_end_matches('\0').to_string(),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Device side (the simulator)

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdoRequest {
    Upload { index: u16, sub: u8, complete: bool },
    UploadSegment { toggle: bool },
    /// Download initiate: expedited data, or the first part of a normal transfer with its complete size.
    Download { index: u16, sub: u8, complete: bool, data: Vec<u8>, size: u32 },
    DownloadSegment { toggle: bool, last: bool, data: Vec<u8> },
    Abort { index: u16, sub: u8, code: u32 },
}

pub fn parse_sdo_request(body: &[u8]) -> Result<SdoRequest, u32> {
    if body.len() < 3 {
        return Err(0x0504_0001);
    }
    let cmd = body[2];
    let at = |i: usize| body.get(i).copied().unwrap_or(0);
    let index = u16::from_le_bytes([at(3), at(4)]);
    let sub = at(5);
    let complete = cmd & 0x10 != 0;
    match cmd >> 5 {
        2 => Ok(SdoRequest::Upload { index, sub, complete }),
        3 => Ok(SdoRequest::UploadSegment { toggle: cmd & 0x10 != 0 }),
        1 => {
            if cmd & 0b10 != 0 {
                let n = if cmd & 1 != 0 { 4 - ((cmd >> 2) & 0b11) as usize } else { 4 };
                Ok(SdoRequest::Download { index, sub, complete, data: body.get(6..6 + n).unwrap_or(&[]).to_vec(), size: n as u32 })
            } else {
                let size = u32::from_le_bytes([at(6), at(7), at(8), at(9)]);
                let data = body.get(10..).unwrap_or(&[]);
                let n = data.len().min(size as usize);
                Ok(SdoRequest::Download { index, sub, complete, data: data[..n].to_vec(), size })
            }
        }
        0 => {
            let last = cmd & 1 != 0;
            let unused = ((cmd >> 1) & 0b111) as usize;
            let data = body.get(3..).unwrap_or(&[]);
            let n = if data.len() <= 7 { 7 - unused } else { data.len() };
            Ok(SdoRequest::DownloadSegment { toggle: cmd & 0x10 != 0, last, data: data[..n.min(data.len())].to_vec() })
        }
        4 => Ok(SdoRequest::Abort { index, sub, code: u32::from_le_bytes([at(6), at(7), at(8), at(9)]) }),
        _ => Err(0x0504_0001),
    }
}

fn sdo_resp(cmd: u8, index: u16, sub: u8, rest: &[u8]) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_RESP).to_vec();
    b.push(cmd);
    b.extend_from_slice(&index.to_le_bytes());
    b.push(sub);
    b.extend_from_slice(rest);
    b
}

pub fn abort_response(index: u16, sub: u8, code: u32) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_REQ).to_vec();
    b.push(4 << 5);
    b.extend_from_slice(&index.to_le_bytes());
    b.push(sub);
    b.extend_from_slice(&code.to_le_bytes());
    b
}

/// Upload response: expedited up to 4 bytes, else normal with as much data as fits in a mailbox of `mbx_size`.
/// Returns the response and how many data bytes it carries (the rest go in segments).
pub fn upload_response(index: u16, sub: u8, data: &[u8], mbx_size: usize) -> (Vec<u8>, usize) {
    if data.len() <= 4 {
        let n = 4 - data.len() as u8;
        let mut d = [0u8; 4];
        d[..data.len()].copy_from_slice(data);
        return (sdo_resp(2 << 5 | n << 2 | 0b11, index, sub, &d), data.len());
    }
    let room = mbx_size.saturating_sub(MBX_HEADER + 10);
    let n = data.len().min(room);
    let mut rest = (data.len() as u32).to_le_bytes().to_vec();
    rest.extend_from_slice(&data[..n]);
    (sdo_resp(2 << 5 | 0b01, index, sub, &rest), n)
}

pub fn upload_segment_response(toggle: bool, last: bool, data: &[u8]) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_RESP).to_vec();
    let unused = 7usize.saturating_sub(data.len()) as u8;
    b.push((toggle as u8) << 4 | unused << 1 | last as u8);
    b.extend_from_slice(data);
    if data.len() < 7 {
        b.resize(b.len() + unused as usize, 0);
    }
    b
}

pub fn download_response(index: u16, sub: u8) -> Vec<u8> {
    sdo_resp(3 << 5, index, sub, &[0; 4])
}

pub fn download_segment_response(toggle: bool) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_RESP).to_vec();
    b.push(1 << 5 | (toggle as u8) << 4);
    b.extend_from_slice(&[0; 7]);
    b
}

pub fn info_response(opcode: u8, incomplete: bool, fragments_left: u16, payload: &[u8]) -> Vec<u8> {
    let mut b = coe_header(SVC_SDO_INFO).to_vec();
    b.push(opcode | if incomplete { 0x80 } else { 0 });
    b.push(0);
    b.extend_from_slice(&fragments_left.to_le_bytes());
    b.extend_from_slice(payload);
    b
}

pub fn info_error(code: u32) -> Vec<u8> {
    info_response(INFO_ERROR, false, 0, &code.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expedited_round_trips() {
        let req = upload_request(0x1018, 1);
        assert_eq!(req, vec![0x00, 0x20, 0x40, 0x18, 0x10, 0x01, 0, 0, 0, 0]);
        assert_eq!(parse_sdo_request(&req), Ok(SdoRequest::Upload { index: 0x1018, sub: 1, complete: false }));
        let (resp, n) = upload_response(0x1018, 1, &2u32.to_le_bytes(), 128);
        assert_eq!(n, 4);
        // Expedited, size indicated, 4 bytes: 0x43.
        assert_eq!(resp[2], 0x43);
        assert_eq!(parse_sdo_response(&resp), Ok(SdoResponse::Upload { data: vec![2, 0, 0, 0], complete_size: Some(4), more: false }));
        let (one, _) = upload_response(0x1C12, 0, &[1], 128);
        assert_eq!(one[2], 0x4F);

        let (dl, _) = download_request(0x6040, 0, &0x000Fu16.to_le_bytes(), 128);
        assert_eq!(dl[2], 0x2B);
        assert_eq!(
            parse_sdo_request(&dl),
            Ok(SdoRequest::Download { index: 0x6040, sub: 0, complete: false, data: vec![0x0F, 0], size: 2 })
        );
        assert_eq!(parse_sdo_response(&download_response(0x6040, 0)), Ok(SdoResponse::Download));
    }

    #[test]
    fn normal_and_segmented_upload() {
        let name = b"EL3102 2Ch. Ana. Input +/-10V, Diff.".to_vec();
        // A mailbox of 32 bytes leaves 32 - 6 - 10 = 16 data bytes in the first response.
        let (first, n) = upload_response(0x1008, 0, &name, 32);
        assert_eq!(n, 16);
        match parse_sdo_response(&first).unwrap() {
            SdoResponse::Upload { data, complete_size, more } => {
                assert_eq!(data, name[..16]);
                assert_eq!(complete_size, Some(name.len() as u32));
                assert!(more);
            }
            other => panic!("{other:?}"),
        }
        let seg = upload_segment_response(false, true, &name[16..]);
        assert_eq!(
            parse_sdo_response(&seg),
            Ok(SdoResponse::UploadSegment { data: name[16..].to_vec(), last: true, toggle: false })
        );
        let short = upload_segment_response(true, true, b"abc");
        assert_eq!(parse_sdo_response(&short), Ok(SdoResponse::UploadSegment { data: b"abc".to_vec(), last: true, toggle: true }));
    }

    #[test]
    fn abort_and_info() {
        let a = abort_response(0x2000, 0, 0x0602_0000);
        assert_eq!(parse_sdo_response(&a), Ok(SdoResponse::Abort { index: 0x2000, sub: 0, code: 0x0602_0000 }));
        assert_eq!(abort_text(0x0602_0000), "The object does not exist in the object dictionary");

        let mut p = 0x1018u16.to_le_bytes().to_vec();
        p.extend_from_slice(&0x0023u16.to_le_bytes());
        p.push(4);
        p.push(OBJ_RECORD);
        p.extend_from_slice(b"Identity");
        let r = info_response(INFO_OBJ_DESC_RESP, false, 0, &p);
        let f = parse_info(&r).unwrap();
        let d = parse_object_desc(&f.payload).unwrap();
        assert_eq!((d.index, d.max_sub, d.name.as_str()), (0x1018, 4, "Identity"));
    }

    #[test]
    fn values() {
        assert_eq!(format_value(0x0003, &(-5i16).to_le_bytes()), "-5");
        assert_eq!(format_value(0x0007, &0x0011_0000u32.to_le_bytes()), "1114112");
        assert_eq!(format_value(0x0001, &[1]), "TRUE");
        assert_eq!(format_value(0x0009, b"EK1100\0"), "EK1100");
        assert_eq!(parse_value(0x0003, 16, "-32768"), Ok(vec![0x00, 0x80]));
        assert!(parse_value(0x0003, 16, "40000").is_err());
        assert_eq!(parse_value(0x0006, 16, "0x1234"), Ok(vec![0x34, 0x12]));
        assert_eq!(parse_value(0x0001, 1, "true"), Ok(vec![1]));
        assert_eq!(parse_value(0x000A, 0, "01 0a FF"), Ok(vec![1, 10, 255]));
        assert_eq!(access_text(ACC_RW_PREOP), "rw (PRE-OP)");
        assert_eq!(access_text(ACC_RO | ACC_TXPDO), "ro, TxPDO");
        assert_eq!(data_type_name(0x0031), "BIT2");
    }
}
