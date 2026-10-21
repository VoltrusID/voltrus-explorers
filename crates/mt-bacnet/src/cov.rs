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
//! using the `SubscribeCOV` confirmed service request and cancelled by sending
//! a new `SubscribeCOV` request with the same subscriber process ID but with
//! the `issue_confirmed_notifications` field omitted and `lifetime` set to 0.

use std::net::SocketAddr;
use std::time::Duration;

use mt_protocol::ProtocolError;
use serde::{Deserialize, Serialize};

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::ObjectIdentifier;
use bacnet_rs::service::ConfirmedServiceChoice;

use crate::helpers::object_type_from_str;
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

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

/// Encode a context-tagged unsigned integer into the buffer.
///
/// Tag byte: `(context_number << 4) | 0x20` for primitive (non-constructed) context tag.
fn encode_context_unsigned(buf: &mut Vec<u8>, tag: u8, value: u32) {
    // Delegate to bacnet_rs (correct context-tag bit 0x08 + length encoding).
    if let Ok(enc) = bacnet_rs::encoding::encode_context_unsigned(value, tag) {
        buf.extend_from_slice(&enc);
    }
}

/// Encode a context-tagged BACnet ObjectIdentifier (4 bytes).
fn encode_context_object_id(buf: &mut Vec<u8>, tag: u8, obj_id: &ObjectIdentifier) {
    if let Ok(enc) = bacnet_rs::encoding::encode_context_object_id(obj_id.clone(), tag) {
        buf.extend_from_slice(&enc);
    }
}

/// Encode a context-tagged boolean.
fn encode_context_bool(buf: &mut Vec<u8>, tag: u8, value: bool) {
    // Context tag, length 1, value 0xFF (true) / 0x00 (false).
    buf.push((tag << 4) | 0x08 | 0x01);
    buf.push(if value { 0xFF } else { 0x00 });
}

// ---------------------------------------------------------------------------
// APDU construction
// ---------------------------------------------------------------------------

/// Build a complete BACnet message (BVLC + NPDU + APDU) for a SubscribeCOV
/// confirmed request.
///
/// The caller provides the object type as a string (e.g. `"analogInput"`) which
/// is resolved to a BACnet [`ObjectType`].
pub fn build_subscribe_cov_apdu(
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: bool,
    lifetime: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let ot = object_type_from_str(object_type).ok_or_else(|| {
        ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}"))
    })?;

    let obj_id = ObjectIdentifier::new(ot, object_instance);

    // Encode SubscribeCOV service data per ASN.1:
    //   [0] Unsigned  subscriberProcessIdentifier
    //   [1] OID       monitoredObjectIdentifier
    //   [2] BOOLEAN   issueConfirmedNotifications  (optional)
    //   [3] Unsigned  lifetime                      (optional)
    let mut service_data = Vec::new();
    encode_context_unsigned(&mut service_data, 0, subscriber_id);
    encode_context_object_id(&mut service_data, 1, &obj_id);

    if confirmed {
        encode_context_bool(&mut service_data, 2, true);
    }

    if lifetime > 0 {
        encode_context_unsigned(&mut service_data, 3, lifetime);
    }

    build_confirmed_apdu(ConfirmedServiceChoice::SubscribeCOV, service_data)
}

/// Build a complete BACnet message for cancelling a COV subscription.
///
/// Cancellation is performed by sending SubscribeCOV with the same subscriber ID
/// and object, but **omitting** `issueConfirmedNotifications` and setting
/// `lifetime` to 0 (or omitting it).
pub fn build_unsubscribe_cov_apdu(
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let ot = object_type_from_str(object_type).ok_or_else(|| {
        ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}"))
    })?;

    let obj_id = ObjectIdentifier::new(ot, object_instance);

    // Cancellation: include subscriber ID and object, omit confirmed flag,
    // omit lifetime (or set to 0).
    let mut service_data = Vec::new();
    encode_context_unsigned(&mut service_data, 0, subscriber_id);
    encode_context_object_id(&mut service_data, 1, &obj_id);

    build_confirmed_apdu(ConfirmedServiceChoice::SubscribeCOV, service_data)
}

