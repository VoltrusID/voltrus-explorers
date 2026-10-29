//! Plays a FANUC controller: `mt-focas`'s simulator serving a three-axis
//! mill that runs a demo program, for any FOCAS/Ethernet client.
//!
//! Points are the controller as an operator sees it. Writing a control
//! point does what the operator would: cycle start or hold, emergency stop,
//! an alarm, the part counter.

use std::io::ErrorKind;
use std::time::Duration;

use mt_focas::{AlarmType, CncModel, EmergencyState, FocasSimulator, RunState, SimulatorConfig};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, FocasSimulatorSpec, LogLevel, PointInfo, Reading, SessionState, Stats, ValueFrame,
};

use super::{good, profile};
use crate::Control;
use crate::cnc::{Texts, fail, frame, until_closed};
use crate::emit::{Emitter, now_ms};

/// How often the simulator reports its values to the app.
pub(crate) const TICK: Duration = Duration::from_millis(500);
/// How often the CNC's own model advances.
const MODEL_TICK: Duration = Duration::from_millis(100);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
/// The alarm the alarm point raises: P/S alarm 100.
const ALARM_NUMBER: i32 = 100;

/// The points before the axes' positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Cycle,
    Emergency,
    Alarm,
    Parts,
    Feed,
    Spindle,
    Program,
    Sequence,
    Run,
}

const POINTS: [Point; 9] = [
    Point::Cycle,
    Point::Emergency,
    Point::Alarm,
    Point::Parts,
    Point::Feed,
    Point::Spindle,
    Point::Program,
    Point::Sequence,
    Point::Run,
];

fn points(model: &CncModel) -> Vec<PointInfo> {
    let mut points = Vec::new();
    let mut add =
        |name: String, address: String, data_type: &str, unit: &str, writable, boolean| {
            points.push(PointInfo {
                id: u32::try_from(points.len()).unwrap_or(u32::MAX),
                name,
                address,
                data_type: data_type.into(),
                unit: unit.into(),
                writable,
                boolean,
                states: Vec::new(),
            });
        };
    for point in POINTS {
        match point {
            Point::Cycle => add("Cycle start".into(), "CYCLE".into(), "Bool", "", true, true),
            Point::Emergency => add(
                "Emergency stop".into(),
                "EMG".into(),
                "Bool",
                "",
                true,
                true,
            ),
            Point::Alarm => add(
                "Alarm P/S 100".into(),
                "ALM100".into(),
                "Bool",
                "",
                true,
                true,
            ),
            Point::Parts => add(
                "Part count".into(),
                format!("#{}", model.part_counter_macro),
                "Number",
                "",
                true,
                false,
            ),
            Point::Feed => add(
                "Feed rate".into(),
                "ACTF".into(),
                "Number",
                "mm/min",
                false,
                false,
            ),
            Point::Spindle => add(
                "Spindle speed".into(),
                "ACTS".into(),
                "Number",
                "rpm",
                false,
                false,
            ),
            Point::Program => add(
                "Program".into(),
                "PRGNUM".into(),
                "O number",
                "",
                false,
                false,
            ),
            Point::Sequence => add(
                "Sequence".into(),
                "SEQNUM".into(),
                "N number",
                "",
                false,
                false,
            ),
            Point::Run => add(
                "Run state".into(),
                "STATINFO.run".into(),
                "Text",
                "",
                false,
                false,
            ),
        }
    }
    for axis in &model.axes {
        add(
            format!("Absolute {}", axis.name),
            format!("POS1.{}", axis.name),
            "Number",
            "mm",
            false,
            false,
        );
    }
    points
}

fn readings(model: &CncModel) -> (Vec<Reading>, Vec<String>) {
    let mut readings = Vec::new();
    let mut texts = Vec::new();
    for point in POINTS {
        let (reading, text) = match point {
            Point::Cycle => (
                good(f64::from(u8::from(model.run == RunState::Start))),
                String::new(),
            ),
            Point::Emergency => (
                good(f64::from(u8::from(
                    model.emergency == EmergencyState::Emergency,
                ))),
                String::new(),
            ),
            Point::Alarm => (
                good(f64::from(u8::from(
                    model
                        .alarms
                        .iter()
                        .any(|alarm| alarm.number == ALARM_NUMBER),
                ))),
                String::new(),
            ),
            Point::Parts => (good(model.part_count() as f64), String::new()),
            Point::Feed => (good(model.feed_rate), String::new()),
            Point::Spindle => (good(model.spindle_speed), String::new()),
            Point::Program => (
                good(f64::from(model.program_numbers().running)),
                String::new(),
            ),
            Point::Sequence => (good(f64::from(model.sequence_number())), String::new()),
            Point::Run => (good(f64::NAN), model.run.label().to_owned()),
        };
        readings.push(reading);
        texts.push(text);
    }
    for axis in &model.axes {
        readings.push(good(axis.absolute));
        texts.push(String::new());
    }
    (readings, texts)
}

