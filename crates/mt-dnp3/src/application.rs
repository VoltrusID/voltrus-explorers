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

/// CROB (Group 12 Var 1) control code operation types: the low nibble of the control code byte.
pub mod crob_code {
    pub const PULSE_ON: u8 = 0x01;
    pub const PULSE_OFF: u8 = 0x02;
    pub const LATCH_ON: u8 = 0x03;
    pub const LATCH_OFF: u8 = 0x04;
}

/// Largest point index a control request can carry: objects use the 1-byte prefix qualifier (0x17).
pub const MAX_CONTROL_INDEX: u16 = 255;

/// Control relay output block (Group 12 Var 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crob {
    pub code: u8,
    pub count: u8,
    pub on_time_ms: u32,
    pub off_time_ms: u32,
}

impl Crob {
    /// One 1000 ms pulse on or off.
    pub fn pulse(on: bool) -> Self {
        Self {
            code: if on { crob_code::PULSE_ON } else { crob_code::PULSE_OFF },
            count: 1,
            on_time_ms: 1000,
            off_time_ms: 0,
        }
    }

    /// True when the operation drives the output on (pulse on or latch on).
    pub fn drives_on(&self) -> bool {
        matches!(self.code & 0x0F, crob_code::PULSE_ON | crob_code::LATCH_ON)
    }
}

/// The object carried by a control request: a CROB (Group 12 Var 1) or an analog output block
/// (Group 41 Var 3, float32).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlObject {
    Crob(Crob),
    AnalogOutput(f32),
}

/// One control point with the value to command (SELECT, OPERATE or DIRECT OPERATE).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlRequest {
    pub index: u16,
    pub object: ControlObject,
}

/// Control status carried in the last byte of an echoed control object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlStatus {
    Success,
    Timeout,
    NoSelect,
    FormatError,
    NotSupported,
    AlreadyActive,
    HardwareError,
    Local,
    TooManyObjs,
    NotAuthorized,
    AutomationInhibit,
    ProcessingLimited,
    OutOfRange,
    NonParticipating,
    Undefined,
    Other(u8),
}

impl ControlStatus {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Success,
            1 => Self::Timeout,
            2 => Self::NoSelect,
            3 => Self::FormatError,
            4 => Self::NotSupported,
            5 => Self::AlreadyActive,
            6 => Self::HardwareError,
            7 => Self::Local,
            8 => Self::TooManyObjs,
            9 => Self::NotAuthorized,
            10 => Self::AutomationInhibit,
            11 => Self::ProcessingLimited,
            12 => Self::OutOfRange,
            126 => Self::NonParticipating,
            127 => Self::Undefined,
            other => Self::Other(other),
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Timeout => 1,
            Self::NoSelect => 2,
            Self::FormatError => 3,
            Self::NotSupported => 4,
            Self::AlreadyActive => 5,
            Self::HardwareError => 6,
            Self::Local => 7,
            Self::TooManyObjs => 8,
            Self::NotAuthorized => 9,
            Self::AutomationInhibit => 10,
            Self::ProcessingLimited => 11,
            Self::OutOfRange => 12,
            Self::NonParticipating => 126,
            Self::Undefined => 127,
            Self::Other(v) => v,
        }
    }
}

impl std::fmt::Display for ControlStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Success => "SUCCESS",
            Self::Timeout => "TIMEOUT (the select expired before the operate)",
            Self::NoSelect => "NO_SELECT (no matching select)",
            Self::FormatError => "FORMAT_ERROR",
            Self::NotSupported => "NOT_SUPPORTED",
            Self::AlreadyActive => "ALREADY_ACTIVE",
            Self::HardwareError => "HARDWARE_ERROR",
            Self::Local => "LOCAL (point is under local control)",
            Self::TooManyObjs => "TOO_MANY_OBJS",
            Self::NotAuthorized => "NOT_AUTHORIZED",
            Self::AutomationInhibit => "AUTOMATION_INHIBIT",
            Self::ProcessingLimited => "PROCESSING_LIMITED",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::NonParticipating => "NON_PARTICIPATING",
            Self::Undefined => "UNDEFINED",
            Self::Other(v) => return write!(f, "status {v}"),
        };
        f.write_str(text)
    }
}

