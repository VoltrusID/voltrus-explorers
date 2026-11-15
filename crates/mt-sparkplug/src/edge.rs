//! Sparkplug B edge node with devices, aliases and commands.
//!
//! [`EdgeNode`] is a state machine without I/O: it is fed sampled device
//! values and received commands, and returns the MQTT messages to publish.
//! [`crate::link`] carries those messages over a real broker connection.
//!
//! What it does, per the Sparkplug 3.0 normative statements:
//!
//! - `open_session` bumps and persists `bdSeq`, returns the NDEATH Will
//!   (register it at QoS 1, retain off) and the births: NBIRTH with
//!   `seq = 0`, `bdSeq`, `Node Control/Rebirth` and the static node metrics,
//!   then one DBIRTH per device that has values.
//! - Every metric gets an alias in its BIRTH; DDATA carries aliases only.
//!   Aliases stay stable for a metric name for the life of the process.
//! - DDATA is report by exception with a per-device deadband and a
//!   maximum interval after which a suppressed change is sent anyway.
//! - A device whose tags all fail gets a DDEATH; it is born again (DBIRTH)
//!   when values come back. A change in a device's metric set (new tag,
//!   different datatype) also triggers a DBIRTH.
//! - `seq` runs 0..=255 across NBIRTH, DBIRTH, NDATA, DDATA and DDEATH.
//! - NCMD `Node Control/Rebirth = true` republishes NBIRTH (seq 0, same
//!   bdSeq) and every DBIRTH. DCMD metrics come back as [`DeviceWrite`]s for
//!   the caller to apply (or refuse).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use prost::Message;

use crate::pb::payload::metric;
use crate::pb::{DataType, Payload};
use crate::{Metric, PAYLOAD_UUID, SparkplugError, bdseq};

/// Metric name hosts write to ask for a rebirth.
pub const REBIRTH_METRIC: &str = "Node Control/Rebirth";
/// Metric name of the birth/death sequence number.
pub const BD_SEQ_METRIC: &str = "bdSeq";

/// A typed Sparkplug B scalar.
#[derive(Debug, Clone, PartialEq)]
pub enum SpValue {
    Boolean(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float(f32),
    Double(f64),
    String(String),
    Bytes(Vec<u8>),
}

impl SpValue {
    pub fn data_type(&self) -> DataType {
        match self {
            SpValue::Boolean(_) => DataType::Boolean,
            SpValue::Int8(_) => DataType::Int8,
            SpValue::Int16(_) => DataType::Int16,
            SpValue::Int32(_) => DataType::Int32,
            SpValue::Int64(_) => DataType::Int64,
            SpValue::UInt8(_) => DataType::UInt8,
            SpValue::UInt16(_) => DataType::UInt16,
            SpValue::UInt32(_) => DataType::UInt32,
            SpValue::UInt64(_) => DataType::UInt64,
            SpValue::Float(_) => DataType::Float,
            SpValue::Double(_) => DataType::Double,
            SpValue::String(_) => DataType::String,
            SpValue::Bytes(_) => DataType::Bytes,
        }
    }

    /// Wire value. Int8/16/32 travel in the uint32 `int_value` as two's
    /// complement of the 32-bit value, Int64 in `long_value`.
    pub fn to_wire(&self) -> metric::Value {
        use metric::Value as V;
        match self {
            SpValue::Boolean(v) => V::BooleanValue(*v),
            SpValue::Int8(v) => V::IntValue(i32::from(*v) as u32),
            SpValue::Int16(v) => V::IntValue(i32::from(*v) as u32),
            SpValue::Int32(v) => V::IntValue(*v as u32),
            SpValue::Int64(v) => V::LongValue(*v as u64),
            SpValue::UInt8(v) => V::IntValue(u32::from(*v)),
            SpValue::UInt16(v) => V::IntValue(u32::from(*v)),
            SpValue::UInt32(v) => V::IntValue(*v),
            SpValue::UInt64(v) => V::LongValue(*v),
            SpValue::Float(v) => V::FloatValue(*v),
            SpValue::Double(v) => V::DoubleValue(*v),
            SpValue::String(v) => V::StringValue(v.clone()),
            SpValue::Bytes(v) => V::BytesValue(v.clone()),
        }
    }

