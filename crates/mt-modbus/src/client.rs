use std::net::SocketAddr;

use mt_protocol::{ProtocolClient, ProtocolError};
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

use crate::types::{ModbusConfig, ModbusTag, ModbusValue, TrafficLogEntry, parse_register};
#[cfg(feature = "serial")]
use crate::types::{BusScanResult, RtuSerialConfig};

const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Modbus TCP/RTU client implementing the [`ProtocolClient`] trait.
///
/// The inner `tokio_modbus::client::Context` is wrapped in an async Mutex so
/// that `ModbusClient` satisfies `Sync` (required by `ProtocolClient`).
pub struct ModbusClient {
    ctx: tokio::sync::Mutex<tokio_modbus::client::Context>,
    endianness: crate::types::Endianness,
    timeout: std::time::Duration,
    traffic_tx: Option<tokio::sync::mpsc::UnboundedSender<TrafficLogEntry>>,
}

impl ModbusClient {
    pub fn set_traffic_channel(&mut self, tx: tokio::sync::mpsc::UnboundedSender<TrafficLogEntry>) {
        self.traffic_tx = Some(tx);
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
}

impl ModbusClient {
    /// Connect via Modbus RTU over serial.
    #[cfg(feature = "serial")]
    pub async fn connect_rtu(
        serial_config: RtuSerialConfig,
        unit_id: u8,
        endianness: crate::types::Endianness,
        timeout: std::time::Duration,
    ) -> Result<Self, ProtocolError> {
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

        let builder = tokio_serial::new(&serial_config.port_name, serial_config.baud_rate)
            .parity(parity)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .timeout(timeout);

        let stream = tokio_serial::SerialStream::open(&builder)
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Serial open failed: {e}")))?;

        let ctx = tokio_modbus::client::rtu::attach_slave(stream, Slave(unit_id));

        Ok(Self {
            ctx: tokio::sync::Mutex::new(ctx),
            endianness,
            timeout,
            traffic_tx: None,
        })
    }

    /// Connect via Modbus RTU using a pre-built `SerialStream` (e.g. virtual PTY).
    #[cfg(all(feature = "serial", unix))]
    pub async fn connect_rtu_from_stream(
        stream: tokio_serial::SerialStream,
        unit_id: u8,
        endianness: crate::types::Endianness,
        timeout: std::time::Duration,
    ) -> Result<Self, ProtocolError> {
        let ctx = tokio_modbus::client::rtu::attach_slave(stream, Slave(unit_id));
        Ok(Self {
            ctx: tokio::sync::Mutex::new(ctx),
            endianness,
            timeout,
            traffic_tx: None,
        })
    }

    /// Scan a range of Modbus RTU unit IDs by reading a single register from each.
    /// Uses `rtu::attach` (no fixed slave) so each request targets a different unit ID.
    #[cfg(feature = "serial")]
    pub async fn scan_bus_rtu(
        serial_config: RtuSerialConfig,
        register_address: u16,
        register_area: &str,
        start_id: u8,
        end_id: u8,
        per_id_timeout: std::time::Duration,
    ) -> Result<Vec<BusScanResult>, ProtocolError> {
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

        let builder = tokio_serial::new(&serial_config.port_name, serial_config.baud_rate)
            .parity(parity)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .timeout(per_id_timeout);

        let stream = tokio_serial::SerialStream::open(&builder)
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Serial open failed: {e}")))?;

        // Use attach (not attach_slave) so we can set slave per-request
        let mut ctx = tokio_modbus::client::rtu::attach(stream);

        let mut found = Vec::new();
        let is_coil = register_area == "coil";
        let is_discrete = register_area == "discrete";
        let is_input = register_area == "input";

        for id in start_id..=end_id {
            ctx.set_slave(Slave(id));

            let scan_ok = if is_coil {
                tokio::time::timeout(per_id_timeout, ctx.read_coils(register_address, 1))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .and_then(|r| r.ok())
                    .map(|bits| bits.first().map_or(0, |b| if *b { 1 } else { 0 }))
            } else if is_discrete {
                tokio::time::timeout(per_id_timeout, ctx.read_discrete_inputs(register_address, 1))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .and_then(|r| r.ok())
                    .map(|bits| bits.first().map_or(0, |b| if *b { 1 } else { 0 }))
            } else if is_input {
                tokio::time::timeout(per_id_timeout, ctx.read_input_registers(register_address, 1))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .and_then(|r| r.ok())
                    .map(|words| words.first().copied().unwrap_or(0))
            } else {
                tokio::time::timeout(per_id_timeout, ctx.read_holding_registers(register_address, 1))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .and_then(|r| r.ok())
                    .map(|words| words.first().copied().unwrap_or(0))
            };

            if let Some(value) = scan_ok {
                found.push(BusScanResult { unit_id: id, value });
            }
        }

        Ok(found)
    }

