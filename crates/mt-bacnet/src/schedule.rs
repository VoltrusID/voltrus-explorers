//! BACnet Schedule browser module.
//!
//! Provides parsing for BACnet Schedule object properties using ReadProperty.
//!
//! A Schedule object defines a weekly schedule of time-value pairs for each day
//! of the week, plus an optional default value and effective period.
//!
//! # Protocol Details
//!
//! Uses BACnet ReadProperty (confirmed service choice 12) to read:
//! - WeeklySchedule (property 123) — 7 days of time-value pairs
//! - ScheduleDefault (property 174) — fallback value
//! - EffectivePeriod (property 32) — date range when schedule is active
//! - ListOfObjectPropertyReferences (property 350) — referenced properties

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::ObjectType;
use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

// ---------------------------------------------------------------------------
// Invoke ID counter (separate from alarm.rs and cov.rs)
// ---------------------------------------------------------------------------

/// Start at 250 to stay out of alarm.rs (100+) and cov.rs (10+) ranges.
static SCHED_INVOKE_ID: AtomicU8 = AtomicU8::new(250);

fn next_invoke_id() -> u8 {
    SCHED_INVOKE_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single time-value entry within a day's schedule.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleEntry {
    /// Time of day as "HH:MM:SS".
    pub time: String,
    /// Display value at this time.
    pub value: String,
}

/// One day's schedule containing time-value entries.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DaySchedule {
    /// Day name: "Monday", "Tuesday", etc.
    pub day_name: String,
    /// Ordered list of time-value entries for this day.
    pub entries: Vec<ScheduleEntry>,
}

/// Complete schedule data for a BACnet Schedule object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleData {
    /// Object instance number of the Schedule object.
    pub object_instance: u32,
    /// Weekly schedule (up to 7 days, Mon-Sun).
    pub weekly_schedule: Vec<DaySchedule>,
    /// Default value when no schedule entry is active.
    pub schedule_default: Option<String>,
    /// Start date of the effective period (YYYY-MM-DD).
    pub effective_period_start: Option<String>,
    /// End date of the effective period (YYYY-MM-DD).
    pub effective_period_end: Option<String>,
    /// Referenced object properties as strings.
    pub object_property_references: Vec<String>,
}

// ---------------------------------------------------------------------------
// Day names
// ---------------------------------------------------------------------------

const DAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

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

/// Encode a context-tagged BACnet object identifier (4-byte packed value).
fn encode_context_object_id(buf: &mut Vec<u8>, tag_num: u8, obj_type: ObjectType, instance: u32) {
    let type_u32: u32 = obj_type.into();
    let value = (type_u32 << 22) | (instance & 0x3FFFFF);
    buf.push((tag_num << 4) | 0x0C); // context tag, length 4
    buf.extend_from_slice(&value.to_be_bytes());
}

// ---------------------------------------------------------------------------
// BER decoding helpers
// ---------------------------------------------------------------------------

/// Read the tag byte and determine tag number, is_opening, is_closing.
/// Returns `(tag_number, is_opening, is_closing, consumed)`.
fn read_tag_info(data: &[u8], pos: usize) -> Option<(u8, bool, bool, usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let tag_num = (byte >> 4) & 0x0F;
    let is_context = (byte & 0x08) != 0;
    let is_opening = is_context && (byte & 0x07) == 0x06;
    let is_closing = is_context && (byte & 0x07) == 0x07;

    if is_opening || is_closing {
        return Some((tag_num, is_opening, is_closing, 1));
    }

    // Primitive: compute total consumed (header + value length)
    let length = (byte & 0x07) as usize;
    let consumed = if length == 5 {
        // Extended length in next byte
        if pos + 1 >= data.len() {
            return None;
        }
        let ext_len = data[pos + 1] as usize;
        2 + ext_len
    } else if length == 6 {
        // Also extended (some encodings use 6 for length-in-next-byte)
        if pos + 1 >= data.len() {
            return None;
        }
        let ext_len = data[pos + 1] as usize;
        2 + ext_len
    } else {
        1 + length
    };

    Some((tag_num, false, false, consumed))
}

