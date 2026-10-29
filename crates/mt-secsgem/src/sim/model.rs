//! The simulated equipment: a wafer processing chamber behind a GEM interface.
//!
//! [`EquipmentModel`] is plain data (`Serialize`/`Deserialize`) plus the behaviour of the tool:
//! the control-state and communication-state machines, the chamber process (start, pause,
//! resume, stop, complete), alarms, collection events with their report definitions and links.
//! Whatever a UI changes here shows in the next message the simulator sends or answers.
//!
//! Messages the model wants to send on its own (S6F11 for an enabled event, S5F1 for an alarm) are
//! built at the moment they happen, with the variable values of that moment, and queued in an
//! outbox the simulator drains. Nothing is spooled: an event that happens while the host is not
//! communicating, or while it is disabled, is dropped.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::gem::{
    AlarmReport, CommunicationState, ControlState, Eac, EcInfo, EventLink, EventReport, IdFormat,
    OflAck, OnlAck, ReportDef, ReportValues, SvInfo, encode_s5f1, encode_s6f11,
};
use crate::item::Item;
use crate::message::SecsMessage;

/// Status variable ids.
pub mod svid {
    pub const CLOCK: u64 = 1;
    pub const CONTROL_STATE: u64 = 2;
    pub const PREVIOUS_CONTROL_STATE: u64 = 3;
    pub const PROCESS_STATE: u64 = 4;
    pub const PREVIOUS_PROCESS_STATE: u64 = 5;
    pub const COMMUNICATION_STATE: u64 = 6;
    pub const CHAMBER_PRESSURE: u64 = 10;
    pub const CHAMBER_TEMPERATURE: u64 = 11;
    pub const WAFER_COUNT: u64 = 12;
    pub const ACTIVE_PPID: u64 = 13;
    pub const PROCESS_ELAPSED: u64 = 14;
    pub const LOT_ID: u64 = 15;
    pub const ALARMS_SET: u64 = 16;
}

/// Equipment constant ids.
pub mod ecid {
    pub const PROCESS_TEMPERATURE: u64 = 1001;
    pub const PROCESS_TIME: u64 = 1002;
    pub const PROCESS_PRESSURE: u64 = 1003;
    pub const MAX_WAFERS_PER_LOT: u64 = 1004;
    pub const ESTABLISH_COMM_TIMEOUT: u64 = 1005;
}

/// Collection event ids.
pub mod ceid {
    pub const PROCESS_STARTED: u64 = 100;
    pub const PROCESS_COMPLETED: u64 = 101;
    pub const PROCESS_STOPPED: u64 = 102;
    pub const PROCESS_PAUSED: u64 = 103;
    pub const PROCESS_RESUMED: u64 = 104;
    pub const EQUIPMENT_OFFLINE: u64 = 201;
    pub const ONLINE_LOCAL: u64 = 202;
    pub const ONLINE_REMOTE: u64 = 203;
    pub const HOST_OFFLINE: u64 = 204;
    pub const ALARM_SET: u64 = 300;
    pub const ALARM_CLEARED: u64 = 301;
}

/// Alarm ids.
pub mod alid {
    pub const OVER_TEMPERATURE: u64 = 1;
    pub const DOOR_OPEN: u64 = 2;
    pub const PUMP_WARNING: u64 = 3;
}

/// The `ProcessState` status variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessState {
    Idle,
    Executing,
    Paused,
}

impl ProcessState {
    pub fn code(self) -> u8 {
        match self {
            ProcessState::Idle => 0,
            ProcessState::Executing => 1,
            ProcessState::Paused => 2,
        }
    }
}

/// Why a process command could not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    /// The chamber is not in a state the command applies to.
    WrongState,
    UnknownPpid,
}

/// The wafer processing chamber.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chamber {
    pub state: ProcessState,
    pub previous_state: ProcessState,
    /// Process program of the running (or last) process.
    pub ppid: String,
    pub lot_id: String,
    /// Wafers processed to completion.
    pub wafer_count: u32,
    pub elapsed_s: f64,
    /// Length of the running process, fixed when it starts.
    pub duration_s: f64,
    pub pressure_mtorr: f64,
    pub temperature_c: f64,
    pub ambient_c: f64,
    pub atmosphere_mtorr: f64,
}

