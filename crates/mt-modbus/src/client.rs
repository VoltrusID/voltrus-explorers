use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::sync::watch;
use tokio_modbus::client::{Context, tcp};
use tokio_modbus::prelude::*;

use crate::types::{
    Endianness, ModbusConfig, ModbusTag, ModbusValue, TrafficLogEntry, check_writable,
    parse_register, value_to_words,
};
#[cfg(feature = "serial")]
use crate::types::{BusScanResult, RtuSerialConfig};

/// Largest register count of one FC03/FC04 request (Modbus Application Protocol v1.1b3, §6.3/§6.4).
const MAX_READ_REGISTERS: u32 = 125;
/// Largest bit count of one FC01/FC02 request (§6.1/§6.2).
const MAX_READ_BITS: u32 = 2000;

/// Human-readable name of a Modbus exception code (Modbus Application Protocol v1.1b3, §7).
pub fn exception_name(code: ExceptionCode) -> &'static str {
    match code {
        ExceptionCode::IllegalFunction => "Illegal Function",
        ExceptionCode::IllegalDataAddress => "Illegal Data Address",
        ExceptionCode::IllegalDataValue => "Illegal Data Value",
        ExceptionCode::ServerDeviceFailure => "Server Device Failure",
        ExceptionCode::Acknowledge => "Acknowledge",
        ExceptionCode::ServerDeviceBusy => "Server Device Busy",
        ExceptionCode::MemoryParityError => "Memory Parity Error",
        ExceptionCode::GatewayPathUnavailable => "Gateway Path Unavailable",
        ExceptionCode::GatewayTargetDevice => "Gateway Target Device Failed to Respond",
        _ => "Unknown exception",
    }
}

/// `"Illegal Data Address (0x02)"`.
fn describe_exception(code: ExceptionCode) -> String {
    format!("{} (0x{:02X})", exception_name(code), u8::from(code))
}

/// Register area a tag lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Area {
    Coil,
    Discrete,
    Input,
    Holding,
}

impl Area {
    fn parse(name: &str) -> Result<Self, ProtocolError> {
        match name {
            "coil" => Ok(Area::Coil),
            "discrete" => Ok(Area::Discrete),
            "input" => Ok(Area::Input),
            "holding" => Ok(Area::Holding),
            other => Err(ProtocolError::InvalidConfig(format!(
                "Unknown register area '{other}' (expected holding, input, coil or discrete)"
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Area::Coil => "coil",
            Area::Discrete => "discrete",
            Area::Input => "input",
            Area::Holding => "holding",
        }
    }

    /// Read function code.
    fn read_fc(self) -> u8 {
        match self {
            Area::Coil => 0x01,
            Area::Discrete => 0x02,
            Area::Holding => 0x03,
            Area::Input => 0x04,
        }
    }

    fn is_bit(self) -> bool {
        matches!(self, Area::Coil | Area::Discrete)
    }

    fn max_per_request(self) -> u32 {
        if self.is_bit() { MAX_READ_BITS } else { MAX_READ_REGISTERS }
    }
}

/// A run of consecutive tags of one area that is fetched with one address range.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadGroup {
    area: Area,
    start: u16,
    /// Exclusive end; `u32` so a range ending at address 65535 does not overflow.
    end: u32,
    first_tag: usize,
    end_tag: usize,
}

/// Group consecutive same-area tags whose address ranges touch or overlap into bulk reads.
fn plan_groups(tags: &[ModbusTag]) -> Result<Vec<ReadGroup>, ProtocolError> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < tags.len() {
        let area = Area::parse(&tags[i].register_area)?;
        let start = tags[i].protocol_address();
        let mut end = start as u32 + tags[i].register_count() as u32;
        let mut next = i + 1;
        while next < tags.len()
            && tags[next].register_area == tags[i].register_area
            && tags[next].protocol_address() >= start
            && tags[next].protocol_address() as u32 <= end
        {
            end = end.max(tags[next].protocol_address() as u32 + tags[next].register_count() as u32);
            next += 1;
        }
        if end > 0x1_0000 {
            return Err(ProtocolError::InvalidConfig(format!(
                "Tag '{}' ({} {}..{}) runs past address 65535",
                tags[i].name,
                area.name(),
                start,
                end - 1
            )));
        }
        groups.push(ReadGroup { area, start, end, first_tag: i, end_tag: next });
        i = next;
    }
    Ok(groups)
}

/// A request that reached the device but did not yield usable data.
#[derive(Debug)]
enum GroupError {
    Exception(ExceptionCode),
    ShortResponse { got: usize, want: usize },
}

impl GroupError {
    fn describe(&self) -> String {
        match self {
            GroupError::Exception(code) => describe_exception(*code),
            GroupError::ShortResponse { got, want } => {
                format!("Short response ({got} of {want} values)")
            }
        }
    }
}

/// A request that never completed.
#[derive(Debug)]
enum IoFailure {
    Timeout(Duration),
    Transport(String),
    Closed,
}

impl IoFailure {
    fn into_read(self, what: &str) -> ProtocolError {
        match self {
            IoFailure::Timeout(d) => ProtocolError::Timeout(format!("{what}: no response within {} ms", d.as_millis())),
            IoFailure::Transport(e) => ProtocolError::ReadFailed(format!("{what}: {e}")),
            IoFailure::Closed => ProtocolError::NotConnected,
        }
    }

