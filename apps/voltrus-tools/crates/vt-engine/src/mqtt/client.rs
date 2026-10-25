//! Subscribes to an MQTT broker through `mt-mqtt`'s client. Every concrete
//! topic it receives becomes a point (or several: the numeric fields of a
//! JSON document, the metrics of a Sparkplug B payload); raw messages go to
//! the traffic monitor and broker events to the log. A write publishes the
//! value to the point's topic.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mt_mqtt::sparkplug::{SparkplugAliases, SparkplugTopic, pb};
use mt_mqtt::{MqttClientConfig, MqttClientEvent, MqttMessage, MqttPubClient, MqttTlsConfig};
use prost::Message as _;
use rumqttc::{AsyncClient, Event, Outgoing};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use vt_protocol::{
    Direction, LogLevel, MqttClientSpec, MqttQos, MqttSubscription, PathStep, PointInfo, PointText,
    Quality, Reading, SessionState, Stats, TrafficFrame, ValueFrame, parse_json_path,
};

use super::values::{self, Extracted, Sample, Sparkplug};
use super::{format_number, preview, wire_qos};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// How often the app sees what arrived.
pub(crate) const TICK: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// Points one session keeps at most: a `#` on a busy broker could otherwise
/// grow without end. Later topics still show in the traffic monitor.
const MAX_POINTS: usize = 5_000;

