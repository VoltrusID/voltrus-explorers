use serde::{Deserialize, Serialize};

/// The registered DNP3 TCP port (IEEE 1815 Annex).
pub const DNP3_PORT: u16 = 20000;

/// Highest link address a station may have: 0xFFF0 and above are reserved
/// (0xFFFD-0xFFFF broadcast, the rest for future use).
pub const DNP3_MAX_ADDRESS: u16 = 0xFFEF;

/// Highest point index a control can address (the one-octet index prefix of
/// qualifier 0x17).
pub const DNP3_MAX_CONTROL_INDEX: u16 = 255;

/// The static object groups a DNP3 point belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dnp3PointKind {
    BinaryInput,
    DoubleBitInput,
    BinaryOutput,
    Counter,
    FrozenCounter,
    AnalogInput,
    AnalogOutput,
}

impl Dnp3PointKind {
    /// In the order a master lists them: inputs, then outputs, then counters.
    pub const ALL: [Dnp3PointKind; 7] = [
        Self::BinaryInput,
        Self::DoubleBitInput,
        Self::AnalogInput,
        Self::Counter,
        Self::FrozenCounter,
        Self::BinaryOutput,
        Self::AnalogOutput,
    ];

    /// The static group: 1, 3, 10, 20, 21, 30 or 40.
    pub fn group(self) -> u8 {
        match self {
            Self::BinaryInput => 1,
            Self::DoubleBitInput => 3,
            Self::BinaryOutput => 10,
            Self::Counter => 20,
            Self::FrozenCounter => 21,
            Self::AnalogInput => 30,
            Self::AnalogOutput => 40,
        }
    }

    pub fn from_group(group: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.group() == group)
    }

    /// Engineers' short form, e.g. `AI`, `BO`.
    pub fn short(self) -> &'static str {
        match self {
            Self::BinaryInput => "BI",
            Self::DoubleBitInput => "DBI",
            Self::BinaryOutput => "BO",
            Self::Counter => "CTR",
            Self::FrozenCounter => "FCTR",
            Self::AnalogInput => "AI",
            Self::AnalogOutput => "AO",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BinaryInput => "Binary input",
            Self::DoubleBitInput => "Double-bit input",
            Self::BinaryOutput => "Binary output",
            Self::Counter => "Counter",
            Self::FrozenCounter => "Frozen counter",
            Self::AnalogInput => "Analog input",
            Self::AnalogOutput => "Analog output",
        }
    }

    /// Binary outputs take CROBs (g12), analog outputs analog output blocks (g41).
    pub fn writable(self) -> bool {
        matches!(self, Self::BinaryOutput | Self::AnalogOutput)
    }

    pub fn is_binary(self) -> bool {
        matches!(
            self,
            Self::BinaryInput | Self::DoubleBitInput | Self::BinaryOutput
        )
    }
}

/// A name (and unit) for one point of the outstation. Points the outstation
/// reports without a name get one from their type and index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dnp3PointName {
    pub kind: Dnp3PointKind,
    pub index: u16,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

/// How a master commands an output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dnp3ControlMode {
    /// SELECT, then OPERATE with the same object: the outstation arms the
    /// point before acting, as utilities require for breakers.
    SelectBeforeOperate,
    /// DIRECT OPERATE: one request.
    DirectOperate,
}

impl Dnp3ControlMode {
    pub const ALL: [Dnp3ControlMode; 2] = [Self::SelectBeforeOperate, Self::DirectOperate];

    pub fn name(self) -> &'static str {
        match self {
            Self::SelectBeforeOperate => "Select before operate",
            Self::DirectOperate => "Direct operate",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::SelectBeforeOperate => "SBO",
            Self::DirectOperate => "Direct",
        }
    }
}

/// Poll a DNP3 outstation over TCP: integrity and event-class polls, flags,
/// controls and time synchronisation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dnp3MasterSpec {
    pub host: String,
    pub port: u16,
    /// This master's link address (the source of its frames).
    pub master_address: u16,
    /// The outstation's link address.
    pub outstation_address: u16,
    /// Integrity poll (class 0, 1, 2, 3) period; also run on connect, after
    /// a restart and after an event buffer overflow.
    pub integrity_ms: u64,
    /// Event poll (class 1, 2, 3) period; 0 polls by integrity only.
    pub event_ms: u64,
    /// How long to wait for each response.
    pub timeout_ms: u64,
    pub control_mode: Dnp3ControlMode,
    /// Write the time whenever the outstation asks for it (IIN1.4 NEED_TIME).
    pub time_sync: bool,
    /// Names for points; every other point the outstation reports is listed too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<Dnp3PointName>,
}

