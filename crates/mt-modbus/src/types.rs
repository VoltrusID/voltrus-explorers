use serde::{Deserialize, Serialize};

/// Connection configuration for a Modbus TCP device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModbusConfig {
    /// Device identifier (used for connection pooling).
    #[serde(default)]
    pub id: Option<String>,
    /// IP address or hostname of the Modbus TCP device.
    #[serde(alias = "host")]
    pub ip: String,
    /// TCP port (default: 502).
    #[serde(default = "default_modbus_port")]
    pub port: u16,
    /// Modbus unit/slave ID.
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    /// Polling interval in milliseconds.
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    /// Byte order for multi-register values.
    #[serde(default)]
    pub endianness: Endianness,
    /// Optional local bind address for the TCP connection.
    #[serde(default)]
    pub bind_addr: Option<String>,
}

fn default_modbus_port() -> u16 {
    502
}
fn default_unit_id() -> u8 {
    1
}
fn default_interval_ms() -> u64 {
    1000
}

/// A Modbus register tag describing what to read or write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModbusTag {
    /// Human-readable tag name.
    pub name: String,
    /// Modbus address (5-digit convention, e.g. 40001).
    pub address: u16,
    /// Register data type.
    #[serde(default, rename = "type")]
    pub reg_type: RegisterType,
    /// Scale factor applied after reading.
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// Register area: "holding", "input", "coil", or "discrete".
    #[serde(default = "default_register_area")]
    pub register_area: String,
}

fn default_scale() -> f64 {
    1.0
}
fn default_register_area() -> String {
    "holding".to_string()
}

impl ModbusTag {
    /// Number of 16-bit registers this tag occupies.
    pub fn register_count(&self) -> u16 {
        match self.reg_type {
            RegisterType::Int16 | RegisterType::Uint16 => 1,
            RegisterType::Int32 | RegisterType::Float32 => 2,
            RegisterType::Float64 => 4,
        }
    }

    /// Return the 0-based protocol address.
    ///
    /// The frontend already sends 0-based addresses. The 5-digit convention
    /// (40001, 30001, etc.) is only applied for display purposes, not for
    /// protocol communication.
    pub fn protocol_address(&self) -> u16 {
        self.address
    }
}

/// A value read from (or to be written to) a Modbus register.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModbusValue {
    /// Single 16-bit unsigned value.
    Uint16(u16),
    /// Signed 16-bit value.
    Int16(i16),
    /// 32-bit integer composed from two registers.
    Int32(i32),
    /// 32-bit IEEE 754 float composed from two registers.
    Float32(f32),
    /// Scaled f64 value (typical output from reading with scale factor).
    Float64(f64),
}

impl ModbusValue {
    /// Convert to f64 regardless of the stored type.
    pub fn as_f64(&self) -> f64 {
        match self {
            ModbusValue::Uint16(v) => *v as f64,
            ModbusValue::Int16(v) => *v as f64,
            ModbusValue::Int32(v) => *v as f64,
            ModbusValue::Float32(v) => *v as f64,
            ModbusValue::Float64(v) => *v,
        }
    }
}

/// Register data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RegisterType {
    Int16,
    #[default]
    Uint16,
    Int32,
    Float32,
    Float64,
}

/// Byte ordering for multi-register values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Endianness {
    #[default]
    Big,
    Little,
    Swapped,
}

/// Parse raw 16-bit register words into an f64 value.
/// Returns `f64::NAN` if `words` is shorter than the type requires.
pub fn parse_register(words: &[u16], reg_type: &RegisterType, endianness: &Endianness) -> f64 {
    let expected: usize = match reg_type {
        RegisterType::Int16 | RegisterType::Uint16 => 1,
        RegisterType::Int32 | RegisterType::Float32 => 2,
        RegisterType::Float64 => 4,
    };
    if words.len() < expected {
        return f64::NAN;
    }
    match reg_type {
        RegisterType::Uint16 => words[0] as f64,
        RegisterType::Int16 => (words[0] as i16) as f64,
        RegisterType::Int32 => {
            let raw = combine_u16(words[0], words[1], endianness);
            raw as i32 as f64
        }
        RegisterType::Float32 => {
            let raw = combine_u16(words[0], words[1], endianness);
            f32::from_bits(raw) as f64
        }
        RegisterType::Float64 => {
            let hi = combine_u16(words[0], words[1], endianness);
            let lo = combine_u16(words[2], words[3], endianness);
            let bits = ((hi as u64) << 32) | (lo as u64);
            f64::from_bits(bits)
        }
    }
}

