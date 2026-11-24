//! Starting and stopping the CPU: the PI service `P_PROGRAM` (snap7 `opPlcStop`,
//! `opPlcHotStart`, `opPlcColdStart`).
//!
//! | Request | Parameters |
//! |---|---|
//! | stop | `29 00 00 00 00 00 09 "P_PROGRAM"` |
//! | hot start (warm restart) | `28 00 00 00 00 00 00 FD 00 00 09 "P_PROGRAM"` |
//! | cold start | `28 00 00 00 00 00 00 FD 00 02 "C " 09 "P_PROGRAM"` |
//!
//! The CPU answers with an ACK_DATA whose first parameter byte repeats the function. A second
//! byte of `07` to a stop means the CPU was already stopped; `03` to a start means it was already
//! running. Both are reported as success: the CPU is in the mode that was asked for.
//!
//! These services work on S7-300/400 and on S7-1200/1500 with PUT/GET allowed and no access
//! protection that forbids it; a protected CPU answers with an access error.

use crate::framing;
use crate::types::S7_PLC_CONTROL;

/// Job function of the PLC stop request.
pub(crate) const S7_PLC_STOP: u8 = 0x29;

const PI_PROGRAM: &[u8] = b"P_PROGRAM";

/// Second parameter byte of a stop answer when the CPU was already in STOP.
const ALREADY_STOPPED: u8 = 0x07;
/// Second parameter byte of a start answer when the CPU was already in RUN.
const ALREADY_RUNNING: u8 = 0x03;

/// How the CPU should start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    /// Warm restart: retentive data is kept.
    Warm,
    /// Cold restart: all data, retentive included, goes back to its start values.
    Cold,
}

pub(crate) fn build_stop_request(pdu_ref: u16) -> Vec<u8> {
    let mut params = vec![S7_PLC_STOP, 0, 0, 0, 0, 0, PI_PROGRAM.len() as u8];
    params.extend_from_slice(PI_PROGRAM);
    framing::job_pdu(pdu_ref, &params, &[])
}

pub(crate) fn build_start_request(pdu_ref: u16, mode: StartMode) -> Vec<u8> {
    let mut params = vec![S7_PLC_CONTROL, 0, 0, 0, 0, 0, 0, 0xFD, 0x00];
    match mode {
        StartMode::Warm => params.push(0x00),
        StartMode::Cold => params.extend_from_slice(&[0x02, b'C', b' ']),
    }
    params.push(PI_PROGRAM.len() as u8);
    params.extend_from_slice(PI_PROGRAM);
    framing::job_pdu(pdu_ref, &params, &[])
}

pub(crate) fn parse_stop_response(pdu: &[u8]) -> Result<(), String> {
    let (params, _) = framing::split_ack(pdu)?;
    match params.first() {
        Some(&S7_PLC_STOP) => Ok(()),
        _ => Err(format!("unexpected stop answer {params:02X?}")),
    }
}

pub(crate) fn parse_start_response(pdu: &[u8]) -> Result<(), String> {
    let (params, _) = framing::split_ack(pdu)?;
    match params.first() {
        Some(&S7_PLC_CONTROL) => Ok(()),
        _ => Err(format!("unexpected start answer {params:02X?}")),
    }
}

// ── Simulator ──

/// What a control request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlRequest {
    Stop,
    Start(StartMode),
}

/// Recognise a stop or start request; `None` for any other PI service (block delete, etc.).
pub(crate) fn parse_control_request(params: &[u8]) -> Option<ControlRequest> {
    let tail_is_program = |at: usize| {
        params.get(at) == Some(&(PI_PROGRAM.len() as u8)) && params.get(at + 1..) == Some(PI_PROGRAM)
    };
    match params.first()? {
        &S7_PLC_STOP if tail_is_program(6) => Some(ControlRequest::Stop),
        &S7_PLC_CONTROL if params.get(7) == Some(&0xFD) => match params.get(9)? {
            0x00 if tail_is_program(10) => Some(ControlRequest::Start(StartMode::Warm)),
            0x02 if params.get(10..12) == Some(b"C ") && tail_is_program(12) => {
                Some(ControlRequest::Start(StartMode::Cold))
            }
            _ => None,
        },
        _ => None,
    }
}

/// The simulator's answer: the function byte, then `07`/`03` when nothing had to change.
pub(crate) fn build_control_response(pdu_ref: u16, request: ControlRequest, was_running: bool) -> Vec<u8> {
    let params = match request {
        ControlRequest::Stop if !was_running => vec![S7_PLC_STOP, ALREADY_STOPPED],
        ControlRequest::Stop => vec![S7_PLC_STOP],
        ControlRequest::Start(_) if was_running => vec![S7_PLC_CONTROL, ALREADY_RUNNING],
        ControlRequest::Start(_) => vec![S7_PLC_CONTROL],
    };
    framing::ack_pdu(pdu_ref, 0, &params, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    /// The S7 part of snap7's `S7_STOP` telegram (s7_micro_client.cpp), PDU reference 0.
    #[test]
    fn stop_matches_snap7() {
        let snap7 = hex("32 01 00 00 00 00 00 10 00 00 29 00 00 00 00 00 09 50 5F 50 52 4F 47 52 41 4D");
        assert_eq!(build_stop_request(0), snap7);
    }

    /// The S7 part of snap7's `S7_HOT_START` and `S7_COLD_START` telegrams.
    #[test]
    fn starts_match_snap7() {
        let hot = hex("32 01 00 00 00 00 00 14 00 00 28 00 00 00 00 00 00 FD 00 00 09 50 5F 50 52 4F 47 52 41 4D");
        assert_eq!(build_start_request(0, StartMode::Warm), hot);
        let cold =
            hex("32 01 00 00 00 00 00 16 00 00 28 00 00 00 00 00 00 FD 00 02 43 20 09 50 5F 50 52 4F 47 52 41 4D");
        assert_eq!(build_start_request(0, StartMode::Cold), cold);
    }

    #[test]
    fn requests_round_trip_through_the_simulator_parser() {
        let params = |pdu: Vec<u8>| pdu[10..].to_vec();
        assert_eq!(parse_control_request(&params(build_stop_request(1))), Some(ControlRequest::Stop));
        assert_eq!(
            parse_control_request(&params(build_start_request(1, StartMode::Warm))),
            Some(ControlRequest::Start(StartMode::Warm))
        );
        assert_eq!(
            parse_control_request(&params(build_start_request(1, StartMode::Cold))),
            Some(ControlRequest::Start(StartMode::Cold))
        );
        // A block delete is a PI service too, but not a start.
        let delete = crate::blocks::build_delete_request(1, crate::types::BlockType::DB, 5);
        assert_eq!(parse_control_request(&params(delete)), None);
    }

    #[test]
    fn already_in_mode_is_success() {
        assert!(parse_stop_response(&build_control_response(1, ControlRequest::Stop, false)).is_ok());
        assert!(parse_start_response(&build_control_response(1, ControlRequest::Start(StartMode::Warm), true)).is_ok());
        assert!(parse_start_response(&build_control_response(1, ControlRequest::Stop, true)).is_err());
    }

    #[test]
    fn access_error_is_reported() {
        // A protected CPU refuses with a header error (0x8104: function not allowed in this context).
        let denied = framing::ack_pdu(1, 0x8104, &[], &[]);
        assert!(parse_stop_response(&denied).is_err());
    }
}