impl Default for Dnp3MasterSpec {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: DNP3_PORT,
            master_address: 1,
            outstation_address: 10,
            integrity_ms: 60_000,
            event_ms: 1000,
            timeout_ms: 2000,
            control_mode: Dnp3ControlMode::SelectBeforeOperate,
            time_sync: true,
            points: Vec::new(),
        }
    }
}

impl Dnp3MasterSpec {
    /// How often values arrive: the event poll, or the integrity poll alone.
    pub fn sample_ms(&self) -> u64 {
        if self.event_ms == 0 {
            self.integrity_ms
        } else {
            self.event_ms
        }
    }
}

/// Play a DNP3 outstation on this computer: a plant's points with simulated
/// values, events, flags and outputs any master can command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dnp3OutstationSpec {
    /// Address to listen on: `0.0.0.0` for every interface.
    pub bind: String,
    pub port: u16,
    pub outstation_address: u16,
    /// The master's link address the outstation answers by default.
    pub master_address: u16,
    /// Built-in plant template id.
    pub template: String,
}

impl Default for Dnp3OutstationSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: DNP3_PORT,
            outstation_address: 10,
            master_address: 1,
            template: "feeder".into(),
        }
    }
}

/// The operation of a control relay output block (g12v1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dnp3CrobCode {
    LatchOn,
    LatchOff,
    PulseOn,
    PulseOff,
    /// Trip/close code TRIP with a pulse.
    Trip,
    /// Trip/close code CLOSE with a pulse.
    Close,
}

impl Dnp3CrobCode {
    pub const ALL: [Dnp3CrobCode; 6] = [
        Self::LatchOn,
        Self::LatchOff,
        Self::PulseOn,
        Self::PulseOff,
        Self::Trip,
        Self::Close,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::LatchOn => "Latch on",
            Self::LatchOff => "Latch off",
            Self::PulseOn => "Pulse on",
            Self::PulseOff => "Pulse off",
            Self::Trip => "Trip",
            Self::Close => "Close",
        }
    }

    /// The control code octet (IEEE 1815-2012 Table 11-15).
    pub fn code(self) -> u8 {
        match self {
            Self::PulseOn => 0x01,
            Self::PulseOff => 0x02,
            Self::LatchOn => 0x03,
            Self::LatchOff => 0x04,
            Self::Close => 0x41,
            Self::Trip => 0x81,
        }
    }

    pub fn is_pulse(self) -> bool {
        !matches!(self, Self::LatchOn | Self::LatchOff)
    }
}

/// A control relay output block: the operation, how many times, and the
/// pulse's on and off times.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dnp3Crob {
    pub code: Dnp3CrobCode,
    pub count: u8,
    pub on_ms: u32,
    pub off_ms: u32,
}

impl Dnp3Crob {
    /// One operation: a 1 s pulse for pulse codes, none for latches.
    pub fn new(code: Dnp3CrobCode) -> Self {
        Self {
            code,
            count: 1,
            on_ms: if code.is_pulse() { 1000 } else { 0 },
            off_ms: 0,
        }
    }
}

/// What a control does to an output.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "object", rename_all = "snake_case")]
pub enum Dnp3Operation {
    /// A CROB (g12v1) to a binary output.
    Crob(Dnp3Crob),
    /// An analog output block (g41v3, float32) to an analog output.
    Analog { value: f64 },
}

/// DNP3 station commands, answered by [`crate::EventKind::Dnp3Done`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Dnp3Action {
    IntegrityPoll,
    /// Poll one event class (1, 2 or 3).
    ClassPoll {
        class: u8,
    },
    TimeSync,
    FreezeCounters,
    ColdRestart,
    WarmRestart,
    /// Command an output point; also answered by [`crate::EventKind::Written`].
    Operate {
        point: u32,
        operation: Dnp3Operation,
        mode: Dnp3ControlMode,
    },
    /// Outstation simulators: take a point offline (COMM_LOST) or back online.
    SetOnline {
        point: u32,
        online: bool,
    },
}

impl Dnp3Action {
    pub fn label(&self) -> String {
        match self {
            Self::IntegrityPoll => "Integrity poll".into(),
            Self::ClassPoll { class } => format!("Class {class} poll"),
            Self::TimeSync => "Time sync".into(),
            Self::FreezeCounters => "Counter freeze".into(),
            Self::ColdRestart => "Cold restart".into(),
            Self::WarmRestart => "Warm restart".into(),
            Self::Operate {
                operation: Dnp3Operation::Crob(crob),
                mode,
                ..
            } => format!("{} ({})", crob.code.name(), mode.short()),
            Self::Operate {
                operation: Dnp3Operation::Analog { value },
                mode,
                ..
            } => format!("Set {value} ({})", mode.short()),
            Self::SetOnline { online: true, .. } => "Bring online".into(),
            Self::SetOnline { online: false, .. } => "Take offline".into(),
        }
    }
}

