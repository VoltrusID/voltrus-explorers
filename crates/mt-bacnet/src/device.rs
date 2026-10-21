//! Standalone BACnet/IP **device** simulator.
//!
//! Unlike the client side (`client.rs`), this module *answers* BACnet requests:
//! Who-Is → I-Am, ReadProperty, WriteProperty, ReadPropertyMultiple and
//! SubscribeCOV (with initial + change notifications). It binds a UDP socket and serves a table of objects
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
    UnconfirmedServiceChoice,
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
    proc_id: u32,
    object_type: ObjectType,
    instance: u32,
    confirmed: bool,
    expires: Option<Instant>,
    subscriber: SocketAddr,
    /// Value at the time of the last notification (COV change reference).
    last_value: f64,
}

/// BACnet Error PDU class/code pair (ASHRAE 135 Clause 18).
#[derive(Debug, Clone, Copy)]
struct SimError {
    class: u8,
    code: u8,
}

const ERR_UNKNOWN_OBJECT: SimError = SimError { class: 1, code: 31 }; // object / unknown-object
const ERR_UNKNOWN_PROPERTY: SimError = SimError { class: 2, code: 32 }; // property / unknown-property
const ERR_WRITE_ACCESS_DENIED: SimError = SimError { class: 2, code: 40 }; // property / write-access-denied
const ERR_INVALID_DATA_TYPE: SimError = SimError { class: 2, code: 9 }; // property / invalid-data-type
const ERR_VALUE_OUT_OF_RANGE: SimError = SimError { class: 2, code: 37 }; // property / value-out-of-range
const ERR_INVALID_ARRAY_INDEX: SimError = SimError { class: 2, code: 42 }; // property / invalid-array-index
const ERR_NOT_AN_ARRAY: SimError = SimError { class: 2, code: 50 }; // property / property-is-not-an-array
const ERR_SERVICE_REQUEST_DENIED: SimError = SimError { class: 5, code: 29 }; // services / service-request-denied

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
                    &mut cov,
                    &on_write,
                ) {
                    if let Err(e) = socket.send_to(&resp, src) {
                        eprintln!("sim: reply to {src} failed: {e}");
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => eprintln!("sim: recv failed: {e}"),
        }

        // Notify subscribers of anything the tick, a UI command or a client write changed
        // (and send the initial notification queued by a new subscription, after its SimpleAck).
        cov.check(&objects, device_id);
        for (addr, frame) in cov.outbox.drain(..) {
            if let Err(e) = socket.send_to(&frame, addr) {
                eprintln!("sim: COV notification to {addr} failed: {e}");
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
    src: SocketAddr,
    device_id: u32,
    vendor_id: u16,
    objects: &mut [SimObj],
    cov: &mut CovState,
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
                apply_write(device_id, invoke_id, &service_data, objects, on_write)
            }
            ConfirmedServiceChoice::SubscribeCOV => {
                handle_subscribe_cov(invoke_id, &service_data, src, device_id, objects, cov)
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
    let Ok(req) = ReadPropertyRequest::decode(service_data) else {
        return Some(simple_reject(invoke_id, RejectReason::InvalidParameterDataType));
    };
    let pid_u32: u32 = property_id_to_u32(&req.property_identifier);
    let array_index = req.property_array_index;

    let values = match property_values(device_id, req.object_identifier, pid_u32, array_index, objects) {
        Ok(v) => v,
        Err(e) => return Some(error_response(invoke_id, ConfirmedServiceChoice::ReadProperty, e)),
    };
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

            let pid_bytes = bacnet_rs::encoding::encode_context_enumerated(pid, 2).ok()?;
            out.extend_from_slice(&pid_bytes);
            if let Some(idx) = array_index {
                let b = bacnet_rs::encoding::encode_context_unsigned(idx, 3).ok()?;
                out.extend_from_slice(&b);
            }
            match property_values(device_id, obj_id, pid, array_index, objects) {
                Ok(values) => {
                    // [4] propertyValue
                    out.push(open_tag(4));
                    for v in values {
                        let _ = encode_property_value(&v, &mut out);
                    }
                    out.push(close_tag(4));
                }
                Err(e) => {
                    // [5] propertyAccessError: Enumerated class, Enumerated code.
                    out.push(open_tag(5));
                    bacnet_rs::encoding::encode_enumerated(&mut out, e.class as u32);
                    bacnet_rs::encoding::encode_enumerated(&mut out, e.code as u32);
                    out.push(close_tag(5));
                }
            }
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

/// A decoded WriteProperty request. Only a single primitive application-tagged
/// value is supported (everything the simulator can store).
struct WriteReq {
    object: ObjectIdentifier,
    property: u32,
    array_index: Option<u32>,
    /// Raw application-tagged value bytes (between the [3] opening/closing tags).
    value: Vec<u8>,
}

/// Parse a tag header from `b`: `(tag_number, content_len, header_len)`.
/// Opening/closing context tags and extended tag numbers are rejected.
fn parse_tag(b: &[u8]) -> Option<(u8, usize, usize)> {
    let first = *b.first()?;
    let num = first >> 4;
    let len_bits = first & 0x07;
    if num == 0x0F || (first & 0x08 != 0 && len_bits >= 6) {
        return None;
    }
    if len_bits == 5 {
        let l = *b.get(1)?;
        if l >= 254 {
            return None;
        }
        return Some((num, l as usize, 2));
    }
    // Application Boolean carries its value in the length bits and has no content.
    let content = if first & 0x08 == 0 && num == 1 { 0 } else { len_bits as usize };
    Some((num, content, 1))
}

fn decode_write_request(data: &[u8]) -> Result<WriteReq, RejectReason> {
    let bad = RejectReason::InvalidParameterDataType;
    let (object, c) = bacnet_rs::encoding::decode_context_object_id(data, 0).map_err(|_| RejectReason::MissingRequiredParameter)?;
    let mut pos = c;
    let (property, c) = bacnet_rs::encoding::decode_context_unsigned(&data[pos..], 1).map_err(|_| RejectReason::MissingRequiredParameter)?;
    pos += c;
    let mut array_index = None;
    if data.get(pos).is_some_and(|b| b & 0xF8 == 0x28) {
        let (idx, c) = bacnet_rs::encoding::decode_context_unsigned(&data[pos..], 2).map_err(|_| bad)?;
        array_index = Some(idx);
        pos += c;
    }
    if data.get(pos) != Some(&0x3E) {
        return Err(RejectReason::MissingRequiredParameter);
    }
    pos += 1;
    let (_, content, hdr) = parse_tag(&data[pos..]).ok_or(bad)?;
    let end = pos + hdr + content;
    if data.get(end) != Some(&0x3F) {
        return Err(bad);
    }
    let value = data[pos..end].to_vec();
    pos = end + 1;
    if pos < data.len() {
        // Optional [4] priority, 1..=16.
        if data[pos] & 0xF8 != 0x48 {
            return Err(bad);
        }
        let (prio, c) = bacnet_rs::encoding::decode_context_unsigned(&data[pos..], 4).map_err(|_| bad)?;
        if pos + c != data.len() {
            return Err(bad);
        }
        if !(1..=16).contains(&prio) {
            return Err(RejectReason::ParameterOutOfRange);
        }
    }
    Ok(WriteReq { object, property, array_index, value })
}

fn apply_write(
    device_id: u32,
    invoke_id: u8,
    service_data: &[u8],
    objects: &mut [SimObj],
    on_write: &dyn Fn(String, f64),
) -> Option<Vec<u8>> {
    let sc = ConfirmedServiceChoice::WriteProperty;
    let req = match decode_write_request(service_data) {
        Ok(r) => r,
        Err(reason) => return Some(simple_reject(invoke_id, reason)),
    };
    if req.object.object_type == ObjectType::Device && req.object.instance == device_id {
        if property_id_from_u32(req.property).is_none() {
            return Some(error_response(invoke_id, sc, ERR_UNKNOWN_PROPERTY));
        }
        return Some(error_response(invoke_id, sc, ERR_WRITE_ACCESS_DENIED));
    }
    let Some(o) = objects
        .iter_mut()
        .find(|o| o.ot == req.object.object_type && o.instance == req.object.instance)
    else {
        return Some(error_response(invoke_id, sc, ERR_UNKNOWN_OBJECT));
    };
    let Some(pid) = property_id_from_u32(req.property) else {
        return Some(error_response(invoke_id, sc, ERR_UNKNOWN_PROPERTY));
    };
    if req.array_index.is_some() {
        return Some(error_response(invoke_id, sc, ERR_NOT_AN_ARRAY));
    }
    if pid != PropertyIdentifier::PresentValue || !o.writable {
        return Some(error_response(invoke_id, sc, ERR_WRITE_ACCESS_DENIED));
    }
    match decode_present_value(&req.value, o) {
        Ok(v) => {
            o.current = v;
            o.overridden = true;
            (on_write)(o.key.clone(), v);
            Some(simple_ack(invoke_id, sc))
        }
        Err(e) => Some(error_response(invoke_id, sc, e)),
    }
}

/// Decode a written present value. Binary objects require an application
/// Enumerated (0/1); analog objects require an application Real within the
/// object's min/max.
fn decode_present_value(bytes: &[u8], o: &SimObj) -> Result<f64, SimError> {
    let (tag, content, hdr) = parse_tag(bytes).ok_or(ERR_INVALID_DATA_TYPE)?;
    if bytes.first().is_some_and(|b| b & 0x08 != 0) || hdr + content != bytes.len() {
        return Err(ERR_INVALID_DATA_TYPE);
    }
    let body = &bytes[hdr..];
    if o.is_bool {
        if tag != 9 || body.is_empty() || body.len() > 4 {
            return Err(ERR_INVALID_DATA_TYPE);
        }
        let v = body.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32);
        if v > 1 {
            return Err(ERR_VALUE_OUT_OF_RANGE);
        }
        Ok(v as f64)
    } else {
        if tag != 4 || body.len() != 4 {
            return Err(ERR_INVALID_DATA_TYPE);
        }
        let v = f32::from_be_bytes([body[0], body[1], body[2], body[3]]) as f64;
        if !v.is_finite() || o.min.is_some_and(|m| v < m) || o.max.is_some_and(|m| v > m) {
            return Err(ERR_VALUE_OUT_OF_RANGE);
        }
        Ok(v)
    }
}

// ── SubscribeCOV + notifications ──

/// Minimum analog change that triggers a COV notification.
const COV_INCREMENT: f64 = 0.1;

struct SubscribeReq {
    proc_id: u32,
    object: ObjectIdentifier,
    /// `None` together with `lifetime == None` means cancellation.
    confirmed: Option<bool>,
    lifetime: Option<u32>,
}

fn decode_subscribe_cov(data: &[u8]) -> Result<SubscribeReq, RejectReason> {
    let missing = RejectReason::MissingRequiredParameter;
    let (proc_id, c) = bacnet_rs::encoding::decode_context_unsigned(data, 0).map_err(|_| missing)?;
    let mut pos = c;
    let (object, c) = bacnet_rs::encoding::decode_context_object_id(&data[pos..], 1).map_err(|_| missing)?;
    pos += c;
    let mut confirmed = None;
    let mut lifetime = None;
    if data.get(pos) == Some(&0x29) {
        // [2] issueConfirmedNotifications: context boolean, one content byte.
        confirmed = Some(*data.get(pos + 1).ok_or(RejectReason::InvalidTag)? != 0);
        pos += 2;
    }
    if data.get(pos).is_some_and(|b| b & 0xF8 == 0x38) {
        let (l, c) = bacnet_rs::encoding::decode_context_unsigned(&data[pos..], 3).map_err(|_| RejectReason::InvalidTag)?;
        lifetime = Some(l);
        pos += c;
    }
    if pos != data.len() {
        return Err(RejectReason::InvalidTag);
    }
    Ok(SubscribeReq { proc_id, object, confirmed, lifetime })
}

fn handle_subscribe_cov(
    invoke_id: u8,
    service_data: &[u8],
    src: SocketAddr,
    device_id: u32,
    objects: &[SimObj],
    cov: &mut CovState,
) -> Option<Vec<u8>> {
    let sc = ConfirmedServiceChoice::SubscribeCOV;
    let req = match decode_subscribe_cov(service_data) {
        Ok(r) => r,
        Err(reason) => return Some(simple_reject(invoke_id, reason)),
    };
    let (ot, inst) = (req.object.object_type, req.object.instance);
    let Some(obj) = objects.iter().find(|o| o.ot == ot && o.instance == inst) else {
        if ot == ObjectType::Device && inst == device_id {
            return Some(error_response(invoke_id, sc, ERR_SERVICE_REQUEST_DENIED));
        }
        return Some(error_response(invoke_id, sc, ERR_UNKNOWN_OBJECT));
    };
    // A subscription is identified by (subscriber, process id, monitored object).
    cov.subs.retain(|s| !(s.subscriber == src && s.proc_id == req.proc_id && s.object_type == ot && s.instance == inst));
    if req.confirmed.is_none() && req.lifetime.is_none() {
        return Some(simple_ack(invoke_id, sc)); // cancellation
    }
    // Lifetime 0 (or absent) = indefinite.
    let expires = req.lifetime.filter(|&l| l > 0).map(|l| Instant::now() + Duration::from_secs(l as u64));
    cov.subs.push(CovSub {
        proc_id: req.proc_id,
        object_type: ot,
        instance: inst,
        confirmed: req.confirmed.unwrap_or(false),
        expires,
        subscriber: src,
        last_value: obj.current,
    });
    // Spec: an initial notification follows a successful subscription.
    let idx = cov.subs.len() - 1;
    cov.queue(idx, obj, device_id);
    Some(simple_ack(invoke_id, sc))
}

/// Live COV subscriptions, the invoke-id counter for confirmed notifications and
/// frames waiting to be sent by the device loop.
struct CovState {
    subs: Vec<CovSub>,
    next_invoke: u8,
    outbox: Vec<(SocketAddr, Vec<u8>)>,
}

impl CovState {
    fn new() -> Self {
        Self { subs: Vec::new(), next_invoke: 0, outbox: Vec::new() }
    }

    /// Queue a notification of `obj`'s current value to subscription `idx`.
    fn queue(&mut self, idx: usize, obj: &SimObj, device_id: u32) {
        let sub = &mut self.subs[idx];
        sub.last_value = obj.current;
        let remaining = sub.expires.map_or(0, |t| {
            let d = t.saturating_duration_since(Instant::now());
            (d.as_secs() + u64::from(d.subsec_nanos() > 0)).clamp(1, u32::MAX as u64) as u32
        });
        let invoke = if sub.confirmed {
            let id = self.next_invoke;
            self.next_invoke = self.next_invoke.wrapping_add(1);
            Some(id)
        } else {
            None
        };
        let frame = build_cov_notification(
            invoke,
            sub.proc_id,
            device_id,
            ObjectIdentifier::new(obj.ot, obj.instance),
            remaining,
            obj.present_value(),
        );
        self.outbox.push((sub.subscriber, frame));
    }

    /// Drop expired subscriptions and queue notifications for every subscribed
    /// object whose value moved enough since it was last notified.
    fn check(&mut self, objects: &[SimObj], device_id: u32) {
        let now = Instant::now();
        self.subs.retain(|s| s.expires.map_or(true, |t| t > now));
        for idx in 0..self.subs.len() {
            let s = &self.subs[idx];
            let Some(obj) = objects.iter().find(|o| o.ot == s.object_type && o.instance == s.instance) else {
                continue;
            };
            let changed = if obj.is_bool {
                (obj.current >= 0.5) != (s.last_value >= 0.5)
            } else {
                (obj.current - s.last_value).abs() >= COV_INCREMENT
            };
            if changed {
                self.queue(idx, obj, device_id);
            }
        }
    }
}

/// Build a BVLC frame carrying a COV notification. `confirmed_invoke` of
/// `Some(id)` → ConfirmedCOVNotification (service 1), `None` →
/// UnconfirmedCOVNotification (service 2). `time_remaining` 0 = indefinite.
fn build_cov_notification(
    confirmed_invoke: Option<u8>,
    proc_id: u32,
    device_id: u32,
    object: ObjectIdentifier,
    time_remaining: u32,
    present_value: PropertyValue,
) -> Vec<u8> {
    let mut data = Vec::new();
    // The sizes below are fixed and valid, so these encoders cannot fail.
    data.extend(bacnet_rs::encoding::encode_context_unsigned(proc_id, 0).unwrap_or_default());
    data.extend(bacnet_rs::encoding::encode_context_object_id(ObjectIdentifier::new(ObjectType::Device, device_id), 1).unwrap_or_default());
    data.extend(bacnet_rs::encoding::encode_context_object_id(object, 2).unwrap_or_default());
    data.extend(bacnet_rs::encoding::encode_context_unsigned(time_remaining, 3).unwrap_or_default());
    data.push(open_tag(4));
    data.extend([0x09, 85, open_tag(2)]); // [0] presentValue, [2] opening
    let _ = encode_property_value(&present_value, &mut data);
    data.push(close_tag(2));
    data.extend([0x09, 111, open_tag(2)]); // [0] statusFlags, [2] opening
    data.extend([0x82, 0x04, 0x00]); // BIT STRING: 4 unused bits, all flags clear
    data.push(close_tag(2));
    data.push(close_tag(4));

    let (npdu_control, apdu_head): (u8, Vec<u8>) = match confirmed_invoke {
        // ConfirmedRequest, max-APDU 1476, invoke id, ConfirmedCOVNotification(1); NPDU expects a reply.
        Some(id) => (0x04, vec![0x00, 0x05, id, 0x01]),
        // UnconfirmedRequest, UnconfirmedCOVNotification(2).
        None => (0x00, vec![0x10, 0x02]),
    };
    let len = (4 + 2 + apdu_head.len() + data.len()) as u16;
    let mut frame = vec![0x81, BVLC_ORIG_UNICAST, (len >> 8) as u8, len as u8, 0x01, npdu_control];
    frame.extend(apdu_head);
    frame.extend(data);
    frame
}

// ── Property value resolver (shared by RP + RPM) ──

fn property_values(
    device_id: u32,
    obj_id: ObjectIdentifier,
    pid_u32: u32,
    array_index: Option<u32>,
    objects: &[SimObj],
) -> Result<Vec<PropertyValue>, SimError> {
    // Device object.
    if obj_id.object_type == ObjectType::Device && obj_id.instance == device_id {
        return device_property(pid_u32, array_index, device_id, objects);
    }
    let Some(o) = objects.iter().find(|o| o.ot == obj_id.object_type && o.instance == obj_id.instance) else {
        return Err(ERR_UNKNOWN_OBJECT);
    };
    let pid = property_id_from_u32(pid_u32).ok_or(ERR_UNKNOWN_PROPERTY)?;
    // None of the modelled point properties is an array.
    let value = match pid {
        PropertyIdentifier::PresentValue => o.present_value(),
        PropertyIdentifier::ObjectName | PropertyIdentifier::Description => {
            PropertyValue::CharacterString(o.label.clone())
        }
        // Binary objects have no Units property.
        PropertyIdentifier::Units if !o.is_bool => PropertyValue::Enumerated(o.units_enum),
        PropertyIdentifier::StatusFlags => PropertyValue::BitString(vec![false; 4]), // in-alarm, fault, overridden, out-of-service
        PropertyIdentifier::ObjectType => PropertyValue::Enumerated(object_type_enum(&o.ot)),
        PropertyIdentifier::ObjectIdentifier => PropertyValue::ObjectIdentifier(obj_id),
        _ => return Err(ERR_UNKNOWN_PROPERTY),
    };
    if array_index.is_some() {
        return Err(ERR_NOT_AN_ARRAY);
    }
    Ok(vec![value])
}

fn device_property(
    pid_u32: u32,
    array_index: Option<u32>,
    device_id: u32,
    objects: &[SimObj],
) -> Result<Vec<PropertyValue>, SimError> {
    let pid = property_id_from_u32(pid_u32).ok_or(ERR_UNKNOWN_PROPERTY)?;
    if pid == PropertyIdentifier::ObjectList {
        // index 0 → count; index N → the Nth object id; no index → whole list.
        let ids: Vec<ObjectIdentifier> = std::iter::once(ObjectIdentifier::new(ObjectType::Device, device_id))
            .chain(objects.iter().map(|o| ObjectIdentifier::new(o.ot, o.instance)))
            .collect();
        return match array_index {
            None => Ok(ids.into_iter().map(PropertyValue::ObjectIdentifier).collect()),
            Some(0) => Ok(vec![PropertyValue::Unsigned(ids.len() as u64)]),
            Some(i) if (i as usize) <= ids.len() => Ok(vec![PropertyValue::ObjectIdentifier(ids[(i as usize) - 1])]),
            Some(_) => Err(ERR_INVALID_ARRAY_INDEX),
        };
    }
    let value = match pid {
        PropertyIdentifier::ObjectName => PropertyValue::CharacterString(format!("SimDevice-{}", device_id)),
        PropertyIdentifier::Description => PropertyValue::CharacterString("MacTools BACnet Simulator".into()),
        PropertyIdentifier::VendorName => PropertyValue::CharacterString("JoyoDigitama".into()),
        PropertyIdentifier::VendorIdentifier => PropertyValue::Unsigned(0),
        PropertyIdentifier::ModelName => PropertyValue::CharacterString("Sim".into()),
        PropertyIdentifier::FirmwareRevision => PropertyValue::CharacterString("1.0.0".into()),
        PropertyIdentifier::ProtocolVersion => PropertyValue::Unsigned(1),
        PropertyIdentifier::SystemStatus => PropertyValue::Enumerated(0), // operational
        PropertyIdentifier::ObjectIdentifier => {
            PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::Device, device_id))
        }
        PropertyIdentifier::ObjectType => PropertyValue::Enumerated(8), // Device
        _ => return Err(ERR_UNKNOWN_PROPERTY),
    };
    if array_index.is_some() {
        return Err(ERR_NOT_AN_ARRAY);
    }
    Ok(vec![value])
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

fn error_response(invoke_id: u8, service_choice: ConfirmedServiceChoice, e: SimError) -> Vec<u8> {
    // Error PDU: 0x50, invoke id, service choice, Enumerated class, Enumerated code.
    wrap_apdu_unicast(&Apdu::Error { invoke_id, service_choice, error_class: e.class, error_code: e.code })
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

/// Map a wire property id to the [`PropertyIdentifier`] the simulator models.
/// `None` means the simulator does not model the property (→ unknown-property).
fn property_id_from_u32(v: u32) -> Option<PropertyIdentifier> {
    Some(match v {
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
        _ => return None,
    })
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
        ReadPropertyMultipleResponse, WritePropertyRequest,
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
        let mut cov = CovState::new();
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

    /// Strip BVLC + NPDU from a response frame and return the APDU bytes.
    fn apdu_of(resp: &[u8]) -> Vec<u8> {
        let (_npdu, nlen) = Npdu::decode(&resp[4..]).unwrap();
        resp[4 + nlen..].to_vec()
    }

    /// BVLC Original-Unicast + plain NPDU (01 00) + the given APDU bytes.
    fn raw_frame(apdu: &[u8]) -> Vec<u8> {
        let len = (4 + 2 + apdu.len()) as u16;
        let mut f = vec![0x81, 0x0A, (len >> 8) as u8, len as u8, 0x01, 0x00];
        f.extend_from_slice(apdu);
        f
    }

    #[test]
    fn write_to_read_only_point_is_denied_with_error_pdu() {
        let mut objects = build_objects(&[pt("temp", "analogInput", 1, 22.5, false, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let written = std::cell::Cell::new(false);
        let apdu = [
            0x00, 0x05, 0x01, 0x0F, // ConfirmedRequest, max-APDU 1476, invoke 1, WriteProperty(15)
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object id: analogInput(0) instance 1
            0x19, 0x55, // [1] property id 85 presentValue
            0x3E, // [3] opening: property value
            0x44, 0x41, 0xA0, 0x00, 0x00, // application Real 20.0
            0x3F, // [3] closing
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| written.set(true)).unwrap();
        assert_eq!(
            apdu_of(&resp),
            vec![
                0x50, 0x01, 0x0F, // Error PDU, invoke 1, WriteProperty
                0x91, 0x02, // Enumerated error class: property (2)
                0x91, 0x28, // Enumerated error code: write-access-denied (40)
            ]
        );
        assert!(!written.get(), "on_write must not fire for a rejected write");
        assert_eq!(objects[0].current, 22.5, "value must be untouched");
    }

    #[test]
    fn write_wrong_type_to_binary_is_invalid_data_type() {
        let mut objects = build_objects(&[pt("cmd", "binaryValue", 1, 0.0, true, true)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x01, 0x0F,
            0x0C, 0x01, 0x40, 0x00, 0x01, // binaryValue(5) instance 1: 5<<22 | 1 = 0x01400001
            0x19, 0x55,
            0x3E, 0x44, 0x3F, 0x80, 0x00, 0x00, 0x3F, // application Real 1.0 where Enumerated is required
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(apdu_of(&resp), vec![0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x09]); // property / invalid-data-type
    }

    #[test]
    fn write_to_unmodeled_property_is_denied() {
        let mut objects = build_objects(&[pt("sp", "analogValue", 1, 23.0, true, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x01, 0x0F,
            0x0C, 0x00, 0x80, 0x00, 0x01, // analogValue(2)<<22 = 0x00800000, instance 1
            0x19, 0x4D, // property 77 objectName
            0x3E, 0x72, 0x00, 0x41, 0x3F, // CharacterString len 2: encoding 0 (UTF-8) + 'A'
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(apdu_of(&resp), vec![0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x28]);
    }

    #[test]
    fn write_real_containing_closing_tag_byte_is_applied() {
        let mut objects = build_objects(&[pt("sp", "analogValue", 1, 23.0, true, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x01, 0x0F,
            0x0C, 0x00, 0x80, 0x00, 0x01, // analogValue(2) instance 1
            0x19, 0x55,
            0x3E, 0x44, 0x3F, 0x80, 0x00, 0x00, 0x3F, // Real 1.0 = 3F 80 00 00 (data starts with the byte 0x3F), then closing tag
            0x49, 0x08, // [4] priority 8
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(apdu_of(&resp), vec![0x20, 0x01, 0x0F]); // SimpleAck
        assert_eq!(objects[0].current, 1.0);
    }

    #[test]
    fn write_to_unknown_object_is_unknown_object() {
        let mut objects = build_objects(&[pt("sp", "analogValue", 1, 23.0, true, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x01, 0x0F,
            0x0C, 0x00, 0x80, 0x00, 0x63, // analogValue(2) instance 99 (absent)
            0x19, 0x55,
            0x3E, 0x44, 0x41, 0xA0, 0x00, 0x00, 0x3F,
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(apdu_of(&resp), vec![0x50, 0x01, 0x0F, 0x91, 0x01, 0x91, 0x1F]); // object / unknown-object (31)
    }

    #[test]
    fn read_unknown_property_is_error_pdu() {
        let mut objects = build_objects(&[pt("temp", "analogInput", 1, 22.5, false, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x02, 0x0C, // ConfirmedRequest, invoke 2, ReadProperty(12)
            0x0C, 0x00, 0x00, 0x00, 0x01, // analogInput 1
            0x19, 0x16, // property 22 covIncrement (not modeled)
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(
            apdu_of(&resp),
            vec![
                0x50, 0x02, 0x0C, // Error PDU, invoke 2, ReadProperty
                0x91, 0x02, // class property
                0x91, 0x20, // code unknown-property (32)
            ]
        );
    }

    #[test]
    fn rpm_unknown_property_is_property_access_error() {
        let mut objects = build_objects(&[pt("temp", "analogInput", 1, 22.5, false, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x03, 0x0E, // ConfirmedRequest, invoke 3, ReadPropertyMultiple(14)
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] analogInput 1
            0x1E, // [1] opening: property references
            0x09, 0x16, // [0] property 22 (not modeled)
            0x1F, // [1] closing
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(
            apdu_of(&resp),
            vec![
                0x30, 0x03, 0x0E, // ComplexAck, not segmented, invoke 3, ReadPropertyMultiple(14)
                0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object id
                0x1E, // [1] opening: list of results
                0x29, 0x16, // [2] property id 22
                0x5E, // [5] opening: propertyAccessError
                0x91, 0x02, // class property
                0x91, 0x20, // code unknown-property
                0x5F, // [5] closing
                0x1F, // [1] closing
            ]
        );
    }

    #[test]
    fn unconfirmed_cov_notification_wire_bytes() {
        let oid = ObjectIdentifier::new(ObjectType::AnalogInput, 1);
        let frame = build_cov_notification(None, 7, 991, oid, 0, PropertyValue::Real(22.5));
        let expected: Vec<u8> = vec![
            0x81, 0x0A, 0x00, 0x28, // BVLC original-unicast, length 40
            0x01, 0x00, // NPDU version 1, no flags
            0x10, 0x02, // UnconfirmedRequest, UnconfirmedCOVNotification(2)
            0x09, 0x07, // [0] subscriberProcessIdentifier 7
            0x1C, 0x02, 0x00, 0x03, 0xDF, // [1] device(8)<<22 | 991 = 0x020003DF
            0x2C, 0x00, 0x00, 0x00, 0x01, // [2] analogInput(0) instance 1
            0x39, 0x00, // [3] timeRemaining 0 (indefinite)
            0x4E, // [4] opening: listOfValues
            0x09, 0x55, // [0] property presentValue (85)
            0x2E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x2F, // [2] { Real 22.5 = 0x41B40000 }
            0x09, 0x6F, // [0] property statusFlags (111)
            0x2E, 0x82, 0x04, 0x00, 0x2F, // [2] { BitString, 4 unused bits, 0b0000 }
            0x4F, // [4] closing
        ];
        assert_eq!(frame, expected);
    }

    #[test]
    fn confirmed_cov_notification_wire_bytes() {
        let oid = ObjectIdentifier::new(ObjectType::BinaryValue, 2);
        let frame = build_cov_notification(Some(9), 7, 991, oid, 60, PropertyValue::Enumerated(1));
        let expected: Vec<u8> = vec![
            0x81, 0x0A, 0x00, 0x27, // BVLC original-unicast, length 39
            0x01, 0x04, // NPDU version 1, expecting reply
            0x00, 0x05, 0x09, 0x01, // ConfirmedRequest, max-APDU 1476, invoke 9, ConfirmedCOVNotification(1)
            0x09, 0x07, // [0] process id 7
            0x1C, 0x02, 0x00, 0x03, 0xDF, // [1] device 991
            0x2C, 0x01, 0x40, 0x00, 0x02, // [2] binaryValue(5)<<22 | 2 = 0x01400002
            0x39, 0x3C, // [3] timeRemaining 60
            0x4E, // [4] opening
            0x09, 0x55, 0x2E, 0x91, 0x01, 0x2F, // presentValue { Enumerated 1 }
            0x09, 0x6F, 0x2E, 0x82, 0x04, 0x00, 0x2F, // statusFlags { all clear }
            0x4F, // [4] closing
        ];
        assert_eq!(frame, expected);
    }

    #[test]
    fn subscribe_sends_initial_notification_then_notifies_on_change() {
        let mut objects = build_objects(&[pt("temp", "analogInput", 1, 22.5, false, false)]);
        let mut cov = CovState::new();
        let src: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let apdu = [
            0x00, 0x05, 0x04, 0x05, // ConfirmedRequest, invoke 4, SubscribeCOV(5)
            0x09, 0x07, // [0] subscriber process id 7
            0x1C, 0x00, 0x00, 0x00, 0x01, // [1] analogInput 1
            0x29, 0x00, // [2] issueConfirmedNotifications FALSE
            0x39, 0x00, // [3] lifetime 0 (indefinite)
        ];
        let resp = handle_packet(&raw_frame(&apdu), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(apdu_of(&resp), vec![0x20, 0x04, 0x05]); // SimpleAck, invoke 4, SubscribeCOV
        assert_eq!(cov.outbox.len(), 1, "initial notification expected");
        assert_eq!(cov.outbox[0].0, src);
        cov.outbox.clear();

        cov.check(&objects, 991);
        assert!(cov.outbox.is_empty(), "no change, no notification");
        objects[0].current = 22.55; // below the 0.1 increment
        cov.check(&objects, 991);
        assert!(cov.outbox.is_empty());
        objects[0].current = 22.7; // delta 0.2 >= 0.1
        cov.check(&objects, 991);
        assert_eq!(cov.outbox.len(), 1);

        // Cancellation: no confirmed flag, no lifetime.
        cov.outbox.clear();
        let cancel = [0x00, 0x05, 0x05, 0x05, 0x09, 0x07, 0x1C, 0x00, 0x00, 0x00, 0x01];
        let resp = handle_packet(&raw_frame(&cancel), src, 991, 0, &mut objects, &mut cov, &|_, _| {}).unwrap();
        assert_eq!(apdu_of(&resp), vec![0x20, 0x05, 0x05]);
        assert!(cov.subs.is_empty());
    }
}
