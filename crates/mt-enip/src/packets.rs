//! EtherNet/IP encapsulation, Common Packet Format and Connection Manager
//! packet builders/parsers (ODVA CIP Vol 2 §2, Vol 1 §3-5.5).

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Result};

use crate::cip::{build_epath, CipReply};
use crate::types::*;

// ── Monotonically increasing sender context ──

static SENDER_CONTEXT_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_sender_context() -> u64 {
    SENDER_CONTEXT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ── Encapsulation header ──

/// Build an encapsulation frame: `command u16 | length u16 | session u32 |
/// status u32 | sender context u64 | options u32 | data` (Vol 2 §2-3).
pub(crate) fn build_enip_header(command: u16, session_handle: u32, _options: u32, body: &[u8]) -> Vec<u8> {
    build_enip_frame(command, session_handle, next_sender_context(), 0, body)
}

/// Same as [`build_enip_header`] with an explicit sender context and status
/// (server replies echo the request's context).
pub(crate) fn build_enip_frame(command: u16, session_handle: u32, sender_context: u64, status: u32, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ENIP_HEADER_SIZE + body.len());
    buf.extend_from_slice(&command.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u16).to_le_bytes());
    buf.extend_from_slice(&session_handle.to_le_bytes());
    buf.extend_from_slice(&status.to_le_bytes());
    buf.extend_from_slice(&sender_context.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // options
    buf.extend_from_slice(body);
    buf
}

// ── Common Packet Format ──

pub(crate) const CPF_NULL_ADDRESS: u16 = 0x0000;
pub(crate) const CPF_LIST_IDENTITY: u16 = 0x000C;
pub(crate) const CPF_CONNECTED_ADDRESS: u16 = 0x00A1;
pub(crate) const CPF_CONNECTED_DATA: u16 = 0x00B1;
pub(crate) const CPF_UNCONNECTED_DATA: u16 = 0x00B2;
/// Sockaddr Info, originator → target (carried in Forward_Open request/reply).
pub(crate) const CPF_SOCKADDR_O_TO_T: u16 = 0x8000;
/// Sockaddr Info, target → originator.
pub(crate) const CPF_SOCKADDR_T_TO_O: u16 = 0x8001;
/// Sequenced Address item used by class 0/1 implicit I/O over UDP.
pub(crate) const CPF_SEQUENCED_ADDRESS: u16 = 0x8002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CpfItem {
    pub type_id: u16,
    pub data: Vec<u8>,
}

impl CpfItem {
    pub fn new(type_id: u16, data: Vec<u8>) -> Self {
        Self { type_id, data }
    }
}

/// Encode a CPF block: `item count u16 | (type u16 | length u16 | data)…`.
pub(crate) fn build_cpf(items: &[CpfItem]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(items.len() as u16).to_le_bytes());
    for it in items {
        out.extend_from_slice(&it.type_id.to_le_bytes());
        out.extend_from_slice(&(it.data.len() as u16).to_le_bytes());
        out.extend_from_slice(&it.data);
    }
    out
}

/// Parse a CPF block starting at the item count.
pub(crate) fn parse_cpf(buf: &[u8]) -> Result<Vec<CpfItem>> {
    if buf.len() < 2 {
        bail!("CPF block too short");
    }
    let count = u16::from_le_bytes([buf[0], buf[1]]) as usize;
    let mut pos = 2;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 4 > buf.len() {
            bail!("CPF item header truncated");
        }
        let type_id = u16::from_le_bytes([buf[pos], buf[pos + 1]]);
        let len = u16::from_le_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
        pos += 4;
        if pos + len > buf.len() {
            bail!("CPF item 0x{type_id:04X} truncated");
        }
        items.push(CpfItem { type_id, data: buf[pos..pos + len].to_vec() });
        pos += len;
    }
    Ok(items)
}

/// SendRRData / SendUnitData body prefix: interface handle u32 (0 = CIP) +
/// timeout u16, followed by the CPF block (Vol 2 §2-5.4). Requests to a
/// target carry [`RR_TIMEOUT_SECS`] (pycomm3 uses 10); replies and SendUnitData
/// use 0.
pub(crate) const RR_TIMEOUT_SECS: u16 = 10;