impl Default for Chamber {
    fn default() -> Self {
        Chamber {
            state: ProcessState::Idle,
            previous_state: ProcessState::Idle,
            ppid: String::new(),
            lot_id: String::new(),
            wafer_count: 0,
            elapsed_s: 0.0,
            duration_s: 0.0,
            pressure_mtorr: 760_000.0,
            temperature_c: 22.0,
            ambient_c: 22.0,
            atmosphere_mtorr: 760_000.0,
        }
    }
}

/// An equipment constant with its range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquipmentConstant {
    pub id: u64,
    pub name: String,
    pub units: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Current value. Its item type is the type of the constant; a set value of another numeric
    /// type is converted when it fits.
    pub value: Item,
    /// Refuse a host change (`EAC` 2, busy) while a process is running or paused.
    pub locked_while_processing: bool,
}

impl EquipmentConstant {
    pub fn value_f64(&self) -> f64 {
        self.value.as_f64().unwrap_or(self.default)
    }

    fn info(&self) -> EcInfo {
        EcInfo {
            id: self.id,
            name: self.name.clone(),
            min: like(&self.value, self.min).unwrap_or_else(Item::empty_list),
            max: like(&self.value, self.max).unwrap_or_else(Item::empty_list),
            default: like(&self.value, self.default).unwrap_or_else(Item::empty_list),
            units: self.units.clone(),
        }
    }
}

/// `value` as an item of the same type as `template`, when it is representable.
pub fn like(template: &Item, value: f64) -> Option<Item> {
    if !value.is_finite() {
        return None;
    }
    macro_rules! int {
        ($variant:ident, $ty:ty) => {{
            if value.fract() != 0.0 || value < <$ty>::MIN as f64 || value > <$ty>::MAX as f64 {
                return None;
            }
            Item::$variant(vec![value as $ty])
        }};
    }
    Some(match template {
        Item::F4(_) => Item::F4(vec![value as f32]),
        Item::F8(_) => Item::F8(vec![value]),
        Item::U1(_) => int!(U1, u8),
        Item::U2(_) => int!(U2, u16),
        Item::U4(_) => int!(U4, u32),
        Item::U8(_) => int!(U8, u64),
        Item::I1(_) => int!(I1, i8),
        Item::I2(_) => int!(I2, i16),
        Item::I4(_) => int!(I4, i32),
        Item::I8(_) => int!(I8, i64),
        _ => return None,
    })
}

/// An alarm the equipment can raise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alarm {
    pub alid: u64,
    /// `ALCD` category, bits 1-7: 1 personal safety, 2 equipment safety, 3 parameter control
    /// warning, 4 parameter control error, 5 irrecoverable error, 6 equipment status warning,
    /// 7 attention flag, 8 data integrity.
    pub category: u8,
    pub text: String,
    /// Report this alarm to the host (S5F3 changes it).
    pub enabled: bool,
    /// Currently set.
    pub set: bool,
}

impl Alarm {
    /// `ALCD`: the category, with bit 8 set while the alarm is set.
    pub fn alcd(&self) -> u8 {
        (self.category & 0x7f) | if self.set { 0x80 } else { 0 }
    }
}

/// A collection event the equipment can report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionEvent {
    pub ceid: u64,
    pub name: String,
    /// Sent to the host when it happens (S2F37 changes it); every event starts disabled.
    pub enabled: bool,
}

/// A line the host sent with S10F3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalMessage {
    pub tid: u8,
    pub text: String,
}

/// The whole simulated tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquipmentModel {
    /// `MDLN` (at most 20 characters in the standard).
    pub mdln: String,
    /// `SOFTREV` (at most 20 characters).
    pub softrev: String,
    /// Item type of the ids the equipment sends.
    pub id_format: IdFormat,
    pub control_state: ControlState,
    pub previous_control_state: ControlState,
    /// Where going online lands: `OnlineLocal` or `OnlineRemote`.
    pub online_substate: ControlState,
    pub comm_state: CommunicationState,
    /// Communication switched on at the equipment (the operator can disable it).
    pub comm_enabled: bool,
    pub chamber: Chamber,
    pub constants: Vec<EquipmentConstant>,
    pub alarms: Vec<Alarm>,
    pub events: Vec<CollectionEvent>,
    /// Report definitions made with S2F33.
    pub reports: Vec<ReportDef>,
    /// Report links made with S2F35.
    pub links: Vec<EventLink>,
    /// Most reports S2F33 may define.
    pub max_reports: usize,
    pub ppids: Vec<String>,
    /// Lines received with S10F3, oldest first.
    pub terminal_messages: Vec<TerminalMessage>,
    pub next_data_id: u64,
    /// Messages waiting to be sent to the host.
    #[serde(skip)]
    pub outbox: Vec<SecsMessage>,
}

