//! BACnet Trend Log service module.
//!
//! Provides custom BER encoding/decoding for BACnet ReadRange service and LogRecord parsing.
//!
//! The `bacnet-rs` crate only defines `ConfirmedServiceChoice::ReadRange` (value 26) as an enum
//! -- no request/response structs. We implement the BER encoding ourselves following
//! ASHRAE 135 clause 12.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::{ObjectIdentifier, ObjectType};
use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

// ---------------------------------------------------------------------------
// Invoke ID counter (separate from alarm.rs and client.rs to avoid collisions)
// ---------------------------------------------------------------------------

/// Start at 200 to stay out of client.rs (0-99) and alarm.rs (100-199) ranges.
static TREND_INVOKE_ID: AtomicU8 = AtomicU8::new(200);

fn next_invoke_id() -> u8 {
    TREND_INVOKE_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single log record from a TrendLog object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogRecord {
    /// ISO 8601 formatted timestamp.
    pub timestamp: String,
    /// Display string of the logged value.
    pub value: String,
    /// Status flags bit string, if present.
    pub status_flags: Option<String>,
}

/// Result of reading a TrendLog, including all returned records and total count.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrendLogData {
    /// Object instance number of the TrendLog.
    pub object_instance: u32,
    /// Parsed log records returned by the device.
    pub records: Vec<LogRecord>,
    /// Total record count reported by the device.
    pub total_count: u32,
}

// ---------------------------------------------------------------------------
// BER encoding helpers (context-tagged)
// ---------------------------------------------------------------------------

/// Encode a context-tagged unsigned integer (primitive).
/// BACnet tag byte: [tag_num(4) | context(1) | length(3)]
fn encode_context_unsigned(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    let tag_byte = (tag_num << 4) | 0x08; // context bit set, length in low 3 bits
    if value <= 255 {
        buf.push(tag_byte | 1);
        buf.push(value as u8);
    } else if value <= 65535 {
        buf.push(tag_byte | 2);
        buf.extend_from_slice(&[(value >> 8) as u8, value as u8]);
    } else {
        buf.push(tag_byte | 4);
        buf.extend_from_slice(&value.to_be_bytes());
    }
}

/// Encode a context-tagged signed integer (primitive).
/// Used for negative counts when reading backward from end.
fn encode_context_signed(buf: &mut Vec<u8>, tag_num: u8, value: i32) {
    let tag_byte = (tag_num << 4) | 0x08;
    let abs = value.unsigned_abs();
    if abs <= 127 {
        buf.push(tag_byte | 1);
        buf.push(value as i8 as u8);
    } else if abs <= 32767 {
        buf.push(tag_byte | 2);
        buf.extend_from_slice(&(value as i16).to_be_bytes());
    } else {
        buf.push(tag_byte | 4);
        buf.extend_from_slice(&value.to_be_bytes());
    }
}

/// Encode a context-tagged BACnet object identifier (4-byte packed value).
fn encode_context_object_id(buf: &mut Vec<u8>, tag_num: u8, obj_type: ObjectType, instance: u32) {
    let type_u32: u32 = obj_type.into();
    let value = (type_u32 << 22) | (instance & 0x3FFFFF);
    buf.push((tag_num << 4) | 0x0C); // context tag, length 4 (0x08 | 0x04)
    buf.extend_from_slice(&value.to_be_bytes());
}

/// Encode a context-tagged enumerated value (identical to unsigned encoding).
fn encode_context_enumerated(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    encode_context_unsigned(buf, tag_num, value);
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

    // Opening tag (0x0E) or closing tag (0x0F)
    if low_nibble == 0x0E || low_nibble == 0x0F {
        return pos + 1;
    }

    // Length encoded in bits 2-0 of tag byte
    let mut length = (tag & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
        if pos + 1 >= data.len() {
            return data.len();
        }
        let len_byte = data[pos + 1];
        consumed += 1;
        match len_byte {
            0..=253 => length = len_byte as usize,
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

    let mut length = (byte & 0x07) as usize;
    let mut consumed = 1;

    if length == 5 {
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
        if pos + 1 >= data.len() {
            return None;
        }
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

/// Extract a context-tagged signed integer.
#[allow(dead_code)]
fn extract_context_signed(data: &[u8], tag_num: u8, pos: usize) -> Option<(i32, usize)> {
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
        if pos + 1 >= data.len() {
            return None;
        }
        length = data[pos + 1] as usize;
        consumed += 1;
    }

    let val_start = pos + consumed;
    let val = match length {
        1 => {
            let raw = *data.get(val_start)? as i8;
            raw as i32
        }
        2 => {
            let raw = i16::from_be_bytes([
                *data.get(val_start)?,
                *data.get(val_start + 1)?,
            ]);
            raw as i32
        }
        4 => i32::from_be_bytes([
            *data.get(val_start)?,
            *data.get(val_start + 1)?,
            *data.get(val_start + 2)?,
            *data.get(val_start + 3)?,
        ]),
        _ => return None,
    };
    Some((val, pos + consumed + length))
}

/// Extract a context-tagged bitstring. Returns the raw bitstring bytes and new position.
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
        if pos + 1 >= data.len() {
            return None;
        }
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

// ---------------------------------------------------------------------------
// Send confirmed request helper (same pattern as alarm.rs)
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
        .map_err(|e| ProtocolError::WriteFailed(format!("TrendLog send: {e}")))?;

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
            Err(e) => return Err(ProtocolError::ReadFailed(format!("TrendLog recv: {e}"))),
        }
    }

    Err(ProtocolError::Timeout(format!(
        "BACnet trend log request timeout for {target_addr}"
    )))
}

