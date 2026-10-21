//! S7 protocol types: configuration, tag addressing, data types, and values.

use serde::{Deserialize, Serialize};

// ── Protocol constants ──

pub(crate) const TPKT_VERSION: u8 = 3;
pub(crate) const COTP_CR: u8 = 0xE0;
pub(crate) const COTP_CC: u8 = 0xD0;
pub(crate) const COTP_DT: u8 = 0xF0;

pub(crate) const S7_PROTOCOL_ID: u8 = 0x32;
pub(crate) const S7_MSG_TYPE_JOB: u8 = 0x01;
#[allow(dead_code)]
pub(crate) const S7_MSG_TYPE_ACK_DATA: u8 = 0x03;

pub(crate) const S7_SETUP_COMM: u8 = 0xF0;
pub(crate) const S7_READ_VAR: u8 = 0x04;
pub(crate) const S7_WRITE_VAR: u8 = 0x05;

/// Message type of "user data" PDUs: SZL reads and block listing travel in these.
pub(crate) const S7_MSG_TYPE_USER_DATA: u8 = 0x07;

// Job functions of block upload and PLC control (snap7 s7_types.h: pduStartUpload, pduUpload,
// pduEndUpload, pduControl).
pub(crate) const S7_START_UPLOAD: u8 = 0x1D;
pub(crate) const S7_UPLOAD: u8 = 0x1E;
pub(crate) const S7_END_UPLOAD: u8 = 0x1F;
/// PI service ("plc control"): deletes and inserts blocks, starts and stops the CPU.
pub(crate) const S7_PLC_CONTROL: u8 = 0x28;

// Area codes
pub(crate) const AREA_DB: u8 = 0x84;
pub(crate) const AREA_INPUT: u8 = 0x81;
pub(crate) const AREA_OUTPUT: u8 = 0x82;
pub(crate) const AREA_MARKER: u8 = 0x83;
pub(crate) const AREA_TIMER: u8 = 0x1D;
pub(crate) const AREA_COUNTER: u8 = 0x1C;

// Transport sizes
pub(crate) const TS_BIT: u8 = 0x01;
pub(crate) const TS_BYTE: u8 = 0x02;
pub(crate) const TS_WORD: u8 = 0x04;
pub(crate) const TS_DWORD: u8 = 0x06;
pub(crate) const TS_REAL: u8 = 0x08;
/// S7 counter (area 0x1C). Elements are 2 bytes each and addressed by number.
pub(crate) const TS_COUNTER: u8 = 0x1C;
/// S7 timer (area 0x1D). Elements are 2 bytes each and addressed by number.
pub(crate) const TS_TIMER: u8 = 0x1D;

// S7 error codes
pub(crate) const S7_ERR_NONE: u16 = 0x0000;

// Default PDU sizes
pub const DEFAULT_PDU_SIZE: u16 = 480;
pub const MAX_PDU_SIZE: u16 = 960;

// ── Configuration ──

/// Configuration required to connect to an S7 PLC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S7Config {
    /// Device identifier (for logging / multiplexing).
    #[serde(default)]
    pub id: String,
    /// PLC IP address or hostname.
    pub host: String,
    /// PLC TCP port (typically 102 for ISO-on-TCP).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Rack number (usually 0).
    #[serde(default)]
    pub rack: u8,
    /// Slot number (usually 1 for S7-300/400, 0 for S7-1200/1500).
    #[serde(default = "default_slot")]
    pub slot: u8,
    /// Polling interval in milliseconds (only relevant for poller use-cases).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
}

fn default_port() -> u16 {
    102
}
fn default_slot() -> u8 {
    1
}
fn default_poll_interval() -> u64 {
    1000
}

// ── Area enum ──

/// Memory area of the S7 PLC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S7Area {
    DB,
    Input,
    Output,
    Marker,
    Timer,
    Counter,
}

impl S7Area {
    pub fn code(&self) -> u8 {
        match self {
            S7Area::DB => AREA_DB,
            S7Area::Input => AREA_INPUT,
            S7Area::Output => AREA_OUTPUT,
            S7Area::Marker => AREA_MARKER,
            S7Area::Timer => AREA_TIMER,
            S7Area::Counter => AREA_COUNTER,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            AREA_DB => Some(S7Area::DB),
            AREA_INPUT => Some(S7Area::Input),
            AREA_OUTPUT => Some(S7Area::Output),
            AREA_MARKER => Some(S7Area::Marker),
            AREA_TIMER => Some(S7Area::Timer),
            AREA_COUNTER => Some(S7Area::Counter),
            _ => None,
        }
    }
}

