use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_protocol::{ProtocolClient, ProtocolError};

use bacnet_rs::network::Npdu;
use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier};
use bacnet_rs::service::{
    ConfirmedServiceChoice, PropertyReference, ReadAccessSpecification, ReadPropertyMultipleRequest,
    ReadPropertyRequest,
};

use crate::helpers::{object_type_from_str, property_id_from_str};
use crate::types::{BacnetConfig, BacnetObject, BacnetValue};

/// Global monotonically-incrementing invoke ID counter (0-255 wrapping).
static NEXT_INVOKE_ID: AtomicU8 = AtomicU8::new(0);

fn next_invoke_id() -> u8 {
    NEXT_INVOKE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Decode the first application-tagged value at the start of `data` into f64,
/// returning the value and the bytes consumed (tag header + value). Handles
/// Real/Double/Unsigned/Signed/Boolean/Enumerated; `None` for types the gateway
/// does not need (strings, dates, octet-strings) or truncated data.
fn decode_app_value(data: &[u8]) -> Option<(f64, usize)> {
    let tag = *data.first()?;
    let tagnum = tag >> 4;
    // Boolean — the value rides in the low nibble, no value bytes follow.
    if tagnum == 1 {
        return Some(((tag & 0x07 != 0) as u8 as f64, 1));
    }
    let mut len = (tag & 0x07) as usize;
    let mut vs = 1usize;
    if len == 5 {
        // Extended length: the real length is the next byte.
        len = *data.get(1)? as usize;
        vs = 2;
    }
    if vs + len > data.len() {
        return None;
    }
    let vb = &data[vs..vs + len];
    let val = match tagnum {
        4 if len == 4 => Some(f32::from_be_bytes(vb.try_into().ok()?) as f64),
        5 if len == 8 => Some(f64::from_be_bytes(vb.try_into().ok()?)),
        2 | 9 => {
            let mut v = 0u64;
            for &b in vb {
                v = (v << 8) | b as u64;
            }
            Some(v as f64)
        }
        3 => {
            let mut v = 0u64;
            for &b in vb {
                v = (v << 8) | b as u64;
            }
            if !vb.is_empty() && vb[0] & 0x80 != 0 {
                v |= !0u64 << (vb.len() * 8);
            }
            Some(v as i64 as f64)
        }
        _ => None,
    };
    val.map(|v| (v, vs + len))
}

/// Standalone BACnet/IP client implementing the [`ProtocolClient`] trait.
///
/// Communication is performed over UDP using `bacnet-rs` encoding and raw
/// socket I/O wrapped in `spawn_blocking` so the async trait methods remain
/// non-blocking on the Tokio runtime.
pub struct BacnetClient {
    addr: SocketAddr,
    socket: Arc<Mutex<std::net::UdpSocket>>,
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

            client_send_confirmed(
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
                "No BACnet response from {socket_addr} (device {device_id}) within 3s: {e}"
            ))
        })?;

        Ok(Self {
            addr: socket_addr,
            socket,
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        // Resolve object types and properties.
        let objects: Vec<(String, bacnet_rs::object::ObjectType, u32, PropertyIdentifier)> = tags
            .iter()
            .filter_map(|t| {
                let ot = object_type_from_str(&t.object_type)?;
                let pid = property_id_from_str(&t.property);
                Some((t.name.clone(), ot, t.object_instance, pid))
            })
            .collect();

        if objects.is_empty() {
            return Ok(Vec::new());
        }

        // Build ReadPropertyMultiple request.
        let mut read_specs = Vec::new();
        for (_, ot, inst, pid) in &objects {
            let obj_id = ObjectIdentifier::new(*ot, *inst);
            let prop_ref = PropertyReference::new(*pid);
            read_specs.push(ReadAccessSpecification::new(obj_id, vec![prop_ref]));
        }

        let rpm_request = ReadPropertyMultipleRequest::new(read_specs);
        let addr = self.addr;
        let socket = Arc::clone(&self.socket);

        let metrics = tokio::task::spawn_blocking(move || -> Result<HashMap<String, f64>, ProtocolError> {
            let sock = socket.lock().map_err(|e| {
                ProtocolError::Other(format!("Socket lock poisoned: {e}"))
            })?;

            sock.set_read_timeout(Some(Duration::from_secs(6)))
                .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

            let mut service_data = Vec::new();
            rpm_request
                .encode(&mut service_data)
                .map_err(|e| ProtocolError::ReadFailed(format!("RPM encode: {e}")))?;

            let response_data = client_send_confirmed(&sock, addr, ConfirmedServiceChoice::ReadPropertyMultiple, service_data, Duration::from_secs(6))?;

            // Parse the ReadAccessResult list. Each entry is:
            //   context-tag-0 objectIdentifier, open-ctx-1,
            //   [ ctx-2 propertyIdentifier, (ctx-3 arrayIndex)?,
            //     open-ctx-4, <application values…>, close-ctx-4 ]*,
            //   close-ctx-1.
            // The first application value of each entry maps to the corresponding
            // requested object (one property per request tag). Walking the
            // structure (rather than byte-scanning) avoids mistaking property-id
            // bytes such as presentValue=85=0x55 for value tags.
            let data = &response_data;
            let mut pos = 0;
            let mut obj_idx = 0;
            let mut result: HashMap<String, f64> = HashMap::new();

            while pos < data.len() && obj_idx < objects.len() {
                // objectIdentifier — context tag 0.
                let (_oid, c) = match bacnet_rs::encoding::decode_context_object_id(&data[pos..], 0) {
                    Ok(v) => v,
                    Err(_) => break,
                };
                pos += c;
                // Opening tag 1 (list of access results).
                let (tag, _, c) = match bacnet_rs::encoding::decode_context_tag(&data[pos..]) {
                    Ok(v) => v,
                    Err(_) => break,
                };
                if tag != 1 {
                    break;
                }
                pos += c;
                // Walk access-result entries until the matching close tag 1.
                loop {
                    let (t, _, c) = match bacnet_rs::encoding::decode_context_tag(&data[pos..]) {
                        Ok(v) => v,
                        Err(_) => break,
                    };
                    if t == 1 {
                        pos += c; // close tag 1 → next object
                        break;
                    }
                    // propertyIdentifier — context tag 2. decode_context_enumerated
                    // consumes the tag header + value together, so do NOT pre-advance
                    // pos (the peek above only checked the tag number).
                    if let Ok((_, c)) =
                        bacnet_rs::encoding::decode_context_enumerated(&data[pos..], 2)
                    {
                        pos += c;
                    }
                    // Optional array index — context tag 3.
                    if let Ok((t3, _, _)) = bacnet_rs::encoding::decode_context_tag(&data[pos..]) {
                        if t3 == 3 {
                            if let Ok((_, c)) =
                                bacnet_rs::encoding::decode_context_unsigned(&data[pos..], 3)
                            {
                                pos += c;
                            }
                        }
                    }
                    // Opening tag 4 wrapping the value(s).
                    let (t4, _, c4) = match bacnet_rs::encoding::decode_context_tag(&data[pos..]) {
                        Ok(v) => v,
                        Err(_) => break,
                    };
                    if t4 != 4 {
                        break;
                    }
                    pos += c4;
                    // First application value → map to this requested object.
                    if let Some((v, c)) = decode_app_value(&data[pos..]) {
                        let (name, _, _, _) = &objects[obj_idx];
                        result.insert(name.clone(), v);
                        obj_idx += 1;
                        pos += c;
                    }
                    // Skip any remaining value bytes up to close tag 4 (0x4F).
                    while pos < data.len() {
                        let (t, _, c) = match bacnet_rs::encoding::decode_context_tag(&data[pos..]) {
                            Ok(v) => v,
                            Err(_) => {
                                pos += 1;
                                continue;
                            }
                        };
                        pos += c;
                        if t == 4 {
                            break;
                        }
                    }
                }
            }

            Ok(result)
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))??;

        // Map back to Vec<BacnetValue> in the same order as `tags`.
        let values = tags
            .iter()
            .map(|t| {
                metrics
                    .get(&t.name)
                    .map(|&v| BacnetValue::Float64(v))
                    .unwrap_or(BacnetValue::Float64(f64::NAN))
            })
            .collect();

        Ok(values)
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let ot = object_type_from_str(&tag.object_type)
            .ok_or_else(|| ProtocolError::InvalidConfig(format!("Unknown object type: {}", tag.object_type)))?;
        let pid = property_id_from_str(&tag.property);
        let obj_id = ObjectIdentifier::new(ot, tag.object_instance);
        let numeric_value = value.as_f64();

        let addr = self.addr;
        let socket = Arc::clone(&self.socket);

        tokio::task::spawn_blocking(move || -> Result<(), ProtocolError> {
            use bacnet_rs::service::WritePropertyRequest;

            let sock = socket.lock().map_err(|e| {
                ProtocolError::Other(format!("Socket lock poisoned: {e}"))
            })?;

            sock.set_read_timeout(Some(Duration::from_secs(6)))
                .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

            // Encode property value as BACnet Real (tag 0x44 + 4-byte float).
            let mut property_value = vec![0x44];
            property_value.extend_from_slice(&(numeric_value as f32).to_be_bytes());

            let pid_u32: u32 = pid.into();
            let write_req = WritePropertyRequest::new(obj_id, pid_u32, property_value);

            let mut service_data = Vec::new();
            write_req
                .encode(&mut service_data)
                .map_err(|e| ProtocolError::WriteFailed(format!("Encode: {e}")))?;

            let response_data = client_send_confirmed(
                &sock,
                addr,
                ConfirmedServiceChoice::WriteProperty,
                service_data,
                Duration::from_secs(6),
            )?;

            // For WriteProperty we expect a SimpleAck which is returned as empty data
            // by our client_send_confirmed helper (it already checked the APDU type).
            let _ = response_data;
            Ok(())
        })
        .await
        .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))??;

        Ok(())
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        // UDP is connectionless -- nothing to do.
        Ok(())
    }
}

