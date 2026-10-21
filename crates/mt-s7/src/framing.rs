//! S7 PDU framing shared by the SZL, block and simulator code: job and ACK_DATA PDUs and the
//! "user data" PDUs (message type 0x07) that carry SZL reads and block listing.
//!
//! A user data PDU is the 10-byte S7 header (`32 07 00 00 <pdu ref> <param len> <data len>`),
//! parameters and data:
//!
//! ```text
//! request, first   00 01 12 04 11 <4x|group> <subfunction> <sequence>                  8 bytes
//! request, next    00 01 12 08 12 <4x|group> <subfunction> <sequence> 00 00 00 00     12 bytes
//! response         00 01 12 08 12 <8x|group> <subfunction> <sequence>
//!                  <data unit ref> <last data unit> <error code (2)>                  12 bytes
//! data             <return code> <transport size> <length (2)> <payload>
//! ```
//!
//! The layouts are those of snap7 (`TReqFunTypedParams`, `TReqFunReadSZLNext`, `TS7Params7` in
//! `s7_types.h`; `opReadSZL`, `opListBlocks` and `opListBlocksOfType` in `s7_micro_client.cpp`) and
//! of the Wireshark sample captures `s7comm_reading_plc_status.pcap` and
//! `s7comm_program_blocklist_onlineview.pcap`.
//!
//! A response is split over several PDUs when it does not fit: the "last data unit" byte is 0x01
//! while more follow, and the next request carries the sequence number the PLC put in its answer.

use crate::types::*;
use crate::vars::describe_header_error;

/// Length of the S7 header of a job or user data PDU.
pub(crate) const HEADER_LEN: usize = 10;
/// Length of the S7 header of an ACK_DATA PDU: the job header plus error class and code.
pub(crate) const ACK_HEADER_LEN: usize = 12;
/// Length of the parameters of a user data response (and of a follow-up request).
pub(crate) const UD_LONG_PARAMS_LEN: usize = 12;
/// Return code, transport size and length in front of the payload of a user data PDU.
pub(crate) const UD_DATA_HEADER_LEN: usize = 4;

/// Function group of block listing (type/group `0x43` in a request).
pub(crate) const GROUP_BLOCKS: u8 = 0x03;
/// Function group of SZL reads (type/group `0x44` in a request).
pub(crate) const GROUP_SZL: u8 = 0x04;

/// Type nibble of a request (high nibble of the type/group byte).
const TYPE_REQUEST: u8 = 0x40;
/// Type nibble of a response.
const TYPE_RESPONSE: u8 = 0x80;

/// Return code of a successful data part.
pub(crate) const RETURN_OK: u8 = 0xFF;
/// Return code of a data part whose object does not exist.
pub(crate) const RETURN_NOT_FOUND: u8 = 0x0A;
/// Transport size of an octet string.
const TS_OCTET: u8 = 0x09;

/// Parameter error: "Information function unavailable", the answer to an SZL the PLC does not have
/// (Wireshark capture `s7comm_reading_plc_status`, packet 42).
pub(crate) const ERR_INFO_UNAVAILABLE: u16 = 0xD402;
/// Parameter error: the given block does not exist on the module (capture
/// `s7comm_program_blocklist_onlineview`, packet 44).
pub(crate) const ERR_BLOCK_NOT_FOUND: u16 = 0xD209;
/// Parameter error: invalid block type.
pub(crate) const ERR_INVALID_BLOCK_TYPE: u16 = 0x0113;
/// Parameter error: invalid service sequence (a follow-up or upload request without a start).
pub(crate) const ERR_INVALID_SEQUENCE: u16 = 0x8401;
/// Parameter error: the block is write-protected.
pub(crate) const ERR_BLOCK_PROTECTED: u16 = 0x0116;

/// An answer in more parts than this is a PLC that never says it is done.
pub(crate) const MAX_PARTS: usize = 4096;

// ── Job and ACK_DATA PDUs ──

/// Assemble a job PDU (message type 1): S7 header, parameters, data.
pub(crate) fn job_pdu(pdu_ref: u16, params: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s7 = Vec::with_capacity(HEADER_LEN + params.len() + data.len());
    s7.push(S7_PROTOCOL_ID);
    s7.push(S7_MSG_TYPE_JOB);
    s7.extend_from_slice(&[0x00, 0x00]);
    s7.extend_from_slice(&pdu_ref.to_be_bytes());
    s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
    s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
    s7.extend_from_slice(params);
    s7.extend_from_slice(data);
    s7
}

