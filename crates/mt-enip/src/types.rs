#![allow(dead_code)]
use serde::{Deserialize, Serialize};

// ── EtherNet/IP command codes ──

pub(crate) const CMD_REGISTER_SESSION: u16 = 0x0065;
pub(crate) const CMD_UNREGISTER_SESSION: u16 = 0x0066;
pub(crate) const CMD_SEND_RR_DATA: u16 = 0x006F;
pub(crate) const _CMD_SEND_UNIT_DATA: u16 = 0x0070;

// ── CIP service codes ──

pub(crate) const CIP_READ_TAG: u8 = 0x4C;
pub(crate) const CIP_READ_TAG_FRAGMENTED: u8 = 0x52;
pub(crate) const CIP_WRITE_TAG: u8 = 0x4D;
pub(crate) const CIP_GET_ATTRIBUTES_ALL: u8 = 0x01;
pub(crate) const _CIP_FORWARD_OPEN: u8 = 0x54;

// ── CIP data type codes ──

pub(crate) const CIP_TYPE_BOOL: u8 = 0xC1;
pub(crate) const CIP_TYPE_SINT: u8 = 0xC2;
pub(crate) const CIP_TYPE_INT: u8 = 0xC3;
pub(crate) const CIP_TYPE_DINT: u8 = 0xC4;
pub(crate) const CIP_TYPE_REAL: u8 = 0xCA;
pub(crate) const CIP_TYPE_STRING: u8 = 0xD0;
pub(crate) const CIP_TYPE_DWORD: u8 = 0xD3;
pub(crate) const CIP_TYPE_LINT: u8 = 0xC5;

// ── CIP status codes ──

pub(crate) const CIP_STATUS_SUCCESS: u8 = 0x00;
pub(crate) const CIP_STATUS_PARTIAL: u8 = 0x06;

// ── Encapsulation header size ──

pub(crate) const ENIP_HEADER_SIZE: usize = 24;

// ── CIP path for Symbol class ──

pub(crate) const SYMBOL_CLASS_ID: u8 = 0x6B;

// ── Public types ──

/// Connection configuration for an EtherNet/IP (Rockwell) device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnipConfig {
    /// Device identifier (optional from frontend; defaults to empty).
    #[serde(default)]
    pub id: String,
    /// IP address or hostname of the PLC.
    pub host: String,
    /// TCP port (default: 44818).
    #[serde(default = "default_enip_port")]
    pub port: u16,
    /// Polling interval in milliseconds.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_enip_port() -> u16 {
    44818
}
fn default_poll_interval_ms() -> u64 {
    1000
}

/// A CIP tag descriptor for read/write operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipTag {
    /// Tag name in the PLC (e.g. "Program:Main.MyTag").
    pub name: String,
    /// CIP data type.
    #[serde(default)]
    pub data_type: CipDataType,
    /// Number of array elements (1 for scalar tags).
    #[serde(default = "default_element_count")]
    pub element_count: u16,
    /// Set for structure (UDT) tags — the Template object instance id
    /// (Symbol type word bit 15 set, low bits = id). None for atomic tags.
    #[serde(default)]
    pub template_id: Option<u16>,
}

fn default_element_count() -> u16 {
    1
}

/// CIP atomic data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CipDataType {
    Bool,
    Sint,
    Int,
    #[default]
    Dint,
    Real,
    String,
    Dword,
    Lint,
}

impl CipDataType {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            CIP_TYPE_BOOL => Some(Self::Bool),
            CIP_TYPE_SINT => Some(Self::Sint),
            CIP_TYPE_INT => Some(Self::Int),
            CIP_TYPE_DINT => Some(Self::Dint),
            CIP_TYPE_REAL => Some(Self::Real),
            CIP_TYPE_STRING => Some(Self::String),
            CIP_TYPE_DWORD => Some(Self::Dword),
            CIP_TYPE_LINT => Some(Self::Lint),
            _ => None,
        }
    }

    pub fn code(&self) -> u8 {
        match self {
            Self::Bool => CIP_TYPE_BOOL,
            Self::Sint => CIP_TYPE_SINT,
            Self::Int => CIP_TYPE_INT,
            Self::Dint => CIP_TYPE_DINT,
            Self::Real => CIP_TYPE_REAL,
            Self::String => CIP_TYPE_STRING,
            Self::Dword => CIP_TYPE_DWORD,
            Self::Lint => CIP_TYPE_LINT,
        }
    }

    pub fn element_size(&self) -> u16 {
        match self {
            Self::Bool => 1,
            Self::Sint => 1,
            Self::Int => 2,
            Self::Dint => 4,
            Self::Real => 4,
            Self::Dword => 4,
            Self::Lint => 8,
            Self::String => 88,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::Sint => "SINT",
            Self::Int => "INT",
            Self::Dint => "DINT",
            Self::Real => "REAL",
            Self::String => "STRING",
            Self::Dword => "DWORD",
            Self::Lint => "LINT",
        }
    }
}

