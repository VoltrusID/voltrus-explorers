//! Serves a Modbus register image, optionally driven by a device template.
//!
//! The session owns the [`Simulation`] instead of handing it to
//! `start_simulation`, so a value the user sets is written into the simulation
//! (and locked) rather than overwritten on the next tick. The register image
//! is the truth: before the simulation moves, values an external master wrote
//! are taken into the simulation and held, and the app sees the image itself.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_modbus::{SimDeviceProfile, Simulation, SlaveHandles, SlaveState, tick_values};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Area, Direction, LogLevel, ModbusSlaveSpec, PointInfo, Quality, Reading, SessionState, Stats,
    TrafficFrame, ValueFrame,
};

use super::function_name;
use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::templates;

/// How often template values drift, as in the Modbus Poll app.
const SIMULATION_TICK: Duration = Duration::from_secs(1);
/// How often the app sees the register image.
const PUBLISH_TICK: Duration = Duration::from_millis(250);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);

pub(crate) async fn run(
    spec: ModbusSlaveSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let profile = match spec.template.as_deref() {
        Some(id) => match templates::find_modbus(id) {
            Some(profile) => Some(profile),
            None => {
                out.state(SessionState::Closed {
                    error: Some(format!("Unknown device template {id}")),
                });
                return;
            }
        },
        None => None,
    };
    let Some(ip) = listen_ip(&spec.bind) else {
        let message = format!("Cannot listen on {}: not an IP address", spec.bind);
        out.log(LogLevel::Error, message.clone());
        out.state(SessionState::Closed {
            error: Some(message),
        });
        return;
    };
    let address = SocketAddr::new(ip, spec.port);
    let registers = register_count(&spec, profile);
    let (state, mut requests, shutdown) =
        match listen(ip, &spec, registers, &mut control, out).await {
            Ok(Some(handles)) => handles,
            Ok(None) => {
                out.state(SessionState::Closed { error: None });
                return;
            }
            Err(error) => {
                let message = format!("Cannot listen on {address}: {error}");
                out.log(LogLevel::Error, message.clone());
                out.state(SessionState::Closed {
                    error: Some(message),
                });
                return;
            }
        };
    out.state(SessionState::Listening {
        address: address.to_string(),
    });
    out.log(
        LogLevel::Info,
        match profile {
            Some(profile) => format!(
                "Simulating {} {} on {address}, unit {}",
                profile.manufacturer, profile.model, spec.unit_id
            ),
            None => format!(
                "Serving {registers} holding registers on {address}, unit {}",
                spec.unit_id
            ),
        },
    );

    let mut image = match profile {
        Some(profile) => Image::Template(Template::new(profile.clone())),
        None => Image::Plain { count: registers },
    };
    out.points(image.points());
    image.sync(&state, false, out);

    let mut simulate = tokio::time::interval(SIMULATION_TICK);
    simulate.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut publish = tokio::time::interval(PUBLISH_TICK);
    publish.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stats = Stats::default();
    let mut traffic = Vec::new();

    loop {
        tokio::select! {
            _ = simulate.tick() => image.sync(&state, true, out),
            _ = publish.tick() => {
                // Pick up a master's writes now, so the app sees them held
                // without waiting for the next simulation tick.
                image.sync(&state, false, out);
                stats.polls += 1;
                out.values(image.frame(&state));
                out.traffic(std::mem::take(&mut traffic));
                out.stats(stats);
            }
            request = requests.recv() => match request {
                Some(entry) => {
                    stats.frames += 1;
                    traffic.push(TrafficFrame {
                        at_ms: entry.timestamp,
                        direction: Direction::Rx,
                        summary: if entry.function_code == 0 {
                            entry.message.clone()
                        } else {
                            format!(
                                "FC{:02} {} @{} ×{}",
                                entry.function_code & 0x7F,
                                function_name(entry.function_code),
                                entry.address,
                                entry.quantity
                            )
                        },
                        detail: entry.message,
                        bytes: u32::from(entry.quantity),
                    });
                }
                None => {
                    out.state(SessionState::Closed { error: Some("The server stopped".into()) });
                    return;
                }
            },
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = image.set(point, value, &state, out);
                    let held = result.is_ok() && matches!(image, Image::Template(_));
                    out.written(point, result);
                    if held {
                        out.locked(point, true);
                    }
                    out.values(image.frame(&state));
                }
                Some(Control::Lock { point, locked }) => {
                    if image.lock(point, locked) {
                        out.locked(point, locked);
                    }
                }
            },
        }
    }
    let _ = shutdown.send(true);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// The listen address of `bind`, which may be a bracketed IPv6 address.
