//! BLE scans and client sessions through the engine, the way the app drives
//! them. There is no software counterpart for a Bluetooth radio, so these
//! run against the engine's simulated peripherals, which sit behind the same
//! central trait as the radio: the scan, connection, GATT discovery, reads,
//! notifications, writes and decoders are the real session code.

use std::time::{Duration, Instant};

use vt_engine::ble::{demo_clients, gatt};
use vt_protocol::{
    BleAdapter, BleCharacteristicInfo, BleDecoder, BleFormat, BleRequest, BleScanSpec, Command,
    Event, EventKind, PointInfo, Quality, Reading, SessionId, SessionSpec, SessionState,
};

/// The engine's events, kept until a test asks for them.
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

fn points(events: &mut Events, session: SessionId) -> Vec<PointInfo> {
    events.wait(session, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    })
}

fn gatt_of(events: &mut Events, session: SessionId) -> Vec<BleCharacteristicInfo> {
    events.wait(session, |kind| match kind {
        EventKind::BleGatt(characteristics) => Some(characteristics.clone()),
        _ => None,
    })
}

fn index(points: &[PointInfo], name: &str) -> u32 {
    let index = points
        .iter()
        .position(|point| point.name == name)
        .unwrap_or_else(|| panic!("no point {name} in {points:?}"));
    u32::try_from(index).expect("a point id")
}

/// Wait for a value of `point` that `accept` takes.
fn reading(
    events: &mut Events,
    session: SessionId,
    point: u32,
    accept: impl Fn(Reading) -> bool,
) -> Reading {
    let index = usize::try_from(point).expect("an index");
    events.wait(session, |kind| match kind {
        EventKind::Values(frame) => Some(frame.readings[index]).filter(|found| accept(*found)),
        _ => None,
    })
}

