//! S7 protocol client implementing [`ProtocolClient`](mt_protocol::ProtocolClient).
//!
//! The client speaks ISO-on-TCP (TPKT + COTP + S7) directly over a TCP socket,
//! with no external S7 library dependency.

use std::sync::atomic::{AtomicU16, Ordering};

use anyhow::{Context as _, Result};
use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::types::*;

// ── Inner client (owns the TCP stream, requires &mut self) ──

/// Low-level S7 PLC client managing a single TCP connection.
///
/// This struct contains the mutable connection state and is wrapped in an async
/// Mutex by [`S7Client`] to satisfy the `ProtocolClient` trait's `&self` methods.
#[allow(dead_code)]
struct Inner {
    stream: TcpStream,
    pdu_ref: AtomicU16,
    pdu_size: u16,
    rack: u8,
    slot: u8,
}

impl Inner {
    /// Perform the full COTP + S7 Setup Communication handshake.
    ///
    /// `stream` must already be connected to the PLC.
    async fn negotiate(
        mut stream: TcpStream,
        rack: u8,
        slot: u8,
    ) -> Result<Self, ProtocolError> {
        // Build & send COTP Connection Request
        let cotp_cr = build_cotp_cr(rack, slot);
        stream
            .write_all(&cotp_cr)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("COTP CR send: {e}")))?;

        // Read COTP Connection Confirm
        let cc = read_cotp_cc(&mut stream).await?;

        // Verify it's a CC
        if cc.len() < 6 || cc[1] != COTP_CC {
            return Err(ProtocolError::ConnectionFailed(format!(
                "Expected COTP CC, got: {:02X?}",
                &cc[..cc.len().min(6)]
            )));
        }

        // Build & send S7 Setup Communication
        let pdu_ref = AtomicU16::new(1u16);
        let setup = build_s7_setup_comm(&pdu_ref);
        send_s7(&mut stream, &setup)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("S7 Setup Comm send: {e}")))?;

        // Read S7 Setup Communication response
        let resp = read_s7_response(&mut stream)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("S7 Setup Comm recv: {e}")))?;
        let pdu_size = parse_setup_comm_response(&resp)?;

        tracing::info!(
            "Connected to S7 PLC rack={rack}, slot={slot}, PDU size={pdu_size}"
        );

        Ok(Self {
            stream,
            pdu_ref,
            pdu_size,
            rack,
            slot,
        })
    }

    fn next_pdu_ref(&self) -> u16 {
        self.pdu_ref.fetch_add(1, Ordering::Relaxed)
    }

    async fn read_tags(&mut self, tags: &[S7Tag]) -> Result<Vec<S7Value>, ProtocolError> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }

        let request = self.build_read_var_request(tags);
        send_s7(&mut self.stream, &request)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Send: {e}")))?;

        let response = read_s7_response(&mut self.stream)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Recv: {e}")))?;

        parse_read_var_response(&response, tags)
    }

    async fn write_tag(&mut self, tag: &S7Tag, value: &S7Value) -> Result<(), ProtocolError> {
        if matches!(tag.area, S7Area::Timer | S7Area::Counter) {
            return Err(ProtocolError::WriteFailed(
                "Timers and counters are read-only".into(),
            ));
        }
        let request = self.build_write_var_request(tag, value);
        send_s7(&mut self.stream, &request)
            .await
            .map_err(|e| ProtocolError::WriteFailed(format!("Send: {e}")))?;

        let response = read_s7_response(&mut self.stream)
            .await
            .map_err(|e| ProtocolError::WriteFailed(format!("Recv: {e}")))?;

        parse_write_var_response(&response)
    }

    async fn shutdown(&mut self) -> Result<(), ProtocolError> {
        let _ = self.stream.shutdown().await;
        Ok(())
    }

    // ── SZL / Diagnostics ──

    async fn read_szl(&mut self, szl_id: u16, szl_index: u16) -> Result<Vec<u8>, ProtocolError> {
        let pdu_ref = self.next_pdu_ref();

        // Parameter: CPU function + Read SZL subfunction + SZL item
        let mut params = Vec::new();
        params.push(S7_CPU_FUNC); // function: CPU functions
        params.push(S7_SZL_READ); // subfunction: Read SZL
        params.push(0x12); // SZL item header
        params.push(0x04); // SZL item: unknown->S7
        params.push(0x01); // SZL item: sequence
        params.extend_from_slice(&szl_id.to_be_bytes());
        params.extend_from_slice(&szl_index.to_be_bytes());
        params.extend_from_slice(&0x0000u16.to_be_bytes()); // padding

        // S7 header: user data message type
        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_USER_DATA);
        s7.extend_from_slice(&0x0012u16.to_be_bytes()); // reserved
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&0x0000u16.to_be_bytes()); // data length
        s7.extend_from_slice(&params);

        send_s7(&mut self.stream, &s7)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("SZL send: {e}")))?;

        let resp = read_s7_response(&mut self.stream)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("SZL recv: {e}")))?;

        parse_szl_response(&resp)
    }

    pub async fn read_cpu_info(&mut self) -> Result<CpuInfo, ProtocolError> {
        let data = self.read_szl(SZL_MODULE_IDENTITY, 0x0000).await?;
        parse_cpu_info(&data)
    }

    pub async fn read_plc_mode(&mut self) -> Result<PlcMode, ProtocolError> {
        let data = self.read_szl(SZL_OPERATING_MODE, 0x0000).await?;
        parse_plc_mode(&data)
    }

    pub async fn read_diag_buffer(&mut self) -> Result<DiagBuffer, ProtocolError> {
        let data = self.read_szl(SZL_DIAG_BUFFER, 0x0000).await?;
        parse_diag_buffer(&data)
    }

    // ── Block management ──

    pub async fn list_blocks(&mut self) -> Result<Vec<BlockListEntry>, ProtocolError> {
        let pdu_ref = self.next_pdu_ref();

        // Request: list all block types
        let mut params = Vec::new();
        params.push(S7_LIST_BLOCKS);
        params.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        params.push(0x01); // list all
        params.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&params);

        send_s7(&mut self.stream, &s7)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("List blocks send: {e}")))?;

        let resp = read_s7_response(&mut self.stream)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("List blocks recv: {e}")))?;

        parse_block_list_response(&resp)
    }

    pub async fn upload_block(
        &mut self,
        block_type: &BlockType,
        block_number: u16,
    ) -> Result<Vec<u8>, ProtocolError> {
        let pdu_ref = self.next_pdu_ref();

        // Block filename: _<prefix><number padded to 5 digits>
        let filename = format!("_{}{}", block_type.file_prefix(), format!("{:05}", block_number));

        let mut params = Vec::new();
        params.push(S7_UPLOAD);
        params.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
        params.extend_from_slice(&(filename.len() as u8).to_be_bytes());
        params.extend_from_slice(filename.as_bytes());

        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&params);

        send_s7(&mut self.stream, &s7)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Upload block send: {e}")))?;

        let resp = read_s7_response(&mut self.stream)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Upload block recv: {e}")))?;

        parse_upload_response(&resp)
    }

    pub async fn delete_block(
        &mut self,
        block_type: &BlockType,
        block_number: u16,
    ) -> Result<(), ProtocolError> {
        let pdu_ref = self.next_pdu_ref();

        let filename = format!("_{}{}", block_type.file_prefix(), format!("{:05}", block_number));

        let mut params = Vec::new();
        params.push(S7_DELETE_BLOCK);
        params.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
        params.extend_from_slice(&(filename.len() as u8).to_be_bytes());
        params.extend_from_slice(filename.as_bytes());

        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&params);

        send_s7(&mut self.stream, &s7)
            .await
            .map_err(|e| ProtocolError::WriteFailed(format!("Delete block send: {e}")))?;

        let resp = read_s7_response(&mut self.stream)
            .await
            .map_err(|e| ProtocolError::WriteFailed(format!("Delete block recv: {e}")))?;

        parse_delete_response(&resp)
    }

    // ── Request builders ──

    fn build_read_var_request(&self, tags: &[S7Tag]) -> Vec<u8> {
        let pdu_ref = self.next_pdu_ref();

        let mut params = Vec::new();
        params.push(S7_READ_VAR);
        params.push(tags.len() as u8);

        let mut total_data_size: u16 = 0;
        let mut items = Vec::new();

        for tag in tags {
            let byte_count = if tag.data_type == S7DataType::Bool {
                1
            } else {
                tag.data_type.byte_size() * tag.length
            };
            total_data_size += byte_count;

            push_item_spec(&mut items, tag);
        }

        params.extend_from_slice(&items);

        // Data area: header + placeholder data
        let mut data = Vec::new();
        data.push(0x00); // return code
        data.push(0xFF); // transport size
        data.extend_from_slice(&total_data_size.to_be_bytes());
        data.extend_from_slice(&vec![0x00u8; total_data_size as usize]);

        // S7 header + params + data
        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
        s7.extend_from_slice(&params);
        s7.extend_from_slice(&data);

        s7
    }

    fn build_write_var_request(&self, tag: &S7Tag, value: &S7Value) -> Vec<u8> {
        let pdu_ref = self.next_pdu_ref();

        let mut params = Vec::new();
        params.push(S7_WRITE_VAR);
        params.push(0x01); // 1 item

        push_item_spec(&mut params, tag);

        let value_bytes = encode_value(value, tag);
        let mut data = Vec::new();
        data.push(0x00);
        data.push(0x00);
        data.extend_from_slice(&(value_bytes.len() as u16).to_be_bytes());
        data.extend_from_slice(&value_bytes);

        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
        s7.extend_from_slice(&params);
        s7.extend_from_slice(&data);

        s7
    }
}