/// Outbox size above which the oldest message is dropped.
const OUTBOX_LIMIT: usize = 1000;

impl Default for EquipmentModel {
    fn default() -> Self {
        let ec = |id, name: &str, units: &str, min, max, default, value: Item, locked| {
            EquipmentConstant {
                id,
                name: name.into(),
                units: units.into(),
                min,
                max,
                default,
                value,
                locked_while_processing: locked,
            }
        };
        let alarm = |alid, category, text: &str| Alarm {
            alid,
            category,
            text: text.into(),
            enabled: true,
            set: false,
        };
        let event = |ceid, name: &str| CollectionEvent {
            ceid,
            name: name.into(),
            enabled: false,
        };
        EquipmentModel {
            mdln: "VOLTRUS-CHAMBER".into(),
            softrev: "1.0.0".into(),
            id_format: IdFormat::U4,
            control_state: ControlState::OnlineRemote,
            previous_control_state: ControlState::HostOffline,
            online_substate: ControlState::OnlineRemote,
            comm_state: CommunicationState::WaitCra,
            comm_enabled: true,
            chamber: Chamber::default(),
            constants: vec![
                ec(
                    ecid::PROCESS_TEMPERATURE,
                    "ProcessTemperature",
                    "degC",
                    20.0,
                    400.0,
                    150.0,
                    Item::f4(150.0),
                    true,
                ),
                ec(
                    ecid::PROCESS_TIME,
                    "ProcessTime",
                    "s",
                    1.0,
                    3600.0,
                    60.0,
                    Item::u4(60),
                    true,
                ),
                ec(
                    ecid::PROCESS_PRESSURE,
                    "ProcessPressure",
                    "mTorr",
                    1.0,
                    1000.0,
                    100.0,
                    Item::f4(100.0),
                    true,
                ),
                ec(
                    ecid::MAX_WAFERS_PER_LOT,
                    "MaxWafersPerLot",
                    "",
                    1.0,
                    25.0,
                    25.0,
                    Item::u2(25),
                    false,
                ),
                ec(
                    ecid::ESTABLISH_COMM_TIMEOUT,
                    "EstablishCommTimeout",
                    "s",
                    1.0,
                    120.0,
                    10.0,
                    Item::u2(10),
                    false,
                ),
            ],
            alarms: vec![
                alarm(alid::OVER_TEMPERATURE, 2, "Chamber over temperature"),
                alarm(alid::DOOR_OPEN, 1, "Chamber door open"),
                alarm(alid::PUMP_WARNING, 6, "Vacuum pump warning"),
            ],
            events: vec![
                event(ceid::PROCESS_STARTED, "ProcessStarted"),
                event(ceid::PROCESS_COMPLETED, "ProcessCompleted"),
                event(ceid::PROCESS_STOPPED, "ProcessStopped"),
                event(ceid::PROCESS_PAUSED, "ProcessPaused"),
                event(ceid::PROCESS_RESUMED, "ProcessResumed"),
                event(ceid::EQUIPMENT_OFFLINE, "ControlStateEquipmentOffline"),
                event(ceid::ONLINE_LOCAL, "ControlStateOnlineLocal"),
                event(ceid::ONLINE_REMOTE, "ControlStateOnlineRemote"),
                event(ceid::HOST_OFFLINE, "ControlStateHostOffline"),
                event(ceid::ALARM_SET, "AlarmSet"),
                event(ceid::ALARM_CLEARED, "AlarmCleared"),
            ],
            reports: Vec::new(),
            links: Vec::new(),
            max_reports: 32,
            ppids: vec![
                "WAFER_CLEAN_A".into(),
                "ETCH_SIO2".into(),
                "ANNEAL_30".into(),
            ],
            terminal_messages: Vec::new(),
            next_data_id: 1,
            outbox: Vec::new(),
        }
    }
}

