//! Plays an EtherNet/IP adapter: `mt-enip`'s Logix-style target (sessions,
//! Read/Write Tag, Multiple_Service_Packet, symbol browse, the Identity and
//! Assembly objects, Forward_Open and class 1 I/O) serving a plant template,
//! answering ListIdentity on the same UDP port.
//!
//! The session moves the plant's values. A value set from the app, or written
//! by any client to a value that drifts, holds it until released, and the
//! app sees every held point as `Locked`.

use std::f64::consts::TAU;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mt_enip::{
    CipDataType, CipTagDef, CipValue, EnipServerConfig, EnipServerLogEntry, EnipServerState,
    SimPointDef,
};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    CipType, Direction, EnipAdapterSpec, LogLevel, PointInfo, Quality, Reading, SessionState,
    Stats, TrafficFrame, ValueFrame,
};

use super::plants::{self, EnipTemplate, assembly_layout, point_type, short_name};
use super::{encode, table_address};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// How often the simulator moves its values and reports them.
pub(crate) const TICK: Duration = Duration::from_millis(500);
/// How often the app sees the requests the adapter served.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
/// Time for the listener and the ListIdentity socket to let go of the port.
const RELEASE: Duration = Duration::from_millis(100);

pub(crate) async fn run(
    spec: EnipAdapterSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let Some(template) = plants::find_enip(&spec.template) else {
        fail(out, format!("Unknown EtherNet/IP plant {}", spec.template));
        return;
    };
    let (state, mut server_log, shutdown) = match listen(&spec, template, &mut control, out).await {
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
    let (frames_tx, mut frames) = mpsc::unbounded_channel::<TrafficFrame>();
    // Keeps the channel open when discovery has no socket, so `recv` waits
    // instead of returning `None` on every turn of the loop.
    let _frames_open = frames_tx.clone();
    match bind_discovery(spec.port, &mut control).await {
        Some(Ok(socket)) => {
            tokio::spawn(mt_enip::answer_list_identity(
                socket,
                [0, 0, 0, 0],
                spec.port,
                shutdown.subscribe(),
                move |peer: SocketAddr, request: &[u8], reply: &[u8]| {
                    for (direction, bytes) in [(Direction::Rx, request), (Direction::Tx, reply)] {
                        let (summary, detail) = mt_enip::describe_frame(bytes);
                        let _ = frames_tx.send(TrafficFrame {
                            at_ms: now_ms(),
                            direction,
                            summary,
                            detail: if detail.is_empty() {
                                peer.to_string()
                            } else {
                                format!("{detail} · {peer}")
                            },
                            bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX),
                        });
                    }
                },
            ));
        }
        Some(Err(error)) => out.log(
            LogLevel::Warn,
            format!(
                "Discovery will not find this adapter: UDP port {} {error}",
                spec.port
            ),
        ),
        None => {
            let _ = shutdown.send(true);
            out.state(SessionState::Closed { error: None });
            return;
        }
    }
    let layout = assembly_layout(template);
    let assembly_size: usize = layout.iter().map(|(_, data_type)| data_type.size()).sum();
    out.state(SessionState::Listening {
        address: format!("0.0.0.0:{}", spec.port),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Playing {} on port {} (TCP sessions, UDP ListIdentity): {} tags, input assembly {} of {assembly_size} bytes",
            template.name,
            spec.port,
            template.points.len(),
            mt_enip::INPUT_ASSEMBLY,
        ),
    );
    out.points(points(template));
    out.enip_addresses(
        template
            .points
            .iter()
            .map(|point| point.name.clone())
            .collect(),
    );

    let mut sim = Simulation::new(template);
    let mut stats = Stats::default();
    let mut traffic = Vec::new();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut flush = tokio::time::interval(TRAFFIC_TICK);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let values = sim.step(&state, out).await;
                stats.polls += 1;
                out.values(frame(&values));
            }
            entry = server_log.recv() => {
                let Some(entry) = entry else { break };
                match entry.service.as_str() {
                    "connect" | "disconnect" => out.log(LogLevel::Info, entry.message),
                    _ => {
                        stats.frames += 1;
                        traffic.push(served(&entry));
                    }
                }
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
                    let result = sim.set(&state, point, value, out).await;
                    out.written(point, result);
                }
                Some(Control::Lock { point, locked }) => sim.hold(point, locked, out),
                Some(Control::CipRequest(_)) => {
                    out.cip_reply(Err("Explicit messages are sent from a client connection".into()));
                }
                Some(_) => {}
            },
        }
    }
    let _ = shutdown.send(true);
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

