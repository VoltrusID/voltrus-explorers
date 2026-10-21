#![allow(dead_code)]

use anyhow::Result;

/// DNP3 application function codes.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum FunctionCode {
    Read = 0x01,
    Write = 0x02,
    Select = 0x03,
    Operate = 0x04,
    DirectOperate = 0x05,
    DirectOperateNoAck = 0x06,
    Freeze = 0x07,
    FreezeClear = 0x08,
    FreezeAtTime = 0x09,
    ColdRestart = 0x0D,
    WarmRestart = 0x0E,
    EnableUnsolicited = 0x14,
    DisableUnsolicited = 0x15,
    Response = 0x81,
    UnsolicitedResponse = 0x82,
}

impl FunctionCode {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0x01 => Ok(Self::Read),
            0x02 => Ok(Self::Write),
            0x03 => Ok(Self::Select),
            0x04 => Ok(Self::Operate),
            0x05 => Ok(Self::DirectOperate),
            0x06 => Ok(Self::DirectOperateNoAck),
            0x07 => Ok(Self::Freeze),
            0x08 => Ok(Self::FreezeClear),
            0x09 => Ok(Self::FreezeAtTime),
            0x0D => Ok(Self::ColdRestart),
            0x0E => Ok(Self::WarmRestart),
            0x14 => Ok(Self::EnableUnsolicited),
            0x15 => Ok(Self::DisableUnsolicited),
            0x81 => Ok(Self::Response),
            0x82 => Ok(Self::UnsolicitedResponse),
            _ => anyhow::bail!("Unknown function code: 0x{:02X}", v),
        }
    }
}

/// DNP3 object header qualifier codes.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum QualifierCode {
    StartStop8 = 0x00,
    StartStop16 = 0x01,
    Count8 = 0x07,
    Count16 = 0x08,
    CountAndPrefix8 = 0x17,
}

/// DNP3 Internal Indications (IIN) -- 2 bytes of device status flags.
#[derive(Debug, Clone, Default)]
pub struct Iin {
    pub all_stations: bool,
    pub device_restart: bool,
    pub device_trouble: bool,
    pub local_control: bool,
    pub need_time: bool,
    pub class_1_events: bool,
    pub class_2_events: bool,
    pub class_3_events: bool,
}

impl Iin {
    pub fn to_low_byte(&self) -> u8 {
        let mut byte = 0u8;
        if self.all_stations {
            byte |= 0x01;
        }
        if self.device_restart {
            byte |= 0x02;
        }
        if self.device_trouble {
            byte |= 0x04;
        }
        if self.local_control {
            byte |= 0x08;
        }
        if self.need_time {
            byte |= 0x10;
        }
        if self.class_1_events {
            byte |= 0x20;
        }
        if self.class_2_events {
            byte |= 0x40;
        }
        if self.class_3_events {
            byte |= 0x80;
        }
        byte
    }

    pub fn from_bytes(low: u8, _high: u8) -> Self {
        Self {
            all_stations: low & 0x01 != 0,
            device_restart: low & 0x02 != 0,
            device_trouble: low & 0x04 != 0,
            local_control: low & 0x08 != 0,
            need_time: low & 0x10 != 0,
            class_1_events: low & 0x20 != 0,
            class_2_events: low & 0x40 != 0,
            class_3_events: low & 0x80 != 0,
        }
    }
}

/// DNP3 ASDU header (4 bytes: function, control, IIN low, IIN high).
#[derive(Debug, Clone)]
pub struct AsduHeader {
    pub function: FunctionCode,
    pub sequence: u8,
    pub iin: Iin,
}

impl AsduHeader {
    pub fn encode(&self) -> [u8; 4] {
        let ac = 0xC0 | (self.sequence & 0x0F);
        [self.function as u8, ac, self.iin.to_low_byte(), 0x00]
    }

    pub fn decode(bytes: &[u8; 4]) -> Result<Self> {
        let function = FunctionCode::from_u8(bytes[0])?;
        let sequence = bytes[1] & 0x0F;
        let iin = Iin::from_bytes(bytes[2], bytes[3]);
        Ok(Self {
            function,
            sequence,
            iin,
        })
    }
}

/// DNP3 object header in an ASDU.
#[derive(Debug, Clone)]
pub struct ObjectHeader {
    pub group: u8,
    pub variation: u8,
    pub qualifier: QualifierCode,
    pub start_stop_or_count: Vec<u8>,
}

