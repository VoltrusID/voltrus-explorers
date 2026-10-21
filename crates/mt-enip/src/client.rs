use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use mt_protocol::{ProtocolClient, ProtocolError};

use crate::packets::*;
use crate::types::*;

// ── Raw ENIP TCP client (internal) ──

/// Low-level EtherNet/IP TCP client for CIP tag operations.
///
/// This struct is NOT `Sync` on its own because `TcpStream` is not `Sync`.
/// The public [`EnipClient`] wraps it in an async Mutex to satisfy the
/// `ProtocolClient: Sync` requirement.
struct EthernetIpClient {
    stream: Option<TcpStream>,
    url: String,
    session_handle: u32,
    sequence_counter: u16,
    /// Optional sink for raw wire frames (packet logger).
    packet_sink: Option<tokio::sync::mpsc::UnboundedSender<PacketRecord>>,
}

impl EthernetIpClient {
    fn new(url: &str) -> Self {
        Self {
            stream: None,
            url: url.to_string(),
            session_handle: 0,
            sequence_counter: 0,
            packet_sink: None,
        }
    }
    /// Install a sink that receives a copy of every raw wire frame (TX/RX).
    pub(crate) fn set_packet_sink(&mut self, tx: tokio::sync::mpsc::UnboundedSender<PacketRecord>) {
        self.packet_sink = Some(tx);
    }

    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Forward a captured frame to the packet sink, if installed. Never blocks;
    /// a full/closed channel is silently ignored.
    fn emit_packet(&self, direction: PacketDir, command: u16, session: u32, bytes: &[u8]) {
        if let Some(tx) = &self.packet_sink {
            let _ = tx.send(PacketRecord {
                direction,
                command,
                session,
                timestamp_ms: self.now_ms(),
                bytes: bytes.to_vec(),
            });
        }
    }

    /// Connect to the PLC and register an EtherNet/IP session.
    async fn connect(&mut self) -> Result<()> {
        let stream = TcpStream::connect(&self.url)
            .await
            .with_context(|| format!("Failed to connect to {} for EtherNet/IP", self.url))?;
        stream.set_nodelay(true)?;
        self.stream = Some(stream);

        self.register_session().await?;
        Ok(())
    }

    /// Read a single CIP tag by name.
    async fn read_tag(&mut self, tag_name: &str, element_count: u16) -> Result<CipValue> {
        let request = build_read_tag_request(tag_name, element_count);
        let response = self.send_rr_data(request).await?;
        parse_read_tag_response(&response)
    }

    /// Write a value to a CIP tag.
    async fn write_tag(&mut self, tag_name: &str, value: &CipValue, element_count: u16) -> Result<()> {
        let request = build_write_tag_request(tag_name, value, element_count);
        let response = self.send_rr_data(request).await?;
        parse_write_tag_response(&response)
    }

    /// Browse available tags using GetAttributeAll on the Symbol class (0x6B).
    async fn browse_tags(&mut self) -> Result<Vec<CipTag>> {
        let mut tags = Vec::new();
        let mut instance_id: u32 = 1;

        loop {
            let request = build_get_attribute_all_request(SYMBOL_CLASS_ID, instance_id);
            let response = match self.send_rr_data(request).await {
                Ok(r) => r,
                Err(_) => break,
            };

            if response.len() < 4 {
                break;
            }

            let status = response[2];
            if status != CIP_STATUS_SUCCESS {
                break;
            }

            match parse_symbol_instance(&response[4..]) {
                Some(tag) => tags.push(tag),
                None => break,
            }

            instance_id += 1;

            if instance_id > 10000 {
                break;
            }
        }

        Ok(tags)
    }

