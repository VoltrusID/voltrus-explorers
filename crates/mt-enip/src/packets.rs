#![allow(clippy::vec_init_then_push)]
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use crate::types::*;

// ── Monotonically increasing sender context ──

static SENDER_CONTEXT_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_sender_context() -> u64 {
    SENDER_CONTEXT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ── Packet construction helpers ──

pub(crate) fn build_enip_header(command: u16, session_handle: u32, _options: u32, body: &[u8]) -> Vec<u8> {
    let sender_context = next_sender_context();
    let mut buf = Vec::with_capacity(ENIP_HEADER_SIZE + body.len());
    buf.extend_from_slice(&command.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u16).to_le_bytes());
    buf.extend_from_slice(&session_handle.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // status
    buf.extend_from_slice(&sender_context.to_le_bytes()); // sender context
    buf.extend_from_slice(&0u32.to_le_bytes()); // options
    buf.extend_from_slice(body);
    buf
}

/// Build a CIP Read Tag request for explicit messaging.
pub(crate) fn build_read_tag_request(tag_name: &str, element_count: u16) -> Vec<u8> {
    let mut request = Vec::new();

    // CIP service: Read Tag
    request.push(CIP_READ_TAG);

    // Request path: encode tag name as symbolic segment
    let path = build_symbolic_path(tag_name);
    request.push((path.len() / 2) as u8); // path size in words
    request.extend_from_slice(&path);

    // Number of elements to read
    request.extend_from_slice(&element_count.to_le_bytes());

    request
}

/// Build a CIP Write Tag request.
pub(crate) fn build_write_tag_request(tag_name: &str, value: &CipValue, element_count: u16) -> Vec<u8> {
    let mut request = Vec::new();

    // CIP service: Write Tag
    request.push(CIP_WRITE_TAG);

    // Request path
    let path = build_symbolic_path(tag_name);
    request.push((path.len() / 2) as u8);
    request.extend_from_slice(&path);

    // Data type
    let (type_code, data) = encode_cip_value(value);
    request.extend_from_slice(&(type_code as u16).to_le_bytes());

    // Number of elements
    request.extend_from_slice(&element_count.to_le_bytes());

    // Value data
    request.extend_from_slice(&data);

    request
}

/// Build a CIP GetAttributeAll request for a class instance.
pub(crate) fn build_get_attribute_all_request(class_id: u8, instance_id: u32) -> Vec<u8> {
    let mut request = Vec::new();

    // CIP service: GetAttributeAll
    request.push(CIP_GET_ATTRIBUTES_ALL);

    // Path: class + instance
    request.push(0x03); // 3 words

    // Class segment: 0x20 = logical class segment, 8-bit
    request.push(0x20);
    request.push(class_id);
    request.push(0x00); // padding

    // Instance segment: 0x24 = logical instance segment, 8-bit
    if instance_id <= 255 {
        request.push(0x24);
        request.push(instance_id as u8);
        request.push(0x00);
        request.push(0x00);
    } else {
        // 16-bit instance
        request[1] = 0x04; // 4 words
        request.push(0x25); // logical instance segment, 16-bit
        request.push(0x00); // padding
        request.extend_from_slice(&(instance_id as u16).to_le_bytes());
    }

    request
}

/// Encode a tag name as a CIP symbolic path segment.
pub(crate) fn build_symbolic_path(tag_name: &str) -> Vec<u8> {
    let bytes = tag_name.as_bytes();
    let len = bytes.len();

    let mut path = Vec::with_capacity(len + 2);

    if len <= 255 {
        // Extended symbolic segment: type 0x91
        path.push(0x91);
        path.push(len as u8);
        path.extend_from_slice(bytes);

        // Pad to even length
        if !len.is_multiple_of(2) {
            path.push(0x00);
        }
    } else {
        // Long symbolic segment (unlikely but handle it)
        path.push(0x91);
        path.push(0x00); // placeholder
        path.extend_from_slice(&(len as u16).to_le_bytes());
        path.extend_from_slice(bytes);
        while path.len() % 2 != 0 {
            path.push(0x00);
        }
    }

    path
}

/// Encode a CipValue to (type_code, data bytes).
pub(crate) fn encode_cip_value(value: &CipValue) -> (u8, Vec<u8>) {
    match value {
        CipValue::Bool(b) => (
            CIP_TYPE_BOOL,
            vec![if *b { 0xFF } else { 0x00 }, 0x00, 0x00, 0x00],
        ),
        CipValue::Sint(v) => (CIP_TYPE_SINT, vec![*v as u8, 0x00, 0x00, 0x00]),
        CipValue::Int(v) => (CIP_TYPE_INT, v.to_le_bytes().to_vec()),
        CipValue::Dint(v) => (CIP_TYPE_DINT, v.to_le_bytes().to_vec()),
        CipValue::Real(v) => (CIP_TYPE_REAL, v.to_le_bytes().to_vec()),
        CipValue::String(s) => {
            let bytes = s.as_bytes();
            let mut data = Vec::with_capacity(88);
            let len = bytes.len().min(82) as u32;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(&bytes[..len as usize]);
            while data.len() < 88 {
                data.push(0x00);
            }
            (CIP_TYPE_STRING, data)
        }
        CipValue::Bytes(b) => (CIP_TYPE_DWORD, {
            let mut d = b.clone();
            while d.len() < 4 {
                d.push(0x00);
            }
            d
        }),
    }
}

/// Parse a CIP Read Tag response into a CipValue.
pub(crate) fn parse_read_tag_response(data: &[u8]) -> Result<CipValue> {
    if data.len() < 4 {
        anyhow::bail!("Read tag response too short ({} bytes)", data.len());
    }

    // CIP reply service (should be 0xCC = Read Tag reply)
    let service = data[0];
    if service != CIP_READ_TAG | 0x80 {
        anyhow::bail!("Unexpected CIP service reply: 0x{:02X}", service);
    }

    // Check for extended status
    let _reserved = data[1];
    let status = data[2];
    if status != CIP_STATUS_SUCCESS && status != CIP_STATUS_PARTIAL {
        let ext_status = if data.len() > 4 { data[3] } else { 0 };
        anyhow::bail!(
            "CIP Read Tag error: status=0x{:02X}, ext=0x{:02X}",
            status,
            ext_status
        );
    }

    if data.len() < 6 {
        anyhow::bail!("Read tag response missing type/data");
    }

    let type_code = u16::from_le_bytes(data[4..6].try_into().unwrap());
    let type_low = type_code as u8;

    // For partial reads, skip to data after the type field
    let data_start = 6;
    let value_data = &data[data_start..];

    decode_cip_value(type_low, value_data)
}

/// Parse a CIP Write Tag response.
pub(crate) fn parse_write_tag_response(data: &[u8]) -> Result<()> {
    if data.len() < 4 {
        anyhow::bail!("Write tag response too short");
    }

    let service = data[0];
    if service != CIP_WRITE_TAG | 0x80 {
        anyhow::bail!("Unexpected CIP service reply: 0x{:02X}", service);
    }

    let status = data[2];
    if status != CIP_STATUS_SUCCESS {
        let ext_status = if data.len() > 4 { data[3] } else { 0 };
        anyhow::bail!(
            "CIP Write Tag error: status=0x{:02X}, ext=0x{:02X}",
            status,
            ext_status
        );
    }

    Ok(())
}

/// Decode a CIP value from type code and raw data.
pub(crate) fn decode_cip_value(type_code: u8, data: &[u8]) -> Result<CipValue> {
    match type_code {
        CIP_TYPE_BOOL => {
            if data.is_empty() {
                anyhow::bail!("No data for BOOL");
            }
            Ok(CipValue::Bool(data[0] != 0))
        }
        CIP_TYPE_SINT => {
            if data.is_empty() {
                anyhow::bail!("No data for SINT");
            }
            Ok(CipValue::Sint(data[0] as i8))
        }
        CIP_TYPE_INT => {
            if data.len() < 2 {
                anyhow::bail!("Not enough data for INT");
            }
            Ok(CipValue::Int(i16::from_le_bytes(
                data[0..2].try_into().unwrap(),
            )))
        }
        CIP_TYPE_DINT => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for DINT");
            }
            Ok(CipValue::Dint(i32::from_le_bytes(
                data[0..4].try_into().unwrap(),
            )))
        }
        CIP_TYPE_REAL => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for REAL");
            }
            Ok(CipValue::Real(f32::from_le_bytes(
                data[0..4].try_into().unwrap(),
            )))
        }
        CIP_TYPE_STRING => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for STRING header");
            }
            let str_len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
            let str_data = if data.len() >= 4 + str_len {
                &data[4..4 + str_len]
            } else {
                &data[4..]
            };
            Ok(CipValue::String(
                String::from_utf8_lossy(str_data).to_string(),
            ))
        }
        CIP_TYPE_DWORD => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for DWORD");
            }
            Ok(CipValue::Bytes(data[0..4].to_vec()))
        }
        CIP_TYPE_LINT => {
            if data.len() < 8 {
                anyhow::bail!("Not enough data for LINT");
            }
            Ok(CipValue::Bytes(data[0..8].to_vec()))
        }
        _ => {
            // Unknown type -- return raw bytes
            Ok(CipValue::Bytes(data.to_vec()))
        }
    }
}

