use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use mt_protocol::{ProtocolClient, ProtocolError};

use crate::cip::*;
use crate::packets::*;
use crate::types::*;

/// Per-request wait for a reply on the TCP session.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest tag write sent in one Write Tag request; bigger values use Write
/// Tag Fragmented. Keeps the request inside a 504-byte unconnected packet.
const WRITE_SINGLE_MAX: usize = 380;
/// Request budget for one Multiple_Service_Packet (unconnected packets carry
/// at most 504 bytes, Unconnected_Send wrapper + route take ~24).
const MSP_MAX_REQUEST: usize = 400;
/// Reply budget for one Multiple_Service_Packet.
const MSP_MAX_REPLY: usize = 440;

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
    /// Unconsumed bytes from the socket. Replies are parsed from this buffer so
    /// a cancelled (timed-out) read can never leave a half-read frame behind.
    rx: Vec<u8>,
    /// Optional sink for raw wire frames (packet logger).
    packet_sink: Option<tokio::sync::mpsc::UnboundedSender<PacketRecord>>,
    /// EPath port segments of the configured route path (empty = direct).
    route: Vec<u8>,
    /// Tag types learned from reads/browse, so writes use the real type.
    type_cache: HashMap<String, TagType>,
}

impl EthernetIpClient {
    fn new(url: &str, route_path: Option<&str>) -> Result<Self> {
        let route = match route_path.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => parse_route_path(s)?,
            None => Vec::new(),
        };
        Ok(Self {
            stream: None,
            url: url.to_string(),
            session_handle: 0,
            sequence_counter: 0,
            rx: Vec::new(),
            packet_sink: None,
            route,
            type_cache: HashMap::new(),
        })
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
        self.rx.clear();