    /// Disconnect and unregister the session.
    async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            let packet = build_enip_header(CMD_UNREGISTER_SESSION, self.session_handle, 0, &[]);
            self.emit_packet(PacketDir::Tx, CMD_UNREGISTER_SESSION, self.session_handle, &packet);
            let _ = stream.write_all(&packet).await;
            let _ = stream.shutdown().await;
        }
        self.session_handle = 0;
        Ok(())
    }

    // ── Internal methods ──

    async fn register_session(&mut self) -> Result<()> {
        let mut body = [0u8; 4];
        body[0..2].copy_from_slice(&1u16.to_le_bytes()); // protocol version
        body[2..4].copy_from_slice(&0u16.to_le_bytes()); // options

        let packet = build_enip_header(CMD_REGISTER_SESSION, 0, 0, &body);
        self.emit_packet(PacketDir::Tx, CMD_REGISTER_SESSION, 0, &packet);

        let stream = self.stream.as_mut().context("Not connected")?;
        stream
            .write_all(&packet)
            .await
            .context("Write RegisterSession failed")?;
        stream.flush().await?;

        let mut header_buf = [0u8; ENIP_HEADER_SIZE];
        stream
            .read_exact(&mut header_buf)
            .await
            .context("Read RegisterSession header failed")?;

        let status = u32::from_le_bytes(header_buf[8..12].try_into().unwrap());
        if status != 0 {
            anyhow::bail!("RegisterSession failed with status 0x{:08X}", status);
        }

        self.session_handle = u32::from_le_bytes(header_buf[4..8].try_into().unwrap());

        let length = u16::from_le_bytes(header_buf[2..4].try_into().unwrap()) as usize;
        let mut body_buf = vec![0u8; length];
        if length > 0 {
            stream
                .read_exact(&mut body_buf)
                .await
                .context("Read RegisterSession body failed")?;
        }
        let mut rx = header_buf.to_vec();
        rx.extend_from_slice(&body_buf);
        self.emit_packet(PacketDir::Rx, CMD_REGISTER_SESSION, self.session_handle, &rx);

        tracing::debug!(
            "EtherNet/IP session registered: handle=0x{:08X}",
            self.session_handle
        );
        Ok(())
    }

    async fn send_rr_data(&mut self, cip_payload: Vec<u8>) -> Result<Vec<u8>> {
        let mut rr_data = Vec::with_capacity(cip_payload.len() + 6);
        rr_data.extend_from_slice(&0u32.to_le_bytes()); // interface handle (CIP = 0)
        rr_data.extend_from_slice(&0u16.to_le_bytes()); // timeout = 0
        let item_count: u16 = 2;
        rr_data.extend_from_slice(&item_count.to_le_bytes());

        // Null address item (unconnected)
        rr_data.extend_from_slice(&0x0000u16.to_le_bytes());
        rr_data.extend_from_slice(&0u16.to_le_bytes());

        // Unconnected data item
        rr_data.extend_from_slice(&0x00B2u16.to_le_bytes());
        rr_data.extend_from_slice(&(cip_payload.len() as u16).to_le_bytes());
        rr_data.extend_from_slice(&cip_payload);

        let packet = build_enip_header(CMD_SEND_RR_DATA, self.session_handle, 0, &rr_data);
        self.emit_packet(PacketDir::Tx, CMD_SEND_RR_DATA, self.session_handle, &packet);

        let stream = self.stream.as_mut().context("Not connected")?;
        stream
            .write_all(&packet)
            .await
            .context("Write SendRRData failed")?;
        stream.flush().await?;

        // Read response header
        let mut header_buf = [0u8; ENIP_HEADER_SIZE];
        stream
            .read_exact(&mut header_buf)
            .await
            .context("Read SendRRData header failed")?;

        let enip_status = u32::from_le_bytes(header_buf[8..12].try_into().unwrap());
        if enip_status != 0 {
            anyhow::bail!("SendRRData ENIP status 0x{:08X}", enip_status);
        }

        let length = u16::from_le_bytes(header_buf[2..4].try_into().unwrap()) as usize;
        let mut body_buf = vec![0u8; length];
        if length > 0 {
            stream
                .read_exact(&mut body_buf)
                .await
                .context("Read SendRRData body failed")?;
        }

        // Parse SendRRData response to extract CIP reply
        // Capture the full response frame (header + body) for the packet logger.
        let mut rx = header_buf.to_vec();
        rx.extend_from_slice(&body_buf);
        self.emit_packet(PacketDir::Rx, CMD_SEND_RR_DATA, self.session_handle, &rx);

        if body_buf.len() < 12 {
            anyhow::bail!("SendRRData response too short");
        }

        let offset = 6;
        let _item_count = u16::from_le_bytes(body_buf[offset..offset + 2].try_into().unwrap());

        let mut pos = offset + 2;
        // Skip address item
        if pos + 4 <= body_buf.len() {
            let _addr_type = u16::from_le_bytes(body_buf[pos..pos + 2].try_into().unwrap());
            let addr_len =
                u16::from_le_bytes(body_buf[pos + 2..pos + 4].try_into().unwrap()) as usize;
            pos += 4 + addr_len;
        }

        // Data item
        if pos + 4 > body_buf.len() {
            anyhow::bail!("No data item in SendRRData response");
        }

        let _data_type = u16::from_le_bytes(body_buf[pos..pos + 2].try_into().unwrap());
        let data_len = u16::from_le_bytes(body_buf[pos + 2..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        if pos + data_len > body_buf.len() {
            anyhow::bail!("Data item truncated in SendRRData response");
        }

        Ok(body_buf[pos..pos + data_len].to_vec())
    }

    #[allow(dead_code)]
    fn next_sequence(&mut self) -> u16 {
        self.sequence_counter = self.sequence_counter.wrapping_add(1);
        self.sequence_counter
    }
}

// ── Public ProtocolClient implementation ──

/// Standalone EtherNet/IP client implementing the [`ProtocolClient`] trait.
///
/// Wraps the internal [`EthernetIpClient`] in an async Mutex so that
/// `EnipClient` satisfies `Sync` (required by `ProtocolClient`).
pub struct EnipClient {
    inner: tokio::sync::Mutex<EthernetIpClient>,
}

impl ProtocolClient for EnipClient {
    type Config = EnipConfig;
    type Tag = CipTag;
    type Value = CipValue;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let url = format!("{}:{}", config.host, config.port);

        let mut inner = EthernetIpClient::new(&url);
        tokio::time::timeout(std::time::Duration::from_secs(5), inner.connect())
            .await
            .map_err(|_| ProtocolError::Timeout("EtherNet/IP connect timeout (5s)".into()))?
            .map_err(|e| ProtocolError::ConnectionFailed(format!("{e}")))?;

        Ok(Self {
            inner: tokio::sync::Mutex::new(inner),
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut inner = self.inner.lock().await;

        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                inner.read_tag(&tag.name, tag.element_count),
            )
            .await
            .map_err(|_| {
                ProtocolError::Timeout(format!("Read timeout for tag '{}'", tag.name))
            })?
            .map_err(|e| ProtocolError::ReadFailed(format!("{e}")))?;

            results.push(value);
        }

        Ok(results)
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            inner.write_tag(&tag.name, &value, tag.element_count),
        )
        .await
        .map_err(|_| {
            ProtocolError::Timeout(format!("Write timeout for tag '{}'", tag.name))
        })?
        .map_err(|e| ProtocolError::WriteFailed(format!("{e}")))?;

        Ok(())
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .disconnect()
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))?;
        Ok(())
    }
}

