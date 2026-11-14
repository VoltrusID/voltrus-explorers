//! Data types over PLC words, and the read planner that turns a tag list into as few batch reads as possible.
//!
//! Both Mitsubishi and Omron keep 32-bit values with the low word at the lower address, so `D100` as Int32 is
//! `D100 | D101 << 16`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Bool,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Float32,
    Bcd16,
    Hex16,
}

impl DataType {
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "Bool" => Self::Bool,
            "Int16" => Self::Int16,
            "UInt16" => Self::UInt16,
            "Int32" => Self::Int32,
            "UInt32" => Self::UInt32,
            "Float32" => Self::Float32,
            "BCD16" | "Bcd16" => Self::Bcd16,
            "Hex16" => Self::Hex16,
            _ => return None,
        })
    }

    pub fn words(self) -> u32 {
        match self {
            Self::Int32 | Self::UInt32 | Self::Float32 => 2,
            _ => 1,
        }
    }

    pub fn bits(self) -> u32 {
        if self == Self::Bool { 1 } else { self.words() * 16 }
    }
}

fn dword(w: &[u16]) -> u32 {
    w[0] as u32 | (w[1] as u32) << 16
}

/// Value of a non-Bool type from its words.
pub fn decode(dt: DataType, w: &[u16]) -> Value {
    match dt {
        DataType::Bool => json!(w.first().is_some_and(|v| *v & 1 != 0)),
        DataType::Int16 => json!(w[0] as i16),
        DataType::UInt16 => json!(w[0]),
        DataType::Int32 => json!(dword(w) as i32),
        DataType::UInt32 => json!(dword(w)),
        DataType::Float32 => {
            let f = f32::from_bits(dword(w));
            if f.is_finite() { json!(f as f64) } else { json!(f.to_string()) }
        }
        DataType::Bcd16 => match bcd_to_u16(w[0]) {
            Some(v) => json!(v),
            None => json!(format!("not BCD (0x{:04X})", w[0])),
        },
        DataType::Hex16 => json!(format!("0x{:04X}", w[0])),
    }
}

fn bcd_to_u16(w: u16) -> Option<u16> {
    let mut out = 0u16;
    for shift in [12, 8, 4, 0] {
        let d = (w >> shift) & 0xF;
        if d > 9 {
            return None;
        }
        out = out * 10 + d;
    }
    Some(out)
}

fn number(v: &Value) -> Result<f64, String> {
    v.as_f64()
        .or_else(|| v.as_bool().map(|b| b as u8 as f64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .ok_or_else(|| format!("'{}' is not a number", v.as_str().map_or_else(|| v.to_string(), str::to_string)))
}

fn integer(v: &Value, min: f64, max: f64) -> Result<i64, String> {
    let n = number(v)?;
    if n.fract() != 0.0 {
        return Err(format!("{n} is not a whole number"));
    }
    if n < min || n > max {
        return Err(format!("{n} is out of range ({min} to {max})"));
    }
    Ok(n as i64)
}

pub fn parse_bool(v: &Value) -> Result<bool, String> {
    if let Some(b) = v.as_bool() {
        return Ok(b);
    }
    match v.as_str().map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if ["true", "on", "1"].contains(&s.as_str()) => Ok(true),
        Some(s) if ["false", "off", "0"].contains(&s.as_str()) => Ok(false),
        _ => Ok(number(v)? != 0.0),
    }
}

/// Words for a value typed in the write dialog (a number, or text holding one; Hex16 also takes `0x1A2B` or `1A2B`).
pub fn encode(dt: DataType, v: &Value) -> Result<Vec<u16>, String> {
    let split = |d: u32| vec![(d & 0xFFFF) as u16, (d >> 16) as u16];
    Ok(match dt {
        DataType::Bool => vec![parse_bool(v)? as u16],
        DataType::Int16 => vec![integer(v, i16::MIN as f64, i16::MAX as f64)? as i16 as u16],
        DataType::UInt16 => vec![integer(v, 0.0, u16::MAX as f64)? as u16],
        DataType::Int32 => split(integer(v, i32::MIN as f64, i32::MAX as f64)? as i32 as u32),
        DataType::UInt32 => split(integer(v, 0.0, u32::MAX as f64)? as u32),
        DataType::Float32 => split((number(v)? as f32).to_bits()),
        DataType::Bcd16 => {
            let n = integer(v, 0.0, 9999.0)? as u16;
            vec![(n / 1000) << 12 | (n / 100 % 10) << 8 | (n / 10 % 10) << 4 | (n % 10)]
        }
        DataType::Hex16 => {
            let s = match v.as_str() {
                Some(s) => s.trim().trim_start_matches("0x").trim_start_matches("0X").to_string(),
                None => return Ok(vec![integer(v, 0.0, u16::MAX as f64)? as u16]),
            };
            vec![u16::from_str_radix(&s, 16).map_err(|_| format!("'{s}' is not a 16-bit hex value"))?]
        }
    })
}

// ── Read planning ──

/// Where a tag lives: an area key, the absolute bit offset of its first bit (word * 16 + bit on word areas, the device
/// number on Mitsubishi bit devices) and its length in bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loc<A> {
    pub area: A,
    pub start_bit: u64,
    pub nbits: u32,
}