/// Wait for the bytes and text behind `point`.
fn payload(
    events: &mut Events,
    session: SessionId,
    point: u32,
    accept: impl Fn(&str) -> bool,
) -> String {
    events.wait(session, |kind| match kind {
        EventKind::Payloads(texts) => texts
            .iter()
            .find(|text| text.point == point && accept(&text.text))
            .map(|text| text.text.clone()),
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

fn connected(events: &mut Events, session: SessionId) {
    events.wait(session, |kind| match kind {
        EventKind::State(SessionState::Connected) => Some(()),
        EventKind::State(SessionState::Closed { error }) => {
            panic!("closed: {error:?}")
        }
        _ => None,
    });
}

fn closed(events: &mut Events, session: SessionId) -> Option<String> {
    events.wait(session, |kind| match kind {
        EventKind::State(SessionState::Closed { error }) => Some(error.clone()),
        _ => None,
    })
}

#[test]
fn a_scan_reports_signal_and_advertisements_until_done() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let scan = SessionId(1);
    engine.send(Command::ScanBle {
        session: scan,
        spec: BleScanSpec {
            adapter: BleAdapter::Simulated,
            duration_ms: 900,
        },
    });
    let mut reports = 0;
    let devices = events.wait(scan, |kind| match kind {
        EventKind::BleDevices(Ok(found)) => {
            reports += 1;
            found.done.then(|| found.devices.clone())
        }
        EventKind::BleDevices(Err(error)) => panic!("the scan failed: {error}"),
        _ => None,
    });
    assert!(reports > 1, "results arrive while scanning");
    assert_eq!(devices.len(), 3, "{devices:?}");
    // Strongest signal first.
    let signals: Vec<i16> = devices.iter().filter_map(|device| device.rssi).collect();
    assert!(
        signals.windows(2).all(|pair| pair[0] >= pair[1]),
        "{signals:?}"
    );

    let monitor = devices
        .iter()
        .find(|device| device.name == "Voltrus HRM")
        .expect("the heart rate monitor advertises");
    assert!(monitor.services.contains(&gatt::uuid16(0x180D)));
    assert_eq!(monitor.manufacturer[0].company, 0x0059);
    assert_eq!(
        gatt::company(monitor.manufacturer[0].company),
        Some("Nordic Semiconductor")
    );
    assert!(monitor.tx_power.is_some());
    // A beacon advertises service data and no name.
    let beacon = devices
        .iter()
        .find(|device| device.name.is_empty())
        .expect("the beacon advertises");
    assert_eq!(beacon.service_data[0].service, gatt::uuid16(0xFEAA));
    engine.send(Command::Shutdown);
}

#[test]
fn a_client_browses_reads_and_subscribes_with_standard_decoders() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let session = SessionId(2);
    let (_, monitor) = demo_clients().remove(0);
    engine.send(Command::Open {
        session,
        spec: SessionSpec::BleClient(monitor),
    });
    let points = points(&mut events, session);
    connected(&mut events, session);

    // Every characteristic is a point, named and decoded by its SIG number.
    let heart_rate = index(&points, "Heart Rate Measurement");
    let battery = index(&points, "Battery Level");
    let name = index(&points, "Device Name");
    let alert = index(&points, "Alert Level");
    let control = index(&points, "Heart Rate Control Point");
    let (rx, tx) = (index(&points, "UART RX"), index(&points, "UART TX"));
    let info = |point: u32| &points[usize::try_from(point).expect("an index")];
    assert_eq!(info(heart_rate).address, "2A37");
    assert_eq!(
        (
            info(heart_rate).unit.as_str(),
            info(heart_rate).data_type.as_str()
        ),
        ("bpm", "Heart rate")
    );
    assert_eq!(info(battery).unit, "%");
    assert_eq!(info(name).data_type, "UTF-8");
    assert!(info(alert).writable && !info(battery).writable);

    // Notifications push the rate; reads fill the rest.
    let rate = reading(&mut events, session, heart_rate, |found| {
        found.quality == Quality::Good
    });
    assert!((55.0..=95.0).contains(&rate.value), "{rate:?}");
    let text = payload(&mut events, session, name, |_| true);
    assert_eq!(text, "Voltrus HRM\n56 6F 6C 74 72 75 73 20 48 52 4D");
    let level = reading(&mut events, session, battery, |found| {
        found.quality == Quality::Good
    });
    assert!((5.0..=87.0).contains(&level.value));

    // A number written in the characteristic's own layout, and one that
    // does not fit it.
    engine.send(Command::Write {
        session,
        point: alert,
        value: 2.0,
    });
    assert_eq!(written(&mut events, session, alert), Ok(()));
    engine.send(Command::Write {
        session,
        point: alert,
        value: 300.0,
    });
    assert!(written(&mut events, session, alert).is_err_and(|error| error.contains("range")));
    // Raw bytes, accepted and refused by the peripheral.
    engine.send(Command::Ble {
        session,
        request: BleRequest::Write {
            point: control,
            data: vec![0x01],
        },
    });
    assert_eq!(written(&mut events, session, control), Ok(()));
    engine.send(Command::Ble {
        session,
        request: BleRequest::Write {
            point: control,
            data: vec![0x07],
        },
    });
    assert!(written(&mut events, session, control).is_err_and(|error| error.contains("0x80")));
    // A characteristic that takes no writes is refused before the radio.
    engine.send(Command::Write {
        session,
        point: battery,
        value: 50.0,
    });
    assert!(written(&mut events, session, battery).is_err());

    // The UART echoes what it receives as a notification.
    engine.send(Command::Ble {
        session,
        request: BleRequest::Write {
            point: rx,
            data: b"ping".to_vec(),
        },
    });
    assert_eq!(written(&mut events, session, rx), Ok(()));
    let echo = payload(&mut events, session, tx, |text| text.starts_with("ping"));
    assert_eq!(echo, "ping\n70 69 6E 67");

    // Notifications off: the characteristic says so.
    engine.send(Command::Ble {
        session,
        request: BleRequest::Notify {
            point: heart_rate,
            enabled: false,
        },
    });
    let gatt = gatt_of(&mut events, session);
    let entry = gatt
        .iter()
        .find(|entry| entry.point == heart_rate)
        .expect("the heart rate characteristic");
    assert!(!entry.notifying && entry.properties.notify);

    engine.send(Command::Close { session });
    assert_eq!(closed(&mut events, session), None);
    engine.send(Command::Shutdown);
}

#[test]
fn chosen_decoders_read_and_write_vendor_values() {
    let (engine, receiver) = vt_engine::start().unwrap();
    let mut events = Events::new(receiver);
    let session = SessionId(3);
    let (_, sensor) = demo_clients().remove(1);
    engine.send(Command::Open {
        session,
        spec: SessionSpec::BleClient(sensor),
    });
    let points = points(&mut events, session);
    connected(&mut events, session);
    let setpoint = index(&points, "Setpoint");
    let temperature = index(&points, "Temperature");
    let body = index(&points, "Temperature Measurement");
    assert_eq!(
        points[usize::try_from(setpoint).expect("an index")].data_type,
        "float32 LE"
    );

    // The vendor setpoint is a float32 by the chosen decoder.
    let start = reading(&mut events, session, setpoint, |found| {
        found.quality == Quality::Good
    });
    assert!((start.value - 22.0).abs() < 1e-6, "{start:?}");
    engine.send(Command::Write {
        session,
        point: setpoint,
        value: 23.5,
    });
    assert_eq!(written(&mut events, session, setpoint), Ok(()));
    reading(&mut events, session, setpoint, |found| {
        (found.value - 23.5).abs() < 1e-6
    });
    // The peripheral refuses bytes that are not a float32.
    engine.send(Command::Ble {
        session,
        request: BleRequest::Write {
            point: setpoint,
            data: vec![1, 2],
        },
    });
    assert!(written(&mut events, session, setpoint).is_err());

    // Indications of an IEEE 11073 body temperature.
    let celsius = reading(&mut events, session, body, |found| {
        found.quality == Quality::Good
    });
    assert!((36.0..=37.0).contains(&celsius.value), "{celsius:?}");

    // Re-decode the temperature as bytes: its type changes and the value
    // becomes the hex behind it.
    engine.send(Command::Ble {
        session,
        request: BleRequest::Decode {
            point: temperature,
            decoder: BleDecoder::of(BleFormat::Hex),
        },
    });
    let redrawn = self::points(&mut events, session);
    assert_eq!(
        redrawn[usize::try_from(temperature).expect("an index")].data_type,
        "Hex"
    );
    let hex = payload(&mut events, session, temperature, |_| true);
    assert_eq!(hex.len(), 5, "two bytes as hex: {hex}");
    let value = reading(&mut events, session, temperature, |_| true);
    assert!(value.quality == Quality::Good && value.value.is_nan());

    engine.send(Command::Close { session });
    assert_eq!(closed(&mut events, session), None);
    engine.send(Command::Shutdown);
}