/// Parse a Symbol class instance to extract tag info.
pub(crate) fn parse_symbol_instance(data: &[u8]) -> Option<CipTag> {
    // The symbol instance data structure from GetAttributeAll:
    // Attribute 1: name (STRING)
    // Attribute 2: type (UINT)
    // The actual layout varies but we try a best-effort parse.

    if data.len() < 8 {
        return None;
    }

    // Try to read name length (first 4 bytes for CIP STRING len)
    let name_len = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if name_len == 0 || name_len > 255 || data.len() < 4 + name_len + 2 {
        return None;
    }

    let name_bytes = &data[4..4 + name_len];
    let name = String::from_utf8(name_bytes.to_vec()).ok()?;
    if name.is_empty() {
        return None;
    }

    // After the name, skip padding to even boundary, then read type info
    let type_offset = 4 + name_len + (name_len % 2);
    if data.len() < type_offset + 2 {
        return None;
    }

    // Type word: upper byte has element count flags, lower byte has type code
    let type_word = u16::from_le_bytes(data[type_offset..type_offset + 2].try_into().ok()?);
    let element_count = (type_word >> 8) & 0xFF;

    // Structure (UDT): bit 15 set, low bits = template id. Don't run from_code
    // on (id & 0xFF) — it returns None and would drop the tag from browse.
    if type_word & 0x8000 != 0 {
        return Some(CipTag {
            name,
            data_type: CipDataType::Dint,
            element_count: 1,
            template_id: Some(type_word & 0x0FFF),
        });
    }

    let type_code = (type_word & 0xFF) as u8;
    let data_type = CipDataType::from_code(type_code)?;

    Some(CipTag {
        name,
        data_type,
        element_count: if element_count == 0 { 1 } else { element_count },
        template_id: None,
    })
}