        self.register_session().await?;
        Ok(())
    }

    // ── Frame I/O ──

    /// Read one complete encapsulation frame (header + data). Cancel-safe.
    async fn read_frame(&mut self) -> Result<(Vec<u8>, Vec<u8>)> {
        loop {
            if self.rx.len() >= ENIP_HEADER_SIZE {
                let len = u16::from_le_bytes([self.rx[2], self.rx[3]]) as usize;
                if self.rx.len() >= ENIP_HEADER_SIZE + len {
                    let frame: Vec<u8> = self.rx.drain(..ENIP_HEADER_SIZE + len).collect();
                    let body = frame[ENIP_HEADER_SIZE..].to_vec();
                    let header = frame[..ENIP_HEADER_SIZE].to_vec();
                    let mut full = header.clone();
                    full.extend_from_slice(&body);
                    let command = u16::from_le_bytes([header[0], header[1]]);
                    let session = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
                    self.emit_packet(PacketDir::Rx, command, session, &full);
                    return Ok((header, body));
                }
            }
            let stream = self.stream.as_mut().context("Not connected")?;
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await.context("Read from EtherNet/IP socket failed")?;
            if n == 0 {
                self.stream = None;
                bail!("EtherNet/IP connection closed by the device");
            }
            self.rx.extend_from_slice(&chunk[..n]);
        }
    }

    async fn write_frame(&mut self, frame: &[u8]) -> Result<()> {
        let command = u16::from_le_bytes([frame[0], frame[1]]);
        self.emit_packet(PacketDir::Tx, command, self.session_handle, frame);
        let stream = self.stream.as_mut().context("Not connected")?;
        stream.write_all(frame).await.context("Write to EtherNet/IP socket failed")?;
        stream.flush().await?;
        Ok(())
    }

    /// Send a request frame and wait for the reply to it. Frames that echo a
    /// different sender context (late replies to requests that timed out) are
    /// discarded. A non-zero encapsulation status is an error.
    async fn exchange(&mut self, frame: Vec<u8>) -> Result<Vec<u8>> {
        let command = u16::from_le_bytes([frame[0], frame[1]]);
        let context = frame[12..20].to_vec();
        self.write_frame(&frame).await?;
        let deadline = tokio::time::Instant::now() + REPLY_TIMEOUT;
        loop {
            let (header, body) = tokio::time::timeout_at(deadline, self.read_frame())
                .await
                .map_err(|_| anyhow!("Timed out waiting for the device's reply"))??;
            if header[12..20] != context[..] {
                continue; // stale reply
            }
            let reply_command = u16::from_le_bytes([header[0], header[1]]);
            let status = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
            if status != 0 {
                bail!(
                    "EtherNet/IP encapsulation error 0x{status:04X}: {}",
                    encapsulation_status_text(status)
                );
            }
            if reply_command != command {
                bail!("Unexpected encapsulation reply command 0x{reply_command:04X} (sent 0x{command:04X})");
            }
            return Ok(body);
        }
    }

    // ── Session ──

    async fn register_session(&mut self) -> Result<()> {
        let mut body = [0u8; 4];
        body[0..2].copy_from_slice(&1u16.to_le_bytes()); // protocol version
        body[2..4].copy_from_slice(&0u16.to_le_bytes()); // options

        let frame = build_enip_header(CMD_REGISTER_SESSION, 0, 0, &body);
        let context = frame[12..20].to_vec();
        self.write_frame(&frame).await?;

        let deadline = tokio::time::Instant::now() + REPLY_TIMEOUT;
        loop {
            let (header, _body) = tokio::time::timeout_at(deadline, self.read_frame())
                .await
                .map_err(|_| anyhow!("Timed out waiting for RegisterSession reply"))??;
            if header[12..20] != context[..] {
                continue;
            }
            let status = u32::from_le_bytes(header[8..12].try_into().unwrap());
            if status != 0 {
                bail!(
                    "RegisterSession failed: encapsulation status 0x{status:04X} ({})",
                    encapsulation_status_text(status)
                );
            }
            let handle = u32::from_le_bytes(header[4..8].try_into().unwrap());
            if handle == 0 {
                bail!("RegisterSession reply carried session handle 0");
            }
            self.session_handle = handle;
            break;
        }
        tracing::debug!("EtherNet/IP session registered: handle=0x{:08X}", self.session_handle);
        Ok(())
    }

    /// Disconnect and unregister the session.
    async fn disconnect(&mut self) -> Result<()> {
        if self.stream.is_some() {
            let packet = build_enip_header(CMD_UNREGISTER_SESSION, self.session_handle, 0, &[]);
            let _ = self.write_frame(&packet).await;
            if let Some(mut stream) = self.stream.take() {
                let _ = stream.shutdown().await;
            }
        }
        self.session_handle = 0;
        self.rx.clear();
        Ok(())
    }

    // ── CIP transport ──

    /// SendRRData: send one unconnected CIP message (plus optional trailing CPF
    /// items) and return the reply message and any trailing reply items.
    async fn send_rr_data_items(&mut self, cip: &[u8], extra: &[CpfItem]) -> Result<(Vec<u8>, Vec<CpfItem>)> {
        let frame = build_send_rr_data(self.session_handle, cip, extra);
        let body = self.exchange(frame).await?;
        let mut items = parse_rr_body(&body)?;
        let pos = items
            .iter()
            .position(|i| i.type_id == CPF_UNCONNECTED_DATA)
            .ok_or_else(|| anyhow!("No unconnected data item in SendRRData reply"))?;
        let data = items.remove(pos).data;
        Ok((data, items))
    }

    async fn send_rr_data(&mut self, cip: &[u8]) -> Result<Vec<u8>> {
        Ok(self.send_rr_data_items(cip, &[]).await?.0)
    }

    /// Send a CIP request through the configured route: wrapped in an
    /// Unconnected_Send when a route path is set, direct otherwise.
    async fn send_cip(&mut self, request: Vec<u8>) -> Result<Vec<u8>> {
        if self.route.is_empty() {
            self.send_rr_data(&request).await
        } else {
            let wrapped = build_unconnected_send(&request, &self.route);
            self.send_rr_data(&wrapped).await
        }
    }

    /// Send a request and return the payload of a success (or partial) reply.
    async fn call(&mut self, request: Vec<u8>) -> Result<Vec<u8>> {
        let service = request[0];
        let resp = self.send_cip(request).await?;
        Ok(CipReply::parse(&resp)?.expect(service, true)?.data)
    }

    // ── Tags ──

    /// Read `count` elements, following partial-transfer replies with Read Tag
    /// Fragmented until the whole value arrives.
    async fn read_tag_raw(&mut self, name: &str, count: u16) -> Result<(TagType, Vec<u8>)> {
        let count = count.max(1);
        let req = build_read_tag_request(name, count)?;
        let resp = self.send_cip(req).await?;
        let first = parse_tag_read_reply(&resp, CIP_READ_TAG).with_context(|| format!("reading tag '{name}'"))?;
        let ty = first.ty;
        let mut data = first.data;
        let mut partial = first.partial;
        let mut rounds = 0;
        while partial {
            rounds += 1;
            if rounds > 4096 {
                bail!("tag '{name}': transfer did not complete");
            }
            let req = build_read_tag_fragmented_request(name, count, data.len() as u32)?;
            let resp = self.send_cip(req).await?;
            let frag = parse_tag_read_reply(&resp, CIP_READ_TAG_FRAGMENTED)
                .with_context(|| format!("reading tag '{name}' at offset {}", data.len()))?;
            if frag.data.is_empty() {
                bail!("tag '{name}': device returned an empty fragment at offset {}", data.len());
            }
            data.extend_from_slice(&frag.data);
            partial = frag.partial;
        }
        self.type_cache.insert(name.to_string(), ty);
        Ok((ty, data))
    }

    /// Read a single CIP tag by name. `Tag.5` style bit selectors read the
    /// parent word and extract the bit (Logix has no bit-level service).
    async fn read_tag(&mut self, tag_name: &str, element_count: u16) -> Result<CipValue> {
        if let Some((parent, bit)) = split_bit_suffix(tag_name) {
            let (ty, data) = self.read_tag_raw(parent, 1).await?;
            let word = decode_tag_value(&ty, &data, 1)?;
            return Ok(CipValue::Bool(bit_of(&word, bit)?));
        }
        let (ty, data) = self.read_tag_raw(tag_name, element_count).await?;
        decode_tag_value(&ty, &data, element_count)
    }

    /// Type to use when writing `tag`: the type seen on a previous read, else
    /// probed with a 1-element read, else the caller's declared type.
    async fn write_type(&mut self, name: &str, declared: CipDataType) -> TagType {
        if let Some(t) = self.type_cache.get(name) {
            return *t;
        }
        match self.read_tag_raw(name, 1).await {
            Ok((ty, _)) => ty,
            Err(_) => TagType::for_data_type(declared),
        }
    }

    /// Write a value to a CIP tag, coercing it to the tag's real data type and
    /// using Write Tag Fragmented for large values.
    async fn write_tag(&mut self, tag: &CipTag, value: &CipValue) -> Result<()> {
        let name = tag.name.as_str();
        if let Some((parent, bit)) = split_bit_suffix(name) {
            let (ty, data) = self.read_tag_raw(parent, 1).await?;
            let word = decode_tag_value(&ty, &data, 1)?;
            let set = int_of(value).map(|n| n != 0).ok_or_else(|| anyhow!("bit write needs a boolean value"))?;
            let new_word = with_bit(&word, bit, set)?;
            return self.write_value(parent, &ty, &new_word).await;
        }
        let ty = self.write_type(name, tag.data_type).await;
        let result = self.write_value(name, &ty, value).await;
        if let Err(e) = &result {
            if e.downcast_ref::<CipError>().is_some_and(|c| c.status == 0xFF && c.ext.first() == Some(&0x2107)) {
                self.type_cache.remove(name); // stale type: re-learn next time
            }
        }
        result
    }

    async fn write_value(&mut self, name: &str, ty: &TagType, value: &CipValue) -> Result<()> {
        let (count, data) = encode_tag_value(ty, value).with_context(|| format!("encoding value for tag '{name}'"))?;
        if data.len() <= WRITE_SINGLE_MAX {
            let req = build_write_tag_request(name, ty, count, &data)?;
            let resp = self.send_cip(req).await?;
            return parse_tag_write_reply(&resp, CIP_WRITE_TAG).with_context(|| format!("writing tag '{name}'"));
        }
        // Fragmented: chunk boundaries on 8-byte multiples so elements never split.
        let chunk = WRITE_SINGLE_MAX - WRITE_SINGLE_MAX % 8;
        let mut offset = 0usize;
        while offset < data.len() {
            let end = (offset + chunk).min(data.len());
            let req = build_write_tag_fragmented_request(name, ty, count, offset as u32, &data[offset..end])?;
            let resp = self.send_cip(req).await?;
            parse_tag_write_reply(&resp, CIP_WRITE_TAG_FRAGMENTED)
                .with_context(|| format!("writing tag '{name}' at offset {offset}"))?;
            offset = end;
        }
        Ok(())
    }

    /// Read several tags, batching scalar-sized reads into Multiple_Service_Packets.
    async fn read_many(&mut self, tags: &[CipTag]) -> Result<Vec<CipValue>> {
        let mut out: Vec<Option<CipValue>> = vec![None; tags.len()];
        let mut group: Vec<usize> = Vec::new();
        let (mut req_bytes, mut reply_bytes) = (0usize, 6usize);

        for (i, tag) in tags.iter().enumerate() {
            let batchable = tag.template_id.is_none() && split_bit_suffix(&tag.name).is_none();
            let est = batch_estimate(tag);
            if let (true, Some((req, reply))) = (batchable, est) {
                if !group.is_empty() && (req_bytes + req > MSP_MAX_REQUEST || reply_bytes + reply > MSP_MAX_REPLY) {
                    self.flush_group(tags, &mut group, &mut out).await?;
                    req_bytes = 0;
                    reply_bytes = 6;
                }
                group.push(i);
                req_bytes += req + 2;
                reply_bytes += reply + 2;
            } else {
                self.flush_group(tags, &mut group, &mut out).await?;
                req_bytes = 0;
                reply_bytes = 6;
                out[i] = Some(self.read_tag(&tag.name, tag.element_count).await?);
            }
        }
        self.flush_group(tags, &mut group, &mut out).await?;
        Ok(out.into_iter().map(|v| v.unwrap_or_default()).collect())
    }

    async fn flush_group(&mut self, tags: &[CipTag], group: &mut Vec<usize>, out: &mut [Option<CipValue>]) -> Result<()> {
        let idxs = std::mem::take(group);
        if idxs.is_empty() {
            return Ok(());
        }
        if idxs.len() == 1 {
            let t = &tags[idxs[0]];
            out[idxs[0]] = Some(self.read_tag(&t.name, t.element_count).await?);
            return Ok(());
        }
        let mut reqs = Vec::with_capacity(idxs.len());
        for &i in &idxs {
            reqs.push(build_read_tag_request(&tags[i].name, tags[i].element_count.max(1))?);
        }
        let msp = build_multiple_service_packet(&reqs);
        let parts = match self.send_cip(msp).await.and_then(|resp| {
            // Status 0x1E ("embedded service error") still carries every reply.
            let reply = CipReply::parse(&resp)?;
            if reply.service != CIP_MULTIPLE_SERVICE_PACKET | 0x80 {
                return Err(reply.error().into());
            }
            if reply.status != 0 && reply.status != 0x1E {
                return Err(reply.error().into());
            }
            split_multiple_service_data(&reply.data)
        }) {
            Ok(p) if p.len() == idxs.len() => p,
            // Target without Multiple_Service_Packet support: read one by one.
            _ => {
                for &i in &idxs {
                    out[i] = Some(self.read_tag(&tags[i].name, tags[i].element_count).await?);
                }
                return Ok(());
            }
        };
        for (&i, sub) in idxs.iter().zip(parts) {
            let tag = &tags[i];
            match parse_tag_read_reply(&sub, CIP_READ_TAG) {
                Ok(r) if !r.partial => {
                    self.type_cache.insert(tag.name.clone(), r.ty);
                    out[i] = Some(decode_tag_value(&r.ty, &r.data, tag.element_count)?);
                }
                // Partial (value too big for the shared packet) or error: re-read
                // alone so the fragmented path / the real error surfaces.
                _ => out[i] = Some(self.read_tag(&tag.name, tag.element_count).await?),
            }
        }
        Ok(())
    }

    /// Browse every tag through the Symbol object (class 0x6B) with
    /// Get_Instance_Attribute_List, controller scope first, then each
    /// `Program:<name>` scope.
    async fn browse_tags(&mut self) -> Result<Vec<CipTag>> {
        let controller = self.list_symbols(None).await?;
        let mut tags = Vec::new();
        let mut programs = Vec::new();
        for sym in &controller {
            if let Some(p) = sym.name.strip_prefix("Program:") {
                programs.push(p.to_string());
            } else if let Some(tag) = symbol_to_tag(&sym.name, sym) {
                tags.push(tag);
            }
        }
        for prog in programs {
            for sym in self.list_symbols(Some(&prog)).await? {
                if let Some(tag) = symbol_to_tag(&format!("Program:{prog}.{}", sym.name), &sym) {
                    tags.push(tag);
                }
            }
        }
        for t in &tags {
            if t.template_id.is_none() && t.element_count == 1 {
                self.type_cache.insert(t.name.clone(), TagType::for_data_type(t.data_type));
            }
        }
        Ok(tags)
    }

    async fn list_symbols(&mut self, program: Option<&str>) -> Result<Vec<Symbol>> {
        let mut out = Vec::new();
        let mut next: u32 = 0;
        for _ in 0..100_000 {
            let mut path = Vec::new();
            if let Some(p) = program {
                path.extend_from_slice(&encode_symbolic_path(&format!("Program:{p}"))?);
            }
            push_logical(&mut path, 0x20, SYMBOL_CLASS_ID as u32);
            push_logical(&mut path, 0x24, next);
            let mut req = vec![CIP_GET_INSTANCE_ATTRIBUTE_LIST, (path.len() / 2) as u8];
            req.extend_from_slice(&path);
            // attributes: 1 name, 2 type, 8 array dimensions
            req.extend_from_slice(&3u16.to_le_bytes());
            for a in [1u16, 2, 8] {
                req.extend_from_slice(&a.to_le_bytes());
            }
            let resp = self.send_cip(req).await?;
            let reply = CipReply::parse(&resp)?.expect(CIP_GET_INSTANCE_ATTRIBUTE_LIST, true)?;
            let (symbols, last) = parse_symbol_records(&reply.data)?;
            out.extend(symbols);
            if reply.status != CIP_STATUS_PARTIAL {
                return Ok(out);
            }
            match last {
                Some(id) if id >= next => next = id + 1,
                _ => bail!("symbol browse made no progress at instance {next}"),
            }
        }
        bail!("symbol browse did not terminate")
    }

    // ── Generic services ──

    /// Read the Identity object (class 0x01, instance 1) of the connected device.
    pub(crate) async fn get_identity(&mut self) -> Result<IdentityInfo> {
        // Get_Attributes_All returns attributes 1–7 in one round trip.
        let req = build_generic_cip_request(CIP_GET_ATTRIBUTES_ALL, IDENTITY_CLASS_ID, 1, None, &[]);
        if let Ok(data) = self.call(req).await {
            if let Some(info) = parse_identity_attributes(&data) {
                return Ok(info);
            }
        }
        let vendor_id = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 1).await?;
        let device_type = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 2).await?;
        let product_code = self.get_attribute_single_u16(IDENTITY_CLASS_ID, 1, 3).await?;
        let rev_bytes = self.get_attribute_single_bytes(IDENTITY_CLASS_ID, 1, 4).await?;
        if rev_bytes.len() < 2 {
            bail!("short revision attribute");
        }
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
            revision_major: rev_bytes[0],
            revision_minor: rev_bytes[1],
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
        self.call(build_get_attribute_single_request(class_id, instance_id, attribute_id)).await
    }

    async fn get_attribute_single_u16(&mut self, c: u16, i: u32, a: u16) -> Result<u16> {
        let b = self.get_attribute_single_bytes(c, i, a).await?;
        if b.len() < 2 {
            bail!("short u16 attribute");
        }
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    async fn get_attribute_single_u32(&mut self, c: u16, i: u32, a: u16) -> Result<u32> {
        let b = self.get_attribute_single_bytes(c, i, a).await?;
        if b.len() < 4 {
            bail!("short u32 attribute");
        }
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    async fn get_attribute_single_short_string(&mut self, c: u16, i: u32, a: u16) -> Result<String> {
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
        self.call(build_set_attribute_single_request(class_id, instance_id, attribute_id, data)).await?;
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
        self.call(build_generic_cip_request(service, class_id, instance_id, attribute_id, data)).await
    }

    // ── Connection Manager ──

    /// Issue a Forward_Open / Large_Forward_Open to the local Connection
    /// Manager (not routed — the connection path carries the route).
    /// `extra` are trailing CPF items (Sockaddr Info). Returns the assigned IDs
    /// plus the reply's trailing CPF items.
    async fn forward_open(&mut self, p: &ForwardOpenParams, extra: &[CpfItem]) -> Result<(ForwardOpenResult, Vec<CpfItem>)> {
        let req = build_forward_open_request(p)?;
        let (resp, items) = self.send_rr_data_items(&req, extra).await?;
        Ok((parse_forward_open_response(&resp, p.large).context("Forward_Open")?, items))
    }

    async fn forward_close(
        &mut self,
        connection_serial: u16,
        vendor_id: u16,
        originator_serial: u32,
        connection_path: &[u8],
    ) -> Result<()> {
        let req = build_forward_close_request(connection_serial, vendor_id, originator_serial, connection_path);
        let resp = self.send_rr_data(&req).await?;
        CipReply::parse(&resp)?.expect(CIP_FORWARD_CLOSE, false).context("Forward_Close")?;
        Ok(())
    }

    /// Enumerate instances of a CIP class via GetAttributesAll (service 0x01).
    /// Stops after `max` instances or 3 consecutive misses.
    pub(crate) async fn list_instances(&mut self, class_id: u16, max: u32) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut out = Vec::new();
        let mut errs = 0u32;
        let mut inst = 1u32;
        while inst <= max && errs < 3 {
            let req = build_generic_cip_request(CIP_GET_ATTRIBUTES_ALL, class_id, inst, None, &[]);
            match self.call(req).await {
                Ok(data) if !data.is_empty() => {
                    errs = 0;
                    out.push((inst, data));
                }
                Ok(_) => errs += 1,
                Err(e) if e.downcast_ref::<CipError>().is_some() => errs += 1,
                Err(e) => return Err(e), // transport failure: do not mask it as "no more instances"
            }
            inst += 1;
        }
        Ok(out)
    }

    /// Read a Template object's definition blob: Read Tag (0x4C) on class 0x6C
    /// carrying `offset u32 | length u16`, continued at the next offset while
    /// the device answers partial transfer (0x06).
    async fn read_template_blob(&mut self, instance: u32, size: usize) -> Result<Vec<u8>> {
        let mut blob = Vec::new();
        for _ in 0..4096 {
            let want = size.saturating_sub(blob.len()).clamp(1, u16::MAX as usize);
            let mut data = (blob.len() as u32).to_le_bytes().to_vec();
            data.extend_from_slice(&(want as u16).to_le_bytes());
            let req = build_generic_cip_request(CIP_READ_TAG, 0x6C, instance, None, &data);
            let resp = self.send_cip(req).await?;
            let reply = CipReply::parse(&resp)?.expect(CIP_READ_TAG, true)?;
            blob.extend_from_slice(&reply.data);
            if reply.status != CIP_STATUS_PARTIAL || reply.data.is_empty() {
                return Ok(blob);
            }
        }
        bail!("Template {instance}: definition transfer did not complete")
    }

    // ── Connected explicit messaging (class 3) ──

    fn next_sequence(&mut self) -> u16 {
        self.sequence_counter = self.sequence_counter.wrapping_add(1);
        self.sequence_counter
    }

    /// Send a connected CIP message and read the immediate reply.
    async fn send_unit_data(&mut self, connection_id: u32, sequence: u16, cip: &[u8]) -> Result<Vec<u8>> {
        let frame = build_send_unit_data(self.session_handle, connection_id, sequence, cip);
        self.write_frame(&frame).await?;
        let deadline = tokio::time::Instant::now() + REPLY_TIMEOUT;
        loop {
            let (_conn, seq, data) = tokio::time::timeout_at(deadline, self.read_one_unit_frame())
                .await
                .map_err(|_| anyhow!("Timed out waiting for the connected reply"))??;
            if seq == sequence {
                return Ok(data);
            }
        }
    }

    /// Read exactly one SendUnitData frame from the session socket.
    async fn read_one_unit_frame(&mut self) -> Result<(u32, u16, Vec<u8>)> {
        loop {
            let (header, body) = self.read_frame().await?;
            let command = u16::from_le_bytes([header[0], header[1]]);
            if command == CMD_SEND_UNIT_DATA {
                return parse_unit_data_body(&body);
            }
        }
    }
}

