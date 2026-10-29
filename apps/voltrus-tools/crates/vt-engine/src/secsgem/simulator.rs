//! Plays equipment: `mt-secsgem`'s simulator, a wafer processing chamber
//! behind a GEM interface, for any GEM host.
//!
//! Points are the chamber as an operator sees it. Writing a control point
//! does what the operator would (start or stop a process, take the equipment
//! on or offline, raise or clear an alarm); equipment constants are set the
//! way an operator sets them at the tool.

use std::io::ErrorKind;
use std::time::Duration;

use mt_secsgem::{
    Direction as GemDirection, EquipmentModel, EquipmentSimulator, HsmsConfig, ProcessState,
    SimMode, SimulatorConfig,
};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    LogLevel, PointInfo, Reading, SecsGemSimulatorSpec, SessionState, Stats, ValueFrame,
};

use super::{clip, eac_error, frame, point, type_of, value_of};
use crate::Control;
use crate::cnc::{Texts, fail, until_closed};
use crate::emit::{Emitter, now_ms};

/// How often the simulator reports its values to the app.
pub(crate) const TICK: Duration = Duration::from_millis(500);
/// How often the chamber's own model advances.
const MODEL_TICK_MS: u64 = 100;
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);

/// The control points before the alarms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Switch {
    Running,
    Paused,
    Online,
    Remote,
}

const CONTROLS: [Switch; 4] = [
    Switch::Running,
    Switch::Paused,
    Switch::Online,
    Switch::Remote,
];

/// What each point is, in order: controls, alarms, status variables,
/// equipment constants.
struct Layout {
    alarms: Vec<u64>,
    svids: Vec<u64>,
    ecids: Vec<u64>,
}

impl Layout {
    fn of(model: &EquipmentModel) -> Self {
        Self {
            alarms: model.alarms.iter().map(|alarm| alarm.alid).collect(),
            svids: model
                .status_variable_infos()
                .iter()
                .map(|info| info.id)
                .collect(),
            ecids: model.constants.iter().map(|ec| ec.id).collect(),
        }
    }

    fn first_alarm(&self) -> usize {
        CONTROLS.len()
    }

    fn first_sv(&self) -> usize {
        self.first_alarm() + self.alarms.len()
    }

    fn first_ec(&self) -> usize {
        self.first_sv() + self.svids.len()
    }

    fn len(&self) -> usize {
        self.first_ec() + self.ecids.len()
    }
}

fn points(model: &EquipmentModel, layout: &Layout) -> Vec<PointInfo> {
    let mut points: Vec<PointInfo> = Vec::with_capacity(layout.len());
    let mut add =
        |name: String, address: String, data_type: &str, unit: &str, writable, boolean| {
            let id = points.len();
            points.push(point(id, name, address, data_type, unit, writable, boolean));
        };
    for control in CONTROLS {
        let (name, address) = match control {
            Switch::Running => ("Process running", "PROCESS"),
            Switch::Paused => ("Process paused", "PAUSE"),
            Switch::Online => ("Equipment online", "ONLINE"),
            Switch::Remote => ("Remote control", "REMOTE"),
        };
        add(name.into(), address.into(), "Bool", "", true, true);
    }
    for alarm in &model.alarms {
        add(
            format!("Alarm: {}", alarm.text),
            format!("ALID {}", alarm.alid),
            "Bool",
            "",
            true,
            true,
        );
    }
    for info in model.status_variable_infos() {
        let item = model.sv_value(info.id);
        add(
            info.name,
            format!("SVID {}", info.id),
            item.as_ref().map_or("", type_of),
            info.units.trim(),
            false,
            false,
        );
    }
    for constant in &model.constants {
        add(
            constant.name.clone(),
            format!("ECID {}", constant.id),
            type_of(&constant.value),
            constant.units.trim(),
            true,
            false,
        );
    }
    points
}

fn good_flag(on: bool) -> Reading {
    value_of(&mt_secsgem::Item::Boolean(vec![on])).reading
}