fn listen_ip(bind: &str) -> Option<IpAddr> {
    let bind = bind.trim();
    bind.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(bind)
        .parse()
        .ok()
}

/// Start the server on `ip`. A port still held by a session that is closing
/// is retried for [`BIND_RETRY`]. `Ok(None)` when the app closed the session
/// meanwhile.
async fn listen(
    ip: IpAddr,
    spec: &ModbusSlaveSpec,
    registers: u16,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<SlaveHandles>, String> {
    // The server parses `{host}:{port}`, which needs IPv6 in brackets.
    let host = match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        match mt_modbus::start_slave_server(spec.port, &host, registers, spec.unit_id).await {
            Ok(handles) => return Ok(Some(handles)),
            Err(error)
                if Instant::now() < deadline
                    && error
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.kind() == io::ErrorKind::AddrInUse) =>
            {
                tokio::select! {
                    () = tokio::time::sleep(BIND_RETRY_STEP) => {}
                    () = until_closed(control, out) => return Ok(None),
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Wait for the app to close the session before it listens, refusing writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("The simulator is not listening yet".into()));
            }
            Some(Control::Lock { .. }) => {}
        }
    }
}

/// Registers the image needs: the template's span after its base address, or
/// the requested size, at most [`ModbusSlaveSpec::MAX_PLAIN_REGISTERS`].
fn register_count(spec: &ModbusSlaveSpec, profile: Option<&SimDeviceProfile>) -> u16 {
    match profile {
        Some(profile) => {
            let end = u32::from(profile.base_address) + templates::register_span(profile);
            u16::try_from(end).unwrap_or(u16::MAX).max(spec.registers)
        }
        None => spec
            .registers
            .clamp(1, ModbusSlaveSpec::MAX_PLAIN_REGISTERS),
    }
}

/// Registers one template value spans: measurements are float32, counters
/// float64.
fn value_width(counter: bool) -> usize {
    if counter { 4 } else { 2 }
}

/// A template value from its big-endian words, as the simulator writes them.
fn decode(words: &[u16]) -> f64 {
    let bits = words
        .iter()
        .fold(0u64, |bits, word| (bits << 16) | u64::from(*word));
    if words.len() == 4 {
        f64::from_bits(bits)
    } else {
        f64::from(f32::from_bits(bits as u32))
    }
}

/// The big-endian words of a template value.
fn encode(value: f64, counter: bool, words: &mut [u16]) {
    let bits = if counter {
        value.to_bits()
    } else {
        u64::from((value as f32).to_bits())
    };
    let count = words.len();
    for (index, word) in words.iter_mut().enumerate() {
        *word = (bits >> (16 * (count - 1 - index))) as u16;
    }
}

/// A template simulation and the registers it last wrote.
struct Template {
    simulation: Simulation,
    /// The template's registers as the simulation last wrote them; a register
    /// that differs from this was written by a master.
    written: Vec<u16>,
}

impl Template {
    fn new(profile: SimDeviceProfile) -> Self {
        let span = usize::try_from(templates::register_span(&profile)).unwrap_or(0);
        Self {
            simulation: Simulation::new(profile),
            // A fresh image is all zeros.
            written: vec![0; span],
        }
    }

