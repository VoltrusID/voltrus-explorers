//! S7 protocol client implementing [`ProtocolClient`](mt_protocol::ProtocolClient).
//!
//! The client speaks ISO-on-TCP (TPKT + COTP + S7) directly over a TCP socket,
//! with no external S7 library dependency.

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::types::*;
use crate::vars::*;
use crate::{blocks, framing, szl};

// ── Inner client (owns the TCP stream, requires &mut self) ──

/// How long to wait for the PLC to answer one request before giving up on the connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// TPDU size (bytes, COTP header included) we ask for in the connection request: code 0x0A = 2^10.
const REQUESTED_TPDU_CODE: u8 = 0x0A;

/// Size of the COTP data (DT) header that precedes every S7 PDU.
const COTP_DT_HEADER_LEN: usize = 3;

/// Smallest PDU that can carry a one-item read (10 + 2 + 12 request, 12 + 2 + 4 + data response).
const MIN_PDU_SIZE: u16 = 32;

/// Low-level S7 PLC client managing a single TCP connection.
///
/// This struct contains the mutable connection state and is wrapped in an async
/// Mutex by [`S7Client`] to satisfy the `ProtocolClient` trait's `&self` methods.
struct Inner {
    stream: TcpStream,
    pdu_ref: AtomicU16,
    /// Negotiated S7 PDU size: the most one request or one response may carry, S7 header included.
    pdu_size: u16,
    /// How long one request may take before the connection is given up on.
    request_timeout: Duration,
    /// Set after a timeout or I/O failure. The stream may hold a half-read or late response, so it
    /// cannot be used again.
    broken: bool,
}

