//! S7 protocol SERVER (simulator).
//!
//! Listens on TCP and speaks ISO-on-TCP (TPKT + COTP + S7) so that external
//! S7 clients (s7-explorer itself, snap7, TIA Portal, etc.) can connect to the
//! simulator and read/write an in-memory tag image.
//!
//! Supported server-side jobs:
//! - S7 Setup Communication (negotiation of the PDU size)
//! - Read Var (0x04) and Write Var (0x05), with the item layout, transport sizes, bit/byte
//!   lengths, pad bytes and return codes of a real CPU (see `vars.rs`)
//! - SZL reads (CPU identity, mode, diagnostic buffer), block listing, upload and delete, with the
//!   telegrams of a real CPU (see `szl.rs`, `blocks.rs` and `sim.rs`). The CPU is fictional; it
//!   lists the DBs of the image plus a few fixed blocks.
//!
//! The image is plain byte memory per area, so a bit, a word and a double word at the same
//! address see the same data.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::blocks::{self, SimBlock};
use crate::framing::{ERR_BLOCK_NOT_FOUND, ERR_BLOCK_PROTECTED};
use crate::sim::SimSession;
use crate::types::*;
use crate::vars::*;

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
        db: if tag.area == S7Area::DB { tag.db_number } else { 0 },
        offset: tag.offset,
        bit: tag.bit_offset,
        data_type: tag.data_type,
    }
}

/// The simulated PLC memory: plain bytes per area, like a real CPU.
///
/// Reading a word that was written as two bytes, or a bit of a byte written as a whole, works the
/// way it does on a PLC. DBs exist once something was put in them; reading a DB nobody created is
/// "object does not exist" (item return code 0x0A), the answer a real CPU gives. Inputs, outputs,
/// markers, timers and counters always exist and read as zero until set.
#[derive(Default)]
struct Image {
    /// Bytes by (area code, DB number). The DB number is 0 for areas that have none. Timers and
    /// counters take two bytes each, at `number * 2`.
    memory: HashMap<(u8, u16), Vec<u8>>,
    /// Tags that were seeded or set, so the image can be listed as tags again.
    tags: HashSet<TagCoord>,
}

impl Image {
    fn exists(&self, area: u8, db: u16) -> bool {
        area != AREA_DB || self.memory.contains_key(&(area, db))
    }

