//! COV (Change of Value) subscription management and notification parsing.
//!
//! This module provides functions for subscribing to and unsubscribing from
//! COV notifications on BACnet devices, as well as parsing incoming COV
//! notification messages.
//!
//! # Protocol Details
//!
//! BACnet COV subscriptions allow a client to receive automatic notifications
//! when a property value changes on a device. The subscription is established
//! using the `SubscribeCOV` confirmed service request (ASHRAE 135 clause 13.14)
//! and cancelled by sending a new `SubscribeCOV` request with the same
//! subscriber process ID and object but with **both** `issueConfirmedNotifications`
//! and `lifetime` omitted. A subscription request therefore always carries
//! `issueConfirmedNotifications`; `lifetime` is omitted for an indefinite
//! subscription.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use mt_protocol::ProtocolError;
use serde::{Deserialize, Serialize};

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::ObjectType;
use bacnet_rs::service::ConfirmedServiceChoice;

use crate::helpers::{object_type_from_str, object_type_to_string};
use crate::transport::{self, REQUEST_TIMEOUT};
use crate::types::BacnetValue;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A COV notification received from a BACnet device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CovNotification {
    /// Subscriber process identifier (assigned by us during subscription).
    pub subscriber_process_identifier: u32,
    /// Device instance that sent the notification.
    pub initiating_device_identifier: u32,
    /// Object type + instance that changed.
    pub monitored_object_type: String,
    /// Object instance that changed.
    pub monitored_object_instance: u32,
    /// Time remaining on the subscription (seconds).
    pub time_remaining: u32,
    /// Property values in the notification.
    pub values: Vec<CovPropertyValue>,
    /// Timestamp (millis since epoch).
    pub timestamp: u64,
}

/// A single property value within a COV notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CovPropertyValue {
    /// Human-readable property name (e.g. "presentValue").
    pub property_name: String,
    /// Parsed value.
    pub value: BacnetValue,
    /// Raw display string.
    pub raw_value: String,
}

/// Info about an active COV subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CovSubscriptionInfo {
    /// Subscriber process ID (assigned when creating the subscription).
    pub subscriber_process_id: u32,
    /// Object type string (e.g. "analogInput").
    pub object_type: String,
    /// Object instance number.
    pub object_instance: u32,
    /// Subscription lifetime in seconds (0 = indefinite).
    pub lifetime: u32,
    /// Whether confirmed notifications were requested.
    pub confirmed_notifications: bool,
    /// Whether the subscription is currently active.
    pub active: bool,
}

/// A COV notification as received from the wire.
#[derive(Debug, Clone)]
pub struct CovPacket {
    /// The decoded notification.
    pub notification: CovNotification,
    /// `Some(invoke_id)` when the notification arrived as a
    /// ConfirmedCOVNotification-Request: the receiver MUST answer it with a
    /// SimpleAck carrying this invoke ID (see [`simple_ack_for`]).
    /// `None` for UnconfirmedCOVNotification.
    pub confirmed_invoke_id: Option<u8>,
}

/// ConfirmedCOVNotification confirmed-service choice (clause 21, ConfirmedServiceChoice).
const SERVICE_CONFIRMED_COV_NOTIFICATION: u8 = 1;
/// UnconfirmedCOVNotification unconfirmed-service choice.
const SERVICE_UNCONFIRMED_COV_NOTIFICATION: u8 = 2;

/// Fraction of a subscription's lifetime after which it should be renewed.
const RENEWAL_FRACTION_PERCENT: u64 = 80;