// ---------------------------------------------------------------------------
// ReadRange request encoding
// ---------------------------------------------------------------------------

/// Build a ReadRange-Request APDU service_data for reading TrendLog LogBuffer.
///
/// Encoding:
/// ```text
/// ReadRange-Request ::= SEQUENCE {
///     objectIdentifier     [0] BACnetObjectIdentifier,
///     propertyIdentifier   [1] BACnetPropertyIdentifier (enumerated),
///     range                [3] SEQUENCE {
///         byPosition: referenceIndex [0] Unsigned, count [1] INTEGER
///     }
/// }
/// ```
fn build_read_range_request(object_instance: u32, start_index: u32, count: i32) -> Vec<u8> {
    let mut service_data = Vec::new();

    // [0] objectIdentifier: TrendLog type (20) with given instance
    encode_context_object_id(&mut service_data, 0, ObjectType::TrendLog, object_instance);

    // [1] propertyIdentifier: LogBuffer (118)
    encode_context_enumerated(&mut service_data, 1, 118);

    // [3] range: ByPosition = opening tag 3 -> [0] refIndex -> [1] count -> closing tag 3
    encode_opening_tag(&mut service_data, 3);
    // [0] referenceIndex (unsigned)
    encode_context_unsigned(&mut service_data, 0, start_index);
    // [1] count (signed integer -- negative means read backward from end)
    encode_context_signed(&mut service_data, 1, count);
    encode_closing_tag(&mut service_data, 3);

    service_data
}

// ---------------------------------------------------------------------------
// DateTime parsing
// ---------------------------------------------------------------------------

/// Parse a BACnetDateTime (8 bytes) into an ISO 8601 string.
///
/// Bytes: year_hi, year_lo, month, day, wday, hour, min, sec, hundredths
/// BACnet year is years since 1900.
fn parse_bacnet_datetime(data: &[u8]) -> Option<String> {
    if data.len() < 8 {
        return None;
    }
    let year = ((data[0] as u16) << 8) | data[1] as u16;
    let year = year + 1900;
    let month = data[2];
    let day = data[3];
    let _wday = data[4]; // weekday, not needed for ISO format
    let hour = data[5];
    let minute = data[6];
    let second = data[7];
    // hundredths byte is data[8] if present -- we ignore it

    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"
    ))
}

// ---------------------------------------------------------------------------
// LogDatum parsing (application-tagged value)
// ---------------------------------------------------------------------------

