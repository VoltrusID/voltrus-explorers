//! Watch-table values: which S7 reads an IEC display type needs, and how the bytes the CPU
//! returns are shown and parsed back, in the notation automation engineers know
//! (`16#00FF`, `T#1s500ms`, `'A'`).

use mt_s7::{S7Area, S7DataType, S7Tag, S7Value};

/// The display types a watch row can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayType {
    Bool,
    Byte,
    Char,
    Sint,
    Usint,
    Word,
    Int,
    Uint,
    Dword,
    Dint,
    Udint,
    Real,
    Time,
    Lword,
    Lint,
    Ulint,
    Lreal,
    String,
    /// S7 timer (`T5`): shown as S5TIME.
    Timer,
    /// S7 counter (`C10`): 0 to 999.
    Counter,
}

impl DisplayType {
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name.trim().to_ascii_uppercase().as_str() {
            "BOOL" => Self::Bool,
            "BYTE" => Self::Byte,
            "CHAR" => Self::Char,
            "SINT" => Self::Sint,
            "USINT" => Self::Usint,
            "WORD" => Self::Word,
            "INT" => Self::Int,
            "UINT" => Self::Uint,
            "DWORD" => Self::Dword,
            "DINT" => Self::Dint,
            "UDINT" => Self::Udint,
            "REAL" => Self::Real,
            "TIME" => Self::Time,
            "LWORD" => Self::Lword,
            "LINT" => Self::Lint,
            "ULINT" => Self::Ulint,
            "LREAL" => Self::Lreal,
            "STRING" => Self::String,
            "S5TIME" | "TIMER" => Self::Timer,
            "COUNTER" => Self::Counter,
            other => return Err(format!("unknown data type {other}")),
        })
    }

    /// The type a watch row gets when the user did not pick one: what the address width implies.
    pub fn default_for(tag: &S7Tag) -> Self {
        match tag.data_type {
            S7DataType::Bool => Self::Bool,
            S7DataType::Byte => Self::Byte,
            S7DataType::Word => Self::Int,
            S7DataType::DWord => Self::Dint,
            S7DataType::Real => Self::Real,
            S7DataType::String => Self::String,
            S7DataType::Timer => Self::Timer,
            S7DataType::Counter => Self::Counter,
        }
    }

    /// Bytes the type occupies in the CPU (strings: unknown until read).
    fn width(self) -> u16 {
        match self {
            Self::Bool | Self::Byte | Self::Char | Self::Sint | Self::Usint => 1,
            Self::Word | Self::Int | Self::Uint | Self::Timer | Self::Counter => 2,
            Self::Dword | Self::Dint | Self::Udint | Self::Real | Self::Time => 4,
            Self::Lword | Self::Lint | Self::Ulint | Self::Lreal => 8,
            Self::String => 0,
        }
    }

    fn is_64_bit(self) -> bool {
        self.width() == 8
    }
}

/// A parsed watch row: where it is and how to show it.
#[derive(Debug, Clone)]
pub struct WatchItem {
    pub display: DisplayType,
    /// The S7 tags to read: one, or two DWords for a 64-bit type (classic S7comm has no
    /// 8-byte transport size).
    pub tags: Vec<S7Tag>,
}

