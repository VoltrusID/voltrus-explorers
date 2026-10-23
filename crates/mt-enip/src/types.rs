#![allow(dead_code)]
use serde::{Deserialize, Serialize};

// ── EtherNet/IP command codes ──

pub(crate) const CMD_REGISTER_SESSION: u16 = 0x0065;
pub(crate) const CMD_UNREGISTER_SESSION: u16 = 0x0066;
pub(crate) const CMD_SEND_RR_DATA: u16 = 0x006F;

// ── CIP service codes ──

pub(crate) const CIP_READ_TAG: u8 = 0x4C;
pub(crate) const CIP_READ_TAG_FRAGMENTED: u8 = 0x52;
pub(crate) const CIP_WRITE_TAG: u8 = 0x4D;
pub(crate) const CIP_WRITE_TAG_FRAGMENTED: u8 = 0x53;
pub(crate) const CIP_GET_ATTRIBUTES_ALL: u8 = 0x01;
pub(crate) const CIP_GET_INSTANCE_ATTRIBUTE_LIST: u8 = 0x55;
/// Connection Manager Unconnected_Send (ODVA Vol 1 §3-5.5.4).
pub(crate) const CIP_UNCONNECTED_SEND: u8 = 0x52;
/// Message Router Multiple_Service_Packet (ODVA Vol 1 §A-5.1).
pub(crate) const CIP_MULTIPLE_SERVICE_PACKET: u8 = 0x0A;
/// Connection Manager Large_Forward_Open (ODVA Vol 1 §3-5.5.2).
pub(crate) const CIP_LARGE_FORWARD_OPEN: u8 = 0x5B;

// ── CIP data type codes (ODVA Vol 1 Appendix C-6) ──

pub(crate) const CIP_TYPE_BOOL: u8 = 0xC1;
pub(crate) const CIP_TYPE_SINT: u8 = 0xC2;
pub(crate) const CIP_TYPE_INT: u8 = 0xC3;
pub(crate) const CIP_TYPE_DINT: u8 = 0xC4;
pub(crate) const CIP_TYPE_LINT: u8 = 0xC5;
pub(crate) const CIP_TYPE_USINT: u8 = 0xC6;
pub(crate) const CIP_TYPE_UINT: u8 = 0xC7;
pub(crate) const CIP_TYPE_UDINT: u8 = 0xC8;
pub(crate) const CIP_TYPE_ULINT: u8 = 0xC9;
pub(crate) const CIP_TYPE_REAL: u8 = 0xCA;
pub(crate) const CIP_TYPE_LREAL: u8 = 0xCB;
/// CIP STRING (UINT length + 1-byte chars). NOT what Logix uses for its
/// STRING tags — see [`CIP_TYPE_STRUCT`] / [`STRUCT_HANDLE_LOGIX_STRING`].
pub(crate) const CIP_TYPE_STRING: u8 = 0xD0;
pub(crate) const CIP_TYPE_BYTE: u8 = 0xD1;
pub(crate) const CIP_TYPE_WORD: u8 = 0xD2;
pub(crate) const CIP_TYPE_DWORD: u8 = 0xD3;
pub(crate) const CIP_TYPE_LWORD: u8 = 0xD4;
pub(crate) const CIP_TYPE_SHORT_STRING: u8 = 0xDA;
/// Logix structure type: the 2-byte type field on the wire is `A0 02` followed
/// by a 2-byte structure handle (1756-PM020, "Logix 5000 Data Access").
pub(crate) const CIP_TYPE_STRUCT: u8 = 0xA0;
/// Structure handle of the built-in Logix STRING (DINT LEN + 82 chars = 88 bytes).
pub(crate) const STRUCT_HANDLE_LOGIX_STRING: u16 = 0x0FCE;
/// Byte size of a Logix STRING structure.
pub(crate) const LOGIX_STRING_SIZE: usize = 88;
/// Maximum characters of a Logix STRING.
pub(crate) const LOGIX_STRING_MAX_CHARS: usize = 82;

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
    /// CIP route path to the target controller, e.g. `"1,0"` (backplane port 1,
    /// slot 0) or `"1,2,2,10.0.0.5,1,0"` (hop through a gateway). When set,
    /// every explicit request is wrapped in a Connection Manager
    /// Unconnected_Send with this route. Required for ControlLogix behind an
    /// EtherNet/IP bridge module; leave unset for CompactLogix, Micro800 and
    /// other devices that terminate CIP on the Ethernet port.
    #[serde(default)]
    pub route_path: Option<String>,
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

