//! Everything the Tauri commands do, without Tauri: the MTConnect client and agent simulator, the
//! FOCAS client and CNC simulator. Events (`cnc-log`, `mtc-stream`, `mtc-devices`) go through an
//! [`Emit`] sink so a test can drive the same functions the UI invokes.

use std::sync::Arc;
use std::time::Duration;

use focas_proto::{
    AlarmMessage, AlarmQuery, AlarmType, CncProfile, ExtraQuery, FocasClient, FocasConfig,
    FocasSimulator, MacroValue, NumberedValues, PmcArea, PmcDataType, PmcValue, ProgramEntry,
    SimAlarm, SimulatorConfig, SysInfo,
};
use mt_mtconnect::{
    AgentClient, AgentConfig, AgentSimulator, AssetList, Category, ConditionLevel, Devices,
    Observation, ResyncReason, SimConfig, SimSnapshot, StreamEvent, StreamTransport, Streams,
    Value,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;

use crate::capture::{self, CaptureBundle, MachineState};

/// Event sink: `(event name, JSON payload)`.
pub type Emit = Arc<dyn Fn(&str, Json) + Send + Sync>;

/// The value of a reading, or `None` with the refusal recorded in `errors`.
fn take<T: Clone>(r: &focas_proto::Reading<T>, what: &str, errors: &mut Vec<String>) -> Option<T> {
    match r {
        focas_proto::Reading::Value(v) => Some(v.clone()),
        focas_proto::Reading::Failed(f) => {
            errors.push(format!("{what}: {}", f.message));
            None
        }
    }
}

fn es(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Default)]
struct MtcSession {
    client: Option<Arc<AgentClient>>,
    base: String,
    stream: Option<JoinHandle<()>>,
}

struct MtcSim {
    sim: AgentSimulator,
    feed: JoinHandle<()>,
}

struct FocasSim {
    sim: Arc<FocasSimulator>,
    monitor: JoinHandle<()>,
}

pub struct Core {
    emit: Emit,
    mtc: Mutex<MtcSession>,
    fanuc: Mutex<Option<Arc<FocasClient>>>,
    mtc_sim: Mutex<Option<MtcSim>>,
    focas_sim: Mutex<Option<FocasSim>>,
}

// ---- result shapes ----------------------------------------------------------------------------

/// A status word with its FOCAS label (`MEM`) and a readable name (`Memory`).
#[derive(Debug, Clone, Serialize)]
pub struct Coded {
    pub label: String,
    pub name: String,
}

