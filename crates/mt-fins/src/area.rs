//! FINS memory areas (CS/CJ/CP/NJ mode codes) and address parsing ("D100", "CIO0.05", "W10.01", "E1_200").

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FinsArea {
    Cio,
    Wr,
    Hr,
    Ar,
    Dm,
    /// Extended data memory bank 0 to 0x18 (CJ2 has banks 0 to 0x18; CJ1/CS1 0 to 0xC).
    Em(u8),
    /// Timer present values (T0000 = word 0).
    Tim,
    /// Counter present values (C0000 = word 0x8000 of the same area code).
    Cnt,
}

impl FinsArea {
    /// Area code for word access.
    pub fn word_code(self) -> u8 {
        match self {
            FinsArea::Cio => 0xB0,
            FinsArea::Wr => 0xB1,
            FinsArea::Hr => 0xB2,
            FinsArea::Ar => 0xB3,
            FinsArea::Dm => 0x82,
            FinsArea::Em(bank) if bank <= 0x0F => 0xA0 + bank,
            FinsArea::Em(bank) => 0x60 + (bank - 0x10),
            FinsArea::Tim | FinsArea::Cnt => 0x89,
        }
    }

    /// Area code for bit access; timers and counters have no bit access to their present values.
    pub fn bit_code(self) -> Option<u8> {
        Some(match self {
            FinsArea::Cio => 0x30,
            FinsArea::Wr => 0x31,
            FinsArea::Hr => 0x32,
            FinsArea::Ar => 0x33,
            FinsArea::Dm => 0x02,
            FinsArea::Em(bank) if bank <= 0x0F => 0x20 + bank,
            FinsArea::Em(bank) => 0xE0 + (bank - 0x10),
            FinsArea::Tim | FinsArea::Cnt => return None,
        })
    }

    /// Offset added to the word address on the wire (counters share the timer area code from 0x8000).
    pub fn wire_offset(self) -> u16 {
        if self == FinsArea::Cnt { 0x8000 } else { 0 }
    }

    /// Area from a code and wire address (server side). Returns the area, whether it is bit access, and the word
    /// address within the area.
    pub fn from_code(code: u8, wire_addr: u16) -> Option<(FinsArea, bool, u16)> {
        let (area, bit) = match code {
            0xB0 => (FinsArea::Cio, false),
            0xB1 => (FinsArea::Wr, false),
            0xB2 => (FinsArea::Hr, false),
            0xB3 => (FinsArea::Ar, false),
            0x82 => (FinsArea::Dm, false),
            0x30 => (FinsArea::Cio, true),
            0x31 => (FinsArea::Wr, true),
            0x32 => (FinsArea::Hr, true),
            0x33 => (FinsArea::Ar, true),
            0x02 => (FinsArea::Dm, true),
            0xA0..=0xAF => (FinsArea::Em(code - 0xA0), false),
            0x60..=0x68 => (FinsArea::Em(code - 0x60 + 0x10), false),
            0x20..=0x2F => (FinsArea::Em(code - 0x20), true),
            0xE0..=0xE8 => (FinsArea::Em(code - 0xE0 + 0x10), true),
            0x89 if wire_addr >= 0x8000 => return Some((FinsArea::Cnt, false, wire_addr - 0x8000)),
            0x89 => (FinsArea::Tim, false),
            _ => return None,
        };
        Some((area, bit, wire_addr))
    }

    pub fn name(self) -> String {
        match self {
            FinsArea::Cio => "CIO".into(),
            FinsArea::Wr => "W".into(),
            FinsArea::Hr => "H".into(),
            FinsArea::Ar => "A".into(),
            FinsArea::Dm => "D".into(),
            FinsArea::Em(b) => format!("E{b:X}_"),
            FinsArea::Tim => "T".into(),
            FinsArea::Cnt => "C".into(),
        }
    }
}

/// A word, or a bit of a word ("CIO0.05" is bit 5 of CIO 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinsAddress {
    pub area: FinsArea,
    pub word: u16,
    pub bit: Option<u8>,
}

impl FinsAddress {
    pub fn new(area: FinsArea, word: u16) -> Self {
        Self { area, word, bit: None }
    }
}

impl fmt::Display for FinsAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.area.name(), self.word)?;
        if let Some(b) = self.bit {
            write!(f, ".{b:02}")?;
        }
        Ok(())
    }
}