// ── Helpers ──

/// (request bytes, reply bytes) a tag read adds to a Multiple_Service_Packet,
/// or `None` when the reply size cannot be bounded.
fn batch_estimate(tag: &CipTag) -> Option<(usize, usize)> {
    let path = encode_symbolic_path(&tag.name).ok()?;
    let es = tag.data_type.element_size() as usize;
    let count = tag.element_count.max(1) as usize;
    let type_len = if tag.data_type == CipDataType::String { 4 } else { 2 };
    Some((2 + path.len() + 2, 4 + type_len + es * count))
}

/// Extract bit `bit` of an integer-like value.
fn bit_of(word: &CipValue, bit: u8) -> Result<bool> {
    let bits = word_bits(word)?;
    Ok(bits >> bit & 1 == 1)
}

fn word_bits(word: &CipValue) -> Result<u64> {
    match word {
        CipValue::Bytes(b) if b.len() <= 8 => {
            let mut a = [0u8; 8];
            a[..b.len()].copy_from_slice(b);
            Ok(u64::from_le_bytes(a))
        }
        other => int_of(other)
            .map(|n| n as u64)
            .ok_or_else(|| anyhow!("bit access needs an integer tag, got {other:?}")),
    }
}

/// The same integer type as `word` with `bit` forced to `set`.
fn with_bit(word: &CipValue, bit: u8, set: bool) -> Result<CipValue> {
    let mut bits = word_bits(word)?;
    if set {
        bits |= 1 << bit;
    } else {
        bits &= !(1 << bit);
    }
    Ok(match word {
        CipValue::Sint(_) => CipValue::Sint(bits as i8),
        CipValue::Int(_) => CipValue::Int(bits as i16),
        CipValue::Dint(_) => CipValue::Dint(bits as i32),
        CipValue::Lint(_) => CipValue::Lint(bits as i64),
        CipValue::Usint(_) => CipValue::Usint(bits as u8),
        CipValue::Uint(_) => CipValue::Uint(bits as u16),
        CipValue::Udint(_) => CipValue::Udint(bits as u32),
        CipValue::Ulint(_) => CipValue::Ulint(bits),
        CipValue::Bytes(b) => CipValue::Bytes(bits.to_le_bytes()[..b.len()].to_vec()),
        other => bail!("bit access needs an integer tag, got {other:?}"),
    })
}

