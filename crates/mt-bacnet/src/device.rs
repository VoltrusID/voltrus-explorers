//! Standalone BACnet/IP **device** simulator.
//!
//! Unlike the client side (`client.rs`), this module *answers* BACnet requests:
//! Who-Is → I-Am, ReadProperty, WriteProperty, ReadPropertyMultiple and
//! SubscribeCOV (with initial + change notifications). It binds a UDP socket and serves a table of objects
//! built from [`SimPointDef`] definitions, ticking their values every second so
//! any external client (YABE, VTS, …) sees live data and can drive writable
//! points.
//!
//! The simulator is deliberately strict: it follows ASHRAE 135 (BVLC/NPDU
//! validation, array and datatype checks, priority arrays, max-APDU limits) so
//! a client that only works against a lenient peer fails here too. The wire
//! encoding is hand-rolled in `wire`; the object model lives in `props`,
//! request handling in `service` and COV in `cov`.

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use bacnet_rs::object::ObjectType;

use serde::{Deserialize, Serialize};

mod cov;
mod props;
mod service;
#[cfg(test)]
mod tests;
mod wire;

use cov::CovState;
use wire::Val;

/// Minimum analog change that triggers a COV notification (also the COV_Increment property).
const COV_INCREMENT: f64 = 0.1;

// ── Public data shapes ──

/// One simulated object definition, supplied by the frontend (single source of
/// truth for templates — avoids duplicating AHU/Chiller/VAV definitions in Rust).
///
/// The object type decides the wire datatype of Present_Value: analog → Real,
/// binary → Enumerated (`is_bool` must be set for exactly the binary types),
/// multi-state → Unsigned (`max` is Number_Of_States, default 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimPointDef {
    pub key: String,
    pub label: String,
    pub object_type: String,
    pub object_instance: u32,
    #[serde(default)]
    pub base: f64,
    #[serde(default)]
    pub variance: f64,
    #[serde(default)]
    pub noise: f64,
    #[serde(default)]
    pub period: u64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub decimals: u32,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub is_bool: bool,
    /// Present value is writable from clients / the control panel. Output
    /// objects are always commandable; for value objects this makes them
    /// commandable (Priority_Array + Relinquish_Default); inputs cannot be writable.
    #[serde(default)]
    pub writable: bool,
    /// Multi-state objects: the name of each state from 1 (State_Text). Empty
    /// for none; otherwise as many as Number_Of_States (`max`).
    #[serde(default)]
    pub state_text: Vec<String>,
}

/// A point's current value as reported to the UI (every tick + on write).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapPoint {
    pub key: String,
    pub label: String,
    pub object_type: String,
    pub object_instance: u32,
    pub unit: Option<String>,
    pub value: f64,
    pub display: String,
    pub writable: bool,
    pub kind: String, // "analog" | "bool"
    pub decimals: u32,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    /// A command, an Out_Of_Service write or a UI value pins the present value:
    /// the simulation leaves it alone until it is relinquished or released.
    #[serde(default)]
    pub held: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapshot {
    pub device_id: u32,
    pub port: u16,
    pub running: bool,
    pub points: Vec<SimSnapPoint>,
}

impl SimSnapshot {
    fn empty(device_id: u32, port: u16) -> Self {
        Self { device_id, port, running: true, points: Vec::new() }
    }
}

/// Present_Value datatype family of a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Analog,
    Binary,
    MultiState,
}

/// Identity of the simulated device as seen on the wire.
#[derive(Debug, Clone, Copy)]
struct DeviceInfo {
    id: u32,
    vendor_id: u16,
}

