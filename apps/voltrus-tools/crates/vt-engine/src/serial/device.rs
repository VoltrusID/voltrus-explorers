//! A serial instrument on a virtual port (a pseudo-terminal): prints a
//! device template's drifting values (the Modbus templates' simulation) as
//! `key=value` or CSV lines, and answers commands typed at it:
//!
//! - `HELP` or `?` lists the commands
//! - `READ` prints the values now
//! - `SET <name> <value>` holds a value, `RELEASE <name|ALL>` lets it drift
//! - `RATE <ms>` changes how often it prints
//!
//! A stable link in the temporary folder (`SerialDeviceSpec::path`) points
//! at the port, so a monitor finds the device again after a restart.

use std::time::Duration;

use mt_modbus::{SimDeviceProfile, Simulation, tick_values};
use tokio::sync::mpsc;
use vt_protocol::{
    LogLevel, PointInfo, Quality, Reading, SerialDeviceSpec, SerialFormat, SessionState, ValueFrame,
};

use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::mqtt::format_number;

/// How often the app sees the simulated values.
pub(crate) const REPORT_TICK: Duration = Duration::from_millis(250);
/// CSV output repeats its header this often, so a monitor that opens the
/// port later learns the column names.
const HEADER_EVERY: u32 = 20;

/// The instrument: its simulation and how it prints.
pub(crate) struct Instrument {
    simulation: Simulation,
    format: SerialFormat,
    lines_since_header: u32,
    pub interval_ms: u64,
}

/// What a command line asks of the device.
#[derive(Debug, PartialEq)]
pub(crate) enum Reply {
    /// Text to print back.
    Text(String),
    /// Text, and the point held (or released).
    Held {
        text: String,
        point: u32,
        held: bool,
    },
    /// Text, and every point released.
    ReleasedAll(String),
}

impl Instrument {
    pub fn new(profile: SimDeviceProfile, spec: &SerialDeviceSpec) -> Self {
        Self {
            simulation: Simulation::new(profile),
            format: spec.format,
            lines_since_header: HEADER_EVERY,
            interval_ms: spec.interval_ms.max(SerialDeviceSpec::MIN_INTERVAL_MS),
        }
    }

    pub fn points(&self) -> Vec<PointInfo> {
        self.simulation
            .profile
            .registers
            .iter()
            .enumerate()
            .map(|(index, register)| PointInfo {
                id: u32::try_from(index).unwrap_or(u32::MAX),
                name: register.name.clone(),
                address: match self.format {
                    SerialFormat::KeyValue => format!("key {}", index + 1),
                    SerialFormat::Csv => format!("col {}", index + 1),
                },
                data_type: "Number".into(),
                unit: register.unit.clone(),
                writable: true,
                boolean: false,
                states: Vec::new(),
            })
            .collect()
    }

    pub fn frame(&self) -> ValueFrame {
        ValueFrame {
            at_ms: now_ms(),
            latency_us: 0,
            readings: self
                .simulation
                .values
                .iter()
                .map(|value| Reading {
                    value: *value,
                    raw: [0; 4],
                    raw_len: 0,
                    quality: Quality::Good,
                })
                .collect(),
        }
    }

    pub fn tick(&mut self) {
        tick_values(&mut self.simulation);
    }