/// One Symbol object instance from Get_Instance_Attribute_List.
#[derive(Debug, Clone)]
pub(crate) struct Symbol {
    pub instance: u32,
    pub name: String,
    pub type_word: u16,
    pub dims: [u32; 3],
}

/// Parse `[instance u32][name: u16 len + bytes][type u16][dims u32×3]` records
/// (attributes 1, 2, 8). Returns the records and the last instance id.
pub(crate) fn parse_symbol_records(data: &[u8]) -> Result<(Vec<Symbol>, Option<u32>)> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < data.len() {
        let need = |n: usize, p: usize| -> Result<()> {
            if p + n > data.len() {
                bail!("symbol record truncated");
            }
            Ok(())
        };
        need(6, p)?;
        let instance = u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
        let name_len = u16::from_le_bytes([data[p + 4], data[p + 5]]) as usize;
        p += 6;
        need(name_len + 2 + 12, p)?;
        let name = String::from_utf8_lossy(&data[p..p + name_len]).into_owned();
        p += name_len;
        let type_word = u16::from_le_bytes([data[p], data[p + 1]]);
        p += 2;
        let mut dims = [0u32; 3];
        for d in dims.iter_mut() {
            *d = u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
            p += 4;
        }
        out.push(Symbol { instance, name, type_word, dims });
    }
    let last = out.last().map(|s| s.instance);
    Ok((out, last))
}