impl<A> Loc<A> {
    fn first_word(&self) -> u64 {
        self.start_bit / 16
    }

    fn last_word(&self) -> u64 {
        (self.start_bit + self.nbits.max(1) as u64 - 1) / 16
    }
}

/// A batch read: `count` words of `area` from word `first`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch<A> {
    pub area: A,
    pub first: u64,
    pub count: u64,
}

/// Group tags into batch reads: per area, word ranges are merged when the gap between them is at most `gap` words and
/// the merged batch stays within `max` words.
pub fn plan<A: Ord + Copy>(locs: &[Loc<A>], gap: u64, max: u64) -> Vec<Batch<A>> {
    let mut by_area: BTreeMap<A, Vec<(u64, u64)>> = BTreeMap::new();
    for l in locs {
        by_area.entry(l.area).or_default().push((l.first_word(), l.last_word()));
    }
    let mut out = Vec::new();
    for (area, mut ranges) in by_area {
        ranges.sort();
        let mut cur: Option<(u64, u64)> = None;
        for (s, e) in ranges {
            cur = match cur {
                Some((cs, ce)) if s <= ce + 1 + gap && e.max(ce) - cs < max => Some((cs, e.max(ce))),
                Some((cs, ce)) => {
                    out.push(Batch { area, first: cs, count: ce - cs + 1 });
                    Some((s, e))
                }
                None => Some((s, e)),
            };
        }
        if let Some((cs, ce)) = cur {
            out.push(Batch { area, first: cs, count: ce - cs + 1 });
        }
    }
    out
}

/// The bits of `loc` from words read from `first`: `ceil(nbits / 16)` words, bit 0 of the first word being the tag's
/// first bit (a single bit for Bool).
pub fn extract(words: &[u16], first: u64, start_bit: u64, nbits: u32) -> Option<Vec<u16>> {
    let bit = |abs: u64| -> Option<bool> {
        let w = (abs / 16).checked_sub(first)? as usize;
        Some(words.get(w)? & (1 << (abs % 16)) != 0)
    };
    let n_out = nbits.div_ceil(16) as usize;
    let mut out = vec![0u16; n_out];
    for i in 0..nbits as u64 {
        if bit(start_bit + i)? {
            out[(i / 16) as usize] |= 1 << (i % 16);
        }
    }
    Some(out)
}