// ── Discovery & generic CIP (explorer parity) ─

/// Build a full EtherNet/IP ListIdentity packet (UDP payload). Body is empty.
pub fn build_list_identity_request() -> Vec<u8> {
    build_enip_header(CMD_LIST_IDENTITY, 0, 0, &[])
}

/// Build a full ListIdentity UDP *response* modelling the simulator's own
/// identity (vendor 1 / serial 0x12345678 / "ENIP-Sim"), so the explorer's
/// device discovery (UDP broadcast to :44818) can find the running adapter.
/// Layout mirrors `parse_list_identity_response` exactly.
pub fn build_list_identity_response() -> Vec<u8> {
    // CIP Identity object (matches `identity_attribute` in server.rs).
    let mut id = Vec::new();
    id.extend_from_slice(&1u16.to_le_bytes());           // vendor id (Rockwell)
    id.extend_from_slice(&0x0Eu16.to_le_bytes());        // device type (communications)
    id.extend_from_slice(&0x0050u16.to_le_bytes());      // product code
    id.push(20);
    id.push(11);                                         // revision 20.11
    id.extend_from_slice(&0x0020u16.to_le_bytes());      // status
    id.extend_from_slice(&0x12345678u32.to_le_bytes());  // serial number
    let name = b"ENIP-Sim";
    id.push(name.len() as u8);                           // SHORT_STRING length
    id.extend_from_slice(name);
    if id.len() % 2 != 0 {
        id.push(0); // parser pads p to even before reading state
    }
    id.push(0x04); // state

    // Common Packet Format: iface handle + timeout + item count(=1) + identity item.
    let mut body = Vec::new();
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&0x000Cu16.to_le_bytes());
    body.extend_from_slice(&(id.len() as u16).to_le_bytes());
    body.extend_from_slice(&id);
    build_enip_header(CMD_LIST_IDENTITY, 0, 0, &body)
}

