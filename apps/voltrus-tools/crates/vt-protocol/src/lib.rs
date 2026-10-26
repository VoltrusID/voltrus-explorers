//! The boundary between the app and the session engine.
//!
//! The app never touches a protocol crate: it sends [`Command`]s and draws the
//! [`Event`]s that come back. Every type here is plain serde data, so the engine
//! can move out of process later (a background server the app attaches to, as
//! in muxy) without changing either side's model.

mod bacnet;
mod dnp3;
mod message;
mod modbus;
mod mqtt;
mod serial;
mod value;

pub use bacnet::{
    BACNET_PORT, BacnetClientSpec, BacnetDeviceInfo, BacnetDeviceSpec, BacnetDiscoverSpec,
};
pub use dnp3::{
    DNP3_MAX_ADDRESS, DNP3_MAX_CONTROL_INDEX, DNP3_PORT, Dnp3Action, Dnp3ControlMode, Dnp3Crob,
    Dnp3CrobCode, Dnp3Iin, Dnp3IinBit, Dnp3MasterSpec, Dnp3Operation, Dnp3OutstationSpec,
    Dnp3PointKind, Dnp3PointName,
};
pub use message::{
    Command, Event, EventKind, LogEntry, LogLevel, PointText, SessionId, SessionState, Stats,
};
pub use modbus::{
    Area, DataType, ModbusLink, ModbusMasterSpec, ModbusSlaveSpec, Parity, RegisterBlock,
    SerialLink, WordOrder,
};
pub use mqtt::{
    MqttBrokerSpec, MqttClientSpec, MqttQos, MqttSubscription, PathStep, SPARKPLUG_GROUP,
    SPARKPLUG_NAMESPACE, parse_json_path, valid_filter, valid_topic,
};
pub use serial::{
    FlowControl, LineEnding, LineParser, SerialChunk, SerialDeviceSpec, SerialFormat, SerialMacro,
    SerialMonitorSpec, SerialPortInfo, encode_send, parse_hex, short_port, unescape,
    virtual_port_path,
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
    /// Subscribe to an MQTT broker's topics and publish to it.
    MqttClient(MqttClientSpec),
    /// Run an MQTT broker, optionally publishing a simulated device.
    MqttBroker(MqttBrokerSpec),
    /// Read, subscribe to and command a BACnet/IP device's objects.
    BacnetClient(BacnetClientSpec),
    /// Play a BACnet/IP device with a plant's simulated objects.
    BacnetDevice(BacnetDeviceSpec),
    /// Poll a DNP3 outstation: integrity and class polls, controls, time sync.
    Dnp3Master(Dnp3MasterSpec),
    /// Play a DNP3 outstation with a plant's simulated points.
    Dnp3Outstation(Dnp3OutstationSpec),
    /// Watch and talk to a serial port; parse its lines into points.
    SerialMonitor(SerialMonitorSpec),
    /// Play an instrument on a virtual serial port.
    SerialDevice(SerialDeviceSpec),
}

impl SessionSpec {
    pub fn protocol(&self) -> Protocol {
        match self {
            Self::ModbusMaster(_) | Self::ModbusSlave(_) => Protocol::Modbus,
            Self::MqttClient(_) | Self::MqttBroker(_) => Protocol::Mqtt,
            Self::BacnetClient(_) | Self::BacnetDevice(_) => Protocol::Bacnet,
            Self::Dnp3Master(_) | Self::Dnp3Outstation(_) => Protocol::Dnp3,
            Self::SerialMonitor(_) | Self::SerialDevice(_) => Protocol::Serial,
        }
    }

    pub fn role(&self) -> Role {
        match self {
            Self::ModbusMaster(_) | Self::MqttClient(_) | Self::BacnetClient(_) => Role::Client,
            Self::ModbusSlave(_) | Self::MqttBroker(_) | Self::BacnetDevice(_) => Role::Simulator,
            Self::Dnp3Master(_) => Role::Client,
            Self::Dnp3Outstation(_) => Role::Simulator,
            Self::SerialMonitor(_) => Role::Client,
            Self::SerialDevice(_) => Role::Simulator,
        }
    }

