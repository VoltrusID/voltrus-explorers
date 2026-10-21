use serde::{Deserialize, Serialize};

/// Serial port parity setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

impl From<Parity> for serialport::Parity {
    fn from(p: Parity) -> Self {
        match p {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        }
    }
}

/// Serial port flow control setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FlowControl {
    #[default]
    None,
    Hardware,
    Software,
}

impl From<FlowControl> for serialport::FlowControl {
    fn from(fc: FlowControl) -> Self {
        match fc {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
            FlowControl::Software => serialport::FlowControl::Software,
        }
    }
}

/// Connection configuration for a serial port (RS232/RS485).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfig {
    /// Serial port name (e.g. "/dev/ttyUSB0", "COM3").
    pub port_name: String,
    /// Baud rate (e.g. 9600, 19200, 115200).
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Data bits (5-8).
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    /// Stop bits (1 or 2).
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    /// Parity setting.
    #[serde(default)]
    pub parity: Parity,
    /// Flow control setting.
    #[serde(default)]
    pub flow_control: FlowControl,
    /// I/O timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_baud_rate() -> u32 {
    9600
}
fn default_data_bits() -> u8 {
    8
}
fn default_stop_bits() -> u8 {
    1
}
fn default_timeout_ms() -> u64 {
    1000
}

impl SerialConfig {
    /// Convert to `serialport::SerialPortBuilder` for use with tokio-serial.
    pub fn to_serial_port_builder(&self) -> Result<serialport::SerialPortBuilder, String> {
        let stop_bits = match self.stop_bits {
            1 => serialport::StopBits::One,
            2 => serialport::StopBits::Two,
            other => return Err(format!("Invalid stop bits: {other} (must be 1 or 2)")),
        };

        let data_bits = match self.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            8 => serialport::DataBits::Eight,
            other => return Err(format!("Invalid data bits: {other} (must be 5-8)")),
        };

        Ok(
            serialport::new(&self.port_name, self.baud_rate)
                .data_bits(data_bits)
                .stop_bits(stop_bits)
                .parity(self.parity.into())
                .flow_control(self.flow_control.into())
                .timeout(std::time::Duration::from_millis(self.timeout_ms)),
        )
    }
}

/// A tag describing what to read from or write to on a serial device.
///
/// For raw serial communication the "address" is simply an identifier or label
/// used to correlate requests with responses.  Protocol-specific framing (e.g.
/// Modbus RTU) is handled at a higher level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialTag {
    /// Human-readable label or address for this tag.
    pub name: String,
    /// Optional slave/device address (useful for RS485 multi-drop).
    #[serde(default)]
    pub address: Option<u8>,
}

/// A value read from (or to be written to) a serial port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialValue {
    /// Raw bytes received from or sent to the serial port.
    pub data: Vec<u8>,
    /// Hex-formatted representation of `data` for display.
    pub formatted_hex: String,
    /// Timestamp when the value was captured (UNIX epoch milliseconds).
    pub timestamp: u64,
}

impl SerialValue {
    /// Create a new `SerialValue` from raw bytes, computing the hex string
    /// and capturing the current time as the timestamp.
    pub fn new(data: Vec<u8>) -> Self {
        let formatted_hex = data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            data,
            formatted_hex,
            timestamp,
        }
    }
}

/// Information about an available serial port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortInfo {
    /// Port name (e.g. "/dev/ttyUSB0", "COM3").
    pub port_name: String,
    /// Port type (e.g. "USB", "PCI", "Bluetooth").
    #[serde(default)]
    pub port_type: String,
    /// Manufacturer string, if available.
    #[serde(default)]
    pub manufacturer: Option<String>,
    /// Product name, if available.
    #[serde(default)]
    pub product: Option<String>,
    /// Serial number, if available.
    #[serde(default)]
    pub serial_number: Option<String>,
}

impl From<&serialport::SerialPortInfo> for PortInfo {
    fn from(info: &serialport::SerialPortInfo) -> Self {
        let (port_type, manufacturer, product, serial_number) = match &info.port_type {
            serialport::SerialPortType::UsbPort(usb) => (
                "USB".to_string(),
                usb.manufacturer.clone(),
                usb.product.clone(),
                usb.serial_number.clone(),
            ),
            serialport::SerialPortType::BluetoothPort => ("Bluetooth".to_string(), None, None, None),
            serialport::SerialPortType::PciPort => ("PCI".to_string(), None, None, None),
            serialport::SerialPortType::Unknown => ("Unknown".to_string(), None, None, None),
        };
        Self {
            port_name: info.port_name.clone(),
            port_type,
            manufacturer,
            product,
            serial_number,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> SerialConfig {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn config_defaults_when_only_port_is_given() {
        let c = config(r#"{"port_name":"/dev/cu.usbserial-1"}"#);
        assert_eq!(c.baud_rate, 9600);
        assert_eq!(c.data_bits, 8);
        assert_eq!(c.stop_bits, 1);
        assert_eq!(c.parity, Parity::None);
        assert_eq!(c.flow_control, FlowControl::None);
        assert_eq!(c.timeout_ms, 1000);
    }

    #[test]
    fn config_parses_lowercase_parity_and_flow_control() {
        let c = config(r#"{"port_name":"p","parity":"even","flow_control":"hardware"}"#);
        assert_eq!(c.parity, Parity::Even);
        assert_eq!(c.flow_control, FlowControl::Hardware);
    }

    #[test]
    fn builder_applies_settings() {
        let c = config(
            r#"{"port_name":"p","baud_rate":115200,"data_bits":7,"stop_bits":2,"parity":"odd","timeout_ms":250}"#,
        );
        let expected = serialport::new("p", 115200)
            .data_bits(serialport::DataBits::Seven)
            .stop_bits(serialport::StopBits::Two)
            .parity(serialport::Parity::Odd)
            .flow_control(serialport::FlowControl::None)
            .timeout(std::time::Duration::from_millis(250));
        assert_eq!(c.to_serial_port_builder().unwrap(), expected);
    }

    #[test]
    fn builder_rejects_invalid_stop_and_data_bits() {
        let err = config(r#"{"port_name":"p","stop_bits":3}"#).to_serial_port_builder().unwrap_err();
        assert!(err.contains("stop bits"), "{err}");
        let err = config(r#"{"port_name":"p","data_bits":9}"#).to_serial_port_builder().unwrap_err();
        assert!(err.contains("data bits"), "{err}");
    }

    #[test]
    fn serial_value_formats_hex() {
        let v = SerialValue::new(vec![0x01, 0xAB, 0x0F]);
        assert_eq!(v.formatted_hex, "01 AB 0F");
        assert!(v.timestamp > 0);
        assert_eq!(SerialValue::new(vec![]).formatted_hex, "");
    }

    #[test]
    fn port_info_defaults_missing_fields() {
        let p: PortInfo = serde_json::from_str(r#"{"port_name":"COM3"}"#).unwrap();
        assert_eq!(p.port_type, "");
        assert!(p.manufacturer.is_none() && p.product.is_none() && p.serial_number.is_none());
    }
}
