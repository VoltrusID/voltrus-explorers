//! A NATS server on this computer (`mt-nats`'s embedded server, with its
//! in-memory JetStream) and a publisher that plays a device template, the
//! same templates and drift as the Modbus simulator.
//!
//! The device publishes one number per value at `<prefix>.<value>` and a
//! JSON nameplate at `<prefix>.status`. A number published or sent as a
//! request to `<prefix>.<value>.set` is taken into the simulation and held;
//! a request to `<prefix>.get` is answered with every value. With JetStream
//! on, a stream keeps everything under the prefix, with a durable consumer.
//!
//! The requests are served by an ordinary client of the server on loopback,
//! as a service on a real NATS server would be.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt as _;
use mt_modbus::{SimDeviceProfile, Simulation, tick_values};
use mt_nats::{
    NatsAuthConfig, NatsClient, NatsClientConfig, NatsServerConfig, NatsServerState, NatsUser,
    OnMessage, start_server_on, stop_server,
};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, LogLevel, NatsHeader, NatsRequest, NatsServerSpec, PointInfo, Quality, Reading,
    SessionState, Stats, TrafficFrame, ValueFrame,
};

use super::{frame, url_host};
use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::mqtt::{format_number, listen_ip, preview};
use crate::templates;

/// How often the app sees the simulated values.
pub(crate) const REPORT_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
const MAX_CONNECTIONS: u32 = 256;
/// Messages the device's stream keeps.
const STREAM_MESSAGES: i64 = 10_000;
/// Connection name of the client that answers the device's requests.
const RESPONDER: &str = "voltrus-simulator";
/// Header that carries a value's unit; the client shows it on the point.
pub(crate) const UNIT_HEADER: &str = "Unit";

/// A message the server routed, as its callback reports it.
struct Routed {
    subject: String,
    payload: Vec<u8>,
    headers: usize,
}

