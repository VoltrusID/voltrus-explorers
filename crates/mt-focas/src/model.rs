//! The simulated CNC: a plain, serializable state that the simulator answers requests from and a
//! UI may change while the simulator runs.
//!
//! The model is a 3-axis milling machine (X, Y, Z) with a spindle, a demo program that
//! [`CncModel::advance`] plays when the cycle is started, a part counter, alarms that can be
//! raised and cleared, macro variables, parameters, diagnosis values and PMC memory. Nothing
//! here is captured from a real controller; the identity strings are plausible, not recorded.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::functions::{diag_type, param_type};
use crate::types::*;

/// Which controller family the simulated CNC presents itself as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CncProfile {
    /// FANUC Series 31i-B. Reports CNC type `31`, which pyfanuc's `statinfo` accepts.
    Series31iB,
    /// FANUC Series 0i-F. Reports CNC type ` 0`.
    Series0iF,
}

impl CncProfile {
    pub fn sysinfo(self, axes: usize) -> SysInfo {
        match self {
            CncProfile::Series31iB => SysInfo {
                // "i" series CNC (bit 1), MODEL B (2 in bits 8..15)
                addinfo: 0x0202,
                max_axis: 32,
                cnc_type: "31".into(),
                mt_type: "M".into(),
                series: "G421".into(),
                version: "42.0".into(),
                axes: axes.to_string(),
            },
            CncProfile::Series0iF => SysInfo {
                // "i" series CNC (bit 1), MODEL F (6 in bits 8..15)
                addinfo: 0x0602,
                max_axis: 8,
                cnc_type: "0".into(),
                mt_type: "M".into(),
                series: "D4F1".into(),
                version: "12.0".into(),
                axes: axes.to_string(),
            },
        }
    }
}

/// One simulated axis. All lengths are millimetres (degrees for a rotary axis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimAxis {
    pub name: AxisName,
    /// Absolute position in the active work coordinate system.
    pub absolute: f64,
    /// Machine position = absolute + `work_offset`.
    pub work_offset: f64,
    /// Relative position = absolute - `relative_origin`.
    pub relative_origin: f64,
    /// Distance still to travel in the current block.
    pub distance_to_go: f64,
    /// Position where the skip signal last fired.
    pub skip_position: f64,
    /// Servo load meter, percent.
    pub servo_load: f64,
}

impl SimAxis {
    fn new(name: char, work_offset: f64) -> Self {
        SimAxis {
            name: AxisName::new(name),
            absolute: 0.0,
            work_offset,
            relative_origin: 0.0,
            distance_to_go: 0.0,
            skip_position: 0.0,
            servo_load: 0.0,
        }
    }

    pub fn machine(&self) -> f64 {
        self.absolute + self.work_offset
    }

    pub fn relative(&self) -> f64 {
        self.absolute - self.relative_origin
    }
}

/// One block of a program: its text and what it does when played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimBlock {
    pub text: String,
    /// Absolute target per axis; `None` leaves the axis where it is.
    pub target: Vec<Option<f64>>,
    /// Rapid traverse instead of cutting feed.
    pub rapid: bool,
    /// Feed in mm/min for a cutting move.
    pub feed: f64,
    /// Spindle command (S with M03), rpm; `Some(0.0)` stops the spindle.
    pub spindle: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimProgram {
    pub comment: String,
    pub blocks: Vec<SimBlock>,
}

impl SimProgram {
    /// Program text as the CNC stores it, one block per line.
    pub fn text(&self) -> String {
        let mut t = String::from("%\n");
        for b in &self.blocks {
            t.push_str(&b.text);
            t.push('\n');
        }
        t.push('%');
        t
    }

    pub fn size(&self) -> u32 {
        self.text().len() as u32
    }
}

/// An alarm that is currently active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimAlarm {
    pub number: i32,
    pub alarm_type: AlarmType,
    /// 1-based axis, 0 for none.
    pub axis: i32,
    pub message: String,
}