/// An outstation's internal indications, as its last response carried them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dnp3Iin {
    pub iin1: u8,
    pub iin2: u8,
}

/// One internal indication: its name and whether it reports a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dnp3IinBit {
    pub name: &'static str,
    pub problem: bool,
}

const IIN1: [(&str, bool); 8] = [
    ("ALL STATIONS", false),
    ("CLASS 1 EVENTS", false),
    ("CLASS 2 EVENTS", false),
    ("CLASS 3 EVENTS", false),
    ("NEED TIME", true),
    ("LOCAL CONTROL", true),
    ("DEVICE TROUBLE", true),
    ("DEVICE RESTART", true),
];

const IIN2: [(&str, bool); 6] = [
    ("NO FUNC CODE SUPPORT", true),
    ("OBJECT UNKNOWN", true),
    ("PARAMETER ERROR", true),
    ("EVENT BUFFER OVERFLOW", true),
    ("ALREADY EXECUTING", true),
    ("CONFIG CORRUPT", true),
];

impl Dnp3Iin {
    /// The indications that are set, IIN1 first.
    pub fn set(self) -> Vec<Dnp3IinBit> {
        let mut bits = Vec::new();
        for (bit, (name, problem)) in IIN1.iter().enumerate() {
            if self.iin1 & (1 << bit) != 0 {
                bits.push(Dnp3IinBit {
                    name,
                    problem: *problem,
                });
            }
        }
        for (bit, (name, problem)) in IIN2.iter().enumerate() {
            if self.iin2 & (1 << bit) != 0 {
                bits.push(Dnp3IinBit {
                    name,
                    problem: *problem,
                });
            }
        }
        bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_kinds_map_to_their_static_groups() {
        for kind in Dnp3PointKind::ALL {
            assert_eq!(Dnp3PointKind::from_group(kind.group()), Some(kind));
        }
        // Event groups are not point kinds of their own.
        assert_eq!(Dnp3PointKind::from_group(32), None);
        assert_eq!(Dnp3PointKind::from_group(2), None);
        let writable: Vec<_> = Dnp3PointKind::ALL
            .into_iter()
            .filter(|kind| kind.writable())
            .collect();
        assert_eq!(
            writable,
            [Dnp3PointKind::BinaryOutput, Dnp3PointKind::AnalogOutput]
        );
    }

    #[test]
    fn crob_codes_follow_the_standard() {
        assert_eq!(Dnp3CrobCode::LatchOn.code(), 0x03);
        assert_eq!(Dnp3CrobCode::LatchOff.code(), 0x04);
        assert_eq!(Dnp3CrobCode::PulseOn.code(), 0x01);
        // Trip/close in bits 7 and 6, with a pulse.
        assert_eq!(Dnp3CrobCode::Trip.code(), 0x81);
        assert_eq!(Dnp3CrobCode::Close.code(), 0x41);
        assert_eq!(Dnp3Crob::new(Dnp3CrobCode::LatchOn).on_ms, 0);
        assert_eq!(Dnp3Crob::new(Dnp3CrobCode::Trip).on_ms, 1000);
    }

    #[test]
    fn internal_indications_are_named_in_bit_order() {
        let iin = Dnp3Iin {
            iin1: 0x92,
            iin2: 0x08,
        };
        let names: Vec<_> = iin.set().iter().map(|bit| bit.name).collect();
        assert_eq!(
            names,
            [
                "CLASS 1 EVENTS",
                "NEED TIME",
                "DEVICE RESTART",
                "EVENT BUFFER OVERFLOW"
            ]
        );
        assert!(!iin.set()[0].problem && iin.set()[1].problem);
        assert!(Dnp3Iin::default().set().is_empty());
        // Reserved IIN2 bits 6 and 7 name nothing.
        assert!(
            Dnp3Iin {
                iin1: 0,
                iin2: 0xC0
            }
            .set()
            .is_empty()
        );
    }

    #[test]
    fn sampling_follows_the_event_poll() {
        let spec = Dnp3MasterSpec::default();
        assert_eq!(spec.sample_ms(), 1000);
        let integrity_only = Dnp3MasterSpec {
            event_ms: 0,
            ..spec
        };
        assert_eq!(integrity_only.sample_ms(), 60_000);
    }
}
