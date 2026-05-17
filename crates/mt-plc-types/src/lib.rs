use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Opcode {
    PushBool = 0x01,
    PushInt = 0x02,
    PushReal = 0x03,
    PushTime = 0x04,
    PushDint = 0x05,
    PushEnum = 0x06,
    PushString = 0x07,

    LoadVar = 0x10,
    StoreVar = 0x11,
    LoadIo = 0x12,
    StoreIo = 0x13,

    Add = 0x20,
    Sub = 0x21,
    Mul = 0x22,
    Div = 0x23,
    Modulo = 0x24,
    Pow = 0x25,
    Neg = 0x26,

    Eq = 0x30,
    Ne = 0x31,
    Lt = 0x32,
    Le = 0x33,
    Gt = 0x34,
    Ge = 0x35,

    And = 0x40,
    Or = 0x41,
    Xor = 0x42,
    Not = 0x43,
    Shl = 0x44,
    Shr = 0x45,
    Rol = 0x46,
    Ror = 0x47,

    Jump = 0x50,
    JumpIfFalse = 0x51,
    Call = 0x52,
    CallPou = 0x53,
    Return = 0x54,

    TonCall = 0x60,
    TofCall = 0x61,
    TpCall = 0x62,
    CtuCall = 0x63,
    CtdCall = 0x64,
    RTrigCall = 0x65,
    FTrigCall = 0x66,
    CtudCall = 0x67,

    // Standard library: Math (0x70-0x7F)
    Sqrt = 0x70,
    Ln = 0x71,
    Log = 0x72,
    Exp = 0x73,
    Sin = 0x74,
    Cos = 0x75,
    Tan = 0x76,
    Asin = 0x77,
    Acos = 0x78,
    Atan = 0x79,
    Abs = 0x7A,
    Trunc = 0x7B,
    Floor = 0x7C,
    Ceil = 0x7D,
    Expt = 0x7E,

    // Standard library: String (0x80-0x8F)
    StrLen = 0x80,
    StrLeft = 0x81,
    StrRight = 0x82,
    StrMid = 0x83,
    Concat = 0x84,
    StrInsert = 0x85,
    StrDelete = 0x86,
    StrReplace = 0x87,
    StrFind = 0x88,

    // Standard library: Conversion (0x90-0x9F)
    IntToReal = 0x90,
    RealToInt = 0x91,
    IntToBool = 0x92,
    BoolToInt = 0x93,
    IntToString = 0x94,
    RealToString = 0x95,
    StringToInt = 0x96,
    StringToReal = 0x97,
    WordToInt = 0x98,
    DwordToInt = 0x99,
    IntToWord = 0x9A,
    IntToDword = 0x9B,
    DwordToReal = 0x9C,
    RealToDword = 0x9D,

    // Standard library: Selection (0xA0-0xAF)
    Sel = 0xA0,
    Mux = 0xA1,
    Limit = 0xA2,
    Max = 0xA3,
    Min = 0xA4,
    Move = 0xA5,

    // Array ops (0xB0-0xBF)
    LoadArrayIndex = 0xB0,
    StoreArrayIndex = 0xB1,
    ArrayLen = 0xB2,

    // Struct ops (0xB3-0xBF)
    LoadStructField = 0xB3,
    StoreStructField = 0xB4,

    Halt = 0xFF,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    pub opcode: Opcode,
    pub operand: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlcValue {
    Bool(bool),
    Byte(u8),
    Int(i16),
    Dint(i32),
    Real(f32),
    Time(i32),
    String(String),
    Word(u16),
    Dword(u32),
    Array(Vec<PlcValue>),
    Struct(Vec<(String, PlcValue)>),
    Enum(String),
}

impl PlcValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            PlcValue::Bool(_) => "BOOL",
            PlcValue::Byte(_) => "BYTE",
            PlcValue::Int(_) => "INT",
            PlcValue::Dint(_) => "DINT",
            PlcValue::Real(_) => "REAL",
            PlcValue::Time(_) => "TIME",
            PlcValue::String(_) => "STRING",
            PlcValue::Word(_) => "WORD",
            PlcValue::Dword(_) => "DWORD",
            PlcValue::Array(_) => "ARRAY",
            PlcValue::Struct(_) => "STRUCT",
            PlcValue::Enum(_) => "ENUM",
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            PlcValue::Bool(b) => *b,
            PlcValue::Byte(b) => *b != 0,
            PlcValue::Int(i) => *i != 0,
            PlcValue::Dint(i) => *i != 0,
            PlcValue::Real(f) => *f != 0.0,
            PlcValue::Word(w) => *w != 0,
            PlcValue::Dword(d) => *d != 0,
            PlcValue::Time(t) => *t != 0,
            PlcValue::String(s) => !s.is_empty(),
            PlcValue::Array(a) => !a.is_empty(),
            PlcValue::Struct(s) => !s.is_empty(),
            PlcValue::Enum(e) => !e.is_empty(),
        }
    }

    pub fn to_int(&self) -> i32 {
        match self {
            PlcValue::Bool(b) => if *b { 1 } else { 0 },
            PlcValue::Byte(b) => *b as i32,
            PlcValue::Int(i) => *i as i32,
            PlcValue::Dint(i) => *i,
            PlcValue::Real(f) => *f as i32,
            PlcValue::Word(w) => *w as i32,
            PlcValue::Dword(d) => *d as i32,
            PlcValue::Time(t) => *t,
            PlcValue::String(_) => 0,
            PlcValue::Array(_) => 0,
            PlcValue::Struct(_) => 0,
            PlcValue::Enum(_) => 0,
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            PlcValue::String(s) => s.clone(),
            PlcValue::Byte(b) => b.to_string(),
            PlcValue::Int(i) => i.to_string(),
            PlcValue::Dint(i) => i.to_string(),
            PlcValue::Real(f) => format!("{:.3}", f),
            PlcValue::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
            PlcValue::Enum(e) => e.clone(),
            _ => String::new(),
        }
    }

    pub fn to_real(&self) -> f32 {
        match self {
            PlcValue::Bool(b) => if *b { 1.0 } else { 0.0 },
            PlcValue::Byte(b) => *b as f32,
            PlcValue::Int(i) => *i as f32,
            PlcValue::Dint(i) => *i as f32,
            PlcValue::Real(f) => *f,
            PlcValue::Word(w) => *w as f32,
            PlcValue::Dword(d) => *d as f32,
            PlcValue::Time(t) => *t as f32,
            PlcValue::String(_) => 0.0,
            PlcValue::Array(_) => 0.0,
            PlcValue::Struct(_) => 0.0,
            PlcValue::Enum(_) => 0.0,
        }
    }

    pub fn zero_of_type(&self) -> PlcValue {
        match self {
            PlcValue::Bool(_) => PlcValue::Bool(false),
            PlcValue::Byte(_) => PlcValue::Byte(0),
            PlcValue::Int(_) => PlcValue::Int(0),
            PlcValue::Dint(_) => PlcValue::Dint(0),
            PlcValue::Real(_) => PlcValue::Real(0.0),
            PlcValue::Time(_) => PlcValue::Time(0),
            PlcValue::String(_) => PlcValue::String(String::new()),
            PlcValue::Word(_) => PlcValue::Word(0),
            PlcValue::Dword(_) => PlcValue::Dword(0),
            PlcValue::Array(_) => PlcValue::Array(vec![]),
            PlcValue::Struct(fields) => PlcValue::Struct(fields.iter().map(|(n, v)| (n.clone(), v.zero_of_type())).collect()),
            PlcValue::Enum(_) => PlcValue::Enum(String::new()),
        }
    }
}

