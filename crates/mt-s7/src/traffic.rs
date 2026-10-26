//! What each PDU on the wire is, in one line, for traffic monitors: the client reports the frames
//! it sends and receives, the simulator the frames it receives and answers.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::framing::describe_error;
use crate::types::*;
use crate::vars::{data_item_bytes, describe_return_code};

/// One frame sent or received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7Frame {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// `true` for frames this side sent.
    pub sent: bool,
    /// The function, e.g. `Read Var` or `Read SZL`.
    pub summary: String,
    /// What the function was about: addresses, SZL ids, item results, errors.
    pub detail: String,
    /// Size on the wire, TPKT and COTP headers included.
    pub bytes: usize,
}

/// Where a client or simulator sends its [`S7Frame`]s.
pub type TrafficSender = tokio::sync::mpsc::UnboundedSender<S7Frame>;

/// TPKT (4) and COTP DT (3) headers around every S7 PDU.
const WRAPPER_LEN: usize = 7;
/// Addresses listed in a detail before the rest is counted.
const SHOWN_ITEMS: usize = 4;

pub(crate) fn emit(traffic: Option<&TrafficSender>, sent: bool, summary: String, detail: String, bytes: usize) {
    if let Some(traffic) = traffic {
        let _ = traffic.send(S7Frame { at_ms: now_ms(), sent, summary, detail, bytes });
    }
}