// ── Convenience functions ──

/// Perform a single read of multiple tags from an EtherNet/IP device without
/// keeping a persistent client. Useful for one-shot reads.
pub async fn read_tags(
    config: &EnipConfig,
    tags: &[CipTag],
) -> Result<Vec<CipValue>, ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    let values = client.read(tags).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single write to an EtherNet/IP device without keeping a persistent client.
pub async fn write_tag(
    config: &EnipConfig,
    tag: &CipTag,
    value: CipValue,
) -> Result<(), ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    client.write(tag, value).await?;
    client.disconnect().await?;
    Ok(())
}

/// Discover all available tags on the PLC by iterating the Symbol class.
pub async fn browse_tags(config: &EnipConfig) -> Result<Vec<CipTag>, ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    let mut inner = client.inner.lock().await;
    inner
        .browse_tags()
        .await
        .map_err(|e| ProtocolError::Other(format!("{e}")))
}


// ── Discovery (explorer parity) ──

use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Broadcast a ListIdentity request and collect responders within `timeout_ms`.
pub async fn discover_devices(timeout_ms: u64) -> Vec<EnipDevice> {
    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let _ = sock.set_broadcast(true);
    let request = build_list_identity_request();
    let bcast: SocketAddr = match "255.255.255.255:44818".parse() {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    let _ = sock.send_to(&request, bcast).await;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    let mut devices: Vec<EnipDevice> = Vec::new();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut buf = vec![0u8; 4096];
    loop {
        let remain = deadline.checked_duration_since(tokio::time::Instant::now());
        let Some(remain) = remain else { break };
        match tokio::time::timeout(remain, sock.recv_from(&mut buf)).await {
            Ok(Ok((n, peer))) => {
                if let Some(mut dev) = parse_list_identity_response(&buf[..n]) {
                    dev.host = peer.ip().to_string();
                    dev.port = peer.port();
                    if seen.insert(dev.serial_number) {
                        devices.push(dev);
                    }
                }
            }
            _ => break,
        }
    }
    devices
}

/// Strip the CIP reply header (service | reserved | status | ext-word-count | ext...)
/// returning the payload bytes. Errors on non-success status.
pub(crate) fn strip_cip_reply(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 {
        anyhow::bail!("CIP reply too short");
    }
    let status = data[2];
    if status != CIP_STATUS_SUCCESS && status != CIP_STATUS_PARTIAL {
        let ext = if data.len() > 4 { data[3] } else { 0 };
        anyhow::bail!("CIP error: status=0x{:02X}, ext=0x{:02X}", status, ext);
    }
    let ext_words = data[3] as usize;
    let skip = 4 + 2 * ext_words;
    Ok(data.get(skip..).unwrap_or(&[]).to_vec())
}

impl EthernetIpClient {
    /// Read the Identity object (class 0x01, instance 1) of the connected device.
    pub(crate) async fn get_identity(&mut self) -> Result<IdentityInfo> {
        let vendor_id = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 1).await?;
        let device_type = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 2).await?;
        let product_code = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 3).await?;
        let rev_bytes = self.get_attribute_single_bytes(IDENTITY_CLASS_ID, 1, 4).await?;
        if rev_bytes.len() < 2 {
            anyhow::bail!("short revision attribute");
        }
        let revision_major = rev_bytes[0];
        let revision_minor = rev_bytes[1];
        let status = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 5).await?;
        let serial = self.get_attribute_single_u32(IDENTITY_CLASS_ID, 1, 6).await?;
        let product_name = self
            .get_attribute_single_short_string(IDENTITY_CLASS_ID, 1, 7)
            .await
            .unwrap_or_default();
        Ok(IdentityInfo {
            vendor_id,
            vendor_name: vendor_name(vendor_id).to_string(),
            device_type,
            product_code,
            revision_major,
            revision_minor,
            status,
            serial_number: serial,
            product_name,
        })
    }

    /// GetAttributeSingle returning raw payload bytes.
    pub(crate) async fn get_attribute_single_bytes(
        &mut self,
        class_id: u16,
        instance_id: u32,
        attribute_id: u16,
    ) -> Result<Vec<u8>> {
        let req = build_get_attribute_single_request(class_id, instance_id, attribute_id);
        let resp = self.send_rr_data(req).await?;
        strip_cip_reply(&resp)
    }

    async fn get_attribute_single_u16(&mut self, c: u16, i: u32, a: u16) -> Result<u16> {
        let b = self.get_attribute_single_bytes(c, i, a).await?;
        if b.len() < 2 {
            anyhow::bail!("short u16 attribute");
        }
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    async fn get_attribute_single_u32(&mut self, c: u16, i: u32, a: u16) -> Result<u32> {
        let b = self.get_attribute_single_bytes(c, i, a).await?;
        if b.len() < 4 {
            anyhow::bail!("short u32 attribute");
        }
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    async fn get_attribute_single_short_string(
        &mut self,
        c: u16,
        i: u32,
        a: u16,
    ) -> Result<String> {
        let b = self.get_attribute_single_bytes(c, i, a).await?;
        if b.is_empty() {
            return Ok(String::new());
        }
        let len = b[0] as usize;
        let s = b.get(1..1 + len).unwrap_or(&[]);
        Ok(String::from_utf8_lossy(s).trim_end_matches('\u{0000}').to_string())
    }

    /// SetAttributeSingle with raw little-endian data.
    pub(crate) async fn set_attribute_single(
        &mut self,
        class_id: u16,
        instance_id: u32,
        attribute_id: u16,
        data: &[u8],
    ) -> Result<()> {
        let req = build_set_attribute_single_request(class_id, instance_id, attribute_id, data);
        let resp = self.send_rr_data(req).await?;
        strip_cip_reply(&resp)?;
        Ok(())
    }

    /// Generic CIP service call returning raw reply payload.
    pub(crate) async fn generic_cip(
        &mut self,
        service: u8,
        class_id: u16,
        instance_id: u32,
        attribute_id: Option<u16>,
        data: &[u8],
    ) -> Result<Vec<u8>> {
        let req = build_generic_cip_request(service, class_id, instance_id, attribute_id, data);
        let resp = self.send_rr_data(req).await?;
        strip_cip_reply(&resp)
    }
}

impl EnipClient {
    /// Read the connected device Identity object.
    pub async fn get_identity(&self) -> Result<IdentityInfo, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .get_identity()
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))
    }

    /// Generic CIP service call on the connected device.
    pub async fn generic_cip(
        &self,
        service: u8,
        class_id: u16,
        instance_id: u32,
        attribute_id: Option<u16>,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .generic_cip(service, class_id, instance_id, attribute_id, &data)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))
    }

    /// SetAttributeSingle on the connected device.
    pub async fn set_attribute_single(
        &self,
        class_id: u16,
        instance_id: u32,
        attribute_id: u16,
        data: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .set_attribute_single(class_id, instance_id, attribute_id, &data)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))
    }

    /// Install a packet sink on the underlying connection.
    pub async fn set_packet_sink(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<PacketRecord>,
    ) {
        let mut inner = self.inner.lock().await;
        inner.set_packet_sink(tx);
    }
}

