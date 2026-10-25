//! A BACnet device simulator and clients talking over loopback through the
//! engine, the way the app drives them: discovery, the object list, COV and
//! polling, priority writes and relinquish.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    BacnetClientSpec, BacnetDeviceSpec, BacnetDiscoverSpec, Command, Direction, Event, EventKind,
    PointInfo, Quality, Reading, SessionId, SessionSpec, SessionState,
};

/// A UDP port nothing on this machine uses, away from 47808 so a BACnet
/// application on the test machine can't interfere.
fn free_port() -> u16 {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| socket.local_addr())
        .map(|address| address.port())
        .expect("a free UDP port")
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

    /// Forget everything received so far.
    fn clear(&mut self) {
        self.kept.clear();
        while self.receiver.try_recv().is_ok() {}
    }
}

fn values(kind: &EventKind) -> Option<Vec<Reading>> {
    match kind {
        EventKind::Values(frame) => Some(frame.readings.clone()),
        _ => None,
    }
}

fn index_of(key: &str) -> u32 {
    let template = templates::find_bacnet("ahu").expect("the AHU plant");
    let index = template
        .points
        .iter()
        .position(|point| point.key == key)
        .expect("a point of the plant");
    u32::try_from(index).expect("a point id")
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

/// Wait until the client reads `point` within 0.01 of `expected`.
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

fn device(port: u16, instance: u32) -> SessionSpec {
    SessionSpec::BacnetDevice(BacnetDeviceSpec {
        port,
        device_instance: instance,
        template: "ahu".into(),
    })
}

fn listening(events: &mut Events, session: SessionId) -> Result<(), String> {
    events.wait(session, |kind| match kind {
        EventKind::State(SessionState::Listening { .. }) => Some(Ok(())),
        EventKind::State(SessionState::Closed { error }) => {
            Some(Err(error.clone().unwrap_or_default()))
        }
        _ => None,
    })
}

#[test]
fn client_discovers_reads_subscribes_and_commands_a_simulated_device() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let template = templates::find_bacnet("ahu").expect("the AHU plant");
    let port = free_port();
    let simulator = SessionId(1);
    let client = SessionId(2);
    let discovery = SessionId(3);

    engine.send(Command::Open {
        session: simulator,
        spec: device(port, 2001),
    });
    assert_eq!(listening(&mut events, simulator), Ok(()));

    // The device picker's Who-Is, directed at the simulator on loopback.
    engine.send(Command::DiscoverBacnet {
        session: discovery,
        spec: BacnetDiscoverSpec {
            bind: "0.0.0.0".into(),
            broadcast_port: None,
            directed: vec![format!("127.0.0.1:{port}")],
            wait_ms: 1000,
        },
    });
    let found = events.wait(discovery, |kind| match kind {
        EventKind::BacnetDevices(found) => Some(found.clone()),
        _ => None,
    });
    let found = found.expect("the Who-Is went out");
    assert!(
        found.iter().any(|device| device.instance == 2001
            && device.address == "127.0.0.1"
            && device.port == port),
        "{found:?}"
    );

    // The client, not told the instance, finds it with its own Who-Is and
    // reads the object list: every object of the plant is a point, named
    // after its object-name.
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::BacnetClient(BacnetClientSpec {
            host: "127.0.0.1".into(),
            port,
            device_instance: None,
            poll_ms: 200,
            cov: true,
            priority: 8,
            ..BacnetClientSpec::default()
        }),
    });
    let points: Vec<PointInfo> = events.wait(client, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    });
    assert_eq!(points.len(), template.points.len());
    for (point, object) in points.iter().zip(&template.points) {
        assert_eq!(point.name, object.label);
        assert_eq!(point.boolean, object.is_bool, "{}", point.name);
    }
    let supply = &points[usize::try_from(index_of("supply_temp")).unwrap()];
    assert_eq!(
        (supply.address.as_str(), supply.unit.as_str()),
        ("AI 1", "°C")
    );
    assert!(!supply.writable, "inputs take no writes");
    let mode = &points[usize::try_from(index_of("mode")).unwrap()];
    assert_eq!(
        (mode.address.as_str(), mode.data_type.as_str()),
        ("MSV 1", "Multi-state")
    );
    assert!(mode.writable);

    // Every point reads good and inside the plant's range.
    let first = events.wait(client, |kind| {
        values(kind).filter(|readings| {
            readings
                .iter()
                .all(|reading| reading.quality == Quality::Good)
        })
    });
    for (reading, object) in first.iter().zip(&template.points) {
        let low = object.min.unwrap_or(f64::MIN) - 1e-3;
        let high = object.max.unwrap_or(f64::MAX) + 1e-3;
        assert!(
            (low..=high).contains(&reading.value),
            "{} = {}",
            object.label,
            reading.value
        );
    }

    // The supply temperature drifts, and the client sees it move.
    let supply_index = usize::try_from(index_of("supply_temp")).unwrap();
    let start = first[supply_index].value;
    events.wait(client, |kind| {
        values(kind).and_then(|readings| {
            let reading = readings[supply_index];
            (reading.quality == Quality::Good && (reading.value - start).abs() > 1e-4).then_some(())
        })
    });

    // The device took the COV subscriptions and notifies changes.
    events.wait(client, |kind| match kind {
        EventKind::Traffic(frames) => frames
            .iter()
            .any(|frame| {
                frame.direction == Direction::Rx
                    && frame.summary.starts_with("UnconfirmedCOVNotification AI")
            })
            .then_some(()),
        _ => None,
    });

    // A command at priority 8 lands: the client reads it back and the
    // simulator holds the point.
    let setpoint = index_of("setpoint");
    engine.send(Command::Write {
        session: client,
        point: setpoint,
        value: 26.5,
    });
    assert_eq!(written(&mut events, client, setpoint), Ok(()));
    reads(&mut events, client, setpoint, 26.5);
    events.wait(simulator, |kind| {
        (*kind
            == EventKind::Locked {
                point: setpoint,
                locked: true,
            })
        .then_some(())
    });
    events.wait(simulator, |kind| {
        values(kind).and_then(|readings| {
            let reading = readings[usize::try_from(setpoint).unwrap()];
            ((reading.value - 26.5).abs() < 0.01).then_some(())
        })
    });

    // The device refuses what its objects can't take, and the client says why.
    engine.send(Command::Write {
        session: client,
        point: setpoint,
        value: 99.0,
    });
    let refused = written(&mut events, client, setpoint).expect_err("past the setpoint's maximum");
    assert!(refused.contains("value-out-of-range"), "{refused}");
    let mode_point = index_of("mode");
    engine.send(Command::Write {
        session: client,
        point: mode_point,
        value: 2.0,
    });
    assert_eq!(written(&mut events, client, mode_point), Ok(()));
    reads(&mut events, client, mode_point, 2.0);
    let fan = index_of("fan_command");
    engine.send(Command::Write {
        session: client,
        point: fan,
        value: 0.0,
    });
    assert_eq!(written(&mut events, client, fan), Ok(()));
    reads(&mut events, client, fan, 0.0);
    let supply_point = index_of("supply_temp");
    engine.send(Command::Write {
        session: client,
        point: supply_point,
        value: 20.0,
    });
    assert!(written(&mut events, client, supply_point).is_err());

    // Relinquishing priority 8 hands the setpoint back to its default, and
    // the simulator lets it go.
    engine.send(Command::Relinquish {
        session: client,
        point: setpoint,
    });
    assert_eq!(written(&mut events, client, setpoint), Ok(()));
    reads(&mut events, client, setpoint, 23.0);
    events.wait(simulator, |kind| {
        (*kind
            == EventKind::Locked {
                point: setpoint,
                locked: false,
            })
        .then_some(())
    });

    // The simulator's traffic shows the client's requests: its waits above
    // consumed the earlier ones, so command once more.
    engine.send(Command::Write {
        session: client,
        point: mode_point,
        value: 4.0,
    });
    assert_eq!(written(&mut events, client, mode_point), Ok(()));
    events.wait(simulator, |kind| match kind {
        EventKind::Traffic(frames) => frames
            .iter()
            .any(|frame| frame.direction == Direction::Rx && frame.summary == "WriteProperty")
            .then_some(()),
        _ => None,
    });

    engine.send(Command::Close { session: client });
    engine.send(Command::Close { session: simulator });
    for session in [client, simulator] {
        events.wait(session, |kind| {
            matches!(kind, EventKind::State(SessionState::Closed { error: None })).then_some(())
        });
    }
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_without_cov_polls_with_read_property_multiple() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let port = free_port();
    let simulator = SessionId(10);
    let client = SessionId(11);
    engine.send(Command::Open {
        session: simulator,
        spec: device(port, 77),
    });
    assert_eq!(listening(&mut events, simulator), Ok(()));
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::BacnetClient(BacnetClientSpec {
            host: "localhost".into(),
            port,
            device_instance: Some(77),
            poll_ms: 150,
            cov: false,
            ..BacnetClientSpec::default()
        }),
    });
    events.wait(client, |kind| {
        values(kind).filter(|readings| {
            !readings.is_empty()
                && readings
                    .iter()
                    .all(|reading| reading.quality == Quality::Good)
        })
    });
    events.wait(client, |kind| match kind {
        EventKind::Traffic(frames) => frames
            .iter()
            .any(|frame| {
                frame.direction == Direction::Tx && frame.summary == "ReadPropertyMultiple"
            })
            .then_some(()),
        _ => None,
    });
    // A held simulator value is what the client reads.
    let damper = index_of("damper");
    engine.send(Command::Write {
        session: simulator,
        point: damper,
        value: 61.0,
    });
    assert_eq!(written(&mut events, simulator, damper), Ok(()));
    reads(&mut events, client, damper, 61.0);
    engine.send(Command::Shutdown);
}

#[test]
fn a_second_device_on_a_taken_port_fails_and_a_missing_device_is_retried() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let port = free_port();
    engine.send(Command::Open {
        session: SessionId(20),
        spec: device(port, 1),
    });
    assert_eq!(listening(&mut events, SessionId(20)), Ok(()));
    engine.send(Command::Open {
        session: SessionId(21),
        spec: device(port, 2),
    });
    let error = listening(&mut events, SessionId(21)).expect_err("the port is taken");
    assert!(error.contains("in use"), "{error}");

    // Nobody answers on this port: the client keeps trying.
    events.clear();
    engine.send(Command::Open {
        session: SessionId(22),
        spec: SessionSpec::BacnetClient(BacnetClientSpec {
            port: free_port(),
            device_instance: None,
            ..BacnetClientSpec::default()
        }),
    });
    events.wait(SessionId(22), |kind| {
        matches!(kind, EventKind::State(SessionState::Reconnecting { .. })).then_some(())
    });
    engine.send(Command::Shutdown);
}