/// CIP data types.
///
/// `String` is the **Logix STRING** structure (DINT `LEN` + 82 `DATA` bytes,
/// structure handle 0x0FCE) — the only string type a ControlLogix/CompactLogix
/// exposes as a tag. Plain CIP STRING (0xD0) / SHORT_STRING (0xDA) attribute
/// values are decoded to [`CipValue::String`] but are not tag types.
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
    Usint,
    Uint,
    Udint,
    Ulint,
    Lreal,
}

impl CipDataType {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            CIP_TYPE_BOOL => Some(Self::Bool),
            CIP_TYPE_SINT => Some(Self::Sint),
            CIP_TYPE_INT => Some(Self::Int),
            CIP_TYPE_DINT => Some(Self::Dint),
            CIP_TYPE_LINT => Some(Self::Lint),
            CIP_TYPE_USINT => Some(Self::Usint),
            CIP_TYPE_UINT => Some(Self::Uint),
            CIP_TYPE_UDINT => Some(Self::Udint),
            CIP_TYPE_ULINT => Some(Self::Ulint),
            CIP_TYPE_REAL => Some(Self::Real),
            CIP_TYPE_LREAL => Some(Self::Lreal),
            CIP_TYPE_STRING => Some(Self::String),
            CIP_TYPE_DWORD => Some(Self::Dword),
            _ => None,
        }
    }

    /// Atomic CIP type code. `String` has no atomic code on a Logix; the value
    /// returned (0xD0) is only used where a template member needs *a* code.
    pub fn code(&self) -> u8 {
        match self {
            Self::Bool => CIP_TYPE_BOOL,
            Self::Sint => CIP_TYPE_SINT,
            Self::Int => CIP_TYPE_INT,
            Self::Dint => CIP_TYPE_DINT,
            Self::Lint => CIP_TYPE_LINT,
            Self::Usint => CIP_TYPE_USINT,
            Self::Uint => CIP_TYPE_UINT,
            Self::Udint => CIP_TYPE_UDINT,
            Self::Ulint => CIP_TYPE_ULINT,
            Self::Real => CIP_TYPE_REAL,
            Self::Lreal => CIP_TYPE_LREAL,
            Self::String => CIP_TYPE_STRING,
            Self::Dword => CIP_TYPE_DWORD,
        }
    }

    pub fn element_size(&self) -> u16 {
        match self {
            Self::Bool | Self::Sint | Self::Usint => 1,
            Self::Int | Self::Uint => 2,
            Self::Dint | Self::Udint | Self::Real | Self::Dword => 4,
            Self::Lint | Self::Ulint | Self::Lreal => 8,
            Self::String => LOGIX_STRING_SIZE as u16,
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
            Self::Usint => "USINT",
            Self::Uint => "UINT",
            Self::Udint => "UDINT",
            Self::Ulint => "ULINT",
            Self::Lreal => "LREAL",
        }
    }
}

