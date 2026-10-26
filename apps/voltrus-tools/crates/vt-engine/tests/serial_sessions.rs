//! Serial monitors and the virtual serial device talking over real
//! pseudo-terminal pairs through the engine, the way the app drives them.
//! There is no serial hardware on CI: a pty is the same line discipline a
//! USB adapter's tty uses, minus the wire.
#![cfg(unix)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use mt_serial::pty::VirtualPort;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vt_protocol::{
    Command, Direction, Event, EventKind, LineParser, PointInfo, Quality, Reading,
    SerialDeviceSpec, SerialLink, SerialMonitorSpec, SerialPortInfo, SessionId, SessionSpec,
    SessionState,
};

/// What one session has reported so far.
#[derive(Default)]
struct Seen {
    states: Vec<SessionState>,
    points: Vec<PointInfo>,
    readings: Vec<Reading>,
    rx: Vec<u8>,
    tx: Vec<u8>,
    sent: Vec<Result<u32, String>>,
    logging: Vec<Result<Option<String>, String>>,
    written: Vec<(u32, Result<(), String>)>,
    locked: Vec<(u32, bool)>,
    ports: Option<Result<Vec<SerialPortInfo>, String>>,
    log: Vec<String>,
}

impl Seen {
    fn apply(&mut self, kind: EventKind) {
        match kind {
            EventKind::State(state) => self.states.push(state),
            EventKind::Points(points) => {
                self.readings = vec![Reading::bad(); points.len()];
                self.points = points;
            }
            EventKind::PointsAdded(points) => {
                for (offset, point) in points.iter().enumerate() {
                    assert_eq!(
                        point.id as usize,
                        self.points.len() + offset,
                        "ids in order"
                    );
                }
                self.points.extend(points);
            }
            EventKind::Values(frame) => {
                assert_eq!(
                    frame.readings.len(),
                    self.points.len(),
                    "one reading per point"
                );
                self.readings = frame.readings;
            }
            EventKind::SerialData(chunks) => {
                for chunk in chunks {
                    match chunk.direction {
                        Direction::Rx => self.rx.extend(chunk.bytes),
                        Direction::Tx => self.tx.extend(chunk.bytes),
                    }
                }
            }
            EventKind::SerialSent(result) => self.sent.push(result),
            EventKind::SerialLogging(result) => self.logging.push(result),
            EventKind::SerialPorts(found) => self.ports = Some(found),
            EventKind::Written { point, result } => self.written.push((point, result)),
            EventKind::Locked { point, locked } => self.locked.push((point, locked)),
            EventKind::Log(entry) => self.log.push(entry.message),
            _ => {}
        }
    }

    fn state(&self) -> Option<&SessionState> {
        self.states.last()
    }

    fn value(&self, name: &str) -> Option<f64> {
        let point = self.points.iter().find(|point| point.name == name)?;
        let reading = self.readings.get(point.id as usize)?;
        (reading.quality == Quality::Good).then_some(reading.value)
    }

    fn unit(&self, name: &str) -> Option<&str> {
        self.points
            .iter()
            .find(|point| point.name == name)
            .map(|point| point.unit.as_str())
    }
}

const A: SessionId = SessionId(1);
const B: SessionId = SessionId(2);

struct Engine {
    engine: vt_engine::EngineHandle,
    events: async_channel::Receiver<Event>,
    a: Seen,
    b: Seen,
}

impl Engine {
    fn start() -> Self {
        let (engine, events) = vt_engine::start().expect("the engine starts");
        Self {
            engine,
            events,
            a: Seen::default(),
            b: Seen::default(),
        }
    }

    fn send(&self, command: Command) {
        assert!(self.engine.send(command), "the engine is running");
    }

    /// Take events until `done` holds, or fail after 15 s.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; A logged {:?}, B logged {:?}",
                self.a.log,
                self.b.log
            );
            match self.events.try_recv() {
                Ok(event) if event.session == A => self.a.apply(event.kind),
                Ok(event) if event.session == B => self.b.apply(event.kind),
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.engine.send(Command::Shutdown);
    }
}

/// A link name no other test (or test run) uses.
fn unique_alias() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "voltrus-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn monitor_of(port: &str, parser: LineParser) -> SessionSpec {
    SessionSpec::SerialMonitor(SerialMonitorSpec {
        link: SerialLink {
            port: port.to_owned(),
            baud: 115_200,
            ..SerialLink::default()
        },
        parser,
        ..SerialMonitorSpec::default()
    })
}

/// Read from the device's end until `want` bytes arrived (or 5 s passed).
fn read_device(
    runtime: &tokio::runtime::Runtime,
    device: &mut VirtualPort,
    want: usize,
) -> Vec<u8> {
    runtime.block_on(async {
        let mut got = Vec::new();
        let mut buffer = [0u8; 256];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while got.len() < want {
            match tokio::time::timeout_at(deadline, device.stream.read(&mut buffer)).await {
                Ok(Ok(count)) => got.extend_from_slice(&buffer[..count]),
                Ok(Err(error)) => panic!("reading the device end failed: {error}"),
                Err(_) => break,
            }
        }
        got
    })
}

