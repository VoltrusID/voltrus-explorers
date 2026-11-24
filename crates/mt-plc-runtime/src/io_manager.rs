use crate::types::PlcValue;
use mt_protocol::ProtocolClient;

/// I/O driver trait for connecting to field devices
pub trait IoDriver: Send + Sync {
    /// Read all mapped input values from the field device
    fn read_inputs(&mut self, addresses: &[IoAddress]) -> Result<Vec<PlcValue>, IoError>;

    /// Write all mapped output values to the field device
    fn write_outputs(&mut self, mappings: &[(IoAddress, PlcValue)]) -> Result<(), IoError>;

    /// Driver name (e.g., "modbus", "opcua", "s7")
    fn name(&self) -> &str;

    /// Check if driver is connected
    fn is_connected(&self) -> bool;

    /// Disconnect from the field device
    fn disconnect(&mut self);
}

#[derive(Debug, Clone)]
pub struct IoAddress {
    pub area: IoArea,
    pub offset: usize,
    pub bit: Option<usize>,
    pub size: IoSize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IoArea {
    Input,  // I (physical inputs)
    Output, // Q (physical outputs)
    Memory, // M (internal memory)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IoSize {
    Bit,   // X
    Byte,  // B
    Word,  // W
    Dword, // D
}

#[derive(Debug, Clone)]
pub struct IoError {
    pub message: String,
    pub recoverable: bool,
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for IoError {}

/// Parse a direct address string like "%IW0", "%QX0.3", "%MW100"
pub fn parse_direct_address(addr: &str) -> Option<IoAddress> {
    let addr = addr.strip_prefix('%')?;

    if addr.is_empty() {
        return None;
    }

    let area = match addr.chars().next()? {
        'I' | 'i' => IoArea::Input,
        'Q' | 'q' => IoArea::Output,
        'M' | 'm' => IoArea::Memory,
        _ => return None,
    };

    let rest = &addr[1..];
    if rest.is_empty() {
        return None;
    }

    let (size_char, num_part) = match rest.chars().next()? {
        'X' | 'x' => (IoSize::Bit, &rest[1..]),
        'B' | 'b' => (IoSize::Byte, &rest[1..]),
        'W' | 'w' => (IoSize::Word, &rest[1..]),
        'D' | 'd' => (IoSize::Dword, &rest[1..]),
        _ => return None,
    };

    // Handle bit addressing (e.g., "0.3")
    let (offset, bit) = if let Some(dot_pos) = num_part.find('.') {
        let off: usize = num_part[..dot_pos].parse().ok()?;
        let b: usize = num_part[dot_pos + 1..].parse().ok()?;
        (off, Some(b))
    } else {
        let off: usize = num_part.parse().ok()?;
        (off, None)
    };

    Some(IoAddress {
        area,
        offset,
        bit,
        size: size_char,
    })
}

/// Modbus I/O driver (wraps mt-modbus)
pub struct ModbusIoDriver {
    pub host: String,
    pub port: u16,
    pub unit_id: u8,
    client: Option<mt_modbus::ModbusClient>,
}

impl ModbusIoDriver {
    pub fn new(host: String, port: u16, unit_id: u8) -> Self {
        Self {
            host,
            port,
            unit_id,
            client: None,
        }
    }

    pub async fn connect(&mut self) -> Result<(), IoError> {
        let config = mt_modbus::ModbusConfig {
            ip: self.host.clone(),
            port: self.port,
            unit_id: self.unit_id,
            interval_ms: 100,
            timeout_ms: 1000,
            endianness: mt_modbus::Endianness::Big,
        };
        let client = mt_modbus::ModbusClient::connect(config)
            .await
            .map_err(|e| IoError {
                message: format!("Modbus connect: {}", e),
                recoverable: true,
            })?;
        self.client = Some(client);
        Ok(())
    }

    fn io_to_tag(&self, addr: &IoAddress) -> mt_modbus::ModbusTag {
        let (register_area, address) = match addr.area {
            IoArea::Input => ("input".into(), addr.offset),
            IoArea::Output => ("holding".into(), addr.offset),
            IoArea::Memory => ("holding".into(), addr.offset + 1000),
        };
        let reg_type = match addr.size {
            IoSize::Bit => mt_modbus::RegisterType::Uint16,
            IoSize::Byte => mt_modbus::RegisterType::Uint16,
            IoSize::Word => mt_modbus::RegisterType::Uint16,
            IoSize::Dword => mt_modbus::RegisterType::Int32,
        };
        mt_modbus::ModbusTag {
            name: String::new(),
            address: address as u16,
            reg_type,
            scale: 1.0,
            register_area,
        }
    }