fn readings(model: &EquipmentModel, layout: &Layout) -> (Vec<Reading>, Vec<String>) {
    let mut readings = Vec::with_capacity(layout.len());
    let mut texts = Vec::with_capacity(layout.len());
    let mut add = |reading: Reading, text: String| {
        readings.push(reading);
        texts.push(text);
    };
    for control in CONTROLS {
        let on = match control {
            Switch::Running => model.chamber.state == ProcessState::Executing,
            Switch::Paused => model.chamber.state == ProcessState::Paused,
            Switch::Online => model.control_state.is_online(),
            Switch::Remote => model.control_state == mt_secsgem::ControlState::OnlineRemote,
        };
        add(good_flag(on), String::new());
    }
    for alid in &layout.alarms {
        let set = model
            .alarms
            .iter()
            .any(|alarm| alarm.alid == *alid && alarm.set);
        add(good_flag(set), String::new());
    }
    for id in &layout.svids {
        match model.sv_value(*id) {
            Some(item) => {
                let value = value_of(&item);
                add(value.reading, value.text);
            }
            None => add(Reading::bad(), String::new()),
        }
    }
    for id in &layout.ecids {
        match model.constant(*id) {
            Some(constant) => {
                let value = value_of(&constant.value);
                add(value.reading, value.text);
            }
            None => add(Reading::bad(), String::new()),
        }
    }
    (readings, texts)
}

pub(crate) async fn run(
    spec: SecsGemSimulatorSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let config = SimulatorConfig {
        mode: SimMode::Passive {
            bind_addr: spec.bind.clone(),
            port: spec.port,
        },
        hsms: HsmsConfig {
            device_id: spec.device_id,
            ..HsmsConfig::default()
        },
        initiate_communication: true,
        tick_ms: Some(MODEL_TICK_MS),
    };
    let simulator = match start(&config, &mut control, out).await {
        Ok(Some(simulator)) => simulator,
        Ok(None) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
        Err(error) => {
            fail(out, bind_error(&spec, &error));
            return;
        }
    };
    let mut log = simulator.subscribe_log();
    out.state(SessionState::Listening {
        address: simulator
            .local_addr()
            .map_or_else(String::new, |a| a.to_string()),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Simulating a wafer processing chamber on HSMS port {}, device id {}; connect any GEM host",
            simulator.port(),
            spec.device_id
        ),
    );
    let first = simulator.model();
    let layout = Layout::of(&first);
    out.points(points(&first, &layout));

    let mut texts = Texts::new(layout.len());
    let mut stats = Stats::default();
    let mut selected = false;
    let mut auto_run = spec.run;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                if auto_run {
                    simulator.with_model(|model| {
                        if model.chamber.state == ProcessState::Idle {
                            let _ = model.start_process(None);
                        }
                    });
                }
                report(&simulator, &layout, &mut texts, out);
                let now = simulator.is_selected();
                if now != selected {
                    selected = now;
                    out.log(
                        LogLevel::Info,
                        if now { "A host selected the session" } else { "The host left" },
                    );
                }
                stats.polls += 1;
                stats.frames = simulator.requests_served();
                out.stats(stats);
            }
            entry = log.recv() => match entry {
                Ok(entry) => {
                    if entry.direction == GemDirection::Tx
                        && matches!(entry.kind.as_str(), "S6F11" | "S5F1")
                    {
                        out.log(LogLevel::Info, format!("Sent {}", clip(&entry.text, 200)));
                    }
                    out.traffic(vec![frame(&entry)]);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = set(&simulator, &layout, &mut auto_run, point, value);
                    if let Err(error) = &result {
                        out.log(LogLevel::Warn, error.clone());
                    }
                    if result.is_ok() {
                        report(&simulator, &layout, &mut texts, out);
                    }
                    out.written(point, result);
                }
                Some(_) => {}
            },
        }
    }
    simulator.stop().await;
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn report(simulator: &EquipmentSimulator, layout: &Layout, texts: &mut Texts, out: &Emitter) {
    let model = simulator.model();
    let (readings, now) = readings(&model, layout);
    let changed = texts.changed(&now);
    if !changed.is_empty() {
        out.payloads(changed);
    }
    out.values(ValueFrame {
        at_ms: now_ms(),
        latency_us: 0,
        readings,
    });
}