/// Name, units and id of every status variable.
const STATUS_VARIABLES: &[(u64, &str, &str)] = &[
    (svid::CLOCK, "Clock", ""),
    (svid::CONTROL_STATE, "ControlState", ""),
    (svid::PREVIOUS_CONTROL_STATE, "PreviousControlState", ""),
    (svid::PROCESS_STATE, "ProcessState", ""),
    (svid::PREVIOUS_PROCESS_STATE, "PreviousProcessState", ""),
    (svid::COMMUNICATION_STATE, "CommunicationState", ""),
    (svid::CHAMBER_PRESSURE, "ChamberPressure", "mTorr"),
    (svid::CHAMBER_TEMPERATURE, "ChamberTemperature", "degC"),
    (svid::WAFER_COUNT, "WaferCount", ""),
    (svid::ACTIVE_PPID, "ActivePPID", ""),
    (svid::PROCESS_ELAPSED, "ProcessElapsed", "s"),
    (svid::LOT_ID, "LotID", ""),
    (svid::ALARMS_SET, "AlarmsSet", ""),
];

/// `YYYYMMDDhhmmsscc` (UTC), the 16-character form of the `TIME` item.
fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        now.subsec_millis() / 10
    )
}

fn approach(value: f64, target: f64, dt: f64, tau: f64) -> f64 {
    let next = target + (value - target) * (-dt / tau).exp();
    if (next - target).abs() < 0.01 {
        target
    } else {
        next
    }
}

fn is_control_ceid(ceid: u64) -> bool {
    matches!(
        ceid,
        ceid::EQUIPMENT_OFFLINE | ceid::ONLINE_LOCAL | ceid::ONLINE_REMOTE | ceid::HOST_OFFLINE
    )
}

impl EquipmentModel {
    // ---- variables -------------------------------------------------------------------------

    /// Id, name and units of every status variable.
    pub fn status_variable_infos(&self) -> Vec<SvInfo> {
        STATUS_VARIABLES
            .iter()
            .map(|(id, name, units)| SvInfo {
                id: *id,
                name: (*name).into(),
                units: (*units).into(),
            })
            .collect()
    }

    /// Value of a status variable.
    pub fn sv_value(&self, id: u64) -> Option<Item> {
        let c = &self.chamber;
        Some(match id {
            svid::CLOCK => Item::ascii(timestamp()),
            svid::CONTROL_STATE => Item::u1(self.control_state.code()),
            svid::PREVIOUS_CONTROL_STATE => Item::u1(self.previous_control_state.code()),
            svid::PROCESS_STATE => Item::u1(c.state.code()),
            svid::PREVIOUS_PROCESS_STATE => Item::u1(c.previous_state.code()),
            svid::COMMUNICATION_STATE => Item::u1(self.comm_state.code()),
            svid::CHAMBER_PRESSURE => Item::f4(c.pressure_mtorr as f32),
            svid::CHAMBER_TEMPERATURE => Item::f4(c.temperature_c as f32),
            svid::WAFER_COUNT => Item::u4(c.wafer_count),
            svid::ACTIVE_PPID => Item::ascii(&c.ppid),
            svid::PROCESS_ELAPSED => Item::f4(c.elapsed_s as f32),
            svid::LOT_ID => Item::ascii(&c.lot_id),
            svid::ALARMS_SET => Item::U4(
                self.alarms
                    .iter()
                    .filter(|a| a.set)
                    .map(|a| a.alid as u32)
                    .collect(),
            ),
            _ => return None,
        })
    }

    pub fn constant(&self, id: u64) -> Option<&EquipmentConstant> {
        self.constants.iter().find(|c| c.id == id)
    }

    pub fn constant_infos(&self) -> Vec<EcInfo> {
        self.constants.iter().map(EquipmentConstant::info).collect()
    }

    /// Value of a variable a report may carry: a status variable or an equipment constant.
    pub fn vid_value(&self, vid: u64) -> Option<Item> {
        self.sv_value(vid)
            .or_else(|| self.constant(vid).map(|c| c.value.clone()))
    }

