//! Standalone BACnet/IP **device** simulator.
//!
//! Unlike the client side (`client.rs`), this module *answers* BACnet requests:
//! Who-Is → I-Am, ReadProperty, WriteProperty, ReadPropertyMultiple and
//! SubscribeCOV (stored). It binds a UDP socket and serves a table of objects
//! built from [`SimPointDef`] definitions, ticking their values every second so
//! any external client (YABE, VTS, …) sees live data and can drive writable
//! points.
//!
//! The frame-building recipe mirrors the client code in `client.rs` /
//! `discovery.rs` (BVLC `0x81` + NPDU + APDU); primitives come from `bacnet-rs`.

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use bacnet_rs::app::{Apdu, MaxApduSize};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_rs::property::{encode_property_value, PropertyValue};
use bacnet_rs::service::{
    ConfirmedServiceChoice, IAmRequest, ReadPropertyRequest, ReadPropertyResponse, RejectReason,
    UnconfirmedServiceChoice, WritePropertyRequest,
};

use serde::{Deserialize, Serialize};

// ── BACnet/IP BVLC function codes ──
const BVLC_ORIG_UNICAST: u8 = 0x0A;

// ── Standard context opening/closing tag bytes (tag N) ──
#[inline]
fn open_tag(n: u8) -> u8 {
    (n << 4) | 0x0E
}
#[inline]
fn close_tag(n: u8) -> u8 {
    (n << 4) | 0x0F
}

// ── Public data shapes ──

/// One simulated object definition, supplied by the frontend (single source of
/// truth for templates — avoids duplicating AHU/Chiller/VAV definitions in Rust).
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
    /// Present value is writable from clients / the control panel.
    #[serde(default)]
    pub writable: bool,
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

/// Internal mutable object state, owned solely by the device thread.
struct SimObj {
    key: String,
    label: String,
    ot: ObjectType,
    instance: u32,
    unit: Option<String>,
    units_enum: u32,
    base: f64,
    variance: f64,
    noise: f64,
    period: u64,
    min: Option<f64>,
    max: Option<f64>,
    decimals: u32,
    is_bool: bool,
    writable: bool,
    /// Current present value. When a write lands this becomes an override that
    /// the tick loop will not overwrite.
    current: f64,
    overridden: bool,
}