/// Wrap service data into a complete BVLC + NPDU + ConfirmedRequest APDU frame.
fn build_confirmed_apdu(
    service_choice: ConfirmedServiceChoice,
    service_data: Vec<u8>,
) -> Result<Vec<u8>, ProtocolError> {
    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id: 0, // Will be overwritten by the caller if needed
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

    // BVLC Original-Unicast-NPDU (type 0x01, function 0x0A)
    let mut bvlc = vec![0x81, 0x0A, 0x00, 0x00];
    bvlc.extend_from_slice(&message);
    let len = bvlc.len() as u16;
    bvlc[2] = (len >> 8) as u8;
    bvlc[3] = (len & 0xFF) as u8;

    Ok(bvlc)
}

// ---------------------------------------------------------------------------
// Notification parsing
// ---------------------------------------------------------------------------

/// Parse an incoming UnconfirmedCOVNotification service data payload into a
/// [`CovNotification`].
///
/// The service data follows the ASN.1 structure:
///
/// ```text
/// ConfirmedCOVNotification-Request ::= SEQUENCE {
///     subscriberProcessIdentifier  [0] Unsigned,
///     initiatingDeviceIdentifier   [1] BACnetObjectIdentifier,
///     monitoredObjectIdentifier    [2] BACnetObjectIdentifier,
///     timeRemaining                [3] Unsigned,
///     listOfValues                 [4] SEQUENCE OF BACnetPropertyValue
/// }
/// ```
///
/// Each `BACnetPropertyValue` is:
///
/// ```text
/// BACnetPropertyValue ::= SEQUENCE {
///     propertyIdentifier   [0] BACnetPropertyIdentifier,
///     propertyArrayIndex   [1] Unsigned OPTIONAL,
///     value                [2] ABSTRACT-SYNTAX.&Type,
///     priority             [3] Unsigned OPTIONAL
/// }
/// ```
pub fn parse_cov_notification(data: &[u8]) -> Option<CovNotification> {
    let mut pos = 0;

    // [0] subscriberProcessIdentifier — context tag 0
    let (subscriber_id, consumed) = parse_context_unsigned(data, pos, 0)?;
    pos += consumed;

    // [1] initiatingDeviceIdentifier — context tag 1, object identifier (4 bytes)
    let (device_id, consumed) = parse_context_object_id(data, pos, 1)?;
    pos += consumed;

    // [2] monitoredObjectIdentifier — context tag 2, object identifier (4 bytes)
    let (monitored_id, consumed) = parse_context_object_id(data, pos, 2)?;
    pos += consumed;

    // [3] timeRemaining — context tag 3
    let (time_remaining, consumed) = parse_context_unsigned(data, pos, 3)?;
    pos += consumed;

    // [4] listOfValues — context tag 4 (opening tag), then property values
    // The opening tag for context 4 is 0x4C (constructed = 0x20 | class bit 0x08 = 0x28?).
    // Actually: context tag 4 constructed = (4 << 4) | 0x20 | 0x08 = 0x58.
    // Wait — BACnet context constructed tag uses the "extended" bit 0x20 for primitive,
    // 0x10 for boolean, and 0x08 for... Let me reconsider.
    //
    // BACnet tagging:
    //   Bits 7-4: tag number (for context tags 0-14)
    //   Bit 3:    class (0 = application, 1 = context)
    //   Bit 2:    constructed (0 = primitive, 1 = constructed)
    //   Bits 1-0: length/type
    //
    // For context tag 4, constructed, no length in tag:
    //   (4 << 4) | (1 << 3) | (1 << 2) | 0x00 = 0x58
    //
    // For context tag 4, primitive (extended length encoding):
    //   (4 << 4) | (1 << 3) | 0x20 = 0x58 also... Let me check.
    //
    // Actually in bacnet-rs encoding: context tag byte for primitive:
    //   (tag_number << 4) | 0x20  where 0x20 = (1<<5) = context bit + extended-length flag
    //
    // For constructed context tag (like opening a SEQUENCE):
    //   (tag_number << 4) | 0x28  where 0x28 = context bit + constructed bit
    //
    // So context tag 4 opening = 0x4C (no, let me recalculate):
    //   4 << 4 = 0x40, plus 0x20 (context) = 0x60... That's tag number 6.
    //
    // Let me look at the actual bacnet-rs encoding convention used in the crate:
    // For context tag 0 unsigned 1-byte: 0x09 = (0 << 4) | 0x09 = 0x09
    //   Wait, that's 0x09. That's tag=0, then 0x09 has bits: 0000 1001
    //   Bits 7-4 = 0000 = tag 0. Bit 3 = 1 (context). Bit 2 = 0 (primitive). Bits 1-0 = 01 = length=1.
    //   So length 1 is encoded IN the tag byte itself! That's the short form.
    //
    // For context tag 1, length 4: 0x1C = 0001 1100
    //   Bits 7-4 = 0001 = tag 1. Bit 3 = 1 (context). Bit 2 = 1 (constructed? or extended?).
    //   Hmm, 0x1C: 0001_1100. Bit 3 = 1 (context), bit 2 = 1, bits 1-0 = 00.
    //   Actually for extended form: bits 1-0 = 00 means "length follows in next byte".
    //   But the bacnet-rs encode puts length in a separate byte.
    //
    // Let me just look at what 0x1C means: 0x1C = (1 << 4) | 0x0C = 0x10 | 0x0C.
    //   0x10 = context bit. 0x0C = 1100 binary. bit 2 = 1, bits 1-0 = 00.
    //   In BACnet, when bits 1-0 = 00, the length follows as the next byte.
    //
    // OK so the format is:
    //   - Bits 7-4: tag number
    //   - Bit 3: 1 = context class
    //   - Bit 2: 1 = extended form (length follows)
    //   - Bits 1-0: when bit 2 = 0, length-1 encoded (0..3 means 1..4 bytes)
    //               when bit 2 = 1, 00 = length in next byte(s)
    //
    // For tag 0, value 1 byte: (0<<4) | (1<<3) | 0 | 0x01 = 0x09. Correct!
    // For tag 1, length 4 follows: (1<<4) | (1<<3) | (1<<2) | 0 = 0x1C. Correct!
    // For tag 2, length 4 follows: (2<<4) | (1<<3) | (1<<2) | 0 = 0x2C. Correct!
    // For tag 3, value 1 byte: (3<<4) | (1<<3) | 0 | 0x01 = 0x39. Correct!
    //
    // So for context tag 4, CONSTRUCTED (opening a sequence/list):
    //   (4<<4) | (1<<3) | (1<<2) | 0x01 = 0x4D  ... no.
    //   Actually constructed bit is bit 2? No, in BACnet:
    //     Bit 2 (0x04) is the "constructed" flag.
    //     Bit 3 (0x08) is the "context" class flag.
    //
    // Wait, I need to re-examine. In BACnet tagging:
    //   Bit 7: 1 = context-specific tag class (0 = application tag class)
    //   ...no that can't be right because tag numbers go up to 14.
    //
    // The standard BACnet tag format (Clause 20.2):
    //   Bits 7-4: Tag number (for context or application tag numbers 0-14)
    //   Bit 3:    Packet type (0 = application, 1 = context)
    //   Bit 2:    Constructed (0 = primitive, 1 = constructed)
    //   Bits 1-0: Length/Type
    //     If Bit 2 = 0: 00 = 0 bytes, 01 = 1 byte, 10 = 2 bytes, 11 = 3 bytes
    //                    4 bytes: use extended form
    //     If Bit 2 = 1: 00 = extended, 01 = 4 bytes, 10 = ... varies
    //
    // Actually let me just look at the actual bytes for context tag 4 constructed:
    //   Bits: 0100 1 1 00 = 0x4C
    //   4 = tag 4, bit 3 = 1 (context), bit 2 = 1 (constructed), bits 1-0 = 00 (extended length)
    //
    // So context tag 4, constructed, extended form = 0x4C, followed by length byte(s).
    // Context tag 4, closing = 0x4F (tag 4, context, constructed, end-of-contents bits 11)

    // But in practice, the listOfValues might just be inline without explicit
    // opening/closing tags (since the notification ends after the values).
    // Let me scan for property values instead.

    let values = parse_cov_property_values(data, pos)?;

    // Extract object type from the monitored object identifier
    let monitored_type_u32 = (monitored_id >> 22) & 0x3FF;
    let monitored_inst = monitored_id & 0x3FFFFF;
    let monitored_type_str = object_type_id_to_string(monitored_type_u32);

    Some(CovNotification {
        subscriber_process_identifier: subscriber_id,
        initiating_device_identifier: device_id & 0x3FFFFF,
        monitored_object_type: monitored_type_str,
        monitored_object_instance: monitored_inst,
        time_remaining,
        values,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    })
}