    fn into_write(self, what: &str) -> ProtocolError {
        match self {
            IoFailure::Timeout(d) => ProtocolError::Timeout(format!("{what}: no response within {} ms", d.as_millis())),
            IoFailure::Transport(e) => ProtocolError::WriteFailed(format!("{what}: {e}")),
            IoFailure::Closed => ProtocolError::NotConnected,
        }
    }
}

type Reply<T> = Result<Result<T, ExceptionCode>, tokio_modbus::Error>;

/// Modbus TCP/RTU client implementing the [`ProtocolClient`] trait.
///
/// Requests are serialised by the inner async mutex, so a `ModbusClient` can
/// be shared (`Arc`) between a polling loop and user writes. [`disconnect`]
/// first aborts whatever request is in flight, then closes the connection.
///
/// [`disconnect`]: ProtocolClient::disconnect
pub struct ModbusClient {
    ctx: tokio::sync::Mutex<Context>,
    unit_id: AtomicU8,
    endianness: Endianness,
    timeout: Duration,
    traffic_tx: Option<tokio::sync::mpsc::UnboundedSender<TrafficLogEntry>>,
    closed: watch::Sender<bool>,
}

impl ModbusClient {
    fn new(ctx: Context, unit_id: u8, endianness: Endianness, timeout: Duration) -> Self {
        Self {
            ctx: tokio::sync::Mutex::new(ctx),
            unit_id: AtomicU8::new(unit_id),
            endianness,
            timeout,
            traffic_tx: None,
            closed: watch::channel(false).0,
        }
    }

    pub fn set_traffic_channel(&mut self, tx: tokio::sync::mpsc::UnboundedSender<TrafficLogEntry>) {
        self.traffic_tx = Some(tx);
    }

    /// Unit ID requests are currently addressed to.
    pub fn unit_id(&self) -> u8 {
        self.unit_id.load(Ordering::Relaxed)
    }

    /// Address subsequent requests to another unit ID on the same connection.
    pub async fn set_unit_id(&self, unit_id: u8) {
        let mut ctx = self.ctx.lock().await;
        ctx.set_slave(Slave(unit_id));
        self.unit_id.store(unit_id, Ordering::Relaxed);
    }

    fn log_traffic(&self, direction: &str, fc: u8, address: u16, data: &str, length: u16) {
        if let Some(ref tx) = self.traffic_tx {
            let _ = tx.send(TrafficLogEntry {
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                direction: direction.into(),
                function_code: fc,
                address,
                data: data.into(),
                length,
            });
        }
    }

