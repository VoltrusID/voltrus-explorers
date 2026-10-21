use serde::{Deserialize, Serialize};

/// Connection configuration for a Modbus TCP device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModbusConfig {
    /// IP address of the Modbus TCP device.
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
    /// Connect and per-request timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Word order for multi-register values.
    #[serde(default)]
    pub endianness: Endianness,
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
fn default_timeout_ms() -> u64 {
    5000
}

/// A Modbus register tag describing what to read or write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModbusTag {
    /// Human-readable tag name.
    pub name: String,
    /// 0-based protocol address (the on-the-wire address, not 40001-style).
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
    /// Whether the tag lives in a single-bit area (coils / discrete inputs).
    pub fn is_bit_area(&self) -> bool {
        self.register_area == "coil" || self.register_area == "discrete"
    }

    /// Number of protocol addresses this tag occupies: one bit for coil and
    /// discrete areas, otherwise the number of 16-bit registers of its type.
    pub fn register_count(&self) -> u16 {
        if self.is_bit_area() {
            1
        } else {
            self.reg_type.word_count() as u16
        }
    }

    /// Return the 0-based protocol address.
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
    /// The device answered the request covering this tag with an exception
    /// (or an unusable response); carries a human-readable description.
    Failed(String),
}

impl ModbusValue {
    /// Convert to f64 regardless of the stored type (`NaN` for [`ModbusValue::Failed`]).
    pub fn as_f64(&self) -> f64 {
        match self {
            ModbusValue::Uint16(v) => *v as f64,
            ModbusValue::Int16(v) => *v as f64,
            ModbusValue::Int32(v) => *v as f64,
            ModbusValue::Float32(v) => *v as f64,
            ModbusValue::Float64(v) => *v,
            ModbusValue::Failed(_) => f64::NAN,
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

impl RegisterType {
    /// Number of 16-bit registers a value of this type spans.
    pub fn word_count(self) -> usize {
        match self {
            RegisterType::Int16 | RegisterType::Uint16 => 1,
            RegisterType::Int32 | RegisterType::Float32 => 2,
            RegisterType::Float64 => 4,
        }
    }
}

/// Word/byte order for multi-register values. Modbus itself only defines the
/// order of the two bytes inside one register (big-endian); how a 32/64-bit
/// value is spread over several registers is up to the device.
///
/// Notation: a 32-bit value is bytes `A B C D` (A most significant); a 64-bit
/// value is `A B C D E F G H`.
///
/// * `Big` ("AB CD" / "AB CD EF GH"): registers hold the value most
///   significant word first, bytes unchanged.
/// * `Little` ("CD AB" / "GH EF CD AB"): the order of the *registers* is
///   reversed; the two bytes inside each register stay as they are.
/// * `Swapped` ("BA DC" / "BA DC FE HG"): the two bytes inside each register
///   are swapped; the order of the registers is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Endianness {
    #[default]
    Big,
    Little,
    Swapped,
}

/// Convert the `N` registers as they appear on the wire into most-significant-first registers.
fn wire_to_canonical<const N: usize>(words: &[u16], endianness: &Endianness) -> [u16; N] {
    let mut canonical = [0u16; N];
    for (i, slot) in canonical.iter_mut().enumerate() {
        *slot = match endianness {
            Endianness::Big => words[i],
            Endianness::Little => words[N - 1 - i],
            Endianness::Swapped => words[i].swap_bytes(),
        };
    }
    canonical
}

/// Inverse of [`wire_to_canonical`]: lay most-significant-first registers out as they go on the wire.
fn canonical_to_wire<const N: usize>(canonical: [u16; N], endianness: &Endianness) -> Vec<u16> {
    (0..N)
        .map(|i| match endianness {
            Endianness::Big => canonical[i],
            Endianness::Little => canonical[N - 1 - i],
            Endianness::Swapped => canonical[i].swap_bytes(),
        })
        .collect()
}

fn join32(c: [u16; 2]) -> u32 {
    ((c[0] as u32) << 16) | c[1] as u32
}

fn join64(c: [u16; 4]) -> u64 {
    ((c[0] as u64) << 48) | ((c[1] as u64) << 32) | ((c[2] as u64) << 16) | c[3] as u64
}

fn split32(raw: u32) -> [u16; 2] {
    [(raw >> 16) as u16, raw as u16]
}

fn split64(raw: u64) -> [u16; 4] {
    [(raw >> 48) as u16, (raw >> 32) as u16, (raw >> 16) as u16, raw as u16]
}

/// Parse raw 16-bit register words (as received) into an f64 value.
/// Returns `f64::NAN` if `words` is shorter than the type requires.
pub fn parse_register(words: &[u16], reg_type: &RegisterType, endianness: &Endianness) -> f64 {
    if words.len() < reg_type.word_count() {
        return f64::NAN;
    }
    match reg_type {
        RegisterType::Uint16 => words[0] as f64,
        RegisterType::Int16 => (words[0] as i16) as f64,
        RegisterType::Int32 => join32(wire_to_canonical::<2>(words, endianness)) as i32 as f64,
        RegisterType::Float32 => {
            f32::from_bits(join32(wire_to_canonical::<2>(words, endianness))) as f64
        }
        RegisterType::Float64 => {
            f64::from_bits(join64(wire_to_canonical::<4>(words, endianness)))
        }
    }
}

/// Check that `value` can be written to a register of `reg_type` without
/// being truncated, wrapped or clamped.
pub fn check_writable(value: f64, reg_type: &RegisterType) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{value} is not a finite number"));
    }
    let integer_in = |min: f64, max: f64, name: &str| {
        if value.fract() != 0.0 {
            Err(format!("{value} is not a whole number (required for {name})"))
        } else if value < min || value > max {
            Err(format!("{value} is outside the {name} range {min}..{max}"))
        } else {
            Ok(())
        }
    };
    match reg_type {
        RegisterType::Uint16 => integer_in(0.0, u16::MAX as f64, "uint16"),
        RegisterType::Int16 => integer_in(i16::MIN as f64, i16::MAX as f64, "int16"),
        RegisterType::Int32 => integer_in(i32::MIN as f64, i32::MAX as f64, "int32"),
        RegisterType::Float32 => {
            if value.abs() > f32::MAX as f64 {
                Err(format!("{value} is outside the float32 range"))
            } else {
                Ok(())
            }
        }
        RegisterType::Float64 => Ok(()),
    }
}