/// Turn a Symbol record into a tag descriptor, skipping system/internal
/// symbols the way pycomm3's `_isolate_user_tags` does.
///
/// Symbol type word (1756-PM020): bit 15 = structure (low 12 bits = template
/// instance id), bit 12 = system tag, bits 13–14 = array dimension count,
/// bits 0–7 = atomic type code (bits 8–10 = BOOL bit position).
pub(crate) fn symbol_to_tag(full_name: &str, sym: &Symbol) -> Option<CipTag> {
    let name = sym.name.as_str();
    if name.starts_with("__")
        || name.starts_with("Routine:")
        || name.starts_with("Task:")
        || name.contains("Map:")
        || name.contains("Cxn:")
    {
        return None;
    }
    if sym.type_word & 0x1000 != 0 {
        return None; // system tag
    }
    let dim_count = ((sym.type_word >> 13) & 3) as usize;
    let element_count = if dim_count == 0 {
        1
    } else {
        sym.dims[..dim_count].iter().map(|d| (*d).max(1) as u64).product::<u64>().min(u16::MAX as u64) as u16
    };
    if sym.type_word & 0x8000 != 0 {
        let tid = sym.type_word & 0x0FFF;
        if tid == STRUCT_HANDLE_LOGIX_STRING {
            return Some(CipTag { name: full_name.into(), data_type: CipDataType::String, element_count, template_id: None });
        }
        return Some(CipTag { name: full_name.into(), data_type: CipDataType::Dint, element_count, template_id: Some(tid) });
    }
    let data_type = CipDataType::from_code((sym.type_word & 0xFF) as u8)?;
    Some(CipTag { name: full_name.into(), data_type, element_count, template_id: None })
}

/// Parse Identity Get_Attributes_All data: vendor, device type, product code,
/// revision, status, serial, product name (SHORT_STRING).
pub(crate) fn parse_identity_attributes(d: &[u8]) -> Option<IdentityInfo> {
    if d.len() < 15 {
        return None;
    }
    let vendor_id = u16::from_le_bytes([d[0], d[1]]);
    let name_len = d[14] as usize;
    let name = d.get(15..15 + name_len)?;
    Some(IdentityInfo {
        vendor_id,
        vendor_name: vendor_name(vendor_id).to_string(),
        device_type: u16::from_le_bytes([d[2], d[3]]),
        product_code: u16::from_le_bytes([d[4], d[5]]),
        revision_major: d[6],
        revision_minor: d[7],
        status: u16::from_le_bytes([d[8], d[9]]),
        serial_number: u32::from_le_bytes([d[10], d[11], d[12], d[13]]),
        product_name: String::from_utf8_lossy(name).trim_end_matches(['\0', ' ']).to_string(),
    })
}

// ── Public ProtocolClient implementation ──

/// Standalone EtherNet/IP client implementing the [`ProtocolClient`] trait.
///
/// Wraps the internal [`EthernetIpClient`] in an async Mutex so that
/// `EnipClient` satisfies `Sync` (required by `ProtocolClient`).
pub struct EnipClient {
    inner: tokio::sync::Mutex<EthernetIpClient>,
}

fn connect_error(e: anyhow::Error) -> ProtocolError {
    ProtocolError::ConnectionFailed(format!("{e:#}"))
}

