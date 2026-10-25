use std::fmt;

use serde::{Deserialize, Serialize};

/// Poll a Modbus device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModbusMasterSpec {
    pub link: ModbusLink,
    pub unit_id: u8,
    pub poll_ms: u64,
    pub timeout_ms: u64,
    pub word_order: WordOrder,
    pub blocks: Vec<RegisterBlock>,
}

impl Default for ModbusMasterSpec {
    fn default() -> Self {
        Self {
            link: ModbusLink::Tcp {
                host: "127.0.0.1".into(),
                port: 502,
            },
            unit_id: 1,
            poll_ms: 1000,
            timeout_ms: 2000,
            word_order: WordOrder::Big,
            blocks: vec![RegisterBlock {
                name: "Holding".into(),
                area: Area::Holding,
                start: 0,
                count: 10,
                data_type: DataType::U16,
                unit: String::new(),
            }],
        }
    }
}

impl ModbusMasterSpec {
    /// Number of values the blocks produce, one point per value.
    pub fn point_count(&self) -> usize {
        self.blocks
            .iter()
            .map(|block| usize::from(block.count))
            .sum()
    }
}

/// How the master reaches the device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum ModbusLink {
    Tcp { host: String, port: u16 },
    Rtu(SerialLink),
}

impl fmt::Display for ModbusLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp { host, port } => write!(f, "{host}:{port}"),
            Self::Rtu(serial) => write!(
                f,
                "{} {} {}{}{}",
                serial.port,
                serial.baud,
                serial.data_bits,
                serial.parity.letter(),
                serial.stop_bits
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialLink {
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
}

impl Default for SerialLink {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 9600,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

impl Parity {
    pub const ALL: [Parity; 3] = [Parity::None, Parity::Even, Parity::Odd];

    pub fn letter(self) -> char {
        match self {
            Self::None => 'N',
            Self::Odd => 'O',
            Self::Even => 'E',
        }
    }
}

/// A run of consecutive values of one type in one area.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterBlock {
    pub name: String,
    pub area: Area,
    /// 0-based protocol address of the first value.
    pub start: u16,
    /// Number of values (not registers) in the block.
    pub count: u16,
    pub data_type: DataType,
    /// Engineering unit of every value, e.g. from a device template.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

impl RegisterBlock {
    /// Protocol address of the `index`-th value in the block, if it fits in
    /// the 16-bit address space.
    pub fn address_of(&self, index: u16) -> Option<u16> {
        let step = if self.area.is_bit() {
            1
        } else {
            self.data_type.words()
        };
        index
            .checked_mul(step)
            .and_then(|offset| self.start.checked_add(offset))
    }

    /// Last protocol address the block touches, or `None` when it runs past
    /// 65535.
    pub fn end_address(&self) -> Option<u16> {
        if self.count == 0 {
            return Some(self.start);
        }
        let last = self.address_of(self.count - 1)?;
        let width = if self.area.is_bit() {
            1
        } else {
            self.data_type.words()
        };
        last.checked_add(width - 1)
    }
}

/// The four Modbus data areas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Coil,
    Discrete,
    Input,
    Holding,
}

impl Area {
    pub const ALL: [Area; 4] = [Area::Holding, Area::Input, Area::Coil, Area::Discrete];

    pub fn name(self) -> &'static str {
        match self {
            Self::Coil => "Coils",
            Self::Discrete => "Discrete inputs",
            Self::Input => "Input registers",
            Self::Holding => "Holding registers",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Coil => "Coil",
            Self::Discrete => "DI",
            Self::Input => "IR",
            Self::Holding => "HR",
        }
    }

    /// Single-bit areas hold booleans, not registers.
    pub fn is_bit(self) -> bool {
        matches!(self, Self::Coil | Self::Discrete)
    }

    pub fn writable(self) -> bool {
        matches!(self, Self::Coil | Self::Holding)
    }

    /// The conventional 1-based "Modicon" number of a protocol address, e.g.
    /// holding register 0 is 40001. Addresses past 9998 use the six-digit
    /// form (holding register 30770 is 430771), as five digits would collide
    /// with the next area.
    pub fn modicon(self, address: u16) -> u32 {
        let prefix = match self {
            Self::Coil => 0,
            Self::Discrete => 1,
            Self::Input => 3,
            Self::Holding => 4,
        };
        let scale = if address < 9_999 { 10_000 } else { 100_000 };
        prefix * scale + 1 + u32::from(address)
    }

    /// [`Self::modicon`] as shown to the user. Coils always use the six-digit
    /// form (coil 0 is 000001), so a coil never reads like a discrete input.
    pub fn modicon_label(self, address: u16) -> String {
        match self {
            Self::Coil => format!("{:06}", self.modicon(address)),
            _ => self.modicon(address).to_string(),
        }
    }
}