/// A value read from (or to be written to) a CIP tag.
///
/// `Array` carries every element of a multi-element read/write. `Bytes` is the
/// raw image of a structure (UDT) or of a type with no richer representation
/// (DWORD/LWORD bit strings, vendor types).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CipValue {
    Bool(bool),
    Sint(i8),
    Int(i16),
    Dint(i32),
    Lint(i64),
    Usint(u8),
    Uint(u16),
    Udint(u32),
    Ulint(u64),
    Real(f32),
    Lreal(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<CipValue>),
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
            Self::Lint(v) => Some(*v as f64),
            Self::Usint(v) => Some(*v as f64),
            Self::Uint(v) => Some(*v as f64),
            Self::Udint(v) => Some(*v as f64),
            Self::Ulint(v) => Some(*v as f64),
            Self::Real(v) => Some(*v as f64),
            Self::Lreal(v) => Some(*v),
            Self::Array(items) => items.first().and_then(|v| v.to_f64()),
            Self::Bytes(_) | Self::String(_) => None,
        }
    }

    /// Get a display-friendly string representation.
    pub fn display_value(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Sint(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Dint(v) => v.to_string(),
            Self::Lint(v) => v.to_string(),
            Self::Usint(v) => v.to_string(),
            Self::Uint(v) => v.to_string(),
            Self::Udint(v) => v.to_string(),
            Self::Ulint(v) => v.to_string(),
            Self::Real(v) => format!("{:.4}", v),
            Self::Lreal(v) => format!("{:.4}", v),
            Self::String(s) => s.clone(),
            Self::Bytes(b) => format!("{:02x?}", b),
            Self::Array(items) => {
                let shown: Vec<String> = items.iter().take(16).map(|v| v.display_value()).collect();
                if items.len() > 16 {
                    format!("[{}, … ({} elements)]", shown.join(", "), items.len())
                } else {
                    format!("[{}]", shown.join(", "))
                }
            }
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
    /// A tag the simulator drives itself: a client's Write Tag is refused with "Attribute not settable" (0x0E).
    #[serde(default)]
    pub read_only: bool,
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
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '.' | '[' | ']' | ',' | ':') {
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
pub(crate) const CMD_LIST_INTERFACES: u16 = 0x0004;

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

/// CIP vendor-name lookup for common industrial vendors. IDs are the ODVA
/// vendor registry values (cross-checked against pycomm3 `status_info._VENDORS`).
pub fn vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        1 => "Rockwell Automation/Allen-Bradley",
        3 => "Honeywell",
        4 => "Parker Hannifin",
        5 => "Rockwell Automation/Reliance Electric",
        7 => "SMC",
        8 => "Molex",
        12 => "Banner Engineering",
        13 => "Belden",
        26 => "Festo",
        40 => "WAGO",
        43 => "Balluff",
        44 => "Yaskawa America",
        46 => "ABB",
        47 => "Omron",
        48 => "Turck",
        56 => "Softing",
        57 => "Pepperl+Fuchs",
        68 => "Eaton Electrical",
        82 | 161 => "Mitsubishi Electric",
        83 => "Opto 22",
        90 => "HMS Networks",
        94 => "Advantech",
        95 => "Square D",
        97 => "Danfoss Drives",
        108 => "Beckhoff Automation",
        176 => "Red Lion",
        181 => "Pilz",
        243 => "Schneider Electric",
        250 => "Yokogawa",
        255 | 287 => "Bosch Rexroth",
        265 => "Baldor Electric",
        283 => "Hilscher",
        309 => "ProSoft Technology",
        315 => "SEW-Eurodrive",
        322 => "ifm electronic",
        345 => "Endress+Hauser",
        367 => "Keyence",
        445 | 587 => "Lenze",
        452 => "Kollmorgen",
        524 => "Leuze",
        562 => "Phoenix Contact",
        640 => "Murrelektronik",
        678 => "Cognex",
        799 => "Delta Electronics",
        808 => "SICK",
        887 => "B&R Industrial Automation",
        939 => "Cisco Systems",
        991 => "Moxa",
        1015 => "Weidmueller",
        1251 => "Siemens",
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

/// Parameters for a ForwardOpen / Large_ForwardOpen request (ODVA Vol 1
/// §3-5.5.1–3-5.5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardOpenParams {
    pub o_to_t_connection_id: u32,
    pub t_to_o_connection_id: u32,
    pub connection_serial: u16,
    pub vendor_id: u16,
    pub originator_serial: u32,
    /// Priority/Time_tick byte (bits 0–3 = tick time 2^n ms, bit 4 = priority).
    #[serde(default = "default_priority_time_tick")]
    pub priority_time_tick: u8,
    /// Time-out_ticks: how many ticks the Connection Manager waits for the open.
    #[serde(default = "default_timeout_ticks")]
    pub timeout_ticks: u8,
    /// Connection timeout multiplier code (0 = ×4, 1 = ×8, 2 = ×16 … 7 = ×512).
    #[serde(default = "default_timeout_multiplier")]
    pub timeout_multiplier: u8,
    pub o_to_t_rpi_us: u32,
    /// O→T network connection parameters. 16-bit for Forward_Open (value must
    /// fit in `u16`); 32-bit for Large_Forward_Open. See [`network_connection_params`].
    pub o_to_t_params: u32,
    pub t_to_o_rpi_us: u32,
    /// T→O network connection parameters (same format as `o_to_t_params`).
    pub t_to_o_params: u32,
    #[serde(default = "default_transport_class_trigger")]
    pub transport_class_trigger: u8,
    /// Use Large_Forward_Open (service 0x5B, 32-bit network parameters, up to
    /// 65 535-byte connection sizes) instead of Forward_Open (0x54, 508 bytes).
    #[serde(default)]
    pub large: bool,
    /// Client behaviour for class 0/1 connections: prefix every O→T data
    /// packet with the 32-bit real-time (run/idle) header. Not a wire field of
    /// Forward_Open itself; the connection size must account for it.
    #[serde(default = "default_true")]
    pub o_to_t_run_idle: bool,
    /// Raw connection path bytes (optional port segments, then the target
    /// class/instance or assembly connection points).
    pub connection_path: Vec<u8>,
}

fn default_true() -> bool {
    true
}
fn default_priority_time_tick() -> u8 {
    0x0A
}
fn default_timeout_ticks() -> u8 {
    0x0A
}
fn default_timeout_multiplier() -> u8 {
    1
}
/// Class 3 server transport, application-object trigger (explicit messaging).
fn default_transport_class_trigger() -> u8 {
    0xA3
}

/// Connection type field of the network connection parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionType {
    Null = 0,
    Multicast = 1,
    PointToPoint = 2,
}