/// Assemble an ACK_DATA PDU (message type 3) with the given header error class and code.
pub(crate) fn ack_pdu(pdu_ref: u16, error: u16, params: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s7 = Vec::with_capacity(ACK_HEADER_LEN + params.len() + data.len());
    s7.push(S7_PROTOCOL_ID);
    s7.push(S7_MSG_TYPE_ACK_DATA);
    s7.extend_from_slice(&[0x00, 0x00]);
    s7.extend_from_slice(&pdu_ref.to_be_bytes());
    s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
    s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
    s7.extend_from_slice(&error.to_be_bytes());
    s7.extend_from_slice(params);
    s7.extend_from_slice(data);
    s7
}

/// Check the ACK_DATA header of a response to a job and return its parameters and data sections.
///
/// A header error (the PLC refused the job) becomes the `Err`.
pub(crate) fn split_ack(pdu: &[u8]) -> Result<(&[u8], &[u8]), String> {
    if pdu.len() < ACK_HEADER_LEN {
        return Err(format!("response too short ({} bytes)", pdu.len()));
    }
    if pdu[0] != S7_PROTOCOL_ID {
        return Err(format!("not an S7 PDU, protocol id 0x{:02X}", pdu[0]));
    }
    if pdu[1] != S7_MSG_TYPE_ACK_DATA {
        return Err(format!("expected an ACK_DATA response, got message type 0x{:02X}", pdu[1]));
    }
    let error = u16::from_be_bytes([pdu[10], pdu[11]]);
    if error != S7_ERR_NONE {
        return Err(describe_error(error));
    }
    let param_len = u16::from_be_bytes([pdu[6], pdu[7]]) as usize;
    let data_len = u16::from_be_bytes([pdu[8], pdu[9]]) as usize;
    let end = ACK_HEADER_LEN + param_len + data_len;
    if pdu.len() < end {
        return Err(format!("response is {} bytes but its header says {end}", pdu.len()));
    }
    Ok((&pdu[ACK_HEADER_LEN..ACK_HEADER_LEN + param_len], &pdu[ACK_HEADER_LEN + param_len..end]))
}

// ── User data requests ──

/// Build a user data request. `follow_up` asks for the next part of a response that did not fit
/// one PDU (`seq` is then the sequence number of the PLC's last answer).
pub(crate) fn build_request(
    pdu_ref: u16,
    group: u8,
    subfunction: u8,
    seq: u8,
    follow_up: bool,
    data: &[u8],
) -> Vec<u8> {
    let (plen, method) = if follow_up { (0x08, 0x12) } else { (0x04, 0x11) };
    let mut params = vec![0x00, 0x01, 0x12, plen, method, TYPE_REQUEST | group, subfunction, seq];
    if follow_up {
        params.extend_from_slice(&[0x00; 4]);
    }
    let mut s7 = job_pdu(pdu_ref, &params, data);
    s7[1] = S7_MSG_TYPE_USER_DATA;
    s7
}

/// A user data request as the simulator sees it.
pub(crate) struct Request<'a> {
    pub group: u8,
    pub subfunction: u8,
    pub seq: u8,
    pub follow_up: bool,
    pub data: &'a [u8],
}

/// Decode the parameters and data of a user data request, `None` if they are not one.
pub(crate) fn parse_request<'a>(params: &[u8], data: &'a [u8]) -> Option<Request<'a>> {
    if params.len() < 8 || params[..3] != [0x00, 0x01, 0x12] || params[5] & 0xF0 != TYPE_REQUEST {
        return None;
    }
    Some(Request {
        group: params[5] & 0x0F,
        subfunction: params[6],
        seq: params[7],
        follow_up: params[3] == 0x08,
        data,
    })
}

// ── User data responses ──

/// A decoded user data response.
#[derive(Debug)]
pub(crate) struct Response<'a> {
    /// Sequence number the PLC gives this answer; the follow-up request echoes it.
    pub seq: u8,
    /// More parts of the answer follow.
    pub more: bool,
    /// The data of this part, after the 4-byte data header.
    pub payload: &'a [u8],
}

