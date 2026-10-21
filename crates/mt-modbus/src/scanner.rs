use anyhow::{Context, Result};
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

use crate::types::{combine_u16, Endianness, ScannedRegister};

/// Scan a range of Modbus registers, detecting readability, value changes,
/// and likely data types.
pub async fn scan_registers(
    ip: &str,
    port: u16,
    unit_id: u8,
    start_addr: u16,
    end_addr: u16,
    endianness: &Endianness,
    register_area: &str,
    scan_delay_ms: u64,
) -> Result<Vec<ScannedRegister>> {
    let addr = format!("{}:{}", ip, port);
    let socket_addr: std::net::SocketAddr = addr.parse().context("Invalid address")?;
    let mut ctx = tcp::connect_slave(socket_addr, Slave(unit_id))
        .await
        .context("Modbus connection failed")?;

    let total = (end_addr - start_addr + 1) as usize;
    let mut results = vec![
        ScannedRegister {
            address: 0,
            raw_value: None,
            raw_value2: None,
            changed: false,
            readable: false,
            likely_type: "uint16".to_string(),
            interpreted_value: None,
        };
        total
    ];

    for (i, r) in results.iter_mut().enumerate() {
        r.address = start_addr + i as u16;
    }

    let batch_size: u16 = 20;
    let use_input = register_area == "input";
    let use_coils = register_area == "coil";
    let use_discrete = register_area == "discrete";

    // First pass: read in batches
    let mut addr = start_addr;
    while addr <= end_addr {
        let count = std::cmp::min(batch_size, end_addr - addr + 1);
        let read_result = read_batch(&mut ctx, addr, count, use_coils, use_discrete, use_input)
            .await;

        if let Ok(response) = read_result {
            let offset = (addr - start_addr) as usize;
            for (j, &val) in response.iter().enumerate() {
                if offset + j < total {
                    results[offset + j].raw_value = Some(val);
                    results[offset + j].readable = true;
                }
            }
        }
        addr += count;
    }

    // Wait and second pass to detect changes
    tokio::time::sleep(std::time::Duration::from_millis(scan_delay_ms)).await;

    let mut addr = start_addr;
    while addr <= end_addr {
        let count = std::cmp::min(batch_size, end_addr - addr + 1);
        let offset = (addr - start_addr) as usize;
        let has_readable = (offset..std::cmp::min(offset + count as usize, total))
            .any(|i| results[i].readable);

        if has_readable {
            let read_result = read_batch(&mut ctx, addr, count, use_coils, use_discrete, use_input)
                .await;

            if let Ok(response) = read_result {
                for (j, &val) in response.iter().enumerate() {
                    if offset + j < total && results[offset + j].readable {
                        results[offset + j].raw_value2 = Some(val);
                        if results[offset + j].raw_value != Some(val) {
                            results[offset + j].changed = true;
                        }
                    }
                }
            }
        }
        addr += count;
    }

    // Type heuristic: skip for coils/discrete (single-bit values)
    if !use_coils && !use_discrete {
        let mut consumed = vec![false; total];
        for i in 0..total {
            if consumed[i] || !results[i].readable {
                continue;
            }
            if i + 1 < total && results[i + 1].readable {
                let v1 = results[i].raw_value.unwrap_or(0);
                let v2 = results[i + 1].raw_value.unwrap_or(0);
                let raw = combine_u16(v1, v2, endianness);
                let f = f32::from_bits(raw);

                if f.is_finite() && f.abs() > 0.001 && f.abs() < 1_000_000.0 {
                    results[i].likely_type = "float32".to_string();
                    results[i].interpreted_value = Some(f as f64);
                    consumed[i] = true;
                    consumed[i + 1] = true;
                    results[i + 1].likely_type = "float32_lo".to_string();
                    continue;
                }

                let signed = raw as i32;
                if signed.abs() < 1_000_000 {
                    results[i].likely_type = "int32".to_string();
                    results[i].interpreted_value = Some(signed as f64);
                    consumed[i] = true;
                    consumed[i + 1] = true;
                    results[i + 1].likely_type = "int32_lo".to_string();
                    continue;
                }
            }
            consumed[i] = true;
            let v = results[i].raw_value.unwrap_or(0);
            if (v as i16) < 0 {
                results[i].likely_type = "int16".to_string();
                results[i].interpreted_value = Some((v as i16) as f64);
            } else {
                results[i].likely_type = "uint16".to_string();
                results[i].interpreted_value = Some(v as f64);
            }
        }
    } else {
        for r in &mut results {
            if r.readable {
                r.interpreted_value = Some(r.raw_value.unwrap_or(0) as f64);
            }
        }
    }

    // Convert to 5-digit convention and filter out "lo" registers
    let offset: u16 = match register_area {
        "input" => 30001,
        "coil" => 1,
        "discrete" => 10001,
        _ => 40001,
    };
    Ok(results
        .into_iter()
        .filter(|r| !r.likely_type.ends_with("_lo"))
        .map(|mut r| {
            r.address = r.address.wrapping_add(offset);
            r
        })
        .collect())
}

/// Read a batch of registers, normalizing coils/discrete to u16.
async fn read_batch(
    ctx: &mut tokio_modbus::client::Context,
    addr: u16,
    count: u16,
    use_coils: bool,
    use_discrete: bool,
    use_input: bool,
) -> Result<Vec<u16>> {
    let timeout = std::time::Duration::from_secs(5);

    if use_coils {
        let result = tokio::time::timeout(timeout, ctx.read_coils(addr, count))
            .await
            .map(|r| {
                r.map(|r| {
                    r.map(|coils| {
                        coils
                            .iter()
                            .map(|&c| if c { 1u16 } else { 0u16 })
                            .collect::<Vec<u16>>()
                    })
                })
            })?;
        result?.map_err(|code| anyhow::anyhow!("Modbus exception: {code:?}"))
    } else if use_discrete {
        let result = tokio::time::timeout(timeout, ctx.read_discrete_inputs(addr, count))
            .await
            .map(|r| {
                r.map(|r| {
                    r.map(|bits| {
                        bits.iter()
                            .map(|&b| if b { 1u16 } else { 0u16 })
                            .collect::<Vec<u16>>()
                    })
                })
            })?;
        result?.map_err(|code| anyhow::anyhow!("Modbus exception: {code:?}"))
    } else if use_input {
        let result = tokio::time::timeout(timeout, ctx.read_input_registers(addr, count)).await?;
        result?.map_err(|code| anyhow::anyhow!("Modbus exception: {code:?}"))
    } else {
        let result =
            tokio::time::timeout(timeout, ctx.read_holding_registers(addr, count)).await?;
        result?.map_err(|code| anyhow::anyhow!("Modbus exception: {code:?}"))
    }
}