    /// Read a wire value back according to `datatype` (used for DCMD).
    /// Values that do not fit the declared type return `None`.
    pub fn from_wire(datatype: u32, value: &metric::Value) -> Option<SpValue> {
        use metric::Value as V;
        let dt = DataType::try_from(datatype as i32).ok()?;
        Some(match (dt, value) {
            (DataType::Boolean, V::BooleanValue(v)) => SpValue::Boolean(*v),
            (DataType::Int8, V::IntValue(v)) => SpValue::Int8(*v as i32 as i8),
            (DataType::Int16, V::IntValue(v)) => SpValue::Int16(*v as i32 as i16),
            (DataType::Int32, V::IntValue(v)) => SpValue::Int32(*v as i32),
            (DataType::Int64, V::LongValue(v)) => SpValue::Int64(*v as i64),
            (DataType::UInt8, V::IntValue(v)) => SpValue::UInt8(u8::try_from(*v).ok()?),
            (DataType::UInt16, V::IntValue(v)) => SpValue::UInt16(u16::try_from(*v).ok()?),
            (DataType::UInt32, V::IntValue(v)) => SpValue::UInt32(*v),
            (DataType::UInt32, V::LongValue(v)) => SpValue::UInt32(u32::try_from(*v).ok()?),
            (DataType::UInt64, V::LongValue(v)) => SpValue::UInt64(*v),
            (DataType::Float, V::FloatValue(v)) => SpValue::Float(*v),
            (DataType::Double, V::DoubleValue(v)) => SpValue::Double(*v),
            (DataType::String | DataType::Text, V::StringValue(v)) => SpValue::String(v.clone()),
            (DataType::Bytes, V::BytesValue(v)) => SpValue::Bytes(v.clone()),
            _ => return None,
        })
    }

    /// Equality that treats NaN as equal to NaN of the same type, so a
    /// value stuck at NaN is not republished on every sample.
    fn same(&self, other: &SpValue) -> bool {
        match (self, other) {
            (SpValue::Float(a), SpValue::Float(b)) => a == b || (a.is_nan() && b.is_nan()),
            (SpValue::Double(a), SpValue::Double(b)) => a == b || (a.is_nan() && b.is_nan()),
            _ => self == other,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        Some(match self {
            SpValue::Int8(v) => f64::from(*v),
            SpValue::Int16(v) => f64::from(*v),
            SpValue::Int32(v) => f64::from(*v),
            SpValue::Int64(v) => *v as f64,
            SpValue::UInt8(v) => f64::from(*v),
            SpValue::UInt16(v) => f64::from(*v),
            SpValue::UInt32(v) => f64::from(*v),
            SpValue::UInt64(v) => *v as f64,
            SpValue::Float(v) => f64::from(*v),
            SpValue::Double(v) => *v,
            _ => return None,
        })
    }
}

/// One MQTT message to publish (QoS 0, retain off, like every non-STATE
/// Sparkplug message).
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// A DCMD metric the host asked to write.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceWrite {
    pub device_id: String,
    pub metric: String,
    pub value: SpValue,
}

/// Result of [`EdgeNode::handle_command`].
#[derive(Debug, Default, PartialEq)]
pub struct CommandOutcome {
    pub publish: Vec<Outgoing>,
    pub writes: Vec<DeviceWrite>,
    /// Metrics in the command that could not be used, with the reason.
    pub rejected: Vec<String>,
}

pub struct EdgeNodeConfig {
    pub group_id: String,
    pub edge_node_id: String,
    /// File keeping `bdSeq` across restarts.
    pub bd_seq_path: PathBuf,
    /// Static node metrics published in NBIRTH (for example
    /// `Properties/Software Version`).
    pub node_metrics: Vec<(String, SpValue)>,
    /// A change suppressed by a deadband is still published after this long.
    pub max_interval_ms: u64,
}

struct MetricState {
    name: String,
    alias: u64,
    datatype: DataType,
    value: Option<SpValue>,
    last_sent: Option<SpValue>,
    last_sent_ms: u64,
}

struct Device {
    deadband: f64,
    /// Born in the current session and not dead since.
    born: bool,
    metrics: Vec<MetricState>,
}

pub struct EdgeNode {
    cfg: EdgeNodeConfig,
    bd_seq: u8,
    seq: u8,
    session_open: bool,
    devices: BTreeMap<String, Device>,
    aliases: HashMap<(String, String), u64>,
    next_alias: u64,
}

fn now_metric(name: &str, alias: Option<u64>, value: &SpValue, ts: u64) -> Metric {
    Metric {
        name: Some(name.to_string()),
        alias,
        timestamp: Some(ts),
        datatype: Some(value.data_type() as u32),
        value: Some(value.to_wire()),
        ..Default::default()
    }
}

fn bd_seq_metric(bd_seq: u8, ts: u64) -> Metric {
    now_metric(BD_SEQ_METRIC, None, &SpValue::Int64(i64::from(bd_seq)), ts)
}

impl EdgeNode {
    pub fn new(cfg: EdgeNodeConfig) -> Self {
        Self {
            cfg,
            bd_seq: 0,
            seq: 0,
            session_open: false,
            devices: BTreeMap::new(),
            aliases: HashMap::new(),
            // Alias 0 is the rebirth control metric.
            next_alias: 1,
        }
    }

