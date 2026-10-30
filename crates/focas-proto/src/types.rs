//! Typed results of the client calls. All of them are serde-serializable.

use serde::{Deserialize, Serialize};

use crate::error::Reading;

/// Declare an enum over a numeric code, with an `Unknown` variant for codes outside the table.
///
/// `code()` and `from_code()` round-trip, `label()` is the short name FANUC's documentation
/// (and the CNC's screen) uses.
macro_rules! code_enum {
    (
        $(#[$meta:meta])*
        $name:ident($repr:ty) {
            $($variant:ident = $code:expr => $label:expr),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant,)+
            /// A code this crate has no name for.
            Unknown($repr),
        }

        impl $name {
            pub fn from_code(code: $repr) -> Self {
                match code {
                    $($code => $name::$variant,)+
                    other => $name::Unknown(other),
                }
            }

            pub fn code(self) -> $repr {
                match self {
                    $($name::$variant => $code,)+
                    $name::Unknown(c) => c,
                }
            }

            /// Short FOCAS name, e.g. `MEM` or `STRT`; `?` for an unknown code.
            pub fn label(self) -> &'static str {
                match self {
                    $($name::$variant => $label,)+
                    $name::Unknown(_) => "?",
                }
            }
        }
    };
}

code_enum! {
    /// `ODBST.aut`: operating mode (series 16/18/21, 0i, 30i/31i/32i table).
    AutoMode(u16) {
        Mdi = 0 => "MDI",
        Memory = 1 => "MEM",
        NoSelection = 2 => "****",
        Edit = 3 => "EDIT",
        Handle = 4 => "HND",
        Jog = 5 => "JOG",
        TeachInJog = 6 => "T-JOG",
        TeachInHandle = 7 => "T-HND",
        IncrementalFeed = 8 => "INC",
        Reference = 9 => "REF",
        Remote = 10 => "RMT",
        Test = 11 => "TEST",
    }
}

code_enum! {
    /// `ODBST.run`: status of automatic operation.
    RunState(u16) {
        Reset = 0 => "****",
        Stop = 1 => "STOP",
        Hold = 2 => "HOLD",
        Start = 3 => "STRT",
        Mstr = 4 => "MSTR",
    }
}

code_enum! {
    /// `ODBST.motion`: axis movement / dwell.
    MotionState(u16) {
        None = 0 => "***",
        Motion = 1 => "MTN",
        Dwell = 2 => "DWL",
    }
}

code_enum! {
    /// `ODBST.mstb`: M, S, T, B function status.
    MstbState(u16) {
        None = 0 => "***",
        Fin = 1 => "FIN",
    }
}

code_enum! {
    /// `ODBST.emergency`.
    EmergencyState(u16) {
        Normal = 0 => "***",
        Emergency = 1 => "EMG",
        Reset = 2 => "RST",
        Wait = 3 => "WAIT",
    }
}

code_enum! {
    /// `ODBST.alarm`.
    AlarmState(u16) {
        None = 0 => "***",
        Alarm = 1 => "ALM",
        BatteryLow = 2 => "BAT",
        Fan = 3 => "FAN",
        PsWarning = 4 => "PSW",
        FsbWarning = 5 => "FSB",
        InsulateWarning = 6 => "INS",
        EncoderWarning = 7 => "ENC",
        PmcAlarm = 8 => "PMC",
    }
}

code_enum! {
    /// `ODBST.edit`: status of program editing (the codes shared by M and T series).
    EditState(u16) {
        NotEditing = 0 => "****",
        Editing = 1 => "EDIT",
        Searching = 2 => "SRCH",
        Output = 3 => "OUT",
        Input = 4 => "IN",
        Compare = 5 => "CMP",
    }
}

code_enum! {
    /// Alarm type of `cnc_rdalmmsg` (series 30i, 0i-D/F table). The same numbers are the bit
    /// positions of the `cnc_alarm` bit field.
    AlarmType(i32) {
        ParameterSwitchOn = 0 => "SW",
        PowerOffParameter = 1 => "PW",
        IoError = 2 => "IO",
        ForegroundPs = 3 => "PS",
        Overtravel = 4 => "OT",
        Overheat = 5 => "OH",
        Servo = 6 => "SV",
        DataIoError = 7 => "SR",
        Macro = 8 => "MC",
        Spindle = 9 => "SP",
        OtherAlarm = 10 => "DS",
        MalfunctionPrevention = 11 => "IE",
        BackgroundPs = 12 => "BG",
        Synchronized = 13 => "SN",
        ExternalMessage = 15 => "EX",
        Pmc = 19 => "PC",
    }
}

/// The seven status words of `cnc_statinfo` as the reference decodes them
/// (`aut, run, motion, mstb, emergency, alarm, edit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatInfo {
    pub aut: AutoMode,
    pub run: RunState,
    pub motion: MotionState,
    pub mstb: MstbState,
    pub emergency: EmergencyState,
    pub alarm: AlarmState,
    pub edit: EditState,
}

