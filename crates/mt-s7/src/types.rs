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

// CPU / diagnostics function codes
pub(crate) const S7_MSG_TYPE_USER_DATA: u8 = 0x07;
pub(crate) const S7_CPU_FUNC: u8 = 0x00;
pub(crate) const S7_SZL_READ: u8 = 0x01;

// SZL IDs
pub(crate) const SZL_MODULE_IDENTITY: u16 = 0x0011;
pub(crate) const SZL_OPERATING_MODE: u16 = 0x0074;
pub(crate) const SZL_DIAG_BUFFER: u16 = 0x0000;

// Block management function codes
pub(crate) const S7_LIST_BLOCKS: u8 = 0x1A;
#[allow(dead_code)]
pub(crate) const S7_GET_BLOCK_INFO: u8 = 0x1B;
pub(crate) const S7_UPLOAD: u8 = 0x1C;
#[allow(dead_code)]
pub(crate) const S7_DOWNLOAD: u8 = 0x1D;
pub(crate) const S7_DELETE_BLOCK: u8 = 0x1E;

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
            _ => None,
        }
    }
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
        }
    }
}

// ── Diagnostics types ──

/// CPU module information read via SZL 0x0011.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    pub module_type: String,
    pub serial_number: String,
    pub as_name: String,
    pub module_name: String,
    pub firmware_version: String,
}

/// PLC operating mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlcMode {
    pub mode: String,
}

/// A single diagnostic buffer entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagEntry {
    pub event_number: u16,
    pub timestamp: String,
    pub event_text: String,
}

/// Diagnostic buffer contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagBuffer {
    pub entry_count: u16,
    pub entries: Vec<DiagEntry>,
}

// ── Block management types ──

/// S7 block type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub fn code(&self) -> u8 {
        match self {
            BlockType::OB => 0x38,
            BlockType::FB => 0x38,
            BlockType::FC => 0x38,
            BlockType::DB => 0x41,
            BlockType::SDB => 0x42,
            BlockType::SFC => 0x44,
            BlockType::SFB => 0x45,
        }
    }

    /// File prefix used in S7 block filenames (e.g. "_DB" for DB blocks).
    pub fn file_prefix(&self) -> &str {
        match self {
            BlockType::OB => "0A",
            BlockType::FB => "0E",
            BlockType::FC => "0C",
            BlockType::DB => "05",
            BlockType::SDB => "03",
            BlockType::SFC => "0D",
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
    pub block_type: String,
    pub block_number: u16,
}

/// Detailed information about a single block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    pub block_type: String,
    pub block_number: u16,
    pub language: String,
    pub size: u32,
}
