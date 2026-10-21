//! CIP wire codec shared by the client and the built-in simulator: status
//! tables, reply framing, symbolic (tag) EPaths, route paths, Unconnected_Send,
//! Multiple_Service_Packet and Logix typed-data encoding.
//!
//! References: ODVA CIP Vol 1 (§2 EPath segments, §3-5.5 Connection Manager,
//! App. B status codes, App. C data types), Rockwell 1756-PM020 "Logix 5000
//! Controllers Data Access", pycomm3 (`packets/util.py`, `cip/status_info.py`),
//! cpppo (`server/enip_test.py` captures).

use anyhow::{anyhow, bail, Result};

use crate::types::*;

// ───────────────────────── Status tables ─────────────────────────

/// Name of a CIP general status code (ODVA Vol 1 App. B, Rockwell 1756-RM003).
pub fn general_status_text(status: u8) -> &'static str {
    match status {
        0x00 => "Success",
        0x01 => "Connection failure",
        0x02 => "Resource unavailable",
        0x03 => "Invalid parameter value",
        0x04 => "Path segment error (IOI syntax / tag name not valid)",
        0x05 => "Path destination unknown (tag, class, instance or member does not exist)",
        0x06 => "Partial transfer",
        0x07 => "Connection lost",
        0x08 => "Service not supported",
        0x09 => "Invalid attribute value",
        0x0A => "Attribute list error",
        0x0B => "Already in requested mode/state",
        0x0C => "Object state conflict",
        0x0D => "Object already exists",
        0x0E => "Attribute not settable",
        0x0F => "Privilege violation",
        0x10 => "Device state conflict",
        0x11 => "Reply data too large",
        0x12 => "Fragmentation of a primitive value",
        0x13 => "Not enough data",
        0x14 => "Attribute not supported",
        0x15 => "Too much data",
        0x16 => "Object does not exist",
        0x17 => "Service fragmentation sequence not in progress",
        0x18 => "No stored attribute data",
        0x19 => "Store operation failure",
        0x1A => "Routing failure, request packet too large",
        0x1B => "Routing failure, response packet too large",
        0x1C => "Missing attribute list entry data",
        0x1D => "Invalid attribute value list",
        0x1E => "Embedded service error",
        0x1F => "Vendor specific error",
        0x20 => "Invalid parameter",
        0x21 => "Write-once value or medium already written",
        0x22 => "Invalid reply received",
        0x25 => "Key failure in path",
        0x26 => "Path size invalid",
        0x27 => "Unexpected attribute in list",
        0x28 => "Invalid member ID",
        0x29 => "Member not settable",
        0x2A => "Group 2 only server general failure",
        0xFF => "General error (see extended status)",
        _ => "Unknown status",
    }
}

/// Meaning of an extended status word for a given general status, where known
/// (ODVA Vol 1 §3-5.5.1.2 Connection Manager; Rockwell 1756-RM003 for 0xFF).
pub fn extended_status_text(status: u8, ext: u16) -> Option<&'static str> {
    match (status, ext) {
        (0x01, 0x0100) => Some("Connection in use or duplicate Forward_Open"),
        (0x01, 0x0103) => Some("Transport class and trigger combination not supported"),
        (0x01, 0x0106) => Some("Ownership conflict"),
        (0x01, 0x0107) => Some("Target connection not found"),
        (0x01, 0x0108) => Some("Invalid network connection parameter"),
        (0x01, 0x0109) => Some("Invalid connection size"),
        (0x01, 0x0110) => Some("Target for connection not configured"),
        (0x01, 0x0111) => Some("RPI not supported"),
        (0x01, 0x0113) => Some("Out of connections"),
        (0x01, 0x0114) => Some("Vendor ID or product code mismatch"),
        (0x01, 0x0115) => Some("Device type mismatch"),
        (0x01, 0x0116) => Some("Revision mismatch"),
        (0x01, 0x0117) => Some("Invalid produced or consumed application path"),
        (0x01, 0x0118) => Some("Invalid or inconsistent configuration application path"),
        (0x01, 0x0119) => Some("Non-listen-only connection not opened"),
        (0x01, 0x011A) => Some("Target object out of connections"),
        (0x01, 0x0127) => Some("Invalid O→T (output) connection size"),
        (0x01, 0x0128) => Some("Invalid T→O (input) connection size"),
        (0x01, 0x0129) => Some("Invalid configuration size"),
        (0x01, 0x0203) => Some("Connection timed out"),
        (0x01, 0x0204) => Some("Unconnected request timed out"),
        (0x01, 0x0205) => Some("Parameter error in unconnected request"),
        (0x01, 0x0206) => Some("Message too large for Unconnected_Send"),
        (0x01, 0x0207) => Some("Unconnected acknowledge without reply"),
        (0x01, 0x0301) => Some("No buffer memory available"),
        (0x01, 0x0302) => Some("Network bandwidth not available"),
        (0x01, 0x0303) => Some("No consumed connection ID filter available"),
        (0x01, 0x0311) => Some("Route path: port not available"),
        (0x01, 0x0312) => Some("Route path: link address not valid"),
        (0x01, 0x0315) => Some("Invalid segment type or value in path"),
        (0x01, 0x0316) => Some("Path and connection not equal in Forward_Close"),
        (0x01, 0x0318) => Some("Link address to self invalid"),
        (0x01, 0x0319) => Some("Secondary resources unavailable"),
        (0x01, 0x031A) => Some("Rack connection already established"),
        (0x01, 0x031B) => Some("Module connection already established"),
        (0x01, 0x0800) => Some("Network link in path to module is offline"),
        (0x01, 0x0810) => Some("No target application data available"),
        (0x01, 0x0811) => Some("No originator application data available"),
        (0x01, 0x0812) => Some("Node address has changed since the network was scheduled"),
        (0x01, 0x0813) => Some("Not configured for off-subnet multicast"),
        (0x01, 0x0814) => Some("Invalid produce/consume data format"),
        (0xFF, 0x2001) => Some("Excessive IOI"),
        (0xFF, 0x2002) => Some("Bad parameter value"),
        (0xFF, 0x2018) => Some("Semaphore reject"),
        (0xFF, 0x201B) => Some("Size too small"),
        (0xFF, 0x201C) => Some("Invalid size"),
        (0xFF, 0x2100) => Some("Privilege failure"),
        (0xFF, 0x2101) => Some("Invalid keyswitch position"),
        (0xFF, 0x2102) => Some("Password invalid"),
        (0xFF, 0x2103) => Some("No password issued"),
        (0xFF, 0x2104) => Some("Address out of range"),
        (0xFF, 0x2105) => Some("Access beyond end of the object"),
        (0xFF, 0x2106) => Some("Data in use"),
        (0xFF, 0x2107) => Some("Tag type used in request does not match the target tag's data type"),
        (0xFF, 0x2108) => Some("Controller in upload or download mode"),
        (0xFF, 0x2109) => Some("Attempt to change number of array dimensions"),
        (0xFF, 0x210A) => Some("Invalid symbol name"),
        (0xFF, 0x210B) => Some("Symbol does not exist"),
        (0xFF, 0x210E) => Some("Search failed"),
        (0xFF, 0x210F) => Some("Task cannot start"),
        (0xFF, 0x2110) => Some("Unable to write"),
        (0xFF, 0x2111) => Some("Unable to read"),
        (0xFF, 0x2112) => Some("Shared routine not editable"),
        (0xFF, 0x2113) => Some("Controller in faulted mode"),
        (0xFF, 0x2114) => Some("Run mode inhibited"),
        _ => None,
    }
}