/// Parse property values from the remaining bytes of a COV notification.
///
/// Each property value starts with a context tag 0 (property identifier) within
/// the list. We scan forward looking for context-tagged fields.
fn parse_cov_property_values(data: &[u8], start: usize) -> Option<Vec<CovPropertyValue>> {
    let mut pos = start;
    let mut values = Vec::new();

    while pos + 1 < data.len() {
        let tag_byte = data[pos];
        let context_tag = (tag_byte >> 4) & 0x0F;

        // Check if this is a context-class tag
        let is_context = (tag_byte & 0x08) != 0;
        if !is_context {
            // Skip application-tagged bytes (property value data)
            pos += 1;
            if (tag_byte & 0x07) == 0x07 {
                // Extended length
                if pos < data.len() {
                    pos += 1 + data[pos] as usize;
                }
            } else {
                let len = match tag_byte & 0x07 {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    4 => 4,
                    5 => {
                        pos += 1;
                        if pos < data.len() {
                            data[pos] as usize
                        } else {
                            break;
                        }
                    }
                    _ => 0,
                };
                pos += len;
            }
            continue;
        }

        // Inside the property value sequence, context tags 0-3 map to:
        //   [0] propertyIdentifier
        //   [1] propertyArrayIndex (optional)
        //   [2] value (constructed/open)
        //   [3] priority (optional)
        match context_tag {
            0 => {
                // Property identifier — start of a new property value entry
                let (prop_id, consumed) = parse_context_unsigned(data, pos, 0)?;
                pos += consumed;

                // Skip optional propertyArrayIndex (context tag 1)
                if pos < data.len() {
                    let next_tag = data[pos];
                    let next_ctx = (next_tag >> 4) & 0x0F;
                    if (next_tag & 0x08) != 0 && next_ctx == 1 {
                        let (_, consumed) = parse_context_unsigned(data, pos, 1)?;
                        pos += consumed;
                    }
                }

                // Skip context tag 2 opening if present, then read the value
                if pos < data.len() {
                    let next_tag = data[pos];
                    let next_ctx = (next_tag >> 4) & 0x0F;
                    if (next_tag & 0x08) != 0 && next_ctx == 2 {
                        // This is the "value" opening tag — skip it
                        pos += 1;
                        // Skip length byte if extended
                        if pos < data.len() && (data[pos - 1] & 0x03) == 0x00 {
                            // Extended form: next byte is length or more
                            // Actually for constructed tags with bits 1-0 = 00, check
                            // if there's a length byte
                            if (data[pos - 1] & 0x04) != 0 {
                                // This was constructed+extended, length follows
                                if pos < data.len() {
                                    pos += 1 + data[pos] as usize;
                                }
                                continue;
                            }
                        }
                    }
                }

                // Now parse the application-tagged value
                let (parsed_value, raw_str, consumed) = parse_application_value(data, pos)?;
                pos += consumed;

                let prop_name = property_id_to_name(prop_id);
                values.push(CovPropertyValue {
                    property_name: prop_name,
                    value: parsed_value,
                    raw_value: raw_str,
                });

                // Skip optional priority (context tag 3)
                if pos < data.len() {
                    let next_tag = data[pos];
                    let next_ctx = (next_tag >> 4) & 0x0F;
                    if (next_tag & 0x08) != 0 && next_ctx == 3 {
                        let (_, consumed) = parse_context_unsigned(data, pos, 3)?;
                        pos += consumed;
                    }
                }
            }
            _ => {
                // Skip unknown context tags
                pos += 1;
            }
        }
    }

    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Parse a context-tagged unsigned integer starting at `pos` in `data`.
/// Returns `(value, bytes_consumed)`.
fn parse_context_unsigned(data: &[u8], pos: usize, expected_tag: u8) -> Option<(u32, usize)> {
    if pos >= data.len() {
        return None;
    }

    let tag_byte = data[pos];
    let tag = (tag_byte >> 4) & 0x0F;
    let is_context = (tag_byte & 0x08) != 0;

    if tag != expected_tag || !is_context {
        return None;
    }

    let is_extended = (tag_byte & 0x04) != 0;
    let len_bits = tag_byte & 0x03;

    let (length, header_size) = if is_extended {
        // Extended form: bits 1-0 = 00, length in next byte(s)
        if len_bits != 0 || pos + 1 >= data.len() {
            return None;
        }
        let len = data[pos + 1] as usize;
        (len, 2)
    } else {
        // Short form: length encoded in bits 1-0 as length-1
        (len_bits as usize + 1, 1)
    };

    let value_start = pos + header_size;
    if value_start + length > data.len() || length > 4 {
        return None;
    }

    let value = match length {
        0 => 0u32,
        1 => data[value_start] as u32,
        2 => ((data[value_start] as u32) << 8) | (data[value_start + 1] as u32),
        3 => {
            ((data[value_start] as u32) << 16)
                | ((data[value_start + 1] as u32) << 8)
                | (data[value_start + 2] as u32)
        }
        4 => u32::from_be_bytes([
            data[value_start],
            data[value_start + 1],
            data[value_start + 2],
            data[value_start + 3],
        ]),
        _ => return None,
    };

    Some((value, header_size + length))
}

/// Parse a context-tagged object identifier (4 bytes) starting at `pos`.
/// Returns `(raw_u32, bytes_consumed)`.
fn parse_context_object_id(data: &[u8], pos: usize, expected_tag: u8) -> Option<(u32, usize)> {
    if pos >= data.len() {
        return None;
    }

    let tag_byte = data[pos];
    let tag = (tag_byte >> 4) & 0x0F;
    let is_context = (tag_byte & 0x08) != 0;

    if tag != expected_tag || !is_context {
        return None;
    }

    let is_extended = (tag_byte & 0x04) != 0;
    let len_bits = tag_byte & 0x03;

    let (length, header_size) = if is_extended {
        if len_bits != 0 || pos + 1 >= data.len() {
            return None;
        }
        let len = data[pos + 1] as usize;
        (len, 2)
    } else {
        (len_bits as usize + 1, 1)
    };

    let value_start = pos + header_size;
    if length != 4 || value_start + 4 > data.len() {
        return None;
    }

    let raw = u32::from_be_bytes([
        data[value_start],
        data[value_start + 1],
        data[value_start + 2],
        data[value_start + 3],
    ]);

    Some((raw, header_size + 4))
}

/// Parse an application-tagged value at `pos`.
/// Returns `(BacnetValue, raw_string, bytes_consumed)`.
fn parse_application_value(data: &[u8], pos: usize) -> Option<(BacnetValue, String, usize)> {
    if pos >= data.len() {
        return None;
    }

    let tag = data[pos];
    let is_context = (tag & 0x08) != 0;

    // If this is a context tag, don't try to parse it as an application value
    if is_context {
        return None;
    }

    // Application tag number is bits 7-4
    let app_tag = (tag >> 4) & 0x0F;
    let len_bits = tag & 0x07;

    let (length, header_size) = if len_bits == 6 {
        // Extended length: next byte is length
        if pos + 1 >= data.len() {
            return None;
        }
        let len = data[pos + 1] as usize;
        (len, 2)
    } else if len_bits == 7 {
        // Segment or special — not a normal value
        return None;
    } else {
        (len_bits as usize, 1)
    };

    let value_start = pos + header_size;
    if value_start + length > data.len() {
        return None;
    }

    let value_bytes = &data[value_start..value_start + length];

    let (parsed, raw) = match app_tag {
        // Null
        0 => (BacnetValue::Null, "Null".into()),
        // Boolean (length = 0 for false, 1 for true)
        1 => {
            let b = length > 0 && value_bytes[0] != 0;
            (BacnetValue::Boolean(b), if b { "true" } else { "false" }.into())
        }
        // Unsigned integer
        2 => {
            let v = bytes_to_u32(value_bytes);
            (BacnetValue::Unsigned(v), v.to_string())
        }
        // Signed integer
        3 => {
            let v = bytes_to_i32(value_bytes);
            (BacnetValue::Unsigned(v as u32), v.to_string())
        }
        // Real (4-byte IEEE 754)
        4 if length == 4 => {
            let bytes: [u8; 4] = value_bytes.try_into().ok()?;
            let f = f32::from_be_bytes(bytes);
            (BacnetValue::Real(f), format!("{f:.4}"))
        }
        // Double (8-byte IEEE 754)
        5 if length == 8 => {
            let bytes: [u8; 8] = value_bytes.try_into().ok()?;
            let f = f64::from_be_bytes(bytes);
            (BacnetValue::Double(f), format!("{f:.6}"))
        }
        // Enumerated
        9 => {
            let v = bytes_to_u32(value_bytes);
            (BacnetValue::Enum(v), v.to_string())
        }
        // Character String (tag 7)
        7 if length >= 1 => {
            // First byte is encoding (0 = ANSI/ASCII)
            if value_bytes[0] == 0 && length > 1 {
                let s = String::from_utf8_lossy(&value_bytes[1..]).to_string();
                (BacnetValue::String(s.clone()), s)
            } else {
                let s = String::from_utf8_lossy(value_bytes).to_string();
                (BacnetValue::String(s.clone()), s)
            }
        }
        // Octet String (tag 10 = 0x0A)
        10 => {
            let hex: String = value_bytes.iter().map(|b| format!("{b:02X}")).collect();
            (BacnetValue::String(hex.clone()), hex)
        }
        _ => (BacnetValue::Null, format!("unknown_tag_{app_tag}")),
    };

    Some((parsed, raw, header_size + length))
}

// ---------------------------------------------------------------------------
// Blocking send helpers (for use from spawn_blocking)
// ---------------------------------------------------------------------------

/// Send a SubscribeCOV request synchronously and wait for acknowledgment.
///
/// Uses a fresh UDP socket with a 6-second receive timeout.
pub fn subscribe_cov_blocking(
    target_addr: SocketAddr,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: bool,
    lifetime: u32,
) -> Result<(), ProtocolError> {
    use std::sync::atomic::{AtomicU8, Ordering};

    static COV_INVOKE_ID: AtomicU8 = AtomicU8::new(10);

    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(6)))
        .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

    // Build SubscribeCOV APDU
    let ot = object_type_from_str(object_type).ok_or_else(|| {
        ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}"))
    })?;
    let obj_id = ObjectIdentifier::new(ot, object_instance);

    // Encode service data
    let mut service_data = Vec::new();
    encode_context_unsigned(&mut service_data, 0, subscriber_id);
    encode_context_object_id(&mut service_data, 1, &obj_id);
    if confirmed {
        encode_context_bool(&mut service_data, 2, true);
    }
    if lifetime > 0 {
        encode_context_unsigned(&mut service_data, 3, lifetime);
    }

    let invoke_id = COV_INVOKE_ID.fetch_add(1, Ordering::Relaxed);

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
        .map_err(|e| ProtocolError::WriteFailed(format!("Send SubscribeCOV: {e}")))?;

    // Wait for SimpleAck or ComplexAck
    wait_for_ack(&socket, target_addr, invoke_id)
}