impl WatchItem {
    pub fn parse(address: &str, display: Option<&str>) -> Result<Self, String> {
        let base = S7Tag::parse_address(address).map_err(|e| e.to_string())?;
        let display = match display.filter(|d| !d.trim().is_empty()) {
            Some(name) => DisplayType::parse(name)?,
            None => DisplayType::default_for(&base),
        };

        match base.area {
            S7Area::Timer if display != DisplayType::Timer => {
                return Err("a timer address (T) shows as S5TIME".into());
            }
            S7Area::Counter if display != DisplayType::Counter => {
                return Err("a counter address (C) shows as COUNTER".into());
            }
            S7Area::Timer | S7Area::Counter => return Ok(Self { display, tags: vec![base] }),
            _ if matches!(display, DisplayType::Timer | DisplayType::Counter) => {
                return Err(format!("{} needs a T or C address", type_name(display)));
            }
            _ => {}
        }

        let is_bit = base.data_type == S7DataType::Bool;
        if display == DisplayType::Bool {
            if !is_bit {
                return Err("BOOL needs a bit address such as M0.0 or DB1.DBX2.3".into());
            }
            return Ok(Self { display, tags: vec![base] });
        }
        if is_bit {
            return Err(format!("{} needs a byte address, not a bit", type_name(display)));
        }

        let mut tag = base;
        tag.bit_offset = 0;
        tag.length = 1;
        let tags = match display.width() {
            0 => {
                tag.data_type = S7DataType::String;
                tag.length = 0; // unsized: the client reads the length bytes first
                vec![tag]
            }
            1 => vec![S7Tag { data_type: S7DataType::Byte, ..tag }],
            2 => vec![S7Tag { data_type: S7DataType::Word, ..tag }],
            4 => vec![S7Tag { data_type: S7DataType::DWord, ..tag }],
            _ => {
                let high = S7Tag { data_type: S7DataType::DWord, ..tag };
                let low = S7Tag { offset: high.offset.checked_add(4).ok_or("offset too large")?, ..high.clone() };
                vec![high, low]
            }
        };
        Ok(Self { display, tags })
    }

    /// Show the values read for this item's tags.
    pub fn format(&self, values: &[S7Value]) -> Result<String, String> {
        let raw = |i: usize| -> Result<u64, String> {
            match values.get(i) {
                Some(S7Value::Bool(b)) => Ok(*b as u64),
                Some(S7Value::Byte(b)) => Ok(*b as u64),
                Some(S7Value::Word(w)) => Ok(*w as u64),
                Some(S7Value::DWord(d)) => Ok(*d as u64),
                Some(S7Value::Timer(t)) => Ok(*t as u64),
                Some(S7Value::Counter(c)) => Ok(*c as u64),
                Some(other) => Err(format!("unexpected value {other:?}")),
                None => Err("no value".into()),
            }
        };
        let bits = if self.display.is_64_bit() { (raw(0)? << 32) | raw(1)? } else if self.display == DisplayType::String {
            0
        } else {
            raw(0)?
        };
        Ok(match self.display {
            DisplayType::Bool => (if bits != 0 { "TRUE" } else { "FALSE" }).into(),
            DisplayType::Byte => format!("16#{bits:02X}"),
            DisplayType::Word => format!("16#{bits:04X}"),
            DisplayType::Dword => format!("16#{bits:08X}"),
            DisplayType::Lword => format!("16#{bits:016X}"),
            DisplayType::Char => format_char(bits as u8),
            DisplayType::Sint => (bits as u8 as i8).to_string(),
            DisplayType::Usint => (bits as u8).to_string(),
            DisplayType::Int => (bits as u16 as i16).to_string(),
            DisplayType::Uint => (bits as u16).to_string(),
            DisplayType::Dint => (bits as u32 as i32).to_string(),
            DisplayType::Udint => (bits as u32).to_string(),
            DisplayType::Lint => (bits as i64).to_string(),
            DisplayType::Ulint => bits.to_string(),
            DisplayType::Real => format_real(f32::from_bits(bits as u32)),
            DisplayType::Lreal => format_float(f64::from_bits(bits)),
            DisplayType::Time => format_time("T#", bits as u32 as i32 as i64),
            DisplayType::Timer => format_time("S5T#", bits as i64),
            DisplayType::Counter => format!("C#{bits}"),
            DisplayType::String => match values.first() {
                Some(S7Value::String(s)) => format!("'{}'", s.replace('\'', "$'")),
                other => return Err(format!("unexpected value {other:?}")),
            },
        })
    }

