//! The mutable simulated machine: observation ring buffer, evolving values, conditions, assets.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use super::template::DeviceTemplate;
use super::xmlout::{self, Owner, OwnerObservations};
use super::xpath::{Node, Selector};
use crate::error::{AgentErrorCode, MtConnectError, Result};
use crate::types::{
    Asset, Category, Condition, ConditionLevel, DataItem, Device, Header, Observation, Value,
};
use crate::util::{iso_from_micros, now_micros};

/// Error from serving a query: becomes an `MTConnectError` document.
#[derive(Debug, Clone)]
pub(crate) struct QueryError {
    pub code: AgentErrorCode,
    pub message: String,
}

impl QueryError {
    fn new(code: AgentErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

/// A rendered `current`/`sample` response and where the next request should continue.
#[derive(Debug)]
pub(crate) struct Rendered {
    pub xml: String,
    pub next_sequence: u64,
}

struct ItemMeta {
    item: DataItem,
    owner: usize,
    chain: Vec<Node>,
}

/// Live state of one data item, for UIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimItemState {
    pub id: String,
    pub name: Option<String>,
    pub category: Category,
    #[serde(rename = "type")]
    pub data_type: String,
    pub sub_type: Option<String>,
    pub units: Option<String>,
    pub value: Value,
    pub condition: Option<Condition>,
    pub timestamp: Option<String>,
    pub sequence: Option<u64>,
    /// True if the value was set by hand and is no longer driven by the simulated program.
    pub pinned: bool,
}

/// Point-in-time view of the whole simulator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSnapshot {
    pub device: Device,
    pub instance_id: u64,
    pub buffer_size: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub next_sequence: u64,
    pub paused: bool,
    pub items: Vec<SimItemState>,
}

// ── Program behaviour ──────────────────────────────────────────────────────

const CYCLE_SECS: f64 = 24.0;
const IDLE_SECS: f64 = 3.0;
const SPINDLE_RPM: f64 = 8000.0;
const SPINDLE_ACCEL: f64 = 4000.0;
const FEED_MM_S: f64 = 25.0;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Running,
}

struct Behavior {
    template: DeviceTemplate,
    phase: Phase,
    t: f64,
    part_count: u64,
    spindle: f64,
    pos: Vec<f64>,
    rng: u64,
}

impl Behavior {
    fn new(template: DeviceTemplate) -> Self {
        let home = Self::home(template);
        Self { template, phase: Phase::Idle, t: 0.0, part_count: 0, spindle: 0.0, pos: home, rng: 0x2545_F491_4F6C_DD1D }
    }

    fn home(template: DeviceTemplate) -> Vec<f64> {
        match template {
            DeviceTemplate::Vmc3Axis => vec![0.0, 0.0, 50.0],
            DeviceTemplate::Lathe2Axis => vec![50.0, 20.0],
        }
    }

