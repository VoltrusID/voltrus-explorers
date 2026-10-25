//! An MQTT broker on this computer with a publisher that plays a device
//! template, the same templates and drift as the Modbus simulator.
//!
//! The device publishes one retained number per value at `<prefix>/<value>`
//! and a retained JSON nameplate at `<prefix>/status`; with Sparkplug on, it
//! is also edge node `Voltrus/Simulator` with the template as its device
//! (births, data by alias, rebirth on request, death on stop). A number
//! published to `<prefix>/<value>` or `<prefix>/<value>/set`, by any client
//! or from the app, is taken into the simulation and held, as a Modbus
//! master's write is.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use mt_modbus::{SimDeviceProfile, Simulation, tick_values};
use mt_mqtt::sparkplug::pb;
use prost::Message as _;
use rumqttc::mqttbytes::QoS;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, LogLevel, MqttBrokerSpec, PointInfo, Quality, Reading, SPARKPLUG_GROUP,
    SPARKPLUG_NAMESPACE, SessionState, Stats, TrafficFrame, ValueFrame,
};

use super::broker::{Broker, BrokerEvent};
use super::{format_number, listen_ip, preview, qos_name, wire_qos};
use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::templates;

/// How often the app sees the simulated values.
pub(crate) const REPORT_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
/// Sparkplug edge node id of the simulator.
const NODE: &str = "Simulator";
const REBIRTH: &str = "Node Control/Rebirth";

pub(crate) async fn run(
    spec: MqttBrokerSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let profile = match spec.template.as_deref() {
        Some(id) => match templates::find_modbus(id) {
            Some(profile) => Some(profile),
            None => {
                out.state(SessionState::Closed {
                    error: Some(format!("Unknown device template {id}")),
                });
                return;
            }
        },
        None => None,
    };
    let Some(ip) = listen_ip(&spec.bind) else {
        let message = format!("Cannot listen on {}: not an IP address", spec.bind);
        out.log(LogLevel::Error, message.clone());
        out.state(SessionState::Closed {
            error: Some(message),
        });
        return;
    };
    let address = SocketAddr::new(ip, spec.port);
    let mut broker = match listen(address, &mut control, out).await {
        Ok(Some(broker)) => broker,
        Ok(None) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
        Err(error) => {
            let message = format!("Cannot listen on {address}: {error}");
            out.log(LogLevel::Error, message.clone());
            out.state(SessionState::Closed {
                error: Some(message),
            });
            return;
        }
    };
    let address = broker.local_addr().unwrap_or(address);
    out.state(SessionState::Listening {
        address: address.to_string(),
    });

    let mut device = profile.map(|profile| Device::new(profile.clone(), &spec));
    out.log(
        LogLevel::Info,
        match &device {
            Some(device) => format!(
                "MQTT broker on {address}, publishing {} {} under {}/#{}",
                device.simulation.profile.manufacturer,
                device.simulation.profile.model,
                device.prefix,
                if device.sparkplug.is_some() {
                    format!(" and as Sparkplug B node {SPARKPLUG_GROUP}/{NODE}")
                } else {
                    String::new()
                }
            ),
            None => format!("MQTT broker on {address}"),
        },
    );
    let mut traffic = Vec::new();
    match &mut device {
        Some(device) => {
            out.points(device.points());
            device.publish(&mut broker, true, &mut traffic);
            out.values(device.frame());
        }
        None => out.points(Vec::new()),
    }

    let mut simulate = tokio::time::interval(Duration::from_millis(
        spec.publish_ms.max(MqttBrokerSpec::MIN_PUBLISH_MS),
    ));
    simulate.set_missed_tick_behavior(MissedTickBehavior::Delay);
    simulate.reset();
    let mut report = tokio::time::interval(REPORT_TICK);
    report.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stats = Stats::default();

    loop {
        tokio::select! {
            event = broker.next() => {
                stats.frames += 1;
                on_broker_event(event, &mut broker, device.as_mut(), &mut traffic, out);
            }
            _ = simulate.tick() => {
                if let Some(device) = &mut device {
                    tick_values(&mut device.simulation);
                    device.publish(&mut broker, false, &mut traffic);
                    stats.polls += 1;
                }
            }
            _ = report.tick() => {
                if let Some(device) = &device {
                    out.values(device.frame());
                }
                out.traffic(std::mem::take(&mut traffic));
                out.stats(stats);
            }
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => match &mut device {
                    Some(device) => {
                        let result = device.set(point, value, &mut broker, &mut traffic);
                        let held = result.is_ok();
                        out.written(point, result);
                        if held {
                            out.locked(point, true);
                            out.values(device.frame());
                        }
                    }
                    None => out.written(point, Err(format!("No point {point}"))),
                },
                Some(Control::Lock { point, locked }) => {
                    if let Some(device) = &mut device
                        && device.lock(point, locked)
                    {
                        out.locked(point, locked);
                    }
                }
                Some(Control::Publish { topic, payload, qos, retain }) => {
                    if !vt_protocol::valid_topic(&topic) {
                        out.published(topic.clone(), Err(format!("“{topic}” is not a topic to publish to")));
                        continue;
                    }
                    traffic.push(frame(
                        Direction::Tx,
                        format!("PUBLISH {topic}"),
                        format!(
                            "{}{} · {}",
                            qos.name(),
                            if retain { " · retained" } else { "" },
                            preview(payload.as_bytes())
                        ),
                        u32::try_from(payload.len()).unwrap_or(u32::MAX),
                    ));
                    broker.publish(&topic, payload.clone().into_bytes(), wire_qos(qos), retain);
                    if let Some(device) = &mut device {
                        device.command("The app", &topic, payload.as_bytes(), &mut broker, &mut traffic, out);
                    }
                    out.published(topic, Ok(()));
                }
                Some(_) => {}
            },
        }
    }
    if let Some(device) = &mut device {
        device.death(&mut broker, &mut traffic);
    }
    let clients = broker.client_count();
    broker.close().await;
    if clients > 0 {
        out.log(
            LogLevel::Info,
            format!(
                "Stopped; {clients} client{} disconnected",
                if clients == 1 { "" } else { "s" }
            ),
        );
    }
    out.traffic(traffic);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Bind the broker. A port still held by a session that is closing is
