//! Characteristic bytes to values and back: the standard layouts of
//! Bluetooth SIG characteristics (heart rate, battery, temperature…) and
//! the plain numeric, text and byte formats a user can pick.

use vt_protocol::{BleDecoder, BleFormat};

use super::gatt;

/// A decoded characteristic value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
}

/// How a SIG characteristic decodes automatically.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Standard {
    /// One number at byte 0.
    Scalar {
        format: BleFormat,
        scale: f64,
        unit: &'static str,
    },
    Text,
    /// Heart Rate Measurement: flags, then the rate as uint8 or uint16.
    HeartRate,
    /// Temperature Measurement: flags, then an IEEE 11073 FLOAT in °C or °F.
    Temperature,
    /// Elevation: a signed 24-bit number of centimetres.
    Elevation,
}

fn standard(uuid: &str) -> Option<Standard> {
    let scalar = |format, scale, unit| Standard::Scalar {
        format,
        scale,
        unit,
    };
    Some(match gatt::short_of(uuid)? {
        0x2A00 | 0x2A24..=0x2A29 => Standard::Text,
        0x2A01 => scalar(BleFormat::U16Le, 1.0, ""),
        0x2A06 | 0x2A1D | 0x2A38 | 0x2A5D | 0x2A76 => scalar(BleFormat::U8, 1.0, ""),
        0x2A07 => scalar(BleFormat::I8, 1.0, "dBm"),
        0x2A19 => scalar(BleFormat::U8, 1.0, "%"),
        0x2A1C | 0x2A1E => Standard::Temperature,
        0x2A37 => Standard::HeartRate,
        0x2A6C => Standard::Elevation,
        0x2A6D => scalar(BleFormat::U32Le, 0.1, "Pa"),
        0x2A6E => scalar(BleFormat::I16Le, 0.01, "°C"),
        0x2A6F => scalar(BleFormat::U16Le, 0.01, "%"),
        0x2A98 => scalar(BleFormat::U16Le, 0.005, "kg"),
        _ => return None,
    })
}

/// The type a point shows for `uuid` decoded with `decoder`.
pub fn label(uuid: &str, decoder: BleDecoder) -> String {
    if decoder.format != BleFormat::Auto {
        let mut label = decoder.format.label().to_owned();
        if decoder.format.size().is_some() {
            if decoder.offset != 0 {
                label.push_str(&format!(" @{}", decoder.offset));
            }
            if decoder.scale != 1.0 {
                label.push_str(&format!(" ×{}", decoder.scale));
            }
        }
        return label;
    }
    match standard(uuid) {
        Some(Standard::Scalar { format, scale, .. }) if scale != 1.0 => {
            format!("{} ×{scale}", format.label())
        }
        Some(Standard::Scalar { format, .. }) => format.label().to_owned(),
        Some(Standard::Text) => BleFormat::Utf8.label().to_owned(),
        Some(Standard::HeartRate) => "Heart rate".to_owned(),
        Some(Standard::Temperature) => "IEEE 11073".to_owned(),
        Some(Standard::Elevation) => "int24 ×0.01".to_owned(),
        None => BleFormat::Auto.label().to_owned(),
    }
}

/// The unit of `uuid` decoded with `decoder`: only standard layouts have one.
pub fn unit(uuid: &str, decoder: BleDecoder) -> &'static str {
    if decoder.format != BleFormat::Auto {
        return "";
    }
    match standard(uuid) {
        Some(Standard::Scalar { unit, .. }) => unit,
        Some(Standard::HeartRate) => "bpm",
        Some(Standard::Temperature) => "°C",
        Some(Standard::Elevation) => "m",
        Some(Standard::Text) | None => "",
    }
}

