//! Plays a DNP3 outstation: `mt-dnp3`'s outstation server serving a plant's
//! points to any master over TCP, with values moving as the plant runs,
//! events for every change, outputs that masters command (and inputs that
//! follow them), and points the app can take offline.
//!
//! A value set from the app holds an input until released; outputs simply
//! take the value, as a master's command would.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mt_dnp3::objects::flag;
use mt_dnp3::types::{OutstationConfig, OutstationLogEntry, SimDnp3PointDef, SimDnp3Profile};
use mt_dnp3::{Dnp3Simulation, OutstationState, TrafficRecord, tick_values};
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Dnp3Action, Dnp3OutstationSpec, Dnp3PointKind, LogLevel, PointInfo, PointText, Quality,
    Reading, SessionState, Stats, ValueFrame,
};

use super::{data_type, flags_good, flags_text, point_label, traffic};
use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::templates::{self, Dnp3Plant, Dnp3PlantPoint};

/// How often the plant moves and the app sees its values.
pub(crate) const TICK: Duration = Duration::from_secs(1);
/// How often the app sees the frames the outstation exchanged.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
/// The server's tasks let go of the listener once they see the shutdown.
const RELEASE: Duration = Duration::from_millis(100);

type Shared = Arc<Mutex<OutstationState>>;

/// The running plant: the inputs the simulation moves and what the app holds.
struct Plant {
    plant: &'static Dnp3Plant,
    /// Inputs (binary, analog, counter) in the simulation, by plant point.
    sim_index: Vec<Option<usize>>,
    sim: Dnp3Simulation,
    /// Inputs the app holds at a value.
    held: Vec<Option<f64>>,
    elapsed_ms: u64,
}

impl Plant {
    fn new(plant: &'static Dnp3Plant) -> Self {
        let mut sim_index = Vec::with_capacity(plant.points.len());
        let mut defs = Vec::new();
        for point in &plant.points {
            if point.kind.writable() {
                sim_index.push(None);
                continue;
            }
            sim_index.push(Some(defs.len()));
            defs.push(SimDnp3PointDef {
                key: point.key.clone(),
                label: point.label.clone(),
                group: point.kind.group(),
                variation: 0,
                index: point.index,
                base: point.base,
                variance: point.variance,
                noise: point.noise,
                period_ms: point.period_ms.max(1),
                min: point.min,
                max: point.max,
                decimals: None,
                unit: None,
                is_bool: point.kind.is_binary(),
            });
        }
        Self {
            plant,
            sim_index,
            sim: Dnp3Simulation::new(SimDnp3Profile {
                id: plant.id.clone(),
                name: plant.name.clone(),
                description: plant.description.clone(),
                points: defs,
            }),
            held: vec![None; plant.points.len()],
            elapsed_ms: 0,
        }
    }

    fn point(&self, point: u32) -> Option<(usize, &'static Dnp3PlantPoint)> {
        let plant: &'static Dnp3Plant = self.plant;
        let index = usize::try_from(point).ok()?;
        plant.points.get(index).map(|found| (index, found))
    }

    /// The outstation's sizes: one past the highest index of each kind.
    fn counts(&self) -> [usize; 5] {
        let mut counts = [0usize; 5];
        for point in &self.plant.points {
            let slot = match point.kind {
                Dnp3PointKind::BinaryInput => 0,
                Dnp3PointKind::AnalogInput => 1,
                Dnp3PointKind::Counter => 2,
                Dnp3PointKind::BinaryOutput => 3,
                Dnp3PointKind::AnalogOutput => 4,
                Dnp3PointKind::DoubleBitInput | Dnp3PointKind::FrozenCounter => continue,
            };
            counts[slot] = counts[slot].max(usize::from(point.index) + 1);
        }
        counts
    }