/// Name of an EtherNet/IP encapsulation status code (ODVA Vol 2 Table 2-3.3).
pub fn encapsulation_status_text(status: u32) -> &'static str {
    match status {
        0x0000 => "Success",
        0x0001 => "Invalid or unsupported encapsulation command",
        0x0002 => "Insufficient memory on the target",
        0x0003 => "Poorly formed or incorrect data in the encapsulation data portion",
        0x0064 => "Invalid session handle",
        0x0065 => "Invalid length in the encapsulation header",
        0x0069 => "Unsupported encapsulation protocol revision",
        _ => "Unknown encapsulation status",
    }
}

/// A CIP error reply (non-zero general status), kept typed so callers can react
/// to specific codes via `anyhow::Error::downcast_ref::<CipError>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CipError {
    pub status: u8,
    pub ext: Vec<u16>,
}

impl std::fmt::Display for CipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CIP error 0x{:02X}: {}", self.status, general_status_text(self.status))?;
        if let Some(first) = self.ext.first() {
            write!(f, " (extended status 0x{:04X}", first)?;
            if let Some(t) = extended_status_text(self.status, *first) {
                write!(f, ": {t}")?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl std::error::Error for CipError {}

// ───────────────────────── Reply framing ─────────────────────────

/// A parsed CIP message-router reply: `service | reserved | status | ext size
/// (words) | ext status words | data` (ODVA Vol 1 §2-4.2).
#[derive(Debug, Clone)]
pub(crate) struct CipReply {
    pub service: u8,
    pub status: u8,
    pub ext: Vec<u16>,
    pub data: Vec<u8>,
}

impl CipReply {
    pub fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() < 4 {
            bail!("CIP reply too short ({} bytes)", buf.len());
        }
        let ext_words = buf[3] as usize;
        let data_start = 4 + 2 * ext_words;
        if buf.len() < data_start {
            bail!("CIP reply truncated inside extended status");
        }
        let ext = buf[4..data_start]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(Self { service: buf[0], status: buf[2], ext, data: buf[data_start..].to_vec() })
    }

    pub fn error(&self) -> CipError {
        CipError { status: self.status, ext: self.ext.clone() }
    }

    /// Validate that this is the reply to `request_service` and that its status
    /// is success (or partial transfer when `allow_partial`). A failed
    /// Unconnected_Send (reply service 0xD2, e.g. routing failure) is surfaced as
    /// the underlying CIP error instead of a service mismatch.
    pub fn expect(self, request_service: u8, allow_partial: bool) -> Result<Self> {
        if self.service != request_service | 0x80 {
            if self.service == CIP_UNCONNECTED_SEND | 0x80 && self.status != CIP_STATUS_SUCCESS {
                return Err(self.error().into());
            }
            bail!(
                "Unexpected CIP reply service 0x{:02X} (expected 0x{:02X})",
                self.service,
                request_service | 0x80
            );
        }
        if self.status == CIP_STATUS_SUCCESS || (allow_partial && self.status == CIP_STATUS_PARTIAL) {
            Ok(self)
        } else {
            Err(self.error().into())
        }
    }
}

// ───────────────────────── EPath: symbolic tag paths ─────────────────────────

/// Encode a Logix tag name (`Program:Main.Motor[3].Speed`, `Grid[1,2]`) as a CIP
/// EPath: one ANSI Extended Symbol segment (0x91) per dotted name, one member
/// segment (0x28/0x29/0x2A) per array index (1756-PM020 "Tag Addressing").
/// Returns the path bytes (always an even length); the caller prepends the
/// word count. Bit selectors (`Tag.5`) are not part of the path — strip them
/// with [`split_bit_suffix`] first.
pub(crate) fn encode_symbolic_path(tag: &str) -> Result<Vec<u8>> {
    let b = tag.as_bytes();
    if b.is_empty() {
        bail!("tag name is empty");
    }
    let mut out = Vec::with_capacity(b.len() + 8);
    let mut i = 0;
    let mut have_ident = false;
    while i < b.len() {
        match b[i] {
            b'.' => {
                if !have_ident {
                    bail!("tag name '{tag}': '.' without a preceding name");
                }
                i += 1;
                if i >= b.len() {
                    bail!("tag name '{tag}': ends with '.'");
                }
                // Numeric selector after '.' is a bit index, resolved by the caller.
                if b[i].is_ascii_digit() && b[i..].iter().all(|c| c.is_ascii_digit()) {
                    bail!("tag name '{tag}': bit selector must be handled by the caller");
                }
            }
            b'[' => {
                if !have_ident {
                    bail!("tag name '{tag}': '[' without a preceding name");
                }
                let close = b[i..]
                    .iter()
                    .position(|&c| c == b']')
                    .ok_or_else(|| anyhow!("tag name '{tag}': missing ']'"))?
                    + i;
                let inner = std::str::from_utf8(&b[i + 1..close])?;
                for part in inner.split(',') {
                    let idx: u32 = part
                        .trim()
                        .parse()
                        .map_err(|_| anyhow!("tag name '{tag}': invalid array index '{part}'"))?;
                    push_member_segment(&mut out, idx);
                }
                i = close + 1;
            }
            b']' => bail!("tag name '{tag}': unexpected ']'"),
            _ => {
                let end = b[i..]
                    .iter()
                    .position(|&c| c == b'.' || c == b'[')
                    .map(|p| p + i)
                    .unwrap_or(b.len());
                let name = &b[i..end];
                if name.len() > 255 {
                    bail!("tag name '{tag}': symbol longer than 255 bytes");
                }
                if name.iter().any(|&c| c.is_ascii_control() || matches!(c, b' ' | b']' | b',')) {
                    bail!("tag name '{tag}': invalid character in symbol");
                }
                out.push(0x91);
                out.push(name.len() as u8);
                out.extend_from_slice(name);
                if !name.len().is_multiple_of(2) {
                    out.push(0x00);
                }
                have_ident = true;
                i = end;
            }
        }
    }
    Ok(out)
}

/// Logical member-ID segment for an array index: 0x28 (8-bit), 0x29 (16-bit,
/// pad byte), 0x2A (32-bit, pad byte).
fn push_member_segment(out: &mut Vec<u8>, idx: u32) {
    if idx <= 0xFF {
        out.extend_from_slice(&[0x28, idx as u8]);
    } else if idx <= 0xFFFF {
        out.extend_from_slice(&[0x29, 0x00]);
        out.extend_from_slice(&(idx as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&[0x2A, 0x00]);
        out.extend_from_slice(&idx.to_le_bytes());
    }
}

/// Inverse of [`encode_symbolic_path`]: rebuild `Name.Member[1,2]` from path
/// bytes. Returns `None` for paths that contain anything but symbolic and
/// member segments.
pub(crate) fn decode_symbolic_path(path: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut i = 0;
    let mut idents = 0;
    while i < path.len() {
        match path[i] {
            0x91 => {
                let len = *path.get(i + 1)? as usize;
                let name = std::str::from_utf8(path.get(i + 2..i + 2 + len)?).ok()?;
                if idents > 0 {
                    out.push('.');
                }
                out.push_str(name);
                idents += 1;
                i += 2 + len;
                if !len.is_multiple_of(2) {
                    i += 1;
                }
            }
            seg @ (0x28 | 0x29 | 0x2A) => {
                let (idx, adv) = match seg {
                    0x28 => (*path.get(i + 1)? as u32, 2),
                    0x29 => (u16::from_le_bytes([*path.get(i + 2)?, *path.get(i + 3)?]) as u32, 4),
                    _ => (
                        u32::from_le_bytes([
                            *path.get(i + 2)?,
                            *path.get(i + 3)?,
                            *path.get(i + 4)?,
                            *path.get(i + 5)?,
                        ]),
                        6,
                    ),
                };
                if idents == 0 {
                    return None;
                }
                if out.ends_with(']') {
                    out.pop();
                    out.push_str(&format!(",{idx}]"));
                } else {
                    out.push_str(&format!("[{idx}]"));
                }
                i += adv;
            }
            _ => return None,
        }
    }
    if idents == 0 { None } else { Some(out) }
}

/// Split a trailing numeric bit selector: `"Status.5"` → `("Status", 5)`.
pub(crate) fn split_bit_suffix(tag: &str) -> Option<(&str, u8)> {
    let (base, last) = tag.rsplit_once('.')?;
    if last.is_empty() || !last.bytes().all(|c| c.is_ascii_digit()) || base.is_empty() {
        return None;
    }
    let bit: u8 = last.parse().ok()?;
    (bit < 64).then_some((base, bit))
}

/// Split `Name[3]` into `("Name", 3)` for single-dimension element access
/// (used by the simulator's array tags).
pub(crate) fn split_element_suffix(tag: &str) -> Option<(&str, u32)> {
    let rest = tag.strip_suffix(']')?;
    let (base, idx) = rest.rsplit_once('[')?;
    if idx.contains(',') {
        return None;
    }
    Some((base, idx.trim().parse().ok()?))
}

// ───────────────────────── EPath: logical class/instance/attribute ─────────────────────────

/// Encode a logical EPath (class + instance + optional attribute) with the
/// smallest segment format each id fits in (8/16/32-bit; ODVA Vol 1 App. C-1.4.2).
/// Returns (bytes, size in 16-bit words).
pub(crate) fn build_epath(class_id: u16, instance_id: u32, attribute_id: Option<u16>) -> (Vec<u8>, u8) {
    let mut path = Vec::new();
    push_logical(&mut path, 0x20, class_id as u32);
    push_logical(&mut path, 0x24, instance_id);
    if let Some(attr) = attribute_id {
        push_logical(&mut path, 0x30, attr as u32);
    }
    let words = path.len().div_ceil(2) as u8;
    if !path.len().is_multiple_of(2) {
        path.push(0x00);
    }
    (path, words)
}

/// Logical segment with type byte `base` (0x20 class / 0x24 instance /
/// 0x28 member / 0x30 attribute) + format bits: 8-bit (+0), 16-bit (+1, pad),
/// 32-bit (+2, pad).
pub(crate) fn push_logical(out: &mut Vec<u8>, base: u8, value: u32) {
    if value <= 0xFF {
        out.extend_from_slice(&[base, value as u8]);
    } else if value <= 0xFFFF {
        out.extend_from_slice(&[base | 0x01, 0x00]);
        out.extend_from_slice(&(value as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&[base | 0x02, 0x00]);
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// Parse class/instance/attribute out of a logical EPath (all three segment
/// widths). The path bytes exclude the size byte.
pub(crate) fn parse_logical_path(path: &[u8]) -> (Option<u16>, Option<u32>, Option<u16>) {
    let (mut class, mut instance, mut attr) = (None, None, None);
    let mut i = 0;
    while i < path.len() {
        let seg = path[i];
        if seg & 0xE0 != 0x20 {
            break; // not a logical segment
        }
        let (value, adv) = match seg & 0x03 {
            0 => match path.get(i + 1) {
                Some(v) => (*v as u32, 2),
                None => break,
            },
            1 => match path.get(i + 2..i + 4) {
                Some(v) => (u16::from_le_bytes([v[0], v[1]]) as u32, 4),
                None => break,
            },
            2 => match path.get(i + 2..i + 6) {
                Some(v) => (u32::from_le_bytes([v[0], v[1], v[2], v[3]]), 6),
                None => break,
            },
            _ => break,
        };
        match seg & 0x1C {
            0x00 => class = Some(value as u16),
            0x04 => instance = Some(value),
            0x10 => attr = Some(value as u16),
            _ => {}
        }
        i += adv;
    }
    (class, instance, attr)
}

// ───────────────────────── Route paths ─────────────────────────

/// Parse a textual route path into EPath port segments, e.g. `"1,0"` →
/// `[01 00]` (backplane, slot 0); `"1,2,2,10.0.0.5,1,0"` → hop out port 1 to
/// node 2, then Ethernet port 2 to 10.0.0.5, then backplane slot 0. Separators
/// are `,` or `/`. Pairs are `port, link address`; a link address containing a
/// `.` is encoded as an extended link address string (ODVA Vol 1 App. C-1.4.1).
pub(crate) fn parse_route_path(s: &str) -> Result<Vec<u8>> {
    let tokens: Vec<&str> = s
        .split([',', '/'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if !tokens.len().is_multiple_of(2) {
        bail!("route path '{s}' must be port,link pairs");
    }
    let mut out = Vec::new();
    for pair in tokens.chunks_exact(2) {
        let port: u16 = pair[0]
            .parse()
            .map_err(|_| anyhow!("route path '{s}': invalid port '{}'", pair[0]))?;
        if port == 0 {
            bail!("route path '{s}': port 0 is reserved");
        }
        let link = pair[1];
        let extended_link = link.contains('.');
        let seg_port = if port >= 15 { 0x0F } else { port as u8 };
        if extended_link {
            if link.len() > 255 {
                bail!("route path '{s}': link address too long");
            }
            out.push(seg_port | 0x10);
            out.push(link.len() as u8);
            if port >= 15 {
                out.extend_from_slice(&port.to_le_bytes());
            }
            out.extend_from_slice(link.as_bytes());
            if !out.len().is_multiple_of(2) {
                out.push(0x00);
            }
        } else {
            let addr: u8 = link
                .parse()
                .map_err(|_| anyhow!("route path '{s}': invalid link address '{link}'"))?;
            out.push(seg_port);
            if port >= 15 {
                // extended port id sits between the segment byte and the link byte
                out.extend_from_slice(&port.to_le_bytes());
            }
            out.push(addr);
            if !out.len().is_multiple_of(2) {
                out.push(0x00);
            }
        }
    }
    Ok(out)
}

// ───────────────────────── Connection Manager: Unconnected_Send ─────────────────────────

/// Wrap `message` (a complete CIP request) in an Unconnected_Send to the
/// Connection Manager (class 6, instance 1) with `route` (EPath port segments):
///
/// `52 02 20 06 24 01 | priority/tick | timeout ticks | msg size u16 | msg |
/// [pad] | route size (words) | 00 | route`
///
/// Matches the cpppo capture `52 02 20 06 24 01 05 9d 10 00 …` and pycomm3
/// `wrap_unconnected_send` (priority 0x0A, timeout ticks 0x05).
pub(crate) fn build_unconnected_send(message: &[u8], route: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + route.len() + 14);
    out.push(CIP_UNCONNECTED_SEND);
    out.push(0x02); // path size: 2 words
    out.extend_from_slice(&[0x20, 0x06, 0x24, 0x01]);
    out.push(0x0A); // priority/time_tick: tick = 1024 ms
    out.push(0x05); // timeout ticks
    out.extend_from_slice(&(message.len() as u16).to_le_bytes());
    out.extend_from_slice(message);
    if !message.len().is_multiple_of(2) {
        out.push(0x00);
    }
    out.push((route.len().div_ceil(2)) as u8);
    out.push(0x00); // reserved
    out.extend_from_slice(route);
    if !route.len().is_multiple_of(2) {
        out.push(0x00);
    }
    out
}

/// Split an Unconnected_Send *request* into (embedded message, route path).
/// Used by the simulator.
pub(crate) fn parse_unconnected_send(request: &[u8]) -> Option<(&[u8], &[u8])> {
    let path_words = *request.get(1)? as usize;
    let mut p = 2 + path_words * 2;
    p += 2; // priority/tick + timeout ticks
    let len = u16::from_le_bytes([*request.get(p)?, *request.get(p + 1)?]) as usize;
    p += 2;
    let msg = request.get(p..p + len)?;
    p += len;
    if !len.is_multiple_of(2) {
        p += 1;
    }
    let route_words = *request.get(p)? as usize;
    p += 2;
    let route = request.get(p..p + route_words * 2).unwrap_or(&[]);
    Some((msg, route))
}

// ───────────────────────── Multiple_Service_Packet ─────────────────────────

/// Build a Multiple_Service_Packet request (service 0x0A to the Message Router,
/// class 2 instance 1): `count u16 | offsets u16[count] | requests…`, offsets
/// counted from the start of the `count` field (ODVA Vol 1 §A-5.1).
pub(crate) fn build_multiple_service_packet(requests: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![CIP_MULTIPLE_SERVICE_PACKET, 0x02, 0x20, 0x02, 0x24, 0x01];
    out.extend_from_slice(&(requests.len() as u16).to_le_bytes());
    let mut offset = 2 + 2 * requests.len();
    for r in requests {
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        offset += r.len();
    }
    for r in requests {
        out.extend_from_slice(r);
    }
    out
}

/// Split the data of a Multiple_Service_Packet request or reply (everything
/// after the path / 4-byte reply header) into its individual messages.
pub(crate) fn split_multiple_service_data(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    if data.len() < 2 {
        bail!("Multiple_Service_Packet data too short");
    }
    let count = u16::from_le_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + 2 * count {
        bail!("Multiple_Service_Packet offset table truncated");
    }
    let offsets: Vec<usize> = (0..count)
        .map(|i| u16::from_le_bytes([data[2 + 2 * i], data[3 + 2 * i]]) as usize)
        .collect();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = offsets[i];
        let end = offsets.get(i + 1).copied().unwrap_or(data.len());
        if start > end || end > data.len() {
            bail!("Multiple_Service_Packet offset out of range");
        }
        out.push(data[start..end].to_vec());
    }
    Ok(out)
}

// ───────────────────────── Logix typed data ─────────────────────────

/// The type field of a Logix tag read reply / write request: `[code][len]
/// [len bytes of type info]`. Atomic types are `C4 00`; structures are
/// `A0 02 <handle lo> <handle hi>` (1756-PM020 Table "Data type reporting").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TagType {
    /// Atomic CIP type code, or [`CIP_TYPE_STRUCT`] for structures.
    pub code: u8,
    /// Structure handle (only for `code == CIP_TYPE_STRUCT`).
    pub handle: Option<u16>,
}

impl TagType {
    pub fn atomic(code: u8) -> Self {
        Self { code, handle: None }
    }

    pub fn structure(handle: u16) -> Self {
        Self { code: CIP_TYPE_STRUCT, handle: Some(handle) }
    }

    pub fn logix_string() -> Self {
        Self::structure(STRUCT_HANDLE_LOGIX_STRING)
    }

    pub fn for_data_type(dt: CipDataType) -> Self {
        match dt {
            CipDataType::String => Self::logix_string(),
            other => Self::atomic(other.code()),
        }
    }

    pub fn is_logix_string(&self) -> bool {
        self.code == CIP_TYPE_STRUCT && self.handle == Some(STRUCT_HANDLE_LOGIX_STRING)
    }

    pub fn encode(&self) -> Vec<u8> {
        match self.handle {
            Some(h) => vec![self.code, 0x02, h as u8, (h >> 8) as u8],
            None => vec![self.code, 0x00],
        }
    }

    /// Parse the type field at the start of `data`; returns (type, bytes consumed).
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < 2 {
            bail!("tag reply missing data type field");
        }
        let code = data[0];
        let extra = data[1] as usize;
        if data.len() < 2 + extra {
            bail!("tag reply data type field truncated");
        }
        let handle = if code == CIP_TYPE_STRUCT && extra >= 2 {
            Some(u16::from_le_bytes([data[2], data[3]]))
        } else {
            None
        };
        Ok((Self { code, handle }, 2 + extra))
    }

    /// Size in bytes of one element, when fixed.
    pub fn element_size(&self) -> Option<usize> {
        if self.is_logix_string() {
            return Some(LOGIX_STRING_SIZE);
        }
        atomic_size(self.code)
    }
}

/// Byte size of a fixed-size atomic CIP type.
pub(crate) fn atomic_size(code: u8) -> Option<usize> {
    Some(match code {
        CIP_TYPE_BOOL | CIP_TYPE_SINT | CIP_TYPE_USINT | CIP_TYPE_BYTE => 1,
        CIP_TYPE_INT | CIP_TYPE_UINT | CIP_TYPE_WORD => 2,
        CIP_TYPE_DINT | CIP_TYPE_UDINT | CIP_TYPE_REAL | CIP_TYPE_DWORD => 4,
        CIP_TYPE_LINT | CIP_TYPE_ULINT | CIP_TYPE_LREAL | CIP_TYPE_LWORD => 8,
        _ => return None,
    })
}

/// A Read Tag / Read Tag Fragmented reply: type + raw element bytes.
#[derive(Debug, Clone)]
pub(crate) struct TagRead {
    pub ty: TagType,
    pub data: Vec<u8>,
    /// Status 0x06: more data remains; continue with Read Tag Fragmented.
    pub partial: bool,
}

pub(crate) fn parse_tag_read_reply(buf: &[u8], request_service: u8) -> Result<TagRead> {
    let reply = CipReply::parse(buf)?.expect(request_service, true)?;
    let (ty, used) = TagType::parse(&reply.data)?;
    Ok(TagRead { ty, data: reply.data[used..].to_vec(), partial: reply.status == CIP_STATUS_PARTIAL })
}

pub(crate) fn parse_tag_write_reply(buf: &[u8], request_service: u8) -> Result<()> {
    CipReply::parse(buf)?.expect(request_service, false)?;
    Ok(())
}

/// Read Tag (0x4C): `4C | path words | path | count u16`.
pub(crate) fn build_read_tag_request(tag: &str, count: u16) -> Result<Vec<u8>> {
    let path = encode_symbolic_path(tag)?;
    let mut r = vec![CIP_READ_TAG, (path.len() / 2) as u8];
    r.extend_from_slice(&path);
    r.extend_from_slice(&count.to_le_bytes());
    Ok(r)
}

/// Read Tag Fragmented (0x52): `52 | path words | path | count u16 | byte offset u32`
/// (cpppo capture `52 04 91 05 SCADA 00 14 00 02 00 00 00`).
pub(crate) fn build_read_tag_fragmented_request(tag: &str, count: u16, offset: u32) -> Result<Vec<u8>> {
    let path = encode_symbolic_path(tag)?;
    let mut r = vec![CIP_READ_TAG_FRAGMENTED, (path.len() / 2) as u8];
    r.extend_from_slice(&path);
    r.extend_from_slice(&count.to_le_bytes());
    r.extend_from_slice(&offset.to_le_bytes());
    Ok(r)
}

/// Write Tag (0x4D): `4D | path words | path | type | count u16 | data`.
pub(crate) fn build_write_tag_request(tag: &str, ty: &TagType, count: u16, data: &[u8]) -> Result<Vec<u8>> {
    let path = encode_symbolic_path(tag)?;
    let mut r = vec![CIP_WRITE_TAG, (path.len() / 2) as u8];
    r.extend_from_slice(&path);
    r.extend_from_slice(&ty.encode());
    r.extend_from_slice(&count.to_le_bytes());
    r.extend_from_slice(data);
    Ok(r)
}

/// Write Tag Fragmented (0x53): `53 | path words | path | type | count u16 |
/// byte offset u32 | data` (cpppo capture `53 05 91 05 SCADA 00 28 0C C3 00 01 00 00 00 00 00 C9 40`).
pub(crate) fn build_write_tag_fragmented_request(
    tag: &str,
    ty: &TagType,
    count: u16,
    offset: u32,
    data: &[u8],
) -> Result<Vec<u8>> {
    let path = encode_symbolic_path(tag)?;
    let mut r = vec![CIP_WRITE_TAG_FRAGMENTED, (path.len() / 2) as u8];
    r.extend_from_slice(&path);
    r.extend_from_slice(&ty.encode());
    r.extend_from_slice(&count.to_le_bytes());
    r.extend_from_slice(&offset.to_le_bytes());
    r.extend_from_slice(data);
    Ok(r)
}

/// Decode one atomic element (little-endian) from the front of `data`.
pub(crate) fn decode_atomic(code: u8, data: &[u8]) -> Result<CipValue> {
    let need = atomic_size(code).ok_or_else(|| anyhow!("unsupported atomic type 0x{code:02X}"))?;
    if data.len() < need {
        bail!("not enough data for type 0x{code:02X}: have {}, need {need}", data.len());
    }
    let d = &data[..need];
    Ok(match code {
        CIP_TYPE_BOOL => CipValue::Bool(d[0] != 0),
        CIP_TYPE_SINT => CipValue::Sint(d[0] as i8),
        CIP_TYPE_INT => CipValue::Int(i16::from_le_bytes([d[0], d[1]])),
        CIP_TYPE_DINT => CipValue::Dint(i32::from_le_bytes(d.try_into().unwrap())),
        CIP_TYPE_LINT => CipValue::Lint(i64::from_le_bytes(d.try_into().unwrap())),
        CIP_TYPE_USINT | CIP_TYPE_BYTE => CipValue::Usint(d[0]),
        CIP_TYPE_UINT | CIP_TYPE_WORD => CipValue::Uint(u16::from_le_bytes([d[0], d[1]])),
        CIP_TYPE_UDINT => CipValue::Udint(u32::from_le_bytes(d.try_into().unwrap())),
        CIP_TYPE_ULINT => CipValue::Ulint(u64::from_le_bytes(d.try_into().unwrap())),
        CIP_TYPE_REAL => CipValue::Real(f32::from_le_bytes(d.try_into().unwrap())),
        CIP_TYPE_LREAL => CipValue::Lreal(f64::from_le_bytes(d.try_into().unwrap())),
        CIP_TYPE_DWORD | CIP_TYPE_LWORD => CipValue::Bytes(d.to_vec()),
        _ => unreachable!("atomic_size covers the same codes"),
    })
}

/// Decode a Logix STRING structure image: DINT LEN + DATA\[82\].
pub(crate) fn decode_logix_string(d: &[u8]) -> Result<String> {
    if d.len() < 4 {
        bail!("STRING structure shorter than its LEN field");
    }
    let len = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize;
    let chars = &d[4..];
    let len = len.min(chars.len()).min(LOGIX_STRING_MAX_CHARS);
    Ok(String::from_utf8_lossy(&chars[..len]).into_owned())
}

/// Encode a Logix STRING structure image (88 bytes).
pub(crate) fn encode_logix_string(s: &str) -> Result<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() > LOGIX_STRING_MAX_CHARS {
        bail!("string of {} bytes exceeds the Logix STRING limit of {LOGIX_STRING_MAX_CHARS}", bytes.len());
    }
    let mut out = vec![0u8; LOGIX_STRING_SIZE];
    out[..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    out[4..4 + bytes.len()].copy_from_slice(bytes);
    Ok(out)
}

/// Decode `count` elements of type `ty` from a tag read.
///
/// * atomic → scalar (`count == 1`) or [`CipValue::Array`]
/// * Logix STRING structure → [`CipValue::String`] / array of strings
/// * other structures → [`CipValue::Bytes`] (raw image; decode with a template)
/// * CIP STRING / SHORT_STRING (attribute values) → [`CipValue::String`]
pub(crate) fn decode_tag_value(ty: &TagType, data: &[u8], count: u16) -> Result<CipValue> {
    if ty.code == CIP_TYPE_STRING || ty.code == CIP_TYPE_SHORT_STRING {
        let (len, skip) = if ty.code == CIP_TYPE_STRING {
            if data.len() < 2 {
                bail!("STRING shorter than its length field");
            }
            (u16::from_le_bytes([data[0], data[1]]) as usize, 2)
        } else {
            (*data.first().ok_or_else(|| anyhow!("empty SHORT_STRING"))? as usize, 1)
        };
        let chars = data.get(skip..skip + len).unwrap_or(&data[skip.min(data.len())..]);
        return Ok(CipValue::String(String::from_utf8_lossy(chars).into_owned()));
    }
    if ty.code == CIP_TYPE_STRUCT {
        if ty.is_logix_string() && data.len() >= LOGIX_STRING_SIZE {
            let n = data.len() / LOGIX_STRING_SIZE;
            if count <= 1 || n == 1 {
                return Ok(CipValue::String(decode_logix_string(&data[..LOGIX_STRING_SIZE])?));
            }
            return Ok(CipValue::Array(
                data.chunks_exact(LOGIX_STRING_SIZE)
                    .map(|c| decode_logix_string(c).map(CipValue::String))
                    .collect::<Result<_>>()?,
            ));
        }
        return Ok(CipValue::Bytes(data.to_vec()));
    }
    match atomic_size(ty.code) {
        Some(size) => {
            if data.len() < size {
                bail!("tag reply has {} data bytes, need at least {size}", data.len());
            }
            let n = data.len() / size;
            if count <= 1 || n == 1 {
                decode_atomic(ty.code, data)
            } else {
                Ok(CipValue::Array(
                    data.chunks_exact(size)
                        .map(|c| decode_atomic(ty.code, c))
                        .collect::<Result<_>>()?,
                ))
            }
        }
        None => Ok(CipValue::Bytes(data.to_vec())),
    }
}

pub(crate) fn int_of(v: &CipValue) -> Option<i128> {
    Some(match v {
        CipValue::Bool(b) => *b as i128,
        CipValue::Sint(x) => *x as i128,
        CipValue::Int(x) => *x as i128,
        CipValue::Dint(x) => *x as i128,
        CipValue::Lint(x) => *x as i128,
        CipValue::Usint(x) => *x as i128,
        CipValue::Uint(x) => *x as i128,
        CipValue::Udint(x) => *x as i128,
        CipValue::Ulint(x) => *x as i128,
        CipValue::Real(x) if x.is_finite() => x.round() as i128,
        CipValue::Lreal(x) if x.is_finite() => x.round() as i128,
        _ => return None,
    })
}

/// Encode one element of atomic type `code`, coercing numeric variants and
/// range-checking integers.
fn encode_atomic(code: u8, v: &CipValue) -> Result<Vec<u8>> {
    macro_rules! int {
        ($t:ty) => {{
            let n = int_of(v).ok_or_else(|| anyhow!("value {v:?} is not numeric"))?;
            let x: $t = n
                .try_into()
                .map_err(|_| anyhow!("value {n} out of range for {}", stringify!($t)))?;
            x.to_le_bytes().to_vec()
        }};
    }
    Ok(match code {
        CIP_TYPE_BOOL => {
            let n = int_of(v).ok_or_else(|| anyhow!("value {v:?} is not boolean"))?;
            // Logix BOOL write: one byte, 0xFF = true (pycomm3 `BOOL.encode`).
            vec![if n != 0 { 0xFF } else { 0x00 }]
        }
        CIP_TYPE_SINT => int!(i8),
        CIP_TYPE_INT => int!(i16),
        CIP_TYPE_DINT => int!(i32),
        CIP_TYPE_LINT => int!(i64),
        CIP_TYPE_USINT | CIP_TYPE_BYTE => int!(u8),
        CIP_TYPE_UINT | CIP_TYPE_WORD => int!(u16),
        CIP_TYPE_UDINT => int!(u32),
        CIP_TYPE_ULINT => int!(u64),
        CIP_TYPE_REAL => {
            let f = v.to_f64().ok_or_else(|| anyhow!("value {v:?} is not numeric"))?;
            (f as f32).to_le_bytes().to_vec()
        }
        CIP_TYPE_LREAL => {
            let f = v.to_f64().ok_or_else(|| anyhow!("value {v:?} is not numeric"))?;
            f.to_le_bytes().to_vec()
        }
        CIP_TYPE_DWORD | CIP_TYPE_LWORD => {
            let size = if code == CIP_TYPE_DWORD { 4 } else { 8 };
            match v {
                CipValue::Bytes(b) if b.len() == size => b.clone(),
                CipValue::Bytes(b) => bail!("{} bytes given for a {size}-byte bit string", b.len()),
                other => {
                    let n = int_of(other).ok_or_else(|| anyhow!("value {other:?} is not numeric"))?;
                    let bits = n as u64;
                    bits.to_le_bytes()[..size].to_vec()
                }
            }
        }
        other => bail!("cannot encode atomic type 0x{other:02X}"),
    })
}

/// Encode `value` as `ty` for a Write Tag request. Returns (element count, bytes).
/// Numeric values are coerced to the tag's real type so a DINT-typed UI value
/// can be written to a REAL tag without a 0xFF/0x2107 type mismatch.
pub(crate) fn encode_tag_value(ty: &TagType, value: &CipValue) -> Result<(u16, Vec<u8>)> {
    let elem = |v: &CipValue| -> Result<Vec<u8>> {
        if ty.is_logix_string() {
            match v {
                CipValue::String(s) => encode_logix_string(s),
                CipValue::Bytes(b) if b.len() == LOGIX_STRING_SIZE => Ok(b.clone()),
                other => bail!("value {other:?} is not a string"),
            }
        } else if ty.code == CIP_TYPE_STRUCT {
            match v {
                CipValue::Bytes(b) => Ok(b.clone()),
                other => bail!("structure tags are written as raw bytes, got {other:?}"),
            }
        } else {
            encode_atomic(ty.code, v)
        }
    };
    match value {
        CipValue::Array(items) => {
            let mut data = Vec::new();
            for it in items {
                data.extend_from_slice(&elem(it)?);
            }
            Ok((u16::try_from(items.len()).map_err(|_| anyhow!("too many array elements"))?, data))
        }
        CipValue::Bytes(b) if ty.code != CIP_TYPE_STRUCT => {
            // Raw bytes for an atomic type: must be whole elements.
            let size = ty.element_size().ok_or_else(|| anyhow!("raw bytes need a fixed-size type"))?;
            if b.is_empty() || b.len() % size != 0 {
                bail!("{} raw bytes is not a whole number of {size}-byte elements", b.len());
            }
            Ok(((b.len() / size) as u16, b.clone()))
        }
        CipValue::Bytes(b) if ty.is_logix_string() => {
            if b.is_empty() || b.len() % LOGIX_STRING_SIZE != 0 {
                bail!("{} raw bytes is not a whole number of STRING structures", b.len());
            }
            Ok(((b.len() / LOGIX_STRING_SIZE) as u16, b.clone()))
        }
        other => Ok((1, elem(other)?)),
    }
}
