//! BACnet Schedule object reader (ASHRAE 135 clause 12.24).
//!
//! Reads, with plain ReadProperty requests:
//! - WeeklySchedule (property 123) — 7 × BACnetDailySchedule, Monday..Sunday
//! - ExceptionSchedule (property 38) — BACnetSpecialEvent list
//! - ScheduleDefault (property 174) — fallback value
//! - EffectivePeriod (property 32) — BACnetDateRange
//! - ListOfObjectPropertyReferences (property 54)
//!
//! The standard requires at least one of WeeklySchedule / ExceptionSchedule;
//! the call fails when the object has neither. Every property is treated as
//! optional in the sense that only an Error PDU with error code
//! `unknown-property` means "the object does not have it"; every other failure
//! (timeout, abort, reject, other errors, malformed data) is propagated. The
//! two array properties fall back to element-by-element reads when the device
//! Aborts because the whole array does not fit one APDU.

use std::net::SocketAddr;

use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

use crate::transport;

/// Object type number of the Schedule object (BACnetObjectType 17).
const SCHEDULE_OBJECT_TYPE: u32 = 17;

const PROP_EFFECTIVE_PERIOD: u32 = 32;
const PROP_EXCEPTION_SCHEDULE: u32 = 38;
const PROP_LIST_OF_OBJECT_PROPERTY_REFERENCES: u32 = 54;
const PROP_WEEKLY_SCHEDULE: u32 = 123;
const PROP_SCHEDULE_DEFAULT: u32 = 174;

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

/// One BACnetSpecialEvent of the ExceptionSchedule (clause 12.24.8).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpecialEvent {
    /// When the event applies: a date, "a to b" date range, week-n-day pattern
    /// or a reference to a Calendar object ("calendar object_6:2").
    pub period: String,
    /// Event priority (1 = highest, 16 = lowest); events override the weekly
    /// schedule and each other by priority.
    pub priority: u32,
    /// Ordered list of time-value entries of the event.
    pub entries: Vec<ScheduleEntry>,
}

/// Complete schedule data for a BACnet Schedule object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleData {
    /// Object instance number of the Schedule object.
    pub object_instance: u32,
    /// Weekly schedule (7 days, Monday..Sunday); empty when the object has
    /// only an ExceptionSchedule.
    pub weekly_schedule: Vec<DaySchedule>,
    /// Exception schedule events; empty when the object does not have one.
    pub exception_schedule: Vec<SpecialEvent>,
    /// Default value when no schedule entry is active.
    pub schedule_default: Option<String>,
    /// Start date of the effective period (YYYY-MM-DD); `None` when the device
    /// does not report the property or the date is fully unspecified.
    pub effective_period_start: Option<String>,
    /// End date of the effective period (YYYY-MM-DD); same rules as the start.
    pub effective_period_end: Option<String>,
    /// Referenced object properties as strings.
    pub object_property_references: Vec<String>,
}

/// BACnetDailySchedule index 0 is Monday (clause 12.24.5).
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
// Request encoding
// ---------------------------------------------------------------------------

/// Context-tagged Unsigned, minimal length (clause 20.2.4).
fn push_context_unsigned(buf: &mut Vec<u8>, tag_number: u8, value: u32) {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take(3).take_while(|b| **b == 0).count();
    let value = &bytes[skip..];
    buf.push((tag_number << 4) | 0x08 | value.len() as u8);
    buf.extend_from_slice(value);
}

/// ReadProperty-Request service data (clause 15.5.1.1):
/// `[0] objectIdentifier, [1] propertyIdentifier, [2] propertyArrayIndex OPTIONAL`.
fn encode_read_property(
    object_instance: u32,
    property_id: u32,
    array_index: Option<u32>,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(13);
    // Context tag 0, length 4: object id = type << 22 | instance.
    buf.push(0x0C);
    buf.extend_from_slice(&((SCHEDULE_OBJECT_TYPE << 22) | (object_instance & 0x3F_FFFF)).to_be_bytes());
    push_context_unsigned(&mut buf, 1, property_id);
    if let Some(index) = array_index {
        push_context_unsigned(&mut buf, 2, index);
    }
    buf
}

// ---------------------------------------------------------------------------
// Tag decoding (clause 20.2.1)
// ---------------------------------------------------------------------------

fn malformed(what: &str) -> ProtocolError {
    ProtocolError::Other(format!("Malformed Schedule response: {what}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Primitive,
    Opening,
    Closing,
}

#[derive(Debug, Clone, Copy)]
struct Tag {
    number: u32,
    context: bool,
    kind: TagKind,
    /// Bytes occupied by the tag header (initial octet + extensions).
    header: usize,
    /// Content length for primitive tags (0 for opening/closing and for
    /// application-tagged Booleans, whose value lives in the header).
    len: usize,
    /// Raw length/value/type bits of the initial octet.
    lvt: u8,
}

impl Tag {
    fn end(&self, pos: usize) -> usize {
        pos + self.header + self.len
    }

    fn content<'a>(&self, data: &'a [u8], pos: usize) -> &'a [u8] {
        &data[pos + self.header..self.end(pos)]
    }
}

fn read_tag(data: &[u8], pos: usize) -> Result<Tag, ProtocolError> {
    let b = *data.get(pos).ok_or_else(|| malformed("truncated tag"))?;
    let mut header = 1;
    let mut number = (b >> 4) as u32;
    if number == 0x0F {
        number = *data.get(pos + 1).ok_or_else(|| malformed("truncated tag number"))? as u32;
        header = 2;
    }
    let context = b & 0x08 != 0;
    let lvt = b & 0x07;
    let tag = |kind, header, len| Tag { number, context, kind, header, len, lvt };

    if context && lvt == 6 {
        return Ok(tag(TagKind::Opening, header, 0));
    }
    if context && lvt == 7 {
        return Ok(tag(TagKind::Closing, header, 0));
    }

    let len = if !context && number == 1 {
        // Application Boolean: the value is in the length bits.
        0
    } else if lvt == 5 {
        let first = *data.get(pos + header).ok_or_else(|| malformed("truncated length"))?;
        header += 1;
        match first {
            254 => {
                let raw = data
                    .get(pos + header..pos + header + 2)
                    .ok_or_else(|| malformed("truncated length"))?;
                header += 2;
                u16::from_be_bytes([raw[0], raw[1]]) as usize
            }
            255 => {
                let raw = data
                    .get(pos + header..pos + header + 4)
                    .ok_or_else(|| malformed("truncated length"))?;
                header += 4;
                u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize
            }
            n => n as usize,
        }
    } else {
        lvt as usize
    };

    if pos + header + len > data.len() {
        return Err(malformed("element extends past end of data"));
    }
    Ok(tag(TagKind::Primitive, header, len))
}