/// Send a COV unsubscription (cancellation) request synchronously.
pub fn unsubscribe_cov_blocking(
    target_addr: SocketAddr,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
) -> Result<(), ProtocolError> {
    use std::sync::atomic::{AtomicU8, Ordering};

    static UNSUB_INVOKE_ID: AtomicU8 = AtomicU8::new(50);

    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(6)))
        .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

    let ot = object_type_from_str(object_type).ok_or_else(|| {
        ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}"))
    })?;
    let obj_id = ObjectIdentifier::new(ot, object_instance);

    let mut service_data = Vec::new();
    encode_context_unsigned(&mut service_data, 0, subscriber_id);
    encode_context_object_id(&mut service_data, 1, &obj_id);
    // Cancellation: omit confirmed flag and lifetime

    let invoke_id = UNSUB_INVOKE_ID.fetch_add(1, Ordering::Relaxed);

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
    };

    let apdu_data = apdu.encode();
    let mut npdu = Npdu::new();
    npdu.control.expecting_reply = true;
    let npdu_data = npdu.encode();

    let mut message = npdu_data;
    message.extend_from_slice(&apdu_data);

    let mut bvlc = vec![0x81, 0x0A, 0x00, 0x00];
    bvlc.extend_from_slice(&message);
    let len = bvlc.len() as u16;
    bvlc[2] = (len >> 8) as u8;
    bvlc[3] = (len & 0xFF) as u8;

    socket
        .send_to(&bvlc, target_addr)
        .map_err(|e| ProtocolError::WriteFailed(format!("Send UnsubscribeCOV: {e}")))?;

    wait_for_ack(&socket, target_addr, invoke_id)
}