/// `cnc_sysinfo` (`ODBSYS`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SysInfo {
    /// Additional information bit field (bit 1: "i" series CNC, bit 3: compound machining, ...).
    pub addinfo: u16,
    /// Maximum controlled axes.
    pub max_axis: u16,
    /// Kind of CNC, ASCII, e.g. `31` or `0` (padding trimmed).
    pub cnc_type: String,
    /// Machine kind, ASCII: `M` mill, `T` lathe, `TT` (padding trimmed).
    pub mt_type: String,
    /// CNC software series, e.g. `G401`.
    pub series: String,
    /// CNC software version.
    pub version: String,
    /// Currently controlled axes as the CNC prints them.
    pub axes: String,
}

impl SysInfo {
    /// Currently controlled axes as a number, if the CNC printed one.
    pub fn axis_count(&self) -> Option<u16> {
        self.axes.trim().parse().ok()
    }

    /// `addinfo` bit 1: an "i" series CNC.
    pub fn is_i_series(&self) -> bool {
        self.addinfo & 0x0002 != 0
    }
}

/// Axis name and optional suffix (`X`, `Y`, `Z`, `B`, `X1`...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisName {
    pub name: char,
    pub suffix: Option<char>,
}

impl AxisName {
    pub fn new(name: char) -> Self {
        AxisName { name, suffix: None }
    }
}

impl std::fmt::Display for AxisName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.suffix {
            Some(s) => write!(f, "{}{}", self.name, s),
            None => write!(f, "{}", self.name),
        }
    }
}

/// Which position `cnc_rdposition`-style call (function `0x26`) reads. The numbers are the
/// request codes of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionKind {
    Absolute,
    /// pyfanuc `REF`: machine coordinates.
    Machine,
    Relative,
    DistanceToGo,
    /// pyfanuc `SKIP`.
    Skip,
}

impl PositionKind {
    /// Argument of the request on the wire.
    pub fn code(self) -> i32 {
        match self {
            PositionKind::Machine => 1,
            PositionKind::Absolute => 4,
            PositionKind::Relative => 6,
            PositionKind::DistanceToGo => 7,
            PositionKind::Skip => 8,
        }
    }

    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            1 => PositionKind::Machine,
            4 => PositionKind::Absolute,
            6 => PositionKind::Relative,
            7 => PositionKind::DistanceToGo,
            8 => PositionKind::Skip,
            _ => return None,
        })
    }
}

/// Axis positions, one value per controlled axis in axis-name order. `None` marks an axis the
/// CNC flagged invalid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisPositions {
    pub axes: Reading<Vec<AxisName>>,
    pub absolute: Reading<Vec<Option<f64>>>,
    pub machine: Reading<Vec<Option<f64>>>,
    pub relative: Reading<Vec<Option<f64>>>,
    pub distance_to_go: Reading<Vec<Option<f64>>>,
}

/// Running and main program numbers (`O` numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramNumbers {
    pub running: i32,
    pub main: i32,
}

/// `cnc_alarm` bit field. Which alarm each bit means depends on the CNC series; the names here
/// are those of the series 30i, 0i-D/F table (same as [`AlarmType`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmBits(pub u32);

impl AlarmBits {
    pub fn any(self) -> bool {
        self.0 != 0
    }

    pub fn is_set(self, bit: u32) -> bool {
        bit < 32 && self.0 & (1 << bit) != 0
    }

    /// Alarm types whose bit is set and that have a name in the 30i table.
    pub fn types(self) -> Vec<AlarmType> {
        (0..32)
            .filter(|b| self.is_set(*b))
            .map(|b| AlarmType::from_code(b as i32))
            .filter(|t| !matches!(t, AlarmType::Unknown(_)))
            .collect()
    }
}

/// How to ask for alarm messages (`cnc_rdalmmsg`, function `0x23`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmQuery {
    /// Alarm type to read; -1 reads all types (FOCAS documentation).
    pub alarm_type: i32,
    /// Most messages to return.
    pub max_messages: u16,
    /// Ask for the message text as well as the number.
    pub with_text: bool,
    /// Bytes reserved per message text (32 in `ODBALMMSG`).
    pub text_length: u16,
}

impl Default for AlarmQuery {
    fn default() -> Self {
        AlarmQuery {
            alarm_type: -1,
            max_messages: 10,
            with_text: true,
            text_length: 32,
        }
    }
}

/// One alarm of `cnc_rdalmmsg`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmMessage {
    /// Alarm number, e.g. 100 for P/S 100.
    pub number: i32,
    pub alarm_type: AlarmType,
    /// Axis number (1-based); 0 for an alarm that is not tied to an axis.
    pub axis: i32,
    /// Message text; `None` when it was not asked for or the CNC sent none.
    pub text: Option<String>,
}

