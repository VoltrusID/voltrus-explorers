use std::net::SocketAddr;
use std::time::Duration;

use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

use crate::types::{ModbusConfig, ModbusTag, ModbusValue, WriteResult, value_to_words};

/// Write a value to a single Modbus register tag on a device described by `config`.
///
/// This is a one-shot operation that opens a new connection, writes the value,
/// and returns a [`WriteResult`] with latency and status information.
pub async fn write_tag(
    config: &ModbusConfig,
    tag: &ModbusTag,
    value: ModbusValue,
) -> WriteResult {
    let start = std::time::Instant::now();
    let addr = format!("{}:{}", config.ip, config.port);
    let socket_addr: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Invalid address: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let connect_result = tokio::time::timeout(
        Duration::from_secs(5),
        tcp::connect_slave(socket_addr, Slave(config.unit_id)),
    )
    .await;

    let mut ctx = match connect_result {
        Ok(Ok(ctx)) => ctx,
        Ok(Err(e)) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Modbus connection failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
        Err(_) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some("Modbus connect timeout (5s)".into()),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let protocol_addr = tag.protocol_address();
    let raw = value.as_f64();

    // Determine write strategy based on register type
    let write_result = match tag.reg_type {
        crate::types::RegisterType::Uint16 | crate::types::RegisterType::Int16 => {
            let word = raw as u16;
            tokio::time::timeout(
                Duration::from_secs(5),
                ctx.write_single_register(protocol_addr, word),
            )
            .await
        }
        _ => {
            let words = value_to_words(raw, &tag.reg_type, &config.endianness);
            tokio::time::timeout(
                Duration::from_secs(5),
                ctx.write_multiple_registers(protocol_addr, &words),
            )
            .await
        }
    };

    match write_result {
        Ok(Ok(_)) => WriteResult {
            success: true,
            device_response: Some(format!(
                "Wrote register {} = {} (addr: {protocol_addr})",
                tag.name, raw
            )),
            error: None,
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(Err(e)) => WriteResult {
            success: false,
            device_response: None,
            error: Some(format!("Modbus write failed: {e}")),
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Err(_) => WriteResult {
            success: false,
            device_response: None,
            error: Some("Modbus write timeout (5s)".into()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}
