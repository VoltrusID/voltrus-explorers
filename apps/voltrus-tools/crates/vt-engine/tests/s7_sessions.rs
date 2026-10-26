//! An S7 CPU simulator (`mt-s7`'s server) and clients talking over loopback
//! through the engine, the way the app drives them: polling, writes, held
//! values, the CPU report, traffic, and losing and finding the CPU again.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    Command, Direction, Event, EventKind, PointInfo, Quality, Reading, S7ClientSpec, S7PlcReport,
    S7ServerSpec, S7TagSpec, S7Type, SessionId, SessionSpec, SessionState,
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
}

impl Events {
    fn new(receiver: async_channel::Receiver<Event>) -> Self {
        Self {
            receiver,
            kept: Vec::new(),
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
                Ok(event) => self.kept.push(event),
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

fn plc_report(events: &mut Events, session: SessionId) -> Result<S7PlcReport, String> {
    events.wait(session, |kind| match kind {
        EventKind::S7Plc(report) => Some(report.clone()),
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

#[test]
fn a_client_polls_writes_and_diagnoses_a_simulated_cpu() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let template = templates::find_s7("motor").expect("the motor template");
    let port = free_port();
    let simulator = SessionId(1);
    let client = SessionId(2);
    let server = S7ServerSpec {
        port,
        template: template.id.clone(),
        pdu_size: 480,
    };

    engine.send(Command::Open {
        session: simulator,
        spec: SessionSpec::S7Server(server.clone()),
    });
    listening(&mut events, simulator);
    let served: Vec<PointInfo> = events.wait(simulator, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    });
    assert_eq!(served.len(), template.points.len());

    // A CPU that offers 480 bytes agrees to the 240 the client asks for.
    let spec = S7ClientSpec {
        pdu_size: 240,
        poll_ms: 200,
        ..templates::s7_client_for(template, "127.0.0.1", port)
    };
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::S7Client(spec),
    });
    let points: Vec<PointInfo> = events.wait(client, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    });
    assert_eq!(points.len(), template.points.len());
    let speed = point_named(&points, "Speed");
    let setpoint = point_named(&points, "Speed setpoint");
    let hours = point_named(&points, "Run hours");
    let drive = point_named(&points, "Drive name");
    let running = point_named(&points, "Running");
    assert!(points[running as usize].boolean);
    assert!(!points[drive as usize].writable, "strings are read-only");
    assert_eq!(points[speed as usize].address, "DB1.DBD0");
    assert_eq!(points[speed as usize].data_type, "Real");
    assert_eq!(
        state(&mut events, client, |state| state.is_live()),
        SessionState::Connected
    );

    // The CPU's identity, mode, diagnostic buffer and blocks come first.
    let report = plc_report(&mut events, client).expect("a report");
    assert_eq!(report.pdu_size, 240);
    let cpu = report.cpu.expect("the CPU identity");
    assert!(
        !cpu.module_type.is_empty() && !cpu.order_code.is_empty(),
        "{cpu:?}"
    );
    assert!(!report.mode.expect("the operating mode").is_empty());
    assert!(
        !report
            .diagnostics
            .expect("the diagnostic buffer")
            .is_empty()
    );
    let blocks = report.blocks.expect("the block list");
    for number in [1, 2] {
        assert!(
            blocks
                .iter()
                .any(|block| block.kind == "DB" && block.number == number),
            "DB{number} in {blocks:?}"
        );
    }

    // Each poll sends a string's text (when it changes) before the values,
    // and waiting consumes what came before: the text first.
    let text = events.wait(client, |kind| match kind {
        EventKind::Payloads(texts) => texts
            .iter()
            .find(|text| text.point == drive)
            .map(|text| text.text.clone()),
        _ => None,
    });
    assert_eq!(text, "M-101 conveyor drive");
    // Every tag reads good.
    let readings = events.wait(client, |kind| {
        values(kind).filter(|readings| {
            readings
                .iter()
                .all(|reading| reading.quality == Quality::Good)
        })
    });
    assert!((1400.0..=1500.0).contains(&readings[speed as usize].value));
    assert_eq!(readings[running as usize].value, 1.0);

    // A write lands in the CPU, which holds it against its simulation.
    engine.send(Command::Write {
        session: client,
        point: setpoint,
        value: 1234.5,
    });
    assert_eq!(written(&mut events, client, setpoint), Ok(()));
    events.wait(simulator, |kind| {
        matches!(kind, EventKind::Locked { point, locked: true } if *point == setpoint)
            .then_some(())
    });
    reads(&mut events, client, setpoint, 1234.5);
    reads(&mut events, simulator, setpoint, 1234.5);

    // Values that do not fit the type never reach the wire.
    for (point, value) in [(hours, 70_000.0), (hours, 1.5), (drive, 1.0)] {
        engine.send(Command::Write {
            session: client,
            point,
            value,
        });
        assert!(
            written(&mut events, client, point).is_err(),
            "{value} to point {point}"
        );
    }

    // Setting a sensor from the simulator holds it there for every client.
    engine.send(Command::Write {
        session: simulator,
        point: speed,
        value: 42.0,
    });
    assert_eq!(written(&mut events, simulator, speed), Ok(()));
    reads(&mut events, client, speed, 42.0);

    // The traffic shows what went over the wire, both ways.
    let frames = events.wait(client, |kind| match kind {
        EventKind::Traffic(frames) if frames.iter().any(|frame| frame.summary == "Read Var") => {
            Some(frames.clone())
        }
        _ => None,
    });
    assert!(
        frames
            .iter()
            .any(|frame| frame.direction == Direction::Tx && frame.detail.contains("DB1.DBD0"))
    );
    assert!(frames.iter().any(|frame| frame.direction == Direction::Rx));
    events.wait(simulator, |kind| match kind {
        EventKind::Traffic(frames) if frames.iter().any(|frame| frame.summary == "Read Var") => {
            Some(())
        }
        _ => None,
    });

    // Asked again, the client reads the CPU again.
    engine.send(Command::ReadS7Plc { session: client });
    assert!(plc_report(&mut events, client).is_ok());

    // Stopping the CPU drops the client, which retries; started again on
    // the same port, the client finds it.
    engine.send(Command::Close { session: simulator });
    state(&mut events, simulator, |state| !state.is_running());
    let lost = state(&mut events, client, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    let SessionState::Reconnecting { reason, .. } = lost else {
        unreachable!("waited for a retry");
    };
    assert!(!reason.is_empty());
    engine.send(Command::ReadS7Plc { session: client });
    assert_eq!(plc_report(&mut events, client), Err("Not connected".into()));
    engine.send(Command::Open {
        session: simulator,
        spec: SessionSpec::S7Server(server),
    });
    listening(&mut events, simulator);
    assert_eq!(
        state(&mut events, client, |state| state.is_live()),
        SessionState::Connected
    );

    engine.send(Command::Close { session: client });
    engine.send(Command::Close { session: simulator });
    state(&mut events, client, |state| !state.is_running());
    state(&mut events, simulator, |state| !state.is_running());
    // A session that is not running answers a PLC read at once.
    engine.send(Command::ReadS7Plc {
        session: SessionId(9),
    });
    assert_eq!(
        plc_report(&mut events, SessionId(9)),
        Err("Not connected".into())
    );
    engine.send(Command::Shutdown);
}

#[test]
fn a_missing_data_block_fails_only_its_own_tag() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let port = free_port();
    let simulator = SessionId(1);
    let client = SessionId(2);
    engine.send(Command::Open {
        session: simulator,
        spec: SessionSpec::S7Server(S7ServerSpec {
            port,
            template: "line".into(),
            ..S7ServerSpec::default()
        }),
    });
    listening(&mut events, simulator);

    let tag = |name: &str, address: &str, data_type| S7TagSpec {
        name: name.into(),
        address: address.parse().expect("an address"),
        data_type,
        unit: String::new(),
    };
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::S7Client(S7ClientSpec {
            host: "127.0.0.1".into(),
            port,
            poll_ms: 200,
            tags: vec![
                tag("Batch size", "DB5.DBW256", S7Type::Int),
                tag("Nowhere", "DB99.DBW0", S7Type::Word),
                tag("E-stop", "I0.1", S7Type::Bool),
            ],
            ..S7ClientSpec::default()
        }),
    });
    // The warning goes out before the values of the same poll.
    let warning = events.wait(client, |kind| match kind {
        EventKind::Log(entry) if entry.message.starts_with("DB99.DBW0") => {
            Some(entry.message.clone())
        }
        _ => None,
    });
    assert!(warning.contains("does not exist"), "{warning}");
    let readings = events.wait(client, values);
    assert_eq!(readings[0].quality, Quality::Good);
    assert_eq!(readings[0].value, 500.0);
    assert_eq!(readings[1].quality, Quality::Bad);
    assert_eq!(readings[2].value, 1.0);
    engine.send(Command::Shutdown);
}

#[test]
fn a_simulator_on_a_taken_port_says_so() {
    let (engine, receiver) = vt_engine::start().expect("the engine starts");
    let mut events = Events::new(receiver);
    let taken = std::net::TcpListener::bind("0.0.0.0:0").expect("a listener");
    let port = taken.local_addr().expect("its address").port();
    engine.send(Command::Open {
        session: SessionId(1),
        spec: SessionSpec::S7Server(S7ServerSpec {
            port,
            ..S7ServerSpec::default()
        }),
    });
    let closed = state(&mut events, SessionId(1), |state| !state.is_running());
    let SessionState::Closed { error: Some(error) } = closed else {
        panic!("expected a failure, got {closed:?}");
    };
    assert!(error.contains("in use"), "{error}");
    engine.send(Command::Shutdown);
}