/// Read a primitive context tag with the given number; returns its content and
/// the position after it.
fn read_context_primitive(
    data: &[u8],
    pos: usize,
    number: u32,
) -> Result<(&[u8], usize), ProtocolError> {
    let tag = read_tag(data, pos)?;
    if !tag.context || tag.kind != TagKind::Primitive || tag.number != number {
        return Err(malformed(&format!("expected context tag [{number}] at offset {pos}")));
    }
    Ok((tag.content(data, pos), tag.end(pos)))
}

/// Expect an opening or closing context tag `number` at `pos`; returns the
/// position after it.
fn expect_bracket(
    data: &[u8],
    pos: usize,
    number: u32,
    kind: TagKind,
) -> Result<usize, ProtocolError> {
    let tag = read_tag(data, pos)?;
    if !tag.context || tag.kind != kind || tag.number != number {
        return Err(malformed(&format!("expected {kind:?} tag [{number}] at offset {pos}")));
    }
    Ok(tag.end(pos))
}

fn be_unsigned(bytes: &[u8]) -> Result<u64, ProtocolError> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(malformed("unsigned integer length"));
    }
    Ok(bytes.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64))
}

fn be_signed(bytes: &[u8]) -> Result<i64, ProtocolError> {
    let raw = be_unsigned(bytes)?;
    let shift = 64 - 8 * bytes.len() as u32;
    Ok(((raw << shift) as i64) >> shift)
}

// ---------------------------------------------------------------------------
// Value formatting
// ---------------------------------------------------------------------------

/// Format a BACnetTime ("HH:MM:SS"); 0xFF fields render as "**".
fn format_time(content: &[u8]) -> Result<String, ProtocolError> {
    let [h, m, s, _hundredths] = content else {
        return Err(malformed("Time must be 4 octets"));
    };
    let field = |v: u8| if v == 0xFF { "**".to_string() } else { format!("{v:02}") };
    Ok(format!("{}:{}:{}", field(*h), field(*m), field(*s)))
}

/// Format a BACnetDate: year (ONE octet, year-1900), month, day, day-of-week.
///
/// Returns `None` when year, month and day are all unspecified (0xFF).
/// Partially specified dates keep wildcards visible ("****", "**", "odd", ...).
fn format_date(content: &[u8]) -> Result<Option<String>, ProtocolError> {
    let [y, m, d, _weekday] = content else {
        return Err(malformed("Date must be 4 octets"));
    };
    if *y == 0xFF && *m == 0xFF && *d == 0xFF {
        return Ok(None);
    }
    let year = if *y == 0xFF { "****".to_string() } else { (1900 + *y as u32).to_string() };
    let month = match *m {
        0xFF => "**".to_string(),
        13 => "odd".to_string(),
        14 => "even".to_string(),
        n => format!("{n:02}"),
    };
    let day = match *d {
        0xFF => "**".to_string(),
        32 => "last".to_string(),
        33 => "odd".to_string(),
        34 => "even".to_string(),
        n => format!("{n:02}"),
    };
    Ok(Some(format!("{year}-{month}-{day}")))
}

fn format_character_string(content: &[u8]) -> Result<String, ProtocolError> {
    let (encoding, text) = content
        .split_first()
        .ok_or_else(|| malformed("empty CharacterString"))?;
    match encoding {
        // ISO 10646 UTF-8
        0 => Ok(String::from_utf8_lossy(text).into_owned()),
        // ISO 10646 UCS-2 (big endian)
        4 => Ok(char::decode_utf16(
            text.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])),
        )
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()),
        // ISO 8859-1
        5 => Ok(text.iter().map(|b| *b as char).collect()),
        n => Err(malformed(&format!("unsupported CharacterString encoding {n}"))),
    }
}

/// Decode one application-tagged value at `pos` into a display string.
fn parse_application_value(data: &[u8], pos: usize) -> Result<(String, usize), ProtocolError> {
    let tag = read_tag(data, pos)?;
    if tag.context || tag.kind != TagKind::Primitive {
        return Err(malformed(&format!("expected application-tagged value at offset {pos}")));
    }
    let content = tag.content(data, pos);
    let text = match tag.number {
        0 => "Null".to_string(),
        1 => (tag.lvt != 0).to_string(),
        2 => be_unsigned(content)?.to_string(),
        3 => be_signed(content)?.to_string(),
        4 => {
            let raw: [u8; 4] = content.try_into().map_err(|_| malformed("Real must be 4 octets"))?;
            f32::from_be_bytes(raw).to_string()
        }
        5 => {
            let raw: [u8; 8] = content.try_into().map_err(|_| malformed("Double must be 8 octets"))?;
            f64::from_be_bytes(raw).to_string()
        }
        6 => content.iter().map(|b| format!("{b:02X}")).collect::<String>(),
        7 => format_character_string(content)?,
        8 => {
            let (unused, bits) = content
                .split_first()
                .ok_or_else(|| malformed("empty BitString"))?;
            let total = (bits.len() * 8).saturating_sub(*unused as usize);
            (0..total)
                .map(|i| if bits[i / 8] & (0x80 >> (i % 8)) != 0 { '1' } else { '0' })
                .collect()
        }
        9 => be_unsigned(content)?.to_string(),
        10 => format_date(content)?.unwrap_or_else(|| "any".to_string()),
        11 => format_time(content)?,
        12 => {
            let (obj_type, instance) = split_object_id(content)?;
            format!("object_{obj_type}:{instance}")
        }
        n => return Err(malformed(&format!("unknown application tag {n}"))),
    };
    Ok((text, tag.end(pos)))
}

