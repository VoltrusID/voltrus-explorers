//! S7 variable access: Read Var (0x04) and Write Var (0x05) telegrams.
//!
//! Pure functions that build requests and parse responses, so they can be checked byte for byte
//! against the S7comm wire format without a socket. The client and the simulator server both use
//! the transport-size tables here.
//!
//! Wire format, as implemented by snap7 (`opReadArea`, `opWriteArea`, `opReadMultiVars` in
//! `src/core/s7_micro_client.cpp`) and shown by Wireshark's S7comm dissector:
//!
//! * Request item (12 bytes): `12 0A 10`, transport size, element count (2), DB number (2),
//!   area, address (3). The address is a bit offset (byte offset * 8 + bit), except for timers and
//!   counters, where it is the number and the transport size is 0x1D / 0x1C.
//! * Response item: return code (0xFF = success), transport size, length (2), data, and one pad
//!   byte when the data length is odd and another item follows. The length is in **bits** for
//!   transport sizes 0x03 (BIT), 0x04 (BYTE/WORD/DWORD) and 0x05 (INTEGER), and in **bytes** for
//!   0x06 (DINTEGER), 0x07 (REAL) and 0x09 (OCTET STRING).
//! * A bit read returns the addressed bit as 0 or 1 in the least significant bit of one byte.

use std::ops::Range;

use mt_protocol::ProtocolError;

use crate::types::*;

/// Most items one request may carry. S7-300/400 CPUs accept 20 (`MaxVars` in snap7); S7-1200/1500 allow more.
pub(crate) const MAX_ITEMS_PER_REQUEST: usize = 20;

/// S7 header length of a job (request) PDU.
const JOB_HEADER_LEN: usize = 10;
/// S7 header length of an ACK_DATA (response) PDU: the job header plus error class and code.
const ACK_HEADER_LEN: usize = 12;
/// Function code and item count that start the parameters of Read Var and Write Var.
const PARAMS_HEADER_LEN: usize = 2;
/// Length of one item specification in a request.
const ITEM_SPEC_LEN: usize = 12;
/// Return code, transport size and length in front of each data item.
const DATA_ITEM_HEADER_LEN: usize = 4;

/// Request transport sizes that need an element size in bytes (see [`request_unit_size`]).
pub(crate) const TS_CHAR: u8 = 0x03;
pub(crate) const TS_INT: u8 = 0x05;
pub(crate) const TS_DINT: u8 = 0x07;

/// Data transport sizes used in responses and write data items.
pub(crate) const DTS_BIT: u8 = 0x03;
pub(crate) const DTS_BYTE: u8 = 0x04;
pub(crate) const DTS_INTEGER: u8 = 0x05;
pub(crate) const DTS_REAL: u8 = 0x07;
pub(crate) const DTS_OCTET_STRING: u8 = 0x09;

/// Item return code for success.
pub(crate) const RC_SUCCESS: u8 = 0xFF;
pub(crate) const RC_ADDRESS_OUT_OF_RANGE: u8 = 0x05;
pub(crate) const RC_TRANSPORT_SIZE_NOT_SUPPORTED: u8 = 0x06;
pub(crate) const RC_OBJECT_DOES_NOT_EXIST: u8 = 0x0A;

// ── Tag helpers ──

impl S7Tag {
    /// Bytes of data one read of this tag returns.
    pub(crate) fn data_len(&self) -> usize {
        match self.data_type {
            S7DataType::Bool => 1,
            _ => self.data_type.byte_size() as usize * self.length as usize,
        }
    }

    /// A readable address for error messages, such as `DB1.DBD0`, `MW10`, `I0.3` or `T5`.
    pub fn label(&self) -> String {
        let size = match self.data_type {
            S7DataType::Bool => 'X',
            S7DataType::Byte | S7DataType::String => 'B',
            S7DataType::Word => 'W',
            S7DataType::DWord | S7DataType::Real => 'D',
            S7DataType::Timer | S7DataType::Counter => ' ',
        };
        let bit = if self.data_type == S7DataType::Bool {
            format!(".{}", self.bit_offset)
        } else {
            String::new()
        };
        match self.area {
            S7Area::DB => format!("DB{}.DB{size}{}{bit}", self.db_number, self.offset),
            S7Area::Input | S7Area::Output | S7Area::Marker => {
                let letter = match self.area {
                    S7Area::Input => 'I',
                    S7Area::Output => 'Q',
                    _ => 'M',
                };
                if self.data_type == S7DataType::Bool {
                    format!("{letter}{}{bit}", self.offset)
                } else {
                    format!("{letter}{size}{}", self.offset)
                }
            }
            S7Area::Timer => format!("T{}", self.offset),
            S7Area::Counter => format!("C{}", self.offset),
        }
    }
}

