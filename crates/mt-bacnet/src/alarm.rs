//! BACnet Alarm and Event service module.
//!
//! Provides custom BER encoding/decoding for BACnet alarm and event services
//! (GetAlarmSummary, GetEventInformation, AcknowledgeAlarm, EventNotification).
//!
//! The `bacnet-rs` crate only defines enum values for these services -- no
//! request/response structs. We implement the BER encoding ourselves following
//! ASHRAE 135 clauses 13 (alarm and event services) and 20.2 (encoding).
//!
//! Requests go through [`crate::transport::confirmed_request`], which owns the
//! invoke IDs, segmentation and Error/Reject/Abort handling.

use std::net::SocketAddr;

use bacnet_rs::object::ObjectType;
use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

use crate::helpers::{object_type_from_str, object_type_to_string};
use crate::transport::{confirmed_request, REQUEST_TIMEOUT};

/// Upper bound on GetEventInformation pages followed in one call.
const MAX_EVENT_PAGES: usize = 256;

/// Longest acknowledgment source we encode, in bytes of UTF-8. Keeps the request
/// inside the smallest APDU every device must accept (50 octets of header and
/// fixed fields plus this text fits in 480).
const MAX_ACK_SOURCE_BYTES: usize = 200;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// BACnetTime. `None` = unspecified (0xFF) in that field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BacnetTime {
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
    pub hundredths: Option<u8>,
}

/// BACnetDate. `None` = unspecified (0xFF) in that field. `year` is the full
/// year (the wire carries year - 1900 in one octet). `month`, `day` and
/// `weekday` keep their wire values, including the special values of clause
/// 20.2.12 (13 = odd months, 32 = last day, ...). Weekday: 1 = Monday ... 7 = Sunday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BacnetDate {
    pub year: Option<u16>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    pub weekday: Option<u8>,
}

/// BACnetTimeStamp CHOICE (clause 21): a time, a sequence number or a date+time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventTimeStamp {
    /// `[0] Time`
    Time { time: BacnetTime },
    /// `[1] Unsigned`
    SequenceNumber { sequence_number: u32 },
    /// `[2] BACnetDateTime`
    DateTime { date: BacnetDate, time: BacnetTime },
}

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

/// A single entry returned by GetEventInformation (BACnetEventSummary).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventInfoEntry {
    /// Object identifier string, e.g. "analogInput:1".
    pub object_identifier: String,
    /// Object type string.
    pub object_type: String,
    /// Numeric BACnetObjectType.
    pub object_type_id: u32,
    /// Object instance number.
    pub object_instance: u32,
    /// Current event state.
    pub event_state: String,
    /// Acknowledged transitions as CSV.
    pub acknowledged_transitions: String,
    /// Enabled transitions as CSV.
    pub event_enable: String,
    /// Event time stamps rendered for display, in the order
    /// [to-offnormal, to-fault, to-normal].
    pub event_time_stamps: Vec<String>,
    /// Event time stamps as received, in the order
    /// [to-offnormal, to-fault, to-normal].
    pub transition_time_stamps: Vec<EventTimeStamp>,
    /// Notify type: "alarm", "event" or "ackNotification".
    pub notify_type: String,
    /// Event priorities for [to-offnormal, to-fault, to-normal].
    pub event_priorities: Vec<u32>,
}

/// Result of GetEventInformation across all pages.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventInformationResponse {
    /// Event summaries returned by the device.
    pub entries: Vec<EventInfoEntry>,
    /// The device's moreEvents flag after the last page fetched. Always false
    /// from [`get_event_information_blocking`], which follows every page.
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
    /// Current event state (the notification's toState).
    pub event_state: String,
    /// Previous state (fromState; "unknown" if the device omitted it).
    pub from_state: String,
    /// New state (toState).
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
// Name helpers
// ---------------------------------------------------------------------------

/// BACnetEventState name (clause 21): normal 0, fault 1, offnormal 2,
/// high-limit 3, low-limit 4, life-safety-alarm 5.
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

/// BACnetNotifyType name.
fn notify_type_name(val: u32) -> String {
    match val {
        0 => "alarm".into(),
        1 => "event".into(),
        2 => "ackNotification".into(),
        v => format!("unknown({v})"),
    }
}

/// Render BACnetEventTransitionBits as CSV. `bits` uses bit 0 = to-offnormal,
/// bit 1 = to-fault, bit 2 = to-normal (see [`transition_bits`]).
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

/// Map a UI/DTO object type string to an [`ObjectType`]. Falls back to the
/// full BACnetObjectType range so types outside `helpers::object_type_from_str`
/// (event-enrollment, loop, ...) can still be acknowledged.
fn resolve_object_type(name: &str) -> Result<ObjectType, ProtocolError> {
    if let Some(t) = object_type_from_str(name) {
        return Ok(t);
    }
    (0u32..1024)
        .map(ObjectType::from)
        .find(|t| object_type_to_string(t).eq_ignore_ascii_case(name))
        .ok_or_else(|| ProtocolError::Other(format!("Unknown BACnet object type '{name}'")))
}

// ---------------------------------------------------------------------------
// Date / time helpers
// ---------------------------------------------------------------------------

fn opt(byte: u8) -> Option<u8> {
    (byte != 0xFF).then_some(byte)
}

fn raw(value: Option<u8>) -> u8 {
    value.unwrap_or(0xFF)
}

impl BacnetTime {
    fn from_bytes(b: &[u8]) -> Decoded<Self> {
        match b {
            [h, m, s, hs] => Ok(Self {
                hour: opt(*h),
                minute: opt(*m),
                second: opt(*s),
                hundredths: opt(*hs),
            }),
            _ => Err(format!("Time must be 4 octets, got {}", b.len())),
        }
    }

    fn to_bytes(self) -> [u8; 4] {
        [raw(self.hour), raw(self.minute), raw(self.second), raw(self.hundredths)]
    }

    fn describe(&self) -> String {
        fn field(v: Option<u8>) -> String {
            v.map_or_else(|| "**".to_string(), |v| format!("{v:02}"))
        }
        format!(
            "{}:{}:{}.{}",
            field(self.hour),
            field(self.minute),
            field(self.second),
            field(self.hundredths)
        )
    }
}