/// Parse a full ListIdentity UDP reply into a device. host/port left empty
/// (caller fills from the UDP peer address).
pub fn parse_list_identity_response(packet: &[u8]) -> Option<EnipDevice> {
    if packet.len() < ENIP_HEADER_SIZE {
        return None;
    }
    let command = u16::from_le_bytes(packet[0..2].try_into().ok()?);
    if command != CMD_LIST_IDENTITY {
        return None;
    }
    let length = u16::from_le_bytes(packet[2..4].try_into().ok()?) as usize;
    if packet.len() < ENIP_HEADER_SIZE + length {
        return None;
    }
    let body = &packet[ENIP_HEADER_SIZE..ENIP_HEADER_SIZE + length];
    // Common Packet Format: interface handle(u32) + timeout(u16) + item count(u16)
    if body.len() < 8 {
        return None;
    }
    let item_count = u16::from_le_bytes(body[6..8].try_into().ok()?);
    let mut pos = 8usize;
    for _ in 0..item_count {
        if pos + 4 > body.len() {
            return None;
        }
        let type_id = u16::from_le_bytes(body[pos..pos + 2].try_into().ok()?);
        let item_len = u16::from_le_bytes(body[pos + 2..pos + 4].try_into().ok()?) as usize;
        pos += 4;
        if pos + item_len > body.len() {
            return None;
        }
        if type_id == 0x0C {
            return parse_identity_object(&body[pos..pos + item_len]);
        }
        pos += item_len;
    }
    None
}

/// Parse the raw CIP Identity object bytes (vendor..state) into an EnipDevice.
fn parse_identity_object(data: &[u8]) -> Option<EnipDevice> {
    if data.len() < 14 {
        return None;
    }
    let mut p = 0usize;
    let vendor_id = u16::from_le_bytes(data[p..p + 2].try_into().ok()?); p += 2;
    let device_type = u16::from_le_bytes(data[p..p + 2].try_into().ok()?); p += 2;
    let product_code = u16::from_le_bytes(data[p..p + 2].try_into().ok()?); p += 2;
    let revision_major = data[p];
    let revision_minor = data[p + 1];
    p += 2;
    let status = u16::from_le_bytes(data[p..p + 2].try_into().ok()?); p += 2;
    let serial_number = u32::from_le_bytes(data[p..p + 4].try_into().ok()?); p += 4;
    let product_name = if p < data.len() {
        let slen = data[p] as usize;
        p += 1;
        let sbytes = data.get(p..p + slen).unwrap_or(&[]);
        p += slen;
        String::from_utf8_lossy(sbytes).trim_end_matches('\u{0000}').to_string()
    } else {
        String::new()
    };
    if p % 2 != 0 {
        p += 1;
    }
    let state = data.get(p).copied().unwrap_or(0);
    Some(EnipDevice {
        host: String::new(),
        port: 44818,
        vendor_name: vendor_name(vendor_id).to_string(),
        vendor_id,
        device_type,
        product_code,
        revision_major,
        revision_minor,
        status,
        serial_number,
        product_name,
        state,
    })
}

/// Build a CIP GetAttributeSingle request for class/instance/attribute.
pub(crate) fn build_get_attribute_single_request(
    class_id: u16,
    instance_id: u32,
    attribute_id: u16,
) -> Vec<u8> {
    let mut request = Vec::new();
    request.push(CIP_GET_ATTRIBUTE_SINGLE);
    let (path, words) = build_epath(class_id, instance_id, Some(attribute_id));
    request.push(words);
    request.extend_from_slice(&path);
    request
}

/// Build a CIP SetAttributeSingle request with raw little-endian data.
pub(crate) fn build_set_attribute_single_request(
    class_id: u16,
    instance_id: u32,
    attribute_id: u16,
    data: &[u8],
) -> Vec<u8> {
    let mut request = Vec::new();
    request.push(CIP_SET_ATTRIBUTE_SINGLE);
    let (path, words) = build_epath(class_id, instance_id, Some(attribute_id));
    request.push(words);
    request.extend_from_slice(&path);
    request.extend_from_slice(data);
    request
}

/// Build a generic CIP service request (arbitrary service + path + data).
pub(crate) fn build_generic_cip_request(
    service: u8,
    class_id: u16,
    instance_id: u32,
    attribute_id: Option<u16>,
    data: &[u8],
) -> Vec<u8> {
    let mut request = Vec::new();
    request.push(service);
    let (path, words) = build_epath(class_id, instance_id, attribute_id);
    request.push(words);
    request.extend_from_slice(&path);
    request.extend_from_slice(data);
    request
}