impl ProtocolClient for EnipClient {
    type Config = EnipConfig;
    type Tag = CipTag;
    type Value = CipValue;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let url = format!("{}:{}", config.host, config.port);

        let mut inner = EthernetIpClient::new(&url, config.route_path.as_deref()).map_err(connect_error)?;
        tokio::time::timeout(Duration::from_secs(5), inner.connect())
            .await
            .map_err(|_| ProtocolError::Timeout("EtherNet/IP connect timeout (5s)".into()))?
            .map_err(connect_error)?;

        Ok(Self { inner: tokio::sync::Mutex::new(inner) })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .read_many(tags)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("{e:#}")))
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .write_tag(tag, &value)
            .await
            .map_err(|e| ProtocolError::WriteFailed(format!("{e:#}")))
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner.disconnect().await.map_err(|e| ProtocolError::Other(format!("{e:#}")))?;
        Ok(())
    }
}

// ── Convenience functions ──

/// Perform a single read of multiple tags from an EtherNet/IP device without
/// keeping a persistent client. Useful for one-shot reads.
pub async fn read_tags(config: &EnipConfig, tags: &[CipTag]) -> Result<Vec<CipValue>, ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    let values = client.read(tags).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single write to an EtherNet/IP device without keeping a persistent client.
pub async fn write_tag(config: &EnipConfig, tag: &CipTag, value: CipValue) -> Result<(), ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    client.write(tag, value).await?;
    client.disconnect().await?;
    Ok(())
}

/// Discover all available tags on the PLC (controller scope + every program).
pub async fn browse_tags(config: &EnipConfig) -> Result<Vec<CipTag>, ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    let tags = client.browse().await;
    let _ = client.disconnect().await;
    tags
}

// ── Discovery (explorer parity) ──

/// Broadcast a ListIdentity request on every IPv4 interface (directed subnet
/// broadcast, like the BACnet scanner — a bare 255.255.255.255 only leaves
/// through the default route on macOS) and collect responders within `timeout_ms`.
pub async fn discover_devices(timeout_ms: u64) -> Vec<EnipDevice> {
    let replies = list_identity(
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        Some(44818),
        &[],
        Duration::from_millis(timeout_ms),
    )
    .await
    .unwrap_or_default();
    let mut devices: Vec<EnipDevice> = Vec::new();
    let mut seen: std::collections::HashSet<(String, u16)> = std::collections::HashSet::new();
    for reply in replies {
        // Same device may answer on several interfaces; serial numbers
        // are not unique across vendors, so key on the endpoint.
        if seen.insert((reply.device.host.clone(), reply.device.port)) {
            devices.push(reply.device);
        }
    }
    devices
}

/// One answer to a ListIdentity request: the datagram's sender and the
/// identity it carried (`device.host` is the sender's IP).
#[derive(Debug, Clone)]
pub struct ListIdentityReply {
    pub from: SocketAddr,
    pub device: EnipDevice,
}

/// Send ListIdentity over UDP from `bind`: broadcast to `broadcast_port` on
/// every IPv4 interface (and 255.255.255.255) when set, and unicast to each of
/// `directed` (devices on other ports, behind routers or on this computer),
/// then collect every answer for `wait`. Fails only when the socket cannot be
/// opened or no request could be sent at all.
pub async fn list_identity(
    bind: SocketAddr,
    broadcast_port: Option<u16>,
    directed: &[SocketAddr],
    wait: Duration,
) -> std::io::Result<Vec<ListIdentityReply>> {
    let sock = UdpSocket::bind(bind).await?;
    let request = build_list_identity_request();

    let mut targets: Vec<SocketAddr> = Vec::new();
    if let Some(port) = broadcast_port {
        sock.set_broadcast(true)?;
        let mut addresses: Vec<Ipv4Addr> = Vec::new();
        if let Ok(ifaces) = if_addrs::get_if_addrs() {
            for iface in ifaces {
                if let if_addrs::IfAddr::V4(v4) = iface.addr {
                    if let Some(b) = v4.broadcast {
                        if !addresses.contains(&b) {
                            addresses.push(b);
                        }
                    }
                }
            }
        }
        if !addresses.contains(&Ipv4Addr::BROADCAST) {
            addresses.push(Ipv4Addr::BROADCAST);
        }
        targets.extend(addresses.into_iter().map(|a| SocketAddr::from((a, port))));
    }
    for target in directed {
        if !targets.contains(target) {
            targets.push(*target);
        }
    }
    let mut sent = 0usize;
    let mut last_error = None;
    for target in &targets {
        match sock.send_to(&request, target).await {
            Ok(_) => sent += 1,
            Err(e) => last_error = Some(e),
        }
    }
    if sent == 0 {
        if let Some(e) = last_error {
            return Err(e);
        }
    }

    let deadline = tokio::time::Instant::now() + wait;
    let mut replies = Vec::new();
    let mut buf = vec![0u8; 4096];
    loop {
        let Some(remain) = deadline.checked_duration_since(tokio::time::Instant::now()) else { break };
        match tokio::time::timeout(remain, sock.recv_from(&mut buf)).await {
            Ok(Ok((n, from))) => {
                if let Some(mut device) = parse_list_identity_response(&buf[..n]) {
                    device.host = from.ip().to_string();
                    replies.push(ListIdentityReply { from, device });
                }
            }
            // An ICMP port-unreachable from a directed target surfaces as a
            // receive error on some systems: keep listening for the others.
            Ok(Err(_)) => continue,
            Err(_) => break,
        }
    }
    Ok(replies)
}

// ── Public methods beyond ProtocolClient ──

impl EnipClient {
    /// Browse all tags over this client's session.
    pub async fn browse(&self) -> Result<Vec<CipTag>, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner.browse_tags().await.map_err(|e| ProtocolError::Other(format!("{e:#}")))
    }

    /// Read the connected device Identity object.
    pub async fn get_identity(&self) -> Result<IdentityInfo, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner.get_identity().await.map_err(|e| ProtocolError::Other(format!("{e:#}")))
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
            .map_err(|e| ProtocolError::Other(format!("{e:#}")))
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
            .map_err(|e| ProtocolError::Other(format!("{e:#}")))
    }

    /// Install a packet sink on the underlying connection.
    pub async fn set_packet_sink(&self, tx: tokio::sync::mpsc::UnboundedSender<PacketRecord>) {
        let mut inner = self.inner.lock().await;
        inner.set_packet_sink(tx);
    }

    pub async fn forward_open(&self, p: ForwardOpenParams) -> Result<ForwardOpenResult, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .forward_open(&p, &[])
            .await
            .map(|(r, _)| r)
            .map_err(|e| ProtocolError::Other(format!("{e:#}")))
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
            .map_err(|e| ProtocolError::Other(format!("{e:#}")))
    }

    /// Read up to `size` bytes of a Template object's definition blob.
    pub(crate) async fn read_template_blob(&self, instance: u32, size: usize) -> Result<Vec<u8>, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .read_template_blob(instance, size)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e:#}")))
    }

    pub async fn list_instances(&self, class_id: u16, max: u32) -> Result<Vec<(u32, Vec<u8>)>, ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .list_instances(class_id, max)
            .await
            .map_err(|e| ProtocolError::Other(format!("{e:#}")))
    }
}