/// A value read from (or to be written to) a CIP tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CipValue {
    Bool(bool),
    Sint(i8),
    Int(i16),
    Dint(i32),
    Real(f32),
    String(String),
    Bytes(Vec<u8>),
}

impl Default for CipValue {
    fn default() -> Self {
        Self::Dint(0)
    }
}

impl CipValue {
    /// Convert CIP value to f64 for numeric operations.
    pub fn to_f64(&self) -> Option<f64> {
        match self {
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Self::Sint(v) => Some(*v as f64),
            Self::Int(v) => Some(*v as f64),
            Self::Dint(v) => Some(*v as f64),
            Self::Real(v) => Some(*v as f64),
            Self::Bytes(_) => None,
            Self::String(_) => None,
        }
    }

    /// Get a display-friendly string representation.
    pub fn display_value(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Sint(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Dint(v) => v.to_string(),
            Self::Real(v) => format!("{:.4}", v),
            Self::String(s) => s.clone(),
            Self::Bytes(b) => format!("{:02x?}", b),
        }
    }
}

// ── Server types ──

/// Configuration for the EtherNet/IP slave server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnipServerConfig {
    /// TCP port to listen on (default: 44818).
    #[serde(default = "default_enip_port")]
    pub port: u16,
    /// Bind address (default: "0.0.0.0").
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
}

fn default_bind_addr() -> String {
    "0.0.0.0".into()
}

impl Default for EnipServerConfig {
    fn default() -> Self {
        Self {
            port: default_enip_port(),
            bind_addr: default_bind_addr(),
        }
    }
}

/// A tag definition for the server's tag database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipTagDef {
    pub name: String,
    #[serde(default)]
    pub data_type: CipDataType,
    #[serde(default = "default_element_count")]
    pub element_count: u16,
    #[serde(default)]
    pub initial_value: CipValue,
    #[serde(default)]
    pub template_id: Option<u16>,
}

/// One member of a CIP UDT (structure) template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipTemplateMember {
    pub name: String,
    #[serde(default)]
    pub data_type: CipDataType,
    /// 0/1 = scalar, >1 = array of this many elements.
    #[serde(default = "default_element_count")]
    pub array_elements: u16,
}

/// A UDT template definition — one Template object (class 0x6C) instance.
/// Served to clients (st-ethernet-ip / RSLinx) so structure-typed tags decode
/// into named members. `id` doubles as the structure handle and the low bits of
/// the Symbol type word (0x8000 | id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipTemplateDef {
    pub id: u16,
    pub name: String,
    pub members: Vec<CipTemplateMember>,
}

/// A log entry from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnipServerLogEntry {
    pub timestamp: u64,
    pub service: String,
    pub tag_name: String,
    pub message: String,
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
            max_delay_ms: 60_000,
        }
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn next(&mut self) -> u64 {
        let delay = 1000 * 2u64.pow(self.attempt.min(6));
        self.attempt += 1;
        delay.min(self.max_delay_ms)
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a CIP tag name. Valid tag names contain alphanumeric characters,
/// underscores, and may use dotted paths for structure members (e.g., "MyTag.Member").
pub fn validate_tag_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Tag name cannot be empty".into());
    }
    if name.len() > 255 {
        return Err("Tag name too long (max 255 chars)".into());
    }

    for (i, ch) in name.chars().enumerate() {
        if i == 0 && ch.is_ascii_digit() {
            return Err("Tag name cannot start with a digit".into());
        }
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.' && ch != '[' && ch != ']' {
            return Err(format!(
                "Tag name contains invalid character '{}' at position {}",
                ch, i
            ));
        }
    }

    Ok(())
}


// ── Discovery & Identity (explorer parity) ──

pub(crate) const CMD_LIST_IDENTITY: u16 = 0x0063;
pub(crate) const CMD_LIST_SERVICES: u16 = 0x0064;

pub(crate) const CIP_GET_ATTRIBUTE_SINGLE: u8 = 0x0E;
pub(crate) const CIP_SET_ATTRIBUTE_SINGLE: u8 = 0x10;

