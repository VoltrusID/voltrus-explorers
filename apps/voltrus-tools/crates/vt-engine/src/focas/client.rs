//! Polls a FANUC controller over FOCAS/Ethernet and writes its macro
//! variables.

use std::time::{Duration, Instant};

use focas_proto::{AlarmQuery, FocasClient, FocasConfig, FocasError, MacroValue, Snapshot};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use vt_protocol::{Direction, FocasClientSpec, LogLevel, SessionState, Stats, ValueFrame};

use super::{axis_labels, client_points, layout, poll_readings, runs};
use crate::Control;
use crate::cnc::{Texts, frame, retry, until_closed};
use crate::emit::{Emitter, now_ms};

/// Fastest poll the engine runs, whatever the spec asks for.
const MIN_POLL: Duration = Duration::from_millis(50);
/// Characters of the executing block asked for.
const BLOCK_CHARS: u16 = 60;

/// A write that came in while a poll was on the wire, done after it.
struct Deferred {
    point: u32,
    value: f64,
}

pub(crate) async fn run(
    spec: FocasClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let poll = Duration::from_millis(spec.poll_ms).max(MIN_POLL);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let config = FocasConfig {
            host: spec.host.clone(),
            port: spec.port,
            connect_timeout_ms: spec.timeout_ms,
            request_timeout_ms: spec.timeout_ms,
        };
        let connected = tokio::select! {
            result = connect(config) => result,
            () = until_closed(&mut control, out, "Not connected") => break 'session,
        };
        let (client, first) = match connected {
            Ok(connected) => connected,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                out.stats(stats);
                let reason = error.to_string();
                out.log(LogLevel::Error, reason.clone());
                if !retry(&mut control, attempt, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        attempt = 0;
        let sysinfo = client.cached_sysinfo();
        let axes = axis_labels(&first, sysinfo.as_ref());
        let points = client_points(&axes, &spec.macros);
        out.points(points.clone());
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            match &sysinfo {
                Some(info) => format!(
                    "Connected to {}:{}: CNC {} {} {} {}, {} axes",
                    spec.host,
                    spec.port,
                    info.cnc_type.trim(),
                    info.mt_type.trim(),
                    info.series.trim(),
                    info.version.trim(),
                    axes.len()
                ),
                None => format!(
                    "Connected to {}:{}; the CNC refused its system info",
                    spec.host, spec.port
                ),
            },
        );
        let mut texts = Texts::new(points.len());
        let mut failing: Vec<String> = Vec::new();
        let mut deferred: Vec<Deferred> = Vec::new();
        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            for job in std::mem::take(&mut deferred) {
                let result = write(&client, &spec, axes.len(), job.point, job.value, out).await;
                if let Err(error) = &result {
                    out.log(LogLevel::Warn, error.clone());
                }
                out.written(job.point, result);
            }
            if !client.is_connected() {
                let reason = "The connection to the CNC was lost".to_owned();
                out.log(LogLevel::Error, reason.clone());
                out.stats(stats);
                let _ = client.disconnect().await;
                if !retry(&mut control, 1, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
            tokio::select! {
                _ = ticker.tick() => {
                    let started = Instant::now();
                    let polled = poll_once(&client, &spec, axes.len(), sysinfo.as_ref()).await;
                    let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
                    stats.polls += 1;
                    stats.frames += 2;
                    stats.last_latency_us = latency_us;
                    let Some(polled) = polled else {
                        stats.errors += 1;
                        out.stats(stats);
                        continue;
                    };
                    if !polled.failed.is_empty() {
                        stats.errors += 1;
                    }
                    if polled.failed != failing {
                        if !polled.failed.is_empty() {
                            out.log(
                                LogLevel::Warn,
                                format!("The CNC refused: {}", polled.failed.join(", ")),
                            );
                        } else if !failing.is_empty() {
                            out.log(LogLevel::Info, "Every item reads again");
                        }
                        failing.clone_from(&polled.failed);
                    }
                    out.traffic(vec![
                        frame(
                            Direction::Tx,
                            "Poll: status, positions, feed, spindle, programs, alarms",
                            format!("{} macro variable(s)", spec.macros.len()),
                        ),
                        frame(
                            Direction::Rx,
                            format!("{} item(s)", polled.readings.len()),
                            polled.summary(),
                        ),
                    ]);
                    let changed = texts.changed(&polled.texts);
                    if !changed.is_empty() {
                        out.payloads(changed);
                    }
                    out.values(ValueFrame {
                        at_ms: now_ms(),
                        latency_us,
                        readings: polled.readings,
                    });
                    out.stats(stats);
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        let _ = client.disconnect().await;
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => deferred.push(Deferred { point, value }),
                    Some(_) => {}
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Connect and take the first snapshot, which tells the axes the points
/// are laid out for.
async fn connect(config: FocasConfig) -> Result<(FocasClient, Snapshot), FocasError> {
    let client = FocasClient::connect(config).await?;
    match client.snapshot().await {
        Ok(snapshot) => Ok((client, snapshot)),
        Err(error) => {
            let _ = client.disconnect().await;
            Err(error)
        }
    }
}

/// One poll: the snapshot, the block being executed, the alarm messages
/// while something is alarmed, and the macro variables. `None` when the
/// link failed during it.
async fn poll_once(
    client: &FocasClient,
    spec: &FocasClientSpec,
    axes: usize,
    sysinfo: Option<&focas_proto::SysInfo>,
) -> Option<super::Polled> {
    let snapshot = client.snapshot().await.ok()?;
    let block = client.executing_block(BLOCK_CHARS).await;
    if !client.is_connected() {
        return None;
    }
    let alarmed = snapshot.alarm_bits.value().is_some_and(|bits| bits.any());
    let messages = if alarmed {
        client
            .alarm_messages(AlarmQuery::default())
            .await
            .ok()
            .map(|messages| {
                messages
                    .iter()
                    .map(|alarm| match &alarm.text {
                        Some(text) => format!(
                            "{} {}: {}",
                            alarm.alarm_type.label(),
                            alarm.number,
                            text.trim()
                        ),
                        None => format!("{} {}", alarm.alarm_type.label(), alarm.number),
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
    } else {
        None
    };
    let mut macros: Vec<(u32, Option<f64>)> =
        spec.macros.iter().map(|number| (*number, None)).collect();
    for (first, last) in runs(&spec.macros) {
        let Ok(values) = client.read_macro(first, last).await else {
            if !client.is_connected() {
                return None;
            }
            continue;
        };
        for MacroValue { number, value } in values {
            for slot in macros.iter_mut().filter(|slot| slot.0 == number) {
                slot.1 = value;
            }
        }
    }
    Some(poll_readings(
        &snapshot,
        sysinfo,
        block.ok().as_ref().map(|block| block.text.as_str()),
        messages.as_deref(),
        axes,
        &macros,
    ))
}

async fn write(
    client: &FocasClient,
    spec: &FocasClientSpec,
    axes: usize,
    point: u32,
    number: f64,
    out: &Emitter,
) -> Result<(), String> {
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    // The macro points come last, after the axes the session was laid out for.
    let (_, first_macro) = layout(axes);
    let slot = index
        .checked_sub(first_macro)
        .and_then(|slot| spec.macros.get(slot))
        .ok_or_else(|| "Only macro variables can be written".to_owned())?;
    if !number.is_finite() {
        return Err("Enter a number".into());
    }
    out.log(LogLevel::Info, format!("Writing macro #{slot} = {number}"));
    out.traffic(vec![frame(
        Direction::Tx,
        format!("Write macro #{slot}"),
        number.to_string(),
    )]);
    client
        .write_macro(*slot, number)
        .await
        .map_err(|error| error.to_string())
}