/// Append the 12-byte S7-any item specification for `tag` to a Read/Write Var parameter block.
///
/// `elements` is the element count for the transport size (bytes for byte arrays, so a string
/// write passes its payload length). Timers and counters are addressed by number, not by bit
/// offset, and carry no DB number.
pub(crate) fn push_item_spec(params: &mut Vec<u8>, tag: &S7Tag, elements: u16) {
    params.push(0x12); // variable specification
    params.push(0x0A); // length of the address specification that follows
    params.push(0x10); // syntax ID: S7 any
    params.push(tag.data_type.transport_size());
    params.extend_from_slice(&elements.to_be_bytes());
    params.extend_from_slice(&tag.db_number.to_be_bytes());
    params.push(tag.area.code());
    let address = if matches!(tag.area, S7Area::Timer | S7Area::Counter) {
        tag.offset as u32
    } else {
        (tag.offset as u32) * 8 + (tag.bit_offset as u32)
    };
    params.push(((address >> 16) & 0xFF) as u8);
    params.push(((address >> 8) & 0xFF) as u8);
    params.push((address & 0xFF) as u8);
}

// ── Planning ──

/// Split a tag that is too large for one PDU into consecutive tags of at most `max_data` bytes.
/// Bit tags and tags that fit are returned as they are.
pub(crate) fn split_tag(tag: &S7Tag, max_data: usize) -> Vec<S7Tag> {
    let element = tag.data_type.byte_size() as usize;
    if tag.data_type == S7DataType::Bool || tag.data_len() <= max_data || max_data < element {
        return vec![tag.clone()];
    }
    let per_chunk = (max_data / element) as u16;
    let mut chunks = Vec::new();
    let mut done: u16 = 0;
    while done < tag.length {
        let elements = per_chunk.min(tag.length - done);
        let mut chunk = tag.clone();
        chunk.length = elements;
        // Timers and counters advance by number, everything else by byte.
        chunk.offset = tag.offset + if matches!(tag.area, S7Area::Timer | S7Area::Counter) {
            done
        } else {
            done * element as u16
        };
        chunks.push(chunk);
        done += elements;
    }
    chunks
}

/// Group consecutive items into requests so that neither the request nor its response is larger
/// than the negotiated PDU, and no request has more than [`MAX_ITEMS_PER_REQUEST`] items.
///
/// Request: 10 byte header, 2 byte parameter header and 12 bytes per item.
/// Response: 12 byte header, 2 byte parameter header and, per item, 4 bytes plus its data padded to
/// an even length (the last item is not padded).
pub(crate) fn plan_batches(items: &[S7Tag], pdu_size: u16) -> Vec<Range<usize>> {
    let pdu = pdu_size as usize;
    let mut batches = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let mut end = start;
        let mut response = ACK_HEADER_LEN + PARAMS_HEADER_LEN;
        let mut prev_len = 0usize;
        while end < items.len() && end - start < MAX_ITEMS_PER_REQUEST {
            let len = items[end].data_len();
            let request = JOB_HEADER_LEN + PARAMS_HEADER_LEN + ITEM_SPEC_LEN * (end - start + 1);
            let padded = if end > start { prev_len % 2 } else { 0 };
            let response_with = response + padded + DATA_ITEM_HEADER_LEN + len;
            // An item that cannot fit even alone still goes out alone; the PLC reports the error.
            if end > start && (request > pdu || response_with > pdu) {
                break;
            }
            response = response_with;
            prev_len = len;
            end += 1;
        }
        batches.push(start..end);
        start = end;
    }
    batches
}

// ── Read Var ──

/// Build a Read Var request for `tags`. A read job carries no data section.
pub(crate) fn build_read_var_request(pdu_ref: u16, tags: &[S7Tag]) -> Vec<u8> {
    let mut params = vec![S7_READ_VAR, tags.len() as u8];
    for tag in tags {
        let elements = if tag.data_type == S7DataType::Bool { 1 } else { tag.length };
        push_item_spec(&mut params, tag, elements);
    }
    job_pdu(pdu_ref, &params, &[])
}

/// Assemble a job PDU: S7 header, parameters, data.
fn job_pdu(pdu_ref: u16, params: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s7 = Vec::with_capacity(JOB_HEADER_LEN + params.len() + data.len());
    s7.push(S7_PROTOCOL_ID);
    s7.push(S7_MSG_TYPE_JOB);
    s7.extend_from_slice(&0x0000u16.to_be_bytes());
    s7.extend_from_slice(&pdu_ref.to_be_bytes());
    s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
    s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
    s7.extend_from_slice(params);
    s7.extend_from_slice(data);
    s7
}