/// Decode a characteristic's bytes.
pub fn decode(uuid: &str, decoder: BleDecoder, bytes: &[u8]) -> Result<Value, String> {
    match decoder.format {
        BleFormat::Auto => match standard(uuid) {
            Some(Standard::Scalar { format, scale, .. }) => number(bytes, format, 0, scale),
            Some(Standard::Text) => Ok(Value::Text(text(bytes))),
            Some(Standard::HeartRate) => heart_rate(bytes),
            Some(Standard::Temperature) => temperature(bytes),
            Some(Standard::Elevation) => elevation(bytes),
            None if printable(bytes) => Ok(Value::Text(text(bytes))),
            None => Ok(Value::Text(hex_or_empty(bytes))),
        },
        BleFormat::Utf8 => Ok(Value::Text(text(bytes))),
        BleFormat::Hex => Ok(Value::Text(hex_or_empty(bytes))),
        format => number(bytes, format, usize::from(decoder.offset), decoder.scale),
    }
}

/// The bytes that write `value` to `uuid` decoded with `decoder`: the
/// number in the decoder's format, divided by its scale.
pub fn encode(uuid: &str, decoder: BleDecoder, value: f64) -> Result<Vec<u8>, String> {
    let (format, scale, offset) = match (decoder.format, standard(uuid)) {
        (BleFormat::Auto, Some(Standard::Scalar { format, scale, .. })) => (format, scale, 0),
        (format, _) if format.size().is_some() => (format, decoder.scale, decoder.offset),
        _ => {
            return Err(
                "This value is not a single number: write bytes or text to it instead".into(),
            );
        }
    };
    if offset != 0 {
        return Err(format!(
            "The number sits at byte {offset}: write the whole value as bytes instead"
        ));
    }
    if !value.is_finite() || scale == 0.0 {
        return Err(format!("{value} cannot be written"));
    }
    let raw = value / scale;
    let integer = |min: f64, max: f64| -> Result<i64, String> {
        let rounded = raw.round();
        if (min..=max).contains(&rounded) {
            Ok(rounded as i64)
        } else {
            Err(format!("{value} is out of range for {}", format.label()))
        }
    };
    Ok(match format {
        BleFormat::U8 => vec![integer(0.0, 255.0)? as u8],
        BleFormat::I8 => (integer(-128.0, 127.0)? as i8).to_le_bytes().to_vec(),
        BleFormat::U16Le => (integer(0.0, 65_535.0)? as u16).to_le_bytes().to_vec(),
        BleFormat::U16Be => (integer(0.0, 65_535.0)? as u16).to_be_bytes().to_vec(),
        BleFormat::I16Le => (integer(-32_768.0, 32_767.0)? as i16)
            .to_le_bytes()
            .to_vec(),
        BleFormat::I16Be => (integer(-32_768.0, 32_767.0)? as i16)
            .to_be_bytes()
            .to_vec(),
        BleFormat::U32Le => (integer(0.0, 4_294_967_295.0)? as u32)
            .to_le_bytes()
            .to_vec(),
        BleFormat::U32Be => (integer(0.0, 4_294_967_295.0)? as u32)
            .to_be_bytes()
            .to_vec(),
        BleFormat::I32Le => (integer(-2_147_483_648.0, 2_147_483_647.0)? as i32)
            .to_le_bytes()
            .to_vec(),
        BleFormat::I32Be => (integer(-2_147_483_648.0, 2_147_483_647.0)? as i32)
            .to_be_bytes()
            .to_vec(),
        BleFormat::F32Le | BleFormat::F32Be => {
            let single = raw as f32;
            if !single.is_finite() {
                return Err(format!("{value} is out of range for float32"));
            }
            if format == BleFormat::F32Le {
                single.to_le_bytes().to_vec()
            } else {
                single.to_be_bytes().to_vec()
            }
        }
        BleFormat::F64Le => raw.to_le_bytes().to_vec(),
        BleFormat::F64Be => raw.to_be_bytes().to_vec(),
        BleFormat::Auto | BleFormat::Utf8 | BleFormat::Hex => {
            return Err("This value is not a single number".into());
        }
    })
}

/// Bytes as two-digit uppercase hex, space separated.
pub fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(&format!("{byte:02X}"));
    }
    text
}