    /// Set a constant directly (the operator's way; the host's S2F15 also checks the lock).
    /// `Eac::OutOfRange` when the value is outside the range or does not fit the type.
    pub fn set_constant(&mut self, id: u64, value: f64) -> Eac {
        let Some(c) = self.constants.iter_mut().find(|c| c.id == id) else {
            return Eac::ConstantDoesNotExist;
        };
        match like(&c.value, value) {
            Some(v) if value >= c.min && value <= c.max => {
                c.value = v;
                Eac::Accepted
            }
            _ => Eac::OutOfRange,
        }
    }

    fn constant_f64(&self, id: u64) -> f64 {
        self.constant(id).map_or(0.0, EquipmentConstant::value_f64)
    }

    // ---- time ------------------------------------------------------------------------------

    /// Advance the chamber by `dt` seconds: temperature and pressure move towards their targets,
    /// a running process counts down and completes.
    pub fn advance(&mut self, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let (target_t, target_p) = match self.chamber.state {
            ProcessState::Idle => (self.chamber.ambient_c, self.chamber.atmosphere_mtorr),
            _ => (
                self.constant_f64(ecid::PROCESS_TEMPERATURE),
                self.constant_f64(ecid::PROCESS_PRESSURE),
            ),
        };
        let c = &mut self.chamber;
        c.temperature_c = approach(c.temperature_c, target_t, dt, 8.0);
        c.pressure_mtorr = approach(c.pressure_mtorr, target_p, dt, 3.0);
        if c.state == ProcessState::Executing {
            c.elapsed_s += dt;
            if c.elapsed_s >= c.duration_s {
                c.elapsed_s = c.duration_s;
                c.wafer_count += 1;
                self.set_process_state(ProcessState::Idle);
                self.fire_event(ceid::PROCESS_COMPLETED);
            }
        }
    }

    fn set_process_state(&mut self, state: ProcessState) {
        self.chamber.previous_state = self.chamber.state;
        self.chamber.state = state;
    }

    // ---- process ---------------------------------------------------------------------------

    /// Start a process with `ppid`, or the last used / first program when `None`.
    pub fn start_process(&mut self, ppid: Option<&str>) -> Result<(), ProcessError> {
        if self.chamber.state != ProcessState::Idle {
            return Err(ProcessError::WrongState);
        }
        let ppid = match ppid {
            Some(p) => p.to_string(),
            None if !self.chamber.ppid.is_empty() => self.chamber.ppid.clone(),
            None => self.ppids.first().cloned().unwrap_or_default(),
        };
        if !self.ppids.contains(&ppid) {
            return Err(ProcessError::UnknownPpid);
        }
        self.chamber.ppid = ppid;
        self.chamber.elapsed_s = 0.0;
        self.chamber.duration_s = self.constant_f64(ecid::PROCESS_TIME);
        self.set_process_state(ProcessState::Executing);
        self.fire_event(ceid::PROCESS_STARTED);
        Ok(())
    }

    /// Abort the running or paused process.
    pub fn stop_process(&mut self) -> Result<(), ProcessError> {
        if self.chamber.state == ProcessState::Idle {
            return Err(ProcessError::WrongState);
        }
        self.set_process_state(ProcessState::Idle);
        self.fire_event(ceid::PROCESS_STOPPED);
        Ok(())
    }

    pub fn pause_process(&mut self) -> Result<(), ProcessError> {
        if self.chamber.state != ProcessState::Executing {
            return Err(ProcessError::WrongState);
        }
        self.set_process_state(ProcessState::Paused);
        self.fire_event(ceid::PROCESS_PAUSED);
        Ok(())
    }

    pub fn resume_process(&mut self) -> Result<(), ProcessError> {
        if self.chamber.state != ProcessState::Paused {
            return Err(ProcessError::WrongState);
        }
        self.set_process_state(ProcessState::Executing);
        self.fire_event(ceid::PROCESS_RESUMED);
        Ok(())
    }

    // ---- alarms ----------------------------------------------------------------------------

    /// Set an alarm. Returns false when it does not exist or is already set. An enabled alarm is
    /// reported with S5F1, and the AlarmSet event fires.
    pub fn raise_alarm(&mut self, alid: u64) -> bool {
        self.change_alarm(alid, true)
    }

    /// Clear an alarm (S5F1 with the set bit off, then the AlarmCleared event).
    pub fn clear_alarm(&mut self, alid: u64) -> bool {
        self.change_alarm(alid, false)
    }