/// The tag's words, or its bit, decoded.
pub fn value_at(words: &[u16], first: u64, start_bit: u64, dt: DataType) -> Option<Value> {
    let w = extract(words, first, start_bit, dt.bits())?;
    Some(decode(dt, &w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_types_low_word_first() {
        assert_eq!(decode(DataType::Int16, &[0xFFFF]), json!(-1));
        assert_eq!(decode(DataType::UInt32, &[0x0002, 0x0001]), json!(0x0001_0002));
        assert_eq!(decode(DataType::Int32, &[0xFFFE, 0xFFFF]), json!(-2));
        let f = 1450.5f32.to_bits();
        assert_eq!(decode(DataType::Float32, &[(f & 0xFFFF) as u16, (f >> 16) as u16]), json!(1450.5));
        assert_eq!(decode(DataType::Bcd16, &[0x1234]), json!(1234));
        assert_eq!(decode(DataType::Bcd16, &[0x12A4]), json!("not BCD (0x12A4)"));
        assert_eq!(decode(DataType::Hex16, &[0x002A]), json!("0x002A"));
    }

    #[test]
    fn encodes_what_the_write_dialog_sends() {
        assert_eq!(encode(DataType::Int16, &json!("-5")).unwrap(), vec![0xFFFB]);
        assert_eq!(encode(DataType::UInt32, &json!(70000)).unwrap(), vec![0x1170, 0x0001]);
        assert_eq!(encode(DataType::Bcd16, &json!("1234")).unwrap(), vec![0x1234]);
        assert_eq!(encode(DataType::Hex16, &json!("0x1a2b")).unwrap(), vec![0x1A2B]);
        assert_eq!(encode(DataType::Bool, &json!("on")).unwrap(), vec![1]);
        let w = encode(DataType::Float32, &json!("2.5")).unwrap();
        assert_eq!(decode(DataType::Float32, &w), json!(2.5));
    }

    #[test]
    fn refuses_values_that_do_not_fit() {
        assert!(encode(DataType::Int16, &json!(40000)).is_err());
        assert!(encode(DataType::UInt16, &json!(-1)).is_err());
        assert!(encode(DataType::UInt16, &json!("1.5")).is_err());
        assert!(encode(DataType::Bcd16, &json!(10000)).is_err());
        assert!(encode(DataType::UInt16, &json!("abc")).unwrap_err().contains("abc"));
    }

    #[test]
    fn plan_merges_close_ranges_per_area() {
        let l = |area: u8, word: u64, bits: u32| Loc { area, start_bit: word * 16, nbits: bits };
        let locs = [l(1, 100, 16), l(1, 101, 32), l(1, 104, 16), l(1, 300, 16), l(2, 0, 1)];
        let p = plan(&locs, 8, 960);
        assert_eq!(
            p,
            vec![
                Batch { area: 1, first: 100, count: 5 },
                Batch { area: 1, first: 300, count: 1 },
                Batch { area: 2, first: 0, count: 1 },
            ]
        );
        // The size cap splits batches.
        let p = plan(&[l(1, 0, 16), l(1, 5, 16)], 8, 4);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn extracts_bits_and_unaligned_words() {
        // Words 6 and 7 (bits 96..127): bit 101 and bit 103 set, and 0xABCD in word 7.
        let words = [(1 << 5) | (1 << 7), 0xABCD];
        assert_eq!(value_at(&words, 6, 101, DataType::Bool), Some(json!(true)));
        assert_eq!(value_at(&words, 6, 102, DataType::Bool), Some(json!(false)));
        assert_eq!(value_at(&words, 6, 112, DataType::UInt16), Some(json!(0xABCD)));
        // 16 bits from bit 100 span both words: bits 100..111 of word 6, then bits 112..115 of word 7.
        let w = extract(&words, 6, 100, 16).unwrap()[0];
        assert_eq!(w & 0x0FFF, ((1 << 5) | (1 << 7)) >> 4);
        assert_eq!(w >> 12, 0xD);
        assert_eq!(extract(&words, 6, 130, 1), None);
    }
}
