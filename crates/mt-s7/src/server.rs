//! S7 protocol SERVER (simulator).
//!
//! Listens on TCP and speaks ISO-on-TCP (TPKT + COTP + S7) so that external
//! S7 clients (s7-explorer itself, snap7, TIA Portal, etc.) can connect to the
//! simulator and read/write an in-memory tag image.
//!
//! Supported server-side jobs:
//! - S7 Setup Communication (negotiation)
//! - Read Var (0x04)
//! - Write Var (0x05)
//!
//! Diagnostics / block management are not simulated — a simulator exists to
//! exercise data access, not to impersonate a CPU's diagnostic buffer.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::types::*;

/// Coordinate identifying a single tag in the simulated PLC image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TagCoord {
    area: u8,
    db: u16,
    offset: u16,
    bit: u8,
    data_type: S7DataType,
}

fn coord_of(tag: &S7Tag) -> TagCoord {
    TagCoord {
        area: tag.area.code(),
        db: tag.db_number,
        offset: tag.offset,
        bit: tag.bit_offset,
        data_type: tag.data_type.clone(),
    }
}

fn ts_to_dtype(ts: u8) -> Option<S7DataType> {
    match ts {
        TS_BIT => Some(S7DataType::Bool),
        TS_BYTE => Some(S7DataType::Byte),
        TS_WORD => Some(S7DataType::Word),
        TS_DWORD => Some(S7DataType::DWord),
        TS_REAL => Some(S7DataType::Real),
        TS_TIMER => Some(S7DataType::Timer),
        TS_COUNTER => Some(S7DataType::Counter),
        _ => None,
    }
}

fn dtype_size(dt: &S7DataType) -> usize {
    match dt {
        S7DataType::Bool => 1,
        S7DataType::Byte => 1,
        S7DataType::Word => 2,
        S7DataType::DWord => 4,
        S7DataType::Real => 4,
        S7DataType::String => 1,
        S7DataType::Timer | S7DataType::Counter => 2,
    }
}

/// Split the 24-bit address of an item into (byte offset, bit). Timers and counters are
/// addressed by number, everything else by bit offset.
fn split_item_address(area: u8, address: u32) -> (u16, u8) {
    if area == AREA_TIMER || area == AREA_COUNTER {
        (address as u16, 0)
    } else {
        ((address / 8) as u16, (address % 8) as u8)
    }
}

/// Transport size a real PLC reports for the data it returns for `ts`: timers and counters come
/// back as octet strings, everything else echoes the request.
fn response_transport_size(ts: u8) -> u8 {
    if ts == TS_TIMER || ts == TS_COUNTER {
        TS_OCTET_STRING
    } else {
        ts
    }
}

fn encode_value_bytes(v: &S7Value, bit: u8) -> Vec<u8> {
    match v {
        S7Value::Bool(b) => vec![if *b { 1u8 << bit } else { 0 }],
        S7Value::Byte(x) => vec![*x],
        S7Value::Word(x) => x.to_be_bytes().to_vec(),
        S7Value::DWord(x) => x.to_be_bytes().to_vec(),
        S7Value::Real(x) => x.to_be_bytes().to_vec(),
        S7Value::Timer(ms) => encode_timer_ms(*ms).to_be_bytes().to_vec(),
        S7Value::Counter(n) => encode_counter(*n).to_be_bytes().to_vec(),
        S7Value::String(s) => {
            let mut b = Vec::new();
            b.push(s.len() as u8 + 2);
            b.push(s.len() as u8);
            b.extend_from_slice(s.as_bytes());
            b
        }
    }
}