/// Append the 12-byte S7-any item specification for `tag` to a Read/Write Var parameter block.
///
/// Timers and counters are addressed by number (not by bit offset), carry no DB number and
/// use their own transport size.
fn push_item_spec(params: &mut Vec<u8>, tag: &S7Tag) {
    params.push(0x12); // variable specification: address
    params.push(0x0A); // length of following address specification
    params.push(0x10); // syntax ID: S7 any
    params.push(tag.data_type.transport_size());
    let num_elements = if tag.data_type == S7DataType::Bool {
        1u16
    } else {
        tag.length
    };
    params.extend_from_slice(&num_elements.to_be_bytes());
    params.extend_from_slice(&tag.db_number.to_be_bytes());
    params.push(tag.area.code());
    let address = if matches!(tag.area, S7Area::Timer | S7Area::Counter) {
        tag.offset as u32
    } else {
        (tag.offset as u32) * 8 + (tag.bit_offset as u32)
    };
    params.push(((address >> 16) & 0xFF) as u8);
    params.push(((address >> 8) & 0xFF) as u8);
    params.push((address & 0xFF) as u8);
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
        let addr = format!("{}:{}", config.host, config.port);
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("{addr}: {e}")))?;

        let inner = Inner::negotiate(stream, config.rack, config.slot).await?;

        Ok(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
        })
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
    cotp_params.push(0x0A); // TPDU size 1024

    let cotp_len = 1 + 2 + 2 + 1 + cotp_params.len();
    let mut cotp = Vec::new();
    cotp.push(cotp_len as u8);
    cotp.push(COTP_CR);
    cotp.extend_from_slice(&0x0001u16.to_be_bytes());
    cotp.extend_from_slice(&0x0001u16.to_be_bytes());
    cotp.push(0x00);
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
    params.extend_from_slice(&MAX_PDU_SIZE.to_be_bytes());
    params.extend_from_slice(&MAX_PDU_SIZE.to_be_bytes());
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
    let mut tpkt_header = [0u8; 4];
    stream
        .read_exact(&mut tpkt_header)
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(format!("TPKT header read: {e}")))?;

    if tpkt_header[0] != TPKT_VERSION {
        return Err(ProtocolError::ConnectionFailed(format!(
            "Invalid TPKT version: {}",
            tpkt_header[0]
        )));
    }

    let packet_len = u16::from_be_bytes([tpkt_header[2], tpkt_header[3]]) as usize;
    if packet_len < 4 {
        return Err(ProtocolError::ConnectionFailed(format!(
            "Invalid TPKT packet length: {packet_len}"
        )));
    }

    let mut payload = vec![0u8; packet_len - 4];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|e| ProtocolError::ConnectionFailed(format!("COTP payload read: {e}")))?;

    Ok(payload)
}