/// A parameter or diagnosis item: storage type code and one value per axis (one value for
/// non-axis items).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimNumbered {
    pub data_type: u16,
    pub values: Vec<i32>,
}

/// Number of PMC address bytes per area. The simulator refuses reads beyond it.
pub fn pmc_area_size(area: PmcArea) -> u32 {
    match area {
        PmcArea::A => 50,
        PmcArea::T | PmcArea::C => 400,
        PmcArea::K => 100,
        PmcArea::D => 10_000,
        PmcArea::R => 9_100,
        PmcArea::G | PmcArea::F | PmcArea::X | PmcArea::Y => 1_000,
    }
}

/// Demo program spindle speed, rpm.
const DEMO_SPINDLE: f64 = 3000.0;
/// Rapid traverse rate, mm/min.
const RAPID_RATE: f64 = 12_000.0;
/// Spindle speed change, rpm per second.
const SPINDLE_ACCEL: f64 = 6000.0;

fn block(
    text: &str,
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
    rapid: bool,
    feed: f64,
    spindle: Option<f64>,
) -> SimBlock {
    SimBlock {
        text: text.into(),
        target: vec![x, y, z],
        rapid,
        feed,
        spindle,
    }
}

/// O3000: a 60 x 40 mm pocket outline, 2 mm deep, with spindle start and stop.
fn pocket_program() -> SimProgram {
    SimProgram {
        comment: "POCKET DEMO".into(),
        blocks: vec![
            block(
                "N10 G90 G54 G00 X0. Y0. S3000 M03",
                Some(0.0),
                Some(0.0),
                Some(50.0),
                true,
                0.0,
                Some(DEMO_SPINDLE),
            ),
            block("N20 G00 Z5.", None, None, Some(5.0), true, 0.0, None),
            block(
                "N30 G01 Z-2. F200",
                None,
                None,
                Some(-2.0),
                false,
                200.0,
                None,
            ),
            block(
                "N40 G01 X60. F600",
                Some(60.0),
                None,
                None,
                false,
                600.0,
                None,
            ),
            block("N50 Y40.", None, Some(40.0), None, false, 600.0, None),
            block("N60 X0.", Some(0.0), None, None, false, 600.0, None),
            block("N70 Y0.", None, Some(0.0), None, false, 600.0, None),
            block(
                "N80 G00 Z50. M05",
                None,
                None,
                Some(50.0),
                true,
                0.0,
                Some(0.0),
            ),
            block("N90 M30", None, None, None, true, 0.0, None),
        ],
    }
}

/// O3001: a short facing pass.
fn face_program() -> SimProgram {
    SimProgram {
        comment: "FACE PASS".into(),
        blocks: vec![
            block(
                "N10 G90 G54 G00 X-10. Y0. S2500 M03",
                Some(-10.0),
                Some(0.0),
                Some(50.0),
                true,
                0.0,
                Some(2500.0),
            ),
            block(
                "N20 G01 Z0. F300",
                None,
                None,
                Some(0.0),
                false,
                300.0,
                None,
            ),
            block("N30 X70. F900", Some(70.0), None, None, false, 900.0, None),
            block(
                "N40 G00 Z50. M05",
                None,
                None,
                Some(50.0),
                true,
                0.0,
                Some(0.0),
            ),
            block("N50 M30", None, None, None, true, 0.0, None),
        ],
    }
}