    /// Bring the simulation and the register image together: take values a
    /// master wrote into the simulation and hold them, optionally run one
    /// simulation step, then write the values back. Returns the points a
    /// master wrote.
    fn sync(&mut self, registers: &mut [u16], tick: bool) -> Vec<usize> {
        let base = usize::from(self.simulation.profile.base_address);
        let mut taken = Vec::new();
        let mut offset = 0;
        for (index, register) in self.simulation.profile.registers.iter().enumerate() {
            let width = value_width(register.is_counter);
            let range = offset..offset + width;
            offset += width;
            let (Some(image), Some(written)) = (
                registers.get(base + range.start..base + range.end),
                self.written.get(range),
            ) else {
                continue;
            };
            if image != written {
                self.simulation.values[index] = decode(image);
                self.simulation.locks.set(index, true);
                taken.push(index);
            }
        }
        if tick {
            tick_values(&mut self.simulation);
        }
        let mut offset = 0;
        for (register, value) in self
            .simulation
            .profile
            .registers
            .iter()
            .zip(&self.simulation.values)
        {
            let width = value_width(register.is_counter);
            let range = offset..offset + width;
            offset += width;
            if let (Some(image), Some(written)) = (
                registers.get_mut(base + range.start..base + range.end),
                self.written.get_mut(range),
            ) {
                encode(*value, register.is_counter, image);
                written.copy_from_slice(image);
            }
        }
        taken
    }
}

enum Image {
    Template(Template),
    Plain { count: u16 },
}

impl Image {
    fn points(&self) -> Vec<PointInfo> {
        match self {
            Self::Template(template) => {
                let profile = &template.simulation.profile;
                let mut address = profile.base_address;
                profile
                    .registers
                    .iter()
                    .enumerate()
                    .map(|(index, register)| {
                        let point = PointInfo {
                            id: u32::try_from(index).unwrap_or(u32::MAX),
                            name: register.name.replace('_', " "),
                            address: Area::Holding.modicon_label(address),
                            data_type: if register.is_counter {
                                "Float64"
                            } else {
                                "Float32"
                            }
                            .into(),
                            unit: register.unit.clone(),
                            writable: true,
                            boolean: false,
                        };
                        address = address.saturating_add(if register.is_counter { 4 } else { 2 });
                        point
                    })
                    .collect()
            }
            Self::Plain { count } => (0..*count)
                .map(|address| PointInfo {
                    id: u32::from(address),
                    name: format!("Register {address}"),
                    address: Area::Holding.modicon_label(address),
                    data_type: "UInt16".into(),
                    unit: String::new(),
                    writable: true,
                    boolean: false,
                })
                .collect(),
        }
    }

    /// Template images: take a master's writes into the simulation (held and
    /// reported), optionally step it, and write it back to the registers.
    /// Plain images are the registers themselves.
    fn sync(&mut self, state: &Arc<Mutex<SlaveState>>, tick: bool, out: &Emitter) {
        let Self::Template(template) = self else {
            return;
        };
        let taken = match state.lock() {
            Ok(mut state) => template.sync(&mut state.registers, tick),
            Err(_) => return,
        };
        let simulation = &template.simulation;
        for index in taken {
            let (Some(register), Some(value)) = (
                simulation.profile.registers.get(index),
                simulation.values.get(index),
            ) else {
                continue;
            };
            let value = if register.is_counter {
                value.to_string()
            } else {
                (*value as f32).to_string()
            };
            out.log(
                LogLevel::Info,
                format!(
                    "A master set {} to {value} {}; holding it",
                    register.name.replace('_', " "),
                    register.unit
                ),
            );
            out.locked(u32::try_from(index).unwrap_or(u32::MAX), true);
        }
    }

    fn frame(&self, state: &Arc<Mutex<SlaveState>>) -> ValueFrame {
        let Ok(state) = state.lock() else {
            return ValueFrame {
                at_ms: now_ms(),
                latency_us: 0,
                readings: Vec::new(),
            };
        };
        let readings = match self {
            Self::Template(template) => {
                let profile = &template.simulation.profile;
                let mut offset = usize::from(profile.base_address);
                profile
                    .registers
                    .iter()
                    .map(|register| {
                        let width = value_width(register.is_counter);
                        let reading = reading_from(&state.registers, offset, width);
                        offset += width;
                        reading
                    })
                    .collect()
            }
            Self::Plain { count } => state
                .registers
                .iter()
                .take(usize::from(*count))
                .map(|word| Reading {
                    value: f64::from(*word),
                    raw: [*word, 0, 0, 0],
                    raw_len: 1,
                    quality: Quality::Good,
                })
                .collect(),
        };
        ValueFrame {
            at_ms: now_ms(),
            latency_us: 0,
            readings,
        }
    }

