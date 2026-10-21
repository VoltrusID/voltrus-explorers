#![allow(dead_code)]

/// A single DNP3 point value parsed from an outstation response.
#[derive(Debug, Clone, PartialEq)]
pub struct Dnp3PointValue {
    pub group: u8,
    pub variation: u8,
    pub index: u16,
    pub value: f64,
}

/// Human-readable name for a group/variation pair.
pub fn group_variation_name(group: u8, variation: u8) -> &'static str {
    match (group, variation) {
        (1, 1) => "Binary Input (packed)",
        (1, 2) => "Binary Input (with flags)",
        (10, 1) => "Binary Output Status (packed)",
        (10, 2) => "Binary Output Status (with flags)",
        (12, 1) => "Control Relay Output Block",
        (20, 1) => "Counter Input (uint32)",
        (20, 2) => "Counter Input (uint16)",
        (20, 5) => "Counter Input (uint32 no flags)",
        (20, 6) => "Counter Input (uint16 no flags)",
        (21, 1) => "Frozen Counter (uint32)",
        (21, 2) => "Frozen Counter (uint16)",
        (21, 5) => "Frozen Counter (uint32 no flags)",
        (21, 6) => "Frozen Counter (uint16 no flags)",
        (21, 9) => "Frozen Counter (uint32 with time)",
        (21, 10) => "Frozen Counter (uint16 with time)",
        (21, 11) => "Frozen Counter (uint32 no flags with time)",
        (21, 12) => "Frozen Counter (uint16 no flags with time)",
        (30, 1) => "Analog Input (int32)",
        (30, 2) => "Analog Input (int16)",
        (30, 3) => "Analog Input (int32 no flags)",
        (30, 4) => "Analog Input (float32)",
        (30, 5) => "Analog Input (float64)",
        (30, 6) => "Analog Input (float32 short)",
        (32, 1) => "Analog Output Status (int32)",
        (32, 2) => "Analog Output Status (int16)",
        (32, 3) => "Analog Output Status (float32)",
        (32, 4) => "Analog Output Status (float64)",
        (40, 1) => "Analog Output Block (int32)",
        (40, 2) => "Analog Output Block (int16)",
        (40, 3) => "Analog Output Block (float32)",
        (40, 4) => "Analog Output Block (float64)",
        (41, 1) => "Analog Output Event (int32)",
        (41, 2) => "Analog Output Event (int16)",
        (41, 3) => "Analog Output Event (float32)",
        (41, 4) => "Analog Output Event (float64)",
        (60, 1) => "Class 0 data",
        (60, 2) => "Class 1 data",
        (60, 3) => "Class 2 data",
        (60, 4) => "Class 3 data",
        _ => "Unknown",
    }
}

/// List of common DNP3 group/variation pairs for point configuration UI.
pub fn common_group_variations() -> Vec<(u8, u8, &'static str)> {
    vec![
        (1, 2, "Binary Input"),
        (10, 2, "Binary Output Status"),
        (20, 1, "Counter Input"),
        (21, 1, "Frozen Counter"),
        (30, 4, "Analog Input (float32)"),
        (30, 1, "Analog Input (int32)"),
        (32, 3, "Analog Output Status (float32)"),
        (40, 3, "Analog Output Block (float32)"),
    ]
}

/// Parse packed binary input (Group 1 Var 1).
pub fn parse_binary_input_packed(data: &[u8], start_index: u16) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for (byte_idx, &byte) in data.iter().enumerate() {
        for bit in 0..8u16 {
            let index = start_index + (byte_idx as u16) * 8 + bit;
            values.push(Dnp3PointValue {
                group: 1,
                variation: 1,
                index,
                value: if byte & (1 << bit) != 0 { 1.0 } else { 0.0 },
            });
        }
    }
    values
}

/// Parse binary input with flags (Group 1 Var 2).
pub fn parse_binary_input_flags(data: &[u8], start_index: u16, count: u16) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i;
        if offset < data.len() {
            let flags = data[offset];
            values.push(Dnp3PointValue {
                group: 1,
                variation: 2,
                index: start_index + i as u16,
                value: if flags & 0x01 != 0 { 1.0 } else { 0.0 },
            });
        }
    }
    values
}

