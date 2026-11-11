//! Everything the Tauri commands do, without Tauri: the open bus, the DBC, periodic transmit,
//! trace replay and the simulator. Frames go out to the UI in batches (`can-frames`, every 50 ms)
//! with their DBC decoding and protocol label already attached; `can-stats` comes once a second.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mt_can::bus::rate_text;
use mt_can::dbc::{Dbc, encode_message};
use mt_can::frame::{TimedFrame, now_us};
use mt_can::protocols::{Layer, describe};
use mt_can::sim::{SIM_DBC, SimAction, SimState, Simulator};
use mt_can::{BusEvent, CanFrame, CanHandle, CanSender, Direction, InterfaceConfig, PortInfo, VirtualBus};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

/// Event sink: `(event name, JSON payload)`.
pub type Emit = Arc<dyn Fn(&str, Json) + Send + Sync>;

const BATCH_MS: u64 = 50;
/// Frames per batch at most; above that the UI gets a "dropped" count instead (the bus is faster than
/// the screen, and the trace keeps the newest frames).
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
    /// DBC signals: `[name, value, unit, label]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sigs: Vec<(String, f64, String, Option<String>)>,
    /// J1939, CANopen or OBD-II label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn view(f: &TimedFrame, dbc: Option<&Dbc>, layer: Layer) -> FrameView {
    let (msg, sigs) = match dbc.and_then(|d| d.decode(&f.frame)) {
        Some((m, v)) => (Some(m.name.clone()), v.into_iter().map(|s| (s.name, s.value, s.unit, s.label)).collect()),
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
        label: describe(&f.frame, layer),
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
}

