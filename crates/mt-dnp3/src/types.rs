use serde::{Deserialize, Serialize};

/// Connection configuration for a DNP3 outstation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dnp3Config {
    /// Device identifier.
    pub id: String,
    /// IP address or hostname of the DNP3 outstation.
    pub host: String,
    /// TCP port (default: 20000).
    #[serde(default = "default_dnp3_port")]
    pub port: u16,
    /// DNP3 master address (source address).
    #[serde(default = "default_master_address")]
    pub master_address: u16,
    /// DNP3 outstation address (destination address).
    #[serde(default = "default_outstation_address")]
    pub outstation_address: u16,
    /// Polling interval in milliseconds.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_dnp3_port() -> u16 {
    20000
}
fn default_master_address() -> u16 {
    1
}
fn default_outstation_address() -> u16 {
    10
}
fn default_poll_interval_ms() -> u64 {
    1000
}

/// A DNP3 point identifier (group, variation, index).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dnp3Point {
    /// Human-readable point name.
    pub name: String,
    /// DNP3 object group number (e.g. 30 for Analog Input).
    pub group: u8,
    /// DNP3 variation number (e.g. 4 for Analog Input float32).
    pub variation: u8,
    /// Point index within the group.
    pub index: u16,
    /// Scale factor applied after reading.
    #[serde(default = "default_scale")]
    pub scale: f64,
}

fn default_scale() -> f64 {
    1.0
}

/// A value read from a DNP3 point.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dnp3Value {
    /// The point this value came from.
    pub point: Dnp3Point,
    /// The numeric value.
    pub value: f64,
}

impl Dnp3Value {
    /// Scaled value (value * point.scale).
    pub fn scaled(&self) -> f64 {
        self.value * self.point.scale
    }
}

/// Result of a DNP3 write/control operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dnp3WriteResult {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Optional response text from the device.
    pub device_response: Option<String>,
    /// Error message if the operation failed.
    pub error: Option<String>,
    /// Round-trip latency in milliseconds.
    pub latency_ms: u64,
}

/// Configuration for running a DNP3 outstation server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutstationConfig {
    /// TCP port to listen on.
    #[serde(default = "default_outstation_port")]
    pub port: u16,
    /// Bind address (default: "0.0.0.0").
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    /// DNP3 master address (source address for incoming requests).
    #[serde(default = "default_master_address")]
    pub master_address: u16,
    /// DNP3 outstation address (this server's address).
    #[serde(default = "default_outstation_address")]
    pub outstation_address: u16,
    /// Number of analog input points (Group 30).
    #[serde(default = "default_analog_count")]
    pub analog_count: u16,
    /// Number of binary input points (Group 1).
    #[serde(default = "default_binary_count")]
    pub binary_count: u16,
    /// Number of counter input points (Group 20).
    #[serde(default = "default_counter_count")]
    pub counter_count: u16,
}

fn default_outstation_port() -> u16 { 20000 }
fn default_bind_addr() -> String { "0.0.0.0".into() }
fn default_analog_count() -> u16 { 10 }
fn default_binary_count() -> u16 { 10 }
fn default_counter_count() -> u16 { 4 }

/// A log entry from the outstation server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutstationLogEntry {
    pub timestamp: u64,
    pub function_code: u8,
    pub message: String,
}

/// A buffered event with sequence-of-events timestamp (SOE).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutstationEvent {
    /// Event class (1, 2, or 3).
    pub class: u8,
    /// Point group (1 = binary, 30 = analog, 20 = counter).
    pub group: u8,
    /// Point index within the group.
    pub index: u16,
    /// Numeric value at the time of the change.
    pub value: f64,
    /// Sequence-of-events timestamp (ms since Unix epoch).
    pub timestamp: u64,
    /// Monotonic event sequence number.
    pub sequence: u32,
}

/// A simulation point definition for DNP3.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDnp3PointDef {
    pub key: String,
    pub label: String,
    pub group: u8,
    pub variation: u8,
    pub index: u16,
    pub base: f64,
    pub variance: f64,
    pub noise: f64,
    pub period_ms: u64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub decimals: Option<u32>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub is_bool: bool,
}

/// A device profile for DNP3 simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDnp3Profile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub points: Vec<SimDnp3PointDef>,
}

/// Exponential backoff for reconnection attempts.
pub struct Backoff {
    attempt: u32,
    max_delay_ms: u64,
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            max_delay_ms: 30_000,
        }
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn next(&mut self) -> u64 {
        let attempt = self.attempt.min(30);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_sequence() {
        let mut b = Backoff::new();
        assert_eq!(b.next(), 1000);
        assert_eq!(b.next(), 2000);
        assert_eq!(b.next(), 4000);
    }

    #[test]
    fn test_backoff_resets() {
        let mut b = Backoff::new();
        b.next();
        b.next();
        b.reset();
        assert_eq!(b.next(), 1000);
    }

    #[test]
    fn test_backoff_max() {
        let mut b = Backoff::new();
        for _ in 0..20 {
            b.next();
        }
        assert_eq!(b.next(), 30_000);
    }

    #[test]
    fn test_dnp3_value_scaled() {
        let val = Dnp3Value {
            point: Dnp3Point {
                name: "voltage".into(),
                group: 30,
                variation: 4,
                index: 0,
                scale: 0.1,
            },
            value: 2205.0,
        };
        assert!((val.scaled() - 220.5).abs() < 0.001);
    }

    #[test]
    fn test_config_defaults() {
        let json = r#"{"id":"d1","host":"192.168.1.1"}"#;
        let config: Dnp3Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 20000);
        assert_eq!(config.master_address, 1);
        assert_eq!(config.outstation_address, 10);
        assert_eq!(config.poll_interval_ms, 1000);
    }
}
