//! A DNP3 outstation simulator and a master talking over loopback TCP
//! through the engine, the way the app drives them: integrity and event
//! polls, flags as quality, controls (select-before-operate and direct
//! operate) whose feedback comes back as events, time sync and a restart.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    Command, Dnp3Action, Dnp3ControlMode, Dnp3Crob, Dnp3CrobCode, Dnp3MasterSpec, Dnp3Operation,
    Dnp3OutstationSpec, Event, EventKind, PointInfo, Quality, Reading, SessionId, SessionSpec,
    SessionState,
};

/// A TCP port nothing on this machine listens on.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
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
                "timed out waiting for an event from {session}"
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

fn id_of(key: &str) -> u32 {
    let plant = templates::find_dnp3("feeder").expect("the feeder plant");
    let index = plant
        .points
        .iter()
        .position(|point| point.key == key)
        .expect("a point of the plant");
    u32::try_from(index).expect("a point id")
}

fn slot(key: &str) -> usize {
    usize::try_from(id_of(key)).expect("an index")
}

/// Wait until the master reads `key` good and `test` accepts its value.
fn reads(events: &mut Events, session: SessionId, key: &str, test: impl Fn(f64) -> bool) -> f64 {
    let index = slot(key);
    events.wait(session, |kind| {
        values(kind).and_then(|readings| {
            let reading = *readings.get(index)?;
            (reading.quality == Quality::Good && test(reading.value)).then_some(reading.value)
        })
    })
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

fn done(events: &mut Events, session: SessionId, action: Dnp3Action) -> Result<String, String> {
    events.wait(session, |kind| match kind {
        EventKind::Dnp3Done {
            action: finished,
            result,
        } if *finished == action => Some(result.clone()),
        _ => None,
    })
}

fn listening(events: &mut Events, session: SessionId) -> Result<String, String> {
    events.wait(session, |kind| match kind {
        EventKind::State(SessionState::Listening { address }) => Some(Ok(address.clone())),
        EventKind::State(SessionState::Closed { error }) => {
            Some(Err(error.clone().unwrap_or_default()))
        }
        _ => None,
    })
}

fn outstation(port: u16) -> SessionSpec {
    SessionSpec::Dnp3Outstation(Dnp3OutstationSpec {
        bind: "127.0.0.1".into(),
        port,
        ..Dnp3OutstationSpec::default()
    })
}

fn master(port: u16) -> Dnp3MasterSpec {
    let plant = templates::find_dnp3("feeder").expect("the feeder plant");
    Dnp3MasterSpec {
        event_ms: 200,
        ..templates::dnp3_master_for(plant, "127.0.0.1", port, 10, 1)
    }
}

#[test]
fn master_polls_commands_and_syncs_a_simulated_outstation() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let plant = templates::find_dnp3("feeder").expect("the feeder plant");
    let port = free_port();
    let simulator = SessionId(1);
    let client = SessionId(2);

    engine.send(Command::Open {
        session: simulator,
        spec: outstation(port),
    });
    assert_eq!(
        listening(&mut events, simulator),
        Ok(format!("127.0.0.1:{port}"))
    );
    let served: Vec<PointInfo> = events.wait(simulator, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    });
    assert_eq!(served.len(), plant.points.len());

    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::Dnp3Master(master(port)),
    });
    // The points come just before the connected state.
    let points: Vec<PointInfo> = events.wait(client, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    });
    events.wait(client, |kind| {
        matches!(kind, EventKind::State(SessionState::Connected)).then_some(())
    });
    // The plant's names come first, in its order, with units and addresses.
    assert_eq!(points.len(), plant.points.len());
    for (point, defined) in points.iter().zip(&plant.points) {
        assert_eq!(point.name, defined.label);
        assert_eq!(point.unit, defined.unit);
    }
    let current = &points[slot("i_a")];
    assert_eq!(
        (current.address.as_str(), current.writable),
        ("AI 3", false)
    );
    let breaker_cmd = &points[slot("breaker_cmd")];
    assert_eq!(
        (breaker_cmd.address.as_str(), breaker_cmd.writable),
        ("BO 0", true)
    );
    assert!(breaker_cmd.boolean);

    // The integrity poll reads every point good: the breaker closed, current flowing.
    let first = events.wait(client, |kind| {
        values(kind).filter(|readings| {
            readings.len() == points.len()
                && readings
                    .iter()
                    .all(|reading| reading.quality == Quality::Good)
        })
    });
    assert_eq!(first[slot("breaker")].value, 1.0);
    assert!(first[slot("i_a")].value > 90.0);
    // The traffic monitor shows the integrity poll as an analyser would.
    events.wait(client, |kind| match kind {
        EventKind::Traffic(frames) => frames
            .iter()
            .any(|frame| frame.summary == "READ class 1, 2, 3, 0" && frame.bytes > 10)
            .then_some(()),
        _ => None,
    });
    // The outstation started unsynchronised; the master wrote the time when it asked.
    events.wait(client, |kind| match kind {
        EventKind::Dnp3Iin(iin) if iin.iin1 & 0x10 == 0 => Some(()),
        _ => None,
    });

    // A plain write to the breaker output: select-before-operate, latch off.
    // The outstation's breaker opens and the current stops, reported as events.
    engine.send(Command::Write {
        session: client,
        point: id_of("breaker_cmd"),
        value: 0.0,
    });
    assert_eq!(written(&mut events, client, id_of("breaker_cmd")), Ok(()));
    reads(&mut events, client, "breaker", |value| value == 0.0);
    reads(&mut events, client, "i_a", |value| value == 0.0);
    reads(&mut events, client, "breaker_cmd", |value| value == 0.0);

    // Close it again with a CROB, by direct operate.
    let close = Dnp3Action::Operate {
        point: id_of("breaker_cmd"),
        operation: Dnp3Operation::Crob(Dnp3Crob::new(Dnp3CrobCode::Close)),
        mode: Dnp3ControlMode::DirectOperate,
    };
    engine.send(Command::Dnp3 {
        session: client,
        action: close,
    });
    assert!(done(&mut events, client, close).is_ok());
    reads(&mut events, client, "breaker", |value| value == 1.0);

    // An analog output block moves the regulator, whose voltage follows.
    let setpoint = Dnp3Action::Operate {
        point: id_of("reg_setpoint"),
        operation: Dnp3Operation::Analog { value: 14.4 },
        mode: Dnp3ControlMode::SelectBeforeOperate,
    };
    engine.send(Command::Dnp3 {
        session: client,
        action: setpoint,
    });
    assert_eq!(written(&mut events, client, id_of("reg_setpoint")), Ok(()));
    reads(&mut events, client, "reg_voltage", |value| {
        (value - 14.4).abs() < 0.5
    });

    // Inputs take no controls, and the master says so without asking the outstation.
    engine.send(Command::Write {
        session: client,
        point: id_of("i_a"),
        value: 5.0,
    });
    let refused = written(&mut events, client, id_of("i_a")).unwrap_err();
    assert!(refused.contains("AI 3 is an input"), "{refused}");

    // The simulator takes a point offline: the master reads it bad, with the flags.
    let offline = Dnp3Action::SetOnline {
        point: id_of("i_b"),
        online: false,
    };
    engine.send(Command::Dnp3 {
        session: simulator,
        action: offline,
    });
    assert!(done(&mut events, simulator, offline).is_ok());
    let index = slot("i_b");
    events.wait(client, |kind| {
        values(kind)
            .and_then(|readings| (readings.get(index)?.quality == Quality::Bad).then_some(()))
    });
    events.wait(client, |kind| match kind {
        EventKind::Payloads(texts) => texts
            .iter()
            .any(|text| text.point == id_of("i_b") && text.text.contains("COMM_LOST"))
            .then_some(()),
        _ => None,
    });

    // A class poll on request.
    let class_1 = Dnp3Action::ClassPoll { class: 1 };
    engine.send(Command::Dnp3 {
        session: client,
        action: class_1,
    });
    assert!(done(&mut events, client, class_1).is_ok());

    // A cold restart: the master sees IIN1.7, clears it and polls everything again.
    engine.send(Command::Dnp3 {
        session: client,
        action: Dnp3Action::ColdRestart,
    });
    assert!(done(&mut events, client, Dnp3Action::ColdRestart).is_ok());
    events.wait(client, |kind| match kind {
        EventKind::Dnp3Iin(iin) if iin.iin1 & 0x80 != 0 => Some(()),
        _ => None,
    });
    events.wait(client, |kind| match kind {
        EventKind::Dnp3Iin(iin) if iin.iin1 & 0x90 == 0 => Some(()),
        _ => None,
    });

    engine.send(Command::Close { session: client });
    events.wait(client, |kind| {
        matches!(kind, EventKind::State(SessionState::Closed { error: None })).then_some(())
    });
    engine.send(Command::Close { session: simulator });
    events.wait(simulator, |kind| {
        matches!(kind, EventKind::State(SessionState::Closed { error: None })).then_some(())
    });

    // The closed simulator let go of its port: a new one listens on it at once.
    let again = SessionId(3);
    engine.send(Command::Open {
        session: again,
        spec: outstation(port),
    });
    assert!(listening(&mut events, again).is_ok());
    engine.send(Command::Shutdown);
}