    /// Where the session talks to, as shown next to its name.
    pub fn endpoint(&self) -> String {
        match self {
            Self::ModbusMaster(spec) => format!("{} · unit {}", spec.link, spec.unit_id),
            Self::ModbusSlave(spec) => {
                format!("{}:{} · unit {}", spec.bind, spec.port, spec.unit_id)
            }
            Self::MqttClient(spec) => format!(
                "{}{} · {}",
                if spec.tls { "mqtts://" } else { "mqtt://" },
                host_port(&spec.host, spec.port),
                match spec.subscriptions.as_slice() {
                    [only] => only.filter.clone(),
                    many => format!("{} subscriptions", many.len()),
                }
            ),
            Self::MqttBroker(spec) => match &spec.template {
                Some(_) => format!("{} · {}/#", host_port(&spec.bind, spec.port), spec.prefix),
                None => host_port(&spec.bind, spec.port),
            },
            Self::BacnetClient(spec) => match spec.device_instance {
                Some(instance) => {
                    format!("{} · device {instance}", host_port(&spec.host, spec.port))
                }
                None => host_port(&spec.host, spec.port),
            },
            Self::BacnetDevice(spec) => {
                format!("0.0.0.0:{} · device {}", spec.port, spec.device_instance)
            }
            Self::Dnp3Master(spec) => format!(
                "{} · outstation {}",
                host_port(&spec.host, spec.port),
                spec.outstation_address
            ),
            Self::Dnp3Outstation(spec) => format!(
                "{} · outstation {}",
                host_port(&spec.bind, spec.port),
                spec.outstation_address
            ),
            Self::SerialMonitor(spec) => format!(
                "{} · {} {}{}{}",
                short_port(&spec.link.port),
                spec.link.baud,
                spec.link.data_bits,
                spec.link.parity.letter(),
                spec.link.stop_bits
            ),
            Self::SerialDevice(spec) => format!("{} · {}", spec.alias, spec.template),
        }
    }

    /// Just the address (host and port, or serial port), for tight spaces.
    pub fn address(&self) -> String {
        match self {
            Self::ModbusMaster(spec) => spec.link.to_string(),
            Self::ModbusSlave(spec) => format!("{}:{}", spec.bind, spec.port),
            Self::MqttClient(spec) => host_port(&spec.host, spec.port),
            Self::MqttBroker(spec) => host_port(&spec.bind, spec.port),
            Self::BacnetClient(spec) => host_port(&spec.host, spec.port),
            Self::BacnetDevice(spec) => format!("0.0.0.0:{}", spec.port),
            Self::Dnp3Master(spec) => host_port(&spec.host, spec.port),
            Self::Dnp3Outstation(spec) => host_port(&spec.bind, spec.port),
            Self::SerialMonitor(spec) => short_port(&spec.link.port).to_owned(),
            Self::SerialDevice(spec) => spec.alias.clone(),
        }
    }
}

/// `host:port`, with an IPv6 address in brackets.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The protocols the suite speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Modbus,
    Mqtt,
    Bacnet,
    Dnp3,
    Serial,
}

impl Protocol {
    pub const ALL: &'static [Protocol] = &[
        Protocol::Modbus,
        Protocol::Mqtt,
        Protocol::Bacnet,
        Protocol::Dnp3,
        Protocol::Serial,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Modbus => "Modbus",
            Self::Mqtt => "MQTT",
            Self::Bacnet => "BACnet",
            Self::Dnp3 => "DNP3",
            Self::Serial => "Serial",
        }
    }

    /// Two-letter monogram for compact badges.
    pub fn monogram(self) -> &'static str {
        match self {
            Self::Modbus => "MB",
            Self::Mqtt => "MQ",
            Self::Bacnet => "BA",
            Self::Dnp3 => "DN",
            Self::Serial => "SE",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Modbus => "TCP and RTU master, register blocks, writes, and a slave simulator",
            Self::Mqtt => {
                "Wildcard subscriptions, JSON and Sparkplug B values, publish, and a broker simulator"
            }
            Self::Bacnet => {
                "Who-Is discovery, object list, COV and priority writes, and a device simulator"
            }
            Self::Dnp3 => {
                "Integrity and class polls, flags and events, SBO controls, and an outstation simulator"
            }
            Self::Serial => {
                "Port picker, text and hex terminal, macros, logging, lines parsed into trends, and a virtual device"
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
