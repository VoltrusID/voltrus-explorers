//! An EtherNet/IP adapter simulator and clients talking over loopback through
//! the engine, the way the app drives them: ListIdentity, Logix tags by name,
//! the input assembly, writes that the adapter holds, explicit messages, a
//! browse-everything client, and a port that is taken.

use std::time::{Duration, Instant};

use vt_engine::templates;
use vt_protocol::{
    CipRequest, CipResponse, Command, EnipAdapterSpec, EnipAddress, EnipClientSpec,
    EnipDiscoverSpec, EnipPoint, Event, EventKind, PointInfo, Quality, Reading, SessionId,
    SessionSpec, SessionState,
};

/// A port free for both TCP (sessions) and UDP (ListIdentity), away from
/// 44818 so an EtherNet/IP application on the test machine can't interfere.
fn free_port() -> u16 {
    loop {
        let port = std::net::TcpListener::bind("0.0.0.0:0")
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .expect("a free TCP port");
        if std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
    }
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

fn points(events: &mut Events, session: SessionId) -> Vec<PointInfo> {
    events.wait(session, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    })
}

fn addresses(events: &mut Events, session: SessionId) -> Vec<String> {
    events.wait(session, |kind| match kind {
        EventKind::EnipAddresses(addresses) => Some(addresses.clone()),
        _ => None,
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

/// Wait until `session` reads `point` good and within 0.01 of `expected`.
fn reads(events: &mut Events, session: SessionId, point: usize, expected: f64) -> Vec<Reading> {
    events.wait(session, |kind| {
        values(kind).filter(|readings| {
            let reading = readings[point];
            reading.quality == Quality::Good && (reading.value - expected).abs() < 0.01
        })
    })
}

fn cip_reply(events: &mut Events, session: SessionId) -> Result<CipResponse, String> {
    events.wait(session, |kind| match kind {
        EventKind::CipReply(result) => Some(result.clone()),
        _ => None,
    })
}

fn state(events: &mut Events, session: SessionId, want: fn(&SessionState) -> bool) -> SessionState {
    events.wait(session, |kind| match kind {
        EventKind::State(state) if want(state) => Some(state.clone()),
        _ => None,
    })
}

fn index_of(key: &str) -> usize {
    templates::find_enip("drive")
        .expect("the drive plant")
        .points
        .iter()
        .position(|point| point.key == key)
        .expect("a point of the plant")
}

#[test]
fn client_discovers_reads_writes_and_messages_a_simulated_adapter() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let template = templates::find_enip("drive").expect("the drive plant");
    let port = free_port();
    let adapter = SessionId(1);
    let client = SessionId(2);
    let discovery = SessionId(3);

    engine.send(Command::Open {
        session: adapter,
        spec: SessionSpec::EnipAdapter(EnipAdapterSpec {
            port,
            template: "drive".into(),
            ..EnipAdapterSpec::default()
        }),
    });
    // The default adapter stays on this computer.
    let listening = state(&mut events, adapter, |state| {
        matches!(state, SessionState::Listening { .. })
    });
    assert_eq!(
        listening,
        SessionState::Listening {
            address: format!("127.0.0.1:{port}"),
        }
    );
    let served = points(&mut events, adapter);
    assert_eq!(served.len(), template.points.len());
    assert_eq!(served[index_of("speed")].name, "Motor1.Speed");
    assert_eq!(served[index_of("speed")].address, "Main");
    assert_eq!(
        addresses(&mut events, adapter)[index_of("speed")],
        "Program:Main.Motor1.Speed"
    );
    assert_eq!(served[index_of("running")].data_type, "BOOL");

    // ListIdentity, directed at the adapter on loopback, finds it once.
    engine.send(Command::DiscoverEnip {
        session: discovery,
        spec: EnipDiscoverSpec {
            broadcast_port: None,
            directed: vec![format!("127.0.0.1:{port}")],
            wait_ms: 1000,
        },
    });
    let found = events
        .wait(discovery, |kind| match kind {
            EventKind::EnipDevices(found) => Some(found.clone()),
            _ => None,
        })
        .expect("ListIdentity went out");
    let ours: Vec<_> = found.iter().filter(|device| device.port == port).collect();
    assert_eq!(ours.len(), 1, "{found:?}");
    assert_eq!(ours[0].address, "127.0.0.1");
    assert_eq!(ours[0].product, "ENIP-Sim");
    assert_eq!(ours[0].device_type, "Programmable Logic Controller");

    // The demo client: every tag by name, the speed once more through the
    // input assembly, and one tag the controller does not have.
    let mut spec = templates::enip_client_for(template, "127.0.0.1", port);
    spec.poll_ms = 200;
    spec.points.push(EnipPoint {
        name: "Missing".into(),
        address: EnipAddress::parse("Program:Main.NoSuchTag").unwrap(),
        unit: String::new(),
    });
    let assembly = template.points.len();
    let missing = assembly + 1;
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::EnipClient(spec),
    });
    let read = points(&mut events, client);
    assert_eq!(read.len(), template.points.len() + 2);
    assert_eq!(
        read[index_of("speed")].data_type,
        "REAL",
        "types come from the device"
    );
    assert_eq!(read[index_of("speed")].unit, "RPM");
    assert_eq!(read[index_of("mode")].data_type, "DINT");
    assert!(read[index_of("running")].boolean);
    assert_eq!(read[assembly].address, "@4/100/3");
    assert_eq!(read[assembly].name, "Motor1.Speed (assembly)");
    let whole = addresses(&mut events, client);
    assert_eq!(whole.len(), read.len());
    assert_eq!(whole[assembly], "@0x04/100/3+24:REAL");
    assert_eq!(whole[missing], "Program:Main.NoSuchTag");

    // One bad tag costs one point, not the poll.
    let first = events.wait(client, values);
    assert_eq!(first[missing].quality, Quality::Bad);
    assert_eq!(first[index_of("speedRef")].value, 1450.0);
    assert!(
        first[..missing]
            .iter()
            .all(|reading| reading.quality == Quality::Good)
    );

    // The reference is a setpoint: the drive's speed follows it.
    let speed_ref = u32::try_from(index_of("speedRef")).unwrap();
    engine.send(Command::Write {
        session: client,
        point: speed_ref,
        value: 900.0,
    });
    assert_eq!(written(&mut events, client, speed_ref), Ok(()));
    reads(&mut events, adapter, index_of("speedRef"), 900.0);
    let speed = index_of("speed");
    events.wait(adapter, |kind| {
        values(kind).filter(|readings| (readings[speed].value - 900.0).abs() < 60.0)
    });

    // Writing a drifting value holds it on the adapter; the assembly carries
    // the same bytes as the tag.
    let speed_point = u32::try_from(speed).unwrap();
    engine.send(Command::Write {
        session: client,
        point: speed_point,
        value: 1234.5,
    });
    assert_eq!(written(&mut events, client, speed_point), Ok(()));
    events.wait(adapter, |kind| match kind {
        EventKind::Locked {
            point,
            locked: true,
        } if *point == speed_point => Some(()),
        _ => None,
    });
    let held = reads(&mut events, client, speed, 1234.5);
    assert_eq!(held[assembly].value, 1234.5);

    // Writes the device would refuse are refused with the reason.
    let mode = u32::try_from(index_of("mode")).unwrap();
    engine.send(Command::Write {
        session: client,
        point: mode,
        value: 1.5,
    });
    assert_eq!(
        written(&mut events, client, mode),
        Err("DINT takes whole numbers".into())
    );
    engine.send(Command::Write {
        session: client,
        point: u32::try_from(missing).unwrap(),
        value: 1.0,
    });
    assert!(written(&mut events, client, u32::try_from(missing).unwrap()).is_err());

    // An attribute point writes back through Set_Attribute_Single; the input
    // assembly is produced by the adapter and refuses it.
    let assembly_point = u32::try_from(assembly).unwrap();
    engine.send(Command::Write {
        session: client,
        point: assembly_point,
        value: 10.0,
    });
    let refused = written(&mut events, client, assembly_point).expect_err("not settable");
    assert!(refused.contains("0x0E"), "{refused}");

    // Explicit messages: the Identity object's product name, and an object
    // the adapter does not have.
    engine.send(Command::CipRequest {
        session: client,
        request: CipRequest {
            service: 0x0E,
            class: 0x01,
            instance: 1,
            attribute: Some(7),
            data: Vec::new(),
        },
    });
    let reply = cip_reply(&mut events, client).expect("the Identity object answers");
    assert_eq!(reply.data, b"\x08ENIP-Sim");
    engine.send(Command::CipRequest {
        session: client,
        request: CipRequest {
            service: 0x0E,
            class: 0x99,
            instance: 1,
            attribute: Some(1),
            data: Vec::new(),
        },
    });
    let error = cip_reply(&mut events, client).expect_err("no such object");
    assert!(error.contains("0x05"), "{error}");
    // Set the output assembly, read it back.
    engine.send(Command::CipRequest {
        session: client,
        request: CipRequest {
            service: 0x10,
            class: 0x04,
            instance: 150,
            attribute: Some(3),
            data: vec![1, 2, 3, 4],
        },
    });
    assert!(cip_reply(&mut events, client).is_ok());
    engine.send(Command::CipRequest {
        session: client,
        request: CipRequest {
            service: 0x0E,
            class: 0x04,
            instance: 150,
            attribute: Some(3),
            data: Vec::new(),
        },
    });
    assert_eq!(
        cip_reply(&mut events, client).map(|reply| reply.data),
        Ok(vec![1, 2, 3, 4])
    );

    // The traffic view names the services on the wire, both sides.
    let summaries: Vec<String> = events.wait(client, |kind| match kind {
        EventKind::Traffic(frames) => {
            Some(frames.iter().map(|frame| frame.summary.clone()).collect())
        }
        _ => None,
    });
    assert!(!summaries.is_empty());
    let mut seen = summaries;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !seen
        .iter()
        .any(|summary| summary.starts_with("Multiple_Service_Packet"))
    {
        assert!(Instant::now() < deadline, "no batched read in {seen:?}");
        seen = events.wait(client, |kind| match kind {
            EventKind::Traffic(frames) => {
                Some(frames.iter().map(|frame| frame.summary.clone()).collect())
            }
            _ => None,
        });
    }
    let served_traffic: Vec<String> = events.wait(adapter, |kind| match kind {
        EventKind::Traffic(frames) => {
            Some(frames.iter().map(|frame| frame.summary.clone()).collect())
        }
        _ => None,
    });
    assert!(!served_traffic.is_empty());

    // The app releases the held speed: it follows the reference again.
    engine.send(Command::Lock {
        session: adapter,
        point: speed_point,
        locked: false,
    });
    events.wait(adapter, |kind| {
        values(kind).filter(|readings| (readings[speed].value - 900.0).abs() < 60.0)
    });

    // A stopped adapter drops its clients, which try again.
    engine.send(Command::Close { session: adapter });
    state(&mut events, adapter, |state| {
        matches!(state, SessionState::Closed { error: None })
    });
    state(&mut events, client, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    engine.send(Command::Close { session: client });
    state(&mut events, client, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_without_points_reads_every_tag_the_controller_lists() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let template = templates::find_enip("tank").expect("the tank plant");
    let port = free_port();
    let adapter = SessionId(1);
    let client = SessionId(2);
    engine.send(Command::Open {
        session: adapter,
        spec: SessionSpec::EnipAdapter(EnipAdapterSpec {
            port,
            template: "tank".into(),
            ..EnipAdapterSpec::default()
        }),
    });
    state(&mut events, adapter, |state| {
        matches!(state, SessionState::Listening { .. })
    });

    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::EnipClient(EnipClientSpec {
            host: "127.0.0.1".into(),
            port,
            poll_ms: 200,
            points: Vec::new(),
            ..EnipClientSpec::default()
        }),
    });
    let read = points(&mut events, client);
    let whole = addresses(&mut events, client);
    let mut browsed: Vec<&str> = whole.iter().map(String::as_str).collect();
    let mut expected: Vec<&str> = template
        .points
        .iter()
        .map(|point| point.name.as_str())
        .collect();
    browsed.sort_unstable();
    expected.sort_unstable();
    assert_eq!(browsed, expected);
    // Program-scoped tags show without their scope.
    assert!(read.iter().any(|point| point.name == "Tank1.Level"));
    let level = whole
        .iter()
        .position(|address| address == "Program:Main.Tank1.Level")
        .expect("the level");
    assert_eq!(read[level].address, "Main");
    assert_eq!(read[level].data_type, "REAL");
    let first = events.wait(client, values);
    assert!(first.iter().all(|reading| reading.quality == Quality::Good));
    assert!((10.0..=95.0).contains(&first[level].value));

    engine.send(Command::Close { session: client });
    engine.send(Command::Close { session: adapter });
    state(&mut events, adapter, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    engine.send(Command::Shutdown);
}

#[test]
fn an_adapter_on_a_taken_port_fails_with_the_reason() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener");
    let port = taken.local_addr().expect("its address").port();
    let adapter = SessionId(1);
    engine.send(Command::Open {
        session: adapter,
        spec: SessionSpec::EnipAdapter(EnipAdapterSpec {
            port,
            template: "drive".into(),
            ..EnipAdapterSpec::default()
        }),
    });
    let closed = state(&mut events, adapter, |state| {
        matches!(state, SessionState::Closed { .. })
    });
    assert_eq!(
        closed,
        SessionState::Closed {
            error: Some(format!("TCP port {port} is in use by another program")),
        }
    );

    // A client of nothing keeps trying, saying why.
    let client = SessionId(2);
    drop(taken);
    engine.send(Command::Open {
        session: client,
        spec: SessionSpec::EnipClient(EnipClientSpec {
            port,
            ..EnipClientSpec::default()
        }),
    });
    let retrying = state(&mut events, client, |state| {
        matches!(state, SessionState::Reconnecting { .. })
    });
    let SessionState::Reconnecting { reason, .. } = retrying else {
        unreachable!()
    };
    assert!(reason.contains("Connection failed"), "{reason}");
    engine.send(Command::Close { session: client });
    engine.send(Command::Shutdown);
}