async fn send_s7(stream: &mut TcpStream, s7_data: &[u8]) -> Result<()> {
    let mut cotp = Vec::with_capacity(3);
    cotp.push(0x02);
    cotp.push(COTP_DT);
    cotp.push(0x80);

    let total_len = 4 + cotp.len() + s7_data.len();
    let mut packet = Vec::with_capacity(total_len);
    packet.push(TPKT_VERSION);
    packet.push(0x00);
    packet.extend_from_slice(&(total_len as u16).to_be_bytes());
    packet.extend_from_slice(&cotp);
    packet.extend_from_slice(s7_data);

    stream
        .write_all(&packet)
        .await
        .context("Failed to send S7 data")?;
    Ok(())
}

async fn read_s7_response(stream: &mut TcpStream) -> Result<Vec<u8>, ProtocolError> {
    let mut tpkt_header = [0u8; 4];
    stream
        .read_exact(&mut tpkt_header)
        .await
        .map_err(|e| ProtocolError::ReadFailed(format!("S7 TPKT header: {e}")))?;

    if tpkt_header[0] != TPKT_VERSION {
        return Err(ProtocolError::ReadFailed(format!(
            "Invalid TPKT version: {}",
            tpkt_header[0]
        )));
    }

    let packet_len = u16::from_be_bytes([tpkt_header[2], tpkt_header[3]]) as usize;
    if packet_len < 4 {
        return Err(ProtocolError::ReadFailed(format!(
            "Invalid TPKT length: {packet_len}"
        )));
    }

    let mut payload = vec![0u8; packet_len - 4];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|e| ProtocolError::ReadFailed(format!("S7 payload: {e}")))?;

    // Skip COTP DT header (3 bytes)
    if payload.len() < 3 {
        return Err(ProtocolError::ReadFailed("S7 COTP payload too short".into()));
    }

    Ok(payload[3..].to_vec())
}