fn hex_or_empty(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "(no bytes)".into()
    } else {
        hex(bytes)
    }
}

/// Bytes typed as hex: pairs of digits, optionally `0x`-prefixed and
/// separated by spaces, colons, commas or dashes.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let mut digits = String::with_capacity(text.len());
    for token in text.split(|c: char| c.is_whitespace() || matches!(c, ':' | ',' | '-')) {
        let token = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        if !token.len().is_multiple_of(2) {
            return Err(format!(
                "“{token}” is not whole bytes: use two hex digits per byte, e.g. 0A FF"
            ));
        }
        digits.push_str(token);
    }
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Bytes are hex digits 0-9 and A-F, e.g. 01 FF 3A".into());
    }
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|error| error.to_string()))
        .collect()
}

/// UTF-8 text, without the NUL padding many peripherals send.
fn text(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Valid UTF-8 without control characters (after NUL padding).
fn printable(bytes: &[u8]) -> bool {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    std::str::from_utf8(&bytes[..end])
        .is_ok_and(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

fn number(bytes: &[u8], format: BleFormat, offset: usize, scale: f64) -> Result<Value, String> {
    let Some(size) = format.size() else {
        return Err(format!("{} is not a number format", format.label()));
    };
    let Some(field) = bytes.get(offset..offset + size) else {
        return Err(format!(
            "{} at byte {offset} needs {} bytes, got {}",
            format.label(),
            offset + size,
            bytes.len()
        ));
    };
    let array = |n: usize| -> [u8; 8] {
        let mut out = [0; 8];
        out[..n].copy_from_slice(&field[..n]);
        out
    };
    let [a, b, c, d, e, f, g, h] = array(size);
    let raw = match format {
        BleFormat::U8 => f64::from(a),
        BleFormat::I8 => f64::from(i8::from_le_bytes([a])),
        BleFormat::U16Le => f64::from(u16::from_le_bytes([a, b])),
        BleFormat::U16Be => f64::from(u16::from_be_bytes([a, b])),
        BleFormat::I16Le => f64::from(i16::from_le_bytes([a, b])),
        BleFormat::I16Be => f64::from(i16::from_be_bytes([a, b])),
        BleFormat::U32Le => f64::from(u32::from_le_bytes([a, b, c, d])),
        BleFormat::U32Be => f64::from(u32::from_be_bytes([a, b, c, d])),
        BleFormat::I32Le => f64::from(i32::from_le_bytes([a, b, c, d])),
        BleFormat::I32Be => f64::from(i32::from_be_bytes([a, b, c, d])),
        BleFormat::F32Le => f64::from(f32::from_le_bytes([a, b, c, d])),
        BleFormat::F32Be => f64::from(f32::from_be_bytes([a, b, c, d])),
        BleFormat::F64Le => f64::from_le_bytes([a, b, c, d, e, f, g, h]),
        BleFormat::F64Be => f64::from_be_bytes([a, b, c, d, e, f, g, h]),
        BleFormat::Auto | BleFormat::Utf8 | BleFormat::Hex => {
            return Err(format!("{} is not a number format", format.label()));
        }
    };
    Ok(Value::Number(raw * scale))
}

fn heart_rate(bytes: &[u8]) -> Result<Value, String> {
    let Some(&flags) = bytes.first() else {
        return Err("Heart rate measurement is empty".into());
    };
    // Flags bit 0: the rate is a uint16 instead of a uint8.
    let rate = if flags & 0x01 != 0 {
        match bytes.get(1..3) {
            Some(&[low, high]) => u16::from_le_bytes([low, high]),
            _ => return Err("Heart rate (uint16) needs 3 bytes".into()),
        }
    } else {
        match bytes.get(1) {
            Some(&rate) => u16::from(rate),
            None => return Err("Heart rate (uint8) needs 2 bytes".into()),
        }
    };
    Ok(Value::Number(f64::from(rate)))
}

fn temperature(bytes: &[u8]) -> Result<Value, String> {
    let (Some(&flags), Some(&[a, b, c, d])) = (bytes.first(), bytes.get(1..5)) else {
        return Err(format!(
            "Temperature measurement needs 5 bytes, got {}",
            bytes.len()
        ));
    };
    let Some(reading) = ieee11073_float(u32::from_le_bytes([a, b, c, d])) else {
        return Err("The thermometer reports no valid reading".into());
    };
    if !reading.is_finite() {
        return Err("The temperature is out of the thermometer's range".into());
    }
    // Flags bit 0: Fahrenheit. Points keep one unit, Celsius.
    Ok(Value::Number(if flags & 0x01 != 0 {
        (reading - 32.0) * 5.0 / 9.0
    } else {
        reading
    }))
}

fn elevation(bytes: &[u8]) -> Result<Value, String> {
    let Some(&[a, b, c]) = bytes.get(..3) else {
        return Err(format!("Elevation needs 3 bytes, got {}", bytes.len()));
    };
    let sign = if c & 0x80 != 0 { 0xFF } else { 0x00 };
    Ok(Value::Number(
        f64::from(i32::from_le_bytes([a, b, c, sign])) * 0.01,
    ))
}

/// An IEEE 11073-20601 32-bit FLOAT: a signed 24-bit mantissa and a signed
/// 8-bit base-10 exponent. `None` for NaN, NRes and the reserved value;
/// infinity for ±INFINITY.
pub fn ieee11073_float(raw: u32) -> Option<f64> {
    let mantissa_bits = raw & 0x00FF_FFFF;
    match mantissa_bits {
        // NaN, NRes and the reserved value.
        0x007F_FFFF..=0x0080_0001 => return None,
        0x007F_FFFE => return Some(f64::INFINITY),
        0x0080_0002 => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    // Sign-extend the 24-bit mantissa.
    let mantissa = if mantissa_bits & 0x0080_0000 != 0 {
        (mantissa_bits | 0xFF00_0000) as i32
    } else {
        mantissa_bits as i32
    };
    let exponent = (raw >> 24) as u8 as i8;
    Some(f64::from(mantissa) * 10f64.powi(i32::from(exponent)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(short: u16, bytes: &[u8]) -> Result<Value, String> {
        decode(&gatt::uuid16(short), BleDecoder::default(), bytes)
    }

    fn close(value: Result<Value, String>, expected: f64) {
        match value {
            Ok(Value::Number(number)) => assert!(
                (number - expected).abs() < 1e-9,
                "{number} is not {expected}"
            ),
            other => panic!("expected {expected}, got {other:?}"),
        }
    }

    #[test]
    fn heart_rate_reads_the_rate_format_from_its_flags() {
        // uint8 rate, sensor contact detected, an RR interval after it.
        close(auto(0x2A37, &[0x16, 72, 0x40, 0x03]), 72.0);
        // uint16 rate: 0x012C = 300.
        close(auto(0x2A37, &[0x01, 0x2C, 0x01]), 300.0);
        assert!(auto(0x2A37, &[0x01, 0x2C]).is_err(), "uint16 cut short");
        assert!(auto(0x2A37, &[0x00]).is_err(), "no rate at all");
        assert!(auto(0x2A37, &[]).is_err());
        assert_eq!(unit(&gatt::uuid16(0x2A37), BleDecoder::default()), "bpm");
    }

    #[test]
    fn ieee_11073_floats_and_their_special_values() {
        // 3650 × 10^-2 = 36.5
        let body = ieee11073_float(0xFE00_0E42).unwrap_or(f64::NAN);
        assert!((body - 36.5).abs() < 1e-9, "{body}");
        // Negative mantissa: -1 × 10^0.
        assert_eq!(ieee11073_float(0x00FF_FFFF), Some(-1.0));
        assert_eq!(ieee11073_float(0x0000_0000), Some(0.0));
        assert_eq!(ieee11073_float(0x007F_FFFF), None, "NaN");
        assert_eq!(ieee11073_float(0x0080_0000), None, "NRes");
        assert_eq!(ieee11073_float(0x0080_0001), None, "reserved");
        assert_eq!(ieee11073_float(0x007F_FFFE), Some(f64::INFINITY));
        assert_eq!(ieee11073_float(0x0080_0002), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn temperature_measurements_end_up_in_celsius() {
        let celsius = [0x00, 0x42, 0x0E, 0x00, 0xFE];
        close(auto(0x2A1C, &celsius), 36.5);
        // 986 × 10^-1 °F = 98.6 °F = 37 °C.
        let fahrenheit = [0x01, 0xDA, 0x03, 0x00, 0xFF];
        close(auto(0x2A1C, &fahrenheit), 37.0);
        let nan = [0x00, 0xFF, 0xFF, 0x7F, 0x00];
        assert!(auto(0x2A1C, &nan).is_err());
        assert!(auto(0x2A1C, &celsius[..4]).is_err());
    }

    #[test]
    fn standard_scalars_carry_scale_and_unit() {
        close(auto(0x2A19, &[87]), 87.0);
        // 2150 hundredths of a degree, and a negative one.
        close(auto(0x2A6E, &2150i16.to_le_bytes()), 21.5);
        close(auto(0x2A6E, &(-512i16).to_le_bytes()), -5.12);
        close(auto(0x2A6F, &4520u16.to_le_bytes()), 45.2);
        close(auto(0x2A6D, &1_013_250u32.to_le_bytes()), 101_325.0);
        close(auto(0x2A07, &[0xF4]), -12.0);
        // Elevation is a signed 24-bit number.
        close(auto(0x2A6C, &[0x18, 0xFC, 0xFF]), -10.0);
        close(auto(0x2A6C, &[0x10, 0x27, 0x00]), 100.0);
        let temperature = gatt::uuid16(0x2A6E);
        assert_eq!(unit(&temperature, BleDecoder::default()), "°C");
        assert_eq!(label(&temperature, BleDecoder::default()), "int16 LE ×0.01");
        // A chosen decoder overrides the standard one, and its unit.
        let raw = BleDecoder::of(BleFormat::U16Be);
        close(decode(&temperature, raw, &[0x08, 0x66]), 2150.0);
        assert_eq!(unit(&temperature, raw), "");
    }

    #[test]
    fn unknown_characteristics_are_text_when_printable_else_bytes() {
        let vendor = "7d2e0002-6c1a-4b7e-9e4f-3a5c8d1b0e21";
        let auto = BleDecoder::default();
        assert_eq!(
            decode(vendor, auto, b"Voltrus\0\0"),
            Ok(Value::Text("Voltrus".into()))
        );
        assert_eq!(
            decode(vendor, auto, &[0x00, 0x7F, 0x10]),
            Ok(Value::Text("00 7F 10".into()))
        );
        assert_eq!(
            decode(vendor, auto, &[]),
            Ok(Value::Text("(no bytes)".into()))
        );
        assert_eq!(label(vendor, auto), "Auto");
        // Invalid UTF-8 is not text.
        assert_eq!(
            decode(vendor, auto, &[0xC3, 0x28]),
            Ok(Value::Text("C3 28".into()))
        );
        assert_eq!(
            decode(vendor, BleDecoder::of(BleFormat::Utf8), &[0xC3, 0xA9, 0x00]),
            Ok(Value::Text("é".into()))
        );
    }

    #[test]
    fn numeric_formats_read_at_an_offset_with_a_scale() {
        let uuid = "7d2e0002-6c1a-4b7e-9e4f-3a5c8d1b0e21";
        let bytes = [0xAA, 0x00, 0x00, 0xB0, 0x41, 0x01];
        let decoder = BleDecoder {
            format: BleFormat::F32Le,
            offset: 1,
            scale: 2.0,
        };
        // 22.0 as float32 LE is 00 00 B0 41.
        close(decode(uuid, decoder, &bytes), 44.0);
        assert_eq!(label(uuid, decoder), "float32 LE @1 ×2");
        let too_far = BleDecoder {
            offset: 3,
            ..decoder
        };
        assert!(decode(uuid, too_far, &bytes).is_err());
        close(decode(uuid, BleDecoder::of(BleFormat::I8), &[0xFF]), -1.0);
        close(
            decode(
                uuid,
                BleDecoder::of(BleFormat::I32Be),
                &[0xFF, 0xFF, 0xFF, 0xFE],
            ),
            -2.0,
        );
        close(
            decode(uuid, BleDecoder::of(BleFormat::U32Le), &[0xFF; 4]),
            4_294_967_295.0,
        );
        close(
            decode(
                uuid,
                BleDecoder::of(BleFormat::F64Be),
                &1.25f64.to_be_bytes(),
            ),
            1.25,
        );
    }

    #[test]
    fn writes_encode_back_to_what_reads_decode() {
        let uuid = "7d2e0002-6c1a-4b7e-9e4f-3a5c8d1b0e21";
        for format in BleFormat::ALL {
            if format.size().is_none() {
                continue;
            }
            let decoder = BleDecoder::of(format);
            let bytes = encode(uuid, decoder, 100.0).unwrap();
            assert_eq!(bytes.len(), format.size().unwrap_or(0));
            close(decode(uuid, decoder, &bytes), 100.0);
        }
        // A standard characteristic writes in its own layout and scale.
        let temperature = gatt::uuid16(0x2A6E);
        assert_eq!(
            encode(&temperature, BleDecoder::default(), 21.5),
            Ok(2150i16.to_le_bytes().to_vec())
        );
        assert_eq!(
            encode(&gatt::uuid16(0x2A06), BleDecoder::default(), 2.0),
            Ok(vec![2])
        );
    }

    #[test]
    fn writes_refuse_what_does_not_fit() {
        let uuid = "7d2e0002-6c1a-4b7e-9e4f-3a5c8d1b0e21";
        assert!(encode(uuid, BleDecoder::of(BleFormat::U8), 256.0).is_err());
        assert!(encode(uuid, BleDecoder::of(BleFormat::U8), -1.0).is_err());
        assert!(encode(uuid, BleDecoder::of(BleFormat::I16Le), 40_000.0).is_err());
        assert!(encode(uuid, BleDecoder::of(BleFormat::F32Le), 1e300).is_err());
        assert!(encode(uuid, BleDecoder::of(BleFormat::Utf8), 1.0).is_err());
        assert!(
            encode(uuid, BleDecoder::default(), 1.0).is_err(),
            "unknown layout"
        );
        assert!(
            encode(&gatt::uuid16(0x2A37), BleDecoder::default(), 60.0).is_err(),
            "heart rate is a structure"
        );
        let offset = BleDecoder {
            offset: 2,
            ..BleDecoder::of(BleFormat::U8)
        };
        assert!(encode(uuid, offset, 1.0).is_err());
        // Rounded to the nearest step of the scale.
        let scaled = BleDecoder {
            scale: 0.5,
            ..BleDecoder::of(BleFormat::U8)
        };
        assert_eq!(encode(uuid, scaled, 10.2), Ok(vec![20]));
    }

    #[test]
    fn typed_hex_in_every_common_spelling() {
        assert_eq!(parse_hex("06 48"), Ok(vec![0x06, 0x48]));
        assert_eq!(parse_hex("0x0648"), Ok(vec![0x06, 0x48]));
        assert_eq!(parse_hex("0x06 0X48"), Ok(vec![0x06, 0x48]));
        assert_eq!(parse_hex("de:ad:BE:ef"), Ok(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(parse_hex("01,02-03"), Ok(vec![1, 2, 3]));
        assert_eq!(parse_hex("  "), Ok(Vec::new()));
        assert!(parse_hex("123").is_err(), "half a byte");
        assert!(parse_hex("zz").is_err());
        assert!(parse_hex("0x").is_ok_and(|bytes| bytes.is_empty()));
        assert_eq!(hex(&[0x06, 0x48, 0xFF]), "06 48 FF");
        assert_eq!(hex(&[]), "");
    }
}