/// Encode a logical EPath (class + instance + optional attribute).
/// Returns (bytes, size_in_16bit_words). Padded to even length.
pub(crate) fn build_epath(
    class_id: u16,
    instance_id: u32,
    attribute_id: Option<u16>,
) -> (Vec<u8>, u8) {
    let mut path = Vec::new();

    if class_id <= 0xFF {
        path.push(0x20);
        path.push(class_id as u8);
    } else {
        path.push(0x21);
        path.push(0x00);
        path.extend_from_slice(&class_id.to_le_bytes());
    }

    if instance_id <= 0xFF {
        path.push(0x24);
        path.push(instance_id as u8);
    } else {
        path.push(0x25);
        path.push(0x00);
        path.extend_from_slice(&(instance_id as u16).to_le_bytes());
    }

    if let Some(attr) = attribute_id {
        if attr <= 0xFF {
            path.push(0x30);
            path.push(attr as u8);
        } else {
            path.push(0x31);
            path.push(0x00);
            path.extend_from_slice(&attr.to_le_bytes());
        }
    }

    let words = ((path.len() + 1) / 2) as u8;
    if path.len() % 2 != 0 {
        path.push(0x00);
    }
    (path, words)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn identity_bytes() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_le_bytes());          // vendor (Rockwell)
        b.extend_from_slice(&0x0Eu16.to_le_bytes());       // device type (comms)
        b.extend_from_slice(&0x50u16.to_le_bytes());       // product code
        b.push(20);
        b.push(11);                                        // rev 20.11
        b.extend_from_slice(&0x0020u16.to_le_bytes());     // status
        b.extend_from_slice(&0x12345678u32.to_le_bytes()); // serial
        let name = b"1769-L1";
        b.push(name.len() as u8);                          // SHORT_STRING len
        b.extend_from_slice(name);
        b.push(0x04);                                      // state (p is even, no pad)
        b
    }

    #[test]
    fn enip_header_layout() {
        let pkt = build_enip_header(0x0063, 0x1234, 0, &[]);
        assert_eq!(pkt.len(), ENIP_HEADER_SIZE);
        assert_eq!(u16::from_le_bytes(pkt[0..2].try_into().unwrap()), 0x0063);
        assert_eq!(u16::from_le_bytes(pkt[2..4].try_into().unwrap()), 0); // body len
        assert_eq!(u32::from_le_bytes(pkt[4..8].try_into().unwrap()), 0x1234);
    }

    #[test]
    fn list_identity_request_is_well_formed() {
        let pkt = build_list_identity_request();
        assert_eq!(u16::from_le_bytes(pkt[0..2].try_into().unwrap()), CMD_LIST_IDENTITY);
        assert_eq!(u16::from_le_bytes(pkt[2..4].try_into().unwrap()), 0); // empty body
        assert_eq!(pkt.len(), ENIP_HEADER_SIZE);
    }

    #[test]
    fn epath_8bit_class_instance_attr() {
        let (path, words) = build_epath(0x6B, 1, Some(1));
        assert_eq!(path, vec![0x20, 0x6B, 0x24, 0x01, 0x30, 0x01]);
        assert_eq!(words, 3);
    }

    #[test]
    fn epath_16bit_class_no_attribute() {
        let (path, words) = build_epath(0x1234, 1, None);
        assert_eq!(path, vec![0x21, 0x00, 0x34, 0x12, 0x24, 0x01]);
        assert_eq!(words, 3);
    }

    #[test]
    fn get_attribute_single_request_shape() {
        let req = build_get_attribute_single_request(0x01, 1, 4);
        assert_eq!(req[0], CIP_GET_ATTRIBUTE_SINGLE);
        assert_eq!(req[1], 3); // 3 path words
        assert_eq!(&req[2..], &[0x20, 0x01, 0x24, 0x01, 0x30, 0x04]);
    }

    #[test]
    fn identity_object_parses() {
        let dev = parse_identity_object(&identity_bytes()).expect("parse identity");
        assert_eq!(dev.vendor_id, 1);
        assert_eq!(dev.device_type, 0x0E);
        assert_eq!(dev.product_code, 0x50);
        assert_eq!(dev.revision_major, 20);
        assert_eq!(dev.revision_minor, 11);
        assert_eq!(dev.status, 0x0020);
        assert_eq!(dev.serial_number, 0x12345678);
        assert_eq!(dev.product_name, "1769-L1");
        assert_eq!(dev.state, 0x04);
    }

    #[test]
    fn list_identity_reply_parses_end_to_end() {
        let id = identity_bytes();
        // CPF: interface handle u32 + timeout u16 + item count u16 + (type 0x000C, len, id)
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&0x000Cu16.to_le_bytes());
        body.extend_from_slice(&(id.len() as u16).to_le_bytes());
        body.extend_from_slice(&id);
        let pkt = build_enip_header(CMD_LIST_IDENTITY, 0, 0, &body);
        let dev = parse_list_identity_response(&pkt).expect("parse reply");
        assert_eq!(dev.vendor_id, 1);
        assert_eq!(dev.serial_number, 0x12345678);
        assert_eq!(dev.product_name, "1769-L1");
    }

    #[test]
    fn list_identity_rejects_wrong_command() {
        let pkt = build_enip_header(0x0065, 0, 0, &[]); // RegisterSession, not ListIdentity
        assert!(parse_list_identity_response(&pkt).is_none());
    }

    #[test]
    fn cip_value_round_trip() {
        let cases = vec![
            CipValue::Bool(true),
            CipValue::Bool(false),
            CipValue::Sint(-5),
            CipValue::Int(1234),
            CipValue::Dint(123456),
            CipValue::Real(3.5),
        ];
        for v in cases {
            let (code, data) = encode_cip_value(&v);
            let back = decode_cip_value(code, &data).unwrap();
            assert_eq!(v, back, "round trip failed for {:?}", v);
        }
    }

    #[test]
    fn strip_cip_reply_extracts_payload() {
        let reply = vec![0x8Eu8, 0x00, 0x00, 0x00, 0x01, 0x02];
        let payload = crate::client::strip_cip_reply(&reply).unwrap();
        assert_eq!(payload, vec![0x01, 0x02]);
    }

    #[test]
    fn strip_cip_reply_errors_on_status() {
        let reply = vec![0x8Eu8, 0x00, 0x05, 0x00]; // status 0x05
        assert!(crate::client::strip_cip_reply(&reply).is_err());
    }
}
// ── ForwardOpen / ForwardClose (implicit messaging) ──