pub(crate) async fn run(
    spec: NatsServerSpec,
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
    let server = NatsServerState::new(NatsServerConfig {
        enabled: true,
        port: spec.port,
        max_connections: MAX_CONNECTIONS,
        auth: if spec.username.is_empty() {
            NatsAuthConfig::default()
        } else {
            NatsAuthConfig {
                mode: "password".into(),
                users: vec![NatsUser {
                    username: spec.username.clone(),
                    password: spec.password.clone(),
                }],
            }
        },
    });
    // The server reports every routed message; the device's own publishes
    // are summarised instead (while `own` is set, which also hides a client
    // message routed in that instant from the traffic view, never from the
    // subscribers).
    let own = Arc::new(AtomicBool::new(false));
    let (routed_tx, mut routed) = mpsc::unbounded_channel::<Routed>();
    let on_message: OnMessage = {
        let own = own.clone();
        Arc::new(
            move |subject: String, payload: Vec<u8>, _at: i64, headers: Vec<(String, String)>| {
                if !own.load(Ordering::Relaxed) {
                    let _ = routed_tx.send(Routed {
                        subject,
                        payload,
                        headers: headers.len(),
                    });
                }
            },
        )
    };
    let address = match listen(&server, ip, &on_message, &mut control, out).await {
        Ok(Some(address)) => address,
        Ok(None) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
        Err(error) => {
            let message = format!(
                "Cannot listen on {}:{}: {error}",
                url_host(&ip.to_string()),
                spec.port
            );
            out.log(LogLevel::Error, message.clone());
            out.state(SessionState::Closed {
                error: Some(message),
            });
            return;
        }
    };
    out.state(SessionState::Listening {
        address: address.clone(),
    });

    let mut device = profile.map(|profile| Device::new(profile.clone(), &spec));
    out.log(
        LogLevel::Info,
        match &device {
            Some(device) => format!(
                "NATS server on {address}, publishing {} {} under {}.>",
                device.simulation.profile.manufacturer,
                device.simulation.profile.model,
                device.prefix,
            ),
            None => format!("NATS server on {address}"),
        },
    );
    let mut traffic = Vec::new();

    // The device's service: a client of this server answering requests.
    let responder = match &device {
        Some(device) => responder(&spec, ip, device, &mut traffic, out).await,
        None => None,
    };
    let (client, mut requests) = match responder {
        Some((client, requests)) => (Some(client), Some(requests)),
        None => (None, None),
    };
    if let (Some(client), Some(device)) = (&client, &device)
        && spec.jetstream
    {
        let stream = spec.stream_name();
        let subjects = vec![format!("{}.>", device.prefix)];
        match client
            .js_create_stream(&stream, &subjects, STREAM_MESSAGES, 0)
            .await
        {
            Ok(_) => {
                let consumer = client
                    .js_create_consumer(&stream, NatsServerSpec::CONSUMER)
                    .await;
                out.log(
                    LogLevel::Info,
                    match consumer {
                        Ok(_) => format!(
                            "JetStream stream {stream} keeps {}.> (last {STREAM_MESSAGES}), with durable consumer {}",
                            device.prefix,
                            NatsServerSpec::CONSUMER
                        ),
                        Err(error) => format!(
                            "JetStream stream {stream} keeps {}.>; consumer {}: {error}",
                            device.prefix,
                            NatsServerSpec::CONSUMER
                        ),
                    },
                );
            }
            Err(error) => out.log(
                LogLevel::Warn,
                format!("JetStream stream {stream}: {error}"),
            ),
        }
    }

    match &mut device {
        Some(device) => {
            out.points(device.points());
            device.publish(&server, &own, &mut traffic).await;
            out.values(device.frame());
        }
        None => out.points(Vec::new()),
    }

    let mut simulate = tokio::time::interval(Duration::from_millis(
        spec.publish_ms.max(NatsServerSpec::MIN_PUBLISH_MS),
    ));
    simulate.set_missed_tick_behavior(MissedTickBehavior::Delay);
    simulate.reset();
    let mut report = tokio::time::interval(REPORT_TICK);
    report.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stats = Stats::default();
    let mut clients: HashMap<u64, String> = HashMap::new();
    let serving = requests.is_some();

    loop {
        tokio::select! {
            Some(message) = routed.recv() => {
                stats.frames += 1;
                // Replies are shown where they are sent.
                if !message.subject.starts_with("_INBOX.") {
                    traffic.push(frame(
                        Direction::Rx,
                        format!("{} {}", if message.headers > 0 { "HPUB" } else { "PUB" }, message.subject),
                        preview(&message.payload),
                        message.payload.len(),
                    ));
                }
            }
            request = next_request(&mut requests), if serving => match (request, &client, &mut device) {
                (Some(request), Some(client), Some(device)) => {
                    device.serve(&request, client, &server, &own, &mut traffic, out).await;
                }
                _ => {
                    requests = None;
                    out.log(LogLevel::Warn, "The device's request service stopped; publishes still run");
                }
            },
            _ = simulate.tick() => {
                if let Some(device) = &mut device {
                    tick_values(&mut device.simulation);
                    device.publish(&server, &own, &mut traffic).await;
                    stats.polls += 1;
                }
            }
            _ = report.tick() => {
                watch_clients(&server, &mut clients, &mut traffic, out).await;
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
                        let result = device.set(point, value, &server, &own, &mut traffic).await;
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
                Some(Control::Publish { topic, payload, .. }) => {
                    let result = route(&server, &own, &topic, payload.into_bytes(), &[], &mut traffic).await;
                    out.published(topic, result);
                }
                Some(Control::Nats(NatsRequest::Publish { subject, payload, headers })) => {
                    let result = route(&server, &own, &subject, payload.into_bytes(), &headers, &mut traffic).await;
                    out.published(subject, result);
                }
                Some(Control::Nats(request)) => {
                    super::client::refuse(request, "Requests and JetStream browsing go from a NATS connection; the simulator only publishes", out);
                }
                // Other controls don't apply to a server.
                Some(_) => {}
            },
        }
    }
    drop(requests);
    drop(client);
    let connected = server
        .get_connections()
        .await
        .into_iter()
        .filter(|connection| connection.name.as_deref() != Some(RESPONDER))
        .count();
    let _ = stop_server(&server).await;
    if connected > 0 {
        out.log(
            LogLevel::Info,
            format!(
                "Stopped; {connected} client{} disconnected",
                if connected == 1 { "" } else { "s" }
            ),
        );
    }
    out.traffic(traffic);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

type Requests = futures_util::stream::SelectAll<async_nats::Subscriber>;

async fn next_request(requests: &mut Option<Requests>) -> Option<async_nats::Message> {
    match requests {
        Some(requests) => requests.next().await,
        None => std::future::pending().await,
    }
}

/// Bind the server. A port still held by a session that is closing is
/// retried for [`BIND_RETRY`]. `Ok(None)` when the app closed the session
/// meanwhile.
async fn listen(
    server: &NatsServerState,
    ip: IpAddr,
    on_message: &OnMessage,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<String>, String> {
    let deadline = Instant::now() + BIND_RETRY;
    let port = server.config.read().await.port;
    loop {
        match start_server_on(server.clone(), ip, Some(on_message.clone())).await {
            Ok(()) => return Ok(Some(format!("{}:{port}", url_host(&ip.to_string())))),
            Err(error) if Instant::now() < deadline => {
                tracing::debug!("NATS server bind: {error}; retrying");
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
                out.written(point, Err("The server is not listening yet".into()));
            }
            Some(Control::Publish { topic, .. }) => {
                out.published(topic, Err("The server is not listening yet".into()));
            }
            Some(Control::Nats(request)) => {
                super::client::refuse(request, "The server is not listening yet", out);
            }
            Some(_) => {}
        }
    }
}

/// Connect the device's service to the server and subscribe to its request
/// subjects. `None` (logged) when it cannot: the device still publishes.
async fn responder(
    spec: &NatsServerSpec,
    ip: IpAddr,
    device: &Device,
    traffic: &mut Vec<TrafficFrame>,
    out: &Emitter,
) -> Option<(NatsClient, Requests)> {
    let host = match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_owned(),
        IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_owned(),
        ip => url_host(&ip.to_string()),
    };
    let config = NatsClientConfig {
        host,
        port: spec.port,
        name: RESPONDER.into(),
        auth: if spec.username.is_empty() {
            mt_nats::NatsAuth::None
        } else {
            mt_nats::NatsAuth::UserPass {
                username: spec.username.clone(),
                password: spec.password.clone(),
            }
        },
        connect_timeout_ms: 3000,
        ..NatsClientConfig::default()
    };
    let client = match NatsClient::connect(&config).await {
        Ok(client) => client,
        Err(error) => {
            out.log(
                LogLevel::Warn,
                format!("The device's request service could not connect: {error}"),
            );
            return None;
        }
    };
    let mut subscribers = Vec::with_capacity(2);
    for subject in [
        format!("{}.*.set", device.prefix),
        format!("{}.get", device.prefix),
    ] {
        match client.inner().subscribe(subject.clone()).await {
            Ok(subscriber) => {
                traffic.push(frame(
                    Direction::Tx,
                    format!("SUB {subject}"),
                    RESPONDER.into(),
                    0,
                ));
                subscribers.push(subscriber);
            }
            Err(error) => {
                out.log(LogLevel::Warn, format!("Serving {subject}: {error}"));
                return None;
            }
        }
    }
    out.log(
        LogLevel::Info,
        format!(
            "Serving requests: {0}.get answers every value, {0}.<value>.set takes one",
            device.prefix
        ),
    );
    Some((client, futures_util::stream::select_all(subscribers)))
}

/// Report clients that connected or left since the last look.
async fn watch_clients(
    server: &NatsServerState,
    clients: &mut HashMap<u64, String>,
    traffic: &mut Vec<TrafficFrame>,
    out: &Emitter,
) {
    let now: HashMap<u64, String> = server
        .get_connections()
        .await
        .into_iter()
        .filter(|connection| connection.name.as_deref() != Some(RESPONDER))
        .map(|connection| {
            let name = connection
                .name
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("client {}", connection.cid));
            (connection.cid, name)
        })
        .collect();
    for (cid, name) in &now {
        if !clients.contains_key(cid) {
            traffic.push(frame(
                Direction::Rx,
                format!("CONNECT {name}"),
                String::new(),
                0,
            ));
            out.log(LogLevel::Info, format!("{name} connected"));
        }
    }
    for (cid, name) in clients.iter() {
        if !now.contains_key(cid) {
            traffic.push(frame(
                Direction::Rx,
                format!("DISCONNECT {name}"),
                String::new(),
                0,
            ));
            out.log(LogLevel::Info, format!("{name} disconnected"));
        }
    }
    *clients = now;
}