impl std::fmt::Display for S7Area {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            S7Area::DB => write!(f, "DB"),
            S7Area::Input => write!(f, "Input"),
            S7Area::Output => write!(f, "Output"),
            S7Area::Marker => write!(f, "Marker"),
            S7Area::Timer => write!(f, "Timer"),
            S7Area::Counter => write!(f, "Counter"),
        }
    }
}

// ── Data type enum ──

/// S7 PLC data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S7DataType {
    Bool,
    Byte,
    Word,
    DWord,
    Real,
    String,
    /// S7 timer (S5 time format). Read as milliseconds. Only valid for the timer area.
    Timer,
    /// S7 counter (BCD, 0 to 999). Only valid for the counter area.
    Counter,
}

impl S7DataType {
    pub fn transport_size(&self) -> u8 {
        match self {
            S7DataType::Bool => TS_BIT,
            S7DataType::Byte => TS_BYTE,
            S7DataType::Word => TS_WORD,
            S7DataType::DWord => TS_DWORD,
            S7DataType::Real => TS_REAL,
            S7DataType::String => TS_BYTE, // Strings transported as byte arrays
            S7DataType::Timer => TS_TIMER,
            S7DataType::Counter => TS_COUNTER,
        }
    }

    pub fn byte_size(&self) -> u16 {
        match self {
            S7DataType::Bool => 1,
            S7DataType::Byte => 1,
            S7DataType::Word => 2,
            S7DataType::DWord => 4,
            S7DataType::Real => 4,
            S7DataType::String => 1, // Per character; length determines total
            S7DataType::Timer | S7DataType::Counter => 2,
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bool" | "bit" => Some(S7DataType::Bool),
            "byte" | "uint8" => Some(S7DataType::Byte),
            "word" | "uint16" | "int16" => Some(S7DataType::Word),
            "dword" | "uint32" | "int32" => Some(S7DataType::DWord),
            "real" | "float" | "float32" => Some(S7DataType::Real),
            "string" | "str" => Some(S7DataType::String),
            "timer" => Some(S7DataType::Timer),
            "counter" => Some(S7DataType::Counter),
            _ => None,
        }
    }
}

// ── Timer and counter encoding ──

/// Decode three BCD digits from the low 12 bits of `word`.
fn bcd3(word: u16) -> u16 {
    ((word >> 8) & 0xF) * 100 + ((word >> 4) & 0xF) * 10 + (word & 0xF)
}

/// Encode 0..=999 as three BCD digits (larger values are clamped to 999).
fn to_bcd3(value: u16) -> u16 {
    let v = value.min(999);
    ((v / 100) << 8) | (((v / 10) % 10) << 4) | (v % 10)
}

/// Time base in milliseconds for the 2-bit base code in bits 12-13 of an S7 timer word.
const TIMER_BASE_MS: [u32; 4] = [10, 100, 1000, 10_000];

/// Decode an S7 timer word to milliseconds: BCD value (bits 0-11) times the time base (bits 12-13).
pub(crate) fn decode_timer_ms(word: u16) -> u32 {
    bcd3(word) as u32 * TIMER_BASE_MS[((word >> 12) & 0x3) as usize]
}

/// Encode milliseconds as an S7 timer word, using the finest time base that fits 999 steps.
/// Values above 9,990,000 ms are clamped.
pub(crate) fn encode_timer_ms(ms: u32) -> u16 {
    let base_code = TIMER_BASE_MS
        .iter()
        .position(|base| ms.div_ceil(*base) <= 999)
        .unwrap_or(3);
    let steps = (ms.saturating_add(TIMER_BASE_MS[base_code] / 2) / TIMER_BASE_MS[base_code]).min(999) as u16;
    ((base_code as u16) << 12) | to_bcd3(steps)
}

/// Decode an S7 counter word: BCD 0..=999 in the low 12 bits.
pub(crate) fn decode_counter(word: u16) -> u16 {
    bcd3(word)
}

/// Encode a counter value as an S7 counter word (clamped to 999).
pub(crate) fn encode_counter(value: u16) -> u16 {
    to_bcd3(value)
}

// ── Tag ──

/// Describes a single tag (address) to read from or write to on the PLC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S7Tag {
    pub area: S7Area,
    pub db_number: u16,
    pub offset: u16,
    pub bit_offset: u8,
    pub data_type: S7DataType,
    pub length: u16,
    /// Optional human-readable name (e.g. "voltage", "current").
    /// Falls back to "{area}_{offset}" when absent.
    #[serde(default)]
    pub name: Option<String>,
}

