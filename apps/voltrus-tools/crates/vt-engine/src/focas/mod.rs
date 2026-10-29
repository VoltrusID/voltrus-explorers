//! FANUC sessions over `mt-focas`: a client that polls a controller's
//! status, axes, spindle, program and alarms (and reads and writes macro
//! variables), and a simulator that plays a controller on its server.
//!
//! `mt-focas` has only been tested against its own simulator, not a real
//! controller; the app labels the protocol Beta.

pub(crate) mod client;
pub(crate) mod simulator;

use mt_focas::{AxisName, CncProfile, Reading as FocasReading, Snapshot, SysInfo};
use vt_protocol::{FocasProfile, PointInfo, Quality, Reading};

/// Per axis: where it is in four coordinate systems.
const POSITIONS: [&str; 4] = ["Absolute", "Machine", "Relative", "Distance to go"];

/// Points before the per-axis ones: [`Fixed`] in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fixed {
    Controller,
    Mode,
    Run,
    Motion,
    Cycle,
    Emergency,
    Alarm,
    Feed,
    Spindle,
    Program,
    Sequence,
    Block,
    Alarms,
}

impl Fixed {
    pub(crate) const ALL: [Fixed; 13] = [
        Fixed::Controller,
        Fixed::Mode,
        Fixed::Run,
        Fixed::Motion,
        Fixed::Cycle,
        Fixed::Emergency,
        Fixed::Alarm,
        Fixed::Feed,
        Fixed::Spindle,
        Fixed::Program,
        Fixed::Sequence,
        Fixed::Block,
        Fixed::Alarms,
    ];
}

fn info(
    id: usize,
    name: impl Into<String>,
    address: impl Into<String>,
    data_type: &str,
    unit: &str,
    writable: bool,
    boolean: bool,
) -> PointInfo {
    PointInfo {
        id: u32::try_from(id).unwrap_or(u32::MAX),
        name: name.into(),
        address: address.into(),
        data_type: data_type.into(),
        unit: unit.into(),
        writable,
        boolean,
        states: Vec::new(),
    }
}

/// The points of a client: the fixed ones, four positions per axis, then
/// one writable point per macro variable.
pub(crate) fn client_points(axes: &[String], macros: &[u32]) -> Vec<PointInfo> {
    let mut points = Vec::new();
    let mut add =
        |name: String, address: String, data_type: &str, unit: &str, writable, boolean| {
            let id = points.len();
            points.push(info(id, name, address, data_type, unit, writable, boolean));
        };
    for fixed in Fixed::ALL {
        match fixed {
            Fixed::Controller => add(
                "Controller".into(),
                "SYSINFO".into(),
                "Text",
                "",
                false,
                false,
            ),
            Fixed::Mode => add(
                "Mode".into(),
                "STATINFO.aut".into(),
                "Text",
                "",
                false,
                false,
            ),
            Fixed::Run => add(
                "Run state".into(),
                "STATINFO.run".into(),
                "Text",
                "",
                false,
                false,
            ),
            Fixed::Motion => add(
                "Motion".into(),
                "STATINFO.motion".into(),
                "Text",
                "",
                false,
                false,
            ),
            Fixed::Cycle => add(
                "Cycle running".into(),
                "STATINFO.run".into(),
                "Bool",
                "",
                false,
                true,
            ),
            Fixed::Emergency => {
                add(
                    "Emergency stop".into(),
                    "STATINFO.emergency".into(),
                    "Bool",
                    "",
                    false,
                    true,
                );
            }
            Fixed::Alarm => add(
                "Alarm active".into(),
                "STATINFO.alarm".into(),
                "Bool",
                "",
                false,
                true,
            ),
            Fixed::Feed => add(
                "Feed rate".into(),
                "ACTF".into(),
                "Number",
                "mm/min",
                false,
                false,
            ),
            Fixed::Spindle => add(
                "Spindle speed".into(),
                "ACTS".into(),
                "Number",
                "rpm",
                false,
                false,
            ),
            Fixed::Program => add(
                "Program".into(),
                "PRGNUM".into(),
                "O number",
                "",
                false,
                false,
            ),
            Fixed::Sequence => add(
                "Sequence".into(),
                "SEQNUM".into(),
                "N number",
                "",
                false,
                false,
            ),
            Fixed::Block => add("Block".into(), "EXECPROG".into(), "Text", "", false, false),
            Fixed::Alarms => add(
                "Alarm messages".into(),
                "ALMMSG".into(),
                "Text",
                "",
                false,
                false,
            ),
        }
    }
    for axis in axes {
        for (index, kind) in POSITIONS.iter().enumerate() {
            add(
                format!("{kind} {axis}"),
                format!("POS{}.{axis}", index + 1),
                "Number",
                "mm",
                false,
                false,
            );
        }
    }
    for number in macros {
        add(
            format!("Macro #{number}"),
            format!("#{number}"),
            "Number",
            "",
            true,
            false,
        );
    }
    points
}