/// Internal mutable object state, owned solely by the device thread.
struct SimObj {
    key: String,
    label: String,
    ot: ObjectType,
    instance: u32,
    unit: Option<String>,
    units_enum: u32,
    /// Resting value: what the sim drifts around and the Relinquish_Default.
    base: f64,
    variance: f64,
    noise: f64,
    period: u64,
    min: Option<f64>,
    max: Option<f64>,
    decimals: u32,
    kind: Kind,
    /// Number_Of_States (multi-state objects).
    states: u32,
    /// State_Text (multi-state objects), empty when the point has none.
    state_text: Vec<String>,
    /// Present_Value is written through the 16-level priority array.
    commandable: bool,
    /// Effective present value. Stays under the tick loop's control until a
    /// command, an Out_Of_Service write or a forced UI value freezes it.
    current: f64,
    /// A UI-forced value on a non-commandable point; the tick loop leaves it alone.
    forced: bool,
    out_of_service: bool,
    /// Priority_Array slots 1..=16 (index 0 = priority 1). Only used when `commandable`.
    slots: [Option<f64>; 16],
}

impl SimObj {
    /// Wire value of `v` in this object's Present_Value datatype.
    fn typed(&self, v: f64) -> Val {
        match self.kind {
            Kind::Analog => Val::Real(v as f32),
            Kind::Binary => Val::Enum(u32::from(v >= 0.5)),
            Kind::MultiState => Val::Unsigned(v.round().max(1.0) as u64),
        }
    }

    /// Re-derive the effective value from the priority array: highest-priority
    /// command, else the Relinquish_Default.
    fn resolve(&mut self) {
        self.current = self.slots.iter().flatten().next().copied().unwrap_or(self.base);
    }

    /// Does anything pin the present value, so the tick loop must not move it?
    fn frozen(&self) -> bool {
        self.forced || self.out_of_service || self.slots.iter().any(Option::is_some)
    }

    fn normalize(&self, v: f64) -> f64 {
        match self.kind {
            Kind::Analog => v,
            Kind::Binary => f64::from(v >= 0.5),
            Kind::MultiState => v.round().clamp(1.0, f64::from(self.states)),
        }
    }

    /// Apply a value from the control panel: a priority-16 command on
    /// commandable points (so clients' higher-priority commands still win),
    /// otherwise a forced sensor value.
    fn ui_set(&mut self, v: f64) {
        let v = self.normalize(v);
        if self.commandable {
            self.slots[15] = Some(v);
            self.resolve();
        } else {
            self.current = v;
            self.forced = true;
        }
    }

    /// Undo [`Self::ui_set`]: drop the forced value and the priority-16 command,
    /// so the simulation (or a client's command) drives the point again.
    fn ui_release(&mut self) {
        self.forced = false;
        if self.commandable {
            self.slots[15] = None;
            self.resolve();
        }
    }
}

/// Control handle returned by [`start`]; cheap to clone (Arc). Used to push
/// writes from the UI, read the latest snapshot, and stop the device.
#[derive(Clone)]
pub struct SimDevice {
    inner: Arc<Inner>,
}

struct Inner {
    running: AtomicBool,
    port: u16,
    device_id: u32,
    cmd_tx: Mutex<Option<mpsc::Sender<Cmd>>>,
    latest: Mutex<SimSnapshot>,
}

enum Cmd {
    Set { key: String, val: f64 },
    SetBool { key: String, active: bool },
    Release { key: String },
}

type TickFn = Box<dyn Fn(SimSnapshot) + Send>;
type WriteFn = Box<dyn Fn(String, f64) + Send>;
/// Sees every datagram the device receives (`true`) or sends (`false`), with the peer's address.
pub type FrameFn = Box<dyn Fn(bool, SocketAddr, &[u8]) + Send>;