/// How long to wait after sending a SubscribeCOV before renewing it, or `None`
/// for an indefinite subscription (`lifetime == 0`) that never expires.
pub fn renewal_delay(lifetime: u32) -> Option<Duration> {
    if lifetime == 0 {
        None
    } else {
        Some(Duration::from_millis(
            lifetime as u64 * 1000 * RENEWAL_FRACTION_PERCENT / 100,
        ))
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Push a context tag header for `number` with content length `len` (< 5).
fn push_context_tag(buf: &mut Vec<u8>, number: u8, len: usize) {
    debug_assert!(number <= 14 && len < 5);
    // Tag number in bits 7-4, class bit 3 = context, length in bits 2-0.
    buf.push((number << 4) | 0x08 | len as u8);
}

/// Context-tagged Unsigned using the minimum number of content octets.
fn push_context_unsigned(buf: &mut Vec<u8>, number: u8, value: u32) {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take(3).take_while(|&&b| b == 0).count();
    push_context_tag(buf, number, 4 - skip);
    buf.extend_from_slice(&bytes[skip..]);
}

/// Context-tagged BACnetObjectIdentifier (type in the top 10 bits, instance in 22).
fn push_context_object_id(
    buf: &mut Vec<u8>,
    number: u8,
    object_type: ObjectType,
    instance: u32,
) -> Result<(), ProtocolError> {
    let type_number: u32 = object_type.into();
    if type_number > 0x3FF || instance > 0x3F_FFFF {
        return Err(ProtocolError::InvalidConfig(format!(
            "Object identifier out of range: type {type_number}, instance {instance}"
        )));
    }
    push_context_tag(buf, number, 4);
    buf.extend_from_slice(&((type_number << 22) | instance).to_be_bytes());
    Ok(())
}

/// Context-tagged BOOLEAN: one content octet, 0x00 = FALSE, 0x01 = TRUE (clause 20.2.3).
fn push_context_bool(buf: &mut Vec<u8>, number: u8, value: bool) {
    push_context_tag(buf, number, 1);
    buf.push(value as u8);
}

/// SubscribeCOV-Request service data (clause 13.14.1):
///
/// ```text
/// subscriberProcessIdentifier  [0] Unsigned
/// monitoredObjectIdentifier    [1] BACnetObjectIdentifier
/// issueConfirmedNotifications  [2] BOOLEAN   OPTIONAL
/// lifetime                     [3] Unsigned  OPTIONAL
/// ```
///
/// With both optional fields absent the request cancels the subscription.
fn subscribe_service_data(
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> Result<Vec<u8>, ProtocolError> {
    let ot = object_type_from_str(object_type)
        .ok_or_else(|| ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}")))?;

    let mut data = Vec::new();
    push_context_unsigned(&mut data, 0, subscriber_id);
    push_context_object_id(&mut data, 1, ot, object_instance)?;
    if let Some(c) = confirmed {
        push_context_bool(&mut data, 2, c);
    }
    if let Some(lt) = lifetime {
        push_context_unsigned(&mut data, 3, lt);
    }
    Ok(data)
}

/// Wrap SubscribeCOV service data in a complete BVLC + NPDU + ConfirmedRequest frame.
fn subscribe_frame(invoke_id: u8, service_data: Vec<u8>) -> Vec<u8> {
    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::SubscribeCOV,
        service_data,
    }
    .encode();
    transport::unicast_frame(&apdu, true)
}

/// Build a full BACnet/IP SubscribeCOV frame (BVLC Original-Unicast-NPDU) that
/// uses `invoke_id`. The caller sends it from a socket so that the device
/// replies to, and later notifies, that socket. Re-sending it with the same
/// `subscriber_id` renews the subscription. `lifetime == 0` omits the lifetime
/// field (indefinite subscription).
pub fn build_subscribe_cov_frame(
    invoke_id: u8,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: bool,
    lifetime: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let data = subscribe_service_data(
        subscriber_id,
        object_type,
        object_instance,
        Some(confirmed),
        (lifetime > 0).then_some(lifetime),
    )?;
    Ok(subscribe_frame(invoke_id, data))
}

/// Build a full BACnet/IP SubscribeCOV cancellation frame (both
/// `issueConfirmedNotifications` and `lifetime` omitted).
pub fn build_cancel_cov_frame(
    invoke_id: u8,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let data = subscribe_service_data(subscriber_id, object_type, object_instance, None, None)?;
    Ok(subscribe_frame(invoke_id, data))
}

// ---------------------------------------------------------------------------
// Notification parsing
// ---------------------------------------------------------------------------

/// What a decoded tag header introduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagKind {
    /// Context opening tag (LVT = 6).
    Opening,
    /// Context closing tag (LVT = 7).
    Closing,
    /// Primitive tag followed by this many content octets.
    Value(usize),
}

#[derive(Clone, Copy, Debug)]
struct Tag {
    number: u8,
    context: bool,
    kind: TagKind,
}

/// Cursor over ASN.1-style BACnet tagged data (clause 20.2).
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Decode the tag at the cursor without consuming it. Returns the tag and
    /// the length of its header (initial octet, extended tag number, extended length).
    fn peek_tag(&self) -> Option<(Tag, usize)> {
        let first = *self.data.get(self.pos)?;
        let mut header = 1;
        let mut number = first >> 4;
        if number == 0x0F {
            number = *self.data.get(self.pos + header)?;
            header += 1;
        }
        let context = first & 0x08 != 0;
        let lvt = first & 0x07;

        let kind = if context && lvt == 6 {
            TagKind::Opening
        } else if context && lvt == 7 {
            TagKind::Closing
        } else if !context && number == 1 {
            // Application BOOLEAN: the value lives in the LVT bits, no content octets.
            TagKind::Value(0)
        } else {
            let len = match lvt {
                0..=4 => lvt as usize,
                5 => {
                    let b = *self.data.get(self.pos + header)?;
                    header += 1;
                    match b {
                        0..=253 => b as usize,
                        254 => {
                            let v = u16::from_be_bytes([
                                *self.data.get(self.pos + header)?,
                                *self.data.get(self.pos + header + 1)?,
                            ]);
                            header += 2;
                            v as usize
                        }
                        255 => {
                            let v = u32::from_be_bytes([
                                *self.data.get(self.pos + header)?,
                                *self.data.get(self.pos + header + 1)?,
                                *self.data.get(self.pos + header + 2)?,
                                *self.data.get(self.pos + header + 3)?,
                            ]);
                            header += 4;
                            v as usize
                        }
                    }
                }
                // LVT 6/7 on an application tag is not valid.
                _ => return None,
            };
            TagKind::Value(len)
        };
        Some((Tag { number, context, kind }, header))
    }

    /// True when the next tag is a primitive context tag with this number.
    fn peek_context_value(&self, number: u8) -> bool {
        matches!(
            self.peek_tag(),
            Some((Tag { number: n, context: true, kind: TagKind::Value(_) }, _)) if n == number
        )
    }

    /// True when the next tag is the closing tag with this number.
    fn peek_closing(&self, number: u8) -> bool {
        matches!(
            self.peek_tag(),
            Some((Tag { number: n, context: true, kind: TagKind::Closing }, _)) if n == number
        )
    }

    /// Consume a primitive context tag with `number`; returns its content octets.
    fn context_value(&mut self, number: u8) -> Option<&'a [u8]> {
        let (tag, header) = self.peek_tag()?;
        let TagKind::Value(len) = tag.kind else {
            return None;
        };
        if !tag.context || tag.number != number {
            return None;
        }
        let start = self.pos + header;
        let content = self.data.get(start..start.checked_add(len)?)?;
        self.pos = start + len;
        Some(content)
    }

    /// Context-tagged Unsigned (1-4 content octets).
    fn context_unsigned(&mut self, number: u8) -> Option<u32> {
        let content = self.context_value(number)?;
        if !(1..=4).contains(&content.len()) {
            return None;
        }
        Some(content.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32))
    }

    /// Context-tagged BACnetObjectIdentifier; returns the raw 32-bit value.
    fn context_object_id(&mut self, number: u8) -> Option<u32> {
        let content = self.context_value(number)?;
        let bytes: [u8; 4] = content.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    fn opening(&mut self, number: u8) -> Option<()> {
        let (tag, header) = self.peek_tag()?;
        if tag.context && tag.number == number && tag.kind == TagKind::Opening {
            self.pos += header;
            Some(())
        } else {
            None
        }
    }

    fn closing(&mut self, number: u8) -> Option<()> {
        let (tag, header) = self.peek_tag()?;
        if tag.context && tag.number == number && tag.kind == TagKind::Closing {
            self.pos += header;
            Some(())
        } else {
            None
        }
    }

    /// Skip everything up to and including the closing tag `number` that matches
    /// an already consumed opening tag, honouring nested constructed values.
    fn skip_until_closing(&mut self, number: u8) -> Option<()> {
        let mut depth = 0usize;
        loop {
            let (tag, header) = self.peek_tag()?;
            match tag.kind {
                TagKind::Opening => depth += 1,
                TagKind::Closing if depth == 0 => {
                    if tag.number != number {
                        return None;
                    }
                    self.pos += header;
                    return Some(());
                }
                TagKind::Closing => depth -= 1,
                TagKind::Value(len) => {
                    let end = self.pos.checked_add(header)?.checked_add(len)?;
                    if end > self.data.len() {
                        return None;
                    }
                    self.pos += header + len;
                    continue;
                }
            }
            self.pos += header;
        }
    }

    /// Consume one application-tagged value. `property_id` selects the
    /// rendering of BIT STRINGs (statusFlags get named flags).
    fn application_value(&mut self, property_id: u32) -> Option<(BacnetValue, String)> {
        let (tag, header) = self.peek_tag()?;
        let TagKind::Value(len) = tag.kind else {
            return None;
        };
        if tag.context {
            return None;
        }
        let start = self.pos + header;
        let content = self.data.get(start..start.checked_add(len)?)?;
        let first = self.data[self.pos];
        self.pos = start + len;

        Some(match tag.number {
            0 => (BacnetValue::Null, "Null".into()),
            // BOOLEAN: value is the LVT field of the initial octet.
            1 => {
                let b = first & 0x07 != 0;
                (BacnetValue::Boolean(b), b.to_string())
            }
            2 => {
                if !(1..=8).contains(&len) {
                    return None;
                }
                let v = content.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64);
                match u32::try_from(v) {
                    Ok(v) => (BacnetValue::Unsigned(v), v.to_string()),
                    Err(_) => (BacnetValue::Float64(v as f64), v.to_string()),
                }
            }
            3 => {
                if !(1..=8).contains(&len) {
                    return None;
                }
                // Two's complement, sign-extended from the top bit of the first octet.
                let seed = if content[0] & 0x80 != 0 { -1i64 } else { 0 };
                let v = content.iter().fold(seed, |acc, &b| (acc << 8) | b as i64);
                (BacnetValue::Float64(v as f64), v.to_string())
            }
            4 => {
                let bytes: [u8; 4] = content.try_into().ok()?;
                let f = f32::from_be_bytes(bytes);
                (BacnetValue::Real(f), format!("{f:.4}"))
            }
            5 => {
                let bytes: [u8; 8] = content.try_into().ok()?;
                let f = f64::from_be_bytes(bytes);
                (BacnetValue::Double(f), format!("{f:.6}"))
            }
            6 => {
                let hex: String = content.iter().map(|b| format!("{b:02X}")).collect();
                (BacnetValue::String(hex.clone()), hex)
            }
            7 => {
                let s = decode_character_string(content)?;
                (BacnetValue::String(s.clone()), s)
            }
            8 => {
                let s = format_bit_string(content, property_id)?;
                (BacnetValue::String(s.clone()), s)
            }
            9 => {
                if !(1..=4).contains(&len) {
                    return None;
                }
                let v = content.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32);
                (BacnetValue::Enum(v), v.to_string())
            }
            10 => {
                let d: [u8; 4] = content.try_into().ok()?;
                let s = format_date(d);
                (BacnetValue::String(s.clone()), s)
            }
            11 => {
                let t: [u8; 4] = content.try_into().ok()?;
                let s = format_time(t);
                (BacnetValue::String(s.clone()), s)
            }
            12 => {
                let bytes: [u8; 4] = content.try_into().ok()?;
                let raw = u32::from_be_bytes(bytes);
                let s = format!(
                    "{}:{}",
                    object_type_to_string(&ObjectType::from(raw >> 22)),
                    raw & 0x3F_FFFF
                );
                (BacnetValue::String(s.clone()), s)
            }
            n => (BacnetValue::Null, format!("unsupported_application_tag_{n}")),
        })
    }
}