fn split_object_id(content: &[u8]) -> Result<(u32, u32), ProtocolError> {
    let raw: [u8; 4] = content
        .try_into()
        .map_err(|_| malformed("ObjectIdentifier must be 4 octets"))?;
    let raw = u32::from_be_bytes(raw);
    Ok((raw >> 22, raw & 0x3F_FFFF))
}

// ---------------------------------------------------------------------------
// ReadProperty-ACK framing and property parsers
// ---------------------------------------------------------------------------

/// Strip the ReadProperty-ACK wrapper (clause 15.5.1.2) and return the bytes
/// between the `[3]` opening and closing tags:
/// `[0] objectIdentifier, [1] propertyIdentifier, [2] arrayIndex OPTIONAL,
/// [3] { propertyValue }`.
fn strip_read_property_ack(data: &[u8], expected_property: u32) -> Result<&[u8], ProtocolError> {
    let (_object, pos) = read_context_primitive(data, 0, 0)?;
    let (property, mut pos) = read_context_primitive(data, pos, 1)?;
    let property = be_unsigned(property)?;
    if property != expected_property as u64 {
        return Err(malformed(&format!(
            "ACK is for property {property}, requested {expected_property}"
        )));
    }

    let next = read_tag(data, pos)?;
    if next.context && next.kind == TagKind::Primitive && next.number == 2 {
        pos = next.end(pos);
    }

    let start = expect_bracket(data, pos, 3, TagKind::Opening)?;
    let mut depth = 1usize;
    let mut p = start;
    loop {
        let tag = read_tag(data, p)?;
        match tag.kind {
            TagKind::Opening => depth += 1,
            TagKind::Closing => {
                depth -= 1;
                if depth == 0 {
                    if tag.end(p) != data.len() {
                        return Err(malformed("trailing bytes after property value"));
                    }
                    return Ok(&data[start..p]);
                }
            }
            TagKind::Primitive => {}
        }
        p = tag.end(p);
    }
}

/// BACnetTimeValue list up to the closing tag `[closing]`, starting at `pos`
/// (just after the matching opening tag). Returns the entries and the
/// position after the closing tag.
///
/// BACnetTimeValue ::= SEQUENCE { time Time, value ABSTRACT-SYNTAX.&Type }
/// Wire form: `{B4 hh mm ss hs <app-value>}*`.
fn parse_time_values(
    data: &[u8],
    mut pos: usize,
    closing: u32,
) -> Result<(Vec<ScheduleEntry>, usize), ProtocolError> {
    let mut entries = Vec::new();
    loop {
        let tag = read_tag(data, pos)?;
        if tag.context && tag.kind == TagKind::Closing && tag.number == closing {
            return Ok((entries, tag.end(pos)));
        }
        if tag.context || tag.kind != TagKind::Primitive || tag.number != 11 {
            return Err(malformed(&format!("expected Time in time-value list at offset {pos}")));
        }
        let time = format_time(tag.content(data, pos))?;
        let (value, next) = parse_application_value(data, tag.end(pos))?;
        entries.push(ScheduleEntry { time, value });
        pos = next;
    }
}

/// WeeklySchedule ::= SEQUENCE SIZE(7) OF BACnetDailySchedule
/// BACnetDailySchedule ::= SEQUENCE { [0] day-schedule SEQUENCE OF BACnetTimeValue }
///
/// Wire form of each day: `0E {B4 hh mm ss hs <app-value>}* 0F`.
fn parse_weekly_schedule(data: &[u8]) -> Result<Vec<DaySchedule>, ProtocolError> {
    let mut days = Vec::with_capacity(7);
    let mut pos = 0;
    while pos < data.len() {
        pos = expect_bracket(data, pos, 0, TagKind::Opening)?;
        let (entries, next) = parse_time_values(data, pos, 0)?;
        pos = next;
        let day_name = DAY_NAMES
            .get(days.len())
            .ok_or_else(|| malformed("WeeklySchedule has more than 7 days"))?;
        days.push(DaySchedule { day_name: (*day_name).to_string(), entries });
    }
    if days.len() != 7 {
        return Err(malformed(&format!("WeeklySchedule has {} days, expected 7", days.len())));
    }
    Ok(days)
}

/// BACnetDateRange / Date elements of a calendar entry rendered as text; a
/// fully unspecified date is "any".
fn date_or_any(content: &[u8]) -> Result<String, ProtocolError> {
    Ok(format_date(content)?.unwrap_or_else(|| "any".to_string()))
}

/// BACnetWeekNDay ::= OCTET STRING (SIZE(3)): month (1-12, 13 odd, 14 even,
/// 0xFF any), weekOfMonth (1-5 = days 1-7 .. 29-31, 6 = last 7 days, 0xFF any),
/// dayOfWeek (1 = Monday .. 7 = Sunday, 0xFF any).
fn format_week_n_day(content: &[u8]) -> Result<String, ProtocolError> {
    let [month, week, day] = content else {
        return Err(malformed("WeekNDay must be 3 octets"));
    };
    let month = match *month {
        0xFF => "any month".to_string(),
        13 => "odd months".to_string(),
        14 => "even months".to_string(),
        n => format!("month {n:02}"),
    };
    let week = match *week {
        0xFF => "any week".to_string(),
        6 => "last 7 days".to_string(),
        n => format!("week {n}"),
    };
    let day = match *day {
        0xFF => "any day".to_string(),
        n @ 1..=7 => DAY_NAMES[n as usize - 1].to_string(),
        n => format!("day {n}"),
    };
    Ok(format!("{month}, {week}, {day}"))
}