pub(crate) async fn run(
    spec: FocasSimulatorSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let config = SimulatorConfig {
        bind_addr: spec.bind.clone(),
        port: spec.port,
        profile: profile(spec.profile),
        tick: Some(MODEL_TICK),
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
    if spec.run {
        simulator.with_model(|model| {
            model.auto_repeat = true;
            model.cycle_start();
        });
    }
    out.state(SessionState::Listening {
        address: simulator.local_addr().to_string(),
    });
    out.log(
        LogLevel::Info,
        format!(
            "Simulating a FANUC {} on TCP port {}; connect any FOCAS/Ethernet client",
            spec.profile.name(),
            simulator.port()
        ),
    );
    let first = simulator.model();
    out.points(points(&first));

    let mut texts = Texts::new(first.axes.len() + POINTS.len());
    let mut stats = Stats::default();
    let mut served = 0u64;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                report(&simulator, &mut texts, out);
                let requests = simulator.requests_served();
                if requests > served {
                    out.traffic(vec![frame(
                        Direction::Rx,
                        format!("{} request(s) answered", requests - served),
                        format!("{} client(s) connected", simulator.active_sessions()),
                    )]);
                    served = requests;
                }
                stats.polls += 1;
                stats.frames = requests;
                out.stats(stats);
            }
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = set(&simulator, point, value);
                    if let Err(error) = &result {
                        out.log(LogLevel::Warn, error.clone());
                    }
                    if result.is_ok() {
                        report(&simulator, &mut texts, out);
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

fn report(simulator: &FocasSimulator, texts: &mut Texts, out: &Emitter) {
    let model = simulator.model();
    let (readings, now) = readings(&model);
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

/// Act as the operator would for a write to a control point.
fn set(simulator: &FocasSimulator, point: u32, number: f64) -> Result<(), String> {
    if !number.is_finite() {
        return Err("Enter a number".into());
    }
    let on = number != 0.0;
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    match POINTS.get(index) {
        Some(Point::Cycle) => simulator.with_model(|model| {
            if on {
                if model.cycle_start() {
                    Ok(())
                } else {
                    Err("The CNC will not start: it is in emergency stop, has an alarm, or has no program".to_owned())
                }
            } else {
                model.feed_hold();
                Ok(())
            }
        }),
        Some(Point::Emergency) => {
            simulator.with_model(|model| model.set_emergency(on));
            Ok(())
        }
        Some(Point::Alarm) => {
            simulator.with_model(|model| {
                if on {
                    model.raise_alarm(ALARM_NUMBER, AlarmType::ParameterSwitchOn, 0, "PARAMETER WRITE ENABLE");
                } else {
                    model.clear_alarm(ALARM_NUMBER);
                }
            });
            Ok(())
        }
        Some(Point::Parts) => {
            if number < 0.0 || number.fract() != 0.0 {
                return Err("A part count is a whole number from 0".into());
            }
            simulator.with_model(|model| {
                let counter = model.part_counter_macro;
                model.set_macro(counter, number);
            });
            Ok(())
        }
        Some(_) => Err("Only the controls and the part count can be set".into()),
        None => Err("The axes follow the program".into()),
    }
}

/// Start the simulator, retrying for [`BIND_RETRY`] while its port is in
/// use, as a closing session may still hold it. `Ok(None)` when the app
/// closed the session meanwhile.
async fn start(
    config: &SimulatorConfig,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<FocasSimulator>, std::io::Error> {
    let deadline = Instant::now() + BIND_RETRY;
    loop {
        match FocasSimulator::start(config.clone()).await {
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

fn bind_error(spec: &FocasSimulatorSpec, error: &std::io::Error) -> String {
    match error.kind() {
        ErrorKind::AddrInUse => format!(
            "TCP port {} is in use by another program. Stop it, or give the simulator another port",
            spec.port
        ),
        ErrorKind::PermissionDenied => format!(
            "This Mac does not let the app listen on TCP port {}. Use a port above 1023, e.g. 8193",
            spec.port
        ),
        ErrorKind::AddrNotAvailable => format!("{} is not an address of this computer", spec.bind),
        _ => format!("Cannot listen on {}:{}: {error}", spec.bind, spec.port),
    }
}

#[cfg(test)]
mod tests {
    use mt_focas::CncProfile;

    use super::*;

    #[test]
    fn points_follow_the_model_and_the_readings_follow_the_points() {
        let model = CncModel::new(CncProfile::Series31iB);
        let infos = points(&model);
        let (values, texts) = readings(&model);
        assert_eq!(infos.len(), POINTS.len() + model.axes.len());
        assert_eq!(values.len(), infos.len());
        assert_eq!(texts.len(), infos.len());
        assert!(infos[0].writable && infos[0].boolean);
        assert!(!infos[POINTS.len()].writable);
        assert_eq!(texts[POINTS.len() - 1], "****");
    }

    #[test]
    fn bind_errors_say_what_to_do() {
        let spec = FocasSimulatorSpec::default();
        let busy = std::io::Error::from(ErrorKind::AddrInUse);
        assert!(bind_error(&spec, &busy).contains("8193 is in use"));
        let denied = std::io::Error::from(ErrorKind::PermissionDenied);
        assert!(bind_error(&spec, &denied).contains("above 1023"));
    }
}