impl ObjectHeader {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![self.group, self.variation, self.qualifier as u8];
        bytes.extend_from_slice(&self.start_stop_or_count);
        bytes
    }
}

/// Build a READ request ASDU for a class-based poll.
pub fn build_read_request(sequence: u8, group: u8, variation: u8) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::Read,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.push(group);
    asdu.push(variation);
    asdu.push(QualifierCode::Count8 as u8);
    asdu.push(0x01);
    asdu
}

/// Build a READ request for integrity poll (Class 0 = Group 60 Var 1).
pub fn build_integrity_poll_request(sequence: u8) -> Vec<u8> {
    build_read_request(sequence, 60, 1)
}

/// Build a READ request for event scan (Class 1/2/3 = Group 60 Var 2/3/4).
pub fn build_event_scan_request(sequence: u8, class: u8) -> Vec<u8> {
    let variation = match class {
        1 => 2,
        2 => 3,
        3 => 4,
        _ => 1,
    };
    build_read_request(sequence, 60, variation)
}

/// Build a FREEZE request.
pub fn build_freeze_request(sequence: u8) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::Freeze,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.push(20);
    asdu.push(0);
    asdu.push(QualifierCode::Count8 as u8);
    asdu.push(0x00);
    asdu
}

/// Build a WRITE request carrying the current time (Group 50 Var 1, Time and Date).
/// DNP3 time is milliseconds since Unix epoch as a little-endian u64.
pub fn build_time_sync_request(sequence: u8) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::Write,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.push(50); // group 50 — Time and Date
    asdu.push(1); // var 1
    asdu.push(QualifierCode::Count8 as u8);
    asdu.push(0x01);
    let now = now_ms();
    asdu.extend_from_slice(&now.to_le_bytes());
    asdu
}

/// Build a COLD RESTART request (0x0D). No objects in the ASDU.
pub fn build_cold_restart_request(sequence: u8) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::ColdRestart,
        sequence,
        iin: Iin::default(),
    };
    header.encode().to_vec()
}

/// Build a WARM RESTART request (0x0E). No objects in the ASDU.
pub fn build_warm_restart_request(sequence: u8) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::WarmRestart,
        sequence,
        iin: Iin::default(),
    };
    header.encode().to_vec()
}

/// Build a DIRECT_OPERATE request for an analog output (Group 41 Var 3 float32).
/// Format: index (1-byte prefix) + float32 value + 1-byte status.
pub fn build_analog_output_direct_operate(sequence: u8, index: u16, value: f64) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::DirectOperate,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.push(41); // group 41 — Analog Output Event
    asdu.push(3); // var 3 — float32
    asdu.push(QualifierCode::CountAndPrefix8 as u8);
    asdu.push(0x01);
    asdu.push(index as u8);
    asdu.extend_from_slice(&(value as f32).to_le_bytes());
    asdu.push(0x00); // status byte
    asdu
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build a DIRECT_OPERATE request for binary control output (Group 12 Var 1).
pub fn build_direct_operate_binary(sequence: u8, index: u16, on: bool) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::DirectOperate,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.push(12);
    asdu.push(1);
    asdu.push(QualifierCode::CountAndPrefix8 as u8);
    asdu.push(0x01);
    asdu.push(index as u8);
    let code = if on { 0x01 } else { 0x02 };
    asdu.push(code);
    asdu.push(0x01);
    asdu.extend_from_slice(&1000u32.to_le_bytes());
    asdu.extend_from_slice(&0u32.to_le_bytes());
    asdu
}

/// A parsed object from an ASDU response.
#[derive(Debug, Clone)]
pub struct ParsedObject {
    pub group: u8,
    pub variation: u8,
    pub start_index: u16,
    pub count: usize,
    pub prefix_size: usize,
    pub data: Vec<u8>,
}