/// Gives sessions without a client id distinct ones: a broker drops the older
/// of two connections sharing an id.
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn run(
    spec: MqttClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let filters: Vec<Filter> = spec
        .subscriptions
        .iter()
        .map(|subscription| Filter {
            path: parse_json_path(&subscription.json_path)
                .ok()
                .filter(|path| !path.is_empty()),
            subscription: subscription.clone(),
        })
        .collect();
    let client_id = if spec.client_id.trim().is_empty() {
        format!(
            "voltrus-tools-{}-{}",
            std::process::id(),
            NEXT_CLIENT.fetch_add(1, Ordering::Relaxed)
        )
    } else {
        spec.client_id.trim().to_owned()
    };
    let url = format!(
        "{}://{}",
        if spec.tls { "mqtts" } else { "mqtt" },
        vt_protocol::SessionSpec::MqttClient(spec.clone()).address()
    );

    // Topics seen in an earlier connection of this session keep their ids.
    out.points(Vec::new());
    let mut topics = Topics::default();
    let mut stats = Stats::default();
    let mut reported = Stats::default();
    let mut traffic = Vec::new();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let connected = tokio::select! {
            result = MqttPubClient::connect(config(&spec, &client_id)) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        let mut client = match connected {
            Ok(client) => client,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                out.log(LogLevel::Error, format!("{url}: {error}"));
                out.stats(stats);
                if !retry(&mut control, attempt, &error, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        out.state(SessionState::Connected);
        out.log(LogLevel::Info, format!("Connected to {url} as {client_id}"));
        traffic.push(frame(
            Direction::Tx,
            format!("CONNECT {client_id}"),
            format!("keep-alive {} s", spec.keep_alive_s),
            0,
        ));
        topics.forget_births();

        for filter in &filters {
            let wanted = filter.subscription.qos;
            traffic.push(frame(
                Direction::Tx,
                format!("SUBSCRIBE {}", filter.subscription.filter),
                wanted.name().into(),
                0,
            ));
            let outcome = client
                .subscribe_confirm(&filter.subscription.filter, mt_qos(wanted))
                .await;
            for event in outcome.buffered {
                if let MqttClientEvent::Message(message) = event {
                    stats.frames += 1;
                    topics.receive(&message, &filters, &mut traffic, client.raw_client(), out);
                }
            }
            match (outcome.granted_qos, outcome.error) {
                (Some(granted), _) => {
                    let granted = vt_qos(granted);
                    traffic.push(frame(
                        Direction::Rx,
                        format!("SUBACK {}", filter.subscription.filter),
                        format!("granted {}", granted.name()),
                        0,
                    ));
                    out.log(
                        LogLevel::Info,
                        if granted == wanted {
                            format!(
                                "Subscribed to {} ({})",
                                filter.subscription.filter,
                                granted.name()
                            )
                        } else {
                            format!(
                                "Subscribed to {}: asked for {}, the broker granted {}",
                                filter.subscription.filter,
                                wanted.name(),
                                granted.name()
                            )
                        },
                    );
                }
                (None, error) => {
                    stats.errors += 1;
                    out.log(
                        LogLevel::Warn,
                        format!(
                            "Subscribing to {} failed: {}",
                            filter.subscription.filter,
                            error.unwrap_or_else(|| "no answer".into())
                        ),
                    );
                }
            }
        }

        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                event = client.next_event() => match event {
                    Some(MqttClientEvent::Message(message)) => {
                        stats.frames += 1;
                        topics.receive(&message, &filters, &mut traffic, client.raw_client(), out);
                    }
                    Some(MqttClientEvent::Rtt(ms)) => {
                        stats.last_latency_us = ms.saturating_mul(1000);
                    }
                    None => {
                        stats.errors += 1;
                        let reason = format!("Lost the connection to {url}");
                        out.log(LogLevel::Error, reason.clone());
                        topics.flush(&mut stats, out);
                        out.traffic(std::mem::take(&mut traffic));
                        attempt = 1;
                        if !retry(&mut control, attempt, &reason, out).await {
                            break 'session;
                        }
                        continue 'session;
                    }
                },
                _ = ticker.tick() => {
                    topics.flush(&mut stats, out);
                    out.traffic(std::mem::take(&mut traffic));
                    if stats != reported {
                        out.stats(stats);
                        reported = stats;
                    }
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        disconnect(&mut client).await;
                        traffic.push(frame(Direction::Tx, "DISCONNECT".into(), String::new(), 0));
                        topics.flush(&mut stats, out);
                        out.traffic(std::mem::take(&mut traffic));
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => {
                        let result = topics.write(point, value, client.raw_client(), &mut traffic);
                        out.written(point, result);
                    }
                    Some(Control::Publish { topic, payload, qos, retain }) => {
                        let result = publish(client.raw_client(), &topic, payload.into_bytes(), qos, retain, &mut traffic);
                        out.published(topic, result);
                    }
                    // Clients serve nothing, so there is nothing to hold.
                    Some(_) => {}
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn config(spec: &MqttClientSpec, client_id: &str) -> MqttClientConfig {
    let username = spec.username.trim();
    MqttClientConfig {
        host: spec.host.trim().to_owned(),
        port: spec.port,
        client_id: client_id.to_owned(),
        username: (!username.is_empty()).then(|| username.to_owned()),
        password: (!username.is_empty()).then(|| spec.password.clone()),
        tls: MqttTlsConfig {
            enabled: spec.tls,
            ..MqttTlsConfig::default()
        },
        keep_alive_secs: u64::from(spec.keep_alive_s),
        last_will: None,
    }
}

/// Say DISCONNECT and give the event loop a moment to send it.
async fn disconnect(client: &mut MqttPubClient) {
    if client.raw_client().try_disconnect().is_err() {
        return;
    }
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match client.raw_eventloop_poll().await {
                Ok(Event::Outgoing(Outgoing::Disconnect)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await;
}

fn mt_qos(qos: MqttQos) -> mt_mqtt::MqttQos {
    match qos {
        MqttQos::AtMostOnce => mt_mqtt::MqttQos::AtMostOnce,
        MqttQos::AtLeastOnce => mt_mqtt::MqttQos::AtLeastOnce,
        MqttQos::ExactlyOnce => mt_mqtt::MqttQos::ExactlyOnce,
    }
}

fn vt_qos(qos: mt_mqtt::MqttQos) -> MqttQos {
    match qos {
        mt_mqtt::MqttQos::AtMostOnce => MqttQos::AtMostOnce,
        mt_mqtt::MqttQos::AtLeastOnce => MqttQos::AtLeastOnce,
        mt_mqtt::MqttQos::ExactlyOnce => MqttQos::ExactlyOnce,
    }
}

fn publish(
    client: &AsyncClient,
    topic: &str,
    payload: Vec<u8>,
    qos: MqttQos,
    retain: bool,
    traffic: &mut Vec<TrafficFrame>,
) -> Result<(), String> {
    if !vt_protocol::valid_topic(topic) {
        return Err(format!("“{topic}” is not a topic to publish to"));
    }
    let detail = format!(
        "{}{} · {}",
        qos.name(),
        if retain { " · retained" } else { "" },
        preview(&payload)
    );
    let bytes = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    client
        .try_publish(topic, wire_qos(qos), retain, payload)
        .map_err(|error| format!("Not published: {error}"))?;
    traffic.push(frame(
        Direction::Tx,
        format!("PUBLISH {topic}"),
        detail,
        bytes,
    ));
    Ok(())
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

/// Report a lost broker and wait before the next attempt. Returns `false`
/// when the app closed the session while waiting.
async fn retry(
    control: &mut mpsc::UnboundedReceiver<Control>,
    attempt: u32,
    reason: &str,
    out: &Emitter,
) -> bool {
    let exponent = attempt.saturating_sub(1).min(5);
    let wait = (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF);
    out.state(SessionState::Reconnecting {
        attempt,
        retry_in_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
        reason: reason.to_owned(),
    });
    tokio::select! {
        () = tokio::time::sleep(wait) => true,
        () = until_closed(control, out) => false,
    }
}

/// Wait for the app to close the session while there is no broker, refusing
/// writes and publishes in the meantime.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("Not connected".into()));
            }
            Some(Control::Publish { topic, .. }) => {
                out.published(topic, Err("Not connected".into()));
            }
            Some(_) => {}
        }
    }
}

struct Filter {
    subscription: MqttSubscription,
    /// The subscription's JSON path, parsed.
    path: Option<Vec<PathStep>>,
}

/// Where a point's value comes from and goes to.
struct Origin {
    /// The topic writes publish to (plain points), or the Sparkplug source.
    topic: String,
    qos: MqttQos,
    boolean: bool,
    writable: bool,
    /// Sparkplug `group/node[/device]`, so a death can mark its values stale.
    sparkplug: Option<String>,
}

/// The points found so far and what changed since the app last heard.
#[derive(Default)]
struct Topics {
    ids: HashMap<(String, String), u32>,
    origins: Vec<Origin>,
    readings: Vec<Reading>,
    added: Vec<PointInfo>,
    texts: HashMap<u32, String>,
    changed: bool,
    full: bool,
    aliases: SparkplugAliases,
    /// Sparkplug sources whose birth this connection saw.
    born: HashSet<String>,
    /// Edge nodes asked to rebirth, so each is asked once per connection.
    asked: HashSet<String>,
}

impl Topics {
    /// A new connection hears births again: aliases and rebirth requests
    /// start over.
    fn forget_births(&mut self) {
        self.aliases.clear();
        self.born.clear();
        self.asked.clear();
    }

    fn receive(
        &mut self,
        message: &MqttMessage,
        filters: &[Filter],
        traffic: &mut Vec<TrafficFrame>,
        client: &AsyncClient,
        out: &Emitter,
    ) {
        let filter = filters
            .iter()
            .find(|filter| mt_mqtt::topic_matches(&filter.subscription.filter, &message.topic));
        let qos = vt_qos(message.qos);
        let mut detail = format!(
            "{}{} · ",
            qos.name(),
            if message.retained { " · retained" } else { "" }
        );
        if let Some(topic) = SparkplugTopic::parse(&message.topic) {
            match values::sparkplug(&topic, &message.payload, &mut self.aliases) {
                Ok(Sparkplug::Metrics { source, metrics }) => {
                    let _ = write!(
                        detail,
                        "{} · {} metric{}",
                        topic.message_type,
                        metrics.len(),
                        if metrics.len() == 1 { "" } else { "s" }
                    );
                    if topic.message_type.ends_with("BIRTH") {
                        self.born.insert(source.clone());
                    } else if !self.born.contains(&source) {
                        self.request_rebirth(&topic, client, traffic, out);
                    }
                    let label = source
                        .split_once('/')
                        .map_or(source.as_str(), |(_, rest)| rest);
                    for metric in metrics {
                        let name = format!("{label} › {}", metric.path);
                        let key = (source.clone(), metric.path.clone());
                        self.update(key, name, metric, qos, None, Some(&source), out);
                    }
                }
                Ok(Sparkplug::Death { source }) => {
                    detail.push_str(&topic.message_type);
                    self.born.retain(|born| !within(born, &source));
                    self.asked.remove(&source);
                    let mut stale = 0;
                    for (origin, reading) in self.origins.iter().zip(self.readings.iter_mut()) {
                        if origin
                            .sparkplug
                            .as_deref()
                            .is_some_and(|of| within(of, &source))
                        {
                            reading.quality = Quality::Bad;
                            stale += 1;
                        }
                    }
                    if stale > 0 {
                        self.changed = true;
                    }
                    out.log(
                        LogLevel::Warn,
                        format!("Sparkplug {source} went offline ({})", topic.message_type),
                    );
                }
                Ok(Sparkplug::Ignored) => detail.push_str(&preview(&message.payload)),
                Err(error) => {
                    detail.push_str(&error);
                    out.log(LogLevel::Warn, format!("{}: {error}", message.topic));
                }
            }
        } else {
            detail.push_str(&preview(&message.payload));
            let path = filter.and_then(|filter| filter.path.as_deref());
            for found in values::plain(&message.payload, path) {
                let name = if found.path.is_empty() {
                    message.topic.clone()
                } else {
                    format!("{} › {}", message.topic, found.path)
                };
                let key = (message.topic.clone(), found.path.clone());
                let writable = found.path.is_empty()
                    && matches!(found.sample, Sample::Number(_) | Sample::Bool(_));
                self.update(key, name, found, qos, Some(writable), None, out);
            }
        }
        traffic.push(frame(
            Direction::Rx,
            format!("PUBLISH {}", message.topic),
            detail,
            u32::try_from(message.payload.len()).unwrap_or(u32::MAX),
        ));
    }

    /// Store one value, adding its point the first time. `writable`: plain
    /// points publish writes back to their topic.
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        key: (String, String),
        name: String,
        found: Extracted,
        qos: MqttQos,
        writable: Option<bool>,
        sparkplug: Option<&str>,
        out: &Emitter,
    ) {
        let boolean = matches!(found.sample, Sample::Bool(_));
        let id = match self.ids.get(&key) {
            Some(id) => *id,
            None => {
                if self.origins.len() >= MAX_POINTS {
                    if !self.full {
                        self.full = true;
                        out.log(
                            LogLevel::Warn,
                            format!(
                                "{MAX_POINTS} points reached; new topics only show in the traffic monitor. Narrow the subscriptions to see them."
                            ),
                        );
                    }
                    return;
                }
                let id = u32::try_from(self.origins.len()).unwrap_or(u32::MAX);
                let writable = writable.unwrap_or(false);
                self.added.push(PointInfo {
                    id,
                    name,
                    address: qos.name().into(),
                    data_type: found.data_type.clone(),
                    unit: found.unit.clone(),
                    writable,
                    boolean,
                });
                self.origins.push(Origin {
                    topic: key.0.clone(),
                    qos,
                    boolean,
                    writable,
                    sparkplug: sparkplug.map(str::to_owned),
                });
                self.readings.push(Reading::bad());
                self.ids.insert(key, id);
                id
            }
        };
        let Some(reading) = usize::try_from(id)
            .ok()
            .and_then(|index| self.readings.get_mut(index))
        else {
            return;
        };
        reading.value = match found.sample {
            Sample::Number(value) => value,
            Sample::Bool(flag) => f64::from(u8::from(flag)),
            Sample::Text(_) => f64::NAN,
        };
        reading.quality = Quality::Good;
        self.texts.insert(id, found.text);
        self.changed = true;
    }

    /// Ask an edge node whose birth this connection missed to publish it
    /// again, as a Sparkplug host application does: data messages name
    /// their metrics by alias only.
    fn request_rebirth(
        &mut self,
        topic: &SparkplugTopic,
        client: &AsyncClient,
        traffic: &mut Vec<TrafficFrame>,
        out: &Emitter,
    ) {
        let Some(group) = &topic.group_id else {
            return;
        };
        let node = format!("{group}/{}", topic.edge_node_id);
        if !self.asked.insert(node.clone()) {
            return;
        }
        let command = pb::Payload {
            timestamp: Some(now_ms()),
            metrics: vec![pb::Metric {
                name: Some("Node Control/Rebirth".into()),
                datatype: Some(11),
                value: Some(pb::metric::Value::BooleanValue(true)),
                ..pb::Metric::default()
            }],
            ..pb::Payload::default()
        };
        let target = format!(
            "{}/{group}/NCMD/{}",
            vt_protocol::SPARKPLUG_NAMESPACE,
            topic.edge_node_id
        );
        match publish(
            client,
            &target,
            command.encode_to_vec(),
            MqttQos::AtMostOnce,
            false,
            traffic,
        ) {
            Ok(()) => out.log(
                LogLevel::Info,
                format!("Asked Sparkplug node {node} to rebirth: its birth was missed"),
            ),
            Err(error) => out.log(
                LogLevel::Warn,
                format!("Rebirth request to {node}: {error}"),
            ),
        }
    }

    /// Publish a value to a plain point's topic, as the payload it carries.
    fn write(
        &self,
        point: u32,
        value: f64,
        client: &AsyncClient,
        traffic: &mut Vec<TrafficFrame>,
    ) -> Result<(), String> {
        let origin = usize::try_from(point)
            .ok()
            .and_then(|index| self.origins.get(index))
            .ok_or_else(|| format!("No point {point}"))?;
        if !origin.writable {
            return Err(
                "Only whole-payload values can be written; publish to the topic instead".into(),
            );
        }
        if !value.is_finite() {
            return Err(format!("{value} is not a number"));
        }
        let payload = if origin.boolean {
            (if value != 0.0 { "true" } else { "false" }).to_owned()
        } else {
            format_number(value)
        };
        publish(
            client,
            &origin.topic,
            payload.into_bytes(),
            origin.qos,
            false,
            traffic,
        )
    }

    /// Tell the app what arrived since the last tick.
    fn flush(&mut self, stats: &mut Stats, out: &Emitter) {
        if !self.added.is_empty() {
            out.points_added(std::mem::take(&mut self.added));
        }
        if self.changed {
            self.changed = false;
            stats.polls += 1;
            out.values(ValueFrame {
                at_ms: now_ms(),
                latency_us: stats.last_latency_us,
                readings: self.readings.clone(),
            });
        }
        if !self.texts.is_empty() {
            let mut texts: Vec<PointText> = self
                .texts
                .drain()
                .map(|(point, text)| PointText { point, text })
                .collect();
            texts.sort_by_key(|text| text.point);
            out.payloads(texts);
        }
    }
}

/// Whether Sparkplug source `of` is `source` or one of its devices.
fn within(of: &str, source: &str) -> bool {
    of == source
        || of
            .strip_prefix(source)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::within;

    #[test]
    fn a_node_death_covers_its_devices() {
        assert!(within("G/node", "G/node"));
        assert!(within("G/node/pump", "G/node"));
        assert!(!within("G/node2/pump", "G/node"));
        assert!(!within("G/node", "G/node/pump"));
    }
}