/// How the words of a register value are interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    U16,
    I16,
    I32,
    F32,
    F64,
}

impl DataType {
    pub const ALL: [DataType; 5] = [
        DataType::U16,
        DataType::I16,
        DataType::I32,
        DataType::F32,
        DataType::F64,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::U16 => "UInt16",
            Self::I16 => "Int16",
            Self::I32 => "Int32",
            Self::F32 => "Float32",
            Self::F64 => "Float64",
        }
    }

    /// Registers one value spans.
    pub fn words(self) -> u16 {
        match self {
            Self::U16 | Self::I16 => 1,
            Self::I32 | Self::F32 => 2,
            Self::F64 => 4,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

/// Order of the words (and bytes) of multi-register values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WordOrder {
    /// AB CD: most significant word first.
    #[default]
    Big,
    /// CD AB: least significant word first.
    Little,
    /// BA DC: bytes swapped inside each register.
    Swapped,
}

impl WordOrder {
    pub const ALL: [WordOrder; 3] = [WordOrder::Big, WordOrder::Little, WordOrder::Swapped];

    pub fn name(self) -> &'static str {
        match self {
            Self::Big => "AB CD",
            Self::Little => "CD AB",
            Self::Swapped => "BA DC",
        }
    }
}

/// Serve a register image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusSlaveSpec {
    pub bind: String,
    pub port: u16,
    pub unit_id: u8,
    /// Device template driving the registers; `None` serves a plain image.
    pub template: Option<String>,
    /// Registers in the image when no template sets the size, at most
    /// [`Self::MAX_PLAIN_REGISTERS`].
    pub registers: u16,
}

impl ModbusSlaveSpec {
    /// Largest plain register image: every register becomes a point with its
    /// own history, so the image stays a size the app can chart.
    pub const MAX_PLAIN_REGISTERS: u16 = 2_000;
}

impl Default for ModbusSlaveSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 5020,
            unit_id: 1,
            template: None,
            registers: 200,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(area: Area, start: u16, count: u16, data_type: DataType) -> RegisterBlock {
        RegisterBlock {
            name: String::new(),
            area,
            start,
            count,
            data_type,
            unit: String::new(),
        }
    }

    #[test]
    fn values_step_by_their_register_width() {
        let floats = block(Area::Holding, 100, 3, DataType::F32);
        assert_eq!(floats.address_of(2), Some(104));
        assert_eq!(floats.end_address(), Some(105));
        // Bit areas ignore the data type: one address per value.
        let coils = block(Area::Coil, 0, 8, DataType::F64);
        assert_eq!(coils.end_address(), Some(7));
    }

    #[test]
    fn blocks_past_the_address_space_have_no_end() {
        let tail = block(Area::Holding, 65_534, 2, DataType::F32);
        assert_eq!(tail.end_address(), None);
        assert_eq!(
            block(Area::Holding, 65_535, 1, DataType::U16).end_address(),
            Some(65_535)
        );
    }

    #[test]
    fn modicon_numbers_switch_to_six_digits_past_9998() {
        assert_eq!(Area::Holding.modicon(0), 40_001);
        assert_eq!(Area::Holding.modicon(9_998), 49_999);
        assert_eq!(Area::Holding.modicon(30_769), 430_770);
        assert_eq!(Area::Input.modicon(12_000), 312_001);
        assert_eq!(Area::Coil.modicon(20_000), 20_001);
    }

    #[test]
    fn coil_labels_never_collide_with_discrete_inputs() {
        assert_eq!(Area::Coil.modicon_label(0), "000001");
        assert_eq!(Area::Coil.modicon_label(9_999), "010000");
        assert_eq!(Area::Coil.modicon_label(65_535), "065536");
        assert_eq!(Area::Discrete.modicon_label(0), "10001");
        assert_eq!(Area::Holding.modicon_label(30_769), "430770");
    }
}