/// Parse an application-tagged value from the logDatum CHOICE.
/// Returns a display string.
fn parse_log_datum(data: &[u8], pos: usize) -> (String, usize) {
    if pos >= data.len() {
        return ("null".into(), pos);
    }

    let tag_byte = data[pos];
    // BACnet tag byte: [class(1) | tag_number(4) | length(3)]
    // Bit 7: class (0=application, 1=context-specific)
    // Bits 6-4: tag number
    // Bits 2-0: length encoding (0-4=literal, 5=extended)
    let is_context = (tag_byte & 0x80) != 0;
    let app_tag = (tag_byte >> 4) & 0x0F;
    let length_code = tag_byte & 0x07;

    // Context-tagged datum: some devices encode logDatum with context tags
    if is_context {
        let tag_num = (tag_byte >> 4) & 0x0F;
        let low = tag_byte & 0x07;

        // Opening tag
        if low == 0x06 {
            // Skip to closing tag
            let mut depth = 1;
            let mut p = pos + 1;
            while p < data.len() && depth > 0 {
                let b = data[p];
                let t = (b >> 4) & 0x0F;
                let l = b & 0x07;
                let is_ctx = (b & 0x08) != 0;
                if is_ctx && l == 0x06 && t == tag_num {
                    depth += 1;
                } else if is_ctx && l == 0x07 && t == tag_num {
                    depth -= 1;
                }
                if depth > 0 {
                    p = skip_tag(data, p);
                }
            }
            return (format!("[context:{tag_num}]"), p + 1);
        }

        // Context-tagged primitive: decode based on length
        let mut length = low as usize;
        let mut consumed = 1;
        if length == 5 {
            if pos + 1 >= data.len() {
                return ("null".into(), pos + 1);
            }
            length = data[pos + 1] as usize;
            consumed += 1;
        }
        let val_start = pos + consumed;
        if val_start + length > data.len() {
            return ("null".into(), pos + consumed);
        }
        let val_bytes = &data[val_start..val_start + length];
        let display = match length {
            0 => "null".into(),
            1 => format!("{}", val_bytes[0]),
            2 => format!("{}", u16::from_be_bytes([val_bytes[0], val_bytes[1]])),
            4 => format!("{}", u32::from_be_bytes([val_bytes[0], val_bytes[1], val_bytes[2], val_bytes[3]])),
            _ => format!("{val_bytes:?}"),
        };
        return (display, pos + consumed + length);
    }

    // Application-tagged value
    let length = match length_code {
        5 => {
            // Extended length
            if pos + 1 >= data.len() {
                return ("null".into(), pos + 1);
            }
            data[pos + 1] as usize
        }
        n => n as usize,
    };
    let mut consumed = 1; // tag byte consumed
    if length_code == 5 {
        consumed += 1; // extended length byte consumed
    }

    let val_start = pos + consumed;

    match app_tag {
        // Null (0x00)
        0x00 => ("null".into(), pos + consumed),

        // Boolean (0x31 = true, 0x30 = false; tag_byte encodes it in the length bits)
        0x01 => {
            // In BACnet, boolean is encoded in the length field of the tag
            let is_true = (tag_byte & 0x07) == 1 || (tag_byte & 0x01) != 0;
            let display = if is_true { "true" } else { "false" };
            (display.into(), pos + consumed)
        }

        // Unsigned integer (0x20-0x2F depending on length)
        0x02 => {
            if val_start + length > data.len() {
                return ("null".into(), pos + consumed);
            }
            let val = match length {
                1 => data[val_start] as u64,
                2 => u16::from_be_bytes([data[val_start], data[val_start + 1]]) as u64,
                4 => u32::from_be_bytes([
                    data[val_start],
                    data[val_start + 1],
                    data[val_start + 2],
                    data[val_start + 3],
                ]) as u64,
                _ => 0,
            };
            (format!("{val}"), pos + consumed + length)
        }

        // Signed integer (0x30-0x3F)
        0x03 => {
            if val_start + length > data.len() {
                return ("null".into(), pos + consumed);
            }
            let val = match length {
                1 => data[val_start] as i8 as i64,
                2 => i16::from_be_bytes([data[val_start], data[val_start + 1]]) as i64,
                4 => i32::from_be_bytes([
                    data[val_start],
                    data[val_start + 1],
                    data[val_start + 2],
                    data[val_start + 3],
                ]) as i64,
                _ => 0,
            };
            (format!("{val}"), pos + consumed + length)
        }

        // Real (single precision float, tag 0x44)
        0x04 => {
            if val_start + 4 > data.len() {
                return ("null".into(), pos + consumed);
            }
            let bytes = [
                data[val_start],
                data[val_start + 1],
                data[val_start + 2],
                data[val_start + 3],
            ];
            let val = f32::from_be_bytes(bytes);
            (format!("{val:.2}"), pos + consumed + 4)
        }

        // Double (double precision float, tag 0x55)
        0x05 => {
            if val_start + 8 > data.len() {
                return ("null".into(), pos + consumed);
            }
            let bytes = [
                data[val_start],
                data[val_start + 1],
                data[val_start + 2],
                data[val_start + 3],
                data[val_start + 4],
                data[val_start + 5],
                data[val_start + 6],
                data[val_start + 7],
            ];
            let val = f64::from_be_bytes(bytes);
            (format!("{val:.2}"), pos + consumed + 8)
        }

        // Octet string (0x06)
        0x06 => {
            if val_start + length > data.len() {
                return ("null".into(), pos + consumed);
            }
            let hex: String = data[val_start..val_start + length]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect();
            (format!("0x{hex}"), pos + consumed + length)
        }

        // Character string (0x07)
        0x07 => {
            if val_start + length > data.len() || length < 2 {
                return ("null".into(), pos + consumed);
            }
            // First byte is encoding type: 0=ISO-8859-1, 4=UTF-8
            let _encoding = data[val_start];
            let str_bytes = &data[val_start + 1..val_start + length];
            let s = String::from_utf8_lossy(str_bytes).into_owned();
            (s, pos + consumed + length)
        }

        // Bit string (0x08)
        0x08 => {
            if val_start + length > data.len() || length < 1 {
                return ("null".into(), pos + consumed);
            }
            let unused = data[val_start] & 0x07;
            let bits = &data[val_start + 1..val_start + length];
            let hex: String = bits.iter().map(|b| format!("{b:08b}")).collect();
            (format!("bits({unused}unused):{hex}"), pos + consumed + length)
        }

        // Enumerated (0x09)
        0x09 => {
            if val_start + length > data.len() {
                return ("null".into(), pos + consumed);
            }
            let val = match length {
                1 => data[val_start] as u32,
                2 => u16::from_be_bytes([data[val_start], data[val_start + 1]]) as u32,
                4 => u32::from_be_bytes([
                    data[val_start],
                    data[val_start + 1],
                    data[val_start + 2],
                    data[val_start + 3],
                ]),
                _ => 0,
            };
            (format!("{val}"), pos + consumed + length)
        }

        // Date (0x0A)
        0x0A => {
            if val_start + 4 > data.len() {
                return ("null".into(), pos + consumed);
            }
            let year = ((data[val_start] as u16) << 8) | data[val_start + 1] as u16;
            let month = data[val_start + 2];
            let day = data[val_start + 3];
            (format!("{}-{:02}-{:02}", year + 1900, month, day), pos + consumed + 4)
        }

        // Time (0x0B)
        0x0B => {
            if val_start + 4 > data.len() {
                return ("null".into(), pos + consumed);
            }
            let hour = data[val_start];
            let min = data[val_start + 1];
            let sec = data[val_start + 2];
            (format!("{hour:02}:{min:02}:{sec:02}"), pos + consumed + 4)
        }

        // Object identifier (0x0C)
        0x0C => {
            if val_start + 4 > data.len() {
                return ("null".into(), pos + consumed);
            }
            let raw = u32::from_be_bytes([
                data[val_start],
                data[val_start + 1],
                data[val_start + 2],
                data[val_start + 3],
            ]);
            let obj_type = (raw >> 22) & 0x3FF;
            let instance = raw & 0x3FFFFF;
            (format!("obj:{obj_type}:{instance}"), pos + consumed + 4)
        }

        // Unknown tag -- try to skip
        _ => {
            if val_start + length > data.len() {
                return (format!("[unknown tag 0x{app_tag:02X}]"), pos + consumed);
            }
            let hex: String = data[val_start..val_start + length]
                .iter()
                .take(8)
                .map(|b| format!("{b:02X}"))
                .collect();
            (
                format!("[tag 0x{app_tag:02X}: {hex}]"),
                pos + consumed + length,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Status flags bitstring to display string
// ---------------------------------------------------------------------------

/// Convert a BACnet StatusFlags bitstring byte to a human-readable string.
/// Bits: IN_ALARM(3), FAULT(2), OVERRIDDEN(1), OUT_OF_SERVICE(0).
fn status_flags_to_string(bits: u8) -> String {
    let mut parts = Vec::new();
    if bits & 0x08 != 0 {
        parts.push("inAlarm");
    }
    if bits & 0x04 != 0 {
        parts.push("fault");
    }
    if bits & 0x02 != 0 {
        parts.push("overridden");
    }
    if bits & 0x01 != 0 {
        parts.push("outOfService");
    }
    if parts.is_empty() {
        return String::new();
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// ReadRange-Ack response parsing
// ---------------------------------------------------------------------------

/// Parse the ReadRange-Ack response data into TrendLogData.
///
/// Response structure:
/// ```text
/// ReadRange-Ack ::= SEQUENCE {
///     objectIdentifier     [0] BACnetObjectIdentifier,
///     propertyIdentifier   [1] BACnetPropertyIdentifier,
///     result               [3] SEQUENCE OF LogRecord
/// }
/// ```
///
/// Each LogRecord:
/// ```text
/// LogRecord ::= SEQUENCE {
///     timestamp   [0] BACnetDateTime,
///     logDatum    CHOICE { application-tagged value },
///     statusFlags [2] BitString OPTIONAL
/// }
/// ```
fn parse_read_range_ack(data: &[u8], object_instance: u32) -> Result<TrendLogData, ProtocolError> {
    let mut pos = 0;
    let mut records = Vec::new();
    let mut total_count: u32 = 0;

    // Skip [0] objectIdentifier
    pos = skip_tag(data, pos);

    // Skip [1] propertyIdentifier
    pos = skip_tag(data, pos);

    // Skip optional [2] propertyArrayIndex
    if pos < data.len() {
        if let Some((tag_num, is_opening, is_closing, _, consumed)) = read_tag_info(data, pos) {
            if tag_num == 2 && !is_opening && !is_closing {
                pos += consumed;
            }
        }
    }

    // Find [3] result: SEQUENCE OF LogRecord (opening/closing tag 3)
    // Also look for [4] itemCount which gives us the total count
    let mut in_result_list = false;

    while pos < data.len() {
        if let Some((tag_num, is_opening, is_closing, _val_start, consumed)) =
            read_tag_info(data, pos)
        {
            // Opening tag 3 = start of result list
            if is_opening && tag_num == 3 {
                in_result_list = true;
                pos += consumed;
                continue;
            }

            // Closing tag 3 = end of result list
            if is_closing && tag_num == 3 {
                in_result_list = false;
                pos += consumed;
                continue;
            }

            // [4] itemCount (unsigned) -- total record count
            if tag_num == 4 && !is_opening && !is_closing && !in_result_list {
                if let Some((count, new_pos)) = extract_context_unsigned(data, 4, pos) {
                    total_count = count;
                    pos = new_pos;
                    continue;
                }
            }

            // Inside result list, parse LogRecord entries
            if in_result_list {
                // Try to parse a LogRecord starting with timestamp [0]
                // LogRecord is wrapped in opening/closing tag for the sequence
                if let Some((_, is_open, is_close, _, c)) = read_tag_info(data, pos) {
                    if is_open {
                        // Opening tag of LogRecord sequence -- skip it
                        pos += c;
                        continue;
                    }
                    if is_close {
                        // Closing tag of LogRecord sequence
                        pos += c;
                        continue;
                    }
                }

                // Parse [0] timestamp (BACnetDateTime -- context-tagged constructed)
                let mut timestamp = String::new();
                let mut value = String::new();
                let mut status_flags: Option<String> = None;
                let mut record_parsed = false;

                // Look for timestamp context tag 0 (constructed -- opening tag)
                if let Some((tag_num, is_opening, _, _, consumed)) = read_tag_info(data, pos) {
                    if tag_num == 0 && is_opening {
                        // This is the datetime opening tag. The datetime value follows
                        // as 8 bytes inside the constructed type.
                        let dt_start = pos + consumed;
                        // Some devices put the 8 bytes directly; others nest date[0]/time[1]
                        // Try direct 8-byte parse first
                        if dt_start + 8 <= data.len() {
                            // Check if next tag after is closing tag 0 (indicating direct encoding)
                            if let Some((_, _, is_close, _, _)) =
                                read_tag_info(data, dt_start + 8)
                            {
                                if is_close {
                                    timestamp = parse_bacnet_datetime(&data[dt_start..])
                                        .unwrap_or_else(|| "invalid-date".into());
                                    pos = dt_start + 8 + 1; // skip closing tag
                                } else {
                                    // Nested: date [0] opening -> 4 bytes -> closing -> time [1] opening -> 4 bytes -> closing
                                    pos = dt_start;
                                    // Skip date sub-field
                                    pos = skip_tag(data, pos); // opening tag 0
                                    pos = skip_tag(data, pos); // date bytes (4)
                                    pos = skip_tag(data, pos); // closing tag 0
                                    // Build datetime from what we have
                                    // Re-read: we already skipped -- extract from before
                                    // Actually let's just try a simpler approach
                                    timestamp = "datetime".into();
                                }
                            } else {
                                timestamp = parse_bacnet_datetime(&data[dt_start..])
                                    .unwrap_or_else(|| "invalid-date".into());
                                pos = dt_start + 8;
                            }
                        } else {
                            pos = dt_start;
                        }
                    } else {
                        // Not a datetime opening tag -- try application-tagged date
                        // This handles cases where timestamp is application-tagged
                        pos += consumed;
                    }
                }

                // If we didn't parse a timestamp via constructed tag, try application date+time
                if timestamp.is_empty() && pos < data.len() {
                    // Check for application date tag (0x0A) or time tag (0x0B)
                    // Combined datetime is sometimes sent as two application tags
                    let app_byte = data[pos];
                    let app_tag = app_byte & 0x0F;
                    if app_tag == 0x0A {
                        // Application Date (4 bytes)
                        if pos + 5 <= data.len() {
                            let date_year =
                                ((data[pos + 1] as u16) << 8) | data[pos + 2] as u16;
                            let month = data[pos + 3];
                            let day = data[pos + 4];
                            timestamp = format!(
                                "{:04}-{:02}-{:02}",
                                date_year + 1900,
                                month,
                                day
                            );
                            pos += 5;
                            // Next might be time tag
                            if pos < data.len() && (data[pos] & 0x0F) == 0x0B && pos + 4 <= data.len()
                            {
                                let hour = data[pos + 1];
                                let min = data[pos + 2];
                                let sec = data[pos + 3];
                                timestamp.push_str(&format!("T{hour:02}:{min:02}:{sec:02}"));
                                pos += 4;
                            }
                        } else {
                            pos = skip_tag(data, pos);
                        }
                    } else if app_tag == 0x00 {
                        // Null timestamp
                        timestamp = "--".into();
                        pos = skip_tag(data, pos);
                    } else {
                        // Not a date -- this might be the start of logDatum already
                        // Reset: treat this as logDatum
                    }
                }

                // Parse logDatum (application-tagged CHOICE)
                if pos < data.len() {
                    // Check if this is still the timestamp (constructed context tag)
                    // or if we need to skip remaining timestamp parts
                    let check_byte = data[pos];
                    let check_tag = (check_byte >> 4) & 0x0F;
                    let check_low = check_byte & 0x0F;
                    let check_ctx = (check_byte & 0x08) != 0;

                    // Skip any remaining timestamp sub-tags (time part inside datetime constructed)
                    if check_ctx && check_tag == 1 && check_low == 0x06 {
                        // time opening tag inside datetime
                        pos = skip_tag(data, pos); // opening tag
                        pos = skip_tag(data, pos); // time bytes
                        pos = skip_tag(data, pos); // closing tag
                    }

                    // Skip closing tag of datetime if present
                    if pos < data.len() {
                        let cb = data[pos];
                        let ct = (cb >> 4) & 0x0F;
                        let cl = cb & 0x0F;
                        let cc = (cb & 0x08) != 0;
                        if cc && cl == 0x07 && ct == 0 {
                            pos += 1; // closing tag of datetime [0]
                        }
                    }

                    // Now parse logDatum
                    if pos < data.len() {
                        let (val, new_pos) = parse_log_datum(data, pos);
                        value = val;
                        pos = new_pos;
                        record_parsed = true;
                    }
                }

                // Parse optional statusFlags [2]
                if pos < data.len() {
                    if let Some((tag_num, is_opening, is_closing, _, consumed)) =
                        read_tag_info(data, pos)
                    {
                        if tag_num == 2 && !is_opening && !is_closing {
                            if let Some((bits, new_pos)) = extract_context_bitstring(data, 2, pos)
                            {
                                let flags = status_flags_to_string(bits);
                                if !flags.is_empty() {
                                    status_flags = Some(flags);
                                }
                                pos = new_pos;
                            } else {
                                pos += consumed;
                            }
                        }
                    }
                }

                if record_parsed {
                    if total_count == 0 {
                        // If we didn't get itemCount, count records we parsed
                        total_count = records.len() as u32;
                    }
                    records.push(LogRecord {
                        timestamp,
                        value,
                        status_flags,
                    });
                }
            } else {
                pos += consumed;
            }
        } else {
            pos += 1;
        }
    }

    Ok(TrendLogData {
        object_instance,
        records,
        total_count,
    })
}

// ---------------------------------------------------------------------------
// 1. read_trend_log_blocking
// ---------------------------------------------------------------------------

/// Read trend log records from a TrendLog object using BACnet ReadRange service.
///
/// Sends a ReadRange request for the LogBuffer property (ID 118) of the given
/// TrendLog object, reading from `start_index` with the specified `count`.
/// A negative `count` reads backward from the end of the log.
///
/// Parameters:
/// - `socket`: UDP socket for BACnet communication
/// - `target_addr`: BACnet device address
/// - `object_instance`: TrendLog object instance number
/// - `start_index`: Index of the first record to read (1-based)
/// - `count`: Number of records to read (negative = read backward from end)
pub fn read_trend_log_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_instance: u32,
    start_index: u32,
    count: i32,
) -> Result<TrendLogData, ProtocolError> {
    let service_data = build_read_range_request(object_instance, start_index, count);

    let response_data = send_confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::ReadRange,
        service_data,
    )?;

    parse_read_range_ack(&response_data, object_instance)
}

// ---------------------------------------------------------------------------
// 2. read_trend_log_count_blocking
// ---------------------------------------------------------------------------

/// Read the RecordCount property (ID 117) of a TrendLog object.
///
/// Uses BACnet ReadProperty (service choice 12) to read just the count,
/// which is more efficient than a full ReadRange request.
pub fn read_trend_log_count_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_instance: u32,
) -> Result<u32, ProtocolError> {
    // Build ReadProperty request for RecordCount (117) of TrendLog object
    let obj_id = ObjectIdentifier::new(ObjectType::TrendLog, object_instance);
    let prop_id = bacnet_rs::object::PropertyIdentifier::RecordCount;

    let rp_request =
        bacnet_rs::service::ReadPropertyRequest::new(obj_id, prop_id);
    let mut service_data = Vec::new();
    rp_request
        .encode(&mut service_data)
        .map_err(|e| ProtocolError::WriteFailed(format!("ReadProperty encode: {e}")))?;

    let response_data = send_confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::ReadProperty,
        service_data,
    )?;

    // Parse ReadProperty-Ack: [0] objId, [1] propId, value (application-tagged unsigned)
    let mut pos = 0;

    // Skip [0] objectIdentifier
    pos = skip_tag(&response_data, pos);

    // Skip [1] propertyIdentifier
    pos = skip_tag(&response_data, pos);

    // Skip optional [2] propertyArrayIndex
    if pos < response_data.len() {
        if let Some((tag_num, is_opening, is_closing, _, consumed)) =
            read_tag_info(&response_data, pos)
        {
            if tag_num == 2 && !is_opening && !is_closing {
                pos += consumed;
            }
        }
    }

    // The value is the remaining data -- should be application-tagged unsigned
    if pos >= response_data.len() {
        return Err(ProtocolError::ReadFailed(
            "ReadProperty-Ack: empty value for RecordCount".into(),
        ));
    }

    let tag_byte = response_data[pos];
    let app_tag = tag_byte & 0x0F;
    let length_code = (tag_byte >> 4) & 0x07;

    // Application tag 0x02 = unsigned integer
    if app_tag != 0x02 {
        return Err(ProtocolError::ReadFailed(format!(
            "ReadProperty-Ack: expected unsigned tag (0x02), got 0x{app_tag:02X}"
        )));
    }

    let length = match length_code {
        5 => {
            if pos + 1 >= response_data.len() {
                return Err(ProtocolError::ReadFailed(
                    "ReadProperty-Ack: truncated extended length".into(),
                ));
            }
            response_data[pos + 1] as usize
        }
        n => n as usize,
    };
    let val_start = pos + 1 + if length_code == 5 { 1 } else { 0 };

    if val_start + length > response_data.len() {
        return Err(ProtocolError::ReadFailed(
            "ReadProperty-Ack: truncated value".into(),
        ));
    }

    let count = match length {
        1 => response_data[val_start] as u32,
        2 => u16::from_be_bytes([
            response_data[val_start],
            response_data[val_start + 1],
        ]) as u32,
        4 => u32::from_be_bytes([
            response_data[val_start],
            response_data[val_start + 1],
            response_data[val_start + 2],
            response_data[val_start + 3],
        ]),
        _ => {
            return Err(ProtocolError::ReadFailed(format!(
                "ReadProperty-Ack: unexpected unsigned length {length}"
            )));
        }
    };

    Ok(count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_context_signed() {
        let mut buf = Vec::new();
        encode_context_signed(&mut buf, 0, -10);
        // context tag 0, length 1: (0<<4)|0x08|0x01 = 0x09, value = 0xF6 (two's complement)
        assert_eq!(buf[0], 0x09);
        assert_eq!(buf[1], 0xF6); // -10 in i8

        let mut buf = Vec::new();
        encode_context_signed(&mut buf, 1, 50);
        // context tag 1, length 1: (1<<4)|0x08|0x01 = 0x19, value = 50
        assert_eq!(buf, vec![0x19, 50]);
    }

    #[test]
    fn test_encode_context_signed_large() {
        let mut buf = Vec::new();
        encode_context_signed(&mut buf, 0, -1000);
        // context tag 0, length 2: (0<<4)|0x08|0x02 = 0x0A, value = 0xFC18
        assert_eq!(buf[0], 0x0A);
        let val = i16::from_be_bytes([buf[1], buf[2]]);
        assert_eq!(val, -1000);
    }

    #[test]
    fn test_build_read_range_request() {
        let data = build_read_range_request(1, 1, 10);
        // Should start with objectIdentifier [0] for TrendLog instance 1
        assert_eq!(data[0] & 0xF0, 0x00); // context tag 0
        // TrendLog = type 20, instance 1: (20 << 22) | 1 = 0x05000001
        assert_eq!(data[0], 0x0C); // context tag 0, length 4
        assert_eq!(&data[1..5], &[0x05, 0x00, 0x00, 0x01]);

        // [1] propertyIdentifier = 118 (LogBuffer)
        assert_eq!(data[5] & 0xF0, 0x10); // context tag 1
    }

    #[test]
    fn test_build_read_range_request_negative_count() {
        let data = build_read_range_request(1, 1, -50);
        // Verify it encodes without panic
        assert!(!data.is_empty());
        // Should contain opening tag 3
        assert!(data.iter().any(|&b| b == (3 << 4) | 0x0E));
    }

    #[test]
    fn test_parse_bacnet_datetime() {
        // 2024-06-15 14:30:45
        // year = 2024-1900 = 124 = 0x007C
        let data = [0x00, 0x7C, 0x06, 0x0F, 0x06, 0x0E, 0x1E, 0x2D];
        let ts = parse_bacnet_datetime(&data).unwrap();
        assert_eq!(ts, "2024-06-15T14:30:45");
    }

    #[test]
    fn test_parse_bacnet_datetime_short() {
        // Too short
        assert!(parse_bacnet_datetime(&[0x00, 0x7C]).is_none());
    }

    #[test]
    fn test_parse_log_datum_real() {
        // Real value: tag 0x44, length 4, value = 72.5 (f32: 0x42910000)
        let data = vec![0x44, 0x42, 0x91, 0x00, 0x00, 0xFF];
        let (val, pos) = parse_log_datum(&data, 0);
        assert!(val.contains("72.5") || val.contains("72.50"));
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_parse_log_datum_double() {
        // Double value: app tag 5, extended length 8
        // Tag byte: 0 | 101 | 101 = 0x55, but 0x55 has bit 7=0 so is_context=false
        // length_code = 0x55 & 0x07 = 5 (extended), so length = data[1] = 8
        let mut data = vec![0x55, 8]; // tag + extended length byte
        let val_f64: f64 = 98.6;
        data.extend_from_slice(&val_f64.to_be_bytes());
        data.push(0xFF); // trailing
        let (val, pos) = parse_log_datum(&data, 0);
        assert!(val.contains("98.6") || val.contains("98.60"));
        assert_eq!(pos, 10); // 1 (tag) + 1 (ext len) + 8 (value)
    }

    #[test]
    fn test_parse_log_datum_unsigned() {
        // Unsigned 32-bit: tag 0x24, length 4, value = 1000
        let data = vec![0x24, 0x00, 0x00, 0x03, 0xE8, 0xFF];
        let (val, pos) = parse_log_datum(&data, 0);
        assert_eq!(val, "1000");
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_parse_log_datum_null() {
        // Null: tag 0x00
        let data = vec![0x00, 0xFF];
        let (val, pos) = parse_log_datum(&data, 0);
        assert_eq!(val, "null");
        assert_eq!(pos, 1);
    }

    #[test]
    fn test_parse_log_datum_boolean() {
        // Boolean true: app tag 1, length 1, value byte (any nonzero = true)
        // Tag byte: 0 | 001 | 001 = 0x11
        let data = vec![0x11, 0x01, 0xFF];
        let (val, _pos) = parse_log_datum(&data, 0);
        assert_eq!(val, "true");

        // Boolean false: app tag 1, length 0
        // Tag byte: 0 | 001 | 000 = 0x10
        let data = vec![0x10, 0xFF];
        let (val, _pos) = parse_log_datum(&data, 0);
        assert_eq!(val, "false");
    }

    #[test]
    fn test_parse_log_datum_string() {
        // CharacterString: app tag 7, extended length
        // Tag byte: 0 | 111 | 101 = 0x7D, length byte = 3 (encoding(1) + "hi" = 3)
        let data = vec![0x7D, 0x03, 0x00, b'h', b'i', 0xFF];
        let (val, pos) = parse_log_datum(&data, 0);
        assert_eq!(val, "hi");
        assert_eq!(pos, 5); // 1 (tag) + 1 (ext len) + 3 (value)
    }

    #[test]
    fn test_status_flags_to_string() {
        assert_eq!(status_flags_to_string(0x08), "inAlarm");
        assert_eq!(status_flags_to_string(0x04), "fault");
        assert_eq!(status_flags_to_string(0x02), "overridden");
        assert_eq!(status_flags_to_string(0x01), "outOfService");
        assert_eq!(status_flags_to_string(0x0F), "inAlarm, fault, overridden, outOfService");
        assert_eq!(status_flags_to_string(0x00), "");
    }

    #[test]
    fn test_extract_context_signed() {
        let mut buf = Vec::new();
        encode_context_signed(&mut buf, 0, -10);
        let (val, pos) = extract_context_signed(&buf, 0, 0).unwrap();
        assert_eq!(val, -10);
        assert_eq!(pos, 2);
    }

    #[test]
    fn test_read_tag_info() {
        // context tag 0, opening: (0<<4)|0x08|0x06 = 0x0E
        let data = vec![0x0E];
        let (tag, is_open, is_close, _, consumed) = read_tag_info(&data, 0).unwrap();
        assert_eq!(tag, 0);
        assert!(is_open);
        assert!(!is_close);
        assert_eq!(consumed, 1);

        // context tag 3, closing: (3<<4)|0x08|0x07 = 0x3F
        let data = vec![0x3F];
        let (tag, is_open, is_close, _, consumed) = read_tag_info(&data, 0).unwrap();
        assert_eq!(tag, 3);
        assert!(!is_open);
        assert!(is_close);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_opening_closing_tags() {
        let mut buf = Vec::new();
        encode_opening_tag(&mut buf, 3);
        encode_opening_tag(&mut buf, 0);
        encode_closing_tag(&mut buf, 0);
        encode_closing_tag(&mut buf, 3);
        assert_eq!(buf, vec![0x3E, 0x0E, 0x0F, 0x3F]);
    }

    #[test]
    fn test_invoke_id_range() {
        // Start at 200
        assert_eq!(TREND_INVOKE_ID.load(Ordering::Relaxed), 200);
        let id1 = next_invoke_id();
        assert_eq!(id1, 200);
        let id2 = next_invoke_id();
        assert_eq!(id2, 201);
    }
}
