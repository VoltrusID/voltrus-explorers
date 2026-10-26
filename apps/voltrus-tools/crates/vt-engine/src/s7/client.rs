//! Polls an S7 CPU's tags, writes them, and reads what the CPU says about
//! itself (identity, mode, diagnostic buffer, blocks).

use std::time::{Duration, Instant};

use mt_protocol::{ProtocolClient, ProtocolError};
use mt_s7::{S7Client, S7Config, S7ConnectOptions, S7Frame, S7Tag};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use vt_protocol::{
    LogLevel, PointText, Reading, S7Block, S7ClientSpec, S7CpuInfo, S7DiagEntry, S7PlcReport,
    SessionState, Stats, ValueFrame,
};

use super::{point_info, reading, tag, traffic_frame, value};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// Fastest poll the engine runs, whatever the spec asks for.
const MIN_POLL: Duration = Duration::from_millis(20);
const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// What came in while a request was on the wire, done after it.
enum Deferred {
    Write(u32, f64),
    ReadPlc,
}

pub(crate) async fn run(
    spec: S7ClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let points = spec
        .tags
        .iter()
        .enumerate()
        .map(|(id, tag)| point_info(id, tag, tag.data_type.writable()))
        .collect();
    out.points(points);
    let tags: Vec<S7Tag> = spec
        .tags
        .iter()
        .map(|spec| tag(&spec.name, spec.address, spec.data_type))
        .collect();
    let poll = Duration::from_millis(spec.poll_ms).max(MIN_POLL);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;
    // The texts last reported, so strings are only sent when they change.
    let mut texts: Vec<Option<String>> = vec![None; tags.len()];
    // The tags that failed on their own at the last poll, logged when that changes.
    let mut failing: Vec<String> = Vec::new();

    'session: loop {
        out.state(SessionState::Connecting);
        let (traffic_tx, mut traffic_rx) = mpsc::unbounded_channel::<S7Frame>();
        let connected = tokio::select! {
            result = connect(&spec, traffic_tx) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        let mut frames = Vec::new();
        drain(&mut traffic_rx, &mut frames);
        stats.frames += frames.len() as u64;
        out.traffic(frames);
        let client = match connected {
            Ok(client) => client,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                out.stats(stats);
                let reason = describe(&error);
                out.log(LogLevel::Error, reason.clone());
                if !retry(&mut control, attempt, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        let pdu = client.pdu_size().await.unwrap_or_default();
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            format!(
                "Connected to {} (rack {}, slot {}), PDU {pdu} bytes",
                spec.host, spec.rack, spec.slot
            ),
        );
        texts.fill(None);

        // What the CPU says about itself comes first.
        let mut deferred = vec![Deferred::ReadPlc];
        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            for job in std::mem::take(&mut deferred) {
                let mut more = Vec::new();
                let done = match job {
                    Deferred::Write(point, value) => or_close(
                        write(&client, &spec, &tags, point, value, out),
                        &mut control,
                        &mut more,
                    )
                    .await
                    .map(|()| true),
                    Deferred::ReadPlc => or_close(report(&client), &mut control, &mut more)
                        .await
                        .map(|report| {
                            out.emit_s7_plc(Ok(report));
                            true
                        }),
                };
                if done.is_none() {
                    let _ = client.disconnect().await;
                    break 'session;
                }
                deferred.extend(more);
            }
            let mut frames = Vec::new();
            drain(&mut traffic_rx, &mut frames);
            stats.frames += frames.len() as u64;
            out.traffic(frames);
            if !client.is_connected().await {
                let reason = "The connection to the PLC was lost".to_owned();
                out.log(LogLevel::Error, reason.clone());
                out.stats(stats);
                let _ = client.disconnect().await;
                attempt = 1;
                if !retry(&mut control, attempt, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
            if !deferred.is_empty() {
                continue;
            }
            tokio::select! {
                _ = ticker.tick() => {
                    let started = Instant::now();
                    let mut more = Vec::new();
                    let Some(results) = or_close(poll_tags(&client, &tags), &mut control, &mut more).await else {
                        let _ = client.disconnect().await;
                        break 'session;
                    };
                    deferred.extend(more);
                    let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
                    stats.polls += 1;
                    stats.last_latency_us = latency_us;
                    let mut readings = Vec::with_capacity(tags.len());
                    let mut changed = Vec::new();
                    let mut problems = Vec::new();
                    for (index, (result, spec_tag)) in results.into_iter().zip(&spec.tags).enumerate() {
                        match result {
                            Ok(read) => {
                                let (read, text) = reading(&read, spec_tag.data_type);
                                if let Some(text) = text
                                    && texts[index].as_deref() != Some(text.as_str())
                                {
                                    texts[index] = Some(text.clone());
                                    changed.push(PointText {
                                        point: u32::try_from(index).unwrap_or(u32::MAX),
                                        text,
                                    });
                                }
                                readings.push(read);
                            }
                            Err(problem) => {
                                let address = spec_tag.address.to_string();
                                let problem = problem
                                    .rsplit_once(&format!("{address}: "))
                                    .map_or(problem.as_str(), |(_, rest)| rest);
                                problems.push(format!("{address}: {problem}"));
                                readings.push(Reading::bad());
                            }
                        }
                    }
                    if !problems.is_empty() {
                        stats.errors += 1;
                    }
                    if problems != failing {
                        for problem in &problems {
                            out.log(LogLevel::Warn, problem.clone());
                        }
                        if problems.is_empty() && !failing.is_empty() {
                            out.log(LogLevel::Info, "Every tag reads again");
                        }
                        failing = problems;
                    }
                    if !changed.is_empty() {
                        out.payloads(changed);
                    }
                    out.values(ValueFrame { at_ms: now_ms(), latency_us, readings });
                    out.stats(stats);
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        let _ = client.disconnect().await;
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => deferred.push(Deferred::Write(point, value)),
                    Some(Control::ReadS7Plc) => deferred.push(Deferred::ReadPlc),
                    // Clients hold nothing and publish nothing.
                    Some(_) => {}
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn drain(
    traffic: &mut mpsc::UnboundedReceiver<S7Frame>,
    into: &mut Vec<vt_protocol::TrafficFrame>,
) {
    while let Ok(frame) = traffic.try_recv() {
        into.push(traffic_frame(frame));
    }
}

/// `mt-s7` errors carry their own explanation; drop the error kind's prefix
/// where it adds nothing.
fn describe(error: &ProtocolError) -> String {
    match error {
        ProtocolError::ConnectionFailed(text)
        | ProtocolError::ReadFailed(text)
        | ProtocolError::WriteFailed(text) => text.clone(),
        other => other.to_string(),
    }
}

async fn connect(
    spec: &S7ClientSpec,
    traffic: mpsc::UnboundedSender<S7Frame>,
) -> Result<S7Client, ProtocolError> {
    // The client joins host and port with a colon: an IPv6 address needs brackets.
    let host = spec.host.trim();
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    S7Client::connect_with_options(
        S7Config {
            id: String::new(),
            host,
            port: spec.port,
            rack: spec.rack,
            slot: spec.slot,
            poll_interval_ms: spec.poll_ms,
        },
        S7ConnectOptions {
            pdu_size: spec.pdu_size,
            timeout: Duration::from_millis(spec.timeout_ms.max(100)),
            traffic: Some(traffic),
        },
    )
    .await
}

/// Read every tag in as few requests as the PDU allows. When one tag fails
/// (a DB that is not there, a PLC refusing an area) the request fails as a
/// whole; then each tag is read on its own, so only the failing ones go bad.
async fn poll_tags(client: &S7Client, tags: &[S7Tag]) -> Vec<Result<mt_s7::S7Value, String>> {
    match client.read(tags).await {
        Ok(values) => values.into_iter().map(Ok).collect(),
        Err(error) => {
            if tags.len() == 1 || !client.is_connected().await {
                let text = describe(&error);
                return tags.iter().map(|_| Err(text.clone())).collect();
            }
            let mut results = Vec::with_capacity(tags.len());
            for tag in tags {
                let result = client
                    .read(std::slice::from_ref(tag))
                    .await
                    .map_err(|error| describe(&error))
                    .and_then(|values| {
                        values
                            .into_iter()
                            .next()
                            .ok_or_else(|| "no value".to_owned())
                    });
                results.push(result);
                if !client.is_connected().await {
                    results.resize_with(tags.len(), || {
                        Err("The connection to the PLC was lost".into())
                    });
                    break;
                }
            }
            results
        }
    }
}

async fn write(
    client: &S7Client,
    spec: &S7ClientSpec,
    tags: &[S7Tag],
    point: u32,
    number: f64,
    out: &Emitter,
) {
    let result = async {
        let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
        let (Some(tag_spec), Some(tag)) = (spec.tags.get(index), tags.get(index)) else {
            return Err(format!("No point {point}"));
        };
        let encoded = value(number, tag_spec.data_type, false)?;
        client
            .write(tag, encoded)
            .await
            .map_err(|error| describe(&error))
    }
    .await;
    if let Err(error) = &result {
        out.log(LogLevel::Warn, error.clone());
    }
    out.written(point, result);
}

/// Identity, mode, diagnostic buffer and blocks, each on its own.
async fn report(client: &S7Client) -> S7PlcReport {
    let text = |error: ProtocolError| describe(&error);
    let cpu = client
        .read_cpu_info()
        .await
        .map_err(text)
        .map(|cpu| S7CpuInfo {
            module_type: cpu.module_type,
            order_code: cpu.order_code,
            firmware: cpu.firmware_version,
            serial_number: cpu.serial_number,
            station: cpu.as_name,
            module_name: cpu.module_name,
        });
    let mode = client
        .read_plc_mode()
        .await
        .map_err(text)
        .map(|mode| mode.mode);
    let diagnostics = client.read_diag_buffer().await.map_err(text).map(|buffer| {
        buffer
            .entries
            .into_iter()
            .map(|entry| S7DiagEntry {
                timestamp: entry.timestamp,
                event_id: entry.event_number,
                text: entry.event_text,
            })
            .collect()
    });
    let blocks = client.list_blocks().await.map_err(text).map(|blocks| {
        blocks
            .into_iter()
            .map(|block| S7Block {
                kind: block.block_type,
                number: block.block_number,
                language: block.language,
            })
            .collect()
    });
    S7PlcReport {
        pdu_size: client.pdu_size().await.unwrap_or_default(),
        cpu,
        mode,
        diagnostics,
        blocks,
    }
}

/// Run `work` unless the app closes the session first (`None`). Writes and
/// PLC reads that arrive meanwhile wait in `deferred`.
async fn or_close<T>(
    work: impl Future<Output = T>,
    control: &mut mpsc::UnboundedReceiver<Control>,
    deferred: &mut Vec<Deferred>,
) -> Option<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result = &mut work => return Some(result),
            command = control.recv() => match command {
                None | Some(Control::Close) => return None,
                Some(Control::Write { point, value }) => deferred.push(Deferred::Write(point, value)),
                Some(Control::ReadS7Plc) => deferred.push(Deferred::ReadPlc),
                Some(_) => {}
            },
        }
    }
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
/// writes and PLC reads in the meantime.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("Not connected".into()));
            }
            Some(Control::ReadS7Plc) => out.emit_s7_plc(Err("Not connected".into())),
            Some(_) => {}
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_secs(1));
        assert_eq!(backoff(30), MAX_BACKOFF);
    }

    #[test]
    fn errors_read_without_their_kind() {
        let error = ProtocolError::ConnectionFailed("127.0.0.1:102: Connection refused".into());
        assert_eq!(describe(&error), "127.0.0.1:102: Connection refused");
    }
}
