//! Polls a Modbus device and writes to it.

use std::time::{Duration, Instant};

use mt_modbus::{ModbusClient, ModbusConfig, ModbusTag, ModbusValue, RtuSerialConfig};
use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use vt_protocol::{
    Area, LogLevel, ModbusLink, ModbusMasterSpec, PointInfo, Quality, Reading, SessionState, Stats,
    ValueFrame,
};

use super::{endianness, parity, register_type, traffic_frame};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// Fastest poll the engine runs, whatever the spec asks for.
const MIN_POLL: Duration = Duration::from_millis(20);
/// Timeouts in a row that make the master drop the link and reconnect.
const TIMEOUTS_BEFORE_RECONNECT: u32 = 3;
const MAX_BACKOFF: Duration = Duration::from_secs(10);

pub(crate) async fn run(
    spec: ModbusMasterSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let (points, tags) = layout(&spec);
    out.points(points);
    let poll = Duration::from_millis(spec.poll_ms).max(MIN_POLL);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let (traffic_tx, mut traffic_rx) = mpsc::unbounded_channel();
        let connected = tokio::select! {
            result = connect(&spec) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        let client = match connected {
            Ok(mut client) => {
                client.set_traffic_channel(traffic_tx);
                client
            }
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                if !retry(&mut control, attempt, &error.to_string(), out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        out.state(SessionState::Connected);
        out.log(LogLevel::Info, format!("Connected to {}", spec.link));

        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut timeouts = 0;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let started = Instant::now();
                    let mut deferred = Vec::new();
                    let Some(result) = read_or_close(&client, &tags, &mut control, &mut deferred).await else {
                        let _ = client.disconnect().await;
                        break 'session;
                    };
                    let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
                    stats.polls += 1;
                    stats.last_latency_us = latency_us;
                    let mut frames = Vec::new();
                    while let Ok(entry) = traffic_rx.try_recv() {
                        frames.push(traffic_frame(entry));
                    }
                    stats.frames += frames.len() as u64;
                    out.traffic(frames);
                    match result {
                        Ok(values) => {
                            timeouts = 0;
                            out.values(frame(&spec, &tags, &values, latency_us));
                        }
                        Err(error) => {
                            stats.errors += 1;
                            out.values(ValueFrame {
                                at_ms: now_ms(),
                                latency_us,
                                readings: vec![Reading::bad(); tags.len()],
                            });
                            let lost = match &error {
                                ProtocolError::Timeout(_) => {
                                    timeouts += 1;
                                    timeouts >= TIMEOUTS_BEFORE_RECONNECT
                                }
                                ProtocolError::ConnectionFailed(_) | ProtocolError::NotConnected => true,
                                _ => false,
                            };
                            out.log(if lost { LogLevel::Error } else { LogLevel::Warn }, error.to_string());
                            if lost {
                                for (point, _) in deferred {
                                    out.written(point, Err("Not connected".into()));
                                }
                                out.stats(stats);
                                let _ = client.disconnect().await;
                                attempt = 1;
                                if !retry(&mut control, attempt, &error.to_string(), out).await {
                                    break 'session;
                                }
                                continue 'session;
                            }
                        }
                    }
                    for (point, value) in deferred {
                        write_and_report(&client, &tags, point, value, out).await;
                    }
                    out.stats(stats);
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        let _ = client.disconnect().await;
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => {
                        write_and_report(&client, &tags, point, value, out).await;
                    }
                    // Masters serve nothing, so there is nothing to hold.
                    Some(_) => {}
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Report a lost link and wait before the next attempt. Returns `false` when
/// the app closed the session while waiting.
async fn retry(
    control: &mut mpsc::UnboundedReceiver<Control>,
    attempt: u32,
    reason: &str,
    out: &Emitter,
) -> bool {
    let wait = backoff(attempt);
    out.state(SessionState::Reconnecting {
        attempt,
        retry_in_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
        reason: reason.to_owned(),
    });
    tokio::select! {
        () = tokio::time::sleep(wait) => true,
        () = until_closed(control, out) => false,
    }
}

/// Wait for the app to close the session while there is no link, refusing
/// writes in the meantime.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("Not connected".into()));
            }
            Some(_) => {}
        }
    }
}