    /// Uniform noise in [-1, 1].
    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }

    fn target_pos(&self, u: f64) -> Vec<f64> {
        let tau = std::f64::consts::TAU;
        match self.template {
            DeviceTemplate::Vmc3Axis => vec![
                100.0 + 80.0 * (tau * u * 3.0).cos(),
                75.0 + 60.0 * (tau * u * 3.0).sin(),
                5.0 - 12.0 * (0.5 - 0.5 * (tau * u).cos()),
            ],
            DeviceTemplate::Lathe2Axis => {
                vec![20.0 + 15.0 * (0.5 - 0.5 * (tau * u * 5.0).cos()), 20.0 - 70.0 * u]
            }
        }
    }

    /// Advance `dt` seconds and return the values of every program-driven data item.
    fn step(&mut self, dt: f64, estop: bool) -> Vec<(&'static str, Value)> {
        let running = if estop {
            self.phase = Phase::Idle;
            self.t = 0.0;
            false
        } else {
            self.t += dt;
            match self.phase {
                Phase::Idle if self.t >= IDLE_SECS => {
                    self.phase = Phase::Running;
                    self.t = 0.0;
                }
                Phase::Running if self.t >= CYCLE_SECS => {
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                    self.part_count += 1;
                }
                _ => {}
            }
            self.phase == Phase::Running
        };

        let target_rpm = if running { SPINDLE_RPM } else { 0.0 };
        let max = SPINDLE_ACCEL * dt;
        self.spindle += (target_rpm - self.spindle).clamp(-max, max);

        let u = (self.t / CYCLE_SECS).clamp(0.0, 1.0);
        let target = if running { self.target_pos(u) } else { Self::home(self.template) };
        let k = if running { 1.0 } else { (dt * 3.0).min(1.0) };
        for (p, t) in self.pos.iter_mut().zip(target) {
            *p += (t - *p) * k;
        }

        let spinning = self.spindle > 50.0;
        let rpm = if spinning { self.spindle + self.noise() * 15.0 } else { 0.0 };
        let load = if spinning {
            let wave = 0.5 + 0.5 * (std::f64::consts::TAU * self.t / 6.0).sin();
            (22.0 + 18.0 * wave * (self.spindle / SPINDLE_RPM) + self.noise() * 1.5).max(0.0)
        } else {
            0.0
        };
        let feed = if running { FEED_MM_S * (1.0 + self.noise() * 0.02) } else { 0.0 };
        let line = if running { 10 * ((u * 100.0) as u64 + 1) } else { 0 };
        let exec = if estop {
            "STOPPED"
        } else if running {
            "ACTIVE"
        } else {
            "READY"
        };

        let mut out: Vec<(&'static str, Value)> = vec![
            ("avail", Value::Text("AVAILABLE".into())),
            ("mode", Value::Text("AUTOMATIC".into())),
            ("exec", Value::Text(exec.into())),
            ("program", Value::Text("O1001".into())),
            ("line", Value::Text(line.to_string())),
            ("pcount", Value::Text(self.part_count.to_string())),
            ("Sspeed", Value::Number(rpm)),
            ("Sload", Value::Number(load)),
            ("pfeed", Value::Number(feed)),
        ];
        for (id, p) in self.template.axis_ids().iter().zip(&self.pos) {
            out.push((*id, Value::Number(*p)));
        }
        out
    }
}

// ── Shared state ───────────────────────────────────────────────────────────

struct State {
    device: Device,
    owners: Vec<Owner>,
    items: Vec<ItemMeta>,
    index: HashMap<String, usize>,
    instance_id: u64,
    buffer_size: usize,
    buffer: VecDeque<Observation>,
    next_seq: u64,
    latest: Vec<Option<Observation>>,
    pinned: HashSet<String>,
    paused: bool,
    behavior: Behavior,
    assets: Vec<Asset>,
    asset_buffer_size: u64,
    pending: Vec<Observation>,
}

fn node(name: &str, attrs: &[(&str, Option<&str>)]) -> Node {
    Node {
        name: name.into(),
        attrs: attrs.iter().filter_map(|(k, v)| v.map(|v| (k.to_string(), v.to_string()))).collect(),
    }
}

fn collect_items(
    device: &Device,
) -> (Vec<Owner>, Vec<ItemMeta>) {
    let mut owners = vec![Owner { element: "Device".into(), id: device.id.clone(), name: Some(device.name.clone()) }];
    let mut items = Vec::new();
    let root = vec![
        node("Devices", &[]),
        node(
            "Device",
            &[("id", Some(&device.id)), ("name", Some(&device.name)), ("uuid", device.uuid.as_deref())],
        ),
    ];

    fn push_items(
        data_items: &[DataItem],
        owner: usize,
        chain: &[Node],
        items: &mut Vec<ItemMeta>,
    ) {
        for d in data_items {
            let mut c = chain.to_vec();
            c.push(node(
                "DataItem",
                &[
                    ("id", Some(&d.id)),
                    ("name", d.name.as_deref()),
                    ("category", Some(d.category.as_str())),
                    ("type", Some(&d.data_type)),
                    ("subType", d.sub_type.as_deref()),
                ],
            ));
            items.push(ItemMeta { item: d.clone(), owner, chain: c });
        }
    }

    fn walk(
        comps: &[crate::types::Component],
        chain: &[Node],
        owners: &mut Vec<Owner>,
        items: &mut Vec<ItemMeta>,
    ) {
        for c in comps {
            owners.push(Owner { element: c.element.clone(), id: c.id.clone(), name: c.name.clone() });
            let owner = owners.len() - 1;
            let mut here = chain.to_vec();
            here.push(node(
                &c.element,
                &[("id", Some(&c.id)), ("name", c.name.as_deref()), ("uuid", c.uuid.as_deref())],
            ));
            push_items(&c.data_items, owner, &here, items);
            walk(&c.components, &here, owners, items);
        }
    }

    push_items(&device.data_items, 0, &root, &mut items);
    walk(&device.components, &root, &mut owners, &mut items);
    (owners, items)
}

