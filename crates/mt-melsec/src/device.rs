//! MELSEC device codes and address parsing ("D100", "M0", "X1F", "D100.F").

use std::fmt;

use serde::{Deserialize, Serialize};

/// A device (memory area) of a MELSEC Q/L/iQ-R/iQ-F CPU, as addressed by the Q-compatible MC protocol commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum McDevice {
    SM,
    SD,
    X,
    Y,
    M,
    L,
    F,
    V,
    B,
    D,
    W,
    TS,
    TC,
    TN,
    SS,
    SC,
    SN,
    CS,
    CC,
    CN,
    SB,
    SW,
    DX,
    DY,
    Z,
    R,
    ZR,
}

/// Every device, two-letter names first so that parsing "DX10" does not stop at "D".
pub const ALL_DEVICES: [McDevice; 27] = [
    McDevice::SM,
    McDevice::SD,
    McDevice::TS,
    McDevice::TC,
    McDevice::TN,
    McDevice::SS,
    McDevice::SC,
    McDevice::SN,
    McDevice::CS,
    McDevice::CC,
    McDevice::CN,
    McDevice::SB,
    McDevice::SW,
    McDevice::DX,
    McDevice::DY,
    McDevice::ZR,
    McDevice::X,
    McDevice::Y,
    McDevice::M,
    McDevice::L,
    McDevice::F,
    McDevice::V,
    McDevice::B,
    McDevice::D,
    McDevice::W,
    McDevice::Z,
    McDevice::R,
];

impl McDevice {
    /// Device code in binary frames (Q/L series table of the MC protocol reference manual).
    pub fn binary_code(self) -> u8 {
        use McDevice::*;
        match self {
            SM => 0x91,
            SD => 0xA9,
            X => 0x9C,
            Y => 0x9D,
            M => 0x90,
            L => 0x92,
            F => 0x93,
            V => 0x94,
            B => 0xA0,
            D => 0xA8,
            W => 0xB4,
            TS => 0xC1,
            TC => 0xC0,
            TN => 0xC2,
            SS => 0xC7,
            SC => 0xC6,
            SN => 0xC8,
            CS => 0xC4,
            CC => 0xC3,
            CN => 0xC5,
            SB => 0xA1,
            SW => 0xB5,
            DX => 0xA2,
            DY => 0xA3,
            Z => 0xCC,
            R => 0xAF,
            ZR => 0xB0,
        }
    }

    /// Device code in ASCII frames: two characters, one-letter devices padded with `*`.
    pub fn ascii_code(self) -> &'static str {
        use McDevice::*;
        match self {
            SM => "SM",
            SD => "SD",
            X => "X*",
            Y => "Y*",
            M => "M*",
            L => "L*",
            F => "F*",
            V => "V*",
            B => "B*",
            D => "D*",
            W => "W*",
            TS => "TS",
            TC => "TC",
            TN => "TN",
            SS => "SS",
            SC => "SC",
            SN => "SN",
            CS => "CS",
            CC => "CC",
            CN => "CN",
            SB => "SB",
            SW => "SW",
            DX => "DX",
            DY => "DY",
            Z => "Z*",
            R => "R*",
            ZR => "ZR",
        }
    }

    pub fn name(self) -> &'static str {
        self.ascii_code().trim_end_matches('*')
    }

    pub fn from_binary_code(code: u8) -> Option<Self> {
        ALL_DEVICES.into_iter().find(|d| d.binary_code() == code)
    }

    pub fn from_ascii_code(code: &str) -> Option<Self> {
        ALL_DEVICES.into_iter().find(|d| d.ascii_code() == code)
    }

    /// Bit devices hold one bit per device number; word devices one 16-bit word.
    pub fn is_bit(self) -> bool {
        use McDevice::*;
        matches!(self, SM | X | Y | M | L | F | V | B | TS | TC | SS | SC | CS | CC | SB | DX | DY)
    }

    /// Devices numbered in hexadecimal (in GX Works and in ASCII frames).
    pub fn hex_numbered(self) -> bool {
        use McDevice::*;
        matches!(self, X | Y | B | W | SB | SW | DX | DY | ZR)
    }
}

/// A parsed device address. `bit` is set for a bit of a word device ("D100.F" is bit 15 of D100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct McAddress {
    pub device: McDevice,
    pub number: u32,
    pub bit: Option<u8>,
}

impl McAddress {
    pub fn new(device: McDevice, number: u32) -> Self {
        Self { device, number, bit: None }
    }
}

impl fmt::Display for McAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.device.hex_numbered() && self.device != McDevice::ZR {
            write!(f, "{}{:X}", self.device.name(), self.number)?;
        } else {
            write!(f, "{}{}", self.device.name(), self.number)?;
        }
        if let Some(b) = self.bit {
            write!(f, ".{b:X}")?;
        }
        Ok(())
    }
}