/// Build a CIP ForwardOpen request (service 0x54) to the Connection Manager.
pub(crate) fn build_forward_open_request(p: &ForwardOpenParams) -> Vec<u8> {
    let mut req = Vec::new();
    req.push(CIP_FORWARD_OPEN);
    let (path, words) = build_epath(CONNECTION_MANAGER_CLASS_ID, 1, None);
    req.push(words);
    req.extend_from_slice(&path);

    req.push(p.timeout_ticks);
    req.extend_from_slice(&p.o_to_t_connection_id.to_le_bytes());
    req.extend_from_slice(&p.t_to_o_connection_id.to_le_bytes());
    req.extend_from_slice(&p.connection_serial.to_le_bytes());
    req.extend_from_slice(&p.vendor_id.to_le_bytes());
    req.extend_from_slice(&p.originator_serial.to_le_bytes());
    req.push(p.timeout_multiplier);
    req.extend_from_slice(&[0u8, 0, 0]); // reserved
    req.extend_from_slice(&p.o_to_t_rpi_us.to_le_bytes());
    req.extend_from_slice(&p.o_to_t_params.to_le_bytes());
    req.extend_from_slice(&p.t_to_o_rpi_us.to_le_bytes());
    req.extend_from_slice(&p.t_to_o_params.to_le_bytes());
    req.push(p.transport_class_trigger);

    let cp_words = ((p.connection_path.len() + 1) / 2) as u16;
    req.extend_from_slice(&cp_words.to_le_bytes());
    req.extend_from_slice(&p.connection_path);
    if p.connection_path.len() % 2 != 0 {
        req.push(0);
    }
    req
}

/// Build a CIP ForwardClose request (service 0x4E).
pub(crate) fn build_forward_close_request(
    connection_serial: u16,
    vendor_id: u16,
    originator_serial: u32,
    connection_path: &[u8],
) -> Vec<u8> {
    let mut req = Vec::new();
    req.push(CIP_FORWARD_CLOSE);
    let (path, words) = build_epath(CONNECTION_MANAGER_CLASS_ID, 1, None);
    req.push(words);
    req.extend_from_slice(&path);

    req.extend_from_slice(&connection_serial.to_le_bytes());
    req.extend_from_slice(&vendor_id.to_le_bytes());
    req.extend_from_slice(&originator_serial.to_le_bytes());

    let cp_words = ((connection_path.len() + 1) / 2) as u16;
    req.extend_from_slice(&cp_words.to_le_bytes());
    req.extend_from_slice(connection_path);
    if connection_path.len() % 2 != 0 {
        req.push(0);
    }
    req
}