fn decode_value_bytes(data: &[u8], dt: &S7DataType, bit: u8) -> S7Value {
    match dt {
        S7DataType::Bool => S7Value::Bool(!data.is_empty() && (data[0] & (1 << bit)) != 0),
        S7DataType::Byte => S7Value::Byte(data.first().copied().unwrap_or(0)),
        S7DataType::Word => S7Value::Word(
            data.get(..2)
                .map(|d| u16::from_be_bytes([d[0], d[1]]))
                .unwrap_or(0),
        ),
        S7DataType::DWord => S7Value::DWord(
            data.get(..4)
                .map(|d| u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
                .unwrap_or(0),
        ),
        S7DataType::Real => S7Value::Real(
            data.get(..4)
                .map(|d| f32::from_bits(u32::from_be_bytes([d[0], d[1], d[2], d[3]])))
                .unwrap_or(0.0),
        ),
        S7DataType::Timer => S7Value::Timer(
            data.get(..2)
                .map(|d| decode_timer_ms(u16::from_be_bytes([d[0], d[1]])))
                .unwrap_or(0),
        ),
        S7DataType::Counter => S7Value::Counter(
            data.get(..2)
                .map(|d| decode_counter(u16::from_be_bytes([d[0], d[1]])))
                .unwrap_or(0),
        ),
        S7DataType::String => S7Value::String(
            data.get(2..)
                .map(|d| String::from_utf8_lossy(d).to_string())
                .unwrap_or_default(),
        ),
    }
}

/// A running S7 simulator instance.
pub struct S7Server {
    running: Arc<AtomicBool>,
    store: Arc<RwLock<HashMap<TagCoord, S7Value>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    port: u16,
}

impl S7Server {
    /// Bind on `port` (0 = OS-assigned) and seed the tag image with `initial`.
    pub async fn start(port: u16, initial: Vec<(S7Tag, S7Value)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port)).await?;
        let actual_port = listener.local_addr()?.port();

        let mut store = HashMap::new();
        for (tag, value) in initial {
            store.insert(coord_of(&tag), value);
        }
        let store = Arc::new(RwLock::new(store));
        let running = Arc::new(AtomicBool::new(true));

        let store_c = Arc::clone(&store);
        let running_c = Arc::clone(&running);
        let handle = tokio::spawn(async move {
            loop {
                if !running_c.load(Ordering::Relaxed) {
                    break;
                }
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let s = Arc::clone(&store_c);
                        tokio::spawn(async move {
                            let _ = handle_client(stream, s).await;
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            running,
            store,
            handle: Mutex::new(Some(handle)),
            port: actual_port,
        })
    }

    /// The actually-bound port (useful when started on port 0).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Overwrite a single tag's value in the image.
    pub async fn set_value(&self, tag: &S7Tag, value: S7Value) {
        self.store.write().await.insert(coord_of(tag), value);
    }

    /// Convenience setter for boolean tags.
    pub async fn set_bool(&self, tag: &S7Tag, active: bool) {
        self.store
            .write()
            .await
            .insert(coord_of(tag), S7Value::Bool(active));
    }

    /// Snapshot of every tag coordinate → value (for UI display).
    pub async fn snapshot(&self) -> Vec<(S7Tag, S7Value)> {
        self.store
            .read()
            .await
            .iter()
            .map(|(c, v)| (tag_from_coord(*c), v.clone()))
            .collect()
    }

    /// Stop accepting connections and abort the accept loop.
    pub async fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.lock().await.take() {
            h.abort();
        }
    }
}

fn tag_from_coord(c: TagCoord) -> S7Tag {
    let area = S7Area::from_code(c.area).unwrap_or(S7Area::DB);
    S7Tag {
        area,
        db_number: c.db,
        offset: c.offset,
        bit_offset: c.bit,
        data_type: c.data_type,
        length: 1,
        name: None,
    }
}

// ── Wire helpers ──

async fn read_tpkt_payload(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut hdr = [0u8; 4];
    stream
        .read_exact(&mut hdr)
        .await
        .map_err(|e| e.to_string())?;
    if hdr[0] != TPKT_VERSION {
        return Err(format!("invalid TPKT version 0x{:02X}", hdr[0]));
    }
    let total = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
    if total < 4 {
        return Err(format!("invalid TPKT length {total}"));
    }
    let mut payload = vec![0u8; total - 4];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|e| e.to_string())?;
    Ok(payload)
}

