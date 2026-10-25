//! A simulator and a master talking over loopback through the engine, the way
//! the app drives them.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    Command, Event, EventKind, ModbusSlaveSpec, Quality, SessionId, SessionSpec, SessionState,
};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .expect("a free loopback port")
}

/// Wait for the first event of `session` that `accept` maps to a value.
fn wait_for<T>(
    events: &async_channel::Receiver<Event>,
    session: SessionId,
    mut accept: impl FnMut(&EventKind) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match events.try_recv() {
            Ok(event) if event.session == session => {
                if let Some(found) = accept(&event.kind) {
                    return found;
                }
            }
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    panic!("timed out waiting for an event from {session}");
}

#[test]
fn master_reads_and_writes_a_simulated_template_device() {
    let (engine, events) = vt_engine::start().unwrap();
    let profile = templates::find_modbus("pm5560").expect("PM5560 template");
    let port = free_port();
    let slave = SessionId(1);
    let master = SessionId(2);

    engine.send(Command::Open {
        session: slave,
        spec: SessionSpec::ModbusSlave(ModbusSlaveSpec {
            port,
            template: Some(profile.id.clone()),
            ..ModbusSlaveSpec::default()
        }),
    });
    wait_for(&events, slave, |kind| {
        matches!(kind, EventKind::State(SessionState::Listening { .. })).then_some(())
    });

    let mut spec = templates::master_for(profile, "localhost", port, 1);
    spec.poll_ms = 50;
    engine.send(Command::Open {
        session: master,
        spec: SessionSpec::ModbusMaster(spec),
    });
    let points = wait_for(&events, master, |kind| match kind {
        EventKind::Points(points) => Some(points.len()),
        _ => None,
    });
    assert_eq!(points, profile.registers.len());

    // Every register of the template reads back with good quality and inside
    // its simulated range.
    let first = wait_for(&events, master, |kind| match kind {
        EventKind::Values(frame) if frame.readings.iter().all(|r| r.quality == Quality::Good) => {
            Some(frame.readings.clone())
        }
        _ => None,
    });
    for (register, reading) in profile.registers.iter().zip(&first) {
        assert!(
            reading.value >= register.min - 1e-3 && reading.value <= register.max + 1e-3,
            "{} = {} outside {}..{}",
            register.name,
            reading.value,
            register.min,
            register.max
        );
    }

    // A write from the master lands in the simulator's registers and stays:
    // the simulator takes it into its simulation and holds it, with no
    // slave-side write or lock.
    let target = profile
        .registers
        .iter()
        .position(|register| !register.is_counter)
        .expect("a measurement register");
    let point = u32::try_from(target).expect("a point id");
    let register = &profile.registers[target];
    let value = f64::from(((register.min + register.max) / 2.0) as f32);
    engine.send(Command::Write {
        session: master,
        point,
        value,
    });
    let written = wait_for(&events, master, |kind| match kind {
        EventKind::Written { point, result } if *point as usize == target => Some(result.clone()),
        _ => None,
    });
    assert_eq!(written, Ok(()));
    wait_for(&events, slave, |kind| {
        (*kind
            == EventKind::Locked {
                point,
                locked: true,
            })
        .then_some(())
    });

    // Several simulation ticks later (well past two publish intervals), the
    // master still reads the value it wrote, and so does the simulator's own
    // view of its registers.
    std::thread::sleep(Duration::from_millis(1200));
    while events.try_recv().is_ok() {}
    let deadline = Instant::now() + Duration::from_millis(2500);
    let mut seen = 0;
    let mut published = 0;
    while Instant::now() < deadline {
        let Ok(event) = events.try_recv() else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        if let EventKind::Values(frame) = event.kind
            && let Some(reading) = frame.readings.get(target)
            && reading.quality == Quality::Good
        {
            assert!(
                (reading.value - value).abs() < 1e-3,
                "{} drifted to {}",
                event.session,
                reading.value
            );
            if event.session == master {
                seen += 1;
            } else if event.session == slave {
                published += 1;
            }
        }
    }
    assert!(seen > 5, "only {seen} master frames after the write");
    assert!(
        published > 2,
        "only {published} simulator frames after the write"
    );

    engine.send(Command::Close { session: master });
    wait_for(&events, master, |kind| {
        matches!(kind, EventKind::State(SessionState::Closed { error: None })).then_some(())
    });
    engine.send(Command::Shutdown);
}

#[test]
fn master_reconnects_when_the_device_appears() {
    let (engine, events) = vt_engine::start().unwrap();
    let port = free_port();
    let master = SessionId(10);
    let slave = SessionId(11);
    let spec = vt_protocol::ModbusMasterSpec {
        link: vt_protocol::ModbusLink::Tcp {
            host: "127.0.0.1".into(),
            port,
        },
        poll_ms: 50,
        ..vt_protocol::ModbusMasterSpec::default()
    };
    engine.send(Command::Open {
        session: master,
        spec: SessionSpec::ModbusMaster(spec),
    });
    wait_for(&events, master, |kind| {
        matches!(kind, EventKind::State(SessionState::Reconnecting { .. })).then_some(())
    });

    engine.send(Command::Open {
        session: slave,
        spec: SessionSpec::ModbusSlave(ModbusSlaveSpec {
            port,
            ..ModbusSlaveSpec::default()
        }),
    });
    wait_for(&events, master, |kind| {
        matches!(kind, EventKind::State(SessionState::Connected)).then_some(())
    });
    let good = wait_for(&events, master, |kind| match kind {
        EventKind::Values(frame) => Some(frame.readings.iter().all(|r| r.quality == Quality::Good)),
        _ => None,
    });
    assert!(good);
    engine.send(Command::Shutdown);
}

/// Wait until `session` listens; `Err` if it closed with an error first.
fn listening(events: &async_channel::Receiver<Event>, session: SessionId) -> Result<(), String> {
    wait_for(events, session, |kind| match kind {
        EventKind::State(SessionState::Listening { .. }) => Some(Ok(())),
        EventKind::State(SessionState::Closed { error: Some(error) }) => Some(Err(error.clone())),
        _ => None,
    })
}

#[test]
fn a_simulator_restarts_on_its_own_port() {
    let (engine, events) = vt_engine::start().expect("the engine starts");
    let port = free_port();
    let spec = || {
        SessionSpec::ModbusSlave(ModbusSlaveSpec {
            port,
            ..ModbusSlaveSpec::default()
        })
    };
    let first = SessionId(20);
    engine.send(Command::Open {
        session: first,
        spec: spec(),
    });
    assert_eq!(listening(&events, first), Ok(()));

    // Saving edits to a running simulator: the app closes the old session and
    // opens a new one on the same port straight away.
    let second = SessionId(21);
    engine.send(Command::Close { session: first });
    engine.send(Command::Open {
        session: second,
        spec: spec(),
    });
    assert_eq!(listening(&events, second), Ok(()));

    // Reconnecting: opening the running session's id again replaces it.
    engine.send(Command::Open {
        session: second,
        spec: spec(),
    });
    assert_eq!(listening(&events, second), Ok(()));
    engine.send(Command::Shutdown);
}

#[test]
fn a_simulator_listens_on_ipv6_loopback() {
    // Hosts without IPv6 cannot run this.
    if std::net::TcpListener::bind("[::1]:0").is_err() {
        return;
    }
    let (engine, events) = vt_engine::start().expect("the engine starts");
    let session = SessionId(30);
    engine.send(Command::Open {
        session,
        spec: SessionSpec::ModbusSlave(ModbusSlaveSpec {
            bind: "::1".into(),
            port: free_port(),
            ..ModbusSlaveSpec::default()
        }),
    });
    assert_eq!(listening(&events, session), Ok(()));
    engine.send(Command::Shutdown);
}