/// Publish through the server's router as if a client had: subscribers,
/// JetStream and the traffic monitor all see it.
async fn route(
    server: &NatsServerState,
    own: &AtomicBool,
    subject: &str,
    payload: Vec<u8>,
    headers: &[NatsHeader],
    traffic: &mut Vec<TrafficFrame>,
) -> Result<(), String> {
    if !vt_protocol::valid_subject(subject) {
        return Err(format!("“{subject}” is not a subject to publish to"));
    }
    traffic.push(frame(
        Direction::Tx,
        format!(
            "{} {subject}",
            if headers.is_empty() { "PUB" } else { "HPUB" }
        ),
        preview(&payload),
        payload.len(),
    ));
    let headers = headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    own.store(true, Ordering::Relaxed);
    let result = server
        .publish(subject.to_owned(), None, headers, payload)
        .await;
    own.store(false, Ordering::Relaxed);
    result
}

/// The simulated device and where it publishes.
struct Device {
    simulation: Simulation,
    prefix: String,
    /// `<prefix>.<value>` per template value.
    subjects: Vec<String>,
    /// The template's value names in subjects, for `.set` requests.
    keys: Vec<String>,
    started: Instant,
}

impl Device {
    fn new(profile: SimDeviceProfile, spec: &NatsServerSpec) -> Self {
        let prefix = spec.prefix.trim().trim_matches('.').to_owned();
        let keys: Vec<String> = profile
            .registers
            .iter()
            .map(|register| subject_token(&register.name))
            .collect();
        let subjects = keys.iter().map(|key| format!("{prefix}.{key}")).collect();
        Self {
            simulation: Simulation::new(profile),
            prefix,
            subjects,
            keys,
            started: Instant::now(),
        }
    }