// ── Value ──

/// A value read from (or to be written to) an S7 tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum S7Value {
    Bool(bool),
    Byte(u8),
    Word(u16),
    DWord(u32),
    Real(f32),
    String(String),
    /// Timer value in milliseconds.
    Timer(u32),
    /// Counter value, 0 to 999.
    Counter(u16),
}

impl S7Value {
    pub fn to_f64(&self) -> f64 {
        match self {
            S7Value::Bool(v) => {
                if *v {
                    1.0
                } else {
                    0.0
                }
            }
            S7Value::Byte(v) => *v as f64,
            S7Value::Word(v) => *v as f64,
            S7Value::DWord(v) => *v as f64,
            S7Value::Real(v) => *v as f64,
            S7Value::String(_) => 0.0,
            S7Value::Timer(v) => *v as f64,
            S7Value::Counter(v) => *v as f64,
        }
    }
}

// ── Diagnostics types ──

/// CPU identity, read from the module identification (SZL 0x0011) and component
/// identification (SZL 0x001C) lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    /// Module type name, e.g. `CPU 315-2 PN/DP` (SZL 0x001C, index 7).
    pub module_type: String,
    /// Serial number of the module (SZL 0x001C, index 5).
    pub serial_number: String,
    /// Name of the automation system, i.e. the station (SZL 0x001C, index 1).
    pub as_name: String,
    /// Name of the module as configured (SZL 0x001C, index 2).
    pub module_name: String,
    /// Firmware version as `V3.2.6` (SZL 0x0011, index 7).
    pub firmware_version: String,
    /// Order number (MLFB) of the module, e.g. `6ES7 151-8AB01-0AB0` (SZL 0x0011, index 1).
    #[serde(default)]
    pub order_code: String,
    /// Copyright entry of the module (SZL 0x001C, index 4).
    #[serde(default)]
    pub copyright: String,
}

/// PLC operating mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlcMode {
    /// `RUN`, `STOP`, `STARTUP`, `HOLD`, `DEFECT` or `UNKNOWN`.
    pub mode: String,
}

/// A single diagnostic buffer entry (SZL 0x00A0, one 20-byte record).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagEntry {
    /// Event ID.
    pub event_number: u16,
    /// Time the CPU logged the event, `YYYY-MM-DD HH:MM:SS.mmm`.
    pub timestamp: String,
    /// Event class and, for the events this crate knows, what happened.
    pub event_text: String,
    /// Priority class of the OB that was running.
    #[serde(default)]
    pub priority_class: u8,
    /// Number of that OB.
    #[serde(default)]
    pub ob_number: u8,
    /// Data ID.
    #[serde(default)]
    pub data_id: u16,
    /// First additional information word.
    #[serde(default)]
    pub info1: u16,
    /// Second additional information double word.
    #[serde(default)]
    pub info2: u32,
}

/// Diagnostic buffer contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagBuffer {
    /// Number of entries the PLC returned.
    pub entry_count: u16,
    /// Newest first, as the PLC keeps them.
    pub entries: Vec<DiagEntry>,
}

// ── Block management types ──

/// S7 block type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    OB,
    FB,
    FC,
    DB,
    SDB,
    SFC,
    SFB,
}

impl BlockType {
    /// Every block type, in the order a PLC lists them.
    pub const ALL: [BlockType; 7] = [
        BlockType::OB,
        BlockType::FB,
        BlockType::FC,
        BlockType::DB,
        BlockType::SDB,
        BlockType::SFC,
        BlockType::SFB,
    ];

    /// The block type code of S7 block telegrams: the ASCII character after the leading `0` of
    /// the two-character sub block type (snap7 `Block_OB` ... `Block_SFB`).
    pub fn code(&self) -> u8 {
        match self {
            BlockType::OB => 0x38,  // '8'
            BlockType::DB => 0x41,  // 'A'
            BlockType::SDB => 0x42, // 'B'
            BlockType::FC => 0x43,  // 'C'
            BlockType::SFC => 0x44, // 'D'
            BlockType::FB => 0x45,  // 'E'
            BlockType::SFB => 0x46, // 'F'
        }
    }