    /// Run one request with a deadline, aborting early if the client is being closed.
    async fn bounded<T>(
        &self,
        timeout: Duration,
        request: impl Future<Output = Reply<T>>,
    ) -> Result<Result<T, ExceptionCode>, IoFailure> {
        let mut closed = self.closed.subscribe();
        tokio::select! {
            biased;
            _ = closed.wait_for(|c| *c) => Err(IoFailure::Closed),
            res = tokio::time::timeout(timeout, request) => match res {
                Ok(Ok(reply)) => Ok(reply),
                Ok(Err(e)) => Err(IoFailure::Transport(e.to_string())),
                Err(_) => Err(IoFailure::Timeout(timeout)),
            },
        }
    }

    /// Read `count` coils/discrete inputs/registers starting at `start`.
    /// Bit areas are returned as one `0`/`1` word per bit.
    async fn read_chunk(
        &self,
        ctx: &mut Context,
        area: Area,
        start: u16,
        count: u16,
        timeout: Duration,
    ) -> Result<Result<Vec<u16>, ExceptionCode>, IoFailure> {
        match area {
            Area::Coil => Ok(self
                .bounded(timeout, ctx.read_coils(start, count))
                .await?
                .map(|bits| bits.into_iter().take(count as usize).map(u16::from).collect())),
            Area::Discrete => Ok(self
                .bounded(timeout, ctx.read_discrete_inputs(start, count))
                .await?
                .map(|bits| bits.into_iter().take(count as usize).map(u16::from).collect())),
            Area::Input => self.bounded(timeout, ctx.read_input_registers(start, count)).await,
            Area::Holding => self.bounded(timeout, ctx.read_holding_registers(start, count)).await,
        }
    }

    /// Read a whole group, fragmented to the protocol's per-request limit.
    async fn read_group(
        &self,
        ctx: &mut Context,
        group: &ReadGroup,
    ) -> Result<Result<Vec<u16>, GroupError>, IoFailure> {
        let total = (group.end - group.start as u32) as usize;
        let mut words = Vec::with_capacity(total);
        let mut cursor = group.start as u32;
        while cursor < group.end {
            let count = (group.end - cursor).min(group.area.max_per_request());
            match self.read_chunk(ctx, group.area, cursor as u16, count as u16, self.timeout).await? {
                Ok(chunk) => {
                    if chunk.len() != count as usize {
                        return Ok(Err(GroupError::ShortResponse { got: chunk.len(), want: count as usize }));
                    }
                    words.extend(chunk);
                    cursor += count;
                }
                Err(code) => return Ok(Err(GroupError::Exception(code))),
            }
        }
        Ok(Ok(words))
    }
}

#[cfg(feature = "serial")]
impl ModbusClient {
    fn serial_builder(serial_config: &RtuSerialConfig, timeout: Duration) -> tokio_serial::SerialPortBuilder {
        let parity = match serial_config.parity {
            crate::types::RtuParity::None => serialport::Parity::None,
            crate::types::RtuParity::Odd => serialport::Parity::Odd,
            crate::types::RtuParity::Even => serialport::Parity::Even,
        };
        let data_bits = match serial_config.data_bits {
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        };
        let stop_bits = match serial_config.stop_bits {
            2 => serialport::StopBits::Two,
            _ => serialport::StopBits::One,
        };
        tokio_serial::new(&serial_config.port_name, serial_config.baud_rate)
            .parity(parity)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .timeout(timeout)
    }

    /// Connect via Modbus RTU over serial.
    pub async fn connect_rtu(
        serial_config: RtuSerialConfig,
        unit_id: u8,
        endianness: Endianness,
        timeout: Duration,
    ) -> Result<Self, ProtocolError> {
        let builder = Self::serial_builder(&serial_config, timeout);
        let stream = tokio_serial::SerialStream::open(&builder)
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Serial open failed: {e}")))?;
        let ctx = tokio_modbus::client::rtu::attach_slave(stream, Slave(unit_id));
        Ok(Self::new(ctx, unit_id, endianness, timeout))
    }