impl Inner {
    /// Perform the full COTP + S7 Setup Communication handshake.
    ///
    /// `stream` must already be connected to the PLC.
    async fn negotiate(
        mut stream: TcpStream,
        rack: u8,
        slot: u8,
        request_timeout: Duration,
    ) -> Result<Self, ProtocolError> {
        // COTP Connection Request, then Connection Confirm.
        let cotp_cr = build_cotp_cr(rack, slot);
        stream
            .write_all(&cotp_cr)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("COTP CR send: {e}")))?;

        let cc = tokio::time::timeout(request_timeout, read_cotp_cc(&mut stream))
            .await
            .map_err(|_| ProtocolError::ConnectionFailed("COTP CC timed out".into()))??;
        let tpdu_size = parse_cotp_cc(&cc)?;

        // S7 Setup Communication.
        let pdu_ref = AtomicU16::new(1u16);
        let setup = build_s7_setup_comm(&pdu_ref);
        send_pdu(&mut stream, &setup)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("S7 Setup Comm send: {e}")))?;

        let resp = tokio::time::timeout(request_timeout, recv_pdu(&mut stream))
            .await
            .map_err(|_| ProtocolError::ConnectionFailed("S7 Setup Comm timed out".into()))?
            .map_err(|e| ProtocolError::ConnectionFailed(format!("S7 Setup Comm recv: {e}")))?;
        let negotiated = parse_setup_comm_response(&resp)?;

        // One S7 PDU plus its COTP DT header has to fit one TPDU, because we never fragment.
        let pdu_size = negotiated.min(tpdu_size.saturating_sub(COTP_DT_HEADER_LEN as u16));
        if pdu_size < MIN_PDU_SIZE {
            return Err(ProtocolError::ConnectionFailed(format!(
                "PLC negotiated a PDU size of {pdu_size} bytes, which is too small to use"
            )));
        }

        tracing::info!(
            "Connected to S7 PLC rack={rack}, slot={slot}, PDU size={pdu_size}"
        );

        Ok(Self {
            stream,
            pdu_ref,
            pdu_size,
            request_timeout,
            broken: false,
        })
    }

    fn next_pdu_ref(&self) -> u16 {
        self.pdu_ref.fetch_add(1, Ordering::Relaxed)
    }

    /// Send one S7 PDU and wait for the PLC's answer to it.
    ///
    /// Answers whose PDU reference does not match (left over from an earlier request) are skipped.
    /// A timeout or I/O error marks the connection broken, since the stream may hold a half-read
    /// or late response.
    async fn exchange(&mut self, s7: &[u8]) -> Result<Vec<u8>, String> {
        if self.broken {
            return Err("the connection to the PLC was lost, reconnect".into());
        }
        let result = tokio::time::timeout(self.request_timeout, async {
            send_pdu(&mut self.stream, s7).await.map_err(|e| e.to_string())?;
            let pdu_ref = s7.get(4..6);
            for _ in 0..4 {
                let response = recv_pdu(&mut self.stream).await?;
                if response.get(4..6) == pdu_ref {
                    return Ok(response);
                }
            }
            Err("the PLC answered a different request".to_string())
        })
        .await;
        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => {
                self.broken = true;
                Err(e)
            }
            Err(_) => {
                self.broken = true;
                Err(format!("no answer from the PLC within {:.1} s", self.request_timeout.as_secs_f32()))
            }
        }
    }

    /// Read the raw data bytes of each tag, splitting what does not fit one PDU into several
    /// requests and several tags into one request where they fit.
    async fn read_raw(&mut self, tags: &[S7Tag]) -> Result<Vec<Vec<u8>>, ProtocolError> {
        // What one item can carry: the PDU minus response header, parameter header and item header.
        let max_data = self.pdu_size as usize - 18;
        let mut pieces: Vec<S7Tag> = Vec::new();
        let mut owner: Vec<usize> = Vec::new();
        for (i, tag) in tags.iter().enumerate() {
            for piece in split_tag(tag, max_data) {
                pieces.push(piece);
                owner.push(i);
            }
        }

        let mut raw: Vec<Vec<u8>> = vec![Vec::new(); tags.len()];
        for range in plan_batches(&pieces, self.pdu_size) {
            let batch = &pieces[range.clone()];
            let request = build_read_var_request(self.next_pdu_ref(), batch);
            let response = self
                .exchange(&request)
                .await
                .map_err(|e| ProtocolError::ReadFailed(format!("Read: {e}")))?;
            for (part, tag_index) in parse_read_var_response(&response, batch)?.into_iter().zip(&owner[range]) {
                raw[*tag_index].extend(part);
            }
        }
        Ok(raw)
    }

    async fn read_tags(&mut self, tags: &[S7Tag]) -> Result<Vec<S7Value>, ProtocolError> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }

        // A string is [max length, actual length, characters]. When the caller does not say how
        // long it is, read the two length bytes first, then the characters that are there.
        let probes: Vec<S7Tag> = tags
            .iter()
            .map(|t| if is_unsized_string(t) { string_header(t) } else { t.clone() })
            .collect();
        let mut raw = self.read_raw(&probes).await?;

        let mut tails: Vec<(usize, S7Tag)> = Vec::new();
        for (i, tag) in tags.iter().enumerate() {
            if !is_unsized_string(tag) {
                continue;
            }
            let length = raw[i][1].min(raw[i][0]);
            raw[i][1] = length; // never claim more characters than the PLC string can hold
            if length > 0 {
                let mut chars = tag.clone();
                chars.data_type = S7DataType::Byte;
                chars.offset += 2;
                chars.length = length as u16;
                tails.push((i, chars));
            }
        }
        if !tails.is_empty() {
            let wanted: Vec<S7Tag> = tails.iter().map(|(_, t)| t.clone()).collect();
            for ((i, _), chars) in tails.iter().zip(self.read_raw(&wanted).await?) {
                raw[*i].extend(chars);
            }
        }

        Ok(tags.iter().zip(&raw).map(|(tag, data)| decode_value(data, tag)).collect())
    }

    async fn write_tag(&mut self, tag: &S7Tag, value: &S7Value) -> Result<(), ProtocolError> {
        if matches!(tag.area, S7Area::Timer | S7Area::Counter) {
            return Err(ProtocolError::WriteFailed(
                "Timers and counters are read-only".into(),
            ));
        }
        if tag.data_type == S7DataType::String {
            return self.write_string(tag, value).await;
        }
        let payload = write_payload(tag, value).map_err(ProtocolError::WriteFailed)?;
        self.write_bytes(tag, &payload).await
    }

    /// Send one Write Var for `payload` at `tag` and check the PLC accepted it.
    async fn write_bytes(&mut self, tag: &S7Tag, payload: &[u8]) -> Result<(), ProtocolError> {
        // job header 10 + params 2 + item 12 + data item header 4
        if 28 + payload.len() > self.pdu_size as usize {
            return Err(ProtocolError::WriteFailed(format!(
                "{} bytes do not fit the PLC's {} byte PDU",
                payload.len(),
                self.pdu_size
            )));
        }
        let request = build_write_var_request(self.next_pdu_ref(), tag, payload);
        let response = self
            .exchange(&request)
            .await
            .map_err(|e| ProtocolError::WriteFailed(format!("Write: {e}")))?;
        parse_write_var_response(&response, tag)
    }

    /// Write a string without touching the PLC string's maximum length byte: read it, check the
    /// text fits, then write the actual length and the characters after it.
    async fn write_string(&mut self, tag: &S7Tag, value: &S7Value) -> Result<(), ProtocolError> {
        let S7Value::String(text) = value else {
            return Err(ProtocolError::WriteFailed(format!(
                "a {value:?} value cannot be written to {} (String)",
                tag.label()
            )));
        };
        if !text.is_ascii() {
            return Err(ProtocolError::WriteFailed(
                "S7 strings hold single-byte characters, use ASCII text".into(),
            ));
        }
        let header = self.read_raw(&[string_header(tag)]).await?;
        let max = header[0][0] as usize;
        if text.len() > max {
            return Err(ProtocolError::WriteFailed(format!(
                "the text is {} characters but {} holds at most {max}",
                text.len(),
                tag.label()
            )));
        }
        let mut payload = vec![text.len() as u8];
        payload.extend_from_slice(text.as_bytes());
        let mut chars = string_header(tag);
        chars.offset += 1;
        chars.length = payload.len() as u16;
        self.write_bytes(&chars, &payload).await
    }

    async fn shutdown(&mut self) -> Result<(), ProtocolError> {
        let _ = self.stream.shutdown().await;
        Ok(())
    }

    // ── SZL / Diagnostics ──

    /// Read one SZL: the first request, then follow-up requests while the PLC says more parts
    /// of the answer follow.
    async fn read_szl(&mut self, id: u16, index: u16) -> Result<szl::Szl, ProtocolError> {
        let fail = |e: String| ProtocolError::ReadFailed(format!("SZL 0x{id:04X} index {index}: {e}"));
        let mut assembler = szl::Assembler::default();
        let mut request = szl::build_first_request(self.next_pdu_ref(), id, index);
        loop {
            let response = self.exchange(&request).await.map_err(fail)?;
            match assembler.push(&response).map_err(fail)? {
                szl::Progress::Done(szl) => return Ok(szl),
                szl::Progress::More { seq } => request = szl::build_next_request(self.next_pdu_ref(), seq),
            }
        }
    }

    pub async fn read_cpu_info(&mut self) -> Result<CpuInfo, ProtocolError> {
        let module = self.read_szl(szl::MODULE_IDENTIFICATION, 0).await?;
        let components = self.read_szl(szl::COMPONENT_IDENTIFICATION, 0).await?;
        szl::parse_cpu_info(&module, &components).map_err(|e| ProtocolError::ReadFailed(format!("CPU info: {e}")))
    }

    pub async fn read_plc_mode(&mut self) -> Result<PlcMode, ProtocolError> {
        let szl = self.read_szl(szl::CPU_MODE, 0).await?;
        szl::parse_cpu_mode(&szl).map_err(|e| ProtocolError::ReadFailed(format!("PLC mode: {e}")))
    }

    pub async fn read_diag_buffer(&mut self) -> Result<DiagBuffer, ProtocolError> {
        let szl = self.read_szl(szl::DIAG_BUFFER, 0).await?;
        szl::parse_diag_buffer(&szl).map_err(|e| ProtocolError::ReadFailed(format!("Diagnostic buffer: {e}")))
    }

    // ── Block management ──

    /// Every block in the PLC: how many there are of each type, then the numbers of each type
    /// that has any.
    pub async fn list_blocks(&mut self) -> Result<Vec<BlockListEntry>, ProtocolError> {
        let fail = |e: String| ProtocolError::ReadFailed(format!("List blocks: {e}"));
        let request = blocks::build_list_all_request(self.next_pdu_ref());
        let response = self.exchange(&request).await.map_err(fail)?;
        let counts = blocks::parse_list_all(&response).map_err(fail)?;

        let mut entries = Vec::new();
        for (block_type, count) in counts {
            if count > 0 {
                entries.extend(self.list_blocks_of_type(block_type).await?);
            }
        }
        Ok(entries)
    }

    async fn list_blocks_of_type(&mut self, block_type: BlockType) -> Result<Vec<BlockListEntry>, ProtocolError> {
        let fail = |e: String| ProtocolError::ReadFailed(format!("List {block_type} blocks: {e}"));
        let mut entries = Vec::new();
        let mut request = blocks::build_list_of_type_request(self.next_pdu_ref(), block_type);
        for _ in 0..framing::MAX_PARTS {
            let response = self.exchange(&request).await.map_err(fail)?;
            let page = blocks::parse_list_of_type(&response).map_err(fail)?;
            entries.extend(page.blocks.iter().map(|b| BlockListEntry {
                block_type: block_type.to_string(),
                block_number: b.number,
                flags: b.flags,
                language: blocks::language_name(b.language),
            }));
            if !page.more {
                return Ok(entries);
            }
            if entries.len() > blocks::MAX_BLOCKS_PER_TYPE {
                break;
            }
            request = blocks::build_list_of_type_next_request(self.next_pdu_ref(), page.seq);
        }
        Err(fail("the PLC does not stop sending parts of the list".into()))
    }

    /// Upload a block. The result is the block as the PLC stores it: a 36-byte header, the
    /// MC7 code or the data of a DB, and a 56-byte footer (snap7 `Cli_FullUpload`).
    pub async fn upload_block(
        &mut self,
        block_type: &BlockType,
        block_number: u16,
    ) -> Result<Vec<u8>, ProtocolError> {
        let fail = |e: String| ProtocolError::ReadFailed(format!("Upload {block_type}{block_number}: {e}"));
        let request = blocks::build_start_upload_request(self.next_pdu_ref(), *block_type, block_number);
        let response = self.exchange(&request).await.map_err(fail)?;
        let upload_id = blocks::parse_start_upload_response(&response).map_err(fail)?;

        let block = self.upload_parts(upload_id).await;
        // End the session even if the transfer failed: a PLC has only a few upload slots.
        let ended = self.end_upload(upload_id).await;
        let block = block.map_err(fail)?;
        ended.map_err(fail)?;
        blocks::check_block(&block).map_err(fail)?;
        Ok(block)
    }

    /// Fetch the parts of an upload until the PLC says there are no more.
    async fn upload_parts(&mut self, upload_id: u32) -> Result<Vec<u8>, String> {
        let mut block = Vec::new();
        for _ in 0..framing::MAX_PARTS {
            let request = blocks::build_upload_request(self.next_pdu_ref(), upload_id);
            let response = self.exchange(&request).await?;
            let part = blocks::parse_upload_response(&response)?;
            block.extend_from_slice(part.payload);
            if !part.more {
                return Ok(block);
            }
            if block.len() > blocks::MAX_BLOCK_BYTES {
                break;
            }
        }
        Err("the PLC does not stop sending parts of the block".into())
    }

    async fn end_upload(&mut self, upload_id: u32) -> Result<(), String> {
        let request = blocks::build_end_upload_request(self.next_pdu_ref(), upload_id);
        let response = self.exchange(&request).await?;
        blocks::parse_end_upload_response(&response)
    }

    pub async fn delete_block(
        &mut self,
        block_type: &BlockType,
        block_number: u16,
    ) -> Result<(), ProtocolError> {
        let fail = |e: String| ProtocolError::WriteFailed(format!("Delete {block_type}{block_number}: {e}"));
        let request = blocks::build_delete_request(self.next_pdu_ref(), *block_type, block_number);
        let response = self.exchange(&request).await.map_err(fail)?;
        blocks::parse_delete_response(&response).map_err(fail)
    }
}