    /// The next output: the values as a line (with the CSV header when due),
    /// ended by CR LF.
    pub fn print(&mut self) -> String {
        let registers = &self.simulation.profile.registers;
        let values = &self.simulation.values;
        match self.format {
            SerialFormat::KeyValue => {
                let fields: Vec<String> = registers
                    .iter()
                    .zip(values)
                    .map(|(register, value)| {
                        let unit = if register.unit.eq_ignore_ascii_case(&register.name) {
                            ""
                        } else {
                            register.unit.as_str()
                        };
                        format!("{}={}{unit}", register.name, format_number(*value))
                    })
                    .collect();
                format!("{}\r\n", fields.join(" "))
            }
            SerialFormat::Csv => {
                let mut text = String::new();
                if self.lines_since_header >= HEADER_EVERY {
                    self.lines_since_header = 0;
                    let names: Vec<&str> = registers
                        .iter()
                        .map(|register| register.name.as_str())
                        .collect();
                    text.push_str(&names.join(","));
                    text.push_str("\r\n");
                }
                self.lines_since_header += 1;
                let fields: Vec<String> =
                    values.iter().map(|value| format_number(*value)).collect();
                text.push_str(&fields.join(","));
                text.push_str("\r\n");
                text
            }
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.simulation
            .profile
            .registers
            .iter()
            .position(|register| register.name.eq_ignore_ascii_case(name))
    }

    /// Set a value and hold it.
    pub fn set(&mut self, point: u32, value: f64) -> Result<(), String> {
        let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
        let register = self
            .simulation
            .profile
            .registers
            .get(index)
            .ok_or_else(|| format!("No point {point}"))?;
        if !value.is_finite() || !(register.min..=register.max).contains(&value) {
            return Err(format!(
                "{} takes {}..{} {}",
                register.name, register.min, register.max, register.unit
            ));
        }
        self.simulation.values[index] = value;
        self.simulation.locks.set(index, true);
        Ok(())
    }

    /// Hold or release a value. Returns whether `point` exists.
    pub fn lock(&mut self, point: u32, locked: bool) -> bool {
        match usize::try_from(point) {
            Ok(index) if index < self.simulation.profile.registers.len() => {
                self.simulation.locks.set(index, locked);
                true
            }
            _ => false,
        }
    }

    /// Answer a line typed at the device; `None` for a blank line.
    pub fn command(&mut self, line: &str) -> Option<Reply> {
        let mut words = line.split_whitespace();
        let verb = words.next()?.to_ascii_uppercase();
        let reply = match verb.as_str() {
            "HELP" | "?" => Reply::Text(
                "Commands: READ, SET <name> <value>, RELEASE <name|ALL>, RATE <ms>, HELP\r\n"
                    .into(),
            ),
            "READ" => Reply::Text(self.print()),
            "SET" => match (words.next(), words.next().map(str::parse::<f64>)) {
                (Some(name), Some(Ok(value))) => match self.index(name) {
                    Some(index) => {
                        let point = u32::try_from(index).unwrap_or(u32::MAX);
                        match self.set(point, value) {
                            Ok(()) => Reply::Held {
                                text: format!(
                                    "OK {}={}\r\n",
                                    self.simulation.profile.registers[index].name,
                                    format_number(value)
                                ),
                                point,
                                held: true,
                            },
                            Err(error) => Reply::Text(format!("ERR {error}\r\n")),
                        }
                    }
                    None => Reply::Text(format!("ERR no value named {name}\r\n")),
                },
                _ => Reply::Text("ERR usage: SET <name> <value>\r\n".into()),
            },
            "RELEASE" => match words.next() {
                Some(name) if name.eq_ignore_ascii_case("ALL") => {
                    for index in 0..self.simulation.profile.registers.len() {
                        self.simulation.locks.set(index, false);
                    }
                    Reply::ReleasedAll("OK released all\r\n".into())
                }
                Some(name) => match self.index(name) {
                    Some(index) => {
                        self.simulation.locks.set(index, false);
                        Reply::Held {
                            text: format!(
                                "OK released {}\r\n",
                                self.simulation.profile.registers[index].name
                            ),
                            point: u32::try_from(index).unwrap_or(u32::MAX),
                            held: false,
                        }
                    }
                    None => Reply::Text(format!("ERR no value named {name}\r\n")),
                },
                None => Reply::Text("ERR usage: RELEASE <name|ALL>\r\n".into()),
            },
            "RATE" => match words.next().map(str::parse::<u64>) {
                Some(Ok(ms)) if (SerialDeviceSpec::MIN_INTERVAL_MS..=3_600_000).contains(&ms) => {
                    self.interval_ms = ms;
                    Reply::Text(format!("OK RATE {ms}\r\n"))
                }
                _ => Reply::Text(format!(
                    "ERR usage: RATE <ms>, {} to 3600000\r\n",
                    SerialDeviceSpec::MIN_INTERVAL_MS
                )),
            },
            _ => Reply::Text(format!("ERR unknown command {verb}: HELP lists them\r\n")),
        };
        Some(reply)
    }

    pub fn point_count(&self) -> usize {
        self.simulation.profile.registers.len()
    }
}

#[cfg(unix)]
pub(crate) async fn run(
    spec: SerialDeviceSpec,
    control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let Some(profile) = crate::templates::find_modbus(&spec.template) else {
        let message = format!("Unknown device template {}", spec.template);
        out.log(LogLevel::Error, message.clone());
        out.state(SessionState::Closed {
            error: Some(message),
        });
        return;
    };
    let instrument = Instrument::new(profile.clone(), &spec);
    let port = match mt_serial::pty::VirtualPort::open() {
        Ok(port) => port,
        Err(error) => {
            let message = format!("Could not create a virtual serial port: {error}");
            out.log(LogLevel::Error, message.clone());
            out.state(SessionState::Closed {
                error: Some(message),
            });
            return;
        }
    };
    unix::serve(spec, instrument, port, control, out).await;
}

#[cfg(not(unix))]
pub(crate) async fn run(
    _spec: SerialDeviceSpec,
    _control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let message = "Virtual serial ports need macOS or Linux".to_owned();
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

#[cfg(unix)]
mod unix {
    use std::path::Path;
    use std::time::Duration;

    use mt_serial::pty::VirtualPort;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::mpsc;
    use tokio::time::{Instant, MissedTickBehavior};
    use vt_protocol::{Direction, LogLevel, SerialChunk, SerialDeviceSpec, SessionState, Stats};

    use super::{Instrument, REPORT_TICK, Reply};
    use crate::Control;
    use crate::emit::{Emitter, now_ms};
    use crate::serial::{Lines, LogFile};

    /// A write the port cannot take this long (nobody reads it and its
    /// buffer is full) is dropped, as a real line would be.
    const WRITE_WAIT: Duration = Duration::from_millis(200);

    pub(super) async fn serve(
        spec: SerialDeviceSpec,
        mut instrument: Instrument,
        mut port: VirtualPort,
        mut control: mpsc::UnboundedReceiver<Control>,
        out: &Emitter,
    ) {
        let link = spec.path();
        let linked = make_link(&port.path, &link);
        if let Err(error) = &linked {
            out.log(
                LogLevel::Warn,
                format!(
                    "No link at {link} ({error}); monitors must open {}",
                    port.path
                ),
            );
        }
        out.state(SessionState::Listening {
            address: port.path.clone(),
        });
        out.log(
            LogLevel::Info,
            format!(
                "{} {} printing on virtual port {} (link {})",
                instrument.simulation.profile.manufacturer,
                instrument.simulation.profile.model,
                port.path,
                spec.alias
            ),
        );
        out.points(instrument.points());
        out.values(instrument.frame());

        let mut log: Option<LogFile> = None;
        let mut lines = Lines::default();
        let mut pending: Vec<SerialChunk> = Vec::new();
        let mut stats = Stats::default();
        let mut buffer = vec![0u8; 1024];
        let mut next_print = Instant::now() + Duration::from_millis(instrument.interval_ms);
        let mut report = tokio::time::interval(REPORT_TICK);
        report.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                () = tokio::time::sleep_until(next_print) => {
                    instrument.tick();
                    let text = instrument.print();
                    write(&mut port, text.into_bytes(), &mut pending, &mut log, out).await;
                    stats.polls += 1;
                    next_print = Instant::now() + Duration::from_millis(instrument.interval_ms);
                }
                read = port.stream.read(&mut buffer) => match read {
                    Ok(count) if count > 0 => {
                        let bytes = buffer[..count].to_vec();
                        let chunk = SerialChunk { at_ms: now_ms(), direction: Direction::Rx, bytes };
                        record(&mut log, &chunk, out);
                        let mut received = Vec::new();
                        lines.push(&chunk.bytes, &mut received);
                        stats.frames += chunk.bytes.len() as u64;
                        pending.push(chunk);
                        for line in received {
                            let Some(reply) = instrument.command(&line) else {
                                continue;
                            };
                            out.log(LogLevel::Info, format!("Received {}", line.trim()));
                            let text = match reply {
                                Reply::Text(text) => text,
                                Reply::Held { text, point, held } => {
                                    out.locked(point, held);
                                    out.values(instrument.frame());
                                    text
                                }
                                Reply::ReleasedAll(text) => {
                                    for point in 0..instrument.point_count() {
                                        out.locked(u32::try_from(point).unwrap_or(u32::MAX), false);
                                    }
                                    text
                                }
                            };
                            write(&mut port, text.into_bytes(), &mut pending, &mut log, out).await;
                        }
                    }
                    // Nobody has the port open: a pty reports a hang-up until
                    // someone does. Wait a little rather than spin.
                    Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
                _ = report.tick() => {
                    out.values(instrument.frame());
                    out.serial_data(std::mem::take(&mut pending));
                    out.stats(stats);
                    if let Some(file) = &mut log
                        && let Err(error) = file.flush()
                    {
                        stop_on_error(&mut log, &error, out);
                    }
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => break,
                    Some(Control::Write { point, value }) => {
                        let result = instrument.set(point, value);
                        let held = result.is_ok();
                        out.written(point, result);
                        if held {
                            out.locked(point, true);
                            out.values(instrument.frame());
                        }
                    }
                    Some(Control::Lock { point, locked }) => {
                        if instrument.lock(point, locked) {
                            out.locked(point, locked);
                        }
                    }
                    Some(Control::SerialSend { bytes }) => {
                        let count = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
                        if write(&mut port, bytes, &mut pending, &mut log, out).await {
                            out.serial_sent(Ok(count));
                        } else {
                            out.serial_sent(Err("Nobody is reading the port: the bytes were dropped".into()));
                        }
                    }
                    Some(Control::SerialLog { path }) => {
                        if let Some(mut file) = log.take() {
                            let _ = file.flush();
                            out.log(LogLevel::Info, format!("Stopped logging to {}", file.path));
                        }
                        match path {
                            None => out.serial_logging(Ok(None)),
                            Some(path) => match LogFile::create(&path, &port.path) {
                                Ok(file) => {
                                    out.log(LogLevel::Info, format!("Logging every byte to {path}"));
                                    log = Some(file);
                                    out.serial_logging(Ok(Some(path)));
                                }
                                Err(error) => {
                                    out.log(LogLevel::Error, error.clone());
                                    out.serial_logging(Err(error));
                                }
                            },
                        }
                    }
                    Some(_) => {}
                },
            }
        }
        if let Some(mut file) = log.take() {
            let _ = file.flush();
        }
        if linked.is_ok() {
            remove_link(&port.path, &link);
        }
        out.serial_data(pending);
        out.stats(stats);
        drop(port);
        out.state(SessionState::Closed { error: None });
    }

    /// Write `bytes` to the port and the terminal; `false` when the port
    /// could not take them in time.
    async fn write(
        port: &mut VirtualPort,
        bytes: Vec<u8>,
        pending: &mut Vec<SerialChunk>,
        log: &mut Option<LogFile>,
        out: &Emitter,
    ) -> bool {
        let written = matches!(
            tokio::time::timeout(WRITE_WAIT, port.stream.write_all(&bytes)).await,
            Ok(Ok(()))
        );
        if written {
            let chunk = SerialChunk {
                at_ms: now_ms(),
                direction: Direction::Tx,
                bytes,
            };
            record(log, &chunk, out);
            pending.push(chunk);
        }
        written
    }

    fn record(log: &mut Option<LogFile>, chunk: &SerialChunk, out: &Emitter) {
        if let Some(file) = log
            && let Err(error) = file.record(chunk)
        {
            stop_on_error(log, &error, out);
        }
    }

    fn stop_on_error(log: &mut Option<LogFile>, error: &std::io::Error, out: &Emitter) {
        if let Some(file) = log.take() {
            let message = format!("Logging to {} stopped: {error}", file.path);
            out.log(LogLevel::Error, message.clone());
            out.serial_logging(Err(message));
        }
    }

    /// Point `link` at the port, replacing a stale link left by an earlier
    /// run (but never a file that is not a link).
    fn make_link(target: &str, link: &str) -> std::io::Result<()> {
        let path = Path::new(link);
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() {
                std::fs::remove_file(path)?;
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "a file that is not a link is in the way",
                ));
            }
        }
        std::os::unix::fs::symlink(target, path)
    }

    /// Remove `link` if it still points at this run's port.
    fn remove_link(target: &str, link: &str) {
        if std::fs::read_link(link).is_ok_and(|current| current == Path::new(target)) {
            let _ = std::fs::remove_file(link);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instrument(format: SerialFormat) -> Instrument {
        let profile = crate::templates::find_modbus("abb-analyzer")
            .expect("the ABB template")
            .clone();
        Instrument::new(
            profile,
            &SerialDeviceSpec {
                format,
                ..SerialDeviceSpec::default()
            },
        )
    }

    #[test]
    fn key_value_lines_name_every_value_with_its_unit() {
        let mut device = instrument(SerialFormat::KeyValue);
        let line = device.print();
        assert!(line.ends_with("\r\n"));
        assert!(line.starts_with("pH=7.2 "), "pH carries no unit: {line}");
        assert!(line.contains(" ORP=245mV "), "{line}");
        assert!(line.contains(" Temperature=15.8°C"), "{line}");
        // What it prints, the monitor's parser reads back.
        let mut parser = crate::serial::Parser::new(&vt_protocol::LineParser::KeyValue).unwrap();
        let parsed = parser.line(line.trim_end());
        assert_eq!(parsed.added.len(), device.point_count());
        assert_eq!(parsed.added[1].unit, "mV");
    }

    #[test]
    fn csv_repeats_its_header() {
        let mut device = instrument(SerialFormat::Csv);
        let first = device.print();
        let mut rows = first.split("\r\n");
        assert_eq!(
            rows.next(),
            Some("pH,ORP,Turbidity,Dissolved_Oxygen,Conductivity,Chlorine_Free,Temperature")
        );
        assert!(rows.next().is_some_and(|row| row.split(',').count() == 7));
        for _ in 1..HEADER_EVERY {
            assert!(
                !device.print().starts_with("pH,"),
                "one header per 20 lines"
            );
        }
        assert!(device.print().starts_with("pH,ORP"));
    }

    #[test]
    fn commands_set_hold_release_and_rate() {
        let mut device = instrument(SerialFormat::KeyValue);
        assert_eq!(device.command("   "), None);
        assert!(
            matches!(device.command("help"), Some(Reply::Text(text)) if text.starts_with("Commands:"))
        );
        assert_eq!(
            device.command("set temperature 21.5"),
            Some(Reply::Held {
                text: "OK Temperature=21.5\r\n".into(),
                point: 6,
                held: true
            })
        );
        device.tick();
        assert_eq!(device.frame().readings[6].value, 21.5, "a held value stays");
        assert!(
            matches!(device.command("SET Temperature 99"), Some(Reply::Text(text)) if text.starts_with("ERR Temperature takes 0..40"))
        );
        assert!(
            matches!(device.command("SET Nope 1"), Some(Reply::Text(text)) if text.contains("no value named Nope"))
        );
        assert!(
            matches!(device.command("SET Temperature x"), Some(Reply::Text(text)) if text.contains("usage"))
        );
        assert_eq!(
            device.command("RELEASE Temperature"),
            Some(Reply::Held {
                text: "OK released Temperature\r\n".into(),
                point: 6,
                held: false
            })
        );
        assert!(matches!(
            device.command("RELEASE all"),
            Some(Reply::ReleasedAll(_))
        ));
        assert_eq!(
            device.command("RATE 250"),
            Some(Reply::Text("OK RATE 250\r\n".into()))
        );
        assert_eq!(device.interval_ms, 250);
        assert!(
            matches!(device.command("RATE 5"), Some(Reply::Text(text)) if text.starts_with("ERR"))
        );
        assert_eq!(device.interval_ms, 250);
        assert!(
            matches!(device.command("READ"), Some(Reply::Text(text)) if text.starts_with("pH="))
        );
        assert!(
            matches!(device.command("FOO"), Some(Reply::Text(text)) if text.contains("unknown command FOO"))
        );
    }

    #[test]
    fn app_writes_hold_within_range_only() {
        let mut device = instrument(SerialFormat::KeyValue);
        assert!(device.set(0, 8.0).is_ok());
        assert!(device.set(0, 15.0).is_err(), "pH stops at 14");
        assert!(device.set(0, f64::NAN).is_err());
        assert!(device.set(99, 1.0).is_err());
        assert!(device.lock(6, false));
        assert!(!device.lock(99, true));
    }
}