/// Decode a user data response to a request of `group` and `subfunction`.
///
/// A PLC that cannot serve the request answers with an error code in the parameters and the
/// return code 0x0A in the data; both are turned into the `Err`.
pub(crate) fn parse_response(pdu: &[u8], group: u8, subfunction: u8) -> Result<Response<'_>, String> {
    if pdu.len() < HEADER_LEN || pdu[0] != S7_PROTOCOL_ID {
        return Err(format!("not an S7 PDU: {:02X?}", &pdu[..pdu.len().min(8)]));
    }
    if pdu[1] == S7_MSG_TYPE_ACK_DATA {
        // The PLC refused the whole function (for example 0x8104, not implemented).
        return match split_ack(pdu) {
            Err(e) => Err(e),
            Ok(_) => Err("the PLC answered a user data request with a plain ACK_DATA".into()),
        };
    }
    if pdu[1] != S7_MSG_TYPE_USER_DATA {
        return Err(format!("expected a user data response, got message type 0x{:02X}", pdu[1]));
    }
    let param_len = u16::from_be_bytes([pdu[6], pdu[7]]) as usize;
    let data_len = u16::from_be_bytes([pdu[8], pdu[9]]) as usize;
    if pdu.len() < HEADER_LEN + param_len + data_len {
        return Err(format!(
            "response is {} bytes but its header says {}",
            pdu.len(),
            HEADER_LEN + param_len + data_len
        ));
    }
    let params = &pdu[HEADER_LEN..HEADER_LEN + param_len];
    let data = &pdu[HEADER_LEN + param_len..HEADER_LEN + param_len + data_len];
    if params.len() < UD_LONG_PARAMS_LEN || params[..3] != [0x00, 0x01, 0x12] {
        return Err(format!("unexpected user data parameters {params:02X?}"));
    }
    if params[5] != TYPE_RESPONSE | group || params[6] != subfunction {
        return Err(format!(
            "the answer is for function group/subfunction {:02X}/{:02X}, not {:02X}/{subfunction:02X}",
            params[5],
            params[6],
            TYPE_RESPONSE | group
        ));
    }
    let error = u16::from_be_bytes([params[10], params[11]]);
    if error != S7_ERR_NONE {
        return Err(describe_error(error));
    }
    if data.len() < UD_DATA_HEADER_LEN {
        return Err(format!("data part too short ({} bytes)", data.len()));
    }
    if data[0] != RETURN_OK {
        return Err(describe_return_code(data[0]));
    }
    let length = u16::from_be_bytes([data[2], data[3]]) as usize;
    let Some(payload) = data.get(UD_DATA_HEADER_LEN..UD_DATA_HEADER_LEN + length) else {
        return Err(format!("data part is {} bytes but its length field says {length}", data.len() - UD_DATA_HEADER_LEN));
    };
    Ok(Response { seq: params[7], more: params[9] != 0, payload })
}

/// What a PLC puts in the parameters of one part of a user data response.
pub(crate) struct Unit {
    pub seq: u8,
    pub data_unit_ref: u8,
    pub more: bool,
}

/// Build a successful user data response carrying `payload`.
pub(crate) fn build_response(pdu_ref: u16, group: u8, subfunction: u8, unit: &Unit, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![RETURN_OK, TS_OCTET];
    data.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    data.extend_from_slice(payload);
    response_pdu(pdu_ref, group, subfunction, unit, S7_ERR_NONE, &data)
}

/// Build a user data response that refuses the request with a parameter error code and return
/// code 0x0A, as the PLC in the captures does.
pub(crate) fn build_error_response(pdu_ref: u16, group: u8, subfunction: u8, seq: u8, error: u16) -> Vec<u8> {
    let unit = Unit { seq, data_unit_ref: 0, more: false };
    response_pdu(pdu_ref, group, subfunction, &unit, error, &[RETURN_NOT_FOUND, 0x00, 0x00, 0x00])
}

fn response_pdu(pdu_ref: u16, group: u8, subfunction: u8, unit: &Unit, error: u16, data: &[u8]) -> Vec<u8> {
    let mut params = vec![0x00, 0x01, 0x12, 0x08, 0x12, TYPE_RESPONSE | group, subfunction, unit.seq];
    params.push(unit.data_unit_ref);
    params.push(u8::from(unit.more));
    params.extend_from_slice(&error.to_be_bytes());
    let mut s7 = job_pdu(pdu_ref, &params, data);
    s7[1] = S7_MSG_TYPE_USER_DATA;
    s7
}

