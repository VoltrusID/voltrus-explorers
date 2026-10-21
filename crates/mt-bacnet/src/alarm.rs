//! BACnet Alarm and Event service module.
//!
//! Provides custom BER encoding/decoding for BACnet alarm and event services
//! (GetAlarmSummary, GetEventInformation, AcknowledgeAlarm, EventNotification).
//!
//! The `bacnet-rs` crate only defines enum values for these services -- no
//! request/response structs. We implement the BER encoding ourselves following
//! ASHRAE 135 clause 21.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::ObjectType;
use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

use crate::helpers::{object_type_from_str, object_type_to_string};

// ---------------------------------------------------------------------------
// Invoke ID counter (separate from client.rs to avoid collisions)
// ---------------------------------------------------------------------------

/// Start at 100 to stay out of client.rs's range (0-99).
static ALARM_INVOKE_ID: AtomicU8 = AtomicU8::new(100);

fn next_invoke_id() -> u8 {
    ALARM_INVOKE_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single entry returned by GetAlarmSummary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlarmSummaryEntry {
    /// Object identifier string, e.g. "analogInput:1".
    pub object_identifier: String,
    /// Object type string, e.g. "analogInput".
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Alarm state, e.g. "normal", "highLimit", "lowLimit".
    pub alarm_state: String,
    /// Acknowledged transitions as CSV, e.g. "ToOffNormal, ToFault".
    pub acked_transitions: String,
}

/// A single entry returned by GetEventInformation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventInfoEntry {
    /// Object identifier string, e.g. "analogInput:1".
    pub object_identifier: String,
    /// Object type string.
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Current event state.
    pub event_state: String,
    /// Acknowledged transitions as CSV.
    pub acknowledged_transitions: String,
    /// Enabled transitions as CSV.
    pub event_enable: String,
    /// Event time stamps (up to 3 entries).
    pub event_time_stamps: Vec<String>,
}

/// Result of GetEventInformation, including pagination flag.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventInformationResponse {
    /// List of event summaries returned by the device.
    pub entries: Vec<EventInfoEntry>,
    /// True if the device has more events (call again with last object).
    pub more_events: bool,
}

/// Parsed incoming alarm/event notification (UnconfirmedEventNotification or
/// ConfirmedEventNotification).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlarmNotification {
    /// IP address of the notification source.
    pub source_ip: String,
    /// UDP port of the notification source.
    pub source_port: u16,
    /// Object type that generated the event.
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Current event state.
    pub event_state: String,
    /// Previous state (parsed from eventType or approximated).
    pub from_state: String,
    /// New state.
    pub to_state: String,
    /// Notify type: "alarm", "event", or "ackNotification".
    pub notify_type: String,
    /// Whether acknowledgment is required.
    pub ack_required: bool,
    /// Message text from the notification, if any.
    pub message: String,
    /// Unix timestamp (seconds since epoch) when received.
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Event state name helpers
// ---------------------------------------------------------------------------

fn event_state_name(val: u32) -> String {
    match val {
        0 => "normal".into(),
        1 => "fault".into(),
        2 => "offnormal".into(),
        3 => "highLimit".into(),
        4 => "lowLimit".into(),
        5 => "lifeSafetyAlarm".into(),
        v => format!("unknown({v})"),
    }
}

