//! Plays a BACnet/IP device: `mt-bacnet`'s device simulator serving a plant
//! template's objects, answering Who-Is, ReadProperty(Multiple),
//! WriteProperty and SubscribeCOV from any client.
//!
//! A value set from the app is a priority-16 command on commandable objects
//! and a forced value on the others; either way the simulation holds it until
//! released. Commands a client writes at its own priority hold the point too,
//! and the app sees every held point as `Locked`.

use std::net::SocketAddr;
use std::time::Duration;

use mt_bacnet::device::SimDevice;
use mt_bacnet::{SimPointDef, SimSnapshot};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    BacnetDeviceSpec, Direction, LogLevel, PointInfo, Quality, Reading, SessionState, Stats,
    TrafficFrame, ValueFrame,
};

use super::{Object, ObjectKind, describe_frame};
use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::templates::{self, BacnetTemplate};

/// How often the simulator moves its values and reports them.
pub(crate) const TICK: Duration = Duration::from_secs(1);
/// How often the app sees the requests the device served.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
/// The device thread lets go of its socket within one receive timeout (250 ms).
const RELEASE: Duration = Duration::from_millis(300);
/// Vendor identifier the device reports in its I-Am (0 = ASHRAE).
const VENDOR_ID: u16 = 0;

pub(crate) async fn run(
    spec: BacnetDeviceSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let Some(template) = templates::find_bacnet(&spec.template) else {
        fail(out, format!("Unknown BACnet plant {}", spec.template));
        return;
    };
    let (snapshot_tx, mut snapshots) = mpsc::unbounded_channel::<SimSnapshot>();
    let (frame_tx, mut frames) = mpsc::unbounded_channel::<TrafficFrame>();
    let device = match start(&spec, template, &snapshot_tx, &frame_tx, &mut control, out).await {
        Ok(Some(device)) => device,
        Ok(None) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
        Err(error) => {
            fail(out, error);
            return;
        }
    };
    out.state(SessionState::Listening {
        address: format!("0.0.0.0:{}", spec.port),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Simulating {} as device {} on UDP port {}, {} objects",
            template.name,
            spec.device_instance,
            spec.port,
            template.points.len()
        ),
    );
    out.points(points(template));

    let mut held = vec![false; template.points.len()];
    let mut values = vec![f64::NAN; template.points.len()];
    let mut stats = Stats::default();
    let mut traffic = Vec::new();
    let mut flush = tokio::time::interval(TRAFFIC_TICK);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            snapshot = snapshots.recv() => {
                let Some(snapshot) = snapshot else { break };
                // A snapshot of a device that could not keep the port and was
                // stopped has another port: not this device's.
                if snapshot.port != spec.port {
                    continue;
                }
                for (index, point) in snapshot.points.iter().enumerate().take(held.len()) {
                    values[index] = point.value;
                    if held[index] != point.held {
                        held[index] = point.held;
                        out.locked(u32::try_from(index).unwrap_or(u32::MAX), point.held);
                    }
                }
                stats.polls += 1;
                out.values(frame(&values));
            }
            frame_in = frames.recv() => {
                if let Some(frame_in) = frame_in {
                    stats.frames += 1;
                    traffic.push(frame_in);
                }
            }
            _ = flush.tick() => {
                out.traffic(std::mem::take(&mut traffic));
                out.stats(stats);
            }
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = match key(template, point) {
                        Some(_) if !value.is_finite() => Err("Set a number".to_owned()),
                        Some(key) => {
                            device.set_value(key, value);
                            Ok(())
                        }
                        None => Err(format!("No point {point}")),
                    };
                    out.written(point, result);
                }
                Some(Control::Lock { point, locked }) => {
                    if let Some(key) = key(template, point) {
                        if locked {
                            let index = usize::try_from(point).unwrap_or(usize::MAX);
                            if let Some(value) = values.get(index).filter(|value| value.is_finite()) {
                                device.set_value(key, *value);
                            }
                        } else {
                            device.release(key);
                        }
                    }
                }
                Some(Control::Relinquish { point }) => {
                    out.written(point, Err("Simulators release a point with Release".into()));
                }
                // Publishing is for message protocols.
                Some(Control::Publish { .. }) => {}
            },
        }
    }
    device.stop();
    tokio::time::sleep(RELEASE).await;
    out.traffic(traffic);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn fail(out: &Emitter, message: String) {
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

/// Start the device on the spec's port. The simulator falls back to any free
/// port when the asked one is taken; that device is stopped and the port
/// retried for [`BIND_RETRY`], as a closing session may still hold it.
/// `Ok(None)` when the app closed the session meanwhile.
async fn start(
    spec: &BacnetDeviceSpec,
    template: &BacnetTemplate,
    snapshots: &mpsc::UnboundedSender<SimSnapshot>,
    frames: &mpsc::UnboundedSender<TrafficFrame>,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<SimDevice>, String> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        let snapshots = snapshots.clone();
        let frames = frames.clone();
        let device = mt_bacnet::device::start_with_traffic(
            spec.device_instance,
            spec.port,
            VENDOR_ID,
            template.points.clone(),
            Box::new(move |snapshot: SimSnapshot| {
                let _ = snapshots.send(snapshot);
            }),
            Box::new(|_: String, _: f64| {}),
            Box::new(move |received: bool, peer: SocketAddr, datagram: &[u8]| {
                let (summary, detail) = describe_frame(datagram);
                let detail = if detail.is_empty() {
                    peer.to_string()
                } else {
                    format!("{detail} · {peer}")
                };
                let _ = frames.send(TrafficFrame {
                    at_ms: now_ms(),
                    direction: if received {
                        Direction::Rx
                    } else {
                        Direction::Tx
                    },
                    summary,
                    detail,
                    bytes: u32::try_from(datagram.len()).unwrap_or(u32::MAX),
                });
            }),
        )
        .map_err(|error| format!("Cannot start the device: {error}"))?;
        if device.port() == spec.port {
            return Ok(Some(device));
        }
        device.stop();
        if Instant::now() >= deadline {
            return Err(format!(
                "UDP port {} is in use by another program",
                spec.port
            ));
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            () = until_closed(control, out) => return Ok(None),
        }
    }
}