impl BacnetDate {
    fn from_bytes(b: &[u8]) -> Decoded<Self> {
        match b {
            [y, m, d, w] => Ok(Self {
                year: opt(*y).map(|y| 1900 + u16::from(y)),
                month: opt(*m),
                day: opt(*d),
                weekday: opt(*w),
            }),
            _ => Err(format!("Date must be 4 octets, got {}", b.len())),
        }
    }

    fn to_bytes(self) -> Result<[u8; 4], ProtocolError> {
        let year = match self.year {
            None => 0xFF,
            // 0xFF (year 2155) is the "unspecified" marker.
            Some(y) if (1900..=2154).contains(&y) => (y - 1900) as u8,
            Some(y) => {
                return Err(ProtocolError::Other(format!(
                    "Year {y} cannot be encoded in a BACnet Date (1900-2154)"
                )))
            }
        };
        Ok([year, raw(self.month), raw(self.day), raw(self.weekday)])
    }

    fn describe(&self) -> String {
        fn field(v: Option<u8>) -> String {
            v.map_or_else(|| "**".to_string(), |v| format!("{v:02}"))
        }
        let year = self
            .year
            .map_or_else(|| "****".to_string(), |y| format!("{y:04}"));
        format!("{year}-{}-{}", field(self.month), field(self.day))
    }
}

impl EventTimeStamp {
    /// Human-readable rendering; unspecified fields show as `*`.
    pub fn describe(&self) -> String {
        match self {
            Self::Time { time } => time.describe(),
            Self::SequenceNumber { sequence_number } => format!("#{sequence_number}"),
            Self::DateTime { date, time } => format!("{} {}", date.describe(), time.describe()),
        }
    }
}

/// Civil date (year, month 1-12, day 1-31) for a day count since 1970-01-01
/// (proleptic Gregorian; H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year from March 1 [0, 365]
    let mp = (5 * doy + 2) / 153; // month from March [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Convert Unix seconds to a BACnet date-time. The result is UTC: the standard
/// library has no timezone database and this crate has no date-time
/// dependency. The value only feeds `timeOfAcknowledgment`, which devices log
/// as informational text.
fn unix_to_date_time(secs: u64) -> EventTimeStamp {
    let days = (secs / 86_400) as i64;
    let time_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday; BACnet weekday 1 = Monday ... 7 = Sunday.
    let weekday = ((days + 3).rem_euclid(7) + 1) as u8;
    EventTimeStamp::DateTime {
        date: BacnetDate {
            year: Some(year as u16),
            month: Some(month),
            day: Some(day),
            weekday: Some(weekday),
        },
        time: BacnetTime {
            hour: Some((time_of_day / 3600) as u8),
            minute: Some((time_of_day % 3600 / 60) as u8),
            second: Some((time_of_day % 60) as u8),
            hundredths: Some(0),
        },
    }
}

fn current_time_stamp() -> EventTimeStamp {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_to_date_time(secs)
}

// ---------------------------------------------------------------------------
// BER encoding helpers
// ---------------------------------------------------------------------------

/// Encode a context-tagged unsigned integer in the minimum number of octets.
/// BACnet tag byte: [tag_num(4) | context(1) | length(3)]
fn encode_context_unsigned(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take(3).take_while(|b| **b == 0).count();
    let content = &bytes[skip..];
    buf.push((tag_num << 4) | 0x08 | content.len() as u8);
    buf.extend_from_slice(content);
}

/// Encode a context-tagged BACnet object identifier (4-byte packed value).
fn encode_context_object_id(buf: &mut Vec<u8>, tag_num: u8, obj_type: ObjectType, instance: u32) {
    let type_u32: u32 = obj_type.into();
    let value = (type_u32 << 22) | (instance & 0x3FFFFF);
    buf.push((tag_num << 4) | 0x08 | 4);
    buf.extend_from_slice(&value.to_be_bytes());
}

/// Encode a context-tagged enumerated value (identical to unsigned encoding).
fn encode_context_enumerated(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    encode_context_unsigned(buf, tag_num, value);
}

/// Encode a context-tagged CharacterString: one charset octet (0 = UTF-8)
/// followed by the text.
fn encode_context_character_string(
    buf: &mut Vec<u8>,
    tag_num: u8,
    s: &str,
) -> Result<(), ProtocolError> {
    let bytes = s.as_bytes();
    if bytes.len() > MAX_ACK_SOURCE_BYTES {
        return Err(ProtocolError::Other(format!(
            "Acknowledgment source is {} bytes; the limit is {MAX_ACK_SOURCE_BYTES}",
            bytes.len()
        )));
    }
    let total_len = bytes.len() + 1; // + charset octet
    if total_len <= 4 {
        buf.push((tag_num << 4) | 0x08 | total_len as u8);
    } else {
        // Length 5 in the tag byte: the real length follows (<= 253 here).
        buf.push((tag_num << 4) | 0x08 | 5);
        buf.push(total_len as u8);
    }
    buf.push(0x00); // charset 0: UTF-8
    buf.extend_from_slice(bytes);
    Ok(())
}

/// Encode opening tag for a context-tagged constructed type.
fn encode_opening_tag(buf: &mut Vec<u8>, tag_num: u8) {
    buf.push((tag_num << 4) | 0x0E);
}

/// Encode closing tag.
fn encode_closing_tag(buf: &mut Vec<u8>, tag_num: u8) {
    buf.push((tag_num << 4) | 0x0F);
}

/// Encode a BACnetTimeStamp CHOICE (`[0]` time, `[1]` sequence number,
/// `[2]` date-time) without any outer wrapper.
fn encode_time_stamp_choice(buf: &mut Vec<u8>, ts: &EventTimeStamp) -> Result<(), ProtocolError> {
    match ts {
        EventTimeStamp::Time { time } => {
            buf.push(0x0C); // context 0, length 4
            buf.extend_from_slice(&time.to_bytes());
        }
        EventTimeStamp::SequenceNumber { sequence_number } => {
            encode_context_unsigned(buf, 1, *sequence_number);
        }
        EventTimeStamp::DateTime { date, time } => {
            encode_opening_tag(buf, 2);
            buf.push(0xA4); // application Date, length 4
            buf.extend_from_slice(&date.to_bytes()?);
            buf.push(0xB4); // application Time, length 4
            buf.extend_from_slice(&time.to_bytes());
            encode_closing_tag(buf, 2);
        }
    }
    Ok(())
}