impl ControlRequest {
    pub fn group(&self) -> u8 {
        match self.object {
            ControlObject::Crob(_) => 12,
            ControlObject::AnalogOutput(_) => 41,
        }
    }

    pub fn variation(&self) -> u8 {
        match self.object {
            ControlObject::Crob(_) => 1,
            ControlObject::AnalogOutput(_) => 3,
        }
    }

    /// Reject requests that cannot be encoded on the wire.
    pub fn validate(&self) -> Result<()> {
        if self.index > MAX_CONTROL_INDEX {
            anyhow::bail!(
                "Point index {} is above {MAX_CONTROL_INDEX}; only 1-byte indices are supported",
                self.index
            );
        }
        match self.object {
            ControlObject::Crob(c) => {
                if !matches!(c.code & 0x0F, 1..=4) {
                    anyhow::bail!("CROB control code {:#04x} is not pulse on/off or latch on/off", c.code);
                }
                if c.count == 0 {
                    anyhow::bail!("CROB count must be at least 1");
                }
            }
            ControlObject::AnalogOutput(v) => {
                if !v.is_finite() {
                    anyhow::bail!("Analog output value must be a finite number");
                }
            }
        }
        Ok(())
    }

    /// Object header and the single prefixed object, with `status` in its last byte.
    /// Requests carry status 0; the outstation echoes the object with the result.
    pub fn encode_objects(&self, status: ControlStatus) -> Vec<u8> {
        let mut out = vec![
            self.group(),
            self.variation(),
            QualifierCode::CountAndPrefix8 as u8,
            0x01,
            self.index as u8,
        ];
        match self.object {
            ControlObject::Crob(c) => {
                out.push(c.code);
                out.push(c.count);
                out.extend_from_slice(&c.on_time_ms.to_le_bytes());
                out.extend_from_slice(&c.off_time_ms.to_le_bytes());
            }
            ControlObject::AnalogOutput(v) => out.extend_from_slice(&v.to_le_bytes()),
        }
        out.push(status.as_u8());
        out
    }
}

/// Build a SELECT, OPERATE or DIRECT OPERATE request ASDU for one control point.
pub fn build_control_request(
    function: FunctionCode,
    sequence: u8,
    request: &ControlRequest,
) -> Result<Vec<u8>> {
    request.validate()?;
    let header = AsduHeader {
        function,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.extend_from_slice(&request.encode_objects(ControlStatus::Success));
    Ok(asdu)
}

/// Build the response to a control request: the request object echoed with `status`.
pub fn build_control_response(sequence: u8, request: &ControlRequest, status: ControlStatus) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::Response,
        sequence,
        iin: Iin::default(),
    };
    let mut asdu = header.encode().to_vec();
    asdu.extend_from_slice(&request.encode_objects(status));
    asdu
}

/// A control object read back from a request or response ASDU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlEcho {
    pub request: ControlRequest,
    pub status: ControlStatus,
}

