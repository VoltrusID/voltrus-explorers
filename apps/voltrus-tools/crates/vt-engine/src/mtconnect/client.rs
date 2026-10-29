//! Watches an MTConnect agent: probes its device tree into points, then
//! keeps them current from the agent's live sample stream.

use std::time::{Duration, Instant};

use mt_mtconnect::{
    AgentClient, AgentConfig, MtConnectError, Observation, ResyncReason, StreamEvent,
    StreamOptions, StreamTransport, Streams,
};
use tokio::sync::mpsc;
use vt_protocol::{
    Direction, LogLevel, MtconnectClientSpec, Reading, SessionState, Stats, ValueFrame,
};

use super::{Layout, reading};
use crate::Control;
use crate::cnc::{Texts, fail, frame, retry, until_closed};
use crate::emit::{Emitter, now_ms};

/// The fastest the stream is asked to deliver.
const MIN_INTERVAL: Duration = Duration::from_millis(50);
/// How often the app is told the session is alive while the machine is quiet.
pub(crate) const TICK: Duration = Duration::from_secs(1);

pub(crate) async fn run(
    spec: MtconnectClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let device = spec.device.trim();
    let client = match AgentClient::new(AgentConfig {
        base_url: spec.url.trim().to_owned(),
        device: (!device.is_empty()).then(|| device.to_owned()),
        timeout_ms: spec.timeout_ms,
    }) {
        Ok(client) => client,
        Err(error) => {
            fail(out, describe(&error));
            return;
        }
    };
    let interval = Duration::from_millis(spec.interval_ms).max(MIN_INTERVAL);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let started = Instant::now();
        let probed = tokio::select! {
            result = client.probe() => result,
            () = until_closed(&mut control, out, "Not connected") => break 'session,
        };
        let devices = match probed {
            Ok(devices) => devices,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                out.stats(stats);
                let reason = describe(&error);
                out.log(LogLevel::Error, reason.clone());
                out.traffic(vec![
                    frame(Direction::Tx, "GET /probe", spec.url.trim()),
                    frame(Direction::Rx, "Probe failed", reason.clone()),
                ]);
                if !retry(&mut control, attempt, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        attempt = 0;
        stats.last_latency_us = micros(started.elapsed());
        let layout = Layout::from_devices(&devices);
        out.points(layout.infos.clone());
        out.traffic(vec![
            frame(Direction::Tx, "GET /probe", spec.url.trim()),
            frame(
                Direction::Rx,
                format!(
                    "MTConnectDevices: {} device(s), {} data items",
                    devices.devices.len(),
                    layout.infos.len()
                ),
                devices
                    .devices
                    .iter()
                    .map(|device| device.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ]);
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            format!(
                "Probed {}: {} device(s), {} data items (MTConnect {})",
                spec.url.trim(),
                devices.devices.len(),
                layout.infos.len(),
                devices.header.version.as_deref().unwrap_or("?")
            ),
        );
        let assets = tokio::select! {
            result = client.assets() => result,
            () = until_closed(&mut control, out, "Not connected") => break 'session,
        };
        match assets {
            Ok(list) if !list.assets.is_empty() => out.log(
                LogLevel::Info,
                format!("The agent holds {} asset(s)", list.assets.len()),
            ),
            // Agents without assets answer with an error; that is not a problem here.
            Ok(_) | Err(_) => {}
        }

        let mut texts = Texts::new(layout.infos.len());
        let mut latest: Vec<Option<Observation>> = vec![None; layout.infos.len()];
        let mut stream = client.stream_sample_with(StreamOptions::new(interval));
        let mut transport = StreamTransport::Connecting;
        let mut quiet = tokio::time::interval(TICK);
        quiet.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                event = stream.next() => {
                    let Some(event) = event else {
                        let reason = "The agent closed the stream".to_owned();
                        out.log(LogLevel::Error, reason.clone());
                        attempt = 1;
                        if !retry(&mut control, attempt, &reason, out).await {
                            break 'session;
                        }
                        continue 'session;
                    };
                    let event = match event {
                        Ok(event) => event,
                        Err(error) => {
                            stats.errors += 1;
                            out.stats(stats);
                            let reason = describe(&error);
                            out.log(LogLevel::Error, reason.clone());
                            attempt += 1;
                            drop(stream);
                            if !retry(&mut control, attempt, &reason, out).await {
                                break 'session;
                            }
                            continue 'session;
                        }
                    };
                    if stream.transport() != transport {
                        transport = stream.transport();
                        out.log(
                            LogLevel::Info,
                            match transport {
                                StreamTransport::Streaming => "Streaming from the agent (multipart)",
                                StreamTransport::Polling => "The agent does not stream: polling /sample",
                                StreamTransport::Connecting => "Connecting the stream",
                            },
                        );
                    }
                    let (what, streams) = match event {
                        StreamEvent::Snapshot(streams) => ("current", streams),
                        StreamEvent::Observations(streams) => ("sample", streams),
                        StreamEvent::Resync { reason, snapshot } => {
                            match reason {
                                ResyncReason::InstanceChanged { previous, current } => {
                                    out.log(
                                        LogLevel::Warn,
                                        format!("The agent restarted (instance {previous} to {current}); probing again"),
                                    );
                                    continue 'session;
                                }
                                ResyncReason::OutOfRange => out.log(
                                    LogLevel::Warn,
                                    "The agent's buffer moved past this stream; resynchronised from current",
                                ),
                            }
                            ("current", snapshot)
                        }
                    };
                    apply(&streams, &layout, &mut latest);
                    stats.polls += 1;
                    stats.frames += 2;
                    out.traffic(vec![
                        frame(Direction::Tx, format!("GET /{what}"), ""),
                        frame(
                            Direction::Rx,
                            format!("MTConnectStreams: {} observation(s)", streams.len()),
                            sequence_detail(&streams),
                        ),
                    ]);
                    publish(&layout, &latest, &mut texts, stats, out);
                }
                _ = quiet.tick() => out.stats(stats),
                command = control.recv() => match command {
                    None | Some(Control::Close) => break 'session,
                    Some(Control::Write { point, .. }) => {
                        out.written(point, Err("MTConnect agents are read-only".into()));
                    }
                    Some(_) => {}
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Keep the newest observation of every data item the layout knows.
fn apply(streams: &Streams, layout: &Layout, latest: &mut [Option<Observation>]) {
    for observation in streams.observations_by_sequence() {
        if let Some(slot) = layout
            .index
            .get(&observation.data_item_id)
            .and_then(|index| latest.get_mut(*index))
        {
            *slot = Some(observation.clone());
        }
    }
}

fn publish(
    layout: &Layout,
    latest: &[Option<Observation>],
    texts: &mut Texts,
    stats: Stats,
    out: &Emitter,
) {
    let mut readings = Vec::with_capacity(latest.len());
    let mut now = Vec::with_capacity(latest.len());
    for (observation, kind) in latest.iter().zip(&layout.kinds) {
        match observation {
            Some(observation) => {
                let (read, text) =
                    reading(kind, &observation.value, observation.condition.as_ref());
                readings.push(read);
                now.push(text);
            }
            None => {
                readings.push(Reading::bad());
                now.push(String::new());
            }
        }
    }
    let changed = texts.changed(&now);
    if !changed.is_empty() {
        out.payloads(changed);
    }
    out.values(ValueFrame {
        at_ms: now_ms(),
        latency_us: stats.last_latency_us,
        readings,
    });
    out.stats(stats);
}

fn sequence_detail(streams: &Streams) -> String {
    let header = &streams.header;
    format!(
        "instance {} · first {} · last {} · next {}",
        header.instance_id,
        header.first_sequence.map_or("?".into(), |n| n.to_string()),
        header.last_sequence.map_or("?".into(), |n| n.to_string()),
        header.next_sequence.map_or("?".into(), |n| n.to_string()),
    )
}

fn micros(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.as_micros()).unwrap_or(u32::MAX)
}

/// `mt-mtconnect` errors explain themselves.
pub(super) fn describe(error: &MtConnectError) -> String {
    error.to_string()
}