impl EthernetIpClient {
    /// Issue a ForwardOpen (CIP implicit/IO connection). Returns assigned IDs.
    pub(crate) async fn forward_open(&mut self, p: &ForwardOpenParams) -> Result<ForwardOpenResult> {
        let req = build_forward_open_request(p);
        let resp = self.send_rr_data(req).await?;
        parse_forward_open_response(&resp)
    }

    /// Issue a ForwardClose.
    pub(crate) async fn forward_close(
        &mut self,
        connection_serial: u16,
        vendor_id: u16,
        originator_serial: u32,
        connection_path: &[u8],
    ) -> Result<()> {
        let req =
            build_forward_close_request(connection_serial, vendor_id, originator_serial, connection_path);
        let resp = self.send_rr_data(req).await?;
        strip_cip_reply(&resp)?;
        Ok(())
    }
}

impl EnipClient {
    pub async fn forward_open(&self, p: ForwardOpenParams) -> Result<ForwardOpenResult, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .forward_open(&p)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))
    }

    pub async fn forward_close(
        &self,
        connection_serial: u16,
        vendor_id: u16,
        originator_serial: u32,
        connection_path: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .forward_close(connection_serial, vendor_id, originator_serial, &connection_path)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))
    }
}

impl EthernetIpClient {
    /// Enumerate instances of a CIP class via GetAttributesAll (service 0x01).
    /// Stops after `max` instances or 3 consecutive misses.
    pub(crate) async fn list_instances(
        &mut self,
        class_id: u16,
        max: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut out = Vec::new();
        let mut errs = 0u32;
        let mut inst = 1u32;
        while inst <= max && errs < 3 {
            let req = build_generic_cip_request(CIP_GET_ATTRIBUTES_ALL, class_id, inst, None, &[]);
            match self.send_rr_data(req).await {
                Ok(resp) => match strip_cip_reply(&resp) {
                    Ok(data) => {
                        if data.is_empty() {
                            errs += 1;
                        } else {
                            errs = 0;
                            out.push((inst, data));
                        }
                    }
                    Err(_) => errs += 1,
                },
                Err(_) => errs += 1,
            }
            inst += 1;
        }
        Ok(out)
    }
}