/// Bytes of data a response or write data item carries, given its transport size and length field.
pub(crate) fn data_item_bytes(transport_size: u8, length: u16) -> usize {
    match transport_size {
        DTS_BIT | DTS_BYTE | DTS_INTEGER => (length as usize).div_ceil(8),
        _ => length as usize,
    }
}

/// Check the ACK_DATA header of a response and return the parameter and data sections.
fn split_ack<'a>(data: &'a [u8], what: &str) -> Result<(&'a [u8], &'a [u8]), String> {
    if data.len() < ACK_HEADER_LEN {
        return Err(format!("{what} response too short ({} bytes)", data.len()));
    }
    if data[0] != S7_PROTOCOL_ID {
        return Err(format!("{what} response has protocol ID 0x{:02X}", data[0]));
    }
    let error = u16::from_be_bytes([data[10], data[11]]);
    if error != S7_ERR_NONE {
        return Err(format!("{what} rejected by the PLC: {}", describe_header_error(error)));
    }
    let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let data_len = u16::from_be_bytes([data[8], data[9]]) as usize;
    let params_end = ACK_HEADER_LEN + param_len;
    let data_end = params_end + data_len;
    if data.len() < data_end {
        return Err(format!("{what} response is shorter than its header says"));
    }
    Ok((&data[ACK_HEADER_LEN..params_end], &data[params_end..data_end]))
}