/// Build a full BACnet/IP SubscribeCOV frame (BVLC Original-Unicast-NPDU).
/// The caller sends it from a socket so that the device replies to that socket.
pub fn build_subscribe_cov_frame(
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: bool,
    lifetime: u32,
) -> Result<Vec<u8>, ProtocolError> {
    use std::sync::atomic::{AtomicU8, Ordering};
    static COV_INVOKE_ID: AtomicU8 = AtomicU8::new(10);
    build_cov_frame(
        COV_INVOKE_ID.fetch_add(1, Ordering::Relaxed),
        subscriber_id,
        object_type,
        object_instance,
        if confirmed { Some(true) } else { None },
        if lifetime > 0 { Some(lifetime) } else { None },
    )
}

/// Build a full BACnet/IP SubscribeCOV cancellation frame.
pub fn build_cancel_cov_frame(
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
) -> Result<Vec<u8>, ProtocolError> {
    use std::sync::atomic::{AtomicU8, Ordering};
    static UNSUB_INVOKE_ID: AtomicU8 = AtomicU8::new(50);
    build_cov_frame(
        UNSUB_INVOKE_ID.fetch_add(1, Ordering::Relaxed),
        subscriber_id,
        object_type,
        object_instance,
        None,
        None,
    )
}

/// Encode a SubscribeCOV (or cancellation when confirmed/lifetime are None) as a
/// complete BVLC Original-Unicast-NPDU.
fn build_cov_frame(
    invoke_id: u8,
    subscriber_id: u32,
    object_type: &str,
    object_instance: u32,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> Result<Vec<u8>, ProtocolError> {
    let ot = object_type_from_str(object_type)
        .ok_or_else(|| ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}")))?;
    let obj_id = ObjectIdentifier::new(ot, object_instance);

    let mut service_data = Vec::new();
    encode_context_unsigned(&mut service_data, 0, subscriber_id);
    encode_context_object_id(&mut service_data, 1, &obj_id);
    if let Some(c) = confirmed {
        encode_context_bool(&mut service_data, 2, c);
    }
    if let Some(lt) = lifetime {
        encode_context_unsigned(&mut service_data, 3, lt);
    }

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
    };

    let mut npdu = Npdu::new();
    npdu.control.expecting_reply = true;
    let mut message = npdu.encode();
    message.extend_from_slice(&apdu.encode());

    // BVLC Original-Unicast-NPDU
    let mut bvlc = vec![0x81, 0x0A, 0x00, 0x00];
    bvlc.extend_from_slice(&message);
    let len = bvlc.len() as u16;
    bvlc[2] = (len >> 8) as u8;
    bvlc[3] = (len & 0xFF) as u8;
    Ok(bvlc)
}

