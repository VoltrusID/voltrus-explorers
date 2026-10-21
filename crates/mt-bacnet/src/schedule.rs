//! BACnet Schedule object reader (ASHRAE 135 clause 12.24).
//!
//! Reads, with plain ReadProperty requests:
//! - WeeklySchedule (property 123) — 7 × BACnetDailySchedule, Monday..Sunday
//! - ScheduleDefault (property 174) — fallback value
//! - EffectivePeriod (property 32) — BACnetDateRange
//! - ListOfObjectPropertyReferences (property 54)
//!
//! WeeklySchedule is mandatory: any failure reading it fails the whole call.
//! The other properties are optional: only an Error PDU with error code
//! `unknown-property` means "the object does not have it"; every other failure
//! (timeout, abort, reject, other errors, malformed data) is propagated.

use std::net::SocketAddr;

use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

use crate::transport;

/// Object type number of the Schedule object (BACnetObjectType 17).
const SCHEDULE_OBJECT_TYPE: u32 = 17;

const PROP_EFFECTIVE_PERIOD: u32 = 32;
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

/// Complete schedule data for a BACnet Schedule object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleData {
    /// Object instance number of the Schedule object.
    pub object_instance: u32,
    /// Weekly schedule (7 days, Monday..Sunday).
    pub weekly_schedule: Vec<DaySchedule>,
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

