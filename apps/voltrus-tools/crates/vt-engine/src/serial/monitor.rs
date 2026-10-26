//! A serial monitor session: opens the port (waiting for it while it is
//! missing or busy, so a device plugged in later is picked up), reports
//! every chunk to the terminal, writes what the app sends, logs to a file
//! and parses received lines into points.

use std::io;
use std::time::Duration;

use mt_serial::{SerialConfig, SerialStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use vt_protocol::{
    Direction, FlowControl, LogLevel, Parity, SerialChunk, SerialMonitorSpec, SessionState, Stats,
    ValueFrame,
};

use super::{Lines, LogFile, Parser};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// How often a monitor's values may arrive, for sizing trend history: lines
/// come as fast as the device prints them.
pub(crate) const SAMPLE: Duration = Duration::from_millis(100);
/// How often received bytes go to the app, batched.
const FLUSH: Duration = Duration::from_millis(50);
/// Wait between attempts to open a missing or busy port.
const RETRY: Duration = Duration::from_secs(1);
/// Longest a send may take before it is given up (a port held by flow
/// control, a cable pulled mid-write).
const SEND_WAIT: Duration = Duration::from_secs(5);

pub(crate) fn config(link: &vt_protocol::SerialLink, flow: FlowControl) -> SerialConfig {
    SerialConfig {
        port_name: link.port.clone(),
        baud_rate: link.baud,
        data_bits: link.data_bits,
        stop_bits: link.stop_bits,
        parity: match link.parity {
            Parity::None => mt_serial::Parity::None,
            Parity::Odd => mt_serial::Parity::Odd,
            Parity::Even => mt_serial::Parity::Even,
        },
        flow_control: match flow {
            FlowControl::None => mt_serial::FlowControl::None,
            FlowControl::Hardware => mt_serial::FlowControl::Hardware,
            FlowControl::Software => mt_serial::FlowControl::Software,
        },
        timeout_ms: 1000,
    }
}

/// What the monitor keeps across reconnects.
struct Monitor<'a> {
    spec: SerialMonitorSpec,
    parser: Parser,
    lines: Lines,
    log: Option<LogFile>,
    stats: Stats,
    pending: Vec<SerialChunk>,
    out: &'a Emitter,
}

enum Ended {
    /// The app closed the session.
    Closed,
    /// The port went away; try again.
    Lost(String),
}

pub(crate) async fn run(
    spec: SerialMonitorSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let parser = match Parser::new(&spec.parser) {
        Ok(parser) => parser,
        Err(error) => {
            out.log(LogLevel::Error, error.clone());
            out.state(SessionState::Closed { error: Some(error) });
            return;
        }
    };
    out.state(SessionState::Connecting);
    out.points(Vec::new());
    let config = config(&spec.link, spec.flow);
    let mut monitor = Monitor {
        spec,
        parser,
        lines: Lines::default(),
        log: None,
        stats: Stats::default(),
        pending: Vec::new(),
        out,
    };
    let mut attempt: u32 = 0;
    let mut last_reason = String::new();
    loop {
        let reason = match mt_serial::open_stream(&config) {
            Ok(mut stream) => {
                attempt = 0;
                last_reason.clear();
                out.state(SessionState::Connected);
                out.log(
                    LogLevel::Info,
                    format!(
                        "Opened {} at {} baud, {}{}{}, flow control {}",
                        vt_protocol::short_port(&config.port_name),
                        config.baud_rate,
                        config.data_bits,
                        monitor.spec.link.parity.letter(),
                        config.stop_bits,
                        monitor.spec.flow.name()
                    ),
                );
                match monitor.connected(&mut stream, &mut control).await {
                    Ended::Closed => break,
                    Ended::Lost(reason) => reason,
                }
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                let message = format!("{}: {error}", config.port_name);
                out.log(LogLevel::Error, message.clone());
                monitor.stop_log();
                out.state(SessionState::Closed {
                    error: Some(message),
                });
                return;
            }
            Err(error) => format!("{}: {error}", vt_protocol::short_port(&config.port_name)),
        };
        // Say why once, not every second while the port stays away.
        if reason != last_reason {
            out.log(LogLevel::Warn, reason.clone());
            last_reason.clone_from(&reason);
        }
        attempt = attempt.saturating_add(1);
        monitor.stats.errors += 1;
        out.stats(monitor.stats);
        out.state(SessionState::Reconnecting {
            attempt,
            retry_in_ms: u64::try_from(RETRY.as_millis()).unwrap_or(u64::MAX),
            reason,
        });
        if !monitor.wait(&mut control).await {
            break;
        }
    }
    monitor.flush();
    monitor.stop_log();
    out.state(SessionState::Closed { error: None });
}