impl State {
    fn first_seq(&self) -> u64 {
        self.buffer.front().map_or(self.next_seq, |o| o.sequence)
    }

    fn last_seq(&self) -> u64 {
        self.next_seq - 1
    }

    fn header(&self, with_sequences: bool) -> Header {
        let mut h = Header {
            creation_time: Some(iso_from_micros(now_micros())),
            sender: Some("mt-mtconnect-simulator".into()),
            instance_id: self.instance_id,
            version: Some(env!("CARGO_PKG_VERSION").into()),
            buffer_size: Some(self.buffer_size as u64),
            ..Header::default()
        };
        if with_sequences {
            h.first_sequence = Some(self.first_seq());
            h.last_sequence = Some(self.last_seq());
            h.next_sequence = Some(self.next_seq);
        }
        h
    }

    fn make_observation(&self, idx: usize, value: Value, condition: Option<Condition>) -> Observation {
        let item = &self.items[idx].item;
        let element = match &condition {
            Some(c) => c.level.element_name().to_string(),
            None => xmlout::pascal(&item.data_type),
        };
        Observation {
            data_item_id: item.id.clone(),
            name: item.name.clone(),
            element,
            category: item.category,
            data_item_type: condition.as_ref().map(|_| item.data_type.clone()),
            sub_type: item.sub_type.clone(),
            timestamp: iso_from_micros(now_micros()),
            sequence: 0,
            value,
            condition,
            composition_id: item.composition_id.clone(),
            statistic: None,
            duration: None,
            reset_triggered: None,
        }
    }

    /// Record a new value for data item `idx` unless it equals the latest one.
    fn observe(&mut self, idx: usize, value: Value, condition: Option<Condition>) {
        let value = match value {
            Value::Number(n) => Value::Number((n * 1000.0).round() / 1000.0),
            v => v,
        };
        if let Some(prev) = &self.latest[idx]
            && prev.value == value
            && prev.condition == condition
        {
            return;
        }
        let mut obs = self.make_observation(idx, value, condition);
        obs.sequence = self.next_seq;
        self.next_seq += 1;
        self.buffer.push_back(obs.clone());
        while self.buffer.len() > self.buffer_size {
            self.buffer.pop_front();
        }
        self.latest[idx] = Some(obs.clone());
        self.pending.push(obs);
    }

    fn observe_all_unavailable(&mut self) {
        for idx in 0..self.items.len() {
            if self.items[idx].item.category == Category::Condition {
                let c = Condition {
                    level: ConditionLevel::Unavailable,
                    native_code: None,
                    native_severity: None,
                    qualifier: None,
                    message: None,
                    condition_id: None,
                };
                self.observe(idx, Value::Unavailable, Some(c));
            } else {
                self.observe(idx, Value::Unavailable, None);
            }
        }
    }

    fn normal() -> Condition {
        Condition {
            level: ConditionLevel::Normal,
            native_code: None,
            native_severity: None,
            qualifier: None,
            message: None,
            condition_id: None,
        }
    }

    /// The machine powers up: availability, e-stop armed, conditions normal, program values.
    fn power_on(&mut self) {
        let estop = self.index["estop"];
        self.observe(estop, Value::Text("ARMED".into()), None);
        for id in ["sys_cond", "motion_cond"] {
            let idx = self.index[id];
            self.observe(idx, Value::Text(String::new()), Some(Self::normal()));
        }
        self.run_program(0.0);
    }

    fn estop_triggered(&self) -> bool {
        self.index
            .get("estop")
            .and_then(|i| self.latest[*i].as_ref())
            .is_some_and(|o| o.value == Value::Text("TRIGGERED".into()))
    }

    fn run_program(&mut self, dt: f64) {
        let estop = self.estop_triggered();
        for (id, value) in self.behavior.step(dt, estop) {
            if self.pinned.contains(id) {
                continue;
            }
            if let Some(&idx) = self.index.get(id) {
                self.observe(idx, value, None);
            }
        }
    }