pub(crate) fn wrap_rr_body(timeout: u16, cpf: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(6 + cpf.len());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&timeout.to_le_bytes());
    body.extend_from_slice(cpf);
    body
}

/// Build a SendRRData frame: null address item + unconnected data item
/// (0xB2) carrying `cip`, plus any `extra` trailing items (Sockaddr Info).
pub(crate) fn build_send_rr_data(session_handle: u32, cip: &[u8], extra: &[CpfItem]) -> Vec<u8> {
    let mut items = vec![
        CpfItem::new(CPF_NULL_ADDRESS, Vec::new()),
        CpfItem::new(CPF_UNCONNECTED_DATA, cip.to_vec()),
    ];
    items.extend_from_slice(extra);
    build_enip_header(CMD_SEND_RR_DATA, session_handle, 0, &wrap_rr_body(RR_TIMEOUT_SECS, &build_cpf(&items)))
}

/// Parse a SendRRData / SendUnitData body (after the encapsulation header)
/// into its CPF items.
pub(crate) fn parse_rr_body(body: &[u8]) -> Result<Vec<CpfItem>> {
    if body.len() < 6 {
        bail!("SendRRData body too short ({} bytes)", body.len());
    }
    parse_cpf(&body[6..])
}

/// Sockaddr Info item payload: big-endian `sockaddr_in` (family 2, port, IPv4, 8 zero bytes).
pub(crate) fn sockaddr_item_data(addr: [u8; 4], port: u16) -> Vec<u8> {
    let mut d = Vec::with_capacity(16);
    d.extend_from_slice(&2u16.to_be_bytes());
    d.extend_from_slice(&port.to_be_bytes());
    d.extend_from_slice(&addr);
    d.extend_from_slice(&[0u8; 8]);
    d
}

/// Parse a Sockaddr Info item payload into (ipv4, port).
pub(crate) fn parse_sockaddr_item(d: &[u8]) -> Option<([u8; 4], u16)> {
    if d.len() < 16 {
        return None;
    }
    Some(([d[4], d[5], d[6], d[7]], u16::from_be_bytes([d[2], d[3]])))
}

// ── ListIdentity (UDP 44818 broadcast / TCP) ──

/// Build a full EtherNet/IP ListIdentity packet (UDP payload). Body is empty.
pub fn build_list_identity_request() -> Vec<u8> {
    build_enip_header(CMD_LIST_IDENTITY, 0, 0, &[])
}

/// Build a ListIdentity UDP *response* for the simulator's identity (vendor 1 /
/// serial 0x12345678 / "ENIP-Sim"), advertising `ip:port` as the TCP endpoint.
///
/// The Identity item (type 0x0C) layout is ODVA Vol 2 Table 2-4.4:
/// `encapsulation protocol version u16 | sockaddr_in (16 bytes, big-endian) |
/// vendor u16 | device type u16 | product code u16 | revision major,minor |
/// status u16 | serial u32 | product name SHORT_STRING | state u8`.
pub fn build_list_identity_response(ip: [u8; 4], port: u16) -> Vec<u8> {
    let mut id = Vec::new();
    id.extend_from_slice(&1u16.to_le_bytes()); // encapsulation protocol version
    id.extend_from_slice(&sockaddr_item_data(ip, port));
    id.extend_from_slice(&1u16.to_le_bytes()); // vendor id (Rockwell)
    id.extend_from_slice(&0x0Eu16.to_le_bytes()); // device type (PLC)
    id.extend_from_slice(&0x0050u16.to_le_bytes()); // product code
    id.push(20);
    id.push(11); // revision 20.11
    id.extend_from_slice(&0x0020u16.to_le_bytes()); // status
    id.extend_from_slice(&0x12345678u32.to_le_bytes()); // serial number
    let name = b"ENIP-Sim";
    id.push(name.len() as u8); // SHORT_STRING length
    id.extend_from_slice(name);
    id.push(0x03); // state: operational (no pad before it)

    let body = build_cpf(&[CpfItem::new(CPF_LIST_IDENTITY, id)]);
    build_enip_frame(CMD_LIST_IDENTITY, 0, 0, 0, &body)
}