type Server = (
    Arc<EnipServerState>,
    mpsc::UnboundedReceiver<EnipServerLogEntry>,
    watch::Sender<bool>,
);

/// Start the target on the spec's TCP port, retrying for [`BIND_RETRY`]
/// while a closing session may still hold it. `Ok(None)` when the app closed
/// the session meanwhile.
async fn listen(
    spec: &EnipAdapterSpec,
    template: &EnipTemplate,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<Server>, String> {
    let tags: Vec<CipTagDef> = template
        .points
        .iter()
        .map(|point| {
            let data_type = point_type(point);
            CipTagDef {
                name: point.name.clone(),
                data_type: cip_data_type(data_type),
                element_count: 1,
                initial_value: cip_value(data_type, point.base),
                template_id: None,
                // Any client may force any value: that is what a simulator is for.
                read_only: false,
            }
        })
        .collect();
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        let config = EnipServerConfig {
            port: spec.port,
            bind_addr: "0.0.0.0".into(),
        };
        let error = match mt_enip::start_enip_server(config, tags.clone(), Vec::new()).await {
            Ok(server) => return Ok(Some(server)),
            Err(error) => error,
        };
        let in_use = error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::AddrInUse);
        if !in_use {
            return Err(format!("Cannot listen on TCP port {}: {error}", spec.port));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "TCP port {} is in use by another program",
                spec.port
            ));
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            () = until_closed(control, out) => return Ok(None),
        }
    }
}

/// The UDP socket ListIdentity requests arrive on: the session's port on
/// every interface, retried while a closing adapter lets go of it. `None`
/// when the app closed the session meanwhile.
async fn bind_discovery(
    port: u16,
    control: &mut mpsc::UnboundedReceiver<Control>,
) -> Option<Result<UdpSocket, String>> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        let error = match UdpSocket::bind(("0.0.0.0", port)).await {
            Ok(socket) => return Some(Ok(socket)),
            Err(error) => error,
        };
        if error.kind() != std::io::ErrorKind::AddrInUse {
            return Some(Err(format!("cannot be opened: {error}")));
        }
        if Instant::now() >= deadline {
            return Some(Err("is in use by another program".into()));
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            command = control.recv() => {
                if matches!(command, None | Some(Control::Close)) {
                    return None;
                }
            }
        }
    }
}

/// Wait for the app to close the session before the adapter runs, refusing
/// writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("The adapter is not running yet".into()));
            }
            Some(_) => {}
        }
    }
}

/// A request the target served, from its log, for the traffic view.
fn served(entry: &EnipServerLogEntry) -> TrafficFrame {
    let summary = match (entry.service.as_str(), entry.tag_name.is_empty()) {
        ("read", false) => format!("Read Tag {}", entry.tag_name),
        ("write", false) => format!("Write Tag {}", entry.tag_name),
        ("multi_service", _) => "Multiple_Service_Packet".to_owned(),
        ("browse", _) => "Get_Instance_Attribute_List Symbol".to_owned(),
        ("template", _) => format!("Template {}", entry.tag_name),
        ("forward_open", _) => "Forward_Open".to_owned(),
        ("forward_close", _) => "Forward_Close".to_owned(),
        (service, _) => service.to_owned(),
    };
    TrafficFrame {
        at_ms: entry.timestamp,
        direction: Direction::Rx,
        summary,
        detail: entry.message.clone(),
        bytes: 0,
    }
}