    /// The block type for a [`code`](Self::code), if it is one.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.code() == code)
    }

    /// Sub block type as two ASCII characters, the part of a block file name (`_0A00001A`) that
    /// says which kind of block it is: `08` OB, `0A` DB, `0B` SDB, `0C` FC, `0D` SFC, `0E` FB,
    /// `0F` SFB (snap7 `SubBlk_*`).
    pub fn file_prefix(&self) -> &'static str {
        match self {
            BlockType::OB => "08",
            BlockType::DB => "0A",
            BlockType::SDB => "0B",
            BlockType::FC => "0C",
            BlockType::SFC => "0D",
            BlockType::FB => "0E",
            BlockType::SFB => "0F",
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "OB" => Some(BlockType::OB),
            "FB" => Some(BlockType::FB),
            "FC" => Some(BlockType::FC),
            "DB" => Some(BlockType::DB),
            "SDB" => Some(BlockType::SDB),
            "SFC" => Some(BlockType::SFC),
            "SFB" => Some(BlockType::SFB),
            _ => None,
        }
    }
}

impl std::fmt::Display for BlockType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockType::OB => write!(f, "OB"),
            BlockType::FB => write!(f, "FB"),
            BlockType::FC => write!(f, "FC"),
            BlockType::DB => write!(f, "DB"),
            BlockType::SDB => write!(f, "SDB"),
            BlockType::SFC => write!(f, "SFC"),
            BlockType::SFB => write!(f, "SFB"),
        }
    }
}

/// A single block entry from the PLC block list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockListEntry {
    /// `OB`, `FB`, `FC`, `DB`, `SDB`, `SFC` or `SFB`.
    pub block_type: String,
    pub block_number: u16,
    /// The PLC's flag byte for the block, as it sent it. Its meaning is not documented.
    #[serde(default)]
    pub flags: u8,
    /// Language the block is written in: `AWL`, `KOP`, `FUP`, `SCL`, `DB`, `GRAPH`, `SDB`, ...
    #[serde(default)]
    pub language: String,
}

/// Detailed information about a single block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    pub block_type: String,
    pub block_number: u16,
    pub language: String,
    pub size: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_bcd_round_trip() {
        for v in [0u16, 1, 9, 10, 99, 100, 247, 999] {
            assert_eq!(decode_counter(encode_counter(v)), v);
        }
        assert_eq!(encode_counter(247), 0x0247);
        assert_eq!(encode_counter(5000), 0x0999, "clamped to 999");
    }

    #[test]
    fn timer_encoding_picks_the_finest_time_base() {
        assert_eq!(encode_timer_ms(0), 0x0000);
        assert_eq!(encode_timer_ms(5_000), 0x0500); // 500 x 10 ms
        assert_eq!(encode_timer_ms(9_990), 0x0999);
        assert_eq!(encode_timer_ms(12_300), 0x1123); // 123 x 100 ms
        assert_eq!(encode_timer_ms(123_000), 0x2123); // 123 x 1 s
        assert_eq!(encode_timer_ms(1_230_000), 0x3123); // 123 x 10 s
        assert_eq!(encode_timer_ms(u32::MAX), 0x3999, "clamped");
    }

    #[test]
    fn timer_round_trip_is_exact_on_the_time_base() {
        for ms in [0u32, 10, 990, 9_990, 10_000, 99_900, 123_000, 9_990_000] {
            assert_eq!(decode_timer_ms(encode_timer_ms(ms)), ms);
        }
    }

    #[test]
    fn timer_decode_uses_only_the_base_and_value_bits() {
        // Bits 14-15 are not part of the value.
        assert_eq!(decode_timer_ms(0xC000 | 0x2123), 123_000);
    }

    #[test]
    fn data_type_names() {
        assert_eq!(S7DataType::from_str_name("Timer"), Some(S7DataType::Timer));
        assert_eq!(S7DataType::from_str_name("COUNTER"), Some(S7DataType::Counter));
        assert_eq!(S7DataType::Timer.transport_size(), 0x1D);
        assert_eq!(S7DataType::Counter.transport_size(), 0x1C);
    }

    /// The app and saved data have the old records; the fields added since default.
    #[test]
    fn records_without_the_added_fields_still_deserialize() {
        let cpu: CpuInfo = serde_json::from_str(
            r#"{"module_type":"a","serial_number":"b","as_name":"c","module_name":"d","firmware_version":"e"}"#,
        )
        .unwrap();
        assert_eq!((cpu.order_code.as_str(), cpu.copyright.as_str()), ("", ""));
        let entry: DiagEntry =
            serde_json::from_str(r#"{"event_number":1,"timestamp":"t","event_text":"x"}"#).unwrap();
        assert_eq!((entry.priority_class, entry.info2), (0, 0));
        let block: BlockListEntry = serde_json::from_str(r#"{"block_type":"DB","block_number":1}"#).unwrap();
        assert_eq!((block.flags, block.language.as_str()), (0, ""));
    }
}