/// Estimate the byte size of a single object instance based on its group,
/// variation, and prefix size. Sizes match the bare-value wire format produced
/// by the outstation and consumed by the `objects` parsers (no flag byte).
fn estimate_object_size(group: u8, variation: u8, prefix_size: usize) -> usize {
    let data_size = match (group, variation) {
        (1, 1) | (10, 1) => 0,                    // packed
        (1, 2) | (10, 2) | (12, 1) => 1,          // flag byte only
        (20, 2) | (21, 2) | (30, 2) | (32, 2) | (40, 2) => 2, // u16
        (20, 1) | (20, 5) | (21, 1) | (21, 5) => 4, // counter u32
        (30, 1) | (30, 3) | (32, 1) | (40, 1) => 4, // int u32
        (30, 4) | (30, 6) | (32, 3) | (40, 3) => 4, // float32
        (32, 4) | (40, 4) => 8,                   // float64
        _ => 0,
    };
    prefix_size + data_size
}

/// Parse a response ASDU into object headers and data.
pub fn parse_response_asdu(data: &[u8]) -> Result<(AsduHeader, Vec<ParsedObject>)> {
    if data.len() < 4 {
        anyhow::bail!("ASDU too short: {} bytes", data.len());
    }
    let header_bytes: [u8; 4] = [data[0], data[1], data[2], data[3]];
    let header = AsduHeader::decode(&header_bytes)?;

    let mut objects = Vec::new();
    let mut offset = 4;
    while offset + 3 <= data.len() {
        let group = data[offset];
        let variation = data[offset + 1];
        let qualifier_byte = data[offset + 2];
        offset += 3;

        let (count, start_index, prefix_size) = match qualifier_byte {
            0x00 => {
                if offset + 2 > data.len() {
                    break;
                }
                let start = data[offset] as u16;
                let stop = data[offset + 1] as u16;
                offset += 2;
                let count = (stop - start + 1) as usize;
                (count, start, 0)
            }
            0x07 => {
                if offset + 1 > data.len() {
                    break;
                }
                let count = data[offset] as usize;
                offset += 1;
                (count, 0, 0)
            }
            0x17 => {
                if offset + 1 > data.len() {
                    break;
                }
                let count = data[offset] as usize;
                offset += 1;
                (count, 0, 1)
            }
            _ => {
                tracing::warn!("Unsupported qualifier: 0x{:02X}", qualifier_byte);
                break;
            }
        };

        let object_data = data[offset..].to_vec();

        let per_object_bytes = estimate_object_size(group, variation, prefix_size);
        let total_data_bytes = per_object_bytes.saturating_mul(count);
        objects.push(ParsedObject {
            group,
            variation,
            start_index,
            count,
            prefix_size,
            data: object_data,
        });
        offset = offset.saturating_add(total_data_bytes).min(data.len());
    }

    Ok((header, objects))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asdu_header_encode() {
        let header = AsduHeader {
            function: FunctionCode::Read,
            sequence: 5,
            iin: Iin::default(),
        };
        let bytes = header.encode();
        assert_eq!(bytes[0], FunctionCode::Read as u8);
        assert_eq!(bytes[1], 0xC5);
        assert_eq!(bytes[2], 0x00);
        assert_eq!(bytes[3], 0x00);
    }

    #[test]
    fn test_asdu_header_decode() {
        let bytes = [0x01, 0xC5, 0x02, 0x00];
        let header = AsduHeader::decode(&bytes).unwrap();
        assert_eq!(header.function, FunctionCode::Read);
        assert_eq!(header.sequence, 5);
        assert!(header.iin.device_restart);
    }

    #[test]
    fn test_object_header_count_based() {
        let oh = ObjectHeader {
            group: 30,
            variation: 4,
            qualifier: QualifierCode::Count8,
            start_stop_or_count: vec![2],
        };
        let bytes = oh.encode();
        assert_eq!(bytes[0], 30);
        assert_eq!(bytes[1], 4);
        assert_eq!(bytes[2], QualifierCode::Count8 as u8);
        assert_eq!(bytes[3], 2);
    }

    #[test]
    fn test_function_code_values() {
        assert_eq!(FunctionCode::Read as u8, 0x01);
        assert_eq!(FunctionCode::Write as u8, 0x02);
        assert_eq!(FunctionCode::DirectOperate as u8, 0x05);
        assert_eq!(FunctionCode::Freeze as u8, 0x07);
        assert_eq!(FunctionCode::Response as u8, 0x81);
    }

    #[test]
    fn test_iin_flags() {
        let mut iin = Iin::default();
        assert!(!iin.device_restart);
        iin.device_restart = true;
        assert!(iin.device_restart);
        let byte = iin.to_low_byte();
        assert_eq!(byte & 0x02, 0x02);
    }
}
