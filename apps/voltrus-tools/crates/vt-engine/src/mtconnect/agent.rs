//! Plays an MTConnect agent: `mt-mtconnect`'s simulator serving a machine
//! over HTTP, with an SHDR adapter port if asked.
//!
//! The machine's program moves the samples and events. A value the app sets
//! is held until released, and the app sees every held point as `Locked`.

use std::collections::HashMap;
use std::time::Duration;

use mt_mtconnect::sim::format_observation;
use mt_mtconnect::{
    AgentSimulator, ConditionLevel, DeviceTemplate, MtConnectError, SimConfig, SimItemState,
    SimModel, Value,
};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, LogLevel, MtconnectAgentSpec, MtconnectTemplate, Reading, SessionState, Stats,
    TrafficFrame, ValueFrame,
};

use super::{EMERGENCY_STOP, Kind, Layout, reading};
use crate::Control;
use crate::cnc::{Texts, fail, frame, until_closed};
use crate::emit::{Emitter, now_ms};

/// How often the simulator reports its values to the app.
pub(crate) const TICK: Duration = Duration::from_millis(500);
/// How often the observations the agent recorded are shown as traffic.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);
/// The machine's program advances this often.
const MACHINE_TICK_MS: u64 = 250;
/// Most observations shown as traffic per flush; a burst is summarised.
const TRAFFIC_BURST: usize = 200;
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);

pub(crate) fn template(template: MtconnectTemplate) -> DeviceTemplate {
    match template {
        MtconnectTemplate::Vmc => DeviceTemplate::Vmc3Axis,
        MtconnectTemplate::Lathe => DeviceTemplate::Lathe2Axis,
    }
}

pub(crate) async fn run(
    spec: MtconnectAgentSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let config = SimConfig {
        bind_addr: spec.bind.clone(),
        http_port: spec.port,
        shdr_port: spec.shdr.then_some(spec.shdr_port),
        template: template(spec.template),
        tick_ms: MACHINE_TICK_MS,
        ..SimConfig::default()
    };
    let simulator = match start(config, &mut control, out).await {
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
    let model = simulator.model().clone();
    out.state(SessionState::Listening {
        address: simulator.http_addr().to_string(),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Simulating a {} at {}/probe, /current and /sample",
            spec.template.name(),
            simulator.base_url()
        ),
    );
    if let Some(address) = simulator.shdr_addr() {
        out.log(
            LogLevel::Info,
            format!("SHDR adapter on {address}, streaming the same observations"),
        );
    }
    let layout = Layout::from_simulated(&model.device());
    out.points(layout.infos.clone());

    let mut texts = Texts::new(layout.infos.len());
    let mut held = vec![false; layout.infos.len()];
    let mut stats = Stats::default();
    let mut recorded = model.subscribe();
    let mut traffic: Vec<TrafficFrame> = Vec::new();
    let mut skipped = 0usize;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut flush = tokio::time::interval(TRAFFIC_TICK);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                report(&model, &layout, &mut held, &mut texts, out);
                stats.polls += 1;
                out.stats(stats);
            }
            observation = recorded.recv() => match observation {
                Ok(observation) => {
                    stats.frames += 1;
                    if traffic.len() < TRAFFIC_BURST {
                        let key = observation.data_item_id.as_str();
                        traffic.push(frame(
                            Direction::Tx,
                            format!("{key} {}", observation.value.to_text()),
                            format_observation(&observation, key),
                        ));
                    } else {
                        skipped += 1;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    skipped += usize::try_from(missed).unwrap_or(usize::MAX);
                }
                Err(broadcast::error::RecvError::Closed) => {}
            },
            _ = flush.tick() => {
                if skipped > 0 {
                    traffic.push(frame(
                        Direction::Tx,
                        format!("{skipped} more observations"),
                        "Too many in one moment to list",
                    ));
                    skipped = 0;
                }
                out.traffic(std::mem::take(&mut traffic));
            }
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = set(&model, &layout, point, value);
                    if result.is_ok() {
                        report(&model, &layout, &mut held, &mut texts, out);
                    }
                    out.written(point, result);
                }
                Some(Control::Lock { point, locked }) => {
                    hold(&model, &layout, &mut held, point, locked, out);
                }
                Some(_) => {}
            },
        }
    }
    simulator.stop().await;
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Start the simulator, retrying for [`BIND_RETRY`] while its port is in
/// use, as a closing session may still hold it. `Ok(None)` when the app
/// closed the session meanwhile.
async fn start(
    config: SimConfig,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<AgentSimulator>, MtConnectError> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        match AgentSimulator::start(config.clone()).await {
            Ok(simulator) => return Ok(Some(simulator)),
            Err(error) if in_use(&error) && Instant::now() < deadline => {}
            Err(error) => return Err(error),
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            () = until_closed(control, out, "The simulator is not running yet") => return Ok(None),
        }
    }
}