    /// Turn what the user typed into the values to write, one per tag.
    pub fn parse_input(&self, input: &str) -> Result<Vec<S7Value>, String> {
        let text = input.trim();
        let d = self.display;
        let int = |min: i128, max: i128| -> Result<u64, String> {
            let v = parse_int(text)?;
            if v < min || v > max {
                return Err(format!("{text} is out of range for {} ({min} to {max})", type_name(d)));
            }
            Ok(v as u64)
        };
        let bits: u64 = match d {
            DisplayType::Bool => match text.to_ascii_uppercase().as_str() {
                "TRUE" | "1" | "ON" => 1,
                "FALSE" | "0" | "OFF" => 0,
                _ => return Err("BOOL takes TRUE or FALSE".into()),
            },
            DisplayType::Byte | DisplayType::Usint => int(0, u8::MAX as i128)?,
            DisplayType::Sint => int(i8::MIN as i128, i8::MAX as i128)? & 0xFF,
            DisplayType::Char => parse_char(text)? as u64,
            DisplayType::Word | DisplayType::Uint => int(0, u16::MAX as i128)?,
            DisplayType::Int => int(i16::MIN as i128, i16::MAX as i128)? & 0xFFFF,
            DisplayType::Dword | DisplayType::Udint => int(0, u32::MAX as i128)?,
            DisplayType::Dint => int(i32::MIN as i128, i32::MAX as i128)? & 0xFFFF_FFFF,
            DisplayType::Lword | DisplayType::Ulint => int(0, u64::MAX as i128)?,
            DisplayType::Lint => int(i64::MIN as i128, i64::MAX as i128)?,
            DisplayType::Real => {
                let f: f32 = text.parse().map_err(|_| format!("{text} is not a number"))?;
                f.to_bits() as u64
            }
            DisplayType::Lreal => {
                let f: f64 = text.parse().map_err(|_| format!("{text} is not a number"))?;
                f.to_bits()
            }
            DisplayType::Time => {
                let ms = parse_time(text)?;
                if ms < i32::MIN as i64 || ms > i32::MAX as i64 {
                    return Err("TIME holds about ±24 days".into());
                }
                ms as i32 as u32 as u64
            }
            DisplayType::String => {
                return Ok(vec![S7Value::String(unquote(text).to_string())]);
            }
            DisplayType::Timer | DisplayType::Counter => {
                return Err("timers and counters are read-only".into());
            }
        };
        Ok(match d.width() {
            1 if d == DisplayType::Bool => vec![S7Value::Bool(bits != 0)],
            1 => vec![S7Value::Byte(bits as u8)],
            2 => vec![S7Value::Word(bits as u16)],
            4 => vec![S7Value::DWord(bits as u32)],
            _ => vec![S7Value::DWord((bits >> 32) as u32), S7Value::DWord(bits as u32)],
        })
    }
}

fn type_name(d: DisplayType) -> String {
    format!("{d:?}").to_ascii_uppercase()
}

