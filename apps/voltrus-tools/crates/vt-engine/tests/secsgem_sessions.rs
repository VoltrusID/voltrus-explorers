//! A SECS/GEM equipment simulator (`mt-secsgem`) and hosts talking over
//! loopback through the engine, the way the app drives them: status
//! variables and equipment constants as points, constant writes, the
//! simulator's controls, events and alarms in the log, and the SML of every
//! message in the traffic view.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    Command, Event, EventKind, PointInfo, Quality, Reading, SecsGemSimulatorSpec, SessionId,
    SessionSpec, SessionState,
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

fn simulator(port: u16) -> SecsGemSimulatorSpec {
    SecsGemSimulatorSpec {
        port,
        run: false,
        ..SecsGemSimulatorSpec::default()
    }
}

fn host(port: u16) -> vt_protocol::SecsGemClientSpec {
    vt_protocol::SecsGemClientSpec {
        poll_ms: 100,
        timeout_ms: 2000,
        ..templates::secsgem_client_for("127.0.0.1", port, 0)
    }
}

/// Wait for a log line of `session` that contains `needle`.
fn logs(events: &mut Events, session: SessionId, needle: &str) -> String {
    events.wait(session, |kind| match kind {
        EventKind::Log(entry) if entry.message.contains(needle) => Some(entry.message.clone()),
        _ => None,
    })
}