/// BACnetCalendarEntry ::= CHOICE {
///     date      [0] Date,
///     dateRange [1] BACnetDateRange,
///     weekNDay  [2] BACnetWeekNDay }
///
/// Returns the entry as text and the position after it.
fn parse_calendar_entry(data: &[u8], pos: usize) -> Result<(String, usize), ProtocolError> {
    let tag = read_tag(data, pos)?;
    if !tag.context {
        return Err(malformed(&format!("expected BACnetCalendarEntry at offset {pos}")));
    }
    match (tag.kind, tag.number) {
        (TagKind::Primitive, 0) => Ok((date_or_any(tag.content(data, pos))?, tag.end(pos))),
        (TagKind::Opening, 1) => {
            let mut p = tag.end(pos);
            let mut dates = Vec::with_capacity(2);
            for _ in 0..2 {
                let date = read_tag(data, p)?;
                if date.context || date.kind != TagKind::Primitive || date.number != 10 {
                    return Err(malformed(&format!("expected application Date at offset {p}")));
                }
                dates.push(date_or_any(date.content(data, p))?);
                p = date.end(p);
            }
            let p = expect_bracket(data, p, 1, TagKind::Closing)?;
            Ok((format!("{} to {}", dates[0], dates[1]), p))
        }
        (TagKind::Primitive, 2) => Ok((format_week_n_day(tag.content(data, pos))?, tag.end(pos))),
        _ => Err(malformed(&format!("unknown BACnetCalendarEntry choice at offset {pos}"))),
    }
}

/// ExceptionSchedule ::= SEQUENCE OF BACnetSpecialEvent
/// BACnetSpecialEvent ::= SEQUENCE {
///     period CHOICE {
///         calendarEntry     [0] BACnetCalendarEntry,
///         calendarReference [1] BACnetObjectIdentifier },
///     listOfTimeValues [2] SEQUENCE OF BACnetTimeValue,
///     eventPriority    [3] Unsigned (1..16) }
fn parse_exception_schedule(data: &[u8]) -> Result<Vec<SpecialEvent>, ProtocolError> {
    let mut events = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let tag = read_tag(data, pos)?;
        let (period, next) = if tag.context && tag.kind == TagKind::Opening && tag.number == 0 {
            let (entry, next) = parse_calendar_entry(data, tag.end(pos))?;
            (entry, expect_bracket(data, next, 0, TagKind::Closing)?)
        } else if tag.context && tag.kind == TagKind::Primitive && tag.number == 1 {
            let (obj_type, instance) = split_object_id(tag.content(data, pos))?;
            (format!("calendar object_{obj_type}:{instance}"), tag.end(pos))
        } else {
            return Err(malformed(&format!("expected BACnetSpecialEvent period at offset {pos}")));
        };
        let list_start = expect_bracket(data, next, 2, TagKind::Opening)?;
        let (entries, next) = parse_time_values(data, list_start, 2)?;
        let (priority, next) = read_context_primitive(data, next, 3)?;
        let priority = u32::try_from(be_unsigned(priority)?)
            .map_err(|_| malformed("eventPriority out of range"))?;
        pos = next;
        events.push(SpecialEvent { period, priority, entries });
    }
    Ok(events)
}

/// ScheduleDefault is a single application-tagged primitive value.
fn parse_schedule_default(data: &[u8]) -> Result<String, ProtocolError> {
    let (value, end) = parse_application_value(data, 0)?;
    if end != data.len() {
        return Err(malformed("trailing bytes after ScheduleDefault"));
    }
    Ok(value)
}

/// BACnetDateRange ::= SEQUENCE { startDate Date, endDate Date }, both
/// application-tagged: `A4 yy mm dd wd A4 yy mm dd wd`.
fn parse_effective_period(data: &[u8]) -> Result<(Option<String>, Option<String>), ProtocolError> {
    let mut dates = [None, None];
    let mut pos = 0;
    for slot in &mut dates {
        let tag = read_tag(data, pos)?;
        if tag.context || tag.kind != TagKind::Primitive || tag.number != 10 {
            return Err(malformed(&format!("expected application Date at offset {pos}")));
        }
        *slot = format_date(tag.content(data, pos))?;
        pos = tag.end(pos);
    }
    if pos != data.len() {
        return Err(malformed("trailing bytes after EffectivePeriod"));
    }
    let [start, end] = dates;
    Ok((start, end))
}

/// ListOfObjectPropertyReferences ::= SEQUENCE OF BACnetDeviceObjectPropertyReference
/// BACnetDeviceObjectPropertyReference ::= SEQUENCE {
///     [0] objectIdentifier, [1] propertyIdentifier,
///     [2] propertyArrayIndex OPTIONAL, [3] deviceIdentifier OPTIONAL }
fn parse_object_property_references(data: &[u8]) -> Result<Vec<String>, ProtocolError> {
    let mut refs = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (object, next) = read_context_primitive(data, pos, 0)?;
        let (obj_type, instance) = split_object_id(object)?;
        let (property, next) = read_context_primitive(data, next, 1)?;
        let mut text = format!("object_{obj_type}:{instance}.{}", be_unsigned(property)?);
        pos = next;

        if pos < data.len() {
            let tag = read_tag(data, pos)?;
            if tag.context && tag.kind == TagKind::Primitive && tag.number == 2 {
                text.push_str(&format!("[{}]", be_unsigned(tag.content(data, pos))?));
                pos = tag.end(pos);
            }
        }
        if pos < data.len() {
            let tag = read_tag(data, pos)?;
            if tag.context && tag.kind == TagKind::Primitive && tag.number == 3 {
                let (dev_type, dev_instance) = split_object_id(tag.content(data, pos))?;
                text.push_str(&format!(" @ object_{dev_type}:{dev_instance}"));
                pos = tag.end(pos);
            }
        }
        refs.push(text);
    }
    Ok(refs)
}

// ---------------------------------------------------------------------------
// Main public API
// ---------------------------------------------------------------------------

/// Only an Error PDU with code `unknown-property` means the object simply
/// lacks an optional property.
fn is_unknown_property(err: &ProtocolError) -> bool {
    err.to_string().contains("unknown-property")
}

/// A device without segmentation answers a ReadProperty whose value exceeds
/// one APDU with an Abort PDU (clause 18.9).
fn is_abort(err: &ProtocolError) -> bool {
    err.to_string().contains("BACnet Abort from device")
}