fn parse_setup_comm_response(data: &[u8]) -> Result<u16, ProtocolError> {
    if data.len() < 17 {
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
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::ConnectionFailed(format!(
            "S7 Setup Comm error: 0x{error_code:04X}"
        )));
    }

    let pdu_size = if data.len() >= 27 {
        let negotiated = u16::from_be_bytes([data[25], data[26]]);
        if negotiated > 0 {
            negotiated.min(MAX_PDU_SIZE)
        } else {
            DEFAULT_PDU_SIZE
        }
    } else {
        DEFAULT_PDU_SIZE
    };

    Ok(pdu_size)
}

fn parse_read_var_response(data: &[u8], tags: &[S7Tag]) -> Result<Vec<S7Value>, ProtocolError> {
    if data.len() < 17 {
        return Err(ProtocolError::ReadFailed("S7 Read Var response too short".into()));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(ProtocolError::ReadFailed(format!(
            "Invalid S7 protocol ID: 0x{:02X}",
            data[0]
        )));
    }
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::ReadFailed(format!(
            "S7 Read Var error: 0x{error_code:04X}"
        )));
    }

    let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let data_start = 12 + param_len;

    if data.len() < data_start + 1 {
        return Err(ProtocolError::ReadFailed(
            "S7 Read Var response data area too short".into(),
        ));
    }

    let mut values = Vec::new();
    let mut offset = data_start;

    for tag in tags {
        if offset >= data.len() {
            values.push(S7Value::Byte(0));
            continue;
        }

        let return_code = data[offset];
        offset += 1;

        if return_code != 0xFF {
            values.push(S7Value::Byte(0));
            if offset + 3 <= data.len() {
                offset += 1; // transport size
                let item_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                offset += 2;
                offset += item_len;
            }
            continue;
        }

        if offset + 2 > data.len() {
            values.push(S7Value::Byte(0));
            break;
        }
        offset += 1; // transport size

        let item_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        if offset + item_len > data.len() {
            values.push(S7Value::Byte(0));
            break;
        }

        let item_data = &data[offset..offset + item_len];
        values.push(decode_value(item_data, tag));
        offset += item_len;
    }

    Ok(values)
}

fn parse_write_var_response(data: &[u8]) -> Result<(), ProtocolError> {
    // ACK_DATA header is 12 bytes (incl. error class + error code); we only
    // inspect those two bytes, so anything shorter than the header is invalid.
    if data.len() < 12 {
        return Err(ProtocolError::WriteFailed(
            "S7 Write Var response too short".into(),
        ));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(ProtocolError::WriteFailed(format!(
            "Invalid S7 protocol ID: 0x{:02X}",
            data[0]
        )));
    }
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::WriteFailed(format!(
            "S7 Write Var error: 0x{error_code:04X}"
        )));
    }
    Ok(())
}