/// Build the network connection parameter word (ODVA Vol 1 §3-5.5.1.1,
/// Figure 3-5.8): size in bits 0–8 (0–15 for the large form), fixed/variable in
/// bit 9 (25 large), priority in bits 10–11 (26–27), connection type in bits
/// 13–14 (29–30), redundant owner in bit 15 (31). `priority`: 0 low, 1 high,
/// 2 scheduled, 3 urgent.
pub fn network_connection_params(
    size: u16,
    connection_type: ConnectionType,
    variable: bool,
    priority: u8,
    large: bool,
) -> u32 {
    let flags: u16 = ((variable as u16) << 9) | (((priority & 3) as u16) << 10) | ((connection_type as u16) << 13);
    if large {
        (size as u32) | ((flags as u32) << 16)
    } else {
        ((size & 0x01FF) | flags) as u32
    }
}

/// Result of a successful ForwardOpen (connection IDs assigned by target).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardOpenResult {
    pub o_to_t_connection_id: u32,
    pub t_to_o_connection_id: u32,
    pub connection_serial: u16,
    pub vendor_id: u16,
    pub originator_serial: u32,
    /// Actual packet interval (µs) the target granted for O→T.
    #[serde(default)]
    pub o_to_t_api_us: u32,
    /// Actual packet interval (µs) the target granted for T→O.
    #[serde(default)]
    pub t_to_o_api_us: u32,
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