    pub fn group_id(&self) -> &str {
        &self.cfg.group_id
    }
    pub fn edge_node_id(&self) -> &str {
        &self.cfg.edge_node_id
    }
    pub fn bd_seq(&self) -> u8 {
        self.bd_seq
    }
    /// `seq` of the next message.
    pub fn seq(&self) -> u8 {
        self.seq
    }
    pub fn session_open(&self) -> bool {
        self.session_open
    }

    /// `(device_id, born, metric count)` for every known device.
    pub fn devices(&self) -> Vec<(String, bool, usize)> {
        self.devices
            .iter()
            .map(|(id, d)| (id.clone(), d.born, d.metrics.len()))
            .collect()
    }

    /// Topic for this edge node: `spBv1.0/<group>/<verb>/<edge>[/<device>]`.
    pub fn topic(&self, verb: &str, device: Option<&str>) -> String {
        match device {
            Some(d) => format!(
                "spBv1.0/{}/{verb}/{}/{d}",
                self.cfg.group_id, self.cfg.edge_node_id
            ),
            None => format!(
                "spBv1.0/{}/{verb}/{}",
                self.cfg.group_id, self.cfg.edge_node_id
            ),
        }
    }

    /// Filters to subscribe to for commands.
    pub fn command_filters(&self) -> [String; 2] {
        [self.topic("NCMD", None), self.topic("DCMD", Some("+"))]
    }

    fn next_seq(&mut self) -> u64 {
        let s = self.seq;
        self.seq = s.wrapping_add(1);
        u64::from(s)
    }

    fn alias_for(&mut self, device: &str, name: &str) -> u64 {
        if let Some(a) = self.aliases.get(&(device.to_string(), name.to_string())) {
            return *a;
        }
        let a = self.next_alias;
        self.next_alias += 1;
        self.aliases
            .insert((device.to_string(), name.to_string()), a);
        a
    }

    /// Start a session. Returns the NDEATH to register as the MQTT Will
    /// (QoS 1, retain off) and the births to publish right after CONNACK.
    pub fn open_session(
        &mut self,
        now_ms: u64,
    ) -> Result<(Outgoing, Vec<Outgoing>), SparkplugError> {
        let will = self.prepare_session(now_ms)?;
        Ok((will, self.begin_session(now_ms)))
    }

    /// First half of [`open_session`](Self::open_session): bump and persist
    /// bdSeq and return the NDEATH Will to connect with. Use with
    /// [`begin_session`](Self::begin_session) when births must wait (for
    /// example for the primary host's STATE).
    pub fn prepare_session(&mut self, now_ms: u64) -> Result<Outgoing, SparkplugError> {
        let bd_seq = bdseq::load(&self.cfg.bd_seq_path)?.wrapping_add(1);
        bdseq::store(&self.cfg.bd_seq_path, bd_seq)?;
        self.bd_seq = bd_seq;
        self.session_open = false;
        Ok(self.ndeath(now_ms))
    }

    /// Second half: the session is live, returns NBIRTH and the DBIRTHs.
    pub fn begin_session(&mut self, now_ms: u64) -> Vec<Outgoing> {
        self.session_open = true;
        self.births(now_ms)
    }

    /// The session ended (connection lost or closed). Births happen again
    /// on the next [`open_session`](Self::open_session).
    pub fn close_session(&mut self) {
        self.session_open = false;
        for d in self.devices.values_mut() {
            d.born = false;
        }
    }

    /// NDEATH for a graceful shutdown. Publish it, then disconnect cleanly.
    pub fn death(&mut self, now_ms: u64) -> Outgoing {
        let msg = self.ndeath(now_ms);
        self.close_session();
        msg
    }

    fn ndeath(&self, now_ms: u64) -> Outgoing {
        let p = Payload {
            timestamp: Some(now_ms),
            metrics: vec![bd_seq_metric(self.bd_seq, now_ms)],
            seq: None,
            uuid: None,
            body: None,
        };
        Outgoing {
            topic: self.topic("NDEATH", None),
            payload: p.encode_to_vec(),
        }
    }