/// Advance `pos` past a single TLV element. Returns the new position.
fn skip_tag(data: &[u8], pos: usize) -> usize {
    if let Some((_, _, _, consumed)) = read_tag_info(data, pos) {
        pos + consumed
    } else {
        pos + 1
    }
}

/// Read the value portion of an application-tagged element.
/// Returns `(app_tag_number, value_bytes, total_consumed)`.
fn read_application_tag(data: &[u8], pos: usize) -> Option<(u8, &[u8], usize)> {
    if pos >= data.len() {
        return None;
    }
    let byte = data[pos];
    let is_context = (byte & 0x08) != 0;
    if is_context {
        return None;
    }

    let app_tag = (byte >> 4) & 0x0F;
    let len_bits = byte & 0x07;

    let (length, header_size) = if len_bits == 6 {
        // Extended length: next byte is length
        if pos + 1 >= data.len() {
            return None;
        }
        let len = data[pos + 1] as usize;
        (len, 2)
    } else if len_bits == 7 {
        return None;
    } else {
        (len_bits as usize, 1)
    };

    let value_start = pos + header_size;
    if value_start + length > data.len() {
        return None;
    }

    Some((app_tag, &data[value_start..value_start + length], header_size + length))
}

// ---------------------------------------------------------------------------
// Parse helpers
// ---------------------------------------------------------------------------

/// Parse a BACnetTime (4 bytes: hour, minute, second, hundredths) at `pos`.
/// Returns "HH:MM:SS" and new position.
fn parse_bacnet_time(data: &[u8], pos: usize) -> (String, usize) {
    if pos + 4 > data.len() {
        return (String::new(), pos);
    }
    let hour = data[pos];
    let minute = data[pos + 1];
    let second = data[pos + 2];
    // hundredths = data[pos + 3] — not displayed
    let time_str = format!("{hour:02}:{minute:02}:{second:02}");
    (time_str, pos + 4)
}

/// Parse a BACnetDate (4 bytes: year MSB, year LSB, month, day, weekday)
/// at `pos`. Returns "YYYY-MM-DD" and new position.
fn parse_bacnet_date(data: &[u8], pos: usize) -> (String, usize) {
    if pos + 4 > data.len() {
        return (String::new(), pos);
    }
    let year = ((data[pos] as u16) << 8) | data[pos + 1] as u16;
    let month = data[pos + 2];
    let day = data[pos + 3];
    // weekday = data[pos + 4] if present, but standard BACnetDate is 4 bytes
    let date_str = format!("{year}-{month:02}-{day:02}");
    (date_str, pos + 4)
}