/// Encode a BACnetTimeStamp as a context-tagged field: the CHOICE enclosed in
/// opening/closing tag `tag_num`.
fn encode_context_time_stamp(
    buf: &mut Vec<u8>,
    tag_num: u8,
    ts: &EventTimeStamp,
) -> Result<(), ProtocolError> {
    encode_opening_tag(buf, tag_num);
    encode_time_stamp_choice(buf, ts)?;
    encode_closing_tag(buf, tag_num);
    Ok(())
}

// ---------------------------------------------------------------------------
// BER decoding helpers
// ---------------------------------------------------------------------------

type Decoded<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Opening,
    Closing,
    Primitive,
}

/// A decoded tag header (clause 20.2.1).
#[derive(Debug, Clone, Copy)]
struct Tag {
    number: u8,
    context: bool,
    kind: TagKind,
    /// Octets occupied by the tag header (tag byte, extended number, extended length).
    header_len: usize,
    /// Content octets following the header (0 for opening/closing tags and
    /// application Booleans, whose value lives in the header).
    len: usize,
}

fn parse_tag(data: &[u8], pos: usize) -> Decoded<Tag> {
    let truncated = || "unexpected end of data".to_string();
    let first = *data.get(pos).ok_or_else(truncated)?;
    let context = first & 0x08 != 0;
    let lvt = (first & 0x07) as usize;
    let mut number = first >> 4;
    let mut header_len = 1usize;
    if number == 0x0F {
        number = *data.get(pos + 1).ok_or_else(truncated)?;
        header_len = 2;
    }

    if context && (lvt == 6 || lvt == 7) {
        let kind = if lvt == 6 { TagKind::Opening } else { TagKind::Closing };
        return Ok(Tag { number, context, kind, header_len, len: 0 });
    }

    let mut len = lvt;
    if lvt == 5 {
        let first_len = *data.get(pos + header_len).ok_or_else(truncated)?;
        header_len += 1;
        len = match first_len {
            254 => {
                let hi = *data.get(pos + header_len).ok_or_else(truncated)?;
                let lo = *data.get(pos + header_len + 1).ok_or_else(truncated)?;
                header_len += 2;
                (usize::from(hi) << 8) | usize::from(lo)
            }
            255 => {
                let mut n = 0usize;
                for i in 0..4 {
                    n = (n << 8) | usize::from(*data.get(pos + header_len + i).ok_or_else(truncated)?);
                }
                header_len += 4;
                n
            }
            n => usize::from(n),
        };
    }
    if !context && number == 1 {
        len = 0; // application Boolean: the value is the length field
    }
    Ok(Tag { number, context, kind: TagKind::Primitive, header_len, len })
}

/// Cursor over BACnet-encoded service data.
struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn peek_tag(&self) -> Option<Tag> {
        parse_tag(self.data, self.pos).ok()
    }

    fn next_is_context(&self, number: u8) -> bool {
        matches!(self.peek_tag(), Some(t) if t.context && t.number == number && t.kind == TagKind::Primitive)
    }

    fn next_is_closing(&self, number: u8) -> bool {
        matches!(self.peek_tag(), Some(t) if t.context && t.number == number && t.kind == TagKind::Closing)
    }

    fn primitive(&mut self, context: bool, number: u8, what: &str) -> Decoded<&'a [u8]> {
        let tag = parse_tag(self.data, self.pos).map_err(|e| format!("{what}: {e}"))?;
        if tag.context != context || tag.number != number || tag.kind != TagKind::Primitive {
            return Err(format!(
                "expected {} tag {number} ({what}) at offset {}",
                if context { "context" } else { "application" },
                self.pos
            ));
        }
        let start = self.pos + tag.header_len;
        let end = start + tag.len;
        let content = self
            .data
            .get(start..end)
            .ok_or_else(|| format!("{what}: truncated value at offset {}", self.pos))?;
        self.pos = end;
        Ok(content)
    }

    fn context(&mut self, number: u8, what: &str) -> Decoded<&'a [u8]> {
        self.primitive(true, number, what)
    }

    fn application(&mut self, number: u8, what: &str) -> Decoded<&'a [u8]> {
        self.primitive(false, number, what)
    }

    fn delimiter(&mut self, kind: TagKind, number: u8, what: &str) -> Decoded<()> {
        let tag = parse_tag(self.data, self.pos).map_err(|e| format!("{what}: {e}"))?;
        if !tag.context || tag.number != number || tag.kind != kind {
            return Err(format!(
                "expected {} tag {number} ({what}) at offset {}",
                if kind == TagKind::Opening { "opening" } else { "closing" },
                self.pos
            ));
        }
        self.pos += tag.header_len;
        Ok(())
    }

    fn open(&mut self, number: u8, what: &str) -> Decoded<()> {
        self.delimiter(TagKind::Opening, number, what)
    }

    fn close(&mut self, number: u8, what: &str) -> Decoded<()> {
        self.delimiter(TagKind::Closing, number, what)
    }
}

/// Unsigned / Enumerated content octets (1-4) to u32.
fn parse_unsigned(content: &[u8], what: &str) -> Decoded<u32> {
    if content.is_empty() || content.len() > 4 {
        return Err(format!("{what}: unsigned value of {} octets", content.len()));
    }
    Ok(content.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)))
}

/// BACnetObjectIdentifier content octets to (type, instance).
fn parse_object_id(content: &[u8], what: &str) -> Decoded<(ObjectType, u32)> {
    let Ok(bytes) = <[u8; 4]>::try_from(content) else {
        return Err(format!("{what}: object identifier of {} octets", content.len()));
    };
    let raw = u32::from_be_bytes(bytes);
    Ok((ObjectType::from(raw >> 22), raw & 0x3F_FFFF))
}