/// The whole simulated machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CncModel {
    pub sysinfo: SysInfo,
    pub mode: AutoMode,
    pub run: RunState,
    pub motion: MotionState,
    pub mstb: MstbState,
    pub emergency: EmergencyState,
    pub edit: EditState,
    pub axes: Vec<SimAxis>,
    /// Decimal places of the position numbers sent on the wire.
    pub position_decimals: u8,
    /// Actual feed rate, mm/min.
    pub feed_rate: f64,
    /// Actual spindle speed, rpm.
    pub spindle_speed: f64,
    /// Speed the spindle ramps to, rpm.
    pub spindle_target: f64,
    pub programs: BTreeMap<u32, SimProgram>,
    /// Main program number; the cycle plays it.
    pub main_program: u32,
    /// Index of the block being executed.
    pub block_index: usize,
    pub alarms: Vec<SimAlarm>,
    /// Macro variables; a missing number reads as vacant (invalid).
    pub macros: BTreeMap<u32, f64>,
    /// The macro variable that counts finished parts.
    pub part_counter_macro: u32,
    pub params: BTreeMap<u32, SimNumbered>,
    pub diags: BTreeMap<u32, SimNumbered>,
    /// PMC bytes by area (index = [`PmcArea::code`]); bytes never written read as 0.
    pub pmc: [BTreeMap<u32, u8>; 10],
    /// Start the cycle again when the program ends.
    pub auto_repeat: bool,
    /// Seconds of simulated time.
    pub clock: f64,
}

impl Default for CncModel {
    fn default() -> Self {
        CncModel::new(CncProfile::Series31iB)
    }
}

impl CncModel {
    pub fn new(profile: CncProfile) -> Self {
        let axes = vec![
            SimAxis::new('X', -250.0),
            SimAxis::new('Y', -150.0),
            SimAxis::new('Z', -300.0),
        ];
        let mut axes = axes;
        axes[2].absolute = 50.0;
        let mut programs = BTreeMap::new();
        programs.insert(3000, pocket_program());
        programs.insert(3001, face_program());

        let mut macros = BTreeMap::new();
        macros.insert(500, 0.0);
        macros.insert(501, 1250.5);
        macros.insert(502, 100.0);

        let mut params = BTreeMap::new();
        let numbered = |data_type, values: &[i32]| SimNumbered {
            data_type,
            values: values.to_vec(),
        };
        // 1020: program axis name (88, 89, 90 = X, Y, Z)
        params.insert(1020, numbered(param_type::BYTE, &[88, 89, 90]));
        // 1320 / 1321: stored stroke limit 1, plus / minus side
        params.insert(
            1320,
            numbered(param_type::INT, &[300_000, 200_000, 100_000]),
        );
        params.insert(
            1321,
            numbered(param_type::INT, &[-300_000, -200_000, -100_000]),
        );
        // 3401: bit parameter
        params.insert(3401, numbered(param_type::BIT, &[0b0000_0001]));
        // 3741: maximum spindle speed
        params.insert(3741, numbered(param_type::INT, &[8000]));
        // 6711 / 6712: number of machined parts / total number of machined parts
        params.insert(6711, numbered(param_type::INT, &[0]));
        params.insert(6712, numbered(param_type::INT, &[0]));

        let mut diags = BTreeMap::new();
        diags.insert(308, numbered(diag_type::INT, &[28, 28, 44]));
        diags.insert(309, numbered(diag_type::INT, &[36, 36, 49]));

        let mut model = CncModel {
            sysinfo: profile.sysinfo(axes.len()),
            mode: AutoMode::Memory,
            run: RunState::Reset,
            motion: MotionState::None,
            mstb: MstbState::None,
            emergency: EmergencyState::Normal,
            edit: EditState::NotEditing,
            axes,
            position_decimals: 3,
            feed_rate: 0.0,
            spindle_speed: 0.0,
            spindle_target: 0.0,
            programs,
            main_program: 3000,
            block_index: 0,
            alarms: Vec::new(),
            macros,
            part_counter_macro: 500,
            params,
            diags,
            pmc: Default::default(),
            auto_repeat: false,
            clock: 0.0,
        };
        // PMC data the reference's own examples read: D2204 as a word, D1870 and D1874 as dwords.
        model.set_pmc(PmcArea::D, PmcDataType::Word, 2204, 600);
        model.set_pmc(PmcArea::D, PmcDataType::DWord, 1870, 100);
        model.set_pmc(PmcArea::D, PmcDataType::DWord, 1874, 500);
        model.set_pmc(PmcArea::X, PmcDataType::Byte, 0, 0x01);
        model
    }

