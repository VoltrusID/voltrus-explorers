//! Connection forms as plain strings, validated into session specs. The UI
//! binds text fields to these; every rule lives here, testable without a
//! window.

use vt_protocol::{
    Area, DataType, ModbusLink, ModbusMasterSpec, ModbusSlaveSpec, RegisterBlock, SerialLink,
    WordOrder,
};

mod bacnet;

pub use bacnet::{BacnetClientForm, BacnetDeviceForm};
mod mqtt;

pub use mqtt::{MqttBrokerForm, MqttClientForm, SubscriptionField, SubscriptionForm};
mod dnp3;

pub use dnp3::{
    Dnp3MasterForm, Dnp3OutstationForm, Dnp3PointField, Dnp3PointForm, next_dnp3_point,
};
mod enip;

pub use enip::{
    CipRequestForm, EnipAdapterForm, EnipClientForm, EnipPointField, EnipPointForm, hex_rows,
    interpret_reply, parse_hex, valid_route_path,
};

/// The first problem found in a form, and the field it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormError {
    pub field: Field,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Host,
    Port,
    SerialPort,
    Baud,
    UnitId,
    PollMs,
    TimeoutMs,
    Bind,
    Registers,
    /// A block row: its index and which of its fields.
    Block(usize, BlockField),
    Blocks,
    /// BACnet: the device object instance, the client's local UDP port and
    /// its command priority.
    DeviceInstance,
    LocalPort,
    Priority,
    /// MQTT: a subscription row and which of its fields.
    Subscription(usize, SubscriptionField),
    Subscriptions,
    KeepAlive,
    Username,
    Prefix,
    PublishMs,
    /// DNP3: the master's own link address and the outstation's, the poll
    /// periods, and a point-name row with which of its fields.
    LinkAddress,
    OutstationAddress,
    IntegrityMs,
    EventMs,
    Dnp3Point(usize, Dnp3PointField),
    /// EtherNet/IP: the CIP route to the controller, and a point row.
    RoutePath,
    EnipPoint(usize, EnipPointField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockField {
    Name,
    Start,
    Count,
}

fn error(field: Field, message: impl Into<String>) -> FormError {
    FormError {
        field,
        message: message.into(),
    }
}

fn number<T: std::str::FromStr + PartialOrd + std::fmt::Display + Copy>(
    text: &str,
    field: Field,
    what: &str,
    min: T,
    max: T,
) -> Result<T, FormError> {
    let value: T = text.trim().parse().map_err(|_| {
        error(
            field,
            format!("{what} must be a number from {min} to {max}"),
        )
    })?;
    if !(min..=max).contains(&value) {
        return Err(error(field, format!("{what} must be from {min} to {max}")));
    }
    Ok(value)
}

fn name(text: &str) -> Result<String, FormError> {
    let name = text.trim();
    if name.is_empty() {
        return Err(error(Field::Name, "Give the connection a name"));
    }
    Ok(name.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Tcp,
    Rtu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockForm {
    pub name: String,
    pub area: Area,
    pub start: String,
    pub count: String,
    pub data_type: DataType,
    /// Not edited in the form; kept so template units survive an edit.
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterForm {
    pub name: String,
    pub transport: Transport,
    pub host: String,
    pub port: String,
    pub serial: SerialLink,
    pub baud: String,
    pub unit_id: String,
    pub poll_ms: String,
    pub timeout_ms: String,
    pub word_order: WordOrder,
    pub blocks: Vec<BlockForm>,
}

impl MasterForm {
    pub fn from_spec(name: &str, spec: &ModbusMasterSpec) -> Self {
        let (transport, host, port, serial) = match &spec.link {
            ModbusLink::Tcp { host, port } => (
                Transport::Tcp,
                host.clone(),
                port.to_string(),
                SerialLink::default(),
            ),
            ModbusLink::Rtu(serial) => (
                Transport::Rtu,
                "127.0.0.1".into(),
                "502".into(),
                serial.clone(),
            ),
        };
        Self {
            name: name.to_owned(),
            transport,
            host,
            port,
            baud: serial.baud.to_string(),
            serial,
            unit_id: spec.unit_id.to_string(),
            poll_ms: spec.poll_ms.to_string(),
            timeout_ms: spec.timeout_ms.to_string(),
            word_order: spec.word_order,
            blocks: spec
                .blocks
                .iter()
                .map(|block| BlockForm {
                    name: block.name.clone(),
                    area: block.area,
                    start: block.start.to_string(),
                    count: block.count.to_string(),
                    data_type: block.data_type,
                    unit: block.unit.clone(),
                })
                .collect(),
        }
    }

    pub fn build(&self) -> Result<(String, ModbusMasterSpec), FormError> {
        let name = name(&self.name)?;
        let link = match self.transport {
            Transport::Tcp => {
                let host = self.host.trim();
                if host.is_empty() {
                    return Err(error(
                        Field::Host,
                        "Enter the device's IP address or host name",
                    ));
                }
                ModbusLink::Tcp {
                    host: host.to_owned(),
                    port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                }
            }
            Transport::Rtu => {
                let port = self.serial.port.trim();
                if port.is_empty() {
                    return Err(error(
                        Field::SerialPort,
                        "Enter the serial port, e.g. /dev/tty.usbserial",
                    ));
                }
                ModbusLink::Rtu(SerialLink {
                    port: port.to_owned(),
                    baud: number(&self.baud, Field::Baud, "Baud rate", 300, 4_000_000)?,
                    ..self.serial.clone()
                })
            }
        };
        // Unit 0 is the serial broadcast address: nobody answers reads there.
        let lowest_unit = if self.transport == Transport::Rtu {
            1
        } else {
            0
        };
        let unit_id = number(
            &self.unit_id,
            Field::UnitId,
            "Unit ID",
            lowest_unit,
            if self.transport == Transport::Rtu {
                247
            } else {
                255
            },
        )?;
        let poll_ms = number(&self.poll_ms, Field::PollMs, "Poll interval", 20, 3_600_000)?;
        let timeout_ms = number(&self.timeout_ms, Field::TimeoutMs, "Timeout", 100, 60_000)?;
        if self.blocks.is_empty() {
            return Err(error(
                Field::Blocks,
                "Add at least one block of registers to read",
            ));
        }
        let mut blocks = Vec::with_capacity(self.blocks.len());
        for (index, block) in self.blocks.iter().enumerate() {
            let label = block.name.trim();
            let start = number(
                &block.start,
                Field::Block(index, BlockField::Start),
                "Start address",
                0,
                u16::MAX,
            )?;
            let count = number(
                &block.count,
                Field::Block(index, BlockField::Count),
                "Count",
                1,
                2000,
            )?;
            let built = RegisterBlock {
                name: if label.is_empty() {
                    format!("{} {start}", block.area.short())
                } else {
                    label.to_owned()
                },
                area: block.area,
                start,
                count,
                data_type: block.data_type,
                unit: block.unit.clone(),
            };
            if built.end_address().is_none() {
                return Err(error(
                    Field::Block(index, BlockField::Count),
                    "The block runs past address 65535",
                ));
            }
            blocks.push(built);
        }
        Ok((
            name,
            ModbusMasterSpec {
                link,
                unit_id,
                poll_ms,
                timeout_ms,
                word_order: self.word_order,
                blocks,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlaveForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub unit_id: String,
    pub template: Option<String>,
    pub registers: String,
}

impl SlaveForm {
    pub fn from_spec(name: &str, spec: &ModbusSlaveSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            unit_id: spec.unit_id.to_string(),
            template: spec.template.clone(),
            registers: spec.registers.to_string(),
        }
    }

    pub fn build(&self) -> Result<(String, ModbusSlaveSpec), FormError> {
        let name = name(&self.name)?;
        // IPv6 addresses may come bracketed, as in URLs; the spec keeps the
        // bare address and the engine brackets it when it listens.
        let bind = self.bind.trim();
        let bare = bind
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(bind);
        let Ok(ip) = bare.parse::<std::net::IpAddr>() else {
            return Err(error(
                Field::Bind,
                "Listen on an IP address, e.g. 127.0.0.1, 0.0.0.0 or ::",
            ));
        };
        let most = if self.template.is_some() {
            u16::MAX
        } else {
            ModbusSlaveSpec::MAX_PLAIN_REGISTERS
        };
        Ok((
            name,
            ModbusSlaveSpec {
                bind: ip.to_string(),
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                unit_id: number(&self.unit_id, Field::UnitId, "Unit ID", 1, 247)?,
                template: self.template.clone(),
                registers: number(&self.registers, Field::Registers, "Registers", 1, most)?,
            },
        ))
    }
}

/// Blank form fields for a new block, continuing after the last one.
pub fn next_block(blocks: &[BlockForm]) -> BlockForm {
    let start = blocks
        .last()
        .and_then(|last| {
            let start: u16 = last.start.trim().parse().ok()?;
            let count: u16 = last.count.trim().parse().ok()?;
            let width = if last.area.is_bit() {
                1
            } else {
                last.data_type.words()
            };
            start.checked_add(count.checked_mul(width)?)
        })
        .unwrap_or(0);
    BlockForm {
        name: String::new(),
        area: blocks.last().map_or(Area::Holding, |last| last.area),
        start: start.to_string(),
        count: "10".into(),
        data_type: blocks.last().map_or(DataType::U16, |last| last.data_type),
        unit: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> MasterForm {
        MasterForm::from_spec("PLC", &ModbusMasterSpec::default())
    }

    #[test]
    fn a_default_form_round_trips_to_the_same_spec() {
        let (name, spec) = form().build().unwrap();
        assert_eq!(name, "PLC");
        assert_eq!(spec, ModbusMasterSpec::default());
    }

    #[test]
    fn errors_point_at_the_offending_field() {
        let mut bad = form();
        bad.port = "70000".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Port);

        let mut bad = form();
        bad.blocks[0].start = "65530".into();
        bad.blocks[0].count = "4".into();
        bad.blocks[0].data_type = DataType::F64;
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::Block(0, BlockField::Count)
        );

        let mut bad = form();
        bad.transport = Transport::Rtu;
        bad.serial.port = "/dev/ttyUSB0".into();
        bad.unit_id = "0".into();
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::UnitId,
            "broadcast unit on RTU"
        );
    }

    #[test]
    fn new_blocks_continue_after_the_last_one() {
        let mut blocks = form().blocks;
        blocks[0].start = "100".into();
        blocks[0].count = "4".into();
        blocks[0].data_type = DataType::F32;
        let next = next_block(&blocks);
        assert_eq!(next.start, "108");
        assert_eq!(next.data_type, DataType::F32);
    }

    #[test]
    fn simulators_must_listen_on_an_ip() {
        let mut slave = SlaveForm::from_spec("Sim", &ModbusSlaveSpec::default());
        assert!(slave.build().is_ok());
        slave.bind = "localhost".into();
        assert_eq!(slave.build().unwrap_err().field, Field::Bind);
    }

    #[test]
    fn simulators_listen_on_ipv6_bracketed_or_not() {
        let mut slave = SlaveForm::from_spec("Sim", &ModbusSlaveSpec::default());
        for bind in ["::", "[::]", " [::1] "] {
            slave.bind = bind.into();
            let (_, spec) = slave.build().expect("an IPv6 listen address");
            assert_eq!(spec.bind, bind.trim().trim_matches(['[', ']']));
        }
        slave.bind = "[127.0.0.1".into();
        assert_eq!(slave.build().unwrap_err().field, Field::Bind);
    }

    #[test]
    fn plain_simulators_serve_at_most_2000_registers() {
        let mut slave = SlaveForm::from_spec("Sim", &ModbusSlaveSpec::default());
        slave.registers = "2000".into();
        assert_eq!(slave.build().expect("2000 registers").1.registers, 2000);
        slave.registers = "2001".into();
        let error = slave.build().unwrap_err();
        assert_eq!(error.field, Field::Registers);
        assert_eq!(error.message, "Registers must be from 1 to 2000");
        slave.registers = "0".into();
        assert_eq!(slave.build().unwrap_err().field, Field::Registers);
        // A template sizes the image itself; the field only pads it.
        slave.template = Some("pm5560".into());
        slave.registers = "5000".into();
        assert!(slave.build().is_ok());
    }
}