    fn change_alarm(&mut self, alid: u64, set: bool) -> bool {
        let Some(alarm) = self.alarms.iter_mut().find(|a| a.alid == alid) else {
            return false;
        };
        if alarm.set == set {
            return false;
        }
        alarm.set = set;
        let report = AlarmReport {
            alcd: alarm.alcd(),
            alid,
            altx: alarm.text.clone(),
        };
        let reportable = alarm.enabled && self.can_report();
        if reportable {
            let body = encode_s5f1(self.id_format, &report);
            self.push(SecsMessage::request(5, 1, Some(body)));
        }
        self.fire_event(if set {
            ceid::ALARM_SET
        } else {
            ceid::ALARM_CLEARED
        });
        true
    }

    // ---- control and communication state ---------------------------------------------------

    fn set_control_state(&mut self, new: ControlState) {
        if new == self.control_state {
            return;
        }
        self.previous_control_state = self.control_state;
        self.control_state = new;
        match new {
            ControlState::EquipmentOffline => self.fire_event(ceid::EQUIPMENT_OFFLINE),
            ControlState::HostOffline => self.fire_event(ceid::HOST_OFFLINE),
            ControlState::OnlineLocal => self.fire_event(ceid::ONLINE_LOCAL),
            ControlState::OnlineRemote => self.fire_event(ceid::ONLINE_REMOTE),
            ControlState::AttemptOnline => {}
        }
    }

    /// The operator takes the equipment offline.
    pub fn operator_offline(&mut self) {
        self.set_control_state(ControlState::EquipmentOffline);
    }

    /// The operator asks to go online: the equipment goes through attempt-online and lands in
    /// `online_substate` when the host is communicating, back in equipment offline otherwise.
    /// Returns whether it is online now.
    pub fn operator_online(&mut self) -> bool {
        if self.control_state != ControlState::EquipmentOffline {
            return self.control_state.is_online();
        }
        self.set_control_state(ControlState::AttemptOnline);
        if self.comm_state == CommunicationState::Communicating {
            let target = self.online_substate;
            self.set_control_state(target);
        } else {
            self.set_control_state(ControlState::EquipmentOffline);
        }
        self.control_state.is_online()
    }

    /// Switch between online local and online remote.
    pub fn operator_set_remote(&mut self, remote: bool) {
        let target = if remote {
            ControlState::OnlineRemote
        } else {
            ControlState::OnlineLocal
        };
        self.online_substate = target;
        if self.control_state.is_online() {
            self.set_control_state(target);
        }
    }

    /// S1F15: the host asks for offline. Always acknowledged; an online equipment goes host offline.
    pub fn host_request_offline(&mut self) -> OflAck {
        if self.control_state.is_online() {
            self.set_control_state(ControlState::HostOffline);
        }
        OflAck::Acknowledged
    }

    /// S1F17: the host asks for online. Only host offline can comply.
    pub fn host_request_online(&mut self) -> OnlAck {
        match self.control_state {
            ControlState::HostOffline => {
                let target = self.online_substate;
                self.set_control_state(target);
                OnlAck::Accepted
            }
            ControlState::OnlineLocal | ControlState::OnlineRemote => OnlAck::AlreadyOnline,
            ControlState::EquipmentOffline | ControlState::AttemptOnline => OnlAck::NotAllowed,
        }
    }

    /// An HSMS session was selected.
    pub fn session_started(&mut self) {
        self.comm_state = if self.comm_enabled {
            CommunicationState::WaitCra
        } else {
            CommunicationState::Disabled
        };
    }

    /// The session ended: communication is lost and queued messages are dropped.
    pub fn session_ended(&mut self) {
        self.session_started();
        self.outbox.clear();
    }

    /// S1F13/S1F14 completed.
    pub fn communication_established(&mut self) {
        if self.comm_enabled {
            self.comm_state = CommunicationState::Communicating;
        }
    }

    /// An S1F13 the equipment sent went unanswered or was refused.
    pub fn communication_failed(&mut self) {
        if self.comm_state != CommunicationState::Disabled {
            self.comm_state = CommunicationState::WaitDelay;
        }
    }

    /// The operator enables or disables communication.
    pub fn set_communication_enabled(&mut self, enabled: bool) {
        self.comm_enabled = enabled;
        self.comm_state = if enabled {
            CommunicationState::WaitCra
        } else {
            CommunicationState::Disabled
        };
    }