fn dbc_view(name: &str, d: &Dbc) -> DbcView {
    DbcView {
        name: name.to_string(),
        version: d.version.clone(),
        nodes: d.nodes.clone(),
        skipped: d.skipped,
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
                        multiplex: s.multiplex.as_ref().map(|m| match m {
                            mt_can::dbc::Multiplex::Switch => "M".to_string(),
                            mt_can::dbc::Multiplex::Value(v) => format!("m{v}"),
                        }),
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
    /// candump log text.
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
}

struct Bus {
    handle: CanHandle,
    sender: CanSender,
    reader: JoinHandle<()>,
    bitrate: Option<u32>,
}

#[derive(Default)]
struct Shared {
    dbc: Option<(String, Dbc)>,
    layer: Layer,
}

pub struct Core {
    emit: Emit,
    vbus: VirtualBus,
    bus: Mutex<Option<Bus>>,
    shared: Arc<RwLock<Shared>>,
    periodic: Mutex<HashMap<String, JoinHandle<()>>>,
    replay: Mutex<Option<JoinHandle<()>>>,
    sim: Mutex<Option<Simulator>>,
}

impl Core {
    pub fn new(emit: Emit) -> Self {
        Core {
            emit,
            vbus: VirtualBus::new(),
            bus: Mutex::new(None),
            shared: Arc::new(RwLock::new(Shared::default())),
            periodic: Mutex::new(HashMap::new()),
            replay: Mutex::new(None),
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
        let bitrate = match &cfg {
            InterfaceConfig::Slcan { bitrate, .. } | InterfaceConfig::GsUsb { bitrate, .. } => Some(*bitrate),
            InterfaceConfig::Virtual => Some(500_000),
            InterfaceConfig::Socketcan { ifname } => socketcan_bitrate(ifname),
        };
        let can_send =
            !matches!(cfg, InterfaceConfig::Slcan { listen_only: true, .. } | InterfaceConfig::GsUsb { listen_only: true, .. });
        let reader = tokio::spawn(reader(events, self.emit.clone(), self.shared.clone(), bitrate));
        let sender = handle.sender();
        let description = handle.description.clone();
        *self.bus.lock().await = Some(Bus { handle, sender, reader, bitrate });
        let rate = bitrate.map(|b| format!(" ({})", rate_text(b))).unwrap_or_default();
        self.log("info", format!("Opened {description}{rate}"));
        Ok(ConnInfo { description, log_name: cfg.log_name(), can_send })
    }

    pub async fn disconnect(&self) {
        self.stop_all_periodic().await;
        self.replay_stop().await;
        if let Some(b) = self.bus.lock().await.take() {
            b.handle.close().await;
            // The reader ends on `Closed`; give it a moment to flush its last batch.
            let _ = tokio::time::timeout(Duration::from_millis(200), b.reader).await;
        }
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
        let f = mt_can::log::parse_frame(&text).map_err(es)?;
        self.sender().await?.send(f).await.map_err(es)
    }

    pub async fn periodic_start(&self, key: String, frame: FrameIn, period_ms: u64, count: Option<u64>) -> Result<(), String> {
        let f = frame.frame()?;
        if period_ms == 0 {
            return Err("The period must be at least 1 ms.".into());
        }
        let tx = self.sender().await?;
        let emit = self.emit.clone();
        let k = key.clone();
        let task = tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_millis(period_ms));
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut n = 0u64;
            loop {
                t.tick().await;
                if let Err(e) = tx.send(f.clone()).await {
                    emit("can-periodic", json!({ "key": k, "stopped": true, "error": e.to_string() }));
                    return;
                }
                n += 1;
                if count.is_some_and(|c| n >= c) {
                    emit("can-periodic", json!({ "key": k, "stopped": true, "sent": n }));
                    return;
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

    // ── DBC ──

    pub async fn dbc_load(&self, name: String, text: String) -> Result<DbcView, String> {
        let d = mt_can::Dbc::parse(&text).map_err(es)?;
        let v = dbc_view(&name, &d);
        self.log("info", format!("Loaded {name}: {} messages, {} signals", d.messages.len(), d.signal_count()));
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
    pub async fn dbc_encode(&self, message: String, values: HashMap<String, f64>) -> Result<FrameView, String> {
        let s = self.shared.read().await;
        let (_, d) = s.dbc.as_ref().ok_or("No DBC file is loaded.")?;
        let m = d.message_by_name(&message).ok_or_else(|| format!("{message} is not in the DBC file."))?;
        let data = encode_message(m, &values, None).map_err(es)?;
        // The DBC's frame format (VFrameFormat, CANFD_BRS); anything over 8 bytes is FD regardless.
        let fd = m.fd || data.len() > 8;
        let frame = CanFrame { id: m.id, extended: m.extended, rtr: false, fd, brs: fd && m.brs, esi: false, data, rtr_len: 0 };
        frame.validate().map_err(es)?;
        Ok(view(&TimedFrame { ts_us: now_us(), dir: Direction::Tx, frame }, Some(d), s.layer))
    }

    pub async fn set_layer(&self, layer: Layer) {
        self.shared.write().await.layer = layer;
    }

    /// Decodes frames that were recorded earlier (an imported trace) with the current DBC and layer.
    pub async fn decode_frames(&self, text: String) -> Result<Vec<FrameView>, String> {
        let recs = mt_can::log::parse_candump(&text).map_err(es)?;
        let s = self.shared.read().await;
        let dbc = s.dbc.as_ref().map(|(_, d)| d);
        Ok(recs
            .into_iter()
            .map(|r| view(&TimedFrame { ts_us: r.ts_us, dir: Direction::Rx, frame: r.frame }, dbc, s.layer))
            .collect())
    }

    // ── Replay ──

    pub async fn replay_start(&self, req: ReplayRequest) -> Result<ReplayInfo, String> {
        let mut recs = mt_can::log::parse_candump(&req.text).map_err(es)?;
        if !req.ids.is_empty() {
            recs.retain(|r| req.ids.contains(&r.frame.id));
        }
        if recs.is_empty() {
            return Err("The log has no frames to replay.".into());
        }
        let tx = self.sender().await?;
        self.replay_stop().await;
        let speed = if req.speed > 0.0 { req.speed } else { 1.0 };
        let t0 = recs[0].ts_us;
        let duration_s = recs.last().map(|r| r.ts_us.saturating_sub(t0)).unwrap_or(0) as f64 / 1e6;
        let n = recs.len();
        let emit = self.emit.clone();
        let repeat = req.repeat;
        let task = tokio::spawn(async move {
            let mut pass = 0u64;
            // Progress goes to the UI four times a second and at the end of every pass.
            let mut last_progress = tokio::time::Instant::now();
            loop {
                let start = tokio::time::Instant::now();
                for (i, r) in recs.iter().enumerate() {
                    let at = Duration::from_micros(((r.ts_us - t0) as f64 / speed) as u64);
                    tokio::time::sleep_until(start + at).await;
                    if let Err(e) = tx.send(r.frame.clone()).await {
                        emit("can-replay", json!({ "done": true, "error": e.to_string() }));
                        return;
                    }
                    if last_progress.elapsed() >= Duration::from_millis(250) {
                        last_progress = tokio::time::Instant::now();
                        emit("can-replay", json!({ "done": false, "pos": i + 1, "total": n, "pass": pass }));
                    }
                }
                pass += 1;
                if repeat {
                    emit("can-replay", json!({ "done": false, "pos": n, "total": n, "pass": pass - 1 }));
                }
                if !repeat {
                    break;
                }
            }
            emit("can-replay", json!({ "done": true, "pos": n, "total": n }));
        });
        *self.replay.lock().await = Some(task);
        Ok(ReplayInfo { frames: n, duration_s })
    }

    pub async fn replay_stop(&self) {
        if let Some(t) = self.replay.lock().await.take() {
            t.abort();
        }
    }

    // ── Simulator ──

    pub async fn sim_start(&self) -> Result<(), String> {
        let mut s = self.sim.lock().await;
        if s.is_none() {
            *s = Some(Simulator::start(&self.vbus));
            self.log("info", "Simulator started on the virtual bus");
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
}

/// The nominal bit rate of a SocketCAN interface, from `ip -details` style sysfs data when present.
fn socketcan_bitrate(ifname: &str) -> Option<u32> {
    let out = std::process::Command::new("ip").args(["-details", "-json", "link", "show", ifname]).output().ok()?;
    let v: Json = serde_json::from_slice(&out.stdout).ok()?;
    v.get(0)?.get("linkinfo")?.get("info_data")?.get("bittiming")?.get("bitrate")?.as_u64().map(|b| b as u32)
}

async fn reader(
    mut events: tokio::sync::mpsc::UnboundedReceiver<BusEvent>,
    emit: Emit,
    shared: Arc<RwLock<Shared>>,
    bitrate: Option<u32>,
) {
    let mut batch: Vec<FrameView> = Vec::new();
    let mut stats = Stats::default();
    let mut dropped_in_batch = 0u64;
    let mut bits_this_sec = 0.0f64;
    let (mut rx_sec, mut tx_sec) = (0u64, 0u64);
    let mut flush = tokio::time::interval(Duration::from_millis(BATCH_MS));
    let mut second = tokio::time::interval(Duration::from_secs(1));
    second.tick().await;
    let closed_reason: Option<String> = loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(BusEvent::Frame(f)) => {
                    match f.dir {
                        Direction::Rx => { stats.rx += 1; rx_sec += 1; }
                        Direction::Tx => { stats.tx += 1; tx_sec += 1; }
                    }
                    bits_this_sec += f.frame.wire_bits(4.0);
                    if batch.len() < MAX_BATCH {
                        let s = shared.read().await;
                        batch.push(view(&f, s.dbc.as_ref().map(|(_, d)| d), s.layer));
                    } else {
                        dropped_in_batch += 1;
                        stats.dropped += 1;
                    }
                }
                Some(BusEvent::Error(e)) => {
                    stats.errors += 1;
                    emit("can-log", json!({ "direction": "error", "message": e }));
                }
                Some(BusEvent::Info(i)) => emit("can-log", json!({ "direction": "info", "message": i })),
                Some(BusEvent::Closed(reason)) => break reason,
                None => break None,
            },
            _ = flush.tick() => {
                if !batch.is_empty() || dropped_in_batch > 0 {
                    emit("can-frames", json!({ "frames": std::mem::take(&mut batch), "dropped": dropped_in_batch }));
                    dropped_in_batch = 0;
                }
            }
            _ = second.tick() => {
                stats.rx_per_sec = rx_sec as f64;
                stats.tx_per_sec = tx_sec as f64;
                stats.load = bitrate.filter(|b| *b > 0).map(|b| (bits_this_sec / b as f64 * 100.0).min(100.0));
                rx_sec = 0; tx_sec = 0; bits_this_sec = 0.0;
                emit("can-stats", serde_json::to_value(&stats).unwrap_or(Json::Null));
            }
        }
    };
    if !batch.is_empty() {
        emit("can-frames", json!({ "frames": batch, "dropped": dropped_in_batch }));
    }
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
        core.sim_start().await.unwrap();
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
        core.periodic_start("a".into(), f, 10, Some(3)).await.unwrap();
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
        let v = core.dbc_encode("ThrottleCommand".into(), [("Throttle".to_string(), 50.0)].into_iter().collect()).await.unwrap();
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

        let v = core.dbc_encode("HostFdPing".into(), [("Count".to_string(), 5.0)].into_iter().collect()).await.unwrap();
        assert!(v.fd && v.brs && v.data.len() == 64 && v.dlc == 15);
        let hb = core.dbc_encode("HostPing".into(), HashMap::new()).await.unwrap();
        assert!(!hb.fd && !hb.brs);
        core.disconnect().await;
    }
}
