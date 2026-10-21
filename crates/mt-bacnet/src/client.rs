use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_protocol::{ProtocolClient, ProtocolError};

use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier};
use bacnet_rs::property::{encode_property_value, PropertyValue};
use bacnet_rs::service::{
    ConfirmedServiceChoice, PropertyReference, PropertyResultValue, ReadAccessSpecification,
    ReadPropertyMultipleRequest, ReadPropertyRequest, WritePropertyRequest,
};

use crate::discovery::{decode_read_property_ack, decode_rpm_ack};
use crate::helpers::{object_type_from_str, property_id_from_str};
use crate::transport::{
    confirmed_request, confirmed_request_detailed, error_class_name, error_code_name, RequestFailure,
};
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
    /// Properties asked per ReadPropertyMultiple request. Starts from the device's
    /// Max_APDU_Length_Accepted and shrinks when a device cannot fit a reply.
    rpm_batch: Arc<AtomicUsize>,
    /// The device rejected ReadPropertyMultiple as an unrecognized service (clause 18.8):
    /// read one property per ReadProperty from now on.
    rpm_unsupported: Arc<AtomicBool>,
}

/// Octets one property takes in a ReadPropertyMultiple request: `[0]` object identifier (5),
/// `[1]` opening tag, `[0]` property identifier (2 to 4), `[1]` closing tag (clause 15.7.1.1).
const RPM_REQUEST_OCTETS_PER_PROPERTY: usize = 12;
/// Octets of a confirmed-request APDU before the service data: type, max-APDU, invoke id, service.
const CONFIRMED_REQUEST_HEADER_OCTETS: usize = 4;
/// Most properties per ReadPropertyMultiple request. The reply carries about 16 to 20 octets
/// per Real value and must fit the 1476 octets this client accepts (a device that cannot
/// segment aborts otherwise).
const RPM_MAX_PROPERTIES: usize = 60;
/// Device object instance that addresses the receiving device itself (ASHRAE 135 clause 12.11.1).
const WILDCARD_DEVICE_INSTANCE: u32 = 4_194_303;
/// Max_APDU_Length_Accepted assumed when the device does not report one (MS/TP-sized, the
/// smallest value commonly seen behind a BACnet/IP router).
const DEFAULT_MAX_APDU: usize = 480;

/// Properties per ReadPropertyMultiple request that fit a device accepting `max_apdu` octets.
fn rpm_batch_for(max_apdu: usize) -> usize {
    (max_apdu.saturating_sub(CONFIRMED_REQUEST_HEADER_OCTETS) / RPM_REQUEST_OCTETS_PER_PROPERTY)
        .clamp(1, RPM_MAX_PROPERTIES)
}

type PropertyKey = (u32, u32, u32);

fn property_key(object: ObjectIdentifier, property: PropertyIdentifier) -> PropertyKey {
    (object.object_type.into(), object.instance, property.into())
}