/// The index of the first position point and of the first macro point.
pub(crate) fn layout(axes: usize) -> (usize, usize) {
    let first_position = Fixed::ALL.len();
    (first_position, first_position + axes * POSITIONS.len())
}

pub(crate) fn good(value: f64) -> Reading {
    Reading {
        value,
        raw: [0; 4],
        raw_len: 0,
        quality: Quality::Good,
    }
}

/// A reading that has no number: its text is what the point shows.
fn text_only() -> Reading {
    good(f64::NAN)
}

fn flag(on: bool) -> Reading {
    good(f64::from(u8::from(on)))
}

/// The axes' names for the layout: what the controller reports, else
/// numbered after the positions it did send.
pub(crate) fn axis_labels(snapshot: &Snapshot, sysinfo: Option<&SysInfo>) -> Vec<String> {
    if let FocasReading::Value(names) = &snapshot.positions.axes {
        return names.iter().map(AxisName::to_string).collect();
    }
    let count = match &snapshot.positions.absolute {
        FocasReading::Value(values) => values.len(),
        FocasReading::Failed(_) => sysinfo.and_then(SysInfo::axis_count).map_or(0, usize::from),
    };
    (1..=count).map(|n| format!("Axis {n}")).collect()
}

/// What one poll found. The readings are in point order, and the texts hold
/// what the text points show (empty for the rest).
pub(crate) struct Polled {
    pub readings: Vec<Reading>,
    pub texts: Vec<String>,
    /// Items the controller refused, by name.
    pub failed: Vec<String>,
}

impl Polled {
    /// What the exchange found, for the traffic view.
    pub(crate) fn summary(&self) -> String {
        if self.failed.is_empty() {
            "every item answered".to_owned()
        } else {
            format!("refused: {}", self.failed.join(", "))
        }
    }
}

