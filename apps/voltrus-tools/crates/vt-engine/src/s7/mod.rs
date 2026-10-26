//! Siemens S7 sessions over `mt-s7`: a client that polls a CPU's tags,
//! writes them and reads its diagnostics, and a simulator that plays a CPU
//! from a template on `mt-s7`'s server.

pub(crate) mod client;
pub(crate) mod server;
pub mod template;

use mt_s7::{S7DataType, S7Frame, S7Tag, S7Value};
use vt_protocol::{
    Direction, PointInfo, Quality, Reading, S7Address, S7Area, S7TagSpec, S7Type, TrafficFrame,
};

/// The `mt-s7` tag that reads `address` as `data_type`.
pub(crate) fn tag(name: &str, address: S7Address, data_type: S7Type) -> S7Tag {
    S7Tag {
        area: match address.area {
            S7Area::Db => mt_s7::S7Area::DB,
            S7Area::Input => mt_s7::S7Area::Input,
            S7Area::Output => mt_s7::S7Area::Output,
            S7Area::Marker => mt_s7::S7Area::Marker,
            S7Area::Timer => mt_s7::S7Area::Timer,
            S7Area::Counter => mt_s7::S7Area::Counter,
        },
        db_number: address.db,
        offset: address.offset,
        bit_offset: address.bit,
        data_type: match data_type {
            S7Type::Bool => S7DataType::Bool,
            S7Type::Byte => S7DataType::Byte,
            S7Type::Word | S7Type::Int => S7DataType::Word,
            S7Type::DWord | S7Type::DInt => S7DataType::DWord,
            S7Type::Real => S7DataType::Real,
            S7Type::String => S7DataType::String,
            S7Type::Timer => S7DataType::Timer,
            S7Type::Counter => S7DataType::Counter,
        },
        length: 1,
        name: (!name.is_empty()).then(|| name.to_owned()),
    }
}

pub(crate) fn point_info(id: usize, tag: &S7TagSpec, writable: bool) -> PointInfo {
    PointInfo {
        id: u32::try_from(id).unwrap_or(u32::MAX),
        name: tag.name.clone(),
        address: tag.address.to_string(),
        data_type: tag.data_type.name().into(),
        unit: tag.unit.clone(),
        writable,
        boolean: tag.data_type == S7Type::Bool,
        states: Vec::new(),
    }
}

/// A value read as `data_type`: the number, the bytes on the wire as words
/// for the inspector, and the text of a string. A value that does not match
/// the type is bad.
pub(crate) fn reading(value: &S7Value, data_type: S7Type) -> (Reading, Option<String>) {
    let good = |value: f64, raw: &[u16]| {
        let mut words = [0u16; 4];
        for (slot, word) in words.iter_mut().zip(raw) {
            *slot = *word;
        }
        Reading {
            value,
            raw: words,
            raw_len: u8::try_from(raw.len().min(4)).unwrap_or(4),
            quality: Quality::Good,
        }
    };
    let split = |bits: u32| [(bits >> 16) as u16, bits as u16];
    let read = match (value, data_type) {
        (S7Value::Bool(on), S7Type::Bool) => good(f64::from(u8::from(*on)), &[]),
        (S7Value::Byte(byte), S7Type::Byte) => good(f64::from(*byte), &[u16::from(*byte)]),
        (S7Value::Word(word), S7Type::Word) => good(f64::from(*word), &[*word]),
        (S7Value::Word(word), S7Type::Int) => good(f64::from(*word as i16), &[*word]),
        (S7Value::DWord(dword), S7Type::DWord) => good(f64::from(*dword), &split(*dword)),
        (S7Value::DWord(dword), S7Type::DInt) => good(f64::from(*dword as i32), &split(*dword)),
        (S7Value::Real(real), S7Type::Real) => good(f64::from(*real), &split(real.to_bits())),
        (S7Value::Timer(ms), S7Type::Timer) => good(f64::from(*ms), &[]),
        (S7Value::Counter(count), S7Type::Counter) => good(f64::from(*count), &[]),
        (S7Value::String(text), S7Type::String) => {
            return (good(f64::NAN, &[]), Some(text.clone()));
        }
        _ => Reading::bad(),
    };
    (read, None)
}