    /// Set the outputs to the plant's starting values.
    fn start(&self, state: &mut OutstationState) {
        let counts = self.counts();
        state.configure_outputs(counts[3], counts[4]);
        for point in &self.plant.points {
            let index = usize::from(point.index);
            match point.kind {
                Dnp3PointKind::BinaryOutput => state.binary_outputs[index] = point.base >= 0.5,
                Dnp3PointKind::AnalogOutput => state.analog_outputs[index] = point.base,
                _ => {}
            }
        }
    }

    fn key_point(&self, key: &str) -> Option<&'static Dnp3PlantPoint> {
        let plant: &'static Dnp3Plant = self.plant;
        plant.points.iter().find(|point| point.key == key)
    }

    /// Move the plant one tick and serve the new values; changes become events.
    fn step(&mut self, state: &mut OutstationState) {
        self.elapsed_ms += u64::try_from(TICK.as_millis()).unwrap_or(1000);
        tick_values(&mut self.sim, self.elapsed_ms);
        // Binary inputs first: analog points may be gated by them.
        let plant: &'static Dnp3Plant = self.plant;
        for binary in [true, false] {
            for (id, point) in plant.points.iter().enumerate() {
                if point.kind.is_binary() != binary {
                    continue;
                }
                let Some(sim) = self.sim_index[id] else {
                    continue;
                };
                let value = match self.held[id] {
                    Some(held) => {
                        self.sim.values[sim] = held;
                        held
                    }
                    None => self.moved(point, sim, state),
                };
                write(state, point, value);
            }
        }
        state.detect_events();
    }

    /// The value of an input this tick: simulated, following its output, or
    /// stopped by its gate.
    fn moved(&mut self, point: &Dnp3PlantPoint, sim: usize, state: &OutstationState) -> f64 {
        let mut value = self.sim.values[sim];
        if point.kind.is_binary() && point.follows.is_none() {
            value = point.base;
        }
        if let Some(source) = point.follows.as_deref().and_then(|key| self.key_point(key)) {
            let output = read(state, source);
            value = if point.kind.is_binary() {
                output
            } else {
                // The setpoint, with the point's own noise around it.
                output + (self.sim.values[sim] - point.base)
            };
        }
        if let Some(gate) = point.gate.as_deref().and_then(|key| self.key_point(key))
            && read(state, gate) < 0.5
        {
            match point.kind {
                // A counter stops where it was.
                Dnp3PointKind::Counter => {
                    value = read(state, point);
                    self.sim.values[sim] = value;
                }
                _ => value = 0.0,
            }
        }
        value
    }
}

/// The value the outstation serves for a plant point.
fn read(state: &OutstationState, point: &Dnp3PlantPoint) -> f64 {
    let index = usize::from(point.index);
    let bit = |value: Option<&bool>| value.map_or(0.0, |on| f64::from(u8::from(*on)));
    match point.kind {
        Dnp3PointKind::BinaryInput => bit(state.binary_values.get(index)),
        Dnp3PointKind::BinaryOutput => bit(state.binary_outputs.get(index)),
        Dnp3PointKind::AnalogInput => state.analog_values.get(index).copied().unwrap_or(f64::NAN),
        Dnp3PointKind::AnalogOutput => state.analog_outputs.get(index).copied().unwrap_or(f64::NAN),
        Dnp3PointKind::Counter => state.counter_values.get(index).copied().unwrap_or(f64::NAN),
        Dnp3PointKind::DoubleBitInput | Dnp3PointKind::FrozenCounter => f64::NAN,
    }
}

