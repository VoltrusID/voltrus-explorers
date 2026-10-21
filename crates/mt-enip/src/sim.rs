//! Device simulator: a real EtherNet/IP server (CIP tag browse/read/write on
//! TCP 44818) whose tag values are driven by a tick engine, mirroring the
//! bacnet-explorer `SimDevice` pattern. Powers the separate drive-style
//! control-panel window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::server::start_enip_server;
use crate::packets::build_list_identity_response;
use crate::types::*;

// ── Public types ──

/// Definition of one simulated point (maps 1:1 to a CIP tag).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimPointDef {
    pub key: String,
    /// CIP tag name exposed to clients (e.g. "Program:Main.Motor1.Speed").
    pub name: String,
    /// "real" | "dint" | "bool" (lowercase, matches CipDataType serde).
    #[serde(default = "default_data_type")]
    pub data_type: String,
    #[serde(default = "default_element_count_sim")]
    pub element_count: u16,
    #[serde(default)]
    pub base: f64,
    #[serde(default)]
    pub variance: f64,
    #[serde(default)]
    pub noise: f64,
    /// Sin period in milliseconds.
    #[serde(default = "default_period_ms")]
    pub period_ms: u64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub decimals: Option<u32>,
    #[serde(default)]
    pub unit: Option<String>,
    /// Manual (setpoint/output) when true; auto-ticked sensor when false.
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub is_bool: bool,
    /// "wave" (default: base + sin*variance + noise) or "drive" (tracks
    /// a `mode` + `speedRef` point like a VFD: Off=0, Hand/Auto=speedRef).
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_data_type() -> String {
    "real".into()
}
fn default_element_count_sim() -> u16 {
    1
}
fn default_period_ms() -> u64 {
    10_000
}
fn default_source() -> String {
    "wave".into()
}

/// One point as seen by the control panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapPoint {
    pub key: String,
    pub name: String,
    pub data_type: String,
    pub unit: String,
    pub value: f64,
    pub display: String,
    pub writable: bool,
    pub is_bool: bool,
    pub decimals: u32,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Full snapshot emitted on every tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapshot {
    pub port: u16,
    pub running: bool,
    pub points: Vec<SimSnapPoint>,
}

type TickFn = Arc<dyn Fn(SimSnapshot) + Send + Sync>;
type WriteFn = Arc<dyn Fn(String, f64) + Send + Sync>;
type LogFn = Arc<dyn Fn(EnipServerLogEntry) + Send + Sync>;

/// Running simulator handle.
pub struct EnipSimDevice {
    state: Arc<crate::server::EnipServerState>,
    shutdown_tx: watch::Sender<bool>,
    running: Arc<AtomicBool>,
    port: u16,
}

// Internal runtime point
#[derive(Clone)]
struct Point {
    key: String,
    name: String,
    data_type: String,
    base: f64,
    variance: f64,
    noise: f64,
    period_ms: f64,
    min: Option<f64>,
    max: Option<f64>,
    decimals: u32,
    unit: String,
    writable: bool,
    is_bool: bool,
    source: String,
}

impl Point {
    fn from_def(d: &SimPointDef) -> Self {
        Self {
            key: d.key.clone(),
            name: d.name.clone(),
            data_type: d.data_type.clone(),
            base: d.base,
            variance: d.variance,
            noise: d.noise,
            period_ms: d.period_ms.max(1) as f64,
            min: d.min,
            max: d.max,
            decimals: d.decimals.unwrap_or(0),
            unit: d.unit.clone().unwrap_or_default(),
            writable: d.writable,
            is_bool: d.is_bool,
            source: if d.source.is_empty() { "wave".into() } else { d.source.clone() },
        }
    }

    fn to_cip_value(&self, v: f64) -> CipValue {
        if self.is_bool {
            CipValue::Bool(v >= 0.5)
        } else if self.data_type == "dint" {
            CipValue::Dint(v.round() as i32)
        } else {
            CipValue::Real(v as f32)
        }
    }
}