// ── Implicit / explicit I/O connection ──

/// One cyclic IO sample delivered over a T→O (target→originator) connection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IoSample {
    pub sequence: u16,
    pub connection_id: u32,
    pub bytes: Vec<u8>,
    pub timestamp_ms: u64,
}

enum IoMode {
    /// Class 3: connected explicit messaging over SendUnitData on the TCP
    /// session, kept alive by one request per RPI.
    Explicit { ticker: tokio::time::Interval },
    /// Class 0/1: cyclic UDP I/O (default port 2222).
    Implicit {
        sock: Arc<UdpSocket>,
        /// Application data of the next O→T packets (after the run/idle header).
        output: Arc<Mutex<Vec<u8>>>,
        producer: Option<tokio::task::JoinHandle<()>>,
    },
}

/// An open CIP connection. Owns its own TCP session so a background task can
/// read cyclic T→O frames without contending the main tag client.
pub struct IoConnection {
    inner: EthernetIpClient,
    pub result: ForwardOpenResult,
    mode: IoMode,
}

impl Drop for IoConnection {
    fn drop(&mut self) {
        if let IoMode::Implicit { producer: Some(h), .. } = &self.mode {
            h.abort();
        }
    }
}

/// Connection size encoded in network connection parameters.
fn connection_size(params: u32, large: bool) -> usize {
    (if large { params & 0xFFFF } else { params & 0x01FF }) as usize
}

fn connection_type(params: u32, large: bool) -> u32 {
    if large { (params >> 29) & 3 } else { (params >> 13) & 3 }
}

impl IoConnection {
    /// Open a dedicated session and Forward_Open a connection. Class 3
    /// (transport 0xA3) opens connected explicit messaging; class 0/1 opens an
    /// implicit I/O connection with UDP cyclic data (T→O received on a local UDP
    /// socket, O→T heartbeats/outputs sent every RPI).
    pub async fn open(config: &EnipConfig, params: ForwardOpenParams) -> Result<Self> {
        let url = format!("{}:{}", config.host, config.port);
        let mut inner = EthernetIpClient::new(&url, config.route_path.as_deref())?;
        tokio::time::timeout(Duration::from_secs(5), inner.connect())
            .await
            .context("IO connect timeout")??;

        let implicit = params.transport_class_trigger & 0x0F <= 1;
        if !implicit {
            let (result, _) = inner.forward_open(&params, &[]).await?;
            let period = Duration::from_micros(params.o_to_t_rpi_us.max(100_000) as u64);
            let ticker = tokio::time::interval(period);
            return Ok(Self { inner, result, mode: IoMode::Explicit { ticker } });
        }

        // UDP socket for cyclic data: the registered port 2222 when free,
        // otherwise an ephemeral port advertised in a T→O Sockaddr Info item.
        let sock = match UdpSocket::bind(("0.0.0.0", IO_UDP_PORT)).await {
            Ok(s) => s,
            Err(_) => UdpSocket::bind(("0.0.0.0", 0)).await.context("bind UDP socket for I/O")?,
        };
        let local_port = sock.local_addr()?.port();
        let (peer_ip, local_ip) = {
            let s = inner.stream.as_ref().context("Not connected")?;
            let peer = s.peer_addr()?.ip();
            let local = match s.local_addr()?.ip() {
                IpAddr::V4(v4) => v4.octets(),
                IpAddr::V6(_) => [0, 0, 0, 0],
            };
            (peer, local)
        };
        let mut extra = Vec::new();
        if local_port != IO_UDP_PORT {
            extra.push(CpfItem::new(CPF_SOCKADDR_T_TO_O, sockaddr_item_data(local_ip, local_port)));
        }

        let (result, reply_items) = inner.forward_open(&params, &extra).await?;

        // O→T destination: the target's Sockaddr Info if it sent one, else port 2222.
        let o_to_t_dest = reply_items
            .iter()
            .find(|i| i.type_id == CPF_SOCKADDR_O_TO_T)
            .and_then(|i| parse_sockaddr_item(&i.data))
            .map(|(ip, port)| {
                let host = if ip == [0, 0, 0, 0] { peer_ip } else { IpAddr::from(ip) };
                SocketAddr::new(host, if port == 0 { IO_UDP_PORT } else { port })
            })
            .unwrap_or_else(|| SocketAddr::new(peer_ip, IO_UDP_PORT));

        let sock = Arc::new(sock);
        let o_size = connection_size(params.o_to_t_params, params.large);
        let header_len = if params.o_to_t_run_idle { 4 } else { 0 };
        let output = Arc::new(Mutex::new(vec![0u8; o_size.saturating_sub(2 + header_len)]));
        let producer = if connection_type(params.o_to_t_params, params.large) != 0 {
            let period_us = if result.o_to_t_api_us > 0 { result.o_to_t_api_us } else { params.o_to_t_rpi_us };
            Some(tokio::spawn(io_output_task(
                sock.clone(),
                o_to_t_dest,
                result.o_to_t_connection_id,
                Duration::from_micros(period_us.max(1000) as u64),
                params.o_to_t_run_idle,
                output.clone(),
            )))
        } else {
            None
        };
        Ok(Self { inner, result, mode: IoMode::Implicit { sock, output, producer } })
    }