/// REAL in its own precision: `9.128201`, not the `9.12820053100586` of its f64 widening.
fn format_real(f: f32) -> String {
    if f.is_finite() && f == f.trunc() && f.abs() < 1e7 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn format_float(f: f64) -> String {
    if f.is_finite() && f == f.trunc() && f.abs() < 1e15 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn format_char(b: u8) -> String {
    if b.is_ascii_graphic() || b == b' ' {
        format!("'{}'", b as char)
    } else {
        format!("'${b:02X}'")
    }
}

/// `T#1d2h3m4s5ms` style, `T#0ms` for zero, a leading `-` for negative durations.
fn format_time(prefix: &str, ms: i64) -> String {
    if ms == 0 {
        return format!("{prefix}0ms");
    }
    let sign = if ms < 0 { "-" } else { "" };
    let mut rest = ms.unsigned_abs();
    let mut out = format!("{prefix}{sign}");
    for (unit, size) in [("d", 86_400_000u64), ("h", 3_600_000), ("m", 60_000), ("s", 1000), ("ms", 1)] {
        let n = rest / size;
        rest %= size;
        if n > 0 {
            out.push_str(&format!("{n}{unit}"));
        }
    }
    out
}

/// A plain number of milliseconds, or `T#`/`TIME#` with d/h/m/s/ms parts (`T#1m30s`, `T#-2s`).
fn parse_time(text: &str) -> Result<i64, String> {
    let upper = text.to_ascii_uppercase();
    let Some(body) = upper.strip_prefix("TIME#").or_else(|| upper.strip_prefix("T#")) else {
        return text.parse::<i64>().map_err(|_| format!("{text} is not a duration, use T#1s500ms or milliseconds"));
    };
    let (sign, mut body) = match body.strip_prefix('-') {
        Some(b) => (-1, b),
        None => (1, body),
    };
    let bad = || format!("{text} is not a duration, use T#1s500ms");
    if body.is_empty() {
        return Err(bad());
    }
    let mut total: i64 = 0;
    while !body.is_empty() {
        let digits = body.find(|c: char| !c.is_ascii_digit() && c != '_').ok_or_else(bad)?;
        if digits == 0 {
            return Err(bad());
        }
        let n: i64 = body[..digits].replace('_', "").parse().map_err(|_| bad())?;
        body = &body[digits..];
        let (size, len) = if body.starts_with("MS") {
            (1, 2)
        } else if body.starts_with('D') {
            (86_400_000, 1)
        } else if body.starts_with('H') {
            (3_600_000, 1)
        } else if body.starts_with('M') {
            (60_000, 1)
        } else if body.starts_with('S') {
            (1000, 1)
        } else {
            return Err(bad());
        };
        total = n.checked_mul(size).and_then(|v| total.checked_add(v)).ok_or_else(bad)?;
        body = &body[len..];
    }
    Ok(sign * total)
}

/// Decimal, `16#FF`, `2#1010` or `8#17`, with optional `_` separators and sign.
fn parse_int(text: &str) -> Result<i128, String> {
    let cleaned = text.replace('_', "");
    let (neg, body) = match cleaned.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, cleaned.trim_start_matches('+').to_string()),
    };
    let upper = body.to_ascii_uppercase();
    let parsed = if let Some(h) = upper.strip_prefix("16#").or_else(|| upper.strip_prefix("0X")) {
        i128::from_str_radix(h, 16)
    } else if let Some(b) = upper.strip_prefix("2#") {
        i128::from_str_radix(b, 2)
    } else if let Some(o) = upper.strip_prefix("8#") {
        i128::from_str_radix(o, 8)
    } else {
        upper.parse()
    };
    let v = parsed.map_err(|_| format!("{text} is not a whole number"))?;
    Ok(if neg { -v } else { v })
}

fn unquote(text: &str) -> &str {
    text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')).unwrap_or(text)
}