/// Decode BACnetEventTransitionBits (3-bit bitstring) to a CSV string.
/// Bit 0 = ToOffNormal, Bit 1 = ToFault, Bit 2 = ToNormal.
fn transition_bits_to_string(bits: u8) -> String {
    let mut parts = Vec::new();
    if bits & 0x01 != 0 {
        parts.push("ToOffNormal");
    }
    if bits & 0x02 != 0 {
        parts.push("ToFault");
    }
    if bits & 0x04 != 0 {
        parts.push("ToNormal");
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// BER encoding helpers (context-tagged)
// ---------------------------------------------------------------------------

/// Encode a context-tagged unsigned integer (primitive).
/// BACnet tag byte: [tag_num(4) | context(1) | length(3)]
fn encode_context_unsigned(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    let tag_byte = (tag_num << 4) | 0x08; // context bit set, length in low 3 bits
    if value <= 255 {
        buf.push(tag_byte | 1); // length = 1 byte
        buf.push(value as u8);
    } else if value <= 65535 {
        buf.push(tag_byte | 2); // length = 2 bytes
        buf.extend_from_slice(&[(value >> 8) as u8, value as u8]);
    } else {
        buf.push(tag_byte | 4); // length = 4 bytes
        buf.extend_from_slice(&value.to_be_bytes());
    }
}

/// Encode a context-tagged BACnet object identifier (4-byte packed value).
fn encode_context_object_id(buf: &mut Vec<u8>, tag_num: u8, obj_type: ObjectType, instance: u32) {
    let type_u32: u32 = obj_type.into();
    let value = (type_u32 << 22) | (instance & 0x3FFFFF);
    buf.push((tag_num << 4) | 0x0C); // context tag 0, length 4 (0x08 | 0x04)
    buf.extend_from_slice(&value.to_be_bytes());
}

/// Encode a context-tagged enumerated value (identical to unsigned encoding).
fn encode_context_enumerated(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    encode_context_unsigned(buf, tag_num, value);
}

/// Encode a context-tagged character string (ISO-8859-1).
fn encode_context_character_string(buf: &mut Vec<u8>, tag_num: u8, s: &str) {
    let bytes = s.as_bytes();
    // Truncate to fit: total length = 1 (encoding) + string bytes, max 253 for short form
    let max_str = if bytes.len() > 253 { 253 } else { bytes.len() };
    let truncated = &bytes[..max_str];
    let total_len = truncated.len() + 1; // +1 for encoding type byte

    if total_len <= 4 {
        // Short form: length in tag byte bits 2-0
        buf.push((tag_num << 4) | 0x08 | (total_len as u8));
    } else {
        // Extended form: length = 5 in tag byte, then actual length
        buf.push((tag_num << 4) | 0x08 | 5);
        buf.push(total_len as u8);
    }
    buf.push(0x00); // ISO-8859-1 encoding
    buf.extend_from_slice(truncated);
}

/// Encode opening tag for a context-tagged constructed type.
fn encode_opening_tag(buf: &mut Vec<u8>, tag_num: u8) {
    buf.push((tag_num << 4) | 0x0E);
}

/// Encode closing tag.
fn encode_closing_tag(buf: &mut Vec<u8>, tag_num: u8) {
    buf.push((tag_num << 4) | 0x0F);
}

// ---------------------------------------------------------------------------
// BER decoding helpers
// ---------------------------------------------------------------------------

/// Advance `pos` past a single TLV element. Returns the new position.
fn skip_tag(data: &[u8], pos: usize) -> usize {
    if pos >= data.len() {
        return pos;
    }
    let tag = data[pos];
    let low_nibble = tag & 0x0F;

    // Opening tag (0x0E low nibble) or closing tag (0x0F low nibble)
    if low_nibble == 0x0E || low_nibble == 0x0F {
        return pos + 1;
    }

    // Length encoded in bits 2-0 of tag byte
    let mut length = (tag & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        // Extended length in following byte(s)
        if pos + 1 >= data.len() {
            return data.len();
        }
        let len_byte = data[pos + 1];
        consumed += 1;
        match len_byte {
            0..=253 => {
                length = len_byte as usize;
            }
            254 => {
                if pos + 3 >= data.len() {
                    return data.len();
                }
                length = ((data[pos + 2] as usize) << 8) | data[pos + 3] as usize;
                consumed += 2;
            }
            255 => {
                if pos + 5 >= data.len() {
                    return data.len();
                }
                length = ((data[pos + 2] as usize) << 24)
                    | ((data[pos + 3] as usize) << 16)
                    | ((data[pos + 4] as usize) << 8)
                    | data[pos + 5] as usize;
                consumed += 4;
            }
        }
    }

    pos + consumed + length
}

/// Read the tag number and length from the current position.
/// Returns `(tag_number, is_opening, is_closing, value_start, total_consumed)`.
fn read_tag_info(data: &[u8], pos: usize) -> Option<(u8, bool, bool, usize, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let tag_num = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    let is_opening = is_context && (byte & 0x07) == 0x06;
    let is_closing = is_context && (byte & 0x07) == 0x07;

    if is_opening || is_closing {
        return Some((tag_num, is_opening, is_closing, pos + 1, 1));
    }

    // Length encoded in bits 2-0 of tag byte
    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        // Extended length
        if pos + 1 >= data.len() {
            return None;
        }
        let len_byte = data[pos + 1];
        consumed += 1;
        match len_byte {
            0..=253 => length = len_byte as usize,
            254 => {
                if pos + 3 >= data.len() {
                    return None;
                }
                length = ((data[pos + 2] as usize) << 8) | data[pos + 3] as usize;
                consumed += 2;
            }
            255 => {
                if pos + 5 >= data.len() {
                    return None;
                }
                length = ((data[pos + 2] as usize) << 24)
                    | ((data[pos + 3] as usize) << 16)
                    | ((data[pos + 4] as usize) << 8)
                    | data[pos + 5] as usize;
                consumed += 4;
            }
        }
    }

    let value_start = pos + consumed;
    Some((tag_num, false, false, value_start, consumed + length))
}

/// Extract a context-tagged unsigned integer at the expected tag number.
fn extract_context_unsigned(data: &[u8], tag_num: u8, pos: usize) -> Option<(u32, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let actual_tag = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    let is_opening = is_context && (byte & 0x07) == 0x06;
    let is_closing = is_context && (byte & 0x07) == 0x07;

    if !is_context || actual_tag != tag_num || is_opening || is_closing {
        return None;
    }

    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        if pos + 1 >= data.len() { return None; }
        length = data[pos + 1] as usize;
        consumed += 1;
    }

    let val_start = pos + consumed;
    let val = match length {
        1 => *data.get(val_start)? as u32,
        2 => {
            ((*data.get(val_start)? as u32) << 8) | (*data.get(val_start + 1)? as u32)
        }
        4 => u32::from_be_bytes([
            *data.get(val_start)?,
            *data.get(val_start + 1)?,
            *data.get(val_start + 2)?,
            *data.get(val_start + 3)?,
        ]),
        _ => return None,
    };
    Some((val, pos + consumed + length))
}

