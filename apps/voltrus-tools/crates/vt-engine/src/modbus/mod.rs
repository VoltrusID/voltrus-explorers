pub(crate) mod master;
pub(crate) mod slave;

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
        2 => "Read Discrete Inputs",
        3 => "Read Holding Registers",
        4 => "Read Input Registers",
        5 => "Write Single Coil",
        6 => "Write Single Register",
        15 => "Write Multiple Coils",
        16 => "Write Multiple Registers",
        _ => "Function",
    }
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
        detail: entry.data,
        bytes: u32::from(entry.length),
    }
}