/// Parse a full ListIdentity UDP/TCP reply into a device. `host` is left empty
/// (caller fills it from the peer address); `port` is the sockaddr port the
/// device advertises (default 44818).
pub fn parse_list_identity_response(packet: &[u8]) -> Option<EnipDevice> {
    if packet.len() < ENIP_HEADER_SIZE {
        return None;
    }
    let command = u16::from_le_bytes(packet[0..2].try_into().ok()?);
    if command != CMD_LIST_IDENTITY {
        return None;
    }
    let length = u16::from_le_bytes(packet[2..4].try_into().ok()?) as usize;
    let status = u32::from_le_bytes(packet[8..12].try_into().ok()?);
    if status != 0 {
        return None;
    }
    let body = packet.get(ENIP_HEADER_SIZE..ENIP_HEADER_SIZE + length)?;
    let items = parse_cpf(body).ok()?;
    items
        .into_iter()
        .find(|i| i.type_id == CPF_LIST_IDENTITY)
        .and_then(|i| parse_identity_item(&i.data))
}

/// Parse the data of a ListIdentity item (type 0x0C): protocol version +
/// sockaddr + the Identity object attributes. Unlike Get_Attributes_All, the
/// trailing `state` byte follows the product name directly — no pad byte.
fn parse_identity_item(data: &[u8]) -> Option<EnipDevice> {
    // version(2) + sockaddr(16) + vendor/type/code(6) + rev(2) + status(2) + serial(4) + name len(1)
    const FIXED: usize = 2 + 16 + 6 + 2 + 2 + 4 + 1;
    if data.len() < FIXED {
        return None;
    }
    let (_, sock_port) = parse_sockaddr_item(&data[2..18])?;
    let mut p = 18usize;
    let vendor_id = u16::from_le_bytes(data[p..p + 2].try_into().ok()?);
    let device_type = u16::from_le_bytes(data[p + 2..p + 4].try_into().ok()?);
    let product_code = u16::from_le_bytes(data[p + 4..p + 6].try_into().ok()?);
    p += 6;
    let revision_major = data[p];
    let revision_minor = data[p + 1];
    p += 2;
    let status = u16::from_le_bytes(data[p..p + 2].try_into().ok()?);
    p += 2;
    let serial_number = u32::from_le_bytes(data[p..p + 4].try_into().ok()?);
    p += 4;
    let name_len = data[p] as usize;
    p += 1;
    let name_bytes = data.get(p..p + name_len)?;
    p += name_len;
    let product_name = String::from_utf8_lossy(name_bytes)
        .trim_end_matches(['\0', ' '])
        .to_string();
    let state = data.get(p).copied().unwrap_or(0);
    Some(EnipDevice {
        host: String::new(),
        port: if sock_port == 0 { 44818 } else { sock_port },
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

// ── Generic CIP requests ──

/// Build a CIP GetAttributeSingle request for class/instance/attribute.
pub(crate) fn build_get_attribute_single_request(class_id: u16, instance_id: u32, attribute_id: u16) -> Vec<u8> {
    build_generic_cip_request(CIP_GET_ATTRIBUTE_SINGLE, class_id, instance_id, Some(attribute_id), &[])
}

/// Build a CIP SetAttributeSingle request with raw little-endian data.
pub(crate) fn build_set_attribute_single_request(
    class_id: u16,
    instance_id: u32,
    attribute_id: u16,
    data: &[u8],
) -> Vec<u8> {
    build_generic_cip_request(CIP_SET_ATTRIBUTE_SINGLE, class_id, instance_id, Some(attribute_id), data)
}

/// Build a generic CIP service request (arbitrary service + path + data).
pub(crate) fn build_generic_cip_request(
    service: u8,
    class_id: u16,
    instance_id: u32,
    attribute_id: Option<u16>,
    data: &[u8],
) -> Vec<u8> {
    let (path, words) = build_epath(class_id, instance_id, attribute_id);
    let mut request = Vec::with_capacity(2 + path.len() + data.len());
    request.push(service);
    request.push(words);
    request.extend_from_slice(&path);
    request.extend_from_slice(data);
    request
}

// ── ForwardOpen / ForwardClose ──

/// Build a Forward_Open (0x54) or Large_Forward_Open (0x5B) request to the
/// Connection Manager (Vol 1 Table 3-5.16). Field order:
///
/// `priority/tick | timeout ticks | O→T conn id u32 | T→O conn id u32 |
/// conn serial u16 | vendor u16 | originator serial u32 | timeout multiplier |
/// 3 reserved | O→T RPI u32 | O→T params (u16, u32 if large) | T→O RPI u32 |
/// T→O params | transport class/trigger | connection path size (USINT words) |
/// connection path`
///
/// Verified against cpppo's RSLinx capture (`54 02 20 06 24 01 07 9b …` with a
/// 1-byte path size) and pycomm3 `_forward_open`.
pub(crate) fn build_forward_open_request(p: &ForwardOpenParams) -> Result<Vec<u8>> {
    let mut req = Vec::new();
    req.push(if p.large { CIP_LARGE_FORWARD_OPEN } else { CIP_FORWARD_OPEN });
    let (path, words) = build_epath(CONNECTION_MANAGER_CLASS_ID, 1, None);
    req.push(words);
    req.extend_from_slice(&path);

    req.push(p.priority_time_tick);
    req.push(p.timeout_ticks);
    req.extend_from_slice(&p.o_to_t_connection_id.to_le_bytes());
    req.extend_from_slice(&p.t_to_o_connection_id.to_le_bytes());
    req.extend_from_slice(&p.connection_serial.to_le_bytes());
    req.extend_from_slice(&p.vendor_id.to_le_bytes());
    req.extend_from_slice(&p.originator_serial.to_le_bytes());
    req.push(p.timeout_multiplier);
    req.extend_from_slice(&[0u8, 0, 0]); // reserved
    req.extend_from_slice(&p.o_to_t_rpi_us.to_le_bytes());
    push_network_params(&mut req, p.o_to_t_params, p.large)?;
    req.extend_from_slice(&p.t_to_o_rpi_us.to_le_bytes());
    push_network_params(&mut req, p.t_to_o_params, p.large)?;
    req.push(p.transport_class_trigger);

    let cp_words = p.connection_path.len().div_ceil(2);
    if cp_words > 0xFF {
        bail!("connection path too long");
    }
    req.push(cp_words as u8);
    req.extend_from_slice(&p.connection_path);
    if !p.connection_path.len().is_multiple_of(2) {
        req.push(0);
    }
    Ok(req)
}

fn push_network_params(req: &mut Vec<u8>, params: u32, large: bool) -> Result<()> {
    if large {
        req.extend_from_slice(&params.to_le_bytes());
    } else {
        let short = u16::try_from(params)
            .map_err(|_| anyhow::anyhow!("network connection parameters 0x{params:08X} need Large_Forward_Open"))?;
        req.extend_from_slice(&short.to_le_bytes());
    }
    Ok(())
}

/// Build a Forward_Close request (0x4E): `priority/tick | timeout ticks |
/// conn serial u16 | vendor u16 | originator serial u32 | path size (USINT
/// words) | reserved | path` (Vol 1 Table 3-5.21).
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

    req.push(0x0A); // priority/time_tick
    req.push(0x05); // timeout ticks
    req.extend_from_slice(&connection_serial.to_le_bytes());
    req.extend_from_slice(&vendor_id.to_le_bytes());
    req.extend_from_slice(&originator_serial.to_le_bytes());

    req.push(connection_path.len().div_ceil(2) as u8);
    req.push(0); // reserved
    req.extend_from_slice(connection_path);
    if !connection_path.len().is_multiple_of(2) {
        req.push(0);
    }
    req
}

/// Parse a Forward_Open reply (0xD4 / 0xDB). Error replies surface as
/// [`crate::cip::CipError`] with the extended status (e.g. 0x0111 RPI not
/// supported) intact.
///
/// Success body: `O→T conn id | T→O conn id | conn serial | vendor |
/// originator serial | O→T API u32 | T→O API u32 | app reply size | reserved`.
pub(crate) fn parse_forward_open_response(data: &[u8], large: bool) -> Result<ForwardOpenResult> {
    let service = if large { CIP_LARGE_FORWARD_OPEN } else { CIP_FORWARD_OPEN };
    let reply = CipReply::parse(data)?.expect(service, false)?;
    let payload = &reply.data;
    if payload.len() < 16 {
        bail!("ForwardOpen reply too short ({} bytes)", payload.len());
    }
    let (o_to_t_api_us, t_to_o_api_us) = if payload.len() >= 24 {
        (
            u32::from_le_bytes(payload[16..20].try_into().unwrap()),
            u32::from_le_bytes(payload[20..24].try_into().unwrap()),
        )
    } else {
        (0, 0)
    };
    Ok(ForwardOpenResult {
        o_to_t_connection_id: u32::from_le_bytes(payload[0..4].try_into().unwrap()),
        t_to_o_connection_id: u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        connection_serial: u16::from_le_bytes(payload[8..10].try_into().unwrap()),
        vendor_id: u16::from_le_bytes(payload[10..12].try_into().unwrap()),
        originator_serial: u32::from_le_bytes(payload[12..16].try_into().unwrap()),
        o_to_t_api_us,
        t_to_o_api_us,
    })
}

/// Parse a Forward_Open / Large_Forward_Open *request* (CIP message, starting
/// at the service byte). Used by the simulator.
pub(crate) fn parse_forward_open_request(cip: &[u8]) -> Option<ForwardOpenParams> {
    let large = *cip.first()? == CIP_LARGE_FORWARD_OPEN;
    let path_words = *cip.get(1)? as usize;
    let b = cip.get(2 + path_words * 2..)?;
    let params_len = if large { 4 } else { 2 };
    // fixed part: 2 + 4 + 4 + 2 + 2 + 4 + 1 + 3 + (4 + p) + (4 + p) + 1 + 1(path size)
    let fixed = 2 + 4 + 4 + 2 + 2 + 4 + 1 + 3 + 2 * (4 + params_len) + 1 + 1;
    if b.len() < fixed {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
    let mut o = 2;
    let o_to_t_connection_id = u32_at(o);
    let t_to_o_connection_id = u32_at(o + 4);
    let connection_serial = u16_at(o + 8);
    let vendor_id = u16_at(o + 10);
    let originator_serial = u32_at(o + 12);
    let timeout_multiplier = b[o + 16];
    o += 20; // past the 3 reserved bytes
    let o_to_t_rpi_us = u32_at(o);
    o += 4;
    let o_to_t_params = if large { u32_at(o) } else { u16_at(o) as u32 };
    o += params_len;
    let t_to_o_rpi_us = u32_at(o);
    o += 4;
    let t_to_o_params = if large { u32_at(o) } else { u16_at(o) as u32 };
    o += params_len;
    let transport_class_trigger = b[o];
    let cp_words = b[o + 1] as usize;
    let connection_path = b.get(o + 2..o + 2 + cp_words * 2)?.to_vec();
    Some(ForwardOpenParams {
        o_to_t_run_idle: true,
        o_to_t_connection_id,
        t_to_o_connection_id,
        connection_serial,
        vendor_id,
        originator_serial,
        priority_time_tick: b[0],
        timeout_ticks: b[1],
        timeout_multiplier,
        o_to_t_rpi_us,
        o_to_t_params,
        t_to_o_rpi_us,
        t_to_o_params,
        transport_class_trigger,
        large,
        connection_path,
    })
}

// ── SendUnitData (connected explicit messaging, class 3) ──

/// Build a SendUnitData (0x0070) frame carrying a connected CIP message:
/// connected address item (0xA1 = connection id) + connected data item
/// (0xB1 = 16-bit sequence count + CIP payload).
pub(crate) fn build_send_unit_data(session_handle: u32, connection_id: u32, sequence: u16, cip: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(2 + cip.len());
    data.extend_from_slice(&sequence.to_le_bytes());
    data.extend_from_slice(cip);
    let items = [
        CpfItem::new(CPF_CONNECTED_ADDRESS, connection_id.to_le_bytes().to_vec()),
        CpfItem::new(CPF_CONNECTED_DATA, data),
    ];
    build_enip_header(CMD_SEND_UNIT_DATA, session_handle, 0, &wrap_rr_body(0, &build_cpf(&items)))
}

/// Split a SendUnitData body (after the encapsulation header) into
/// (connection id, sequence count, CIP payload).
pub(crate) fn parse_unit_data_body(body: &[u8]) -> Result<(u32, u16, Vec<u8>)> {
    let items = parse_rr_body(body)?;
    let mut conn = None;
    let mut seq_data = None;
    for it in items {
        match it.type_id {
            CPF_CONNECTED_ADDRESS if it.data.len() >= 4 => {
                conn = Some(u32::from_le_bytes(it.data[0..4].try_into().unwrap()));
            }
            CPF_CONNECTED_DATA if it.data.len() >= 2 => {
                seq_data = Some((u16::from_le_bytes([it.data[0], it.data[1]]), it.data[2..].to_vec()));
            }
            _ => {}
        }
    }
    match (conn, seq_data) {
        (Some(c), Some((s, d))) => Ok((c, s, d)),
        _ => bail!("SendUnitData missing connected address/data item"),
    }
}

/// Parse a full SendUnitData frame into (connection_id, sequence, cip_data).
#[cfg(test)]
pub(crate) fn parse_send_unit_data(packet: &[u8]) -> Result<(u32, u16, Vec<u8>)> {
    if packet.len() < ENIP_HEADER_SIZE {
        bail!("SendUnitData frame too short");
    }
    let cmd = u16::from_le_bytes(packet[0..2].try_into().unwrap());
    if cmd != CMD_SEND_UNIT_DATA {
        bail!("not a SendUnitData frame (0x{:04X})", cmd);
    }
    let body_len = u16::from_le_bytes(packet[2..4].try_into().unwrap()) as usize;
    let body = packet
        .get(ENIP_HEADER_SIZE..ENIP_HEADER_SIZE + body_len)
        .ok_or_else(|| anyhow::anyhow!("SendUnitData body truncated"))?;
    parse_unit_data_body(body)
}

// ── Class 0/1 implicit I/O over UDP 2222 ──

/// UDP port for implicit I/O (Vol 2 §3-3).
pub(crate) const IO_UDP_PORT: u16 = 2222;

/// Build an implicit I/O datagram. It is a bare CPF block (no encapsulation
/// header): Sequenced Address item (connection id + 32-bit encapsulation
/// sequence number) and Connected Data item (16-bit CIP sequence count +
/// payload; Vol 2 §3-3.2.2).
pub(crate) fn build_io_packet(connection_id: u32, seq32: u32, cip_seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut addr = Vec::with_capacity(8);
    addr.extend_from_slice(&connection_id.to_le_bytes());
    addr.extend_from_slice(&seq32.to_le_bytes());
    let mut data = Vec::with_capacity(2 + payload.len());
    data.extend_from_slice(&cip_seq.to_le_bytes());
    data.extend_from_slice(payload);
    build_cpf(&[CpfItem::new(CPF_SEQUENCED_ADDRESS, addr), CpfItem::new(CPF_CONNECTED_DATA, data)])
}

/// Parse an implicit I/O datagram into (connection id, sequence number,
/// CIP sequence count, payload).
pub(crate) fn parse_io_packet(buf: &[u8]) -> Result<(u32, u32, u16, Vec<u8>)> {
    let items = parse_cpf(buf)?;
    let mut addr = None;
    let mut data = None;
    for it in items {
        match it.type_id {
            CPF_SEQUENCED_ADDRESS if it.data.len() >= 8 => {
                addr = Some((
                    u32::from_le_bytes(it.data[0..4].try_into().unwrap()),
                    u32::from_le_bytes(it.data[4..8].try_into().unwrap()),
                ));
            }
            CPF_CONNECTED_DATA if it.data.len() >= 2 => {
                data = Some((u16::from_le_bytes([it.data[0], it.data[1]]), it.data[2..].to_vec()));
            }
            _ => {}
        }
    }
    match (addr, data) {
        (Some((c, s32)), Some((s16, d))) => Ok((c, s32, s16, d)),
        _ => bail!("I/O packet missing sequenced address or connected data item"),
    }
}
