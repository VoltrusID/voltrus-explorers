//! Everything the Tauri commands do, without Tauri: the open bus, the DBC, periodic transmit, recording,
//! opened trace files, trace replay and the simulator. Frames go out to the UI in batches (`can-frames`,
//! every 50 ms) with their DBC decoding and protocol label already attached; `can-stats` comes once a
//! second. Recording happens here, from every frame the bus delivers, not from what reaches the UI.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use mt_can::bus::rate_text;
use mt_can::dbc::{Dbc, DbcWarning, PhysValue, encode_message_values};
use mt_can::frame::{TimedFrame, now_us};
use mt_can::log::{LogRecord, candump_line, parse_log};
use mt_can::protocols::{Decoder, Layer};
use mt_can::sim::{SIM_DBC, SimAction, SimState, Simulator};
use mt_can::{BusEvent, CanFrame, CanHandle, CanSender, Direction, InterfaceConfig, PortInfo, VirtualBus};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

/// Event sink: `(event name, JSON payload)`.
pub type Emit = Arc<dyn Fn(&str, Json) + Send + Sync>;

const BATCH_MS: u64 = 50;
/// Frames per batch at most; above that the oldest frames of the batch make way and the UI gets the number it
/// did not see (`dropped`). Recording and the counters still see every frame.
const MAX_BATCH: usize = 2000;