impl std::fmt::Display for PlcValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlcValue::Bool(b) => write!(f, "{}", if *b { "TRUE" } else { "FALSE" }),
            PlcValue::Byte(b) => write!(f, "{}", b),
            PlcValue::Int(i) => write!(f, "{}", i),
            PlcValue::Dint(i) => write!(f, "{}", i),
            PlcValue::Real(r) => write!(f, "{:.2}", r),
            PlcValue::Time(t) => write!(f, "{}ms", t),
            PlcValue::String(s) => write!(f, "'{}'", s),
            PlcValue::Word(w) => write!(f, "{}", w),
            PlcValue::Dword(d) => write!(f, "{}", d),
            PlcValue::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            PlcValue::Struct(fields) => {
                let items: Vec<String> = fields.iter().map(|(n, v)| format!("{}:={}", n, v)).collect();
                write!(f, "({})", items.join(", "))
            }
            PlcValue::Enum(e) => write!(f, "{}", e),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledModule {
    pub name: String,
    pub instructions: Vec<Instruction>,
    pub variable_slots: Vec<(String, PlcValue)>,
    #[serde(default)]
    pub pou_table: Vec<(String, usize)>,
    #[serde(default)]
    pub name_constants: Vec<String>,
    #[serde(default)]
    pub line_map: Vec<(usize, usize)>,
}