/// Combine two 16-bit words into a 32-bit value according to the given endianness.
pub fn combine_u16(high: u16, low: u16, endianness: &Endianness) -> u32 {
    match endianness {
        Endianness::Big => ((high as u32) << 16) | (low as u32),
        Endianness::Little => ((low as u32) << 16) | (high as u32),
        Endianness::Swapped => {
            let h = high.swap_bytes();
            let l = low.swap_bytes();
            ((h as u32) << 16) | (l as u32)
        }
    }
}

/// Convert an f64 to raw u16 register words for writing.
pub fn value_to_words(value: f64, reg_type: &RegisterType, endianness: &Endianness) -> Vec<u16> {
    match reg_type {
        RegisterType::Uint16 => vec![value as u16],
        RegisterType::Int16 => vec![(value as i16) as u16],
        RegisterType::Int32 => {
            let raw = (value as i32) as u32;
            split_u32(raw, endianness)
        }
        RegisterType::Float32 => {
            let raw = (value as f32).to_bits();
            split_u32(raw, endianness)
        }
        RegisterType::Float64 => {
            let raw = value.to_bits();
            let hi = split_u32((raw >> 32) as u32, endianness);
            let lo = split_u32((raw & 0xFFFFFFFF) as u32, endianness);
            [hi, lo].concat()
        }
    }
}

fn split_u32(raw: u32, endianness: &Endianness) -> Vec<u16> {
    match endianness {
        Endianness::Big => vec![(raw >> 16) as u16, raw as u16],
        Endianness::Little => vec![raw as u16, (raw >> 16) as u16],
        Endianness::Swapped => {
            let h = (raw >> 16) as u16;
            let l = raw as u16;
            vec![h.swap_bytes(), l.swap_bytes()]
        }
    }
}

/// Result of a register scan operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedRegister {
    /// Modbus address (5-digit convention).
    pub address: u16,
    /// First-pass raw value.
    pub raw_value: Option<u16>,
    /// Second-pass raw value (for change detection).
    pub raw_value2: Option<u16>,
    /// Whether the value changed between passes.
    pub changed: bool,
    /// Whether the register is readable.
    pub readable: bool,
    /// Heuristically determined data type label.
    pub likely_type: String,
    /// Interpreted numeric value.
    pub interpreted_value: Option<f64>,
}

/// A log entry for protocol-level traffic monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficLogEntry {
    pub timestamp: u64,
    pub direction: String,
    pub function_code: u8,
    pub address: u16,
    pub data: String,
    pub length: u16,
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

/// Result of a bus scan operation — a device that responded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusScanResult {
    /// Modbus unit/slave ID that responded.
    pub unit_id: u8,
    /// Raw register value read from the device.
    pub value: u16,
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

/// Configuration for running as a Modbus TCP slave (server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaveConfig {
    #[serde(default = "default_modbus_port")]
    pub port: u16,
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    #[serde(default = "default_register_count")]
    pub register_count: u16,
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
}

fn default_register_count() -> u16 {
    100
}
fn default_bind_addr() -> String {
    "0.0.0.0".to_string()
}

/// Serial port parameters for Modbus RTU connections.
#[cfg(feature = "serial")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtuSerialConfig {
    pub port_name: String,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default)]
    pub parity: RtuParity,
}

#[cfg(feature = "serial")]
fn default_baud_rate() -> u32 {
    9600
}
#[cfg(feature = "serial")]
fn default_data_bits() -> u8 {
    8
}
#[cfg(feature = "serial")]
fn default_stop_bits() -> u8 {
    1
}