#[test]
fn a_host_reads_a_simulated_chamber_writes_a_constant_and_sees_events_and_alarms() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let chamber = SessionId(1);
    let watcher = SessionId(2);

    engine.send(Command::Open {
        session: chamber,
        spec: SessionSpec::SecsGemSimulator(simulator(port)),
    });
    listening(&mut events, chamber);
    let controls = points_of(&mut events, chamber);
    let running = point_named(&controls, "Process running");
    let alarm = point_named(&controls, "Alarm: Chamber over temperature");
    assert!(controls[running as usize].writable && controls[running as usize].boolean);

    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::SecsGemClient(host(port)),
    });
    let points = points_of(&mut events, watcher);
    let temperature = point_named(&points, "ChamberTemperature");
    let process = point_named(&points, "ProcessState");
    let ppid = point_named(&points, "ActivePPID");
    let setpoint = point_named(&points, "ProcessTemperature");
    let alarms_set = point_named(&points, "AlarmsSet");
    assert_eq!(points[temperature as usize].unit, "degC");
    assert_eq!(points[temperature as usize].data_type, "F4");
    assert!(
        !points[temperature as usize].writable,
        "status variables are read-only"
    );
    assert!(
        points[setpoint as usize].writable,
        "equipment constants are writable"
    );
    assert_eq!(points[setpoint as usize].address, "ECID 1001");
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Connected)
    });
    let connected = logs(&mut events, watcher, "Connected to");
    assert!(connected.contains("VOLTRUS-CHAMBER"), "{connected}");

    // Every message is in the traffic view as SML.
    let frame = events.wait(watcher, |kind| match kind {
        EventKind::Traffic(frames) => frames
            .iter()
            .find(|frame| frame.detail.starts_with("S1F13"))
            .cloned(),
        _ => None,
    });
    assert!(frame.summary.starts_with("S1F13"), "{}", frame.summary);
    assert!(frame.detail.contains("<L"), "{}", frame.detail);

    reads(&mut events, watcher, process, 0.0);
    reads(&mut events, watcher, temperature, 22.0);

    // A constant is written and read back; refusals say why.
    engine.send(Command::Write {
        session: watcher,
        point: setpoint,
        value: 200.0,
    });
    assert_eq!(written(&mut events, watcher, setpoint), Ok(()));
    reads(&mut events, watcher, setpoint, 200.0);
    engine.send(Command::Write {
        session: watcher,
        point: setpoint,
        value: 999.0,
    });
    let refused = written(&mut events, watcher, setpoint).expect_err("out of range");
    assert!(refused.contains("20 to 400"), "{refused}");
    engine.send(Command::Write {
        session: watcher,
        point: temperature,
        value: 1.0,
    });
    assert!(
        written(&mut events, watcher, temperature)
            .expect_err("read-only")
            .contains("read-only")
    );

    // Starting a process on the chamber is seen as a status change and as an
    // event report.
    engine.send(Command::Write {
        session: chamber,
        point: running,
        value: 1.0,
    });
    assert_eq!(written(&mut events, chamber, running), Ok(()));
    let event = logs(&mut events, watcher, "Event 100 reported");
    assert!(event.contains('<'), "the report carries values: {event}");
    reads(&mut events, watcher, process, 1.0);
    says(&mut events, watcher, ppid, "WAFER_CLEAN_A");
    events.wait(watcher, |kind| {
        values(kind).and_then(|readings: Vec<Reading>| {
            let hot = readings[temperature as usize];
            (hot.quality == Quality::Good && hot.value > 30.0).then_some(())
        })
    });

    // An alarm is reported by S5F1.
    engine.send(Command::Write {
        session: chamber,
        point: alarm,
        value: 1.0,
    });
    assert_eq!(written(&mut events, chamber, alarm), Ok(()));
    let set = logs(&mut events, watcher, "Alarm 1 set");
    assert!(set.contains("Chamber over temperature"), "{set}");
    reads(&mut events, watcher, alarms_set, 1.0);
    engine.send(Command::Write {
        session: chamber,
        point: alarm,
        value: 0.0,
    });
    logs(&mut events, watcher, "Alarm 1 cleared");

    // Stopping the process.
    engine.send(Command::Write {
        session: chamber,
        point: running,
        value: 0.0,
    });
    assert_eq!(written(&mut events, chamber, running), Ok(()));
    reads(&mut events, watcher, process, 0.0);

    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Close { session: chamber });
    state(&mut events, chamber, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn the_simulator_keeps_processing_wafers_and_refuses_what_an_operator_could_not_do() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let chamber = SessionId(1);

    engine.send(Command::Open {
        session: chamber,
        spec: SessionSpec::SecsGemSimulator(SecsGemSimulatorSpec {
            port,
            ..SecsGemSimulatorSpec::default()
        }),
    });
    listening(&mut events, chamber);
    let points = points_of(&mut events, chamber);
    let running = point_named(&points, "Process running");
    let paused = point_named(&points, "Process paused");
    let state_point = point_named(&points, "ProcessState");
    let time = point_named(&points, "ProcessTime");
    let wafers = point_named(&points, "WaferCount");

    // `run` starts a process as soon as the chamber is idle.
    reads(&mut events, chamber, running, 1.0);
    reads(&mut events, chamber, state_point, 1.0);
    engine.send(Command::Write {
        session: chamber,
        point: running,
        value: 1.0,
    });
    assert!(
        written(&mut events, chamber, running)
            .expect_err("already processing")
            .contains("already")
    );
    engine.send(Command::Write {
        session: chamber,
        point: paused,
        value: 1.0,
    });
    assert_eq!(written(&mut events, chamber, paused), Ok(()));
    reads(&mut events, chamber, state_point, 2.0);
    engine.send(Command::Write {
        session: chamber,
        point: paused,
        value: 0.0,
    });
    assert_eq!(written(&mut events, chamber, paused), Ok(()));
    // A short process completes a wafer, then the next one starts.
    engine.send(Command::Write {
        session: chamber,
        point: time,
        value: 1.0,
    });
    assert_eq!(written(&mut events, chamber, time), Ok(()));
    engine.send(Command::Write {
        session: chamber,
        point: running,
        value: 0.0,
    });
    assert_eq!(written(&mut events, chamber, running), Ok(()));
    engine.send(Command::Write {
        session: chamber,
        point: running,
        value: 1.0,
    });
    assert_eq!(written(&mut events, chamber, running), Ok(()));
    events.wait(chamber, |kind| {
        values(kind).and_then(|readings: Vec<Reading>| {
            let count = readings[wafers as usize];
            (count.quality == Quality::Good && count.value >= 1.0).then_some(())
        })
    });
    engine.send(Command::Write {
        session: chamber,
        point: wafers,
        value: 1.0,
    });
    assert!(
        written(&mut events, chamber, wafers).is_err(),
        "a status variable"
    );

    engine.send(Command::Close { session: chamber });
    state(&mut events, chamber, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn a_host_reconnects_after_the_equipment_goes_away() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let chamber = SessionId(1);
    let watcher = SessionId(2);

    engine.send(Command::Open {
        session: chamber,
        spec: SessionSpec::SecsGemSimulator(simulator(port)),
    });
    listening(&mut events, chamber);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::SecsGemClient(host(port)),
    });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Connected)
    });
    engine.send(Command::Close { session: chamber });
    state(&mut events, chamber, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    engine.send(Command::Open {
        session: chamber,
        spec: SessionSpec::SecsGemSimulator(simulator(port)),
    });
    listening(&mut events, chamber);
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Connected)
    });
    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Close { session: chamber });
    engine.send(Command::Shutdown);
}

#[test]
fn a_host_with_nothing_to_connect_to_keeps_trying_and_closes_on_request() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let watcher = SessionId(1);
    engine.send(Command::Open {
        session: watcher,
        spec: SessionSpec::SecsGemClient(host(free_port())),
    });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    engine.send(Command::Close { session: watcher });
    state(&mut events, watcher, |state| {
        matches!(state, SessionState::Closed { error: None })
    });
    engine.send(Command::Shutdown);
}
