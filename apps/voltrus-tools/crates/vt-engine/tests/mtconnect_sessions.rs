//! An MTConnect agent simulator and clients talking over loopback through
//! the engine, the way the app drives them: the probe becoming points, the
//! live stream keeping them current, conditions, values set on the machine,
//! an SHDR adapter, and losing the agent.

use std::io::Read;
use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    Command, Event, EventKind, MtconnectAgentSpec, MtconnectClientSpec, MtconnectTemplate,
    PointInfo, Quality, Reading, SessionId, SessionSpec, SessionState,
};

/// A TCP port nothing on this machine listens on.
fn free_port() -> u16 {
    std::net::TcpListener::bind("0.0.0.0:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .expect("a free TCP port")
}

/// The engine's events, kept until a test asks for them: two sessions talk at
/// once, and waiting for one must not lose the other's.
struct Events {
    receiver: async_channel::Receiver<Event>,
    kept: Vec<Event>,
    /// The last text reported for each point: one event carries every change
    /// of a poll, so a test cannot wait for each text on its own.
    texts: std::collections::HashMap<(SessionId, u32), String>,
}

impl Events {
    fn new(receiver: async_channel::Receiver<Event>) -> Self {
        Self {
            receiver,
            kept: Vec::new(),
            texts: std::collections::HashMap::new(),
        }
    }

    /// The first event of `session` (kept or new) that `accept` maps to a
    /// value; it and every earlier event of the session are consumed.
    fn wait<T>(
        &mut self,
        session: SessionId,
        mut accept: impl FnMut(&EventKind) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut scanned = 0;
        loop {
            while scanned < self.kept.len() {
                let event = &self.kept[scanned];
                if event.session == session
                    && let Some(found) = accept(&event.kind)
                {
                    let mut index = 0;
                    self.kept.retain(|event| {
                        index += 1;
                        index > scanned + 1 || event.session != session
                    });
                    return found;
                }
                scanned += 1;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for an event from {session}; it sent: {:#?}",
                self.kept
                    .iter()
                    .filter(|event| event.session == session
                        && !matches!(event.kind, EventKind::Traffic(_) | EventKind::Stats(_)))
                    .map(|event| &event.kind)
                    .collect::<Vec<_>>()
            );
            match self.receiver.try_recv() {
                Ok(event) => {
                    if let EventKind::Payloads(texts) = &event.kind {
                        for text in texts {
                            self.texts
                                .insert((event.session, text.point), text.text.clone());
                        }
                    }
                    self.kept.push(event);
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

fn values(kind: &EventKind) -> Option<Vec<Reading>> {
    match kind {
        EventKind::Values(frame) => Some(frame.readings.clone()),
        _ => None,
    }
}

fn state(events: &mut Events, session: SessionId, want: fn(&SessionState) -> bool) -> SessionState {
    events.wait(session, |kind| match kind {
        EventKind::State(state) if want(state) => Some(state.clone()),
        _ => None,
    })
}

fn listening(events: &mut Events, session: SessionId) {
    let reached = state(events, session, |state| {
        matches!(
            state,
            SessionState::Listening { .. } | SessionState::Closed { .. }
        )
    });
    assert!(
        matches!(reached, SessionState::Listening { .. }),
        "the simulator failed: {reached:?}"
    );
}

fn written(events: &mut Events, session: SessionId, point: u32) -> Result<(), String> {
    events.wait(session, |kind| match kind {
        EventKind::Written {
            point: written,
            result,
        } if *written == point => Some(result.clone()),
        _ => None,
    })
}

/// Wait until `session` reads `point` within 0.01 of `expected`.
fn reads(events: &mut Events, session: SessionId, point: u32, expected: f64) {
    let index = usize::try_from(point).expect("an index");
    events.wait(session, |kind| {
        values(kind).and_then(|readings| {
            let reading = readings[index];
            (reading.quality == Quality::Good && (reading.value - expected).abs() < 0.01)
                .then_some(())
        })
    });
}

fn points_of(events: &mut Events, session: SessionId) -> Vec<PointInfo> {
    events.wait(session, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    })
}

fn point_named(points: &[PointInfo], name: &str) -> u32 {
    points
        .iter()
        .find(|point| point.name == name)
        .unwrap_or_else(|| panic!("a point named {name}"))
        .id
}

/// Wait until the text of `point` is `expected` (a point never reported has
/// none).
fn says(events: &mut Events, session: SessionId, point: u32, expected: &str) {
    says_where(events, session, point, |text| text == expected);
}

fn says_where(events: &mut Events, session: SessionId, point: u32, accept: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let current = events
            .texts
            .get(&(session, point))
            .map_or("", String::as_str);
        if accept(current) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "point {point} of {session} says {current:?}"
        );
        match events.receiver.try_recv() {
            Ok(event) => {
                if let EventKind::Payloads(texts) = &event.kind {
                    for text in texts {
                        events
                            .texts
                            .insert((event.session, text.point), text.text.clone());
                    }
                }
                events.kept.push(event);
            }
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn agent(port: u16, template: MtconnectTemplate) -> MtconnectAgentSpec {
    MtconnectAgentSpec {
        port,
        template,
        ..MtconnectAgentSpec::default()
    }
}

fn client(port: u16) -> MtconnectClientSpec {
    MtconnectClientSpec {
        interval_ms: 100,
        timeout_ms: 2000,
        ..templates::mtconnect_client_for("127.0.0.1", port)
    }
}

#[test]
fn a_client_streams_a_simulated_machine_and_sees_what_is_set_on_it() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let simulator = SessionId(1);
    let watcher = SessionId(2);

    engine.send(Command::Open {
        session: simulator,
        spec: SessionSpec::MtconnectAgent(agent(port, MtconnectTemplate::Vmc)),
    });
    listening(&mut events, simulator);
    let served = points_of(&mut events, simulator);
    let x = point_named(&served, "Axes/X/Xact");
    assert!(
        served[x as usize].writable,
        "samples can be set on the machine"
    );
    assert_eq!(served[x as usize].unit, "mm");

    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::MtconnectClient(client(port)),
    });
    let points = points_of(&mut events, watcher);
    assert_eq!(points.len(), served.len(), "every data item is a point");
    assert!(
        points.iter().all(|point| !point.writable),
        "an agent is read-only"
    );
    let x = point_named(&points, "Axes/X/Xact");
    let estop = point_named(&points, "Controller/estop");
    let system = point_named(&points, "Controller/system");
    let exec = point_named(&points, "Controller/path/execution");
    assert!(points[estop as usize].boolean);
    assert_eq!(points[system as usize].states.len(), 3);
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Connected)
    });
    // Every point has a value from the first snapshot.
    events.wait(watcher, |kind| {
        values(kind).and_then(|readings| {
            readings
                .iter()
                .all(|reading| reading.quality == Quality::Good)
                .then_some(())
        })
    });
    says(&mut events, watcher, exec, "READY");

    // A value forced on the machine reaches the client through the stream.
    engine.send(Command::Write {
        session: simulator,
        point: point_named(&served, "Axes/X/Xact"),
        value: 12.5,
    });
    assert_eq!(
        written(&mut events, simulator, point_named(&served, "Axes/X/Xact")),
        Ok(())
    );
    reads(&mut events, watcher, x, 12.5);

    // So does a fault, with the controller's own code and message.
    engine.send(Command::Write {
        session: simulator,
        point: point_named(&served, "Controller/system"),
        value: 3.0,
    });
    assert_eq!(
        written(
            &mut events,
            simulator,
            point_named(&served, "Controller/system")
        ),
        Ok(())
    );
    reads(&mut events, watcher, system, 3.0);
    says(&mut events, watcher, system, "F1001: Simulated fault");
    engine.send(Command::Write {
        session: simulator,
        point: point_named(&served, "Controller/system"),
        value: 1.0,
    });
    reads(&mut events, watcher, system, 1.0);
    says(&mut events, watcher, system, "");

    // The emergency stop is pressed by writing 1.
    engine.send(Command::Write {
        session: simulator,
        point: point_named(&served, "Controller/estop"),
        value: 1.0,
    });
    reads(&mut events, watcher, estop, 1.0);

    // A machine's own values are not the app's to set, and a client writes nothing.
    engine.send(Command::Write {
        session: simulator,
        point: point_named(&served, "Controller/path/execution"),
        value: 1.0,
    });
    assert!(
        written(
            &mut events,
            simulator,
            point_named(&served, "Controller/path/execution")
        )
        .is_err()
    );
    engine.send(Command::Write {
        session: watcher,
        point: x,
        value: 1.0,
    });
    assert!(written(&mut events, watcher, x).is_err());

    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Close { session: simulator });
    state(&mut events, simulator, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn the_lathe_has_no_y_axis_and_holding_a_value_is_reported() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let simulator = SessionId(1);
    engine.send(Command::Open {
        session: simulator,
        spec: SessionSpec::MtconnectAgent(agent(free_port(), MtconnectTemplate::Lathe)),
    });
    listening(&mut events, simulator);
    let served = points_of(&mut events, simulator);
    assert!(served.iter().all(|point| point.address != "Ypos"));
    let z = point_named(&served, "Axes/Z/Zact");
    engine.send(Command::Lock {
        session: simulator,
        point: z,
        locked: true,
    });
    events.wait(simulator, |kind| match kind {
        EventKind::Locked {
            point,
            locked: true,
        } if *point == z => Some(()),
        _ => None,
    });
    engine.send(Command::Lock {
        session: simulator,
        point: z,
        locked: false,
    });
    events.wait(simulator, |kind| match kind {
        EventKind::Locked {
            point,
            locked: false,
        } if *point == z => Some(()),
        _ => None,
    });
    engine.send(Command::Shutdown);
}

#[test]
fn the_shdr_adapter_streams_the_machines_observations() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let simulator = SessionId(1);
    let shdr_port = free_port();
    engine.send(Command::Open {
        session: simulator,
        spec: SessionSpec::MtconnectAgent(MtconnectAgentSpec {
            shdr: true,
            shdr_port,
            ..agent(free_port(), MtconnectTemplate::Vmc)
        }),
    });
    listening(&mut events, simulator);
    let mut adapter =
        std::net::TcpStream::connect(("127.0.0.1", shdr_port)).expect("the adapter port");
    adapter
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a timeout");
    let mut text = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !text.contains("|Xpos|") && Instant::now() < deadline {
        let mut buffer = [0u8; 4096];
        match adapter.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => text.push_str(&String::from_utf8_lossy(&buffer[..read])),
            Err(_) => {}
        }
    }
    assert!(text.contains("|Xpos|"), "SHDR lines: {text}");
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_without_an_agent_keeps_trying_until_closed() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let watcher = SessionId(1);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::MtconnectClient(client(free_port())),
    });
    let reconnecting = state(&mut events, watcher, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    let SessionState::Reconnecting { attempt, .. } = reconnecting else {
        unreachable!("matched above");
    };
    assert_eq!(attempt, 1);
    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_with_a_bad_address_says_so() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let watcher = SessionId(1);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::MtconnectClient(MtconnectClientSpec {
            url: "ftp://machine".into(),
            ..MtconnectClientSpec::default()
        }),
    });
    let closed = state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    assert!(matches!(closed, SessionState::Closed { error: Some(_) }));
    engine.send(Command::Shutdown);
}

/// The public demo agent of the MTConnect Institute. Run by hand:
/// `cargo test -p vt-engine --test mtconnect_sessions -- --ignored`.
#[test]
#[ignore = "needs the internet"]
fn a_client_streams_the_public_demo_agent() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let watcher = SessionId(1);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::MtconnectClient(MtconnectClientSpec {
            url: "https://demo.mtconnect.org".into(),
            ..MtconnectClientSpec::default()
        }),
    });
    let points = points_of(&mut events, watcher);
    assert!(!points.is_empty(), "the demo agent has data items");
    events.wait(watcher, |kind| {
        values(kind).and_then(|readings: Vec<Reading>| {
            readings
                .iter()
                .any(|reading| reading.quality == Quality::Good && reading.value.is_finite())
                .then_some(())
        })
    });
    engine.send(Command::Shutdown);
}