    /// NBIRTH plus a DBIRTH for every device with values. Resets seq to 0.
    fn births(&mut self, now_ms: u64) -> Vec<Outgoing> {
        self.seq = 0;
        let mut metrics = vec![
            bd_seq_metric(self.bd_seq, now_ms),
            now_metric(REBIRTH_METRIC, Some(0), &SpValue::Boolean(false), now_ms),
        ];
        let node_metrics = self.cfg.node_metrics.clone();
        for (name, value) in &node_metrics {
            let alias = self.alias_for("", name);
            metrics.push(now_metric(name, Some(alias), value, now_ms));
        }
        let seq = self.next_seq();
        let nbirth = Payload {
            timestamp: Some(now_ms),
            metrics,
            seq: Some(seq),
            uuid: Some(PAYLOAD_UUID.into()),
            body: None,
        };
        let mut out = vec![Outgoing {
            topic: self.topic("NBIRTH", None),
            payload: nbirth.encode_to_vec(),
        }];
        let ids: Vec<String> = self.devices.keys().cloned().collect();
        for id in ids {
            if let Some(m) = self.dbirth(&id, now_ms) {
                out.push(m);
            }
        }
        out
    }

    /// DBIRTH with every metric that has a value; `None` when it has none.
    fn dbirth(&mut self, id: &str, now_ms: u64) -> Option<Outgoing> {
        let has_values = self
            .devices
            .get(id)?
            .metrics
            .iter()
            .any(|m| m.value.is_some());
        if !has_values {
            return None;
        }
        let seq = self.next_seq();
        let dev = self.devices.get_mut(id)?;
        let mut metrics = Vec::new();
        for m in dev.metrics.iter_mut() {
            if let Some(v) = &m.value {
                metrics.push(now_metric(&m.name, Some(m.alias), v, now_ms));
                m.last_sent = Some(v.clone());
                m.last_sent_ms = now_ms;
            } else {
                m.last_sent = None;
            }
        }
        dev.born = true;
        let p = Payload {
            timestamp: Some(now_ms),
            metrics,
            seq: Some(seq),
            uuid: Some(PAYLOAD_UUID.into()),
            body: None,
        };
        Some(Outgoing {
            topic: self.topic("DBIRTH", Some(id)),
            payload: p.encode_to_vec(),
        })
    }

    /// Feed one sample of a device's tags. `None` marks a tag that failed
    /// this time. Returns what to publish now (nothing while no session is
    /// open: the values are kept for the next birth).
    pub fn update_device(
        &mut self,
        id: &str,
        deadband: f64,
        values: Vec<(String, Option<SpValue>)>,
        now_ms: u64,
    ) -> Vec<Outgoing> {
        // Make sure every name has an alias before borrowing the device.
        let aliases: Vec<u64> = values.iter().map(|(n, _)| self.alias_for(id, n)).collect();
        let dev = self.devices.entry(id.to_string()).or_insert(Device {
            deadband,
            born: false,
            metrics: Vec::new(),
        });
        dev.deadband = deadband;

        // The metric set changed (tags added/removed, or a datatype changed):
        // hosts need a new DBIRTH to learn it.
        let mut shape_changed = dev.metrics.len() != values.len();
        let mut next: Vec<MetricState> = Vec::with_capacity(values.len());
        for ((name, value), alias) in values.into_iter().zip(aliases) {
            let prev = dev
                .metrics
                .iter()
                .position(|m| m.name == name)
                .map(|i| dev.metrics.swap_remove(i));
            let datatype = value
                .as_ref()
                .map(SpValue::data_type)
                .or(prev.as_ref().map(|p| p.datatype))
                .unwrap_or(DataType::Unknown);
            match &prev {
                None => shape_changed = true,
                Some(p) if p.datatype != datatype && value.is_some() => shape_changed = true,
                _ => {}
            }
            let (last_sent, last_sent_ms) = prev
                .map(|p| (p.last_sent, p.last_sent_ms))
                .unwrap_or((None, 0));
            next.push(MetricState {
                name,
                alias,
                datatype,
                value,
                last_sent,
                last_sent_ms,
            });
        }
        dev.metrics = next;

        if !self.session_open {
            return Vec::new();
        }
        let dev = self.devices.get(id).expect("inserted above");
        let all_failed = dev.metrics.iter().all(|m| m.value.is_none());
        if all_failed {
            return self.device_offline(id, now_ms).into_iter().collect();
        }
        if !dev.born || shape_changed {
            return self.dbirth(id, now_ms).into_iter().collect();
        }
        self.ddata(id, now_ms).into_iter().collect()
    }