/// Parse a Read Var response into the raw data bytes of each item, in request order.
///
/// Fails with a message naming every item the PLC refused (with its return code) or answered with
/// the wrong amount of data.
pub(crate) fn parse_read_var_response(data: &[u8], tags: &[S7Tag]) -> Result<Vec<Vec<u8>>, ProtocolError> {
    let fail = ProtocolError::ReadFailed;
    let (params, items) = split_ack(data, "Read Var").map_err(fail)?;
    if params.len() < PARAMS_HEADER_LEN || params[0] != S7_READ_VAR {
        return Err(fail("Read Var response has the wrong function code".into()));
    }
    if params[1] as usize != tags.len() {
        return Err(fail(format!(
            "Read Var response has {} items, expected {}",
            params[1],
            tags.len()
        )));
    }

    let mut out = Vec::with_capacity(tags.len());
    let mut problems = Vec::new();
    let mut pos = 0;
    for (i, tag) in tags.iter().enumerate() {
        if items.len() < pos + DATA_ITEM_HEADER_LEN {
            return Err(fail("Read Var response ends before its last item".into()));
        }
        let (code, ts) = (items[pos], items[pos + 1]);
        let len = u16::from_be_bytes([items[pos + 2], items[pos + 3]]);
        let size = data_item_bytes(ts, len);
        let body = items
            .get(pos + DATA_ITEM_HEADER_LEN..pos + DATA_ITEM_HEADER_LEN + size)
            .ok_or_else(|| fail("Read Var response ends inside an item".into()))?;
        if code != RC_SUCCESS {
            problems.push(format!("{}: {}", tag.label(), describe_return_code(code)));
            out.push(Vec::new());
        } else if size != tag.data_len() {
            problems.push(format!("{}: the PLC returned {size} bytes, expected {}", tag.label(), tag.data_len()));
            out.push(Vec::new());
        } else {
            out.push(body.to_vec());
        }
        pos += DATA_ITEM_HEADER_LEN + size;
        if size % 2 == 1 && i + 1 < tags.len() {
            pos += 1; // pad byte after an odd-sized item
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(fail(problems.join("; ")))
    }
}

/// Decode the data bytes of a read into a value.
pub(crate) fn decode_value(data: &[u8], tag: &S7Tag) -> S7Value {
    match tag.data_type {
        S7DataType::Bool => S7Value::Bool(data.first().is_some_and(|b| b & 1 != 0)),
        S7DataType::Byte => S7Value::Byte(data.first().copied().unwrap_or(0)),
        S7DataType::Word => S7Value::Word(
            data.get(..2).map(|d| u16::from_be_bytes([d[0], d[1]])).unwrap_or(0),
        ),
        S7DataType::DWord => S7Value::DWord(
            data.get(..4).map(|d| u32::from_be_bytes([d[0], d[1], d[2], d[3]])).unwrap_or(0),
        ),
        S7DataType::Real => S7Value::Real(
            data.get(..4)
                .map(|d| f32::from_bits(u32::from_be_bytes([d[0], d[1], d[2], d[3]])))
                .unwrap_or(0.0),
        ),
        S7DataType::Timer => S7Value::Timer(
            data.get(..2).map(|d| decode_timer_ms(u16::from_be_bytes([d[0], d[1]]))).unwrap_or(0),
        ),
        S7DataType::Counter => S7Value::Counter(
            data.get(..2).map(|d| decode_counter(u16::from_be_bytes([d[0], d[1]]))).unwrap_or(0),
        ),
        // S7 string: [max length, actual length, characters...]
        S7DataType::String => match data {
            [_, actual, rest @ ..] if rest.len() >= *actual as usize => {
                S7Value::String(String::from_utf8_lossy(&rest[..*actual as usize]).into_owned())
            }
            _ => S7Value::String(String::new()),
        },
    }
}

// ── Write Var ──

/// The bytes to write for `value` on `tag`, or why the value does not fit the tag.
pub(crate) fn write_payload(tag: &S7Tag, value: &S7Value) -> Result<Vec<u8>, String> {
    let bytes = match (tag.data_type, value) {
        (S7DataType::Bool, S7Value::Bool(v)) => vec![u8::from(*v)],
        (S7DataType::Byte, S7Value::Byte(v)) => vec![*v],
        (S7DataType::Word, S7Value::Word(v)) => v.to_be_bytes().to_vec(),
        (S7DataType::DWord, S7Value::DWord(v)) => v.to_be_bytes().to_vec(),
        (S7DataType::Real, S7Value::Real(v)) => v.to_be_bytes().to_vec(),
        (S7DataType::Timer, S7Value::Timer(ms)) => encode_timer_ms(*ms).to_be_bytes().to_vec(),
        (S7DataType::Counter, S7Value::Counter(n)) => encode_counter(*n).to_be_bytes().to_vec(),
        (S7DataType::String, S7Value::String(s)) => s.as_bytes().to_vec(),
        _ => {
            return Err(format!(
                "a {value:?} value cannot be written to {} ({:?})",
                tag.label(),
                tag.data_type
            ))
        }
    };
    Ok(bytes)
}

/// Build a Write Var request that writes `payload` to `tag`: one item, with the data item
/// `00, transport size, length, bytes`.
///
/// For a string, `tag` must be the byte-array view of the characters (see `S7Client::write`) and
/// `payload` the `[actual length, characters]` bytes.
pub(crate) fn build_write_var_request(pdu_ref: u16, tag: &S7Tag, payload: &[u8]) -> Vec<u8> {
    let element = tag.data_type.byte_size() as usize;
    let elements = if tag.data_type == S7DataType::Bool {
        1
    } else {
        (payload.len() / element) as u16
    };
    let mut params = vec![S7_WRITE_VAR, 0x01];
    push_item_spec(&mut params, tag, elements);

    // Data item transport size and length, as snap7's opWriteArea sends them.
    let (ts, length) = match tag.data_type {
        S7DataType::Bool => (DTS_BIT, payload.len() as u16),
        S7DataType::Real => (DTS_REAL, payload.len() as u16),
        S7DataType::Timer | S7DataType::Counter => (DTS_OCTET_STRING, payload.len() as u16),
        _ => (DTS_BYTE, (payload.len() * 8) as u16),
    };
    let mut data = vec![0x00, ts];
    data.extend_from_slice(&length.to_be_bytes());
    data.extend_from_slice(payload);
    job_pdu(pdu_ref, &params, &data)
}

/// Check the response to a one-item Write Var.
pub(crate) fn parse_write_var_response(data: &[u8], tag: &S7Tag) -> Result<(), ProtocolError> {
    let fail = ProtocolError::WriteFailed;
    let (params, codes) = split_ack(data, "Write Var").map_err(fail)?;
    if params.first() != Some(&S7_WRITE_VAR) {
        return Err(fail("Write Var response has the wrong function code".into()));
    }
    match codes.first() {
        Some(&RC_SUCCESS) => Ok(()),
        Some(&code) => Err(fail(format!("{}: {}", tag.label(), describe_return_code(code)))),
        None => Err(fail("Write Var response has no item result".into())),
    }
}

// ── Error descriptions ──

/// Describe the 16-bit error class and code of an ACK_DATA header (bytes 10-11).
pub(crate) fn describe_header_error(code: u16) -> String {
    let text = match code {
        0x8104 => "function not available in this PLC",
        0x8500 => "request or response does not fit the negotiated PDU size",
        0xD241 => "a password is needed",
        0xD602 => "the password is wrong",
        0xDC01 => "invalid value",
        _ => "",
    };
    if text.is_empty() {
        format!("0x{code:04X}")
    } else {
        format!("{text} (0x{code:04X})")
    }
}

/// Describe the return code the PLC gives one item of a read or write.
pub(crate) fn describe_return_code(code: u8) -> String {
    let text = match code {
        0x01 => "hardware fault",
        0x03 => "access not allowed (on S7-1200/1500 enable PUT/GET and turn off optimized block access for the DB)",
        RC_ADDRESS_OUT_OF_RANGE => "address out of range",
        RC_TRANSPORT_SIZE_NOT_SUPPORTED => "data type not supported",
        0x07 => "data type inconsistent with the item size",
        RC_OBJECT_DOES_NOT_EXIST => "object does not exist (is the DB there?)",
        _ => "",
    };
    if text.is_empty() {
        format!("return code 0x{code:02X}")
    } else {
        format!("{text} (0x{code:02X})")
    }
}

// ── Request item tables (used by the simulator server) ──

/// Size in bytes of one element for a request transport size, or `None` if it is not supported.
/// Bits count as one element of one byte.
pub(crate) fn request_unit_size(ts: u8) -> Option<usize> {
    match ts {
        TS_BIT | TS_BYTE | TS_CHAR => Some(1),
        TS_WORD | TS_INT | TS_TIMER | TS_COUNTER => Some(2),
        TS_DWORD | TS_DINT | TS_REAL => Some(4),
        _ => None,
    }
}

/// Transport size and length field a PLC puts in front of the `bytes` it returns for a request item.
pub(crate) fn response_item_header(request_ts: u8, bytes: usize) -> (u8, u16) {
    match request_ts {
        TS_BIT => (DTS_BIT, 1),
        TS_REAL => (DTS_REAL, bytes as u16),
        TS_TIMER | TS_COUNTER => (DTS_OCTET_STRING, bytes as u16),
        TS_INT | TS_DINT => (DTS_INTEGER, (bytes * 8) as u16),
        _ => (DTS_BYTE, (bytes * 8) as u16),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    fn tag(address: &str) -> S7Tag {
        S7Tag::parse_address(address).unwrap()
    }

    /// A byte-array tag like the ones libnodave reads in the capture below.
    fn bytes(address: &str, length: u16) -> S7Tag {
        let mut t = tag(address);
        t.data_type = S7DataType::Byte;
        t.length = length;
        t
    }

    // The packets below are the S7 PDUs (TPKT and COTP stripped) of the Wireshark sample capture
    // `s7comm_varservice_libnodavedemo.pcap` (https://wiki.wireshark.org/SampleCaptures, S7comm),
    // taken from the numbered packet. They are real PLC traffic, not made up.

    /// Packet 11: read 64 bytes of DB1 from address 0. The request has no data section.
    #[test]
    fn read_request_matches_capture_packet_11() {
        let expected = hex("32 01 00 00 00 00 00 0e 00 00 04 01 12 0a 10 02 00 40 00 01 84 00 00 00");
        assert_eq!(build_read_var_request(0, &[bytes("DB1.DBB0", 64)]), expected);
    }

    /// Packet 13: read 16 bytes of the marker area, PDU reference 1.
    #[test]
    fn read_request_for_markers_matches_capture_packet_13() {
        let expected = hex("32 01 00 00 00 01 00 0e 00 00 04 01 12 0a 10 02 00 10 00 00 83 00 00 00");
        assert_eq!(build_read_var_request(1, &[bytes("MB0", 16)]), expected);
    }

    /// Packet 12: the answer for 64 bytes. Return code FF, transport size 04, length 0x0200 = 512
    /// BITS = 64 bytes. This is the case the old code got wrong (it read the length as bytes).
    #[test]
    fn read_response_length_is_in_bits_capture_packet_12() {
        let mut response = hex("32 03 00 00 00 00 00 02 00 44 00 00 04 01 ff 04 02 00");
        response.extend(std::iter::repeat(0u8).take(64));
        let data = parse_read_var_response(&response, &[bytes("DB1.DBB0", 64)]).unwrap();
        assert_eq!(data[0].len(), 64);
    }

    /// Packets 14 and 24: 16 bytes of markers, length 0x0080 = 128 bits.
    #[test]
    fn read_response_for_markers_capture_packets_14_and_24() {
        let packet_14 = hex("32 03 00 00 00 01 00 02 00 14 00 00 04 01 ff 04 00 80 a9 10 00 00 00 00 01 01 00 00 00 00 00 00 00 00");
        let data = parse_read_var_response(&packet_14, &[bytes("MB0", 16)]).unwrap();
        assert_eq!(data[0], hex("a9 10 00 00 00 00 01 01 00 00 00 00 00 00 00 00"));
        assert_eq!(decode_value(&data[0], &tag("MD0")), S7Value::DWord(0xA910_0000));

        let packet_24 = hex("32 03 00 00 00 06 00 02 00 14 00 00 04 01 ff 04 00 80 a0 10 00 01 00 00 01 03 00 00 00 03 3f 8c cc cd");
        let data = parse_read_var_response(&packet_24, &[bytes("MB0", 16)]).unwrap();
        let mut real = tag("MD12");
        real.data_type = S7DataType::Real;
        assert_eq!(decode_value(&data[0][12..], &real), S7Value::Real(1.1));
    }

    /// Packets 15, 17 and 21: write 4 bytes to the marker area at byte 0, 4 and 12. The address is
    /// in bits, the data item is `00 04 <length in bits> <bytes>`.
    #[test]
    fn write_requests_match_capture_packets_15_17_21() {
        let cases = [
            (2, "MB0", "a9 10 00 01", "32 01 00 00 00 02 00 0e 00 08 05 01 12 0a 10 02 00 04 00 00 83 00 00 00 00 04 00 20 a9 10 00 01"),
            (3, "MB4", "00 00 01 03", "32 01 00 00 00 03 00 0e 00 08 05 01 12 0a 10 02 00 04 00 00 83 00 00 20 00 04 00 20 00 00 01 03"),
            (5, "MB12", "3f 8c cc cd", "32 01 00 00 00 05 00 0e 00 08 05 01 12 0a 10 02 00 04 00 00 83 00 00 60 00 04 00 20 3f 8c cc cd"),
        ];
        for (pdu_ref, address, payload, expected) in cases {
            let request = build_write_var_request(pdu_ref, &bytes(address, 4), &hex(payload));
            assert_eq!(request, hex(expected), "{address}");
        }
    }

    /// Packet 16: the write acknowledgement, one item with return code FF.
    #[test]
    fn write_response_capture_packet_16() {
        let response = hex("32 03 00 00 00 02 00 02 00 01 00 00 05 01 ff");
        parse_write_var_response(&response, &bytes("MB0", 4)).unwrap();

        let refused = hex("32 03 00 00 00 02 00 02 00 01 00 00 05 01 05");
        let error = parse_write_var_response(&refused, &tag("DB1.DBD0")).unwrap_err().to_string();
        assert!(error.contains("DB1.DBD0") && error.contains("address out of range"), "{error}");
    }

    // Layouts below come from snap7's structs (`TReqFunWriteDataItem`, `TResFunReadItem` in
    // `src/core/s7_types.h`) and its handling in `opWriteArea`, `opReadArea`, `opReadMultiVars`
    // (`src/core/s7_micro_client.cpp`): TS 0x03 / 0x04 / 0x05 lengths are bits, 0x07 / 0x09 bytes,
    // items are padded to an even length except the last. The PDUs are assembled by hand.

    #[test]
    fn write_data_item_headers_follow_snap7_op_write_area() {
        // Bit: transport size 03, length 1 (not 8). The item address carries the bit number.
        let request = build_write_var_request(1, &tag("DB1.DBX4.3"), &[1]);
        assert_eq!(request[10..], hex("05 01 12 0a 10 01 00 01 00 01 84 00 00 23 00 03 00 01 01")[..]);
        // Word: 04 with 16 bits. Real: 07 with 4 BYTES. Counter and timer words: 09 with bytes.
        let word = build_write_var_request(1, &tag("MW2"), &[0x12, 0x34]);
        assert_eq!(word[word.len() - 6..], hex("00 04 00 10 12 34")[..]);
        let mut real = tag("MD8");
        real.data_type = S7DataType::Real;
        let request = build_write_var_request(1, &real, &1.5f32.to_be_bytes());
        assert_eq!(request[request.len() - 8..], hex("00 07 00 04 3f c0 00 00")[..]);
    }

    #[test]
    fn read_response_handles_bits_padding_and_item_errors() {
        // Three items: a bit (1 byte, padded), a word, and a DB that does not exist.
        let tags = [tag("DB1.DBX0.5"), tag("DB1.DBW2"), tag("DB9.DBW0")];
        let items = hex("ff 03 00 01 01 00  ff 04 00 10 12 34  0a 00 00 00");
        let mut response = hex("32 03 00 00 00 07 00 02 00 00 00 00 04 03");
        let len = items.len() as u16;
        response[8..10].copy_from_slice(&len.to_be_bytes());
        response.extend(items);
        let error = parse_read_var_response(&response, &tags).unwrap_err().to_string();
        assert!(error.contains("DB9.DBW0") && error.contains("object does not exist") && error.contains("0x0A"), "{error}");

        // Without the failing item the values come back, and the pad byte after the bit is skipped.
        let items = hex("ff 03 00 01 01 00  ff 04 00 10 12 34");
        let mut response = hex("32 03 00 00 00 07 00 02 00 00 00 00 04 02");
        response[8..10].copy_from_slice(&(items.len() as u16).to_be_bytes());
        response.extend(items);
        let data = parse_read_var_response(&response, &tags[..2]).unwrap();
        assert_eq!(data[0], vec![1]);
        assert_eq!(data[1], vec![0x12, 0x34]);
    }

    #[test]
    fn read_response_real_and_octet_lengths_are_bytes() {
        // snap7 does not shift the length for TS 07 (REAL) and 09 (OCTET STRING).
        let mut real = tag("DB1.DBD0");
        real.data_type = S7DataType::Real;
        let items = hex("ff 07 00 04 40 49 0f db  ff 09 00 02 01 23");
        let mut response = hex("32 03 00 00 00 07 00 02 00 00 00 00 04 02");
        response[8..10].copy_from_slice(&(items.len() as u16).to_be_bytes());
        response.extend(items);
        let data = parse_read_var_response(&response, &[real.clone(), tag("T5")]).unwrap();
        assert_eq!(decode_value(&data[0], &real), S7Value::Real(std::f32::consts::PI));
        assert_eq!(decode_value(&data[1], &tag("T5")), S7Value::Timer(1230));
    }

    #[test]
    fn read_response_errors_are_reported_not_zeroed() {
        // Header error 0x8500 (PDU size) and a short item.
        let rejected = hex("32 03 00 00 00 07 00 00 00 00 85 00");
        let error = parse_read_var_response(&rejected, &[tag("MW0")]).unwrap_err().to_string();
        assert!(error.contains("PDU size") && error.contains("0x8500"), "{error}");

        let wrong_size = hex("32 03 00 00 00 07 00 02 00 06 00 00 04 01 ff 04 00 08 12 00");
        let error = parse_read_var_response(&wrong_size, &[tag("MW0")]).unwrap_err().to_string();
        assert!(error.contains("MW0") && error.contains("expected 2"), "{error}");

        let truncated = hex("32 03 00 00 00 07 00 02 00 08 00 00 04 01 ff 04 00 20 12");
        assert!(parse_read_var_response(&truncated, &[tag("MD0")]).is_err());
    }

    #[test]
    fn bit_reads_use_the_least_significant_bit() {
        // A PLC answers a bit read with the addressed bit alone, whatever the bit number.
        for bit in 0..8 {
            let t = tag(&format!("DB1.DBX0.{bit}"));
            assert_eq!(decode_value(&[1], &t), S7Value::Bool(true), "bit {bit}");
            assert_eq!(decode_value(&[0], &t), S7Value::Bool(false), "bit {bit}");
        }
    }

    #[test]
    fn strings_decode_from_length_prefixed_bytes() {
        let mut t = tag("DB1.DBB0");
        t.data_type = S7DataType::String;
        assert_eq!(decode_value(&[254, 3, b'a', b'b', b'c'], &t), S7Value::String("abc".into()));
        assert_eq!(decode_value(&[254, 9, b'a'], &t), S7Value::String(String::new()), "claims more than there is");
    }

    #[test]
    fn item_spec_encoding() {
        let mut buf = Vec::new();
        push_item_spec(&mut buf, &tag("T5"), 1);
        assert_eq!(buf, hex("12 0a 10 1d 00 01 00 00 1d 00 00 05"));
        buf.clear();
        push_item_spec(&mut buf, &tag("C300"), 1);
        assert_eq!(buf, hex("12 0a 10 1c 00 01 00 00 1c 00 01 2c"));
        buf.clear();
        push_item_spec(&mut buf, &tag("DB1.DBW10"), 1);
        assert_eq!(buf, hex("12 0a 10 04 00 01 00 01 84 00 00 50"));
        buf.clear();
        push_item_spec(&mut buf, &tag("Q0.1"), 1);
        assert_eq!(buf, hex("12 0a 10 01 00 01 00 00 82 00 00 01"));
    }

    #[test]
    fn decodes_timer_and_counter_words() {
        let timer = tag("T1");
        // time base 1 s (0b10), BCD 0x123 = 123
        assert_eq!(decode_value(&[0x21, 0x23], &timer), S7Value::Timer(123_000));
        // time base 10 ms (0b00), BCD 0x999
        assert_eq!(decode_value(&[0x09, 0x99], &timer), S7Value::Timer(9_990));
        // time base 10 s (0b11), BCD 0x001
        assert_eq!(decode_value(&[0x30, 0x01], &timer), S7Value::Timer(10_000));
        assert_eq!(decode_value(&[], &timer), S7Value::Timer(0));

        let counter = tag("C1");
        assert_eq!(decode_value(&[0x02, 0x47], &counter), S7Value::Counter(247));
        assert_eq!(decode_value(&[0x09, 0x99], &counter), S7Value::Counter(999));
    }

    #[test]
    fn write_payload_rejects_a_value_of_the_wrong_type() {
        let error = write_payload(&tag("MW0"), &S7Value::Real(1.0)).unwrap_err();
        assert!(error.contains("MW0"), "{error}");
        assert_eq!(write_payload(&tag("DB1.DBX0.6"), &S7Value::Bool(true)).unwrap(), vec![1]);
        assert_eq!(write_payload(&tag("MW0"), &S7Value::Word(0x0102)).unwrap(), vec![1, 2]);
    }

    #[test]
    fn labels() {
        assert_eq!(tag("DB1.DBD0").label(), "DB1.DBD0");
        assert_eq!(tag("db2.dbx4.3").label(), "DB2.DBX4.3");
        assert_eq!(tag("IW10").label(), "IW10");
        assert_eq!(tag("I0.3").label(), "I0.3");
        assert_eq!(tag("%QB5").label(), "QB5");
        assert_eq!(tag("T5").label(), "T5");
        assert_eq!(tag("C9").label(), "C9");
    }

    // ── PDU planning ──

    fn words(n: usize) -> Vec<S7Tag> {
        (0..n).map(|i| tag(&format!("MW{}", i * 2))).collect()
    }

    #[test]
    fn batches_respect_the_request_size_and_item_limit() {
        // PDU 240: 10 + 2 + 12 * n <= 240 allows 19 items.
        let batches = plan_batches(&words(25), 240);
        assert_eq!(batches, vec![0..19, 19..25]);
        // PDU 480 would allow 39, but a request carries at most 20 items.
        assert_eq!(plan_batches(&words(45), 480), vec![0..20, 20..40, 40..45]);
        assert_eq!(plan_batches(&[], 240), Vec::<Range<usize>>::new());
    }

    #[test]
    fn batches_respect_the_response_size() {
        // 4 byte items on a 240 byte PDU: response 14 + 8 * n <= 240 allows 28; the 12 byte
        // request items are the tighter limit at 19. With 40 byte items the response limits:
        // 14 + 44 * n <= 240 allows 5.
        let big: Vec<S7Tag> = (0..12).map(|i| bytes(&format!("DB1.DBB{}", i * 40), 40)).collect();
        let batches = plan_batches(&big, 240);
        assert_eq!(batches, vec![0..5, 5..10, 10..12]);
    }

    #[test]
    fn odd_sized_items_count_their_pad_byte() {
        // Two 31 byte items need 12 + 2 + (4 + 31) + 1 pad + (4 + 31) = 85 bytes in the response.
        let odd: Vec<S7Tag> = (0..5).map(|i| bytes(&format!("DB1.DBB{}", i * 31), 31)).collect();
        assert_eq!(plan_batches(&odd, 85), vec![0..2, 2..4, 4..5]);
        assert_eq!(plan_batches(&odd, 84), vec![0..1, 1..2, 2..3, 3..4, 4..5]);
    }

    #[test]
    fn an_item_larger_than_the_pdu_goes_out_alone() {
        let huge = [bytes("DB1.DBB0", 500), bytes("DB1.DBB500", 2)];
        assert_eq!(plan_batches(&huge, 240), vec![0..1, 1..2]);
    }

    #[test]
    fn large_tags_are_split_into_pdu_sized_pieces() {
        let pieces = split_tag(&bytes("DB1.DBB10", 500), 222);
        assert_eq!(pieces.iter().map(|p| (p.offset, p.length)).collect::<Vec<_>>(), vec![(10, 222), (232, 222), (454, 56)]);

        // Word arrays split on element boundaries.
        let mut words = tag("DB1.DBW0");
        words.length = 200;
        let pieces = split_tag(&words, 222);
        assert_eq!(pieces.iter().map(|p| (p.offset, p.length)).collect::<Vec<_>>(), vec![(0, 111), (222, 89)]);

        // Bits and tags that fit are untouched.
        assert_eq!(split_tag(&tag("DB1.DBX0.1"), 4).len(), 1);
        assert_eq!(split_tag(&tag("MW0"), 222).len(), 1);
    }

    #[test]
    fn request_tables() {
        assert_eq!(request_unit_size(TS_WORD), Some(2));
        assert_eq!(request_unit_size(TS_REAL), Some(4));
        assert_eq!(request_unit_size(TS_TIMER), Some(2));
        assert_eq!(request_unit_size(0x77), None);
        assert_eq!(response_item_header(TS_WORD, 2), (DTS_BYTE, 16));
        assert_eq!(response_item_header(TS_BIT, 1), (DTS_BIT, 1));
        assert_eq!(response_item_header(TS_REAL, 4), (DTS_REAL, 4));
        assert_eq!(response_item_header(TS_TIMER, 2), (DTS_OCTET_STRING, 2));
    }
}
