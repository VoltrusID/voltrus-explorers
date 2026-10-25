//! An MQTT broker simulator and a client talking over loopback through the
//! engine, the way the app drives them.

use std::time::{Duration, Instant};

use vt_protocol::{
    Command, Event, EventKind, MqttBrokerSpec, MqttQos, MqttSubscription, PointInfo, Quality,
    Reading, SessionId, SessionSpec, SessionState,
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
    /// Every values frame, oldest first.
    frames: Vec<Vec<Reading>>,
    written: Vec<(u32, Result<(), String>)>,
    published: Vec<(String, Result<(), String>)>,
    locked: Vec<(u32, bool)>,
    traffic: Vec<String>,
    log: Vec<String>,
}

impl Seen {
    fn apply(&mut self, kind: EventKind) {
        match kind {
            EventKind::State(state) => self.state = Some(state),
            EventKind::Points(points) => {
                self.readings = vec![Reading::bad(); points.len()];
                self.points = points;
            }
            EventKind::PointsAdded(points) => {
                for point in &points {
                    assert_eq!(
                        point.id as usize,
                        self.points.len(),
                        "ids continue in order"
                    );
                }
                self.points.extend(points);
            }
            EventKind::Values(frame) => {
                self.readings = frame.readings.clone();
                self.frames.push(frame.readings);
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
            EventKind::Stats(_) | EventKind::Payloads(_) => {}
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
}

struct Engine {
    engine: vt_engine::EngineHandle,
    events: async_channel::Receiver<Event>,
    broker: Seen,
    client: Seen,
}

const BROKER: SessionId = SessionId(1);
const CLIENT: SessionId = SessionId(2);

impl Engine {
    fn start() -> Self {
        let (engine, events) = vt_engine::start().expect("the engine starts");
        Self {
            engine,
            events,
            broker: Seen::default(),
            client: Seen::default(),
        }
    }

    fn send(&self, command: Command) {
        assert!(self.engine.send(command), "the engine is running");
    }

    /// Take events until `done` holds, or fail after 15 s.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            match self.events.try_recv() {
                Ok(event) if event.session == BROKER => self.broker.apply(event.kind),
                Ok(event) if event.session == CLIENT => self.client.apply(event.kind),
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

#[test]
fn a_client_watches_and_publishes_to_the_simulated_broker() {
    let mut engine = Engine::start();
    let port = free_port();
    let spec = MqttBrokerSpec {
        port,
        template: Some("sma-inverter".into()),
        prefix: "plant/inverter".into(),
        publish_ms: 100,
        sparkplug: true,
        ..MqttBrokerSpec::default()
    };
    engine.send(Command::Open {
        session: BROKER,
        spec: SessionSpec::MqttBroker(spec.clone()),
    });
    engine.until("the broker to listen", |engine| {
        matches!(engine.broker.state, Some(SessionState::Listening { .. }))
    });
    assert_eq!(
        engine.broker.points.len(),
        20,
        "one point per template value"
    );

    let mut client = spec.client();
    client.subscriptions.push(MqttSubscription {
        filter: "test/#".into(),
        qos: MqttQos::AtLeastOnce,
        json_path: String::new(),
    });
    engine.send(Command::Open {
        session: CLIENT,
        spec: SessionSpec::MqttClient(client),
    });

    // Plain topics, the JSON nameplate and the Sparkplug metrics (named and
    // given units by the births the client asks for) all become points.
    engine.until("the device's points", |engine| {
        let seen = &engine.client;
        seen.value("plant/inverter/ac_power").is_some()
            && seen.value("plant/inverter/status › online").is_some()
            && seen.value("Simulator/sma-inverter › AC Power").is_some()
    });
    let seen = &engine.client;
    assert_eq!(seen.state, Some(SessionState::Connected));
    let online = seen
        .point("plant/inverter/status › online")
        .expect("nameplate");
    assert!(
        online.boolean && !online.writable,
        "JSON fields are read-only"
    );
    assert_eq!(seen.value("plant/inverter/status › online"), Some(1.0));
    let sparkplug = seen
        .point("Simulator/sma-inverter › AC Power")
        .expect("a Sparkplug metric");
    assert_eq!(sparkplug.unit, "W", "the unit from the birth's engUnit");
    assert_eq!(sparkplug.data_type, "Double");
    let power = seen.value("plant/inverter/ac_power").expect("AC power");
    assert!((0.0..=60_000.0).contains(&power), "AC power {power}");
    assert!(
        seen.log
            .iter()
            .any(|line| line.starts_with("Asked Sparkplug node Voltrus/Simulator")),
        "the client asked the node for its births: {:?}",
        seen.log
    );
    assert!(
        engine
            .broker
            .log
            .iter()
            .any(|line| line.contains("asked for a Sparkplug rebirth")),
        "{:?}",
        engine.broker.log
    );

    // Values move as the simulation runs.
    let ac_power = engine
        .client
        .point("plant/inverter/ac_power")
        .expect("AC power")
        .id as usize;
    let first = engine.client.frames.len();
    engine.until("the values to change", |engine| {
        let frames = &engine.client.frames[first.min(engine.client.frames.len())..];
        frames
            .windows(2)
            .any(|pair| pair[0][ac_power].value != pair[1][ac_power].value)
    });

    // A write publishes to the value's topic; the simulator takes the value
    // and holds it, and the client sees it come back and stay.
    let temperature = engine
        .client
        .point("plant/inverter/temperature")
        .expect("the temperature topic")
        .clone();
    assert!(temperature.writable);
    engine.send(Command::Write {
        session: CLIENT,
        point: temperature.id,
        value: 61.5,
    });
    engine.until("the write to be held", |engine| {
        engine.client.written.contains(&(temperature.id, Ok(())))
            && engine.broker.locked.contains(&(15, true))
    });
    let held = engine.client.frames.len();
    engine.until("several frames after the hold", |engine| {
        engine.client.frames.len() >= held + 8
            && engine.client.value("plant/inverter/temperature") == Some(61.5)
    });
    assert_eq!(
        engine.client.value("plant/inverter/temperature"),
        Some(61.5)
    );

    // Publishing any topic lands on the broker and comes back to the
    // subscriber: a JSON payload's number becomes a point.
    engine.send(Command::Publish {
        session: CLIENT,
        topic: "test/limit".into(),
        payload: r#"{"kw": 42.5, "mode": "eco"}"#.into(),
        qos: MqttQos::AtLeastOnce,
        retain: true,
    });
    engine.until("the published message to come back", |engine| {
        engine.client.value("test/limit › kw") == Some(42.5)
    });
    assert_eq!(engine.client.published, [("test/limit".to_owned(), Ok(()))]);
    assert!(
        engine
            .broker
            .traffic
            .iter()
            .any(|line| line.starts_with("PUBLISH test/limit") && line.contains("retained")),
        "the broker saw the publish"
    );
    // A topic that is not one to publish to is refused before the wire.
    engine.send(Command::Publish {
        session: CLIENT,
        topic: "test/+".into(),
        payload: "1".into(),
        qos: MqttQos::AtMostOnce,
        retain: false,
    });
    engine.until("the refusal", |engine| engine.client.published.len() == 2);
    assert!(engine.client.published[1].1.is_err());

    // Stopping the broker frees its port at once: it starts again on it,
    // and the client notices the broker went away.
    engine.send(Command::Close { session: BROKER });
    engine.until("the broker to stop", |engine| {
        matches!(
            engine.broker.state,
            Some(SessionState::Closed { error: None })
        )
    });
    engine.until("the client to lose the broker", |engine| {
        matches!(engine.client.state, Some(SessionState::Reconnecting { .. }))
    });
    engine.broker = Seen::default();
    engine.send(Command::Open {
        session: BROKER,
        spec: SessionSpec::MqttBroker(spec),
    });
    engine.until("the broker to listen again", |engine| {
        matches!(engine.broker.state, Some(SessionState::Listening { .. }))
    });
    engine.until("the client to reconnect", |engine| {
        engine.client.state == Some(SessionState::Connected)
    });

    engine.send(Command::Close { session: CLIENT });
    engine.send(Command::Close { session: BROKER });
    engine.until("both to stop", |engine| {
        matches!(engine.client.state, Some(SessionState::Closed { .. }))
            && matches!(engine.broker.state, Some(SessionState::Closed { .. }))
    });
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_without_a_broker_keeps_retrying() {
    let mut engine = Engine::start();
    let mut client = MqttBrokerSpec {
        port: free_port(),
        ..MqttBrokerSpec::default()
    }
    .client();
    client.keep_alive_s = 5;
    engine.send(Command::Open {
        session: CLIENT,
        spec: SessionSpec::MqttClient(client),
    });
    engine.until("a retry", |engine| {
        matches!(engine.client.state, Some(SessionState::Reconnecting { .. }))
    });
    engine.send(Command::Publish {
        session: CLIENT,
        topic: "a/b".into(),
        payload: "1".into(),
        qos: MqttQos::AtMostOnce,
        retain: false,
    });
    engine.until("the publish to be refused", |engine| {
        !engine.client.published.is_empty()
    });
    assert_eq!(
        engine.client.published[0],
        ("a/b".to_owned(), Err("Not connected".to_owned()))
    );
    engine.send(Command::Close { session: CLIENT });
    engine.until("it to stop", |engine| {
        engine.client.state == Some(SessionState::Closed { error: None })
    });
}