    fn chain_selected(&self, selector: &Option<Selector>, idx: usize) -> bool {
        selector.as_ref().is_none_or(|s| s.selects(&self.items[idx].chain))
    }

    fn render_streams(&self, mut obs: Vec<&Observation>, next_sequence: u64) -> Rendered {
        obs.sort_by_key(|o| o.sequence);
        let mut by_owner: Vec<Vec<&Observation>> = vec![Vec::new(); self.owners.len()];
        for o in obs {
            if let Some(&idx) = self.index.get(&o.data_item_id) {
                by_owner[self.items[idx].owner].push(o);
            }
        }
        let groups: Vec<OwnerObservations<'_>> = by_owner
            .into_iter()
            .enumerate()
            .map(|(i, observations)| OwnerObservations { owner: &self.owners[i], observations })
            .collect();
        let mut header = self.header(true);
        header.next_sequence = Some(next_sequence);
        Rendered { xml: xmlout::streams(&header, &self.device, &groups), next_sequence }
    }
}

fn parse_selector(path: Option<&str>) -> std::result::Result<Option<Selector>, QueryError> {
    match path.filter(|p| !p.trim().is_empty()) {
        None => Ok(None),
        Some(p) => Selector::parse(p)
            .map(Some)
            .map_err(|e| QueryError::new(AgentErrorCode::InvalidXPath, e)),
    }
}

// ── Public handle ──────────────────────────────────────────────────────────

struct Shared {
    state: Mutex<State>,
    events: broadcast::Sender<Observation>,
    seq: watch::Sender<(u64, u64)>,
}

/// Handle to the simulated machine. Cheap to clone; every clone mutates the same model, which
/// the running agent and SHDR adapter serve from. All methods are safe to call while the
/// simulator is running.
#[derive(Clone)]
pub struct SimModel {
    shared: Arc<Shared>,
}

