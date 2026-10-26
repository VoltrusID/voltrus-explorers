//! A NATS server simulator (mt-nats's embedded server with JetStream) and a
//! client talking over loopback through the engine, the way the app drives
//! them.

use std::time::{Duration, Instant};

use vt_protocol::{
    Command, Event, EventKind, NatsEvent, NatsHeader, NatsReply, NatsRequest, NatsServerSpec,
    NatsStoredMessage, NatsStream, NatsStreamDetail, NatsSubscription, PointInfo, Quality, Reading,
    SessionId, SessionSpec, SessionState,
};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .expect("a free loopback port")
}

/// What one session has reported so far.
#[derive(Default)]
struct Seen {
    state: Option<SessionState>,
    points: Vec<PointInfo>,
    readings: Vec<Reading>,
    frames: usize,
    payloads: Vec<String>,
    written: Vec<(u32, Result<(), String>)>,
    published: Vec<(String, Result<(), String>)>,
    locked: Vec<(u32, bool)>,
    replies: Vec<(String, Result<NatsReply, String>)>,
    streams: Option<Result<Vec<NatsStream>, String>>,
    stream: Option<Result<NatsStreamDetail, String>>,
    fetched: Option<Result<Vec<NatsStoredMessage>, String>>,
    traffic: Vec<String>,
    log: Vec<String>,
}

impl Seen {
    fn apply(&mut self, kind: EventKind) {
        match kind {
            EventKind::State(state) => self.state = Some(state),
            EventKind::Points(points) => {
                self.readings = vec![Reading::bad(); points.len()];
                self.payloads = vec![String::new(); points.len()];
                self.points = points;
            }
            EventKind::PointsAdded(points) => {
                for (offset, point) in points.iter().enumerate() {
                    assert_eq!(
                        point.id as usize,
                        self.points.len() + offset,
                        "ids continue in order"
                    );
                }
                self.points.extend(points);
                self.payloads.resize(self.points.len(), String::new());
            }
            EventKind::Values(frame) => {
                self.readings = frame.readings;
                self.frames += 1;
            }
            EventKind::Payloads(texts) => {
                for text in texts {
                    if let Some(slot) = self.payloads.get_mut(text.point as usize) {
                        *slot = text.text;
                    }
                }
            }
            EventKind::Written { point, result } => self.written.push((point, result)),
            EventKind::Published { topic, result } => self.published.push((topic, result)),
            EventKind::Locked { point, locked } => self.locked.push((point, locked)),
            EventKind::Traffic(frames) => self.traffic.extend(
                frames
                    .into_iter()
                    .map(|frame| format!("{} {}", frame.summary, frame.detail)),
            ),
            EventKind::Log(entry) => self.log.push(entry.message),
            EventKind::Nats(NatsEvent::Reply { subject, result }) => {
                self.replies.push((subject, result));
            }
            EventKind::Nats(NatsEvent::Streams(result)) => self.streams = Some(result),
            EventKind::Nats(NatsEvent::Stream { result, .. }) => self.stream = Some(result),
            EventKind::Nats(NatsEvent::Fetched { result, .. }) => self.fetched = Some(result),
            _ => {}
        }
    }

    fn point(&self, name: &str) -> Option<&PointInfo> {
        self.points.iter().find(|point| point.name == name)
    }

    fn value(&self, name: &str) -> Option<f64> {
        let point = self.point(name)?;
        let reading = self.readings.get(point.id as usize)?;
        (reading.quality == Quality::Good).then_some(reading.value)
    }

    fn payload(&self, name: &str) -> Option<&str> {
        let point = self.point(name)?;
        self.payloads.get(point.id as usize).map(String::as_str)
    }

    fn reply(&self, subject: &str) -> Option<&Result<NatsReply, String>> {
        self.replies
            .iter()
            .rev()
            .find(|(asked, _)| asked == subject)
            .map(|(_, result)| result)
    }
}

struct Engine {
    engine: vt_engine::EngineHandle,
    events: async_channel::Receiver<Event>,
    server: Seen,
    client: Seen,
}

const SERVER: SessionId = SessionId(1);
const CLIENT: SessionId = SessionId(2);

impl Engine {
    fn start() -> Self {
        let (engine, events) = vt_engine::start().expect("the engine starts");
        Self {
            engine,
            events,
            server: Seen::default(),
            client: Seen::default(),
        }
    }

    fn send(&self, command: Command) {
        assert!(self.engine.send(command), "the engine is running");
    }