/// retried for [`BIND_RETRY`]. `Ok(None)` when the app closed the session
/// meanwhile.
async fn listen(
    address: SocketAddr,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> io::Result<Option<Broker>> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        match Broker::bind(address).await {
            Ok(broker) => return Ok(Some(broker)),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse && Instant::now() < deadline => {
                tokio::select! {
                    () = tokio::time::sleep(BIND_RETRY_STEP) => {}
                    () = until_closed(control, out) => return Ok(None),
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Wait for the app to close the session before it listens, refusing writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("The broker is not listening yet".into()));
            }
            Some(Control::Publish { topic, .. }) => {
                out.published(topic, Err("The broker is not listening yet".into()));
            }
            Some(_) => {}
        }
    }
}

fn on_broker_event(
    event: BrokerEvent,
    broker: &mut Broker,
    device: Option<&mut Device>,
    traffic: &mut Vec<TrafficFrame>,
    out: &Emitter,
) {
    match event {
        BrokerEvent::Connected {
            client,
            address,
            username,
            keep_alive,
        } => {
            traffic.push(frame(
                Direction::Rx,
                format!("CONNECT {client}"),
                format!("{address} · keep-alive {keep_alive} s"),
                0,
            ));
            out.log(
                LogLevel::Info,
                match username {
                    Some(username) => format!("{client} connected from {address} as {username}"),
                    None => format!("{client} connected from {address}"),
                },
            );
        }
        BrokerEvent::Disconnected { client, reason } => {
            traffic.push(frame(
                Direction::Rx,
                format!("DISCONNECT {client}"),
                reason.clone(),
                0,
            ));
            out.log(LogLevel::Info, format!("{client} {reason}"));
        }
        BrokerEvent::Subscribed { client, filters } => {
            let list = filters
                .iter()
                .map(|(filter, qos)| format!("{filter} ({})", qos_name(*qos)))
                .collect::<Vec<_>>()
                .join(", ");
            traffic.push(frame(
                Direction::Rx,
                format!(
                    "SUBSCRIBE {}",
                    filters
                        .iter()
                        .map(|(filter, _)| filter.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                format!("{client} · {list}"),
                0,
            ));
            out.log(LogLevel::Info, format!("{client} subscribed to {list}"));
        }
        BrokerEvent::Unsubscribed { client, filters } => {
            traffic.push(frame(
                Direction::Rx,
                format!("UNSUBSCRIBE {}", filters.join(" ")),
                client,
                0,
            ));
        }
        BrokerEvent::Published { client, publish } => {
            traffic.push(frame(
                Direction::Rx,
                format!("PUBLISH {}", publish.topic),
                format!(
                    "{client} · {}{} · {}",
                    qos_name(publish.qos),
                    if publish.retain { " · retained" } else { "" },
                    preview(&publish.payload)
                ),
                u32::try_from(publish.payload.len()).unwrap_or(u32::MAX),
            ));
            if let Some(device) = device {
                device.command(
                    &client,
                    &publish.topic,
                    &publish.payload,
                    broker,
                    traffic,
                    out,
                );
            }
        }
    }
}

fn frame(direction: Direction, summary: String, detail: String, bytes: u32) -> TrafficFrame {
    TrafficFrame {
        at_ms: now_ms(),
        direction,
        summary,
        detail,
        bytes,
    }
}

/// A Sparkplug edge node's sequence numbers.
struct Node {
    /// Message sequence, 0 at NBIRTH, wrapping after 255.
    seq: u64,
    /// Birth/death sequence, paired between an NBIRTH and its NDEATH.
    bd_seq: u64,
}

impl Node {
    fn next_seq(&mut self) -> u64 {
        let seq = self.seq;
        self.seq = (self.seq + 1) % 256;
        seq
    }
}

/// The simulated device and where it publishes.
struct Device {
    simulation: Simulation,
    prefix: String,
    /// `<prefix>/<value>` per template value.
    topics: Vec<String>,
    started: Instant,
    sparkplug: Option<Node>,
}

impl Device {
    fn new(profile: SimDeviceProfile, spec: &MqttBrokerSpec) -> Self {
        let prefix = spec.prefix.trim_matches('/').to_owned();
        let topics = profile
            .registers
            .iter()
            .map(|register| format!("{prefix}/{}", register.name.to_lowercase()))
            .collect();
        Self {
            simulation: Simulation::new(profile),
            prefix,
            topics,
            started: Instant::now(),
            sparkplug: spec.sparkplug.then_some(Node { seq: 0, bd_seq: 0 }),
        }
    }

    fn points(&self) -> Vec<PointInfo> {
        self.topics
            .iter()
            .zip(&self.simulation.profile.registers)
            .enumerate()
            .map(|(index, (topic, register))| PointInfo {
                id: u32::try_from(index).unwrap_or(u32::MAX),
                name: topic.clone(),
                address: qos_name(QoS::AtMostOnce).into(),
                data_type: "Number".into(),
                unit: register.unit.clone(),
                writable: true,
                boolean: false,
            })
            .collect()
    }

    fn frame(&self) -> ValueFrame {
        ValueFrame {
            at_ms: now_ms(),
            latency_us: 0,
            readings: self
                .simulation
                .values
                .iter()
                .map(|value| Reading {
                    value: *value,
                    raw: [0; 4],
                    raw_len: 0,
                    quality: Quality::Good,
                })
                .collect(),
        }
    }

    /// Publish every value, the nameplate and, with Sparkplug, its data (or
    /// its births when `birth`).
    fn publish(&mut self, broker: &mut Broker, birth: bool, traffic: &mut Vec<TrafficFrame>) {
        for (topic, value) in self.topics.iter().zip(&self.simulation.values) {
            broker.publish(
                topic,
                format_number(*value).into_bytes(),
                QoS::AtMostOnce,
                true,
            );
        }
        let profile = &self.simulation.profile;
        let status = json!({
            "online": true,
            "manufacturer": profile.manufacturer,
            "model": profile.model,
            "uptime_s": self.started.elapsed().as_secs(),
        });
        broker.publish(
            &format!("{}/status", self.prefix),
            status.to_string().into_bytes(),
            QoS::AtMostOnce,
            true,
        );
        traffic.push(frame(
            Direction::Tx,
            format!("PUBLISH {} topics", self.topics.len() + 1),
            format!("{}/# · QoS 0 · retained", self.prefix),
            0,
        ));
        if self.sparkplug.is_some() {
            if birth {
                self.births(broker, traffic);
            } else {
                self.data(broker, traffic);
            }
        }
    }

    fn device_id(&self) -> &str {
        &self.simulation.profile.id
    }

    fn births(&mut self, broker: &mut Broker, traffic: &mut Vec<TrafficFrame>) {
        let metrics: Vec<pb::Metric> = self
            .simulation
            .profile
            .registers
            .iter()
            .zip(&self.simulation.values)
            .enumerate()
            .map(|(index, (register, value))| pb::Metric {
                name: Some(register.name.replace('_', " ")),
                alias: Some(alias(index)),
                datatype: Some(DOUBLE),
                value: Some(pb::metric::Value::DoubleValue(*value)),
                properties: (!register.unit.is_empty()).then(|| pb::PropertySet {
                    keys: vec!["engUnit".into()],
                    values: vec![pb::PropertyValue {
                        r#type: Some(STRING),
                        is_null: None,
                        value: Some(pb::property_value::Value::StringValue(
                            register.unit.clone(),
                        )),
                    }],
                }),
                ..pb::Metric::default()
            })
            .collect();
        let device = self.device_id().to_owned();
        let Some(node) = &mut self.sparkplug else {
            return;
        };
        node.seq = 0;
        let node_birth = pb::Payload {
            timestamp: Some(now_ms()),
            metrics: vec![
                pb::Metric {
                    name: Some("bdSeq".into()),
                    datatype: Some(INT64),
                    value: Some(pb::metric::Value::LongValue(node.bd_seq)),
                    ..pb::Metric::default()
                },
                pb::Metric {
                    name: Some(REBIRTH.into()),
                    datatype: Some(BOOLEAN),
                    value: Some(pb::metric::Value::BooleanValue(false)),
                    ..pb::Metric::default()
                },
            ],
            seq: Some(node.next_seq()),
            ..pb::Payload::default()
        };
        let device_birth = pb::Payload {
            timestamp: Some(now_ms()),
            metrics,
            seq: Some(node.next_seq()),
            ..pb::Payload::default()
        };
        sparkplug(broker, traffic, "NBIRTH", None, &node_birth);
        sparkplug(broker, traffic, "DBIRTH", Some(&device), &device_birth);
    }

    fn data(&mut self, broker: &mut Broker, traffic: &mut Vec<TrafficFrame>) {
        let metrics: Vec<pb::Metric> = self
            .simulation
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| pb::Metric {
                alias: Some(alias(index)),
                datatype: Some(DOUBLE),
                value: Some(pb::metric::Value::DoubleValue(*value)),
                ..pb::Metric::default()
            })
            .collect();
        let device = self.device_id().to_owned();
        let Some(node) = &mut self.sparkplug else {
            return;
        };
        let payload = pb::Payload {
            timestamp: Some(now_ms()),
            metrics,
            seq: Some(node.next_seq()),
            ..pb::Payload::default()
        };
        sparkplug(broker, traffic, "DDATA", Some(&device), &payload);
    }

    /// The node goes offline: what its MQTT will would say.
    fn death(&mut self, broker: &mut Broker, traffic: &mut Vec<TrafficFrame>) {
        let Some(node) = &mut self.sparkplug else {
            return;
        };
        let payload = pb::Payload {
            timestamp: Some(now_ms()),
            metrics: vec![pb::Metric {
                name: Some("bdSeq".into()),
                datatype: Some(INT64),
                value: Some(pb::metric::Value::LongValue(node.bd_seq)),
                ..pb::Metric::default()
            }],
            ..pb::Payload::default()
        };
        node.bd_seq += 1;
        sparkplug(broker, traffic, "NDEATH", None, &payload);
    }

    /// Take a value a client (or the app) published to one of the device's
    /// topics, or answer a Sparkplug rebirth request.
    fn command(
        &mut self,
        who: &str,
        topic: &str,
        payload: &[u8],
        broker: &mut Broker,
        traffic: &mut Vec<TrafficFrame>,
        out: &Emitter,
    ) {
        let commands = format!("{SPARKPLUG_NAMESPACE}/{SPARKPLUG_GROUP}/NCMD/{NODE}");
        if self.sparkplug.is_some() && topic == commands.as_str() {
            if rebirth_requested(payload) {
                out.log(
                    LogLevel::Info,
                    format!("{who} asked for a Sparkplug rebirth"),
                );
                self.births(broker, traffic);
            }
            return;
        }
        let target = topic.strip_suffix("/set").unwrap_or(topic);
        let Some(index) = self.topics.iter().position(|candidate| candidate == target) else {
            return;
        };
        let Some(register) = self.simulation.profile.registers.get(index) else {
            return;
        };
        let text = String::from_utf8_lossy(payload);
        let name = register.name.replace('_', " ");
        let value = match text.trim().parse::<f64>() {
            Ok(value) if value.is_finite() => value,
            // An empty payload clears the retained value; the next tick
            // publishes it again.
            _ if text.trim().is_empty() => return,
            _ => {
                out.log(
                    LogLevel::Warn,
                    format!("{who} published “{}” to {topic}: not a number", text.trim()),
                );
                return;
            }
        };
        let unit = register.unit.clone();
        let point = u32::try_from(index).unwrap_or(u32::MAX);
        match self.set(point, value, broker, traffic) {
            Ok(()) => {
                out.log(
                    LogLevel::Info,
                    format!(
                        "{who} set {name} to {} {unit}; holding it",
                        format_number(value)
                    ),
                );
                out.locked(point, true);
                out.values(self.frame());
            }
            Err(error) => out.log(LogLevel::Warn, format!("{who}: {error}")),
        }
    }

    /// Set a value and hold it, publishing it at once.
    fn set(
        &mut self,
        point: u32,
        value: f64,
        broker: &mut Broker,
        traffic: &mut Vec<TrafficFrame>,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("{value} is not a number"));
        }
        let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
        let register = self
            .simulation
            .profile
            .registers
            .get(index)
            .ok_or_else(|| format!("No point {point}"))?;
        if !(register.min..=register.max).contains(&value) {
            return Err(format!(
                "{} takes {}..{} {}",
                register.name.replace('_', " "),
                register.min,
                register.max,
                register.unit
            ));
        }
        self.simulation.values[index] = value;
        // Hold the value: otherwise the next tick drifts it away.
        self.simulation.locks.set(index, true);
        if let Some(topic) = self.topics.get(index) {
            broker.publish(
                topic,
                format_number(value).into_bytes(),
                QoS::AtMostOnce,
                true,
            );
            traffic.push(frame(
                Direction::Tx,
                format!("PUBLISH {topic}"),
                format!("QoS 0 · retained · {}", format_number(value)),
                0,
            ));
        }
        Ok(())
    }

    /// Hold or release a value. Returns whether `point` exists.
    fn lock(&mut self, point: u32, locked: bool) -> bool {
        match usize::try_from(point) {
            Ok(index) if index < self.simulation.profile.registers.len() => {
                self.simulation.locks.set(index, locked);
                true
            }
            _ => false,
        }
    }
}