/// Parse a ForwardOpen reply (service 0xD4) into the assigned connection IDs.
pub(crate) fn parse_forward_open_response(data: &[u8]) -> Result<ForwardOpenResult> {
    let payload = crate::client::strip_cip_reply(data)?;
    if payload.len() < 16 {
        anyhow::bail!("ForwardOpen reply too short ({} bytes)", payload.len());
    }
    Ok(ForwardOpenResult {
        o_to_t_connection_id: u32::from_le_bytes(payload[0..4].try_into().unwrap()),
        t_to_o_connection_id: u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        connection_serial: u16::from_le_bytes(payload[8..10].try_into().unwrap()),
        vendor_id: u16::from_le_bytes(payload[10..12].try_into().unwrap()),
        originator_serial: u32::from_le_bytes(payload[12..16].try_into().unwrap()),
    })
}

// ── SendUnitData (connected/CIP transport, implicit IO) ──

/// Build a SendUnitData (ENIP command 0x0070) frame carrying a connected CIP
/// message: connected address item (0xA1 = connection_id) + connected data item
/// (0xB1 = sequence counter + CIP payload).
pub(crate) fn build_send_unit_data(
    session_handle: u32,
    connection_id: u32,
    sequence: u16,
    cip: &[u8],
) -> Vec<u8> {
    let mut unit = Vec::new();
    unit.extend_from_slice(&0u32.to_le_bytes()); // interface handle (CIP)
    unit.extend_from_slice(&0u16.to_le_bytes()); // timeout
    unit.extend_from_slice(&2u16.to_le_bytes()); // item count

    // Connected Address item: type 0xA1, length 4, connection id
    unit.extend_from_slice(&0x00A1u16.to_le_bytes());
    unit.extend_from_slice(&4u16.to_le_bytes());
    unit.extend_from_slice(&connection_id.to_le_bytes());

    // Connected Data item: type 0xB1, length 2 + cip.len(), sequence + cip
    unit.extend_from_slice(&0x00B1u16.to_le_bytes());
    unit.extend_from_slice(&((cip.len() + 2) as u16).to_le_bytes());
    unit.extend_from_slice(&sequence.to_le_bytes());
    unit.extend_from_slice(cip);

    build_enip_header(CMD_SEND_UNIT_DATA, session_handle, 0, &unit)
}