/// Parse analog input int32 (Group 30 Var 1 or 3).
pub fn parse_analog_input_int32(data: &[u8], start_index: u16, count: u16) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 30,
                variation: 3,
                index: start_index + i as u16,
                value: raw as i32 as f64,
            });
        }
    }
    values
}

/// Parse analog input float32 (Group 30 Var 4 or 6).
pub fn parse_analog_input_float32(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 30,
                variation: 4,
                index: start_index + i as u16,
                value: f32::from_bits(raw) as f64,
            });
        }
    }
    values
}

/// Parse binary output status packed (Group 10 Var 1).
pub fn parse_binary_output_packed(data: &[u8], start_index: u16) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for (byte_idx, &byte) in data.iter().enumerate() {
        for bit in 0..8u16 {
            let index = start_index + (byte_idx as u16) * 8 + bit;
            values.push(Dnp3PointValue {
                group: 10,
                variation: 1,
                index,
                value: if byte & (1 << bit) != 0 { 1.0 } else { 0.0 },
            });
        }
    }
    values
}

/// Parse binary output status with flags (Group 10 Var 2).
pub fn parse_binary_output_flags(data: &[u8], start_index: u16, count: u16) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i;
        if offset < data.len() {
            let flags = data[offset];
            values.push(Dnp3PointValue {
                group: 10,
                variation: 2,
                index: start_index + i as u16,
                value: if flags & 0x01 != 0 { 1.0 } else { 0.0 },
            });
        }
    }
    values
}

/// Parse analog output status int32 (Group 32 Var 1 or 2).
pub fn parse_analog_output_status_int32(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 32,
                variation: 1,
                index: start_index + i as u16,
                value: raw as i32 as f64,
            });
        }
    }
    values
}

/// Parse analog output status int16 (Group 32 Var 2).
pub fn parse_analog_output_status_int16(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 2;
        if offset + 2 <= data.len() {
            let raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
            values.push(Dnp3PointValue {
                group: 32,
                variation: 2,
                index: start_index + i as u16,
                value: raw as i16 as f64,
            });
        }
    }
    values
}

/// Parse analog output status float32 (Group 32 Var 3 or float64 Var 4).
pub fn parse_analog_output_status_float32(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 32,
                variation: 3,
                index: start_index + i as u16,
                value: f32::from_bits(raw) as f64,
            });
        }
    }
    values
}

/// Parse analog output status float64 (Group 32 Var 4).
pub fn parse_analog_output_status_float64(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 8;
        if offset + 8 <= data.len() {
            let raw = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            values.push(Dnp3PointValue {
                group: 32,
                variation: 4,
                index: start_index + i as u16,
                value: f64::from_bits(raw),
            });
        }
    }
    values
}

/// Parse analog output block int32 (Group 40 Var 1).
pub fn parse_analog_output_block_int32(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 40,
                variation: 1,
                index: start_index + i as u16,
                value: raw as i32 as f64,
            });
        }
    }
    values
}

/// Parse analog output block int16 (Group 40 Var 2).
pub fn parse_analog_output_block_int16(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 2;
        if offset + 2 <= data.len() {
            let raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
            values.push(Dnp3PointValue {
                group: 40,
                variation: 2,
                index: start_index + i as u16,
                value: raw as i16 as f64,
            });
        }
    }
    values
}

/// Parse analog output block float32 (Group 40 Var 3).
pub fn parse_analog_output_block_float32(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 40,
                variation: 3,
                index: start_index + i as u16,
                value: f32::from_bits(raw) as f64,
            });
        }
    }
    values
}

/// Parse analog output block float64 (Group 40 Var 4).
pub fn parse_analog_output_block_float64(
    data: &[u8],
    start_index: u16,
    count: u16,
) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 8;
        if offset + 8 <= data.len() {
            let raw = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            values.push(Dnp3PointValue {
                group: 40,
                variation: 4,
                index: start_index + i as u16,
                value: f64::from_bits(raw),
            });
        }
    }
    values
}

/// Parse counter input uint32 (Group 20 Var 1 or 5).
pub fn parse_counter_input(data: &[u8], start_index: u16, count: u16) -> Vec<Dnp3PointValue> {
    let mut values = Vec::new();
    for i in 0..count as usize {
        let offset = i * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            values.push(Dnp3PointValue {
                group: 20,
                variation: 1,
                index: start_index + i as u16,
                value: raw as f64,
            });
        }
    }
    values
}