/// One macro variable. `None` when the CNC flags the value empty/invalid (a vacant `#` variable).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MacroValue {
    pub number: u32,
    pub value: Option<f64>,
}

/// A parameter or diagnosis number with its value per axis.
///
/// A non-axis item has one value. For bit items the value is the byte; bit 0 is the least
/// significant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberedValues {
    pub number: u32,
    /// Axis the CNC reports: -1 for all axes, 0 for a non-axis item, else the 1-based axis.
    pub axis: i16,
    /// The CNC's storage type code. Parameters: 0 byte, 1 bit, 2 short, 3 int. Diagnosis values:
    /// 0 or 4 byte, 1 short, 2 int, 3 bit (pyfanuc `readparam` / `readdiag`).
    pub data_type: u16,
    pub values: Vec<i32>,
}

/// PMC data width (pyfanuc `readpmc` datatype).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmcDataType {
    Byte,
    Word,
    DWord,
}

impl PmcDataType {
    pub fn code(self) -> i32 {
        match self {
            PmcDataType::Byte => 0,
            PmcDataType::Word => 1,
            PmcDataType::DWord => 2,
        }
    }

    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => PmcDataType::Byte,
            1 => PmcDataType::Word,
            2 => PmcDataType::DWord,
            _ => return None,
        })
    }

    /// Bytes per value.
    pub fn size(self) -> usize {
        1 << self.code()
    }
}

/// PMC memory type (README "Memory Type" table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmcArea {
    /// G: PMC to CNC.
    G,
    /// F: CNC to PMC.
    F,
    /// Y: PMC to machine.
    Y,
    /// X: machine to PMC.
    X,
    /// A: message display.
    A,
    /// R: internal relays.
    R,
    /// T: timers.
    T,
    /// K: keep relays.
    K,
    /// C: counters.
    C,
    /// D: data table.
    D,
}

impl PmcArea {
    pub const ALL: [PmcArea; 10] = [
        PmcArea::G,
        PmcArea::F,
        PmcArea::Y,
        PmcArea::X,
        PmcArea::A,
        PmcArea::R,
        PmcArea::T,
        PmcArea::K,
        PmcArea::C,
        PmcArea::D,
    ];

    pub fn code(self) -> i32 {
        self as i32
    }

    pub fn from_code(code: i32) -> Option<Self> {
        usize::try_from(code)
            .ok()
            .and_then(|i| PmcArea::ALL.get(i).copied())
    }

    pub fn letter(self) -> char {
        match self {
            PmcArea::G => 'G',
            PmcArea::F => 'F',
            PmcArea::Y => 'Y',
            PmcArea::X => 'X',
            PmcArea::A => 'A',
            PmcArea::R => 'R',
            PmcArea::T => 'T',
            PmcArea::K => 'K',
            PmcArea::C => 'C',
            PmcArea::D => 'D',
        }
    }
}

/// A PMC value at its byte address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmcValue {
    pub address: u32,
    pub value: u32,
}

/// The block the CNC is executing (`readexecprog`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecBlock {
    /// Value of the first word of the payload; the reference calls it "block".
    pub block: i32,
    pub text: String,
}

/// One entry of the program directory (function `0x06`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramEntry {
    pub number: u32,
    /// Size in bytes.
    pub size: u32,
    pub comment: String,
}

/// Everything a status page polls, fetched in as few packets as the reference's multi-request
/// allows. Items the CNC refuses stay `Failed` and do not affect the others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub status: Reading<StatInfo>,
    pub positions: AxisPositions,
    /// Actual feed rate; `None` when the CNC flags it invalid.
    pub feed_rate: Reading<Option<f64>>,
    /// Actual spindle speed; `None` when the CNC flags it invalid.
    pub spindle_speed: Reading<Option<f64>>,
    pub programs: Reading<ProgramNumbers>,
    pub sequence_number: Reading<i32>,
    pub alarm_bits: Reading<AlarmBits>,
}

/// Optional reads of [`FocasClient::extended_snapshot`](crate::FocasClient::extended_snapshot),
/// on top of what [`Snapshot`] carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraQuery {
    /// Servo load meter of every axis slot (function `0x56`).
    pub servo_load: bool,
    /// Main program name with path (function `0xb9`, series 30i and later).
    pub program_name: bool,
    /// Macro variables to read, one sub-packet each so that a refused one does not hide the
    /// others. Numbers start at 1.
    pub macros: Vec<u32>,
}

/// Result of [`FocasClient::extended_snapshot`](crate::FocasClient::extended_snapshot). A field
/// is `None` when it was not asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraSnapshot {
    /// One value per axis slot of the payload (32 in the reference sample); keep the first
    /// `n` for `n` controlled axes.
    pub servo_load: Option<Reading<Vec<Option<f64>>>>,
    pub program_name: Option<Reading<String>>,
    /// One entry per requested macro number, in request order.
    pub macros: Vec<Reading<MacroValue>>,
}