/// Turn a snapshot (plus the extra calls of a poll) into the point order of
/// [`client_points`]. A refused item is bad on its own; the rest read on.
pub(crate) fn poll_readings(
    snapshot: &Snapshot,
    sysinfo: Option<&SysInfo>,
    block: Option<&str>,
    alarm_messages: Option<&str>,
    axes: usize,
    macros: &[(u32, Option<f64>)],
) -> Polled {
    let total = Fixed::ALL.len() + axes * POSITIONS.len() + macros.len();
    let mut polled = Polled {
        readings: vec![Reading::bad(); total],
        texts: vec![String::new(); total],
        failed: Vec::new(),
    };
    let fail = |what: &str, failed: &mut Vec<String>| failed.push(what.to_owned());
    let put = |index: usize, reading: Reading, text: String, polled: &mut Polled| {
        polled.readings[index] = reading;
        polled.texts[index] = text;
    };
    let slot = |fixed: Fixed| {
        Fixed::ALL
            .iter()
            .position(|each| *each == fixed)
            .unwrap_or(0)
    };

    if let Some(sysinfo) = sysinfo {
        let text = format!(
            "{} {} {} {}",
            sysinfo.cnc_type.trim(),
            sysinfo.mt_type.trim(),
            sysinfo.series.trim(),
            sysinfo.version.trim()
        );
        put(slot(Fixed::Controller), text_only(), text, &mut polled);
    }
    match &snapshot.status {
        FocasReading::Value(status) => {
            put(
                slot(Fixed::Mode),
                text_only(),
                status.aut.label().to_owned(),
                &mut polled,
            );
            put(
                slot(Fixed::Run),
                text_only(),
                status.run.label().to_owned(),
                &mut polled,
            );
            put(
                slot(Fixed::Motion),
                text_only(),
                status.motion.label().to_owned(),
                &mut polled,
            );
            put(
                slot(Fixed::Cycle),
                flag(status.run == mt_focas::RunState::Start),
                String::new(),
                &mut polled,
            );
            put(
                slot(Fixed::Emergency),
                flag(status.emergency == mt_focas::EmergencyState::Emergency),
                String::new(),
                &mut polled,
            );
            put(
                slot(Fixed::Alarm),
                flag(status.alarm != mt_focas::AlarmState::None),
                String::new(),
                &mut polled,
            );
        }
        FocasReading::Failed(_) => fail("status", &mut polled.failed),
    }
    match &snapshot.feed_rate {
        FocasReading::Value(Some(feed)) => {
            put(slot(Fixed::Feed), good(*feed), String::new(), &mut polled)
        }
        FocasReading::Value(None) => {}
        FocasReading::Failed(_) => fail("feed rate", &mut polled.failed),
    }
    match &snapshot.spindle_speed {
        FocasReading::Value(Some(speed)) => {
            put(
                slot(Fixed::Spindle),
                good(*speed),
                String::new(),
                &mut polled,
            );
        }
        FocasReading::Value(None) => {}
        FocasReading::Failed(_) => fail("spindle speed", &mut polled.failed),
    }
    match &snapshot.programs {
        FocasReading::Value(programs) => put(
            slot(Fixed::Program),
            good(f64::from(programs.running)),
            String::new(),
            &mut polled,
        ),
        FocasReading::Failed(_) => fail("program number", &mut polled.failed),
    }
    match &snapshot.sequence_number {
        FocasReading::Value(sequence) => {
            put(
                slot(Fixed::Sequence),
                good(f64::from(*sequence)),
                String::new(),
                &mut polled,
            );
        }
        FocasReading::Failed(_) => fail("sequence number", &mut polled.failed),
    }
    if let Some(block) = block {
        put(
            slot(Fixed::Block),
            text_only(),
            block.trim().to_owned(),
            &mut polled,
        );
    }
    match &snapshot.alarm_bits {
        FocasReading::Value(bits) => {
            let alarmed = bits.any();
            if alarmed || alarm_messages.is_some() {
                put(
                    slot(Fixed::Alarms),
                    text_only(),
                    alarm_messages.map_or_else(
                        || format!("{} alarm type(s)", bits.types().len()),
                        str::to_owned,
                    ),
                    &mut polled,
                );
            } else {
                put(slot(Fixed::Alarms), text_only(), String::new(), &mut polled);
            }
        }
        FocasReading::Failed(_) => fail("alarms", &mut polled.failed),
    }

    let (first, macro_start) = layout(axes);
    let kinds = [
        &snapshot.positions.absolute,
        &snapshot.positions.machine,
        &snapshot.positions.relative,
        &snapshot.positions.distance_to_go,
    ];
    for (kind, positions) in kinds.iter().enumerate() {
        match positions {
            FocasReading::Value(values) => {
                for axis in 0..axes {
                    if let Some(Some(value)) = values.get(axis) {
                        put(
                            first + axis * POSITIONS.len() + kind,
                            good(*value),
                            String::new(),
                            &mut polled,
                        );
                    }
                }
            }
            FocasReading::Failed(_) => fail(&POSITIONS[kind].to_lowercase(), &mut polled.failed),
        }
    }
    for (index, (number, value)) in macros.iter().enumerate() {
        match value {
            Some(value) => put(
                macro_start + index,
                good(*value),
                String::new(),
                &mut polled,
            ),
            None => polled.failed.push(format!("macro #{number}")),
        }
    }
    polled
}

pub(crate) fn profile(profile: FocasProfile) -> CncProfile {
    match profile {
        FocasProfile::Series31iB => CncProfile::Series31iB,
        FocasProfile::Series0iF => CncProfile::Series0iF,
    }
}

/// Runs of consecutive numbers in `numbers`, each as `(first, last)`, so a
/// list of macro variables is read in as few requests as it has gaps.
pub(crate) fn runs(numbers: &[u32]) -> Vec<(u32, u32)> {
    let mut sorted: Vec<u32> = numbers.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for number in sorted {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == number => *last = number,
            _ => runs.push((number, number)),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_numbers_are_read_in_runs() {
        assert_eq!(
            runs(&[502, 500, 501, 600, 500]),
            vec![(500, 502), (600, 600)]
        );
        assert!(runs(&[]).is_empty());
    }

    #[test]
    fn client_points_are_the_fixed_ones_then_axes_then_macros() {
        let axes = vec!["X".to_owned(), "Y".to_owned()];
        let points = client_points(&axes, &[500, 510]);
        let (first, macro_start) = layout(axes.len());
        assert_eq!(points.len(), Fixed::ALL.len() + 8 + 2);
        assert_eq!(points[first].name, "Absolute X");
        assert_eq!(points[first + 4].name, "Absolute Y");
        assert_eq!(points[macro_start].name, "Macro #500");
        assert!(points[macro_start].writable);
        assert!(!points[first].writable);
        for (index, point) in points.iter().enumerate() {
            assert_eq!(point.id as usize, index);
        }
    }
}