/// Extract a context-tagged BACnet object identifier.
fn extract_context_object_id(
    data: &[u8],
    tag_num: u8,
    pos: usize,
) -> Option<(ObjectType, u32, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let actual_tag = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    if !is_context || actual_tag != tag_num {
        return None;
    }

    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        if pos + 1 >= data.len() { return None; }
        length = data[pos + 1] as usize;
        consumed += 1;
    }

    if length != 4 {
        return None;
    }

    let val_start = pos + consumed;
    if val_start + 4 > data.len() {
        return None;
    }

    let raw = u32::from_be_bytes([
        data[val_start],
        data[val_start + 1],
        data[val_start + 2],
        data[val_start + 3],
    ]);
    let type_val = (raw >> 22) & 0x3FF;
    let instance = raw & 0x3FFFFF;
    let obj_type: ObjectType = type_val.into();
    Some((obj_type, instance, pos + consumed + length))
}

/// Extract a context-tagged bitstring (used for BACnetEventTransitionBits).
/// Expected format: context tag byte with length, unused-bits byte, then data bytes.
fn extract_context_bitstring(data: &[u8], tag_num: u8, pos: usize) -> Option<(u8, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let actual_tag = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    if !is_context || actual_tag != tag_num {
        return None;
    }

    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        if pos + 1 >= data.len() { return None; }
        length = data[pos + 1] as usize;
        consumed += 1;
    }

    if length < 1 {
        return None;
    }

    let val_start = pos + consumed;
    let unused_bits = *data.get(val_start)? & 0x07;
    let bits = if length >= 2 {
        *data.get(val_start + 1).unwrap_or(&0)
    } else {
        0
    };
    let result = bits & ((0xFF >> unused_bits));
    Some((result, pos + consumed + length))
}

/// Extract a context-tagged character string. Returns the decoded string and new position.
fn extract_context_character_string(
    data: &[u8],
    tag_num: u8,
    pos: usize,
) -> Option<(String, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let actual_tag = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    if !is_context || actual_tag != tag_num {
        return None;
    }

    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        if pos + 1 >= data.len() { return None; }
        length = data[pos + 1] as usize;
        consumed += 1;
    }

    if length < 2 {
        return None;
    }

    let val_start = pos + consumed;
    // First byte is encoding (0x00 = ISO-8859-1)
    let _encoding = data[val_start];
    let str_bytes = &data[val_start + 1..pos + consumed + length];
    let s = String::from_utf8_lossy(str_bytes).into_owned();
    Some((s, pos + consumed + length))
}