    fn points(&self) -> Vec<PointInfo> {
        self.subjects
            .iter()
            .zip(&self.simulation.profile.registers)
            .enumerate()
            .map(|(index, (subject, register))| PointInfo {
                id: u32::try_from(index).unwrap_or(u32::MAX),
                name: subject.clone(),
                // Short for the address column; a new value goes to
                // `<subject>.set`.
                address: "PUB".into(),
                data_type: "Number".into(),
                unit: register.unit.clone(),
                writable: true,
                boolean: false,
                states: Vec::new(),
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

    fn status(&self) -> serde_json::Value {
        let profile = &self.simulation.profile;
        json!({
            "online": true,
            "manufacturer": profile.manufacturer,
            "model": profile.model,
            "uptime_s": self.started.elapsed().as_secs(),
        })
    }

    /// Publish every value and the nameplate.
    async fn publish(
        &self,
        server: &NatsServerState,
        own: &AtomicBool,
        traffic: &mut Vec<TrafficFrame>,
    ) {
        own.store(true, Ordering::Relaxed);
        for ((subject, value), register) in self
            .subjects
            .iter()
            .zip(&self.simulation.values)
            .zip(&self.simulation.profile.registers)
        {
            let _ = server
                .publish(
                    subject.clone(),
                    None,
                    unit_header(&register.unit)
                        .into_iter()
                        .map(|header| (header.name, header.value))
                        .collect(),
                    format_number(*value).into_bytes(),
                )
                .await;
        }
        let _ = server
            .publish(
                format!("{}.status", self.prefix),
                None,
                Vec::new(),
                self.status().to_string().into_bytes(),
            )
            .await;
        own.store(false, Ordering::Relaxed);
        traffic.push(frame(
            Direction::Tx,
            format!("PUB {} subjects", self.subjects.len() + 1),
            format!("{}.>", self.prefix),
            0,
        ));
    }

    /// Answer a request (or take a plain publish) on the device's service
    /// subjects.
    async fn serve(
        &mut self,
        request: &async_nats::Message,
        client: &NatsClient,
        server: &NatsServerState,
        own: &AtomicBool,
        traffic: &mut Vec<TrafficFrame>,
        out: &Emitter,
    ) {
        let subject = request.subject.as_str();
        let who = request
            .reply
            .as_ref()
            .map_or("A client", |_| "A requester")
            .to_owned();
        let answer = if subject == format!("{}.get", self.prefix) {
            let mut values = serde_json::Map::new();
            for (key, value) in self.keys.iter().zip(&self.simulation.values) {
                values.insert(key.clone(), json!(value));
            }
            json!({ "status": self.status(), "values": values })
        } else {
            let key = subject
                .strip_prefix(&format!("{}.", self.prefix))
                .and_then(|rest| rest.strip_suffix(".set"))
                .unwrap_or_default();
            match self.keys.iter().position(|candidate| candidate == key) {
                Some(index) => {
                    let text = String::from_utf8_lossy(&request.payload);
                    match text.trim().parse::<f64>() {
                        Ok(value) if value.is_finite() => {
                            let point = u32::try_from(index).unwrap_or(u32::MAX);
                            match self.set(point, value, server, own, traffic).await {
                                Ok(()) => {
                                    out.log(
                                        LogLevel::Info,
                                        format!(
                                            "{who} set {key} to {}; holding it",
                                            format_number(value)
                                        ),
                                    );
                                    out.locked(point, true);
                                    out.values(self.frame());
                                    json!({ "ok": true, "value": value })
                                }
                                Err(error) => {
                                    out.log(LogLevel::Warn, format!("{who}: {error}"));
                                    json!({ "ok": false, "error": error })
                                }
                            }
                        }
                        _ => {
                            let error = format!("“{}” is not a number", text.trim());
                            out.log(LogLevel::Warn, format!("{who} sent {subject}: {error}"));
                            json!({ "ok": false, "error": error })
                        }
                    }
                }
                None => json!({ "ok": false, "error": format!("No value {key}") }),
            }
        };
        if let Some(reply) = &request.reply {
            let body = answer.to_string().into_bytes();
            traffic.push(frame(
                Direction::Tx,
                format!("REPLY {subject}"),
                preview(&body),
                body.len(),
            ));
            if let Err(error) = client.publish(reply.as_str(), &body).await {
                out.log(LogLevel::Warn, format!("Reply to {subject}: {error}"));
            }
        }
    }

    /// Set a value and hold it, publishing it at once.
    async fn set(
        &mut self,
        point: u32,
        value: f64,
        server: &NatsServerState,
        own: &AtomicBool,
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
        let headers = unit_header(&register.unit);
        self.simulation.values[index] = value;
        // Hold the value: otherwise the next tick drifts it away.
        self.simulation.locks.set(index, true);
        if let Some(subject) = self.subjects.get(index) {
            route(
                server,
                own,
                subject,
                format_number(value).into_bytes(),
                &headers,
                traffic,
            )
            .await?;
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

/// The `Unit` header the device's values carry, so clients show units.
fn unit_header(unit: &str) -> Vec<NatsHeader> {
    if unit.is_empty() {
        Vec::new()
    } else {
        vec![NatsHeader {
            name: UNIT_HEADER.into(),
            value: unit.to_owned(),
        }]
    }
}

/// A template value name as one subject token: lower case, with anything
/// that is not a letter, digit, `-` or `_` replaced by `_`.
fn subject_token(name: &str) -> String {
    let token: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if token.is_empty() {
        "value".to_owned()
    } else {
        token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_names_become_single_subject_tokens() {
        assert_eq!(subject_token("AC_Power"), "ac_power");
        assert_eq!(subject_token("Temp. (°C) > 5"), "temp____c____5");
        assert_eq!(subject_token(""), "value");
        assert!(vt_protocol::valid_subject(&format!(
            "plant.{}",
            subject_token("a.b*c >")
        )));
    }

    #[test]
    fn every_template_publishes_distinct_valid_subjects() {
        for profile in templates::modbus() {
            let device = Device::new(profile.clone(), &NatsServerSpec::default());
            for subject in &device.subjects {
                assert!(vt_protocol::valid_subject(subject), "{subject}");
            }
            let mut unique = device.subjects.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), device.subjects.len(), "{}", profile.id);
        }
    }
}