/// Parse a SendUnitData frame into (connection_id, sequence, cip_data).
/// Used to decode cyclic T→O frames produced by the target.
pub(crate) fn parse_send_unit_data(packet: &[u8]) -> Result<(u32, u16, Vec<u8>)> {
    if packet.len() < ENIP_HEADER_SIZE + 8 {
        anyhow::bail!("SendUnitData frame too short");
    }
    let cmd = u16::from_le_bytes(packet[0..2].try_into().unwrap());
    if cmd != CMD_SEND_UNIT_DATA {
        anyhow::bail!("not a SendUnitData frame (0x{:04X})", cmd);
    }
    let body_len = u16::from_le_bytes(packet[2..4].try_into().unwrap()) as usize;
    let body = packet
        .get(ENIP_HEADER_SIZE..ENIP_HEADER_SIZE + body_len)
        .ok_or_else(|| anyhow::anyhow!("SendUnitData body truncated"))?;

    // interface handle u32 + timeout u16 + item count u16 = 8 bytes
    if body.len() < 8 {
        anyhow::bail!("SendUnitData body too short");
    }
    let mut pos = 8usize;
    let mut connection_id = 0u32;
    let mut sequence = 0u16;
    let mut data: Vec<u8> = Vec::new();

    for _ in 0..2 {
        if pos + 4 > body.len() {
            break;
        }
        let type_id = u16::from_le_bytes(body[pos..pos + 2].try_into().unwrap());
        let len = u16::from_le_bytes(body[pos + 2..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + len > body.len() {
            break;
        }
        let item = &body[pos..pos + len];
        match type_id {
            0x00A1 if len >= 4 => {
                connection_id = u32::from_le_bytes(item[0..4].try_into().unwrap());
            }
            0x00B1 if len >= 2 => {
                sequence = u16::from_le_bytes(item[0..2].try_into().unwrap());
                data = item[2..].to_vec();
            }
            _ => {}
        }
        pos += len;
    }

    Ok((connection_id, sequence, data))
}

#[cfg(test)]
mod forward_tests {
    use super::*;

    fn sample_params() -> ForwardOpenParams {
        ForwardOpenParams {
            o_to_t_connection_id: 0,
            t_to_o_connection_id: 0,
            connection_serial: 0x1234,
            vendor_id: 0x0001,
            originator_serial: 0xAABBCCDD,
            timeout_ticks: 0x0A,
            timeout_multiplier: 1,
            o_to_t_rpi_us: 1_000_000,
            o_to_t_params: 0x0200,
            t_to_o_rpi_us: 1_000_000,
            t_to_o_params: 0x0200,
            transport_class_trigger: 0xA3,
            connection_path: vec![0x01, 0x00, 0x20, 0x04, 0x24, 0x01],
        }
    }

    #[test]
    fn forward_open_request_layout() {
        let req = build_forward_open_request(&sample_params());
        assert_eq!(req[0], CIP_FORWARD_OPEN);
        // EPath to ConnMgr (0x06, inst 1): [0x20,0x06,0x24,0x01], 2 words
        assert_eq!(req[1], 2);
        assert_eq!(&req[2..6], &[0x20, 0x06, 0x24, 0x01]);
        // body[0] = timeout_ticks at index 6
        assert_eq!(req[6], 0x0A);
        // o_to_t(4)@7, t_to_o(4)@11, connection_serial(2)@15
        assert_eq!(u16::from_le_bytes(req[15..17].try_into().unwrap()), 0x1234);
        // vendor(2)@17
        assert_eq!(u16::from_le_bytes(req[17..19].try_into().unwrap()), 0x0001);
        // originator_serial(4)@19
        assert_eq!(u32::from_le_bytes(req[19..23].try_into().unwrap()), 0xAABBCCDD);
    }

    #[test]
    fn forward_close_request_layout() {
        let path = vec![0x01, 0x00, 0x20, 0x04, 0x24, 0x01];
        let req = build_forward_close_request(0x1234, 0x0001, 0xAABBCCDD, &path);
        assert_eq!(req[0], CIP_FORWARD_CLOSE);
        assert_eq!(req[1], 2);
        // body: conn_serial(2)@6, vendor(2)@8, originator_serial(4)@10
        assert_eq!(u16::from_le_bytes(req[6..8].try_into().unwrap()), 0x1234);
        assert_eq!(u16::from_le_bytes(req[8..10].try_into().unwrap()), 0x0001);
        assert_eq!(u32::from_le_bytes(req[10..14].try_into().unwrap()), 0xAABBCCDD);
    }

    #[test]
    fn parse_forward_open_reply_extracts_ids() {
        // CIP reply: service 0xD4 | reserved | status 0 | ext-words 0 | payload
        let mut reply = vec![0xD4u8, 0x00, 0x00, 0x00];
        reply.extend_from_slice(&0x10000000u32.to_le_bytes()); // o_to_t
        reply.extend_from_slice(&0x20000000u32.to_le_bytes()); // t_to_o
        reply.extend_from_slice(&0x1234u16.to_le_bytes()); // conn serial
        reply.extend_from_slice(&0x0001u16.to_le_bytes()); // vendor
        reply.extend_from_slice(&0xAABBCCDDu32.to_le_bytes()); // orig serial
        let r = parse_forward_open_response(&reply).unwrap();
        assert_eq!(r.o_to_t_connection_id, 0x10000000);
        assert_eq!(r.t_to_o_connection_id, 0x20000000);
        assert_eq!(r.connection_serial, 0x1234);
        assert_eq!(r.vendor_id, 0x0001);
        assert_eq!(r.originator_serial, 0xAABBCCDD);
    }

    #[test]
    fn parse_forward_open_reply_rejects_error_status() {
        let reply = vec![0xD4u8, 0x00, 0x01, 0x00]; // status 0x01
        assert!(parse_forward_open_response(&reply).is_err());
    }

    #[test]
    fn send_unit_data_round_trip() {
        let cip = vec![0x4Du8, 0x00, 0xAA, 0xBB];
        let frame = build_send_unit_data(0x0000ABCD, 0x12345678, 0x0001, &cip);
        // ENIP command 0x0070 little-endian
        assert_eq!(u16::from_le_bytes(frame[0..2].try_into().unwrap()), 0x0070);
        // round-trip parse
        let (conn, seq, data) = parse_send_unit_data(&frame).expect("parse");
        assert_eq!(conn, 0x12345678);
        assert_eq!(seq, 0x0001);
        assert_eq!(data, cip);
    }

    #[test]
    fn send_unit_data_rejects_wrong_command() {
        // SendRRData (0x006F) is not SendUnitData
        let frame = build_enip_header(CMD_SEND_RR_DATA, 0, 0, &[]);
        assert!(parse_send_unit_data(&frame).is_err());
    }
}