/// Send a confirmed BACnet request and return the service_data from the response APDU.
fn client_send_confirmed(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    service_choice: ConfirmedServiceChoice,
    service_data: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, ProtocolError> {
    use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};

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
        .map_err(|e| ProtocolError::WriteFailed(format!("Send: {e}")))?;

    let mut buf = [0u8; 1500];
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
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
                    if let Ok(Apdu::ComplexAck { invoke_id: resp_invoke_id, service_data, .. }) =
                        Apdu::decode(&buf[apdu_start..len])
                    {
                        if resp_invoke_id != invoke_id {
                            continue; // Not our response, keep waiting
                        }
                        return Ok(service_data);
                    }
                    // SimpleAck for WriteProperty
                    if let Ok(Apdu::SimpleAck { invoke_id: resp_invoke_id, .. }) = Apdu::decode(&buf[apdu_start..len]) {
                        if resp_invoke_id != invoke_id {
                            continue; // Not our response, keep waiting
                        }
                        return Ok(Vec::new());
                    }
                }
            }
            Ok(_) => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(ProtocolError::ReadFailed(format!("Recv: {e}"))),
        }
    }

    Err(ProtocolError::Timeout(format!(
        "BACnet confirmed request timeout for {target_addr}"
    )))
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

/// Register as a foreign device with a BBMD (BACnet/IP Broadcast Management Device).
pub fn register_bbmd(bbmd_ip: &str, bbmd_port: u16, ttl: u16) -> Result<(), ProtocolError> {
    use bacnet_rs::datalink::bip::BacnetIpDataLink;
    let mut datalink = BacnetIpDataLink::new("0.0.0.0:0")
        .map_err(|e| ProtocolError::ConnectionFailed(format!("Datalink: {e}")))?;
    let bbmd_addr: std::net::SocketAddr = format!("{}:{}", bbmd_ip, bbmd_port)
        .parse()
        .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid BBMD addr: {e}")))?;
    datalink.register_foreign_device(bbmd_addr, ttl)
        .map_err(|e| ProtocolError::Other(format!("BBMD registration: {e}")))?;
    Ok(())
}