/// A string tag whose length the caller did not give: its characters are found by reading the
/// length bytes first.
fn is_unsized_string(tag: &S7Tag) -> bool {
    tag.data_type == S7DataType::String && tag.length <= 2
}

/// The two length bytes at the start of a string, as a byte-array tag.
fn string_header(tag: &S7Tag) -> S7Tag {
    let mut header = tag.clone();
    header.data_type = S7DataType::Byte;
    header.length = 2;
    header
}

// ── Public wrapper implementing ProtocolClient ──

/// Siemens S7 protocol client implementing [`ProtocolClient`](mt_protocol::ProtocolClient).
///
/// The inner TCP connection is wrapped in an async Mutex so that the `&self`
/// methods required by the trait can be satisfied.
///
/// # Example
///
/// ```no_run
/// use mt_s7::{S7Client, S7Config, S7Tag, S7Area, S7DataType, S7Value};
/// use mt_protocol::ProtocolClient;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = S7Config {
///         id: "plc-1".into(),
///         host: "192.168.1.100".into(),
///         port: 102,
///         rack: 0,
///         slot: 1,
///         poll_interval_ms: 1000,
///     };
///     let client = S7Client::connect(config).await?;
///     let tags = vec![S7Tag {
///         area: S7Area::DB,
///         db_number: 1,
///         offset: 0,
///         bit_offset: 0,
///         data_type: S7DataType::DWord,
///         length: 1,
///         name: Some("voltage".into()),
///     }];
///     let values = client.read(&tags).await?;
///     println!("{:?}", values);
///     client.disconnect().await?;
///     Ok(())
/// }
/// ```
pub struct S7Client {
    inner: tokio::sync::Mutex<Option<Inner>>,
}