/// Serve an input's value (outputs change only by command).
fn write(state: &mut OutstationState, point: &Dnp3PlantPoint, value: f64) {
    let index = usize::from(point.index);
    match point.kind {
        Dnp3PointKind::BinaryInput => {
            if let Some(slot) = state.binary_values.get_mut(index) {
                *slot = value >= 0.5;
            }
        }
        Dnp3PointKind::AnalogInput => {
            if let Some(slot) = state.analog_values.get_mut(index) {
                *slot = value;
            }
        }
        Dnp3PointKind::Counter => {
            if let Some(slot) = state.counter_values.get_mut(index) {
                *slot = value.max(0.0).floor();
            }
        }
        _ => {}
    }
}

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, OutstationState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) async fn run(
    spec: Dnp3OutstationSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let Some(template) = templates::find_dnp3(&spec.template) else {
        fail(out, format!("Unknown DNP3 plant {}", spec.template));
        return;
    };
    if spec.outstation_address == spec.master_address {
        fail(
            out,
            "The outstation and its master need different link addresses".into(),
        );
        return;
    }
    let Some(bind) = crate::bacnet::parse_ip(&spec.bind) else {
        fail(out, format!("Listen on an IP address, not {}", spec.bind));
        return;
    };
    let mut plant = Plant::new(template);
    let (tap, mut records) = mpsc::unbounded_channel::<TrafficRecord>();
    let counts = plant.counts();
    let config = OutstationConfig {
        port: spec.port,
        bind_addr: if bind.is_ipv6() {
            format!("[{bind}]")
        } else {
            bind.to_string()
        },
        master_address: spec.master_address,
        outstation_address: spec.outstation_address,
        analog_count: u16::try_from(counts[1]).unwrap_or(u16::MAX),
        binary_count: u16::try_from(counts[0]).unwrap_or(u16::MAX),
        counter_count: u16::try_from(counts[2]).unwrap_or(u16::MAX),
    };
    let (state, mut log, shutdown) = match start(config, tap, &mut control, out).await {
        Ok(Some(started)) => started,
        Ok(None) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
        Err(error) => {
            fail(out, error);
            return;
        }
    };
    {
        let mut state = lock(&state);
        plant.start(&mut state);
        plant.step(&mut state);
    }
    let address = if bind.is_ipv6() {
        format!("[{bind}]:{}", spec.port)
    } else {
        format!("{bind}:{}", spec.port)
    };
    out.state(SessionState::Listening {
        address: address.clone(),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Simulating {} as outstation {} on TCP {address}, {} points",
            template.name,
            spec.outstation_address,
            template.points.len()
        ),
    );
    out.points(points(template));
    let mut statuses = vec![String::new(); template.points.len()];
    report(&plant, &state, &mut statuses, out);

    let mut stats = Stats::default();
    let mut ticker = tokio::time::interval_at(Instant::now() + TICK, TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut flush = tokio::time::interval(TRAFFIC_TICK);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut logging = true;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                plant.step(&mut lock(&state));
                stats.polls += 1;
                report(&plant, &state, &mut statuses, out);
            }
            _ = flush.tick() => {
                flush_traffic(&mut records, &mut stats, out);
                out.stats(stats);
            }
            entry = log.recv(), if logging => match entry {
                Some(entry) => log_entry(&entry, out),
                None => logging = false,
            },
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = set(&mut plant, &state, point, value, out);
                    out.written(point, result);
                    report(&plant, &state, &mut statuses, out);
                }
                Some(Control::Lock { point, locked }) => match plant.point(point) {
                    Some((id, found)) if !found.kind.writable() => {
                        plant.held[id] = locked.then(|| read(&lock(&state), found));
                        out.locked(point, locked);
                    }
                    // Outputs hold whatever they were last commanded: nothing to lock.
                    _ => out.locked(point, false),
                },
                Some(Control::Dnp3(action)) => {
                    let result = match action {
                        Dnp3Action::SetOnline { point, online } => {
                            set_online(&plant, &state, point, online)
                        }
                        _ => Err("An outstation answers polls and commands; it sends none".into()),
                    };
                    out.dnp3_done(action, result);
                    report(&plant, &state, &mut statuses, out);
                }
                Some(Control::Relinquish { point }) => {
                    out.written(point, Err("Simulators release a point with Release".into()));
                }
                Some(_) => {}
            },
        }
    }
    let _ = shutdown.send(true);
    tokio::time::sleep(RELEASE).await;
    flush_traffic(&mut records, &mut stats, out);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn fail(out: &Emitter, message: String) {
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

type Started = (
    Shared,
    mpsc::UnboundedReceiver<OutstationLogEntry>,
    watch::Sender<bool>,
);

/// Listen on the spec's port, retrying for [`BIND_RETRY`] while a closing
/// session may still hold it. `Ok(None)` when the app closed the session
/// meanwhile.
async fn start(
    config: OutstationConfig,
    tap: mpsc::UnboundedSender<TrafficRecord>,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<Started>, String> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        match mt_dnp3::start_outstation_server_with_tap(config.clone(), Some(tap.clone())).await {
            Ok(started) => return Ok(Some(started)),
            Err(error) => {
                let kind = error
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind);
                let message = match kind {
                    Some(std::io::ErrorKind::AddrInUse) => {
                        if Instant::now() < deadline {
                            tokio::select! {
                                () = tokio::time::sleep(BIND_RETRY_STEP) => continue,
                                () = until_closed(control, out) => return Ok(None),
                            }
                        }
                        format!("TCP port {} is in use by another program", config.port)
                    }
                    Some(std::io::ErrorKind::PermissionDenied) => format!(
                        "Not allowed to listen on TCP port {}: pick a port above 1023",
                        config.port
                    ),
                    Some(std::io::ErrorKind::AddrNotAvailable) => {
                        format!("{} is not an address of this computer", config.bind_addr)
                    }
                    _ => format!(
                        "Cannot listen on {}:{}: {error}",
                        config.bind_addr, config.port
                    ),
                };
                return Err(message);
            }
        }
    }
}

