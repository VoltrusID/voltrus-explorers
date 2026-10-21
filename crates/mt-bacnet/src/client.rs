use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_protocol::{ProtocolClient, ProtocolError};

use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier};
use bacnet_rs::property::{encode_property_value, PropertyValue};
use bacnet_rs::service::{
    ConfirmedServiceChoice, PropertyReference, PropertyResultValue, ReadAccessSpecification,
    ReadPropertyMultipleRequest, ReadPropertyMultipleResponse, ReadPropertyRequest,
    WritePropertyRequest,
};

use crate::helpers::{object_type_from_str, property_id_from_str};
use crate::transport::{confirmed_request, error_class_name, error_code_name};
use crate::types::{BacnetConfig, BacnetObject, BacnetValue};

/// Convert the first value of a ReadAccessResult property into a [`BacnetValue`].
/// Numeric types become `Float64` (the gateway's normalised form); character
/// strings stay strings; every other type is returned as its display string so
/// no property is silently turned into NaN.
fn property_value_to_bacnet(values: &[PropertyValue]) -> BacnetValue {
    match values.first() {
        None | Some(PropertyValue::Null) => BacnetValue::Null,
        Some(PropertyValue::Real(v)) => BacnetValue::Float64(*v as f64),
        Some(PropertyValue::Double(v)) => BacnetValue::Float64(*v),
        Some(PropertyValue::Unsigned(v)) => BacnetValue::Float64(*v as f64),
        Some(PropertyValue::Signed(v)) => BacnetValue::Float64(*v as f64),
        Some(PropertyValue::Boolean(v)) => BacnetValue::Float64(if *v { 1.0 } else { 0.0 }),
        Some(PropertyValue::Enumerated(v)) => BacnetValue::Float64(*v as f64),
        Some(PropertyValue::CharacterString(s)) => BacnetValue::String(s.clone()),
        Some(other) => BacnetValue::String(other.as_display_string()),
    }
}

/// Encode a [`BacnetValue`] as the application-tagged value of a WriteProperty
/// request. The caller chooses the variant matching the property's data type
/// (ASHRAE 135 clause 12): Real for analog objects, Enumerated for binary
/// objects, Unsigned for multi-state objects, Null to relinquish a priority slot.
/// `Float64` is the gateway's untyped number and is written as Real.
fn encode_write_value(value: &BacnetValue) -> Result<Vec<u8>, ProtocolError> {
    let pv = match value {
        BacnetValue::Real(v) => PropertyValue::Real(*v),
        BacnetValue::Double(v) => PropertyValue::Double(*v),
        BacnetValue::Unsigned(v) => PropertyValue::Unsigned(*v as u64),
        BacnetValue::Boolean(v) => PropertyValue::Boolean(*v),
        BacnetValue::Enum(v) => PropertyValue::Enumerated(*v),
        BacnetValue::Float64(v) => PropertyValue::Real(*v as f32),
        BacnetValue::String(s) => PropertyValue::CharacterString(s.clone()),
        BacnetValue::Null => PropertyValue::Null,
    };
    let mut out = Vec::new();
    encode_property_value(&pv, &mut out)
        .map_err(|e| ProtocolError::WriteFailed(format!("Value encode: {e}")))?;
    Ok(out)
}

/// Standalone BACnet/IP client implementing the [`ProtocolClient`] trait.
///
/// Communication is performed over UDP using `bacnet-rs` encoding and raw
/// socket I/O wrapped in `spawn_blocking` so the async trait methods remain
/// non-blocking on the Tokio runtime. Cloning is cheap (shared socket), which
/// lets callers release their own locks before awaiting network I/O.
#[derive(Clone)]
pub struct BacnetClient {
    addr: SocketAddr,
    socket: Arc<Mutex<std::net::UdpSocket>>,
}