    /// `len` bytes at `offset`, zero-filled past what was ever written. `None` if the DB is missing.
    fn read(&self, area: u8, db: u16, offset: usize, len: usize) -> Option<Vec<u8>> {
        if !self.exists(area, db) {
            return None;
        }
        let mut out = vec![0u8; len];
        if let Some(mem) = self.memory.get(&(area, db)) {
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = mem.get(offset + i).copied().unwrap_or(0);
            }
        }
        Some(out)
    }

    /// Write `bytes` at `offset`, growing the area. `false` if the DB is missing.
    fn write(&mut self, area: u8, db: u16, offset: usize, bytes: &[u8]) -> bool {
        if !self.exists(area, db) {
            return false;
        }
        let mem = self.memory.entry((area, db)).or_default();
        if mem.len() < offset + bytes.len() {
            mem.resize(offset + bytes.len(), 0);
        }
        mem[offset..offset + bytes.len()].copy_from_slice(bytes);
        true
    }

    fn write_bit(&mut self, area: u8, db: u16, offset: usize, bit: u8, on: bool) -> bool {
        let Some(current) = self.read(area, db, offset, 1) else {
            return false;
        };
        let mask = 1u8 << bit;
        let byte = if on { current[0] | mask } else { current[0] & !mask };
        self.write(area, db, offset, &[byte])
    }

    /// Byte offset of a tag inside its area (timers and counters are numbered, not byte addressed).
    fn byte_offset(tag: &S7Tag) -> usize {
        match tag.area {
            S7Area::Timer | S7Area::Counter => tag.offset as usize * 2,
            _ => tag.offset as usize,
        }
    }

    /// Put a tag's value into memory, creating its DB if needed.
    fn put(&mut self, tag: &S7Tag, value: &S7Value) {
        let area = tag.area.code();
        let db = if tag.area == S7Area::DB { tag.db_number } else { 0 };
        self.memory.entry((area, db)).or_default();
        let offset = Self::byte_offset(tag);
        match value {
            S7Value::Bool(on) => {
                self.write_bit(area, db, offset, tag.bit_offset, *on);
            }
            // The simulator's strings are the TIA default, String[254]: [254, length, characters].
            S7Value::String(text) => {
                let mut bytes = vec![254u8, text.len().min(254) as u8];
                bytes.extend_from_slice(&text.as_bytes()[..text.len().min(254)]);
                self.write(area, db, offset, &bytes);
            }
            other => {
                if let Ok(bytes) = write_payload(tag, other) {
                    self.write(area, db, offset, &bytes);
                }
            }
        }
        self.tags.insert(coord_of(tag));
    }

    /// The value currently in memory for a tag coordinate.
    fn get(&self, coord: TagCoord) -> S7Value {
        let tag = tag_from_coord(coord);
        let len = match tag.data_type {
            S7DataType::String => 256,
            _ => tag.data_len(),
        };
        let data = self
            .read(coord.area, coord.db, Self::byte_offset(&tag), len)
            .unwrap_or_default();
        let data = if tag.data_type == S7DataType::Bool {
            vec![data.first().map_or(0, |b| (b >> coord.bit) & 1)]
        } else {
            data
        };
        decode_value(&data, &tag)
    }

    fn db_numbers(&self) -> Vec<u16> {
        let mut dbs: Vec<u16> = self
            .memory
            .keys()
            .filter(|(area, _)| *area == AREA_DB)
            .map(|(_, db)| *db)
            .collect();
        dbs.sort_unstable();
        dbs
    }
}

type SharedImage = Arc<RwLock<Image>>;

/// A running S7 simulator instance.
pub struct S7Server {
    running: Arc<AtomicBool>,
    store: SharedImage,
    handle: Mutex<Option<JoinHandle<()>>>,
    port: u16,
}

impl S7Server {
    /// Bind on `port` (0 = OS-assigned) and seed the tag image with `initial`.
    pub async fn start(port: u16, initial: Vec<(S7Tag, S7Value)>) -> std::io::Result<Self> {
        Self::start_with_pdu_size(port, initial, DEFAULT_PDU_SIZE).await
    }