impl ProtocolClient for S7Client {
    type Config = S7Config;
    type Tag = S7Tag;
    type Value = S7Value;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        Self::connect_with_timeout(config, REQUEST_TIMEOUT).await
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard
            .as_mut()
            .ok_or(ProtocolError::NotConnected)?;
        inner.read_tags(tags).await
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard
            .as_mut()
            .ok_or(ProtocolError::NotConnected)?;
        inner.write_tag(tag, &value).await
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        let mut guard = self.inner.lock().await;
        if let Some(inner) = guard.take() {
            let mut inner = inner;
            inner.shutdown().await?;
        }
        Ok(())
    }
}

// ── Diagnostics methods on S7Client ──

impl S7Client {
    /// Connect, giving up on the TCP connection, the handshake and every later request after
    /// `timeout`.
    pub(crate) async fn connect_with_timeout(
        config: S7Config,
        timeout: Duration,
    ) -> Result<Self, ProtocolError> {
        let addr = format!("{}:{}", config.host, config.port);
        let stream = tokio::time::timeout(timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| ProtocolError::ConnectionFailed(format!("{addr}: no answer")))?
            .map_err(|e| ProtocolError::ConnectionFailed(format!("{addr}: {e}")))?;

        let inner = Inner::negotiate(stream, config.rack, config.slot, timeout).await?;

        Ok(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
        })
    }

    pub async fn read_cpu_info(&self) -> Result<CpuInfo, ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(ProtocolError::NotConnected)?;
        inner.read_cpu_info().await
    }

    pub async fn read_plc_mode(&self) -> Result<PlcMode, ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(ProtocolError::NotConnected)?;
        inner.read_plc_mode().await
    }

    pub async fn read_diag_buffer(&self) -> Result<DiagBuffer, ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(ProtocolError::NotConnected)?;
        inner.read_diag_buffer().await
    }

    pub async fn list_blocks(&self) -> Result<Vec<BlockListEntry>, ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(ProtocolError::NotConnected)?;
        inner.list_blocks().await
    }

    pub async fn upload_block(&self, block_type: &BlockType, block_number: u16) -> Result<Vec<u8>, ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(ProtocolError::NotConnected)?;
        inner.upload_block(block_type, block_number).await
    }

    pub async fn delete_block(&self, block_type: &BlockType, block_number: u16) -> Result<(), ProtocolError> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(ProtocolError::NotConnected)?;
        inner.delete_block(block_type, block_number).await
    }
}