impl BacnetClient {
    /// Write `value` to `tag`, optionally at a command priority (1-16).
    /// Without a priority the device applies its default (16 for commandable
    /// properties).
    pub async fn write_with_priority(
        &self,
        tag: &BacnetObject,
        value: BacnetValue,
        priority: Option<u8>,
    ) -> Result<(), ProtocolError> {
        if let Some(p) = priority {
            if !(1..=16).contains(&p) {
                return Err(ProtocolError::InvalidConfig(format!(
                    "BACnet write priority must be 1-16, got {p}"
                )));
            }
        }
        let ot = object_type_from_str(&tag.object_type).ok_or_else(|| {
            ProtocolError::InvalidConfig(format!("Unknown object type: {}", tag.object_type))
        })?;
        let pid: u32 = property_id_from_str(&tag.property).into();
        let obj_id = ObjectIdentifier::new(ot, tag.object_instance);
        let encoded = encode_write_value(&value)?;

        let mut write_req = WritePropertyRequest::new(obj_id, pid, encoded);
        write_req.priority = priority;
        let mut service_data = Vec::new();
        write_req
            .encode(&mut service_data)
            .map_err(|e| ProtocolError::WriteFailed(format!("Encode: {e}")))?;

        let addr = self.addr;
        let socket = Arc::clone(&self.socket);
        tokio::task::spawn_blocking(move || -> Result<(), ProtocolError> {
            let sock = socket
                .lock()
                .map_err(|e| ProtocolError::Other(format!("Socket lock poisoned: {e}")))?;
            sock.set_read_timeout(Some(Duration::from_secs(6)))
                .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;
            // SimpleAck -> Ok; Error / Reject / Abort -> descriptive Err.
            confirmed_request(
                &sock,
                addr,
                ConfirmedServiceChoice::WriteProperty,
                service_data,
                Duration::from_secs(6),
            )
            .map(|_| ())
            .map_err(|e| match e {
                ProtocolError::Other(msg) => ProtocolError::WriteFailed(msg),
                other => other,
            })
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
    }
}

impl ProtocolClient for BacnetClient {
    type Config = BacnetConfig;
    type Tag = BacnetObject;
    type Value = BacnetValue;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let addr = format!("{}:{}", config.ip, config.port);
        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address '{addr}': {e}")))?;

        let socket = tokio::task::spawn_blocking(move || {
            std::net::UdpSocket::bind("0.0.0.0:0")
                .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))??;

        let socket = Arc::new(Mutex::new(socket));

        // Probe: ReadProperty on the device object itself. UDP is connectionless,
        // so a bare bind succeeds for any address -- this confirms the device is
        // actually reachable and speaking BACnet before reporting "connected".
        let probe_socket = Arc::clone(&socket);
        let probe_addr = socket_addr;
        let device_id = config.device_id;
        tokio::task::spawn_blocking(move || -> Result<(), ProtocolError> {
            let sock = probe_socket
                .lock()
                .map_err(|e| ProtocolError::Other(format!("Socket lock poisoned: {e}")))?;
            sock.set_read_timeout(Some(Duration::from_millis(500)))
                .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

            let obj_id = ObjectIdentifier::new(ObjectType::Device, device_id);
            let req = ReadPropertyRequest::new(obj_id, PropertyIdentifier::ObjectIdentifier);
            let mut service_data = Vec::new();
            req.encode(&mut service_data)
                .map_err(|e| ProtocolError::ConnectionFailed(format!("Probe encode: {e}")))?;

            confirmed_request(
                &sock,
                probe_addr,
                ConfirmedServiceChoice::ReadProperty,
                service_data,
                Duration::from_secs(3),
            )?;
            Ok(())
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
        .map_err(|e| {
            ProtocolError::ConnectionFailed(format!(
                "BACnet probe of device {device_id} at {socket_addr} failed: {e}"
            ))
        })?;

        Ok(Self {
            addr: socket_addr,
            socket,
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve every tag up front; a tag that cannot be resolved is an error,
        // not a silently missing value.
        let mut resolved: Vec<(ObjectIdentifier, u32)> = Vec::with_capacity(tags.len());
        for t in tags {
            let ot = object_type_from_str(&t.object_type).ok_or_else(|| {
                ProtocolError::InvalidConfig(format!(
                    "Tag '{}': unknown object type '{}'",
                    t.name, t.object_type
                ))
            })?;
            let pid: u32 = property_id_from_str(&t.property).into();
            resolved.push((ObjectIdentifier::new(ot, t.object_instance), pid));
        }

        let read_specs = tags
            .iter()
            .zip(&resolved)
            .map(|(t, (obj_id, _))| {
                ReadAccessSpecification::new(
                    *obj_id,
                    vec![PropertyReference::new(property_id_from_str(&t.property))],
                )
            })
            .collect();
        let rpm_request = ReadPropertyMultipleRequest::new(read_specs);
        let addr = self.addr;
        let socket = Arc::clone(&self.socket);

        let response = tokio::task::spawn_blocking(move || -> Result<ReadPropertyMultipleResponse, ProtocolError> {
            let sock = socket
                .lock()
                .map_err(|e| ProtocolError::Other(format!("Socket lock poisoned: {e}")))?;
            sock.set_read_timeout(Some(Duration::from_secs(6)))
                .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

            let mut service_data = Vec::new();
            rpm_request
                .encode(&mut service_data)
                .map_err(|e| ProtocolError::ReadFailed(format!("RPM encode: {e}")))?;

            let response_data = confirmed_request(
                &sock,
                addr,
                ConfirmedServiceChoice::ReadPropertyMultiple,
                service_data,
                Duration::from_secs(6),
            )?;
            ReadPropertyMultipleResponse::decode(&response_data)
                .map_err(|e| ProtocolError::ReadFailed(format!("RPM response decode: {e}")))
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))??;

        // Attribute every returned value by the object identifier and property
        // the device reported, never by position.
        let mut by_key: HashMap<(u32, u32, u32), PropertyResultValue> = HashMap::new();
        for access in &response.read_access_results {
            let ot: u32 = access.object_identifier.object_type.into();
            for prop in &access.results {
                by_key.insert(
                    (ot, access.object_identifier.instance, prop.property_identifier.into()),
                    prop.value.clone(),
                );
            }
        }

        let mut values = Vec::with_capacity(tags.len());
        let mut failures: Vec<String> = Vec::new();
        for (tag, (obj_id, pid)) in tags.iter().zip(&resolved) {
            let ot: u32 = obj_id.object_type.into();
            match by_key.get(&(ot, obj_id.instance, *pid)) {
                Some(PropertyResultValue::Value(vs)) => values.push(property_value_to_bacnet(vs)),
                Some(PropertyResultValue::Error(class, code)) => {
                    failures.push(format!(
                        "{}: {} / {}",
                        tag.name,
                        error_class_name(*class),
                        error_code_name(*code)
                    ));
                    values.push(BacnetValue::Null);
                }
                None => {
                    failures.push(format!("{}: missing from device response", tag.name));
                    values.push(BacnetValue::Null);
                }
            }
        }

        if failures.is_empty() {
            Ok(values)
        } else {
            Err(ProtocolError::ReadFailed(format!(
                "ReadPropertyMultiple failed for {} tag(s): {}",
                failures.len(),
                failures.join("; ")
            )))
        }
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        self.write_with_priority(tag, value, None).await
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        // UDP is connectionless -- nothing to do.
        Ok(())
    }
}

/// Perform a single read of multiple objects from a BACnet device without keeping
/// a persistent client. Useful for one-shot reads.
pub async fn read_objects(
    config: &BacnetConfig,
    objects: &[BacnetObject],
) -> Result<Vec<BacnetValue>, ProtocolError> {
    let client = BacnetClient::connect(config.clone()).await?;
    let values = client.read(objects).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single write to a BACnet device without keeping a persistent client.
pub async fn write_object(
    config: &BacnetConfig,
    object: &BacnetObject,
    value: BacnetValue,
) -> Result<(), ProtocolError> {
    let client = BacnetClient::connect(config.clone()).await?;
    client.write(object, value).await?;
    client.disconnect().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_values_use_the_application_tag_of_the_property_type() {
        // Real 72.5: tag 0x44 (app 4, len 4) + IEEE-754 0x42910000.
        assert_eq!(
            encode_write_value(&BacnetValue::Real(72.5)).unwrap(),
            vec![0x44, 0x42, 0x91, 0x00, 0x00]
        );
        // Enumerated 1 (binary Active): tag 0x91 (app 9, len 1) + 0x01.
        assert_eq!(
            encode_write_value(&BacnetValue::Enum(1)).unwrap(),
            vec![0x91, 0x01]
        );
        // Unsigned 3 (multi-state): tag 0x21 (app 2, len 1) + 0x03.
        assert_eq!(
            encode_write_value(&BacnetValue::Unsigned(3)).unwrap(),
            vec![0x21, 0x03]
        );
        // Unsigned 300: tag 0x22 (len 2) + 0x012C.
        assert_eq!(
            encode_write_value(&BacnetValue::Unsigned(300)).unwrap(),
            vec![0x22, 0x01, 0x2C]
        );
        // Null (relinquish): tag 0x00.
        assert_eq!(encode_write_value(&BacnetValue::Null).unwrap(), vec![0x00]);
        // Boolean true carries its value in the tag byte: 0x11.
        assert_eq!(
            encode_write_value(&BacnetValue::Boolean(true)).unwrap(),
            vec![0x11]
        );
        // Untyped Float64 is written as Real.
        assert_eq!(
            encode_write_value(&BacnetValue::Float64(72.5)).unwrap(),
            vec![0x44, 0x42, 0x91, 0x00, 0x00]
        );
    }

    #[test]
    fn write_property_request_bytes_with_priority() {
        // WriteProperty, analogValue (type 2) instance 1, presentValue (85), Real 72.5, priority 8.
        let obj = ObjectIdentifier::new(ObjectType::AnalogValue, 1);
        let mut req = WritePropertyRequest::new(obj, 85, encode_write_value(&BacnetValue::Real(72.5)).unwrap());
        req.priority = Some(8);
        let mut out = Vec::new();
        req.encode(&mut out).unwrap();
        assert_eq!(
            out,
            vec![
                0x0C, 0x00, 0x80, 0x00, 0x01, // [0] object id: type 2 << 22 | 1
                0x19, 0x55, // [1] property 85
                0x3E, 0x44, 0x42, 0x91, 0x00, 0x00, 0x3F, // [3] opening, Real 72.5, closing
                0x49, 0x08, // [4] priority 8
            ]
        );
    }

    #[test]
    fn read_values_keep_their_type() {
        assert!(matches!(
            property_value_to_bacnet(&[PropertyValue::Real(1.5)]),
            BacnetValue::Float64(v) if v == 1.5
        ));
        assert!(matches!(
            property_value_to_bacnet(&[PropertyValue::Enumerated(2)]),
            BacnetValue::Float64(v) if v == 2.0
        ));
        assert!(matches!(
            property_value_to_bacnet(&[PropertyValue::CharacterString("AHU".into())]),
            BacnetValue::String(s) if s == "AHU"
        ));
        assert!(matches!(property_value_to_bacnet(&[]), BacnetValue::Null));
    }

    /// Hand-assembled ReadPropertyMultiple-ACK for two requested objects where
    /// the FIRST object's property is a character string. The old positional
    /// parser skipped the string and attributed the second object's value to the
    /// first tag.
    #[test]
    fn rpm_ack_values_are_attributed_by_object_not_position() {
        let ack: Vec<u8> = vec![
            // --- ReadAccessResult 1: analogInput,1 ---
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object id AI:1
            0x1E, // [1] opening: list of results
            0x29, 0x1C, // [2] property 28 (description)
            0x4E, // [4] opening: value
            0x74, 0x00, b'a', b'b', b'c', // CharacterString, length 4: charset 0 + "abc"
            0x4F, // [4] closing
            0x1F, // [1] closing
            // --- ReadAccessResult 2: analogInput,2 ---
            0x0C, 0x00, 0x00, 0x00, 0x02, // [0] object id AI:2
            0x1E, // [1] opening
            0x29, 0x55, // [2] property 85 (presentValue)
            0x4E, // [4] opening
            0x44, 0x42, 0x91, 0x00, 0x00, // Real 72.5
            0x4F, // [4] closing
            0x1F, // [1] closing
        ];
        let resp = ReadPropertyMultipleResponse::decode(&ack).unwrap();
        assert_eq!(resp.read_access_results.len(), 2);
        let first = &resp.read_access_results[0];
        assert_eq!(first.object_identifier.instance, 1);
        match &first.results[0].value {
            PropertyResultValue::Value(v) => {
                assert!(matches!(property_value_to_bacnet(v), BacnetValue::String(s) if s == "abc"))
            }
            other => panic!("unexpected {other:?}"),
        }
        let second = &resp.read_access_results[1];
        assert_eq!(second.object_identifier.instance, 2);
        match &second.results[0].value {
            PropertyResultValue::Value(v) => {
                assert!(matches!(property_value_to_bacnet(v), BacnetValue::Float64(x) if x == 72.5))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// A propertyAccessError ([5] opening, error class, error code, [5] closing)
    /// is reported explicitly and does not corrupt later results.
    #[test]
    fn rpm_ack_property_error_is_reported() {
        let ack: Vec<u8> = vec![
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object id AI:1
            0x1E, // [1] opening
            0x29, 0x55, // [2] property 85
            0x5E, // [5] opening: propertyAccessError
            0x91, 0x02, // error class: property (2)
            0x91, 0x20, // error code: unknown-property (32)
            0x5F, // [5] closing
            0x1F, // [1] closing
        ];
        let resp = ReadPropertyMultipleResponse::decode(&ack).unwrap();
        assert_eq!(
            resp.read_access_results[0].results[0].value,
            PropertyResultValue::Error(2, 32)
        );
    }
}