    /// Like [`start`](Self::start), but offers at most `pdu_size` bytes as the S7 PDU size, the way
    /// a CPU with a smaller limit (S7-300: 240) does. Requests and responses larger than the
    /// negotiated size are answered with the PLC's "PDU size" error.
    pub async fn start_with_pdu_size(
        port: u16,
        initial: Vec<(S7Tag, S7Value)>,
        pdu_size: u16,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port)).await?;
        let actual_port = listener.local_addr()?.port();

        let mut image = Image::default();
        for (tag, value) in &initial {
            image.put(tag, value);
        }
        let store: SharedImage = Arc::new(RwLock::new(image));
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
                            let _ = handle_client(stream, s, pdu_size).await;
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
        self.store.write().await.put(tag, &value);
    }

    /// Convenience setter for boolean tags.
    pub async fn set_bool(&self, tag: &S7Tag, active: bool) {
        self.store.write().await.put(tag, &S7Value::Bool(active));
    }

    /// Snapshot of every tag coordinate → value (for UI display).
    pub async fn snapshot(&self) -> Vec<(S7Tag, S7Value)> {
        let image = self.store.read().await;
        image
            .tags
            .iter()
            .map(|c| (tag_from_coord(*c), image.get(*c)))
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
    store: SharedImage,
    pdu_limit: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. COTP Connection Request → Connection Confirm.
    let cr = read_tpkt_payload(&mut stream).await?;
    let mut cc = cr.clone();
    if cc.len() >= 2 {
        cc[1] = COTP_CC;
    }
    write_tpkt(&mut stream, &cc).await?;

    // 2. S7 Setup Communication: answer with the smaller of the PDU size asked for and ours.
    let setup = read_tpkt_payload(&mut stream).await?;
    if setup.len() < 4 {
        return Ok(());
    }
    let pdu_ref = setup
        .get(3 + 4..3 + 6)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .unwrap_or(0);
    let requested = setup
        .get(3 + 16..3 + 18)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .unwrap_or(pdu_limit);
    let pdu_size = requested.min(pdu_limit);
    let setup_resp = build_setup_comm_resp(pdu_ref, pdu_size);
    write_tpkt(&mut stream, &cotp_dt_wrap(&setup_resp)).await?;

    // 3. Request loop. What SZL reads, block lists and uploads leave open between requests is
    // kept per connection.
    let mut sim = SimSession::default();
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

        let resp = if s7.len() > pdu_size as usize {
            ack_error(pdu_ref, 0x8500)
        } else {
            match (msg_type, func) {
                (S7_MSG_TYPE_JOB, S7_READ_VAR) => build_read_resp(pdu_ref, params, &store).await,
                (S7_MSG_TYPE_JOB, S7_WRITE_VAR) => build_write_resp(pdu_ref, params, data, &store).await,
                (S7_MSG_TYPE_USER_DATA, _) => {
                    let list = sim_block_list(&store).await;
                    sim.user_data(pdu_ref, params, data, &list, pdu_size)
                        .unwrap_or_else(|| build_unsupported_resp(pdu_ref))
                }
                (S7_MSG_TYPE_JOB, S7_START_UPLOAD) => match blocks::parse_start_upload_request(params) {
                    Some((block_type, number)) => sim.start_upload(pdu_ref, sim_block(&store, block_type, number).await),
                    None => build_unsupported_resp(pdu_ref),
                },
                (S7_MSG_TYPE_JOB, S7_UPLOAD) => sim.build_upload_resp(pdu_ref, params, pdu_size),
                (S7_MSG_TYPE_JOB, S7_END_UPLOAD) => sim.end_upload(pdu_ref, params),
                (S7_MSG_TYPE_JOB, S7_PLC_CONTROL) => match blocks::parse_delete_request(params) {
                    Some((block_type, number)) => {
                        blocks::build_delete_response(pdu_ref, sim_delete_block(&store, block_type, number).await)
                    }
                    None => build_unsupported_resp(pdu_ref),
                },
                _ => build_unsupported_resp(pdu_ref),
            }
        };
        // A real CPU answers a request whose response would not fit the PDU with an error.
        let resp = if resp.len() > pdu_size as usize { ack_error(pdu_ref, 0x8500) } else { resp };
        if write_tpkt(&mut stream, &cotp_dt_wrap(&resp)).await.is_err() {
            break;
        }
    }
    Ok(())
}

/// An ACK_DATA with no parameters or data and the given error class and code.
fn ack_error(pdu_ref: u16, error: u16) -> Vec<u8> {
    let mut s7 = ack_header(pdu_ref, 0, 0);
    let n = s7.len();
    s7[n - 2..].copy_from_slice(&error.to_be_bytes());
    s7
}

fn build_setup_comm_resp(pdu_ref: u16, pdu_size: u16) -> Vec<u8> {
    let mut params = Vec::new();
    params.push(S7_SETUP_COMM);
    params.push(0x00);
    params.extend_from_slice(&1u16.to_be_bytes()); // max AmQ caller
    params.extend_from_slice(&1u16.to_be_bytes()); // max AmQ callee
    params.extend_from_slice(&pdu_size.to_be_bytes());

    let mut s7 = ack_header(pdu_ref, params.len() as u16, 0);
    s7.extend_from_slice(&params);
    s7
}

/// One 12-byte item of a Read/Write Var request, decoded.
struct RequestItem {
    ts: u8,
    elements: usize,
    db: u16,
    area: u8,
    address: u32,
}