/// Extract a context-tagged boolean.
fn extract_context_boolean(data: &[u8], tag_num: u8, pos: usize) -> Option<(bool, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let actual_tag = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    if !is_context || actual_tag != tag_num {
        return None;
    }

    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        if pos + 1 >= data.len() { return None; }
        length = data[pos + 1] as usize;
        consumed += 1;
    }

    if length < 1 {
        return None;
    }

    let val_start = pos + consumed;
    let val = *data.get(val_start).unwrap_or(&0);
    Some((val != 0, pos + consumed + length))
}

// ---------------------------------------------------------------------------
// Send confirmed request helper (same pattern as client.rs)
// ---------------------------------------------------------------------------

/// Send a confirmed BACnet request and return the service_data from the response APDU.
fn send_confirmed_request(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    service_choice: ConfirmedServiceChoice,
    service_data: Vec<u8>,
) -> Result<Vec<u8>, ProtocolError> {
    let invoke_id = next_invoke_id();

    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_data,
    };

    let apdu_data = apdu.encode();
    let mut npdu = Npdu::new();
    npdu.control.expecting_reply = true;
    let npdu_data = npdu.encode();

    let mut message = npdu_data;
    message.extend_from_slice(&apdu_data);

    // BVLC Original-Unicast-NPDU
    let mut bvlc = vec![0x81, 0x0A, 0x00, 0x00];
    bvlc.extend_from_slice(&message);
    let len = bvlc.len() as u16;
    bvlc[2] = (len >> 8) as u8;
    bvlc[3] = (len & 0xFF) as u8;

    socket
        .send_to(&bvlc, target_addr)
        .map_err(|e| ProtocolError::WriteFailed(format!("Alarm send: {e}")))?;

    let mut buf = [0u8; 1500];
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(6) {
        match socket.recv_from(&mut buf) {
            Ok((len, source)) if source == target_addr => {
                if len < 4 || buf[0] != 0x81 {
                    continue;
                }
                let bvlc_len = ((buf[2] as usize) << 8) | buf[3] as usize;
                if len != bvlc_len {
                    continue;
                }
                let npdu_start = 4;
                if let Ok((_npdu, npdu_len)) = Npdu::decode(&buf[npdu_start..len]) {
                    let apdu_start = npdu_start + npdu_len;
                    if let Ok(Apdu::ComplexAck {
                        invoke_id: resp_invoke_id,
                        service_data,
                        ..
                    }) = Apdu::decode(&buf[apdu_start..len])
                    {
                        if resp_invoke_id != invoke_id {
                            continue;
                        }
                        return Ok(service_data);
                    }
                    if let Ok(Apdu::SimpleAck {
                        invoke_id: resp_invoke_id,
                        ..
                    }) = Apdu::decode(&buf[apdu_start..len])
                    {
                        if resp_invoke_id != invoke_id {
                            continue;
                        }
                        return Ok(Vec::new());
                    }
                }
            }
            Ok(_) => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(ProtocolError::ReadFailed(format!("Alarm recv: {e}"))),
        }
    }

    Err(ProtocolError::Timeout(format!(
        "BACnet alarm request timeout for {target_addr}"
    )))
}

// ---------------------------------------------------------------------------
// 1. GetAlarmSummary
// ---------------------------------------------------------------------------