#[test]
fn failures_to_reach_or_listen_are_reported_not_fatal() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);

    // Nobody listens: the master keeps trying, saying why.
    let client = SessionId(1);
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::Dnp3Master(master(free_port())),
    });
    let reason = events.wait(client, |kind| match kind {
        EventKind::State(SessionState::Reconnecting { reason, .. }) => Some(reason.clone()),
        _ => None,
    });
    assert!(reason.contains("Cannot reach the outstation"), "{reason}");
    // Commands while there is no link are refused, not lost.
    engine.send(Command::Dnp3 {
        session: client,
        action: Dnp3Action::TimeSync,
    });
    assert_eq!(
        done(&mut events, client, Dnp3Action::TimeSync),
        Err("Not connected".into())
    );
    engine.send(Command::Close { session: client });

    // The port is taken by another program: the simulator stops with a clear error.
    let holder = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener");
    let port = holder.local_addr().expect("its address").port();
    let simulator = SessionId(2);
    engine.send(Command::Open {
        session: simulator,
        spec: outstation(port),
    });
    let error = listening(&mut events, simulator).unwrap_err();
    assert_eq!(
        error,
        format!("TCP port {port} is in use by another program")
    );

    // Addresses the master and outstation can't both have.
    let clash = SessionId(3);
    engine.send(Command::Open {
        session: clash,
        spec: SessionSpec::Dnp3Master(Dnp3MasterSpec {
            master_address: 10,
            ..master(free_port())
        }),
    });
    let error = events.wait(clash, |kind| match kind {
        EventKind::State(SessionState::Closed { error }) => error.clone(),
        _ => None,
    });
    assert!(error.contains("different link addresses"), "{error}");
    engine.send(Command::Shutdown);
}