/// Wait for a SimpleAck or ComplexAck matching the given `invoke_id`.
fn wait_for_ack(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    invoke_id: u8,
) -> Result<(), ProtocolError> {
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
                    if let Ok(Apdu::SimpleAck { invoke_id: resp_id, .. }) =
                        Apdu::decode(&buf[apdu_start..len])
                    {
                        if resp_id == invoke_id {
                            return Ok(());
                        }
                        continue;
                    }
                    if let Ok(Apdu::ComplexAck { invoke_id: resp_id, .. }) =
                        Apdu::decode(&buf[apdu_start..len])
                    {
                        if resp_id == invoke_id {
                            return Ok(());
                        }
                        continue;
                    }
                }
            }
            Ok(_) => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(ProtocolError::ReadFailed(format!("Recv: {e}"))),
        }
    }

    Err(ProtocolError::Timeout(format!(
        "SubscribeCOV request timeout for {target_addr}"
    )))
}

/// Parse a raw UDP BACnet/IP packet. Returns `Some(CovNotification)` if the packet
/// is a ConfirmedCOVNotification or UnconfirmedCOVNotification, `None` otherwise.
pub fn parse_cov_packet(packet: &[u8]) -> Option<CovNotification> {
    // Minimum: BVLC (4) + NPDU (2) + APDU (2)
    if packet.len() < 8 { return None; }
    // BVLC header: type 0x81
    if packet[0] != 0x81 { return None; }
    let bvlc_len = ((packet[2] as usize) << 8) | packet[3] as usize;
    if packet.len() < bvlc_len { return None; }

    let npdu_start = 4;
    let npdu_ok = Npdu::decode(&packet[npdu_start..bvlc_len]).ok();
    let npdu_len = npdu_ok.as_ref().map(|(_, l)| *l).unwrap_or(2);
    let apdu_start = npdu_start + npdu_len;

    if apdu_start >= bvlc_len { return None; }
    let apdu_data = &packet[apdu_start..bvlc_len];
    if apdu_data.len() < 2 { return None; }

    // Determine APDU type from the top nibble and extract service data.
    //   PDU type 0 (0x0X): Confirmed-Request  -> [pdu, max_apdu, invoke_id, service_choice, data]
    //   PDU type 1 (0x1X): Unconfirmed-Request -> [pdu, service_choice, data]
    let pdu_type = apdu_data[0] >> 4;
    let service_data: &[u8] = if pdu_type == 0 {
        // Confirmed request carrying a ConfirmedCOVNotification (service 0).
        if apdu_data.len() < 4 { return None; }
        let service_choice = apdu_data[3];
        if service_choice != 0 { return None; } // ConfirmedCOVNotification = 0
        &apdu_data[4..]
    } else if pdu_type == 1 {
        // Unconfirmed request carrying an UnconfirmedCOVNotification (service 2).
        let service_choice = apdu_data[1];
        if service_choice != 2 { return None; } // UnconfirmedCOVNotification = 2
        &apdu_data[2..]
    } else {
        return None;
    };

    parse_cov_notification(service_data)
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

/// Map a BACnet object type number to a human-readable string.
fn object_type_id_to_string(type_id: u32) -> String {
    match type_id {
        0 => "analogInput".into(),
        1 => "analogOutput".into(),
        2 => "analogValue".into(),
        3 => "binaryInput".into(),
        4 => "binaryOutput".into(),
        5 => "binaryValue".into(),
        13 => "multiStateInput".into(),
        14 => "multiStateOutput".into(),
        19 => "multiStateValue".into(),
        8 => "device".into(),
        12 => "trendLog".into(),
        20 => "trendLogMultiple".into(),
        17 => "schedule".into(),
        16 => "calendar".into(),
        11 => "notificationClass".into(),
        _ => format!("object_{type_id}"),
    }
}

/// Map a BACnet property identifier number to a human-readable name.
fn property_id_to_name(prop_id: u32) -> String {
    match prop_id {
        85 => "presentValue".into(),
        77 => "objectName".into(),
        28 => "description".into(),
        111 => "statusFlags".into(),
        117 => "units".into(),
        103 => "reliability".into(),
        81 => "outOfService".into(),
        36 => "eventState".into(),
        44 => "notificationClass".into(),
        39 => "highLimit".into(),
        40 => "lowLimit".into(),
        34 => "covIncrement".into(),
        87 => "relinquishDefault".into(),
        75 => "minPresValue".into(),
        76 => "maxPresValue".into(),
        166 => "priorityArray".into(),
        _ => format!("property_{prop_id}"),
    }
}