// ── Error text ──

/// Describe a 16-bit error code of a user data or block function: the error class and code of
/// an ACK_DATA header, or the error code in user data parameters.
///
/// Meanings are from Wireshark's S7comm dissector (`param_errcode_names`) and snap7.
pub(crate) fn describe_error(code: u16) -> String {
    let text = match code {
        0x0110 => "invalid block number",
        ERR_INVALID_BLOCK_TYPE => "invalid block type",
        0x0114 => "block not found",
        ERR_BLOCK_PROTECTED => "the block is write-protected",
        0x8001 => "the service cannot be performed while the block is in its current state",
        0x8304 => "no further parallel upload possible (resource bottleneck)",
        0x8305 => "function not available",
        ERR_INVALID_SEQUENCE => "invalid service sequence",
        0x8702 => "the requested service is not supported by the module",
        0x8703 => "access to the object was refused",
        ERR_BLOCK_NOT_FOUND => "the block does not exist on the PLC",
        ERR_INFO_UNAVAILABLE => "information function unavailable (the PLC does not have this SZL)",
        _ => return describe_header_error(code),
    };
    format!("{text} (0x{code:04X})")
}

/// Describe the return code of the data part of a user data response.
fn describe_return_code(code: u8) -> String {
    match code {
        RETURN_NOT_FOUND => "the PLC has no such object (return code 0x0A)".into(),
        _ => format!("return code 0x{code:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::*;

    /// snap7 `opReadSZL`, first request: header `32 07 0000 <seq> 0008 0008`, params
    /// `Head 00 01 12, Plen 04, Uk 11, Tg grSZL (44), SubFun 01, Seq 00`.
    #[test]
    fn first_request_is_the_snap7_template() {
        let data = hex("ff 09 00 04 00 11 00 00");
        let pdu = build_request(0x0102, GROUP_SZL, 0x01, 0, false, &data);
        assert_eq!(pdu, hex("32 07 00 00 01 02 00 08 00 08  00 01 12 04 11 44 01 00  ff 09 00 04 00 11 00 00"));
    }

    /// snap7 `opReadSZL`, next request: params `Plen 08, Uk 12, Tg, SubFun, Seq, Rsvd 0000,
    /// ErrNo 0000` (12 bytes), data `Ret 0A, TS 00, DLen 0000`.
    #[test]
    fn follow_up_request_is_the_snap7_template() {
        let pdu = build_request(7, GROUP_SZL, 0x01, 1, true, &hex("0a 00 00 00"));
        assert_eq!(pdu, hex("32 07 00 00 00 07 00 0c 00 04  00 01 12 08 12 44 01 01 00 00 00 00  0a 00 00 00"));
    }

    #[test]
    fn requests_parse_back() {
        let pdu = build_request(9, GROUP_BLOCKS, 0x02, 3, true, &hex("0a 00 00 00"));
        let params = &pdu[10..22];
        let request = parse_request(params, &pdu[22..]).unwrap();
        assert_eq!((request.group, request.subfunction, request.seq, request.follow_up), (GROUP_BLOCKS, 2, 3, true));
        assert!(parse_request(&[0, 1, 0x12], &[]).is_none());
        // A response is not a request.
        assert!(parse_request(&hex("00 01 12 08 12 84 01 01 00 00 00 00"), &[]).is_none());
    }

    /// Capture packet `STATUS_39`: the answer to an SZL read, ending in the last data unit.
    #[test]
    fn response_with_data_parses() {
        let pdu = hex(STATUS_39);
        let response = parse_response(&pdu, GROUP_SZL, 0x01).unwrap();
        assert_eq!(response.seq, 1);
        assert!(!response.more);
        assert_eq!(response.payload.len(), 0x24);
        assert_eq!(response.payload[..4], hex("01 11 00 01"));
    }

    /// `STATUS_17` has the last data unit byte 0x01 (more follows) and the data unit reference
    /// 0x05, `STATUS_20` has 0x00.
    #[test]
    fn last_data_unit_flag_says_whether_more_follows() {
        let packet = hex(STATUS_17);
        let first = parse_response(&packet, GROUP_SZL, 0x01).unwrap();
        assert!(first.more);
        assert_eq!(first.seq, 1);
        assert!(!parse_response(&hex(STATUS_20), GROUP_SZL, 0x01).unwrap().more);
    }

    /// `STATUS_41` asks for SZL 0x0C91, which the CPU does not have; `STATUS_42` is the refusal.
    #[test]
    fn error_response_from_the_capture() {
        let pdu = hex(STATUS_42);
        let error = parse_response(&pdu, GROUP_SZL, 0x01).err().unwrap();
        assert!(error.contains("0xD402") && error.contains("SZL"), "{error}");
        // The simulator's refusal is these very bytes.
        assert_eq!(build_error_response(pdu_ref(&pdu), GROUP_SZL, 0x01, 1, ERR_INFO_UNAVAILABLE), pdu);
    }

    /// `BLOCKS_43` asks for the info of SDB 32768, which does not exist; `BLOCKS_44` is the answer.
    #[test]
    fn block_not_found_response_from_the_capture() {
        let pdu = hex(BLOCKS_44);
        let request = hex(BLOCKS_43);
        let parsed = parse_request(&request[10..18], &request[18..]).unwrap();
        assert_eq!((parsed.group, parsed.subfunction, parsed.follow_up), (GROUP_BLOCKS, 0x03, false));
        assert_eq!(pdu_ref(&request), pdu_ref(&pdu));
        let error = parse_response(&pdu, GROUP_BLOCKS, 0x03).err().unwrap();
        assert!(error.contains("0xD209") && error.contains("does not exist"), "{error}");
        assert_eq!(build_error_response(pdu_ref(&pdu), GROUP_BLOCKS, 0x03, 0, ERR_BLOCK_NOT_FOUND), pdu);
    }

    #[test]
    fn a_response_for_another_function_is_rejected() {
        let error = parse_response(&hex(BLOCKS_59), GROUP_SZL, 0x01).err().unwrap();
        assert!(error.contains("83/01"), "{error}");
        let error = parse_response(&hex(BLOCKS_59), GROUP_BLOCKS, 0x02).err().unwrap();
        assert!(error.contains("83/01"), "{error}");
    }

    #[test]
    fn a_refused_function_is_reported_from_the_ack_header() {
        // ACK_DATA with error class/code 0x8104: function not implemented.
        let pdu = hex("32 03 00 00 00 03 00 00 00 00 81 04");
        let error = parse_response(&pdu, GROUP_SZL, 0x01).err().unwrap();
        assert!(error.contains("0x8104"), "{error}");
    }

    #[test]
    fn truncated_responses_are_errors() {
        let pdu = hex(STATUS_39);
        assert!(parse_response(&pdu[..28], GROUP_SZL, 0x01).is_err());
        assert!(parse_response(&pdu[..8], GROUP_SZL, 0x01).is_err());
        // The header claims more than the PDU holds.
        assert!(parse_response(&pdu[..pdu.len() - 1], GROUP_SZL, 0x01).is_err());
        // The data length field says 2 bytes more than the data part holds.
        let mut pdu = pdu;
        pdu[25] += 2;
        assert!(parse_response(&pdu, GROUP_SZL, 0x01).is_err());
    }

    #[test]
    fn built_responses_parse_back() {
        let unit = Unit { seq: 1, data_unit_ref: 5, more: true };
        let pdu = build_response(4, GROUP_BLOCKS, 0x02, &unit, &[1, 2, 3, 4]);
        assert_eq!(pdu, hex("32 07 00 00 00 04 00 0c 00 08  00 01 12 08 12 83 02 01 05 01 00 00  ff 09 00 04 01 02 03 04"));
        let response = parse_response(&pdu, GROUP_BLOCKS, 0x02).unwrap();
        assert!(response.more);
        assert_eq!(response.payload, [1, 2, 3, 4]);
    }

    /// `DOWNLOAD_59`: the PLC's answer to a PI service, an ACK_DATA with one parameter byte.
    #[test]
    fn ack_pdu_layout() {
        let pdu = hex(DOWNLOAD_59);
        assert_eq!(ack_pdu(pdu_ref(&pdu), 0, &[0x28], &[]), pdu);
        let (params, data) = split_ack(&pdu).unwrap();
        assert_eq!(params, [0x28]);
        assert!(data.is_empty());
        assert!(split_ack(&ack_pdu(1, 0xD241, &[], &[])).unwrap_err().contains("password"));
    }
}