    /// Scan bus using the existing connected context.
    /// Reuses `self.ctx` so no second serial port is opened.
    /// Uses `Notify` for instant cancellation — no 1s delay on stop.
    #[cfg(feature = "serial")]
    pub async fn scan_bus(
        &self,
        register_address: u16,
        register_area: &str,
        start_id: u8,
        end_id: u8,
        per_id_timeout: std::time::Duration,
        cancel: &std::sync::atomic::AtomicBool,
        cancel_notify: &tokio::sync::Notify,
    ) -> Result<Vec<BusScanResult>, ProtocolError> {
        let mut ctx = self.ctx.lock().await;

        let mut found = Vec::new();
        let is_coil = register_area == "coil";
        let is_discrete = register_area == "discrete";
        let is_input = register_area == "input";

        for id in start_id..=end_id {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            ctx.set_slave(Slave(id));

            // Race the modbus read against cancel notification for instant stop
            let notified = cancel_notify.notified();

            let scan_ok = if is_coil {
                tokio::select! {
                    result = tokio::time::timeout(per_id_timeout, ctx.read_coils(register_address, 1)) => {
                        result.ok()
                            .and_then(|r| r.ok())
                            .and_then(|r| r.ok())
                            .map(|bits| bits.first().map_or(0, |b| if *b { 1 } else { 0 }))
                    }
                    _ = notified => None,
                }
            } else if is_discrete {
                tokio::select! {
                    result = tokio::time::timeout(per_id_timeout, ctx.read_discrete_inputs(register_address, 1)) => {
                        result.ok()
                            .and_then(|r| r.ok())
                            .and_then(|r| r.ok())
                            .map(|bits| bits.first().map_or(0, |b| if *b { 1 } else { 0 }))
                    }
                    _ = notified => None,
                }
            } else if is_input {
                tokio::select! {
                    result = tokio::time::timeout(per_id_timeout, ctx.read_input_registers(register_address, 1)) => {
                        result.ok()
                            .and_then(|r| r.ok())
                            .and_then(|r| r.ok())
                            .map(|words| words.first().copied().unwrap_or(0))
                    }
                    _ = notified => None,
                }
            } else {
                tokio::select! {
                    result = tokio::time::timeout(per_id_timeout, ctx.read_holding_registers(register_address, 1)) => {
                        result.ok()
                            .and_then(|r| r.ok())
                            .and_then(|r| r.ok())
                            .map(|words| words.first().copied().unwrap_or(0))
                    }
                    _ = notified => None,
                }
            };

            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            if let Some(value) = scan_ok {
                found.push(BusScanResult { unit_id: id, value });
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

        let ctx = tcp::connect_slave(socket_addr, Slave(config.unit_id))
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("{e}")))?;

        Ok(Self {
            ctx: tokio::sync::Mutex::new(ctx),
            endianness: config.endianness,
            timeout: DEFAULT_TIMEOUT,
            traffic_tx: None,
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut ctx = self.ctx.lock().await;
        let mut results = Vec::with_capacity(tags.len());
        if tags.is_empty() {
            return Ok(results);
        }

        // Group contiguous same-area tags into bulk reads
        let mut i = 0;
        while i < tags.len() {
            let first = &tags[i];
            let area = &first.register_area;
            let start_addr = first.protocol_address();
            let mut end_addr = start_addr + first.register_count();

            let mut group_end = i + 1;
            while group_end < tags.len()
                && tags[group_end].register_area == *area
                && tags[group_end].protocol_address() >= start_addr
                && tags[group_end].protocol_address() <= end_addr
            {
                let t = &tags[group_end];
                let t_end = t.protocol_address() + t.register_count();
                if t_end > end_addr {
                    end_addr = t_end;
                }
                group_end += 1;
            }

            let total = (end_addr - start_addr) as usize;
            let is_coil = area == "coil";
            let is_discrete = area == "discrete";
            let is_input = area == "input";
            let fc = if is_coil { 1 } else if is_discrete { 2 } else if is_input { 4 } else { 3 };

            self.log_traffic("TX", fc, start_addr, &format!("count={}", total), total as u16);

            // Reads are fragmented to protocol PDU limits
            // (125 registers / 2000 bits per request) so large blocks work
            // against real devices.
            let max_chunk: usize = if is_coil || is_discrete { 2000 } else { 125 };
            let mut words: Vec<u16> = Vec::with_capacity(total);
            let mut cur = start_addr;
            let mut chunk_failed = false;
            while cur < end_addr {
                let count = (end_addr - cur).min(max_chunk as u16) as u16;
                let read_result = if is_coil || is_discrete {
                    let bits = if is_coil {
                        tokio::time::timeout(self.timeout, ctx.read_coils(cur, count)).await
                    } else {
                        tokio::time::timeout(self.timeout, ctx.read_discrete_inputs(cur, count))
                            .await
                    };
                    match bits {
                        Ok(Ok(Ok(b))) => Ok(b.into_iter().map(|c| if c { 1u16 } else { 0u16 }).collect::<Vec<u16>>()),
                        _ => Err(()),
                    }
                } else if is_input {
                    match tokio::time::timeout(self.timeout, ctx.read_input_registers(cur, count)).await {
                        Ok(Ok(Ok(w))) => Ok(w),
                        _ => Err(()),
                    }
                } else {
                    match tokio::time::timeout(self.timeout, ctx.read_holding_registers(cur, count)).await {
                        Ok(Ok(Ok(w))) => Ok(w),
                        _ => Err(()),
                    }
                };
                match read_result {
                    Ok(mut chunk) => {
                        words.append(&mut chunk);
                        cur += count;
                    }
                    Err(()) => {
                        chunk_failed = true;
                        break;
                    }
                }
            }

            self.log_traffic(
                "RX",
                fc,
                start_addr,
                &format!(
                    "{:?}",
                    &words.iter().take(8).map(|w| format!("0x{:04X}", w)).collect::<Vec<_>>()
                ),
                words.len() as u16,
            );

            if chunk_failed {
                for _ in i..group_end {
                    results.push(ModbusValue::Float64(f64::NAN));
                }
                i = group_end;
                continue;
            }

            // Parse each tag from the bulk response
            for j in i..group_end {
                let tag = &tags[j];
                let offset = (tag.protocol_address() - start_addr) as usize;
                let count = tag.register_count() as usize;
                if offset + count > words.len() {
                    results.push(ModbusValue::Float64(f64::NAN));
                    continue;
                }
                let raw = parse_register(&words[offset..offset + count], &tag.reg_type, &self.endianness);
                let scaled = raw * tag.scale;
                results.push(ModbusValue::Float64(scaled));
            }

            i = group_end;
        }

        Ok(results)
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let mut ctx = self.ctx.lock().await;
        let addr = tag.protocol_address();
        let raw = value.as_f64();

        if tag.register_area == "coil" {
            let coil_on = raw != 0.0;
            self.log_traffic("TX", 5, addr, &format!("coil={}", if coil_on { "ON" } else { "OFF" }), 1);

            // Try FC05 (Write Single Coil) first, fall back to FC0F (Write Multiple Coils)
            let result = tokio::time::timeout(
                self.timeout,
                ctx.write_single_coil(addr, coil_on),
            ).await;

            match result {
                Ok(Ok(Ok(()))) => return Ok(()),
                _ => {
                    self.log_traffic("TX", 15, addr, "(retry with FC0F)", 1);
                    tokio::time::timeout(
                        self.timeout,
                        ctx.write_multiple_coils(addr, &[coil_on]),
                    )
                    .await
                    .map_err(|_| ProtocolError::Timeout(format!("Write timeout for tag '{}'", tag.name)))?
                    .map_err(|e| ProtocolError::WriteFailed(format!("{e}")))?
                    .map_err(|code| ProtocolError::WriteFailed(format!(
                        "Device rejected write ({code:?}) — device may not support writes"
                    )))?;
                    return Ok(());
                }
            }
        }

        let words = crate::types::value_to_words(raw, &tag.reg_type, &self.endianness);

        if words.len() == 1 {
            // Use FC06 (Write Single Register) for single-register writes
            self.log_traffic("TX", 6, addr,
                &format!("0x{:04X}", words[0]), 1);

            tokio::time::timeout(
                self.timeout,
                ctx.write_single_register(addr, words[0]),
            )
            .await
            .map_err(|_| ProtocolError::Timeout(format!("Write timeout for tag '{}'", tag.name)))?
            .map_err(|e| ProtocolError::WriteFailed(format!("{e}")))?
            .map_err(|code| ProtocolError::WriteFailed(format!(
                "Device rejected write ({code:?}) — device may not support writes"
            )))?;
        } else {
            // Use FC16 (Write Multiple Registers) for multi-register writes
            self.log_traffic("TX", 16, addr,
                &format!("{:?}", words.iter().map(|w| format!("0x{:04X}", w)).collect::<Vec<_>>()),
                words.len() as u16);

            tokio::time::timeout(
                self.timeout,
                ctx.write_multiple_registers(addr, &words),
            )
            .await
            .map_err(|_| ProtocolError::Timeout(format!("Write timeout for tag '{}'", tag.name)))?
            .map_err(|e| ProtocolError::WriteFailed(format!("{e}")))?
            .map_err(|code| ProtocolError::WriteFailed(format!(
                "Device rejected write ({code:?}) — device may not support writes"
            )))?;
        }

        Ok(())
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        // tokio-modbus Context drops the connection on drop.
        // We lock and explicitly drop to close the connection now.
        let mut ctx = self.ctx.lock().await;
        ctx.disconnect().await.map_err(|e| ProtocolError::Other(format!("{e}")))?;
        Ok(())
    }
}

/// Perform a single read of multiple tags from a Modbus device without keeping
/// a persistent client. Useful for one-shot reads.
pub async fn read_tags(
    config: &ModbusConfig,
    tags: &[ModbusTag],
) -> Result<Vec<ModbusValue>, ProtocolError> {
    let client = ModbusClient::connect(config.clone()).await?;
    let values = client.read(tags).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single write to a Modbus device without keeping a persistent client.
pub async fn write_tag(
    config: &ModbusConfig,
    tag: &ModbusTag,
    value: ModbusValue,
) -> Result<(), ProtocolError> {
    let client = ModbusClient::connect(config.clone()).await?;
    client.write(tag, value).await?;
    client.disconnect().await?;
    Ok(())
}