fn points(template: &EnipTemplate) -> Vec<PointInfo> {
    template
        .points
        .iter()
        .enumerate()
        .map(|(id, point)| {
            let data_type = point_type(point);
            PointInfo {
                id: u32::try_from(id).unwrap_or(u32::MAX),
                name: short_name(&point.name).to_owned(),
                address: table_address(&vt_protocol::EnipAddress::Tag {
                    tag: point.name.clone(),
                }),
                data_type: data_type.name().into(),
                unit: point.unit.clone().unwrap_or_default(),
                // Setting a sensor is what a simulator is for.
                writable: true,
                boolean: data_type == CipType::Bool,
                states: Vec::new(),
            }
        })
        .collect()
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

pub(crate) fn cip_data_type(data_type: CipType) -> CipDataType {
    match data_type {
        CipType::Bool => CipDataType::Bool,
        CipType::Sint => CipDataType::Sint,
        CipType::Int => CipDataType::Int,
        CipType::Dint => CipDataType::Dint,
        CipType::Lint => CipDataType::Lint,
        CipType::Usint => CipDataType::Usint,
        CipType::Uint => CipDataType::Uint,
        CipType::Udint => CipDataType::Udint,
        CipType::Ulint => CipDataType::Ulint,
        CipType::Real => CipDataType::Real,
        CipType::Lreal => CipDataType::Lreal,
    }
}

/// `value` as a tag value of `data_type`, rounded and clamped to its range.
pub(crate) fn cip_value(data_type: CipType, value: f64) -> CipValue {
    let whole = value.round();
    match data_type {
        CipType::Bool => CipValue::Bool(value >= 0.5),
        CipType::Sint => CipValue::Sint(whole.clamp(f64::from(i8::MIN), f64::from(i8::MAX)) as i8),
        CipType::Int => CipValue::Int(whole.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16),
        CipType::Dint => {
            CipValue::Dint(whole.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
        }
        CipType::Lint => CipValue::Lint(whole as i64),
        CipType::Usint => CipValue::Usint(whole.clamp(0.0, f64::from(u8::MAX)) as u8),
        CipType::Uint => CipValue::Uint(whole.clamp(0.0, f64::from(u16::MAX)) as u16),
        CipType::Udint => CipValue::Udint(whole.clamp(0.0, f64::from(u32::MAX)) as u32),
        CipType::Ulint => CipValue::Ulint(whole.max(0.0) as u64),
        CipType::Real => CipValue::Real(value as f32),
        CipType::Lreal => CipValue::Lreal(value),
    }
}

/// How the plant's values move.
struct Simulation {
    points: Vec<SimPointDef>,
    types: Vec<CipType>,
    /// Each point's own value (counters keep their fractions here).
    values: Vec<f64>,
    /// What the simulation last put in each tag: anything else came from a client.
    last_set: Vec<f64>,
    held: Vec<bool>,
    started: Instant,
    last_step: Instant,
    rng: u64,
}

impl Simulation {
    fn new(template: &EnipTemplate) -> Self {
        let types: Vec<CipType> = template.points.iter().map(point_type).collect();
        let values: Vec<f64> = template
            .points
            .iter()
            .zip(&types)
            .map(|(point, data_type)| {
                cip_value(*data_type, point.base)
                    .to_f64()
                    .unwrap_or(point.base)
            })
            .collect();
        let seed = now_ms() | 1;
        let now = Instant::now();
        Self {
            points: template.points.clone(),
            last_set: values.clone(),
            held: vec![false; values.len()],
            values,
            types,
            started: now,
            last_step: now,
            rng: seed,
        }
    }

    fn noise(&mut self) -> f64 {
        // xorshift64: plenty for a plant's jitter.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        #[allow(clippy::cast_precision_loss)]
        let unit = (x >> 11) as f64 / (1u64 << 53) as f64;
        unit * 2.0 - 1.0
    }

    fn value_of(&self, key: &str) -> Option<(f64, f64)> {
        self.points
            .iter()
            .position(|point| point.key == key)
            .map(|index| (self.values[index], self.points[index].base))
    }

    /// Notice what clients wrote, move every free value, and serve the result.
    /// Returns the values as clients read them.
    async fn step(&mut self, state: &EnipServerState, out: &Emitter) -> Vec<f64> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_step).as_secs_f64();
        self.last_step = now;
        let elapsed_ms = now.duration_since(self.started).as_secs_f64() * 1000.0;
        let mut tags = state.tags.lock().await;

        for index in 0..self.points.len() {
            let Some(current) = tags
                .get(&self.points[index].name)
                .and_then(|entry| entry.value.to_f64())
            else {
                continue;
            };
            if current != self.last_set[index] {
                // A client wrote it: a value that drifts stays where it was put.
                self.values[index] = current;
                self.last_set[index] = current;
                if !self.points[index].writable && !self.held[index] {
                    self.held[index] = true;
                    out.locked(u32::try_from(index).unwrap_or(u32::MAX), true);
                }
            }
        }

        let mode = self.value_of("mode").map_or(2.0, |(value, _)| value);
        let reference = self.value_of("speedRef");
        for index in 0..self.points.len() {
            if self.points[index].writable || self.held[index] {
                continue;
            }
            let noise = self.noise();
            let point = &self.points[index];
            let mut value = match point.source.as_str() {
                "drive" => {
                    let target = match reference {
                        Some((speed, base)) if mode > 0.5 && base != 0.0 => {
                            speed / base * point.base
                        }
                        Some(_) if mode > 0.5 => point.base,
                        Some(_) => 0.0,
                        None if mode > 0.5 => point.base,
                        None => 0.0,
                    };
                    if point.is_bool {
                        f64::from(u8::from(target > 0.0))
                    } else {
                        let current = self.values[index];
                        let jitter = if target > 0.0 {
                            noise * point.noise
                        } else {
                            0.0
                        };
                        current + (target - current) * 0.25 + jitter
                    }
                }
                "counter" => self.values[index] + point.variance * dt,
                _ if point.is_bool => f64::from(u8::from(point.base >= 0.5)),
                _ => {
                    #[allow(clippy::cast_precision_loss)]
                    let period = point.period_ms.max(1) as f64;
                    point.base
                        + (elapsed_ms / period * TAU).sin() * point.variance
                        + noise * point.noise
                }
            };
            if let Some(min) = point.min {
                value = value.max(min);
            }
            if let Some(max) = point.max {
                value = value.min(max);
            }
            self.values[index] = value;
            let served = cip_value(self.types[index], value);
            self.last_set[index] = served.to_f64().unwrap_or(value);
            if let Some(entry) = tags.get_mut(&point.name) {
                entry.value = served;
            }
        }

        self.points
            .iter()
            .map(|point| {
                tags.get(&point.name)
                    .and_then(|entry| entry.value.to_f64())
                    .unwrap_or(f64::NAN)
            })
            .collect()
    }

    /// The app sets a point: served at once, and held if it drifts.
    async fn set(
        &mut self,
        state: &EnipServerState,
        point: u32,
        value: f64,
        out: &Emitter,
    ) -> Result<(), String> {
        let index = usize::try_from(point)
            .ok()
            .filter(|index| *index < self.points.len())
            .ok_or_else(|| format!("No point {point}"))?;
        encode(self.types[index], value)?;
        let served = cip_value(self.types[index], value);
        let mut tags = state.tags.lock().await;
        let entry = tags
            .get_mut(&self.points[index].name)
            .ok_or_else(|| format!("No tag {}", self.points[index].name))?;
        self.last_set[index] = served.to_f64().unwrap_or(value);
        self.values[index] = self.last_set[index];
        entry.value = served;
        if !self.points[index].writable && !self.held[index] {
            self.held[index] = true;
            out.locked(point, true);
        }
        Ok(())
    }

    fn hold(&mut self, point: u32, locked: bool, out: &Emitter) {
        if let Some(held) = usize::try_from(point)
            .ok()
            .and_then(|index| self.held.get_mut(index))
        {
            *held = locked;
            out.locked(point, locked);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_served_in_their_tag_type() {
        assert_eq!(cip_value(CipType::Bool, 0.7), CipValue::Bool(true));
        assert_eq!(cip_value(CipType::Dint, 41.6), CipValue::Dint(42));
        assert_eq!(cip_value(CipType::Sint, 1000.0), CipValue::Sint(127));
        assert_eq!(cip_value(CipType::Uint, -5.0), CipValue::Uint(0));
        assert_eq!(cip_value(CipType::Real, 1.25), CipValue::Real(1.25));
        for data_type in CipType::ALL {
            assert_eq!(
                cip_data_type(data_type).as_str(),
                data_type.name(),
                "{data_type:?}"
            );
        }
    }
}