    /// Connect via Modbus RTU using a pre-built `SerialStream` (e.g. virtual PTY).
    #[cfg(unix)]
    pub async fn connect_rtu_from_stream(
        stream: tokio_serial::SerialStream,
        unit_id: u8,
        endianness: Endianness,
        timeout: Duration,
    ) -> Result<Self, ProtocolError> {
        let ctx = tokio_modbus::client::rtu::attach_slave(stream, Slave(unit_id));
        Ok(Self::new(ctx, unit_id, endianness, timeout))
    }

    /// Scan a range of unit IDs by reading one register/bit from each, reusing
    /// the connected context so no second serial port is opened.
    ///
    /// Unit IDs are switched on the shared context; the configured unit ID is
    /// restored before returning, whether the scan completed, was cancelled
    /// (`cancel` / `cancel_notify`, which stop only the scan) or the client
    /// was closed.
    pub async fn scan_bus(
        &self,
        register_address: u16,
        register_area: &str,
        start_id: u8,
        end_id: u8,
        per_id_timeout: Duration,
        cancel: &std::sync::atomic::AtomicBool,
        cancel_notify: &tokio::sync::Notify,
    ) -> Result<Vec<BusScanResult>, ProtocolError> {
        let area = Area::parse(register_area)?;
        let mut ctx = self.ctx.lock().await;
        let result = self
            .scan_ids(&mut ctx, area, register_address, start_id, end_id, per_id_timeout, cancel, cancel_notify)
            .await;
        ctx.set_slave(Slave(self.unit_id()));
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn scan_ids(
        &self,
        ctx: &mut Context,
        area: Area,
        address: u16,
        start_id: u8,
        end_id: u8,
        per_id_timeout: Duration,
        cancel: &std::sync::atomic::AtomicBool,
        cancel_notify: &tokio::sync::Notify,
    ) -> Result<Vec<BusScanResult>, ProtocolError> {
        let mut found = Vec::new();
        for id in start_id..=end_id {
            // Create the notification future before checking the flag so a cancel
            // arriving in between is not lost.
            let notified = cancel_notify.notified();
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            ctx.set_slave(Slave(id));

            let probe = self.read_chunk(ctx, area, address, 1, per_id_timeout);
            let outcome = tokio::select! {
                res = probe => res,
                _ = notified => break,
            };
            match outcome {
                Ok(Ok(words)) => {
                    if let Some(&value) = words.first() {
                        found.push(BusScanResult { unit_id: id, value });
                    }
                }
                // No answer, a transport hiccup or an exception: nothing usable at this ID.
                Ok(Err(_)) | Err(IoFailure::Timeout(_)) | Err(IoFailure::Transport(_)) => {}
                Err(IoFailure::Closed) => return Err(ProtocolError::NotConnected),
            }
        }
        Ok(found)
    }
}

impl ProtocolClient for ModbusClient {
    type Config = ModbusConfig;
    type Tag = ModbusTag;
    type Value = ModbusValue;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let addr = format!("{}:{}", config.ip, config.port);
        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address '{addr}': {e}")))?;
        let timeout = Duration::from_millis(config.timeout_ms.max(1));

        let ctx = tokio::time::timeout(timeout, tcp::connect_slave(socket_addr, Slave(config.unit_id)))
            .await
            .map_err(|_| {
                ProtocolError::Timeout(format!("Connecting to {addr} timed out after {} ms", timeout.as_millis()))
            })?
            .map_err(|e| ProtocolError::ConnectionFailed(e.to_string()))?;

        Ok(Self::new(ctx, config.unit_id, config.endianness, timeout))
    }

    /// Read tags. Transport failures (timeout, I/O error, closed client) fail
    /// the whole call. When the device answers a request with a Modbus
    /// exception, the tags it covered come back as [`ModbusValue::Failed`] so
    /// the other groups are still delivered; if every group failed that way the
    /// call fails with the exception descriptions.
    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }
        let groups = plan_groups(tags)?;
        let mut ctx = self.ctx.lock().await;
        let mut results = Vec::with_capacity(tags.len());
        let mut failures: Vec<String> = Vec::new();

