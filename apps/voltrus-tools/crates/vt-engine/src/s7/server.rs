//! Plays an S7 CPU: `mt-s7`'s server serving a template's memory, its
//! identity, diagnostic buffer and blocks to any S7 client.
//!
//! The template's values move every tick. A value set from the app, or
//! written by any client, is held until released, and the app sees every
//! held point as `Locked`.

use std::io::ErrorKind;
use std::net::IpAddr;
use std::time::Duration;

use mt_s7::{S7Frame, S7Server, S7ServerOptions, S7Tag, S7Value};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    LogLevel, PointText, Reading, S7ServerSpec, S7TagSpec, S7Type, SessionState, Stats,
    TrafficFrame, ValueFrame,
};

use super::template::{S7Template, find_s7};
use super::{point_info, reading, tag, traffic_frame, value};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// How often the simulator moves its values and reports them.
pub(crate) const TICK: Duration = Duration::from_secs(1);
/// How often the app sees the requests the CPU served.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);

pub(crate) async fn run(
    spec: S7ServerSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let Some(template) = find_s7(&spec.template) else {
        fail(out, format!("Unknown S7 template {}", spec.template));
        return;
    };
    let Some(bind) = crate::bacnet::parse_ip(&spec.bind) else {
        fail(out, format!("Listen on an IP address, not {}", spec.bind));
        return;
    };
    let specs: Vec<S7TagSpec> = template
        .points
        .iter()
        .map(|point| S7TagSpec {
            name: point.label.clone(),
            address: point.address,
            data_type: point.data_type,
            unit: point.unit.clone(),
        })
        .collect();
    let tags: Vec<S7Tag> = specs
        .iter()
        .map(|spec| tag(&spec.name, spec.address, spec.data_type))
        .collect();
    let (frame_tx, mut frames) = mpsc::unbounded_channel::<S7Frame>();
    let server = match start(&spec, bind, template, &tags, &frame_tx, &mut control, out).await {
        Ok(Some(server)) => server,
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
        address: server.local_addr().to_string(),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Simulating the {} ({}) on TCP port {}, {} tags, PDU up to {} bytes",
            template.name,
            template.cpu,
            spec.port,
            tags.len(),
            spec.pdu_size
        ),
    );
    // The app may set any point: forcing a sensor is what a simulator is for.
    out.points(
        specs
            .iter()
            .enumerate()
            .map(|(id, spec)| point_info(id, spec, spec.data_type != S7Type::String))
            .collect(),
    );
    let texts: Vec<PointText> = template
        .points
        .iter()
        .enumerate()
        .filter(|(_, point)| point.data_type == S7Type::String)
        .map(|(index, point)| PointText {
            point: u32::try_from(index).unwrap_or(u32::MAX),
            text: point.text.clone(),
        })
        .collect();

    let started = Instant::now();
    let mut held = vec![false; tags.len()];
    let mut stats = Stats::default();
    let mut traffic: Vec<TrafficFrame> = Vec::new();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut flush = tokio::time::interval(TRAFFIC_TICK);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut texts_sent = false;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let seconds = started.elapsed().as_secs_f64();
                let updates: Vec<(S7Tag, S7Value)> = template
                    .points
                    .iter()
                    .enumerate()
                    .filter(|(index, point)| !held[*index] && point.data_type != S7Type::String)
                    .filter_map(|(index, point)| {
                        value(point.value_at(seconds, index), point.data_type, true)
                            .ok()
                            .map(|value| (tags[index].clone(), value))
                    })
                    .collect();
                let writes = server.apply_unless_written(&updates).await;
                for (index, tag) in tags.iter().enumerate() {
                    if !held[index] && writes.iter().any(|write| write.touches(tag)) {
                        held[index] = true;
                        out.locked(u32::try_from(index).unwrap_or(u32::MAX), true);
                        out.log(
                            LogLevel::Info,
                            format!("A client wrote {}; holding it", specs[index].address),
                        );
                    }
                }
                report(&server, &tags, &specs, out).await;
                if !texts_sent {
                    out.payloads(texts.clone());
                    texts_sent = true;
                }
                stats.polls += 1;
                stats.frames = server.requests_served();
                out.stats(stats);
            }
            frame = frames.recv() => {
                if let Some(frame) = frame {
                    traffic.push(traffic_frame(frame));
                }
            }
            _ = flush.tick() => {
                out.traffic(std::mem::take(&mut traffic));
            }
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value: number }) => {
                    let result = set(&server, &tags, &specs, point, number).await;
                    if result.is_ok() {
                        if let Some(slot) = usize::try_from(point).ok().and_then(|index| held.get_mut(index)) {
                            *slot = true;
                        }
                        out.locked(point, true);
                        report(&server, &tags, &specs, out).await;
                    }
                    out.written(point, result);
                }
                Some(Control::Lock { point, locked }) => {
                    if let Some(slot) = usize::try_from(point).ok().and_then(|index| held.get_mut(index)) {
                        *slot = locked;
                        out.locked(point, locked);
                    }
                }
                // Diagnostics are read by clients; publishing is for message protocols.
                Some(_) => {}
            },
        }
    }
    server.stop().await;
    while let Ok(frame) = frames.try_recv() {
        traffic.push(traffic_frame(frame));
    }
    out.traffic(traffic);
    stats.frames = server.requests_served();
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn fail(out: &Emitter, message: String) {
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

/// Set a point from the app.
async fn set(
    server: &S7Server,
    tags: &[S7Tag],
    specs: &[S7TagSpec],
    point: u32,
    number: f64,
) -> Result<(), String> {
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    let (Some(tag), Some(spec)) = (tags.get(index), specs.get(index)) else {
        return Err(format!("No point {point}"));
    };
    let encoded = value(number, spec.data_type, true)?;
    server.set_value(tag, encoded).await;
    Ok(())
}

/// The values clients read now.
async fn report(server: &S7Server, tags: &[S7Tag], specs: &[S7TagSpec], out: &Emitter) {
    let values = server.read_values(tags).await;
    let readings: Vec<Reading> = values
        .iter()
        .zip(specs)
        .map(|(value, spec)| reading(value, spec.data_type).0)
        .collect();
    out.values(ValueFrame {
        at_ms: now_ms(),
        latency_us: 0,
        readings,
    });
}

/// Start serving on the spec's port, retrying for [`BIND_RETRY`] while it is
/// in use, as a closing session may still hold it. `Ok(None)` when the app
/// closed the session meanwhile.
async fn start(
    spec: &S7ServerSpec,
    bind: IpAddr,
    template: &S7Template,
    tags: &[S7Tag],
    frames: &mpsc::UnboundedSender<S7Frame>,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<S7Server>, String> {
    let initial: Vec<(S7Tag, S7Value)> = template
        .points
        .iter()
        .zip(tags)
        .filter_map(|(point, tag)| {
            let initial = if point.data_type == S7Type::String {
                Some(S7Value::String(point.text.clone()))
            } else {
                value(point.value_at(0.0, 0), point.data_type, true).ok()
            };
            initial.map(|initial| (tag.clone(), initial))
        })
        .collect();
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        let options = S7ServerOptions {
            bind,
            pdu_size: spec.pdu_size,
            traffic: Some(frames.clone()),
        };
        match S7Server::start_with_options(spec.port, initial.clone(), options).await {
            Ok(server) => return Ok(Some(server)),
            Err(error) if error.kind() == ErrorKind::AddrInUse && Instant::now() < deadline => {}
            Err(error) => return Err(bind_error(bind, spec.port, &error)),
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            () = until_closed(control, out) => return Ok(None),
        }
    }
}

fn bind_error(bind: IpAddr, port: u16, error: &std::io::Error) -> String {
    match error.kind() {
        ErrorKind::AddrInUse => format!(
            "TCP port {port} is in use by another program. Stop it, or give the simulator another port"
        ),
        ErrorKind::PermissionDenied => format!(
            "This Mac does not let the app listen on TCP port {port}. Use a port above 1023, e.g. 1102"
        ),
        ErrorKind::AddrNotAvailable => format!("{bind} is not an address of this computer"),
        _ => format!(
            "Cannot listen on {}: {error}",
            std::net::SocketAddr::new(bind, port)
        ),
    }
}

/// Wait for the app to close the session before the CPU runs, refusing
/// writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("The simulator is not running yet".into()));
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_errors_say_what_to_do() {
        let any = IpAddr::from([0, 0, 0, 0]);
        let busy = std::io::Error::from(ErrorKind::AddrInUse);
        assert!(bind_error(any, 102, &busy).contains("in use"));
        let denied = std::io::Error::from(ErrorKind::PermissionDenied);
        assert!(bind_error(any, 102, &denied).contains("above 1023"));
        let elsewhere = std::io::Error::from(ErrorKind::AddrNotAvailable);
        assert!(bind_error(IpAddr::from([10, 9, 9, 9]), 102, &elsewhere).contains("10.9.9.9"));
    }
}