/// ReadProperty of one property, with an Error PDU turned into a per-property result.
fn read_one_property(
    sock: &std::net::UdpSocket,
    addr: SocketAddr,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<PropertyResultValue, RequestFailure> {
    let mut data = Vec::new();
    ReadPropertyRequest::new(object, property)
        .encode(&mut data)
        .map_err(|e| RequestFailure::Local(ProtocolError::ReadFailed(format!("ReadProperty encode: {e}"))))?;
    match confirmed_request_detailed(sock, addr, ConfirmedServiceChoice::ReadProperty, data, Duration::from_secs(6)) {
        Ok(reply) => match decode_read_property_ack(&reply) {
            // A request for the wildcard device instance is answered with the device's real instance.
            Some(ack)
                if ack.object.object_type == object.object_type
                    && (ack.object.instance == object.instance || object.instance == WILDCARD_DEVICE_INSTANCE)
                    && ack.property == u32::from(property) =>
            {
                Ok(PropertyResultValue::Value(ack.values))
            }
            _ => Err(RequestFailure::Local(ProtocolError::ReadFailed(
                "ReadProperty ACK could not be decoded or answers another property".into(),
            ))),
        },
        Err(RequestFailure::Error { class, code }) => Ok(PropertyResultValue::Error(class, code)),
        Err(other) => Err(other),
    }
}

/// Read every `(object, property)` and return the device's answer for each, keyed by what the
/// device reported.
///
/// ReadPropertyMultiple requests carry at most `batch` properties. A device that cannot take the
/// request or fit the reply (Abort buffer-overflow / segmentation-not-supported / apdu-too-long,
/// Reject buffer-overflow) gets half as many next time. A device without ReadPropertyMultiple
/// (Reject unrecognized-service) is read one ReadProperty per property.
fn read_properties(
    sock: &std::net::UdpSocket,
    addr: SocketAddr,
    wanted: &[(ObjectIdentifier, PropertyIdentifier)],
    batch: &AtomicUsize,
    rpm_unsupported: &AtomicBool,
) -> Result<HashMap<PropertyKey, PropertyResultValue>, ProtocolError> {
    let mut answers = HashMap::new();
    let mut start = 0;
    while start < wanted.len() {
        if rpm_unsupported.load(Ordering::Relaxed) {
            for &(object, property) in &wanted[start..] {
                answers.insert(property_key(object, property), read_one_property(sock, addr, object, property)?);
            }
            return Ok(answers);
        }

        let size = batch.load(Ordering::Relaxed).clamp(1, wanted.len() - start);
        let specs = wanted[start..start + size]
            .iter()
            .map(|&(object, property)| ReadAccessSpecification::new(object, vec![PropertyReference::new(property)]))
            .collect();
        let mut service_data = Vec::new();
        ReadPropertyMultipleRequest::new(specs)
            .encode(&mut service_data)
            .map_err(|e| ProtocolError::ReadFailed(format!("RPM encode: {e}")))?;

        match confirmed_request_detailed(
            sock,
            addr,
            ConfirmedServiceChoice::ReadPropertyMultiple,
            service_data,
            Duration::from_secs(6),
        ) {
            Ok(reply) => {
                let results = decode_rpm_ack(&reply)
                    .ok_or_else(|| ProtocolError::ReadFailed("RPM response could not be decoded".into()))?;
                for access in &results {
                    for prop in &access.results {
                        answers.insert(
                            property_key(access.object_identifier, prop.property_identifier),
                            prop.value.clone(),
                        );
                    }
                }
                start += size;
            }
            Err(RequestFailure::Reject(9)) => rpm_unsupported.store(true, Ordering::Relaxed),
            Err(failure) if failure.is_size_limit() && size > 1 => {
                batch.store(size / 2, Ordering::Relaxed);
            }
            Err(failure) => return Err(failure.into()),
        }
    }
    Ok(answers)
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
        let pid: u32 = property_id_from_str(&tag.property)
            .ok_or_else(|| ProtocolError::InvalidConfig(format!("Unknown property: {}", tag.property)))?
            .into();
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
        // The wildcard instance 4194303 addresses the receiving device whatever its own
        // instance number is (clause 12.11.1), so a wrong configured device id does not
        // make a reachable device look dead. Any BACnet reply, even an Error, proves it
        // is there; only silence or a local failure fails the probe.
        let probe_socket = Arc::clone(&socket);
        let probe_addr = socket_addr;
        let device_id = config.device_id;
        let max_apdu = tokio::task::spawn_blocking(move || -> Result<usize, ProtocolError> {
            let sock = probe_socket
                .lock()
                .map_err(|e| ProtocolError::Other(format!("Socket lock poisoned: {e}")))?;
            sock.set_read_timeout(Some(Duration::from_millis(500)))
                .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

            let device = ObjectIdentifier::new(ObjectType::Device, WILDCARD_DEVICE_INSTANCE);
            match read_one_property(&sock, probe_addr, device, PropertyIdentifier::ObjectIdentifier) {
                Ok(_) | Err(RequestFailure::Error { .. } | RequestFailure::UndecodableError | RequestFailure::Reject(_) | RequestFailure::Abort(_)) => {}
                Err(failure) => return Err(failure.into()),
            }
            // How large a request the device takes decides how many properties go into one RPM.
            let max_apdu = match read_one_property(&sock, probe_addr, device, PropertyIdentifier::MaxApduLengthAccepted) {
                Ok(PropertyResultValue::Value(values)) => match values.first() {
                    Some(PropertyValue::Unsigned(n)) => usize::try_from(*n).unwrap_or(DEFAULT_MAX_APDU),
                    _ => DEFAULT_MAX_APDU,
                },
                _ => DEFAULT_MAX_APDU,
            };
            Ok(max_apdu)
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
            rpm_batch: Arc::new(AtomicUsize::new(rpm_batch_for(max_apdu))),
            rpm_unsupported: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve every tag up front; a tag that cannot be resolved is an error,
        // not a silently missing value (and never a different property).
        let mut resolved: Vec<(ObjectIdentifier, PropertyIdentifier)> = Vec::with_capacity(tags.len());
        for t in tags {
            let ot = object_type_from_str(&t.object_type).ok_or_else(|| {
                ProtocolError::InvalidConfig(format!(
                    "Tag '{}': unknown object type '{}'",
                    t.name, t.object_type
                ))
            })?;
            let property = property_id_from_str(&t.property).ok_or_else(|| {
                ProtocolError::InvalidConfig(format!("Tag '{}': unknown property '{}'", t.name, t.property))
            })?;
            resolved.push((ObjectIdentifier::new(ot, t.object_instance), property));
        }

        let addr = self.addr;
        let socket = Arc::clone(&self.socket);
        let batch = Arc::clone(&self.rpm_batch);
        let rpm_unsupported = Arc::clone(&self.rpm_unsupported);
        let wanted = resolved.clone();

        // Attribute every returned value by the object identifier and property
        // the device reported, never by position.
        let by_key = tokio::task::spawn_blocking(move || -> Result<HashMap<PropertyKey, PropertyResultValue>, ProtocolError> {
            let sock = socket
                .lock()
                .map_err(|e| ProtocolError::Other(format!("Socket lock poisoned: {e}")))?;
            read_properties(&sock, addr, &wanted, &batch, &rpm_unsupported)
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))??;

        let mut values = Vec::with_capacity(tags.len());
        let mut failures: Vec<String> = Vec::new();
        for (tag, &(obj_id, property)) in tags.iter().zip(&resolved) {
            match by_key.get(&property_key(obj_id, property)) {
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
        let results = decode_rpm_ack(&ack).unwrap();
        assert_eq!(results.len(), 2);
        let first = &results[0];
        assert_eq!(first.object_identifier.instance, 1);
        match &first.results[0].value {
            PropertyResultValue::Value(v) => {
                assert!(matches!(property_value_to_bacnet(v), BacnetValue::String(s) if s == "abc"))
            }
            other => panic!("unexpected {other:?}"),
        }
        let second = &results[1];
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
        let results = decode_rpm_ack(&ack).unwrap();
        assert_eq!(results[0].results[0].value, PropertyResultValue::Error(2, 32));
    }
    // ---- Reads against a scripted device on loopback ----

    use crate::transport::{apdu_of_frame, unicast_frame};
    use std::sync::atomic::AtomicUsize;

    /// UDP device on loopback; `handler(invoke_id, service_choice, service_data)` returns the reply APDU.
    fn spawn_device(handler: impl Fn(u8, u8, &[u8]) -> Vec<u8> + Send + 'static) -> SocketAddr {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while let Ok((n, peer)) = socket.recv_from(&mut buf) {
                let Some(apdu) = apdu_of_frame(&buf[..n]) else { continue };
                if apdu.len() < 4 || apdu[0] >> 4 != 0 {
                    continue;
                }
                let reply = handler(apdu[2], apdu[3], &apdu[4..]);
                socket.send_to(&unicast_frame(&reply, false), peer).unwrap();
            }
        });
        addr
    }

    fn complex_ack(invoke: u8, service: u8, data: &[u8]) -> Vec<u8> {
        let mut apdu = vec![0x30, invoke, service];
        apdu.extend_from_slice(data);
        apdu
    }

    /// ReadProperty answers for the connect probe: Object_Identifier (75) and Max_APDU_Length_Accepted
    /// (62, as `max_apdu`) of the device addressed with the wildcard instance 4194303. A real device
    /// answers with its own instance (8 here), not the wildcard.
    fn probe_reply(invoke: u8, data: &[u8], max_apdu: u16) -> Vec<u8> {
        // Request: 0C 02 3F FF FF (device 4194303), 19 <property>.
        assert_eq!(&data[..5], &[0x0C, 0x02, 0x3F, 0xFF, 0xFF], "probe must use the wildcard device instance");
        match data[6] {
            75 => complex_ack(invoke, 12, &[0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 75, 0x3E, 0xC4, 0x02, 0x00, 0x00, 0x08, 0x3F]),
            62 => {
                let mut ack = vec![0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 62, 0x3E, 0x22];
                ack.extend_from_slice(&max_apdu.to_be_bytes());
                ack.push(0x3F);
                complex_ack(invoke, 12, &ack)
            }
            other => panic!("unexpected probe property {other}"),
        }
    }

    /// Instances asked for in a ReadPropertyMultiple request made of
    /// `0C 00 00 00 <n> 1E 09 55 1F` (analogInput n, presentValue) entries.
    fn rpm_instances(data: &[u8]) -> Vec<u8> {
        assert_eq!(data.len() % 9, 0, "unexpected RPM request shape: {data:02X?}");
        data.chunks(9)
            .map(|c| {
                assert_eq!(&c[..4], &[0x0C, 0x00, 0x00, 0x00]);
                assert_eq!(&c[5..], &[0x1E, 0x09, 0x55, 0x1F]);
                c[4]
            })
            .collect()
    }

    /// RPM-ACK for those instances; analogInput n reads as Real n.0.
    fn rpm_reply(invoke: u8, instances: &[u8]) -> Vec<u8> {
        let mut ack = Vec::new();
        for &n in instances {
            ack.extend_from_slice(&[0x0C, 0x00, 0x00, 0x00, n, 0x1E, 0x29, 0x55, 0x4E, 0x44]);
            ack.extend_from_slice(&(n as f32).to_be_bytes());
            ack.extend_from_slice(&[0x4F, 0x1F]);
        }
        complex_ack(invoke, 14, &ack)
    }

    fn ai_tags(count: u8) -> Vec<BacnetObject> {
        (1..=count)
            .map(|n| BacnetObject {
                name: format!("ai{n}"),
                object_type: "analogInput".into(),
                object_instance: n as u32,
                property: "presentValue".into(),
            })
            .collect()
    }

    async fn client_for(addr: SocketAddr) -> BacnetClient {
        BacnetClient::connect(BacnetConfig {
            id: "t".into(),
            ip: addr.ip().to_string(),
            port: addr.port(),
            device_id: 8,
            poll_interval_ms: 1000,
        })
        .await
        .unwrap()
    }

    fn assert_values(values: &[BacnetValue], count: u8) {
        assert_eq!(values.len(), count as usize);
        for (i, v) in values.iter().enumerate() {
            assert!(matches!(v, BacnetValue::Float64(x) if *x == (i + 1) as f64), "{i}: {v:?}");
        }
    }

    #[tokio::test]
    async fn reads_are_split_to_what_the_devices_max_apdu_accepts() {
        // Max_APDU_Length_Accepted 206: (206 - 4) / 12 = 16 properties per request.
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&sizes);
        let addr = spawn_device(move |invoke, service, data| match service {
            12 => probe_reply(invoke, data, 206),
            14 => {
                let instances = rpm_instances(data);
                seen.lock().unwrap().push(instances.len());
                rpm_reply(invoke, &instances)
            }
            other => panic!("unexpected service {other}"),
        });
        let client = client_for(addr).await;
        assert_values(&client.read(&ai_tags(40)).await.unwrap(), 40);
        assert_eq!(*sizes.lock().unwrap(), vec![16, 16, 8]);

        // A property name that is not one is refused, not read as presentValue.
        let mut bad = ai_tags(1);
        bad[0].property = "presentvaleu".into();
        let err = client.read(&bad).await.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidConfig(_)), "{err:?}");
        let err = client.write(&bad[0], BacnetValue::Real(1.0)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidConfig(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_device_that_cannot_fit_the_reply_gets_smaller_requests() {
        // Advertises 1476 but aborts (4 = segmentation-not-supported) any RPM above 10 properties.
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&sizes);
        let addr = spawn_device(move |invoke, service, data| match service {
            12 => probe_reply(invoke, data, 1476),
            14 => {
                let instances = rpm_instances(data);
                seen.lock().unwrap().push(instances.len());
                if instances.len() > 10 {
                    vec![0x71, invoke, 0x04]
                } else {
                    rpm_reply(invoke, &instances)
                }
            }
            other => panic!("unexpected service {other}"),
        });
        let client = client_for(addr).await;
        assert_values(&client.read(&ai_tags(40)).await.unwrap(), 40);
        assert_eq!(*sizes.lock().unwrap(), vec![40, 20, 10, 10, 10, 10]);
        // The smaller size is remembered.
        sizes.lock().unwrap().clear();
        assert_values(&client.read(&ai_tags(20)).await.unwrap(), 20);
        assert_eq!(*sizes.lock().unwrap(), vec![10, 10]);
    }

    #[tokio::test]
    async fn a_device_without_read_property_multiple_is_read_one_property_at_a_time() {
        let rpm_requests = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rpm_requests);
        let addr = spawn_device(move |invoke, service, data| match service {
            12 if data[1] == 0x02 => probe_reply(invoke, data, 480),
            12 => {
                // ReadProperty 0C 00 00 00 <n> 19 55 for analogInput n.
                let n = data[4];
                let mut ack = vec![0x0C, 0x00, 0x00, 0x00, n, 0x19, 0x55, 0x3E, 0x44];
                ack.extend_from_slice(&(n as f32).to_be_bytes());
                ack.push(0x3F);
                complex_ack(invoke, 12, &ack)
            }
            // Reject, reason 9 = unrecognized-service.
            14 => {
                counted.fetch_add(1, Ordering::Relaxed);
                vec![0x60, invoke, 0x09]
            }
            other => panic!("unexpected service {other}"),
        });
        let client = client_for(addr).await;
        assert_values(&client.read(&ai_tags(5)).await.unwrap(), 5);
        assert_values(&client.read(&ai_tags(3)).await.unwrap(), 3);
        assert_eq!(rpm_requests.load(Ordering::Relaxed), 1, "RPM must not be retried once rejected");
    }
}