async fn write_tpkt(stream: &mut TcpStream, payload: &[u8]) -> Result<(), String> {
    let total = 4 + payload.len();
    let mut pkt = Vec::with_capacity(total);
    pkt.push(TPKT_VERSION);
    pkt.push(0x00);
    pkt.extend_from_slice(&(total as u16).to_be_bytes());
    pkt.extend_from_slice(payload);
    stream
        .write_all(&pkt)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Wrap an S7 PDU in a COTP DT header (0x02, 0xF0, 0x80).
fn cotp_dt_wrap(s7: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(3 + s7.len());
    v.push(0x02);
    v.push(COTP_DT);
    v.push(0x80);
    v.extend_from_slice(s7);
    v
}

/// Build a 12-byte ACK_DATA header (error class + error code = 0).
fn ack_header(pdu_ref: u16, param_len: u16, data_len: u16) -> Vec<u8> {
    let mut h = Vec::with_capacity(12);
    h.push(S7_PROTOCOL_ID);
    h.push(0x03); // ACK_DATA
    h.extend_from_slice(&0x0000u16.to_be_bytes()); // reserved
    h.extend_from_slice(&pdu_ref.to_be_bytes());
    h.extend_from_slice(&param_len.to_be_bytes());
    h.extend_from_slice(&data_len.to_be_bytes());
    h.push(0x00); // error class
    h.push(0x00); // error code
    h
}

async fn handle_client(
    mut stream: TcpStream,
    store: Arc<RwLock<HashMap<TagCoord, S7Value>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. COTP Connection Request → Connection Confirm.
    let cr = read_tpkt_payload(&mut stream).await?;
    let mut cc = cr.clone();
    if cc.len() >= 2 {
        cc[1] = COTP_CC;
    }
    write_tpkt(&mut stream, &cc).await?;

    // 2. S7 Setup Communication.
    let setup = read_tpkt_payload(&mut stream).await?;
    if setup.len() < 4 {
        return Ok(());
    }
    let pdu_ref = setup
        .get(3 + 4..3 + 6)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .unwrap_or(0);
    let setup_resp = build_setup_comm_resp(pdu_ref);
    write_tpkt(&mut stream, &cotp_dt_wrap(&setup_resp)).await?;

    // 3. Request loop.
    loop {
        let pkt = match read_tpkt_payload(&mut stream).await {
            Ok(p) => p,
            Err(_) => break, // peer closed / error
        };
        if pkt.len() < 4 {
            break;
        }
        let s7 = &pkt[3..]; // skip COTP DT header
        if s7.len() < 10 || s7[0] != S7_PROTOCOL_ID {
            break;
        }
        let msg_type = s7[1];
        let pdu_ref = u16::from_be_bytes([s7[4], s7[5]]);
        let param_len = u16::from_be_bytes([s7[6], s7[7]]) as usize;
        let data_len = u16::from_be_bytes([s7[8], s7[9]]) as usize;
        let params = s7.get(10..10 + param_len).unwrap_or(&[]);
        let data = s7
            .get(10 + param_len..10 + param_len + data_len)
            .unwrap_or(&[]);
        let func = params.first().copied().unwrap_or(0);

        let resp = match (msg_type, func) {
            (S7_MSG_TYPE_JOB, S7_READ_VAR) => build_read_resp(pdu_ref, params, &store).await,
            (S7_MSG_TYPE_JOB, S7_WRITE_VAR) => build_write_resp(pdu_ref, params, data, &store).await,
            (S7_MSG_TYPE_JOB, S7_LIST_BLOCKS) => build_block_list_resp(pdu_ref, &store).await,
            (S7_MSG_TYPE_JOB, S7_UPLOAD) => build_upload_resp(pdu_ref),
            (S7_MSG_TYPE_JOB, S7_DELETE_BLOCK) => build_delete_resp(pdu_ref),
            // SZL / CPU diagnostics request (user-data, function 0x00).
            (S7_MSG_TYPE_USER_DATA, S7_CPU_FUNC) => build_szl_resp(pdu_ref, params),
            _ => build_unsupported_resp(pdu_ref),
        };
        if write_tpkt(&mut stream, &cotp_dt_wrap(&resp)).await.is_err() {
            break;
        }
    }
    Ok(())
}

fn build_setup_comm_resp(pdu_ref: u16) -> Vec<u8> {
    let mut params = Vec::new();
    params.push(S7_SETUP_COMM);
    params.push(0x00);
    params.extend_from_slice(&0x0100u16.to_be_bytes()); // max AMQ caller
    params.extend_from_slice(&0x0100u16.to_be_bytes()); // max AMQ callee
    params.extend_from_slice(&DEFAULT_PDU_SIZE.to_be_bytes());

    let mut s7 = ack_header(pdu_ref, params.len() as u16, 0);
    s7.extend_from_slice(&params);
    s7
}

async fn build_read_resp(
    pdu_ref: u16,
    params: &[u8],
    store: &Arc<RwLock<HashMap<TagCoord, S7Value>>>,
) -> Vec<u8> {
    let item_count = params.get(1).copied().unwrap_or(0) as usize;
    let items = params.get(2..).unwrap_or(&[]);
    let guard = store.read().await;

    let mut resp_data = Vec::new();
    for i in 0..item_count {
        let off = i * 12;
        let item = items.get(off..off + 12).unwrap_or(&[]);
        if item.len() < 12 {
            resp_data.extend_from_slice(&[0x05, 0x00, 0x00, 0x00]);
            continue;
        }
        let ts = item[3];
        let num_elems = u16::from_be_bytes([item[4], item[5]]);
        let db = u16::from_be_bytes([item[6], item[7]]);
        let area = item[8];
        let off_bits =
            ((item[9] as u32) << 16) | ((item[10] as u32) << 8) | (item[11] as u32);
        let (offset, bit) = split_item_address(area, off_bits);

        let bytes = match ts_to_dtype(ts) {
            Some(dt) => {
                let coord = TagCoord {
                    area,
                    db,
                    offset,
                    bit,
                    data_type: dt.clone(),
                };
                match guard.get(&coord) {
                    Some(v) => encode_value_bytes(v, bit),
                    None => vec![0u8; dtype_size(&dt) * num_elems as usize],
                }
            }
            None => vec![0u8; 1],
        };

        resp_data.push(0xFF); // item return code: success
        resp_data.push(response_transport_size(ts));
        resp_data.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        resp_data.extend_from_slice(&bytes);
    }
    drop(guard);

    let mut out_params = Vec::new();
    out_params.push(S7_READ_VAR);
    out_params.push(item_count as u8);

    let mut s7 = ack_header(pdu_ref, out_params.len() as u16, resp_data.len() as u16);
    s7.extend_from_slice(&out_params);
    s7.extend_from_slice(&resp_data);
    s7
}

async fn build_write_resp(
    pdu_ref: u16,
    params: &[u8],
    data: &[u8],
    store: &Arc<RwLock<HashMap<TagCoord, S7Value>>>,
) -> Vec<u8> {
    let item_count = params.get(1).copied().unwrap_or(0) as usize;
    let items = params.get(2..).unwrap_or(&[]);
    let mut return_codes: Vec<u8> = Vec::new();

    if item_count >= 1 && items.len() >= 12 {
        let item = &items[..12];
        let ts = item[3];
        let db = u16::from_be_bytes([item[6], item[7]]);
        let area = item[8];
        let off_bits =
            ((item[9] as u32) << 16) | ((item[10] as u32) << 8) | (item[11] as u32);
        let (offset, bit) = split_item_address(area, off_bits);

        // Write request data: [return_code, transport_size, len(2), value_bytes...]
        if data.len() >= 4 {
            let vlen = u16::from_be_bytes([data[2], data[3]]) as usize;
            let vbytes = data.get(4..4 + vlen).unwrap_or(&[]);
            if let Some(dt) = ts_to_dtype(ts) {
                let coord = TagCoord {
                    area,
                    db,
                    offset,
                    bit,
                    data_type: dt.clone(),
                };
                let val = decode_value_bytes(vbytes, &dt, bit);
                store.write().await.insert(coord, val);
            }
        }
        return_codes.push(0xFF);
    }

    let mut out_params = Vec::new();
    out_params.push(S7_WRITE_VAR);
    out_params.push(item_count as u8);

    let mut s7 = ack_header(pdu_ref, out_params.len() as u16, return_codes.len() as u16);
    s7.extend_from_slice(&out_params);
    s7.extend_from_slice(&return_codes);
    s7
}

/// Synthesize a block list from the simulated image: every distinct DB
/// referenced by the seeded tags is reported as a DB block. Other block types
/// (OB/FB/FC) aren't modelled — this is a data-access simulator, not a CPU.
async fn build_block_list_resp(
    pdu_ref: u16,
    store: &Arc<RwLock<HashMap<TagCoord, S7Value>>>,
) -> Vec<u8> {
    let guard = store.read().await;
    let mut dbs: Vec<u16> = guard
        .keys()
        .filter(|c| c.area == AREA_DB)
        .map(|c| c.db)
        .collect();
    dbs.sort_unstable();
    dbs.dedup();
    drop(guard);

    // Data field: [return_code 0xFF][transport_size 0x00][inner_len(2)][type, count, nums...]
    let inner_len = 2 + dbs.len() * 2;
    let mut body = vec![0xFFu8, 0x00];
    body.extend_from_slice(&(inner_len as u16).to_be_bytes());
    body.push(0x41); // type code: DB (matches client's block-list parser)
    body.push(dbs.len() as u8);
    for db in &dbs {
        body.extend_from_slice(&db.to_be_bytes());
    }

    let params = vec![S7_LIST_BLOCKS];
    let mut s7 = ack_header(pdu_ref, params.len() as u16, body.len() as u16);
    s7.extend_from_slice(&params);
    s7.extend_from_slice(&body);
    s7
}

/// Synthetic block upload — returns a placeholder body so the "Save" action in
/// the UI produces a file instead of erroring. Not a valid .mc7.
fn build_upload_resp(pdu_ref: u16) -> Vec<u8> {
    let body = b"SIM7-SIMULATED-BLOCK".to_vec();
    let mut data = vec![0xFFu8, 0x00];
    data.extend_from_slice(&(body.len() as u16).to_be_bytes());
    data.extend_from_slice(&body);

    let params = vec![S7_UPLOAD];
    let mut s7 = ack_header(pdu_ref, params.len() as u16, data.len() as u16);
    s7.extend_from_slice(&params);
    s7.extend_from_slice(&data);
    s7
}

/// Simulated block delete — always succeeds (no persistent block store).
fn build_delete_resp(pdu_ref: u16) -> Vec<u8> {
    let params = vec![S7_DELETE_BLOCK];
    let mut s7 = ack_header(pdu_ref, params.len() as u16, 0);
    s7.extend_from_slice(&params);
    s7
}

/// Synthesize SZL responses so diagnostics clients get a clean (error-free)
/// picture of a fictional CPU instead of a protocol error.
fn build_szl_resp(pdu_ref: u16, params: &[u8]) -> Vec<u8> {
    // SZL request param: [CPU_FUNC, SZL_READ, 0x12, 0x04, 0x01, szl_id(2), index(2), 0x0000]
    let szl_id = if params.len() >= 7 {
        u16::from_be_bytes([params[5], params[6]])
    } else {
        0
    };

    let data: Vec<u8> = match szl_id {
        SZL_MODULE_IDENTITY => {
            let mut tlv = Vec::new();
            tlv.extend(tlv_item(0x01, b"S7 Simulator")); // module type
            tlv.extend(tlv_item(0x05, b"SIM-000001")); // serial number
            tlv.extend(tlv_item(0x06, b"SIMULATOR")); // AS name
            tlv.extend(tlv_item(0x07, b"S7-1500 SIM")); // module name
            tlv.extend(tlv_item(0x08, b"1.0.0")); // firmware
            let mut d = vec![0xFFu8, 0x00]; // return code + transport size
            d.extend_from_slice(&(tlv.len() as u16).to_be_bytes());
            d.extend_from_slice(&tlv);
            d
        }
        // parse_plc_mode scans for the first 0x08/0x04/0x00 byte → lead with 0x08 = RUN.
        SZL_OPERATING_MODE => vec![0x08],
        // parse_diag_buffer reads entry_count at data[4..6]; report zero entries.
        SZL_DIAG_BUFFER => vec![0xFF, 0x00, 0x00, 0x04, 0x00, 0x00],
        _ => vec![0xFF, 0x00, 0x00, 0x00],
    };

    let out_params = vec![S7_CPU_FUNC, S7_SZL_READ, 0x12, 0x0A, 0x00, 0x01];
    let mut s7 = ack_header(pdu_ref, out_params.len() as u16, data.len() as u16);
    s7.extend_from_slice(&out_params);
    s7.extend_from_slice(&data);
    s7
}

fn tlv_item(id: u8, val: &[u8]) -> Vec<u8> {
    let mut v = vec![id, val.len() as u8];
    v.extend_from_slice(val);
    v
}

fn build_unsupported_resp(pdu_ref: u16) -> Vec<u8> {
    // ACK_DATA with error class 0x81 (protocol error) / code 0x00.
    let mut s7 = ack_header(pdu_ref, 0, 0);
    // Overwrite error class/code bytes (last two of the 12-byte header).
    let n = s7.len();
    if n >= 2 {
        s8_set(&mut s7, n - 2, 0x81);
        s8_set(&mut s7, n - 1, 0x00);
    }
    s7
}

fn s8_set(buf: &mut Vec<u8>, idx: usize, val: u8) {
    if idx < buf.len() {
        buf[idx] = val;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::S7Client;
    use mt_protocol::ProtocolClient;

    fn cfg(port: u16) -> S7Config {
        S7Config {
            id: "t".into(),
            host: "127.0.0.1".into(),
            port,
            rack: 0,
            slot: 0,
            poll_interval_ms: 1000,
        }
    }

    /// Real S7 client → real S7 server round trip (read + write).
    /// Guards the 12-byte ACK_DATA header fix and the simulator wire path.
    #[tokio::test]
    async fn server_roundtrip_read_write() {
        // Seed DB1.DBD0 = Real(1234.5), DB1.DBW4 = Word(4660), DB1.DBX8.0 = Bool(true).
        // "DBD" parses to DWord; override to Real so the coord + value variant agree.
        let mut real_tag = S7Tag::parse_address("DB1.DBD0").unwrap();
        real_tag.data_type = S7DataType::Real;
        let seed = vec![
            (real_tag.clone(), S7Value::Real(1234.5)),
            (
                S7Tag::parse_address("DB1.DBW4").unwrap(),
                S7Value::Word(4660),
            ),
            (
                S7Tag::parse_address("DB1.DBX8.0").unwrap(),
                S7Value::Bool(true),
            ),
        ];

        let server = S7Server::start(0, seed).await.expect("bind");
        let port = server.port();

        let client = S7Client::connect(cfg(port)).await.expect("connect");

        // Read all three back (Real tag overridden to match the seed).
        let mut read_real = S7Tag::parse_address("DB1.DBD0").unwrap();
        read_real.data_type = S7DataType::Real;
        let tags = vec![
            read_real,
            S7Tag::parse_address("DB1.DBW4").unwrap(),
            S7Tag::parse_address("DB1.DBX8.0").unwrap(),
        ];
        let values = client.read(&tags).await.expect("read");
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], S7Value::Real(1234.5));
        assert_eq!(values[1], S7Value::Word(4660));
        assert_eq!(values[2], S7Value::Bool(true));

        // Write a new Real value, then read it back.
        let mut tag = S7Tag::parse_address("DB1.DBD0").unwrap();
        tag.data_type = S7DataType::Real;
        client
            .write(&tag, S7Value::Real(42.0))
            .await
            .expect("write");

        let after = client.read(&[tag]).await.expect("read after write");
        assert_eq!(after[0], S7Value::Real(42.0));

        // Unknown address returns zeros (not an error).
        let unknown = vec![S7Tag::parse_address("DB1.DBD200").unwrap()];
        let z = client.read(&unknown).await.expect("read unknown");
        assert_eq!(z.len(), 1);

        client.disconnect().await.expect("disconnect");
        server.stop().await;
    }

    /// Two simultaneous clients can read the same image (multi-client sim).
    #[tokio::test]
    async fn server_multiple_clients() {
        let mut t = S7Tag::parse_address("DB1.DBD0").unwrap();
        t.data_type = S7DataType::Real;
        let seed = vec![(t.clone(), S7Value::Real(9.0))];
        let server = S7Server::start(0, seed).await.unwrap();
        let port = server.port();

        let a = S7Client::connect(cfg(port)).await.unwrap();
        let b = S7Client::connect(cfg(port)).await.unwrap();
        let tags = vec![t];

        assert_eq!(a.read(&tags).await.unwrap()[0], S7Value::Real(9.0));
        assert_eq!(b.read(&tags).await.unwrap()[0], S7Value::Real(9.0));

        a.disconnect().await.unwrap();
        b.disconnect().await.unwrap();
        server.stop().await;
    }

    /// Diagnostics + block list against the simulator succeed (no 0x8100).
    #[tokio::test]
    async fn server_diagnostics_and_blocks() {
        let server = S7Server::start(0, Vec::new()).await.unwrap();
        let port = server.port();
        let client = S7Client::connect(cfg(port)).await.unwrap();

        let cpu = client.read_cpu_info().await.expect("cpu info");
        assert!(
            cpu.module_type.contains("Simulator") || cpu.firmware_version == "1.0.0",
            "got module={:?} fw={:?}",
            cpu.module_type,
            cpu.firmware_version
        );

        let mode = client.read_plc_mode().await.expect("plc mode");
        assert_eq!(mode.mode, "RUN");

        let diag = client.read_diag_buffer().await.expect("diag buffer");
        assert_eq!(diag.entry_count, 0);

        let blocks = client.list_blocks().await.expect("block list");
        assert!(blocks.is_empty());

        client.disconnect().await.unwrap();
        server.stop().await;
    }

    /// Seeded DBs surface in the simulated block list.
    #[tokio::test]
    async fn server_block_list_reflects_image() {
        let seed = vec![
            (S7Tag::parse_address("DB1.DBD0").unwrap(), S7Value::Real(1.0)),
            (S7Tag::parse_address("DB2.DBD0").unwrap(), S7Value::Real(2.0)),
            (S7Tag::parse_address("MW100").unwrap(), S7Value::Word(5)), // marker, not a block
        ];
        let server = S7Server::start(0, seed).await.unwrap();
        let port = server.port();
        let client = S7Client::connect(cfg(port)).await.unwrap();

        let blocks = client.list_blocks().await.unwrap();
        let nums: Vec<u16> = blocks.iter().map(|b| b.block_number).collect();
        assert!(nums.contains(&1), "DB1 missing: {:?}", blocks);
        assert!(nums.contains(&2), "DB2 missing: {:?}", blocks);
        assert!(blocks.iter().all(|b| b.block_type == "DB"));

        client.disconnect().await.unwrap();
        server.stop().await;
    }
    /// Timers and counters read back through the real client and server as BCD words.
    #[tokio::test]
    async fn server_timers_and_counters() {
        let seed = vec![
            (S7Tag::parse_address("T5").unwrap(), S7Value::Timer(12_300)),
            (S7Tag::parse_address("%C10").unwrap(), S7Value::Counter(247)),
        ];
        let server = S7Server::start(0, seed).await.unwrap();
        let port = server.port();
        let client = S7Client::connect(cfg(port)).await.unwrap();

        let tags = vec![
            S7Tag::parse_address("T5").unwrap(),
            S7Tag::parse_address("C10").unwrap(),
            S7Tag::parse_address("DB1.DBD0").unwrap(),
            S7Tag::parse_address("C11").unwrap(), // not seeded: zeros, and the item after it still lines up
            S7Tag::parse_address("MW4").unwrap(),
        ];
        let values = client.read(&tags).await.expect("read");
        assert_eq!(values[0], S7Value::Timer(12_300));
        assert_eq!(values[1], S7Value::Counter(247));
        assert_eq!(values[2], S7Value::DWord(0));
        assert_eq!(values[3], S7Value::Counter(0));
        assert_eq!(values[4], S7Value::Word(0));

        // The client refuses to write them.
        let err = client
            .write(&tags[0], S7Value::Timer(1000))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("read-only"), "{err}");

        // The simulator can be updated and the new value shows up.
        server.set_value(&tags[1], S7Value::Counter(999)).await;
        assert_eq!(client.read(&tags[1..2]).await.unwrap()[0], S7Value::Counter(999));

        client.disconnect().await.unwrap();
        server.stop().await;
    }
}