/// Wait for the app to close the session before the outstation listens,
/// refusing writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. } | Control::Relinquish { point }) => {
                out.written(point, Err("The outstation is not listening yet".into()));
            }
            Some(Control::Dnp3(action)) => {
                out.dnp3_done(action, Err("The outstation is not listening yet".into()));
            }
            Some(_) => {}
        }
    }
}

/// Set a point from the app: an output takes the value as a command would
/// (and reports an event), an input is held at it.
fn set(
    plant: &mut Plant,
    state: &Shared,
    point: u32,
    value: f64,
    out: &Emitter,
) -> Result<(), String> {
    let Some((id, found)) = plant.point(point) else {
        return Err(format!("No point {point}"));
    };
    if !value.is_finite() {
        return Err("Set a number".into());
    }
    if found.kind.is_binary() && value != 0.0 && value != 1.0 {
        return Err("Set 1 for ON, 0 for OFF".into());
    }
    if found.kind == Dnp3PointKind::Counter && value < 0.0 {
        return Err("A counter counts up from 0".into());
    }
    let mut state = lock(state);
    let index = usize::from(found.index);
    match found.kind {
        Dnp3PointKind::BinaryOutput => {
            state.set_binary_output(index, value == 1.0);
        }
        Dnp3PointKind::AnalogOutput => {
            state.set_analog_output(index, value);
        }
        _ => {
            plant.held[id] = Some(value);
            write(&mut state, found, value);
            state.detect_events();
            out.locked(point, true);
        }
    }
    Ok(())
}

/// Take a point offline (COMM_LOST) or bring it back, as a field device
/// losing its sensor would: masters see the flags and an event.
fn set_online(plant: &Plant, state: &Shared, point: u32, online: bool) -> Result<String, String> {
    let Some((_, found)) = plant.point(point) else {
        return Err(format!("No point {point}"));
    };
    let flags = if online {
        flag::ONLINE
    } else {
        flag::COMM_LOST
    };
    if lock(state).set_flags(found.kind.group(), found.index, flags) {
        Ok(format!(
            "{} {}: {}",
            point_label(found.kind, found.index),
            found.label,
            flags_text(found.kind, Some(flags))
        ))
    } else {
        Err(format!(
            "The outstation does not serve {}",
            point_label(found.kind, found.index)
        ))
    }
}