/// Highest device number the 3-byte binary field (and the 6-character ASCII field) can carry.
pub const MAX_DEVICE_NUMBER: u32 = 0xFF_FFFF;

/// Parse an address as GX Works writes it: `D100`, `M0`, `X1F` (hex), `W1A` (hex), `ZR1000`, `D100.F` (bit 15 of D100).
///
/// `xy_octal`: iQ-F (FX5) CPUs number X and Y in octal (`X17` is the 16th input); every other series uses hex.
/// ZR is entered in decimal, as GX Works shows it.
pub fn parse_address(input: &str, xy_octal: bool) -> Result<McAddress, String> {
    let s = input.trim().to_ascii_uppercase();
    if s.is_empty() {
        return Err("empty address".into());
    }
    let device = ALL_DEVICES
        .into_iter()
        .find(|d| {
            let name = d.name();
            s.starts_with(name) && s[name.len()..].chars().next().is_some_and(|c| c.is_ascii_hexdigit())
        })
        .ok_or_else(|| format!("unknown device in '{input}' (use D, W, R, ZR, M, X, Y, B, L, F, SM, SD, TN, CN ...)"))?;
    let rest = &s[device.name().len()..];
    let (num_part, bit_part) = match rest.split_once('.') {
        Some((n, b)) => (n, Some(b)),
        None => (rest, None),
    };
    let radix = match device {
        McDevice::X | McDevice::Y if xy_octal => 8,
        McDevice::ZR => 10,
        d if d.hex_numbered() => 16,
        _ => 10,
    };
    let number = u32::from_str_radix(num_part, radix)
        .map_err(|_| format!("'{num_part}' is not a valid {} device number", radix_name(radix)))?;
    if number > MAX_DEVICE_NUMBER {
        return Err(format!("device number {number} is out of range"));
    }
    let bit = match bit_part {
        None => None,
        Some(b) => {
            if device.is_bit() {
                return Err(format!("{} is a bit device; a bit number (.n) only applies to word devices", device.name()));
            }
            let v = if b.len() == 1 {
                u8::from_str_radix(b, 16).ok()
            } else {
                b.parse::<u8>().ok()
            };
            match v {
                Some(v) if v <= 15 => Some(v),
                _ => return Err(format!("bit '.{b}' must be 0 to F")),
            }
        }
    };
    Ok(McAddress { device, number, bit })
}

fn radix_name(radix: u32) -> &'static str {
    match radix {
        8 => "octal",
        16 => "hexadecimal",
        _ => "decimal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_word_and_bit_devices() {
        assert_eq!(parse_address("D100", false).unwrap(), McAddress::new(McDevice::D, 100));
        assert_eq!(parse_address("m0", false).unwrap(), McAddress::new(McDevice::M, 0));
        assert_eq!(parse_address("X1F", false).unwrap(), McAddress::new(McDevice::X, 0x1F));
        assert_eq!(parse_address("W1A", false).unwrap(), McAddress::new(McDevice::W, 0x1A));
        assert_eq!(parse_address("ZR1000", false).unwrap(), McAddress::new(McDevice::ZR, 1000));
        assert_eq!(parse_address("SM400", false).unwrap(), McAddress::new(McDevice::SM, 400));
        assert_eq!(parse_address("DX10", false).unwrap(), McAddress::new(McDevice::DX, 0x10));
        assert_eq!(parse_address("TN5", false).unwrap(), McAddress::new(McDevice::TN, 5));
    }

    #[test]
    fn bit_of_word_is_hex() {
        let a = parse_address("D100.F", false).unwrap();
        assert_eq!(a.bit, Some(15));
        assert_eq!(parse_address("D100.12", false).unwrap().bit, Some(12));
        assert!(parse_address("D100.16", false).is_err());
        assert!(parse_address("M10.1", false).is_err());
        assert_eq!(a.to_string(), "D100.F");
    }

    #[test]
    fn fx5_inputs_are_octal() {
        assert_eq!(parse_address("X17", true).unwrap().number, 15);
        assert!(parse_address("X18", true).is_err());
        assert_eq!(parse_address("X17", false).unwrap().number, 0x17);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_address("", false).is_err());
        assert!(parse_address("Q100", false).is_err());
        assert!(parse_address("D", false).is_err());
        assert!(parse_address("D1X", false).is_err());
    }

    #[test]
    fn codes_round_trip() {
        for d in ALL_DEVICES {
            assert_eq!(McDevice::from_binary_code(d.binary_code()), Some(d));
            assert_eq!(McDevice::from_ascii_code(d.ascii_code()), Some(d));
        }
    }
}