fn request_items(params: &[u8]) -> Vec<RequestItem> {
    let count = params.get(1).copied().unwrap_or(0) as usize;
    (0..count)
        .filter_map(|i| params.get(2 + i * 12..2 + (i + 1) * 12))
        .map(|item| RequestItem {
            ts: item[3],
            elements: u16::from_be_bytes([item[4], item[5]]) as usize,
            db: u16::from_be_bytes([item[6], item[7]]),
            area: item[8],
            address: ((item[9] as u32) << 16) | ((item[10] as u32) << 8) | item[11] as u32,
        })
        .collect()
}

impl RequestItem {
    /// Where the item starts in the image: (DB key, byte offset, bit, byte length), or the item
    /// return code if the request cannot be served.
    fn locate(&self) -> Result<(u16, usize, u8, usize), u8> {
        let unit = request_unit_size(self.ts).ok_or(RC_TRANSPORT_SIZE_NOT_SUPPORTED)?;
        if S7Area::from_code(self.area).is_none() {
            return Err(RC_ADDRESS_OUT_OF_RANGE);
        }
        let db = if self.area == AREA_DB { self.db } else { 0 };
        Ok(match self.ts {
            TS_BIT => (db, (self.address / 8) as usize, (self.address % 8) as u8, 1),
            // Timers and counters are addressed by number; the image keeps 2 bytes for each.
            TS_TIMER | TS_COUNTER => (db, self.address as usize * 2, 0, self.elements * unit),
            _ => (db, (self.address / 8) as usize, 0, self.elements * unit),
        })
    }
}

async fn build_read_resp(pdu_ref: u16, params: &[u8], store: &SharedImage) -> Vec<u8> {
    let items = request_items(params);
    let image = store.read().await;

    let mut resp_data = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let result = item.locate().and_then(|(db, offset, bit, len)| {
            image
                .read(item.area, db, offset, len)
                .map(|bytes| if item.ts == TS_BIT { vec![(bytes[0] >> bit) & 1] } else { bytes })
                .ok_or(RC_OBJECT_DOES_NOT_EXIST)
        });
        match result {
            Ok(bytes) => {
                let (ts, length) = response_item_header(item.ts, bytes.len());
                resp_data.push(RC_SUCCESS);
                resp_data.push(ts);
                resp_data.extend_from_slice(&length.to_be_bytes());
                resp_data.extend_from_slice(&bytes);
                if bytes.len() % 2 == 1 && i + 1 < items.len() {
                    resp_data.push(0x00); // pad to an even length unless last
                }
            }
            Err(code) => resp_data.extend_from_slice(&[code, 0x00, 0x00, 0x00]),
        }
    }
    drop(image);

    let out_params = [S7_READ_VAR, items.len() as u8];
    let mut s7 = ack_header(pdu_ref, out_params.len() as u16, resp_data.len() as u16);
    s7.extend_from_slice(&out_params);
    s7.extend_from_slice(&resp_data);
    s7
}

async fn build_write_resp(pdu_ref: u16, params: &[u8], data: &[u8], store: &SharedImage) -> Vec<u8> {
    let items = request_items(params);
    let mut image = store.write().await;

    // Write data items follow each other: [return code, transport size, length, bytes], padded to
    // an even length unless last.
    let mut pos = 0;
    let mut return_codes = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(header) = data.get(pos..pos + 4) else {
            return_codes.push(RC_ADDRESS_OUT_OF_RANGE);
            continue;
        };
        let size = data_item_bytes(header[1], u16::from_be_bytes([header[2], header[3]]));
        let bytes = data.get(pos + 4..pos + 4 + size).unwrap_or(&[]);
        pos += 4 + size + usize::from(size % 2 == 1 && i + 1 < items.len());

        let code = match item.locate() {
            Err(code) => code,
            Ok((_, _, _, len)) if bytes.len() != len => 0x07, // data type inconsistent with the item size
            Ok((db, offset, bit, _)) => {
                let written = if item.ts == TS_BIT {
                    image.write_bit(item.area, db, offset, bit, bytes[0] & 1 != 0)
                } else {
                    image.write(item.area, db, offset, bytes)
                };
                if written { RC_SUCCESS } else { RC_OBJECT_DOES_NOT_EXIST }
            }
        };
        return_codes.push(code);
    }
    drop(image);

    let out_params = [S7_WRITE_VAR, items.len() as u8];
    let mut s7 = ack_header(pdu_ref, out_params.len() as u16, return_codes.len() as u16);
    s7.extend_from_slice(&out_params);
    s7.extend_from_slice(&return_codes);
    s7
}