// ── Tag address parser ──

impl S7Tag {
    /// Parse an S7 tag address string into an [`S7Tag`].
    ///
    /// Supported formats:
    /// - `"DB1.DBD0"`     -> DB1, DWord at offset 0
    /// - `"DB1.DBX0.1"`   -> DB1, Bool at offset 0, bit 1
    /// - `"DB1.DBW0"`     -> DB1, Word at offset 0
    /// - `"DB1.DBB0"`     -> DB1, Byte at offset 0
    /// - `"I0.0"`         -> Input, Bool at offset 0, bit 0
    /// - `"IB0"`          -> Input, Byte at offset 0
    /// - `"IW100"`        -> Input, Word at offset 100
    /// - `"Q0.0"`         -> Output, Bool at offset 0, bit 0
    /// - `"MW100"`        -> Marker, Word at offset 100
    /// - `"T5"`           -> Timer 5 (read as milliseconds)
    /// - `"C10"`          -> Counter 10 (BCD, 0 to 999)
    ///
    /// A leading `%` (TIA Portal notation, e.g. `%MW100`, `%T5`) is accepted and ignored.
    pub fn parse_address(addr: &str) -> Result<Self> {
        let addr = addr.trim().trim_start_matches('%').to_uppercase();

        // DB addresses: DB<n>.DB<type><offset>[.bit]
        if let Some(rest) = addr.strip_prefix("DB") {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 2 {
                let db_number: u16 = parts[0]
                    .parse()
                    .with_context(|| format!("Invalid DB number in '{addr}'"))?;

                let type_and_offset = parts[1];
                if type_and_offset.starts_with("DBX")
                    || type_and_offset.starts_with("DBB")
                    || type_and_offset.starts_with("DBW")
                    || type_and_offset.starts_with("DBD")
                {
                    let (_, rest) = type_and_offset.split_at(3);
                    let data_type = if type_and_offset.starts_with("DBX") {
                        S7DataType::Bool
                    } else if type_and_offset.starts_with("DBB") {
                        S7DataType::Byte
                    } else if type_and_offset.starts_with("DBW") {
                        S7DataType::Word
                    } else {
                        S7DataType::DWord
                    };
                    let offset: u16 = rest
                        .parse()
                        .with_context(|| format!("Invalid offset in '{addr}'"))?;

                    let bit_offset = if parts.len() >= 3 && data_type == S7DataType::Bool {
                        parts[2]
                            .parse()
                            .with_context(|| format!("Invalid bit offset in '{addr}'"))?
                    } else {
                        0
                    };

                    return Ok(S7Tag {
                        area: S7Area::DB,
                        db_number,
                        offset,
                        bit_offset,
                        data_type,
                        length: 1,
                        name: None,
                    });
                }
            }
        }

        // Timer and counter addresses: T<n>, C<n>
        for (prefix, area, data_type) in [
            ('T', S7Area::Timer, S7DataType::Timer),
            ('C', S7Area::Counter, S7DataType::Counter),
        ] {
            if let Some(number) = addr.strip_prefix(prefix) {
                let offset: u16 = number
                    .parse()
                    .with_context(|| format!("Invalid {area} number in '{addr}'"))?;
                return Ok(S7Tag {
                    area,
                    db_number: 0,
                    offset,
                    bit_offset: 0,
                    data_type,
                    length: 1,
                    name: None,
                });
            }
        }

        // I, Q, M addresses
        let (area, rest): (S7Area, &str) = if let Some(r) = addr.strip_prefix('I') {
            (S7Area::Input, r)
        } else if let Some(r) = addr.strip_prefix('Q') {
            (S7Area::Output, r)
        } else if let Some(r) = addr.strip_prefix('M') {
            (S7Area::Marker, r)
        } else {
            anyhow::bail!(
                "Unsupported tag address format: '{addr}'. \
                 Use DB<n>.DBx<m>, Ix, Qx, Mx, T<n> or C<n> format."
            );
        };

        // Bit address (e.g. "I0.0", "Q0.1", "M10.3")
        if rest.contains('.') {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() == 2 {
                let offset: u16 = parts[0]
                    .parse()
                    .with_context(|| format!("Invalid offset in '{addr}'"))?;
                let bit_offset: u8 = parts[1]
                    .parse()
                    .with_context(|| format!("Invalid bit offset in '{addr}'"))?;
                return Ok(S7Tag {
                    area,
                    db_number: 0,
                    offset,
                    bit_offset,
                    data_type: S7DataType::Bool,
                    length: 1,
                    name: None,
                });
            }
        }

        // Byte/Word/DWord addresses (e.g. "IB0", "IW100", "MW100")
        let (data_type, offset_str): (S7DataType, &str) =
            if let Some(s) = rest.strip_prefix('B') {
                (S7DataType::Byte, s)
            } else if let Some(s) = rest.strip_prefix('W') {
                (S7DataType::Word, s)
            } else if let Some(s) = rest.strip_prefix('D') {
                (S7DataType::DWord, s)
            } else {
                (S7DataType::Byte, rest)
            };

        let offset: u16 = offset_str
            .parse()
            .with_context(|| format!("Invalid offset in '{addr}'"))?;

        Ok(S7Tag {
            area,
            db_number: 0,
            offset,
            bit_offset: 0,
            data_type,
            length: 1,
            name: None,
        })
    }
}