fn parse_char(text: &str) -> Result<u8, String> {
    let inner = unquote(text);
    match inner.as_bytes() {
        [b] if b.is_ascii() => Ok(*b),
        [b'$', h1, h2] => u8::from_str_radix(&format!("{}{}", *h1 as char, *h2 as char), 16)
            .map_err(|_| format!("{text} is not a character")),
        _ => Err("CHAR takes one ASCII character, e.g. 'A'".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(addr: &str, t: &str) -> WatchItem {
        WatchItem::parse(addr, Some(t)).unwrap()
    }

    #[test]
    fn default_types_follow_the_address_width() {
        assert_eq!(WatchItem::parse("M0.0", None).unwrap().display, DisplayType::Bool);
        assert_eq!(WatchItem::parse("%MB4", None).unwrap().display, DisplayType::Byte);
        assert_eq!(WatchItem::parse("DB1.DBW2", None).unwrap().display, DisplayType::Int);
        assert_eq!(WatchItem::parse("MD10", None).unwrap().display, DisplayType::Dint);
        assert_eq!(WatchItem::parse("T5", None).unwrap().display, DisplayType::Timer);
    }

    #[test]
    fn the_type_decides_the_read_width() {
        let real = item("MW10", "REAL");
        assert_eq!(real.tags.len(), 1);
        assert_eq!(real.tags[0].data_type, S7DataType::DWord);
        let lreal = item("DB3.DBD8", "LREAL");
        assert_eq!(lreal.tags.iter().map(|t| t.offset).collect::<Vec<_>>(), vec![8, 12]);
        let s = item("DB1.DBB20", "STRING");
        assert_eq!((s.tags[0].data_type, s.tags[0].length), (S7DataType::String, 0));
    }

    #[test]
    fn mismatched_addresses_are_refused() {
        assert!(WatchItem::parse("MW0", Some("BOOL")).is_err());
        assert!(WatchItem::parse("M0.1", Some("INT")).is_err());
        assert!(WatchItem::parse("T1", Some("INT")).is_err());
        assert!(WatchItem::parse("MW0", Some("COUNTER")).is_err());
        assert!(WatchItem::parse("MW0", Some("FLOAT128")).is_err());
    }

    #[test]
    fn values_show_in_iec_notation() {
        assert_eq!(item("MW0", "INT").format(&[S7Value::Word(0xFFFE)]).unwrap(), "-2");
        assert_eq!(item("MW0", "WORD").format(&[S7Value::Word(0xFF)]).unwrap(), "16#00FF");
        assert_eq!(item("MD0", "REAL").format(&[S7Value::DWord(1.5f32.to_bits())]).unwrap(), "1.5");
        assert_eq!(item("MD0", "REAL").format(&[S7Value::DWord(2.0f32.to_bits())]).unwrap(), "2.0");
        assert_eq!(item("MD0", "REAL").format(&[S7Value::DWord(9.128201f32.to_bits())]).unwrap(), "9.128201");
        assert_eq!(item("MD0", "TIME").format(&[S7Value::DWord(90_500)]).unwrap(), "T#1m30s500ms");
        assert_eq!(item("MD0", "TIME").format(&[S7Value::DWord((-2000i32) as u32)]).unwrap(), "T#-2s");
        assert_eq!(item("MB0", "CHAR").format(&[S7Value::Byte(b'A')]).unwrap(), "'A'");
        assert_eq!(item("M0.0", "BOOL").format(&[S7Value::Bool(true)]).unwrap(), "TRUE");
        let bits = 3.25f64.to_bits();
        let lreal = item("MD0", "LREAL").format(&[S7Value::DWord((bits >> 32) as u32), S7Value::DWord(bits as u32)]);
        assert_eq!(lreal.unwrap(), "3.25");
        assert_eq!(item("MD0", "LINT").format(&[S7Value::DWord(u32::MAX), S7Value::DWord(u32::MAX)]).unwrap(), "-1");
        assert_eq!(item("T5", "S5TIME").format(&[S7Value::Timer(2500)]).unwrap(), "S5T#2s500ms");
        assert_eq!(item("DB1.DBB0", "STRING").format(&[S7Value::String("it's".into())]).unwrap(), "'it$'s'");
    }

    #[test]
    fn input_round_trips() {
        let cases = [
            ("MW0", "INT", "-2"),
            ("MW0", "WORD", "16#00FF"),
            ("MD0", "REAL", "1.5"),
            ("MD0", "TIME", "T#1m30s500ms"),
            ("MD0", "DINT", "-100000"),
            ("MD0", "LREAL", "-0.125"),
            ("MD0", "LINT", "-9000000000"),
            ("MB0", "SINT", "-128"),
            ("MB0", "CHAR", "'z'"),
            ("M0.3", "BOOL", "TRUE"),
        ];
        for (addr, t, text) in cases {
            let it = item(addr, t);
            let values = it.parse_input(text).unwrap();
            assert_eq!(it.format(&values).unwrap(), text, "{addr} {t}");
        }
    }

    #[test]
    fn bad_input_is_explained() {
        assert!(item("MW0", "INT").parse_input("40000").unwrap_err().contains("out of range"));
        assert!(item("MB0", "BYTE").parse_input("-1").is_err());
        assert!(item("M0.0", "BOOL").parse_input("maybe").is_err());
        assert!(item("MD0", "TIME").parse_input("T#5x").is_err());
        assert!(item("T1", "S5TIME").parse_input("T#1s").is_err());
        assert_eq!(item("MW0", "INT").parse_input("2#1010").unwrap(), vec![S7Value::Word(10)]);
        assert_eq!(item("MD0", "TIME").parse_input("1500").unwrap(), vec![S7Value::DWord(1500)]);
    }
}