fn write_device(runtime: &tokio::runtime::Runtime, device: &mut VirtualPort, bytes: &[u8]) {
    runtime
        .block_on(device.stream.write_all(bytes))
        .expect("the device end takes the bytes");
}

#[test]
fn a_monitor_parses_lines_sends_and_logs_over_a_pty() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let mut device = {
        let _context = runtime.enter();
        VirtualPort::open().expect("a pty pair")
    };
    let mut engine = Engine::start();
    engine.send(Command::Open {
        session: A,
        spec: monitor_of(&device.path, LineParser::KeyValue),
    });
    engine.until("the monitor to open the pty", |engine| {
        engine.a.state() == Some(&SessionState::Connected)
    });

    // A line split across two writes, CR LF ended, with units.
    write_device(&runtime, &mut device, b"T=21.5C RH=4");
    write_device(&runtime, &mut device, b"0% status=OK\r\n");
    engine.until("both values", |engine| {
        engine.a.value("T") == Some(21.5) && engine.a.value("RH") == Some(40.0)
    });
    assert_eq!(engine.a.unit("T"), Some("C"));
    assert_eq!(engine.a.unit("RH"), Some("%"));
    assert!(
        engine.a.points.iter().all(|point| point.name != "status"),
        "OK is no number"
    );
    assert!(engine.a.points.iter().all(|point| !point.writable));
    engine.until("the raw bytes on the terminal", |engine| {
        engine.a.rx == b"T=21.5C RH=40% status=OK\r\n"
    });

    // What the app sends reaches the device, byte for byte (binary too).
    let sent = b"PING\r\n\x00\xFF".to_vec();
    engine.send(Command::SerialSend {
        session: A,
        bytes: sent.clone(),
    });
    engine.until("the send to complete", |engine| !engine.a.sent.is_empty());
    assert_eq!(engine.a.sent[0], Ok(8));
    assert_eq!(read_device(&runtime, &mut device, sent.len()), sent);
    engine.until("the send on the terminal", |engine| engine.a.tx == sent);

    // Points take no writes: the reason says what to do instead.
    engine.send(Command::Write {
        session: A,
        point: 0,
        value: 1.0,
    });
    engine.until("the write refused", |engine| !engine.a.written.is_empty());
    assert!(
        engine.a.written[0]
            .1
            .as_ref()
            .is_err_and(|error| error.contains("send a command"))
    );

    // Logging: every chunk from now on goes to the file.
    let log = std::env::temp_dir().join(format!("{}.csv", unique_alias()));
    let log_path = log.to_string_lossy().into_owned();
    engine.send(Command::SerialLog {
        session: A,
        path: Some(log_path.clone()),
    });
    engine.until("logging to start", |engine| !engine.a.logging.is_empty());
    assert_eq!(engine.a.logging[0], Ok(Some(log_path.clone())));
    write_device(&runtime, &mut device, b"T=22\n");
    engine.until("the next value", |engine| engine.a.value("T") == Some(22.0));
    engine.send(Command::SerialLog {
        session: A,
        path: None,
    });
    engine.until("logging to stop", |engine| engine.a.logging.len() == 2);
    assert_eq!(engine.a.logging[1], Ok(None));
    let text = std::fs::read_to_string(&log).expect("the log file");
    let mut rows = text.lines();
    assert_eq!(rows.next(), Some("timestamp_utc,port,direction,hex,ascii"));
    let row = rows.next().expect("the received line");
    assert!(row.ends_with(",RX,54 3D 32 32 0A,T=22."), "{row}");
    assert_eq!(rows.next(), None, "nothing after logging stopped");
    let _ = std::fs::remove_file(&log);

    // A log the system refuses is an error in the app, not a crash.
    engine.send(Command::SerialLog {
        session: A,
        path: Some("/voltrus-no-such-folder/log.csv".into()),
    });
    engine.until("the refusal", |engine| engine.a.logging.len() == 3);
    assert!(
        engine.a.logging[2]
            .as_ref()
            .is_err_and(|error| error.contains("Cannot create"))
    );

    engine.send(Command::Close { session: A });
    engine.until("the monitor to close", |engine| {
        engine.a.state() == Some(&SessionState::Closed { error: None })
    });
}

