pub(crate) mod master;
pub(crate) mod slave;

use std::fmt::Write as _;

use mt_modbus::{Endianness, RegisterType, RtuParity, TrafficLogEntry};
use vt_protocol::{DataType, Direction, Parity, TrafficFrame, WordOrder};

pub(crate) fn register_type(data_type: DataType) -> RegisterType {
    match data_type {
        DataType::U16 => RegisterType::Uint16,
        DataType::I16 => RegisterType::Int16,
        DataType::I32 => RegisterType::Int32,
        DataType::F32 => RegisterType::Float32,
        DataType::F64 => RegisterType::Float64,
    }
}

pub(crate) fn endianness(order: WordOrder) -> Endianness {
    match order {
        WordOrder::Big => Endianness::Big,
        WordOrder::Little => Endianness::Little,
        WordOrder::Swapped => Endianness::Swapped,
    }
}

pub(crate) fn parity(parity: Parity) -> RtuParity {
    match parity {
        Parity::None => RtuParity::None,
        Parity::Odd => RtuParity::Odd,
        Parity::Even => RtuParity::Even,
    }
}

pub(crate) fn function_name(code: u8) -> &'static str {
    match code & 0x7F {
        1 => "Read Coils",
        2 => "Read Discrete",
        3 => "Read Holding",
        4 => "Read Input",
        5 => "Write Coil",
        6 => "Write Register",
        15 => "Write Coils",
        16 => "Write Registers",
        _ => "Function",
    }
}

/// Words shown per frame before the rest is summarised.
const SHOWN_WORDS: usize = 8;

/// The client logs register payloads as a debug-printed list of `0x` words
/// and requests as `count=N`; show them the way a protocol analyser does.
fn describe_data(data: &str, function: u8, length: u16) -> String {
    if let Some(count) = data.strip_prefix("count=") {
        let unit = if matches!(function & 0x7F, 1 | 2) {
            "bits"
        } else {
            "registers"
        };
        return format!("{count} {unit}");
    }
    let Ok(words) = serde_json::from_str::<Vec<String>>(data) else {
        return data.to_owned();
    };
    let mut text = words
        .iter()
        .take(SHOWN_WORDS)
        .map(|word| word.trim_start_matches("0x"))
        .collect::<Vec<_>>()
        .join(" ");
    let total = usize::from(length).max(words.len());
    if total > SHOWN_WORDS.min(words.len()) {
        let _ = write!(text, " … {total} words");
    }
    text
}

pub(crate) fn traffic_frame(entry: TrafficLogEntry) -> TrafficFrame {
    let direction = if entry.direction.eq_ignore_ascii_case("rx") {
        Direction::Rx
    } else {
        Direction::Tx
    };
    let exception = entry.function_code & 0x80 != 0;
    let summary = format!(
        "FC{:02} {}{} @{}",
        entry.function_code & 0x7F,
        function_name(entry.function_code),
        if exception { " (exception)" } else { "" },
        entry.address
    );
    TrafficFrame {
        at_ms: entry.timestamp,
        direction,
        summary,
        detail: describe_data(&entry.data, entry.function_code, entry.length),
        bytes: u32::from(entry.length),
    }
}

#[cfg(test)]
mod tests {
    use super::describe_data;

    #[test]
    fn payloads_read_like_an_analyser() {
        assert_eq!(describe_data("count=46", 3, 46), "46 registers");
        assert_eq!(describe_data("count=8", 1, 8), "8 bits");
        assert_eq!(describe_data(r#"["0x4411", "0x0E16"]"#, 3, 2), "4411 0E16");
        let long: Vec<String> = (0..8).map(|word| format!("0x{word:04X}")).collect();
        let long = format!("{long:?}");
        assert_eq!(
            describe_data(&long, 3, 46),
            "0000 0001 0002 0003 0004 0005 0006 0007 … 46 words"
        );
        assert_eq!(describe_data("Timeout", 3, 0), "Timeout");
    }
}