/// Sparkplug B datatypes (Tahu `DataType`).
const INT64: u32 = 4;
const DOUBLE: u32 = 10;
const BOOLEAN: u32 = 11;
const STRING: u32 = 12;

/// Metric aliases start at 1.
fn alias(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX - 1) + 1
}

fn sparkplug(
    broker: &mut Broker,
    traffic: &mut Vec<TrafficFrame>,
    kind: &str,
    device: Option<&str>,
    payload: &pb::Payload,
) {
    let mut topic = format!("{SPARKPLUG_NAMESPACE}/{SPARKPLUG_GROUP}/{kind}/{NODE}");
    if let Some(device) = device {
        topic.push('/');
        topic.push_str(device);
    }
    let bytes = payload.encode_to_vec();
    traffic.push(frame(
        Direction::Tx,
        format!("PUBLISH {topic}"),
        format!(
            "QoS 0 · {kind} · {} metric{}",
            payload.metrics.len(),
            if payload.metrics.len() == 1 { "" } else { "s" }
        ),
        u32::try_from(bytes.len()).unwrap_or(u32::MAX),
    ));
    broker.publish(&topic, bytes, QoS::AtMostOnce, false);
}

/// Whether an NCMD payload sets `Node Control/Rebirth` to true.
fn rebirth_requested(payload: &[u8]) -> bool {
    pb::Payload::decode(payload).is_ok_and(|command| {
        command.metrics.iter().any(|metric| {
            metric.name.as_deref() == Some(REBIRTH)
                && metric.value == Some(pb::metric::Value::BooleanValue(true))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_true_rebirth_metric_asks_for_births() {
        let command = |value: bool| {
            pb::Payload {
                metrics: vec![pb::Metric {
                    name: Some(REBIRTH.into()),
                    datatype: Some(BOOLEAN),
                    value: Some(pb::metric::Value::BooleanValue(value)),
                    ..pb::Metric::default()
                }],
                ..pb::Payload::default()
            }
            .encode_to_vec()
        };
        assert!(rebirth_requested(&command(true)));
        assert!(!rebirth_requested(&command(false)));
        assert!(!rebirth_requested(b"\xFF\xFF garbage"));
    }

    #[test]
    fn sequence_numbers_wrap_after_255() {
        let mut node = Node {
            seq: 254,
            bd_seq: 0,
        };
        assert_eq!(
            [node.next_seq(), node.next_seq(), node.next_seq()],
            [254, 255, 0]
        );
    }
}