/// Send the plant's values, and the status text of points whose flags changed.
fn report(plant: &Plant, state: &Shared, statuses: &mut [String], out: &Emitter) {
    let state = lock(state);
    let mut texts = Vec::new();
    let readings = plant
        .plant
        .points
        .iter()
        .enumerate()
        .map(|(id, point)| {
            let flags = state.flags(point.kind.group(), point.index);
            let text = format!("Flags: {}", flags_text(point.kind, Some(flags)));
            if statuses[id] != text {
                statuses[id].clone_from(&text);
                texts.push(PointText {
                    point: u32::try_from(id).unwrap_or(u32::MAX),
                    text,
                });
            }
            let value = read(&state, point);
            if value.is_finite() {
                Reading {
                    value,
                    raw: [0; 4],
                    raw_len: 0,
                    quality: if flags_good(point.kind, Some(flags)) {
                        Quality::Good
                    } else {
                        Quality::Bad
                    },
                }
            } else {
                Reading::bad()
            }
        })
        .collect();
    drop(state);
    out.values(ValueFrame {
        at_ms: now_ms(),
        latency_us: 0,
        readings,
    });
    if !texts.is_empty() {
        out.payloads(texts);
    }
}

fn flush_traffic(
    records: &mut mpsc::UnboundedReceiver<TrafficRecord>,
    stats: &mut Stats,
    out: &Emitter,
) {
    let mut frames = Vec::new();
    while let Ok(record) = records.try_recv() {
        frames.push(traffic(&record));
    }
    stats.frames += frames.len() as u64;
    out.traffic(frames);
}

/// The server's log, without what the traffic monitor already shows.
fn log_entry(entry: &OutstationLogEntry, out: &Emitter) {
    let message = entry.message.as_str();
    if message.starts_with("Received function")
        || message.starts_with("Class ")
        || message.starts_with("Link ")
    {
        return;
    }
    let level = if message.starts_with("Ignored frame") || message.contains("not supported") {
        LogLevel::Warn
    } else {
        LogLevel::Info
    };
    out.log(level, message);
}

