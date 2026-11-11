//! The app core, independent of Tauri so it is tested directly: the current capture, the decoder
//! stack and its cached results, the capture thread, files and firmware.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mt_can::dbc::Dbc;
use mt_can::log::parse_candump;
use mt_logic::can_link;
use mt_logic::capture::Measurement;
use mt_logic::decode::can::{CanBitFrame, CanConfig};
use mt_logic::engine::{self, CaptureSettings, Phase, SampleStream};
use mt_logic::fx2::firmware::{self, FirmwareDirs, FoundFirmware};
use mt_logic::fx2::{self, ConnectOptions, ConnectStep, Detected, Fx2Lafw, UsbBackend};
use mt_logic::sim::{self, ScenarioId, ScenarioInfo, SimStream};
use mt_logic::{Annotation, Capture, ChannelInfo, DecodeOutput, DecoderConfig, Level, LogicError, Table, format};
use serde::{Deserialize, Serialize};

pub type Emit = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Source {
    Simulator { scenario: ScenarioId },
    Fx2 { id: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    pub source: Source,
    pub settings: CaptureSettings,
    pub channels: Vec<ChannelInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureInfo {
    pub sample_rate: u64,
    pub samples: u64,
    pub trigger: Option<u64>,
    pub channels: Vec<ChannelInfo>,
    /// "Simulator: Modbus RTU on RS-485", "Saleae Logic compatible", or the file name.
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Devices {
    pub scenarios: Vec<ScenarioInfo>,
    pub boards: Vec<Detected>,
    /// Listing USB devices failed (shown, not fatal: the simulator still works).
    pub usb_error: Option<String>,
    pub sample_rates: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecoderView {
    pub id: u32,
    pub rows: Vec<String>,
    pub summary: String,
    pub table: Option<Table>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub info: CaptureInfo,
    pub session: serde_json::Value,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareStatus {
    /// Per board firmware file: where it was found, if anywhere.
    pub files: Vec<(String, Option<FoundFirmware>)>,
    pub app_dir: Option<String>,
    pub bundled_dir: Option<String>,
    pub user_file: Option<String>,
    pub release: &'static str,
    pub licence: &'static str,
    pub source_url: &'static str,
    /// Our copy of the source, offered with the bundled images.
    pub mirror_source_url: &'static str,
    pub download_url: &'static str,
}

struct Running {
    stop: Arc<AtomicBool>,
}

struct Decoded {
    config: DecoderConfig,
    out: Arc<DecodeOutput>,
}

pub struct Core {
    emit: Emit,
    usb: Option<Arc<dyn UsbBackend>>,
    fw: Mutex<FirmwareDirs>,
    capture: Mutex<Option<(Arc<Capture>, String)>>,
    decoders: Mutex<HashMap<u32, Decoded>>,
    /// Wall-clock time of the current capture's first sample (µs since the epoch), for live captures.
    wall_start_us: Mutex<Option<u64>>,
    running: Mutex<Option<Running>>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Core {
    pub fn new(emit: Emit, usb: Option<Arc<dyn UsbBackend>>, app_fw_dir: Option<PathBuf>, bundled_fw_dir: Option<PathBuf>) -> Self {
        let fw = FirmwareDirs { user_file: None, app_dir: app_fw_dir, bundled_dir: bundled_fw_dir, system_dirs: firmware::system_dirs() };
        Self { emit, usb, fw: Mutex::new(fw), capture: Mutex::new(None), decoders: Mutex::new(HashMap::new()), wall_start_us: Mutex::new(None), running: Mutex::new(None) }
    }

    fn emit(&self, event: &str, v: impl Serialize) {
        (self.emit)(event, serde_json::to_value(v).unwrap_or_default());
    }

    pub fn devices(&self) -> Devices {
        let (boards, usb_error) = match &self.usb {
            Some(u) => match fx2::detect(u.as_ref()) {
                Ok(b) => (b, None),
                Err(e) => (vec![], Some(e.to_string())),
            },
            None => (vec![], Some("USB support is not built into this version".into())),
        };
        Devices { scenarios: sim::SCENARIOS.iter().map(|&s| sim::info(s)).collect(), boards, usb_error, sample_rates: fx2::SAMPLE_RATES.to_vec() }
    }

    // ── Firmware ──

    pub fn firmware_status(&self) -> FirmwareStatus {
        let fw = self.fw.lock().unwrap();
        let mut names: Vec<&str> = fx2::BOARDS.iter().map(|b| b.firmware).collect();
        names.dedup();
        FirmwareStatus {
            files: names.into_iter().map(|n| (n.to_string(), fw.find(n))).collect(),
            app_dir: fw.app_dir.as_ref().map(|p| p.display().to_string()),
            bundled_dir: fw.bundled_dir.as_ref().map(|p| p.display().to_string()),
            user_file: fw.user_file.as_ref().map(|p| p.display().to_string()),
            release: firmware::RELEASE,
            licence: firmware::LICENCE,
            source_url: firmware::SOURCE_URL,
            mirror_source_url: firmware::MIRROR_SOURCE_URL,
            download_url: firmware::TARBALL_URL,
        }
    }

    pub fn firmware_choose(&self, path: Option<String>) -> Result<FirmwareStatus, String> {
        if let Some(p) = &path {
            let b = std::fs::read(p).map_err(err)?;
            firmware::check_image(&b).map_err(err)?;
        }
        self.fw.lock().unwrap().user_file = path.map(PathBuf::from);
        Ok(self.firmware_status())
    }

    pub fn firmware_dir(&self) -> Option<PathBuf> {
        self.fw.lock().unwrap().app_dir.clone()
    }

    // ── Capture ──

    pub fn is_running(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    /// Starts a capture on its own thread. Events: `la-progress`, then `la-capture` (the new
    /// capture's info) or `la-error`.
    pub fn start(self: &Arc<Self>, req: CaptureRequest) -> Result<(), String> {
        req.settings.validate().map_err(err)?;
        let mut running = self.running.lock().unwrap();
        if running.is_some() {
            return Err("a capture is already running".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        *running = Some(Running { stop: stop.clone() });
        drop(running);
        let me = self.clone();
        std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || {
                let r = me.capture_blocking(&req, &stop);
                *me.running.lock().unwrap() = None;
                match r {
                    Ok(info) => me.emit("la-capture", info),
                    Err(e) => me.emit("la-error", serde_json::json!({ "message": e })),
                }
            })
            .map_err(err)?;
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().unwrap().as_ref() {
            r.stop.store(true, Ordering::Relaxed);
        }
    }

    fn open_stream(&self, req: &CaptureRequest) -> Result<(Box<dyn SampleStream>, String), String> {
        match &req.source {
            Source::Simulator { scenario } => {
                let s = SimStream::new(*scenario, req.settings.sample_rate, true).map_err(err)?;
                Ok((Box::new(s), format!("Simulator: {}", sim::info(*scenario).name)))
            }
            Source::Fx2 { id } => {
                let usb = self.usb.clone().ok_or("USB support is not built into this version")?;
                let fw = self.fw.lock().unwrap().clone();
                let dev = Fx2Lafw::connect(usb.as_ref(), id, &fw, &ConnectOptions::default(), &mut |step| {
                    let text = match &step {
                        ConnectStep::Opening => "Opening the board".to_string(),
                        ConnectStep::UploadingFirmware { file } => format!("Loading {file} into the board"),
                        ConnectStep::WaitingForRenumeration => "Waiting for the board to restart with the firmware".to_string(),
                        ConnectStep::Ready { version } => format!("fx2lafw {version} running"),
                    };
                    self.emit("la-log", serde_json::json!({ "direction": "info", "message": text }));
                })
                .map_err(|e| match e {
                    LogicError::Permission(m) => m,
                    e => e.to_string(),
                })?;
                let name = dev.board.name.to_string();
                Ok((Box::new(dev.start(req.settings.sample_rate).map_err(err)?), name))
            }
        }
    }

    fn capture_blocking(&self, req: &CaptureRequest, stop: &AtomicBool) -> Result<CaptureInfo, String> {
        let (mut stream, source) = self.open_stream(req)?;
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut phase = None;
        let cap = engine::run(stream.as_mut(), &req.settings, req.channels.clone(), stop, &mut |p| {
            // Progress at most 10 times a second, and on every phase change.
            if phase != Some(p.phase) || last.elapsed() >= Duration::from_millis(100) {
                phase = Some(p.phase);
                last = Instant::now();
                self.emit("la-progress", serde_json::json!({ "phase": if p.phase == Phase::Waiting { "waiting" } else { "recording" }, "samples": p.samples, "total": p.total }));
            }
        })
        .map_err(err)?;
        if cap.is_empty() {
            return Err("the capture was stopped before any samples arrived".into());
        }
        let span_us = (cap.len() as f64 / cap.sample_rate as f64 * 1e6) as u64;
        let info = self.set_capture(cap, source);
        *self.wall_start_us.lock().unwrap() = Some(now_us().saturating_sub(span_us));
        Ok(info)
    }

    /// Makes `cap` the current capture; cached decoder results are dropped (the app re-runs its stack).
    pub fn set_capture(&self, cap: Capture, source: String) -> CaptureInfo {
        let info = info_of(&cap, &source);
        *self.capture.lock().unwrap() = Some((Arc::new(cap), source));
        *self.wall_start_us.lock().unwrap() = None;
        self.decoders.lock().unwrap().clear();
        info
    }

    fn cap(&self) -> Result<Arc<Capture>, String> {
        self.capture.lock().unwrap().as_ref().map(|c| c.0.clone()).ok_or_else(|| "no capture yet".to_string())
    }

    pub fn info(&self) -> Option<CaptureInfo> {
        self.capture.lock().unwrap().as_ref().map(|(c, s)| info_of(c, s))
    }

    /// Renames and recolours channels of the current capture (kept in saved files).
    pub fn set_channels(&self, channels: Vec<ChannelInfo>) -> Result<(), String> {
        let mut g = self.capture.lock().unwrap();
        let (c, src) = g.as_ref().ok_or("no capture yet")?;
        let mut cap = (**c).clone();
        for (i, ch) in channels.into_iter().enumerate().take(cap.channels.len()) {
            cap.channels[i] = ch;
        }
        *g = Some((Arc::new(cap), src.clone()));
        Ok(())
    }

    pub fn waveform(&self, start: u64, end: u64, buckets: usize) -> Result<Vec<u8>, String> {
        let cap = self.cap()?;
        Ok(cap.summary(start, end, buckets.min(8192), cap.channels.len().min(8)))
    }

    pub fn edge(&self, channel: u8, from: u64, forward: bool) -> Result<Option<u64>, String> {
        let cap = self.cap()?;
        Ok(if forward { cap.next_edge(channel, from) } else { cap.prev_edge(channel, from) })
    }

    pub fn measure(&self, channel: u8, start: u64, end: u64) -> Result<Option<Measurement>, String> {
        Ok(self.cap()?.measure(channel, start, end))
    }

    pub fn estimate_baud(&self, channel: u8) -> Result<Option<f64>, String> {
        let cap = self.cap()?;
        Ok(mt_logic::decode::uart::estimate_baud(&cap, channel))
    }

    // ── Decoders ──

    pub fn decode(&self, id: u32, config: DecoderConfig) -> Result<DecoderView, String> {
        let cap = self.cap()?;
        let out = Arc::new(match config.decode_can(&cap) {
            Some(r) => {
                let (mut out, frames) = r?;
                let DecoderConfig::Can(c) = &config else { unreachable!() };
                can_extras(&mut out, &cap, &frames, c)?;
                out
            }
            None => config.decode(&cap)?,
        });
        let view = DecoderView { id, rows: out.rows.clone(), summary: out.summary.clone(), table: out.table.clone(), count: out.annotations.len() };
        self.decoders.lock().unwrap().insert(id, Decoded { config, out });
        Ok(view)
    }

    /// The frames of CAN decoder `id` as a candump log that CAN Explorer opens in Replay and Trace.
    /// Timed from the capture's wall-clock start for live captures, from 0 for opened files.
    pub fn export_candump(&self, id: u32, path: &Path) -> Result<usize, String> {
        let cap = self.cap()?;
        let config = self.decoders.lock().unwrap().get(&id).map(|d| d.config.clone()).ok_or("decoder not found")?;
        let (_, frames) = config.decode_can(&cap).ok_or("only a CAN decoder exports a candump log")??;
        let start = self.wall_start_us.lock().unwrap().unwrap_or(0);
        let text = can_link::to_candump(&cap, &frames, start, "can0");
        std::fs::write(path, &text).map_err(err)?;
        Ok(text.lines().count())
    }

    pub fn remove_decoder(&self, id: u32) {
        self.decoders.lock().unwrap().remove(&id);
    }

    pub fn decoder_configs(&self) -> Vec<(u32, DecoderConfig)> {
        let mut v: Vec<_> = self.decoders.lock().unwrap().iter().map(|(k, d)| (*k, d.config.clone())).collect();
        v.sort_by_key(|x| x.0);
        v
    }

    /// Annotations of decoder `id` overlapping `[start, end)`. Boxes narrower than
    /// `min_width` samples are merged into dense blocks (empty text) so a zoomed-out view of a
    /// CAN bus does not ship a million bit boxes.
    pub fn annotations(&self, id: u32, start: u64, end: u64, min_width: f64) -> Result<Vec<Annotation>, String> {
        let out = self.decoders.lock().unwrap().get(&id).map(|d| d.out.clone()).ok_or("decoder not found")?;
        Ok(window(&out.annotations, start, end, min_width))
    }

    // ── Files ──

    pub fn open_file(&self, path: &Path) -> Result<Opened, String> {
        let bytes = std::fs::read(path).map_err(err)?;
        let (cap, session, warnings) = format::open(&bytes).map_err(err)?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let info = self.set_capture(cap, name);
        Ok(Opened { info, session, warnings })
    }

    /// Saves as `.vla` (with the session) or `.sr` (samples and channel names only), by extension.
    pub fn save_file(&self, path: &Path, session: &serde_json::Value) -> Result<(), String> {
        let cap = self.cap()?;
        let tmp = path.with_extension("part");
        let f = std::fs::File::create(&tmp).map_err(err)?;
        let mut w = std::io::BufWriter::new(f);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let r = match ext.as_str() {
            "sr" => format::sigrok::write_sr(&mut w, &cap).map_err(err),
            "vcd" => format::vcd::write_vcd(&mut w, &cap).map_err(err),
            _ => format::write_vla(&mut w, &cap, session).map_err(err),
        };
        let r = r.and_then(|_| std::io::Write::flush(&mut w).map_err(err));
        drop(w);
        match r {
            Ok(()) => std::fs::rename(&tmp, path).map_err(err),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

/// The CAN Explorer link on a CAN decoder's output: DBC names, then the log comparison.
fn can_extras(out: &mut DecodeOutput, cap: &Capture, frames: &[CanBitFrame], c: &CanConfig) -> Result<(), String> {
    if let Some(path) = &c.dbc {
        let text = if path == BUILTIN_SIM_DBC {
            can_link::SIM_DBC.to_string()
        } else {
            std::fs::read_to_string(path).map_err(|e| format!("cannot read the DBC file {path}: {e}"))?
        };
        let dbc = Dbc::parse(&text).map_err(|e| format!("DBC file: {e}"))?;
        can_link::add_signals(out, frames, &dbc);
    }
    if let Some(path) = &c.log {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read the log {path}: {e}"))?;
        let log = parse_candump(&text).map_err(|e| format!("candump log: {e}"))?;
        can_link::compare_log(out, cap, frames, &log, (c.log_window_ms * 1000.0).round().max(1.0) as i64);
    }
    Ok(())
}

/// `CanConfig::dbc` value that selects CAN Explorer's simulator DBC.
pub const BUILTIN_SIM_DBC: &str = "builtin:can-explorer-sim";

fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

fn info_of(c: &Capture, source: &str) -> CaptureInfo {
    CaptureInfo { sample_rate: c.sample_rate, samples: c.len(), trigger: c.trigger, channels: c.channels.clone(), source: source.to_string() }
}

fn worse(a: Level, b: Level) -> Level {
    let rank = |l: Level| match l {
        Level::Error => 4,
        Level::Warning => 3,
        Level::Info => 2,
        Level::Control => 1,
        Level::Data => 0,
    };
    if rank(b) > rank(a) { b } else { a }
}

pub fn window(all: &[Annotation], start: u64, end: u64, min_width: f64) -> Vec<Annotation> {
    let mut out: Vec<Annotation> = Vec::new();
    // Open dense block per row.
    let mut dense: HashMap<u8, Annotation> = HashMap::new();
    let gap = min_width.max(1.0) as u64;
    for a in all.iter().filter(|a| a.end > start && a.start < end) {
        if ((a.end - a.start) as f64) >= min_width {
            out.push(a.clone());
            continue;
        }
        match dense.get_mut(&a.row) {
            Some(d) if a.start <= d.end + gap => {
                d.end = d.end.max(a.end);
                d.level = worse(d.level, a.level);
            }
            _ => {
                if let Some(d) = dense.insert(a.row, Annotation::new(a.row, a.start, a.end, a.level, "", "")) {
                    out.push(d);
                }
            }
        }
    }
    out.extend(dense.into_values());
    out.sort_by_key(|a| (a.row, a.start));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_logic::fx2::mock::MockFx2;
    use mt_logic::trigger::{Edge, Trigger};

    type Events = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    fn core(usb: Option<Arc<dyn UsbBackend>>) -> (Arc<Core>, Events) {
        let ev: Events = Arc::default();
        let e = ev.clone();
        let c = Arc::new(Core::new(Arc::new(move |n: &str, v| e.lock().unwrap().push((n.to_string(), v))), usb, None, None));
        (c, ev)
    }

    fn wait_done(ev: &Events) -> (String, serde_json::Value) {
        let t = Instant::now();
        loop {
            if let Some(x) = ev.lock().unwrap().iter().find(|(n, _)| n == "la-capture" || n == "la-error").cloned() {
                return x;
            }
            assert!(t.elapsed() < Duration::from_secs(20), "capture did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn sim_request(id: ScenarioId, samples: u64, trigger: Trigger) -> CaptureRequest {
        let i = sim::info(id);
        CaptureRequest {
            source: Source::Simulator { scenario: id },
            settings: CaptureSettings { sample_rate: i.default_rate, samples, trigger, pre_trigger: 0.1 },
            channels: i.channels,
        }
    }

    #[test]
    fn simulator_capture_decode_and_annotations() {
        let (c, ev) = core(None);
        c.start(sim_request(ScenarioId::ModbusRtu, 500_000, Trigger::Edge { channel: 2, edge: Edge::Rising })).unwrap();
        assert!(c.start(sim_request(ScenarioId::ModbusRtu, 10, Trigger::None)).is_err(), "one capture at a time");
        let (name, v) = wait_done(&ev);
        assert_eq!(name, "la-capture", "{v}");
        assert_eq!(v["samples"], 500_000);
        // The first DE edge comes 19.8 ms in: less pre-trigger than the 10 % asked for.
        assert_eq!(v["trigger"], 19_800);
        assert!(ev.lock().unwrap().iter().any(|(n, _)| n == "la-progress"));
        assert!(!c.is_running());

        let d = sim::info(ScenarioId::ModbusRtu).decoders[0].clone();
        let view = c.decode(1, d).unwrap();
        let table = view.table.unwrap();
        assert!(table.rows.len() >= 4);
        assert_eq!(table.rows[0].cells[6], "OK");
        let all = c.annotations(1, 0, 500_000, 0.0).unwrap();
        assert_eq!(all.len(), view.count);
        // Zoomed far out (1000 samples per pixel, boxes under 3 px merged): the byte boxes merge, frame boxes stay.
        let coarse = c.annotations(1, 0, 500_000, 3000.0).unwrap();
        assert!(coarse.len() < all.len() / 3, "{} vs {}", coarse.len(), all.len());
        let w = c.waveform(0, 500_000, 1000).unwrap();
        assert_eq!(w.len(), 8 * 1000);
    }

    #[test]
    fn stop_returns_partial_capture() {
        let (c, ev) = core(None);
        c.start(sim_request(ScenarioId::UartConsole, 100_000_000, Trigger::None)).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        c.stop();
        let (name, v) = wait_done(&ev);
        assert_eq!(name, "la-capture", "{v}");
        assert!(v["samples"].as_u64().unwrap() < 100_000_000);
    }

    #[test]
    fn fx2_capture_through_the_mock() {
        let mock = MockFx2::new(true).with_scenario(ScenarioId::ModbusRtu);
        let (c, ev) = core(Some(Arc::new(mock)));
        let d = c.devices();
        assert_eq!(d.boards.len(), 1);
        let id = d.boards[0].info.id.clone();
        c.start(CaptureRequest {
            source: Source::Fx2 { id },
            settings: CaptureSettings { sample_rate: 1_000_000, samples: 200_000, trigger: Trigger::None, pre_trigger: 0.0 },
            channels: mt_logic::capture::default_channels(8),
        })
        .unwrap();
        let (name, v) = wait_done(&ev);
        assert_eq!(name, "la-capture", "{v}");
        assert!(v["source"].as_str().unwrap().contains("Saleae"));
        assert!(ev.lock().unwrap().iter().any(|(n, v)| n == "la-log" && v["message"].as_str().unwrap().contains("fx2lafw 1.4")));
    }

    #[test]
    fn fx2_without_firmware_explains_itself() {
        let mock = MockFx2::new(false);
        let (c, ev) = core(Some(Arc::new(mock)));
        let id = c.devices().boards[0].info.id.clone();
        c.start(CaptureRequest {
            source: Source::Fx2 { id },
            settings: CaptureSettings { sample_rate: 1_000_000, samples: 1000, trigger: Trigger::None, pre_trigger: 0.0 },
            channels: mt_logic::capture::default_channels(8),
        })
        .unwrap();
        let (name, v) = wait_done(&ev);
        assert_eq!(name, "la-error");
        assert!(v["message"].as_str().unwrap().contains("fx2lafw-saleae-logic.fw"));
    }

    #[test]
    fn save_and_open_every_format() {
        let (c, _) = core(None);
        let i = sim::info(ScenarioId::I2cSensors);
        c.set_capture(Capture::new(i.default_rate, i.channels.clone(), sim::render(ScenarioId::I2cSensors, i.default_rate)), "test".into());
        c.set_channels(vec![ChannelInfo { name: "SCL bus".into(), color: Some("#ff0".into()) }]).unwrap();
        let dir = std::env::temp_dir().join(format!("la-core-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = serde_json::json!({ "cursors": [1, 2] });
        for ext in ["vla", "sr"] {
            let p = dir.join(format!("x.{ext}"));
            c.save_file(&p, &session).unwrap();
            let o = c.open_file(&p).unwrap();
            assert_eq!(o.info.samples, (i.duration * i.default_rate as f64) as u64);
            assert_eq!(o.info.channels[0].name, "SCL bus");
            if ext == "vla" {
                assert_eq!(o.session, session);
                assert_eq!(o.info.channels[0].color.as_deref(), Some("#ff0"));
            }
        }
        let p = dir.join("x.vcd");
        c.save_file(&p, &session).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("$enddefinitions"));
        assert!(c.open_file(&p).is_err());
        assert!(!dir.join("x.part").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dense_windows_keep_worst_level() {
        let a = |s, e, l| Annotation::new(0, s, e, l, "x", "x");
        let all = vec![a(0, 2, Level::Data), a(2, 4, Level::Error), a(4, 6, Level::Data), a(100, 200, Level::Info), a(300, 301, Level::Data)];
        let w = window(&all, 0, 1000, 10.0);
        assert_eq!(w.len(), 3);
        assert_eq!((w[0].start, w[0].end, w[0].level), (0, 6, Level::Error));
        assert_eq!(w[1].text, "x");
        assert_eq!(window(&all, 150, 160, 1.0).len(), 1);
    }

    /// The images shipped in the bundle are byte-identical to sigrok's 0.1.7 release, cover every
    /// board, and travel with their licences and the source offer.
    #[test]
    fn bundled_firmware_is_the_official_release() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("firmware").join(format!("fx2lafw-{}", firmware::RELEASE));
        for (name, _) in firmware::KNOWN {
            let b = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(firmware::verify(name, &b), firmware::Verified::Official, "{name}");
        }
        for b in fx2::BOARDS {
            assert!(dir.join(b.firmware).exists(), "{} has no bundled firmware", b.firmware);
        }
        for f in ["COPYING", "COPYING.LESSER", "README"] {
            assert!(dir.join(f).exists(), "{f}");
        }
        let src = std::fs::read_to_string(dir.join("SOURCE.txt")).unwrap();
        assert!(src.contains(firmware::MIRROR_SOURCE_URL) && src.contains(firmware::SOURCE_SHA256) && src.contains(firmware::TARBALL_SHA256));
        let fw = FirmwareDirs { bundled_dir: Some(dir), ..Default::default() };
        assert_eq!(fw.find("fx2lafw-saleae-logic.fw").unwrap().source, "bundled with the app");
    }

    /// CAN Explorer link end to end: the CAN scenario named by CAN Explorer's simulator DBC, its
    /// frames exported as candump, and that log compared back with the capture.
    #[test]
    fn can_explorer_link() {
        let (c, _) = core(None);
        let i = sim::info(ScenarioId::CanBus);
        c.set_capture(Capture::new(i.default_rate, i.channels.clone(), sim::render(ScenarioId::CanBus, i.default_rate)), "test".into());
        let DecoderConfig::Can(base) = i.decoders[0].clone() else { panic!("the CAN scenario starts with a CAN decoder") };
        let named = c.decode(1, DecoderConfig::Can(CanConfig { dbc: Some(BUILTIN_SIM_DBC.into()), ..base.clone() })).unwrap();
        assert_eq!(named.rows[4], "Signals");
        let t = named.table.unwrap();
        assert!(t.rows.iter().any(|r| r.cells.contains(&"EEC1".to_string())), "{:?}", t.rows);

        let dir = std::env::temp_dir().join(format!("la-can-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("bus.log");
        let n = c.export_candump(1, &log).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(n, text.lines().count());
        // 7 intact frames; the unacknowledged one ends in an error flag and the CRC error is damaged, so neither is exported.
        assert!(n == 7 && text.lines().all(|l| l.starts_with("(0.") && l.contains(" can0 ")), "{text}");

        let compared = c.decode(2, DecoderConfig::Can(CanConfig { log: Some(log.display().to_string()), ..base.clone() })).unwrap();
        assert_eq!(compared.rows[5], "CAN Explorer log");
        assert!(compared.summary.contains(&format!("log: {n} matched")) && !compared.summary.contains("only"), "{}", compared.summary);

        let missing = c.decode(3, DecoderConfig::Can(CanConfig { dbc: Some(dir.join("nope.dbc").display().to_string()), ..base }));
        assert!(missing.unwrap_err().contains("cannot read the DBC file"));
        assert!(c.export_candump(99, &log).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
