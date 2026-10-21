use serde::{Deserialize, Serialize};

/// Connection configuration for a BACnet/IP device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacnetConfig {
    /// Device identifier (used for logging / deduplication).
    pub id: String,
    /// IP address or hostname of the BACnet device.
    pub ip: String,
    /// UDP port (default: 47808).
    #[serde(default = "default_bacnet_port")]
    pub port: u16,
    /// BACnet device instance number.
    #[serde(default)]
    pub device_id: u32,
    /// Polling interval in milliseconds.
    #[serde(default = "default_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_bacnet_port() -> u16 {
    47808
}
fn default_interval_ms() -> u64 {
    1000
}

/// A BACnet object-property reference describing what to read or write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacnetObject {
    /// Human-readable tag name.
    pub name: String,
    /// BACnet object type (e.g. "analogInput", "analogValue", "binaryOutput").
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Property identifier (e.g. "presentValue", "description", "statusFlags").
    #[serde(default = "default_property")]
    pub property: String,
}

fn default_property() -> String {
    "presentValue".to_string()
}

/// A value read from (or to be written to) a BACnet property.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BacnetValue {
    /// 32-bit IEEE 754 real.
    Real(f32),
    /// 64-bit IEEE 754 double.
    Double(f64),
    /// Unsigned integer.
    Unsigned(u32),
    /// Boolean.
    Boolean(bool),
    /// Enumerated value.
    Enum(u32),
    /// Raw f64 (typical output after normalisation).
    Float64(f64),
    /// String property value.
    String(String),
    /// Null / missing value.
    Null,
}

impl BacnetValue {
    /// Convert to f64 regardless of the stored type.
    pub fn as_f64(&self) -> f64 {
        match self {
            BacnetValue::Real(v) => *v as f64,
            BacnetValue::Double(v) => *v,
            BacnetValue::Unsigned(v) => *v as f64,
            BacnetValue::Boolean(v) => {
                if *v {
                    1.0
                } else {
                    0.0
                }
            }
            BacnetValue::Enum(v) => *v as f64,
            BacnetValue::Float64(v) => *v,
            BacnetValue::String(_) => f64::NAN,
            BacnetValue::Null => f64::NAN,
        }
    }
}

/// Information about a BACnet object discovered on a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacnetObjectInfo {
    /// Object type string (e.g. "analogInput").
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Object name, if retrieved.
    pub object_name: Option<String>,
}

/// Information about a BACnet device discovered on the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacnetDiscoveredDevice {
    /// Device instance number.
    pub device_instance: u32,
    /// IP address.
    pub ip: String,
    /// UDP port.
    pub port: u16,
    /// Vendor name, if identified.
    pub vendor_name: Option<String>,
    /// Model name, if retrieved.
    pub model_name: Option<String>,
    /// Firmware revision, if retrieved.
    pub firmware_version: Option<String>,
    /// Number of objects in the device's object list, if known.
    pub object_list_count: Option<u32>,
}

/// Result of a write operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResult {
    /// Whether the write succeeded.
    pub success: bool,
    /// Optional response text from the device.
    pub device_response: Option<String>,
    /// Error message if the write failed.
    pub error: Option<String>,
    /// Round-trip latency in milliseconds.
    pub latency_ms: u64,
}

/// A single property value read from a BACnet object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacnetPropertyEntry {
    /// Property name (e.g. "presentValue", "description").
    pub property_name: String,
    /// BACnet property identifier number.
    pub property_identifier: u32,
    /// Parsed numeric value if available.
    pub value: BacnetValue,
    /// Raw display string for any value type.
    pub raw_value: String,
}

/// Detailed information about a BACnet object, including its properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacnetObjectDetail {
    /// Object type string (e.g. "analogInput").
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Object name, if retrieved.
    pub object_name: Option<String>,
    /// Description, if retrieved.
    pub description: Option<String>,
    /// Present value display string.
    pub present_value: Option<String>,
    /// Engineering units.
    pub units: Option<String>,
    /// Engineering units symbol (e.g. "°C").
    pub units_symbol: Option<String>,
    /// Status flags display string.
    pub status_flags: Option<String>,
    /// Reliability display string.
    pub reliability: Option<String>,
    /// Out of service flag.
    pub out_of_service: Option<bool>,
    /// All read properties.
    pub properties: Vec<BacnetPropertyEntry>,
}

/// Full device inspection result from reading Device Object properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInspection {
    pub device_instance: u32,
    pub object_name: Option<String>,
    pub vendor_name: Option<String>,
    pub vendor_id: Option<u32>,
    pub model_name: Option<String>,
    pub firmware_revision: Option<String>,
    pub application_software_version: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub protocol_version: Option<u32>,
    pub protocol_revision: Option<u32>,
    pub max_apdu_length: Option<u32>,
    pub segmentation_supported: Option<String>,
    pub apdu_timeout: Option<u32>,
    pub number_of_apdu_retries: Option<u32>,
    pub device_address_binding: Option<String>,
    pub object_count: Option<u32>,
    pub system_status: Option<String>,
    pub local_time: Option<String>,
    pub local_date: Option<String>,
    pub database_revision: Option<u32>,
    pub services_supported: Option<String>,
    pub object_types_supported: Option<String>,
}

/// Exponential backoff for reconnection / retry attempts.
pub struct Backoff {
    attempt: u32,
    max_delay_ms: u64,
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            max_delay_ms: 60_000,
        }
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn next(&mut self) -> u64 {
        let attempt = self.attempt.min(6);
        let delay = 1000 * 2u64.pow(attempt);
        self.attempt = self.attempt.saturating_add(1);
        delay.min(self.max_delay_ms)
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}