fn es(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A frame as the UI sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameView {
    pub ts: u64,
    pub dir: Direction,
    pub id: u32,
    pub ext: bool,
    pub rtr: bool,
    pub fd: bool,
    pub brs: bool,
    /// CAN FD error state indicator (the sender was error passive).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub esi: bool,
    pub dlc: u8,
    pub data: Vec<u8>,
    /// DBC message name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
    /// DBC signals: `[name, value, unit, label]`. The label is the value table text, or the exact
    /// value when a JSON number can not carry it (`NaN`, `Inf`, integers wider than 53 bits).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sigs: Vec<(String, f64, String, Option<String>)>,
    /// J1939, CANopen or OBD-II label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `protocol` keeps ISO-TP / J1939 transport state between the frames of one stream.
fn view(f: &TimedFrame, dbc: Option<&Dbc>, layer: Layer, protocol: &mut Decoder) -> FrameView {
    let (msg, sigs) = match dbc.and_then(|d| d.decode(&f.frame)) {
        Some((m, v)) => (Some(m.name.clone()), v.into_iter().map(|s| (s.name, s.value, s.unit, s.label.or(s.text))).collect()),
        None => (None, Vec::new()),
    };
    FrameView {
        ts: f.ts_us,
        dir: f.dir,
        id: f.frame.id,
        ext: f.frame.extended,
        rtr: f.frame.rtr,
        fd: f.frame.fd,
        brs: f.frame.brs,
        esi: f.frame.esi,
        dlc: f.frame.dlc(),
        data: f.frame.data.clone(),
        msg,
        sigs,
        label: protocol.describe(&f.frame, layer),
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub rx: u64,
    pub tx: u64,
    pub errors: u64,
    /// Frames the UI did not get because the bus was faster than the batches.
    pub dropped: u64,
    pub rx_per_sec: f64,
    pub tx_per_sec: f64,
    /// Estimated bus load in percent (needs a known bit rate).
    pub load: Option<f64>,
    /// The controller's error state and counters, once the adapter has reported them.
    pub bus_state: Option<mt_can::frame::BusStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnInfo {
    pub description: String,
    pub log_name: String,
    pub can_send: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcSignalView {
    pub name: String,
    pub start_bit: u16,
    pub length: u16,
    pub byte_order: String,
    pub value_type: String,
    pub factor: f64,
    pub offset: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub multiplex: Option<String>,
    pub comment: String,
    pub values: Vec<(i64, String)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcMessageView {
    pub id: u32,
    pub ext: bool,
    pub name: String,
    pub size: u16,
    pub transmitter: String,
    pub cycle_ms: Option<u32>,
    pub fd: bool,
    pub brs: bool,
    pub comment: String,
    pub signals: Vec<DbcSignalView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcView {
    pub name: String,
    pub version: String,
    pub nodes: Vec<String>,
    pub messages: Vec<DbcMessageView>,
    pub skipped: usize,
    /// Statements that could not be read (skipped) and other problems, with their line.
    pub warnings: Vec<DbcWarning>,
    /// J1939 database: frames match messages by PGN.
    pub j1939: bool,
}

fn dbc_view(name: &str, d: &Dbc) -> DbcView {
    DbcView {
        name: name.to_string(),
        version: d.version.clone(),
        nodes: d.nodes.clone(),
        skipped: d.skipped,
        warnings: d.warnings.clone(),
        j1939: d.j1939,
        messages: d
            .messages
            .iter()
            .map(|m| DbcMessageView {
                id: m.id,
                ext: m.extended,
                name: m.name.clone(),
                size: m.size,
                transmitter: m.transmitter.clone(),
                cycle_ms: m.cycle_ms,
                fd: m.fd,
                brs: m.brs,
                comment: m.comment.clone(),
                signals: m
                    .signals
                    .iter()
                    .map(|s| DbcSignalView {
                        name: s.name.clone(),
                        start_bit: s.start_bit,
                        length: s.length,
                        byte_order: format!("{:?}", s.byte_order).to_lowercase(),
                        value_type: format!("{:?}", s.value_type).to_lowercase(),
                        factor: s.factor,
                        offset: s.offset,
                        min: s.min,
                        max: s.max,
                        unit: s.unit.clone(),
                        multiplex: s.mux_label(),
                        comment: s.comment.clone(),
                        values: s.values.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// A frame to send, as the UI writes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameIn {
    pub id: u32,
    pub ext: bool,
    #[serde(default)]
    pub rtr: bool,
    #[serde(default)]
    pub fd: bool,
    #[serde(default)]
    pub brs: bool,
    /// Hex bytes.
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub rtr_len: u8,
}

impl FrameIn {
    pub fn frame(&self) -> Result<CanFrame, String> {
        let data = mt_can::frame::parse_hex_bytes(&self.data).map_err(es)?;
        let f = CanFrame { id: self.id, extended: self.ext, rtr: self.rtr, fd: self.fd, brs: self.brs, esi: false, data, rtr_len: self.rtr_len };
        f.validate().map_err(es)?;
        Ok(f)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayRequest {
    /// The log's text: candump, Vector ASC, PCAN TRC or CSV.
    pub text: String,
    /// 1.0 is the recorded timing.
    pub speed: f64,
    pub repeat: bool,
    /// Only frames whose identifiers are listed (empty: all).
    #[serde(default)]
    pub ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayInfo {
    pub frames: usize,
    pub duration_s: f64,
    /// What was skipped or made up while reading the log.
    pub notes: Vec<String>,
}

/// An opened trace file, kept in the backend; the UI reads it a page at a time with [`Core::trace_page`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceInfo {
    pub name: String,
    /// Frames in the file.
    pub frames: usize,
    pub format: &'static str,
    pub first_ts: u64,
    pub last_ts: u64,
    /// What was skipped or made up while reading the file.
    pub notes: Vec<String>,
}

/// One identifier of an opened trace, for the Monitor: its last frame, how many there were, and when each byte last changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceIdView {
    pub last: FrameView,
    pub count: u64,
    pub first_ts: u64,
    /// Mean period, ms; `None` for a single frame.
    pub period_ms: Option<f64>,
    pub changed_at: Vec<u64>,
}

/// A recording that ended.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordInfo {
    pub path: String,
    pub frames: u64,
}

/// Most frames [`Core::trace_page`] returns at once.
pub const TRACE_PAGE_MAX: usize = 20_000;

struct Bus {
    handle: CanHandle,
    sender: CanSender,
    reader: JoinHandle<()>,
    bitrate: Option<u32>,
    data_bitrate: Option<u32>,
    log_name: String,
}

#[derive(Default)]
struct Shared {
    dbc: Option<(String, Dbc)>,
    layer: Layer,
}

/// Every frame the reader sees goes to the recording file, whatever reaches the UI.
struct Recorder {
    path: String,
    iface: String,
    out: BufWriter<File>,
    frames: u64,
    last_flush: Instant,
}

type RecorderSlot = Arc<StdMutex<Option<Recorder>>>;

/// How often the recording file is flushed while frames come in.
const RECORD_FLUSH: Duration = Duration::from_millis(500);

impl Recorder {
    fn write(&mut self, f: &TimedFrame) -> std::io::Result<()> {
        self.out.write_all(candump_line(f, &self.iface).as_bytes())?;
        self.out.write_all(b"\n")?;
        self.frames += 1;
        Ok(())
    }
}

/// Ends the recording (if any): flushes the file and tells the UI with `can-record`.
fn finish_recording(slot: &RecorderSlot, emit: &Emit, reason: Option<&str>, error: Option<String>) -> Option<RecordInfo> {
    let mut rec = slot.lock().unwrap_or_else(|p| p.into_inner()).take()?;
    let error = error.or_else(|| rec.out.flush().err().map(|e| e.to_string()));
    let info = RecordInfo { path: rec.path.clone(), frames: rec.frames };
    emit(
        "can-record",
        json!({ "state": "stopped", "path": info.path, "frames": info.frames, "reason": reason, "error": error }),
    );
    Some(info)
}

/// The running replay, and what tells the UI when it ends.
struct Replay {
    task: JoinHandle<()>,
    /// Frames sent in the current pass, and the frame count of a pass, for the stop event.
    pos: Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
}

pub struct Core {
    emit: Emit,
    vbus: VirtualBus,
    bus: Mutex<Option<Bus>>,
    shared: Arc<RwLock<Shared>>,
    recorder: RecorderSlot,
    periodic: Mutex<HashMap<String, JoinHandle<()>>>,
    replay: Mutex<Option<Replay>>,
    trace: Mutex<Option<(String, Vec<LogRecord>)>>,
    sim: Mutex<Option<Simulator>>,
}

impl Core {
    pub fn new(emit: Emit) -> Self {
        Core {
            emit,
            vbus: VirtualBus::new(),
            bus: Mutex::new(None),
            shared: Arc::new(RwLock::new(Shared::default())),
            recorder: Arc::new(StdMutex::new(None)),
            periodic: Mutex::new(HashMap::new()),
            replay: Mutex::new(None),
            trace: Mutex::new(None),
            sim: Mutex::new(None),
        }
    }

    fn log(&self, direction: &str, message: impl Into<String>) {
        (self.emit)("can-log", json!({ "direction": direction, "message": message.into() }));
    }

    pub fn list_ports(&self) -> Result<Vec<PortInfo>, String> {
        mt_can::list_ports().map_err(es)
    }

    /// gs_usb (candleLight) adapters that are plugged in.
    pub async fn list_gsusb(&self) -> Result<Vec<mt_can::gsusb::GsUsbDevice>, String> {
        mt_can::gsusb::list_devices().await.map_err(es)
    }

    /// SocketCAN interfaces (Linux): names of `/sys/class/net` entries of type 280 (ARPHRD_CAN).
    pub fn list_socketcan(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/sys/class/net") {
            for e in rd.flatten() {
                let t = std::fs::read_to_string(e.path().join("type")).unwrap_or_default();
                if t.trim() == "280" {
                    out.push(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
        out.sort();
        out
    }

    pub async fn connect(&self, cfg: InterfaceConfig) -> Result<ConnInfo, String> {
        self.disconnect().await;
        let mut handle = mt_can::open(&cfg, &self.vbus).await.map_err(es)?;
        let events = handle.take_events().ok_or("bus has no event stream")?;
        let (bitrate, data_bitrate) = match &cfg {
            InterfaceConfig::Slcan { bitrate, data_bitrate, .. } | InterfaceConfig::GsUsb { bitrate, data_bitrate, .. } => {
                (Some(*bitrate), data_bitrate.filter(|d| *d > 0))
            }
            // The simulator's nominal rate; its FD frames are counted at a 2 Mbit/s data phase.
            InterfaceConfig::Virtual => (Some(500_000), Some(2_000_000)),
            InterfaceConfig::Socketcan { ifname } => socketcan_bitrates(ifname),
        };
        let can_send =
            !matches!(cfg, InterfaceConfig::Slcan { listen_only: true, .. } | InterfaceConfig::GsUsb { listen_only: true, .. });
        let rates = Rates { nominal: bitrate, data: data_bitrate };
        let reader = tokio::spawn(reader(events, self.emit.clone(), self.shared.clone(), self.recorder.clone(), rates));
        let sender = handle.sender();
        let description = handle.description.clone();
        let log_name = cfg.log_name();
        *self.bus.lock().await = Some(Bus { handle, sender, reader, bitrate, data_bitrate, log_name: log_name.clone() });
        let rate = match (bitrate, data_bitrate) {
            (Some(b), Some(d)) => format!(" ({} / {} data)", rate_text(b), rate_text(d)),
            (Some(b), None) => format!(" ({})", rate_text(b)),
            _ => String::new(),
        };
        self.log("info", format!("Opened {description}{rate}"));
        Ok(ConnInfo { description, log_name, can_send })
    }

    pub async fn disconnect(&self) {
        self.stop_all_periodic().await;
        self.stop_replay(Some("disconnected")).await;
        if let Some(b) = self.bus.lock().await.take() {
            b.handle.close().await;
            // The reader ends on `Closed`; give it a moment to flush its last batch.
            let _ = tokio::time::timeout(Duration::from_millis(200), b.reader).await;
        }
        finish_recording(&self.recorder, &self.emit, Some("disconnected"), None);
    }

    async fn sender(&self) -> Result<CanSender, String> {
        self.bus.lock().await.as_ref().map(|b| b.sender.clone()).ok_or_else(|| "Not connected to a bus.".to_string())
    }

    pub async fn send(&self, frame: FrameIn) -> Result<(), String> {
        let f = frame.frame()?;
        self.sender().await?.send(f).await.map_err(es)
    }

    /// cansend syntax: `123#DEADBEEF`.
    pub async fn send_text(&self, text: String) -> Result<(), String> {
        let mut f = mt_can::log::parse_frame(&text).map_err(es)?;
        if f.esi {
            // The error state indicator is the controller's own state; no controller sends it on request.
            f.esi = false;
            self.log("info", "ESI (flag 2) is set by a controller that is error passive and cannot be sent; the frame went out without it.");
        }
        self.sender().await?.send(f).await.map_err(es)
    }

    /// Sends `frame` every `period_ms` (at least 1 ms; fractions allowed) on a fixed schedule: each frame is due one
    /// period after the previous one was due, not after it was sent, so the average rate holds. After a stall of
    /// more than [`MAX_BEHIND`] the schedule restarts from now instead of sending the backlog in one burst.
    pub async fn periodic_start(&self, key: String, frame: FrameIn, period_ms: Option<f64>, count: Option<u64>) -> Result<(), String> {
        let f = frame.frame()?;
        let period_ms = period_ms.filter(|p| p.is_finite() && *p >= 1.0 && *p <= 86_400_000.0).ok_or(
            "The period must be a number of milliseconds, at least 1.",
        )?;
        if count == Some(0) {
            return Err("The count must be at least 1.".into());
        }
        let tx = self.sender().await?;
        let emit = self.emit.clone();
        let k = key.clone();
        let period = Duration::from_secs_f64(period_ms / 1000.0);
        let task = tokio::spawn(async move {
            let mut next = tokio::time::Instant::now();
            let mut n = 0u64;
            loop {
                if next > tokio::time::Instant::now() {
                    tokio::time::sleep_until(next).await;
                }
                if let Err(e) = tx.send(f.clone()).await {
                    emit("can-periodic", json!({ "key": k, "stopped": true, "error": e.to_string() }));
                    return;
                }
                n += 1;
                if count.is_some_and(|c| n >= c) {
                    emit("can-periodic", json!({ "key": k, "stopped": true, "sent": n }));
                    return;
                }
                next += period;
                let now = tokio::time::Instant::now();
                if now > next + MAX_BEHIND.max(period * 2) {
                    next = now;
                }
            }
        });
        if let Some(old) = self.periodic.lock().await.insert(key, task) {
            old.abort();
        }
        Ok(())
    }

    pub async fn periodic_stop(&self, key: &str) {
        if let Some(t) = self.periodic.lock().await.remove(key) {
            t.abort();
        }
    }

    async fn stop_all_periodic(&self) {
        for (_, t) in self.periodic.lock().await.drain() {
            t.abort();
        }
    }

    // ── Recording ──

    /// Records every frame of the open bus to `path` (candump log), from now until [`Core::record_stop`] or the
    /// bus closes. The file is replaced.
    pub async fn record_start(&self, path: String) -> Result<(), String> {
        let iface = self.bus.lock().await.as_ref().map(|b| b.log_name.clone()).ok_or("Connect to a bus first.")?;
        let file = File::create(&path).map_err(|e| format!("Cannot record to {path}: {e}"))?;
        finish_recording(&self.recorder, &self.emit, Some("restarted"), None);
        *self.recorder.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Recorder { path, iface, out: BufWriter::with_capacity(256 * 1024, file), frames: 0, last_flush: Instant::now() });
        Ok(())
    }

    /// Ends the recording; `None` when there was none.
    pub async fn record_stop(&self) -> Option<RecordInfo> {
        finish_recording(&self.recorder, &self.emit, Some("stopped"), None)
    }

    // ── DBC ──

    pub async fn dbc_load(&self, name: String, text: String) -> Result<DbcView, String> {
        let d = mt_can::Dbc::parse(&text).map_err(es)?;
        let v = dbc_view(&name, &d);
        self.log("info", format!("Loaded {name}: {} messages, {} signals", d.messages.len(), d.signal_count()));
        for w in &d.warnings {
            let at = if w.line > 0 { format!(" line {}", w.line) } else { String::new() };
            let src = if w.source.is_empty() { String::new() } else { format!(": {}", w.source) };
            self.log("error", format!("DBC{at}: {}{src}", w.message));
        }
        self.shared.write().await.dbc = Some((name, d));
        Ok(v)
    }

    pub async fn dbc_load_sim(&self) -> Result<DbcView, String> {
        self.dbc_load("simulator.dbc".into(), SIM_DBC.into()).await
    }

    pub async fn dbc_clear(&self) {
        self.shared.write().await.dbc = None;
    }

    pub fn sim_dbc_text(&self) -> &'static str {
        SIM_DBC
    }

    /// Builds the data of a DBC message from signal values (hex).
    /// Values are numbers, or text for exact integers of signals wider than 53 bits.
    pub async fn dbc_encode(&self, message: String, values: HashMap<String, PhysValue>) -> Result<FrameView, String> {
        let s = self.shared.read().await;
        let (_, d) = s.dbc.as_ref().ok_or("No DBC file is loaded.")?;
        let m = d.message_by_name(&message).ok_or_else(|| format!("{message} is not in the DBC file."))?;
        let data = encode_message_values(m, &values, None).map_err(es)?;
        // The DBC's frame format (VFrameFormat, CANFD_BRS); anything over 8 bytes is FD regardless.
        let fd = m.fd || data.len() > 8;
        let frame = CanFrame { id: m.id, extended: m.extended, rtr: false, fd, brs: fd && m.brs, esi: false, data, rtr_len: 0 };
        frame.validate().map_err(es)?;
        Ok(view(&TimedFrame { ts_us: now_us(), dir: Direction::Tx, frame }, Some(d), s.layer, &mut Decoder::new()))
    }

    pub async fn set_layer(&self, layer: Layer) {
        self.shared.write().await.layer = layer;
    }

    /// Decodes a whole trace file with the current DBC and layer. Prefer [`Core::trace_open`] and
    /// [`Core::trace_page`] for files of any size.
    pub async fn decode_frames(&self, text: String) -> Result<Vec<FrameView>, String> {
        let log = parse_log(&text).map_err(es)?;
        let s = self.shared.read().await;
        let dbc = s.dbc.as_ref().map(|(_, d)| d);
        let mut protocol = Decoder::new();
        Ok(log.records.iter().map(|r| view(&timed(r), dbc, s.layer, &mut protocol)).collect())
    }

    // ── Opened trace files ──

    /// Reads a trace file (candump, Vector ASC, PCAN TRC or CSV) and keeps it here; nothing is decoded until the UI
    /// asks for a page.
    pub async fn trace_open(&self, name: String, text: String) -> Result<TraceInfo, String> {
        let log = parse_log(&text).map_err(es)?;
        drop(text);
        let notes = log.notes();
        let info = TraceInfo {
            name: name.clone(),
            frames: log.records.len(),
            format: log.format.name(),
            first_ts: log.records.first().map(|r| r.ts_us).unwrap_or(0),
            last_ts: log.records.last().map(|r| r.ts_us).unwrap_or(0),
            notes,
        };
        *self.trace.lock().await = Some((name, log.records));
        Ok(info)
    }

    /// Frames `start..start + count` of the opened trace, decoded with the current DBC and layer.
    pub async fn trace_page(&self, start: usize, count: usize) -> Result<Vec<FrameView>, String> {
        let t = self.trace.lock().await;
        let (_, recs) = t.as_ref().ok_or("No trace file is open.")?;
        let s = self.shared.read().await;
        let dbc = s.dbc.as_ref().map(|(_, d)| d);
        let end = start.saturating_add(count.min(TRACE_PAGE_MAX)).min(recs.len());
        let mut protocol = Decoder::new();
        Ok(recs[start.min(end)..end].iter().map(|r| view(&timed(r), dbc, s.layer, &mut protocol)).collect())
    }

    /// One row per identifier (and direction) of the opened trace, for the Monitor.
    pub async fn trace_ids(&self) -> Result<Vec<TraceIdView>, String> {
        let t = self.trace.lock().await;
        let (_, recs) = t.as_ref().ok_or("No trace file is open.")?;
        struct Acc {
            last: usize,
            count: u64,
            first_ts: u64,
            changed_at: Vec<u64>,
        }
        let mut by_key: HashMap<(u32, bool), Acc> = HashMap::new();
        let mut order = Vec::new();
        for (i, r) in recs.iter().enumerate() {
            let key = (r.frame.key(), r.dir == Direction::Tx);
            match by_key.get_mut(&key) {
                None => {
                    order.push(key);
                    by_key.insert(key, Acc { last: i, count: 1, first_ts: r.ts_us, changed_at: vec![r.ts_us; r.frame.data.len()] });
                }
                Some(a) => {
                    let prev = &recs[a.last].frame.data;
                    a.changed_at.resize(r.frame.data.len(), r.ts_us);
                    for (k, b) in r.frame.data.iter().enumerate() {
                        if prev.get(k) != Some(b) {
                            a.changed_at[k] = r.ts_us;
                        }
                    }
                    a.last = i;
                    a.count += 1;
                }
            }
        }
        let s = self.shared.read().await;
        let dbc = s.dbc.as_ref().map(|(_, d)| d);
        Ok(order
            .into_iter()
            .filter_map(|k| by_key.remove(&k))
            .map(|a| {
                let last = &recs[a.last];
                let span = last.ts_us.saturating_sub(a.first_ts) as f64 / 1000.0;
                TraceIdView {
                    last: view(&timed(last), dbc, s.layer, &mut Decoder::new()),
                    count: a.count,
                    first_ts: a.first_ts,
                    period_ms: (a.count > 1).then(|| span / (a.count - 1) as f64),
                    changed_at: a.changed_at,
                }
            })
            .collect())
    }

    pub async fn trace_close(&self) {
        *self.trace.lock().await = None;
    }

    // ── Replay ──

    pub async fn replay_start(&self, req: ReplayRequest) -> Result<ReplayInfo, String> {
        let log = parse_log(&req.text).map_err(es)?;
        let notes = log.notes();
        let mut recs = log.records;
        if !req.ids.is_empty() {
            recs.retain(|r| req.ids.contains(&r.frame.id));
        }
        if recs.is_empty() {
            return Err("The log has no frames to replay.".into());
        }
        let tx = self.sender().await?;
        self.stop_replay(None).await;
        let speed = if req.speed.is_finite() && req.speed > 0.0 { req.speed } else { 1.0 };
        let plan = ReplayPlan::new(recs, speed);
        let n = plan.frames.len();
        let duration_s = plan.duration.as_secs_f64() * speed;
        let emit = self.emit.clone();
        let repeat = req.repeat;
        let pos = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sent = pos.clone();
        let task = tokio::spawn(async move {
            let mut pass = 0u64;
            let mut last_progress = tokio::time::Instant::now();
            let progress_every = Duration::from_millis(250);
            loop {
                let start = tokio::time::Instant::now();
                for (i, (at, frame)) in plan.frames.iter().enumerate() {
                    let due = start + *at;
                    if due > tokio::time::Instant::now() {
                        tokio::time::sleep_until(due).await;
                    }
                    if let Err(e) = tx.send(frame.clone()).await {
                        emit("can-replay", json!({ "done": true, "error": e.to_string(), "pos": i, "total": n, "pass": pass }));
                        return;
                    }
                    sent.store(i + 1, std::sync::atomic::Ordering::Relaxed);
                    if last_progress.elapsed() >= progress_every {
                        last_progress = tokio::time::Instant::now();
                        emit("can-replay", json!({ "done": false, "pos": i + 1, "total": n, "pass": pass }));
                    }
                }
                if !repeat {
                    break;
                }
                pass += 1;
                tokio::time::sleep_until(start + plan.pass_len).await;
            }
            emit("can-replay", json!({ "done": true, "pos": n, "total": n, "pass": pass }));
        });
        *self.replay.lock().await = Some(Replay { task, pos, total: n });
        Ok(ReplayInfo { frames: n, duration_s, notes })
    }

    pub async fn replay_stop(&self) {
        self.stop_replay(Some("stopped")).await;
    }

    /// Stops a running replay; with a reason, the UI hears about it (`can-replay` with `done` and `stopped`).
    async fn stop_replay(&self, reason: Option<&str>) {
        if let Some(r) = self.replay.lock().await.take() {
            let running = !r.task.is_finished();
            r.task.abort();
            if running && let Some(reason) = reason {
                let pos = r.pos.load(std::sync::atomic::Ordering::Relaxed);
                (self.emit)("can-replay", json!({ "done": true, "stopped": reason, "pos": pos, "total": r.total }));
                if reason == "disconnected" {
                    self.log("info", "Replay stopped: the bus was disconnected");
                }
            }
        }
    }

    // ── Simulator ──

    /// Starts the simulator on the virtual bus, or on a SocketCAN interface (Linux) when `ifname` is given.
    pub async fn sim_start(&self, ifname: Option<String>) -> Result<(), String> {
        let mut s = self.sim.lock().await;
        if s.is_none() {
            match ifname.filter(|n| !n.trim().is_empty()) {
                None => {
                    *s = Some(Simulator::start(&self.vbus));
                    self.log("info", "Simulator started on the virtual bus");
                }
                Some(ifname) => {
                    let cfg = InterfaceConfig::Socketcan { ifname: ifname.trim().to_string() };
                    let handle = mt_can::open(&cfg, &self.vbus).await.map_err(es)?;
                    *s = Some(Simulator::start_on(handle));
                    self.log("info", format!("Simulator started on SocketCAN {}", ifname.trim()));
                }
            }
        }
        Ok(())
    }

    pub async fn sim_stop(&self) {
        if let Some(s) = self.sim.lock().await.take() {
            s.stop().await;
            self.log("info", "Simulator stopped");
        }
    }

    pub async fn sim_state(&self) -> Result<Option<SimState>, String> {
        Ok(match self.sim.lock().await.as_ref() {
            Some(s) => Some(s.state().await),
            None => None,
        })
    }

    pub async fn sim_action(&self, action: SimAction) -> Result<(), String> {
        let g = self.sim.lock().await;
        let s = g.as_ref().ok_or("The simulator is not running.")?;
        s.apply(action).await;
        Ok(())
    }

    pub async fn bitrate(&self) -> Option<u32> {
        self.bus.lock().await.as_ref().and_then(|b| b.bitrate)
    }

    pub async fn data_bitrate(&self) -> Option<u32> {
        self.bus.lock().await.as_ref().and_then(|b| b.data_bitrate)
    }
}

/// A cyclic frame further behind its schedule than this (the process stalled) starts again from now.
const MAX_BEHIND: Duration = Duration::from_millis(100);

/// A loop pass of a replay lasts at least this long, so a one-frame or zero-length log does not flood the bus.
const MIN_PASS: Duration = Duration::from_millis(10);

/// When each frame of a replay goes out, relative to the start of the pass.
struct ReplayPlan {
    frames: Vec<(Duration, CanFrame)>,
    /// From the first to the last frame, at the replay speed.
    duration: Duration,
    /// From the start of one loop pass to the start of the next: the duration plus the log's mean frame spacing (so
    /// the last and first frames are not sent back to back), at least [`MIN_PASS`].
    pass_len: Duration,
}

impl ReplayPlan {
    fn new(recs: Vec<LogRecord>, speed: f64) -> ReplayPlan {
        // Times that go backwards (an unsorted or merged log) count as no wait.
        let mut at_us = 0u64;
        let mut prev = recs[0].ts_us;
        let n = recs.len();
        let frames: Vec<(Duration, CanFrame)> = recs
            .into_iter()
            .map(|r| {
                at_us += r.ts_us.saturating_sub(prev);
                prev = r.ts_us;
                let mut f = r.frame;
                // No controller sends the error state indicator on request.
                f.esi = false;
                (Duration::from_secs_f64(at_us as f64 / 1e6 / speed), f)
            })
            .collect();
        let duration = frames.last().map(|f| f.0).unwrap_or_default();
        let gap = if n > 1 { duration / (n as u32 - 1).max(1) } else { Duration::ZERO };
        ReplayPlan { frames, duration, pass_len: (duration + gap).max(MIN_PASS) }
    }
}

fn timed(r: &LogRecord) -> TimedFrame {
    TimedFrame { ts_us: r.ts_us, dir: r.dir, frame: r.frame.clone() }
}

/// Nominal and data bit rates of a SocketCAN interface, from `ip -details -json link show`.
fn socketcan_bitrates(ifname: &str) -> (Option<u32>, Option<u32>) {
    let Ok(out) = std::process::Command::new("ip").args(["-details", "-json", "link", "show", ifname]).output() else {
        return (None, None);
    };
    let v: Json = serde_json::from_slice(&out.stdout).unwrap_or(Json::Null);
    socketcan_rates_of(&v)
}

fn socketcan_rates_of(v: &Json) -> (Option<u32>, Option<u32>) {
    let info = &v[0]["linkinfo"]["info_data"];
    let rate = |k: &str| info[k]["bitrate"].as_u64().filter(|b| *b > 0).map(|b| b as u32);
    (rate("bittiming"), rate("data_bittiming"))
}

/// The bit rates the bus load is computed with.
#[derive(Debug, Clone, Copy)]
struct Rates {
    nominal: Option<u32>,
    data: Option<u32>,
}

impl Rates {
    /// Seconds the frame occupies the bus, with the stuff bits it really has (classic CAN: exact; CAN FD: exact for the
    /// dynamic part, fixed stuff bits for the CRC field) and the data phase at the data bit rate when BRS is set.
    fn bus_time(&self, f: &CanFrame) -> Option<f64> {
        let nominal = self.nominal.filter(|b| *b > 0)? as f64;
        let (arb_bits, data_bits) = frame_bits(f);
        let data_rate = if f.fd && f.brs { self.data.filter(|d| *d > 0).map(|d| d as f64).unwrap_or(nominal) } else { nominal };
        Some(arb_bits / nominal + data_bits / data_rate)
    }
}

/// Bits of a frame on the wire including stuff bits and the inter-frame space: `(at the nominal rate, at the data
/// rate)`. The data-rate part is zero for classic frames.
fn frame_bits(f: &CanFrame) -> (f64, f64) {
    let mut bits: Vec<bool> = Vec::with_capacity(32 + f.data.len() * 8);
    let push = |bits: &mut Vec<bool>, v: u32, n: u32| {
        for i in (0..n).rev() {
            bits.push(v >> i & 1 == 1);
        }
    };
    bits.push(false); // SOF
    if f.extended {
        push(&mut bits, f.id >> 18, 11);
        bits.push(true); // SRR
        bits.push(true); // IDE
        push(&mut bits, f.id & 0x3FFFF, 18);
    } else {
        push(&mut bits, f.id, 11);
    }
    let dlc = f.dlc() as u32;
    if !f.fd {
        bits.push(f.rtr); // RTR
        if f.extended {
            bits.extend([false, false]); // r1 r0
        } else {
            bits.extend([false, false]); // IDE r0
        }
        push(&mut bits, dlc, 4);
        if !f.rtr {
            for b in &f.data {
                push(&mut bits, *b as u32, 8);
            }
        }
        let crc = crc15(&bits);
        push(&mut bits, crc as u32, 15);
        let stuff = stuff_count(&bits);
        // CRC delimiter, ACK slot and delimiter, EOF, intermission.
        return ((bits.len() + stuff + 1 + 2 + 7 + 3) as f64, 0.0);
    }
    // CAN FD: RRS (or r1), IDE for base frames, FDF, res; then BRS, ESI, DLC, data in the data phase.
    if f.extended {
        bits.push(false); // r1
    } else {
        bits.push(false); // RRS
        bits.push(false); // IDE
    }
    bits.extend([true, false]); // FDF, res
    bits.push(f.brs);
    let arb_len = bits.len();
    bits.push(f.esi);
    push(&mut bits, dlc, 4);
    for b in &f.data {
        push(&mut bits, *b as u32, 8);
    }
    let (stuff_arb, stuff_data) = stuff_split(&bits, arb_len);
    // Stuff count (4) and CRC (17 or 21) with a fixed stuff bit before and after every 4 bits.
    let crc_field = 4 + if f.data.len() > 16 { 21 } else { 17 };
    let fixed = 1 + crc_field / 4;
    let data_bits = (bits.len() - arb_len + stuff_data + crc_field + fixed) as f64;
    // CRC delimiter, ACK slot and delimiter, EOF, intermission at the nominal rate.
    let arb_bits = (arb_len + stuff_arb + 1 + 2 + 7 + 3) as f64;
    (arb_bits, data_bits)
}

/// Stuff bits a transmitter inserts in `bits` (after five equal bits, one of the other value, which counts itself).
fn stuff_count(bits: &[bool]) -> usize {
    stuff_split(bits, bits.len()).0
}

/// Stuff bits inserted before and after position `at`.
fn stuff_split(bits: &[bool], at: usize) -> (usize, usize) {
    let (mut before, mut after) = (0, 0);
    let mut run = 0;
    let mut last = None;
    for (i, &b) in bits.iter().enumerate() {
        if Some(b) == last {
            run += 1;
        } else {
            run = 1;
            last = Some(b);
        }
        if run == 5 {
            if i < at {
                before += 1;
            } else {
                after += 1;
            }
            last = Some(!b);
            run = 1;
        }
    }
    (before, after)
}

/// CAN's CRC-15 (polynomial 0x4599) over the unstuffed bits from SOF to the end of the data.
fn crc15(bits: &[bool]) -> u16 {
    let mut crc: u16 = 0;
    for &b in bits {
        let next = b ^ (crc >> 14 & 1 == 1);
        crc = (crc << 1) & 0x7FFF;
        if next {
            crc ^= 0x4599;
        }
    }
    crc
}

/// Error-like events (error frames, overflows, refused frames) go to the log at most this often per second; the
/// rest are summed up.
const MAX_ERROR_LOGS_PER_SEC: u32 = 10;

/// The frames for the next UI batch: the newest [`MAX_BATCH`]; older ones make way when the bus is faster than the
/// screen, and are counted.
#[derive(Default)]
struct Batch {
    frames: VecDeque<TimedFrame>,
    dropped: u64,
}

impl Batch {
    /// Adds a frame; true when an older one had to make way.
    fn push(&mut self, f: TimedFrame) -> bool {
        let full = self.frames.len() >= MAX_BATCH;
        if full {
            self.frames.pop_front();
            self.dropped += 1;
        }
        self.frames.push_back(f);
        full
    }

    /// The frames and how many were dropped since the last batch; `None` when there is nothing to say.
    fn take(&mut self) -> Option<(VecDeque<TimedFrame>, u64)> {
        if self.frames.is_empty() && self.dropped == 0 {
            return None;
        }
        Some((std::mem::take(&mut self.frames), std::mem::take(&mut self.dropped)))
    }
}

async fn reader(
    mut events: tokio::sync::mpsc::UnboundedReceiver<BusEvent>,
    emit: Emit,
    shared: Arc<RwLock<Shared>>,
    recorder: RecorderSlot,
    rates: Rates,
) {
    let mut batch = Batch::default();
    let mut protocol = Decoder::new();
    let mut stats = Stats::default();
    let mut busy_s = 0.0f64;
    let (mut rx_sec, mut tx_sec) = (0u64, 0u64);
    let (mut errors_logged, mut errors_held, mut last_held): (u32, u64, Option<String>) = (0, 0, None);
    let mut flush = tokio::time::interval(Duration::from_millis(BATCH_MS));
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    flush.tick().await;
    let mut second = tokio::time::interval(Duration::from_secs(1));
    second.tick().await;
    let record = |f: &TimedFrame| {
        let mut slot = recorder.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(r) = slot.as_mut()
            && let Err(e) = r.write(f)
        {
            drop(slot);
            finish_recording(&recorder, &emit, Some("error"), Some(e.to_string()));
        }
    };
    let closed_reason: Option<String> = loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(BusEvent::Frame(f)) => {
                    match f.dir {
                        Direction::Rx => { stats.rx += 1; rx_sec += 1; }
                        Direction::Tx => { stats.tx += 1; tx_sec += 1; }
                    }
                    busy_s += rates.bus_time(&f.frame).unwrap_or(0.0);
                    record(&f);
                    if batch.push(f) {
                        stats.dropped += 1;
                    }
                }
                Some(BusEvent::Error(e)) => {
                    stats.errors += 1;
                    if errors_logged < MAX_ERROR_LOGS_PER_SEC {
                        errors_logged += 1;
                        emit("can-log", json!({ "direction": "error", "message": e }));
                    } else {
                        errors_held += 1;
                        last_held = Some(e);
                    }
                }
                // A burst the driver collapsed: count all of them, log the latest once.
                Some(BusEvent::Errors { count, latest }) => {
                    stats.errors += count as u64;
                    emit("can-log", json!({ "direction": "error", "message": format!("{latest} ({count} errors like this since the last one shown)") }));
                }
                Some(BusEvent::Status(s)) => stats.bus_state = Some(s),
                Some(BusEvent::Info(i)) => emit("can-log", json!({ "direction": "info", "message": i })),
                Some(BusEvent::Closed(reason)) => break reason,
                None => break None,
            },
            _ = flush.tick() => {
                if let Some((frames, dropped)) = batch.take() {
                    let frames = {
                        let s = shared.read().await;
                        let dbc = s.dbc.as_ref().map(|(_, d)| d);
                        frames.iter().map(|f| view(f, dbc, s.layer, &mut protocol)).collect::<Vec<_>>()
                    };
                    emit("can-frames", json!({ "frames": frames, "dropped": dropped }));
                }
                let mut slot = recorder.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(r) = slot.as_mut()
                    && r.last_flush.elapsed() >= RECORD_FLUSH
                {
                    r.last_flush = Instant::now();
                    if let Err(e) = r.out.flush() {
                        drop(slot);
                        finish_recording(&recorder, &emit, Some("error"), Some(e.to_string()));
                    }
                }
            }
            _ = second.tick() => {
                if errors_held > 0 {
                    let last = last_held.take().unwrap_or_default();
                    emit("can-log", json!({ "direction": "error", "message": format!("{errors_held} more errors in the last second (latest: {last})") }));
                    errors_held = 0;
                }
                errors_logged = 0;
                stats.rx_per_sec = rx_sec as f64;
                stats.tx_per_sec = tx_sec as f64;
                stats.load = rates.nominal.filter(|b| *b > 0).map(|_| (busy_s * 100.0).min(100.0));
                rx_sec = 0; tx_sec = 0; busy_s = 0.0;
                emit("can-stats", serde_json::to_value(&stats).unwrap_or(Json::Null));
            }
        }
    };
    if let Some((frames, dropped)) = batch.take() {
        let frames = {
            let s = shared.read().await;
            let dbc = s.dbc.as_ref().map(|(_, d)| d);
            frames.iter().map(|f| view(f, dbc, s.layer, &mut protocol)).collect::<Vec<_>>()
        };
        emit("can-frames", json!({ "frames": frames, "dropped": dropped }));
    }
    if errors_held > 0 {
        emit("can-log", json!({ "direction": "error", "message": format!("{errors_held} more errors (latest: {})", last_held.unwrap_or_default()) }));
    }
    let reason = if closed_reason.is_some() { "closed" } else { "disconnected" };
    finish_recording(&recorder, &emit, Some(reason), None);
    emit("can-conn", json!({ "state": "closed", "reason": closed_reason }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    type Seen = Arc<StdMutex<Vec<(String, Json)>>>;

    fn sink() -> (Emit, Seen) {
        let got = Arc::new(StdMutex::new(Vec::new()));
        let g = got.clone();
        (Arc::new(move |e: &str, p: Json| g.lock().unwrap().push((e.to_string(), p))), got)
    }

    #[tokio::test]
    async fn virtual_bus_with_simulator_and_dbc() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        core.sim_start(None).await.unwrap();
        let info = core.connect(InterfaceConfig::Virtual).await.unwrap();
        assert!(info.can_send);
        core.dbc_load_sim().await.unwrap();
        core.set_layer(Layer::J1939).await;
        core.send_text("7DF#02010C0000000000".into()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        core.disconnect().await;
        core.sim_stop().await;

        let events = got.lock().unwrap();
        let frames: Vec<&Json> = events
            .iter()
            .filter(|(e, _)| e == "can-frames")
            .flat_map(|(_, p)| p["frames"].as_array().unwrap().iter())
            .collect();
        let eec1 = frames.iter().find(|f| f["msg"] == "EEC1").expect("EEC1 decoded");
        assert!(eec1["label"].as_str().unwrap().starts_with("PGN 61444"));
        assert!(eec1["sigs"].as_array().unwrap().iter().any(|s| s[0] == "EngineSpeed"));
        assert!(frames.iter().any(|f| f["id"] == 0x7E8));
        // The CAN FD battery pack: 64 bytes with bit rate switch, decoded by the simulator DBC.
        let fd = frames.iter().find(|f| f["msg"] == "BatteryPackFD").expect("BatteryPackFD decoded");
        assert_eq!((fd["id"].as_u64(), fd["fd"].as_bool(), fd["brs"].as_bool()), (Some(0x4A0), Some(true), Some(true)));
        assert_eq!(fd["data"].as_array().unwrap().len(), 64);
        assert!(fd["sigs"].as_array().unwrap().iter().any(|s| s[0] == "SOC"));
        assert!(frames.iter().any(|f| f["dir"] == "tx" && f["id"] == 0x7DF));
        assert!(events.iter().any(|(e, p)| e == "can-conn" && p["state"] == "closed"));
    }

    #[tokio::test]
    async fn periodic_replay_and_encode() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        // Two nodes on the virtual bus: the core, and a listener standing in for another device.
        let mut other = core.vbus.attach();
        let mut ev = other.take_events().unwrap();
        core.connect(InterfaceConfig::Virtual).await.unwrap();
        let f = FrameIn { id: 0x321, ext: false, rtr: false, fd: false, brs: false, data: "01 02".into(), rtr_len: 0 };
        core.periodic_start("a".into(), f, Some(10.0), Some(3)).await.unwrap();
        let mut n = 0;
        while n < 3 {
            if let Some(BusEvent::Frame(fr)) = ev.recv().await {
                assert_eq!(fr.frame.id, 0x321);
                n += 1;
            }
        }
        let info = core
            .replay_start(ReplayRequest {
                text: "(1.000000) can0 100#01\n(1.010000) can0 200#02\n(1.020000) can0 100#03\n".into(),
                speed: 2.0,
                repeat: false,
                ids: vec![0x100],
            })
            .await
            .unwrap();
        assert_eq!(info.frames, 2);
        let mut ids = Vec::new();
        while ids.len() < 2 {
            if let Some(BusEvent::Frame(fr)) = ev.recv().await {
                ids.push((fr.frame.id, fr.frame.data[0]));
            }
        }
        assert_eq!(ids, vec![(0x100, 1), (0x100, 3)]);

        assert!(core.dbc_encode("EEC1".into(), HashMap::new()).await.is_err());
        core.dbc_load_sim().await.unwrap();
        let v = core.dbc_encode("ThrottleCommand".into(), [("Throttle".to_string(), 50.0.into())].into_iter().collect()).await.unwrap();
        assert_eq!(v.id, 0x511);
        assert_eq!(v.data[0], 100);
        assert!(!v.fd && !v.brs, "classic DBC messages stay classic");
        let bad = FrameIn { id: 0x900, ext: false, rtr: false, fd: false, brs: false, data: "".into(), rtr_len: 0 };
        assert!(core.send(bad).await.is_err());
        core.disconnect().await;
        assert!(core.send_text("1#00".into()).await.is_err());
        drop(got);
    }

    /// CAN FD end to end on the virtual bus: the yocto image's can-demo telemetry (64 bytes, BRS) in,
    /// decoded with its DBC; an FD message encoded from the DBC goes out as FD with BRS.
    #[tokio::test]
    async fn fd_frames_decode_and_encode() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        let other = core.vbus.attach();
        core.connect(InterfaceConfig::Virtual).await.unwrap();
        let dbc = core.dbc_load("can-demo.dbc".into(), include_str!("../../../../crates/mt-can/tests/fixtures/can-demo.dbc").into()).await.unwrap();
        let ping = dbc.messages.iter().find(|m| m.name == "HostFdPing").unwrap();
        assert!(ping.fd && ping.brs && ping.size == 64);

        let mut data = vec![0u8; 64];
        data[0] = 0x2A;
        data[4..8].copy_from_slice(b"FDT1");
        for (i, b) in data[32..].iter_mut().enumerate() {
            *b = 0x2A + i as u8;
        }
        let mut f = CanFrame::fd(0x201, false, true, &data).unwrap();
        f.esi = true;
        other.send(f).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        {
            let events = got.lock().unwrap();
            let fr = events
                .iter()
                .filter(|(e, _)| e == "can-frames")
                .flat_map(|(_, p)| p["frames"].as_array().unwrap().iter())
                .find(|f| f["id"] == 0x201)
                .expect("the FD frame reached the UI");
            assert_eq!((fr["fd"].as_bool(), fr["brs"].as_bool(), fr["esi"].as_bool()), (Some(true), Some(true), Some(true)));
            assert_eq!(fr["dlc"], 15);
            assert_eq!(fr["data"].as_array().unwrap().len(), 64);
            assert_eq!(fr["msg"], "EdgeTelemetryFd");
            let sig = |n: &str| fr["sigs"].as_array().unwrap().iter().find(|s| s[0] == n).map(|s| s[1].as_f64().unwrap());
            assert_eq!((sig("Seq"), sig("PatternFirst"), sig("PatternLast")), (Some(42.0), Some(42.0), Some(73.0)));
        }

        let v = core.dbc_encode("HostFdPing".into(), [("Count".to_string(), 5.0.into())].into_iter().collect()).await.unwrap();
        assert!(v.fd && v.brs && v.data.len() == 64 && v.dlc == 15);
        let hb = core.dbc_encode("HostPing".into(), HashMap::new()).await.unwrap();
        assert!(!hb.fd && !hb.brs);
        core.disconnect().await;
    }

    fn frames_of(got: &Seen) -> Vec<Json> {
        got.lock()
            .unwrap()
            .iter()
            .filter(|(e, _)| e == "can-frames")
            .flat_map(|(_, p)| p["frames"].as_array().unwrap().clone())
            .collect()
    }

    fn events_named(got: &Seen, name: &str) -> Vec<Json> {
        got.lock().unwrap().iter().filter(|(e, _)| e == name).map(|(_, p)| p.clone()).collect()
    }

    fn temp_path(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("canx-core-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    /// Recording writes every frame the bus delivers, even when the UI batches drop frames, and replaces the file.
    #[tokio::test]
    async fn recording_keeps_every_frame() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        let other = core.vbus.attach();
        core.connect(InterfaceConfig::Virtual).await.unwrap();
        let path = temp_path("flood.log");
        std::fs::write(&path, "old contents that must go\n".repeat(100)).unwrap();
        core.record_start(path.clone()).await.unwrap();
        const N: u32 = 12_000;
        for i in 0..N {
            other.send(CanFrame::new(0x100 + (i % 16), &i.to_le_bytes()).unwrap()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let info = core.record_stop().await.unwrap();
        assert_eq!(info.frames, N as u64);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("old contents"));
        let recs = mt_can::log::parse_candump(&text).unwrap();
        assert_eq!(recs.len(), N as usize);
        assert!(recs.iter().enumerate().all(|(i, r)| r.frame.data == (i as u32).to_le_bytes()), "in order, none missing");
        assert!(recs.windows(2).all(|w| w[0].ts_us <= w[1].ts_us));
        // The UI got every frame or a count of those it did not; the newest are the ones kept.
        let shown = frames_of(&got);
        let dropped: u64 = events_named(&got, "can-frames").iter().map(|p| p["dropped"].as_u64().unwrap()).sum();
        assert_eq!(shown.len() as u64 + dropped, N as u64);
        let last = shown.last().unwrap();
        assert_eq!(last["data"].as_array().unwrap()[0].as_u64(), Some(((N - 1) & 0xFF) as u64), "the last frame sent is shown");
        assert!(core.record_stop().await.is_none());
        // Disconnecting ends a recording and tells the UI; recording needs a bus.
        core.record_start(path.clone()).await.unwrap();
        other.send(CanFrame::new(0x1, &[1]).unwrap()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        core.disconnect().await;
        let stopped = events_named(&got, "can-record");
        assert_eq!(stopped.last().unwrap()["frames"], 1);
        assert_eq!(stopped.last().unwrap()["reason"], "disconnected");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
        assert!(core.record_start(path.clone()).await.is_err());
        let _ = std::fs::remove_file(&path);
    }

    /// The UI batch keeps the newest frames when it overflows.
    #[test]
    fn batch_overflow_drops_oldest() {
        let mut b = Batch::default();
        assert!(b.take().is_none());
        let n = MAX_BATCH as u64 + 500;
        let dropped = (0..n).filter(|i| b.push(TimedFrame { ts_us: *i, dir: Direction::Rx, frame: CanFrame::new(0x10, &[]).unwrap() })).count();
        assert_eq!(dropped, 500);
        let (frames, d) = b.take().unwrap();
        assert_eq!((frames.len(), d), (MAX_BATCH, 500));
        assert_eq!((frames[0].ts_us, frames.back().unwrap().ts_us), (500, n - 1));
        assert!(b.take().is_none());
    }

    /// A storm of error frames gives a handful of log lines and a summary, not one event each.
    #[tokio::test]
    async fn error_storm_is_coalesced() {
        let (emit, got) = sink();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let rates = Rates { nominal: None, data: None };
        let task = tokio::spawn(reader(rx, emit, Arc::new(RwLock::new(Shared::default())), Arc::new(StdMutex::new(None)), rates));
        for i in 0..5000 {
            tx.send(BusEvent::Error(format!("error frame {i}"))).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(1200)).await;
        tx.send(BusEvent::Closed(None)).unwrap();
        task.await.unwrap();
        let logs = events_named(&got, "can-log");
        assert!(logs.len() <= 12, "{} log events", logs.len());
        assert!(logs.iter().any(|l| l["message"].as_str().unwrap().starts_with("4990 more errors")), "{logs:?}");
        let stats = events_named(&got, "can-stats");
        assert_eq!(stats.last().unwrap()["errors"], 5000);
    }

    #[test]
    fn bus_load_bits() {
        // Classic 8-byte frames: 111 bits plus the stuff bits they really have (0x00 data stuffs often, 0x55 never).
        let zeros = frame_bits(&CanFrame::new(0x123, &[0; 8]).unwrap()).0;
        let alt = frame_bits(&CanFrame::new(0x123, &[0x55; 8]).unwrap()).0;
        assert!((111.0..=135.0).contains(&zeros) && (111.0..=135.0).contains(&alt), "{zeros} {alt}");
        assert!(zeros > alt + 8.0, "{zeros} {alt}");
        assert!(frame_bits(&CanFrame::new(0x1234_5678, &[0x55; 8]).unwrap()).0 > alt + 18.0);
        assert_eq!(frame_bits(&CanFrame::remote(0x7DF, false, 8).unwrap()).1, 0.0);
        // Stuffing: five equal bits get one of the other value, which starts the next run.
        assert_eq!(stuff_count(&[false; 5]), 1);
        assert_eq!(stuff_count(&[false; 9]), 1);
        assert_eq!(stuff_count(&[false; 10]), 2);
        assert_eq!(stuff_count(&[true, false, true, false]), 0);
        assert_eq!(crc15(&[false; 19]), 0);
        assert_eq!(crc15(&[true]), 0x4599);
        // FD with BRS: the data phase at the data rate.
        let fd = CanFrame::fd(0x4A0, false, true, &[0xA5; 64]).unwrap();
        let (arb, data) = frame_bits(&fd);
        assert!((25.0..40.0).contains(&arb) && (545.0..660.0).contains(&data), "{arb} {data}");
        let with = Rates { nominal: Some(500_000), data: Some(2_000_000) }.bus_time(&fd).unwrap();
        let without = Rates { nominal: Some(500_000), data: None }.bus_time(&fd).unwrap();
        assert!(with < without / 3.0, "{with} {without}");
        assert!(Rates { nominal: None, data: Some(2_000_000) }.bus_time(&fd).is_none());
        // SocketCAN details as `ip -details -json` prints them.
        let v: Json = serde_json::from_str(
            r#"[{"linkinfo":{"info_data":{"bittiming":{"bitrate":500000},"data_bittiming":{"bitrate":4000000}}}}]"#,
        )
        .unwrap();
        assert_eq!(socketcan_rates_of(&v), (Some(500_000), Some(4_000_000)));
        assert_eq!(socketcan_rates_of(&json!([{"linkinfo":{"info_kind":"vcan"}}])), (None, None));
    }

    #[tokio::test]
    async fn load_is_measured_with_the_data_rate() {
        let (emit, got) = sink();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let rates = Rates { nominal: Some(500_000), data: Some(2_000_000) };
        let task = tokio::spawn(reader(rx, emit, Arc::new(RwLock::new(Shared::default())), Arc::new(StdMutex::new(None)), rates));
        let fd = CanFrame::fd(0x4A0, false, true, &[0xA5; 64]).unwrap();
        let per = rates.bus_time(&fd).unwrap();
        for _ in 0..1000 {
            tx.send(BusEvent::Frame(TimedFrame { ts_us: 0, dir: Direction::Rx, frame: fd.clone() })).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
        tx.send(BusEvent::Closed(None)).unwrap();
        task.await.unwrap();
        let stats = events_named(&got, "can-stats");
        let load = stats.iter().filter_map(|s| s["load"].as_f64()).fold(0.0, f64::max);
        assert!((load - per * 1000.0 * 100.0).abs() < 0.01, "load {load}, {per}");
    }

    #[tokio::test]
    async fn replay_handles_backwards_times_and_tight_loops() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        let mut other = core.vbus.attach();
        let mut ev = other.take_events().unwrap();
        core.connect(InterfaceConfig::Virtual).await.unwrap();
        // Unsorted: 0.5 s, then 0.1 s (backwards: no wait), then 0.9 s (0.8 s after the 0.1 s frame); at 4x.
        let t0 = std::time::Instant::now();
        core.replay_start(ReplayRequest {
            text: "(1700000000.500000) can0 123#01\n(1700000000.100000) can0 123#02\n(1700000000.900000) can0 123#03\n".into(),
            speed: 4.0,
            repeat: false,
            ids: vec![],
        })
        .await
        .unwrap();
        let mut seen = 0;
        while seen < 3 {
            match tokio::time::timeout(Duration::from_secs(3), ev.recv()).await {
                Ok(Some(BusEvent::Frame(_))) => seen += 1,
                Ok(Some(_)) => {}
                _ => panic!("replay stalled after {seen} frames"),
            }
        }
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(190) && took < Duration::from_secs(2), "{took:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(events_named(&got, "can-replay").iter().any(|p| p["done"] == true && p["error"].is_null()));

        // One frame on a loop: at most one pass per 10 ms, progress at most four times a second.
        got.lock().unwrap().clear();
        while ev.try_recv().is_ok() {}
        core.replay_start(ReplayRequest { text: "(5.0) can0 7DF#02010C\n".into(), speed: 1.0, repeat: true, ids: vec![] }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        core.replay_stop().await;
        let mut sent = 0;
        while let Ok(e) = ev.try_recv() {
            if matches!(e, BusEvent::Frame(_)) {
                sent += 1;
            }
        }
        assert!((5..=62).contains(&sent), "{sent} frames in 600 ms");
        let progress = events_named(&got, "can-replay");
        assert!(progress.len() <= 5, "{} replay events", progress.len());
        assert_eq!(progress.last().unwrap()["stopped"], "stopped");

        // Disconnecting stops a replay and says so.
        got.lock().unwrap().clear();
        core.replay_start(ReplayRequest { text: "(1.0) can0 1#01\n(100.0) can0 1#02\n".into(), speed: 1.0, repeat: false, ids: vec![] })
            .await
            .unwrap();
        core.disconnect().await;
        let r = events_named(&got, "can-replay");
        assert_eq!(r.last().unwrap()["stopped"], "disconnected", "{r:?}");
        assert_eq!(r.last().unwrap()["done"], true);
    }

    #[tokio::test]
    async fn periodic_checks_period_and_keeps_rate() {
        let (emit, _got) = sink();
        let core = Core::new(emit);
        let mut other = core.vbus.attach();
        let mut ev = other.take_events().unwrap();
        core.connect(InterfaceConfig::Virtual).await.unwrap();
        let f = || FrameIn { id: 0x321, ext: false, rtr: false, fd: false, brs: false, data: "01".into(), rtr_len: 0 };
        for bad in [None, Some(0.0), Some(0.5), Some(f64::NAN), Some(-3.0)] {
            assert!(core.periodic_start("x".into(), f(), bad, None).await.is_err(), "{bad:?}");
        }
        assert!(core.periodic_start("x".into(), f(), Some(5.0), Some(0)).await.is_err());
        // 100 frames at 2 ms: a fixed schedule takes 198 ms, never less.
        let t0 = std::time::Instant::now();
        core.periodic_start("x".into(), f(), Some(2.0), Some(100)).await.unwrap();
        let mut n = 0;
        while n < 100 {
            if let Some(BusEvent::Frame(_)) = ev.recv().await {
                n += 1;
            }
        }
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(195) && took < Duration::from_secs(3), "{took:?}");
        core.disconnect().await;
    }

    #[tokio::test]
    async fn remote_and_esi_frames_from_the_ui() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        let mut other = core.vbus.attach();
        let mut ev = other.take_events().unwrap();
        core.connect(InterfaceConfig::Virtual).await.unwrap();
        // The remote length the UI sends reaches the wire.
        let rtr = FrameIn { id: 0x7DF, ext: false, rtr: true, fd: false, brs: false, data: "".into(), rtr_len: 8 };
        core.send(rtr).await.unwrap();
        // ESI cannot be sent: it goes out without it, with a note.
        core.send_text("123##3AA".into()).await.unwrap();
        let mut wire = Vec::new();
        while wire.len() < 2 {
            if let Some(BusEvent::Frame(f)) = ev.recv().await {
                wire.push(f.frame);
            }
        }
        assert!(wire[0].rtr && wire[0].rtr_len == 8 && wire[0].dlc() == 8);
        assert!(wire[1].fd && wire[1].brs && !wire[1].esi);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(events_named(&got, "can-log").iter().any(|l| l["message"].as_str().unwrap().contains("ESI")));
        assert!(frames_of(&got).iter().all(|f| f["esi"].is_null()), "the sent frame does not show ESI");
        core.disconnect().await;
    }

    #[tokio::test]
    async fn opened_traces_are_paged() {
        let (emit, _got) = sink();
        let core = Core::new(emit);
        assert!(core.trace_page(0, 10).await.is_err());
        let mut text = String::from("; a comment\n");
        for i in 0..50_000u64 {
            text.push_str(&format!("({}.{:06}) can0 {:03X}#{:02X}\n", 1_700_000_000 + i / 1000, (i % 1000) * 1000, 0x100 + i % 3, i % 256));
        }
        text.push_str("garbage\n(1.0) can0 1#0");
        core.dbc_load_sim().await.unwrap();
        let info = core.trace_open("big.log".into(), text).await.unwrap();
        assert_eq!(info.frames, 50_000);
        assert_eq!(info.format, "candump");
        assert!(info.notes.iter().any(|n| n.contains("1 line could not be read")), "{:?}", info.notes);
        assert!(info.notes.iter().any(|n| n.contains("cut off")), "{:?}", info.notes);
        let page = core.trace_page(49_990, 2000).await.unwrap();
        assert_eq!(page.len(), 10);
        assert_eq!(page[9].id, 0x100 + 49_999 % 3);
        assert_eq!(core.trace_page(0, 1_000_000).await.unwrap().len(), TRACE_PAGE_MAX);
        assert!(core.trace_page(60_000, 10).await.unwrap().is_empty());
        let ids = core.trace_ids().await.unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids.iter().map(|r| r.count).sum::<u64>(), 50_000);
        let p = ids[0].period_ms.unwrap();
        assert!((p - 3.0).abs() < 0.01, "{p}");
        core.trace_close().await;
        assert!(core.trace_page(0, 1).await.is_err());
        // Other formats, and why a file is refused.
        let asc = "date Wed Nov 15 05:13:20.0 2023\nbase hex  timestamps absolute\n0.001 1 123 Tx d 1 05\n";
        let asc = core.trace_open("a.asc".into(), asc.into()).await.unwrap();
        assert_eq!((asc.format, asc.frames), ("Vector ASC", 1));
        assert_eq!(core.trace_page(0, 1).await.unwrap()[0].dir, Direction::Tx);
        assert!(core.trace_open("x".into(), "nothing here\n".into()).await.unwrap_err().contains("No frames"));
    }

    #[tokio::test]
    async fn dbc_warnings_wide_values_and_reassembled_labels() {
        let (emit, got) = sink();
        let core = Core::new(emit);
        let text = "BO_ 100 Wide: 8 A\n SG_ Counter : 0|64@1+ (1,0) [0|0] \"\" B\n SG_ Bad : 0|8@1+ (1,0 [0|1] \"\" B\n";
        let v = core.dbc_load("wide.dbc".into(), text.into()).await.unwrap();
        assert_eq!(v.messages[0].signals.len(), 1);
        assert_eq!(v.warnings.len(), 1);
        assert_eq!(v.warnings[0].line, 3);
        // The warning reaches the communication log with its line.
        assert!(got.lock().unwrap().iter().any(|(e, p)| e == "can-log" && p["message"].as_str().unwrap().starts_with("DBC line 3: ")));
        // A 64-bit value goes out exactly as text and comes back the same.
        let values = [("Counter".to_string(), PhysValue::Text("18446744073709551557".into()))].into_iter().collect();
        let f = core.dbc_encode("Wide".into(), values).await.unwrap();
        assert_eq!(f.data, 18446744073709551557u64.to_le_bytes());
        assert_eq!(f.sigs[0].3.as_deref(), Some("18446744073709551557"));
        assert!(core.dbc_load("notes.txt".into(), "just some text\n".into()).await.is_err());

        // An imported trace is labelled as one stream: the VIN is reassembled.
        core.set_layer(Layer::Obd2).await;
        let trace = "(1.0) can0 7DF#0209020000000000\n(1.1) can0 7E8#1014490201314847\n(1.2) can0 7E0#3000000000000000\n\
(1.3) can0 7E8#21434D3832363333\n(1.4) can0 7E8#2241303034333532\n";
        let frames = core.decode_frames(trace.into()).await.unwrap();
        assert_eq!(frames[1].label.as_deref(), Some("OBD-II response from ECU 0: ISO-TP first frame, 20 bytes: mode 09 PID 02 (VIN)"));
        assert_eq!(
            frames[4].label.as_deref(),
            Some("OBD-II response from ECU 0: ISO-TP consecutive frame 2, complete (20 bytes): VIN 1HGCM82633A004352")
        );
    }
}