/// CharacterString (clause 20.2.9): first octet is the character set.
fn decode_character_string(content: &[u8]) -> Option<String> {
    let (&charset, text) = content.split_first()?;
    Some(match charset {
        // ISO 10646 UCS-2 (big endian).
        4 => {
            if text.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = text
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        // ISO 8859-1.
        5 => text.iter().map(|&b| b as char).collect(),
        // 0 = UTF-8 (ANSI X3.4); other legacy sets are rendered best-effort.
        _ => String::from_utf8_lossy(text).into_owned(),
    })
}

/// BIT STRING: first content octet is the number of unused bits in the last octet.
fn format_bit_string(content: &[u8], property_id: u32) -> Option<String> {
    let (&unused, bytes) = content.split_first()?;
    if unused > 7 || (bytes.is_empty() && unused != 0) {
        return None;
    }
    let bit_count = bytes.len() * 8 - unused as usize;
    let bits: Vec<bool> = (0..bit_count)
        .map(|i| bytes[i / 8] & (0x80 >> (i % 8)) != 0)
        .collect();

    // BACnetStatusFlags (property 111): in-alarm, fault, overridden, out-of-service.
    if property_id == 111 && bit_count == 4 {
        const NAMES: [&str; 4] = ["in-alarm", "fault", "overridden", "out-of-service"];
        let set: Vec<&str> = NAMES
            .iter()
            .zip(&bits)
            .filter(|(_, on)| **on)
            .map(|(n, _)| *n)
            .collect();
        return Some(if set.is_empty() {
            "normal".to_string()
        } else {
            set.join(", ")
        });
    }
    Some(bits.iter().map(|&b| if b { '1' } else { '0' }).collect())
}

/// BACnet Date: year-1900, month, day, day-of-week; 0xFF = unspecified.
fn format_date(d: [u8; 4]) -> String {
    let part = |v: u8, width: usize| {
        if v == 0xFF {
            "*".to_string()
        } else {
            format!("{v:0width$}")
        }
    };
    let year = if d[0] == 0xFF {
        "*".to_string()
    } else {
        (1900 + d[0] as u32).to_string()
    };
    format!("{year}-{}-{}", part(d[1], 2), part(d[2], 2))
}

/// BACnet Time: hour, minute, second, hundredths; 0xFF = unspecified.
fn format_time(t: [u8; 4]) -> String {
    let part = |v: u8| {
        if v == 0xFF {
            "**".to_string()
        } else {
            format!("{v:02}")
        }
    };
    format!("{}:{}:{}.{}", part(t[0]), part(t[1]), part(t[2]), part(t[3]))
}

/// Parse a COV notification service-data payload (the bytes after the service
/// choice, identical for ConfirmedCOVNotification and UnconfirmedCOVNotification,
/// clause 13.7 / 13.8):
///
/// ```text
/// subscriberProcessIdentifier  [0] Unsigned,
/// initiatingDeviceIdentifier   [1] BACnetObjectIdentifier,
/// monitoredObjectIdentifier    [2] BACnetObjectIdentifier,
/// timeRemaining                [3] Unsigned,
/// listOfValues                 [4] SEQUENCE OF BACnetPropertyValue
/// ```
///
/// with
///
/// ```text
/// BACnetPropertyValue ::= SEQUENCE {
///     propertyIdentifier   [0] BACnetPropertyIdentifier,
///     propertyArrayIndex   [1] Unsigned OPTIONAL,
///     value                [2] ABSTRACT-SYNTAX.&Type,
///     priority             [3] Unsigned OPTIONAL
/// }
/// ```
///
/// Returns `None` when the payload is not a well-formed notification.
pub fn parse_cov_notification(data: &[u8]) -> Option<CovNotification> {
    let mut r = Reader::new(data);

    let subscriber_id = r.context_unsigned(0)?;
    let device_id = r.context_object_id(1)?;
    let monitored_id = r.context_object_id(2)?;
    let time_remaining = r.context_unsigned(3)?;

    r.opening(4)?;
    let mut values = Vec::new();
    while !r.peek_closing(4) {
        values.push(parse_property_value(&mut r)?);
    }
    r.closing(4)?;

    Some(CovNotification {
        subscriber_process_identifier: subscriber_id,
        initiating_device_identifier: device_id & 0x3F_FFFF,
        monitored_object_type: object_type_to_string(&ObjectType::from(monitored_id >> 22)),
        monitored_object_instance: monitored_id & 0x3F_FFFF,
        time_remaining,
        values,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    })
}

/// Parse one BACnetPropertyValue. Only the first application-tagged element of
/// `value` is kept (constructed values are reported as `Null`).
fn parse_property_value(r: &mut Reader<'_>) -> Option<CovPropertyValue> {
    let property_id = r.context_unsigned(0)?;
    if r.peek_context_value(1) {
        r.context_unsigned(1)?;
    }
    r.opening(2)?;
    let (value, raw_value) = match r.peek_tag()? {
        (Tag { kind: TagKind::Closing, .. }, _) => return None,
        (Tag { context: false, kind: TagKind::Value(_), .. }, _) => r.application_value(property_id)?,
        _ => (BacnetValue::Null, "constructed".to_string()),
    };
    r.skip_until_closing(2)?;
    if r.peek_context_value(3) {
        r.context_unsigned(3)?;
    }
    Some(CovPropertyValue {
        property_name: property_id_to_name(property_id),
        value,
        raw_value,
    })
}

/// Parse a raw BACnet/IP frame. Returns a [`CovPacket`] if it is an unsegmented
/// ConfirmedCOVNotification-Request (service choice 1) or an
/// UnconfirmedCOVNotification-Request (service choice 2) that decodes, `None`
/// otherwise. Segmented confirmed notifications are not supported.
pub fn parse_cov_packet(frame: &[u8]) -> Option<CovPacket> {
    let apdu = transport::apdu_of_frame(frame)?;

    match apdu[0] >> 4 {
        // Confirmed-Request: type/flags, max segments/APDU, invoke id, service
        // choice, service data. (A segmented request would carry sequence number
        // and window size before the service choice.)
        0 => {
            if apdu[0] & 0x08 != 0 || apdu.len() < 4 {
                return None;
            }
            if apdu[3] != SERVICE_CONFIRMED_COV_NOTIFICATION {
                return None;
            }
            let notification = parse_cov_notification(&apdu[4..])?;
            Some(CovPacket {
                notification,
                confirmed_invoke_id: Some(apdu[2]),
            })
        }
        // Unconfirmed-Request: type, service choice, service data.
        1 => {
            if apdu.len() < 2 || apdu[1] != SERVICE_UNCONFIRMED_COV_NOTIFICATION {
                return None;
            }
            let notification = parse_cov_notification(&apdu[2..])?;
            Some(CovPacket {
                notification,
                confirmed_invoke_id: None,
            })
        }
        _ => None,
    }
}

/// UDP-level originator of a received BVLC frame: the 6-octet B/IP address that
/// follows the header of a Forwarded-NPDU (Annex J.2.5), otherwise `src`. When a
/// BBMD relays a frame it is the UDP source, but replies belong to the originator.
pub fn frame_origin(frame: &[u8], src: SocketAddr) -> SocketAddr {
    match frame {
        [0x81, 0x04, _, _, a, b, c, d, port_hi, port_lo, ..] => SocketAddr::from((
            Ipv4Addr::new(*a, *b, *c, *d),
            u16::from_be_bytes([*port_hi, *port_lo]),
        )),
        _ => src,
    }
}

/// Build the SimpleAck answering a confirmed request received in `frame` from
/// UDP source `src`, together with the address to send it to. Returns `None`
/// when `frame` is not an application-layer BVLC frame.
///
/// The destination is [`frame_origin`]. When the request carried an NPDU source
/// (SNET/SADR, clause 6.2.2) the sender sits behind a router, so the ack names
/// that network and MAC address as its destination (DNET/DADR, hop count 255).
pub fn simple_ack_for(
    frame: &[u8],
    src: SocketAddr,
    invoke_id: u8,
    service_choice: u8,
) -> Option<(SocketAddr, Vec<u8>)> {
    let npdu_start = match frame {
        [0x81, 0x0A | 0x0B, ..] => 4,
        [0x81, 0x04, ..] => 10,
        _ => return None,
    };
    let (received, _) = Npdu::decode(frame.get(npdu_start..)?).ok()?;

    let mut npdu = Npdu::new();
    if let Some(source) = received.source {
        npdu.control.destination_present = true;
        npdu.destination = Some(source);
        npdu.hop_count = Some(255);
    }
    let mut body = npdu.encode();
    body.extend_from_slice(
        &Apdu::SimpleAck {
            invoke_id,
            service_choice,
        }
        .encode(),
    );

    let mut ack = Vec::with_capacity(body.len() + 4);
    ack.extend_from_slice(&[0x81, 0x0A]);
    ack.extend_from_slice(&((body.len() + 4) as u16).to_be_bytes());
    ack.extend_from_slice(&body);
    Some((frame_origin(frame, src), ack))
}

// ---------------------------------------------------------------------------
// Blocking helpers (for use from spawn_blocking)
// ---------------------------------------------------------------------------

/// Send a SubscribeCOV request from a fresh socket and wait for the device's
/// reply. Error / Reject / Abort replies are returned as errors.
///
/// Notifications go to the fresh socket's port, which is closed on return; use
/// the long-lived listener socket (see [`build_subscribe_cov_frame`]) to actually
/// receive them.
pub fn subscribe_cov_blocking(
    target_addr: SocketAddr,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: bool,
    lifetime: u32,
) -> Result<(), ProtocolError> {
    let data = subscribe_service_data(
        subscriber_id,
        object_type,
        object_instance,
        Some(confirmed),
        (lifetime > 0).then_some(lifetime),
    )?;
    subscribe_request(target_addr, data)
}

/// Send a COV cancellation request from a fresh socket and wait for the reply.
pub fn unsubscribe_cov_blocking(
    target_addr: SocketAddr,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
) -> Result<(), ProtocolError> {
    let data = subscribe_service_data(subscriber_id, object_type, object_instance, None, None)?;
    subscribe_request(target_addr, data)
}

fn subscribe_request(target_addr: SocketAddr, service_data: Vec<u8>) -> Result<(), ProtocolError> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;
    transport::confirmed_request(
        &socket,
        target_addr,
        ConfirmedServiceChoice::SubscribeCOV,
        service_data,
        REQUEST_TIMEOUT,
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Map a BACnet property identifier number to its camelCase name
/// (BACnetPropertyIdentifier, clause 21).
fn property_id_to_name(prop_id: u32) -> String {
    match prop_id {
        0 => "ackedTransitions".into(),
        17 => "notificationClass".into(),
        22 => "covIncrement".into(),
        28 => "description".into(),
        35 => "eventEnable".into(),
        36 => "eventState".into(),
        45 => "highLimit".into(),
        59 => "lowLimit".into(),
        65 => "maxPresValue".into(),
        69 => "minPresValue".into(),
        77 => "objectName".into(),
        81 => "outOfService".into(),
        85 => "presentValue".into(),
        87 => "priorityArray".into(),
        103 => "reliability".into(),
        104 => "relinquishDefault".into(),
        111 => "statusFlags".into(),
        117 => "units".into(),
        _ => format!("property_{prop_id}"),
    }
}

// ---------------------------------------------------------------------------
// Tests: byte strings are assembled by hand from ASHRAE 135, not produced by
// the encoder under test.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap an APDU in BVLC Original-Unicast-NPDU (81 0A len) + NPDU (01 00).
    fn frame(apdu: &[u8]) -> Vec<u8> {
        let len = (4 + 2 + apdu.len()) as u16;
        let mut f = vec![0x81, 0x0A, (len >> 8) as u8, len as u8, 0x01, 0x00];
        f.extend_from_slice(apdu);
        f
    }

    /// COV-Notification service data for analogInput,1 from device,2 with a
    /// presentValue (Real 20.0) and statusFlags (normal). `process_id` is the
    /// already-tagged [0] subscriberProcessIdentifier.
    fn notification_data(process_id: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(process_id);
        d.extend_from_slice(&[
            0x1C, 0x02, 0x00, 0x00, 0x02, // [1] initiatingDevice: device (type 8 << 22 = 0x02000000), instance 2
            0x2C, 0x00, 0x00, 0x00, 0x01, // [2] monitoredObject: analogInput (type 0), instance 1
            0x39, 0x00, //                   [3] timeRemaining: Unsigned, 1 octet, 0
            0x4E, //                         [4] listOfValues opening
            0x09, 0x55, //                     [0] propertyIdentifier: 85 (presentValue)
            0x2E, //                           [2] value opening
            0x44, 0x41, 0xA0, 0x00, 0x00, //     application Real 20.0 (0x41A00000)
            0x2F, //                           [2] value closing
            0x09, 0x6F, //                     [0] propertyIdentifier: 111 (statusFlags)
            0x2E, //                           [2] value opening
            0x82, 0x04, 0x00, //                 application BitString, 2 octets: 4 unused bits, flags 0000
            0x2F, //                           [2] value closing
            0x4F, //                         [4] listOfValues closing
        ]);
        d
    }

    #[test]
    fn unconfirmed_notification_real_and_status_flags() {
        let raw = [
            0x81, 0x0A, 0x00, 0x28, // BVLC: Original-Unicast-NPDU, 40 octets
            0x01, 0x00, //             NPDU: version 1, no flags
            0x10, 0x02, //             APDU: Unconfirmed-Request, UnconfirmedCOVNotification (2)
            0x09, 0x01, //             [0] subscriberProcessIdentifier = 1
            0x1C, 0x02, 0x00, 0x00, 0x02, //  [1] device,2
            0x2C, 0x00, 0x00, 0x00, 0x01, //  [2] analogInput,1
            0x39, 0x00, //             [3] timeRemaining = 0
            0x4E, //                   [4] opening
            0x09, 0x55, 0x2E, 0x44, 0x41, 0xA0, 0x00, 0x00, 0x2F, // presentValue = Real 20.0
            0x09, 0x6F, 0x2E, 0x82, 0x04, 0x00, 0x2F, //            statusFlags = 0000, 4 unused
            0x4F, //                   [4] closing
        ];
        assert_eq!(raw.len(), 0x28);
        let pkt = parse_cov_packet(&raw).expect("parses");
        assert_eq!(pkt.confirmed_invoke_id, None);
        let n = pkt.notification;
        assert_eq!(n.subscriber_process_identifier, 1);
        assert_eq!(n.initiating_device_identifier, 2);
        assert_eq!(n.monitored_object_type, "analogInput");
        assert_eq!(n.monitored_object_instance, 1);
        assert_eq!(n.time_remaining, 0);
        assert_eq!(n.values.len(), 2);
        assert_eq!(n.values[0].property_name, "presentValue");
        assert!(matches!(n.values[0].value, BacnetValue::Real(v) if v == 20.0));
        assert_eq!(n.values[1].property_name, "statusFlags");
        assert_eq!(n.values[1].raw_value, "normal");
    }

    #[test]
    fn two_byte_process_id_and_set_status_flags() {
        // [0] Unsigned, 2 octets: 0x0102 = 258.
        let mut data = notification_data(&[0x0A, 0x01, 0x02]);
        // Flip the statusFlags octet to 0xA0 = 1010 -> in-alarm + overridden.
        let pos = data.iter().position(|&b| b == 0x82).unwrap();
        data[pos + 2] = 0xA0;
        let mut apdu = vec![0x10, 0x02];
        apdu.extend_from_slice(&data);
        let n = parse_cov_packet(&frame(&apdu)).unwrap().notification;
        assert_eq!(n.subscriber_process_identifier, 258);
        assert_eq!(n.values[1].raw_value, "in-alarm, overridden");
    }

    #[test]
    fn confirmed_notification_returns_invoke_id() {
        let data = notification_data(&[0x09, 0x07]);
        // Confirmed-Request: type 0 no flags, max segs unspecified / max APDU 1476 (5),
        // invoke id 0x2A, service choice 1 (confirmedCOVNotification).
        let mut apdu = vec![0x00, 0x05, 0x2A, 0x01];
        apdu.extend_from_slice(&data);
        let pkt = parse_cov_packet(&frame(&apdu)).expect("parses");
        assert_eq!(pkt.confirmed_invoke_id, Some(0x2A));
        assert_eq!(pkt.notification.subscriber_process_identifier, 7);
        assert_eq!(pkt.notification.values.len(), 2);
    }

    #[test]
    fn confirmed_request_with_other_service_is_not_a_notification() {
        let data = notification_data(&[0x09, 0x07]);
        // Service choice 0 is acknowledgeAlarm, not a COV notification.
        let mut apdu = vec![0x00, 0x05, 0x2A, 0x00];
        apdu.extend_from_slice(&data);
        assert!(parse_cov_packet(&frame(&apdu)).is_none());
    }

    #[test]
    fn unconfirmed_request_with_other_service_is_ignored() {
        let data = notification_data(&[0x09, 0x07]);
        // Unconfirmed service 0 = i-Am.
        let mut apdu = vec![0x10, 0x00];
        apdu.extend_from_slice(&data);
        assert!(parse_cov_packet(&frame(&apdu)).is_none());
    }

    #[test]
    fn segmented_confirmed_notification_is_not_parsed() {
        let data = notification_data(&[0x09, 0x07]);
        // 0x08 = segmented flag; sequence number and window size precede the service choice.
        let mut apdu = vec![0x08, 0x05, 0x2A, 0x00, 0x01, 0x01];
        apdu.extend_from_slice(&data);
        assert!(parse_cov_packet(&frame(&apdu)).is_none());
    }

    #[test]
    fn truncated_notification_is_rejected() {
        let data = notification_data(&[0x09, 0x01]);
        let mut apdu = vec![0x10, 0x02];
        apdu.extend_from_slice(&data[..data.len() - 1]); // missing closing [4]
        assert!(parse_cov_packet(&frame(&apdu)).is_none());
    }

    #[test]
    fn extended_length_string_array_index_and_priority() {
        let mut data = vec![
            0x09, 0x01, //                   [0] process id 1
            0x1C, 0x02, 0x00, 0x00, 0x02, // [1] device,2
            0x2C, 0x00, 0x00, 0x00, 0x05, // [2] analogInput,5
            0x39, 0x3C, //                   [3] timeRemaining 60
            0x4E, //                         [4] opening
            0x09, 0x4D, //                     [0] property 77 (objectName)
            0x2E, //                           [2] opening
            0x75, 0x06, 0x00, b'A', b'B', b'C', b'D', b'E', // CharacterString: LVT 5 -> length octet 6 = charset + 5 chars; charset 0
            0x2F, //                           [2] closing
            0x09, 0x57, //                     [0] property 87 (priorityArray)
            0x19, 0x03, //                     [1] propertyArrayIndex 3
            0x2E, //                           [2] opening
            0x44, 0x00, 0x00, 0x00, 0x00, //     Real 0.0
            0x2F, //                           [2] closing
            0x39, 0x08, //                     [3] priority 8
        ];
        data.push(0x4F); // [4] closing
        let mut apdu = vec![0x10, 0x02];
        apdu.extend_from_slice(&data);
        let n = parse_cov_packet(&frame(&apdu)).unwrap().notification;
        assert_eq!(n.monitored_object_instance, 5);
        assert_eq!(n.time_remaining, 60);
        assert_eq!(n.values.len(), 2);
        assert_eq!(n.values[0].property_name, "objectName");
        assert_eq!(n.values[0].raw_value, "ABCDE");
        assert_eq!(n.values[1].property_name, "priorityArray");
        assert!(matches!(n.values[1].value, BacnetValue::Real(v) if v == 0.0));
    }

    #[test]
    fn enumerated_boolean_unsigned_and_signed_values() {
        let data = vec![
            0x09, 0x01, //                   [0] process id 1
            0x1C, 0x02, 0x00, 0x00, 0x02, // [1] device,2
            0x2C, 0x00, 0xC0, 0x00, 0x03, // [2] type 3 (binaryInput) = 0x00C00000, instance 3
            0x39, 0x00, //                   [3] 0
            0x4E, //                         [4] opening
            0x09, 0x55, 0x2E, 0x91, 0x01, 0x2F, //           presentValue = Enumerated 1 (tag 9, length 1)
            0x09, 0x51, 0x2E, 0x11, 0x2F, //                 outOfService = BOOLEAN TRUE (tag 1, LVT 1 = value)
            0x09, 0x75, 0x2E, 0x22, 0x01, 0x2C, 0x2F, //      property 117 = Unsigned 300 (tag 2, length 2)
            0x09, 0x1C, 0x2E, 0x31, 0xFE, 0x2F, //            property 28 = Signed -2 (tag 3, length 1, 0xFE)
            0x4F, //                         [4] closing
        ];
        let mut apdu = vec![0x10, 0x02];
        apdu.extend_from_slice(&data);
        let n = parse_cov_packet(&frame(&apdu)).unwrap().notification;
        assert_eq!(n.monitored_object_type, "binaryInput");
        assert!(matches!(n.values[0].value, BacnetValue::Enum(1)));
        assert!(matches!(n.values[1].value, BacnetValue::Boolean(true)));
        assert!(matches!(n.values[2].value, BacnetValue::Unsigned(300)));
        assert!(matches!(n.values[3].value, BacnetValue::Float64(v) if v == -2.0));
    }

    #[test]
    fn subscribe_frame_matches_clause_13_14() {
        let f = build_subscribe_cov_frame(0x11, 5, "analogInput", 1, false, 3600).unwrap();
        let expected = [
            0x81, 0x0A, 0x00, 0x16, // BVLC Original-Unicast-NPDU, 22 octets
            0x01, 0x04, //             NPDU: version 1, expecting reply
            0x02, 0x05, 0x11, 0x05, // APDU: Confirmed-Request, segmented-response-accepted; max APDU 1476; invoke 0x11; SubscribeCOV (5)
            0x09, 0x05, //             [0] subscriberProcessIdentifier = 5
            0x1C, 0x00, 0x00, 0x00, 0x01, // [1] monitoredObject analogInput,1
            0x29, 0x00, //             [2] issueConfirmedNotifications = FALSE (always present on subscribe)
            0x3A, 0x0E, 0x10, //       [3] lifetime = 3600 (0x0E10), 2 octets
        ];
        assert_eq!(f, expected);
    }

    #[test]
    fn subscribe_confirmed_indefinite_omits_lifetime() {
        let f = build_subscribe_cov_frame(0x01, 300, "binaryValue", 7, true, 0).unwrap();
        let expected = [
            0x81, 0x0A, 0x00, 0x14, // 20 octets
            0x01, 0x04, //
            0x02, 0x05, 0x01, 0x05, //
            0x0A, 0x01, 0x2C, //       [0] process id 300 = 0x012C, 2 octets
            0x1C, 0x01, 0x40, 0x00, 0x07, // [1] binaryValue (5 << 22 = 0x01400000), instance 7
            0x29, 0x01, //             [2] TRUE
        ];
        assert_eq!(f, expected);
    }

    #[test]
    fn cancel_frame_omits_confirmed_and_lifetime() {
        let f = build_cancel_cov_frame(0x22, 5, "analogInput", 1).unwrap();
        let expected = [
            0x81, 0x0A, 0x00, 0x11, // 17 octets
            0x01, 0x04, //
            0x02, 0x05, 0x22, 0x05, //
            0x09, 0x05, //             [0] process id 5
            0x1C, 0x00, 0x00, 0x00, 0x01, // [1] analogInput,1; no [2], no [3]
        ];
        assert_eq!(f, expected);
    }

    #[test]
    fn unknown_object_type_and_oversized_instance_are_errors() {
        assert!(build_subscribe_cov_frame(0, 1, "bogus", 1, false, 0).is_err());
        assert!(build_cancel_cov_frame(0, 1, "analogInput", 0x40_0000).is_err());
    }

    #[test]
    fn renewal_is_eighty_percent_of_lifetime_and_never_for_indefinite() {
        assert_eq!(renewal_delay(3600), Some(Duration::from_secs(2880)));
        assert_eq!(renewal_delay(10), Some(Duration::from_secs(8)));
        assert_eq!(renewal_delay(0), None);
    }

    /// ConfirmedCOVNotification, service choice 1, invoke id 0x2A, process id 7.
    fn confirmed_notification_apdu() -> Vec<u8> {
        let mut apdu = vec![0x00, 0x05, 0x2A, 0x01];
        apdu.extend_from_slice(&notification_data(&[0x09, 0x07]));
        apdu
    }

    // Clause 5.4.1 / 21: SimpleAck = PDU type 2 in the high nibble (0x20), invoke id,
    // service choice (1 = confirmedCOVNotification). NPDU 01 00 = version 1, no flags.
    #[test]
    fn confirmed_notification_is_acked_to_the_udp_source() {
        let src: SocketAddr = "192.168.1.10:47808".parse().unwrap();
        let (to, ack) = simple_ack_for(&frame(&confirmed_notification_apdu()), src, 0x2A, 1).unwrap();
        assert_eq!(to, src);
        assert_eq!(ack, [0x81, 0x0A, 0x00, 0x09, 0x01, 0x00, 0x20, 0x2A, 0x01]);
    }

    // Annex J.2.5: Forwarded-NPDU = 81 04 <len> <4-octet IP> <2-octet port> <NPDU>.
    // The ack goes to the originating B/IP address (192.168.0.5:47808 = C0 A8 00 05 BA C0),
    // not to the BBMD that relayed the frame.
    #[test]
    fn forwarded_notification_is_acked_to_the_bvlc_origin() {
        let apdu = confirmed_notification_apdu();
        let len = (4 + 6 + 2 + apdu.len()) as u16;
        let mut f = vec![0x81, 0x04, (len >> 8) as u8, len as u8, 0xC0, 0xA8, 0x00, 0x05, 0xBA, 0xC0, 0x01, 0x00];
        f.extend_from_slice(&apdu);

        let pkt = parse_cov_packet(&f).expect("forwarded frame parses");
        assert_eq!(pkt.confirmed_invoke_id, Some(0x2A));

        let bbmd: SocketAddr = "10.0.0.1:47808".parse().unwrap();
        let origin: SocketAddr = "192.168.0.5:47808".parse().unwrap();
        assert_eq!(frame_origin(&f, bbmd), origin);
        let (to, ack) = simple_ack_for(&f, bbmd, 0x2A, 1).unwrap();
        assert_eq!(to, origin);
        assert_eq!(ack, [0x81, 0x0A, 0x00, 0x09, 0x01, 0x00, 0x20, 0x2A, 0x01]);
    }

    // Clause 6.2.2: a request routed to us carries SNET/SADR (control bit 3). The reply
    // names that network/MAC as DNET/DADR (control bit 5) followed by hop count 255:
    // source 01 08 | 00 05 01 0A  ->  ack 01 20 | 00 05 01 0A | FF.
    #[test]
    fn routed_notification_is_acked_with_dnet_dadr() {
        let apdu = confirmed_notification_apdu();
        let len = (4 + 6 + apdu.len()) as u16;
        let mut f = vec![0x81, 0x0A, (len >> 8) as u8, len as u8, 0x01, 0x08, 0x00, 0x05, 0x01, 0x0A];
        f.extend_from_slice(&apdu);

        let pkt = parse_cov_packet(&f).expect("routed frame parses");
        assert_eq!(pkt.confirmed_invoke_id, Some(0x2A));

        let router: SocketAddr = "192.168.1.1:47808".parse().unwrap();
        let (to, ack) = simple_ack_for(&f, router, 0x2A, 1).unwrap();
        assert_eq!(to, router);
        assert_eq!(
            ack,
            [0x81, 0x0A, 0x00, 0x0E, 0x01, 0x20, 0x00, 0x05, 0x01, 0x0A, 0xFF, 0x20, 0x2A, 0x01]
        );
    }

    #[test]
    fn no_ack_for_non_application_frames() {
        let src: SocketAddr = "192.168.1.10:47808".parse().unwrap();
        // BVLC-Result (function 0x00) is not an NPDU carrier.
        assert!(simple_ack_for(&[0x81, 0x00, 0x00, 0x06, 0x00, 0x00], src, 1, 1).is_none());
        assert!(simple_ack_for(&[0x80, 0x0A, 0x00, 0x04], src, 1, 1).is_none());
        assert!(simple_ack_for(&[], src, 1, 1).is_none());
    }
}