/// BACnetEventTransitionBits (BIT STRING of 3) content octets to a mask with
/// bit 0 = to-offnormal, bit 1 = to-fault, bit 2 = to-normal.
///
/// On the wire named bit N is the Nth most significant bit of the first data
/// octet (clause 20.2.10), so to-offnormal is 0x80 there.
fn transition_bits(content: &[u8], what: &str) -> Decoded<u8> {
    let (unused, data) = content
        .split_first()
        .ok_or_else(|| format!("{what}: empty bit string"))?;
    if *unused > 7 {
        return Err(format!("{what}: bit string with {unused} unused bits"));
    }
    let first = data.first().copied().unwrap_or(0);
    Ok((0..3).fold(0u8, |acc, i| if first & (0x80 >> i) != 0 { acc | (1 << i) } else { acc }))
}

/// Decode a BACnetTimeStamp CHOICE at the cursor.
fn decode_time_stamp(d: &mut Decoder) -> Decoded<EventTimeStamp> {
    let tag = d.peek_tag().ok_or_else(|| "missing BACnetTimeStamp".to_string())?;
    match (tag.context, tag.number, tag.kind) {
        (true, 0, TagKind::Primitive) => {
            let c = d.context(0, "timestamp time")?;
            Ok(EventTimeStamp::Time { time: BacnetTime::from_bytes(c)? })
        }
        (true, 1, TagKind::Primitive) => {
            let c = d.context(1, "timestamp sequence number")?;
            Ok(EventTimeStamp::SequenceNumber {
                sequence_number: parse_unsigned(c, "timestamp sequence number")?,
            })
        }
        (true, 2, TagKind::Opening) => {
            d.open(2, "timestamp date-time")?;
            let date = BacnetDate::from_bytes(d.application(10, "timestamp date")?)?;
            let time = BacnetTime::from_bytes(d.application(11, "timestamp time")?)?;
            d.close(2, "timestamp date-time")?;
            Ok(EventTimeStamp::DateTime { date, time })
        }
        _ => Err(format!("unexpected tag {} while reading a BACnetTimeStamp", tag.number)),
    }
}

/// Character set octet + text to a String. Supports UTF-8 (0) and ISO 8859-1 (5).
fn decode_character_string(content: &[u8]) -> Decoded<String> {
    let (charset, text) = content
        .split_first()
        .ok_or_else(|| "empty character string".to_string())?;
    match charset {
        0 => Ok(String::from_utf8_lossy(text).into_owned()),
        5 => Ok(text.iter().map(|b| char::from(*b)).collect()),
        c => Err(format!("unsupported character set {c}")),
    }
}

// ---------------------------------------------------------------------------
// 1. GetAlarmSummary
// ---------------------------------------------------------------------------

/// Send GetAlarmSummary (confirmed service choice 3) and parse the response.
///
/// The ACK (clause 13.7) is a SEQUENCE OF SEQUENCE { objectIdentifier,
/// alarmState, acknowledgedTransitions } with application tags and no wrapper:
/// `C4 <oid>`, `91 <state>`, `82 <unused> <bits>` per entry.
pub fn get_alarm_summary_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
) -> Result<Vec<AlarmSummaryEntry>, ProtocolError> {
    let response = confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::GetAlarmSummary,
        Vec::new(),
        REQUEST_TIMEOUT,
    )?;
    parse_alarm_summary_ack(&response)
        .map_err(|e| ProtocolError::ReadFailed(format!("GetAlarmSummary: {e}")))
}

fn parse_alarm_summary_ack(data: &[u8]) -> Decoded<Vec<AlarmSummaryEntry>> {
    let mut d = Decoder::new(data);
    let mut entries = Vec::new();
    while !d.at_end() {
        let (obj_type, instance) = parse_object_id(d.application(12, "objectIdentifier")?, "objectIdentifier")?;
        let state = parse_unsigned(d.application(9, "alarmState")?, "alarmState")?;
        let acked = transition_bits(d.application(8, "acknowledgedTransitions")?, "acknowledgedTransitions")?;

        let type_str = object_type_to_string(&obj_type);
        entries.push(AlarmSummaryEntry {
            object_identifier: format!("{type_str}:{instance}"),
            object_type: type_str,
            object_instance: instance,
            alarm_state: event_state_name(state),
            acked_transitions: transition_bits_to_string(acked),
        });
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// 2. GetEventInformation
// ---------------------------------------------------------------------------

/// Send GetEventInformation (confirmed service choice 29) and follow
/// `moreEvents` until the device has returned every summary.
pub fn get_event_information_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
) -> Result<EventInformationResponse, ProtocolError> {
    let mut entries: Vec<EventInfoEntry> = Vec::new();
    let mut last_object: Option<(ObjectType, u32)> = None;

    for _ in 0..MAX_EVENT_PAGES {
        let (page, more_events) = get_event_information_page(socket, target_addr, last_object)?;
        let next_last = page
            .last()
            .map(|e| (ObjectType::from(e.object_type_id), e.object_instance));
        entries.extend(page);
        if !more_events {
            return Ok(EventInformationResponse { entries, more_events: false });
        }
        match next_last {
            Some(last) => last_object = Some(last),
            None => {
                return Err(ProtocolError::ReadFailed(
                    "GetEventInformation: device reported moreEvents with an empty page".into(),
                ))
            }
        }
    }
    Err(ProtocolError::ReadFailed(format!(
        "GetEventInformation: more than {MAX_EVENT_PAGES} pages"
    )))
}

/// One GetEventInformation round trip. Request (clause 13.12): optional
/// `[0] lastReceivedObjectIdentifier`.
fn get_event_information_page(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    last_object: Option<(ObjectType, u32)>,
) -> Result<(Vec<EventInfoEntry>, bool), ProtocolError> {
    let mut service_data = Vec::new();
    if let Some((ot, inst)) = last_object {
        encode_context_object_id(&mut service_data, 0, ot, inst);
    }

    let response = confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::GetEventInformation,
        service_data,
        REQUEST_TIMEOUT,
    )?;
    parse_event_information_ack(&response)
        .map_err(|e| ProtocolError::ReadFailed(format!("GetEventInformation: {e}")))
}