/// Read every tag, unless the app closes the session first (`None`). Writes
/// that arrive meanwhile are kept in `deferred` for after the read.
async fn read_or_close(
    client: &ModbusClient,
    tags: &[ModbusTag],
    control: &mut mpsc::UnboundedReceiver<Control>,
    deferred: &mut Vec<(u32, f64)>,
) -> Option<Result<Vec<ModbusValue>, ProtocolError>> {
    let read = client.read(tags);
    tokio::pin!(read);
    loop {
        tokio::select! {
            result = &mut read => return Some(result),
            command = control.recv() => match command {
                None | Some(Control::Close) => return None,
                Some(Control::Write { point, value }) => deferred.push((point, value)),
                // Masters serve nothing, so there is nothing to hold.
                Some(_) => {}
            },
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF)
}

async fn connect(spec: &ModbusMasterSpec) -> Result<ModbusClient, ProtocolError> {
    let order = endianness(spec.word_order);
    let timeout = Duration::from_millis(spec.timeout_ms.max(100));
    match &spec.link {
        ModbusLink::Tcp { host, port } => {
            // The client takes an IP literal; resolve names here so hosts like
            // `plc-1.local` work too.
            let address =
                tokio::time::timeout(timeout, tokio::net::lookup_host((host.as_str(), *port)))
                    .await
                    .map_err(|_| ProtocolError::Timeout(format!("Resolving {host} timed out")))?
                    .map_err(|error| {
                        ProtocolError::ConnectionFailed(format!("Cannot resolve {host}: {error}"))
                    })?
                    .min_by_key(|address| address.is_ipv6())
                    .ok_or_else(|| {
                        ProtocolError::ConnectionFailed(format!("{host} has no address"))
                    })?;
            let ip = match address.ip() {
                std::net::IpAddr::V4(ip) => ip.to_string(),
                std::net::IpAddr::V6(ip) => format!("[{ip}]"),
            };
            ModbusClient::connect(ModbusConfig {
                ip,
                port: address.port(),
                unit_id: spec.unit_id,
                interval_ms: spec.poll_ms,
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                endianness: order,
            })
            .await
        }
        ModbusLink::Rtu(serial) => {
            ModbusClient::connect_rtu(
                RtuSerialConfig {
                    port_name: serial.port.clone(),
                    baud_rate: serial.baud,
                    data_bits: serial.data_bits,
                    stop_bits: serial.stop_bits,
                    parity: parity(serial.parity),
                },
                spec.unit_id,
                order,
                timeout,
            )
            .await
        }
    }
}

/// One point and one read tag per value of every block, in block order.
pub(crate) fn layout(spec: &ModbusMasterSpec) -> (Vec<PointInfo>, Vec<ModbusTag>) {
    let mut points = Vec::with_capacity(spec.point_count());
    let mut tags = Vec::with_capacity(spec.point_count());
    for block in &spec.blocks {
        for index in 0..block.count {
            let Some(address) = block.address_of(index) else {
                break;
            };
            let id = u32::try_from(points.len()).unwrap_or(u32::MAX);
            let name = if block.count == 1 {
                block.name.clone()
            } else {
                format!("{} {}", block.name, index + 1)
            };
            points.push(PointInfo {
                id,
                name: name.clone(),
                address: block.area.modicon_label(address),
                data_type: if block.area.is_bit() {
                    "Bool".into()
                } else {
                    block.data_type.name().into()
                },
                unit: block.unit.clone(),
                writable: block.area.writable(),
                boolean: block.area.is_bit(),
                states: Vec::new(),
            });
            tags.push(ModbusTag {
                name,
                address,
                reg_type: register_type(block.data_type),
                scale: 1.0,
                register_area: area_name(block.area).into(),
            });
        }
    }
    (points, tags)
}

fn area_name(area: Area) -> &'static str {
    match area {
        Area::Coil => "coil",
        Area::Discrete => "discrete",
        Area::Input => "input",
        Area::Holding => "holding",
    }
}

fn frame(
    spec: &ModbusMasterSpec,
    tags: &[ModbusTag],
    values: &[ModbusValue],
    latency_us: u32,
) -> ValueFrame {
    let order = endianness(spec.word_order);
    let readings = tags
        .iter()
        .zip(values)
        .map(|(tag, value)| match value {
            ModbusValue::Failed(_) => Reading::bad(),
            value => {
                let value = value.as_f64();
                let mut raw = [0u16; 4];
                let raw_len = if tag.is_bit_area() {
                    raw[0] = u16::from(value != 0.0);
                    1
                } else {
                    let words = mt_modbus::value_to_words(value, &tag.reg_type, &order);
                    for (slot, word) in raw.iter_mut().zip(&words) {
                        *slot = *word;
                    }
                    words.len().min(4)
                };
                Reading {
                    value,
                    raw,
                    raw_len: u8::try_from(raw_len).unwrap_or(4),
                    quality: Quality::Good,
                }
            }
        })
        .collect();
    ValueFrame {
        at_ms: now_ms(),
        latency_us,
        readings,
    }
}

async fn write_and_report(
    client: &ModbusClient,
    tags: &[ModbusTag],
    point: u32,
    value: f64,
    out: &Emitter,
) {
    let result = write(client, tags, point, value).await;
    if let Err(error) = &result {
        out.log(LogLevel::Warn, error.clone());
    }
    out.written(point, result);
}

async fn write(
    client: &ModbusClient,
    tags: &[ModbusTag],
    point: u32,
    value: f64,
) -> Result<(), String> {
    let tag = usize::try_from(point)
        .ok()
        .and_then(|index| tags.get(index))
        .ok_or_else(|| format!("No point {point}"))?;
    client
        .write(tag, ModbusValue::Float64(value))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use vt_protocol::{DataType, RegisterBlock, WordOrder};

    use super::*;

    #[test]
    fn layout_numbers_points_by_modicon_address() {
        let spec = ModbusMasterSpec {
            blocks: vec![
                RegisterBlock {
                    name: "Power".into(),
                    area: Area::Holding,
                    start: 10,
                    count: 2,
                    data_type: DataType::F32,
                    unit: "kW".into(),
                },
                RegisterBlock {
                    name: "Run".into(),
                    area: Area::Coil,
                    start: 0,
                    count: 1,
                    data_type: DataType::U16,
                    unit: String::new(),
                },
            ],
            word_order: WordOrder::Big,
            ..ModbusMasterSpec::default()
        };
        let (points, tags) = layout(&spec);
        let addresses: Vec<_> = points.iter().map(|point| point.address.as_str()).collect();
        assert_eq!(addresses, ["40011", "40013", "000001"]);
        assert_eq!(tags[1].address, 12);
        assert_eq!(points[1].unit, "kW");
        assert!(points[2].boolean && points[2].writable);
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(3), Duration::from_secs(2));
        assert_eq!(backoff(40), MAX_BACKOFF);
    }
}