    fn nats(&self, request: NatsRequest) {
        self.send(Command::Nats {
            session: CLIENT,
            request,
        });
    }

    /// Take events until `done` holds, or fail after 20 s.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}\nclient log: {:?}\nserver log: {:?}",
                self.client.log,
                self.server.log
            );
            match self.events.try_recv() {
                Ok(event) if event.session == SERVER => self.server.apply(event.kind),
                Ok(event) if event.session == CLIENT => self.client.apply(event.kind),
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

#[test]
fn a_client_watches_requests_and_browses_the_simulated_server() {
    let mut engine = Engine::start();
    let port = free_port();
    let spec = NatsServerSpec {
        port,
        publish_ms: 100,
        username: "plant".into(),
        password: "s3cret".into(),
        ..NatsServerSpec::default()
    };
    engine.send(Command::Open {
        session: SERVER,
        spec: SessionSpec::NatsServer(spec.clone()),
    });
    engine.until("the server to listen and report its points", |engine| {
        matches!(engine.server.state, Some(SessionState::Listening { .. }))
            && !engine.server.points.is_empty()
    });
    assert_eq!(
        engine.server.points.len(),
        20,
        "one point per template value"
    );

    let mut client = spec.client();
    client.subscriptions.push(NatsSubscription {
        subject: "test.*".into(),
        queue: "panel".into(),
        json_path: String::new(),
    });
    engine.send(Command::Open {
        session: CLIENT,
        spec: SessionSpec::NatsClient(client.clone()),
    });

    // Plain subjects and the JSON nameplate become points.
    engine.until("the device's points", |engine| {
        let seen = &engine.client;
        seen.value("plant.inverter.ac_power").is_some()
            && seen.value("plant.inverter.status › online").is_some()
    });
    assert_eq!(engine.client.state, Some(SessionState::Connected));
    let online = engine
        .client
        .point("plant.inverter.status › online")
        .expect("nameplate");
    assert!(
        online.boolean && !online.writable,
        "JSON fields are read-only"
    );
    assert_eq!(
        online.address, "plant.inverter.>",
        "the matching subscription"
    );
    let power = engine
        .client
        .value("plant.inverter.ac_power")
        .expect("AC power");
    assert!((0.0..=60_000.0).contains(&power), "AC power {power}");
    assert_eq!(
        engine
            .client
            .point("plant.inverter.ac_power")
            .map(|point| point.unit.as_str()),
        Some("W"),
        "the unit from the Unit header"
    );
    engine.until("the server to see the client", |engine| {
        engine
            .server
            .log
            .iter()
            .any(|line| line.starts_with("voltrus-tools-") && line.ends_with(" connected"))
    });

    // A write publishes to the value's subject; the simulator does not take
    // plain publishes to a value (only `.set`), so the next tick overwrites
    // it. A request to `.set` is taken, held and confirmed.
    let temperature = engine
        .client
        .point("plant.inverter.temperature")
        .expect("the temperature subject")
        .clone();
    assert!(temperature.writable);
    engine.send(Command::Write {
        session: CLIENT,
        point: temperature.id,
        value: 61.5,
    });
    engine.until("the write", |engine| {
        engine.client.written.contains(&(temperature.id, Ok(())))
    });
    engine.nats(NatsRequest::Request {
        subject: "plant.inverter.temperature.set".into(),
        payload: "48.5".into(),
        headers: Vec::new(),
        timeout_ms: 3000,
    });
    engine.until("the set to be confirmed", |engine| {
        engine
            .client
            .reply("plant.inverter.temperature.set")
            .is_some()
    });
    let reply = engine
        .client
        .reply("plant.inverter.temperature.set")
        .cloned()
        .expect("a reply")
        .expect("the set is confirmed");
    assert!(reply.payload.contains("\"ok\":true"), "{}", reply.payload);
    assert!(reply.subject.starts_with("_INBOX."), "{}", reply.subject);
    let server_temperature = engine
        .server
        .point("plant.inverter.temperature")
        .expect("served")
        .id;
    engine.until("the value to be held", |engine| {
        engine.server.locked.contains(&(server_temperature, true))
            && engine.client.value("plant.inverter.temperature") == Some(48.5)
    });
    let held = engine.client.frames;
    engine.until("several frames after the hold", |engine| {
        engine.client.frames >= held + 8
    });
    assert_eq!(
        engine.client.value("plant.inverter.temperature"),
        Some(48.5)
    );

    // Out-of-range and non-numeric sets are refused in the reply.
    engine.nats(NatsRequest::Request {
        subject: "plant.inverter.temperature.set".into(),
        payload: "warm".into(),
        headers: Vec::new(),
        timeout_ms: 3000,
    });
    engine.until("the refusal", |engine| {
        engine
            .client
            .reply("plant.inverter.temperature.set")
            .is_some_and(|reply| {
                reply
                    .as_ref()
                    .is_ok_and(|reply| reply.payload.contains("\"ok\":false"))
            })
    });

    // `.get` answers with every value.
    engine.nats(NatsRequest::Request {
        subject: "plant.inverter.get".into(),
        payload: String::new(),
        headers: Vec::new(),
        timeout_ms: 3000,
    });
    engine.until("the snapshot", |engine| {
        engine.client.reply("plant.inverter.get").is_some()
    });
    let snapshot = engine
        .client
        .reply("plant.inverter.get")
        .cloned()
        .expect("a reply")
        .expect("a snapshot");
    let document: serde_json::Value = serde_json::from_str(&snapshot.payload).expect("JSON values");
    assert_eq!(document["values"]["temperature"], 48.5);
    assert_eq!(document["status"]["online"], true);

    // Nobody answers: the request times out, it does not hang.
    engine.nats(NatsRequest::Request {
        subject: "nobody.home".into(),
        payload: "?".into(),
        headers: Vec::new(),
        timeout_ms: 300,
    });
    engine.until("the timeout", |engine| {
        engine.client.reply("nobody.home").is_some()
    });
    assert!(
        engine
            .client
            .reply("nobody.home")
            .expect("answered")
            .is_err()
    );

    // A publish with headers comes back through the queue subscription; the
    // JSON number is a point and the headers show with the message.
    engine.nats(NatsRequest::Publish {
        subject: "test.limit".into(),
        payload: r#"{"kw": 42.5, "mode": "eco"}"#.into(),
        headers: vec![NatsHeader {
            name: "X-Unit".into(),
            value: "kW".into(),
        }],
    });
    // Values and the message text behind them arrive in the same tick.
    engine.until("the published message to come back", |engine| {
        engine.client.value("test.limit › kw") == Some(42.5)
            && engine
                .client
                .payload("test.limit › kw")
                .is_some_and(|text| !text.is_empty())
    });
    assert_eq!(engine.client.published, [("test.limit".to_owned(), Ok(()))]);
    assert!(
        engine
            .client
            .payload("test.limit › kw")
            .is_some_and(|text| text.starts_with("X-Unit: kW")),
        "{:?}",
        engine.client.payload("test.limit › kw")
    );
    assert_eq!(
        engine
            .client
            .point("test.limit › kw")
            .map(|point| point.address.as_str()),
        Some("test.* · panel")
    );
    // The server reports its traffic on its own tick.
    engine.until("the server to see the publish", |engine| {
        engine
            .server
            .traffic
            .iter()
            .any(|line| line.starts_with("HPUB test.limit"))
    });
    // Wildcards are refused before the wire.
    engine.nats(NatsRequest::Publish {
        subject: "test.*".into(),
        payload: "1".into(),
        headers: Vec::new(),
    });
    engine.until("the refusal", |engine| engine.client.published.len() == 2);
    assert!(engine.client.published[1].1.is_err());

    // JetStream: the simulator's stream keeps what the device publishes.
    engine.nats(NatsRequest::Streams);
    engine.until("the streams", |engine| engine.client.streams.is_some());
    let streams = engine
        .client
        .streams
        .clone()
        .expect("answered")
        .expect("JetStream works");
    let stream = streams
        .iter()
        .find(|stream| stream.name == spec.stream_name())
        .expect("the device's stream");
    assert_eq!(stream.subjects, ["plant.inverter.>"]);
    assert!(stream.messages > 20, "messages {}", stream.messages);
    assert_eq!(stream.consumers, 1);

    engine.nats(NatsRequest::Stream {
        name: spec.stream_name(),
        messages: 5,
    });
    engine.until("the stream detail", |engine| engine.client.stream.is_some());
    let detail = engine
        .client
        .stream
        .clone()
        .expect("answered")
        .expect("a stream");
    assert_eq!(detail.messages.len(), 5);
    assert!(
        detail
            .messages
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence),
        "oldest first"
    );
    assert_eq!(
        detail.messages.last().map(|message| message.sequence),
        Some(detail.stream.last_seq)
    );
    let consumer = detail.consumers.first().expect("the durable consumer");
    assert_eq!(consumer.name, NatsServerSpec::CONSUMER);
    assert!(consumer.durable && consumer.pending > 0);

    engine.nats(NatsRequest::Fetch {
        stream: spec.stream_name(),
        consumer: NatsServerSpec::CONSUMER.into(),
        batch: 3,
    });
    engine.until("the fetch", |engine| engine.client.fetched.is_some());
    let fetched = engine
        .client
        .fetched
        .clone()
        .expect("answered")
        .expect("messages");
    let sequences: Vec<u64> = fetched.iter().map(|message| message.sequence).collect();
    assert_eq!(
        sequences,
        [1, 2, 3],
        "a new consumer starts at the beginning"
    );
    assert!(fetched[0].subject.starts_with("plant.inverter."));

    engine.nats(NatsRequest::Stream {
        name: "NOPE".into(),
        messages: 5,
    });
    engine.client.stream = None;
    engine.until("the missing stream", |engine| {
        engine.client.stream.is_some()
    });
    assert!(engine.client.stream.clone().expect("answered").is_err());

    // Stopping the server frees its port at once: it starts again on it,
    // and the client notices the server went away and comes back.
    engine.send(Command::Close { session: SERVER });
    engine.until("the server to stop", |engine| {
        matches!(
            engine.server.state,
            Some(SessionState::Closed { error: None })
        )
    });
    engine.until("the client to lose the server", |engine| {
        matches!(engine.client.state, Some(SessionState::Reconnecting { .. }))
    });
    engine.server = Seen::default();
    engine.send(Command::Open {
        session: SERVER,
        spec: SessionSpec::NatsServer(spec.clone()),
    });
    engine.until("the server to listen again", |engine| {
        matches!(engine.server.state, Some(SessionState::Listening { .. }))
    });
    engine.until("the client to reconnect", |engine| {
        engine.client.state == Some(SessionState::Connected)
    });

    engine.send(Command::Close { session: CLIENT });
    engine.send(Command::Close { session: SERVER });
    engine.until("both to stop", |engine| {
        matches!(engine.client.state, Some(SessionState::Closed { .. }))
            && matches!(engine.server.state, Some(SessionState::Closed { .. }))
    });
}