/// I/O mapping for ESP32 bytecode deployment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoMapping {
    pub variable_index: u32,
    pub io_type: IoType,
    pub protocol: IoProtocol,
    pub address: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum IoType {
    Input = 0,
    Output = 1,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum IoProtocol {
    None = 0,
    Modbus = 1,
    S7 = 2,
    Enip = 3,
    Gpio = 4,
    Mqtt = 5,
    Opcua = 6,
}

// ESP32 bytecode format constants
const PLC_BYTECODE_MAGIC: u32 = 0x564C5431;
const PLC_BYTECODE_VERSION: u16 = 1;
const PLC_MAX_NAME_LEN: usize = 32;

/// Variable type tags matching ESP32 plc_value_type_t
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum EspVarType {
    Bool = 0,
    Byte = 1,
    Int = 2,
    Dint = 3,
    Real = 4,
    Time = 5,
    String = 6,
    Word = 7,
    Dword = 8,
}

/// Serialize a CompiledModule into the ESP32 bytecode binary format.
///
/// Format:
///   [4B magic] [2B version] [4B instr_count] [4B var_count] [4B io_count] [4B name_count]
///   [instructions] [variable_inits] [variable_names] [io_mappings] [name_constants]
///   [4B CRC32]
pub fn serialize_bytecode(
    module: &CompiledModule,
    io_mappings: &[IoMapping],
) -> Result<Vec<u8>, String> {
    let mut name_constants: Vec<&str> = module.name_constants.iter().map(|s| s.as_str()).collect();
    let mut name_constant_indices: std::collections::HashMap<&str, u16> = module
        .name_constants
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i as u16))
        .collect();

    // Collect string/enum values from variable_slots into name_constants
    for (_name, val) in &module.variable_slots {
        match val {
            PlcValue::String(s) | PlcValue::Enum(s) => {
                if !name_constant_indices.contains_key(s.as_str()) {
                    name_constant_indices.insert(s.as_str(), name_constants.len() as u16);
                    name_constants.push(s.as_str());
                }
            }
            _ => {}
        }
    }

    let instr_count = module.instructions.len() as u32;
    let var_count = module.variable_slots.len() as u32;
    let io_count = io_mappings.len() as u32;
    let name_count = name_constants.len() as u32;

    // Pre-calculate body size
    let body_size = 22 // header
        + instr_count as usize * 6
        + var_count as usize * 5
        + module.variable_slots.iter().map(|(n, _)| 1 + n.len().min(PLC_MAX_NAME_LEN - 1)).sum::<usize>()
        + io_count as usize * 10
        + name_constants.iter().map(|s| 1 + s.len().min(PLC_MAX_NAME_LEN - 1)).sum::<usize>()
        + 4; // CRC32

    let mut buf = Vec::with_capacity(body_size);

    // --- Header (22 bytes) ---
    buf.extend_from_slice(&PLC_BYTECODE_MAGIC.to_le_bytes());
    buf.extend_from_slice(&PLC_BYTECODE_VERSION.to_le_bytes());
    buf.extend_from_slice(&instr_count.to_le_bytes());
    buf.extend_from_slice(&var_count.to_le_bytes());
    buf.extend_from_slice(&io_count.to_le_bytes());
    buf.extend_from_slice(&name_count.to_le_bytes());

    // --- Instructions (6 bytes each: 1 opcode + 1 has_operand + 4 operand) ---
    for instr in &module.instructions {
        buf.push(instr.opcode as u8);
        buf.push(if instr.operand.is_some() { 1 } else { 0 });
        buf.extend_from_slice(&(instr.operand.unwrap_or(0)).to_le_bytes());
    }

    // --- Variable init values (5 bytes each: 1 type + 4 value) ---
    for (_name, val) in &module.variable_slots {
        let (type_tag, raw_value) = plvalue_to_esp_raw(val, &name_constant_indices);
        buf.push(type_tag as u8);
        buf.extend_from_slice(&raw_value.to_le_bytes());
    }

    // --- Variable names (1 len + bytes each) ---
    for (name, _val) in &module.variable_slots {
        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(PLC_MAX_NAME_LEN - 1) as u8;
        buf.push(len);
        buf.extend_from_slice(&name_bytes[..len as usize]);
    }

    // --- I/O mappings (10 bytes each: 4 var_index + 1 io_type + 1 protocol + 4 address) ---
    for mapping in io_mappings {
        buf.extend_from_slice(&mapping.variable_index.to_le_bytes());
        buf.push(mapping.io_type as u8);
        buf.push(mapping.protocol as u8);
        buf.extend_from_slice(&mapping.address.to_le_bytes());
    }

    // --- Name constants (1 len + bytes each) ---
    for name in &name_constants {
        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(PLC_MAX_NAME_LEN - 1) as u8;
        buf.push(len);
        buf.extend_from_slice(&name_bytes[..len as usize]);
    }

    // --- CRC32 (4 bytes) ---
    let crc = crc32fast::hash(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());

    Ok(buf)
}

fn plvalue_to_esp_raw(
    val: &PlcValue,
    name_indices: &std::collections::HashMap<&str, u16>,
) -> (EspVarType, u32) {
    match val {
        PlcValue::Bool(b) => (EspVarType::Bool, if *b { 1 } else { 0 }),
        PlcValue::Byte(b) => (EspVarType::Byte, *b as u32),
        PlcValue::Int(i) => (EspVarType::Int, (*i as i32) as u32),
        PlcValue::Dint(i) => (EspVarType::Dint, *i as u32),
        PlcValue::Real(f) => (EspVarType::Real, f.to_bits()),
        PlcValue::Time(t) => (EspVarType::Time, *t as u32),
        PlcValue::Word(w) => (EspVarType::Word, *w as u32),
        PlcValue::Dword(d) => (EspVarType::Dword, *d),
        PlcValue::String(s) | PlcValue::Enum(s) => {
            let idx = name_indices.get(s.as_str()).copied().unwrap_or(0);
            (EspVarType::String, idx as u32)
        }
        PlcValue::Array(_) | PlcValue::Struct(_) => {
            // ESP32 bytecode doesn't support complex type init
            (EspVarType::Dint, 0)
        }
    }
}