/// Validate the point definitions and build the object table.
fn build_objects(points: &[SimPointDef]) -> Result<Vec<SimObj>, String> {
    #[derive(PartialEq)]
    enum Role {
        Input,
        Output,
        Value,
    }
    let mut objects: Vec<SimObj> = Vec::with_capacity(points.len());
    for p in points {
        let ot = object_type_from_str(&p.object_type)
            .ok_or_else(|| format!("{}: unknown object type '{}'", p.key, p.object_type))?;
        let (kind, role) = match ot {
            ObjectType::AnalogInput => (Kind::Analog, Role::Input),
            ObjectType::AnalogOutput => (Kind::Analog, Role::Output),
            ObjectType::AnalogValue => (Kind::Analog, Role::Value),
            ObjectType::BinaryInput => (Kind::Binary, Role::Input),
            ObjectType::BinaryOutput => (Kind::Binary, Role::Output),
            ObjectType::BinaryValue => (Kind::Binary, Role::Value),
            ObjectType::MultiStateInput => (Kind::MultiState, Role::Input),
            ObjectType::MultiStateOutput => (Kind::MultiState, Role::Output),
            ObjectType::MultiStateValue => (Kind::MultiState, Role::Value),
            _ => return Err(format!("{}: '{}' is not a simulatable point type", p.key, p.object_type)),
        };
        if p.is_bool != (kind == Kind::Binary) {
            return Err(format!("{}: is_bool must be set exactly for binary objects ({})", p.key, p.object_type));
        }
        if role == Role::Input && p.writable {
            return Err(format!("{}: input objects cannot be writable", p.key));
        }
        if objects.iter().any(|o| o.ot == ot && o.instance == p.object_instance) {
            return Err(format!("{}: duplicate object {}:{}", p.key, p.object_type, p.object_instance));
        }
        let states = p.max.map_or(2, |m| m.round().max(1.0) as u32);
        if !p.state_text.is_empty() && (kind != Kind::MultiState || p.state_text.len() != states as usize) {
            return Err(format!("{}: state_text needs one name per state of a multi-state object", p.key));
        }
        let base = match kind {
            Kind::Analog => p.base,
            Kind::Binary => f64::from(p.base >= 0.5),
            Kind::MultiState => p.base.round().clamp(1.0, f64::from(states)),
        };
        objects.push(SimObj {
            key: p.key.clone(),
            label: p.label.clone(),
            ot,
            instance: p.object_instance,
            unit: p.unit.clone(),
            units_enum: unit_enum(p.unit.as_deref()),
            base,
            variance: p.variance,
            noise: p.noise,
            period: p.period.max(1),
            min: p.min,
            max: p.max,
            decimals: p.decimals,
            kind,
            states,
            state_text: p.state_text.clone(),
            commandable: role == Role::Output || (role == Role::Value && p.writable),
            current: base,
            forced: false,
            out_of_service: false,
            slots: [None; 16],
        });
    }
    Ok(objects)
}

/// Bind the UDP socket and spawn the device loop. Returns a [`SimDevice`]
/// control handle. `on_tick` fires every ~1s (and immediately after a write)
/// with the full snapshot; `on_write` fires when a client/UI writes a point.
pub fn start(
    device_id: u32,
    requested_port: u16,
    vendor_id: u16,
    points: Vec<SimPointDef>,
    on_tick: TickFn,
    on_write: WriteFn,
) -> Result<SimDevice, String> {
    let no_traffic: FrameFn = Box::new(|_: bool, _: SocketAddr, _: &[u8]| {});
    start_with_traffic(device_id, requested_port, vendor_id, points, on_tick, on_write, no_traffic)
}