    /// Block until the next sample is available. Implicit connections deliver
    /// the next cyclic T→O frame. Explicit (class 3) connections time out when
    /// idle, so each call sends one connected Get_Attribute_Single (Identity
    /// vendor id) per RPI and returns the reply as the sample.
    pub async fn next_sample(&mut self) -> Result<IoSample> {
        match &mut self.mode {
            IoMode::Explicit { ticker } => {
                ticker.tick().await;
                let seq = self.inner.next_sequence();
                let req = build_get_attribute_single_request(IDENTITY_CLASS_ID, 1, 1);
                let data = self.inner.send_unit_data(self.result.o_to_t_connection_id, seq, &req).await?;
                Ok(IoSample {
                    sequence: seq,
                    connection_id: self.result.t_to_o_connection_id,
                    bytes: data,
                    timestamp_ms: self.inner.now_ms(),
                })
            }
            IoMode::Implicit { sock, .. } => {
                let sock = sock.clone();
                let mut buf = vec![0u8; 2048];
                loop {
                    let (n, _peer) = sock.recv_from(&mut buf).await?;
                    let Ok((conn, _seq32, seq, payload)) = parse_io_packet(&buf[..n]) else { continue };
                    if conn != self.result.t_to_o_connection_id {
                        continue;
                    }
                    return Ok(IoSample { sequence: seq, connection_id: conn, bytes: payload, timestamp_ms: self.inner.now_ms() });
                }
            }
        }
    }

    /// Class 3: send a connected CIP request and return the reply. Class 1:
    /// replace the O→T output data (the cyclic task sends it next RPI).
    pub async fn produce(&mut self, sequence: u16, cip: &[u8]) -> Result<Vec<u8>> {
        match &self.mode {
            IoMode::Explicit { .. } => {
                let seq = if sequence == 0 { self.inner.next_sequence() } else { sequence };
                self.inner.send_unit_data(self.result.o_to_t_connection_id, seq, cip).await
            }
            IoMode::Implicit { output, .. } => {
                let mut o = output.lock();
                let len = o.len();
                o.clear();
                o.extend_from_slice(cip);
                o.resize(len, 0);
                Ok(Vec::new())
            }
        }
    }

    /// Forward_Close + disconnect.
    pub async fn close(mut self, params: &ForwardOpenParams) -> Result<()> {
        if let IoMode::Implicit { producer: Some(h), .. } = &self.mode {
            h.abort();
        }
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

/// Cyclic O→T producer for class 0/1 connections: one datagram per RPI with a
/// sequenced address item, 16-bit sequence count, optional run/idle header and
/// the output image. The target drops the connection if these stop.
async fn io_output_task(
    sock: Arc<UdpSocket>,
    dest: SocketAddr,
    connection_id: u32,
    period: Duration,
    run_idle: bool,
    output: Arc<Mutex<Vec<u8>>>,
) {
    let mut ticker = tokio::time::interval(period);
    let mut seq32 = 0u32;
    let mut seq16 = 0u16;
    loop {
        ticker.tick().await;
        seq32 = seq32.wrapping_add(1);
        seq16 = seq16.wrapping_add(1);
        let mut payload = Vec::new();
        if run_idle {
            payload.extend_from_slice(&1u32.to_le_bytes()); // bit 0 = run
        }
        payload.extend_from_slice(&output.lock());
        let frame = build_io_packet(connection_id, seq32, seq16, &payload);
        if sock.send_to(&frame, dest).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::start_enip_server;

    /// Real adapters close an implicit connection when the originator stops
    /// producing O→T packets (RPI × timeout multiplier). The simulated target
    /// does the same, which is what lets this client's heartbeat be verified.
    #[tokio::test]
    async fn target_drops_the_io_connection_when_o_to_t_heartbeats_stop() {
        let (state, mut rx, _shutdown) = start_enip_server(
            EnipServerConfig { port: 0, bind_addr: "127.0.0.1".into() },
            vec![CipTagDef {
                name: "A".into(),
                data_type: CipDataType::Dint,
                element_count: 1,
                initial_value: CipValue::Dint(1),
                template_id: None,
                read_only: false,
            }],
            vec![],
        )
        .await
        .unwrap();
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let cfg = EnipConfig {
            id: "wd".into(),
            host: "127.0.0.1".into(),
            port: state.tcp_port(),
            poll_interval_ms: 1000,
            route_path: None,
        };
        let params = ForwardOpenParams {
            o_to_t_connection_id: 0,
            t_to_o_connection_id: 7,
            connection_serial: 9,
            vendor_id: 1,
            originator_serial: 1,
            priority_time_tick: 0x0A,
            timeout_ticks: 0x05,
            timeout_multiplier: 0, // ×4
            o_to_t_rpi_us: 20_000,
            o_to_t_params: network_connection_params(10, ConnectionType::PointToPoint, false, 0, false),
            t_to_o_rpi_us: 20_000,
            t_to_o_params: network_connection_params(10, ConnectionType::PointToPoint, false, 0, false),
            transport_class_trigger: 0x01,
            large: false,
            o_to_t_run_idle: true,
            connection_path: vec![0x20, 0x04, 0x24, 0x66, 0x2C, 0x96, 0x2C, 0x64],
        };
        let mut conn = IoConnection::open(&cfg, params).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.unwrap().unwrap();

        // Heartbeats keep the connection alive well past the watchdog period.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(state.open_io_connections().await, 1);

        // Stop sending; the target must give up and stop producing.
        match &conn.mode {
            IoMode::Implicit { producer: Some(h), .. } => h.abort(),
            _ => panic!("expected an implicit connection"),
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(state.open_io_connections().await, 0, "watchdog expired");
        let mut drained = 0;
        while tokio::time::timeout(Duration::from_millis(150), conn.next_sample()).await.is_ok() {
            drained += 1;
            assert!(drained < 500, "target keeps producing after its watchdog expired");
        }
    }

    #[test]
    fn bit_helpers_round_trip_every_integer_width() {
        for (word, bit, want) in [
            (CipValue::Sint(0b100), 2, true),
            (CipValue::Int(0b100), 1, false),
            (CipValue::Dint(1 << 31), 31, true),
            (CipValue::Lint(1 << 40), 40, true),
            (CipValue::Udint(0x8000_0000), 31, true),
            (CipValue::Bytes(vec![0x00, 0x01, 0, 0]), 8, true),
        ] {
            assert_eq!(bit_of(&word, bit).unwrap(), want, "{word:?} bit {bit}");
        }
        assert_eq!(with_bit(&CipValue::Dint(0), 3, true).unwrap(), CipValue::Dint(8));
        assert_eq!(with_bit(&CipValue::Sint(-1), 7, false).unwrap(), CipValue::Sint(0x7F));
        assert_eq!(with_bit(&CipValue::Bytes(vec![0; 4]), 31, true).unwrap(), CipValue::Bytes(vec![0, 0, 0, 0x80]));
        assert!(with_bit(&CipValue::String("x".into()), 0, true).is_err());
    }
}