fn decode_value(data: &[u8], tag: &S7Tag) -> S7Value {
    match tag.data_type {
        S7DataType::Bool => {
            if data.is_empty() {
                S7Value::Bool(false)
            } else {
                S7Value::Bool((data[0] & (1 << tag.bit_offset)) != 0)
            }
        }
        S7DataType::Byte => S7Value::Byte(data.first().copied().unwrap_or(0)),
        S7DataType::Word => {
            if data.len() >= 2 {
                S7Value::Word(u16::from_be_bytes([data[0], data[1]]))
            } else {
                S7Value::Word(0)
            }
        }
        S7DataType::DWord => {
            if data.len() >= 4 {
                S7Value::DWord(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
            } else {
                S7Value::DWord(0)
            }
        }
        S7DataType::Real => {
            if data.len() >= 4 {
                let bits = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                S7Value::Real(f32::from_bits(bits))
            } else {
                S7Value::Real(0.0)
            }
        }
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
        S7DataType::String => {
            if data.len() >= 2 {
                let actual_len = data[1] as usize;
                if data.len() >= 2 + actual_len {
                    let s = String::from_utf8_lossy(&data[2..2 + actual_len]);
                    S7Value::String(s.to_string())
                } else {
                    S7Value::String(String::new())
                }
            } else {
                S7Value::String(String::new())
            }
        }
    }
}

fn encode_value(value: &S7Value, tag: &S7Tag) -> Vec<u8> {
    match value {
        S7Value::Bool(v) => {
            let byte = if *v { 1u8 << tag.bit_offset } else { 0u8 };
            vec![byte]
        }
        S7Value::Byte(v) => vec![*v],
        S7Value::Word(v) => v.to_be_bytes().to_vec(),
        S7Value::DWord(v) => v.to_be_bytes().to_vec(),
        S7Value::Real(v) => v.to_be_bytes().to_vec(),
        S7Value::Timer(ms) => encode_timer_ms(*ms).to_be_bytes().to_vec(),
        S7Value::Counter(v) => encode_counter(*v).to_be_bytes().to_vec(),
        S7Value::String(s) => {
            let mut bytes = Vec::new();
            bytes.push(tag.length as u8); // max length
            bytes.push(s.len() as u8); // actual length
            bytes.extend_from_slice(s.as_bytes());
            bytes
        }
    }
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

// ── SZL response parsing ──

fn parse_szl_response(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if data.len() < 12 {
        return Err(ProtocolError::ReadFailed(
            "SZL response too short".into(),
        ));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(ProtocolError::ReadFailed(format!(
            "Invalid S7 protocol ID: 0x{:02X}",
            data[0]
        )));
    }
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::ReadFailed(format!(
            "SZL error: 0x{error_code:04X}"
        )));
    }

    let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let data_len = u16::from_be_bytes([data[8], data[9]]) as usize;
    let data_start = 12 + param_len;

    if data.len() < data_start + data_len {
        return Err(ProtocolError::ReadFailed(
            "SZL response data area too short".into(),
        ));
    }

    Ok(data[data_start..data_start + data_len].to_vec())
}

fn parse_cpu_info(data: &[u8]) -> Result<CpuInfo, ProtocolError> {
    // SZL 0x0011 response: data area has header then items as TLV
    // Layout: [return_code(1)] [transport_size(1)] [len(2)] [item_count(1)] [items...]
    // Each item: [id(1)] [len(1)] [value(len)]
    if data.len() < 4 {
        return Err(ProtocolError::ReadFailed(
            "CPU info data too short".into(),
        ));
    }

    let mut module_type = String::new();
    let mut serial_number = String::new();
    let mut as_name = String::new();
    let mut module_name = String::new();
    let mut firmware_version = String::new();

    // Skip header bytes and parse TLV items
    let mut offset = if data[0] == 0xFF { 4 } else { 0 };
    while offset + 2 <= data.len() {
        let _id = data[offset];
        let len = data[offset + 1] as usize;
        if offset + 2 + len > data.len() {
            break;
        }
        let val = &data[offset + 2..offset + 2 + len];
        let s = String::from_utf8_lossy(val)
            .trim_end_matches('\0')
            .trim()
            .to_string();

        // SZL 0x0011 items by index position
        match _id {
            0x01 => module_type = s,
            0x05 => serial_number = s,
            0x06 => as_name = s,
            0x07 => module_name = s,
            0x08 => firmware_version = s,
            _ => {}
        }
        offset += 2 + len;
    }

    Ok(CpuInfo {
        module_type,
        serial_number,
        as_name,
        module_name,
        firmware_version,
    })
}