        for group in &groups {
            let span = format!("{} {}..{}", group.area.name(), group.start, group.end - 1);
            let fc = group.area.read_fc();
            let total = group.end - group.start as u32;
            self.log_traffic("TX", fc, group.start, &format!("count={total}"), total as u16);

            match self.read_group(&mut ctx, group).await {
                Err(failure) => {
                    self.log_traffic("RX", fc, group.start, &format!("{failure:?}"), 0);
                    return Err(failure.into_read(&format!("Read {span}")));
                }
                Ok(Ok(words)) => {
                    self.log_traffic(
                        "RX",
                        fc,
                        group.start,
                        &format!("{:?}", words.iter().take(8).map(|w| format!("0x{w:04X}")).collect::<Vec<_>>()),
                        words.len() as u16,
                    );
                    for tag in &tags[group.first_tag..group.end_tag] {
                        let offset = (tag.protocol_address() - group.start) as usize;
                        let count = tag.register_count() as usize;
                        let raw = if tag.is_bit_area() {
                            words[offset] as f64
                        } else {
                            parse_register(&words[offset..offset + count], &tag.reg_type, &self.endianness)
                        };
                        results.push(ModbusValue::Float64(raw * tag.scale));
                    }
                }
                Ok(Err(error)) => {
                    let message = format!("{} reading {span}", error.describe());
                    // Exception responses set the high bit of the function code (§7).
                    self.log_traffic("RX", fc | 0x80, group.start, &message, 0);
                    for _ in group.first_tag..group.end_tag {
                        results.push(ModbusValue::Failed(message.clone()));
                    }
                    failures.push(message);
                }
            }
        }

        if failures.len() == groups.len() {
            return Err(ProtocolError::ReadFailed(failures.join("; ")));
        }
        Ok(results)
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let area = Area::parse(&tag.register_area)?;
        if matches!(area, Area::Input | Area::Discrete) {
            return Err(ProtocolError::WriteFailed(format!(
                "'{}' is in the {} area, which is read-only",
                tag.name,
                area.name()
            )));
        }
        let addr = tag.protocol_address();
        let raw = value.as_f64();
        let what = format!("Write '{}'", tag.name);

        let mut ctx = self.ctx.lock().await;

        if area == Area::Coil {
            let on = raw != 0.0;
            self.log_traffic("TX", 5, addr, &format!("coil={}", if on { "ON" } else { "OFF" }), 1);
            let single = self.bounded(self.timeout, ctx.write_single_coil(addr, on)).await.map_err(|f| f.into_write(&what))?;
            let Err(first) = single else { return Ok(()) };

            // The device rejected FC05; some only implement FC15.
            self.log_traffic("RX", 5 | 0x80, addr, &describe_exception(first), 0);
            self.log_traffic("TX", 15, addr, "(retry with FC0F)", 1);
            return self
                .bounded(self.timeout, ctx.write_multiple_coils(addr, &[on]))
                .await
                .map_err(|f| f.into_write(&what))?
                .map_err(|code| ProtocolError::WriteFailed(format!("{what}: device rejected the write: {}", describe_exception(code))));
        }

        check_writable(raw, &tag.reg_type)
            .map_err(|e| ProtocolError::InvalidConfig(format!("{what}: {e}")))?;
        let words = value_to_words(raw, &tag.reg_type, &self.endianness);