    fn set(
        &mut self,
        point: u32,
        value: f64,
        state: &Arc<Mutex<SlaveState>>,
        out: &Emitter,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("{value} is not a number"));
        }
        let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
        match self {
            Self::Template(template) => {
                let register = template
                    .simulation
                    .profile
                    .registers
                    .get(index)
                    .ok_or_else(|| format!("No point {point}"))?;
                if !(register.min..=register.max).contains(&value) {
                    return Err(format!(
                        "{} takes {}..{} {}",
                        register.name, register.min, register.max, register.unit
                    ));
                }
                // Take a master's pending writes first, so they don't
                // overwrite this value on the next sync.
                self.sync(state, false, out);
                if let Self::Template(template) = self {
                    template.simulation.values[index] = value;
                    // Hold the value: otherwise the next tick drifts it away.
                    template.simulation.locks.set(index, true);
                }
                self.sync(state, false, out);
                Ok(())
            }
            Self::Plain { count } => {
                if index >= usize::from(*count) {
                    return Err(format!("No point {point}"));
                }
                if value.fract() != 0.0 || !(0.0..=65_535.0).contains(&value) {
                    return Err(format!("{value} is not a 16-bit unsigned value"));
                }
                let mut state = state
                    .lock()
                    .map_err(|_| "The register image is unavailable")?;
                if let Some(word) = state.registers.get_mut(index) {
                    *word = value as u16;
                }
                Ok(())
            }
        }
    }

    /// Hold or release a template value. Returns whether `point` exists.
    fn lock(&mut self, point: u32, locked: bool) -> bool {
        let Self::Template(template) = self else {
            return false;
        };
        match usize::try_from(point) {
            Ok(index) if index < template.simulation.profile.registers.len() => {
                template.simulation.locks.set(index, locked);
                true
            }
            _ => false,
        }
    }
}

/// A template value read from the register image.
fn reading_from(registers: &[u16], offset: usize, width: usize) -> Reading {
    let mut raw = [0u16; 4];
    let words = registers.get(offset..offset + width).unwrap_or_default();
    for (slot, word) in raw.iter_mut().zip(words) {
        *slot = *word;
    }
    let complete = words.len() == width;
    Reading {
        value: if complete { decode(words) } else { f64::NAN },
        raw,
        raw_len: u8::try_from(words.len()).unwrap_or(4),
        quality: if complete {
            Quality::Good
        } else {
            Quality::Bad
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_through_big_endian_words() {
        let mut words = [0u16; 2];
        encode(230.5, false, &mut words);
        assert_eq!(words, [0x4366, 0x8000]);
        assert_eq!(decode(&words), 230.5);
        let mut words = [0u16; 4];
        encode(12_345.678_9, true, &mut words);
        assert_eq!(decode(&words), 12_345.678_9);
    }

    #[test]
    fn ipv6_listen_addresses_parse_bracketed_or_not() {
        assert_eq!(listen_ip("::"), "::".parse().ok());
        assert_eq!(listen_ip("[::1]"), "::1".parse().ok());
        assert_eq!(listen_ip(" 127.0.0.1 "), "127.0.0.1".parse().ok());
        assert_eq!(listen_ip("localhost"), None);
    }

    #[test]
    fn a_master_write_is_taken_and_held() {
        let profile = templates::find_modbus("pm5560").expect("PM5560 template");
        let mut template = Template::new(profile.clone());
        let base = usize::from(profile.base_address);
        let mut registers = vec![0u16; base + templates::register_span(profile) as usize];
        assert!(template.sync(&mut registers, false).is_empty());

        let target = profile
            .registers
            .iter()
            .position(|register| !register.is_counter)
            .expect("a measurement register");
        let offset = base
            + profile.registers[..target]
                .iter()
                .map(|register| value_width(register.is_counter))
                .sum::<usize>();
        encode(42.25, false, &mut registers[offset..offset + 2]);
        assert_eq!(template.sync(&mut registers, true), [target]);
        for _ in 0..5 {
            assert!(template.sync(&mut registers, true).is_empty());
        }
        assert_eq!(decode(&registers[offset..offset + 2]), 42.25);
        assert_eq!(template.simulation.values[target], 42.25);
        assert!(template.simulation.locks.is_locked(target));
    }
}