fn parse_plc_mode(data: &[u8]) -> Result<PlcMode, ProtocolError> {
    // SZL 0x0074 response data: [return_code] [transport_size] [len] [mode_byte]
    // mode_byte: 0x08=RUN, 0x04=STOP, 0x00=STARTUP
    let mode_byte = data.iter().find(|&&b| b == 0x08 || b == 0x04 || b == 0x00).copied();
    let mode = match mode_byte {
        Some(0x08) => "RUN",
        Some(0x04) => "STOP",
        Some(0x00) => "STARTUP",
        _ => "UNKNOWN",
    };
    Ok(PlcMode { mode: mode.to_string() })
}

fn parse_diag_buffer(data: &[u8]) -> Result<DiagBuffer, ProtocolError> {
    // SZL 0x0000 response: header with entry count, then entries
    // Layout: [return_code(1)] [transport_size(1)] [data_len(2)] [entry_count(2)] [entries...]
    if data.len() < 6 {
        return Ok(DiagBuffer {
            entry_count: 0,
            entries: Vec::new(),
        });
    }

    let entry_count = u16::from_be_bytes([data[4], data[5]]);

    // Each diag entry: event_number(2) + timestamp(8 BCD) + event_data(variable)
    let mut entries = Vec::new();
    let mut offset = 6;

    for _ in 0..entry_count.min(50) {
        if offset + 10 > data.len() {
            break;
        }
        let event_num = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let ts_bytes = &data[offset + 2..offset + 10];
        let timestamp = format!(
            "20{:02X}-{:02X}-{:02X} {:02X}:{:02X}:{:02X}.{:02X}{:02X}",
            ts_bytes[0], ts_bytes[1], ts_bytes[2],
            ts_bytes[3], ts_bytes[4], ts_bytes[5],
            ts_bytes[6], ts_bytes[7],
        );

        let event_text = format!("Event 0x{event_num:04X}");

        entries.push(DiagEntry {
            event_number: event_num,
            timestamp,
            event_text,
        });

        // Entries are typically 12-16 bytes; advance by a safe minimum
        offset += 12;
    }

    Ok(DiagBuffer {
        entry_count,
        entries,
    })
}

// ── Block response parsing ──

fn parse_block_list_response(data: &[u8]) -> Result<Vec<BlockListEntry>, ProtocolError> {
    if data.len() < 12 {
        return Err(ProtocolError::ReadFailed(
            "Block list response too short".into(),
        ));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(ProtocolError::ReadFailed(format!(
            "Invalid S7 protocol ID: 0x{:02X}",
            data[0]
        )));
    }
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::ReadFailed(format!(
            "Block list error: 0x{error_code:04X}"
        )));
    }

    let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let data_len = u16::from_be_bytes([data[8], data[9]]) as usize;
    let data_start = 12 + param_len;

    if data.len() < data_start + data_len || data_len < 4 {
        return Ok(Vec::new());
    }

    let block_data = &data[data_start..data_start + data_len];
    if block_data.len() < 4 {
        return Ok(Vec::new());
    }

    // Parse the block list from SZL-like data
    // Format: [return_code(1)] [transport_size(1)] [len(2)] [items...]
    // Each item has a block type byte followed by count + block numbers
    let mut entries = Vec::new();
    let mut offset = if block_data[0] == 0xFF { 4 } else { 0 };

    while offset + 2 <= block_data.len() {
        let block_type_code = block_data[offset];
        let count = block_data[offset + 1] as usize;
        offset += 2;

        let type_name = match block_type_code {
            0x38 => "OB/FB/FC",
            0x41 => "DB",
            0x42 => "SDB",
            0x44 => "SFC",
            0x45 => "SFB",
            _ => "Unknown",
        };

        for _ in 0..count {
            if offset + 2 > block_data.len() {
                break;
            }
            let block_num = u16::from_be_bytes([block_data[offset], block_data[offset + 1]]);
            entries.push(BlockListEntry {
                block_type: type_name.to_string(),
                block_number: block_num,
            });
            offset += 2;
        }
    }

    Ok(entries)
}

