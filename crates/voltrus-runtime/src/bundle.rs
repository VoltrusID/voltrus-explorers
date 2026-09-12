//! VLT1 bundle loading.
//!
//! [`mt_plc_types::serialize_bytecode`](mt_plc_types::serialize_bytecode)
//! writes the ESP32 VLT1 format; this module is its reader, so `--bundle`
//! accepts the exact artifact the IDE and the ESP32 firmware already
//! exchange. The format stores neither the module name nor `line_map`, and
//! Enum-typed inits ride the string constant pool, so a loaded module has an
//! empty name/line map and loads Enum inits as String.

use mt_plc_types::{
    crc32_le, CompiledModule, Instruction, IoMapping, IoProtocol, IoType, Opcode, PlcValue,
};
use thiserror::Error;

/// Header magic `VLT1` as written by the serializer.
pub const PLC_BYTECODE_MAGIC: u32 = 0x564C_5431;

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("not a VLT1 bundle (bad magic)")]
    BadMagic,
    #[error("unsupported VLT1 version {found} (expected 1)")]
    UnsupportedVersion { found: u16 },
    #[error("bundle truncated: needed {needed} more byte(s) at offset {offset}")]
    Truncated { needed: usize, offset: usize },
    #[error("bundle corrupted: CRC32 mismatch (stored {stored:#010x}, computed {computed:#010x})")]
    CrcMismatch { stored: u32, computed: u32 },
    #[error("bundle corrupted: unknown opcode {opcode:#04x} at instruction {index}")]
    BadOpcode { opcode: u8, index: usize },
    #[error("bundle corrupted: variable init tag {tag:#04x}")]
    BadVarTag { tag: u8 },
    #[error("bundle corrupted: string init index {index} out of range (pool has {len})")]
    BadStringIndex { index: usize, len: usize },
    #[error("bundle corrupted: unknown I/O type {io_type}")]
    BadIoType { io_type: u8 },
    #[error("bundle corrupted: unknown I/O protocol {protocol}")]
    BadIoProtocol { protocol: u8 },
    #[error("bundle corrupted: invalid UTF-8 in a name")]
    BadName(#[from] std::string::FromUtf8Error),
}

/// A parsed VLT1 bundle: the program plus its I/O mappings.
///
/// The mappings are informational here: VLT1 carries no driver connection
/// parameters, so field I/O stays configured through the service's
/// `config_json` (IDE deploy) rather than the boot bundle.
pub struct Bundle {
    pub module: CompiledModule,
    pub io_mappings: Vec<IoMapping>,
}