// ── Blocks of the simulated CPU ──
//
// The CPU lists a DB for every DB in the image and the few fixed blocks of
// `blocks::sim_fixed_blocks`. SZL reads, listing, upload and delete are answered in `sim.rs`
// and `blocks.rs` from plain inputs; these functions are the only place that knows the image.

impl Image {
    /// The bytes of DB `number`, `None` if there is no such DB.
    fn db_data(&self, number: u16) -> Option<Vec<u8>> {
        self.memory.get(&(AREA_DB, number)).cloned()
    }

    /// Remove DB `number` and the tags that were put in it. `false` if there is no such DB.
    fn remove_db(&mut self, number: u16) -> bool {
        let removed = self.memory.remove(&(AREA_DB, number)).is_some();
        self.tags.retain(|c| !(c.area == AREA_DB && c.db == number));
        removed
    }
}

/// The blocks the CPU lists. The DBs carry no data; [`sim_block`] fetches that for an upload.
async fn sim_block_list(store: &SharedImage) -> Vec<SimBlock> {
    let mut list = blocks::sim_fixed_blocks();
    list.extend(store.read().await.db_numbers().into_iter().map(|number| blocks::sim_db(number, Vec::new())));
    list
}

/// One block with its data, `None` if the CPU does not have it.
async fn sim_block(store: &SharedImage, block_type: BlockType, number: u16) -> Option<SimBlock> {
    if block_type == BlockType::DB {
        return store.read().await.db_data(number).map(|data| blocks::sim_db(number, data));
    }
    blocks::sim_fixed_blocks().into_iter().find(|b| b.block_type == block_type && b.number == number)
}