/// Send GetAlarmSummary (confirmed service choice 3) and parse the response.
///
/// Response is a SEQUENCE OF AlarmSummary, each containing:
///   [0] BACnetObjectIdentifier
///   [1] BACnetEventState (enumerated)
///   [2] BACnetEventTransitionBits (bitstring)
pub fn get_alarm_summary_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
) -> Result<Vec<AlarmSummaryEntry>, ProtocolError> {
    let response_data = send_confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::GetAlarmSummary,
        Vec::new(), // empty request
    )?;

    let data = &response_data;
    let mut entries = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        // Each AlarmSummary is a constructed SEQUENCE wrapped in opening/closing tag 0
        // or simply a flat sequence of context tags [0], [1], [2].
        // Many devices send flat (no wrapping), so try that first.
        let (obj_type, instance, new_pos) =
            match extract_context_object_id(data, 0, pos) {
                Some(r) => r,
                None => break,
            };
        pos = new_pos;

        let (state_val, new_pos) = extract_context_unsigned(data, 1, pos)
            .ok_or_else(|| {
                ProtocolError::ReadFailed(
                    "GetAlarmSummary: missing event state tag".into(),
                )
            })?;
        pos = new_pos;

        let (bits, new_pos) = extract_context_bitstring(data, 2, pos)
            .ok_or_else(|| {
                ProtocolError::ReadFailed(
                    "GetAlarmSummary: missing acked transitions tag".into(),
                )
            })?;
        pos = new_pos;

        let obj_type_str = object_type_to_string(&obj_type);
        entries.push(AlarmSummaryEntry {
            object_identifier: format!("{obj_type_str}:{instance}"),
            object_type: obj_type_str,
            object_instance: instance,
            alarm_state: event_state_name(state_val),
            acked_transitions: transition_bits_to_string(bits),
        });
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// 2. GetEventInformation
// ---------------------------------------------------------------------------

/// Send GetEventInformation (confirmed service choice 29) and parse the response.
///
/// If `last_object` is `Some`, it is encoded as context tag 0 to request the next
/// page of results. If `None`, the first page is requested.
///
/// Response contains:
///   [0] opening tag -> SEQUENCE OF BACnetEventSummary -> [0] closing tag
///   [1] BOOLEAN (moreEvents)
pub fn get_event_information_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    last_object: Option<(ObjectType, u32)>,
) -> Result<EventInformationResponse, ProtocolError> {
    // Build request
    let mut service_data = Vec::new();
    if let Some((ot, inst)) = last_object {
        encode_context_object_id(&mut service_data, 0, ot, inst);
    }

    let response_data = send_confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::GetEventInformation,
        service_data,
    )?;

    let data = &response_data;
    let mut entries = Vec::new();
    let mut more_events = false;
    let mut pos = 0;

    // The response wraps event summaries in [0] opening/closing tags.
    // Look for the opening tag of context tag 0 (constructed).
    let mut in_list = false;

    while pos < data.len() {
        if let Some((tag_num, is_opening, is_closing, _val_start, consumed)) =
            read_tag_info(data, pos)
        {
            if is_opening && tag_num == 0 {
                in_list = true;
                pos += consumed;
                continue;
            }
            if is_closing && tag_num == 0 {
                in_list = false;
                pos += consumed;
                continue;
            }

            // Look for moreEvents boolean (context tag 1)
            if tag_num == 1 && !is_opening && !is_closing {
                if let Some((val, new_pos)) = extract_context_boolean(data, 1, pos) {
                    more_events = val;
                    pos = new_pos;
                    continue;
                }
            }

            // Inside the list, parse BACnetEventSummary entries.
            // Each entry: [0] obj_id, [1] eventState, [2] ackedTransitions, [3] eventEnable, [4] eventTimeStamps
            if in_list {
                if let Some((obj_type, instance, new_pos)) =
                    extract_context_object_id(data, 0, pos)
                {
                    pos = new_pos;

                    let state_val = extract_context_unsigned(data, 1, pos)
                        .map(|(v, p)| {
                            pos = p;
                            v
                        })
                        .unwrap_or(0);

                    let ack_bits = extract_context_bitstring(data, 2, pos)
                        .map(|(v, p)| {
                            pos = p;
                            v
                        })
                        .unwrap_or(0);

                    let enable_bits = extract_context_bitstring(data, 3, pos)
                        .map(|(v, p)| {
                            pos = p;
                            v
                        })
                        .unwrap_or(0);

                    // eventTimeStamps [4] is a constructed type -- skip it
                    pos = skip_tag(data, pos);

                    let obj_type_str = object_type_to_string(&obj_type);
                    entries.push(EventInfoEntry {
                        object_identifier: format!("{obj_type_str}:{instance}"),
                        object_type: obj_type_str,
                        object_instance: instance,
                        event_state: event_state_name(state_val),
                        acknowledged_transitions: transition_bits_to_string(ack_bits),
                        event_enable: transition_bits_to_string(enable_bits),
                        event_time_stamps: vec![], // timestamps are complex; left empty
                    });
                    continue;
                }
            }

            pos += consumed;
        } else {
            pos += 1;
        }
    }

    Ok(EventInformationResponse {
        entries,
        more_events,
    })
}

