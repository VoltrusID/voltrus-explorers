//! Control requests: SELECT, OPERATE and DIRECT OPERATE with a CROB (Group 12 Var 1) or an analog
//! output block (Group 41 Var 3, float32), and the echoed object with its control status.

use anyhow::Result;

use crate::application::{FunctionCode, Iin, build_request, build_response};

/// Qualifier 0x17: 1-byte count, 1-byte index prefix on each object.
const PREFIX8_COUNT8: u8 = 0x17;

/// CROB control code bits: operation type in the low nibble, queue/clear in bits 4 and 5,
/// trip/close code in bits 6 and 7.
pub mod crob_code {
    pub const PULSE_ON: u8 = 0x01;
    pub const PULSE_OFF: u8 = 0x02;
    pub const LATCH_ON: u8 = 0x03;
    pub const LATCH_OFF: u8 = 0x04;
    /// Trip/close code CLOSE (bit 6). Combine with an operation, for example `CLOSE | PULSE_ON`.
    pub const CLOSE: u8 = 0x40;
    /// Trip/close code TRIP (bit 7).
    pub const TRIP: u8 = 0x80;
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

    /// True when the operation drives the output on: close, or pulse/latch on. When the code
    /// carries a trip/close pair, the trip/close bits decide.
    pub fn drives_on(&self) -> bool {
        match self.code & 0xC0 {
            crob_code::CLOSE => true,
            crob_code::TRIP => false,
            _ => matches!(self.code & 0x0F, crob_code::PULSE_ON | crob_code::LATCH_ON),
        }
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
                if !matches!(c.code & 0x0F, 1..=4) || c.code & 0xC0 == 0xC0 {
                    anyhow::bail!("CROB control code {:#04x} is not a valid pulse or latch operation", c.code);
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
            PREFIX8_COUNT8,
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
    Ok(build_request(function, sequence, &request.encode_objects(ControlStatus::Success)))
}

/// Build the response to a control request: the request object echoed with `status`.
pub fn build_control_response(
    sequence: u8,
    iin: &Iin,
    request: &ControlRequest,
    status: ControlStatus,
) -> Vec<u8> {
    build_response(sequence, iin, &request.encode_objects(status))
}

/// A control object read back from a request or response.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlEcho {
    pub request: ControlRequest,
    pub status: ControlStatus,
}

/// Parse the first control object (CROB or analog output block) from the object bytes of a
/// request or response (everything after the application header).
/// Returns `Ok(None)` when there are no objects.
pub fn parse_control_objects(objects: &[u8]) -> Result<Option<ControlEcho>> {
    if objects.is_empty() {
        return Ok(None);
    }
    if objects.len() < 5 {
        anyhow::bail!("Control object truncated ({} bytes)", objects.len());
    }
    let (group, variation, qualifier, count) = (objects[0], objects[1], objects[2], objects[3]);
    if qualifier != PREFIX8_COUNT8 {
        anyhow::bail!("Unsupported control qualifier {qualifier:#04x}");
    }
    if count == 0 {
        anyhow::bail!("Control object count is 0");
    }
    let index = objects[4] as u16;
    let body = &objects[5..];
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

#[cfg(test)]
mod tests {
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
        // Request header is two octets: application control, function code.
        assert_eq!(select[..2], [0xC6, 0x03]);
        assert_eq!(operate[..2], [0xC7, 0x04]);
        assert_eq!(select[2..], operate[2..]);
    }

    #[test]
    fn crob_wire_layout() {
        let asdu = build_control_request(FunctionCode::Select, 0, &crob(5)).unwrap();
        let mut expected = vec![12, 1, 0x17, 1, 5, crob_code::LATCH_OFF, 2];
        expected.extend_from_slice(&300u32.to_le_bytes());
        expected.extend_from_slice(&700u32.to_le_bytes());
        expected.push(0); // status in a request
        assert_eq!(asdu[2..], expected[..]);
    }

    #[test]
    fn analog_output_wire_layout() {
        let req = ControlRequest { index: 7, object: ControlObject::AnalogOutput(-2.5) };
        let asdu = build_control_request(FunctionCode::Operate, 0, &req).unwrap();
        let mut expected = vec![41, 3, 0x17, 1, 7];
        expected.extend_from_slice(&(-2.5f32).to_le_bytes());
        expected.push(0);
        assert_eq!(asdu[2..], expected[..]);
    }

    #[test]
    fn response_round_trips_with_status() {
        for req in [crob(3), ControlRequest { index: 9, object: ControlObject::AnalogOutput(10.25) }] {
            let reply = build_control_response(4, &Iin::default(), &req, ControlStatus::NoSelect);
            // Response header is four octets: control, function, IIN1, IIN2.
            assert_eq!(reply[..4], [0xC4, 0x81, 0x00, 0x00]);
            let echo = parse_control_objects(&reply[4..]).unwrap().unwrap();
            assert_eq!(echo.request, req);
            assert_eq!(echo.status, ControlStatus::NoSelect);
        }
    }

    #[test]
    fn empty_and_truncated_objects() {
        assert_eq!(parse_control_objects(&[]).unwrap(), None);
        let objects = crob(1).encode_objects(ControlStatus::Success);
        assert!(parse_control_objects(&objects[..objects.len() - 3]).is_err());
        assert!(parse_control_objects(&objects[..3]).is_err());
    }

    #[test]
    fn validation_rejects_unencodable_requests() {
        assert!(crob(256).validate().is_err());
        assert!(crob(255).validate().is_ok());
        let mut bad_code = crob(0);
        bad_code.object = ControlObject::Crob(Crob { code: 0x00, count: 1, on_time_ms: 0, off_time_ms: 0 });
        assert!(bad_code.validate().is_err());
        let both_tcc = Crob { code: 0xC1, count: 1, on_time_ms: 0, off_time_ms: 0 };
        assert!(ControlRequest { index: 0, object: ControlObject::Crob(both_tcc) }.validate().is_err());
        let nan = ControlRequest { index: 0, object: ControlObject::AnalogOutput(f32::NAN) };
        assert!(build_control_request(FunctionCode::Select, 0, &nan).is_err());
    }

    #[test]
    fn trip_close_bits_decide_direction() {
        let close = Crob { code: crob_code::CLOSE | crob_code::PULSE_ON, count: 1, on_time_ms: 100, off_time_ms: 100 };
        let trip = Crob { code: crob_code::TRIP | crob_code::PULSE_ON, ..close };
        assert!(close.drives_on());
        assert!(!trip.drives_on());
        assert!(Crob::pulse(true).drives_on());
        assert!(Crob { code: crob_code::LATCH_ON, ..Crob::pulse(true) }.drives_on());
        assert!(!Crob { code: crob_code::LATCH_OFF, ..Crob::pulse(true) }.drives_on());
    }

    #[test]
    fn status_codes_round_trip() {
        for v in 0..=255u8 {
            assert_eq!(ControlStatus::from_u8(v).as_u8(), v);
        }
        assert_eq!(ControlStatus::from_u8(2), ControlStatus::NoSelect);
    }
}