    fn value_to_modbus(&self, val: &PlcValue) -> mt_modbus::ModbusValue {
        match val {
            PlcValue::Bool(b) => mt_modbus::ModbusValue::Uint16(if *b { 1 } else { 0 }),
            PlcValue::Byte(b) => mt_modbus::ModbusValue::Uint16(*b as u16),
            PlcValue::Int(i) => mt_modbus::ModbusValue::Int16(*i),
            PlcValue::Dint(i) => mt_modbus::ModbusValue::Int32(*i),
            PlcValue::Real(f) => mt_modbus::ModbusValue::Float32(*f),
            PlcValue::Word(w) => mt_modbus::ModbusValue::Uint16(*w),
            _ => mt_modbus::ModbusValue::Uint16(0),
        }
    }
}

impl IoDriver for ModbusIoDriver {
    fn read_inputs(&mut self, addresses: &[IoAddress]) -> Result<Vec<PlcValue>, IoError> {
        if let Some(ref client) = self.client {
            let tags: Vec<_> = addresses.iter().map(|a| self.io_to_tag(a)).collect();
            let rt = tokio::runtime::Handle::current();
            let values = rt
                .block_on(async { client.read(&tags).await })
                .map_err(|e| IoError {
                    message: format!("Modbus read: {}", e),
                    recoverable: true,
                })?;

            Ok(values
                .into_iter()
                .zip(addresses.iter())
                .map(|(v, addr)| match v {
                    mt_modbus::ModbusValue::Uint16(u) => {
                        if let Some(bit) = addr.bit {
                            PlcValue::Bool((u >> bit) & 1 == 1)
                        } else {
                            PlcValue::Word(u)
                        }
                    }
                    mt_modbus::ModbusValue::Int16(i) => PlcValue::Int(i),
                    mt_modbus::ModbusValue::Int32(i) => PlcValue::Dint(i),
                    mt_modbus::ModbusValue::Float32(f) => PlcValue::Real(f),
                    mt_modbus::ModbusValue::Uint32(u) => PlcValue::Dint(u as i32),
                    mt_modbus::ModbusValue::Float64(f) => PlcValue::Real(f as f32),
                    mt_modbus::ModbusValue::Failed(_) => PlcValue::Dint(0),
                })
                .collect())
        } else {
            Ok(addresses.iter().map(|_| PlcValue::Dint(0)).collect())
        }
    }

    fn write_outputs(&mut self, mappings: &[(IoAddress, PlcValue)]) -> Result<(), IoError> {
        if let Some(ref client) = self.client {
            let rt = tokio::runtime::Handle::current();
            for (addr, val) in mappings {
                let tag = self.io_to_tag(addr);
                let value = self.value_to_modbus(val);
                rt.block_on(async { client.write(&tag, value).await })
                    .map_err(|e| IoError {
                        message: format!("Modbus write: {}", e),
                        recoverable: true,
                    })?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "modbus"
    }

    fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    fn disconnect(&mut self) {
        if let Some(client) = self.client.take() {
            let rt = tokio::runtime::Handle::current();
            let _ = rt.block_on(async { client.disconnect().await });
        }
    }
}

/// Create an I/O driver from configuration. The returned driver is connected.
pub fn create_driver(
    protocol: &str,
    host: &str,
    port: u16,
    unit_id: u8,
    s7_rack: u8,
    s7_slot: u8,
    opcua_endpoint_url: &str,
    mqtt_topic_prefix: &str,
    mqtt_client_id: &str,
) -> Result<Box<dyn IoDriver>, IoError> {
    let rt = tokio::runtime::Handle::current();
    match protocol {
        "modbus" => {
            let mut driver = ModbusIoDriver::new(host.to_string(), port, unit_id);
            rt.block_on(driver.connect())?;
            Ok(Box::new(driver))
        }
        "opcua" => {
            let endpoint = if opcua_endpoint_url.is_empty() {
                format!("opc.tcp://{}:{}/", host, port)
            } else {
                opcua_endpoint_url.to_string()
            };
            let mut driver = crate::io_opcua::OpcUaIoDriver::new(endpoint);
            rt.block_on(driver.connect())?;
            Ok(Box::new(driver))
        }
        "s7" => {
            let mut driver =
                crate::io_s7::S7IoDriver::new(host.to_string(), port, s7_rack, s7_slot);
            rt.block_on(driver.connect())?;
            Ok(Box::new(driver))
        }
        "mqtt" => {
            let client_id = if mqtt_client_id.is_empty() {
                format!("voltrus-plc-{}", std::process::id())
            } else {
                mqtt_client_id.to_string()
            };
            let mut driver = crate::io_mqtt::MqttIoDriver::new(
                host.to_string(),
                port,
                client_id,
                mqtt_topic_prefix.to_string(),
            );
            rt.block_on(driver.connect())?;
            Ok(Box::new(driver))
        }
        "enip" => {
            let port = if port == 0 { 44818 } else { port };
            let mut driver = crate::io_enip::EnipIoDriver::new(host.to_string(), port);
            rt.block_on(driver.connect())?;
            Ok(Box::new(driver))
        }
        other => Err(IoError {
            message: format!("Unknown protocol: {}", other),
            recoverable: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_input_word() {
        let addr = parse_direct_address("%IW0").unwrap();
        assert_eq!(addr.area, IoArea::Input);
        assert_eq!(addr.size, IoSize::Word);
        assert_eq!(addr.offset, 0);
        assert_eq!(addr.bit, None);
    }

    #[test]
    fn test_parse_output_bit() {
        let addr = parse_direct_address("%QX0.3").unwrap();
        assert_eq!(addr.area, IoArea::Output);
        assert_eq!(addr.size, IoSize::Bit);
        assert_eq!(addr.offset, 0);
        assert_eq!(addr.bit, Some(3));
    }

    #[test]
    fn test_parse_memory_dword() {
        let addr = parse_direct_address("%MD100").unwrap();
        assert_eq!(addr.area, IoArea::Memory);
        assert_eq!(addr.size, IoSize::Dword);
        assert_eq!(addr.offset, 100);
    }

    #[test]
    fn test_parse_lowercase() {
        let addr = parse_direct_address("%qw5").unwrap();
        assert_eq!(addr.area, IoArea::Output);
        assert_eq!(addr.size, IoSize::Word);
        assert_eq!(addr.offset, 5);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_direct_address("").is_none());
        assert!(parse_direct_address("IW0").is_none());
        assert!(parse_direct_address("%").is_none());
    }
}