impl SimModel {
    /// A powered-up machine from `template` with a ring buffer of `buffer_size` observations.
    pub fn new(template: DeviceTemplate, buffer_size: usize) -> Self {
        let device = template.build();
        let (owners, items) = collect_items(&device);
        let index = items.iter().enumerate().map(|(i, m)| (m.item.id.clone(), i)).collect();
        let n = items.len();
        let mut state = State {
            device,
            owners,
            items,
            index,
            instance_id: new_instance_id(),
            buffer_size: buffer_size.max(16),
            buffer: VecDeque::new(),
            next_seq: 1,
            latest: vec![None; n],
            pinned: HashSet::new(),
            paused: false,
            behavior: Behavior::new(template),
            assets: Vec::new(),
            asset_buffer_size: 1024,
            pending: Vec::new(),
        };
        state.observe_all_unavailable();
        state.power_on();
        let (events, _) = broadcast::channel(4096);
        let (seq, _) = watch::channel((state.instance_id, state.last_seq()));
        let model = Self { shared: Arc::new(Shared { state: Mutex::new(state), events, seq }) };
        {
            let mut st = model.lock();
            let device_uuid = st.device.uuid.clone().unwrap_or_default();
            for n in 1..=3u32 {
                st.assets.push(cutting_tool(&device_uuid, n));
            }
            model.flush(&mut st);
        }
        model
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish pending observations to subscribers and wake streaming requests.
    fn flush(&self, st: &mut State) {
        for o in st.pending.drain(..) {
            let _ = self.shared.events.send(o);
        }
        self.shared.seq.send_replace((st.instance_id, st.last_seq()));
    }

    /// The device description served by `/probe`.
    pub fn device(&self) -> Device {
        self.lock().device.clone()
    }

    /// Observations as they are recorded (one per change), for UIs and the SHDR adapter.
    pub fn subscribe(&self) -> broadcast::Receiver<Observation> {
        self.shared.events.subscribe()
    }

    pub(crate) fn watch_sequence(&self) -> watch::Receiver<(u64, u64)> {
        self.shared.seq.subscribe()
    }

    pub(crate) fn instance_id(&self) -> u64 {
        self.lock().instance_id
    }

    /// Advance the simulated program by `dt_secs`. The background task calls this every tick;
    /// it is public so tests and UIs can drive time deterministically. Does nothing while paused.
    pub fn tick(&self, dt_secs: f64) {
        let mut st = self.lock();
        if st.paused {
            return;
        }
        st.run_program(dt_secs);
        self.flush(&mut st);
    }

    /// Freeze (`true`) or resume the simulated program. While paused no observations are made
    /// except those caused by explicit changes.
    pub fn set_paused(&self, paused: bool) {
        self.lock().paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    /// Set a SAMPLE or EVENT data item by hand. The item stays pinned to this value until
    /// [`release`](Self::release); the simulated program no longer updates it.
    pub fn set_value(&self, data_item_id: &str, value: Value) -> Result<()> {
        let mut st = self.lock();
        let idx = *st
            .index
            .get(data_item_id)
            .ok_or_else(|| MtConnectError::Simulator(format!("unknown data item '{data_item_id}'")))?;
        match (st.items[idx].item.category, &value) {
            (Category::Condition, _) => {
                return Err(MtConnectError::Simulator(format!(
                    "'{data_item_id}' is a CONDITION: use raise_condition / clear_condition"
                )));
            }
            (Category::Sample, Value::Number(_) | Value::Unavailable) => {}
            (Category::Sample, _) => {
                return Err(MtConnectError::Simulator(format!(
                    "'{data_item_id}' is a SAMPLE and needs a number or UNAVAILABLE"
                )));
            }
            (Category::Event, Value::Number(_)) => {
                return Err(MtConnectError::Simulator(format!("'{data_item_id}' is an EVENT: use a text value")));
            }
            (Category::Event, _) => {}
        }
        st.pinned.insert(data_item_id.to_string());
        st.observe(idx, value, None);
        self.flush(&mut st);
        Ok(())
    }

    /// Hand a pinned data item back to the simulated program.
    pub fn release(&self, data_item_id: &str) {
        self.lock().pinned.remove(data_item_id);
    }

    /// Hand every pinned data item back to the simulated program.
    pub fn release_all(&self) {
        self.lock().pinned.clear();
    }

    /// Set a CONDITION data item to an arbitrary state (e.g. `Unavailable`).
    pub fn set_condition(&self, data_item_id: &str, condition: Condition) -> Result<()> {
        let mut st = self.lock();
        let idx = *st
            .index
            .get(data_item_id)
            .ok_or_else(|| MtConnectError::Simulator(format!("unknown data item '{data_item_id}'")))?;
        if st.items[idx].item.category != Category::Condition {
            return Err(MtConnectError::Simulator(format!("'{data_item_id}' is not a CONDITION")));
        }
        let value = match condition.level {
            ConditionLevel::Unavailable => Value::Unavailable,
            _ => Value::Text(condition.message.clone().unwrap_or_default()),
        };
        st.observe(idx, value, Some(condition));
        self.flush(&mut st);
        Ok(())
    }

    /// Raise a WARNING or FAULT. `native_code` is the controller's own alarm code.
    pub fn raise_condition(
        &self,
        data_item_id: &str,
        level: ConditionLevel,
        native_code: &str,
        message: &str,
    ) -> Result<()> {
        if !matches!(level, ConditionLevel::Warning | ConditionLevel::Fault) {
            return Err(MtConnectError::Simulator("raise_condition needs level Warning or Fault".into()));
        }
        self.set_condition(
            data_item_id,
            Condition {
                level,
                native_code: Some(native_code.into()),
                native_severity: Some(if level == ConditionLevel::Fault { "1000" } else { "500" }.into()),
                qualifier: None,
                message: Some(message.into()),
                condition_id: None,
            },
        )
    }

    /// Return a condition to NORMAL.
    pub fn clear_condition(&self, data_item_id: &str) -> Result<()> {
        self.set_condition(data_item_id, State::normal())
    }

    /// Simulate an agent restart: new `instanceId`, empty buffer, sequence numbers restart at 1,
    /// then the machine's current state is republished.
    pub fn restart_agent(&self) {
        let mut st = self.lock();
        let old = st.instance_id;
        let mut id = new_instance_id();
        if id == old {
            id += 1;
        }
        st.instance_id = id;
        st.buffer.clear();
        st.next_seq = 1;
        let previous: Vec<Option<Observation>> = st.latest.iter_mut().map(Option::take).collect();
        st.pending.clear();
        st.observe_all_unavailable();
        st.power_on();
        // The machine did not restart, only the agent: carry over hand-set values, the e-stop
        // and condition states into the new buffer.
        for (idx, prev) in previous.into_iter().enumerate() {
            let Some(prev) = prev else { continue };
            let item = &st.items[idx].item;
            if st.pinned.contains(&item.id) || item.id == "estop" || item.category == Category::Condition {
                st.observe(idx, prev.value, prev.condition);
            }
        }
        self.flush(&mut st);
    }

    /// Add an asset (raw XML element such as `<CuttingTool assetId="..." ...>...</CuttingTool>`).
    pub fn add_asset(&self, asset: Asset) {
        let mut st = self.lock();
        st.assets.retain(|a| a.asset_id != asset.asset_id);
        st.assets.push(asset);
        let cap = st.asset_buffer_size as usize;
        while st.assets.len() > cap {
            st.assets.remove(0);
        }
    }

    /// Remove an asset by id; returns whether it existed.
    pub fn remove_asset(&self, asset_id: &str) -> bool {
        let mut st = self.lock();
        let before = st.assets.len();
        st.assets.retain(|a| a.asset_id != asset_id);
        st.assets.len() != before
    }

    pub fn assets(&self) -> Vec<Asset> {
        self.lock().assets.clone()
    }

    /// The latest observation of every data item, in device order.
    pub fn current_observations(&self) -> Vec<Observation> {
        self.lock().latest.iter().flatten().cloned().collect()
    }

    pub fn snapshot(&self) -> SimSnapshot {
        let st = self.lock();
        let items = st
            .items
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let latest = st.latest[i].as_ref();
                SimItemState {
                    id: m.item.id.clone(),
                    name: m.item.name.clone(),
                    category: m.item.category,
                    data_type: m.item.data_type.clone(),
                    sub_type: m.item.sub_type.clone(),
                    units: m.item.units.clone(),
                    value: latest.map_or(Value::Unavailable, |o| o.value.clone()),
                    condition: latest.and_then(|o| o.condition.clone()),
                    timestamp: latest.map(|o| o.timestamp.clone()),
                    sequence: latest.map(|o| o.sequence),
                    pinned: st.pinned.contains(&m.item.id),
                }
            })
            .collect();
        SimSnapshot {
            device: st.device.clone(),
            instance_id: st.instance_id,
            buffer_size: st.buffer_size as u64,
            first_sequence: st.first_seq(),
            last_sequence: st.last_seq(),
            next_sequence: st.next_seq,
            paused: st.paused,
            items,
        }
    }