impl EnipClient {
    pub async fn list_instances(
        &self,
        class_id: u16,
        max: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .list_instances(class_id, max)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e}")))
    }
}

// ── SendUnitData (connected transport) + implicit IO connection ──

impl EthernetIpClient {
    /// Send a connected CIP message and read the immediate reply.
    pub(crate) async fn send_unit_data(
        &mut self,
        connection_id: u32,
        sequence: u16,
        cip: &[u8],
    ) -> Result<Vec<u8>> {
        let frame = build_send_unit_data(self.session_handle, connection_id, sequence, cip);
        self.emit_packet(PacketDir::Tx, CMD_SEND_UNIT_DATA, self.session_handle, &frame);

        let stream = self.stream.as_mut().context("Not connected")?;
        stream
            .write_all(&frame)
            .await
            .context("Write SendUnitData failed")?;
        stream.flush().await?;

        let (_conn, _seq, data) = self.read_one_unit_frame().await?;
        Ok(data)
    }

    /// Read exactly one ENIP frame from the socket, capture it, and parse as
    /// SendUnitData into (connection_id, sequence, cip_data).
    pub(crate) async fn read_one_unit_frame(&mut self) -> Result<(u32, u16, Vec<u8>)> {
        let stream = self.stream.as_mut().context("Not connected")?;
        let mut header_buf = [0u8; ENIP_HEADER_SIZE];
        stream
            .read_exact(&mut header_buf)
            .await
            .context("Read UnitData header failed")?;
        let length = u16::from_le_bytes(header_buf[2..4].try_into().unwrap()) as usize;
        let mut body_buf = vec![0u8; length];
        if length > 0 {
            stream
                .read_exact(&mut body_buf)
                .await
                .context("Read UnitData body failed")?;
        }
        let mut full = header_buf.to_vec();
        full.extend_from_slice(&body_buf);
        self.emit_packet(PacketDir::Rx, CMD_SEND_UNIT_DATA, self.session_handle, &full);
        parse_send_unit_data(&full)
    }
}