/// Delete a block. A DB goes with its data; the fixed blocks are write-protected.
async fn sim_delete_block(store: &SharedImage, block_type: BlockType, number: u16) -> Result<(), u16> {
    if block_type != BlockType::DB {
        let fixed = blocks::sim_fixed_blocks().iter().any(|b| b.block_type == block_type && b.number == number);
        return Err(if fixed { ERR_BLOCK_PROTECTED } else { ERR_BLOCK_NOT_FOUND });
    }
    if store.write().await.remove_db(number) {
        Ok(())
    } else {
        Err(ERR_BLOCK_NOT_FOUND)
    }
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
            S7Tag::parse_address("C11").unwrap(), // not seeded: zeros, and the item after it still lines up
            S7Tag::parse_address("MW4").unwrap(),
        ];
        let values = client.read(&tags).await.expect("read");
        assert_eq!(values[0], S7Value::Timer(12_300));
        assert_eq!(values[1], S7Value::Counter(247));
        assert_eq!(values[2], S7Value::Counter(0));
        assert_eq!(values[3], S7Value::Word(0));

        // A DB nobody created is "object does not exist", as on a real CPU, and the error says which tag.
        let missing = [tags[0].clone(), S7Tag::parse_address("DB7.DBD0").unwrap()];
        let error = client.read(&missing).await.unwrap_err().to_string();
        assert!(error.contains("DB7.DBD0") && error.contains("0x0A"), "{error}");

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

    // ── Wire-level conformance ──
    //
    // The requests and expected answers below are packets of the Wireshark sample capture
    // `s7comm_varservice_libnodavedemo.pcap` (https://wiki.wireshark.org/SampleCaptures, S7comm):
    // real traffic between libnodave and a PLC that negotiated a 240 byte PDU. The simulator,
    // started with the same PDU limit, has to answer byte for byte the way that PLC did.

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    async fn send_tpkt(stream: &mut TcpStream, payload: &[u8]) {
        let mut packet = vec![0x03, 0x00];
        packet.extend_from_slice(&((payload.len() + 4) as u16).to_be_bytes());
        packet.extend_from_slice(payload);
        stream.write_all(&packet).await.unwrap();
    }

    async fn recv_tpkt(stream: &mut TcpStream) -> Vec<u8> {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.unwrap();
        let mut payload = vec![0u8; u16::from_be_bytes([header[2], header[3]]) as usize - 4];
        stream.read_exact(&mut payload).await.unwrap();
        payload
    }

    /// Send one S7 PDU in a COTP data TPDU and return the S7 PDU of the answer.
    async fn raw_exchange(stream: &mut TcpStream, s7: &[u8]) -> Vec<u8> {
        let mut payload = vec![0x02, 0xf0, 0x80];
        payload.extend_from_slice(s7);
        send_tpkt(stream, &payload).await;
        recv_tpkt(stream).await[3..].to_vec()
    }

    /// Connect with the capture's Connection Request (packet 7) and Setup Communication (packet 9);
    /// returns the stream and the setup answer.
    async fn raw_connect(port: u16) -> (TcpStream, Vec<u8>) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        send_tpkt(&mut stream, &hex("11 e0 00 00 00 01 00 c1 02 01 00 c2 02 01 02 c0 01 09")).await;
        let cc = recv_tpkt(&mut stream).await;
        assert_eq!(cc[1], 0xd0, "connection confirm");
        let setup = raw_exchange(&mut stream, &hex("32 01 00 00 ff ff 00 08 00 00 f0 00 00 01 00 01 07 80")).await;
        (stream, setup)
    }

    #[tokio::test]
    async fn simulator_answers_like_the_plc_in_the_capture() {
        let seed = vec![(S7Tag::parse_address("DB1.DBB63").unwrap(), S7Value::Byte(0))];
        let server = S7Server::start_with_pdu_size(0, seed, 240).await.unwrap();
        let (mut stream, setup) = raw_connect(server.port()).await;

        // Packet 10: the PLC offers 240 bytes although the client asked for 1920.
        assert_eq!(setup, hex("32 03 00 00 ff ff 00 08 00 00 00 00 f0 00 00 01 00 01 00 f0"));

        // Packets 11 and 12: 64 bytes of DB1. Length 0x0200 is in bits.
        let read = raw_exchange(&mut stream, &hex("32 01 00 00 00 00 00 0e 00 00 04 01 12 0a 10 02 00 40 00 01 84 00 00 00")).await;
        let mut expected = hex("32 03 00 00 00 00 00 02 00 44 00 00 04 01 ff 04 02 00");
        expected.extend(std::iter::repeat(0u8).take(64));
        assert_eq!(read, expected);

        // Packets 15 and 16: write four bytes to the markers.
        let write = raw_exchange(&mut stream, &hex("32 01 00 00 00 02 00 0e 00 08 05 01 12 0a 10 02 00 04 00 00 83 00 00 00 00 04 00 20 a9 10 00 01")).await;
        assert_eq!(write, hex("32 03 00 00 00 02 00 02 00 01 00 00 05 01 ff"));

        // Packets 13 and 14: read 16 marker bytes back (the capture's other bytes differ).
        let read = raw_exchange(&mut stream, &hex("32 01 00 00 00 01 00 0e 00 00 04 01 12 0a 10 02 00 10 00 00 83 00 00 00")).await;
        assert_eq!(read[..22], hex("32 03 00 00 00 01 00 02 00 14 00 00 04 01 ff 04 00 80 a9 10 00 01")[..]);
        assert_eq!(read.len(), 12 + 2 + 4 + 16);

        server.stop().await;
    }

    /// A multi-item read has the pad byte after odd-sized items and a bit comes back as 0 or 1.
    #[tokio::test]
    async fn simulator_pads_odd_items_and_answers_bits_like_a_plc() {
        let mut word = S7Tag::parse_address("DB1.DBW2").unwrap();
        word.data_type = S7DataType::Word;
        let seed = vec![
            (S7Tag::parse_address("DB1.DBX0.5").unwrap(), S7Value::Bool(true)),
            (word, S7Value::Word(0x1234)),
        ];
        let server = S7Server::start(0, seed).await.unwrap();
        let (mut stream, _) = raw_connect(server.port()).await;

        // Bit DB1.DBX0.5 (address 5), word DB1.DBW2 (address 16), DB9 byte 0 (does not exist).
        let request = hex(
            "32 01 00 00 00 07 00 26 00 00 04 03
             12 0a 10 01 00 01 00 01 84 00 00 05
             12 0a 10 04 00 01 00 01 84 00 00 10
             12 0a 10 02 00 01 00 09 84 00 00 00",
        );
        let response = raw_exchange(&mut stream, &request).await;
        let items = &response[14..];
        assert_eq!(items, hex("ff 03 00 01 01 00  ff 04 00 10 12 34  0a 00 00 00"), "bit, pad, word, missing DB");
        server.stop().await;
    }

    #[tokio::test]
    async fn simulator_refuses_what_does_not_fit_its_pdu() {
        let server = S7Server::start_with_pdu_size(0, Vec::new(), 240).await.unwrap();
        let (mut stream, _) = raw_connect(server.port()).await;

        // 300 bytes of marker data would make a 318 byte response.
        let read = raw_exchange(&mut stream, &hex("32 01 00 00 00 09 00 0e 00 00 04 01 12 0a 10 02 01 2c 00 00 83 00 00 00")).await;
        assert_eq!(read, hex("32 03 00 00 00 09 00 00 00 00 85 00"));

        // 25 items make a 312 byte request.
        let mut request = hex("32 01 00 00 00 0a 01 2e 00 00 04 19");
        for _ in 0..25 {
            request.extend(hex("12 0a 10 04 00 01 00 00 83 00 00 00"));
        }
        let read = raw_exchange(&mut stream, &request).await;
        assert_eq!(read, hex("32 03 00 00 00 0a 00 00 00 00 85 00"));
        server.stop().await;
    }

    #[tokio::test]
    async fn client_splits_reads_to_fit_a_small_pdu() {
        let mut seed = Vec::new();
        let mut tags = Vec::new();
        for i in 0..45u16 {
            let tag = S7Tag::parse_address(&format!("MW{}", i * 2)).unwrap();
            seed.push((tag.clone(), S7Value::Word(1000 + i)));
            tags.push(tag);
        }
        // A 200 character string, read with an explicit length so it needs two 222 byte pieces.
        let mut text = S7Tag::parse_address("DB1.DBB0").unwrap();
        text.data_type = S7DataType::String;
        let long = "abcdefghij".repeat(20);
        seed.push((text.clone(), S7Value::String(long.clone())));
        let mut explicit = text.clone();
        explicit.length = 256;
        tags.push(explicit);

        let server = S7Server::start_with_pdu_size(0, seed, 240).await.unwrap();
        let client = S7Client::connect(cfg(server.port())).await.unwrap();
        let values = client.read(&tags).await.expect("split read");
        for i in 0..45u16 {
            assert_eq!(values[i as usize], S7Value::Word(1000 + i));
        }
        assert_eq!(values[45], S7Value::String(long));

        client.disconnect().await.unwrap();
        server.stop().await;
    }

    #[tokio::test]
    async fn bits_words_and_dwords_share_the_same_memory() {
        let server = S7Server::start(0, vec![(S7Tag::parse_address("DB1.DBB0").unwrap(), S7Value::Byte(0))])
            .await
            .unwrap();
        let client = S7Client::connect(cfg(server.port())).await.unwrap();

        let bit = |n: u8| S7Tag::parse_address(&format!("DB1.DBX0.{n}")).unwrap();
        client.write(&bit(5), S7Value::Bool(true)).await.unwrap();
        client.write(&bit(0), S7Value::Bool(true)).await.unwrap();
        let values = client
            .read(&[bit(0), bit(4), bit(5), S7Tag::parse_address("DB1.DBB0").unwrap()])
            .await
            .unwrap();
        assert_eq!(
            values,
            vec![S7Value::Bool(true), S7Value::Bool(false), S7Value::Bool(true), S7Value::Byte(0b0010_0001)]
        );
        client.write(&bit(5), S7Value::Bool(false)).await.unwrap();
        assert_eq!(client.read(&[S7Tag::parse_address("DB1.DBB0").unwrap()]).await.unwrap()[0], S7Value::Byte(1));

        // A double word written whole is readable as its two words, big endian like the PLC.
        client
            .write(&S7Tag::parse_address("MD10").unwrap(), S7Value::DWord(0x0102_0304))
            .await
            .unwrap();
        let words = client
            .read(&[S7Tag::parse_address("MW10").unwrap(), S7Tag::parse_address("MW12").unwrap()])
            .await
            .unwrap();
        assert_eq!(words, vec![S7Value::Word(0x0102), S7Value::Word(0x0304)]);

        // Writing to a DB that does not exist is refused with the PLC's return code.
        let error = client
            .write(&S7Tag::parse_address("DB9.DBW0").unwrap(), S7Value::Word(1))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("DB9.DBW0") && error.contains("0x0A"), "{error}");

        // So is a value of the wrong type for the tag.
        let error = client
            .write(&S7Tag::parse_address("MW0").unwrap(), S7Value::Real(1.0))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("MW0"), "{error}");

        client.disconnect().await.unwrap();
        server.stop().await;
    }

    #[tokio::test]
    async fn strings_are_read_by_their_own_length_and_written_within_their_maximum() {
        let mut text = S7Tag::parse_address("DB1.DBB10").unwrap();
        text.data_type = S7DataType::String;
        // Smaller PDU: the 200 character string needs a second read of the characters in pieces.
        let long = "0123456789".repeat(20);
        let server = S7Server::start_with_pdu_size(
            0,
            vec![(text.clone(), S7Value::String("hello".into()))],
            240,
        )
        .await
        .unwrap();
        let client = S7Client::connect(cfg(server.port())).await.unwrap();

        assert_eq!(client.read(&[text.clone()]).await.unwrap()[0], S7Value::String("hello".into()));

        client.write(&text, S7Value::String(long.clone())).await.unwrap();
        assert_eq!(client.read(&[text.clone()]).await.unwrap()[0], S7Value::String(long));

        client.write(&text, S7Value::String(String::new())).await.unwrap();
        assert_eq!(client.read(&[text.clone()]).await.unwrap()[0], S7Value::String(String::new()));

        let error = client
            .write(&text, S7Value::String("x".repeat(255)))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most 254"), "{error}");

        client.disconnect().await.unwrap();
        server.stop().await;
    }

    /// A PLC that stops answering must not hang the app: the request fails, and the connection
    /// then refuses further use instead of reading the late answer as the next one.
    #[tokio::test]
    async fn a_silent_plc_times_out_and_the_connection_is_dropped() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut cr = recv_tpkt(&mut stream).await;
            cr[1] = 0xd0;
            send_tpkt(&mut stream, &cr).await;
            let setup = recv_tpkt(&mut stream).await;
            let pdu_ref = u16::from_be_bytes([setup[7], setup[8]]);
            let mut reply = vec![0x02, 0xf0, 0x80];
            reply.extend(build_setup_comm_resp(pdu_ref, 240));
            send_tpkt(&mut stream, &reply).await;
            let _ = recv_tpkt(&mut stream).await; // the read request, never answered
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });

        let client = S7Client::connect_with_timeout(cfg(port), std::time::Duration::from_millis(300))
            .await
            .unwrap();
        let tag = S7Tag::parse_address("MW0").unwrap();
        let error = client.read(&[tag.clone()]).await.unwrap_err().to_string();
        assert!(error.contains("no answer"), "{error}");
        let error = client.read(&[tag]).await.unwrap_err().to_string();
        assert!(error.contains("reconnect"), "{error}");
    }
}