    // ---- derived values ----------------------------------------------------------------------

    pub fn status(&self) -> StatInfo {
        StatInfo {
            aut: self.mode,
            run: self.run,
            motion: self.motion,
            mstb: self.mstb,
            emergency: self.emergency,
            alarm: if self.alarms.is_empty() {
                AlarmState::None
            } else {
                AlarmState::Alarm
            },
            edit: self.edit,
        }
    }

    /// `cnc_alarm` bit field: one bit per alarm type with an active alarm.
    pub fn alarm_bits(&self) -> AlarmBits {
        let mut bits = 0u32;
        for a in &self.alarms {
            if (0..32).contains(&a.alarm_type.code()) {
                bits |= 1 << a.alarm_type.code();
            }
        }
        AlarmBits(bits)
    }

    pub fn program_numbers(&self) -> ProgramNumbers {
        ProgramNumbers {
            running: self.main_program as i32,
            main: self.main_program as i32,
        }
    }

    /// Text of the block being executed, empty without a program.
    pub fn current_block_text(&self) -> String {
        self.programs
            .get(&self.main_program)
            .and_then(|p| p.blocks.get(self.block_index))
            .map(|b| b.text.clone())
            .unwrap_or_default()
    }

    /// Sequence (N) number of the executing block, 0 when it has none.
    pub fn sequence_number(&self) -> i32 {
        let text = self.current_block_text();
        text.strip_prefix('N')
            .and_then(|t| {
                let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok()
            })
            .unwrap_or(0)
    }

    pub fn part_count(&self) -> u64 {
        self.macros
            .get(&self.part_counter_macro)
            .map_or(0, |v| *v as u64)
    }

    // ---- operator actions --------------------------------------------------------------------

    /// Start, or resume after a feed hold. Refused (returns `false`) during an emergency stop,
    /// with an alarm active, outside MEM/MDI mode or without a main program.
    pub fn cycle_start(&mut self) -> bool {
        let runnable = self.emergency == EmergencyState::Normal
            && self.alarms.is_empty()
            && matches!(self.mode, AutoMode::Memory | AutoMode::Mdi)
            && self
                .programs
                .get(&self.main_program)
                .is_some_and(|p| !p.blocks.is_empty());
        if !runnable {
            return false;
        }
        if self.run == RunState::Reset {
            self.block_index = 0;
        }
        self.run = RunState::Start;
        true
    }

    /// Feed hold: stop moving, keep the program position.
    pub fn feed_hold(&mut self) {
        if self.run == RunState::Start {
            self.run = RunState::Hold;
            self.motion = MotionState::None;
            self.feed_rate = 0.0;
        }
    }

    /// Reset: end the cycle and rewind the program. The spindle stops; alarms stay until cleared.
    pub fn reset(&mut self) {
        self.halt_cycle();
        self.block_index = 0;
    }

    fn halt_cycle(&mut self) {
        self.run = RunState::Reset;
        self.motion = MotionState::None;
        self.feed_rate = 0.0;
        self.spindle_target = 0.0;
        for a in &mut self.axes {
            a.distance_to_go = 0.0;
        }
    }

    pub fn set_emergency(&mut self, on: bool) {
        if on {
            self.emergency = EmergencyState::Emergency;
            self.halt_cycle();
        } else {
            self.emergency = EmergencyState::Normal;
        }
    }

    pub fn set_mode(&mut self, mode: AutoMode) {
        self.mode = mode;
    }

    /// Raise an alarm. A running cycle stops. An alarm with the same number is replaced.
    pub fn raise_alarm(
        &mut self,
        number: i32,
        alarm_type: AlarmType,
        axis: i32,
        message: impl Into<String>,
    ) {
        self.alarms.retain(|a| a.number != number);
        self.alarms.push(SimAlarm {
            number,
            alarm_type,
            axis,
            message: message.into(),
        });
        self.halt_cycle();
    }