impl S7Tag {
    /// Apply a data type chosen by the user, given by name.
    ///
    /// Unknown names are ignored. Timers and counters keep the type their address implies,
    /// and the Timer and Counter types cannot be applied to any other area.
    pub fn set_data_type_by_name(&mut self, name: &str) {
        if matches!(self.area, S7Area::Timer | S7Area::Counter) {
            return;
        }
        if let Some(dt) = S7DataType::from_str_name(name) {
            if !matches!(dt, S7DataType::Timer | S7DataType::Counter) {
                self.data_type = dt;
            }
        }
    }
}

// ── Standalone helpers (one-shot read/write) ──

/// Perform a single read of multiple tags without keeping a persistent client.
pub async fn read_tags(
    config: &S7Config,
    tags: &[S7Tag],
) -> Result<Vec<S7Value>, ProtocolError> {
    let client = S7Client::connect(config.clone()).await?;
    let values = client.read(tags).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single write without keeping a persistent client.
pub async fn write_tag(
    config: &S7Config,
    tag: &S7Tag,
    value: S7Value,
) -> Result<(), ProtocolError> {
    let client = S7Client::connect(config.clone()).await?;
    client.write(tag, value).await?;
    client.disconnect().await?;
    Ok(())
}

// ── Wire-level functions ──

fn build_cotp_cr(rack: u8, slot: u8) -> Vec<u8> {
    let src_tsap: u16 = 0x0100;
    let dst_tsap: u16 = 0x0100 + ((rack as u16) << 5) + slot as u16;

    let mut cotp_params = Vec::new();
    cotp_params.push(0xC1);
    cotp_params.push(0x02);
    cotp_params.extend_from_slice(&src_tsap.to_be_bytes());
    cotp_params.push(0xC2);
    cotp_params.push(0x02);
    cotp_params.extend_from_slice(&dst_tsap.to_be_bytes());
    cotp_params.push(0xC0);
    cotp_params.push(0x01);
    cotp_params.push(REQUESTED_TPDU_CODE);

    let cotp_len = 1 + 2 + 2 + 1 + cotp_params.len();
    let mut cotp = Vec::new();
    cotp.push(cotp_len as u8);
    cotp.push(COTP_CR);
    cotp.extend_from_slice(&0x0000u16.to_be_bytes()); // destination reference: 0 in a CR
    cotp.extend_from_slice(&0x0001u16.to_be_bytes()); // source reference: S7 CPUs want it non-zero
    cotp.push(0x00); // class 0, no options
    cotp.extend_from_slice(&cotp_params);

    let total_len = 4 + cotp.len();
    let mut tpkt = Vec::with_capacity(total_len);
    tpkt.push(TPKT_VERSION);
    tpkt.push(0x00);
    tpkt.extend_from_slice(&(total_len as u16).to_be_bytes());
    tpkt.extend_from_slice(&cotp);

    tpkt
}

fn build_s7_setup_comm(pdu_ref: &AtomicU16) -> Vec<u8> {
    let pdu_ref = pdu_ref.fetch_add(1, Ordering::Relaxed);

    let mut params = Vec::new();
    params.push(S7_SETUP_COMM);
    params.push(0x00);
    params.extend_from_slice(&1u16.to_be_bytes()); // max AmQ calling: one job at a time
    params.extend_from_slice(&1u16.to_be_bytes()); // max AmQ called
    params.extend_from_slice(&DEFAULT_PDU_SIZE.to_be_bytes());

    let mut s7 = Vec::new();
    s7.push(S7_PROTOCOL_ID);
    s7.push(S7_MSG_TYPE_JOB);
    s7.extend_from_slice(&0x0000u16.to_be_bytes());
    s7.extend_from_slice(&pdu_ref.to_be_bytes());
    s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
    s7.extend_from_slice(&0x0000u16.to_be_bytes());
    s7.extend_from_slice(&params);

    s7
}

async fn read_cotp_cc(stream: &mut TcpStream) -> Result<Vec<u8>, ProtocolError> {
    read_tpkt(stream)
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(format!("COTP CC: {e}")))
}

/// Wrap one S7 PDU in a COTP DT header (EOT set) and a TPKT header and send it.
async fn send_pdu(stream: &mut TcpStream, s7_data: &[u8]) -> Result<()> {
    let total_len = 4 + COTP_DT_HEADER_LEN + s7_data.len();
    let mut packet = Vec::with_capacity(total_len);
    packet.push(TPKT_VERSION);
    packet.push(0x00);
    packet.extend_from_slice(&(total_len as u16).to_be_bytes());
    packet.extend_from_slice(&[0x02, COTP_DT, 0x80]); // LI = 2, DT, TPDU nr 0 + EOT
    packet.extend_from_slice(s7_data);

    stream
        .write_all(&packet)
        .await
        .context("Failed to send S7 data")?;
    Ok(())
}

