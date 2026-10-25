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

    // A write from the master reaches the simulator's registers.
    let target = profile
        .registers
        .iter()
        .position(|register| !register.is_counter)
        .expect("a measurement register");
    let register = &profile.registers[target];
    let value = f64::from(((register.min + register.max) / 2.0) as f32);
    engine.send(Command::Write {
        session: master,
        point: u32::try_from(target).unwrap(),
        value,
    });
    let written = wait_for(&events, master, |kind| match kind {
        EventKind::Written { point, result } if *point as usize == target => Some(result.clone()),
        _ => None,
    });
    assert_eq!(written, Ok(()));

    // Simulated values drift on every tick unless pinned. Setting a value on
    // the simulator session pins it; the master must keep reading it.
    engine.send(Command::Write {
        session: slave,
        point: u32::try_from(target).unwrap(),
        value,
    });
    let pinned = wait_for(&events, slave, |kind| match kind {
        EventKind::Written { result, .. } => Some(result.clone()),
        _ => None,
    });
    assert_eq!(pinned, Ok(()));
    // Frames polled before the pin landed may still be queued: skip a few polls.
    std::thread::sleep(Duration::from_millis(300));
    while events.try_recv().is_ok() {}
    let deadline = Instant::now() + Duration::from_millis(2500);
    let mut seen = 0;
    while Instant::now() < deadline {
        if let Ok(event) = events.try_recv()
            && event.session == master
            && let EventKind::Values(frame) = event.kind
            && let Some(reading) = frame.readings.get(target)
            && reading.quality == Quality::Good
        {
            assert!(
                (reading.value - value).abs() < 1e-3,
                "drifted to {}",
                reading.value
            );
            seen += 1;
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(seen > 5, "only {seen} frames after the write");

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
    let mut spec = vt_protocol::ModbusMasterSpec::default();
    spec.link = vt_protocol::ModbusLink::Tcp {
        host: "127.0.0.1".into(),
        port,
    };
    spec.poll_ms = 50;
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