/// Parse an application-tagged value at `pos` into a display string.
/// Returns `(display_string, new_position)`.
fn parse_application_value(data: &[u8], pos: usize) -> (String, usize) {
    if let Some((app_tag, value_bytes, consumed)) = read_application_tag(data, pos) {
        let display = match app_tag {
            // Null
            0 => "Null".into(),
            // Boolean
            1 => {
                if !value_bytes.is_empty() && value_bytes[0] != 0 {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            // Unsigned integer
            2 => bytes_to_u32(value_bytes).to_string(),
            // Signed integer
            3 => bytes_to_i32(value_bytes).to_string(),
            // Real (4-byte IEEE 754)
            4 if value_bytes.len() == 4 => {
                let bytes: [u8; 4] = value_bytes.try_into().unwrap_or([0; 4]);
                let f = f32::from_be_bytes(bytes);
                format!("{f:.4}")
            }
            // Double (8-byte IEEE 754)
            5 if value_bytes.len() == 8 => {
                let bytes: [u8; 8] = value_bytes.try_into().unwrap_or([0; 8]);
                let f = f64::from_be_bytes(bytes);
                format!("{f:.6}")
            }
            // Enumerated
            9 => bytes_to_u32(value_bytes).to_string(),
            // Character String (tag 7)
            7 if value_bytes.len() >= 1 => {
                // First byte is encoding (0 = ISO-8859-1)
                if value_bytes[0] == 0 && value_bytes.len() > 1 {
                    String::from_utf8_lossy(&value_bytes[1..]).into_owned()
                } else {
                    String::from_utf8_lossy(value_bytes).into_owned()
                }
            }
            // Octet String (tag 10)
            10 => value_bytes
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect(),
            // Object Identifier (tag 12)
            12 if value_bytes.len() == 4 => {
                let raw = u32::from_be_bytes(
                    value_bytes.try_into().unwrap_or([0; 4]),
                );
                let type_val = (raw >> 22) & 0x3FF;
                let instance = raw & 0x3FFFFF;
                format!("object_{type_val}:{instance}")
            }
            _ => format!("unknown_tag_{app_tag}"),
        };
        (display, pos + consumed)
    } else {
        (String::new(), skip_tag(data, pos))
    }
}

/// Parse an application-tagged value and also try to read it as BACnetTime
/// (4 raw bytes). Used when the tag is context-encoded inside a daily schedule.
fn parse_schedule_value(data: &[u8], pos: usize) -> (String, usize) {
    if pos >= data.len() {
        return (String::new(), pos);
    }
    let byte = data[pos];
    let is_context = (byte & 0x08) != 0;

    if is_context {
        // Context-tagged value inside schedule entry — try to extract as unsigned
        let tag_num = (byte >> 4) & 0x0F;
        let len_bits = byte & 0x07;

        let (length, header_size) = if len_bits == 6 || (byte & 0x04) != 0 && len_bits == 0 {
            // Extended length
            if pos + 1 >= data.len() {
                return (String::new(), skip_tag(data, pos));
            }
            let ext_len = data[pos + 1] as usize;
            (ext_len, 2)
        } else if len_bits == 7 {
            return (String::new(), skip_tag(data, pos));
        } else {
            (len_bits as usize, 1)
        };

        let value_start = pos + header_size;
        if value_start + length > data.len() {
            return (String::new(), skip_tag(data, pos));
        }

        // The value is the application-encoded data within the context wrapper
        let inner = &data[value_start..value_start + length];
        if inner.is_empty() {
            return (String::new(), pos + header_size + length);
        }

        // Decode inner application tag
        let inner_tag = inner[0];
        let inner_app = (inner_tag >> 4) & 0x0F;
        let inner_len = inner_tag & 0x07;
        let inner_is_context = (inner_tag & 0x08) != 0;

        if inner_is_context {
            // Nested context tag — just skip
            return (format!("ctx_tag_{tag_num}"), pos + header_size + length);
        }

        let (data_len, data_offset) = if inner_len == 6 {
            if inner.len() < 2 {
                return (String::new(), pos + header_size + length);
            }
            (inner[1] as usize, 2)
        } else if inner_len == 7 {
            return (String::new(), pos + header_size + length);
        } else {
            (inner_len as usize, 1)
        };

        if data_offset + data_len > inner.len() {
            return (String::new(), pos + header_size + length);
        }

        let val_bytes = &inner[data_offset..data_offset + data_len];
        let display = match inner_app {
            0 => "Null".into(),
            1 => {
                if !val_bytes.is_empty() && val_bytes[0] != 0 {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            2 => bytes_to_u32(val_bytes).to_string(),
            3 => bytes_to_i32(val_bytes).to_string(),
            4 if val_bytes.len() == 4 => {
                let b: [u8; 4] = val_bytes.try_into().unwrap_or([0; 4]);
                format!("{:.4}", f32::from_be_bytes(b))
            }
            5 if val_bytes.len() == 8 => {
                let b: [u8; 8] = val_bytes.try_into().unwrap_or([0; 8]);
                format!("{:.6}", f64::from_be_bytes(b))
            }
            9 => bytes_to_u32(val_bytes).to_string(),
            7 if val_bytes.len() >= 1 => {
                if val_bytes[0] == 0 && val_bytes.len() > 1 {
                    String::from_utf8_lossy(&val_bytes[1..]).into_owned()
                } else {
                    String::from_utf8_lossy(val_bytes).into_owned()
                }
            }
            _ => format!("tag_{inner_app}"),
        };

        (display, pos + header_size + length)
    } else {
        // Application-tagged — parse directly
        parse_application_value(data, pos)
    }
}

/// Parse the WeeklySchedule from raw service data bytes.
///
/// WeeklySchedule ::= SEQUENCE OF [7] BACnetDailySchedule
/// BACnetDailySchedule ::= SEQUENCE OF BACnetTimeValue
/// BACnetTimeValue ::= SEQUENCE {
///     Time  BACnetTime (application tag, 4 bytes),
///     Value (application-tagged)
/// }
///
/// The data may be wrapped in opening/closing tags or be a flat sequence.
fn parse_weekly_schedule(data: &[u8]) -> Vec<DaySchedule> {
    let mut pos = 0;
    let mut days = Vec::new();

    // Check for outer opening tag (context tag constructed)
    if let Some((_, is_opening, _, _)) = read_tag_info(data, pos) {
        if is_opening {
            pos += 1; // skip opening tag
        }
    }

    for day_idx in 0..7 {
        if pos >= data.len() {
            break;
        }

        let mut entries = Vec::new();

        // Check for inner opening tag (constructed)
        let had_inner_open = if let Some((_, is_opening, is_closing, _)) = read_tag_info(data, pos)
        {
            if is_opening {
                pos += 1;
                true
            } else if is_closing {
                // This is the outer closing tag — we're done
                break;
            } else {
                false
            }
        } else {
            false
        };

        // Read time-value pairs until inner closing tag or next opening tag or end
        while pos < data.len() {
            if let Some((_, _, is_closing, _)) = read_tag_info(data, pos) {
                if is_closing {
                    pos += 1;
                    break;
                }
            }

            // Check for next day's opening tag — stop current day
            if let Some((_, is_opening, _, _)) = read_tag_info(data, pos) {
                if is_opening && had_inner_open {
                    break;
                }
            }

            // Try to read a BACnetTimeValue sequence
            // The Time is application-tagged: tag byte where low nibble encodes length.
            // BACnetTime is always 4 bytes with application tag 10 (0x0A... but no).
            // Actually BACnetTime is just 4 bytes. In the TLV encoding:
            //   - It could be context-tagged with the Time as primitive value
            //   - Or it could be the raw 4 bytes within a context-tagged wrapper

            // Look for a SEQUENCE wrapper (context tag constructed) for TimeValue
            if let Some((_, is_opening, _, consumed)) = read_tag_info(data, pos) {
                if is_opening {
                    // TimeValue is wrapped in opening/closing tags
                    pos += consumed;
                    // Read Time (should be context tag 0 with 4 bytes, or application Time)
                    let (time_str, new_pos) = read_time_value_time(data, pos);
                    pos = new_pos;
                    // Read Value
                    let (value_str, new_pos) = parse_schedule_value(data, pos);
                    pos = new_pos;
                    // Skip closing tag
                    if let Some((_, _, is_closing, consumed)) = read_tag_info(data, pos) {
                        if is_closing {
                            pos += consumed;
                        }
                    }
                    entries.push(ScheduleEntry {
                        time: time_str,
                        value: value_str,
                    });
                    continue;
                }
            }

            // Flat format: time bytes directly, then value
            // Check if this looks like a time (context tag with 4 bytes of hour/min/sec/hundredths)
            let (time_str, new_pos) = read_time_value_time(data, pos);
            if time_str.is_empty() || new_pos == pos {
                // Not a time — skip this element
                pos = skip_tag(data, pos);
                continue;
            }
            pos = new_pos;

            // Read the value following the time
            let (value_str, new_pos) = parse_schedule_value(data, pos);
            pos = new_pos;

            entries.push(ScheduleEntry {
                time: time_str,
                value: value_str,
            });
        }

        let day_name = if day_idx < DAY_NAMES.len() {
            DAY_NAMES[day_idx].to_string()
        } else {
            format!("Day {}", day_idx + 1)
        };

        days.push(DaySchedule {
            day_name,
            entries,
        });
    }

    days
}

/// Try to read a BACnetTime from the current position.
/// The time could be:
/// - Context tag 0 with 4 raw bytes (hour, min, sec, hundredths)
/// - Application-tagged with 4 bytes
fn read_time_value_time(data: &[u8], pos: usize) -> (String, usize) {
    if pos >= data.len() {
        return (String::new(), pos);
    }

    let byte = data[pos];
    let is_context = (byte & 0x08) != 0;
    let tag_num = (byte >> 4) & 0x0F;

    if is_context {
        // Context-tagged time: tag_num, length in low bits
        let len_bits = byte & 0x07;
        let (length, header_size) = if len_bits == 6 || (byte & 0x04) != 0 && len_bits == 0 {
            if pos + 1 >= data.len() {
                return (String::new(), pos);
            }
            (data[pos + 1] as usize, 2)
        } else if len_bits == 7 {
            return (String::new(), pos);
        } else {
            (len_bits as usize, 1)
        };

        let value_start = pos + header_size;
        if length == 4 && value_start + 4 <= data.len() {
            return parse_bacnet_time(data, value_start);
        }
        (String::new(), pos + header_size + length)
    } else {
        // Application-tagged: check if it's 4 bytes that look like time
        let _app_tag = tag_num;
        let len_bits = byte & 0x07;
        let (length, header_size) = if len_bits == 6 {
            if pos + 1 >= data.len() {
                return (String::new(), pos);
            }
            (data[pos + 1] as usize, 2)
        } else if len_bits == 7 {
            return (String::new(), pos);
        } else {
            (len_bits as usize, 1)
        };

        let value_start = pos + header_size;
        if length == 4 && value_start + 4 <= data.len() {
            let h = data[value_start];
            let m = data[value_start + 1];
            let s = data[value_start + 2];
            // Validate: hour < 24, minute < 60, second < 60
            if h < 24 && m < 60 && s < 60 {
                return parse_bacnet_time(data, value_start);
            }
        }
        (String::new(), pos + header_size + length)
    }
}

/// Parse ScheduleDefault from raw service data.
/// ScheduleDefault is a single application-tagged value.
fn parse_schedule_default(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    let (value, _) = parse_application_value(data, 0);
    if value.is_empty() || value == "Null" {
        None
    } else {
        Some(value)
    }
}

/// Parse EffectivePeriod from raw service data.
/// EffectivePeriod is a SEQUENCE containing:
///   [0] BACnetDate (start date)
///   [1] BACnetDate (end date)
fn parse_effective_period(data: &[u8]) -> (Option<String>, Option<String>) {
    let mut pos = 0;

    // Skip opening tag if present
    if let Some((_, is_opening, _, consumed)) = read_tag_info(data, pos) {
        if is_opening {
            pos += consumed;
        }
    }

    // [0] Start date — context tag 0, 4 bytes
    let start_date = if let Some((tag_num, _, _, consumed)) = read_tag_info(data, pos) {
        if tag_num == 0 && (data[pos] & 0x08) != 0 {
            let len_bits = data[pos] & 0x07;
            let value_start = pos + 1;
            if len_bits == 4 && value_start + 4 <= data.len() {
                let (date_str, new_pos) = parse_bacnet_date(data, value_start);
                pos = new_pos;
                Some(date_str)
            } else if (data[pos] & 0x04) != 0 && len_bits == 0 && pos + 1 < data.len() {
                // Extended length
                let ext_len = data[pos + 1] as usize;
                let vs = pos + 2;
                if ext_len == 4 && vs + 4 <= data.len() {
                    let (date_str, new_pos) = parse_bacnet_date(data, vs);
                    pos = new_pos;
                    Some(date_str)
                } else {
                    pos += consumed;
                    None
                }
            } else {
                pos += consumed;
                None
            }
        } else {
            pos += consumed;
            None
        }
    } else {
        None
    };

    // [1] End date — context tag 1, 4 bytes
    let end_date = if let Some((tag_num, _, _, _consumed)) = read_tag_info(data, pos) {
        if tag_num == 1 && (data[pos] & 0x08) != 0 {
            let len_bits = data[pos] & 0x07;
            let value_start = pos + 1;
            if len_bits == 4 && value_start + 4 <= data.len() {
                let (date_str, _new_pos) = parse_bacnet_date(data, value_start);
                Some(date_str)
            } else if (data[pos] & 0x04) != 0 && len_bits == 0 && pos + 1 < data.len() {
                let ext_len = data[pos + 1] as usize;
                let vs = pos + 2;
                if ext_len == 4 && vs + 4 <= data.len() {
                    let (date_str, _) = parse_bacnet_date(data, vs);
                    Some(date_str)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    (start_date, end_date)
}

/// Parse ListOfObjectPropertyReferences from raw service data.
/// Each reference is: ObjectIdentifier + PropertyIdentifier (context tags 0 and 1).
fn parse_object_property_references(data: &[u8]) -> Vec<String> {
    let mut pos = 0;
    let mut refs = Vec::new();

    // Skip outer opening tag if present
    if let Some((_, is_opening, _, consumed)) = read_tag_info(data, pos) {
        if is_opening {
            pos += consumed;
        }
    }

    while pos < data.len() {
        // Check for closing tag
        if let Some((_, _, is_closing, _)) = read_tag_info(data, pos) {
            if is_closing {
                break;
            }
        }

        // Check for opening tag (next reference entry)
        if let Some((_, is_opening, _, consumed)) = read_tag_info(data, pos) {
            if is_opening {
                pos += consumed;
            }
        }

        // [0] ObjectIdentifier — context tag 0, 4 bytes
        let obj_ref = if let Some((tag_num, _, _, consumed)) = read_tag_info(data, pos) {
            if tag_num == 0 && (data[pos] & 0x08) != 0 {
                let len_bits = data[pos] & 0x07;
                let value_start = pos + 1;
                if len_bits == 4 && value_start + 4 <= data.len() {
                    let raw = u32::from_be_bytes([
                        data[value_start],
                        data[value_start + 1],
                        data[value_start + 2],
                        data[value_start + 3],
                    ]);
                    let type_val = (raw >> 22) & 0x3FF;
                    let instance = raw & 0x3FFFFF;
                    pos = value_start + 4;
                    Some(format!("object_{type_val}:{instance}"))
                } else {
                    pos += consumed;
                    None
                }
            } else {
                pos += consumed;
                None
            }
        } else {
            pos += 1;
            None
        };

        // [1] PropertyIdentifier — context tag 1, 1-4 bytes (enumerated)
        let prop_ref = if let Some((tag_num, _, _, consumed)) = read_tag_info(data, pos) {
            if tag_num == 1 && (data[pos] & 0x08) != 0 {
                let len_bits = data[pos] & 0x07;
                let value_start = pos + 1;
                if value_start + (len_bits as usize) <= data.len() && len_bits > 0 {
                    let val = match len_bits {
                        1 => data[value_start] as u32,
                        2 => ((data[value_start] as u32) << 8) | data[value_start + 1] as u32,
                        4 => u32::from_be_bytes([
                            data[value_start],
                            data[value_start + 1],
                            data[value_start + 2],
                            data[value_start + 3],
                        ]),
                        _ => 0
                    };
                    pos = value_start + (len_bits as usize);
                    Some(val.to_string())
                } else {
                    pos += consumed;
                    None
                }
            } else {
                pos += consumed;
                None
            }
        } else {
            pos += 1;
            None
        };

        // Skip closing tag if present
        if let Some((_, _, is_closing, consumed)) = read_tag_info(data, pos) {
            if is_closing {
                pos += consumed;
            }
        }

        match (obj_ref, prop_ref) {
            (Some(obj), Some(prop)) => refs.push(format!("{obj}.{prop}")),
            (Some(obj), None) => refs.push(obj),
            _ => {}
        }
    }

    refs
}

// ---------------------------------------------------------------------------
// Send confirmed request helper
// ---------------------------------------------------------------------------

/// Send a ReadProperty confirmed request and return the raw service data.
fn send_read_property(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_type: ObjectType,
    object_instance: u32,
    property_id: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let invoke_id = next_invoke_id();

    // Build ReadProperty service data:
    //   context-tag 0: ObjectIdentifier (Schedule type=17, given instance)
    //   context-tag 1: PropertyIdentifier (enumerated value)
    let mut service_data = Vec::new();
    encode_context_object_id(&mut service_data, 0, object_type, object_instance);
    encode_context_unsigned(&mut service_data, 1, property_id);

    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::ReadProperty,
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
        .map_err(|e| ProtocolError::WriteFailed(format!("Schedule send ReadProperty: {e}")))?;

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
            Err(e) => return Err(ProtocolError::ReadFailed(format!("Schedule recv: {e}"))),
        }
    }

    Err(ProtocolError::Timeout(format!(
        "BACnet ReadProperty timeout for schedule at {target_addr}"
    )))
}

// ---------------------------------------------------------------------------
// Strip ReadProperty response wrapper
// ---------------------------------------------------------------------------

/// A ReadProperty response wraps the value in:
///   context-tag 0: PropertyIdentifier (enumerated)
///   context-tag 1: ArrayIndex (optional, unsigned)
///   application-tagged: the actual value
///
/// This helper strips the wrapper and returns just the value data.
fn strip_read_property_response(data: &[u8]) -> Vec<u8> {
    let mut pos = 0;

    // Skip context-tag 0 (PropertyIdentifier)
    pos = skip_tag(data, pos);
    // Skip optional context-tag 1 (ArrayIndex)
    if pos < data.len() {
        let byte = data[pos];
        let tag_num = (byte >> 4) & 0x0F;
        let is_context = (byte & 0x08) != 0;
        if is_context && tag_num == 1 {
            pos = skip_tag(data, pos);
        }
    }

    // The remaining bytes are the value
    if pos < data.len() {
        data[pos..].to_vec()
    } else {
        data.to_vec()
    }
}

// ---------------------------------------------------------------------------
// Main public API
// ---------------------------------------------------------------------------

/// Read all schedule properties from a BACnet Schedule object.
///
/// Reads WeeklySchedule, ScheduleDefault, EffectivePeriod, and
/// ListOfObjectPropertyReferences using individual ReadProperty requests.
///
/// Uses `ObjectType::Schedule` (type 17) for all requests.
pub fn read_schedule_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_instance: u32,
) -> Result<ScheduleData, ProtocolError> {
    // Property IDs:
    // WeeklySchedule = 123
    // ScheduleDefault = 174
    // EffectivePeriod = 32
    // ListOfObjectPropertyReferences = 350

    // Read WeeklySchedule
    let weekly_schedule = match send_read_property(
        socket,
        target_addr,
        ObjectType::Schedule,
        object_instance,
        123,
    ) {
        Ok(data) => {
            let stripped = strip_read_property_response(&data);
            parse_weekly_schedule(&stripped)
        }
        Err(e) => {
            // If WeeklySchedule read fails, return empty schedule
            // rather than failing the entire operation
            eprintln!("Warning: WeeklySchedule read failed: {e}");
            Vec::new()
        }
    };

    // Read ScheduleDefault
    let schedule_default = match send_read_property(
        socket,
        target_addr,
        ObjectType::Schedule,
        object_instance,
        174,
    ) {
        Ok(data) => {
            let stripped = strip_read_property_response(&data);
            parse_schedule_default(&stripped)
        }
        Err(_) => None,
    };

    // Read EffectivePeriod
    let (effective_period_start, effective_period_end) = match send_read_property(
        socket,
        target_addr,
        ObjectType::Schedule,
        object_instance,
        32,
    ) {
        Ok(data) => {
            let stripped = strip_read_property_response(&data);
            parse_effective_period(&stripped)
        }
        Err(_) => (None, None),
    };

    // Read ListOfObjectPropertyReferences
    let object_property_references = match send_read_property(
        socket,
        target_addr,
        ObjectType::Schedule,
        object_instance,
        350,
    ) {
        Ok(data) => {
            let stripped = strip_read_property_response(&data);
            parse_object_property_references(&stripped)
        }
        Err(_) => Vec::new(),
    };

    Ok(ScheduleData {
        object_instance,
        weekly_schedule,
        schedule_default,
        effective_period_start,
        effective_period_end,
        object_property_references,
    })
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Convert raw bytes to a big-endian u32.
fn bytes_to_u32(bytes: &[u8]) -> u32 {
    match bytes.len() {
        0 => 0,
        1 => bytes[0] as u32,
        2 => ((bytes[0] as u32) << 8) | (bytes[1] as u32),
        3 => ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | (bytes[2] as u32),
        _ => u32::from_be_bytes(
            bytes[..4.min(bytes.len())]
                .try_into()
                .unwrap_or([0; 4]),
        ),
    }
}

/// Convert raw bytes to a big-endian i32.
fn bytes_to_i32(bytes: &[u8]) -> i32 {
    bytes_to_u32(bytes) as i32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bacnet_time() {
        // 14:30:00.00
        let data = [14, 30, 0, 0];
        let (time, pos) = parse_bacnet_time(&data, 0);
        assert_eq!(time, "14:30:00");
        assert_eq!(pos, 4);
    }

    #[test]
    fn test_parse_bacnet_time_midnight() {
        let data = [0, 0, 0, 0];
        let (time, pos) = parse_bacnet_time(&data, 0);
        assert_eq!(time, "00:00:00");
        assert_eq!(pos, 4);
    }

    #[test]
    fn test_parse_bacnet_date() {
        // 2024-06-15
        let data = [0x07, 0xE8, 0x06, 0x0F]; // year=2024, month=6, day=15
        let (date, pos) = parse_bacnet_date(&data, 0);
        assert_eq!(date, "2024-06-15");
        assert_eq!(pos, 4);
    }

    #[test]
    fn test_parse_application_value_real() {
        // Application tag 4 (Real), length 4: value 72.0
        let data = vec![0x44, 0x42, 0x90, 0x00, 0x00];
        let (val, pos) = parse_application_value(&data, 0);
        assert_eq!(val, "72.0000");
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_parse_application_value_unsigned() {
        // Application tag 2 (Unsigned), length 1: value 42
        let data = vec![0x21, 42];
        let (val, pos) = parse_application_value(&data, 0);
        assert_eq!(val, "42");
        assert_eq!(pos, 2);
    }

    #[test]
    fn test_parse_application_value_boolean() {
        // Application tag 1 (Boolean), length 1: true
        let data = vec![0x11, 0x01];
        let (val, pos) = parse_application_value(&data, 0);
        assert_eq!(val, "true");
        assert_eq!(pos, 2);
    }

    #[test]
    fn test_parse_application_value_enumerated() {
        // Application tag 9 (Enumerated), length 1: value 3
        let data = vec![0x91, 3];
        let (val, pos) = parse_application_value(&data, 0);
        assert_eq!(val, "3");
        assert_eq!(pos, 2);
    }

    #[test]
    fn test_encode_context_unsigned() {
        let mut buf = Vec::new();
        encode_context_unsigned(&mut buf, 0, 42);
        assert_eq!(buf, vec![0x09, 42]);
    }

    #[test]
    fn test_encode_context_object_id() {
        let mut buf = Vec::new();
        encode_context_object_id(&mut buf, 0, ObjectType::Schedule, 1);
        // Schedule = type 17 => (17 << 22) | 1 = 0x04400001
        assert_eq!(buf, vec![0x0C, 0x04, 0x40, 0x00, 0x01]);
    }

    #[test]
    fn test_strip_read_property_response() {
        // Context tag 0 (property identifier), context tag 1 (array index),
        // then application-tagged value (Real 72.0)
        let data = vec![
            0x09, 123,       // context tag 0, value 123 (WeeklySchedule)
            0x19, 0x00,      // context tag 1, value 0 (array index)
            0x44, 0x42, 0x90, 0x00, 0x00, // application tag 4 (Real), 72.0
        ];
        let stripped = strip_read_property_response(&data);
        assert_eq!(stripped, vec![0x44, 0x42, 0x90, 0x00, 0x00]);
    }

    #[test]
    fn test_skip_tag_primitive() {
        let data = vec![0x09, 42]; // context tag 0, length 1
        assert_eq!(skip_tag(&data, 0), 2);
    }

    #[test]
    fn test_skip_tag_opening() {
        let data = vec![0x0E]; // context tag 0, opening
        assert_eq!(skip_tag(&data, 0), 1);
    }

    #[test]
    fn test_bytes_to_u32() {
        assert_eq!(bytes_to_u32(&[1]), 1);
        assert_eq!(bytes_to_u32(&[0, 1]), 1);
        assert_eq!(bytes_to_u32(&[0, 0, 0, 1]), 1);
        assert_eq!(bytes_to_u32(&[]), 0);
    }
}