    // ── Query side (HTTP) ─────────────────────────────────────────────────

    pub(crate) fn check_device(&self, device: Option<&str>) -> std::result::Result<(), QueryError> {
        let st = self.lock();
        match device {
            Some(d) if d != st.device.name && st.device.uuid.as_deref() != Some(d) && d != st.device.id => {
                Err(QueryError::new(AgentErrorCode::NoDevice, format!("Could not find the device '{d}'")))
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn render_probe(&self) -> String {
        let st = self.lock();
        let mut h = st.header(false);
        h.asset_buffer_size = Some(st.asset_buffer_size);
        h.asset_count = Some(st.assets.len() as u64);
        xmlout::devices(&h, &st.device)
    }

    pub(crate) fn render_current(&self, path: Option<&str>) -> std::result::Result<Rendered, QueryError> {
        let selector = parse_selector(path)?;
        let st = self.lock();
        let obs: Vec<&Observation> = st
            .latest
            .iter()
            .enumerate()
            .filter(|(i, _)| st.chain_selected(&selector, *i))
            .filter_map(|(_, o)| o.as_ref())
            .collect();
        Ok(st.render_streams(obs, st.next_seq))
    }

    pub(crate) fn render_sample(
        &self,
        path: Option<&str>,
        from: Option<u64>,
        count: Option<i64>,
    ) -> std::result::Result<Rendered, QueryError> {
        let selector = parse_selector(path)?;
        let st = self.lock();
        let count = count.unwrap_or_else(|| 100.min(st.buffer_size as i64));
        if count < 1 || count as usize > st.buffer_size {
            return Err(QueryError::new(
                AgentErrorCode::OutOfRange,
                format!("'count' must be between 1 and {}", st.buffer_size),
            ));
        }
        let first = st.first_seq();
        let from = from.unwrap_or(first);
        if from < first || from > st.next_seq {
            return Err(QueryError::new(
                AgentErrorCode::OutOfRange,
                format!("'from' must be between {first} and {}", st.next_seq),
            ));
        }
        let mut selected: Vec<&Observation> = Vec::new();
        let mut next = st.next_seq;
        for o in st.buffer.iter().skip_while(|o| o.sequence < from) {
            let idx = st.index[&o.data_item_id];
            if st.chain_selected(&selector, idx) {
                if selected.len() as i64 >= count {
                    next = o.sequence;
                    break;
                }
                selected.push(o);
            }
        }
        Ok(st.render_streams(selected, next))
    }

    pub(crate) fn render_assets(&self, asset_type: Option<&str>, count: Option<usize>, removed: bool) -> String {
        let st = self.lock();
        let mut h = st.header(false);
        h.buffer_size = None;
        h.asset_buffer_size = Some(st.asset_buffer_size);
        h.asset_count = Some(st.assets.len() as u64);
        let mut list: Vec<&Asset> = st
            .assets
            .iter()
            .rev()
            .filter(|a| removed || !a.removed)
            .filter(|a| asset_type.is_none_or(|t| a.asset_type == t))
            .take(count.unwrap_or(100))
            .collect();
        list.reverse();
        xmlout::assets(&h, &list)
    }

    pub(crate) fn render_asset(&self, ids: &[&str]) -> std::result::Result<String, QueryError> {
        let st = self.lock();
        let mut found: Vec<&Asset> = Vec::new();
        for id in ids {
            match st.assets.iter().find(|a| a.asset_id == *id) {
                Some(a) => found.push(a),
                None => {
                    return Err(QueryError::new(AgentErrorCode::AssetNotFound, format!("Cannot find asset '{id}'")));
                }
            }
        }
        let mut h = st.header(false);
        h.buffer_size = None;
        h.asset_buffer_size = Some(st.asset_buffer_size);
        h.asset_count = Some(st.assets.len() as u64);
        Ok(xmlout::assets(&h, &found))
    }

    pub(crate) fn render_error(&self, code: &AgentErrorCode, message: &str) -> String {
        let st = self.lock();
        xmlout::error(&st.header(false), code, message)
    }
}

/// A simulated CuttingTool asset, shaped like the `CuttingTool` assets cppagent serves.
fn cutting_tool(device_uuid: &str, n: u32) -> Asset {
    let id = format!("TOOL.{n}");
    let timestamp = iso_from_micros(now_micros());
    let raw_xml = format!(
        "<CuttingTool assetId=\"{id}\" deviceUuid=\"{device_uuid}\" serialNumber=\"{n:04}\" timestamp=\"{timestamp}\" toolId=\"{n}\">\
<CuttingToolLifeCycle><CutterStatus><Status>AVAILABLE</Status></CutterStatus>\
<ToolLife countDirection=\"UP\" initial=\"0\" limit=\"500\" type=\"PART_COUNT\">0</ToolLife>\
<ProgramToolGroup>{n}</ProgramToolGroup><ProgramToolNumber>{n}</ProgramToolNumber>\
<Location type=\"POT\">{n}</Location></CuttingToolLifeCycle></CuttingTool>"
    );
    Asset {
        asset_type: "CuttingTool".into(),
        asset_id: id,
        device_uuid: Some(device_uuid.into()),
        timestamp: Some(timestamp),
        removed: false,
        raw_xml,
    }
}

fn new_instance_id() -> u64 {
    // Seconds since the epoch, like cppagent: strictly increasing across restarts.
    (now_micros() / 1_000_000).max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_state_and_ring_buffer_bounds() {
        let m = SimModel::new(DeviceTemplate::Vmc3Axis, 64);
        let s = m.snapshot();
        assert_eq!(s.first_sequence, 1);
        assert_eq!(s.next_sequence, s.last_sequence + 1);
        let by_id = |id: &str| s.items.iter().find(|i| i.id == id).unwrap().clone();
        assert_eq!(by_id("avail").value, Value::Text("AVAILABLE".into()));
        assert_eq!(by_id("estop").value, Value::Text("ARMED".into()));
        assert_eq!(by_id("sys_cond").condition.unwrap().level, ConditionLevel::Normal);

        // Run long enough to overflow the 64-slot buffer.
        for _ in 0..2000 {
            m.tick(0.25);
        }
        let s = m.snapshot();
        assert_eq!(s.last_sequence - s.first_sequence + 1, 64);
        assert_eq!(s.next_sequence, s.last_sequence + 1);
    }

    #[test]
    fn program_cycles_and_counts_parts() {
        let m = SimModel::new(DeviceTemplate::Vmc3Axis, 5000);
        let mut saw_active = false;
        for _ in 0..((IDLE_SECS + CYCLE_SECS) * 4.0) as usize + 8 {
            m.tick(0.25);
            let s = m.snapshot();
            if s.items.iter().any(|i| i.id == "exec" && i.value == Value::Text("ACTIVE".into())) {
                saw_active = true;
                let rpm = s.items.iter().find(|i| i.id == "Sspeed").unwrap().value.as_f64().unwrap();
                assert!((0.0..9000.0).contains(&rpm));
            }
        }
        assert!(saw_active);
        let s = m.snapshot();
        let parts = s.items.iter().find(|i| i.id == "pcount").unwrap().value.as_f64().unwrap();
        assert_eq!(parts, 1.0);
    }

    #[test]
    fn set_value_pins_and_validates() {
        let m = SimModel::new(DeviceTemplate::Lathe2Axis, 100);
        assert!(m.set_value("Ypos", Value::Number(1.0)).is_err(), "lathe has no Y axis");
        assert!(m.set_value("sys_cond", Value::Text("x".into())).is_err());
        assert!(m.set_value("Sspeed", Value::Text("fast".into())).is_err());
        m.set_value("Sspeed", Value::Number(1234.5)).unwrap();
        for _ in 0..40 {
            m.tick(0.25);
        }
        let s = m.snapshot();
        let speed = s.items.iter().find(|i| i.id == "Sspeed").unwrap();
        assert_eq!(speed.value, Value::Number(1234.5));
        assert!(speed.pinned);
        m.release("Sspeed");
        m.tick(0.25);
        let speed = m.snapshot().items.into_iter().find(|i| i.id == "Sspeed").unwrap();
        assert_ne!(speed.value, Value::Number(1234.5));
    }

    #[test]
    fn conditions_pause_and_estop() {
        let m = SimModel::new(DeviceTemplate::Vmc3Axis, 1000);
        m.raise_condition("motion_cond", ConditionLevel::Fault, "E42", "Servo overload").unwrap();
        let c = m
            .snapshot()
            .items
            .into_iter()
            .find(|i| i.id == "motion_cond")
            .unwrap()
            .condition
            .unwrap();
        assert_eq!((c.level, c.native_code.as_deref(), c.message.as_deref()), (ConditionLevel::Fault, Some("E42"), Some("Servo overload")));
        m.clear_condition("motion_cond").unwrap();
        assert!(m.raise_condition("motion_cond", ConditionLevel::Normal, "x", "y").is_err());

        let before = m.snapshot().next_sequence;
        m.set_paused(true);
        for _ in 0..50 {
            m.tick(0.25);
        }
        assert_eq!(m.snapshot().next_sequence, before, "paused model records nothing");
        m.set_paused(false);

        m.set_value("estop", Value::Text("TRIGGERED".into())).unwrap();
        m.tick(0.25);
        let exec = m.snapshot().items.into_iter().find(|i| i.id == "exec").unwrap();
        assert_eq!(exec.value, Value::Text("STOPPED".into()));
    }

    #[test]
    fn sample_range_rules_and_restart() {
        let m = SimModel::new(DeviceTemplate::Vmc3Axis, 100);
        for _ in 0..400 {
            m.tick(0.25);
        }
        let s = m.snapshot();
        assert!(s.first_sequence > 1);
        let err = m.render_sample(None, Some(1), None).unwrap_err();
        assert_eq!(err.code, AgentErrorCode::OutOfRange);
        assert!(m.render_sample(None, Some(s.first_sequence), Some(5)).is_ok());
        assert!(m.render_sample(None, Some(s.next_sequence), None).is_ok());
        assert!(m.render_sample(None, Some(s.next_sequence + 1), None).is_err());
        assert!(m.render_sample(None, None, Some(0)).is_err());
        assert_eq!(m.render_current(Some("//[")).err().unwrap().code, AgentErrorCode::InvalidXPath);

        let old = s.instance_id;
        m.restart_agent();
        let s2 = m.snapshot();
        assert_ne!(s2.instance_id, old);
        assert_eq!(s2.first_sequence, 1);
    }
}