impl SimObj {
    fn present_value(&self) -> PropertyValue {
        if self.is_bool {
            PropertyValue::Enumerated(if self.current >= 0.5 { 1 } else { 0 })
        } else {
            PropertyValue::Real(self.current as f32)
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
}

type TickFn = Box<dyn Fn(SimSnapshot) + Send>;
type WriteFn = Box<dyn Fn(String, f64) + Send>;

struct CovSub {
    _proc_id: u32,
    object_type: ObjectType,
    instance: u32,
    _confirmed: bool,
    _expires: Option<Instant>,
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
    std::thread::Builder::new()
        .name("bacnet-sim-device".into())
        .spawn(move || {
            run_loop(inner_loop, socket, rx, device_id, vendor_id, points, on_tick, on_write);
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
}

// ── The device loop ──

fn run_loop(
    inner: Arc<Inner>,
    socket: UdpSocket,
    rx: mpsc::Receiver<Cmd>,
    device_id: u32,
    vendor_id: u16,
    points: Vec<SimPointDef>,
    on_tick: TickFn,
    on_write: WriteFn,
) {
    let mut objects: Vec<SimObj> = points
        .iter()
        .map(|p| {
            let ot = object_type_from_str(&p.object_type).unwrap_or(ObjectType::AnalogValue);
            SimObj {
                key: p.key.clone(),
                label: p.label.clone(),
                ot,
                instance: p.object_instance,
                unit: p.unit.clone(),
                units_enum: unit_enum(p.unit.as_deref()),
                base: p.base,
                variance: p.variance,
                noise: p.noise,
                period: p.period.max(1),
                min: p.min,
                max: p.max,
                decimals: p.decimals,
                is_bool: p.is_bool,
                writable: p.writable,
                current: p.base,
                overridden: false,
            }
        })
        .collect();

    let mut cov_subs: Vec<CovSub> = Vec::new();
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
                        o.current = val;
                        o.overridden = true;
                        (on_write)(key.clone(), val);
                        publish(&inner, &objects, &on_tick);
                    }
                }
                Ok(Cmd::SetBool { key, active }) => {
                    if let Some(o) = objects.iter_mut().find(|o| o.key == key) {
                        o.current = if active { 1.0 } else { 0.0 };
                        o.overridden = true;
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
            // Expire COV subs.
            cov_subs.retain(|s| s._expires.map(|t| t > Instant::now()).unwrap_or(true));
        }

        // Receive a request (non-blocking via the 250ms read timeout).
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                if let Some(resp) = handle_packet(
                    &buf[..len],
                    src,
                    device_id,
                    vendor_id,
                    &mut objects,
                    &mut cov_subs,
                    &on_write,
                ) {
                    let _ = socket.send_to(&resp, src);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => continue,
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
            writable: o.writable,
            kind: if o.is_bool { "bool".into() } else { "analog".into() },
            decimals: o.decimals,
            min: o.min,
            max: o.max,
        })
        .collect();
    SimSnapshot { device_id, port, running, points }
}

fn display_value(o: &SimObj) -> String {
    if o.is_bool {
        if o.current >= 0.5 { "Active".into() } else { "Inactive".into() }
    } else {
        format!("{:.*}{}", o.decimals as usize, o.current, o.unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default())
    }
}

/// Advance simulated values. When `apply_noise` is true, non-overridden analogs
/// drift around their base; bools hold steady.
fn tick_values(objects: &mut [SimObj], start: &Instant, apply_noise: bool) {
    let t = start.elapsed().as_millis() as f64;
    for o in objects.iter_mut() {
        if o.overridden {
            continue;
        }
        if o.is_bool {
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

// ── Packet handling ──

fn handle_packet(
    data: &[u8],
    _src: SocketAddr,
    device_id: u32,
    vendor_id: u16,
    objects: &mut [SimObj],
    cov_subs: &mut Vec<CovSub>,
    on_write: &dyn Fn(String, f64),
) -> Option<Vec<u8>> {
    if data.len() < 4 || data[0] != 0x81 {
        return None;
    }

    // BVLC length sanity (Original-Unicast-NPDU 0x0A or broadcast 0x0B).
    let npdu_start = 4;
    let (npdu, npdu_len) = Npdu::decode(&data[npdu_start..]).ok()?;
    let apdu_start = npdu_start + npdu_len;
    if apdu_start >= data.len() {
        return None;
    }
    let apdu_bytes = &data[apdu_start..];
    let apdu = Apdu::decode(apdu_bytes).ok()?;

    let _ = npdu; // NPDU is trivial (unicast); we reply directly to the sender.

    match apdu {
        Apdu::UnconfirmedRequest { service_choice, service_data: _ } => {
            if service_choice == UnconfirmedServiceChoice::WhoIs {
                build_iam(device_id, vendor_id)
            } else {
                None
            }
        }
        Apdu::ConfirmedRequest { invoke_id, service_choice, service_data, .. } => match service_choice {
            ConfirmedServiceChoice::ReadProperty => {
                build_read_property(device_id, invoke_id, &service_data, objects)
            }
            ConfirmedServiceChoice::ReadPropertyMultiple => {
                build_read_property_multiple(device_id, invoke_id, &service_data, objects)
            }
            ConfirmedServiceChoice::WriteProperty => {
                apply_write(invoke_id, &service_data, objects, on_write)
            }
            ConfirmedServiceChoice::SubscribeCOV => {
                handle_subscribe_cov(invoke_id, &service_data, objects, cov_subs)
            }
            _ => Some(simple_reject(invoke_id, RejectReason::UnrecognizedService)),
        },
        _ => None,
    }
}

// ── I-Am ──

fn build_iam(device_id: u32, vendor_id: u16) -> Option<Vec<u8>> {
    let dev_oid = ObjectIdentifier::new(ObjectType::Device, device_id);
    let iam = IAmRequest::new(dev_oid, MaxApduSize::Up480.size() as u32, Segmentation::NoSegmentation, vendor_id);
    let mut service_data = Vec::new();
    iam.encode(&mut service_data).ok()?;
    let apdu = Apdu::UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::IAm,
        service_data,
    };
    Some(wrap_apdu_unicast(&apdu))
}

// ── ReadProperty ──

fn build_read_property(
    device_id: u32,
    invoke_id: u8,
    service_data: &[u8],
    objects: &[SimObj],
) -> Option<Vec<u8>> {
    let req = ReadPropertyRequest::decode(service_data).ok()?;
    let pid_u32: u32 = property_id_to_u32(&req.property_identifier);
    let array_index = req.property_array_index;

    let values = property_values(device_id, req.object_identifier, pid_u32, array_index, objects);
    if values.is_empty() {
        return Some(error_response(invoke_id, ConfirmedServiceChoice::ReadProperty));
    }
    let resp = ReadPropertyResponse::new(req.object_identifier, pid_u32.into(), values);
    let mut data = Vec::new();
    resp.encode(&mut data).ok()?;
    let apdu = Apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::ReadProperty,
        service_data: data,
    };
    Some(wrap_apdu_unicast(&apdu))
}

// ── ReadPropertyMultiple ──

fn build_read_property_multiple(
    device_id: u32,
    invoke_id: u8,
    service_data: &[u8],
    objects: &[SimObj],
) -> Option<Vec<u8>> {
    // Parse the list of ReadAccessSpecifications (request uses context tags:
    // obj-id ctx0, list wrapped in ctx1, each prop = pid ctx0 [+ array idx ctx1]).
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < service_data.len() {
        let (obj_id, c) = match bacnet_rs::encoding::decode_context_object_id(&service_data[pos..], 0) {
            Ok(v) => v,
            Err(_) => break,
        };
        pos += c;
        // Opening tag 1.
        if let Ok((tag, size, c)) = bacnet_rs::encoding::decode_context_tag(&service_data[pos..]) {
            if tag == 1 {
                pos += c;
                let _ = size;
            } else {
                break;
            }
        } else {
            break;
        }

        // Emit ReadAccessResult header: obj-id ctx0 + open ctx1.
        let oid_bytes = bacnet_rs::encoding::encode_context_object_id(obj_id, 0).ok()?;
        out.extend_from_slice(&oid_bytes);
        out.push(open_tag(1));

        // Iterate properties until closing tag 1.
        loop {
            let (t, _s, c) = bacnet_rs::encoding::decode_context_tag(&service_data[pos..]).ok()?;
            if t == 1 {
                pos += c; // consume close tag 1
                break;
            }
            // property identifier ctx0
            let (pid, c) = match bacnet_rs::encoding::decode_context_enumerated(&service_data[pos..], 0) {
                Ok(v) => v,
                Err(_) => break,
            };
            pos += c;
            // optional array index ctx1
            let mut array_index = None;
            if let Ok((tag, _, _)) = bacnet_rs::encoding::decode_tag(&service_data[pos..]) {
                if matches!(tag, bacnet_rs::encoding::BACnetTag::Context(1)) {
                    if let Ok((idx, c)) = bacnet_rs::encoding::decode_context_unsigned(&service_data[pos..], 1) {
                        array_index = Some(idx);
                        pos += c;
                    }
                }
            }

            let values = property_values(device_id, obj_id, pid, array_index, objects);
            // property id ctx2 + optional array idx ctx3 + value wrapped in ctx4.
            let pid_bytes = bacnet_rs::encoding::encode_context_enumerated(pid, 2).ok()?;
            out.extend_from_slice(&pid_bytes);
            if let Some(idx) = array_index {
                let b = bacnet_rs::encoding::encode_context_unsigned(idx, 3).ok()?;
                out.extend_from_slice(&b);
            }
            out.push(open_tag(4));
            if values.is_empty() {
                // Unknown property → emit Null so parsers stay aligned.
                let _ = encode_property_value(&PropertyValue::Null, &mut out);
            } else {
                for v in values {
                    let _ = encode_property_value(&v, &mut out);
                }
            }
            out.push(close_tag(4));
        }
        out.push(close_tag(1));
    }

    let apdu = Apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::ReadPropertyMultiple,
        service_data: out,
    };
    Some(wrap_apdu_unicast(&apdu))
}

// ── WriteProperty ──

fn apply_write(
    invoke_id: u8,
    service_data: &[u8],
    objects: &mut [SimObj],
    on_write: &dyn Fn(String, f64),
) -> Option<Vec<u8>> {
    let req = match WritePropertyRequest::decode(service_data) {
        Ok(r) => r,
        Err(_) => return Some(error_response(invoke_id, ConfirmedServiceChoice::WriteProperty)),
    };
    let pid_u32 = req.property_identifier;
    let pid = property_id_from_u32(pid_u32);
    if pid != PropertyIdentifier::PresentValue {
        return Some(simple_ack(invoke_id, ConfirmedServiceChoice::WriteProperty));
    }
    if let Some(o) = objects.iter_mut().find(|o| o.ot == req.object_identifier.object_type && o.instance == req.object_identifier.instance) {
        if let Some(v) = decode_present_value(&req.property_value, o.is_bool) {
            o.current = v;
            o.overridden = true;
            (on_write)(o.key.clone(), v);
        }
    }
    Some(simple_ack(invoke_id, ConfirmedServiceChoice::WriteProperty))
}

/// Decode a written present value from its raw application-tagged bytes.
fn decode_present_value(bytes: &[u8], is_bool: bool) -> Option<f64> {
    if bytes.is_empty() {
        return None;
    }
    let tag = bytes[0];
    let tagnum = tag >> 4; // application tag number (high nibble); class bit is clear
    let len = (tag & 0x07) as usize;
    match tagnum {
        4 if bytes.len() >= 5 => {
            // Application Real (0x44) + 4-byte big-endian f32.
            let mut a = [0u8; 4];
            a.copy_from_slice(&bytes[1..5]);
            Some(f32::from_be_bytes(a) as f64)
        }
        2 | 9 if bytes.len() >= 1 + len && len <= 4 => {
            // Application Unsigned (0x2X) or Enumerated (0x9X).
            let mut v: u64 = 0;
            for &b in &bytes[1..1 + len] {
                v = (v << 8) | b as u64;
            }
            if is_bool {
                Some(if v >= 1 { 1.0 } else { 0.0 })
            } else {
                Some(v as f64)
            }
        }
        1 => {
            // Application Boolean — value carried in the low nibble (TRUE=0x11, FALSE=0x10).
            Some(if (tag & 0x07) != 0 { 1.0 } else { 0.0 })
        }
        _ => None,
    }
}

// ── SubscribeCOV (stored only; no notification emission in v1) ──

fn handle_subscribe_cov(
    invoke_id: u8,
    service_data: &[u8],
    objects: &[SimObj],
    cov_subs: &mut Vec<CovSub>,
) -> Option<Vec<u8>> {
    // Layout: proc_id ctx0, obj_id ctx1, [issueConfirmed ctx2 bool], [lifetime ctx3 unsigned].
    let mut pos = 0;
    let proc_id = bacnet_rs::encoding::decode_context_unsigned(service_data, 0)
        .map(|(v, c)| { pos = c; v })
        .unwrap_or(0);
    let (obj_id, c) = bacnet_rs::encoding::decode_context_object_id(&service_data[pos..], 1).ok()?;
    pos += c;
    let mut confirmed = false;
    let mut lifetime: Option<u32> = None;
    while pos < service_data.len() {
        if let Ok((tag, _, c)) = bacnet_rs::encoding::decode_tag(&service_data[pos..]) {
            match tag {
                bacnet_rs::encoding::BACnetTag::Context(2) => {
                    // boolean: ctx tag 2 len1; value in next byte (0/1).
                    if pos + 1 < service_data.len() {
                        confirmed = service_data[pos + 1] != 0;
                    }
                    pos += c;
                }
                bacnet_rs::encoding::BACnetTag::Context(3) => {
                    if let Ok((v, c2)) = bacnet_rs::encoding::decode_context_unsigned(&service_data[pos..], 3) {
                        lifetime = Some(v);
                        pos += c2;
                    } else {
                        pos += c;
                    }
                }
                _ => {
                    pos += c;
                }
            }
        } else {
            break;
        }
    }
    let expires = lifetime.filter(|&l| l > 0).map(|l| Instant::now() + Duration::from_secs(l as u64));
    if objects.iter().any(|o| o.ot == obj_id.object_type && o.instance == obj_id.instance) {
        cov_subs.retain(|s| !(s.object_type == obj_id.object_type && s.instance == obj_id.instance));
        cov_subs.push(CovSub {
            _proc_id: proc_id,
            object_type: obj_id.object_type,
            instance: obj_id.instance,
            _confirmed: confirmed,
            _expires: expires,
        });
    }
    Some(simple_ack(invoke_id, ConfirmedServiceChoice::SubscribeCOV))
}

// ── Property value resolver (shared by RP + RPM) ──

fn property_values(
    device_id: u32,
    obj_id: ObjectIdentifier,
    pid_u32: u32,
    array_index: Option<u32>,
    objects: &[SimObj],
) -> Vec<PropertyValue> {
    // Device object.
    if obj_id.object_type == ObjectType::Device && obj_id.instance == device_id {
        return device_property(pid_u32, array_index, device_id, objects);
    }
    let Some(o) = objects.iter().find(|o| o.ot == obj_id.object_type && o.instance == obj_id.instance) else {
        return Vec::new();
    };
    let pid = property_id_from_u32(pid_u32);
    match pid {
        PropertyIdentifier::PresentValue => vec![o.present_value()],
        PropertyIdentifier::ObjectName => vec![PropertyValue::CharacterString(o.label.clone())],
        PropertyIdentifier::Description => vec![PropertyValue::CharacterString(o.label.clone())],
        PropertyIdentifier::Units if !o.is_bool => vec![PropertyValue::Enumerated(o.units_enum)],
        PropertyIdentifier::Units => vec![PropertyValue::Enumerated(0)],
        PropertyIdentifier::StatusFlags => vec![PropertyValue::Unsigned(0)], // no flags
        PropertyIdentifier::ObjectType => vec![PropertyValue::Enumerated(object_type_enum(&o.ot))],
        PropertyIdentifier::ObjectIdentifier => vec![PropertyValue::ObjectIdentifier(obj_id)],
        _ => Vec::new(),
    }
}

fn device_property(
    pid_u32: u32,
    array_index: Option<u32>,
    device_id: u32,
    objects: &[SimObj],
) -> Vec<PropertyValue> {
    let pid = property_id_from_u32(pid_u32);
    match pid {
        PropertyIdentifier::ObjectList => {
            // index 0 → count; index N → the Nth object id; no index → whole list.
            let ids: Vec<ObjectIdentifier> = std::iter::once(ObjectIdentifier::new(ObjectType::Device, device_id))
                .chain(objects.iter().map(|o| ObjectIdentifier::new(o.ot, o.instance)))
                .collect();
            match array_index {
                None => ids.into_iter().map(PropertyValue::ObjectIdentifier).collect(),
                Some(0) => vec![PropertyValue::Unsigned(ids.len() as u64)],
                Some(i) if (i as usize) <= ids.len() => vec![PropertyValue::ObjectIdentifier(ids[(i as usize) - 1])],
                _ => Vec::new(),
            }
        }
        PropertyIdentifier::ObjectName => vec![PropertyValue::CharacterString(format!("SimDevice-{}", device_id))],
        PropertyIdentifier::Description => vec![PropertyValue::CharacterString("MacTools BACnet Simulator".into())],
        PropertyIdentifier::VendorName => vec![PropertyValue::CharacterString("JoyoDigitama".into())],
        PropertyIdentifier::VendorIdentifier => vec![PropertyValue::Unsigned(0)],
        PropertyIdentifier::ModelName => vec![PropertyValue::CharacterString("Sim".into())],
        PropertyIdentifier::FirmwareRevision => vec![PropertyValue::CharacterString("1.0.0".into())],
        PropertyIdentifier::ProtocolVersion => vec![PropertyValue::Enumerated(1)],
        PropertyIdentifier::SystemStatus => vec![PropertyValue::Enumerated(0)], // operational
        PropertyIdentifier::ObjectIdentifier => vec![PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::Device, device_id))],
        PropertyIdentifier::ObjectType => vec![PropertyValue::Enumerated(8)], // Device
        _ => Vec::new(),
    }
}

