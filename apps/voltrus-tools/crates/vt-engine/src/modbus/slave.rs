//! Serves a Modbus register image, optionally driven by a device template.
//!
//! The session owns the [`Simulation`] instead of handing it to
//! `start_simulation`, so a value the user sets is written into the simulation
//! (and locked) rather than overwritten on the next tick.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_modbus::{SimDeviceProfile, Simulation, SlaveState, tick_values};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
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
    let registers = register_count(&spec, profile);
    let (state, mut requests, shutdown) =
        match mt_modbus::start_slave_server(spec.port, &spec.bind, registers, spec.unit_id).await {
            Ok(handles) => handles,
            Err(error) => {
                let message = format!("Cannot listen on {}:{}: {error}", spec.bind, spec.port);
                out.log(LogLevel::Error, message.clone());
                out.state(SessionState::Closed {
                    error: Some(message),
                });
                return;
            }
        };
    let address = format!("{}:{}", spec.bind, spec.port);
    out.state(SessionState::Listening {
        address: address.clone(),
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
        Some(profile) => Image::Template(Simulation::new(profile.clone())),
        None => Image::Plain { count: registers },
    };
    out.points(image.points());
    image.store(&state);

    let mut simulate = tokio::time::interval(SIMULATION_TICK);
    simulate.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut publish = tokio::time::interval(PUBLISH_TICK);
    publish.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stats = Stats::default();
    let mut traffic = Vec::new();

    loop {
        tokio::select! {
            _ = simulate.tick() => {
                if let Image::Template(simulation) = &mut image {
                    tick_values(simulation);
                    simulation.write_to_slave(&state);
                }
            }
            _ = publish.tick() => {
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
                    let result = image.set(point, value, &state);
                    out.written(point, result);
                    out.values(image.frame(&state));
                }
                Some(Control::Lock { point, locked }) => image.lock(point, locked),
            },
        }
    }
    let _ = shutdown.send(true);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Registers the image needs: the template's span after its base address, or
/// the requested size.
fn register_count(spec: &ModbusSlaveSpec, profile: Option<&SimDeviceProfile>) -> u16 {
    match profile {
        Some(profile) => {
            let end = u32::from(profile.base_address) + templates::register_span(profile);
            u16::try_from(end).unwrap_or(u16::MAX).max(spec.registers)
        }
        None => spec.registers.max(1),
    }
}

enum Image {
    Template(Simulation),
    Plain { count: u16 },
}

impl Image {
    fn points(&self) -> Vec<PointInfo> {
        match self {
            Self::Template(simulation) => {
                let mut address = simulation.profile.base_address;
                simulation
                    .profile
                    .registers
                    .iter()
                    .enumerate()
                    .map(|(index, register)| {
                        let point = PointInfo {
                            id: u32::try_from(index).unwrap_or(u32::MAX),
                            name: register.name.replace('_', " "),
                            address: Area::Holding.modicon(address).to_string(),
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
                    address: Area::Holding.modicon(address).to_string(),
                    data_type: "UInt16".into(),
                    unit: String::new(),
                    writable: true,
                    boolean: false,
                })
                .collect(),
        }
    }

    fn store(&self, state: &Arc<Mutex<SlaveState>>) {
        if let Self::Template(simulation) = self {
            simulation.write_to_slave(state);
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
            Self::Template(simulation) => {
                let mut offset = usize::from(simulation.profile.base_address);
                simulation
                    .profile
                    .registers
                    .iter()
                    .zip(&simulation.values)
                    .map(|(register, value)| {
                        let width = if register.is_counter { 4 } else { 2 };
                        let reading = reading_from(
                            &state.registers,
                            offset,
                            width,
                            if register.is_counter {
                                *value
                            } else {
                                f64::from(*value as f32)
                            },
                        );
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
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("{value} is not a number"));
        }
        let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
        match self {
            Self::Template(simulation) => {
                let register = simulation
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
                simulation.values[index] = value;
                // Hold the value: otherwise the next tick drifts it away.
                simulation.locks.set(index, true);
                simulation.write_to_slave(state);
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

    fn lock(&mut self, point: u32, locked: bool) {
        if let (Self::Template(simulation), Ok(index)) = (self, usize::try_from(point)) {
            simulation.locks.set(index, locked);
        }
    }
}

fn reading_from(registers: &[u16], offset: usize, width: usize, value: f64) -> Reading {
    let mut raw = [0u16; 4];
    let words = registers.get(offset..offset + width).unwrap_or_default();
    for (slot, word) in raw.iter_mut().zip(words) {
        *slot = *word;
    }
    Reading {
        value,
        raw,
        raw_len: u8::try_from(words.len()).unwrap_or(4),
        quality: if words.len() == width {
            Quality::Good
        } else {
            Quality::Bad
        },
    }
}