    fn ddata(&mut self, id: &str, now_ms: u64) -> Option<Outgoing> {
        let max_interval = self.cfg.max_interval_ms;
        let dev = self.devices.get_mut(id)?;
        let deadband = dev.deadband;
        let mut metrics = Vec::new();
        for m in dev.metrics.iter_mut() {
            let Some(v) = &m.value else { continue };
            let send = match &m.last_sent {
                None => true,
                Some(last) if last.same(v) => false,
                Some(last) => match (v.as_f64(), last.as_f64()) {
                    (Some(a), Some(b)) if (a - b).abs() < deadband => {
                        max_interval > 0 && now_ms.saturating_sub(m.last_sent_ms) >= max_interval
                    }
                    _ => true,
                },
            };
            if send {
                metrics.push(Metric {
                    alias: Some(m.alias),
                    timestamp: Some(now_ms),
                    datatype: Some(m.datatype as u32),
                    value: Some(v.to_wire()),
                    ..Default::default()
                });
                m.last_sent = Some(v.clone());
                m.last_sent_ms = now_ms;
            }
        }
        if metrics.is_empty() {
            return None;
        }
        let seq = self.next_seq();
        let p = Payload {
            timestamp: Some(now_ms),
            metrics,
            seq: Some(seq),
            uuid: Some(PAYLOAD_UUID.into()),
            body: None,
        };
        Some(Outgoing {
            topic: self.topic("DDATA", Some(id)),
            payload: p.encode_to_vec(),
        })
    }

    /// The device stopped answering (or was removed): DDEATH if it was born.
    pub fn device_offline(&mut self, id: &str, now_ms: u64) -> Option<Outgoing> {
        let born = self.devices.get(id).map(|d| d.born).unwrap_or(false);
        if !born || !self.session_open {
            return None;
        }
        if let Some(d) = self.devices.get_mut(id) {
            d.born = false;
        }
        let seq = self.next_seq();
        let p = Payload {
            timestamp: Some(now_ms),
            metrics: Vec::new(),
            seq: Some(seq),
            uuid: None,
            body: None,
        };
        Some(Outgoing {
            topic: self.topic("DDEATH", Some(id)),
            payload: p.encode_to_vec(),
        })
    }

    /// Forget a device, publishing its DDEATH if it was born.
    pub fn remove_device(&mut self, id: &str, now_ms: u64) -> Option<Outgoing> {
        let death = self.device_offline(id, now_ms);
        self.devices.remove(id);
        death
    }

    /// Handle a received NCMD or DCMD for this edge node.
    pub fn handle_command(&mut self, topic: &str, payload: &[u8], now_ms: u64) -> CommandOutcome {
        let mut out = CommandOutcome::default();
        let Ok(p) = Payload::decode(payload) else {
            out.rejected
                .push(format!("{topic}: not a Sparkplug B payload"));
            return out;
        };
        let ncmd = self.topic("NCMD", None);
        let dcmd_prefix = format!("{}/", self.topic("DCMD", None));
        if topic == ncmd {
            for m in &p.metrics {
                let is_rebirth = m.name.as_deref() == Some(REBIRTH_METRIC)
                    || (m.name.is_none() && m.alias == Some(0));
                if is_rebirth {
                    if m.value == Some(metric::Value::BooleanValue(true)) && self.session_open {
                        out.publish = self.births(now_ms);
                    }
                } else {
                    out.rejected.push(format!(
                        "NCMD metric {} is not writable",
                        m.name.as_deref().unwrap_or("?")
                    ));
                }
            }
        } else if let Some(device) = topic.strip_prefix(&dcmd_prefix) {
            let Some(dev) = self.devices.get(device) else {
                out.rejected
                    .push(format!("DCMD for unknown device {device}"));
                return out;
            };
            for m in &p.metrics {
                let state = match (&m.name, m.alias) {
                    (Some(n), _) => dev.metrics.iter().find(|s| &s.name == n),
                    (None, Some(a)) => dev.metrics.iter().find(|s| s.alias == a),
                    _ => None,
                };
                let Some(state) = state else {
                    out.rejected.push(format!(
                        "DCMD {device}: unknown metric {}",
                        m.name
                            .clone()
                            .unwrap_or_else(|| format!("alias {}", m.alias.unwrap_or(0)))
                    ));
                    continue;
                };
                let datatype = m.datatype.unwrap_or(state.datatype as u32);
                match m
                    .value
                    .as_ref()
                    .and_then(|v| SpValue::from_wire(datatype, v))
                {
                    Some(value) => out.writes.push(DeviceWrite {
                        device_id: device.to_string(),
                        metric: state.name.clone(),
                        value,
                    }),
                    None => out.rejected.push(format!(
                        "DCMD {device}/{}: value does not match datatype",
                        state.name
                    )),
                }
            }
        }
        out
    }
}