/// ReadProperty-Request service data (clause 15.5.1.1):
/// `[0] objectIdentifier, [1] propertyIdentifier`.
fn encode_read_property(object_instance: u32, property_id: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(11);
    // Context tag 0, length 4: object id = type << 22 | instance.
    buf.push(0x0C);
    buf.extend_from_slice(&((SCHEDULE_OBJECT_TYPE << 22) | (object_instance & 0x3F_FFFF)).to_be_bytes());
    // Context tag 1, minimal-length unsigned property id.
    let bytes = property_id.to_be_bytes();
    let skip = bytes.iter().take(3).take_while(|b| **b == 0).count();
    let value = &bytes[skip..];
    buf.push(0x18 | value.len() as u8);
    buf.extend_from_slice(value);
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

/// WeeklySchedule ::= SEQUENCE SIZE(7) OF BACnetDailySchedule
/// BACnetDailySchedule ::= SEQUENCE { [0] day-schedule SEQUENCE OF BACnetTimeValue }
/// BACnetTimeValue ::= SEQUENCE { time Time, value ABSTRACT-SYNTAX.&Type }
///
/// Wire form of each day: `0E {B4 hh mm ss hs <app-value>}* 0F`.
fn parse_weekly_schedule(data: &[u8]) -> Result<Vec<DaySchedule>, ProtocolError> {
    let mut days = Vec::with_capacity(7);
    let mut pos = 0;
    while pos < data.len() {
        pos = expect_bracket(data, pos, 0, TagKind::Opening)?;
        let mut entries = Vec::new();
        loop {
            let tag = read_tag(data, pos)?;
            if tag.context && tag.kind == TagKind::Closing && tag.number == 0 {
                pos = tag.end(pos);
                break;
            }
            if tag.context || tag.kind != TagKind::Primitive || tag.number != 11 {
                return Err(malformed(&format!("expected Time in day schedule at offset {pos}")));
            }
            let time = format_time(tag.content(data, pos))?;
            let (value, next) = parse_application_value(data, tag.end(pos))?;
            entries.push(ScheduleEntry { time, value });
            pos = next;
        }
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

/// Read and parse an optional property: `Ok(None)` only for `unknown-property`.
fn read_optional<R, T>(
    read_property: &mut R,
    property_id: u32,
    parse: impl FnOnce(&[u8]) -> Result<T, ProtocolError>,
) -> Result<Option<T>, ProtocolError>
where
    R: FnMut(u32) -> Result<Vec<u8>, ProtocolError>,
{
    match read_property(property_id) {
        Ok(ack) => parse(strip_read_property_ack(&ack, property_id)?).map(Some),
        Err(e) if is_unknown_property(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Assemble [`ScheduleData`] using `read_property(property_id)` to obtain the
/// ReadProperty-ACK service data of each property.
fn read_schedule_with<R>(
    object_instance: u32,
    mut read_property: R,
) -> Result<ScheduleData, ProtocolError>
where
    R: FnMut(u32) -> Result<Vec<u8>, ProtocolError>,
{
    let weekly_ack = read_property(PROP_WEEKLY_SCHEDULE)?;
    let weekly_schedule =
        parse_weekly_schedule(strip_read_property_ack(&weekly_ack, PROP_WEEKLY_SCHEDULE)?)?;

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
        weekly_schedule,
        schedule_default,
        effective_period_start,
        effective_period_end,
        object_property_references,
    })
}

/// Read all schedule properties from a BACnet Schedule object.
///
/// Failure to read WeeklySchedule, or any failure other than
/// `unknown-property` on an optional property, is returned as an error.
pub fn read_schedule_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    object_instance: u32,
) -> Result<ScheduleData, ProtocolError> {
    read_schedule_with(object_instance, |property_id| {
        transport::confirmed_request(
            socket,
            target_addr,
            ConfirmedServiceChoice::ReadProperty,
            encode_read_property(object_instance, property_id),
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

    fn weekly_value() -> Vec<u8> {
        let mut v = Vec::new();
        // Monday: 08:00 -> true, 18:00 -> false
        v.extend_from_slice(&[
            0x0E, // [0] opening (day-schedule)
            0xB4, 0x08, 0x00, 0x00, 0x00, // Time 08:00:00.00
            0x11, // application Boolean, value true (in length bits)
            0xB4, 0x12, 0x00, 0x00, 0x00, // Time 18:00:00.00
            0x10, // application Boolean, value false
            0x0F, // [0] closing
        ]);
        // Tuesday: 06:30 -> Real 21.5
        v.extend_from_slice(&[
            0x0E, 0xB4, 0x06, 0x1E, 0x00, 0x00, // Time 06:30:00.00
            0x44, 0x41, 0xAC, 0x00, 0x00, // Real 0x41AC0000 = 21.5
            0x0F,
        ]);
        // Wednesday .. Saturday: empty
        for _ in 0..4 {
            v.extend_from_slice(&[0x0E, 0x0F]);
        }
        // Sunday: 00:00 -> Unsigned 3
        v.extend_from_slice(&[0x0E, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x21, 0x03, 0x0F]);
        v
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
            encode_read_property(1, PROP_LIST_OF_OBJECT_PROPERTY_REFERENCES),
            vec![
                0x0C, 0x04, 0x40, 0x00, 0x01, // [0] Schedule #1
                0x19, 0x36, // [1] property 54 (list-of-object-property-references)
            ]
        );
        assert_eq!(
            encode_read_property(4_194_303, PROP_WEEKLY_SCHEDULE),
            vec![
                0x0C, 0x04, 0x7F, 0xFF, 0xFF, // [0] Schedule #4194303 (0x04400000 | 0x3FFFFF)
                0x19, 0x7B, // [1] property 123
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
        let data = read_schedule_with(1, |p| match p {
            123 => Ok(ack(123, &weekly_value())),
            174 => Ok(default_ack()),
            32 => Ok(period_ack()),
            54 => Ok(refs.clone()),
            other => panic!("unexpected property {other}"),
        })
        .unwrap();
        assert_eq!(data.weekly_schedule.len(), 7);
        assert_eq!(data.schedule_default.as_deref(), Some("20"));
        assert_eq!(data.effective_period_start.as_deref(), Some("2024-01-01"));
        assert_eq!(data.effective_period_end.as_deref(), Some("2024-12-31"));
        assert_eq!(data.object_property_references, vec!["object_0:5.85"]);
    }

    #[test]
    fn weekly_schedule_failure_propagates() {
        let err = read_schedule_with(1, |_| {
            Err(ProtocolError::Timeout("no reply".into()))
        })
        .unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)));

        // Even unknown-property on the mandatory WeeklySchedule is an error.
        assert!(read_schedule_with(1, |_| Err(unknown_property())).is_err());

        // Malformed WeeklySchedule data is an error, not an empty schedule.
        assert!(read_schedule_with(1, |_| Ok(ack(123, &[0x0E]))).is_err());
    }

    #[test]
    fn optional_property_only_tolerates_unknown_property() {
        let weekly = ack(123, &weekly_value());

        // unknown-property on every optional property: omitted.
        let data = read_schedule_with(1, |p| match p {
            123 => Ok(weekly.clone()),
            _ => Err(unknown_property()),
        })
        .unwrap();
        assert_eq!(data.schedule_default, None);
        assert_eq!(data.effective_period_start, None);
        assert_eq!(data.effective_period_end, None);
        assert!(data.object_property_references.is_empty());

        // Timeout on EffectivePeriod: propagates.
        let err = read_schedule_with(1, |p| match p {
            123 => Ok(weekly.clone()),
            174 => Ok(default_ack()),
            32 => Err(ProtocolError::Timeout("no reply".into())),
            _ => Err(unknown_property()),
        })
        .unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)));

        // A different Error PDU on the references list: propagates.
        let err = read_schedule_with(1, |p| match p {
            123 => Ok(weekly.clone()),
            54 => Err(ProtocolError::Other(
                "BACnet Error from device: object / unknown-object".into(),
            )),
            _ => Err(unknown_property()),
        })
        .unwrap_err();
        assert!(err.to_string().contains("unknown-object"));
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