/// Wait for the app to close the session before the device runs, refusing
/// writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. } | Control::Relinquish { point }) => {
                out.written(point, Err("The device is not running yet".into()));
            }
            Some(_) => {}
        }
    }
}

fn key(template: &BacnetTemplate, point: u32) -> Option<&str> {
    template
        .points
        .get(usize::try_from(point).ok()?)
        .map(|point| point.key.as_str())
}

/// The template's objects as points. The app may set any of them: forcing a
/// sensor is what a simulator is for.
fn points(template: &BacnetTemplate) -> Vec<PointInfo> {
    template
        .points
        .iter()
        .enumerate()
        .map(|(id, point)| point_info(id, point))
        .collect()
}

fn point_info(id: usize, point: &SimPointDef) -> PointInfo {
    let kind = ObjectKind::from_name(&point.object_type);
    PointInfo {
        id: u32::try_from(id).unwrap_or(u32::MAX),
        name: point.label.clone(),
        address: kind.map_or_else(
            || format!("{} {}", point.object_type, point.object_instance),
            |kind| {
                Object {
                    kind,
                    instance: point.object_instance,
                }
                .label()
            },
        ),
        data_type: kind.map_or("Object", ObjectKind::label).into(),
        // The units a client reads from the device, not the template's spelling.
        unit: mt_bacnet::units_label(mt_bacnet::device::unit_enum(point.unit.as_deref())),
        writable: true,
        boolean: point.is_bool,
        states: point.state_text.clone(),
    }
}

fn frame(values: &[f64]) -> ValueFrame {
    ValueFrame {
        at_ms: now_ms(),
        latency_us: 0,
        readings: values
            .iter()
            .map(|&value| {
                if value.is_finite() {
                    Reading {
                        value,
                        raw: [0; 4],
                        raw_len: 0,
                        quality: Quality::Good,
                    }
                } else {
                    Reading::bad()
                }
            })
            .collect(),
    }
}