/// Parse objects from an ASDU response body.
pub fn parse_objects(
    group: u8,
    variation: u8,
    start_index: u16,
    count: u16,
    data: &[u8],
) -> Vec<Dnp3PointValue> {
    match (group, variation) {
        (1, 1) => parse_binary_input_packed(data, start_index),
        (1, 2) => parse_binary_input_flags(data, start_index, count),
        (10, 1) => parse_binary_output_packed(data, start_index),
        (10, 2) => parse_binary_output_flags(data, start_index, count),
        (30, 1) | (30, 3) => parse_analog_input_int32(data, start_index, count),
        (30, 4) | (30, 6) => parse_analog_input_float32(data, start_index, count),
        (32, 1) => parse_analog_output_status_int32(data, start_index, count),
        (32, 2) => parse_analog_output_status_int16(data, start_index, count),
        (32, 3) => parse_analog_output_status_float32(data, start_index, count),
        (32, 4) => parse_analog_output_status_float64(data, start_index, count),
        (40, 1) => parse_analog_output_block_int32(data, start_index, count),
        (40, 2) => parse_analog_output_block_int16(data, start_index, count),
        (40, 3) => parse_analog_output_block_float32(data, start_index, count),
        (40, 4) => parse_analog_output_block_float64(data, start_index, count),
        (20, 1) | (20, 5) => parse_counter_input(data, start_index, count),
        (21, 1) | (21, 5) => parse_counter_input(data, start_index, count),
        _ => {
            tracing::debug!("Unsupported DNP3 object: Group {} Var {}", group, variation);
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_input_packed() {
        let data = vec![0b0000_0101];
        let values = parse_binary_input_packed(&data, 0);
        assert_eq!(values.len(), 8);
        assert_eq!(values[0].value, 1.0);
        assert_eq!(values[1].value, 0.0);
        assert_eq!(values[2].value, 1.0);
    }

    #[test]
    fn test_analog_input_int32() {
        let data = vec![0xE8, 0x03, 0x00, 0x00];
        let values = parse_analog_input_int32(&data, 0, 1);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].value, 1000.0);
    }

    #[test]
    fn test_analog_input_int32_negative() {
        let data = vec![0x18, 0xFC, 0xFF, 0xFF];
        let values = parse_analog_input_int32(&data, 0, 1);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].value, -1000.0);
    }

    #[test]
    fn test_analog_input_float32() {
        let data = vec![0x00, 0x00, 0x80, 0x3F];
        let values = parse_analog_input_float32(&data, 0, 1);
        assert_eq!(values.len(), 1);
        assert!((values[0].value - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_counter_input_uint32() {
        let data: Vec<u8> = 9876546u32.to_le_bytes().to_vec();
        let values = parse_counter_input(&data, 0, 1);
        assert_eq!(values.len(), 1);
        assert!((values[0].value - 9876546.0).abs() < 0.001);
    }

    #[test]
    fn test_group_variation_from_known_types() {
        assert_eq!(group_variation_name(1, 1), "Binary Input (packed)");
        assert_eq!(group_variation_name(1, 2), "Binary Input (with flags)");
        assert_eq!(group_variation_name(30, 1), "Analog Input (int32)");
        assert_eq!(group_variation_name(30, 4), "Analog Input (float32)");
        assert_eq!(group_variation_name(30, 6), "Analog Input (float32 short)");
        assert_eq!(group_variation_name(20, 1), "Counter Input (uint32)");
        assert_eq!(group_variation_name(60, 1), "Class 0 data");
        assert_eq!(group_variation_name(60, 2), "Class 1 data");
        assert_eq!(group_variation_name(60, 3), "Class 2 data");
        assert_eq!(group_variation_name(60, 4), "Class 3 data");
    }

    #[test]
    fn test_dnp3_point_value_creation() {
        let pv = Dnp3PointValue {
            group: 30,
            variation: 3,
            index: 5,
            value: 220.5,
        };
        assert_eq!(pv.group, 30);
        assert_eq!(pv.index, 5);
        assert!((pv.value - 220.5).abs() < 0.001);
    }

    #[test]
    fn test_parse_objects_dispatch() {
        let data = vec![0xE8, 0x03, 0x00, 0x00];
        let values = parse_objects(30, 3, 0, 1, &data);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].value, 1000.0);
    }
}