fn coded(label: &str, name: impl std::fmt::Debug) -> Coded {
    Coded {
        label: label.to_string(),
        name: format!("{name:?}"),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub mode: Coded,
    pub run: Coded,
    pub motion: Coded,
    pub mstb: Coded,
    pub emergency: Coded,
    pub alarm: Coded,
    pub edit: Coded,
}

#[derive(Debug, Clone, Serialize)]
pub struct AxisRow {
    pub name: String,
    pub absolute: Option<f64>,
    pub machine: Option<f64>,
    pub relative: Option<f64>,
    pub distance_to_go: Option<f64>,
    pub servo_load: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgramView {
    pub running: Option<i32>,
    pub main: Option<i32>,
    pub name: Option<String>,
    pub sequence: Option<i32>,
    pub block: Option<String>,
}

/// A watched macro variable; `value` is `None` when the controller flags it empty or refuses it.
#[derive(Debug, Clone, Serialize)]
pub struct MacroRow {
    pub number: u32,
    pub value: Option<f64>,
}

/// One poll of a FANUC CNC. An item the controller refuses is `None` and listed in `errors`.
#[derive(Debug, Clone, Serialize)]
pub struct FanucPoll {
    pub status: Option<StatusView>,
    pub axes: Vec<AxisRow>,
    pub feed_rate: Option<f64>,
    pub spindle_speed: Option<f64>,
    pub program: ProgramView,
    pub macros: Vec<MacroRow>,
    pub alarm_bits: Option<u32>,
    pub alarm_types: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtcSimInfo {
    pub base_url: String,
    pub http_port: u16,
    pub shdr_port: Option<u16>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocasSimConfig {
    pub profile: CncProfile,
    pub port: u16,
}

/// What the CNC simulator panel shows.
#[derive(Debug, Clone, Serialize)]
pub struct FocasSimView {
    pub profile: String,
    pub port: u16,
    pub status: StatusView,
    pub axes: Vec<(String, f64)>,
    pub feed_rate: f64,
    pub spindle_speed: f64,
    pub program: i32,
    pub block: String,
    pub sequence: i32,
    pub part_count: u64,
    pub alarms: Vec<SimAlarm>,
    pub macros: Vec<(u32, f64)>,
    pub sessions: u64,
    pub requests: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocasSimAction {
    CycleStart,
    FeedHold,
    Reset,
    EmergencyOn,
    EmergencyOff,
}

fn status_view(s: &focas_proto::StatInfo) -> StatusView {
    StatusView {
        mode: coded(s.aut.label(), s.aut),
        run: coded(s.run.label(), s.run),
        motion: coded(s.motion.label(), s.motion),
        mstb: coded(s.mstb.label(), s.mstb),
        emergency: coded(s.emergency.label(), s.emergency),
        alarm: coded(s.alarm.label(), s.alarm),
        edit: coded(s.edit.label(), s.edit),
    }
}

fn observation_line(o: &Observation) -> String {
    let name = o.name.as_deref().unwrap_or(&o.data_item_id);
    match &o.condition {
        Some(c) => format!(
            "{name} {} {} {}",
            c.level.element_name(),
            c.native_code.as_deref().unwrap_or(""),
            c.message.as_deref().unwrap_or("")
        )
        .trim_end()
        .to_string(),
        None => format!("{name} = {}", o.value.to_text()),
    }
}

impl Core {
    pub fn new(emit: Emit) -> Self {
        Core {
            emit,
            mtc: Mutex::new(MtcSession::default()),
            fanuc: Mutex::new(None),
            mtc_sim: Mutex::new(None),
            focas_sim: Mutex::new(None),
        }
    }

    fn emit(&self, event: &str, payload: Json) {
        (self.emit)(event, payload);
    }

    fn log(&self, direction: &str, message: impl Into<String>) {
        self.emit(
            "cnc-log",
            json!({ "direction": direction, "message": message.into() }),
        );
    }

    /// Logs a failure and returns its text for the command result.
    fn fail(&self, what: &str, e: impl std::fmt::Display) -> String {
        let text = e.to_string();
        self.log("error", format!("{what}: {text}"));
        text
    }

    // ---- MTConnect client -------------------------------------------------------------------

    async fn mtc_client(&self) -> Result<(Arc<AgentClient>, String), String> {
        let s = self.mtc.lock().await;
        let c = s.client.clone().ok_or("Not connected")?;
        Ok((c, s.base.clone()))
    }

    pub async fn mtc_connect(&self, config: AgentConfig) -> Result<Devices, String> {
        self.mtc_disconnect().await;
        let base = config.base_url.trim().trim_end_matches('/').to_string();
        let client =
            Arc::new(AgentClient::new(config).map_err(|e| self.fail("MTConnect connect", e))?);
        self.log("tx", format!("GET {base}/probe"));
        let devices = client.probe().await.map_err(|e| self.fail("probe", e))?;
        let items: usize = devices
            .devices
            .iter()
            .map(|d| d.flatten_data_items().len())
            .sum();
        self.log(
            "rx",
            format!(
                "probe: {} device(s), {items} data items, instance {}",
                devices.devices.len(),
                devices.header.instance_id
            ),
        );
        let mut s = self.mtc.lock().await;
        s.client = Some(client);
        s.base = base;
        Ok(devices)
    }

    pub async fn mtc_disconnect(&self) {
        let mut s = self.mtc.lock().await;
        if let Some(h) = s.stream.take() {
            h.abort();
        }
        if s.client.take().is_some() {
            drop(s);
            self.log("info", "MTConnect disconnected");
        }
    }

    pub async fn mtc_current(&self, path: Option<String>) -> Result<Streams, String> {
        let (client, base) = self.mtc_client().await?;
        self.log(
            "tx",
            format!(
                "GET {base}/current{}",
                path.as_deref()
                    .map(|p| format!("?path={p}"))
                    .unwrap_or_default()
            ),
        );
        let s = client
            .current(path.as_deref())
            .await
            .map_err(|e| self.fail("current", e))?;
        self.log(
            "rx",
            match s.header.next_sequence {
                Some(n) => format!("current: {} observations, next sequence {n}", s.len()),
                None => format!("current: {} observations", s.len()),
            },
        );
        Ok(s)
    }

    pub async fn mtc_assets(&self) -> Result<AssetList, String> {
        let (client, base) = self.mtc_client().await?;
        self.log("tx", format!("GET {base}/assets"));
        let a = client.assets().await.map_err(|e| self.fail("assets", e))?;
        self.log("rx", format!("assets: {}", a.assets.len()));
        Ok(a)
    }

    /// Starts the live stream: an initial `mtc-stream` snapshot, then batches of new observations.
    pub async fn mtc_stream_start(&self, interval_ms: u64) -> Result<(), String> {
        let (client, base) = self.mtc_client().await?;
        let interval = Duration::from_millis(interval_ms.clamp(100, 60_000));
        let mut s = self.mtc.lock().await;
        if let Some(h) = s.stream.take() {
            h.abort();
        }
        self.log(
            "tx",
            format!("GET {base}/sample?interval={}", interval.as_millis()),
        );
        let emit = self.emit.clone();
        s.stream = Some(tokio::spawn(stream_task(client, interval, emit)));
        Ok(())
    }

    pub async fn mtc_stream_stop(&self) {
        if let Some(h) = self.mtc.lock().await.stream.take() {
            h.abort();
            self.log("info", "Stream stopped");
        }
    }

    // ---- FOCAS client -----------------------------------------------------------------------

    async fn fanuc_client(&self) -> Result<Arc<FocasClient>, String> {
        self.fanuc
            .lock()
            .await
            .clone()
            .ok_or_else(|| "Not connected".to_string())
    }

    pub async fn fanuc_connect(&self, config: FocasConfig) -> Result<SysInfo, String> {
        self.fanuc_disconnect().await;
        self.log(
            "tx",
            format!("Open {}:{} (FOCAS/Ethernet)", config.host, config.port),
        );
        let client = FocasClient::connect(config)
            .await
            .map_err(|e| self.fail("FANUC connect", e))?;
        let info = client
            .sysinfo()
            .await
            .map_err(|e| self.fail("sysinfo", e))?;
        self.log(
            "rx",
            format!(
                "sysinfo: {} {} {} v{}, {} axes",
                info.cnc_type.trim(),
                info.series.trim(),
                info.mt_type.trim(),
                info.version.trim(),
                info.axes.trim()
            ),
        );
        *self.fanuc.lock().await = Some(Arc::new(client));
        Ok(info)
    }

    pub async fn fanuc_disconnect(&self) {
        if let Some(c) = self.fanuc.lock().await.take() {
            let _ = c.disconnect().await;
            self.log("info", "FANUC disconnected");
        }
    }

    /// One poll: the batched snapshot, then servo load, program name and the `macros` the UI
    /// watches in one extended read, and the executing block.
    pub async fn fanuc_poll(&self, macros: Vec<u32>) -> Result<FanucPoll, String> {
        let c = self.fanuc_client().await?;
        let macros: Vec<u32> = macros
            .into_iter()
            .filter(|n| *n > 0)
            .take(capture::MAX_WATCHED_MACROS)
            .collect();
        let snap = c.snapshot().await.map_err(|e| self.fail("poll", e))?;
        let query = ExtraQuery {
            servo_load: true,
            program_name: true,
            macros: macros.clone(),
        };
        let (extra, block) = tokio::join!(c.extended_snapshot(&query), c.executing_block(80));
        let mut errors = Vec::new();

        let status = take(&snap.status, "status", &mut errors).map(|s| status_view(&s));
        let names = take(&snap.positions.axes, "axis names", &mut errors).unwrap_or_default();
        let abs =
            take(&snap.positions.absolute, "absolute position", &mut errors).unwrap_or_default();
        let mach =
            take(&snap.positions.machine, "machine position", &mut errors).unwrap_or_default();
        let rel =
            take(&snap.positions.relative, "relative position", &mut errors).unwrap_or_default();
        let dtg = take(
            &snap.positions.distance_to_go,
            "distance to go",
            &mut errors,
        )
        .unwrap_or_default();
        // A failed request (not a refused item) loses the extras but not the snapshot.
        let extra = extra
            .map_err(|e| errors.push(format!("extended read: {e}")))
            .ok();
        let servo = extra
            .as_ref()
            .and_then(|x| x.servo_load.as_ref())
            .and_then(|r| take(r, "servo load", &mut errors))
            .unwrap_or_default();
        let name = extra
            .as_ref()
            .and_then(|x| x.program_name.as_ref())
            .and_then(|r| r.value().cloned());
        let macro_rows: Vec<MacroRow> = extra
            .iter()
            .flat_map(|x| &x.macros)
            .zip(&macros)
            .map(|(r, n)| MacroRow {
                number: *n,
                value: take(r, &format!("macro #{n}"), &mut errors).and_then(|m| m.value),
            })
            .collect();
        let at = |v: &[Option<f64>], i: usize| v.get(i).copied().flatten();
        let axes = names
            .iter()
            .enumerate()
            .map(|(i, n)| AxisRow {
                name: n.to_string(),
                absolute: at(&abs, i),
                machine: at(&mach, i),
                relative: at(&rel, i),
                distance_to_go: at(&dtg, i),
                servo_load: at(&servo, i),
            })
            .collect();
        let feed_rate = take(&snap.feed_rate, "feed", &mut errors).flatten();
        let spindle_speed = take(&snap.spindle_speed, "spindle", &mut errors).flatten();
        let programs = take(&snap.programs, "program numbers", &mut errors);
        let sequence = take(&snap.sequence_number, "sequence number", &mut errors);
        let bits = take(&snap.alarm_bits, "alarm bits", &mut errors);
        // The program name call is 30i-and-later only; a refusal is normal on older controls, so
        // it is not listed in `errors`.
        Ok(FanucPoll {
            status,
            axes,
            feed_rate,
            spindle_speed,
            program: ProgramView {
                running: programs.map(|p| p.running),
                main: programs.map(|p| p.main),
                name,
                sequence,
                block: block.ok().map(|b| b.text),
            },
            macros: macro_rows,
            alarm_bits: bits.map(|b| b.0),
            alarm_types: bits
                .map(|b| b.types().iter().map(|t| format!("{t:?}")).collect())
                .unwrap_or_default(),
            errors,
        })
    }

    /// Records a read-only probe of the connected CNC on a session of its own (see
    /// [`crate::capture`]) and returns the files to save. The live session keeps polling; the
    /// controller sees a second, short connection.
    pub async fn fanuc_capture(
        &self,
        state: MachineState,
        macros: Vec<u32>,
        app_version: &str,
    ) -> Result<CaptureBundle, String> {
        let config = self.fanuc_client().await?.config().clone();
        self.log(
            "tx",
            format!(
                "Capture session to {}:{} (state: {})",
                config.host,
                config.port,
                state.label()
            ),
        );
        let bundle = capture::record(config, state, &macros, app_version)
            .await
            .map_err(|e| self.fail("capture", e))?;
        self.log(
            "rx",
            format!(
                "capture: {} frame exchange(s), {} call(s), {} failed, replay {}",
                bundle.exchanges,
                bundle.calls,
                bundle.failed_calls,
                if bundle.replay_ok { "ok" } else { "differs" }
            ),
        );
        Ok(bundle)
    }

    pub async fn fanuc_alarms(&self) -> Result<Vec<AlarmMessage>, String> {
        let c = self.fanuc_client().await?;
        self.log("tx", "Read alarm messages");
        let a = c
            .alarm_messages(AlarmQuery::default())
            .await
            .map_err(|e| self.fail("alarms", e))?;
        self.log("rx", format!("{} active alarm message(s)", a.len()));
        Ok(a)
    }

    pub async fn fanuc_programs(&self) -> Result<Vec<ProgramEntry>, String> {
        let c = self.fanuc_client().await?;
        self.log("tx", "List programs");
        let p = c
            .list_programs()
            .await
            .map_err(|e| self.fail("programs", e))?;
        self.log("rx", format!("{} program(s)", p.len()));
        Ok(p)
    }

    pub async fn fanuc_read_macro(&self, first: u32, last: u32) -> Result<Vec<MacroValue>, String> {
        if last < first || last - first >= 200 {
            return Err("Read 1 to 200 macro variables at a time".into());
        }
        let c = self.fanuc_client().await?;
        self.log("tx", format!("Read macro #{first}..#{last}"));
        let v = c
            .read_macro(first, last)
            .await
            .map_err(|e| self.fail("read macro", e))?;
        self.log("rx", format!("{} macro value(s)", v.len()));
        Ok(v)
    }

    pub async fn fanuc_write_macro(&self, number: u32, value: f64) -> Result<(), String> {
        let c = self.fanuc_client().await?;
        self.log("tx", format!("Write macro #{number} = {value}"));
        c.write_macro(number, value)
            .await
            .map_err(|e| self.fail("write macro", e))?;
        self.log("rx", format!("macro #{number} written"));
        Ok(())
    }

    pub async fn fanuc_read_params(
        &self,
        first: u32,
        last: u32,
        axis: i32,
    ) -> Result<Vec<NumberedValues>, String> {
        let c = self.fanuc_client().await?;
        self.log("tx", format!("Read parameters {first}..{last} axis {axis}"));
        let v = c
            .read_params(first, last, axis)
            .await
            .map_err(|e| self.fail("read parameters", e))?;
        self.log("rx", format!("{} parameter(s)", v.len()));
        Ok(v)
    }

    pub async fn fanuc_read_diag(
        &self,
        first: u32,
        last: u32,
        axis: i32,
    ) -> Result<Vec<NumberedValues>, String> {
        let c = self.fanuc_client().await?;
        self.log("tx", format!("Read diagnosis {first}..{last} axis {axis}"));
        let v = c
            .read_diag(first, last, axis)
            .await
            .map_err(|e| self.fail("read diagnosis", e))?;
        self.log("rx", format!("{} diagnosis value(s)", v.len()));
        Ok(v)
    }

    pub async fn fanuc_read_pmc(
        &self,
        area: PmcArea,
        data_type: PmcDataType,
        first: u32,
        count: u32,
    ) -> Result<Vec<PmcValue>, String> {
        let c = self.fanuc_client().await?;
        self.log(
            "tx",
            format!("Read PMC {}{first} x{count} ({data_type:?})", area.letter()),
        );
        let v = c
            .read_pmc(area, data_type, first, count)
            .await
            .map_err(|e| self.fail("read PMC", e))?;
        self.log("rx", format!("{} PMC value(s)", v.len()));
        Ok(v)
    }

    // ---- MTConnect agent simulator ----------------------------------------------------------

    pub async fn mtc_sim_start(&self, mut config: SimConfig) -> Result<MtcSimInfo, String> {
        self.mtc_sim_stop().await;
        config.bind_addr = "0.0.0.0".into();
        let sim = AgentSimulator::start(config)
            .await
            .map_err(|e| self.fail("Agent simulator start", e))?;
        let info = MtcSimInfo {
            base_url: format!("http://localhost:{}", sim.http_addr().port()),
            http_port: sim.http_addr().port(),
            shdr_port: sim.shdr_addr().map(|a| a.port()),
        };
        let rx = sim.model().subscribe();
        let feed = tokio::spawn(sim_feed(rx, self.emit.clone()));
        self.log(
            "info",
            format!(
                "MTConnect agent simulator on port {}{}",
                info.http_port,
                info.shdr_port
                    .map(|p| format!(", SHDR adapter on port {p}"))
                    .unwrap_or_default()
            ),
        );
        *self.mtc_sim.lock().await = Some(MtcSim { sim, feed });
        Ok(info)
    }

    pub async fn mtc_sim_stop(&self) {
        if let Some(s) = self.mtc_sim.lock().await.take() {
            s.feed.abort();
            s.sim.stop().await;
            self.log("info", "MTConnect agent simulator stopped");
        }
    }

    async fn with_mtc_sim<R>(
        &self,
        f: impl FnOnce(&mt_mtconnect::SimModel) -> R,
    ) -> Result<R, String> {
        let g = self.mtc_sim.lock().await;
        let s = g.as_ref().ok_or("Agent simulator not running")?;
        Ok(f(s.sim.model()))
    }

    pub async fn mtc_sim_snapshot(&self) -> Result<SimSnapshot, String> {
        self.with_mtc_sim(|m| m.snapshot()).await
    }

    /// Pins a SAMPLE (number) or EVENT (text) data item. `UNAVAILABLE` works for both.
    pub async fn mtc_sim_set_value(&self, id: String, text: String) -> Result<(), String> {
        let text = text.trim().to_string();
        self.with_mtc_sim(|m| {
            let category = m
                .snapshot()
                .items
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.category)
                .ok_or_else(|| format!("unknown data item '{id}'"))?;
            let value = if text.eq_ignore_ascii_case("UNAVAILABLE") {
                Value::Unavailable
            } else if category == Category::Sample {
                Value::Number(
                    text.parse()
                        .map_err(|_| format!("'{text}' is not a number"))?,
                )
            } else {
                Value::Text(text.clone())
            };
            m.set_value(&id, value).map_err(es)
        })
        .await??;
        self.log("info", format!("Simulator: {id} set to {text}"));
        Ok(())
    }

    pub async fn mtc_sim_release(&self, id: String) -> Result<(), String> {
        self.with_mtc_sim(|m| m.release(&id)).await
    }

    pub async fn mtc_sim_raise(
        &self,
        id: String,
        level: ConditionLevel,
        code: String,
        message: String,
    ) -> Result<(), String> {
        self.with_mtc_sim(|m| m.raise_condition(&id, level, &code, &message).map_err(es))
            .await??;
        Ok(())
    }

    pub async fn mtc_sim_clear(&self, id: String) -> Result<(), String> {
        self.with_mtc_sim(|m| m.clear_condition(&id).map_err(es))
            .await??;
        Ok(())
    }

    pub async fn mtc_sim_set_paused(&self, paused: bool) -> Result<(), String> {
        self.with_mtc_sim(|m| m.set_paused(paused)).await?;
        self.log(
            "info",
            if paused {
                "Simulator program paused"
            } else {
                "Simulator program resumed"
            },
        );
        Ok(())
    }

    pub async fn mtc_sim_restart_agent(&self) -> Result<(), String> {
        self.with_mtc_sim(|m| m.restart_agent()).await?;
        self.log("info", "Simulated agent restarted (new instance id)");
        Ok(())
    }

    // ---- FANUC CNC simulator ----------------------------------------------------------------

    pub async fn focas_sim_start(&self, config: FocasSimConfig) -> Result<u16, String> {
        self.focas_sim_stop().await;
        let sim = FocasSimulator::start(SimulatorConfig {
            bind_addr: "0.0.0.0".into(),
            port: config.port,
            profile: config.profile,
            tick: Some(Duration::from_millis(100)),
        })
        .await
        .map_err(|e| {
            self.fail(
                "CNC simulator start",
                format!("cannot listen on port {}: {e}", config.port),
            )
        })?;
        let sim = Arc::new(sim);
        let port = sim.port();
        let monitor = tokio::spawn(focas_monitor(sim.clone(), self.emit.clone()));
        self.log(
            "info",
            format!("FANUC CNC simulator ({:?}) on port {port}", config.profile),
        );
        *self.focas_sim.lock().await = Some(FocasSim { sim, monitor });
        Ok(port)
    }

    pub async fn focas_sim_stop(&self) {
        if let Some(s) = self.focas_sim.lock().await.take() {
            s.monitor.abort();
            s.sim.stop().await;
            self.log("info", "FANUC CNC simulator stopped");
        }
    }

    async fn focas_sim(&self) -> Result<Arc<FocasSimulator>, String> {
        self.focas_sim
            .lock()
            .await
            .as_ref()
            .map(|s| s.sim.clone())
            .ok_or_else(|| "CNC simulator not running".into())
    }

    pub async fn focas_sim_view(&self) -> Result<FocasSimView, String> {
        let sim = self.focas_sim().await?;
        let m = sim.model();
        Ok(FocasSimView {
            profile: format!("{} {}", m.sysinfo.cnc_type.trim(), m.sysinfo.series.trim())
                .trim()
                .to_string(),
            port: sim.port(),
            status: status_view(&m.status()),
            axes: m
                .axes
                .iter()
                .map(|a| (a.name.to_string(), a.absolute))
                .collect(),
            feed_rate: m.feed_rate,
            spindle_speed: m.spindle_speed,
            program: m.program_numbers().running,
            block: m.current_block_text(),
            sequence: m.sequence_number(),
            part_count: m.part_count(),
            alarms: m.alarms.clone(),
            macros: m.macros.iter().take(100).map(|(k, v)| (*k, *v)).collect(),
            sessions: sim.active_sessions(),
            requests: sim.requests_served(),
        })
    }

    pub async fn focas_sim_action(&self, action: FocasSimAction) -> Result<(), String> {
        let sim = self.focas_sim().await?;
        let ok = sim.with_model(|m| match action {
            FocasSimAction::CycleStart => m.cycle_start(),
            FocasSimAction::FeedHold => {
                m.feed_hold();
                true
            }
            FocasSimAction::Reset => {
                m.reset();
                true
            }
            FocasSimAction::EmergencyOn => {
                m.set_emergency(true);
                true
            }
            FocasSimAction::EmergencyOff => {
                m.set_emergency(false);
                true
            }
        });
        if !ok {
            return Err(
                "Cycle start refused: emergency stop, active alarm, or no program in MEM/MDI mode"
                    .into(),
            );
        }
        self.log("info", format!("CNC simulator: {action:?}"));
        Ok(())
    }

    pub async fn focas_sim_raise_alarm(
        &self,
        number: i32,
        alarm_type: AlarmType,
        message: String,
    ) -> Result<(), String> {
        let sim = self.focas_sim().await?;
        sim.with_model(|m| m.raise_alarm(number, alarm_type, 0, message.clone()));
        self.log(
            "info",
            format!("CNC simulator: alarm {number} raised ({message})"),
        );
        Ok(())
    }

    /// Clears one alarm, or all of them when `number` is `None`.
    pub async fn focas_sim_clear_alarm(&self, number: Option<i32>) -> Result<(), String> {
        let sim = self.focas_sim().await?;
        sim.with_model(|m| match number {
            Some(n) => {
                m.clear_alarm(n);
            }
            None => m.clear_alarms(),
        });
        self.log("info", "CNC simulator: alarm cleared");
        Ok(())
    }

    pub async fn focas_sim_set_macro(&self, number: u32, value: f64) -> Result<(), String> {
        let sim = self.focas_sim().await?;
        sim.with_model(|m| m.set_macro(number, value));
        self.log("info", format!("CNC simulator: macro #{number} = {value}"));
        Ok(())
    }

    /// Stops everything (window close).
    pub async fn shutdown(&self) {
        self.mtc_disconnect().await;
        self.fanuc_disconnect().await;
        self.mtc_sim_stop().await;
        self.focas_sim_stop().await;
    }
}

// ---- background tasks ---------------------------------------------------------------------------

async fn stream_task(client: Arc<AgentClient>, interval: Duration, emit: Emit) {
    let log = |direction: &str, message: String| {
        emit(
            "cnc-log",
            json!({ "direction": direction, "message": message }),
        )
    };
    let mut stream = client.stream_sample(interval);
    let mut transport = StreamTransport::Connecting;
    while let Some(item) = stream.next().await {
        let ev = match item {
            Ok(ev) => ev,
            Err(e) => {
                log("error", format!("stream: {e}"));
                emit(
                    "mtc-stream",
                    json!({ "event": "error", "message": e.to_string() }),
                );
                return;
            }
        };
        if stream.transport() != transport {
            transport = stream.transport();
            log("info", format!("stream transport: {transport:?}"));
        }
        let (kind, streams) = match ev {
            StreamEvent::Snapshot(s) => ("snapshot", s),
            StreamEvent::Observations(s) => ("observations", s),
            StreamEvent::Resync { reason, snapshot } => {
                log("info", format!("stream resynchronised: {reason:?}"));
                if matches!(reason, ResyncReason::InstanceChanged { .. }) {
                    // Sequence numbers restarted and the device model may have changed.
                    match client.probe().await {
                        Ok(d) => emit("mtc-devices", serde_json::to_value(d).unwrap_or(Json::Null)),
                        Err(e) => log("error", format!("re-probe: {e}")),
                    }
                }
                ("resync", snapshot)
            }
        };
        let observations: Vec<&Observation> = streams.observations_by_sequence();
        if kind != "observations" || !observations.is_empty() {
            log(
                "rx",
                match streams.header.next_sequence {
                    Some(n) => format!(
                        "{kind}: {} observation(s), next sequence {n}",
                        observations.len()
                    ),
                    None => format!("{kind}: {} observation(s)", observations.len()),
                },
            );
        }
        emit(
            "mtc-stream",
            json!({ "event": kind, "observations": observations, "nextSequence": streams.header.next_sequence }),
        );
    }
}

/// Forwards the simulated agent's EVENT and CONDITION observations (not the position stream) to the log.
async fn sim_feed(mut rx: broadcast::Receiver<Observation>, emit: Emit) {
    loop {
        match rx.recv().await {
            Ok(o) if o.category != Category::Sample => {
                emit(
                    "cnc-log",
                    json!({ "direction": "rx", "message": format!("[agent] {}", observation_line(&o)) }),
                );
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Logs CNC simulator client connections and disconnections when the session count changes.
async fn focas_monitor(sim: Arc<FocasSimulator>, emit: Emit) {
    let mut sessions = 0;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tick.tick().await;
        let s = sim.active_sessions();
        if s != sessions {
            emit(
                "cnc-log",
                json!({ "direction": "info", "message": format!("[cnc] {s} client(s) connected") }),
            );
            sessions = s;
        }
    }
}