impl Monitor<'_> {
    /// Between attempts: answer the app until it is time to try again.
    /// `false` once the app closed the session.
    async fn wait(&mut self, control: &mut mpsc::UnboundedReceiver<Control>) -> bool {
        let retry = tokio::time::sleep(RETRY);
        tokio::pin!(retry);
        loop {
            tokio::select! {
                () = &mut retry => return true,
                command = control.recv() => match command {
                    None | Some(Control::Close) => return false,
                    Some(Control::SerialSend { .. }) => {
                        self.out.serial_sent(Err("The port is not open".into()));
                    }
                    Some(Control::SerialLog { path }) => self.set_log(path),
                    Some(Control::Write { point, .. }) => self.out.written(point, Err(READ_ONLY.into())),
                    Some(_) => {}
                },
            }
        }
    }

    async fn connected(
        &mut self,
        stream: &mut SerialStream,
        control: &mut mpsc::UnboundedReceiver<Control>,
    ) -> Ended {
        let mut buffer = vec![0u8; 4096];
        let mut flush = tokio::time::interval(FLUSH);
        flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut dirty = false;
        loop {
            tokio::select! {
                read = stream.read(&mut buffer) => match read {
                    Ok(0) => {
                        self.flush();
                        return Ended::Lost(format!("{} closed", self.port()));
                    }
                    Ok(count) => {
                        self.received(&buffer[..count]);
                        dirty = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        self.flush();
                        return Ended::Lost(format!("{}: {error}", self.port()));
                    }
                },
                _ = flush.tick() => {
                    if dirty {
                        self.flush();
                        dirty = false;
                    }
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        self.flush();
                        return Ended::Closed;
                    }
                    Some(Control::SerialSend { bytes }) => {
                        // No flush: on a tty it is tcdrain, which blocks the
                        // runtime thread (beyond any timeout) until the far
                        // end reads. Written bytes are in the driver's queue.
                        let sent =
                            tokio::time::timeout(SEND_WAIT, stream.write_all(&bytes)).await;
                        match sent {
                            Ok(Ok(())) => {
                                self.chunk(Direction::Tx, bytes.clone());
                                self.out.serial_sent(Ok(u32::try_from(bytes.len()).unwrap_or(u32::MAX)));
                                dirty = true;
                            }
                            Ok(Err(error)) => {
                                let message = format!("Sending failed: {error}");
                                self.out.log(LogLevel::Error, message.clone());
                                self.out.serial_sent(Err(message));
                            }
                            Err(_) => {
                                let message = format!(
                                    "Sending timed out after {} s: is flow control holding the port?",
                                    SEND_WAIT.as_secs()
                                );
                                self.out.log(LogLevel::Error, message.clone());
                                self.out.serial_sent(Err(message));
                            }
                        }
                    }
                    Some(Control::SerialLog { path }) => self.set_log(path),
                    Some(Control::Write { point, .. }) => self.out.written(point, Err(READ_ONLY.into())),
                    Some(_) => {}
                },
            }
        }
    }

    fn port(&self) -> &str {
        vt_protocol::short_port(&self.spec.link.port)
    }

    /// Bytes from the device: to the terminal and the log, and through the
    /// parser into values.
    fn received(&mut self, bytes: &[u8]) {
        self.chunk(Direction::Rx, bytes.to_vec());
        if self.parser.is_none() {
            return;
        }
        let mut lines = Vec::new();
        self.lines.push(bytes, &mut lines);
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parsed = self.parser.line(&line);
            if !parsed.added.is_empty() {
                self.out.points_added(parsed.added);
            }
            if parsed.values {
                self.stats.polls += 1;
                self.out.values(ValueFrame {
                    at_ms: now_ms(),
                    latency_us: 0,
                    readings: self.parser.readings(),
                });
            } else {
                self.stats.errors += 1;
            }
        }
    }

    fn chunk(&mut self, direction: Direction, bytes: Vec<u8>) {
        let chunk = SerialChunk {
            at_ms: now_ms(),
            direction,
            bytes,
        };
        self.stats.frames += chunk.bytes.len() as u64;
        if let Some(log) = &mut self.log
            && let Err(error) = log.record(&chunk)
        {
            let message = format!("Logging to {} stopped: {error}", log.path);
            self.log = None;
            self.out.log(LogLevel::Error, message.clone());
            self.out.serial_logging(Err(message));
        }
        self.pending.push(chunk);
    }

    /// Send the batched chunks and counters to the app.
    fn flush(&mut self) {
        self.out.serial_data(std::mem::take(&mut self.pending));
        self.out.stats(self.stats);
        if let Some(log) = &mut self.log
            && let Err(error) = log.flush()
        {
            let message = format!("Logging to {} stopped: {error}", log.path);
            self.log = None;
            self.out.log(LogLevel::Error, message.clone());
            self.out.serial_logging(Err(message));
        }
    }

    fn set_log(&mut self, path: Option<String>) {
        self.stop_log();
        let Some(path) = path else {
            self.out.serial_logging(Ok(None));
            return;
        };
        match LogFile::create(&path, &self.spec.link.port) {
            Ok(log) => {
                self.out
                    .log(LogLevel::Info, format!("Logging every byte to {path}"));
                self.log = Some(log);
                self.out.serial_logging(Ok(Some(path)));
            }
            Err(error) => {
                self.out.log(LogLevel::Error, error.clone());
                self.out.serial_logging(Err(error));
            }
        }
    }

    fn stop_log(&mut self) {
        if let Some(mut log) = self.log.take() {
            let _ = log.flush();
            self.out
                .log(LogLevel::Info, format!("Stopped logging to {}", log.path));
        }
    }
}

/// Why a monitor's points take no writes.
pub(crate) const READ_ONLY: &str =
    "Serial points come from received lines; send a command to the device instead";
