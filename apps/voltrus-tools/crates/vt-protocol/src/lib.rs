//! The boundary between the app and the session engine.
//!
//! The app never touches a protocol crate: it sends [`Command`]s and draws the
//! [`Event`]s that come back. Every type here is plain serde data, so the engine
//! can move out of process later (a background server the app attaches to, as
//! in muxy) without changing either side's model.

mod bacnet;
mod message;
mod modbus;
mod value;

pub use bacnet::{
    BACNET_PORT, BacnetClientSpec, BacnetDeviceInfo, BacnetDeviceSpec, BacnetDiscoverSpec,
};
pub use message::{Command, Event, EventKind, LogEntry, LogLevel, SessionId, SessionState, Stats};
pub use modbus::{
    Area, DataType, ModbusLink, ModbusMasterSpec, ModbusSlaveSpec, Parity, RegisterBlock,
    SerialLink, WordOrder,
};
pub use value::{Direction, PointInfo, Quality, Reading, TrafficFrame, ValueFrame};

use serde::{Deserialize, Serialize};

/// What a session does. One variant per protocol role the engine can run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionSpec {
    /// Poll a Modbus device (TCP or RTU) and write to it.
    ModbusMaster(ModbusMasterSpec),
    /// Serve a Modbus register image, optionally driven by a device template.
    ModbusSlave(ModbusSlaveSpec),
    /// Read, subscribe to and command a BACnet/IP device's objects.
    BacnetClient(BacnetClientSpec),
    /// Play a BACnet/IP device with a plant's simulated objects.
    BacnetDevice(BacnetDeviceSpec),
}

impl SessionSpec {
    pub fn protocol(&self) -> Protocol {
        match self {
            Self::ModbusMaster(_) | Self::ModbusSlave(_) => Protocol::Modbus,
            Self::BacnetClient(_) | Self::BacnetDevice(_) => Protocol::Bacnet,
        }
    }

    pub fn role(&self) -> Role {
        match self {
            Self::ModbusMaster(_) | Self::BacnetClient(_) => Role::Client,
            Self::ModbusSlave(_) | Self::BacnetDevice(_) => Role::Simulator,
        }
    }

    /// Where the session talks to, as shown next to its name.
    pub fn endpoint(&self) -> String {
        match self {
            Self::ModbusMaster(spec) => format!("{} · unit {}", spec.link, spec.unit_id),
            Self::ModbusSlave(spec) => {
                format!("{}:{} · unit {}", spec.bind, spec.port, spec.unit_id)
            }
            Self::BacnetClient(spec) => match spec.device_instance {
                Some(instance) => format!("{}:{} · device {instance}", spec.host, spec.port),
                None => format!("{}:{}", spec.host, spec.port),
            },
            Self::BacnetDevice(spec) => {
                format!("0.0.0.0:{} · device {}", spec.port, spec.device_instance)
            }
        }
    }

    /// Just the address (host and port, or serial port), for tight spaces.
    pub fn address(&self) -> String {
        match self {
            Self::ModbusMaster(spec) => spec.link.to_string(),
            Self::ModbusSlave(spec) => format!("{}:{}", spec.bind, spec.port),
            Self::BacnetClient(spec) => format!("{}:{}", spec.host, spec.port),
            Self::BacnetDevice(spec) => format!("0.0.0.0:{}", spec.port),
        }
    }
}

/// The protocols the suite speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Modbus,
    Bacnet,
}

impl Protocol {
    pub const ALL: &'static [Protocol] = &[Protocol::Modbus, Protocol::Bacnet];

    pub fn name(self) -> &'static str {
        match self {
            Self::Modbus => "Modbus",
            Self::Bacnet => "BACnet",
        }
    }

    /// Two-letter monogram for compact badges.
    pub fn monogram(self) -> &'static str {
        match self {
            Self::Modbus => "MB",
            Self::Bacnet => "BA",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Modbus => "TCP and RTU master, register blocks, writes, and a slave simulator",
            Self::Bacnet => {
                "Who-Is discovery, object list, COV and priority writes, and a device simulator"
            }
        }
    }
}

/// Whether a session reaches out to a device or plays one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Client,
    Simulator,
}