/// Parse a VLT1 bundle as produced by
/// [`mt_plc_types::serialize_bytecode`](mt_plc_types::serialize_bytecode).
pub fn load_vlt1(bytes: &[u8]) -> Result<Bundle, BundleError> {
    // The trailing 4 bytes are a CRC32 over everything before them.
    if bytes.len() < 28 {
        return Err(BundleError::Truncated { needed: 28, offset: bytes.len() });
    }
    let split = bytes.len() - 4;
    let stored = u32::from_le_bytes([
        bytes[split],
        bytes[split + 1],
        bytes[split + 2],
        bytes[split + 3],
    ]);
    let computed = crc32_le(&bytes[..split]);
    if stored != computed {
        return Err(BundleError::CrcMismatch { stored, computed });
    }

    let mut r = Reader::new(bytes);
    if r.u32le()? != PLC_BYTECODE_MAGIC {
        return Err(BundleError::BadMagic);
    }
    let version = r.u16le()?;
    if version != 1 {
        return Err(BundleError::UnsupportedVersion { found: version });
    }
    r.take(2)?; // alignment padding after the u16 version

    let instr_count = r.u32le()? as usize;
    let var_count = r.u32le()? as usize;
    let io_count = r.u32le()? as usize;
    let name_count = r.u32le()? as usize;

    // Instructions: 6 bytes each (opcode, has-operand flag, i32 operand —
    // the operand bytes are always present, even when the flag is 0).
    let mut instructions = Vec::with_capacity(instr_count);
    for index in 0..instr_count {
        let raw_opcode = r.u8()?;
        let has_operand = r.u8()?;
        let raw_operand = r.i32le()?;
        let opcode =
            Opcode::from_u8(raw_opcode).ok_or(BundleError::BadOpcode { opcode: raw_opcode, index })?;
        instructions.push(Instruction {
            opcode,
            operand: (has_operand != 0).then_some(raw_operand),
        });
    }

    // Variable init values (tag + raw), then their names, in matching order.
    let mut inits = Vec::with_capacity(var_count);
    for _ in 0..var_count {
        let tag = r.u8()?;
        let raw = r.u32le()?;
        inits.push((tag, raw));
    }
    let mut names = Vec::with_capacity(var_count);
    for _ in 0..var_count {
        names.push(r.name()?);
    }

    let mut io_mappings = Vec::with_capacity(io_count);
    for _ in 0..io_count {
        let variable_index = r.u32le()?;
        let io_type = r.u8()?;
        let protocol = r.u8()?;
        let address = r.u32le()?;
        io_mappings.push(IoMapping {
            variable_index,
            io_type: io_type_from_u8(io_type).ok_or(BundleError::BadIoType { io_type })?,
            protocol: io_protocol_from_u8(protocol)
                .ok_or(BundleError::BadIoProtocol { protocol })?,
            address,
        });
    }

    let mut name_constants = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        name_constants.push(r.name()?);
    }

    let variable_slots = names
        .into_iter()
        .zip(inits)
        .map(|(name, (tag, raw))| Ok((name, init_from_raw(tag, raw, &name_constants)?)))
        .collect::<Result<Vec<_>, BundleError>>()?;

    Ok(Bundle {
        module: CompiledModule {
            name: String::new(),
            instructions,
            variable_slots,
            pou_table: Vec::new(),
            name_constants,
            line_map: Vec::new(),
        },
        io_mappings,
    })
}

/// Decode one variable init. Tags mirror mt-plc-types' private `EspVarType`;
/// Enum inits share the String tag (they load as String), and Array/Struct
/// inits are stored as a Dint zero by the serializer and cannot round-trip.
fn init_from_raw(tag: u8, raw: u32, pool: &[String]) -> Result<PlcValue, BundleError> {
    Ok(match tag {
        0 => PlcValue::Bool(raw != 0),
        1 => PlcValue::Byte(raw as u8),
        2 => PlcValue::Int(raw as i32 as i16),
        3 => PlcValue::Dint(raw as i32),
        4 => PlcValue::Real(f32::from_bits(raw)),
        5 => PlcValue::Time(raw as i32),
        6 => PlcValue::String(pool_string(raw as usize, pool)?),
        7 => PlcValue::Word(raw as u16),
        8 => PlcValue::Dword(raw),
        other => return Err(BundleError::BadVarTag { tag: other }),
    })
}

fn pool_string(index: usize, pool: &[String]) -> Result<String, BundleError> {
    pool.get(index)
        .cloned()
        .ok_or(BundleError::BadStringIndex { index, len: pool.len() })
}

fn io_type_from_u8(v: u8) -> Option<IoType> {
    match v {
        0 => Some(IoType::Input),
        1 => Some(IoType::Output),
        _ => None,
    }
}