#[cfg(feature = "serial")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RtuParity {
    #[default]
    None,
    Odd,
    Even,
}

/// A log entry emitted by the slave server when it handles a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaveLogEntry {
    pub timestamp: u64,
    pub function_code: u8,
    pub address: u16,
    pub quantity: u16,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uint16() {
        assert_eq!(
            parse_register(&[1000u16], &RegisterType::Uint16, &Endianness::Big),
            1000.0
        );
    }

    #[test]
    fn test_parse_int16_negative() {
        assert_eq!(
            parse_register(&[0xFFFFu16], &RegisterType::Int16, &Endianness::Big),
            -1.0
        );
    }

    #[test]
    fn test_parse_float32_big_endian() {
        let val = parse_register(&[0x435C, 0x8000], &RegisterType::Float32, &Endianness::Big);
        assert!((val - 220.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_float32_little_endian() {
        let val = parse_register(
            &[0x8000, 0x435C],
            &RegisterType::Float32,
            &Endianness::Little,
        );
        assert!((val - 220.5).abs() < 0.01);
    }

    #[test]
    fn test_combine_big_endian() {
        assert_eq!(combine_u16(0x1234, 0x5678, &Endianness::Big), 0x12345678);
    }

    #[test]
    fn test_combine_little_endian() {
        assert_eq!(combine_u16(0x5678, 0x1234, &Endianness::Little), 0x12345678);
    }

    #[test]
    fn test_register_count() {
        let tag = ModbusTag {
            name: "v".into(),
            address: 40001,
            reg_type: RegisterType::Uint16,
            scale: 1.0,
            register_area: "holding".into(),
        };
        assert_eq!(tag.register_count(), 1);

        let tag = ModbusTag {
            name: "v".into(),
            address: 40001,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
        };
        assert_eq!(tag.register_count(), 2);
    }

    #[test]
    fn test_protocol_address() {
        // protocol_address() returns the address as-is (0-based from frontend)
        let tag = ModbusTag {
            name: "v".into(),
            address: 0,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
        };
        assert_eq!(tag.protocol_address(), 0);

        let tag = ModbusTag {
            name: "v".into(),
            address: 2,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
        };
        assert_eq!(tag.protocol_address(), 2);

        let tag = ModbusTag {
            name: "v".into(),
            address: 10,
            reg_type: RegisterType::Uint16,
            scale: 1.0,
            register_area: "holding".into(),
        };
        assert_eq!(tag.protocol_address(), 10);

        let tag = ModbusTag {
            name: "v".into(),
            address: 40001,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
        };
        assert_eq!(tag.protocol_address(), 40001);
    }

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
    fn test_value_to_words_uint16() {
        let words = value_to_words(42.0, &RegisterType::Uint16, &Endianness::Big);
        assert_eq!(words, vec![42]);
    }

    #[test]
    fn test_value_to_words_float32() {
        let words = value_to_words(220.5, &RegisterType::Float32, &Endianness::Big);
        assert_eq!(words, vec![0x435C, 0x8000]);
    }

    #[test]
    fn test_modbus_value_as_f64() {
        assert_eq!(ModbusValue::Uint16(100).as_f64(), 100.0);
        assert_eq!(ModbusValue::Int16(-1).as_f64(), -1.0);
        assert_eq!(ModbusValue::Float64(220.5).as_f64(), 220.5);
    }

    // ── New tests ──

    #[test]
    fn test_parse_int32_negative() {
        let val = parse_register(&[0xFFFF, 0xFFFF], &RegisterType::Int32, &Endianness::Big);
        assert_eq!(val, -1.0);
    }

    #[test]
    fn test_parse_int32_big_endian() {
        // 0xFFFF_FFFE = -2 in i32
        let val = parse_register(&[0xFFFF, 0xFFFE], &RegisterType::Int32, &Endianness::Big);
        assert_eq!(val, -2.0);
    }

    #[test]
    fn test_parse_float64_big_endian() {
        let pi = std::f64::consts::PI;
        let bits = pi.to_bits();
        // Reconstruct from hi/lo u32 halves
        let hi32 = (bits >> 32) as u32;
        let lo32 = (bits & 0xFFFFFFFF) as u32;
        let w0 = (hi32 >> 16) as u16;
        let w1 = (hi32 & 0xFFFF) as u16;
        let w2 = (lo32 >> 16) as u16;
        let w3 = (lo32 & 0xFFFF) as u16;
        let val = parse_register(&[w0, w1, w2, w3], &RegisterType::Float64, &Endianness::Big);
        assert!((val - pi).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_float64_little_endian() {
        let pi = std::f64::consts::PI;
        let bits = pi.to_bits();
        let hi32 = (bits >> 32) as u32;
        let lo32 = (bits & 0xFFFFFFFF) as u32;
        // Little endian: low word first in each u32 half
        let w0 = (hi32 & 0xFFFF) as u16;
        let w1 = (hi32 >> 16) as u16;
        let w2 = (lo32 & 0xFFFF) as u16;
        let w3 = (lo32 >> 16) as u16;
        let val = parse_register(&[w0, w1, w2, w3], &RegisterType::Float64, &Endianness::Little);
        assert!((val - pi).abs() < f64::EPSILON);
    }

    #[test]
    fn test_combine_u16_swapped() {
        // 0x1234 -> swap_bytes -> 0x3412, 0x5678 -> swap_bytes -> 0x7856
        // result: (0x3412 << 16) | 0x7856 = 0x34127856
        assert_eq!(combine_u16(0x1234, 0x5678, &Endianness::Swapped), 0x3412_7856);
    }

    #[test]
    fn test_value_to_words_int16() {
        let words = value_to_words(-1.0, &RegisterType::Int16, &Endianness::Big);
        assert_eq!(words, vec![0xFFFF]);
    }

    #[test]
    fn test_value_to_words_int32_roundtrip() {
        let original = -100.0;
        let words = value_to_words(original, &RegisterType::Int32, &Endianness::Big);
        let parsed = parse_register(&words, &RegisterType::Int32, &Endianness::Big);
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_value_to_words_float64_roundtrip() {
        let original = 3.14159265358979;
        let words = value_to_words(original, &RegisterType::Float64, &Endianness::Big);
        assert_eq!(words.len(), 4);
        let parsed = parse_register(&words, &RegisterType::Float64, &Endianness::Big);
        assert!((parsed - original).abs() < f64::EPSILON);
    }

    #[test]
    fn test_modbus_config_serde_roundtrip() {
        let config = ModbusConfig {
            id: Some("test-device".into()),
            ip: "192.168.1.1".into(),
            port: 502,
            unit_id: 1,
            interval_ms: 1000,
            endianness: Endianness::Big,
            bind_addr: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ModbusConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.ip, config.ip);
        assert_eq!(parsed.port, config.port);
        assert_eq!(parsed.unit_id, config.unit_id);
        assert_eq!(parsed.endianness, config.endianness);
    }

    #[test]
    fn test_register_type_serde_lowercase() {
        let json = serde_json::to_string(&RegisterType::Float32).unwrap();
        assert_eq!(json, "\"float32\"");
        let parsed: RegisterType = serde_json::from_str("\"int16\"").unwrap();
        assert_eq!(parsed, RegisterType::Int16);
    }

    #[test]
    fn test_endianness_serde_roundtrip() {
        for end in [Endianness::Big, Endianness::Little, Endianness::Swapped] {
            let json = serde_json::to_string(&end).unwrap();
            let parsed: Endianness = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, end);
        }
    }

    #[test]
    fn test_parse_uint16_max() {
        assert_eq!(
            parse_register(&[0xFFFF], &RegisterType::Uint16, &Endianness::Big),
            65535.0
        );
    }

    // ── Boundary / edge-case tests ──

    #[test]
    fn test_parse_register_empty_input_returns_nan() {
        assert!(parse_register(&[], &RegisterType::Uint16, &Endianness::Big).is_nan());
        assert!(parse_register(&[], &RegisterType::Int16, &Endianness::Big).is_nan());
        assert!(parse_register(&[], &RegisterType::Int32, &Endianness::Big).is_nan());
        assert!(parse_register(&[], &RegisterType::Float32, &Endianness::Big).is_nan());
        assert!(parse_register(&[], &RegisterType::Float64, &Endianness::Big).is_nan());
    }

    #[test]
    fn test_parse_register_short_input_returns_nan() {
        // Int32/Float32 need 2 words — 1 is insufficient
        assert!(parse_register(&[0x1234], &RegisterType::Int32, &Endianness::Big).is_nan());
        assert!(parse_register(&[0x1234], &RegisterType::Float32, &Endianness::Big).is_nan());
        // Float64 needs 4 words — 3 is insufficient
        assert!(parse_register(&[1, 2, 3], &RegisterType::Float64, &Endianness::Big).is_nan());
    }

    #[test]
    fn test_parse_register_zero_words() {
        assert_eq!(parse_register(&[0x0000], &RegisterType::Uint16, &Endianness::Big), 0.0);
        assert_eq!(parse_register(&[0x0000], &RegisterType::Int16, &Endianness::Big), 0.0);
    }

    // Full round-trip for all types × all endianness
    #[test]
    fn test_roundtrip_all_types_all_endianness() {
        let test_values = [
            (RegisterType::Uint16, 1000.0_f64),
            (RegisterType::Int16, -100.0_f64),
            (RegisterType::Int32, -100000.0_f64),
            (RegisterType::Float32, 3.14_f64),
            (RegisterType::Float64, std::f64::consts::E),
        ];
        for end in [Endianness::Big, Endianness::Little, Endianness::Swapped] {
            for (reg_type, val) in &test_values {
                let words = value_to_words(*val, reg_type, &end);
                let parsed = parse_register(&words, reg_type, &end);
                if *reg_type == RegisterType::Float32 {
                    assert!((parsed - *val).abs() < 0.01,
                        "round-trip failed for {:?} + {:?}: {} != {}", reg_type, end, parsed, val);
                } else if *reg_type == RegisterType::Int16 || *reg_type == RegisterType::Uint16 {
                    assert_eq!(parsed, *val, "round-trip failed for {:?} + {:?}", reg_type, end);
                } else {
                    assert!((parsed - *val).abs() < 1.0,
                        "round-trip failed for {:?} + {:?}: {} != {}", reg_type, end, parsed, val);
                }
            }
        }
    }

    #[test]
    fn test_parse_float32_swapped_endianness() {
        let words = value_to_words(220.5, &RegisterType::Float32, &Endianness::Swapped);
        let parsed = parse_register(&words, &RegisterType::Float32, &Endianness::Swapped);
        assert!((parsed - 220.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_int32_little_endian() {
        let words = value_to_words(-50.0, &RegisterType::Int32, &Endianness::Little);
        let parsed = parse_register(&words, &RegisterType::Int32, &Endianness::Little);
        assert_eq!(parsed, -50.0);
    }

    #[test]
    fn test_modbus_config_defaults() {
        let json = r#"{"ip":"10.0.0.1"}"#;
        let config: ModbusConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.ip, "10.0.0.1");
        assert_eq!(config.port, 502);
        assert_eq!(config.unit_id, 1);
        assert_eq!(config.interval_ms, 1000);
        assert_eq!(config.endianness, Endianness::Big);
    }

    #[test]
    fn test_modbus_tag_serde_roundtrip() {
        let tag = ModbusTag {
            name: "temperature".into(),
            address: 40001,
            reg_type: RegisterType::Float32,
            scale: 0.1,
            register_area: "holding".into(),
        };
        let json = serde_json::to_string(&tag).unwrap();
        let parsed: ModbusTag = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, tag.name);
        assert_eq!(parsed.address, tag.address);
        assert_eq!(parsed.reg_type, tag.reg_type);
        assert_eq!(parsed.scale, tag.scale);
        assert_eq!(parsed.register_area, tag.register_area);
    }
}