/// One cyclic IO sample delivered over a T→O (target→originator) connection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IoSample {
    pub sequence: u16,
    pub connection_id: u32,
    pub bytes: Vec<u8>,
    pub timestamp_ms: u64,
}

/// An open implicit/IO connection. Owns its own socket/session so a background
/// task can read cyclic T→O frames without contending the main tag client.
pub struct IoConnection {
    inner: EthernetIpClient,
    pub result: ForwardOpenResult,
}

impl IoConnection {
    /// Open a dedicated session and ForwardOpen an IO connection.
    pub async fn open(config: &EnipConfig, params: ForwardOpenParams) -> Result<Self> {
        let url = format!("{}:{}", config.host, config.port);
        let mut inner = EthernetIpClient::new(&url);
        tokio::time::timeout(std::time::Duration::from_secs(5), inner.connect())
            .await
            .context("IO connect timeout")??;
        let result = inner.forward_open(&params).await?;
        Ok(Self { inner, result })
    }

    /// Block until the next cyclic T→O frame arrives, decode it.
    pub async fn next_sample(&mut self) -> Result<IoSample> {
        let (conn, seq, data) = self.inner.read_one_unit_frame().await?;
        Ok(IoSample {
            sequence: seq,
            connection_id: conn,
            bytes: data,
            timestamp_ms: self.inner.now_ms(),
        })
    }

    /// Produce O→T data (manual write into the output assembly).
    pub async fn produce(&mut self, sequence: u16, cip: &[u8]) -> Result<Vec<u8>> {
        self.inner
            .send_unit_data(self.result.o_to_t_connection_id, sequence, cip)
            .await
    }

    /// ForwardClose + disconnect.
    pub async fn close(mut self, params: &ForwardOpenParams) -> Result<()> {
        let _ = self
            .inner
            .forward_close(
                params.connection_serial,
                params.vendor_id,
                params.originator_serial,
                &params.connection_path,
            )
            .await;
        self.inner.disconnect().await
    }
}