/// CIP Identity object class id (0x01).
pub const IDENTITY_CLASS_ID: u16 = 0x01;

/// A device discovered via EtherNet/IP ListIdentity broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnipDevice {
    pub host: String,
    pub port: u16,
    pub vendor_id: u16,
    #[serde(default)]
    pub vendor_name: String,
    pub device_type: u16,
    pub product_code: u16,
    pub revision_major: u8,
    pub revision_minor: u8,
    pub status: u16,
    pub serial_number: u32,
    #[serde(default)]
    pub product_name: String,
    #[serde(default)]
    pub state: u8,
}

/// Identity info read from a connected device (Identity object 0x01, instance 1).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityInfo {
    pub vendor_id: u16,
    pub vendor_name: String,
    pub device_type: u16,
    pub product_code: u16,
    pub revision_major: u8,
    pub revision_minor: u8,
    pub status: u16,
    pub serial_number: u32,
    pub product_name: String,
}

/// Best-effort CIP vendor-name lookup for common industrial vendors.
pub fn vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        1 => "Rockwell Automation / Allen-Bradley",
        6 => "Bosch Rexroth",
        9 => "Yaskawa",
        10 => "Omron",
        14 => "Allen-Bradley (Redistributor)",
        24 => "Mitsubishi Electric",
        40 => "ABB",
        63 => "Emerson",
        64 => "Baldor",
        68 => "Phoenix Contact",
        84 => "Modicon",
        100 => "Hilscher",
        114 => "Beckhoff",
        123 => "Schneider Electric",
        157 => "Wago",
        163 => "Ifm Electronic",
        173 => "Lenze",
        189 => "Festo",
        199 => "B&R Industrial",
        209 => "Prosoft",
        224 => "Cutler-Hammer",
        233 => "Honeywell",
        245 => "Siemens",
        264 => "Softing",
        300 => "Belden",
        340 => "Moxa",
        _ => "Unknown",
    }
}
// ── Implicit/IO messaging (ForwardOpen/ForwardClose) ──

pub(crate) const CIP_FORWARD_OPEN: u8 = 0x54;
pub(crate) const CIP_FORWARD_CLOSE: u8 = 0x4E;
/// ENIP SendUnitData command (connected/CIP transport).
pub const CMD_SEND_UNIT_DATA: u16 = 0x0070;
/// Connection Manager object class id.
pub const CONNECTION_MANAGER_CLASS_ID: u16 = 0x06;

/// Parameters for a ForwardOpen request (CIP implicit/IO connection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardOpenParams {
    pub o_to_t_connection_id: u32,
    pub t_to_o_connection_id: u32,
    pub connection_serial: u16,
    pub vendor_id: u16,
    pub originator_serial: u32,
    #[serde(default = "default_timeout_ticks")]
    pub timeout_ticks: u8,
    #[serde(default = "default_timeout_multiplier")]
    pub timeout_multiplier: u8,
    pub o_to_t_rpi_us: u32,
    pub o_to_t_params: u16,
    pub t_to_o_rpi_us: u32,
    pub t_to_o_params: u16,
    #[serde(default = "default_transport_class_trigger")]
    pub transport_class_trigger: u8,
    /// Raw connection path bytes (port + class/instance of target object).
    pub connection_path: Vec<u8>,
}

fn default_timeout_ticks() -> u8 {
    0x0A
}
fn default_timeout_multiplier() -> u8 {
    1
}
fn default_transport_class_trigger() -> u8 {
    0xA3
}

/// Result of a successful ForwardOpen (connection IDs assigned by target).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardOpenResult {
    pub o_to_t_connection_id: u32,
    pub t_to_o_connection_id: u32,
    pub connection_serial: u16,
    pub vendor_id: u16,
    pub originator_serial: u32,
}

// ── Packet capture (raw TX/RX wire bytes for the packet logger) ──

/// Direction of a captured wire frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PacketDir {
    Tx,
    Rx,
}

/// A raw EtherNet/IP frame captured for the packet logger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketRecord {
    pub direction: PacketDir,
    /// EtherNet/IP command code (e.g. 0x006F SendRRData, 0x0070 SendUnitData).
    pub command: u16,
    /// Session handle, if the frame carries one (0 for ListIdentity).
    pub session: u32,
    /// Unix-epoch milliseconds at capture time.
    pub timestamp_ms: u64,
    pub bytes: Vec<u8>,
}