// ── APDU/BVLC framing ──

fn wrap_apdu_unicast(apdu: &Apdu) -> Vec<u8> {
    let apdu_data = apdu.encode();
    let npdu = Npdu::new(); // plain data NPDU, no reply expected
    let mut message = npdu.encode();
    message.extend_from_slice(&apdu_data);

    let mut frame = vec![0x81, BVLC_ORIG_UNICAST, 0x00, 0x00];
    frame.extend_from_slice(&message);
    let len = frame.len() as u16;
    frame[2] = (len >> 8) as u8;
    frame[3] = (len & 0xFF) as u8;
    frame
}

fn simple_ack(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Vec<u8> {
    wrap_apdu_unicast(&Apdu::SimpleAck { invoke_id, service_choice: service_choice as u8 })
}

fn simple_reject(invoke_id: u8, reason: RejectReason) -> Vec<u8> {
    wrap_apdu_unicast(&Apdu::Reject { invoke_id, reject_reason: reason })
}

fn error_response(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Vec<u8> {
    // error_class=Object(0), error_code=UnknownObject(1) — good enough signal.
    wrap_apdu_unicast(&Apdu::Error { invoke_id, service_choice, error_class: 0, error_code: 1 })
}

// ── Small helpers ──

fn object_type_from_str(s: &str) -> Option<ObjectType> {
    crate::helpers::object_type_from_str(s)
}

fn object_type_enum(ot: &ObjectType) -> u32 {
    let v: u32 = (*ot).into();
    v
}

/// Map a [`PropertyIdentifier`] to its wire u32. `bacnet-rs` exposes the enum
/// discriminants directly.
fn property_id_to_u32(p: &PropertyIdentifier) -> u32 {
    // ReadPropertyRequest stores PropertyIdentifier directly.
    let v: u32 = (*p).into();
    v
}

fn property_id_from_u32(v: u32) -> PropertyIdentifier {
    // The enum is a C-like enum with fixed discriminants; reconstruct via match
    // on the common values we serve. Falls back to PresentValue.
    match v {
        85 => PropertyIdentifier::PresentValue,
        77 => PropertyIdentifier::ObjectName,
        28 => PropertyIdentifier::Description,
        117 => PropertyIdentifier::Units,
        76 => PropertyIdentifier::ObjectList,
        111 => PropertyIdentifier::StatusFlags,
        79 => PropertyIdentifier::ObjectType,
        75 => PropertyIdentifier::ObjectIdentifier,
        121 => PropertyIdentifier::VendorName,
        120 => PropertyIdentifier::VendorIdentifier,
        70 => PropertyIdentifier::ModelName,
        44 => PropertyIdentifier::FirmwareRevision,
        98 => PropertyIdentifier::ProtocolVersion,
        112 => PropertyIdentifier::SystemStatus,
        _ => PropertyIdentifier::PresentValue,
    }
}

/// Best-effort mapping of template unit strings to BACnet engineering-units
/// enumerations. Unknown/missing → 0 (no units).
fn unit_enum(unit: Option<&str>) -> u32 {
    match unit {
        Some("°C") => 62,        // degrees-celsius
        Some("%") => 98,         // percent
        Some("Pa") => 80,        // pascals? treated loosely
        Some("kPa") => 83,
        Some("kW") => 179,
        Some("W") => 178,
        Some("RPM") => 184,
        Some("CFM") | Some("m³/s") => 166,
        Some("°F") => 64,
        Some("bar") => 112,
        Some("Hz") => 237,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_rs::app::MaxSegments;
    use bacnet_rs::service::{
        PropertyReference, ReadAccessSpecification, ReadPropertyMultipleRequest,
        ReadPropertyMultipleResponse,
    };

    fn pt(key: &str, ot: &str, inst: u32, base: f64, writable: bool, is_bool: bool) -> SimPointDef {
        SimPointDef {
            key: key.into(),
            label: key.into(),
            object_type: ot.into(),
            object_instance: inst,
            base,
            variance: 0.0,
            noise: 0.0,
            period: 1000,
            min: None,
            max: None,
            decimals: 1,
            unit: Some("°C".into()),
            is_bool,
            writable,
        }
    }

    fn build_objects(points: &[SimPointDef]) -> Vec<SimObj> {
        points
            .iter()
            .map(|p| {
                SimObj {
                    key: p.key.clone(),
                    label: p.label.clone(),
                    ot: object_type_from_str(&p.object_type).unwrap_or(ObjectType::AnalogValue),
                    instance: p.object_instance,
                    unit: p.unit.clone(),
                    units_enum: unit_enum(p.unit.as_deref()),
                    base: p.base,
                    variance: p.variance,
                    noise: p.noise,
                    period: p.period.max(1),
                    min: p.min,
                    max: p.max,
                    decimals: p.decimals,
                    is_bool: p.is_bool,
                    writable: p.writable,
                    current: p.base,
                    overridden: false,
                }
            })
            .collect()
    }

    /// Wrap an APDU in BVLC(Original-Unicast) + a plain NPDU — the inbound frame
    /// shape a BACnet client sends.
    fn frame_in(apdu: &Apdu) -> Vec<u8> {
        let apdu_bytes = apdu.encode();
        let mut msg = Npdu::new().encode();
        msg.extend_from_slice(&apdu_bytes);
        let mut frame = vec![0x81, BVLC_ORIG_UNICAST, 0x00, 0x00];
        frame.extend_from_slice(&msg);
        let len = frame.len() as u16;
        frame[2] = (len >> 8) as u8;
        frame[3] = (len & 0xFF) as u8;
        frame
    }

    fn confirmed(service_choice: ConfirmedServiceChoice, service_data: Vec<u8>) -> Apdu {
        Apdu::ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: MaxSegments::Unspecified,
            max_response_size: MaxApduSize::Up1476,
            invoke_id: 1,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_data,
        }
    }

    /// Extract the ComplexAck service_data from a response frame.
    fn ack_service_data(resp: &[u8]) -> Vec<u8> {
        let (_npdu, nlen) = Npdu::decode(&resp[4..]).unwrap();
        let apdu = Apdu::decode(&resp[4 + nlen..]).unwrap();
        match apdu {
            Apdu::ComplexAck { service_data, .. } => service_data,
            other => panic!("expected ComplexAck, got {other:?}"),
        }
    }

    fn is_simple_ack(resp: &[u8]) -> bool {
        let (_npdu, nlen) = Npdu::decode(&resp[4..]).unwrap();
        matches!(Apdu::decode(&resp[4 + nlen..]).unwrap(), Apdu::SimpleAck { .. })
    }

    fn as_real(v: &PropertyValue) -> Option<f32> {
        if let PropertyValue::Real(f) = v { Some(*f) } else { None }
    }

    #[test]
    fn read_property_and_rpm_and_write() {
        let points = vec![
            pt("temp", "analogInput", 1, 22.5, false, false),
            pt("sp", "analogValue", 1, 23.0, true, false),
        ];
        let device_id = 991u32;
        let mut objects = build_objects(&points);
        let mut cov = Vec::<CovSub>::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        // ── ReadProperty on analogInput:1 presentValue ──
        let temp_oid = ObjectIdentifier::new(ObjectType::AnalogInput, 1);
        let rp = ReadPropertyRequest::new(temp_oid, PropertyIdentifier::PresentValue);
        let mut sd = Vec::new();
        rp.encode(&mut sd).unwrap();
        let resp = handle_packet(&frame_in(&confirmed(ConfirmedServiceChoice::ReadProperty, sd)), src, device_id, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        let parsed = ReadPropertyResponse::decode(&ack_service_data(&resp)).unwrap();
        assert_eq!(as_real(&parsed.property_values[0]), Some(22.5));

        // ── ReadPropertyMultiple (two objects) — structural decode ──
        let specs = vec![
            ReadAccessSpecification::new(temp_oid, vec![PropertyReference::new(PropertyIdentifier::PresentValue)]),
            ReadAccessSpecification::new(ObjectIdentifier::new(ObjectType::AnalogValue, 1), vec![PropertyReference::new(PropertyIdentifier::PresentValue)]),
        ];
        let mut sd = Vec::new();
        ReadPropertyMultipleRequest::new(specs).encode(&mut sd).unwrap();
        let resp = handle_packet(&frame_in(&confirmed(ConfirmedServiceChoice::ReadPropertyMultiple, sd)), src, device_id, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        let rpm = ReadPropertyMultipleResponse::decode(&ack_service_data(&resp)).unwrap();
        assert_eq!(rpm.read_access_results.len(), 2);
        let v0 = &rpm.read_access_results[0].results[0].value;
        let v1 = &rpm.read_access_results[1].results[0].value;
        use bacnet_rs::service::PropertyResultValue;
        match v0 {
            PropertyResultValue::Value(vs) => assert_eq!(as_real(&vs[0]), Some(22.5)),
            other => panic!("rpm[0] value {other:?}"),
        }
        match v1 {
            PropertyResultValue::Value(vs) => assert_eq!(as_real(&vs[0]), Some(23.0)),
            other => panic!("rpm[1] value {other:?}"),
        }

        // ── WriteProperty to the setpoint, then ReadProperty it back ──
        let sp_oid = ObjectIdentifier::new(ObjectType::AnalogValue, 1);
        let pid: u32 = PropertyIdentifier::PresentValue.into();
        let mut value_bytes = vec![0x44];
        value_bytes.extend_from_slice(&(21.0f32).to_be_bytes());
        let wp = WritePropertyRequest::new(sp_oid, pid, value_bytes);
        let mut sd = Vec::new();
        wp.encode(&mut sd).unwrap();
        let resp = handle_packet(&frame_in(&confirmed(ConfirmedServiceChoice::WriteProperty, sd)), src, device_id, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert!(is_simple_ack(&resp), "write should return SimpleAck");

        let rp = ReadPropertyRequest::new(sp_oid, PropertyIdentifier::PresentValue);
        let mut sd = Vec::new();
        rp.encode(&mut sd).unwrap();
        let resp = handle_packet(&frame_in(&confirmed(ConfirmedServiceChoice::ReadProperty, sd)), src, device_id, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        let parsed = ReadPropertyResponse::decode(&ack_service_data(&resp)).unwrap();
        assert_eq!(as_real(&parsed.property_values[0]), Some(21.0), "written value should read back");
    }
}