/// Parse the first control object (CROB or analog output block) of an ASDU.
/// Returns `Ok(None)` when the ASDU carries no objects.
pub fn parse_control_objects(asdu: &[u8]) -> Result<Option<ControlEcho>> {
    if asdu.len() <= 4 {
        return Ok(None);
    }
    if asdu.len() < 9 {
        anyhow::bail!("Control object truncated ({} bytes)", asdu.len());
    }
    let (group, variation, qualifier, count) = (asdu[4], asdu[5], asdu[6], asdu[7]);
    if qualifier != QualifierCode::CountAndPrefix8 as u8 {
        anyhow::bail!("Unsupported control qualifier {qualifier:#04x}");
    }
    if count == 0 {
        anyhow::bail!("Control object count is 0");
    }
    let index = asdu[8] as u16;
    let body = &asdu[9..];
    let (object, status) = match (group, variation) {
        (12, 1) => {
            if body.len() < 11 {
                anyhow::bail!("CROB truncated ({} of 11 bytes)", body.len());
            }
            let crob = Crob {
                code: body[0],
                count: body[1],
                on_time_ms: u32::from_le_bytes([body[2], body[3], body[4], body[5]]),
                off_time_ms: u32::from_le_bytes([body[6], body[7], body[8], body[9]]),
            };
            (ControlObject::Crob(crob), body[10])
        }
        (41, 3) => {
            if body.len() < 5 {
                anyhow::bail!("Analog output block truncated ({} of 5 bytes)", body.len());
            }
            let value = f32::from_le_bytes([body[0], body[1], body[2], body[3]]);
            (ControlObject::AnalogOutput(value), body[4])
        }
        _ => anyhow::bail!("Unexpected object g{group}v{variation} in a control message"),
    };
    Ok(Some(ControlEcho {
        request: ControlRequest { index, object },
        status: ControlStatus::from_u8(status),
    }))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

#[cfg(test)]
mod control_tests {
    use super::*;

    fn crob(index: u16) -> ControlRequest {
        ControlRequest {
            index,
            object: ControlObject::Crob(Crob {
                code: crob_code::LATCH_OFF,
                count: 2,
                on_time_ms: 300,
                off_time_ms: 700,
            }),
        }
    }

    #[test]
    fn select_and_operate_use_function_codes_3_and_4() {
        let select = build_control_request(FunctionCode::Select, 6, &crob(5)).unwrap();
        let operate = build_control_request(FunctionCode::Operate, 7, &crob(5)).unwrap();
        assert_eq!(select[0], 0x03);
        assert_eq!(operate[0], 0x04);
        assert_eq!(select[1], 0xC6);
        assert_eq!(operate[1], 0xC7);
        // Same object bytes after the 4-byte header.
        assert_eq!(select[4..], operate[4..]);
    }

    #[test]
    fn crob_wire_layout() {
        let asdu = build_control_request(FunctionCode::Select, 0, &crob(5)).unwrap();
        let mut expected = vec![12, 1, 0x17, 1, 5, crob_code::LATCH_OFF, 2];
        expected.extend_from_slice(&300u32.to_le_bytes());
        expected.extend_from_slice(&700u32.to_le_bytes());
        expected.push(0); // status in a request
        assert_eq!(asdu[4..], expected[..]);
    }

    #[test]
    fn analog_output_wire_layout() {
        let req = ControlRequest { index: 7, object: ControlObject::AnalogOutput(-2.5) };
        let asdu = build_control_request(FunctionCode::Operate, 0, &req).unwrap();
        let mut expected = vec![41, 3, 0x17, 1, 7];
        expected.extend_from_slice(&(-2.5f32).to_le_bytes());
        expected.push(0);
        assert_eq!(asdu[4..], expected[..]);
    }

    #[test]
    fn response_round_trips_with_status() {
        for req in [crob(3), ControlRequest { index: 9, object: ControlObject::AnalogOutput(10.25) }] {
            let reply = build_control_response(4, &req, ControlStatus::NoSelect);
            assert_eq!(reply[0], FunctionCode::Response as u8);
            let echo = parse_control_objects(&reply).unwrap().unwrap();
            assert_eq!(echo.request, req);
            assert_eq!(echo.status, ControlStatus::NoSelect);
        }
    }

    #[test]
    fn empty_and_truncated_responses() {
        let bare = AsduHeader { function: FunctionCode::Response, sequence: 0, iin: Iin::default() }.encode();
        assert_eq!(parse_control_objects(&bare).unwrap(), None);
        let reply = build_control_response(0, &crob(1), ControlStatus::Success);
        assert!(parse_control_objects(&reply[..reply.len() - 3]).is_err());
        assert!(parse_control_objects(&reply[..7]).is_err());
    }

    #[test]
    fn validation_rejects_unencodable_requests() {
        assert!(crob(256).validate().is_err());
        assert!(crob(255).validate().is_ok());
        let mut bad_code = crob(0);
        bad_code.object = ControlObject::Crob(Crob { code: 0x00, count: 1, on_time_ms: 0, off_time_ms: 0 });
        assert!(bad_code.validate().is_err());
        let nan = ControlRequest { index: 0, object: ControlObject::AnalogOutput(f32::NAN) };
        assert!(build_control_request(FunctionCode::Select, 0, &nan).is_err());
    }

    #[test]
    fn status_codes_round_trip() {
        for v in 0..=255u8 {
            assert_eq!(ControlStatus::from_u8(v).as_u8(), v);
        }
        assert_eq!(ControlStatus::from_u8(2), ControlStatus::NoSelect);
    }
}
