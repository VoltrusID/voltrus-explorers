//! BACnet Trend Log reading (ASHRAE 135 clause 15.8 ReadRange, clause 12.25 Trend Log object).
//!
//! `bacnet-rs` only defines `ConfirmedServiceChoice::ReadRange` (value 26); the request
//! encoding and the ReadRange-ACK / BACnetLogRecord decoding are implemented here.

use std::net::SocketAddr;

use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier};
use bacnet_rs::service::ConfirmedServiceChoice;

use mt_protocol::ProtocolError;

use crate::transport::{self, confirmed_request, REQUEST_TIMEOUT};

/// Which Trend Log object type is being read. Both use LogBuffer (131) and
/// RecordCount (141) but have different log record layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendLogKind {
    /// Trend Log (object type 20): BACnetLogRecord items.
    TrendLog,
    /// Trend Log Multiple (object type 27): BACnetLogMultipleRecord items.
    TrendLogMultiple,
}

impl TrendLogKind {
    /// Parse the object type name used by the UI (`trendLog` / `trendLogMultiple`).
    pub fn parse(name: &str) -> Result<Self, ProtocolError> {
        match name {
            "trendLog" => Ok(Self::TrendLog),
            "trendLogMultiple" => Ok(Self::TrendLogMultiple),
            other => Err(bad(format!(
                "unsupported object type '{other}': expected trendLog or trendLogMultiple"
            ))),
        }
    }