fn data_type_to_cip(s: &str) -> CipDataType {
    match s {
        "bool" => CipDataType::Bool,
        "sint" => CipDataType::Sint,
        "int" => CipDataType::Int,
        "dint" => CipDataType::Dint,
        "string" => CipDataType::String,
        "dword" => CipDataType::Dword,
        "lint" => CipDataType::Lint,
        _ => CipDataType::Real,
    }
}

// Tiny xorshift RNG (no external dep).
struct Rng(u64);
impl Rng {
    fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1;
        Rng(seed)
    }
    fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        // map to [0,1)
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl EnipSimDevice {
    /// Start the simulator: launches the ENIP TCP server + tick engine.
    pub async fn start<F1, F2, F3>(
        port: u16,
        bind_addr: String,
        defs: Vec<SimPointDef>,
        templates: Vec<CipTemplateDef>,
        udt_tags: Vec<CipTagDef>,
        on_tick: F1,
        on_write: F2,
        on_log: F3,
    ) -> anyhow::Result<Self>
    where
        F1: Fn(SimSnapshot) + Send + Sync + 'static,
        F2: Fn(String, f64) + Send + Sync + 'static,
        F3: Fn(EnipServerLogEntry) + Send + Sync + 'static,
    {
        let points: Vec<Point> = defs.iter().map(Point::from_def).collect();

        let tags: Vec<CipTagDef> = defs
            .iter()
            .map(|d| CipTagDef {
                name: d.name.clone(),
                data_type: data_type_to_cip(&d.data_type),
                element_count: d.element_count,
                initial_value: Point::from_def(d).to_cip_value(d.base),
                template_id: None,
            })
            .collect();

        let config = EnipServerConfig {
            port,
            bind_addr: bind_addr.clone(),
        };
        let mut all_tags = tags;
        all_tags.extend(udt_tags);
        let (state, mut log_rx, shutdown_tx) = start_enip_server(config, all_tags, templates).await?;

        let running = Arc::new(AtomicBool::new(true));
        let on_tick: TickFn = Arc::new(on_tick);
        let on_write: WriteFn = Arc::new(on_write);
        let on_log: LogFn = Arc::new(on_log);

        let tick_state = state.clone();
        let tick_running = running.clone();
        let tick_points = points.clone();
        let tick_on_tick = on_tick.clone();
        let tick_on_write = on_write.clone();
        let tick_on_log = on_log.clone();
        let tick_port = port;

        tokio::spawn(async move {
            let start = Instant::now();
            let mut rng = Rng::new();
            loop {
                if !tick_running.load(Ordering::Relaxed) {
                    break;
                }

                let elapsed_ms = start.elapsed().as_millis() as f64;

                // Auto-tick non-writable points.
                {
                    let mut tags = tick_state.tags.lock().await;

                    // Drive-source points track Mode + SpeedReference (VFD semantics).
                    let mode_name = tick_points.iter().find(|p| p.key == "mode").map(|p| p.name.clone());
                    let ref_name = tick_points.iter().find(|p| p.key == "speedRef").map(|p| p.name.clone());
                    let mode_val = mode_name.as_ref().and_then(|n| tags.get(n)).and_then(|e| e.value.to_f64()).unwrap_or(2.0);
                    let ref_val = ref_name.as_ref().and_then(|n| tags.get(n)).and_then(|e| e.value.to_f64()).unwrap_or(0.0);
                    for p in &tick_points {
                        if p.writable || p.source != "drive" {
                            continue;
                        }
                        let target = if mode_val <= 0.5 { 0.0 } else { ref_val };
                        let cur = tags.get(&p.name).and_then(|e| e.value.to_f64()).unwrap_or(0.0);
                        let mut v = cur + (target - cur) * 0.25;
                        if mode_val > 0.5 {
                            v += (rng.next() - 0.5) * 2.0 * p.noise;
                        }
                        if let Some(min) = p.min {
                            v = v.max(min);
                        }
                        if let Some(max) = p.max {
                            v = v.min(max);
                        }
                        if let Some(entry) = tags.get_mut(&p.name) {
                            entry.value = p.to_cip_value(v);
                        }
                    }

                    for p in &tick_points {
                        if p.writable || p.source == "drive" {
                            continue;
                        }
                        let phase = if p.period_ms > 0.0 {
                            (elapsed_ms / p.period_ms) * 2.0 * std::f64::consts::PI
                        } else {
                            0.0
                        };
                        let mut v = p.base + phase.sin() * p.variance + (rng.next() - 0.5) * 2.0 * p.noise;
                        if let Some(min) = p.min {
                            v = v.max(min);
                        }
                        if let Some(max) = p.max {
                            v = v.min(max);
                        }
                        if let Some(entry) = tags.get_mut(&p.name) {
                            entry.value = p.to_cip_value(v);
                        }
                    }
                }

                // Emit snapshot.
                let snap = build_snapshot(&tick_state, &tick_points, tick_port, true).await;
                tick_on_tick(snap);

                // Drain server log channel: forward all, detect external writes.
                while let Ok(entry) = log_rx.try_recv() {
                    tick_on_log(entry.clone());
                    if entry.service == "write" {
                        // value already applied by server; read it back for the key.
                        let key = tick_points.iter().find(|p| p.name == entry.tag_name).map(|p| p.key.clone());
                        if let Some(key) = key {
                            let tags = tick_state.tags.lock().await;
                            if let Some(e) = tags.get(&entry.tag_name) {
                                let v = e.value.to_f64().unwrap_or(0.0);
                                drop(tags);
                                tick_on_write(key, v);
                            }
                        }
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        });

        // UDP ListIdentity responder — lets device discovery find the sim on the
        // same port as the TCP listener (44818). The reply's source port equals
        // the bind port, so the scanner records the connectable TCP port.
        let udp_running = running.clone();
        let udp_bind = format!("{}:{}", bind_addr, port);
        tokio::spawn(async move {
            let sock = match tokio::net::UdpSocket::bind(udp_bind).await {
                Ok(s) => s,
                Err(_) => return, // port busy/unavailable — sim still works over TCP
            };
            let resp = build_list_identity_response();
            let mut buf = vec![0u8; 512];
            loop {
                if !udp_running.load(Ordering::Relaxed) {
                    break;
                }
                match tokio::time::timeout(
                    std::time::Duration::from_millis(250),
                    sock.recv_from(&mut buf),
                )
                .await
                {
                    Ok(Ok((n, peer))) => {
                        if n >= 2 && u16::from_le_bytes([buf[0], buf[1]]) == CMD_LIST_IDENTITY {
                            let _ = sock.send_to(&resp, peer).await;
                        }
                    }
                    _ => continue,
                }
            }
        });

        Ok(EnipSimDevice {
            state,
            shutdown_tx,
            running,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Shared server tag state (so host apps can reuse read/write helpers).
    pub fn server_state(&self) -> Arc<crate::server::EnipServerState> {
        self.state.clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Set a point's value by its CIP tag name.
    pub async fn set_by_name(&self, name: &str, value: CipValue) {
        let mut tags = self.state.tags.lock().await;
        if let Some(entry) = tags.get_mut(name) {
            entry.value = value;
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.shutdown_tx.send(true);
    }
}

async fn build_snapshot(
    state: &Arc<crate::server::EnipServerState>,
    points: &[Point],
    port: u16,
    running: bool,
) -> SimSnapshot {
    let tags = state.tags.lock().await;
    let mut out = Vec::with_capacity(points.len());
    for p in points {
        let raw = tags.get(&p.name).and_then(|e| e.value.to_f64()).unwrap_or(0.0);
        let display = if p.is_bool {
            if raw >= 0.5 { "On".into() } else { "Off".into() }
        } else {
            let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
            format!("{:.*}{}", p.decimals as usize, raw, unit)
        };
        out.push(SimSnapPoint {
            key: p.key.clone(),
            name: p.name.clone(),
            data_type: p.data_type.clone(),
            unit: p.unit.clone(),
            value: raw,
            display,
            writable: p.writable,
            is_bool: p.is_bool,
            decimals: p.decimals,
            min: p.min,
            max: p.max,
        });
    }
    SimSnapshot { port, running, points: out }
}