    /// Clear the alarm with `number`; `true` if there was one.
    pub fn clear_alarm(&mut self, number: i32) -> bool {
        let before = self.alarms.len();
        self.alarms.retain(|a| a.number != number);
        self.alarms.len() != before
    }

    pub fn clear_alarms(&mut self) {
        self.alarms.clear();
    }

    pub fn set_macro(&mut self, number: u32, value: f64) {
        self.macros.insert(number, value);
    }

    /// Write PMC memory (big-endian multi-byte values, as the PMC read returns them).
    pub fn set_pmc(&mut self, area: PmcArea, data_type: PmcDataType, address: u32, value: u32) {
        let size = data_type.size();
        let bytes = value.to_be_bytes();
        for (i, b) in bytes[4 - size..].iter().enumerate() {
            self.pmc[area.code() as usize].insert(address + i as u32, *b);
        }
    }

    pub fn pmc_byte(&self, area: PmcArea, address: u32) -> u8 {
        self.pmc[area.code() as usize]
            .get(&address)
            .copied()
            .unwrap_or(0)
    }

    // ---- time --------------------------------------------------------------------------------

    /// Advance the simulation by `dt` seconds: ramp the spindle and, while the cycle runs, move
    /// the axes along the program.
    pub fn advance(&mut self, dt: f64) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.clock += dt;

        if self.run == RunState::Start && self.emergency == EmergencyState::Normal {
            let mut left = dt;
            // A program with only zero-length blocks must not spin forever.
            let mut guard = 0;
            while left > 0.0 && self.run == RunState::Start && guard < 10_000 {
                guard += 1;
                left = self.step_block(left);
            }
        } else {
            self.feed_rate = 0.0;
            for a in &mut self.axes {
                a.servo_load = 0.0;
            }
        }