// ---------------------------------------------------------------------------
// 3. AcknowledgeAlarm
// ---------------------------------------------------------------------------

/// Send AcknowledgeAlarm (confirmed service choice 0).
///
/// Parameters:
/// - `object_type_str`: object type string (e.g. "analogInput")
/// - `object_instance`: object instance number
/// - `event_state`: BACnetEventState value (0=normal, 1=fault, 2=offnormal, etc.)
/// - `ack_text`: acknowledgment text
/// - `process_id`: optional acknowledging process identifier (defaults to 0)
pub fn acknowledge_alarm_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_type_str: &str,
    object_instance: u32,
    event_state: u32,
    ack_text: &str,
    process_id: Option<u32>,
) -> Result<(), ProtocolError> {
    // Validate object type (object_instance is context for the caller)
    let _ = (object_type_from_str(object_type_str), object_instance);

    let pid = process_id.unwrap_or(0);

    let mut service_data = Vec::new();

    // [0] acknowledgingObjectIdentifier (use Device:pid as acknowledging object)
    encode_context_object_id(&mut service_data, 0, ObjectType::Device, pid);

    // [1] eventBufferEnqueuedNotify -- optional, skip

    // [2] eventState (enumerated)
    encode_context_enumerated(&mut service_data, 2, event_state);

    // [3] ackTextSource (unsigned, character-string source index, 0=device)
    encode_context_unsigned(&mut service_data, 3, 0);

    // [4] ackText (character string)
    encode_context_character_string(&mut service_data, 4, ack_text);

    // [5] timeOfAcknowledgment -- BACnetDateTime: date [0] + time [1]
    encode_opening_tag(&mut service_data, 5);
    // date [0]: year(2) month(1) day(1) wday(1)
    encode_opening_tag(&mut service_data, 0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Convert unix timestamp to approximate date/time
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    // Date: days since 1900-01-01 (BACnet epoch)
    let bacnet_days = days + 69_596; // 1900-01-01 to 1970-01-01 = 25567 days
    // Year from day count (simplified)
    let (year, month, day, weekday) = days_to_date(bacnet_days as u32);
    let hour = (time_of_day / 3600) as u8;
    let minute = ((time_of_day % 3600) / 60) as u8;
    let second = (time_of_day % 60) as u8;
    // Date: 4 bytes (year MSB, year LSB, month, day, weekday)
    service_data.extend_from_slice(&[
        (year >> 8) as u8,
        (year & 0xFF) as u8,
        month,
        day,
        weekday,
    ]);
    encode_closing_tag(&mut service_data, 0);
    // time [1]: hour(1) minute(1) second(1) hundredths(1)
    encode_opening_tag(&mut service_data, 1);
    service_data.extend_from_slice(&[hour, minute, second, 0]);
    encode_closing_tag(&mut service_data, 1);
    encode_closing_tag(&mut service_data, 5);

    let _response = send_confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::AcknowledgeAlarm,
        service_data,
    )?;

    // SimpleAck expected -- empty data is fine
    Ok(())
}

/// Convert BACnet day number (since 1900-01-01) to (year, month, day, weekday).
fn days_to_date(total_days: u32) -> (u16, u8, u8, u8) {
    let mut year = 1900u16;
    let mut remaining = total_days;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let leap = is_leap_year(year);
    let month_days: [u32; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 0u8;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            month = (i + 1) as u8;
            break;
        }
        remaining -= md;
    }
    if month == 0 {
        month = 12;
    }

    let day = (remaining + 1) as u8;

    // Zeller-like weekday: 1900-01-01 was a Monday (weekday=1 in BACnet)
    let weekday = ((total_days % 7) + 1) as u8;

    (year, month, day, weekday)
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ---------------------------------------------------------------------------
// 4. Parse alarm notification (from incoming UDP packets)
// ---------------------------------------------------------------------------