#[test]
fn a_monitor_waits_for_a_port_and_reads_the_virtual_device() {
    let mut engine = Engine::start();
    let device = SerialDeviceSpec {
        alias: unique_alias(),
        interval_ms: 200,
        ..SerialDeviceSpec::default()
    };
    let link = device.path();

    // The monitor starts first: its port is not there yet.
    engine.send(Command::Open {
        session: B,
        spec: SessionSpec::SerialMonitor(device.monitor()),
    });
    engine.until("the monitor to wait for its port", |engine| {
        matches!(engine.b.state(), Some(SessionState::Reconnecting { reason, .. }) if reason.contains(&device.alias))
    });

    engine.send(Command::Open {
        session: A,
        spec: SessionSpec::SerialDevice(device.clone()),
    });
    engine.until("the device to listen", |engine| {
        matches!(engine.a.state(), Some(SessionState::Listening { .. }))
    });
    assert!(
        std::fs::symlink_metadata(&link).is_ok_and(|meta| meta.file_type().is_symlink()),
        "a link at {link}"
    );
    engine.until("the monitor to find the device", |engine| {
        engine.b.state() == Some(&SessionState::Connected)
    });
    engine.until("the printed values as points", |engine| {
        engine.b.value("Temperature").is_some() && engine.b.value("pH").is_some()
    });
    assert_eq!(engine.b.unit("Temperature"), Some("°C"));
    assert_eq!(engine.b.unit("ORP"), Some("mV"));

    // A command typed at the device holds a value; the device tells the app
    // and the monitor reads it back.
    engine.send(Command::SerialSend {
        session: B,
        bytes: b"SET Temperature 33.5\r\n".to_vec(),
    });
    engine.until("the device to hold the value", |engine| {
        engine.a.locked.contains(&(6, true))
    });
    engine.until("the monitor to read the held value", |engine| {
        engine.b.value("Temperature") == Some(33.5)
    });
    engine.until("the reply on the monitor's terminal", |engine| {
        String::from_utf8_lossy(&engine.b.rx).contains("OK Temperature=33.5\r\n")
    });
    engine.until("the command on the device's terminal", |engine| {
        String::from_utf8_lossy(&engine.a.rx).contains("SET Temperature 33.5")
    });

    // A write from the app to the device's point is held as well.
    engine.send(Command::Write {
        session: A,
        point: 0,
        value: 8.25,
    });
    engine.until("the app's write", |engine| !engine.a.written.is_empty());
    assert_eq!(engine.a.written[0], (0, Ok(())));
    engine.until("the monitor to see pH 8.25", |engine| {
        engine.b.value("pH") == Some(8.25)
    });

    // Stopping the device takes its link away; the monitor waits again.
    engine.send(Command::Close { session: A });
    engine.until("the device to stop", |engine| {
        engine.a.state() == Some(&SessionState::Closed { error: None })
    });
    assert!(
        std::fs::symlink_metadata(&link).is_err(),
        "the link is gone"
    );
    engine.until("the monitor to lose the port", |engine| {
        matches!(engine.b.state(), Some(SessionState::Reconnecting { .. }))
    });
    engine.send(Command::Close { session: B });
    engine.until("the monitor to close", |engine| {
        engine.b.state() == Some(&SessionState::Closed { error: None })
    });
}

#[test]
fn bad_settings_close_with_a_reason() {
    let mut engine = Engine::start();
    engine.send(Command::Open {
        session: A,
        spec: monitor_of(
            "/dev/null",
            LineParser::Regex {
                pattern: "T=\\d+".into(),
            },
        ),
    });
    engine.send(Command::Open {
        session: B,
        spec: SessionSpec::SerialMonitor(SerialMonitorSpec {
            link: SerialLink {
                port: "/dev/null".into(),
                data_bits: 9,
                ..SerialLink::default()
            },
            ..SerialMonitorSpec::default()
        }),
    });
    engine.until("both to close", |engine| {
        matches!(
            engine.a.state(),
            Some(SessionState::Closed { error: Some(_) })
        ) && matches!(
            engine.b.state(),
            Some(SessionState::Closed { error: Some(_) })
        )
    });
    let Some(SessionState::Closed { error: Some(regex) }) = engine.a.state() else {
        unreachable!()
    };
    assert!(regex.contains("capture group"), "{regex}");
    let Some(SessionState::Closed { error: Some(bits) }) = engine.b.state() else {
        unreachable!()
    };
    assert!(bits.contains("data bits"), "{bits}");
}

#[test]
fn the_port_list_answers_under_its_request() {
    let mut engine = Engine::start();
    engine.send(Command::ListSerialPorts { session: A });
    engine.until("the port list", |engine| engine.a.ports.is_some());
    let ports = engine
        .a
        .ports
        .clone()
        .expect("an answer")
        .expect("the system lists its ports");
    // A runner may have no ports at all: which call-in twins are dropped is
    // tested on a made-up list next to `without_call_in_twins`. Here: the
    // list comes sorted, each port once.
    assert!(
        ports.windows(2).all(|pair| pair[0].path < pair[1].path),
        "{ports:?}"
    );
}