fn in_use(error: &MtConnectError) -> bool {
    error.to_string().contains("in use")
}

fn bind_error(spec: &MtconnectAgentSpec, error: &MtConnectError) -> String {
    let text = error.to_string();
    if in_use(error) {
        let port = if text.contains("SHDR") {
            spec.shdr_port
        } else {
            spec.port
        };
        format!(
            "TCP port {port} is in use by another program. Stop it, or give the simulator another port"
        )
    } else if text.contains("ermission denied") {
        "This Mac does not let the app listen on that port. Use a port above 1023, e.g. 5100".into()
    } else {
        text
    }
}

/// The values of the machine now, and the points held by hand.
fn report(model: &SimModel, layout: &Layout, held: &mut [bool], texts: &mut Texts, out: &Emitter) {
    let snapshot = model.snapshot();
    let items: HashMap<&str, &SimItemState> = snapshot
        .items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect();
    let mut readings = Vec::with_capacity(layout.ids.len());
    let mut now = Vec::with_capacity(layout.ids.len());
    for (index, (id, kind)) in layout.ids.iter().zip(&layout.kinds).enumerate() {
        let Some(item) = items.get(id.as_str()) else {
            readings.push(Reading::bad());
            now.push(String::new());
            continue;
        };
        let (read, text) = reading(kind, &item.value, item.condition.as_ref());
        readings.push(read);
        now.push(text);
        if let Some(slot) = held.get_mut(index)
            && *slot != item.pinned
        {
            *slot = item.pinned;
            out.locked(u32::try_from(index).unwrap_or(u32::MAX), item.pinned);
        }
    }
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

/// Set a point from the app: a sample is forced, the emergency stop pressed
/// or released, a condition raised (2 warning, 3 fault) or cleared (1).
fn set(model: &SimModel, layout: &Layout, point: u32, number: f64) -> Result<(), String> {
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    let (Some(info), Some(kind), Some(id)) = (
        layout.infos.get(index),
        layout.kinds.get(index),
        layout.ids.get(index),
    ) else {
        return Err(format!("No point {point}"));
    };
    if !info.writable {
        return Err(format!("{} follows the machine's program", info.name));
    }
    if !number.is_finite() {
        return Err("Enter a number".into());
    }
    let result = match kind {
        Kind {
            category: mt_mtconnect::Category::Condition,
            ..
        } => match number.round() as i64 {
            1 => model.clear_condition(id),
            2 => model.raise_condition(id, ConditionLevel::Warning, "W1001", "Simulated warning"),
            3 => model.raise_condition(id, ConditionLevel::Fault, "F1001", "Simulated fault"),
            _ => return Err("A condition is 1 (normal), 2 (warning) or 3 (fault)".into()),
        },
        Kind { data_type, .. } if data_type == EMERGENCY_STOP => model.set_value(
            id,
            Value::Text(if number != 0.0 { "TRIGGERED" } else { "ARMED" }.into()),
        ),
        _ => model.set_value(id, Value::Number(number)),
    };
    result.map_err(|error| error.to_string())
}

/// Hold a point where it is, or hand it back to the machine's program.
fn hold(
    model: &SimModel,
    layout: &Layout,
    held: &mut [bool],
    point: u32,
    locked: bool,
    out: &Emitter,
) {
    let Some(index) = usize::try_from(point).ok() else {
        return;
    };
    let (Some(info), Some(id), Some(slot)) = (
        layout.infos.get(index),
        layout.ids.get(index),
        held.get_mut(index),
    ) else {
        return;
    };
    if !info.writable || layout.kinds[index].category == mt_mtconnect::Category::Condition {
        return;
    }
    if locked {
        let current = model
            .snapshot()
            .items
            .into_iter()
            .find(|item| &item.id == id)
            .map(|item| item.value);
        if let Some(value) = current
            && model.set_value(id, value).is_err()
        {
            return;
        }
    } else {
        model.release(id);
    }
    *slot = locked;
    out.locked(point, locked);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> (SimModel, Layout) {
        let model = SimModel::new(DeviceTemplate::Vmc3Axis, 256);
        let layout = Layout::from_simulated(&model.device());
        (model, layout)
    }

    #[test]
    fn a_sample_is_forced_and_released() {
        let (model, layout) = machine();
        let x = u32::try_from(layout.index["Xpos"]).unwrap_or(0);
        assert_eq!(set(&model, &layout, x, 12.5), Ok(()));
        let forced = model
            .snapshot()
            .items
            .into_iter()
            .find(|item| item.id == "Xpos")
            .expect("the axis");
        assert!(forced.pinned);
        assert_eq!(forced.value, Value::Number(12.5));
    }

    #[test]
    fn conditions_take_their_level_and_the_program_owns_the_rest() {
        let (model, layout) = machine();
        let system = u32::try_from(layout.index["sys_cond"]).unwrap_or(0);
        assert_eq!(set(&model, &layout, system, 3.0), Ok(()));
        let level = |model: &SimModel| {
            model
                .snapshot()
                .items
                .into_iter()
                .find(|item| item.id == "sys_cond")
                .and_then(|item| item.condition)
                .map(|condition| condition.level)
        };
        assert_eq!(level(&model), Some(ConditionLevel::Fault));
        assert_eq!(set(&model, &layout, system, 1.0), Ok(()));
        assert_eq!(level(&model), Some(ConditionLevel::Normal));
        assert!(set(&model, &layout, system, 9.0).is_err());
        let exec = u32::try_from(layout.index["exec"]).unwrap_or(0);
        assert!(set(&model, &layout, exec, 1.0).is_err());
        assert!(set(&model, &layout, 999, 1.0).is_err());
    }

    #[test]
    fn the_emergency_stop_is_pressed_by_writing_one() {
        let (model, layout) = machine();
        let estop = u32::try_from(layout.index["estop"]).unwrap_or(0);
        assert_eq!(set(&model, &layout, estop, 1.0), Ok(()));
        let value = model
            .snapshot()
            .items
            .into_iter()
            .find(|item| item.id == "estop")
            .map(|item| item.value);
        assert_eq!(value, Some(Value::Text("TRIGGERED".into())));
    }

    #[test]
    fn bind_errors_say_what_to_do() {
        let spec = MtconnectAgentSpec::default();
        let busy = MtConnectError::Simulator(
            "cannot bind HTTP 127.0.0.1:5100: Address already in use".into(),
        );
        assert!(bind_error(&spec, &busy).contains("TCP port 5100 is in use"));
        let shdr = MtConnectError::Simulator(
            "cannot bind SHDR 127.0.0.1:7878: Address already in use".into(),
        );
        assert!(bind_error(&spec, &shdr).contains("TCP port 7878"));
    }
}