/// The plant's points. The app may set any of them: forcing an input is
/// what a simulator is for, and an output takes a value as a command would.
fn points(plant: &Dnp3Plant) -> Vec<PointInfo> {
    plant
        .points
        .iter()
        .enumerate()
        .map(|(id, point)| PointInfo {
            id: u32::try_from(id).unwrap_or(u32::MAX),
            name: point.label.clone(),
            address: point_label(point.kind, point.index),
            data_type: data_type(point.kind).into(),
            unit: point.unit.clone(),
            writable: true,
            boolean: point.kind.is_binary(),
            states: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feeder() -> &'static Dnp3Plant {
        templates::find_dnp3("feeder").expect("the feeder plant")
    }

    fn id_of(plant: &Dnp3Plant, key: &str) -> u32 {
        let id = plant
            .points
            .iter()
            .position(|point| point.key == key)
            .expect("a plant point");
        u32::try_from(id).expect("a point id")
    }

    fn value_of(plant: &Plant, state: &Shared, key: &str) -> f64 {
        let point = plant.key_point(key).expect("a plant point");
        read(&lock(state), point)
    }

    /// The outstation's state for `plant`, served on a free loopback port.
    async fn served(plant: &Plant) -> Shared {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .expect("a free port");
        let counts = plant.counts();
        let (state, _log, _shutdown) = mt_dnp3::start_outstation_server(OutstationConfig {
            port,
            bind_addr: "127.0.0.1".into(),
            master_address: 1,
            outstation_address: 10,
            analog_count: u16::try_from(counts[1]).expect("analog count"),
            binary_count: u16::try_from(counts[0]).expect("binary count"),
            counter_count: u16::try_from(counts[2]).expect("counter count"),
        })
        .await
        .expect("the outstation listens");
        {
            let mut served = lock(&state);
            plant.start(&mut served);
        }
        state
    }

    #[test]
    fn counts_cover_every_index_of_each_kind() {
        let plant = Plant::new(feeder());
        // 4 binary inputs, 11 analog inputs, 2 counters, 2 binary and 2 analog outputs.
        assert_eq!(plant.counts(), [4, 11, 2, 2, 2]);
    }

    #[tokio::test]
    async fn tripping_the_breaker_opens_it_and_stops_the_power() {
        let mut plant = Plant::new(feeder());
        let state = served(&plant).await;
        plant.step(&mut lock(&state));
        assert_eq!(value_of(&plant, &state, "breaker"), 1.0);
        assert!(value_of(&plant, &state, "i_a") > 90.0);
        for _ in 0..3 {
            plant.step(&mut lock(&state));
        }
        let energy = value_of(&plant, &state, "kwh");
        assert!(energy > 0.0, "energy counts while the breaker is closed");

        // A master's CROB opens the breaker output; the status input follows.
        assert!(lock(&state).set_binary_output(0, false));
        plant.step(&mut lock(&state));
        assert_eq!(value_of(&plant, &state, "breaker"), 0.0);
        assert_eq!(value_of(&plant, &state, "i_a"), 0.0);
        assert_eq!(value_of(&plant, &state, "mw"), 0.0);
        let stopped = value_of(&plant, &state, "kwh");
        plant.step(&mut lock(&state));
        assert_eq!(
            value_of(&plant, &state, "kwh"),
            stopped,
            "the counter stops"
        );
        // Voltage is on the bus side: it stays.
        assert!(value_of(&plant, &state, "v_ab") > 13.0);
        // The change is an event a master's next class poll gets.
        assert!(
            lock(&state)
                .events
                .iter()
                .any(|event| event.group == 1 && event.index == 0)
        );

        // The regulator follows its setpoint.
        assert!(lock(&state).set_analog_output(0, 14.4));
        plant.step(&mut lock(&state));
        assert!((value_of(&plant, &state, "reg_voltage") - 14.4).abs() < 0.5);
    }

    #[tokio::test]
    async fn held_inputs_stay_put_and_offline_points_read_bad() {
        let plant_def = feeder();
        let mut plant = Plant::new(plant_def);
        let state = served(&plant).await;
        let (sender, events) = async_channel::unbounded();
        let out = Emitter::new(vt_protocol::SessionId(1), sender);

        let freq = id_of(plant_def, "freq");
        set(&mut plant, &state, freq, 49.5, &out).expect("a frequency to hold");
        for _ in 0..3 {
            plant.step(&mut lock(&state));
        }
        assert_eq!(value_of(&plant, &state, "freq"), 49.5);
        assert!(set(&mut plant, &state, id_of(plant_def, "breaker"), 0.5, &out).is_err());
        // Setting an output is a command, not a hold.
        set(&mut plant, &state, id_of(plant_def, "mw_limit"), 3.0, &out).expect("an output");
        assert_eq!(value_of(&plant, &state, "mw_limit"), 3.0);
        assert!(plant.held[usize::try_from(id_of(plant_def, "mw_limit")).expect("id")].is_none());

        let current = id_of(plant_def, "i_b");
        let text = set_online(&plant, &state, current, false).expect("a point to take offline");
        assert_eq!(text, "AI 4 Current B: OFFLINE, COMM_LOST");
        let mut statuses = vec![String::new(); plant_def.points.len()];
        report(&plant, &state, &mut statuses, &out);
        let mut quality = None;
        let mut status = None;
        while let Ok(event) = events.try_recv() {
            match event.kind {
                vt_protocol::EventKind::Values(frame) => {
                    quality = Some(frame.readings[current as usize].quality);
                }
                vt_protocol::EventKind::Payloads(texts) => {
                    status = texts
                        .into_iter()
                        .find(|text| text.point == current)
                        .map(|text| text.text);
                }
                _ => {}
            }
        }
        assert_eq!(quality, Some(Quality::Bad));
        assert_eq!(status.as_deref(), Some("Flags: OFFLINE, COMM_LOST"));
        set_online(&plant, &state, current, true).expect("back online");
        assert_eq!(lock(&state).flags(30, 4), flag::ONLINE);
    }
}