/// Report one S7 PDU (without its TPKT and COTP headers).
pub(crate) fn emit_pdu(traffic: Option<&TrafficSender>, sent: bool, s7: &[u8]) {
    if traffic.is_some() {
        let (summary, detail) = describe_pdu(s7);
        emit(traffic, sent, summary, detail, s7.len() + WRAPPER_LEN);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

/// The function of an S7 PDU and what it carries, as a protocol analyser shows them.
pub fn describe_pdu(s7: &[u8]) -> (String, String) {
    if s7.len() < 10 || s7[0] != S7_PROTOCOL_ID {
        return ("S7".into(), format!("{} bytes, not an S7 PDU", s7.len()));
    }
    let msg_type = s7[1];
    let header_len = if matches!(msg_type, 0x02 | 0x03) { 12 } else { 10 };
    let param_len = be16(s7, 6).unwrap_or(0) as usize;
    let data_len = be16(s7, 8).unwrap_or(0) as usize;
    let params = s7.get(header_len..header_len + param_len).unwrap_or(&[]);
    let data_end = (header_len + param_len + data_len).min(s7.len());
    let data = s7.get(header_len + param_len..data_end).unwrap_or(&[]);
    match msg_type {
        S7_MSG_TYPE_JOB => job(params),
        0x02 | 0x03 => {
            let error = be16(s7, 10).unwrap_or(0);
            ack(params, data, error)
        }
        S7_MSG_TYPE_USER_DATA => user_data(params, data),
        other => (format!("Message type 0x{other:02X}"), String::new()),
    }
}

fn function_name(code: u8) -> String {
    match code {
        S7_SETUP_COMM => "Setup Communication".into(),
        S7_READ_VAR => "Read Var".into(),
        S7_WRITE_VAR => "Write Var".into(),
        S7_START_UPLOAD => "Start Upload".into(),
        S7_UPLOAD => "Upload".into(),
        S7_END_UPLOAD => "End Upload".into(),
        S7_PLC_CONTROL => "PLC Control".into(),
        0x29 => "PLC Stop".into(),
        0x1A => "Request Download".into(),
        0x1B => "Download Block".into(),
        0x1C => "Download Ended".into(),
        other => format!("Function 0x{other:02X}"),
    }
}

fn job(params: &[u8]) -> (String, String) {
    let Some(&function) = params.first() else {
        return ("Job".into(), "no parameters".into());
    };
    let detail = match function {
        S7_SETUP_COMM => be16(params, 6).map_or_else(String::new, |size| format!("PDU {size} bytes")),
        S7_READ_VAR | S7_WRITE_VAR => items(params),
        _ => String::new(),
    };
    (function_name(function), detail)
}

/// The addresses of a Read/Write Var request, e.g. `DB1.DBD0, M10.3, T5`.
fn items(params: &[u8]) -> String {
    let count = params.get(1).copied().unwrap_or(0) as usize;
    let labels: Vec<String> = (0..count)
        .filter_map(|i| params.get(2 + i * 12..2 + (i + 1) * 12))
        .take(SHOWN_ITEMS)
        .map(item_label)
        .collect();
    let mut text = labels.join(", ");
    if count > SHOWN_ITEMS {
        text.push_str(&format!(" … {count} items"));
    }
    text
}

/// One 12-byte S7-any item as an address, e.g. `DB1.DBB0 ×28`.
fn item_label(item: &[u8]) -> String {
    let ts = item[3];
    let count = u16::from_be_bytes([item[4], item[5]]);
    let db = u16::from_be_bytes([item[6], item[7]]);
    let address = (u32::from(item[9]) << 16) | (u32::from(item[10]) << 8) | u32::from(item[11]);
    let times = if count > 1 { format!(" ×{count}") } else { String::new() };
    let area = item[8];
    if area == AREA_TIMER || area == AREA_COUNTER {
        let letter = if area == AREA_TIMER { 'T' } else { 'C' };
        return format!("{letter}{address}{times}");
    }
    let (byte, bit) = (address / 8, address % 8);
    let size = match ts {
        TS_BIT => 'X',
        TS_WORD | 0x05 => 'W',
        TS_DWORD | TS_REAL | 0x07 => 'D',
        _ => 'B',
    };
    let bit = if ts == TS_BIT { format!(".{bit}") } else { String::new() };
    match area {
        AREA_DB => format!("DB{db}.DB{size}{byte}{bit}{times}"),
        AREA_INPUT | AREA_OUTPUT | AREA_MARKER => {
            let letter = match area {
                AREA_INPUT => 'I',
                AREA_OUTPUT => 'Q',
                _ => 'M',
            };
            if ts == TS_BIT {
                format!("{letter}{byte}{bit}")
            } else {
                format!("{letter}{size}{byte}{times}")
            }
        }
        other => format!("area 0x{other:02X} {byte}{bit}{times}"),
    }
}

fn ack(params: &[u8], data: &[u8], error: u16) -> (String, String) {
    let summary = params.first().map_or_else(|| "Acknowledge".to_owned(), |f| function_name(*f));
    if error != 0 {
        return (summary, describe_error(error));
    }
    let detail = match params.first().copied() {
        Some(S7_SETUP_COMM) => be16(params, 6).map_or_else(String::new, |size| format!("PDU {size} bytes")),
        Some(S7_READ_VAR) => read_results(params.get(1).copied().unwrap_or(0) as usize, data),
        Some(S7_WRITE_VAR) => results(data.iter().take(params.get(1).copied().unwrap_or(0) as usize).copied()),
        _ => String::new(),
    };
    (summary, detail)
}

/// The return codes of a Read Var answer's data items.
fn read_results(count: usize, data: &[u8]) -> String {
    let mut codes = Vec::with_capacity(count);
    let mut pos = 0;
    for i in 0..count {
        let Some(header) = data.get(pos..pos + 4) else { break };
        codes.push(header[0]);
        if header[0] != 0xFF {
            pos += 4;
            continue;
        }
        let size = data_item_bytes(header[1], u16::from_be_bytes([header[2], header[3]]));
        pos += 4 + size + usize::from(size % 2 == 1 && i + 1 < count);
    }
    results(codes.into_iter())
}

/// `3 OK` or `2 OK, item 3: …`.
fn results(codes: impl Iterator<Item = u8>) -> String {
    let mut ok = 0;
    let mut failed = Vec::new();
    for (i, code) in codes.enumerate() {
        if code == 0xFF {
            ok += 1;
        } else if failed.len() < 2 {
            failed.push(format!("item {}: {}", i + 1, describe_return_code(code)));
        }
    }
    let mut parts = Vec::new();
    if ok > 0 || failed.is_empty() {
        parts.push(format!("{ok} OK"));
    }
    parts.extend(failed);
    parts.join(", ")
}

/// SZL reads, block lists and the rest of the user data functions.
fn user_data(params: &[u8], data: &[u8]) -> (String, String) {
    if params.len() < 8 || params[..3] != [0x00, 0x01, 0x12] {
        return ("User Data".into(), String::new());
    }
    let response = params[5] & 0xF0 == 0x80;
    let group = params[5] & 0x0F;
    let subfunction = params[6];
    let summary = match (group, subfunction) {
        (0x04, 0x01) => "Read SZL".to_owned(),
        (0x03, 0x01) => "List Blocks".to_owned(),
        (0x03, 0x02) => "List Blocks of Type".to_owned(),
        (0x03, 0x03) => "Block Info".to_owned(),
        (0x07, 0x01) => "Read Clock".to_owned(),
        (0x07, 0x02) => "Set Clock".to_owned(),
        (group, subfunction) => format!("User Data {group}/{subfunction}"),
    };
    let follow_up = params[3] == 0x08 && !response;
    if follow_up {
        return (summary, "next part".into());
    }
    if response {
        if let Some(error) = be16(params, 10).filter(|e| *e != 0) {
            return (summary, describe_error(error));
        }
        if data.first().is_some_and(|rc| *rc != 0xFF) {
            return (summary, describe_return_code(data[0]));
        }
    }
    let payload = data.get(4..).unwrap_or(&[]);
    let more = response && params.get(9).is_some_and(|m| *m != 0);
    let mut detail = match (group, subfunction) {
        (0x04, 0x01) => match (be16(payload, 0), be16(payload, 2)) {
            (Some(id), Some(index)) if response => {
                let records = be16(payload, 6).unwrap_or(0);
                format!("SZL 0x{id:04X} index 0x{index:04X} · {records} records")
            }
            (Some(id), Some(index)) => format!("SZL 0x{id:04X} index 0x{index:04X}"),
            _ => String::new(),
        },
        (0x03, 0x02) if !response => payload
            .get(1)
            .and_then(|code| BlockType::from_code(*code))
            .map_or_else(String::new, |t| t.to_string()),
        (0x03, 0x01) if response => format!("{} types", payload.len() / 4),
        (0x03, 0x02) if response => format!("{} blocks", payload.len() / 4),
        _ => String::new(),
    };
    if more {
        detail.push_str(if detail.is_empty() { "more follows" } else { " · more follows" });
    }
    (summary, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn setup_communication_shows_the_pdu_size() {
        // Packets 9 and 10 of the libnodave capture (see client.rs).
        let job = hex("32 01 00 00 ff ff 00 08 00 00 f0 00 00 01 00 01 07 80");
        assert_eq!(describe_pdu(&job), ("Setup Communication".into(), "PDU 1920 bytes".into()));
        let ack = hex("32 03 00 00 ff ff 00 08 00 00 00 00 f0 00 00 01 00 01 00 f0");
        assert_eq!(describe_pdu(&ack), ("Setup Communication".into(), "PDU 240 bytes".into()));
    }

    #[test]
    fn read_var_lists_its_addresses_and_results() {
        let mut params = vec![S7_READ_VAR, 3];
        // DB1.DBD0 (real), M10.3 (bit), 28 bytes of DB1 from 30.
        params.extend(hex("12 0a 10 08 00 01 00 01 84 00 00 00"));
        params.extend(hex("12 0a 10 01 00 01 00 00 83 00 00 53"));
        params.extend(hex("12 0a 10 02 00 1c 00 01 84 00 00 f0"));
        let mut job = hex("32 01 00 00 00 05");
        job.extend((params.len() as u16).to_be_bytes());
        job.extend([0, 0]);
        job.extend(&params);
        assert_eq!(describe_pdu(&job).1, "DB1.DBD0, M10.3, DB1.DBB30 ×28");

        // Answer: a real, then a missing object (0x0A, header only), then one bit.
        let data = hex("ff 07 00 04 42 28 00 00 0a 00 00 00 ff 03 00 01 01");
        let mut ack = hex("32 03 00 00 00 05 00 02");
        ack.extend((data.len() as u16).to_be_bytes());
        ack.extend([0, 0, S7_READ_VAR, 3]);
        ack.extend(&data);
        let (summary, detail) = describe_pdu(&ack);
        assert_eq!(summary, "Read Var");
        assert!(detail.starts_with("2 OK, item 2: "), "{detail}");
    }

    #[test]
    fn refused_jobs_show_the_error_and_garbage_does_not_panic() {
        let refused = hex("32 03 00 00 00 07 00 00 00 00 81 04");
        let (_, detail) = describe_pdu(&refused);
        assert!(detail.contains("0x8104"), "{detail}");
        for bad in [&[][..], &[0x32][..], &hex("32 01 00 00 00 01 ff ff ff ff")[..], &hex("33 01 00 00 00 00 00 00 00 00")[..]] {
            let _ = describe_pdu(bad);
        }
        // A Read Var whose item count runs past its parameters lists what is there.
        let short = hex("32 01 00 00 00 01 00 0e 00 00 04 05 12 0a 10 02 00 01 00 00 83 00 00 00");
        assert_eq!(describe_pdu(&short).1, "MB0 … 5 items");
    }

    #[test]
    fn szl_reads_name_the_list() {
        let request = framing_request();
        assert_eq!(describe_pdu(&request), ("Read SZL".into(), "SZL 0x0011 index 0x0000".into()));
    }

    fn framing_request() -> Vec<u8> {
        crate::szl::build_first_request(9, 0x0011, 0)
    }
}