/// [`start`], also handing every received and sent datagram to `on_frame`
/// (for a traffic monitor).
pub fn start_with_traffic(
    device_id: u32,
    requested_port: u16,
    vendor_id: u16,
    points: Vec<SimPointDef>,
    on_tick: TickFn,
    on_write: WriteFn,
    on_frame: FrameFn,
) -> Result<SimDevice, String> {
    let objects = build_objects(&points)?;
    let socket = UdpSocket::bind(("0.0.0.0", requested_port))
        .or_else(|_| UdpSocket::bind("0.0.0.0:0"))
        .map_err(|e| format!("Sim socket bind: {e}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| format!("set_read_timeout: {e}"))?;
    let port = socket
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();

    let (tx, rx) = mpsc::channel::<Cmd>();
    let inner = Arc::new(Inner {
        running: AtomicBool::new(true),
        port,
        device_id,
        cmd_tx: Mutex::new(Some(tx)),
        latest: Mutex::new(SimSnapshot::empty(device_id, port)),
    });

    let inner_loop = Arc::clone(&inner);
    let dev = DeviceInfo { id: device_id, vendor_id };
    std::thread::Builder::new()
        .name("bacnet-sim-device".into())
        .spawn(move || {
            run_loop(inner_loop, socket, rx, dev, objects, on_tick, on_write, on_frame);
        })
        .map_err(|e| format!("spawn device thread: {e}"))?;

    Ok(SimDevice { inner })
}

impl SimDevice {
    pub fn port(&self) -> u16 {
        self.inner.port
    }
    pub fn device_id(&self) -> u32 {
        self.inner.device_id
    }
    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Relaxed)
    }
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::Relaxed);
        // Drop the sender so the loop's `try_recv` sees a disconnect cleanly.
        let _ = self.inner.cmd_tx.lock().map(|mut g| g.take());
    }
    pub fn snapshot(&self) -> SimSnapshot {
        self.inner.latest.lock().map(|g| g.clone()).unwrap_or_else(|_| {
            SimSnapshot::empty(self.inner.device_id, self.inner.port)
        })
    }
    pub fn set_value(&self, key: &str, val: f64) {
        if let Ok(g) = self.inner.cmd_tx.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(Cmd::Set { key: key.to_string(), val });
            }
        }
    }
    pub fn set_bool(&self, key: &str, active: bool) {
        if let Ok(g) = self.inner.cmd_tx.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(Cmd::SetBool { key: key.to_string(), active });
            }
        }
    }
    /// Hand a point back to the simulation: undo a value set from the UI.
    /// Commands a client wrote at other priorities stay in force.
    pub fn release(&self, key: &str) {
        if let Ok(g) = self.inner.cmd_tx.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(Cmd::Release { key: key.to_string() });
            }
        }
    }
}

// ── The device loop ──