    fn object_type(self) -> ObjectType {
        match self {
            Self::TrendLog => ObjectType::TrendLog,
            Self::TrendLogMultiple => ObjectType::TrendLogMultiple,
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single log record from a TrendLog object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogRecord {
    /// ISO 8601 formatted timestamp (unspecified fields are rendered as `*`).
    pub timestamp: String,
    /// Display string of the logged value.
    pub value: String,
    /// Status flags, if present.
    pub status_flags: Option<String>,
}

/// Result of one ReadRange request on a TrendLog LogBuffer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrendLogData {
    /// Object instance number of the TrendLog.
    pub object_instance: u32,
    /// Records returned by the device, oldest first.
    pub records: Vec<LogRecord>,
    /// Number of records the device reports it returned (`itemCount`).
    pub item_count: u32,
    /// The first returned record is the first record of the log.
    pub first_item: bool,
    /// The last returned record is the last record of the log.
    pub last_item: bool,
    /// More records exist beyond the returned range.
    pub more_items: bool,
    /// Sequence number of the first returned record, if the device reports it.
    pub first_sequence_number: Option<u32>,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Application-tagged Unsigned (tag 2), minimal length.
fn encode_app_unsigned(buf: &mut Vec<u8>, value: u32) {
    let bytes = value.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(3);
    buf.push(0x20 | (4 - start) as u8);
    buf.extend_from_slice(&bytes[start..]);
}

/// Application-tagged Signed (tag 3), minimal two's-complement length.
fn encode_app_signed(buf: &mut Vec<u8>, value: i32) {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < 3
        && ((bytes[start] == 0x00 && bytes[start + 1] & 0x80 == 0)
            || (bytes[start] == 0xFF && bytes[start + 1] & 0x80 != 0))
    {
        start += 1;
    }
    buf.push(0x30 | (4 - start) as u8);
    buf.extend_from_slice(&bytes[start..]);
}

/// Context-tagged Unsigned / Enumerated (tags 0-14), minimal length.
fn encode_context_unsigned(buf: &mut Vec<u8>, tag_num: u8, value: u32) {
    let bytes = value.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(3);
    buf.push((tag_num << 4) | 0x08 | (4 - start) as u8);
    buf.extend_from_slice(&bytes[start..]);
}

/// Context-tagged BACnetObjectIdentifier.
fn encode_context_object_id(buf: &mut Vec<u8>, tag_num: u8, obj_type: ObjectType, instance: u32) {
    let type_u32: u32 = obj_type.into();
    let value = (type_u32 << 22) | (instance & 0x3F_FFFF);
    buf.push((tag_num << 4) | 0x0C);
    buf.extend_from_slice(&value.to_be_bytes());
}

/// Build the ReadRange-Request service data for a TrendLog's LogBuffer using
/// the `byPosition` range (clause 15.8.1.1.1):
///
/// ```text
/// ReadRange-Request ::= SEQUENCE {
///     objectIdentifier   [0] BACnetObjectIdentifier,
///     propertyIdentifier [1] BACnetPropertyIdentifier,
///     propertyArrayIndex [2] Unsigned OPTIONAL,
///     range CHOICE {
///         byPosition [3] SEQUENCE { referenceIndex Unsigned, count INTEGER },
///         ...
///     }
/// }
/// ```
///
/// `referenceIndex` and `count` are application-tagged. A negative `count`
/// returns the `|count|` items that END at `referenceIndex`.
fn build_read_range_request(
    kind: TrendLogKind,
    object_instance: u32,
    reference_index: u32,
    count: i32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    encode_context_object_id(&mut buf, 0, kind.object_type(), object_instance);
    encode_context_unsigned(&mut buf, 1, u32::from(PropertyIdentifier::LogBuffer));
    buf.push(0x3E); // opening [3] byPosition
    encode_app_unsigned(&mut buf, reference_index);
    encode_app_signed(&mut buf, count);
    buf.push(0x3F); // closing [3]
    buf
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

fn bad(msg: impl Into<String>) -> ProtocolError {
    ProtocolError::ReadFailed(msg.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Opening,
    Closing,
    /// Primitive with this many content octets (0 for application Boolean,
    /// whose value lives in the length field).
    Value(usize),
}

#[derive(Debug, Clone, Copy)]
struct Tag {
    number: u8,
    context: bool,
    kind: Kind,
    /// Octets occupied by the tag header (tag byte + extensions).
    header: usize,
}

fn read_tag(data: &[u8], pos: usize) -> Result<Tag, ProtocolError> {
    let truncated = || bad("truncated BACnet tag");
    let b = *data.get(pos).ok_or_else(truncated)?;
    let context = b & 0x08 != 0;
    let mut header = 1;
    let mut number = b >> 4;
    if number == 0x0F {
        number = *data.get(pos + 1).ok_or_else(truncated)?;
        header = 2;
    }
    let lvt = (b & 0x07) as usize;
    let kind = if context && lvt == 6 {
        Kind::Opening
    } else if context && lvt == 7 {
        Kind::Closing
    } else if !context && number == 1 {
        Kind::Value(0)
    } else if lvt == 5 {
        let first = *data.get(pos + header).ok_or_else(truncated)?;
        header += 1;
        let len = match first {
            254 => {
                let s = data.get(pos + header..pos + header + 2).ok_or_else(truncated)?;
                header += 2;
                u16::from_be_bytes([s[0], s[1]]) as usize
            }
            255 => {
                let s = data.get(pos + header..pos + header + 4).ok_or_else(truncated)?;
                header += 4;
                u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize
            }
            n => n as usize,
        };
        Kind::Value(len)
    } else {
        Kind::Value(lvt)
    };
    Ok(Tag { number, context, kind, header })
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn peek(&self) -> Result<Option<Tag>, ProtocolError> {
        if self.pos >= self.data.len() {
            Ok(None)
        } else {
            read_tag(self.data, self.pos).map(Some)
        }
    }

    fn next_tag(&self) -> Result<Tag, ProtocolError> {
        self.peek()?.ok_or_else(|| bad("unexpected end of data"))
    }

    fn open(&mut self, number: u8) -> Result<(), ProtocolError> {
        let t = self.next_tag()?;
        if !(t.context && t.number == number && t.kind == Kind::Opening) {
            return Err(bad(format!("expected opening tag [{number}]")));
        }
        self.pos += t.header;
        Ok(())
    }

    fn close(&mut self, number: u8) -> Result<(), ProtocolError> {
        let t = self.next_tag()?;
        if !(t.context && t.number == number && t.kind == Kind::Closing) {
            return Err(bad(format!("expected closing tag [{number}]")));
        }
        self.pos += t.header;
        Ok(())
    }

    fn at_closing(&self, number: u8) -> Result<bool, ProtocolError> {
        let t = self.next_tag()?;
        Ok(t.context && t.number == number && t.kind == Kind::Closing)
    }

    fn is_opening(&self, number: u8) -> Result<bool, ProtocolError> {
        Ok(matches!(self.peek()?, Some(t) if t.context && t.number == number && t.kind == Kind::Opening))
    }

    /// Consume a primitive element and return its content octets.
    fn value(&mut self) -> Result<(Tag, &'a [u8]), ProtocolError> {
        let t = self.next_tag()?;
        let Kind::Value(len) = t.kind else {
            return Err(bad("expected a primitive value, found opening/closing tag"));
        };
        let start = self.pos + t.header;
        let content = self
            .data
            .get(start..start + len)
            .ok_or_else(|| bad("truncated BACnet value"))?;
        self.pos = start + len;
        Ok((t, content))
    }

    fn context(&mut self, number: u8) -> Result<&'a [u8], ProtocolError> {
        self.optional_context(number)?
            .ok_or_else(|| bad(format!("missing context tag [{number}]")))
    }

    fn optional_context(&mut self, number: u8) -> Result<Option<&'a [u8]>, ProtocolError> {
        match self.peek()? {
            Some(t) if t.context && t.number == number && matches!(t.kind, Kind::Value(_)) => {
                self.value().map(|(_, c)| Some(c))
            }
            _ => Ok(None),
        }
    }

    fn app(&mut self, number: u8) -> Result<&'a [u8], ProtocolError> {
        let t = self.next_tag()?;
        if t.context || t.number != number || !matches!(t.kind, Kind::Value(_)) {
            return Err(bad(format!("expected application tag {number}")));
        }
        self.value().map(|(_, c)| c)
    }

    /// Skip one element; an opening tag is skipped through its matching closing tag.
    fn skip(&mut self) -> Result<(), ProtocolError> {
        let t = self.next_tag()?;
        match t.kind {
            Kind::Value(_) => {
                self.value()?;
            }
            Kind::Closing => return Err(bad("unexpected closing tag")),
            Kind::Opening => {
                self.pos += t.header;
                let mut depth = 1;
                while depth > 0 {
                    let inner = self.next_tag()?;
                    match inner.kind {
                        Kind::Opening => {
                            depth += 1;
                            self.pos += inner.header;
                        }
                        Kind::Closing => {
                            depth -= 1;
                            self.pos += inner.header;
                        }
                        Kind::Value(_) => {
                            self.value()?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Decode a 1-4 octet unsigned integer.
fn decode_unsigned(bytes: &[u8]) -> Result<u32, ProtocolError> {
    if bytes.is_empty() || bytes.len() > 4 {
        return Err(bad(format!("unsupported Unsigned length {}", bytes.len())));
    }
    Ok(bytes.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32))
}

/// Decode a 1-4 octet two's-complement signed integer.
fn decode_signed(bytes: &[u8]) -> Result<i32, ProtocolError> {
    if bytes.is_empty() || bytes.len() > 4 {
        return Err(bad(format!("unsupported Signed length {}", bytes.len())));
    }
    let seed: i32 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
    Ok(bytes.iter().fold(seed, |acc, &b| (acc << 8) | b as i32))
}

/// Decode BitString content octets (unused-bit count, then bits MSB first).
fn decode_bits(bytes: &[u8]) -> Result<Vec<bool>, ProtocolError> {
    let (&unused, data) = bytes.split_first().ok_or_else(|| bad("empty BitString"))?;
    if unused > 7 || (data.is_empty() && unused != 0) {
        return Err(bad("invalid BitString unused-bit count"));
    }
    let total = data.len() * 8 - unused as usize;
    Ok((0..total).map(|i| data[i / 8] & (0x80 >> (i % 8)) != 0).collect())
}

fn set_names(bits: &[bool], names: &[&str]) -> Vec<String> {
    bits.iter()
        .enumerate()
        .filter(|(_, set)| **set)
        .map(|(i, _)| names.get(i).map_or_else(|| format!("bit{i}"), |n| (*n).to_string()))
        .collect()
}

/// BACnetStatusFlags: in-alarm(0), fault(1), overridden(2), out-of-service(3).
/// Returns `None` when no flag is set.
fn status_flags_to_string(bytes: &[u8]) -> Result<Option<String>, ProtocolError> {
    let names = set_names(
        &decode_bits(bytes)?,
        &["inAlarm", "fault", "overridden", "outOfService"],
    );
    Ok((!names.is_empty()).then(|| names.join(", ")))
}

fn field(v: u8, width: usize) -> String {
    if v == 0xFF {
        "*".repeat(width)
    } else {
        format!("{v:0width$}")
    }
}

/// Decode an application Date + Time pair (BACnetDateTime body) into ISO 8601.
/// Date octets: year-1900, month, day, weekday; Time: hour, minute, second, hundredths.
fn parse_datetime(r: &mut Reader) -> Result<String, ProtocolError> {
    let date = r.app(10)?;
    let time = r.app(11)?;
    if date.len() != 4 || time.len() != 4 {
        return Err(bad("BACnetDateTime Date/Time must be 4 octets each"));
    }
    let year = if date[0] == 0xFF {
        "****".to_string()
    } else {
        format!("{:04}", 1900 + date[0] as u16)
    };
    Ok(format!(
        "{year}-{}-{}T{}:{}:{}",
        field(date[1], 2),
        field(date[2], 2),
        field(time[0], 2),
        field(time[1], 2),
        field(time[2], 2),
    ))
}

fn log_status_string(bytes: &[u8]) -> Result<String, ProtocolError> {
    let names = set_names(
        &decode_bits(bytes)?,
        &["log-disabled", "buffer-purged", "log-interrupted"],
    );
    Ok(if names.is_empty() {
        "log-status: (none)".to_string()
    } else {
        format!("log-status: {}", names.join(", "))
    })
}

fn time_change_string(v: f32) -> String {
    format!("time-change: {v}s")
}

fn decode_bool(bytes: &[u8]) -> Result<bool, ProtocolError> {
    match bytes {
        [b] => Ok(*b != 0),
        _ => Err(bad("boolean-value must be 1 octet")),
    }
}

fn decode_real(bytes: &[u8]) -> Result<f32, ProtocolError> {
    let raw: [u8; 4] = bytes
        .try_into()
        .map_err(|_| bad("real-value must be 4 octets"))?;
    Ok(f32::from_be_bytes(raw))
}

fn decode_null(bytes: &[u8]) -> Result<String, ProtocolError> {
    if bytes.is_empty() {
        Ok("null".to_string())
    } else {
        Err(bad("null-value must be empty"))
    }
}

fn bits_string(bytes: &[u8]) -> Result<String, ProtocolError> {
    Ok(decode_bits(bytes)?
        .iter()
        .map(|&b| if b { '1' } else { '0' })
        .collect())
}

/// Error ::= SEQUENCE { error-class, error-code } (application enumerated) inside
/// opening/closing `[number]`; returns "Class / Code".
fn parse_error_names(r: &mut Reader, number: u8) -> Result<String, ProtocolError> {
    r.open(number)?;
    let class = decode_unsigned(r.app(9)?)?;
    let code = decode_unsigned(r.app(9)?)?;
    r.close(number)?;
    Ok(format!(
        "{} / {}",
        transport::error_class_name(class),
        transport::error_code_name(code)
    ))
}

/// Decode the BACnetLogRecord.logDatum CHOICE body (inside opening/closing [1]).
fn parse_log_datum(r: &mut Reader) -> Result<String, ProtocolError> {
    let tag = r.next_tag()?;
    if !tag.context {
        return Err(bad("logDatum must be context-tagged"));
    }
    match tag.kind {
        Kind::Closing => Err(bad("empty logDatum")),
        Kind::Opening => match tag.number {
            8 => Ok(format!("failure: {}", parse_error_names(r, 8)?)),
            10 => {
                r.skip()?;
                Ok("[any-value]".to_string())
            }
            n => Err(bad(format!("unsupported logDatum choice [{n}]"))),
        },
        Kind::Value(_) => {
            let n = tag.number;
            let bytes = r.context(n)?;
            match n {
                0 => log_status_string(bytes),
                1 => Ok(decode_bool(bytes)?.to_string()),
                2 => Ok(decode_real(bytes)?.to_string()),
                3 | 4 => Ok(decode_unsigned(bytes)?.to_string()),
                5 => Ok(decode_signed(bytes)?.to_string()),
                6 => bits_string(bytes),
                7 => decode_null(bytes),
                9 => Ok(time_change_string(decode_real(bytes)?)),
                n => Err(bad(format!("unsupported logDatum choice [{n}]"))),
            }
        }
    }
}

/// BACnetLogRecord ::= SEQUENCE {
///     timestamp   [0] BACnetDateTime,
///     logDatum    [1] CHOICE { ... },
///     statusFlags [2] BACnetStatusFlags OPTIONAL
/// }
fn parse_log_record(r: &mut Reader) -> Result<LogRecord, ProtocolError> {
    r.open(0)?;
    let timestamp = parse_datetime(r)?;
    r.close(0)?;
    r.open(1)?;
    let value = parse_log_datum(r)?;
    r.close(1)?;
    let status_flags = match r.optional_context(2)? {
        Some(bytes) => status_flags_to_string(bytes)?,
        None => None,
    };
    Ok(LogRecord { timestamp, value, status_flags })
}

/// BACnetLogMultipleRecord (clause 12.30), the item type of a TrendLogMultiple LogBuffer:
///
/// ```text
/// BACnetLogMultipleRecord ::= SEQUENCE {
///     timestamp [0] BACnetDateTime,
///     log-data  [1] BACnetLogData
/// }
/// BACnetLogData ::= CHOICE {
///     log-status [0] BACnetLogStatus,
///     log-data   [1] SEQUENCE OF CHOICE { ... },
///     time-change [2] REAL
/// }
/// ```
///
/// The record field `[1]` is explicitly tagged, so the choice is nested inside its own
/// opening/closing [1] pair. The values are joined with ", " into `value`.
fn parse_log_multiple_record(r: &mut Reader) -> Result<LogRecord, ProtocolError> {
    r.open(0)?;
    let timestamp = parse_datetime(r)?;
    r.close(0)?;
    r.open(1)?;
    let choice = r.next_tag()?;
    let value = match (choice.context, choice.number, choice.kind) {
        (true, 0, Kind::Value(_)) => log_status_string(r.context(0)?)?,
        (true, 1, Kind::Opening) => {
            r.open(1)?;
            let mut values = Vec::new();
            while !r.at_closing(1)? {
                values.push(parse_log_multiple_value(r)?);
            }
            r.close(1)?;
            values.join(", ")
        }
        (true, 2, Kind::Value(_)) => time_change_string(decode_real(r.context(2)?)?),
        _ => return Err(bad("unsupported BACnetLogData choice")),
    };
    r.close(1)?;
    Ok(LogRecord { timestamp, value, status_flags: None })
}

/// One BACnetLogDataValue of a TrendLogMultiple record:
/// [0] boolean, [1] real, [2] enumerated, [3] unsigned, [4] signed, [5] bitstring,
/// [6] null, [7] failure (BACnetError), [8] any-value.
fn parse_log_multiple_value(r: &mut Reader) -> Result<String, ProtocolError> {
    let tag = r.next_tag()?;
    if !tag.context {
        return Err(bad("log-data value must be context-tagged"));
    }
    match tag.kind {
        Kind::Closing => Err(bad("unexpected closing tag in log-data")),
        Kind::Opening => match tag.number {
            7 => Ok(format!("failure({})", parse_error_names(r, 7)?)),
            8 => {
                r.skip()?;
                Ok("[any-value]".to_string())
            }
            n => Err(bad(format!("unsupported log-data choice [{n}]"))),
        },
        Kind::Value(_) => {
            let bytes = r.context(tag.number)?;
            match tag.number {
                0 => Ok(decode_bool(bytes)?.to_string()),
                1 => Ok(decode_real(bytes)?.to_string()),
                2 | 3 => Ok(decode_unsigned(bytes)?.to_string()),
                4 => Ok(decode_signed(bytes)?.to_string()),
                5 => bits_string(bytes),
                6 => decode_null(bytes),
                n => Err(bad(format!("unsupported log-data choice [{n}]"))),
            }
        }
    }
}

/// Parse ReadRange-ACK service data (clause 15.8.2):
///
/// ```text
/// ReadRange-ACK ::= SEQUENCE {
///     objectIdentifier     [0] BACnetObjectIdentifier,
///     propertyIdentifier   [1] BACnetPropertyIdentifier,
///     propertyArrayIndex   [2] Unsigned OPTIONAL,
///     resultFlags          [3] BACnetResultFlags (first-item, last-item, more-items),
///     itemCount            [4] Unsigned,
///     itemData             [5] SEQUENCE OF ABSTRACT-SYNTAX.&Type,
///     firstSequenceNumber  [6] Unsigned32 OPTIONAL
/// }
/// ```
fn parse_read_range_ack(
    data: &[u8],
    kind: TrendLogKind,
    object_instance: u32,
) -> Result<TrendLogData, ProtocolError> {
    let mut r = Reader::new(data);
    r.context(0)?;
    r.context(1)?;
    r.optional_context(2)?;
    let flags = decode_bits(r.context(3)?)?;
    let flag = |i: usize| flags.get(i).copied().unwrap_or(false);
    let item_count = decode_unsigned(r.context(4)?)?;

    let mut records = Vec::new();
    if r.is_opening(5)? {
        r.open(5)?;
        while !r.at_closing(5)? {
            records.push(match kind {
                TrendLogKind::TrendLog => parse_log_record(&mut r)?,
                TrendLogKind::TrendLogMultiple => parse_log_multiple_record(&mut r)?,
            });
        }
        r.close(5)?;
    }
    let first_sequence_number = r.optional_context(6)?.map(decode_unsigned).transpose()?;

    Ok(TrendLogData {
        object_instance,
        records,
        item_count,
        first_item: flag(0),
        last_item: flag(1),
        more_items: flag(2),
        first_sequence_number,
    })
}

/// Parse a ReadProperty-ACK for an Unsigned property (RecordCount):
/// `[0] objectIdentifier, [1] propertyIdentifier, [2] arrayIndex OPTIONAL,
/// [3] opening, application Unsigned, [3] closing`.
fn parse_record_count_ack(data: &[u8]) -> Result<u32, ProtocolError> {
    let mut r = Reader::new(data);
    r.context(0)?;
    r.context(1)?;
    r.optional_context(2)?;
    r.open(3)?;
    let count = decode_unsigned(r.app(2)?)?;
    r.close(3)?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Read TrendLog records with ReadRange `byPosition` on the LogBuffer property.
///
/// - `reference_index`: 1-based position of the reference record.
/// - `count`: positive returns `count` records starting at `reference_index`;
///   negative returns `|count|` records ending at `reference_index`
///   (so `reference_index = total, count = -N` is the newest N records).
///
/// Records are returned oldest first.
pub fn read_trend_log_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    kind: TrendLogKind,
    object_instance: u32,
    reference_index: u32,
    count: i32,
) -> Result<TrendLogData, ProtocolError> {
    if reference_index == 0 {
        return Err(bad("ReadRange reference index is 1-based; 0 is invalid"));
    }
    if count == 0 {
        return Err(bad("ReadRange count must not be 0"));
    }
    let service_data = build_read_range_request(kind, object_instance, reference_index, count);
    let response = confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::ReadRange,
        service_data,
        REQUEST_TIMEOUT,
    )?;
    parse_read_range_ack(&response, kind, object_instance)
}

/// Read the RecordCount property of a TrendLog object with ReadProperty.
pub fn read_trend_log_count_blocking(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    kind: TrendLogKind,
    object_instance: u32,
) -> Result<u32, ProtocolError> {
    let obj_id = ObjectIdentifier::new(kind.object_type(), object_instance);
    let request =
        bacnet_rs::service::ReadPropertyRequest::new(obj_id, PropertyIdentifier::RecordCount);
    let mut service_data = Vec::new();
    request
        .encode(&mut service_data)
        .map_err(|e| ProtocolError::WriteFailed(format!("ReadProperty encode: {e}")))?;

    let response = confirmed_request(
        socket,
        target_addr,
        ConfirmedServiceChoice::ReadProperty,
        service_data,
        REQUEST_TIMEOUT,
    )?;
    parse_record_count_ack(&response)
}

// ---------------------------------------------------------------------------
// Tests (wire bytes assembled by hand from ASHRAE 135)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_range_request_by_position_forward() {
        let expected: Vec<u8> = vec![
            0x0C, 0x05, 0x00, 0x00, 0x01, // [0] objectId: trendLog(20)<<22 | 1 = 0x05000001
            0x19, 0x83, //                   [1] propertyIdentifier = 131 (log-buffer)
            0x3E, //                         opening [3] byPosition
            0x21, 0x01, //                   referenceIndex: application Unsigned 1
            0x31, 0x0A, //                   count: application Signed 10
            0x3F, //                         closing [3]
        ];
        assert_eq!(build_read_range_request(TrendLogKind::TrendLog, 1, 1, 10), expected);
    }

    #[test]
    fn read_range_request_by_position_backward() {
        let expected: Vec<u8> = vec![
            0x0C, 0x05, 0x00, 0x00, 0x01, // [0] objectId trendLog 1
            0x19, 0x83, //                   [1] log-buffer
            0x3E, //                         opening [3]
            0x21, 0xC8, //                   referenceIndex: Unsigned 200
            0x31, 0xCE, //                   count: Signed -50 (0xCE)
            0x3F, //                         closing [3]
        ];
        assert_eq!(build_read_range_request(TrendLogKind::TrendLog, 1, 200, -50), expected);
    }

    #[test]
    fn signed_count_uses_minimal_two_complement() {
        let mut buf = Vec::new();
        encode_app_signed(&mut buf, 200); // needs a leading 0x00 so it stays positive
        assert_eq!(buf, vec![0x32, 0x00, 0xC8]);
        let mut buf = Vec::new();
        encode_app_signed(&mut buf, -1000); // 0xFC18
        assert_eq!(buf, vec![0x32, 0xFC, 0x18]);
        let mut buf = Vec::new();
        encode_app_signed(&mut buf, -128);
        assert_eq!(buf, vec![0x31, 0x80]);
    }

    fn two_record_ack() -> Vec<u8> {
        vec![
            0x0C, 0x05, 0x00, 0x00, 0x01, // [0] objectId trendLog 1
            0x19, 0x83, //                   [1] log-buffer
            0x3A, 0x05, 0xC0, //             [3] resultFlags: 5 unused bits, first-item+last-item
            0x49, 0x02, //                   [4] itemCount = 2
            0x5E, //                         opening [5] itemData
            // record 1: 2024-06-15 (Saturday) 14:30:45.00, real 72.5, no status flags
            0x0E, //                         opening [0] timestamp
            0xA4, 0x7C, 0x06, 0x0F, 0x06, // Date: year 0x7C=124 -> 2024, month 6, day 15, wday 6
            0xB4, 0x0E, 0x1E, 0x2D, 0x00, // Time: 14:30:45.00
            0x0F, //                         closing [0]
            0x1E, //                         opening [1] logDatum
            0x2C, 0x42, 0x91, 0x00, 0x00, // [2] real-value 72.5 (0x42910000)
            0x1F, //                         closing [1]
            // record 2: 14:30:46, unsigned 42, status flags in-alarm
            0x0E, 0xA4, 0x7C, 0x06, 0x0F, 0x06, 0xB4, 0x0E, 0x1E, 0x2E, 0x00, 0x0F, //
            0x1E, //                         opening [1]
            0x49, 0x2A, //                   [4] unsigned-value 42
            0x1F, //                         closing [1]
            0x2A, 0x04, 0x80, //             [2] statusFlags: 4 unused bits, in-alarm set
            0x5F, //                         closing [5]
            0x69, 0x07, //                   [6] firstSequenceNumber = 7
        ]
    }

    #[test]
    fn read_range_ack_two_records() {
        let d = parse_read_range_ack(&two_record_ack(), TrendLogKind::TrendLog, 1).unwrap();
        assert_eq!(d.object_instance, 1);
        assert_eq!(d.item_count, 2);
        assert!(d.first_item && d.last_item && !d.more_items);
        assert_eq!(d.first_sequence_number, Some(7));
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.records[0].timestamp, "2024-06-15T14:30:45");
        assert_eq!(d.records[0].value, "72.5");
        assert_eq!(d.records[0].status_flags, None);
        assert_eq!(d.records[1].timestamp, "2024-06-15T14:30:46");
        assert_eq!(d.records[1].value, "42");
        assert_eq!(d.records[1].status_flags.as_deref(), Some("inAlarm"));
    }

    #[test]
    fn read_range_ack_more_items_flag_and_empty_data() {
        let ack = vec![
            0x0C, 0x05, 0x00, 0x00, 0x01, // objectId
            0x19, 0x83, //                   log-buffer
            0x3A, 0x05, 0x20, //             resultFlags: more-items only (bit 2)
            0x49, 0x00, //                   itemCount = 0
            0x5E, 0x5F, //                   empty itemData
        ];
        let d = parse_read_range_ack(&ack, TrendLogKind::TrendLog, 1).unwrap();
        assert!(!d.first_item && !d.last_item && d.more_items);
        assert!(d.records.is_empty());
        assert_eq!(d.first_sequence_number, None);
    }

    #[test]
    fn read_range_ack_truncated_is_an_error() {
        let mut ack = two_record_ack();
        ack.truncate(30);
        assert!(parse_read_range_ack(&ack, TrendLogKind::TrendLog, 1).is_err());
    }

    fn datum(bytes: &[u8]) -> Result<String, ProtocolError> {
        parse_log_datum(&mut Reader::new(bytes))
    }

    #[test]
    fn log_datum_choices() {
        assert_eq!(datum(&[0x19, 0x01]).unwrap(), "true"); //  [1] boolean-value, 1 content octet
        assert_eq!(datum(&[0x39, 0xFE]).unwrap(), "254"); //   [3] enum-value 0xFE (unsigned)
        assert_eq!(datum(&[0x59, 0xFE]).unwrap(), "-2"); //    [5] signed-value 0xFE = -2
        assert_eq!(datum(&[0x78]).unwrap(), "null"); //        [7] null-value, no content
        assert_eq!(
            datum(&[0x0A, 0x05, 0xA0]).unwrap(), // [0] log-status: 5 unused, bits 101
            "log-status: log-disabled, log-interrupted"
        );
        assert_eq!(datum(&[0x9C, 0x3F, 0x80, 0x00, 0x00]).unwrap(), "time-change: 1s"); // [9] real 1.0
        assert_eq!(datum(&[0xAE, 0x91, 0x00, 0xAF]).unwrap(), "[any-value]"); // [10] opening, app enum, closing
    }

    #[test]
    fn log_datum_failure() {
        // [8] opening, Error { class enumerated 2 (property), code enumerated 32 (unknown-property) }, closing
        let s = datum(&[0x8E, 0x91, 0x02, 0x91, 0x20, 0x8F]).unwrap();
        assert!(s.starts_with("failure: "), "{s}");
    }

    #[test]
    fn datetime_unspecified_fields() {
        // Date with unspecified year/month/day/weekday and Time 12:00:00.00
        let mut r = Reader::new(&[0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0x0C, 0x00, 0x00, 0x00]);
        assert_eq!(parse_datetime(&mut r).unwrap(), "****-**-**T12:00:00");
    }

    #[test]
    fn record_count_unsigned_lengths() {
        let ack = |value: &[u8]| {
            let mut v = vec![
                0x0C, 0x05, 0x00, 0x00, 0x01, // [0] objectId
                0x19, 0x8D, //                   [1] record-count (141)
                0x3E, //                         opening [3]
            ];
            v.extend_from_slice(value);
            v.push(0x3F); //                     closing [3]
            v
        };
        assert_eq!(parse_record_count_ack(&ack(&[0x21, 0x05])).unwrap(), 5); // 1 octet
        assert_eq!(parse_record_count_ack(&ack(&[0x22, 0x01, 0x00])).unwrap(), 256); // 2 octets
        assert_eq!(parse_record_count_ack(&ack(&[0x23, 0x01, 0x00, 0x00])).unwrap(), 65536); // 3 octets
        assert_eq!(
            parse_record_count_ack(&ack(&[0x24, 0x00, 0x01, 0x00, 0x00])).unwrap(), // 4 octets
            65536
        );
        // 0x71 is application CharacterString (tag 7), not Unsigned
        assert!(parse_record_count_ack(&ack(&[0x71, 0x00])).is_err());
    }

    fn trend_log_multiple_ack() -> Vec<u8> {
        vec![
            0x0C, 0x06, 0xC0, 0x00, 0x03, // [0] objectId: trendLogMultiple(27)<<22 | 3 = 0x06C00003
            0x19, 0x83, //                   [1] log-buffer
            0x3A, 0x05, 0xC0, //             [3] resultFlags: first-item + last-item
            0x49, 0x02, //                   [4] itemCount = 2
            0x5E, //                         opening [5] itemData
            // record 1: 2024-06-15 14:30:45, values: real 72.5, unsigned 41
            0x0E, 0xA4, 0x7C, 0x06, 0x0F, 0x06, 0xB4, 0x0E, 0x1E, 0x2D, 0x00, 0x0F, // [0] timestamp
            0x1E, //                         opening [1] log-data wrapper
            0x1E, //                         opening [1] log-data SEQUENCE OF
            0x1C, 0x42, 0x91, 0x00, 0x00, // [1] real-value 72.5
            0x39, 0x29, //                   [3] unsigned-value 41
            0x1F, //                         closing [1] SEQUENCE OF
            0x1F, //                         closing [1] wrapper
            // record 2: 14:30:46, values: real 73.0, failure(class 2, code 32)
            0x0E, 0xA4, 0x7C, 0x06, 0x0F, 0x06, 0xB4, 0x0E, 0x1E, 0x2E, 0x00, 0x0F, // [0] timestamp
            0x1E, 0x1E, //                   opening [1] wrapper, opening [1] SEQUENCE OF
            0x1C, 0x42, 0x92, 0x00, 0x00, // [1] real-value 73.0
            0x7E, //                         opening [7] failure
            0x91, 0x02, //                   error-class enumerated 2 (property)
            0x91, 0x20, //                   error-code enumerated 32 (unknown-property)
            0x7F, //                         closing [7]
            0x1F, 0x1F, //                   closing [1] SEQUENCE OF, closing [1] wrapper
            0x5F, //                         closing [5]
        ]
    }

    #[test]
    fn read_range_ack_trend_log_multiple() {
        let d = parse_read_range_ack(&trend_log_multiple_ack(), TrendLogKind::TrendLogMultiple, 3).unwrap();
        assert_eq!(d.item_count, 2);
        assert!(d.first_item && d.last_item && !d.more_items);
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.records[0].timestamp, "2024-06-15T14:30:45");
        assert_eq!(d.records[0].value, "72.5, 41");
        assert_eq!(d.records[0].status_flags, None);
        assert_eq!(d.records[1].timestamp, "2024-06-15T14:30:46");
        let v = &d.records[1].value;
        assert!(v.starts_with("73, failure(") && v.ends_with(')'), "{v}");
    }

    #[test]
    fn trend_log_multiple_record_with_single_trend_log_parser_is_an_error() {
        // Same bytes under single-value numbering: [1] real is not a valid logDatum there.
        assert!(parse_read_range_ack(&trend_log_multiple_ack(), TrendLogKind::TrendLog, 3).is_err());
    }

    #[test]
    fn log_multiple_record_status_and_time_change() {
        let ts: &[u8] = &[0x0E, 0xA4, 0x7C, 0x06, 0x0F, 0x06, 0xB4, 0x0E, 0x1E, 0x2D, 0x00, 0x0F];
        // [1] wrapper { [0] log-status BIT STRING: 5 unused bits, 010 = buffer-purged }
        let mut status = ts.to_vec();
        status.extend_from_slice(&[0x1E, 0x0A, 0x05, 0x40, 0x1F]);
        assert_eq!(
            parse_log_multiple_record(&mut Reader::new(&status)).unwrap().value,
            "log-status: buffer-purged"
        );
        // [1] wrapper { [2] time-change REAL 1.0 }
        let mut change = ts.to_vec();
        change.extend_from_slice(&[0x1E, 0x2C, 0x3F, 0x80, 0x00, 0x00, 0x1F]);
        assert_eq!(
            parse_log_multiple_record(&mut Reader::new(&change)).unwrap().value,
            "time-change: 1s"
        );
    }

    #[test]
    fn log_multiple_values() {
        // Items of the SEQUENCE OF: [0] boolean, [2] enum, [4] signed, [5] bitstring, [6] null, [8] any-value
        let items: &[u8] = &[
            0x09, 0x01, //             [0] boolean-value true
            0x29, 0x07, //             [2] enumerated-value 7
            0x49, 0xFE, //             [4] signed-value -2
            0x5A, 0x04, 0xA0, //       [5] bitstring-value: 4 unused, 1010
            0x68, //                   [6] null-value
            0x8E, 0x91, 0x00, 0x8F, // [8] any-value opening, app enum 0, closing
        ];
        let mut r = Reader::new(items);
        let mut out = Vec::new();
        while r.peek().unwrap().is_some() {
            out.push(parse_log_multiple_value(&mut r).unwrap());
        }
        assert_eq!(out, vec!["true", "7", "-2", "1010", "null", "[any-value]"]);
    }

    #[test]
    fn read_range_request_trend_log_multiple_object_id() {
        let expected: Vec<u8> = vec![
            0x0C, 0x06, 0xC0, 0x00, 0x03, // [0] objectId: 27<<22 | 3 = 0x06C00003
            0x19, 0x83, //                   [1] log-buffer
            0x3E, 0x21, 0x01, //             byPosition opening, referenceIndex Unsigned 1
            0x31, 0xF6, //                   count Signed -10 (0xF6)
            0x3F, //                         closing [3]
        ];
        assert_eq!(build_read_range_request(TrendLogKind::TrendLogMultiple, 3, 1, -10), expected);
    }
}