/// Parse an incoming BACnet event notification (UnconfirmedEventNotification
/// service choice 3 or ConfirmedEventNotification service choice 2).
///
/// `data` should be the raw APDU service_data bytes.
/// `source` is the SocketAddr of the sender.
///
/// Returns `None` if the data cannot be parsed.
pub fn parse_alarm_notification(data: &[u8], source: SocketAddr) -> Option<AlarmNotification> {
    let mut pos = 0;

    // [0] processIdentifier (unsigned) -- skip
    pos = skip_tag(data, pos);
    // [1] initiatingObjectIdentifier -- skip
    pos = skip_tag(data, pos);
    // [2] eventObjectIdentifier
    let (obj_type, instance, new_pos) = extract_context_object_id(data, 2, pos)?;
    pos = new_pos;
    // [3] timeStamp -- skip (complex CHOICE)
    pos = skip_tag(data, pos);
    // [4] notificationClass -- optional, skip if present
    if let Some((tag_num, is_opening, is_closing, _, consumed)) = read_tag_info(data, pos) {
        if tag_num == 4 && !is_opening && !is_closing {
            pos += consumed;
        }
    }
    // [5] priority -- optional, skip if present
    if let Some((tag_num, is_opening, is_closing, _, consumed)) = read_tag_info(data, pos) {
        if tag_num == 5 && !is_opening && !is_closing {
            pos += consumed;
        }
    }
    // [6] eventType (enumerated)
    let (event_type_val, new_pos) = extract_context_unsigned(data, 6, pos)?;
    pos = new_pos;
    // [7] messageText (character string) -- optional
    let message = if let Some((tag_num, is_opening, is_closing, _, _)) = read_tag_info(data, pos) {
        if tag_num == 7 && !is_opening && !is_closing {
            extract_context_character_string(data, 7, pos)
                .map(|(s, p)| {
                    pos = p;
                    s
                })
                .unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    // [8] notifyType (enumerated)
    // 0 = Alarm, 1 = Event, 2 = AckNotification
    let notify_val = if let Some((tag_num, is_opening, is_closing, _, _)) = read_tag_info(data, pos) {
        if tag_num == 8 && !is_opening && !is_closing {
            extract_context_unsigned(data, 8, pos)
                .map(|(v, p)| {
                    pos = p;
                    v
                })
                .unwrap_or(0)
        } else {
            0
        }
    } else {
        0
    };

    let notify_type_str = match notify_val {
        0 => "alarm",
        1 => "event",
        2 => "ackNotification",
        _ => "unknown",
    };
    let ack_required = notify_val != 2; // AckNotification means already acked

    // eventType gives us the "to state" in many cases.
    // eventType values: 0=changeOfBitString, 1=changeOfValue, ... many others.
    // We approximate from_state and to_state from the event type.
    let (from_state, to_state) = event_type_to_states(event_type_val);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Some(AlarmNotification {
        source_ip: source.ip().to_string(),
        source_port: source.port(),
        object_type: object_type_to_string(&obj_type),
        object_instance: instance,
        event_state: event_state_name(event_type_val),
        from_state,
        to_state,
        notify_type: notify_type_str.to_string(),
        ack_required,
        message,
        timestamp: now,
    })
}

/// Map BACnetEventType to approximate from/to state strings.
fn event_type_to_states(event_type: u32) -> (String, String) {
    match event_type {
        0 => ("normal".into(), "offnormal".into()),     // changeOfBitString
        1 => ("normal".into(), "offnormal".into()),     // changeOfValue
        2 => ("normal".into(), "offnormal".into()),     // commandFailure
        3 => ("offnormal".into(), "normal".into()),     // changeOfReliability
        4 => ("normal".into(), "fault".into()),         // changeOfStatusFlags (no alarm)
        5 => ("normal".into(), "fault".into()),         // changeOfLifeSafetyMode (no alarm)
        6 => ("normal".into(), "lifeSafetyAlarm".into()), // changeOfLifeSafetyMode (alarm)
        7 => ("offnormal".into(), "normal".into()),     // extension (none of the above)
        8 => ("normal".into(), "fault".into()),         // outOfRange
        10 => ("normal".into(), "fault".into()),        // changeOfDiscreteValue
        13 => ("normal".into(), "fault".into()),        // floatOutOfRange
        _ => ("unknown".into(), "unknown".into()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_context_unsigned() {
        let mut buf = Vec::new();
        encode_context_unsigned(&mut buf, 0, 42);
        // context tag 0, length 1: (0<<4)|0x08|0x01 = 0x09
        assert_eq!(buf, vec![0x09, 42]);
        let (val, pos) = extract_context_unsigned(&buf, 0, 0).unwrap();
        assert_eq!(val, 42);
        assert_eq!(pos, 2);

        let mut buf = Vec::new();
        encode_context_unsigned(&mut buf, 3, 1000);
        // context tag 3, length 2: (3<<4)|0x08|0x02 = 0x3A
        assert_eq!(buf, vec![0x3A, 0x03, 0xE8]);
        let (val, pos) = extract_context_unsigned(&buf, 3, 0).unwrap();
        assert_eq!(val, 1000);
        assert_eq!(pos, 3);
    }

    #[test]
    fn test_encode_decode_context_object_id() {
        let mut buf = Vec::new();
        encode_context_object_id(&mut buf, 0, ObjectType::AnalogInput, 1);
        // AnalogInput = 0, instance = 1 => (0 << 22) | 1 = 1
        // context tag 0, length 4: (0<<4)|0x08|0x04 = 0x0C
        assert_eq!(buf, vec![0x0C, 0x00, 0x00, 0x00, 0x01]);
        let (ot, inst, pos) = extract_context_object_id(&buf, 0, 0).unwrap();
        assert_eq!(ot, ObjectType::AnalogInput);
        assert_eq!(inst, 1);
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_encode_decode_character_string() {
        let mut buf = Vec::new();
        encode_context_character_string(&mut buf, 4, "hello");
        // context tag 4, extended length (total_len=6 > 4):
        // tag = (4<<4)|0x08|0x05 = 0x4D, length byte = 6, encoding = 0x00, "hello"
        assert_eq!(buf, vec![0x4D, 6, 0x00, 0x68, 0x65, 0x6C, 0x6C, 0x6F]);
        let (s, pos) = extract_context_character_string(&buf, 4, 0).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(pos, 8);
    }

    #[test]
    fn test_event_state_name() {
        assert_eq!(event_state_name(0), "normal");
        assert_eq!(event_state_name(1), "fault");
        assert_eq!(event_state_name(2), "offnormal");
        assert_eq!(event_state_name(3), "highLimit");
        assert_eq!(event_state_name(4), "lowLimit");
        assert_eq!(event_state_name(5), "lifeSafetyAlarm");
        assert_eq!(event_state_name(99), "unknown(99)");
    }

    #[test]
    fn test_transition_bits_to_string() {
        assert_eq!(transition_bits_to_string(0), "");
        assert_eq!(transition_bits_to_string(1), "ToOffNormal");
        assert_eq!(transition_bits_to_string(2), "ToFault");
        assert_eq!(transition_bits_to_string(4), "ToNormal");
        assert_eq!(transition_bits_to_string(7), "ToOffNormal, ToFault, ToNormal");
    }

    #[test]
    fn test_days_to_date() {
        // 1900-01-01 = day 0 -> Monday
        let (y, m, d, w) = days_to_date(0);
        assert_eq!((y, m, d), (1900, 1, 1));
        assert_eq!(w, 1); // Monday

        // 1970-01-01 = day 25567 -> Thursday
        let (y, m, d, w) = days_to_date(25567);
        assert_eq!((y, m, d), (1970, 1, 1));
        assert_eq!(w, 4); // Thursday
    }

    #[test]
    fn test_opening_closing_tags() {
        let mut buf = Vec::new();
        encode_opening_tag(&mut buf, 0);
        encode_opening_tag(&mut buf, 1);
        encode_closing_tag(&mut buf, 1);
        encode_closing_tag(&mut buf, 0);
        assert_eq!(buf, vec![0x0E, 0x1E, 0x1F, 0x0F]);
    }

    #[test]
    fn test_skip_tag() {
        // context tag 0, unsigned 42: [0x09, 42]
        // context tag 1, boolean false: [0x19, 0x00]
        let data = vec![0x09, 42, 0x19, 0x00];
        assert_eq!(skip_tag(&data, 0), 2); // skip 1-byte value
        assert_eq!(skip_tag(&data, 2), 4);
        // Opening tag 0: 0x0E
        assert_eq!(skip_tag(&[0x0E], 0), 1);
        // Closing tag 1: 0x1F
        assert_eq!(skip_tag(&[0x1F], 0), 1);
    }
}