        // The spindle follows its command whether or not the cycle runs.
        let step = SPINDLE_ACCEL * dt;
        let delta = self.spindle_target - self.spindle_speed;
        self.spindle_speed += delta.clamp(-step, step);
    }

    /// Play the current block for up to `left` seconds; returns the time not used.
    fn step_block(&mut self, left: f64) -> f64 {
        let Some(program) = self.programs.get(&self.main_program) else {
            self.finish_cycle();
            return left;
        };
        let Some(blk) = program.blocks.get(self.block_index).cloned() else {
            self.finish_cycle();
            return left;
        };
        if let Some(s) = blk.spindle {
            self.spindle_target = s;
        }
        // Per-axis remaining distance to this block's target.
        let delta: Vec<f64> = self
            .axes
            .iter()
            .enumerate()
            .map(|(i, a)| {
                blk.target
                    .get(i)
                    .copied()
                    .flatten()
                    .map_or(0.0, |t| t - a.absolute)
            })
            .collect();
        let dist = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
        let feed = if blk.rapid {
            RAPID_RATE
        } else {
            blk.feed.max(1.0)
        };
        let speed = feed / 60.0; // mm/s

        if dist <= f64::EPSILON {
            for a in &mut self.axes {
                a.distance_to_go = 0.0;
                a.servo_load = 0.0;
            }
            self.motion = MotionState::None;
            self.feed_rate = 0.0;
            self.next_block();
            return left;
        }

        self.motion = MotionState::Motion;
        self.feed_rate = if blk.rapid { 0.0 } else { feed };
        let travel = speed * left;
        if travel >= dist {
            for (a, d) in self.axes.iter_mut().zip(&delta) {
                a.absolute += d;
                a.distance_to_go = 0.0;
            }
            self.next_block();
            left - dist / speed
        } else {
            let k = travel / dist;
            for (a, d) in self.axes.iter_mut().zip(&delta) {
                a.absolute += d * k;
                a.distance_to_go = d * (1.0 - k);
                // deterministic load: idle 2 %, rising with the share of the move this axis does
                a.servo_load = 2.0 + 20.0 * (d.abs() / dist) * (feed / 1000.0).min(3.0);
            }
            0.0
        }
    }

    fn next_block(&mut self) {
        self.block_index += 1;
        let len = self
            .programs
            .get(&self.main_program)
            .map_or(0, |p| p.blocks.len());
        if self.block_index >= len {
            self.finish_cycle();
        }
    }

    fn finish_cycle(&mut self) {
        // M30: one more part, spindle off, program rewound
        let n = self.part_counter_macro;
        let count = self.macros.get(&n).copied().unwrap_or(0.0) + 1.0;
        self.macros.insert(n, count);
        for p in [6711, 6712] {
            if let Some(v) = self.params.get_mut(&p).and_then(|p| p.values.first_mut()) {
                *v += 1;
            }
        }
        self.block_index = 0;
        self.motion = MotionState::None;
        self.feed_rate = 0.0;
        self.spindle_target = 0.0;
        for a in &mut self.axes {
            a.distance_to_go = 0.0;
            a.servo_load = 0.0;
        }
        self.run = if self.auto_repeat {
            RunState::Start
        } else {
            RunState::Reset
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_runs_to_the_end_and_counts_a_part() {
        let mut m = CncModel::default();
        assert_eq!(m.part_count(), 0);
        assert!(m.cycle_start());
        assert_eq!(m.run, RunState::Start);
        let mut t = 0.0;
        while m.run == RunState::Start && t < 600.0 {
            m.advance(0.1);
            t += 0.1;
        }
        assert_eq!(m.run, RunState::Reset, "cycle did not finish in {t} s");
        assert_eq!(m.part_count(), 1);
        assert_eq!(m.params[&6711].values, vec![1]);
        assert_eq!(m.block_index, 0);
        // back home at the program's last position
        assert!((m.axes[2].absolute - 50.0).abs() < 1e-6);
        assert!(m.axes[0].absolute.abs() < 1e-6);
    }

    #[test]
    fn axes_move_and_distance_to_go_shrinks() {
        let mut m = CncModel::default();
        m.cycle_start();
        m.advance(0.5);
        assert_eq!(m.motion, MotionState::Motion);
        let moved = m.axes.iter().any(|a| a.distance_to_go != 0.0);
        assert!(moved || m.block_index > 0);
        assert!(m.spindle_speed > 0.0);
        m.feed_hold();
        let held = m.axes.iter().map(|a| a.absolute).collect::<Vec<_>>();
        m.advance(1.0);
        assert_eq!(held, m.axes.iter().map(|a| a.absolute).collect::<Vec<_>>());
        assert_eq!(m.run, RunState::Hold);
        assert_eq!(m.feed_rate, 0.0);
    }

    #[test]
    fn alarm_stops_cycle_and_blocks_restart() {
        let mut m = CncModel::default();
        m.cycle_start();
        m.advance(0.2);
        m.raise_alarm(401, AlarmType::Servo, 2, "SERVO ALARM: VRDY OFF");
        assert_eq!(m.run, RunState::Reset);
        assert_eq!(m.status().alarm, AlarmState::Alarm);
        assert_eq!(m.alarm_bits(), AlarmBits(1 << 6));
        assert!(!m.cycle_start());
        assert!(m.clear_alarm(401));
        assert!(!m.clear_alarm(401));
        assert_eq!(m.status().alarm, AlarmState::None);
        assert!(m.cycle_start());
    }

    #[test]
    fn emergency_stop_blocks_the_cycle() {
        let mut m = CncModel::default();
        m.set_emergency(true);
        assert_eq!(m.status().emergency, EmergencyState::Emergency);
        assert!(!m.cycle_start());
        m.set_emergency(false);
        assert!(m.cycle_start());
    }

    #[test]
    fn sequence_number_follows_the_block() {
        let mut m = CncModel::default();
        assert_eq!(m.sequence_number(), 10);
        m.block_index = 3;
        assert_eq!(m.sequence_number(), 40);
        assert_eq!(m.current_block_text(), "N40 G01 X60. F600");
    }

    #[test]
    fn model_serializes() {
        let m = CncModel::default();
        let json = serde_json::to_string(&m).unwrap();
        let back: CncModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