/// Parse an address as CX-Programmer writes it: `D100` (or `DM100`), `CIO100` (or a bare `100`), `W10`, `H0` (`HR0`),
/// `A0` (`AR0`), `E0_100` (EM bank 0 word 100), `T0`, `C0`. A bit is `.00` to `.15` (`CIO0.05`, `D100.15`).
pub fn parse_address(input: &str) -> Result<FinsAddress, String> {
    let s: String = input.trim().to_ascii_uppercase().chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Err("empty address".into());
    }
    let (area, rest): (FinsArea, &str) = if let Some(r) = s.strip_prefix("CIO") {
        (FinsArea::Cio, r)
    } else if let Some(r) = s.strip_prefix("DM") {
        (FinsArea::Dm, r)
    } else if let Some(r) = s.strip_prefix("HR") {
        (FinsArea::Hr, r)
    } else if let Some(r) = s.strip_prefix("AR") {
        (FinsArea::Ar, r)
    } else if let Some(r) = s.strip_prefix("WR") {
        (FinsArea::Wr, r)
    } else if let Some(r) = s.strip_prefix("TIM") {
        (FinsArea::Tim, r)
    } else if let Some(r) = s.strip_prefix("CNT") {
        (FinsArea::Cnt, r)
    } else if s.starts_with('E') && s.contains('_') {
        let (bank, r) = s[1..].split_once('_').expect("checked for _");
        let bank = u8::from_str_radix(bank, 16).map_err(|_| format!("'{bank}' is not an EM bank (0 to 18 hex)"))?;
        if bank > 0x18 {
            return Err(format!("EM bank {bank:X} does not exist (0 to 18 hex)"));
        }
        (FinsArea::Em(bank), r)
    } else {
        match s.chars().next() {
            Some('D') => (FinsArea::Dm, &s[1..]),
            Some('W') => (FinsArea::Wr, &s[1..]),
            Some('H') => (FinsArea::Hr, &s[1..]),
            Some('A') => (FinsArea::Ar, &s[1..]),
            Some('T') => (FinsArea::Tim, &s[1..]),
            Some('C') => (FinsArea::Cnt, &s[1..]),
            Some(c) if c.is_ascii_digit() => (FinsArea::Cio, s.as_str()),
            _ => return Err(format!("unknown area in '{input}' (use D, CIO, W, H, A, E0_, T, C)")),
        }
    };
    let (word_part, bit_part) = match rest.split_once('.') {
        Some((w, b)) => (w, Some(b)),
        None => (rest, None),
    };
    let word: u32 = word_part.parse().map_err(|_| format!("'{word_part}' is not a word number"))?;
    let max = match area {
        FinsArea::Tim | FinsArea::Cnt => 4095,
        _ => 32767,
    };
    if word > max {
        return Err(format!("word {word} is out of range (0 to {max})"));
    }
    let bit = match bit_part {
        None => None,
        Some(b) => {
            if area.bit_code().is_none() {
                return Err("timer and counter present values have no bits".into());
            }
            match b.parse::<u8>() {
                Ok(v) if v <= 15 => Some(v),
                _ => return Err(format!("bit '.{b}' must be 00 to 15")),
            }
        }
    };
    Ok(FinsAddress { area, word: word as u16, bit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cx_programmer_addresses() {
        assert_eq!(parse_address("D100").unwrap(), FinsAddress::new(FinsArea::Dm, 100));
        assert_eq!(parse_address("dm100").unwrap(), FinsAddress::new(FinsArea::Dm, 100));
        assert_eq!(parse_address("CIO 0.05").unwrap(), FinsAddress { area: FinsArea::Cio, word: 0, bit: Some(5) });
        assert_eq!(parse_address("0.05").unwrap(), FinsAddress { area: FinsArea::Cio, word: 0, bit: Some(5) });
        assert_eq!(parse_address("W10.15").unwrap(), FinsAddress { area: FinsArea::Wr, word: 10, bit: Some(15) });
        assert_eq!(parse_address("HR5").unwrap(), FinsAddress::new(FinsArea::Hr, 5));
        assert_eq!(parse_address("A448").unwrap(), FinsAddress::new(FinsArea::Ar, 448));
        assert_eq!(parse_address("E1_200").unwrap(), FinsAddress::new(FinsArea::Em(1), 200));
        assert_eq!(parse_address("T5").unwrap(), FinsAddress::new(FinsArea::Tim, 5));
        assert_eq!(parse_address("C7").unwrap(), FinsAddress::new(FinsArea::Cnt, 7));
        assert_eq!(parse_address("CIO0.05").unwrap().to_string(), "CIO0.05");
    }

    #[test]
    fn rejects_bad_addresses() {
        assert!(parse_address("").is_err());
        assert!(parse_address("Q1").is_err());
        assert!(parse_address("D100.16").is_err());
        assert!(parse_address("D40000").is_err());
        assert!(parse_address("T5.01").is_err());
        assert!(parse_address("E1F_0").is_err());
    }

    #[test]
    fn area_codes_round_trip() {
        for a in [FinsArea::Cio, FinsArea::Wr, FinsArea::Hr, FinsArea::Ar, FinsArea::Dm, FinsArea::Em(0), FinsArea::Em(0xC), FinsArea::Em(0x12)] {
            assert_eq!(FinsArea::from_code(a.word_code(), 3), Some((a, false, 3)));
            assert_eq!(FinsArea::from_code(a.bit_code().unwrap(), 3), Some((a, true, 3)));
        }
        assert_eq!(FinsArea::from_code(0x89, 0x8005), Some((FinsArea::Cnt, false, 5)));
        assert_eq!(FinsArea::from_code(0x89, 5), Some((FinsArea::Tim, false, 5)));
    }
}
