//! Everything the Tauri commands do, without Tauri: the GEM host session and the equipment
//! simulator. Events (`gem-msg`, `gem-event`, `gem-conn`) go through an [`Emit`] sink so a test can
//! drive the same functions the UI invokes.

use std::sync::Arc;

use mt_secsgem::sim::model::like;
use mt_secsgem::{
    Ackc5, Ackc10, CommandParam, ControlState, Direction, Drack, Eac, EquipmentSimulator, Erack,
    EventLink, GemHost, HostEvent, HsmsConfig, HsmsConnection, HsmsListener, Hcack,
    IdFormat, Item, LogEntry, Lrack, OnlAck, ProcessState, ReportDef, SecsMessage, SimMode,
    SimulatorConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;

/// Event sink: `(event name, JSON payload)`.
pub type Emit = Arc<dyn Fn(&str, Json) + Send + Sync>;

fn es(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---- value rendering --------------------------------------------------------------------------

/// A short display text: ASCII as is, one number or boolean as a plain value, anything else as
/// one-line SML.
pub fn item_text(item: &Item) -> String {
    match item {
        Item::Ascii(s) => s.clone(),
        Item::Boolean(v) if v.len() == 1 => v[0].to_string(),
        Item::F4(v) if v.len() == 1 => v[0].to_string(),
        Item::F8(v) if v.len() == 1 => v[0].to_string(),
        i if i.is_numeric() && i.len() == 1 => {
            i.as_i64()
                .map(|n| n.to_string())
                .or_else(|| i.as_u64().map(|n| n.to_string()))
                .unwrap_or_else(|| i.to_sml())
        }
        other => other.to_sml(),
    }
}

fn item_number(item: &Item) -> Option<f64> {
    if item.is_numeric() || matches!(item, Item::F4(_) | Item::F8(_)) {
        if item.len() == 1 { item.as_f64() } else { None }
    } else {
        None
    }
}

/// The value of a status variable or equipment constant.
#[derive(Debug, Clone, Serialize)]
pub struct ValueView {
    pub text: String,
    pub number: Option<f64>,
    /// SECS-II item type (`U4`, `A`, `L`...).
    pub kind: String,
}

fn value_view(item: &Item) -> ValueView {
    ValueView {
        text: item_text(item),
        number: item_number(item),
        kind: item.type_name().to_string(),
    }
}

/// An acknowledge code with its name.
#[derive(Debug, Clone, Serialize)]
pub struct AckView {
    pub code: u8,
    pub name: String,
    pub ok: bool,
}

macro_rules! ack {
    ($a:expr) => {{
        let a = $a;
        AckView {
            code: a.code(),
            name: format!("{a:?}"),
            ok: a.is_ok(),
        }
    }};
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlView {
    pub code: u8,
    pub name: &'static str,
    pub online: bool,
}

fn control_view(code: u8) -> Option<ControlView> {
    ControlState::from_code(code).map(|c| ControlView {
        code,
        name: control_name(c),
        online: c.is_online(),
    })
}

fn control_name(c: ControlState) -> &'static str {
    match c {
        ControlState::EquipmentOffline => "Equipment offline",
        ControlState::AttemptOnline => "Attempt online",
        ControlState::HostOffline => "Host offline",
        ControlState::OnlineLocal => "Online local",
        ControlState::OnlineRemote => "Online remote",
    }
}

// ---- host: request and result shapes ------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConnect {
    /// `active` (connect to the equipment) or `passive` (wait for it).
    pub mode: String,
    /// Equipment address (active) or the address to listen on (passive).
    pub host: String,
    pub port: u16,
    pub device_id: u16,
    pub t3_ms: u64,
    pub t5_ms: u64,
    pub t6_ms: u64,
    pub t7_ms: u64,
    pub t8_ms: u64,
    /// SVID that carries the GEM control state on this equipment; 0 for none.
    pub control_svid: u64,
}

impl HostConnect {
    fn hsms(&self) -> HsmsConfig {
        HsmsConfig {
            device_id: self.device_id,
            t3_ms: self.t3_ms,
            t5_ms: self.t5_ms,
            t6_ms: self.t6_ms,
            t7_ms: self.t7_ms,
            t8_ms: self.t8_ms,
            linktest_interval_ms: Some(30_000),
            ..HsmsConfig::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnInfo {
    /// `listening` (passive, waiting) or `selected`.
    pub state: &'static str,
    pub peer: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EstablishView {
    pub ack: AckView,
    pub mdln: String,
    pub softrev: String,
    pub control: Option<ControlView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnlineView {
    pub ack: AckView,
    pub control: Option<ControlView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SvRow {
    pub id: u64,
    pub name: String,
    pub units: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SvValue {
    pub id: u64,
    #[serde(flatten)]
    pub value: ValueView,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcRow {
    pub id: u64,
    pub name: String,
    pub units: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub default: String,
    #[serde(flatten)]
    pub value: ValueView,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcWriteView {
    pub ack: AckView,
    /// What the equipment holds after the write.
    pub value: Option<ValueView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WizardRequest {
    pub rptid: u64,
    pub vids: Vec<u64>,
    pub ceids: Vec<u64>,
    pub enable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WizardStep {
    /// `S2F33 define report`, `S2F35 link events`, `S2F37 enable events`.
    pub step: &'static str,
    pub ack: AckView,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlarmView {
    pub alid: u64,
    pub set: bool,
    pub category: u8,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParamIn {
    pub name: String,
    /// `ascii`, `u1`, `u2`, `u4`, `i1`, `i2`, `i4`, `f4`, `f8`, `bool`, `binary` (hex bytes) or `sml`.
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandView {
    pub hcack: AckView,
    pub params: Vec<ParamAck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamAck {
    pub name: String,
    pub cpack: AckView,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawReply {
    /// `None` when the message had no W-bit, so no reply was awaited.
    pub name: Option<String>,
    pub sml: Option<String>,
}

/// One message of the message log, as sent to the UI.
fn log_payload(source: &str, e: &LogEntry) -> Json {
    // Data messages carry their one-line SML; render it indented for the detail view.
    let pretty = SecsMessage::from_sml(&e.text)
        .map(|m| m.to_sml_pretty())
        .unwrap_or_else(|_| e.text.clone());
    json!({
        "source": source,
        "timeMs": e.time_ms,
        "dir": if e.direction == Direction::Tx { "tx" } else { "rx" },
        "kind": e.kind,
        "system": e.system,
        "text": e.text,
        "pretty": pretty,
        "data": SecsMessage::from_sml(&e.text).is_ok(),
    })
}

fn forward_log(source: &'static str, mut rx: broadcast::Receiver<LogEntry>, emit: Emit) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(e) => emit("gem-msg", log_payload(source, &e)),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

fn event_payload(e: &HostEvent) -> Json {
    match e {
        HostEvent::EventReport(r) => json!({
            "kind": "event",
            "dataid": r.dataid,
            "ceid": r.ceid,
            "reports": r.reports.iter().map(|rv| json!({
                "rptid": rv.rptid,
                "values": rv.values.iter().map(value_view).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
        HostEvent::Alarm(a) => json!({
            "kind": "alarm",
            "alid": a.alid,
            "set": a.is_set(),
            "category": a.category(),
            "text": a.altx,
        }),
        HostEvent::Terminal(t) => json!({ "kind": "terminal", "tid": t.tid, "text": t.text }),
        HostEvent::S9(e) => json!({ "kind": "s9", "text": e.to_string() }),
        HostEvent::Unhandled(m) => json!({ "kind": "unhandled", "text": m.to_sml() }),
        HostEvent::Closed { reason } => json!({ "kind": "closed", "reason": reason }),
    }
}

#[derive(Default)]
struct HostSlot {
    host: Option<Arc<GemHost>>,
    control_svid: u64,
    tasks: Vec<JoinHandle<()>>,
}

impl HostSlot {
    fn clear(&mut self) {
        if let Some(h) = self.host.take() {
            h.disconnect();
        }
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

struct SimSlot {
    sim: Arc<EquipmentSimulator>,
    tasks: Vec<JoinHandle<()>>,
}

pub struct Core {
    emit: Emit,
    host: Arc<Mutex<HostSlot>>,
    sim: Mutex<Option<SimSlot>>,
}

/// Parses `text` into an item of the same type as `template` (an equipment constant keeps its
/// type when it is written).
pub fn parse_like(template: &Item, text: &str) -> Result<Item, String> {
    let t = text.trim();
    match template {
        Item::Ascii(_) => Ok(Item::ascii(text)),
        Item::Boolean(_) => match t.to_ascii_lowercase().as_str() {
            "true" | "1" | "t" | "yes" => Ok(Item::boolean(true)),
            "false" | "0" | "f" | "no" => Ok(Item::boolean(false)),
            _ => Err(format!("\"{t}\" is not a boolean (use true or false)")),
        },
        i if i.is_numeric() || matches!(i, Item::F4(_) | Item::F8(_)) => {
            let n: f64 = t.parse().map_err(|_| format!("\"{t}\" is not a number"))?;
            like(i, n).ok_or_else(|| format!("{t} does not fit the constant's type {}", i.type_name()))
        }
        other => Err(format!("a {} constant cannot be edited here; use the SML composer", other.type_name())),
    }
}

/// Builds a remote command parameter value from its kind and text.
pub fn param_item(kind: &str, text: &str) -> Result<Item, String> {
    let t = text.trim();
    macro_rules! num {
        ($ctor:ident, $ty:ty) => {
            t.parse::<$ty>()
                .map(Item::$ctor)
                .map_err(|_| format!("\"{t}\" is not a valid {kind}"))
        };
    }
    match kind {
        "ascii" => Ok(Item::ascii(text)),
        "u1" => num!(u1, u8),
        "u2" => num!(u2, u16),
        "u4" => num!(u4, u32),
        "i1" => num!(i1, i8),
        "i2" => num!(i2, i16),
        "i4" => num!(i4, i32),
        "f4" => num!(f4, f32),
        "f8" => num!(f8, f64),
        "bool" => match t.to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(Item::boolean(true)),
            "false" | "0" => Ok(Item::boolean(false)),
            _ => Err(format!("\"{t}\" is not a boolean")),
        },
        "binary" => {
            let bytes: Result<Vec<u8>, _> = t
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .map(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16))
                .collect();
            bytes.map(Item::binary).map_err(|_| format!("\"{t}\" is not hex bytes"))
        }
        "sml" => Item::from_sml(t).map_err(es),
        other => Err(format!("unknown parameter type {other}")),
    }
}

impl Core {
    pub fn new(emit: Emit) -> Self {
        Core {
            emit,
            host: Arc::new(Mutex::new(HostSlot::default())),
            sim: Mutex::new(None),
        }
    }

    fn log(&self, direction: &str, message: impl Into<String>) {
        (self.emit)("gem-log", json!({ "direction": direction, "message": message.into() }));
    }

    /// Logs a failure and returns its text for the command result.
    fn fail(&self, what: &str, e: impl std::fmt::Display) -> String {
        let text = e.to_string();
        self.log("error", format!("{what}: {text}"));
        text
    }

    async fn host(&self) -> Result<Arc<GemHost>, String> {
        self.host.lock().await.host.clone().ok_or_else(|| "Not connected".to_string())
    }

    // ---- host connection --------------------------------------------------------------------

    pub async fn connect(&self, cfg: HostConnect) -> Result<ConnInfo, String> {
        self.disconnect().await;
        let (log_tx, log_rx) = broadcast::channel(1024);
        let mut slot = self.host.lock().await;
        slot.control_svid = cfg.control_svid;
        slot.tasks.push(forward_log("host", log_rx, self.emit.clone()));
        let hsms = cfg.hsms();
        if cfg.mode == "passive" {
            let addr = format!("{}:{}", cfg.host.trim(), cfg.port);
            let listener = HsmsListener::bind(addr.as_str(), hsms)
                .await
                .map_err(|e| self.fail("listen", e))?
                .with_log(log_tx);
            self.log("info", format!("Listening on {addr}; waiting for the equipment to connect"));
            let (emit, state) = (self.emit.clone(), self.host.clone());
            slot.tasks.push(tokio::spawn(async move {
                match GemHost::accept(&listener, IdFormat::U4).await {
                    Ok(host) => {
                        let peer = host
                            .connection()
                            .peer_addr()
                            .map(|a| a.to_string())
                            .unwrap_or_default();
                        let host = Arc::new(host);
                        let mut s = state.lock().await;
                        s.host = Some(host.clone());
                        s.tasks.push(forward_events(host, state.clone(), emit.clone()));
                        emit("gem-conn", json!({ "state": "selected", "peer": peer }));
                    }
                    Err(e) => emit("gem-conn", json!({ "state": "closed", "reason": e.to_string() })),
                }
            }));
            return Ok(ConnInfo { state: "listening", peer: addr });
        }
        let addr = format!("{}:{}", cfg.host.trim(), cfg.port);
        self.log("info", format!("Connecting to {addr} (HSMS active)"));
        let (conn, rx) = match HsmsConnection::connect_with_log(addr.as_str(), hsms, Some(log_tx)).await {
            Ok(c) => c,
            Err(e) => {
                slot.clear();
                drop(slot);
                return Err(self.fail("connect", e));
            }
        };
        let host = Arc::new(GemHost::from_connection(conn, rx, IdFormat::U4));
        slot.host = Some(host.clone());
        slot.tasks.push(forward_events(host, self.host.clone(), self.emit.clone()));
        Ok(ConnInfo { state: "selected", peer: addr })
    }

    pub async fn disconnect(&self) {
        self.host.lock().await.clear();
    }

    async fn control_state(&self, host: &GemHost) -> Option<ControlView> {
        let svid = self.host.lock().await.control_svid;
        if svid == 0 {
            return None;
        }
        let v = host.status_variables(&[svid]).await.ok()?;
        control_view(u8::try_from(v.first()?.as_u64()?).ok()?)
    }

    pub async fn control(&self) -> Result<Option<ControlView>, String> {
        let h = self.host().await?;
        Ok(self.control_state(&h).await)
    }

    pub async fn are_you_there(&self) -> Result<String, String> {
        let h = self.host().await?;
        let id = h.are_you_there().await.map_err(|e| self.fail("S1F1", e))?;
        Ok(id.map(|i| format!("{} {}", i.mdln, i.softrev)).unwrap_or_default())
    }

    pub async fn establish(&self) -> Result<EstablishView, String> {
        let h = self.host().await?;
        let (ack, id) = h.establish_communications().await.map_err(|e| self.fail("S1F13", e))?;
        let (mdln, softrev) = id.map(|i| (i.mdln, i.softrev)).unwrap_or_default();
        self.log("info", format!("Communications established with {mdln} {softrev}"));
        let control = self.control_state(&h).await;
        Ok(EstablishView { ack: ack!(ack), mdln, softrev, control })
    }

    pub async fn online(&self) -> Result<OnlineView, String> {
        let h = self.host().await?;
        let a: OnlAck = h.request_online().await.map_err(|e| self.fail("S1F17", e))?;
        Ok(OnlineView { ack: ack!(a), control: self.control_state(&h).await })
    }

    pub async fn offline(&self) -> Result<OnlineView, String> {
        let h = self.host().await?;
        let a = h.request_offline().await.map_err(|e| self.fail("S1F15", e))?;
        Ok(OnlineView { ack: ack!(a), control: self.control_state(&h).await })
    }

    // ---- status variables and equipment constants ---------------------------------------------

    pub async fn sv_names(&self) -> Result<Vec<SvRow>, String> {
        let h = self.host().await?;
        let names = h.status_variable_names(&[]).await.map_err(|e| self.fail("S1F11", e))?;
        Ok(names.into_iter().map(|s| SvRow { id: s.id, name: s.name, units: s.units }).collect())
    }

    pub async fn sv_read(&self, ids: Vec<u64>) -> Result<Vec<SvValue>, String> {
        let h = self.host().await?;
        let values = h.status_variables(&ids).await.map_err(|e| self.fail("S1F3", e))?;
        if !ids.is_empty() && values.len() != ids.len() {
            return Err(self.fail("S1F3", format!("asked for {} values, got {}", ids.len(), values.len())));
        }
        Ok(ids
            .iter()
            .zip(values.iter())
            .map(|(id, v)| SvValue { id: *id, value: value_view(v) })
            .collect())
    }

    pub async fn ec_table(&self) -> Result<Vec<EcRow>, String> {
        let h = self.host().await?;
        let infos = h.equipment_constant_names(&[]).await.map_err(|e| self.fail("S2F29", e))?;
        let ids: Vec<u64> = infos.iter().map(|i| i.id).collect();
        let values = h.equipment_constants(&ids).await.map_err(|e| self.fail("S2F13", e))?;
        Ok(infos
            .iter()
            .zip(values.iter())
            .map(|(i, v)| EcRow {
                id: i.id,
                name: i.name.clone(),
                units: i.units.clone(),
                min: item_number(&i.min),
                max: item_number(&i.max),
                default: item_text(&i.default),
                value: value_view(v),
            })
            .collect())
    }

    /// Writes one equipment constant. The value is converted to the constant's own item type and
    /// checked against the range the equipment reported before anything is sent.
    pub async fn ec_write(&self, id: u64, text: String) -> Result<EcWriteView, String> {
        let h = self.host().await?;
        let current = h
            .equipment_constants(&[id])
            .await
            .map_err(|e| self.fail("S2F13", e))?
            .into_iter()
            .next()
            .ok_or("The equipment returned no value for this constant")?;
        if matches!(&current, Item::List(v) if v.is_empty()) {
            return Err(self.fail("S2F13", format!("The equipment has no equipment constant {id}")));
        }
        let item = parse_like(&current, &text)?;
        if let Some(info) = h.equipment_constant_names(&[id]).await.ok().and_then(|v| v.into_iter().next())
            && let (Some(n), Some(min), Some(max)) = (item_number(&item), item_number(&info.min), item_number(&info.max))
            && (n < min || n > max)
        {
            return Err(self.fail(
                &format!("Write {}", info.name),
                format!("{n} is out of range; {} accepts {min} to {max}", info.name),
            ));
        }
        self.log("info", format!("Writing constant {id} = {}", item.to_sml()));
        let eac: Eac = h.set_equipment_constants(&[(id, item)]).await.map_err(|e| self.fail("S2F15", e))?;
        let value = h.equipment_constants(&[id]).await.ok().and_then(|v| v.first().map(value_view));
        Ok(EcWriteView { ack: ack!(eac), value })
    }

    // ---- reports and events -------------------------------------------------------------------

    /// Defines a report, links it to events and optionally enables them (S2F33, S2F35, S2F37). Stops
    /// at the first step the equipment refuses.
    pub async fn report_wizard(&self, req: WizardRequest) -> Result<Vec<WizardStep>, String> {
        let h = self.host().await?;
        let mut steps = Vec::new();
        let d: Drack = h
            .define_reports(&[ReportDef { rptid: req.rptid, vids: req.vids }])
            .await
            .map_err(|e| self.fail("S2F33", e))?;
        steps.push(WizardStep { step: "S2F33 define report", ack: ack!(d) });
        if !d.is_ok() {
            return Ok(steps);
        }
        let links: Vec<EventLink> = req
            .ceids
            .iter()
            .map(|c| EventLink { ceid: *c, rptids: vec![req.rptid] })
            .collect();
        let l: Lrack = h.link_event_reports(&links).await.map_err(|e| self.fail("S2F35", e))?;
        steps.push(WizardStep { step: "S2F35 link events", ack: ack!(l) });
        if !l.is_ok() || !req.enable {
            return Ok(steps);
        }
        let e: Erack = h.enable_events(true, &req.ceids).await.map_err(|e| self.fail("S2F37", e))?;
        steps.push(WizardStep { step: "S2F37 enable events", ack: ack!(e) });
        Ok(steps)
    }

    /// Deletes a report definition (S2F33 with no variables).
    pub async fn report_delete(&self, rptid: u64) -> Result<AckView, String> {
        let h = self.host().await?;
        let d = h
            .define_reports(&[ReportDef { rptid, vids: vec![] }])
            .await
            .map_err(|e| self.fail("S2F33", e))?;
        Ok(ack!(d))
    }

    /// Enables or disables collection events; an empty list means all of them.
    pub async fn events_enable(&self, enable: bool, ceids: Vec<u64>) -> Result<AckView, String> {
        let h = self.host().await?;
        let e = h.enable_events(enable, &ceids).await.map_err(|e| self.fail("S2F37", e))?;
        Ok(ack!(e))
    }

    // ---- alarms, commands, programs, terminal ---------------------------------------------------

    pub async fn alarm_list(&self) -> Result<Vec<AlarmView>, String> {
        let h = self.host().await?;
        let alarms = h.list_alarms(&[]).await.map_err(|e| self.fail("S5F5", e))?;
        Ok(alarms
            .into_iter()
            .map(|a| AlarmView { alid: a.alid, set: a.is_set(), category: a.category(), text: a.altx })
            .collect())
    }

    pub async fn alarm_enable(&self, enable: bool, alid: Option<u64>) -> Result<AckView, String> {
        let h = self.host().await?;
        let a: Ackc5 = h.enable_alarms(enable, alid).await.map_err(|e| self.fail("S5F3", e))?;
        Ok(ack!(a))
    }

    pub async fn remote_command(&self, rcmd: String, params: Vec<ParamIn>) -> Result<CommandView, String> {
        let h = self.host().await?;
        let params = params
            .iter()
            .map(|p| param_item(&p.kind, &p.value).map(|v| CommandParam::new(p.name.clone(), v)).map_err(|e| format!("{}: {e}", p.name)))
            .collect::<Result<Vec<_>, _>>()?;
        let r = h.remote_command(&rcmd, &params).await.map_err(|e| self.fail("S2F41", e))?;
        let hcack: Hcack = r.hcack;
        Ok(CommandView {
            hcack: ack!(hcack),
            params: r.params.into_iter().map(|(name, c)| ParamAck { name, cpack: ack!(c) }).collect(),
        })
    }

    pub async fn ppid_list(&self) -> Result<Vec<String>, String> {
        let h = self.host().await?;
        h.process_program_list().await.map_err(|e| self.fail("S7F19", e))
    }

    pub async fn terminal_send(&self, tid: u8, text: String) -> Result<AckView, String> {
        let h = self.host().await?;
        let a: Ackc10 = h.terminal_display(tid, &text).await.map_err(|e| self.fail("S10F3", e))?;
        Ok(ack!(a))
    }

    /// Sends a message written in SML. Without the W-bit nothing is awaited.
    pub async fn send_sml(&self, sml: String) -> Result<RawReply, String> {
        let h = self.host().await?;
        let msg = SecsMessage::from_sml(&sml).map_err(es)?;
        if !msg.w_bit {
            h.connection().send(msg).map_err(|e| self.fail("send", e))?;
            return Ok(RawReply { name: None, sml: None });
        }
        let reply = h.send_raw(msg).await.map_err(|e| self.fail("send", e))?;
        Ok(RawReply { name: Some(reply.name()), sml: Some(reply.to_sml_pretty()) })
    }

    // ---- simulator ----------------------------------------------------------------------------

    pub async fn sim_start(&self, cfg: SimStart) -> Result<u16, String> {
        self.sim_stop().await;
        let mut hsms = HsmsConfig { device_id: cfg.device_id, ..HsmsConfig::default() };
        hsms.t3_ms = cfg.t3_ms;
        hsms.t5_ms = cfg.t5_ms;
        hsms.t6_ms = cfg.t6_ms;
        hsms.t7_ms = cfg.t7_ms;
        hsms.t8_ms = cfg.t8_ms;
        let mode = if cfg.mode == "active" {
            SimMode::Active { address: format!("{}:{}", cfg.host.trim(), cfg.port) }
        } else {
            SimMode::Passive { bind_addr: cfg.host.trim().to_string(), port: cfg.port }
        };
        let config = SimulatorConfig {
            mode,
            hsms,
            initiate_communication: cfg.initiate_communication,
            tick_ms: Some(100),
        };
        let sim = EquipmentSimulator::start(config).await.map_err(|e| self.fail("Simulator", e))?;
        let sim = Arc::new(sim);
        let port = sim.port();
        let task = forward_log("sim", sim.subscribe_log(), self.emit.clone());
        *self.sim.lock().await = Some(SimSlot { sim, tasks: vec![task] });
        self.log("info", format!("Equipment simulator started ({})", if cfg.mode == "active" { "active" } else { "passive" }));
        Ok(port)
    }

    pub async fn sim_stop(&self) {
        if let Some(s) = self.sim.lock().await.take() {
            s.sim.stop().await;
            for t in s.tasks {
                t.abort();
            }
            self.log("info", "Equipment simulator stopped");
        }
    }

    async fn sim(&self) -> Result<Arc<EquipmentSimulator>, String> {
        self.sim.lock().await.as_ref().map(|s| s.sim.clone()).ok_or_else(|| "The simulator is not running".to_string())
    }

    pub async fn sim_view(&self) -> Result<SimView, String> {
        let sim = self.sim().await?;
        let m = sim.model();
        let infos = m.status_variable_infos();
        Ok(SimView {
            port: sim.port(),
            selected: sim.is_selected(),
            sessions: sim.active_sessions(),
            requests: sim.requests_served(),
            control: control_view(m.control_state.code()).ok_or("bad control state")?,
            online_remote: m.online_substate == ControlState::OnlineRemote,
            comm: format!("{:?}", m.comm_state),
            comm_enabled: m.comm_enabled,
            process: ProcessView {
                state: match m.chamber.state {
                    ProcessState::Idle => "Idle",
                    ProcessState::Executing => "Executing",
                    ProcessState::Paused => "Paused",
                },
                ppid: m.chamber.ppid.clone(),
                lot_id: m.chamber.lot_id.clone(),
                wafers: m.chamber.wafer_count,
                elapsed_s: m.chamber.elapsed_s,
                duration_s: m.chamber.duration_s,
                pressure: m.chamber.pressure_mtorr,
                temperature: m.chamber.temperature_c,
            },
            svs: infos
                .into_iter()
                .map(|s| SimSv {
                    id: s.id,
                    name: s.name,
                    units: s.units,
                    text: m.sv_value(s.id).map(|i| item_text(&i)).unwrap_or_default(),
                    editable: matches!(s.id, 10..=13 | 15),
                })
                .collect(),
            constants: m
                .constants
                .iter()
                .map(|c| SimEc {
                    id: c.id,
                    name: c.name.clone(),
                    units: c.units.clone(),
                    min: c.min,
                    max: c.max,
                    value: c.value_f64(),
                    locked: c.locked_while_processing && m.chamber.state != ProcessState::Idle,
                })
                .collect(),
            alarms: m
                .alarms
                .iter()
                .map(|a| SimAlarm { alid: a.alid, text: a.text.clone(), category: a.category, enabled: a.enabled, set: a.set })
                .collect(),
            events: m.events.iter().map(|e| SimEvent { ceid: e.ceid, name: e.name.clone(), enabled: e.enabled }).collect(),
            ppids: m.ppids.clone(),
            terminal: m.terminal_messages.iter().map(|t| format!("[{}] {}", t.tid, t.text)).collect(),
        })
    }

    pub async fn sim_action(&self, action: SimAction) -> Result<(), String> {
        let sim = self.sim().await?;
        let result: Result<(), String> = sim.with_model(|m| match action {
            SimAction::OperatorOffline => {
                m.operator_offline();
                Ok(())
            }
            SimAction::OperatorOnline => {
                if m.operator_online() {
                    Ok(())
                } else {
                    Err("The equipment stayed offline: no host is communicating".into())
                }
            }
            SimAction::SetRemote { remote } => {
                m.operator_set_remote(remote);
                Ok(())
            }
            SimAction::CommEnable { enabled } => {
                m.set_communication_enabled(enabled);
                Ok(())
            }
            SimAction::StartProcess { ppid } => m.start_process(ppid.as_deref()).map_err(|e| format!("Cannot start: {e:?}")),
            SimAction::StopProcess => m.stop_process().map_err(|e| format!("Cannot stop: {e:?}")),
            SimAction::PauseProcess => m.pause_process().map_err(|e| format!("Cannot pause: {e:?}")),
            SimAction::ResumeProcess => m.resume_process().map_err(|e| format!("Cannot resume: {e:?}")),
            SimAction::RaiseAlarm { alid } => {
                if m.raise_alarm(alid) { Ok(()) } else { Err("Unknown alarm, or it is already set".into()) }
            }
            SimAction::ClearAlarm { alid } => {
                if m.clear_alarm(alid) { Ok(()) } else { Err("Unknown alarm, or it is not set".into()) }
            }
            SimAction::SetConstant { id, value } => {
                let eac = m.set_constant(id, value);
                if eac.is_ok() { Ok(()) } else { Err(format!("Constant {id} not changed: {eac:?}")) }
            }
            SimAction::SetSv { id, text } => set_sv(m, id, &text),
        });
        result.inspect_err(|e| self.log("error", e.clone()))
    }
}

fn set_sv(m: &mut mt_secsgem::EquipmentModel, id: u64, text: &str) -> Result<(), String> {
    use mt_secsgem::sim::svid;
    let num = || text.trim().parse::<f64>().map_err(|_| format!("\"{}\" is not a number", text.trim()));
    match id {
        svid::CHAMBER_PRESSURE => m.chamber.pressure_mtorr = num()?,
        svid::CHAMBER_TEMPERATURE => m.chamber.temperature_c = num()?,
        svid::WAFER_COUNT => {
            m.chamber.wafer_count = text.trim().parse().map_err(|_| format!("\"{}\" is not a wafer count", text.trim()))?;
        }
        svid::ACTIVE_PPID => m.chamber.ppid = text.trim().to_string(),
        svid::LOT_ID => m.chamber.lot_id = text.trim().to_string(),
        _ => return Err(format!("SV {id} is derived from the state model and cannot be edited")),
    }
    Ok(())
}

fn forward_events(host: Arc<GemHost>, slot: Arc<Mutex<HostSlot>>, emit: Emit) -> JoinHandle<()> {
    let mut rx = host.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(e) => {
                    let closed = matches!(e, HostEvent::Closed { .. });
                    emit("gem-event", event_payload(&e));
                    if closed {
                        let mut s = slot.lock().await;
                        if s.host.as_ref().is_some_and(|h| Arc::ptr_eq(h, &host)) {
                            s.host = None;
                        }
                        emit("gem-conn", json!({ "state": "closed" }));
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

// ---- simulator shapes ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimStart {
    /// `passive` listens on `host:port`; `active` connects to `host:port`.
    pub mode: String,
    pub host: String,
    pub port: u16,
    pub device_id: u16,
    pub t3_ms: u64,
    pub t5_ms: u64,
    pub t6_ms: u64,
    pub t7_ms: u64,
    pub t8_ms: u64,
    pub initiate_communication: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SimAction {
    OperatorOffline,
    OperatorOnline,
    SetRemote { remote: bool },
    CommEnable { enabled: bool },
    StartProcess { ppid: Option<String> },
    StopProcess,
    PauseProcess,
    ResumeProcess,
    RaiseAlarm { alid: u64 },
    ClearAlarm { alid: u64 },
    SetConstant { id: u64, value: f64 },
    SetSv { id: u64, text: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessView {
    pub state: &'static str,
    pub ppid: String,
    pub lot_id: String,
    pub wafers: u32,
    pub elapsed_s: f64,
    pub duration_s: f64,
    pub pressure: f64,
    pub temperature: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimSv {
    pub id: u64,
    pub name: String,
    pub units: String,
    pub text: String,
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimEc {
    pub id: u64,
    pub name: String,
    pub units: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimAlarm {
    pub alid: u64,
    pub text: String,
    pub category: u8,
    pub enabled: bool,
    pub set: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimEvent {
    pub ceid: u64,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimView {
    pub port: u16,
    pub selected: bool,
    pub sessions: u64,
    pub requests: u64,
    pub control: ControlView,
    pub online_remote: bool,
    pub comm: String,
    pub comm_enabled: bool,
    pub process: ProcessView,
    pub svs: Vec<SimSv>,
    pub constants: Vec<SimEc>,
    pub alarms: Vec<SimAlarm>,
    pub events: Vec<SimEvent>,
    pub ppids: Vec<String>,
    pub terminal: Vec<String>,
}
