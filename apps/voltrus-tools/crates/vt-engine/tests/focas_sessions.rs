//! A FANUC CNC simulator (`focas-proto`) and clients talking over loopback
//! through the engine, the way the app drives them: the controller's status,
//! axes, spindle and alarms as points, macro writes, controls set on the
//! simulator, and losing the controller.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    Command, Event, EventKind, FocasProfile, FocasSimulatorSpec, PointInfo, Quality, Reading,
    SessionId, SessionSpec, SessionState,
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

fn simulator(port: u16, profile: FocasProfile) -> FocasSimulatorSpec {
    FocasSimulatorSpec {
        port,
        profile,
        ..FocasSimulatorSpec::default()
    }
}

fn client(port: u16) -> vt_protocol::FocasClientSpec {
    vt_protocol::FocasClientSpec {
        poll_ms: 100,
        timeout_ms: 1000,
        ..templates::focas_client_for("127.0.0.1", port)
    }
}

#[test]
fn a_client_polls_a_simulated_cnc_writes_a_macro_and_sees_the_controls() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let cnc = SessionId(1);
    let watcher = SessionId(2);

    engine.send(Command::Open {
        session: cnc,
        spec: SessionSpec::FocasSimulator(simulator(port, FocasProfile::Series31iB)),
    });
    listening(&mut events, cnc);
    let controls = points_of(&mut events, cnc);
    let cycle = point_named(&controls, "Cycle start");
    let emergency = point_named(&controls, "Emergency stop");
    let alarm = point_named(&controls, "Alarm P/S 100");
    assert!(controls[cycle as usize].writable);

    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::FocasClient(client(port)),
    });
    let points = points_of(&mut events, watcher);
    let identity = point_named(&points, "Controller");
    let running = point_named(&points, "Cycle running");
    let estop = point_named(&points, "Emergency stop");
    let alarmed = point_named(&points, "Alarm active");
    let mode = point_named(&points, "Mode");
    let macro_500 = point_named(&points, "Macro #500");
    let x = point_named(&points, "Absolute X");
    assert_eq!(points[x as usize].unit, "mm");
    assert!(points[macro_500 as usize].writable);
    assert!(!points[running as usize].writable);
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Connected)
    });
    says(&mut events, watcher, identity, "31 M G421 42.0");
    says(&mut events, watcher, mode, "MEM");
    // The demo program is cutting: the cycle runs and the axes move.
    reads(&mut events, watcher, running, 1.0);
    events.wait(watcher, |kind| {
        values(kind).and_then(|readings: Vec<Reading>| {
            let feed = readings[point_named(&points, "Feed rate") as usize];
            (feed.quality == Quality::Good && feed.value > 0.0).then_some(())
        })
    });

    // A macro variable is written and read back.
    engine.send(Command::Write {
        session: watcher,
        point: macro_500,
        value: 7.0,
    });
    assert_eq!(written(&mut events, watcher, macro_500), Ok(()));
    reads(&mut events, watcher, macro_500, 7.0);
    engine.send(Command::Write {
        session: watcher,
        point: x,
        value: 1.0,
    });
    assert!(
        written(&mut events, watcher, x).is_err(),
        "only macros are written"
    );

    // The simulator's controls act like an operator's.
    engine.send(Command::Write {
        session: cnc,
        point: alarm,
        value: 1.0,
    });
    assert_eq!(written(&mut events, cnc, alarm), Ok(()));
    reads(&mut events, watcher, alarmed, 1.0);
    reads(&mut events, watcher, running, 0.0);
    engine.send(Command::Write {
        session: cnc,
        point: cycle,
        value: 1.0,
    });
    assert!(
        written(&mut events, cnc, cycle).is_err(),
        "an alarm stops the cycle from starting"
    );
    engine.send(Command::Write {
        session: cnc,
        point: alarm,
        value: 0.0,
    });
    reads(&mut events, watcher, alarmed, 0.0);
    engine.send(Command::Write {
        session: cnc,
        point: emergency,
        value: 1.0,
    });
    reads(&mut events, watcher, estop, 1.0);
    engine.send(Command::Write {
        session: cnc,
        point: emergency,
        value: 0.0,
    });
    reads(&mut events, watcher, estop, 0.0);
    engine.send(Command::Write {
        session: cnc,
        point: cycle,
        value: 1.0,
    });
    assert_eq!(written(&mut events, cnc, cycle), Ok(()));
    reads(&mut events, watcher, running, 1.0);

    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Close { session: cnc });
    state(&mut events, cnc, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn the_alarm_text_reaches_the_client() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let cnc = SessionId(1);
    let watcher = SessionId(2);
    engine.send(Command::Open {
        session: cnc,
        spec: SessionSpec::FocasSimulator(simulator(port, FocasProfile::Series0iF)),
    });
    listening(&mut events, cnc);
    let controls = points_of(&mut events, cnc);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::FocasClient(client(port)),
    });
    let points = points_of(&mut events, watcher);
    // The 0i-F differs from the 31i-B in its identity.
    says(
        &mut events,
        watcher,
        point_named(&points, "Controller"),
        "0 M D4F1 12.0",
    );
    engine.send(Command::Write {
        session: cnc,
        point: point_named(&controls, "Alarm P/S 100"),
        value: 1.0,
    });
    let alarms = point_named(&points, "Alarm messages");
    says_where(&mut events, watcher, alarms, |text| text.contains("100"));
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_without_a_cnc_keeps_trying_until_closed() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let watcher = SessionId(1);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::FocasClient(client(free_port())),
    });
    let reconnecting = state(&mut events, watcher, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    assert!(matches!(
        reconnecting,
        SessionState::Reconnecting { attempt: 1, .. }
    ));
    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_finds_a_cnc_that_starts_later() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let watcher = SessionId(1);
    let cnc = SessionId(2);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::FocasClient(client(port)),
    });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    engine.send(Command::Open {
        session: cnc,
        spec: SessionSpec::FocasSimulator(simulator(port, FocasProfile::Series31iB)),
    });
    listening(&mut events, cnc);
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Connected)
    });
    engine.send(Command::Shutdown);
}

#[test]
fn a_simulator_on_a_taken_port_says_so() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = held.local_addr().expect("an address").port();
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let cnc = SessionId(1);
    engine.send(Command::Open {
        session: cnc,
        spec: SessionSpec::FocasSimulator(simulator(port, FocasProfile::Series31iB)),
    });
    let closed = state(&mut events, cnc, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    let SessionState::Closed { error: Some(error) } = closed else {
        panic!("the simulator should have failed: {closed:?}");
    };
    assert!(error.contains("in use"), "{error}");
    engine.send(Command::Shutdown);
}