fn io_protocol_from_u8(v: u8) -> Option<IoProtocol> {
    match v {
        0 => Some(IoProtocol::None),
        1 => Some(IoProtocol::Modbus),
        2 => Some(IoProtocol::S7),
        3 => Some(IoProtocol::Enip),
        4 => Some(IoProtocol::Gpio),
        5 => Some(IoProtocol::Mqtt),
        6 => Some(IoProtocol::Opcua),
        _ => None,
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], BundleError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(BundleError::Truncated { needed: n, offset: self.pos })?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or(BundleError::Truncated { needed: n, offset: self.pos })?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, BundleError> {
        Ok(self.take(1)?[0])
    }

    fn u16le(&mut self) -> Result<u16, BundleError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32le(&mut self) -> Result<u32, BundleError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32le(&mut self) -> Result<i32, BundleError> {
        Ok(self.u32le()? as i32)
    }

    /// A length-prefixed name (1 length byte, then that many bytes).
    fn name(&mut self) -> Result<String, BundleError> {
        let len = self.u8()? as usize;
        let bytes = self.take(len)?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_module() -> CompiledModule {
        CompiledModule {
            name: "Pump".into(),
            instructions: vec![
                Instruction { opcode: Opcode::PushBool, operand: Some(1) },
                Instruction { opcode: Opcode::StoreVar, operand: Some(0) },
                Instruction { opcode: Opcode::Halt, operand: None },
            ],
            variable_slots: vec![
                ("pump".to_string(), PlcValue::Bool(true)),
                ("level".to_string(), PlcValue::Real(1.5)),
                ("label".to_string(), PlcValue::String("west".into())),
                ("steps".to_string(), PlcValue::Dint(-25)),
            ],
            pou_table: vec![("Main".into(), 0)],
            name_constants: vec!["west".to_string()],
            line_map: vec![(0, 1)],
        }
    }

    #[test]
    fn round_trips_through_serializer() {
        let bytes = mt_plc_types::serialize_bytecode(&sample_module(), &[]).unwrap();
        let bundle = load_vlt1(&bytes).unwrap();

        assert!(bundle.io_mappings.is_empty());
        let loaded = bundle.module;
        assert_eq!(loaded.instructions.len(), 3);
        assert_eq!(loaded.instructions[1].opcode, Opcode::StoreVar);
        assert_eq!(loaded.instructions[2].operand, None);
        assert_eq!(
            loaded.variable_slots[0],
            ("pump".to_string(), PlcValue::Bool(true))
        );
        assert_eq!(
            loaded.variable_slots[1],
            ("level".to_string(), PlcValue::Real(1.5))
        );
        assert_eq!(
            loaded.variable_slots[2],
            ("label".to_string(), PlcValue::String("west".into()))
        );
        assert_eq!(
            loaded.variable_slots[3],
            ("steps".to_string(), PlcValue::Dint(-25))
        );
        // The format carries no line info or module name.
        assert!(loaded.line_map.is_empty());
        assert_eq!(loaded.name, "");
    }

    #[test]
    fn round_trips_io_mappings() {
        let mappings = vec![IoMapping {
            variable_index: 1,
            io_type: IoType::Input,
            protocol: IoProtocol::Modbus,
            address: 0x1001,
        }];
        let bytes = mt_plc_types::serialize_bytecode(&sample_module(), &mappings).unwrap();
        let bundle = load_vlt1(&bytes).unwrap();
        let loaded = &bundle.io_mappings;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].variable_index, 1);
        assert!(matches!(loaded[0].io_type, IoType::Input));
        assert!(matches!(loaded[0].protocol, IoProtocol::Modbus));
        assert_eq!(loaded[0].address, 0x1001);
    }

    #[test]
    fn rejects_corrupted_crc() {
        let mut bytes = mt_plc_types::serialize_bytecode(&sample_module(), &[]).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(matches!(load_vlt1(&bytes), Err(BundleError::CrcMismatch { .. })));
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(matches!(load_vlt1(&[0u8; 4]), Err(BundleError::Truncated { .. })));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = mt_plc_types::serialize_bytecode(&sample_module(), &[]).unwrap();
        bytes[0] = 0;
        // Keep the stored CRC valid so the magic check is what fires.
        let crc = crc32_le(&bytes[..bytes.len() - 4]);
        let n = bytes.len();
        bytes[n - 4..].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(load_vlt1(&bytes), Err(BundleError::BadMagic)));
    }
}