/// The `mt-s7` value that writes `value` as `data_type`, after checking it
/// fits. Timers and counters only from the simulator (`simulated`).
pub(crate) fn value(value: f64, data_type: S7Type, simulated: bool) -> Result<S7Value, String> {
    match data_type {
        S7Type::Timer if simulated && value.is_finite() && (0.0..=9_990_000.0).contains(&value) => {
            return Ok(S7Value::Timer(value.round() as u32));
        }
        S7Type::Counter if simulated && value.is_finite() && (0.0..=999.0).contains(&value) => {
            return Ok(S7Value::Counter(value.round() as u16));
        }
        S7Type::Timer if simulated => return Err("A timer runs from 0 to 9990000 ms".into()),
        S7Type::Counter if simulated => return Err("A counter counts from 0 to 999".into()),
        _ => data_type.check(value)?,
    }
    Ok(match data_type {
        S7Type::Bool => S7Value::Bool(value != 0.0),
        S7Type::Byte => S7Value::Byte(value as u8),
        S7Type::Word => S7Value::Word(value as u16),
        S7Type::Int => S7Value::Word(value as i16 as u16),
        S7Type::DWord => S7Value::DWord(value as u32),
        S7Type::DInt => S7Value::DWord(value as i32 as u32),
        S7Type::Real => S7Value::Real(value as f32),
        S7Type::String | S7Type::Timer | S7Type::Counter => {
            return Err(format!("A {} is read-only here", data_type.name()));
        }
    })
}

pub(crate) fn traffic_frame(frame: S7Frame) -> TrafficFrame {
    TrafficFrame {
        at_ms: frame.at_ms,
        direction: if frame.sent {
            Direction::Tx
        } else {
            Direction::Rx
        },
        summary: frame.summary,
        detail: frame.detail,
        bytes: u32::try_from(frame.bytes).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_types_read_the_same_bits_as_signed() {
        let (int, _) = reading(&S7Value::Word(0xFFFE), S7Type::Int);
        assert_eq!(int.value, -2.0);
        assert_eq!(int.raw_words(), [0xFFFE]);
        let (word, _) = reading(&S7Value::Word(0xFFFE), S7Type::Word);
        assert_eq!(word.value, 65_534.0);
        let (dint, _) = reading(&S7Value::DWord(0x8000_0000), S7Type::DInt);
        assert_eq!(dint.value, -2_147_483_648.0);
        assert_eq!(dint.raw_words(), [0x8000, 0x0000]);
        let (real, _) = reading(&S7Value::Real(1.5), S7Type::Real);
        assert_eq!(real.raw_words(), [0x3FC0, 0x0000]);
    }

    #[test]
    fn strings_have_text_and_mismatches_are_bad() {
        let (text, payload) = reading(&S7Value::String("LOT-7".into()), S7Type::String);
        assert_eq!(text.quality, Quality::Good);
        assert!(text.value.is_nan());
        assert_eq!(payload.as_deref(), Some("LOT-7"));
        let (bad, _) = reading(&S7Value::Real(1.0), S7Type::Word);
        assert_eq!(bad.quality, Quality::Bad);
    }

    #[test]
    fn writes_encode_signed_values_and_refuse_what_does_not_fit() {
        assert_eq!(value(-2.0, S7Type::Int, false), Ok(S7Value::Word(0xFFFE)));
        assert_eq!(
            value(-1.0, S7Type::DInt, false),
            Ok(S7Value::DWord(u32::MAX))
        );
        assert_eq!(value(1.0, S7Type::Bool, false), Ok(S7Value::Bool(true)));
        assert!(value(70_000.0, S7Type::Word, false).is_err());
        assert!(value(1.5, S7Type::Int, false).is_err());
        assert!(
            value(100.0, S7Type::Timer, false).is_err(),
            "the CPU runs its timers"
        );
        assert_eq!(value(100.0, S7Type::Timer, true), Ok(S7Value::Timer(100)));
        assert_eq!(value(7.0, S7Type::Counter, true), Ok(S7Value::Counter(7)));
        assert!(value(1000.0, S7Type::Counter, true).is_err());
        assert!(value(1.0, S7Type::String, true).is_err());
    }

    #[test]
    fn tags_carry_the_address_and_the_width_of_the_type() {
        let address: S7Address = "DB3.DBX4.5".parse().expect("a bit address");
        let bit = tag("Run", address, S7Type::Bool);
        assert_eq!((bit.db_number, bit.offset, bit.bit_offset), (3, 4, 5));
        assert_eq!(bit.data_type, S7DataType::Bool);
        let counter: S7Address = "C7".parse().expect("a counter");
        let counter = tag("", counter, S7Type::Counter);
        assert_eq!((counter.area, counter.offset), (mt_s7::S7Area::Counter, 7));
        assert!(counter.name.is_none());
        let int: S7Address = "MW2".parse().expect("a word");
        assert_eq!(tag("x", int, S7Type::Int).data_type, S7DataType::Word);
    }
}