    // ---- events ----------------------------------------------------------------------------

    fn can_report(&self) -> bool {
        self.comm_state == CommunicationState::Communicating && self.control_state.is_online()
    }

    fn push(&mut self, message: SecsMessage) {
        if self.outbox.len() >= OUTBOX_LIMIT {
            self.outbox.remove(0);
        }
        self.outbox.push(message);
    }

    /// An event happened: queue its S6F11 when the event is enabled and the host can be told
    /// (communicating and online; control state changes are also reported on the way offline).
    pub fn fire_event(&mut self, ceid: u64) {
        let enabled = self.events.iter().any(|e| e.ceid == ceid && e.enabled);
        if !enabled
            || self.comm_state != CommunicationState::Communicating
            || !(self.control_state.is_online() || is_control_ceid(ceid))
        {
            return;
        }
        let reports = self
            .links
            .iter()
            .find(|l| l.ceid == ceid)
            .map(|l| {
                l.rptids
                    .iter()
                    .filter_map(|rptid| self.reports.iter().find(|r| r.rptid == *rptid))
                    .map(|r| ReportValues {
                        rptid: r.rptid,
                        values: r
                            .vids
                            .iter()
                            .map(|v| self.vid_value(*v).unwrap_or_else(Item::empty_list))
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let report = EventReport {
            dataid: self.next_data_id,
            ceid,
            reports,
        };
        self.next_data_id += 1;
        let body = encode_s6f11(self.id_format, &report);
        self.push(SecsMessage::request(6, 11, Some(body)));
    }

    /// Take the queued messages.
    pub fn take_outbox(&mut self) -> Vec<SecsMessage> {
        std::mem::take(&mut self.outbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn communicating() -> EquipmentModel {
        let mut m = EquipmentModel::default();
        m.communication_established();
        m
    }

    #[test]
    fn process_runs_to_completion() {
        let mut m = communicating();
        m.set_constant(ecid::PROCESS_TIME, 10.0);
        m.start_process(Some("ETCH_SIO2")).unwrap();
        assert_eq!(m.chamber.state, ProcessState::Executing);
        assert_eq!(m.start_process(None), Err(ProcessError::WrongState));
        for _ in 0..50 {
            m.advance(0.1);
        }
        assert!(m.chamber.temperature_c > 22.0);
        assert!(m.chamber.pressure_mtorr < 760_000.0);
        for _ in 0..60 {
            m.advance(0.1);
        }
        assert_eq!(m.chamber.state, ProcessState::Idle);
        assert_eq!(m.chamber.wafer_count, 1);
        assert_eq!(m.chamber.previous_state, ProcessState::Executing);
        // idle again: the chamber heads back to ambient and atmosphere
        for _ in 0..4000 {
            m.advance(0.1);
        }
        assert_eq!(m.chamber.temperature_c, 22.0);
        assert_eq!(m.chamber.pressure_mtorr, 760_000.0);
    }

    #[test]
    fn pause_resume_stop() {
        let mut m = communicating();
        assert_eq!(m.stop_process(), Err(ProcessError::WrongState));
        assert_eq!(m.pause_process(), Err(ProcessError::WrongState));
        assert_eq!(
            m.start_process(Some("nope")),
            Err(ProcessError::UnknownPpid)
        );
        m.start_process(None).unwrap();
        assert_eq!(m.chamber.ppid, "WAFER_CLEAN_A");
        m.advance(5.0);
        m.pause_process().unwrap();
        let held = m.chamber.elapsed_s;
        m.advance(5.0);
        assert_eq!(m.chamber.elapsed_s, held);
        assert_eq!(m.resume_process(), Ok(()));
        m.stop_process().unwrap();
        assert_eq!(m.chamber.state, ProcessState::Idle);
        assert_eq!(m.chamber.wafer_count, 0);
    }

    #[test]
    fn events_follow_links_and_enable() {
        let mut m = communicating();
        m.reports.push(ReportDef {
            rptid: 5,
            vids: vec![
                svid::WAFER_COUNT,
                svid::PROCESS_STATE,
                ecid::PROCESS_TIME,
                9999,
            ],
        });
        m.links.push(EventLink {
            ceid: ceid::PROCESS_STARTED,
            rptids: vec![5],
        });
        // disabled: nothing queued
        m.start_process(None).unwrap();
        assert!(m.take_outbox().is_empty());
        m.stop_process().unwrap();

        m.events
            .iter_mut()
            .find(|e| e.ceid == ceid::PROCESS_STARTED)
            .unwrap()
            .enabled = true;
        m.start_process(None).unwrap();
        let out = m.take_outbox();
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].to_sml(),
            "S6F11 W <L [3] <U4 1> <U4 100> <L [1] <L [2] <U4 5> <L [4] <U4 0> <U1 1> <U4 60> <L [0]>>>>>."
        );
        // not communicating: dropped
        m.stop_process().unwrap();
        m.session_ended();
        m.start_process(None).unwrap();
        assert!(m.take_outbox().is_empty());
    }

    #[test]
    fn alarms_report_and_clear() {
        let mut m = communicating();
        assert!(m.raise_alarm(alid::DOOR_OPEN));
        assert!(!m.raise_alarm(alid::DOOR_OPEN));
        assert!(!m.raise_alarm(77));
        let out = m.take_outbox();
        assert_eq!(
            out[0].to_sml(),
            r#"S5F1 W <L [3] <B 0x81> <U4 2> <A "Chamber door open">>."#
        );
        assert_eq!(m.sv_value(svid::ALARMS_SET), Some(Item::U4(vec![2])));
        assert!(m.clear_alarm(alid::DOOR_OPEN));
        let out = m.take_outbox();
        assert_eq!(
            out[0].to_sml(),
            r#"S5F1 W <L [3] <B 0x01> <U4 2> <A "Chamber door open">>."#
        );
        // a disabled alarm sets silently
        m.alarms[0].enabled = false;
        assert!(m.raise_alarm(alid::OVER_TEMPERATURE));
        assert!(m.take_outbox().is_empty());
    }

    #[test]
    fn control_state_machine() {
        let mut m = EquipmentModel::default();
        assert_eq!(m.control_state, ControlState::OnlineRemote);
        assert_eq!(m.host_request_online(), OnlAck::AlreadyOnline);
        assert_eq!(m.host_request_offline(), OflAck::Acknowledged);
        assert_eq!(m.control_state, ControlState::HostOffline);
        assert_eq!(m.previous_control_state, ControlState::OnlineRemote);
        assert_eq!(m.host_request_online(), OnlAck::Accepted);
        assert_eq!(m.control_state, ControlState::OnlineRemote);
        m.operator_set_remote(false);
        assert_eq!(m.control_state, ControlState::OnlineLocal);
        m.operator_offline();
        assert_eq!(m.control_state, ControlState::EquipmentOffline);
        assert_eq!(m.host_request_online(), OnlAck::NotAllowed);
        // no host communicating: attempt online falls back to offline
        assert!(!m.operator_online());
        assert_eq!(m.control_state, ControlState::EquipmentOffline);
        m.communication_established();
        assert!(m.operator_online());
        assert_eq!(m.control_state, ControlState::OnlineLocal);
        m.operator_set_remote(true);
        assert_eq!(m.control_state, ControlState::OnlineRemote);
    }

    #[test]
    fn constants_check_range_and_type() {
        let mut m = EquipmentModel::default();
        assert_eq!(m.set_constant(ecid::PROCESS_TIME, 90.0), Eac::Accepted);
        assert_eq!(m.constant(ecid::PROCESS_TIME).unwrap().value, Item::u4(90));
        assert_eq!(m.set_constant(ecid::PROCESS_TIME, 0.0), Eac::OutOfRange);
        assert_eq!(m.set_constant(ecid::PROCESS_TIME, 1.5), Eac::OutOfRange);
        assert_eq!(m.set_constant(42, 1.0), Eac::ConstantDoesNotExist);
        let infos = m.constant_infos();
        assert_eq!(infos[1].min, Item::u4(1));
        assert_eq!(infos[0].max, Item::f4(400.0));
    }

    #[test]
    fn timestamp_has_sixteen_digits() {
        let t = timestamp();
        assert_eq!(t.len(), 16);
        assert!(t.chars().all(|c| c.is_ascii_digit()));
        assert!(t.starts_with("20"));
    }

    #[test]
    fn model_serializes() {
        let m = EquipmentModel::default();
        let json = serde_json::to_string(&m).unwrap();
        let back: EquipmentModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