/// Read one TPKT packet and return what follows the 4-byte TPKT header.
async fn read_tpkt(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut tpkt_header = [0u8; 4];
    stream
        .read_exact(&mut tpkt_header)
        .await
        .map_err(|e| format!("TPKT header: {e}"))?;
    if tpkt_header[0] != TPKT_VERSION {
        return Err(format!("invalid TPKT version {}", tpkt_header[0]));
    }
    let packet_len = u16::from_be_bytes([tpkt_header[2], tpkt_header[3]]) as usize;
    if packet_len < 4 {
        return Err(format!("invalid TPKT length {packet_len}"));
    }
    let mut payload = vec![0u8; packet_len - 4];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|e| format!("TPKT payload: {e}"))?;
    Ok(payload)
}

/// Receive one S7 PDU. COTP data TPDUs the PLC split across several TPKT packets (EOT bit clear)
/// are reassembled.
async fn recv_pdu(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut pdu = Vec::new();
    loop {
        let payload = read_tpkt(stream).await?;
        let header_len = payload.first().map(|li| *li as usize + 1).unwrap_or(0);
        if payload.len() < header_len || header_len < COTP_DT_HEADER_LEN || payload[1] != COTP_DT {
            return Err(format!(
                "expected a COTP data packet, got {:02X?}",
                &payload[..payload.len().min(8)]
            ));
        }
        pdu.extend_from_slice(&payload[header_len..]);
        if payload[2] & 0x80 != 0 {
            return Ok(pdu);
        }
        if pdu.len() > 2 * MAX_PDU_SIZE as usize {
            return Err("fragmented PDU is larger than any PDU size S7 allows".into());
        }
    }
}

/// Read the TPDU size out of a COTP Connection Confirm (the payload after the TPKT header).
/// A CC without a TPDU size parameter agrees to what we asked for.
fn parse_cotp_cc(cc: &[u8]) -> Result<u16, ProtocolError> {
    if cc.len() < 7 || cc[1] != COTP_CC {
        return Err(ProtocolError::ConnectionFailed(format!(
            "Expected COTP CC, got: {:02X?}",
            &cc[..cc.len().min(6)]
        )));
    }
    let header_end = (cc[0] as usize + 1).min(cc.len());
    let mut code = REQUESTED_TPDU_CODE;
    let mut params = cc.get(7..header_end).unwrap_or(&[]);
    while params.len() >= 2 {
        let len = params[1] as usize;
        let Some(value) = params.get(2..2 + len) else { break };
        if params[0] == 0xC0 && len == 1 {
            code = value[0];
        }
        params = &params[2 + len..];
    }
    // TPDU size is 2^code bytes, 7 (128) to 13 (8192) per ISO 8073.
    if !(7..=13).contains(&code) {
        return Err(ProtocolError::ConnectionFailed(format!(
            "COTP CC has an invalid TPDU size code 0x{code:02X}"
        )));
    }
    Ok(1u16 << code)
}

/// Read the PDU size out of a Setup Communication response (S7 PDU, ACK_DATA header included).
fn parse_setup_comm_response(data: &[u8]) -> Result<u16, ProtocolError> {
    // A refusal is just the 12 byte ACK_DATA header with an error class and code.
    if data.len() >= 12 && data[0] == S7_PROTOCOL_ID {
        let error_code = u16::from_be_bytes([data[10], data[11]]);
        if error_code != S7_ERR_NONE {
            return Err(ProtocolError::ConnectionFailed(format!(
                "S7 Setup Comm error: {}",
                describe_header_error(error_code)
            )));
        }
    }
    // ACK_DATA header (12) + [F0, reserved, AmQ calling (2), AmQ called (2), PDU size (2)].
    if data.len() < 20 {
        return Err(ProtocolError::ConnectionFailed(format!(
            "Setup Comm response too short: {} bytes",
            data.len()
        )));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(ProtocolError::ConnectionFailed(format!(
            "Invalid S7 protocol ID: 0x{:02X}",
            data[0]
        )));
    }
    if data[12] != S7_SETUP_COMM {
        return Err(ProtocolError::ConnectionFailed(format!(
            "Setup Comm response has function 0x{:02X}",
            data[12]
        )));
    }
    Ok(u16::from_be_bytes([data[18], data[19]]))
}