/// Convert an f64 to raw u16 register words for writing. The caller is
/// expected to have validated the value with [`check_writable`].
pub fn value_to_words(value: f64, reg_type: &RegisterType, endianness: &Endianness) -> Vec<u16> {
    match reg_type {
        RegisterType::Uint16 => vec![value as u16],
        RegisterType::Int16 => vec![(value as i16) as u16],
        RegisterType::Int32 => canonical_to_wire(split32((value as i32) as u32), endianness),
        RegisterType::Float32 => canonical_to_wire(split32((value as f32).to_bits()), endianness),
        RegisterType::Float64 => canonical_to_wire(split64(value.to_bits()), endianness),
    }
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

/// Result of a bus scan operation — a device that responded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusScanResult {
    /// Modbus unit/slave ID that responded.
    pub unit_id: u8,
    /// Raw register value read from the device.
    pub value: u16,
}

/// Configuration for running as a Modbus slave (server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaveConfig {
    #[serde(default = "default_modbus_port")]
    pub port: u16,
    /// Unit ID the slave answers to.
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

    /// Decode the register payload of an FC03/FC04 response PDU
    /// (`fc, byte_count, data...`): each register is two bytes, high byte first.
    fn registers_from_pdu(pdu: &[u8]) -> Vec<u16> {
        assert_eq!(pdu[1] as usize, pdu.len() - 2, "byte count must match the payload");
        pdu[2..]
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect()
    }

    const ALL: [Endianness; 3] = [Endianness::Big, Endianness::Little, Endianness::Swapped];

    // ── 16-bit ──

    #[test]
    fn uint16_and_int16_ignore_word_order() {
        for e in ALL {
            assert_eq!(parse_register(&[1000], &RegisterType::Uint16, &e), 1000.0);
            assert_eq!(parse_register(&[0xFFFF], &RegisterType::Uint16, &e), 65535.0);
            // 0xFFFE two's complement = -2
            assert_eq!(parse_register(&[0xFFFE], &RegisterType::Int16, &e), -2.0);
            assert_eq!(value_to_words(-2.0, &RegisterType::Int16, &e), vec![0xFFFE]);
        }
    }

    // ── 32-bit: FC03 response `03 04 <4 data bytes>` ──

    #[test]
    fn float32_220_5_all_word_orders() {
        // 220.5f32 = 0x435C8000 -> bytes A=43 B=5C C=80 D=00
        let cases = [
            // AB CD: registers 0x435C 0x8000
            (Endianness::Big, [0x03, 0x04, 0x43, 0x5C, 0x80, 0x00]),
            // CD AB: registers 0x8000 0x435C
            (Endianness::Little, [0x03, 0x04, 0x80, 0x00, 0x43, 0x5C]),
            // BA DC: registers 0x5C43 0x0080
            (Endianness::Swapped, [0x03, 0x04, 0x5C, 0x43, 0x00, 0x80]),
        ];
        for (e, pdu) in cases {
            let words = registers_from_pdu(&pdu);
            assert_eq!(parse_register(&words, &RegisterType::Float32, &e), 220.5, "{e:?}");
            assert_eq!(value_to_words(220.5, &RegisterType::Float32, &e), words, "{e:?}");
        }
    }

    #[test]
    fn int32_0x12345678_all_word_orders() {
        // 0x12345678 = 305419896 -> A=12 B=34 C=56 D=78
        let cases = [
            (Endianness::Big, [0x03, 0x04, 0x12, 0x34, 0x56, 0x78]),
            (Endianness::Little, [0x03, 0x04, 0x56, 0x78, 0x12, 0x34]),
            (Endianness::Swapped, [0x03, 0x04, 0x34, 0x12, 0x78, 0x56]),
        ];
        for (e, pdu) in cases {
            let words = registers_from_pdu(&pdu);
            assert_eq!(parse_register(&words, &RegisterType::Int32, &e), 305419896.0, "{e:?}");
            assert_eq!(value_to_words(305419896.0, &RegisterType::Int32, &e), words, "{e:?}");
        }
    }

    #[test]
    fn int32_negative_all_word_orders() {
        // -2 = 0xFFFFFFFE -> A=FF B=FF C=FF D=FE
        let cases = [
            (Endianness::Big, [0x03, 0x04, 0xFF, 0xFF, 0xFF, 0xFE]),
            (Endianness::Little, [0x03, 0x04, 0xFF, 0xFE, 0xFF, 0xFF]),
            (Endianness::Swapped, [0x03, 0x04, 0xFF, 0xFF, 0xFE, 0xFF]),
        ];
        for (e, pdu) in cases {
            let words = registers_from_pdu(&pdu);
            assert_eq!(parse_register(&words, &RegisterType::Int32, &e), -2.0, "{e:?}");
            assert_eq!(value_to_words(-2.0, &RegisterType::Int32, &e), words, "{e:?}");
        }
    }

    // ── 64-bit: FC03 response `03 08 <8 data bytes>` ──

    #[test]
    fn float64_pi_all_word_orders() {
        // pi = 0x400921FB54442D18 -> A=40 B=09 C=21 D=FB E=54 F=44 G=2D H=18
        let cases = [
            // AB CD EF GH
            (
                Endianness::Big,
                [0x03, 0x08, 0x40, 0x09, 0x21, 0xFB, 0x54, 0x44, 0x2D, 0x18],
            ),
            // GH EF CD AB (registers reversed, not pairwise swapped)
            (
                Endianness::Little,
                [0x03, 0x08, 0x2D, 0x18, 0x54, 0x44, 0x21, 0xFB, 0x40, 0x09],
            ),
            // BA DC FE HG
            (
                Endianness::Swapped,
                [0x03, 0x08, 0x09, 0x40, 0xFB, 0x21, 0x44, 0x54, 0x18, 0x2D],
            ),
        ];
        for (e, pdu) in cases {
            let words = registers_from_pdu(&pdu);
            assert_eq!(
                parse_register(&words, &RegisterType::Float64, &e),
                std::f64::consts::PI,
                "{e:?}"
            );
            assert_eq!(
                value_to_words(std::f64::consts::PI, &RegisterType::Float64, &e),
                words,
                "{e:?}"
            );
        }
    }

    #[test]
    fn float64_1_5_little_is_fully_reversed() {
        // 1.5 = 0x3FF8000000000000: only the first register is non-zero on the wire
        // for AB CD EF GH, and only the last for GH EF CD AB.
        let big = registers_from_pdu(&[0x03, 0x08, 0x3F, 0xF8, 0, 0, 0, 0, 0, 0]);
        let little = registers_from_pdu(&[0x03, 0x08, 0, 0, 0, 0, 0, 0, 0x3F, 0xF8]);
        assert_eq!(parse_register(&big, &RegisterType::Float64, &Endianness::Big), 1.5);
        assert_eq!(parse_register(&little, &RegisterType::Float64, &Endianness::Little), 1.5);
        // The same registers read with the other order are a different number.
        assert_ne!(parse_register(&little, &RegisterType::Float64, &Endianness::Big), 1.5);
        assert_eq!(value_to_words(1.5, &RegisterType::Float64, &Endianness::Little), little);
    }

    #[test]
    fn float64_negative_swapped() {
        // -2.5 = 0xC004000000000000 -> BA DC FE HG: 04C0 0000 0000 0000
        let words = registers_from_pdu(&[0x03, 0x08, 0x04, 0xC0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(parse_register(&words, &RegisterType::Float64, &Endianness::Swapped), -2.5);
        assert_eq!(value_to_words(-2.5, &RegisterType::Float64, &Endianness::Swapped), words);
    }

    // ── short input ──

    #[test]
    fn short_input_is_nan() {
        assert!(parse_register(&[], &RegisterType::Uint16, &Endianness::Big).is_nan());
        assert!(parse_register(&[0x1234], &RegisterType::Int32, &Endianness::Big).is_nan());
        assert!(parse_register(&[0x1234], &RegisterType::Float32, &Endianness::Little).is_nan());
        assert!(parse_register(&[1, 2, 3], &RegisterType::Float64, &Endianness::Swapped).is_nan());
    }

    // ── write validation ──

    #[test]
    fn check_writable_rejects_values_that_would_be_altered() {
        assert!(check_writable(3.5, &RegisterType::Uint16).is_err());
        assert!(check_writable(70000.0, &RegisterType::Uint16).is_err());
        assert!(check_writable(-1.0, &RegisterType::Uint16).is_err());
        assert!(check_writable(-32769.0, &RegisterType::Int16).is_err());
        assert!(check_writable(2147483648.0, &RegisterType::Int32).is_err());
        assert!(check_writable(1e39, &RegisterType::Float32).is_err());
        assert!(check_writable(f64::NAN, &RegisterType::Float64).is_err());
        assert!(check_writable(f64::INFINITY, &RegisterType::Float64).is_err());

        assert!(check_writable(65535.0, &RegisterType::Uint16).is_ok());
        assert!(check_writable(-32768.0, &RegisterType::Int16).is_ok());
        assert!(check_writable(-2147483648.0, &RegisterType::Int32).is_ok());
        assert!(check_writable(3.5, &RegisterType::Float32).is_ok());
        assert!(check_writable(1e300, &RegisterType::Float64).is_ok());
    }

    // ── tags ──

    fn tag(address: u16, reg_type: RegisterType, area: &str) -> ModbusTag {
        ModbusTag {
            name: "t".into(),
            address,
            reg_type,
            scale: 1.0,
            register_area: area.into(),
        }
    }

    #[test]
    fn register_count_follows_type_and_area() {
        assert_eq!(tag(0, RegisterType::Uint16, "holding").register_count(), 1);
        assert_eq!(tag(0, RegisterType::Float32, "input").register_count(), 2);
        assert_eq!(tag(0, RegisterType::Float64, "holding").register_count(), 4);
        // Bit areas always occupy one address, whatever type was configured.
        assert_eq!(tag(0, RegisterType::Float64, "coil").register_count(), 1);
        assert_eq!(tag(0, RegisterType::Int32, "discrete").register_count(), 1);
    }

    #[test]
    fn protocol_address_is_zero_based_as_given() {
        assert_eq!(tag(0, RegisterType::Float32, "holding").protocol_address(), 0);
        assert_eq!(tag(2999, RegisterType::Float32, "holding").protocol_address(), 2999);
    }

    #[test]
    fn failed_value_is_nan() {
        assert!(ModbusValue::Failed("x".into()).as_f64().is_nan());
        assert_eq!(ModbusValue::Uint16(100).as_f64(), 100.0);
        assert_eq!(ModbusValue::Int16(-1).as_f64(), -1.0);
        assert_eq!(ModbusValue::Float64(220.5).as_f64(), 220.5);
    }

    // ── serde ──

    #[test]
    fn config_defaults() {
        let config: ModbusConfig = serde_json::from_str(r#"{"ip":"10.0.0.1"}"#).unwrap();
        assert_eq!(config.ip, "10.0.0.1");
        assert_eq!(config.port, 502);
        assert_eq!(config.unit_id, 1);
        assert_eq!(config.interval_ms, 1000);
        assert_eq!(config.timeout_ms, 5000);
        assert_eq!(config.endianness, Endianness::Big);
    }

    #[test]
    fn config_accepts_frontend_payload() {
        let json = r#"{"host":"127.0.0.1","port":5020,"unit_id":3,"timeout_ms":2500,"poll_interval_ms":500,"endianness":"swapped"}"#;
        let config: ModbusConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.ip, "127.0.0.1");
        assert_eq!(config.port, 5020);
        assert_eq!(config.unit_id, 3);
        assert_eq!(config.timeout_ms, 2500);
        assert_eq!(config.endianness, Endianness::Swapped);
    }

    #[test]
    fn register_type_serde_lowercase() {
        assert_eq!(serde_json::to_string(&RegisterType::Float32).unwrap(), "\"float32\"");
        let parsed: RegisterType = serde_json::from_str("\"int16\"").unwrap();
        assert_eq!(parsed, RegisterType::Int16);
        // "string" is not a register type.
        assert!(serde_json::from_str::<RegisterType>("\"string\"").is_err());
    }

    #[test]
    fn endianness_serde_names() {
        for (e, name) in [
            (Endianness::Big, "\"big\""),
            (Endianness::Little, "\"little\""),
            (Endianness::Swapped, "\"swapped\""),
        ] {
            assert_eq!(serde_json::to_string(&e).unwrap(), name);
            assert_eq!(serde_json::from_str::<Endianness>(name).unwrap(), e);
        }
    }

    #[test]
    fn tag_serde_roundtrip() {
        let t = ModbusTag {
            name: "temperature".into(),
            address: 3025,
            reg_type: RegisterType::Float32,
            scale: 0.1,
            register_area: "holding".into(),
        };
        let parsed: ModbusTag = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(parsed.name, t.name);
        assert_eq!(parsed.address, t.address);
        assert_eq!(parsed.reg_type, t.reg_type);
        assert_eq!(parsed.scale, t.scale);
        assert_eq!(parsed.register_area, t.register_area);
    }
}