/// Parse GetEventInformation-ACK (clause 13.12):
///
/// ```text
/// [0] listOfEventSummaries { BACnetEventSummary ... }
/// [1] moreEvents BOOLEAN
/// ```
/// where each summary is `[0] objectIdentifier, [1] eventState,
/// [2] acknowledgedTransitions, [3] eventTimeStamps { 3 x BACnetTimeStamp },
/// [4] notifyType, [5] eventEnable, [6] eventPriorities { 3 x Unsigned }`.
fn parse_event_information_ack(data: &[u8]) -> Decoded<(Vec<EventInfoEntry>, bool)> {
    let mut d = Decoder::new(data);
    let mut entries = Vec::new();

    d.open(0, "listOfEventSummaries")?;
    while !d.next_is_closing(0) {
        let (obj_type, instance) = parse_object_id(d.context(0, "objectIdentifier")?, "objectIdentifier")?;
        let state = parse_unsigned(d.context(1, "eventState")?, "eventState")?;
        let acked = transition_bits(d.context(2, "acknowledgedTransitions")?, "acknowledgedTransitions")?;

        d.open(3, "eventTimeStamps")?;
        let mut stamps = Vec::with_capacity(3);
        for _ in 0..3 {
            stamps.push(decode_time_stamp(&mut d)?);
        }
        d.close(3, "eventTimeStamps")?;

        let notify = parse_unsigned(d.context(4, "notifyType")?, "notifyType")?;
        let enable = transition_bits(d.context(5, "eventEnable")?, "eventEnable")?;

        d.open(6, "eventPriorities")?;
        let mut priorities = Vec::with_capacity(3);
        for _ in 0..3 {
            priorities.push(parse_unsigned(d.application(2, "eventPriority")?, "eventPriority")?);
        }
        d.close(6, "eventPriorities")?;

        let type_str = object_type_to_string(&obj_type);
        entries.push(EventInfoEntry {
            object_identifier: format!("{type_str}:{instance}"),
            object_type: type_str,
            object_type_id: obj_type.into(),
            object_instance: instance,
            event_state: event_state_name(state),
            acknowledged_transitions: transition_bits_to_string(acked),
            event_enable: transition_bits_to_string(enable),
            event_time_stamps: stamps.iter().map(EventTimeStamp::describe).collect(),
            transition_time_stamps: stamps,
            notify_type: notify_type_name(notify),
            event_priorities: priorities,
        });
    }
    d.close(0, "listOfEventSummaries")?;

    let more = d.context(1, "moreEvents")?;
    let more_events = match more {
        [v] => *v != 0,
        _ => return Err(format!("moreEvents: boolean of {} octets", more.len())),
    };
    Ok((entries, more_events))
}

// ---------------------------------------------------------------------------
// 3. AcknowledgeAlarm
// ---------------------------------------------------------------------------

/// Send AcknowledgeAlarm (confirmed service choice 0, clause 13.5).
///
/// Parameters:
/// - `object_type_str` / `object_instance`: the alarmed object (`eventObjectIdentifier`)
/// - `event_state`: BACnetEventState being acknowledged (0=normal, 1=fault, 2=offnormal, ...)
/// - `event_time_stamp`: time stamp of the transition being acknowledged, as
///   reported by GetEventInformation for that transition
/// - `ack_source`: `acknowledgmentSource` text (who acknowledges)
/// - `process_id`: acknowledging process identifier (defaults to 0)
#[allow(clippy::too_many_arguments)]
pub fn acknowledge_alarm_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_type_str: &str,
    object_instance: u32,
    event_state: u32,
    event_time_stamp: &EventTimeStamp,
    ack_source: &str,
    process_id: Option<u32>,
) -> Result<(), ProtocolError> {
    let object_type = resolve_object_type(object_type_str)?;
    let service_data = encode_acknowledge_alarm(
        process_id.unwrap_or(0),
        object_type,
        object_instance,
        event_state,
        event_time_stamp,
        ack_source,
        &current_time_stamp(),
    )?;

    // SimpleAck expected -- empty data is fine.
    confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::AcknowledgeAlarm,
        service_data,
        REQUEST_TIMEOUT,
    )?;
    Ok(())
}

/// Encode AcknowledgeAlarm-Request (clause 13.5):
///
/// ```text
/// [0] acknowledgingProcessIdentifier  Unsigned32
/// [1] eventObjectIdentifier           BACnetObjectIdentifier
/// [2] eventStateAcknowledged          BACnetEventState
/// [3] timeStamp                       BACnetTimeStamp
/// [4] acknowledgmentSource            CharacterString
/// [5] timeOfAcknowledgment            BACnetTimeStamp
/// ```
fn encode_acknowledge_alarm(
    process_id: u32,
    object_type: ObjectType,
    object_instance: u32,
    event_state: u32,
    event_time_stamp: &EventTimeStamp,
    ack_source: &str,
    ack_time: &EventTimeStamp,
) -> Result<Vec<u8>, ProtocolError> {
    let mut buf = Vec::new();
    encode_context_unsigned(&mut buf, 0, process_id);
    encode_context_object_id(&mut buf, 1, object_type, object_instance);
    encode_context_enumerated(&mut buf, 2, event_state);
    encode_context_time_stamp(&mut buf, 3, event_time_stamp)?;
    encode_context_character_string(&mut buf, 4, ack_source)?;
    encode_context_time_stamp(&mut buf, 5, ack_time)?;
    Ok(buf)
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
    let parsed = decode_event_notification(data).ok()?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let type_str = object_type_to_string(&parsed.object_type);
    Some(AlarmNotification {
        source_ip: source.ip().to_string(),
        source_port: source.port(),
        object_type: type_str,
        object_instance: parsed.object_instance,
        event_state: event_state_name(parsed.to_state),
        from_state: parsed
            .from_state
            .map_or_else(|| "unknown".to_string(), event_state_name),
        to_state: event_state_name(parsed.to_state),
        notify_type: notify_type_name(parsed.notify_type),
        ack_required: parsed.ack_required,
        message: parsed.message,
        timestamp,
    })
}

struct DecodedNotification {
    object_type: ObjectType,
    object_instance: u32,
    message: String,
    notify_type: u32,
    ack_required: bool,
    from_state: Option<u32>,
    to_state: u32,
}