/// Parse an area string (case-insensitive) to an [`S7Area`].
pub fn parse_area_string(s: &str) -> Option<S7Area> {
    match s.to_lowercase().as_str() {
        "db" | "data" => Some(S7Area::DB),
        "input" | "i" => Some(S7Area::Input),
        "output" | "q" | "o" => Some(S7Area::Output),
        "marker" | "m" => Some(S7Area::Marker),
        "timer" | "t" => Some(S7Area::Timer),
        "counter" | "c" => Some(S7Area::Counter),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timer_and_counter_addresses() {
        for (addr, area, number, dt) in [
            ("T5", S7Area::Timer, 5, S7DataType::Timer),
            ("t12", S7Area::Timer, 12, S7DataType::Timer),
            ("%T0", S7Area::Timer, 0, S7DataType::Timer),
            ("C10", S7Area::Counter, 10, S7DataType::Counter),
            ("%c255", S7Area::Counter, 255, S7DataType::Counter),
        ] {
            let tag = S7Tag::parse_address(addr).unwrap_or_else(|e| panic!("{addr}: {e}"));
            assert_eq!(tag.area, area, "{addr}");
            assert_eq!(tag.offset, number, "{addr}");
            assert_eq!(tag.bit_offset, 0, "{addr}");
            assert_eq!(tag.db_number, 0, "{addr}");
            assert_eq!(tag.data_type, dt, "{addr}");
        }
        for bad in ["T", "C", "T5.0", "Tx", "T70000", "C-1"] {
            assert!(S7Tag::parse_address(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn percent_prefix_is_accepted_for_other_areas() {
        let tag = S7Tag::parse_address("%MW20").unwrap();
        assert_eq!((tag.area, tag.offset, tag.data_type), (S7Area::Marker, 20, S7DataType::Word));
        let bit = S7Tag::parse_address("%I0.3").unwrap();
        assert_eq!((bit.area, bit.offset, bit.bit_offset), (S7Area::Input, 0, 3));
    }

    #[test]
    fn type_override_cannot_change_timers_counters_or_apply_to_other_areas() {
        let mut timer = S7Tag::parse_address("T1").unwrap();
        timer.set_data_type_by_name("Word");
        assert_eq!(timer.data_type, S7DataType::Timer);

        let mut marker = S7Tag::parse_address("MD4").unwrap();
        marker.set_data_type_by_name("Timer");
        assert_eq!(marker.data_type, S7DataType::DWord);
        marker.set_data_type_by_name("Real");
        assert_eq!(marker.data_type, S7DataType::Real);
        marker.set_data_type_by_name("nonsense");
        assert_eq!(marker.data_type, S7DataType::Real);
    }

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    // The packets below are from the Wireshark sample capture `s7comm_varservice_libnodavedemo.pcap`
    // (https://wiki.wireshark.org/SampleCaptures, S7comm), real traffic between libnodave and a PLC.

    /// Packet 7 is a Connection Request for rack 0, slot 2 (TSAP 01 02). Ours differs only in the
    /// TPDU size we ask for (0x0A = 1024 bytes instead of 0x09).
    #[test]
    fn connection_request_matches_capture_packet_7() {
        let captured = hex("03 00 00 16 11 e0 00 00 00 01 00 c1 02 01 00 c2 02 01 02 c0 01 09");
        let mut ours = build_cotp_cr(0, 2);
        assert_eq!(ours.pop(), Some(REQUESTED_TPDU_CODE));
        assert_eq!(ours, captured[..captured.len() - 1]);
        // snap7: remote TSAP = (connection type << 8) + rack * 0x20 + slot, type PG = 1 (`SetConnectionParams`
        // callers in s7_micro_client.cpp:2680).
        assert_eq!(&build_cotp_cr(1, 3)[15..19], [0xC2, 0x02, 0x01, 0x23]);
    }

    /// Packet 8: the Connection Confirm (TPKT stripped). Its TPDU size parameter comes first.
    #[test]
    fn connection_confirm_capture_packet_8() {
        let cc = hex("11 d0 00 01 00 03 00 c0 01 09 c1 02 01 00 c2 02 01 02");
        assert_eq!(parse_cotp_cc(&cc).unwrap(), 512);
        // Without a TPDU parameter the PLC accepted what we asked for.
        assert_eq!(parse_cotp_cc(&hex("06 d0 00 01 00 03 00")).unwrap(), 1024);
        assert!(parse_cotp_cc(&hex("06 e0 00 01 00 03 00")).is_err());
        assert!(parse_cotp_cc(&hex("0a d0 00 01 00 03 00 c0 01 20")).is_err());
    }

    /// Packet 9 is the Setup Communication request (the capture asks for PDU 1920, we ask for 480;
    /// both use one job each way). Packet 10 is the answer: 20 bytes, PDU size 0x00F0 = 240.
    #[test]
    fn setup_communication_matches_capture_packets_9_and_10() {
        let captured = hex("32 01 00 00 ff ff 00 08 00 00 f0 00 00 01 00 01 07 80");
        let ours = build_s7_setup_comm(&AtomicU16::new(0xFFFF));
        assert_eq!(ours[..16], captured[..16]);
        assert_eq!(ours[16..], DEFAULT_PDU_SIZE.to_be_bytes());

        let response = hex("32 03 00 00 ff ff 00 08 00 00 00 00 f0 00 00 01 00 01 00 f0");
        assert_eq!(parse_setup_comm_response(&response).unwrap(), 240);
        assert!(parse_setup_comm_response(&response[..19]).is_err());
        let refused = hex("32 03 00 00 ff ff 00 00 00 00 81 04");
        assert!(parse_setup_comm_response(&refused).unwrap_err().to_string().contains("0x8104"));
    }
}