        let reply = if let [word] = words[..] {
            // FC06 for single-register writes
            self.log_traffic("TX", 6, addr, &format!("0x{word:04X}"), 1);
            self.bounded(self.timeout, ctx.write_single_register(addr, word)).await
        } else {
            // FC16 for multi-register writes
            self.log_traffic(
                "TX",
                16,
                addr,
                &format!("{:?}", words.iter().map(|w| format!("0x{w:04X}")).collect::<Vec<_>>()),
                words.len() as u16,
            );
            self.bounded(self.timeout, ctx.write_multiple_registers(addr, &words)).await
        };
        reply
            .map_err(|f| f.into_write(&what))?
            .map_err(|code| ProtocolError::WriteFailed(format!("{what}: device rejected the write: {}", describe_exception(code))))
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        // Abort any request in flight so the lock below is released promptly.
        self.closed.send_replace(true);
        let mut ctx = self.ctx.lock().await;
        ctx.disconnect().await.map_err(|e| ProtocolError::Other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::start_slave_server;
    use crate::types::RegisterType;

    fn tag(name: &str, address: u16, reg_type: RegisterType, area: &str) -> ModbusTag {
        ModbusTag {
            name: name.into(),
            address,
            reg_type,
            scale: 1.0,
            register_area: area.into(),
        }
    }

    // ── grouping ──

    #[test]
    fn contiguous_tags_share_one_read() {
        let tags = [
            tag("a", 100, RegisterType::Float32, "holding"),
            tag("b", 102, RegisterType::Float32, "holding"),
            tag("c", 104, RegisterType::Uint16, "holding"),
        ];
        let groups = plan_groups(&tags).unwrap();
        assert_eq!(groups, vec![ReadGroup { area: Area::Holding, start: 100, end: 105, first_tag: 0, end_tag: 3 }]);
    }

    #[test]
    fn gaps_and_area_changes_split_groups() {
        let tags = [
            tag("a", 100, RegisterType::Uint16, "holding"),
            tag("b", 200, RegisterType::Uint16, "holding"),
            tag("c", 200, RegisterType::Uint16, "input"),
        ];
        let groups = plan_groups(&tags).unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[1].start, 200);
        assert_eq!(groups[2].area, Area::Input);
    }

    #[test]
    fn bit_areas_count_one_address_per_tag_whatever_the_type() {
        let tags = [tag("c0", 0, RegisterType::Float64, "coil"), tag("c1", 1, RegisterType::Float64, "coil")];
        let groups = plan_groups(&tags).unwrap();
        assert_eq!(groups, vec![ReadGroup { area: Area::Coil, start: 0, end: 2, first_tag: 0, end_tag: 2 }]);
    }

    #[test]
    fn range_past_65535_is_rejected_not_wrapped() {
        // A float32 at 65535 would need register 65536.
        let tags = [tag("x", 65535, RegisterType::Float32, "holding")];
        assert!(plan_groups(&tags).is_err());
        // A float32 at 65534 ends exactly on the last register.
        let ok = [tag("x", 65534, RegisterType::Float32, "holding")];
        assert_eq!(plan_groups(&ok).unwrap()[0].end, 65536);
    }

    #[test]
    fn unknown_area_is_rejected() {
        assert!(plan_groups(&[tag("x", 0, RegisterType::Uint16, "string")]).is_err());
    }

    // ── against a live slave on loopback ──

    async fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    async fn slave_and_client(
        unit_id: u8,
        endianness: Endianness,
    ) -> (
        std::sync::Arc<std::sync::Mutex<crate::server::SlaveState>>,
        tokio::sync::watch::Sender<bool>,
        ModbusClient,
    ) {
        let port = free_port().await;
        let (state, _log, shutdown) = start_slave_server(port, "127.0.0.1", 64, unit_id).await.unwrap();
        let client = ModbusClient::connect(ModbusConfig {
            ip: "127.0.0.1".into(),
            port,
            unit_id,
            interval_ms: 1000,
            timeout_ms: 2000,
            endianness,
        })
        .await
        .unwrap();
        (state, shutdown, client)
    }