/// Read and parse an optional property: `Ok(None)` only for `unknown-property`.
fn read_optional<R, T>(
    read_property: &mut R,
    property_id: u32,
    parse: impl FnOnce(&[u8]) -> Result<T, ProtocolError>,
) -> Result<Option<T>, ProtocolError>
where
    R: FnMut(u32, Option<u32>) -> Result<Vec<u8>, ProtocolError>,
{
    match read_property(property_id, None) {
        Ok(ack) => parse(strip_read_property_ack(&ack, property_id)?).map(Some),
        Err(e) if is_unknown_property(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// How many elements an array property has when it has to be read element by element.
enum ArraySize {
    /// Fixed by the standard (WeeklySchedule has exactly 7).
    Fixed(u32),
    /// Read from array index 0 (clause 15.5.1.1: element 0 is the array size).
    FromIndexZero,
}

/// Upper bound on elements fetched one by one, so a bogus array size cannot
/// trigger an unbounded request storm.
const MAX_ARRAY_ELEMENTS: u32 = 1024;

/// Read an optional array property and return the concatenated BACnetARRAY
/// elements (the bytes inside the `[3]` brackets). `Ok(None)` only for
/// `unknown-property`.
///
/// The whole array is requested first. If the device Aborts because the
/// array does not fit one APDU and it cannot segment, the elements are read
/// one by one with propertyArrayIndex (clause 15.5.1.1).
fn read_array_property<R>(
    read_property: &mut R,
    property_id: u32,
    size: ArraySize,
) -> Result<Option<Vec<u8>>, ProtocolError>
where
    R: FnMut(u32, Option<u32>) -> Result<Vec<u8>, ProtocolError>,
{
    match read_property(property_id, None) {
        Ok(ack) => Ok(Some(strip_read_property_ack(&ack, property_id)?.to_vec())),
        Err(e) if is_unknown_property(&e) => Ok(None),
        Err(e) if is_abort(&e) => {
            let count = match size {
                ArraySize::Fixed(n) => n,
                ArraySize::FromIndexZero => {
                    let ack = read_property(property_id, Some(0))?;
                    let value = strip_read_property_ack(&ack, property_id)?;
                    let tag = read_tag(value, 0)?;
                    if tag.context || tag.kind != TagKind::Primitive || tag.number != 2 {
                        return Err(malformed("array size (index 0) must be an application Unsigned"));
                    }
                    u32::try_from(be_unsigned(tag.content(value, 0))?)
                        .map_err(|_| malformed("array size out of range"))?
                }
            };
            if count > MAX_ARRAY_ELEMENTS {
                return Err(malformed(&format!("array of {count} elements is too large")));
            }
            let mut elements = Vec::new();
            for index in 1..=count {
                let ack = read_property(property_id, Some(index))?;
                elements.extend_from_slice(strip_read_property_ack(&ack, property_id)?);
            }
            Ok(Some(elements))
        }
        Err(e) => Err(e),
    }
}

/// Assemble [`ScheduleData`] using `read_property(property_id, array_index)` to
/// obtain the ReadProperty-ACK service data of each property.
///
/// WeeklySchedule and ExceptionSchedule are both optional, but the standard
/// requires at least one of them (clause 12.24).
fn read_schedule_with<R>(
    object_instance: u32,
    mut read_property: R,
) -> Result<ScheduleData, ProtocolError>
where
    R: FnMut(u32, Option<u32>) -> Result<Vec<u8>, ProtocolError>,
{
    let weekly_schedule = read_array_property(
        &mut read_property,
        PROP_WEEKLY_SCHEDULE,
        ArraySize::Fixed(7),
    )?
    .map(|elements| parse_weekly_schedule(&elements))
    .transpose()?;
    let exception_schedule = read_array_property(
        &mut read_property,
        PROP_EXCEPTION_SCHEDULE,
        ArraySize::FromIndexZero,
    )?
    .map(|elements| parse_exception_schedule(&elements))
    .transpose()?;
    if weekly_schedule.is_none() && exception_schedule.is_none() {
        return Err(ProtocolError::Other(format!(
            "Schedule object {object_instance} has neither WeeklySchedule nor ExceptionSchedule"
        )));
    }

    let schedule_default =
        read_optional(&mut read_property, PROP_SCHEDULE_DEFAULT, parse_schedule_default)?;
    let (effective_period_start, effective_period_end) =
        read_optional(&mut read_property, PROP_EFFECTIVE_PERIOD, parse_effective_period)?
            .unwrap_or((None, None));
    let object_property_references = read_optional(
        &mut read_property,
        PROP_LIST_OF_OBJECT_PROPERTY_REFERENCES,
        parse_object_property_references,
    )?
    .unwrap_or_default();

    Ok(ScheduleData {
        object_instance,
        weekly_schedule: weekly_schedule.unwrap_or_default(),
        exception_schedule: exception_schedule.unwrap_or_default(),
        schedule_default,
        effective_period_start,
        effective_period_end,
        object_property_references,
    })
}

/// Read all schedule properties from a BACnet Schedule object.
///
/// Fails when the object has neither WeeklySchedule nor ExceptionSchedule, or
/// on any failure other than `unknown-property` on an optional property.
pub fn read_schedule_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_instance: u32,
) -> Result<ScheduleData, ProtocolError> {
    read_schedule_with(object_instance, |property_id, array_index| {
        transport::confirmed_request(
            socket,
            target_addr,
            ConfirmedServiceChoice::ReadProperty,
            encode_read_property(object_instance, property_id, array_index),
            transport::REQUEST_TIMEOUT,
        )
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// ReadProperty-ACK for Schedule #1 wrapping `value` in [3] open/close.
    fn ack(property: u8, value: &[u8]) -> Vec<u8> {
        let mut v = vec![
            0x0C, 0x04, 0x40, 0x00, 0x01, // [0] object id: type 17 (Schedule) << 22 | 1
            0x19, property, // [1] property identifier, 1 octet
            0x3E, // [3] opening
        ];
        v.extend_from_slice(value);
        v.push(0x3F); // [3] closing
        v
    }

    /// The 7 BACnetDailySchedule elements of the test WeeklySchedule, Monday first.
    fn weekly_days() -> Vec<Vec<u8>> {
        vec![
            // Monday: 08:00 -> true, 18:00 -> false
            vec![
                0x0E, // [0] opening (day-schedule)
                0xB4, 0x08, 0x00, 0x00, 0x00, // Time 08:00:00.00
                0x11, // application Boolean, value true (in length bits)
                0xB4, 0x12, 0x00, 0x00, 0x00, // Time 18:00:00.00
                0x10, // application Boolean, value false
                0x0F, // [0] closing
            ],
            // Tuesday: 06:30 -> Real 21.5
            vec![
                0x0E, 0xB4, 0x06, 0x1E, 0x00, 0x00, // Time 06:30:00.00
                0x44, 0x41, 0xAC, 0x00, 0x00, // Real 0x41AC0000 = 21.5
                0x0F,
            ],
            // Wednesday .. Saturday: empty
            vec![0x0E, 0x0F],
            vec![0x0E, 0x0F],
            vec![0x0E, 0x0F],
            vec![0x0E, 0x0F],
            // Sunday: 00:00 -> Unsigned 3
            vec![0x0E, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x21, 0x03, 0x0F],
        ]
    }

    fn weekly_value() -> Vec<u8> {
        weekly_days().concat()
    }

    /// ReadProperty-ACK for one array element: `[2] arrayIndex` between property and value.
    fn indexed_ack(property: u8, index: u8, value: &[u8]) -> Vec<u8> {
        let mut v = vec![
            0x0C, 0x04, 0x40, 0x00, 0x01, // [0] object id: Schedule #1
            0x19, property, //               [1] property identifier
            0x29, index, //                  [2] propertyArrayIndex
            0x3E, //                         [3] opening
        ];
        v.extend_from_slice(value);
        v.push(0x3F); //                     [3] closing
        v
    }

    /// ExceptionSchedule elements (BACnetSpecialEvent), assembled by hand from the
    /// ASHRAE 135-2016 clause 21 productions BACnetSpecialEvent / BACnetCalendarEntry /
    /// BACnetWeekNDay (period [0] calendarEntry | [1] calendarReference,
    /// listOfTimeValues [2], eventPriority [3]).
    fn exception_events() -> Vec<Vec<u8>> {
        vec![
            // calendarEntry [0] { date [0] 2024-12-25 (Wednesday) }, 08:00 -> Real 20.0, priority 1
            vec![
                0x0E, 0x0C, 0x7C, 0x0C, 0x19, 0x03, 0x0F, //
                0x2E, 0xB4, 0x08, 0x00, 0x00, 0x00, 0x44, 0x41, 0xA0, 0x00, 0x00, 0x2F, //
                0x39, 0x01,
            ],
            // calendarReference [1] Calendar (type 6) #2 = 0x01800002, 00:00 -> Null, priority 16
            vec![
                0x1C, 0x01, 0x80, 0x00, 0x02, //
                0x2E, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2F, //
                0x39, 0x10,
            ],
            // calendarEntry [0] { dateRange [1] { 2024-06-01, 2024-08-31 } }, no values, priority 5
            vec![
                0x0E, 0x1E, 0xA4, 0x7C, 0x06, 0x01, 0x06, 0xA4, 0x7C, 0x08, 0x1F, 0x06, 0x1F, 0x0F, //
                0x2E, 0x2F, //
                0x39, 0x05,
            ],
            // calendarEntry [0] { weekNDay [2] month 3, week 2, Monday }, no values, priority 3
            vec![0x0E, 0x2B, 0x03, 0x02, 0x01, 0x0F, 0x2E, 0x2F, 0x39, 0x03],
        ]
    }

    fn exception_value() -> Vec<u8> {
        exception_events().concat()
    }

    fn default_ack() -> Vec<u8> {
        ack(174, &[0x44, 0x41, 0xA0, 0x00, 0x00]) // Real 0x41A00000 = 20.0
    }

    fn period_ack() -> Vec<u8> {
        ack(
            32,
            &[
                0xA4, 0x7C, 0x01, 0x01, 0x01, // Date: 1900+124=2024, Jan 1, Monday(1)
                0xA4, 0x7C, 0x0C, 0x1F, 0x02, // Date: 2024, Dec 31, Tuesday(2)
            ],
        )
    }

    fn unknown_property() -> ProtocolError {
        ProtocolError::Other("BACnet Error from device: property / unknown-property".into())
    }

    #[test]
    fn read_property_request_uses_property_54_for_references() {
        assert_eq!(
            encode_read_property(1, PROP_LIST_OF_OBJECT_PROPERTY_REFERENCES, None),
            vec![
                0x0C, 0x04, 0x40, 0x00, 0x01, // [0] Schedule #1
                0x19, 0x36, // [1] property 54 (list-of-object-property-references)
            ]
        );
        assert_eq!(
            encode_read_property(4_194_303, PROP_WEEKLY_SCHEDULE, None),
            vec![
                0x0C, 0x04, 0x7F, 0xFF, 0xFF, // [0] Schedule #4194303 (0x04400000 | 0x3FFFFF)
                0x19, 0x7B, // [1] property 123
            ]
        );
    }

    #[test]
    fn read_property_request_with_array_index() {
        // ReadProperty-Request clause 15.5.1.1: [2] propertyArrayIndex follows the property.
        assert_eq!(
            encode_read_property(1, PROP_EXCEPTION_SCHEDULE, Some(0)),
            vec![
                0x0C, 0x04, 0x40, 0x00, 0x01, // [0] Schedule #1
                0x19, 0x26, //                   [1] property 38 (exception-schedule)
                0x29, 0x00, //                   [2] array index 0 (array size)
            ]
        );
        assert_eq!(
            encode_read_property(1, PROP_SCHEDULE_DEFAULT, Some(300)),
            vec![
                0x0C, 0x04, 0x40, 0x00, 0x01, // [0] Schedule #1
                0x19, 0xAE, //                   [1] property 174 (schedule-default)
                0x2A, 0x01, 0x2C, //             [2] array index 300, two octets
            ]
        );
    }

    #[test]
    fn effective_period_decodes_application_dates() {
        let data = period_ack();
        let value = strip_read_property_ack(&data, 32).unwrap();
        let (start, end) = parse_effective_period(value).unwrap();
        assert_eq!(start.as_deref(), Some("2024-01-01"));
        assert_eq!(end.as_deref(), Some("2024-12-31"));
    }

    #[test]
    fn effective_period_wildcards_are_none() {
        // Both dates entirely unspecified (0xFF everywhere): schedule always effective.
        let value = [0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(parse_effective_period(&value).unwrap(), (None, None));
        // Start year specified, month/day wildcards.
        let value = [0xA4, 0x7C, 0xFF, 0xFF, 0xFF, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF];
        let (start, end) = parse_effective_period(&value).unwrap();
        assert_eq!(start.as_deref(), Some("2024-**-**"));
        assert_eq!(end, None);
    }

    #[test]
    fn effective_period_rejects_context_tagged_dates() {
        // The old (wrong) encoding: [0] and [1] context tags.
        let value = [0x0C, 0x7C, 0x01, 0x01, 0x01, 0x1C, 0x7C, 0x0C, 0x1F, 0x02];
        assert!(parse_effective_period(&value).is_err());
    }

    #[test]
    fn weekly_schedule_decodes_seven_days() {
        let data = ack(123, &weekly_value());
        let value = strip_read_property_ack(&data, 123).unwrap();
        let days = parse_weekly_schedule(value).unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[0].day_name, "Monday");
        assert_eq!(days[6].day_name, "Sunday");
        assert_eq!(days[0].entries.len(), 2);
        assert_eq!(days[0].entries[0].time, "08:00:00");
        assert_eq!(days[0].entries[0].value, "true");
        assert_eq!(days[0].entries[1].time, "18:00:00");
        assert_eq!(days[0].entries[1].value, "false");
        assert_eq!(days[1].entries[0].time, "06:30:00");
        assert_eq!(days[1].entries[0].value, "21.5");
        assert!(days[2..6].iter().all(|d| d.entries.is_empty()));
        assert_eq!(days[6].entries[0].value, "3");
    }

    #[test]
    fn weekly_schedule_with_wrong_day_count_is_error() {
        let value = [0x0E, 0x0F, 0x0E, 0x0F]; // only two days
        assert!(parse_weekly_schedule(&value).is_err());
    }

    #[test]
    fn ack_with_array_index_and_wrong_property() {
        // [2] arrayIndex present between property and value.
        let data = [
            0x0C, 0x04, 0x40, 0x00, 0x01, 0x19, 0x7B, 0x29, 0x01, // index 1
            0x3E, 0x0E, 0x0F, 0x3F,
        ];
        assert_eq!(strip_read_property_ack(&data, 123).unwrap(), &[0x0E, 0x0F]);
        assert!(strip_read_property_ack(&data, 32).is_err());
    }

    #[test]
    fn references_decode_device_object_property_references() {
        let value = [
            0x0C, 0x00, 0x00, 0x00, 0x05, // [0] analog-input #5
            0x19, 0x55, // [1] property 85 (present-value)
            0x0C, 0x00, 0xC0, 0x00, 0x02, // [0] type 3 (binary-input) << 22 | 2
            0x19, 0x55, // [1] present-value
            0x29, 0x02, // [2] array index 2
            0x3C, 0x02, 0x00, 0x00, 0x09, // [3] device #9 (type 8 << 22 | 9)
        ];
        assert_eq!(
            parse_object_property_references(&value).unwrap(),
            vec!["object_0:5.85", "object_3:2.85[2] @ object_8:9"]
        );
    }

    #[test]
    fn read_schedule_assembles_all_properties() {
        let refs = ack(54, &[0x0C, 0x00, 0x00, 0x00, 0x05, 0x19, 0x55]);
        let data = read_schedule_with(1, |p, idx| {
            assert_eq!(idx, None, "whole-property reads only when the device does not Abort");
            match p {
                123 => Ok(ack(123, &weekly_value())),
                38 => Ok(ack(38, &exception_value())),
                174 => Ok(default_ack()),
                32 => Ok(period_ack()),
                54 => Ok(refs.clone()),
                other => panic!("unexpected property {other}"),
            }
        })
        .unwrap();
        assert_eq!(data.weekly_schedule.len(), 7);
        assert_eq!(data.exception_schedule.len(), 4);
        assert_eq!(data.schedule_default.as_deref(), Some("20"));
        assert_eq!(data.effective_period_start.as_deref(), Some("2024-01-01"));
        assert_eq!(data.effective_period_end.as_deref(), Some("2024-12-31"));
        assert_eq!(data.object_property_references, vec!["object_0:5.85"]);
    }

    #[test]
    fn weekly_schedule_failure_propagates() {
        let err = read_schedule_with(1, |_, _| {
            Err(ProtocolError::Timeout("no reply".into()))
        })
        .unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)));

        // Malformed WeeklySchedule data is an error, not an empty schedule.
        assert!(read_schedule_with(1, |_, _| Ok(ack(123, &[0x0E]))).is_err());
    }

    #[test]
    fn schedule_needs_weekly_or_exception_schedule() {
        // ASHRAE 135-2016 clause 12.24: Weekly_Schedule and Exception_Schedule are both
        // optional, but at least one of them shall be present.
        let err = read_schedule_with(1, |_, _| Err(unknown_property())).unwrap_err();
        assert!(err.to_string().contains("neither WeeklySchedule nor ExceptionSchedule"), "{err}");

        // Exception-only schedule: reading succeeds, the weekly list is empty.
        let data = read_schedule_with(1, |p, _| match p {
            38 => Ok(ack(38, &exception_value())),
            _ => Err(unknown_property()),
        })
        .unwrap();
        assert!(data.weekly_schedule.is_empty());
        assert_eq!(data.exception_schedule.len(), 4);

        // Weekly-only schedule: the exception list is empty.
        let data = read_schedule_with(1, |p, _| match p {
            123 => Ok(ack(123, &weekly_value())),
            _ => Err(unknown_property()),
        })
        .unwrap();
        assert_eq!(data.weekly_schedule.len(), 7);
        assert!(data.exception_schedule.is_empty());
    }

    #[test]
    fn optional_property_only_tolerates_unknown_property() {
        let weekly = ack(123, &weekly_value());

        // unknown-property on every optional property: omitted.
        let data = read_schedule_with(1, |p, _| match p {
            123 => Ok(weekly.clone()),
            _ => Err(unknown_property()),
        })
        .unwrap();
        assert_eq!(data.schedule_default, None);
        assert_eq!(data.effective_period_start, None);
        assert_eq!(data.effective_period_end, None);
        assert!(data.object_property_references.is_empty());

        // Timeout on EffectivePeriod: propagates.
        let err = read_schedule_with(1, |p, _| match p {
            123 => Ok(weekly.clone()),
            174 => Ok(default_ack()),
            32 => Err(ProtocolError::Timeout("no reply".into())),
            _ => Err(unknown_property()),
        })
        .unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)));

        // A different Error PDU on the references list: propagates.
        let err = read_schedule_with(1, |p, _| match p {
            123 => Ok(weekly.clone()),
            54 => Err(ProtocolError::Other(
                "BACnet Error from device: object / unknown-object".into(),
            )),
            _ => Err(unknown_property()),
        })
        .unwrap_err();
        assert!(err.to_string().contains("unknown-object"));
    }

    fn abort() -> ProtocolError {
        ProtocolError::Other("BACnet Abort from device: segmentation-not-supported".into())
    }

    #[test]
    fn weekly_schedule_falls_back_to_element_reads_after_abort() {
        // A device that cannot segment Aborts the whole-array ReadProperty; the 7 daily
        // schedules are then read with propertyArrayIndex 1..=7 (clause 15.5.1.1).
        let days = weekly_days();
        let mut requested = Vec::new();
        let data = read_schedule_with(1, |p, idx| {
            requested.push((p, idx));
            match (p, idx) {
                (123, None) => Err(abort()),
                (123, Some(i)) => Ok(indexed_ack(123, i as u8, &days[i as usize - 1])),
                _ => Err(unknown_property()),
            }
        })
        .unwrap();
        assert_eq!(data.weekly_schedule.len(), 7);
        assert_eq!(data.weekly_schedule[1].entries[0].value, "21.5");
        assert_eq!(data.weekly_schedule[6].entries[0].value, "3");
        assert!(requested.contains(&(123, Some(1))) && requested.contains(&(123, Some(7))));
        assert!(!requested.contains(&(123, Some(0))), "WeeklySchedule size is fixed at 7");
    }

    #[test]
    fn exception_schedule_falls_back_to_element_reads_after_abort() {
        // Element count comes from array index 0 (an application Unsigned).
        let events = exception_events();
        let data = read_schedule_with(1, |p, idx| match (p, idx) {
            (38, None) => Err(abort()),
            (38, Some(0)) => Ok(indexed_ack(38, 0, &[0x21, events.len() as u8])),
            (38, Some(i)) => Ok(indexed_ack(38, i as u8, &events[i as usize - 1])),
            _ => Err(unknown_property()),
        })
        .unwrap();
        assert!(data.weekly_schedule.is_empty());
        assert_eq!(data.exception_schedule.len(), 4);
        assert_eq!(data.exception_schedule[3].period, "month 03, week 2, Monday");
    }

    #[test]
    fn element_read_failure_after_abort_propagates() {
        let err = read_schedule_with(1, |p, idx| match (p, idx) {
            (123, None) => Err(abort()),
            (123, Some(_)) => Err(ProtocolError::Timeout("no reply".into())),
            _ => Err(unknown_property()),
        })
        .unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)));

        // Only an Abort triggers the fallback; a Reject is returned as is.
        let err = read_schedule_with(1, |_, idx| {
            assert_eq!(idx, None);
            Err(ProtocolError::Other("BACnet Reject from device: buffer-overflow".into()))
        })
        .unwrap_err();
        assert!(err.to_string().contains("Reject"));
    }

    #[test]
    fn exception_schedule_decodes_all_period_choices() {
        let events = parse_exception_schedule(&exception_value()).unwrap();
        assert_eq!(events.len(), 4);

        // calendarEntry date [0]: 2024-12-25 -> Real 20.0 at 08:00, priority 1
        assert_eq!(events[0].period, "2024-12-25");
        assert_eq!(events[0].priority, 1);
        assert_eq!(events[0].entries.len(), 1);
        assert_eq!(events[0].entries[0].time, "08:00:00");
        assert_eq!(events[0].entries[0].value, "20");

        // calendarReference [1]: Calendar object (type 6) #2, Null at 00:00, priority 16
        assert_eq!(events[1].period, "calendar object_6:2");
        assert_eq!(events[1].priority, 16);
        assert_eq!(events[1].entries[0].value, "Null");

        // calendarEntry dateRange [1]
        assert_eq!(events[2].period, "2024-06-01 to 2024-08-31");
        assert_eq!(events[2].priority, 5);
        assert!(events[2].entries.is_empty());

        // calendarEntry weekNDay [2]: March, 2nd week, Monday
        assert_eq!(events[3].period, "month 03, week 2, Monday");
        assert_eq!(events[3].priority, 3);
    }

    #[test]
    fn week_n_day_wildcards() {
        assert_eq!(
            format_week_n_day(&[0xFF, 0xFF, 0xFF]).unwrap(),
            "any month, any week, any day"
        );
        assert_eq!(
            format_week_n_day(&[13, 6, 7]).unwrap(),
            "odd months, last 7 days, Sunday"
        );
        assert_eq!(format_week_n_day(&[14, 1, 5]).unwrap(), "even months, week 1, Friday");
        assert!(format_week_n_day(&[1, 2]).is_err());
    }

    #[test]
    fn exception_schedule_rejects_truncated_event() {
        let mut value = exception_value();
        value.truncate(12); // cuts inside the first event's time-value list
        assert!(parse_exception_schedule(&value).is_err());
    }

    #[test]
    fn application_values() {
        assert_eq!(parse_application_value(&[0x44, 0x42, 0x90, 0x00, 0x00], 0).unwrap(), ("72".into(), 5));
        assert_eq!(parse_application_value(&[0x21, 42], 0).unwrap(), ("42".into(), 2));
        assert_eq!(parse_application_value(&[0x91, 3], 0).unwrap(), ("3".into(), 2));
        assert_eq!(parse_application_value(&[0x00], 0).unwrap(), ("Null".into(), 1));
        // Signed, 1 octet, 0xFE = -2
        assert_eq!(parse_application_value(&[0x31, 0xFE], 0).unwrap(), ("-2".into(), 2));
        // CharacterString length 3: encoding 0 (UTF-8) + "hi"
        assert_eq!(
            parse_application_value(&[0x73, 0x00, b'h', b'i'], 0).unwrap(),
            ("hi".into(), 4)
        );
        // Truncated Real is an error.
        assert!(parse_application_value(&[0x44, 0x42, 0x90], 0).is_err());
    }
}