/// Decode EventNotification-Request (clause 13.8 / 13.9) up to `toState`:
/// `[0] process id, [1] initiating device, [2] event object, [3] timeStamp,
/// [4] notificationClass, [5] priority, [6] eventType, [7] messageText (opt),
/// [8] notifyType, [9] ackRequired (opt), [10] fromState (opt), [11] toState`;
/// `[12] eventValues` is not interpreted.
fn decode_event_notification(data: &[u8]) -> Decoded<DecodedNotification> {
    let mut d = Decoder::new(data);
    d.context(0, "processIdentifier")?;
    d.context(1, "initiatingDeviceIdentifier")?;
    let (object_type, object_instance) =
        parse_object_id(d.context(2, "eventObjectIdentifier")?, "eventObjectIdentifier")?;
    d.open(3, "timeStamp")?;
    decode_time_stamp(&mut d)?;
    d.close(3, "timeStamp")?;
    d.context(4, "notificationClass")?;
    d.context(5, "priority")?;
    d.context(6, "eventType")?;
    let message = if d.next_is_context(7) {
        decode_character_string(d.context(7, "messageText")?)?
    } else {
        String::new()
    };
    let notify_type = parse_unsigned(d.context(8, "notifyType")?, "notifyType")?;
    let ack_required = if d.next_is_context(9) {
        d.context(9, "ackRequired")?.first().is_some_and(|v| *v != 0)
    } else {
        false
    };
    let from_state = if d.next_is_context(10) {
        Some(parse_unsigned(d.context(10, "fromState")?, "fromState")?)
    } else {
        None
    };
    let to_state = parse_unsigned(d.context(11, "toState")?, "toState")?;

    Ok(DecodedNotification {
        object_type,
        object_instance,
        message,
        notify_type,
        ack_required,
        from_state,
        to_state,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dt_2024_12_31() -> EventTimeStamp {
        EventTimeStamp::DateTime {
            date: BacnetDate { year: Some(2024), month: Some(12), day: Some(31), weekday: Some(2) },
            time: BacnetTime { hour: Some(23), minute: Some(59), second: Some(58), hundredths: Some(0) },
        }
    }

    #[test]
    fn context_unsigned_uses_minimum_octets() {
        let mut buf = Vec::new();
        encode_context_unsigned(&mut buf, 0, 0);
        assert_eq!(buf, [0x09, 0x00]); // ctx 0, length 1
        buf.clear();
        encode_context_unsigned(&mut buf, 3, 1000);
        assert_eq!(buf, [0x3A, 0x03, 0xE8]); // ctx 3, length 2
        buf.clear();
        encode_context_unsigned(&mut buf, 1, 0x01_0000);
        assert_eq!(buf, [0x1B, 0x01, 0x00, 0x00]); // ctx 1, length 3 (not padded to 4)
        buf.clear();
        encode_context_unsigned(&mut buf, 1, 0x0100_0000);
        assert_eq!(buf, [0x1C, 0x01, 0x00, 0x00, 0x00]); // length 4
    }

    #[test]
    fn character_string_encoding() {
        let mut buf = Vec::new();
        encode_context_character_string(&mut buf, 4, "hello").unwrap();
        // ctx 4, extended length: 0x4D, length byte 6 (charset + 5), charset 0 (UTF-8), "hello"
        assert_eq!(buf, [0x4D, 6, 0x00, 0x68, 0x65, 0x6C, 0x6C, 0x6F]);
        buf.clear();
        encode_context_character_string(&mut buf, 4, "ab").unwrap();
        assert_eq!(buf, [0x4B, 0x00, 0x61, 0x62]); // ctx 4, length 3 in the tag byte
        assert!(encode_context_character_string(&mut buf, 4, &"x".repeat(201)).is_err());
    }

    #[test]
    fn acknowledge_alarm_request_vector() {
        let oper = "operator";
        let data = encode_acknowledge_alarm(
            0,
            ObjectType::BinaryInput,
            2,
            2,
            &dt_2024_12_31(),
            oper,
            &unix_to_date_time(1_735_689_605), // 2025-01-01T00:00:05Z
        )
        .unwrap();
        let expected: Vec<u8> = vec![
            0x09, 0x00, // [0] acknowledgingProcessIdentifier = 0
            0x1C, 0x00, 0xC0, 0x00, 0x02, // [1] eventObjectIdentifier binary-input(3)<<22 | 2 = 0x00C00002
            0x29, 0x02, // [2] eventStateAcknowledged = offnormal(2)
            0x3E, // [3] timeStamp opening
            0x2E, // CHOICE [2] dateTime opening
            0xA4, 0x7C, 0x0C, 0x1F, 0x02, // app Date: 2024-1900=124, December, 31, Tuesday
            0xB4, 0x17, 0x3B, 0x3A, 0x00, // app Time: 23:59:58.00
            0x2F, // CHOICE [2] closing
            0x3F, // [3] closing
            0x4D, 0x09, 0x00, // [4] acknowledgmentSource: length 9 = charset 0 + "operator"
            0x6F, 0x70, 0x65, 0x72, 0x61, 0x74, 0x6F, 0x72, // "operator"
            0x5E, // [5] timeOfAcknowledgment opening
            0x2E, // CHOICE [2] dateTime opening
            0xA4, 0x7D, 0x01, 0x01, 0x03, // app Date: 2025-01-01, Wednesday
            0xB4, 0x00, 0x00, 0x05, 0x00, // app Time: 00:00:05.00
            0x2F, // CHOICE [2] closing
            0x5F, // [5] closing
        ];
        assert_eq!(data, expected);
    }

    #[test]
    fn acknowledge_alarm_other_time_stamp_choices() {
        let ack_time = unix_to_date_time(0);
        let seq = encode_acknowledge_alarm(
            1,
            ObjectType::AnalogInput,
            1,
            0,
            &EventTimeStamp::SequenceNumber { sequence_number: 258 },
            "x",
            &ack_time,
        )
        .unwrap();
        // [3] { [1] sequence number 258 = 0x0102 (length 2) }
        assert_eq!(&seq[..14], [0x09, 0x01, 0x1C, 0x00, 0x00, 0x00, 0x01, 0x29, 0x00, 0x3E, 0x1A, 0x01, 0x02, 0x3F]);

        let time = encode_acknowledge_alarm(
            1,
            ObjectType::AnalogInput,
            1,
            0,
            &EventTimeStamp::Time {
                time: BacnetTime { hour: Some(8), minute: Some(30), second: None, hundredths: None },
            },
            "x",
            &ack_time,
        )
        .unwrap();
        // [3] { [0] time 08:30:FF.FF }
        assert_eq!(&time[9..17], [0x3E, 0x0C, 0x08, 0x1E, 0xFF, 0xFF, 0x3F, 0x4A]);
    }

    #[test]
    fn date_cannot_be_encoded_outside_one_octet_year() {
        let bad = EventTimeStamp::DateTime {
            date: BacnetDate { year: Some(1899), month: None, day: None, weekday: None },
            time: BacnetTime { hour: None, minute: None, second: None, hundredths: None },
        };
        assert!(encode_time_stamp_choice(&mut Vec::new(), &bad).is_err());
    }

    #[test]
    fn unix_dates() {
        let ymd = |days: i64| civil_from_days(days);
        assert_eq!(ymd(0), (1970, 1, 1));
        assert_eq!(ymd(11_016), (2000, 2, 29));
        assert_eq!(ymd(20_088), (2024, 12, 31));
        assert_eq!(ymd(-1), (1969, 12, 31));

        let EventTimeStamp::DateTime { date, time } = unix_to_date_time(0) else {
            unreachable!()
        };
        assert_eq!((date.year, date.month, date.day, date.weekday), (Some(1970), Some(1), Some(1), Some(4))); // Thursday
        assert_eq!((time.hour, time.minute, time.second), (Some(0), Some(0), Some(0)));

        // 2000-02-29T12:34:56Z, a Tuesday
        let EventTimeStamp::DateTime { date, time } = unix_to_date_time(951_782_400 + 12 * 3600 + 34 * 60 + 56)
        else {
            unreachable!()
        };
        assert_eq!((date.year, date.month, date.day, date.weekday), (Some(2000), Some(2), Some(29), Some(2)));
        assert_eq!((time.hour, time.minute, time.second), (Some(12), Some(34), Some(56)));

        // 2024-12-31T23:59:58Z, a Tuesday
        assert_eq!(unix_to_date_time(1_735_603_200 + 23 * 3600 + 59 * 60 + 58), dt_2024_12_31());
    }

    /// One GetEventInformation-ACK: two summaries and moreEvents = TRUE.
    fn event_information_ack() -> Vec<u8> {
        vec![
            0x0E, // [0] listOfEventSummaries opening
            // --- summary 1: analog-input 5 ---
            0x0C, 0x00, 0x00, 0x00, 0x05, // [0] objectIdentifier analog-input(0)<<22 | 5
            0x19, 0x02, // [1] eventState = offnormal
            0x2A, 0x05, 0xA0, // [2] acknowledgedTransitions: 5 unused bits, 101 -> to-offnormal + to-normal
            0x3E, // [3] eventTimeStamps opening
            0x2E, 0xA4, 0x7C, 0x0C, 0x1F, 0x02, 0xB4, 0x17, 0x3B, 0x3A, 0x00, 0x2F, // to-offnormal: [2] dateTime 2024-12-31 23:59:58.00
            0x19, 0x07, // to-fault: [1] sequence number 7
            0x0C, 0x08, 0x1E, 0x00, 0x00, // to-normal: [0] time 08:30:00.00
            0x3F, // [3] closing
            0x49, 0x00, // [4] notifyType = alarm
            0x5A, 0x05, 0xE0, // [5] eventEnable: 111 -> all three
            0x6E, 0x21, 0x03, 0x21, 0x04, 0x21, 0x05, 0x6F, // [6] eventPriorities { 3, 4, 5 } as application Unsigned
            // --- summary 2: binary-input 2, normal, no acks, unspecified stamps ---
            0x0C, 0x00, 0xC0, 0x00, 0x02, // [0] binary-input(3)<<22 | 2
            0x19, 0x00, // [1] eventState = normal
            0x2A, 0x05, 0x00, // [2] acknowledgedTransitions: none
            0x3E, // [3] eventTimeStamps opening
            0x0C, 0xFF, 0xFF, 0xFF, 0xFF, // [0] time all unspecified
            0x0C, 0xFF, 0xFF, 0xFF, 0xFF,
            0x0C, 0xFF, 0xFF, 0xFF, 0xFF,
            0x3F, // [3] closing
            0x49, 0x01, // [4] notifyType = event
            0x5A, 0x05, 0x80, // [5] eventEnable: 100 -> to-offnormal only
            0x6E, 0x21, 0x80, 0x21, 0x80, 0x21, 0x80, 0x6F, // [6] eventPriorities { 128, 128, 128 }
            0x0F, // [0] closing
            0x19, 0x01, // [1] moreEvents = TRUE (context boolean: one content octet)
        ]
    }

    #[test]
    fn parses_event_information_ack() {
        let (entries, more) = parse_event_information_ack(&event_information_ack()).unwrap();
        assert!(more);
        assert_eq!(entries.len(), 2);

        let e = &entries[0];
        assert_eq!(e.object_identifier, "analogInput:5");
        assert_eq!(e.object_type_id, 0);
        assert_eq!(e.object_instance, 5);
        assert_eq!(e.event_state, "offnormal");
        assert_eq!(e.acknowledged_transitions, "ToOffNormal, ToNormal");
        assert_eq!(e.event_enable, "ToOffNormal, ToFault, ToNormal");
        assert_eq!(e.notify_type, "alarm");
        assert_eq!(e.event_priorities, vec![3, 4, 5]);
        assert_eq!(
            e.transition_time_stamps,
            vec![
                dt_2024_12_31(),
                EventTimeStamp::SequenceNumber { sequence_number: 7 },
                EventTimeStamp::Time {
                    time: BacnetTime { hour: Some(8), minute: Some(30), second: Some(0), hundredths: Some(0) }
                },
            ]
        );
        assert_eq!(
            e.event_time_stamps,
            vec!["2024-12-31 23:59:58.00", "#7", "08:30:00.00"]
        );

        let e = &entries[1];
        assert_eq!(e.object_identifier, "binaryInput:2");
        assert_eq!(e.event_state, "normal");
        assert_eq!(e.acknowledged_transitions, "");
        assert_eq!(e.event_enable, "ToOffNormal");
        assert_eq!(e.notify_type, "event");
        assert_eq!(e.event_priorities, vec![128, 128, 128]);
        assert_eq!(e.event_time_stamps, vec!["**:**:**.**"; 3]);
    }

    #[test]
    fn event_information_ack_without_events() {
        // [0] opening, [0] closing, [1] moreEvents = FALSE
        let (entries, more) = parse_event_information_ack(&[0x0E, 0x0F, 0x19, 0x00]).unwrap();
        assert!(entries.is_empty());
        assert!(!more);
    }

    #[test]
    fn event_information_ack_rejects_old_wrong_layout() {
        // The previous parser read [3] as eventEnable; here [3] is the time stamp list.
        // A summary whose [3] is a bit string must be rejected, not misparsed.
        let bad = [
            0x0E, 0x0C, 0x00, 0x00, 0x00, 0x05, 0x19, 0x02, 0x2A, 0x05, 0xA0, 0x3A, 0x05, 0xE0, 0x0F, 0x19, 0x00,
        ];
        assert!(parse_event_information_ack(&bad).is_err());
    }

    #[test]
    fn parses_alarm_summary_ack() {
        let data = [
            0xC4, 0x00, 0x00, 0x00, 0x05, // application ObjectIdentifier analog-input:5
            0x91, 0x03, // application Enumerated alarmState = high-limit
            0x82, 0x05, 0xC0, // application BitString: 5 unused, 110 -> to-offnormal + to-fault acked
            0xC4, 0x00, 0xC0, 0x00, 0x02, // binary-input:2
            0x91, 0x00, // normal
            0x82, 0x05, 0xE0, // 111 -> all acked
        ];
        let entries = parse_alarm_summary_ack(&data).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].object_identifier, "analogInput:5");
        assert_eq!(entries[0].alarm_state, "highLimit");
        assert_eq!(entries[0].acked_transitions, "ToOffNormal, ToFault");
        assert_eq!(entries[1].object_identifier, "binaryInput:2");
        assert_eq!(entries[1].alarm_state, "normal");
        assert_eq!(entries[1].acked_transitions, "ToOffNormal, ToFault, ToNormal");

        assert!(parse_alarm_summary_ack(&[]).unwrap().is_empty());
        // Context-tagged layout (what the previous parser expected) is not valid.
        assert!(parse_alarm_summary_ack(&[0x0C, 0x00, 0x00, 0x00, 0x05, 0x19, 0x03, 0x2A, 0x05, 0xE0]).is_err());
    }

    #[test]
    fn parses_event_notification() {
        let data = [
            0x09, 0x01, // [0] processIdentifier = 1
            0x1C, 0x02, 0x00, 0x00, 0x01, // [1] initiating device: device(8)<<22 | 1
            0x2C, 0x00, 0x00, 0x00, 0x05, // [2] eventObjectIdentifier analog-input:5
            0x3E, 0x1A, 0x01, 0x07, 0x3F, // [3] timeStamp { [1] sequence number 263 }
            0x49, 0x0A, // [4] notificationClass = 10
            0x59, 0x80, // [5] priority = 128
            0x69, 0x05, // [6] eventType = out-of-range(5)
            0x7D, 0x05, 0x00, 0x48, 0x69, 0x67, 0x68, // [7] messageText: charset 0 + "High"
            0x89, 0x00, // [8] notifyType = alarm
            0x99, 0x01, // [9] ackRequired = TRUE
            0xA9, 0x00, // [10] fromState = normal
            0xB9, 0x03, // [11] toState = high-limit
        ];
        let src: SocketAddr = "192.168.1.10:47808".parse().unwrap();
        let n = parse_alarm_notification(&data, src).unwrap();
        assert_eq!(n.source_ip, "192.168.1.10");
        assert_eq!(n.source_port, 47808);
        assert_eq!(n.object_type, "analogInput");
        assert_eq!(n.object_instance, 5);
        assert_eq!(n.message, "High");
        assert_eq!(n.notify_type, "alarm");
        assert!(n.ack_required);
        assert_eq!(n.from_state, "normal");
        assert_eq!(n.to_state, "highLimit");
        assert_eq!(n.event_state, "highLimit");

        assert!(parse_alarm_notification(&data[..10], src).is_none());
    }

    #[test]
    fn resolves_object_types() {
        assert_eq!(resolve_object_type("binaryInput").unwrap(), ObjectType::BinaryInput);
        assert_eq!(resolve_object_type("EventEnrollment").unwrap(), ObjectType::EventEnrollment);
        assert!(resolve_object_type("notAType").is_err());
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
    fn transition_bit_string_is_msb_first() {
        // BIT STRING of 3 bits, 5 unused: named bit 0 is the MSB (0x80).
        assert_eq!(transition_bits(&[0x05, 0x80], "t").unwrap(), 0b001); // to-offnormal
        assert_eq!(transition_bits(&[0x05, 0x40], "t").unwrap(), 0b010); // to-fault
        assert_eq!(transition_bits(&[0x05, 0x20], "t").unwrap(), 0b100); // to-normal
        assert_eq!(transition_bits(&[0x00], "t").unwrap(), 0);
        assert!(transition_bits(&[], "t").is_err());
    }

    #[test]
    fn tag_parsing() {
        // opening tag 3, closing tag 3, context 1 length 1, application boolean TRUE (0x11)
        let t = parse_tag(&[0x3E], 0).unwrap();
        assert_eq!((t.number, t.kind), (3, TagKind::Opening));
        let t = parse_tag(&[0x3F], 0).unwrap();
        assert_eq!((t.number, t.kind), (3, TagKind::Closing));
        let t = parse_tag(&[0x19, 0x01], 0).unwrap();
        assert_eq!((t.number, t.context, t.len, t.header_len), (1, true, 1, 1));
        let t = parse_tag(&[0x11], 0).unwrap();
        assert_eq!((t.number, t.context, t.len), (1, false, 0));
        // extended length: context 4, length byte 6
        let t = parse_tag(&[0x4D, 0x06], 0).unwrap();
        assert_eq!((t.len, t.header_len), (6, 2));
        assert!(parse_tag(&[0x4D], 0).is_err());
    }
}