#[test]
fn a_wrong_password_and_a_taken_port_are_reported_not_panicked() {
    let mut engine = Engine::start();
    let port = free_port();
    let spec = NatsServerSpec {
        port,
        template: None,
        username: "plant".into(),
        password: "right".into(),
        ..NatsServerSpec::default()
    };
    engine.send(Command::Open {
        session: SERVER,
        spec: SessionSpec::NatsServer(spec.clone()),
    });
    engine.until("the server to listen", |engine| {
        matches!(engine.server.state, Some(SessionState::Listening { .. }))
    });

    let mut client = spec.client();
    client.auth = vt_protocol::NatsAuth::UserPassword {
        username: "plant".into(),
        password: "wrong".into(),
    };
    engine.send(Command::Open {
        session: CLIENT,
        spec: SessionSpec::NatsClient(client),
    });
    engine.until("the refusal", |engine| {
        matches!(engine.client.state, Some(SessionState::Reconnecting { .. }))
    });
    assert!(
        engine
            .client
            .log
            .iter()
            .any(|line| line.contains(&port.to_string())),
        "{:?}",
        engine.client.log
    );
    // While it is not connected, requests are answered at once.
    engine.nats(NatsRequest::Streams);
    engine.until("the answer", |engine| engine.client.streams.is_some());
    assert!(engine.client.streams.clone().expect("answered").is_err());
    engine.send(Command::Close { session: CLIENT });

    // A second server on the same port fails with a message once the bind
    // retry runs out.
    let other = SessionId(3);
    engine.send(Command::Open {
        session: other,
        spec: SessionSpec::NatsServer(spec),
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let error = loop {
        assert!(Instant::now() < deadline, "the taken port is reported");
        match engine.events.try_recv() {
            Ok(Event {
                session,
                kind: EventKind::State(SessionState::Closed { error: Some(error) }),
            }) if session == other => break error,
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    assert!(error.contains(&port.to_string()), "{error}");

    engine.send(Command::Close { session: SERVER });
    engine.until("the server to stop", |engine| {
        matches!(engine.server.state, Some(SessionState::Closed { .. }))
    });
}