    #[tokio::test]
    async fn reads_one_value_per_tag_with_word_order_applied() {
        let (state, _shutdown, client) = slave_and_client(1, Endianness::Little).await;
        {
            let mut s = state.lock().unwrap();
            // 220.5f32 = 0x435C8000, stored CD AB: 0x8000 0x435C
            s.registers[10] = 0x8000;
            s.registers[11] = 0x435C;
            // pi as float64, GH EF CD AB: 0x2D18 0x5444 0x21FB 0x4009
            s.registers[12..16].copy_from_slice(&[0x2D18, 0x5444, 0x21FB, 0x4009]);
            s.registers[16] = 7;
        }
        let tags = [
            tag("f32", 10, RegisterType::Float32, "holding"),
            tag("f64", 12, RegisterType::Float64, "holding"),
            tag("u16", 16, RegisterType::Uint16, "holding"),
        ];
        let values = client.read(&tags).await.unwrap();
        assert_eq!(values.len(), 3, "exactly one value per tag, not per register");
        assert_eq!(values[0].as_f64(), 220.5);
        assert_eq!(values[1].as_f64(), std::f64::consts::PI);
        assert_eq!(values[2].as_f64(), 7.0);
    }

    #[tokio::test]
    async fn write_uses_the_configured_word_order() {
        let (state, _shutdown, client) = slave_and_client(1, Endianness::Swapped).await;
        client
            .write(&tag("w", 4, RegisterType::Float32, "holding"), ModbusValue::Float64(220.5))
            .await
            .unwrap();
        // 220.5f32 = 0x435C8000 -> BA DC: 0x5C43 0x0080
        let s = state.lock().unwrap();
        assert_eq!(&s.registers[4..6], &[0x5C43, 0x0080]);
    }