/// Act as the operator would for a write to a point.
fn set(
    simulator: &EquipmentSimulator,
    layout: &Layout,
    auto_run: &mut bool,
    point: u32,
    number: f64,
) -> Result<(), String> {
    if !number.is_finite() {
        return Err("Enter a number".into());
    }
    let on = number != 0.0;
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    if let Some(control) = CONTROLS.get(index) {
        return match control {
            Switch::Running => {
                *auto_run = on;
                simulator.with_model(|model| {
                    let result = if on {
                        model.start_process(None)
                    } else {
                        model.stop_process()
                    };
                    result.map_err(|_| {
                        if on {
                            "The chamber is already processing".to_owned()
                        } else {
                            "The chamber is not processing".to_owned()
                        }
                    })
                })
            }
            Switch::Paused => simulator.with_model(|model| {
                let result = if on {
                    model.pause_process()
                } else {
                    model.resume_process()
                };
                result.map_err(|_| {
                    if on {
                        "Only a running process can be paused".to_owned()
                    } else {
                        "Only a paused process can be resumed".to_owned()
                    }
                })
            }),
            Switch::Online => simulator.with_model(|model| {
                if on {
                    if model.operator_online() {
                        Ok(())
                    } else {
                        Err(
                            "The equipment could not go online (the host must be communicating)"
                                .to_owned(),
                        )
                    }
                } else {
                    model.operator_offline();
                    Ok(())
                }
            }),
            Switch::Remote => {
                simulator.with_model(|model| model.operator_set_remote(on));
                Ok(())
            }
        };
    }
    if index < layout.first_sv() {
        let alid = layout.alarms[index - layout.first_alarm()];
        simulator.with_model(|model| {
            if on {
                model.raise_alarm(alid);
            } else {
                model.clear_alarm(alid);
            }
        });
        return Ok(());
    }
    if index < layout.first_ec() {
        return Err(
            "Status variables follow the chamber; use the controls and the constants".into(),
        );
    }
    let id = layout
        .ecids
        .get(index - layout.first_ec())
        .copied()
        .ok_or_else(|| format!("No point {point}"))?;
    let eac = simulator.with_model(|model| model.set_constant(id, number));
    if eac.is_ok() {
        Ok(())
    } else {
        let range = simulator
            .model()
            .constant(id)
            .map(|constant| (constant.min, constant.max));
        Err(eac_error(eac, id, range))
    }
}

/// Start the simulator, retrying for [`BIND_RETRY`] while its port is in
/// use, as a closing session may still hold it. `Ok(None)` when the app
/// closed the session meanwhile.
async fn start(
    config: &SimulatorConfig,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<EquipmentSimulator>, std::io::Error> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        match EquipmentSimulator::start(config.clone()).await {
            Ok(simulator) => return Ok(Some(simulator)),
            Err(error) if error.kind() == ErrorKind::AddrInUse && Instant::now() < deadline => {}
            Err(error) => return Err(error),
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            () = until_closed(control, out, "The simulator is not running yet") => return Ok(None),
        }
    }
}

fn bind_error(spec: &SecsGemSimulatorSpec, error: &std::io::Error) -> String {
    match error.kind() {
        ErrorKind::AddrInUse => format!(
            "TCP port {} is in use by another program. Stop it, or give the simulator another port",
            spec.port
        ),
        ErrorKind::PermissionDenied => format!(
            "This Mac does not let the app listen on TCP port {}. Use a port above 1023, e.g. 5600",
            spec.port
        ),
        ErrorKind::AddrNotAvailable => format!("{} is not an address of this computer", spec.bind),
        _ => format!("Cannot listen on {}:{}: {error}", spec.bind, spec.port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_secsgem::sim::model::{ecid, svid};

    #[test]
    fn points_follow_the_model_and_the_readings_follow_the_points() {
        let model = EquipmentModel::default();
        let layout = Layout::of(&model);
        let infos = points(&model, &layout);
        let (values, texts) = readings(&model, &layout);
        assert_eq!(infos.len(), layout.len());
        assert_eq!(values.len(), infos.len());
        assert_eq!(texts.len(), infos.len());
        assert!(infos[0].writable && infos[0].boolean);
        assert!(!infos[layout.first_sv()].writable);
        let temperature = infos
            .iter()
            .find(|info| info.address == format!("ECID {}", ecid::PROCESS_TEMPERATURE))
            .expect("the process temperature constant");
        assert!(temperature.writable);
        assert_eq!(temperature.unit, "degC");
        let clock = infos
            .iter()
            .position(|info| info.address == format!("SVID {}", svid::CLOCK))
            .expect("the clock");
        assert!(!texts[clock].is_empty(), "the clock is text");
    }

    #[test]
    fn bind_errors_say_what_to_do() {
        let spec = SecsGemSimulatorSpec::default();
        let busy = std::io::Error::from(ErrorKind::AddrInUse);
        assert!(bind_error(&spec, &busy).contains("5600 is in use"));
        let denied = std::io::Error::from(ErrorKind::PermissionDenied);
        assert!(bind_error(&spec, &denied).contains("above 1023"));
    }
}