fn parse_upload_response(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if data.len() < 12 {
        return Err(ProtocolError::ReadFailed(
            "Upload response too short".into(),
        ));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(ProtocolError::ReadFailed(format!(
            "Invalid S7 protocol ID: 0x{:02X}",
            data[0]
        )));
    }
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::ReadFailed(format!(
            "Upload error: 0x{error_code:04X}"
        )));
    }

    let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let data_len = u16::from_be_bytes([data[8], data[9]]) as usize;
    let data_start = 12 + param_len;

    if data.len() < data_start + data_len {
        return Err(ProtocolError::ReadFailed(
            "Upload response data area too short".into(),
        ));
    }

    // The block data is in the data area, after the header bytes
    let block_data = &data[data_start..data_start + data_len];
    if block_data.len() < 4 {
        return Ok(Vec::new());
    }

    // Skip return code + transport size + length header
    let skip = if block_data[0] == 0xFF { 4 } else { 0 };
    Ok(block_data[skip..].to_vec())
}

fn parse_delete_response(data: &[u8]) -> Result<(), ProtocolError> {
    if data.len() < 12 {
        return Err(ProtocolError::WriteFailed(
            "Delete response too short".into(),
        ));
    }
    let error_code = u16::from_be_bytes([data[10], data[11]]);
    if error_code != S7_ERR_NONE {
        return Err(ProtocolError::WriteFailed(format!(
            "Delete block error: 0x{error_code:04X}"
        )));
    }
    Ok(())
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

    /// Item layout from the S7 any-pointer: timers and counters are addressed by number with
    /// their own transport size, other areas by bit offset.
    #[test]
    fn item_spec_encoding() {
        let mut buf = Vec::new();
        push_item_spec(&mut buf, &S7Tag::parse_address("T5").unwrap());
        assert_eq!(buf, [0x12, 0x0A, 0x10, 0x1D, 0x00, 0x01, 0x00, 0x00, 0x1D, 0x00, 0x00, 0x05]);

        buf.clear();
        push_item_spec(&mut buf, &S7Tag::parse_address("C300").unwrap());
        assert_eq!(buf, [0x12, 0x0A, 0x10, 0x1C, 0x00, 0x01, 0x00, 0x00, 0x1C, 0x00, 0x01, 0x2C]);

        buf.clear();
        push_item_spec(&mut buf, &S7Tag::parse_address("DB1.DBW10").unwrap());
        assert_eq!(buf, [0x12, 0x0A, 0x10, 0x04, 0x00, 0x01, 0x00, 0x01, 0x84, 0x00, 0x00, 0x50]);
    }

    #[test]
    fn decodes_timer_and_counter_words() {
        let timer = S7Tag::parse_address("T1").unwrap();
        // time base 1 s (0b10), BCD 0x123 = 123
        assert_eq!(decode_value(&[0x21, 0x23], &timer), S7Value::Timer(123_000));
        // time base 10 ms (0b00), BCD 0x999
        assert_eq!(decode_value(&[0x09, 0x99], &timer), S7Value::Timer(9_990));
        // time base 10 s (0b11), BCD 0x001
        assert_eq!(decode_value(&[0x30, 0x01], &timer), S7Value::Timer(10_000));
        assert_eq!(decode_value(&[], &timer), S7Value::Timer(0));

        let counter = S7Tag::parse_address("C1").unwrap();
        assert_eq!(decode_value(&[0x02, 0x47], &counter), S7Value::Counter(247));
        assert_eq!(decode_value(&[0x09, 0x99], &counter), S7Value::Counter(999));
    }

    #[test]
    fn read_response_for_timers_and_counters() {
        let tags = vec![
            S7Tag::parse_address("T1").unwrap(),
            S7Tag::parse_address("C1").unwrap(),
        ];
        // ACK_DATA header (12 bytes) + param (read var, 2 items), then two octet-string items.
        let mut pdu = vec![S7_PROTOCOL_ID, 0x03, 0, 0, 0, 1, 0, 2, 0, 12, 0, 0, S7_READ_VAR, 2];
        pdu.extend_from_slice(&[0xFF, 0x09, 0x00, 0x02, 0x10, 0x50]); // time base 100 ms, BCD 050
        pdu.extend_from_slice(&[0xFF, 0x09, 0x00, 0x02, 0x00, 0x42]);
        let values = parse_read_var_response(&pdu, &tags).unwrap();
        assert_eq!(values, vec![S7Value::Timer(5_000), S7Value::Counter(42)]);
    }
}