    #[tokio::test]
    async fn exception_on_one_group_is_reported_per_tag_and_others_survive() {
        let (state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        state.lock().unwrap().registers[0] = 42;
        // Register 64 is past the slave's 64 registers -> Illegal Data Address (0x02).
        let tags = [
            tag("ok", 0, RegisterType::Uint16, "holding"),
            tag("bad", 64, RegisterType::Uint16, "holding"),
        ];
        let values = client.read(&tags).await.unwrap();
        assert_eq!(values[0].as_f64(), 42.0);
        match &values[1] {
            ModbusValue::Failed(msg) => {
                assert!(msg.contains("Illegal Data Address"), "{msg}");
                assert!(msg.contains("0x02"), "{msg}");
                assert!(msg.contains("holding 64..64"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exception_on_every_group_fails_the_read() {
        let (_state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        let err = client
            .read(&[tag("bad", 64, RegisterType::Uint16, "holding")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Illegal Data Address"), "{err}");
    }

    #[tokio::test]
    async fn unsupported_function_surfaces_as_exception_01() {
        let (_state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        // The test slave only implements FC03/FC06/FC16; FC04 gets Illegal Function (0x01).
        let err = client
            .read(&[tag("in", 0, RegisterType::Uint16, "input")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Illegal Function"), "{err}");
    }

    #[tokio::test]
    async fn dropped_connection_is_an_error_not_nan() {
        let (_state, shutdown, client) = slave_and_client(1, Endianness::Big).await;
        let tags = [tag("a", 0, RegisterType::Uint16, "holding")];
        assert!(client.read(&tags).await.is_ok());
        shutdown.send(true).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let err = client.read(&tags).await.unwrap_err();
        assert!(
            matches!(err, ProtocolError::ReadFailed(_) | ProtocolError::Timeout(_)),
            "expected a transport error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn writes_to_read_only_areas_are_refused_before_hitting_the_wire() {
        let (state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        for area in ["input", "discrete"] {
            let err = client
                .write(&tag("ro", 9, RegisterType::Uint16, area), ModbusValue::Float64(5.0))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("read-only"), "{err}");
        }
        assert_eq!(state.lock().unwrap().registers[9], 0, "holding register 9 must be untouched");
    }

    #[tokio::test]
    async fn unrepresentable_writes_are_refused() {
        let (state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        let t = tag("w", 3, RegisterType::Uint16, "holding");
        assert!(client.write(&t, ModbusValue::Float64(3.5)).await.is_err());
        assert!(client.write(&t, ModbusValue::Float64(70000.0)).await.is_err());
        assert_eq!(state.lock().unwrap().registers[3], 0);
        client.write(&t, ModbusValue::Float64(65535.0)).await.unwrap();
        assert_eq!(state.lock().unwrap().registers[3], 65535);
    }

    #[tokio::test]
    async fn write_exception_is_reported() {
        let (_state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        let err = client
            .write(&tag("w", 64, RegisterType::Uint16, "holding"), ModbusValue::Float64(1.0))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Illegal Data Address"), "{err}");
    }

    #[tokio::test]
    async fn concurrent_reads_and_writes_are_serialised_not_rejected() {
        let (_state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        let client = std::sync::Arc::new(client);
        let mut tasks = Vec::new();
        for i in 0..20u16 {
            let c = client.clone();
            tasks.push(tokio::spawn(async move {
                if i % 2 == 0 {
                    c.read(&[tag("r", 0, RegisterType::Uint16, "holding")]).await.map(|_| ())
                } else {
                    c.write(&tag("w", i, RegisterType::Uint16, "holding"), ModbusValue::Float64(i as f64)).await
                }
            }));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn disconnect_aborts_in_flight_work_and_rejects_later_calls() {
        let (_state, _shutdown, client) = slave_and_client(1, Endianness::Big).await;
        client.disconnect().await.ok();
        let err = client.read(&[tag("a", 0, RegisterType::Uint16, "holding")]).await.unwrap_err();
        assert!(matches!(err, ProtocolError::NotConnected), "{err:?}");
    }

    #[tokio::test]
    async fn connect_to_unroutable_address_times_out() {
        // 192.0.2.0/24 (TEST-NET-1) never answers; the connect must give up on its own.
        let started = std::time::Instant::now();
        let res = ModbusClient::connect(ModbusConfig {
            ip: "192.0.2.1".into(),
            port: 502,
            unit_id: 1,
            interval_ms: 1000,
            timeout_ms: 300,
            endianness: Endianness::Big,
        })
        .await;
        assert!(res.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(feature = "serial")]
    #[tokio::test]
    async fn bus_scan_restores_the_configured_unit_id() {
        // Slave answers only unit 2. Scan 1..=3 leaves the shared context on unit 3;
        // the client must be back on unit 2 afterwards.
        let (state, _shutdown, client) = slave_and_client(2, Endianness::Big).await;
        state.lock().unwrap().registers[0] = 0x1234;

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let notify = tokio::sync::Notify::new();
        let found = client
            .scan_bus(0, "holding", 1, 3, Duration::from_millis(500), &cancel, &notify)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].unit_id, 2);
        assert_eq!(found[0].value, 0x1234);

        let values = client.read(&[tag("a", 0, RegisterType::Uint16, "holding")]).await.unwrap();
        assert_eq!(values[0].as_f64(), 4660.0);
    }

    #[cfg(feature = "serial")]
    #[tokio::test]
    async fn cancelled_bus_scan_restores_unit_and_keeps_the_client_usable() {
        let (_state, _shutdown, client) = slave_and_client(2, Endianness::Big).await;
        let cancel = std::sync::atomic::AtomicBool::new(true); // cancelled before the first probe
        let notify = tokio::sync::Notify::new();
        let found = client
            .scan_bus(0, "holding", 1, 247, Duration::from_millis(500), &cancel, &notify)
            .await
            .unwrap();
        assert!(found.is_empty());
        client.read(&[tag("a", 0, RegisterType::Uint16, "holding")]).await.unwrap();
    }

    #[tokio::test]
    async fn set_unit_id_retargets_requests() {
        let (_state, _shutdown, client) = slave_and_client(5, Endianness::Big).await;
        client.set_unit_id(6).await;
        assert_eq!(client.unit_id(), 6);
        // The TCP slave answers unit 6 with Gateway Target Device Failed to Respond (0x0B).
        let err = client.read(&[tag("a", 0, RegisterType::Uint16, "holding")]).await.unwrap_err();
        assert!(err.to_string().contains("Gateway Target Device"), "{err}");
        client.set_unit_id(5).await;
        client.read(&[tag("a", 0, RegisterType::Uint16, "holding")]).await.unwrap();
    }
}