#[allow(clippy::too_many_arguments)]
fn run_loop(
    inner: Arc<Inner>,
    socket: UdpSocket,
    rx: mpsc::Receiver<Cmd>,
    dev: DeviceInfo,
    mut objects: Vec<SimObj>,
    on_tick: TickFn,
    on_write: WriteFn,
    on_frame: FrameFn,
) {
    let mut cov = CovState::new();
    let start = Instant::now();
    let mut last_tick = Instant::now();
    let mut buf = [0u8; 1500];

    // Initial snapshot so the panel has data immediately.
    tick_values(&mut objects, &start, false);
    publish(&inner, &objects, &on_tick);

    while inner.running.load(Ordering::Relaxed) {
        // Drain UI commands.
        loop {
            match rx.try_recv() {
                Ok(Cmd::Set { key, val }) => {
                    if let Some(o) = objects.iter_mut().find(|o| o.key == key) {
                        o.ui_set(val);
                        (on_write)(key.clone(), o.current);
                        publish(&inner, &objects, &on_tick);
                    }
                }
                Ok(Cmd::SetBool { key, active }) => {
                    if let Some(o) = objects.iter_mut().find(|o| o.key == key) {
                        o.ui_set(f64::from(active));
                        (on_write)(key.clone(), o.current);
                        publish(&inner, &objects, &on_tick);
                    }
                }
                Ok(Cmd::Release { key }) => {
                    if let Some(o) = objects.iter_mut().find(|o| o.key == key) {
                        o.ui_release();
                        (on_write)(key.clone(), o.current);
                        publish(&inner, &objects, &on_tick);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    inner.running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }

        // Periodic sim tick (~1s).
        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
            tick_values(&mut objects, &start, true);
            publish(&inner, &objects, &on_tick);
        }

        // Receive a request (non-blocking via the 250ms read timeout).
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                on_frame(true, src, &buf[..len]);
                if let Some(reply) = service::handle_packet(
                    &buf[..len],
                    src,
                    dev,
                    &mut objects,
                    &mut cov,
                    Instant::now(),
                    &on_write,
                ) {
                    match socket.send_to(&reply.frame, reply.to) {
                        Ok(_) => on_frame(false, reply.to, &reply.frame),
                        Err(e) => eprintln!("sim: reply to {} failed: {e}", reply.to),
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => eprintln!("sim: recv failed: {e}"),
        }

        // Notify subscribers of anything the tick, a UI command or a client write changed
        // (and send the initial notification queued by a new subscription, after its SimpleAck),
        // and retransmit unacknowledged confirmed notifications.
        cov.check(&objects, dev.id, Instant::now());
        for reply in cov.outbox.drain(..) {
            match socket.send_to(&reply.frame, reply.to) {
                Ok(_) => on_frame(false, reply.to, &reply.frame),
                Err(e) => eprintln!("sim: COV notification to {} failed: {e}", reply.to),
            }
        }
    }

    inner.running.store(false, Ordering::Relaxed);
}

fn publish(inner: &Arc<Inner>, objects: &[SimObj], on_tick: &TickFn) {
    let snap = build_snapshot(inner.device_id, inner.port, true, objects);
    if let Ok(mut g) = inner.latest.lock() {
        *g = snap.clone();
    }
    (on_tick)(snap);
}

fn build_snapshot(device_id: u32, port: u16, running: bool, objects: &[SimObj]) -> SimSnapshot {
    let points = objects
        .iter()
        .map(|o| SimSnapPoint {
            key: o.key.clone(),
            label: o.label.clone(),
            object_type: crate::helpers::object_type_to_string(&o.ot),
            object_instance: o.instance,
            unit: o.unit.clone(),
            value: o.current,
            display: display_value(o),
            writable: o.commandable,
            kind: if o.kind == Kind::Binary { "bool".into() } else { "analog".into() },
            decimals: o.decimals,
            min: o.min,
            max: o.max,
            held: o.frozen(),
        })
        .collect();
    SimSnapshot { device_id, port, running, points }
}

fn display_value(o: &SimObj) -> String {
    if o.kind == Kind::Binary {
        if o.current >= 0.5 { "Active".into() } else { "Inactive".into() }
    } else {
        format!("{:.*}{}", o.decimals as usize, o.current, o.unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default())
    }
}

/// Advance simulated values. When `apply_noise` is true, unpinned analogs
/// drift around their base; binary and multi-state points hold their base.
fn tick_values(objects: &mut [SimObj], start: &Instant, apply_noise: bool) {
    let t = start.elapsed().as_millis() as f64;
    for o in objects.iter_mut() {
        if o.frozen() {
            continue;
        }
        if o.kind != Kind::Analog {
            o.current = o.base;
        } else if apply_noise {
            let mut v = o.base
                + (t / o.period as f64 * std::f64::consts::TAU).sin() * o.variance
                + pseudo_noise(t as u64, &o.key) * o.noise;
            if let Some(min) = o.min {
                v = v.max(min);
            }
            if let Some(max) = o.max {
                v = v.min(max);
            }
            o.current = v;
        }
    }
}

/// Deterministic-ish pseudo noise in [-0.5, 0.5) derived from time + key, so we
/// don't pull in a RNG dependency.
fn pseudo_noise(t: u64, key: &str) -> f64 {
    let mut h: u64 = 0xcbf29ce484222325;
    h ^= t.wrapping_mul(0x9E3779B97F4A7C15);
    for b in key.as_bytes() {
        h = h.wrapping_mul(0x100000001b3) ^ (*b as u64);
    }
    (h % 1000) as f64 / 1000.0 - 0.5
}

// ── Small helpers ──

fn object_type_from_str(s: &str) -> Option<ObjectType> {
    crate::helpers::object_type_from_str(s)
}

/// Map template unit strings to BACnetEngineeringUnits enumerations (clause 21):
/// degrees-celsius 62, degrees-fahrenheit 64, percent 98, pascals 53,
/// kilopascals 54, bars 55, watts 47, kilowatts 48, revolutions-per-minute 104,
/// cubic-feet-per-minute 84, cubic-meters-per-second 85, hertz 27.
/// Unknown/missing → no-units (95).
pub fn unit_enum(unit: Option<&str>) -> u32 {
    match unit {
        Some("°C") => 62,
        Some("°F") => 64,
        Some("%") => 98,
        Some("Pa") => 53,
        Some("kPa") => 54,
        Some("bar") => 55,
        Some("W") => 47,
        Some("kW") => 48,
        Some("RPM") => 104,
        Some("CFM") => 84,
        Some("m³/s") => 85,
        Some("Hz") => 27,
        _ => 95,
    }
}
