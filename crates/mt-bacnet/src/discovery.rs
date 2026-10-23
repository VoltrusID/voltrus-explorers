use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use if_addrs::get_if_addrs;
use mt_protocol::ProtocolError;

use crate::transport::{confirmed_request, REQUEST_TIMEOUT};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::engineering_units::EngineeringUnits;
use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier};
use bacnet_rs::property::PropertyValue;
use bacnet_rs::service::{
    ConfirmedServiceChoice, IAmRequest, PropertyReference, PropertyResult, PropertyResultValue,
    ReadAccessResult, ReadAccessSpecification, ReadPropertyMultipleRequest, ReadPropertyRequest,
    UnconfirmedServiceChoice, WhoIsRequest,
};
use bacnet_rs::vendor::get_vendor_name;

use crate::types::{
    BacnetDiscoveredDevice, BacnetObjectDetail, BacnetPropertyEntry, BacnetValue, DeviceInspection,
};
use crate::BacnetObjectInfo;

/// Default BACnet/IP UDP port (0xBAC0).
const BACNET_IP_PORT: u16 = 47808;

/// BVLC function: Original-Unicast-NPDU (ASHRAE 135 Annex J.2.11).
const BVLC_ORIGINAL_UNICAST: u8 = 0x0A;
/// BVLC function: Original-Broadcast-NPDU (Annex J.2.12).
const BVLC_ORIGINAL_BROADCAST: u8 = 0x0B;
/// BVLC function: Forwarded-NPDU (Annex J.2.5): a 6-byte B/IP address of the
/// originating device follows the BVLC header.
const BVLC_FORWARDED_NPDU: u8 = 0x04;

/// Build the BVLC Original-Broadcast-NPDU frame carrying a Who-Is behind a
/// global-broadcast NPDU (DNET 0xFFFF, DLEN 0, hop count 255; clause 6.2.2).
fn who_is_frame(request: &WhoIsRequest) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Npdu::global_broadcast().encode();
    // Unconfirmed-Request PDU (type 1) carrying service choice 8, then the
    // optional [0] low-limit / [1] high-limit (clause 16.10.1).
    body.extend_from_slice(&[0x10, UnconfirmedServiceChoice::WhoIs as u8]);
    request
        .encode(&mut body)
        .map_err(|e| ProtocolError::Other(format!("WhoIs encode: {e}")))?;

    let total = u16::try_from(body.len() + 4)
        .map_err(|_| ProtocolError::Other("WhoIs frame too long".into()))?;
    let mut frame = vec![0x81, BVLC_ORIGINAL_BROADCAST];
    frame.extend_from_slice(&total.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Directed-broadcast address of `ip`/`netmask` (host bits all ones). `None` for
/// loopback and for /31 and /32 networks, which have no broadcast address.
fn directed_broadcast(ip: Ipv4Addr, netmask: Ipv4Addr) -> Option<Ipv4Addr> {
    let mask = u32::from(netmask);
    if ip.is_loopback() || mask >= 0xFFFF_FFFE {
        return None;
    }
    Some(Ipv4Addr::from(u32::from(ip) | !mask))
}

/// Every address a Who-Is must be sent to: the directed broadcast of each host
/// interface (computed from the netmask when the OS does not report one) plus
/// the limited broadcast 255.255.255.255.
fn who_is_targets() -> Vec<Ipv4Addr> {
    let mut targets: Vec<Ipv4Addr> = Vec::new();
    if let Ok(ifaces) = get_if_addrs() {
        for iface in ifaces {
            if let if_addrs::IfAddr::V4(v4) = iface.addr {
                let bcast = v4
                    .broadcast
                    .filter(|b| !b.is_unspecified())
                    .or_else(|| directed_broadcast(v4.ip, v4.netmask));
                if let Some(bcast) = bcast {
                    if !targets.contains(&bcast) {
                        targets.push(bcast);
                    }
                }
            }
        }
    }
    let limited = Ipv4Addr::BROADCAST;
    if !targets.contains(&limited) {
        targets.push(limited);
    }
    targets
}

/// Broadcast a BACnet Who-Is and collect I-Am responses for the given timeout.
pub async fn discover_devices(
    timeout_secs: u64,
) -> Result<Vec<BacnetDiscoveredDevice>, ProtocolError> {
    tokio::task::spawn_blocking(move || {
        let frame = who_is_frame(&WhoIsRequest::new())?;

        // Devices answer a broadcast Who-Is with a broadcast I-Am to port 47808
        // (clause 16.10.2), so the receive socket must own that port.
        let socket = match UdpSocket::bind(("0.0.0.0", BACNET_IP_PORT)) {
            Ok(socket) => socket,
            Err(e) => {
                // Another BACnet application (or this app's COV listener) holds
                // 47808. Replies sent unicast to our source port still arrive,
                // but broadcast I-Ams do not.
                tracing::warn!(
                    "BACnet port {BACNET_IP_PORT} unavailable ({e}); using an ephemeral port, \
                     devices that answer Who-Is with a broadcast I-Am will not be seen"
                );
                UdpSocket::bind("0.0.0.0:0")
                    .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?
            }
        };
        tracing::info!(
            "BACnet discovery socket bound to {:?}",
            socket.local_addr().ok()
        );
        socket
            .set_broadcast(true)
            .map_err(|e| ProtocolError::ConnectionFailed(format!("set_broadcast: {e}")))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| ProtocolError::ConnectionFailed(format!("set_read_timeout: {e}")))?;

        // Directed broadcast per interface: bacnet-rs would compute `0.0.0.255`
        // for a socket bound to 0.0.0.0 (os error 65 on send), and macOS limited
        // broadcast does not reach vmnet subnets.
        let targets = who_is_targets();
        tracing::info!("BACnet Who-Is targets: {:?}", targets);
        let mut sent = 0usize;
        let mut last_error = None;
        for target in &targets {
            match socket.send_to(&frame, SocketAddr::from((*target, BACNET_IP_PORT))) {
                Ok(_) => sent += 1,
                Err(e) => {
                    tracing::warn!("Who-Is send to {target} failed: {e}");
                    last_error = Some(e);
                }
            }
        }
        // iOS refuses broadcast sends unless the app holds Apple's multicast networking
        // entitlement; report that instead of waiting out the timeout for replies that cannot come.
        if sent == 0 {
            if let Some(e) = last_error {
                return Err(ProtocolError::ConnectionFailed(format!(
                    "Who-Is broadcast could not be sent: {e}"
                )));
            }
        }

        let mut discovered: Vec<BacnetDiscoveredDevice> = Vec::new();
        let start = std::time::Instant::now();
        let mut buf = [0u8; 2048];

        while start.elapsed() < Duration::from_secs(timeout_secs) {
            if let Ok((n, source)) = socket.recv_from(&mut buf) {
                if let Some(device) = parse_iam_response(&buf[..n], source) {
                    if !discovered
                        .iter()
                        .any(|d| d.device_instance == device.device_instance)
                    {
                        discovered.push(device);
                    }
                }
            }
        }

        Ok(discovered)
    })
    .await
    .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
}

/// Parse a received BVLC datagram into a [`BacnetDiscoveredDevice`] if it carries
/// an I-Am (unconfirmed service 0, clause 16.10).
///
/// The device address is the UDP source, except for a Forwarded-NPDU (a BBMD
/// relayed the I-Am from another subnet), where the 6-byte B/IP address after the
/// BVLC header is the real device. For a device behind a router the NPDU carries
/// SNET/SADR and the UDP source is the router; the device is still listed under
/// the router's address because [`BacnetDiscoveredDevice`] has no network field.
fn parse_iam_response(data: &[u8], source: SocketAddr) -> Option<BacnetDiscoveredDevice> {
    if data.len() < 4 || data[0] != 0x81 {
        return None;
    }
    if usize::from(u16::from_be_bytes([data[2], data[3]])) != data.len() {
        return None;
    }

    let (origin, npdu_start) = match data[1] {
        BVLC_ORIGINAL_UNICAST | BVLC_ORIGINAL_BROADCAST => (source, 4),
        BVLC_FORWARDED_NPDU => {
            let addr = data.get(4..10)?;
            let ip = Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]);
            (
                SocketAddr::from((ip, u16::from_be_bytes([addr[4], addr[5]]))),
                10,
            )
        }
        _ => return None,
    };

    let npdu_bytes = data.get(npdu_start..)?;
    let (npdu, npdu_len) = Npdu::decode(npdu_bytes).ok()?;
    if npdu.control.network_message {
        return None;
    }

    let apdu = npdu_bytes.get(npdu_len..)?;
    // PDU type 1 (Unconfirmed-Request) in the high nibble; the low nibble is reserved.
    if apdu.len() < 2 || apdu[0] >> 4 != 1 || apdu[1] != UnconfirmedServiceChoice::IAm as u8 {
        return None;
    }

    let iam = IAmRequest::decode(&apdu[2..]).ok()?;
    if iam.device_identifier.object_type != ObjectType::Device {
        return None;
    }

    Some(BacnetDiscoveredDevice {
        device_instance: iam.device_identifier.instance,
        ip: origin.ip().to_string(),
        port: origin.port(),
        vendor_name: Some(
            get_vendor_name(iam.vendor_identifier)
                .unwrap_or("Unknown")
                .to_string(),
        ),
        model_name: None,
        firmware_version: None,
        object_list_count: None,
    })
}

/// Browse the object list of a specific BACnet device.
pub async fn browse_objects(
    ip: &str,
    port: u16,
    device_instance: u32,
) -> Result<Vec<BacnetObjectInfo>, ProtocolError> {
    let addr = format!("{ip}:{port}")
        .parse::<SocketAddr>()
        .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address: {e}")))?;

    tokio::task::spawn_blocking(move || {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
        let device = ObjectIdentifier::new(ObjectType::Device, device_instance);

        let objects = read_object_list(&socket, addr, device)
            .map_err(|e| match e {
                ProtocolError::Timeout(_) => e,
                other => ProtocolError::ReadFailed(format!("Object list: {other}")),
            })?
            .iter()
            .filter(|id| id.object_type != ObjectType::Device)
            .map(|id| BacnetObjectInfo {
                object_type: crate::helpers::object_type_to_string(&id.object_type),
                object_instance: id.instance,
                object_name: None,
            })
            .collect();

        Ok(objects)
    })
    .await
    .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
}

/// Read standard properties for a single BACnet object.
pub async fn read_object_properties(
    ip: &str,
    port: u16,
    object_type: &str,
    object_instance: u32,
) -> Result<Vec<BacnetObjectDetail>, ProtocolError> {
    let ot = crate::helpers::object_type_from_str(object_type)
        .ok_or_else(|| ProtocolError::InvalidConfig(format!("Unknown object type: {object_type}")))?;

    let addr = format!("{ip}:{port}")
        .parse::<SocketAddr>()
        .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address: {e}")))?;

    tokio::task::spawn_blocking(move || {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;

        let obj_id = ObjectIdentifier::new(ot, object_instance);
        let prop_refs = vec![
            PropertyReference::new(PropertyIdentifier::ObjectName),
            PropertyReference::new(PropertyIdentifier::Description),
            PropertyReference::new(PropertyIdentifier::PresentValue),
            PropertyReference::new(PropertyIdentifier::StatusFlags),
            PropertyReference::new(PropertyIdentifier::Units),
            PropertyReference::new(PropertyIdentifier::Reliability),
            PropertyReference::new(PropertyIdentifier::OutOfService),
            PropertyReference::new(PropertyIdentifier::EventState),
            PropertyReference::new(PropertyIdentifier::NotificationClass),
            PropertyReference::new(PropertyIdentifier::HighLimit),
            PropertyReference::new(PropertyIdentifier::LowLimit),
            PropertyReference::new(PropertyIdentifier::CovIncrement),
        ];

        let access_results = read_access_results(&socket, addr, obj_id, &prop_refs)?;

        let mut details = Vec::new();
        for access_result in &access_results {
            let mut detail = BacnetObjectDetail {
                object_type: crate::helpers::object_type_to_string(&access_result.object_identifier.object_type),
                object_instance: access_result.object_identifier.instance,
                object_name: None,
                description: None,
                present_value: None,
                units: None,
                units_symbol: None,
                status_flags: None,
                reliability: None,
                out_of_service: None,
                properties: Vec::new(),
            };

            for prop_result in &access_result.results {
                let prop_id_u32: u32 = prop_result.property_identifier.into();
                let prop_name = property_id_name(prop_id_u32);
                let raw_value = match &prop_result.value {
                    bacnet_rs::service::PropertyResultValue::Value(values) => {
                        match (prop_result.property_identifier, values.as_slice()) {
                            (PropertyIdentifier::StatusFlags, [PropertyValue::BitString(bits)]) => status_flags_text(bits),
                            (PropertyIdentifier::Reliability, [PropertyValue::Enumerated(n)]) => reliability_text(*n),
                            (PropertyIdentifier::PresentValue, [PropertyValue::Enumerated(n)]) if detail.object_type.starts_with("binary") => {
                                if *n == 0 { "inactive".into() } else { "active".into() }
                            }
                            _ => values
                                .iter()
                                .map(|v| v.as_display_string())
                                .collect::<Vec<_>>()
                                .join(", "),
                        }
                    }
                    bacnet_rs::service::PropertyResultValue::Error(c, code) => {
                        format!("Error({c}, {code})")
                    }
                };
                let read_ok = matches!(prop_result.value, bacnet_rs::service::PropertyResultValue::Value(_));

                let parsed_value = match &prop_result.value {
                    bacnet_rs::service::PropertyResultValue::Value(values) => {
                        extract_bacnet_value(values)
                    }
                    bacnet_rs::service::PropertyResultValue::Error(_, _) => BacnetValue::Null,
                };

                let mut entry = BacnetPropertyEntry {
                    property_name: prop_name.clone(),
                    property_identifier: prop_id_u32,
                    value: parsed_value,
                    raw_value: raw_value.clone(),
                };

                // Map to convenience fields
                match prop_result.property_identifier {
                    PropertyIdentifier::ObjectName => {
                        detail.object_name = Some(raw_value);
                    }
                    PropertyIdentifier::Description => {
                        detail.description = Some(raw_value);
                    }
                    PropertyIdentifier::PresentValue => {
                        detail.present_value = Some(raw_value);
                    }
                    PropertyIdentifier::Units if read_ok => {
                        // Map the BACnet engineering-units enum to a human label
                        // + symbol (e.g. 62 -> "Degrees Celsius" / "°C").
                        let (name, symbol) = match &entry.value {
                            BacnetValue::Enum(n) => {
                                let eu = EngineeringUnits::from(*n);
                                let name = title_case_units(&eu.bacnet_name());
                                let sym = eu.unit_symbol();
                                (name, if sym.is_empty() { None } else { Some(sym.to_string()) })
                            }
                            _ => (raw_value.clone(), None),
                        };
                        entry.raw_value = name.clone();
                        detail.units = Some(name);
                        detail.units_symbol = symbol;
                    }
                    // statusFlags and reliability are optional: a device that lacks them answers with an error, which is
                    // no value to show in the object list or the CSV (the property list keeps the error text).
                    PropertyIdentifier::StatusFlags if read_ok => {
                        detail.status_flags = Some(raw_value);
                    }
                    PropertyIdentifier::Reliability if read_ok => {
                        detail.reliability = Some(raw_value);
                    }
                    PropertyIdentifier::OutOfService => {
                        detail.out_of_service = extract_bool(&prop_result.value);
                    }
                    _ => {}
                }

                detail.properties.push(entry);
            }

            details.push(detail);
        }

        Ok(details)
    })
    .await
    .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
}

/// Inspect a BACnet device by reading its Device Object properties.
pub async fn inspect_device(
    ip: &str,
    port: u16,
    device_instance: u32,
) -> Result<DeviceInspection, ProtocolError> {
    let addr = format!("{ip}:{port}")
        .parse::<SocketAddr>()
        .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address: {e}")))?;

    tokio::task::spawn_blocking(move || {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;

        let device_obj = ObjectIdentifier::new(ObjectType::Device, device_instance);
        let prop_refs = vec![
            PropertyReference::new(PropertyIdentifier::ObjectName),
            PropertyReference::new(PropertyIdentifier::VendorName),
            PropertyReference::new(PropertyIdentifier::VendorIdentifier),
            PropertyReference::new(PropertyIdentifier::ModelName),
            PropertyReference::new(PropertyIdentifier::FirmwareRevision),
            PropertyReference::new(PropertyIdentifier::ApplicationSoftwareVersion),
            PropertyReference::new(PropertyIdentifier::Description),
            PropertyReference::new(PropertyIdentifier::Location),
            PropertyReference::new(PropertyIdentifier::ProtocolVersion),
            PropertyReference::new(PropertyIdentifier::ProtocolRevision),
            PropertyReference::new(PropertyIdentifier::ProtocolServicesSupported),
            PropertyReference::new(PropertyIdentifier::ProtocolObjectTypesSupported),
            PropertyReference::new(PropertyIdentifier::SystemStatus),
            PropertyReference::new(PropertyIdentifier::MaxApduLengthAccepted),
            PropertyReference::new(PropertyIdentifier::SegmentationSupported),
            PropertyReference::new(PropertyIdentifier::ApduTimeout),
            PropertyReference::new(PropertyIdentifier::NumberOfApduRetries),
            PropertyReference::new(PropertyIdentifier::LocalTime),
            PropertyReference::new(PropertyIdentifier::LocalDate),
            PropertyReference::new(PropertyIdentifier::DatabaseRevision),
            // Array index 0 of Object_List is its element count (clause 12.11.34).
            PropertyReference::with_array_index(PropertyIdentifier::ObjectList, 0),
        ];

        let access_results = read_access_results(&socket, addr, device_obj, &prop_refs)?;

        let mut inspection = DeviceInspection {
            device_instance,
            object_name: None,
            vendor_name: None,
            vendor_id: None,
            model_name: None,
            firmware_revision: None,
            application_software_version: None,
            description: None,
            location: None,
            protocol_version: None,
            protocol_revision: None,
            max_apdu_length: None,
            segmentation_supported: None,
            apdu_timeout: None,
            number_of_apdu_retries: None,
            device_address_binding: None,
            object_count: None,
            system_status: None,
            local_time: None,
            local_date: None,
            database_revision: None,
            services_supported: None,
            object_types_supported: None,
        };

        for access_result in &access_results {
            for prop_result in &access_result.results {
                // A property the device cannot provide (e.g. Protocol_Revision on a
                // pre-1.2 device) is left unset rather than shown as an error string.
                let PropertyResultValue::Value(values) = &prop_result.value else {
                    continue;
                };
                let raw = values
                    .iter()
                    .map(|v| v.as_display_string())
                    .collect::<Vec<_>>()
                    .join(", ");

                match prop_result.property_identifier {
                    PropertyIdentifier::ObjectName => inspection.object_name = Some(raw),
                    PropertyIdentifier::VendorName => inspection.vendor_name = Some(raw),
                    PropertyIdentifier::VendorIdentifier => {
                        inspection.vendor_id = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::ModelName => inspection.model_name = Some(raw),
                    PropertyIdentifier::FirmwareRevision => {
                        inspection.firmware_revision = Some(raw);
                    }
                    PropertyIdentifier::ApplicationSoftwareVersion => {
                        inspection.application_software_version = Some(raw);
                    }
                    PropertyIdentifier::Description => inspection.description = Some(raw),
                    PropertyIdentifier::Location => inspection.location = Some(raw),
                    PropertyIdentifier::ProtocolVersion => {
                        inspection.protocol_version = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::ProtocolRevision => {
                        inspection.protocol_revision = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::ProtocolServicesSupported => {
                        inspection.services_supported = Some(raw);
                    }
                    PropertyIdentifier::ProtocolObjectTypesSupported => {
                        inspection.object_types_supported = Some(raw);
                    }
                    PropertyIdentifier::SystemStatus => inspection.system_status = Some(raw),
                    PropertyIdentifier::MaxApduLengthAccepted => {
                        inspection.max_apdu_length = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::SegmentationSupported => {
                        inspection.segmentation_supported = Some(raw);
                    }
                    PropertyIdentifier::ApduTimeout => {
                        inspection.apdu_timeout = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::NumberOfApduRetries => {
                        inspection.number_of_apdu_retries = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::LocalTime => inspection.local_time = Some(raw),
                    PropertyIdentifier::LocalDate => inspection.local_date = Some(raw),
                    PropertyIdentifier::DatabaseRevision => {
                        inspection.database_revision = extract_u32(&prop_result.value);
                    }
                    PropertyIdentifier::ObjectList if prop_result.array_index == Some(0) => {
                        inspection.object_count = extract_u32(&prop_result.value);
                    }
                    _ => {}
                }
            }
        }

        Ok(inspection)
    })
    .await
    .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
}

// ---------------------------------------------------------------------------
// Helpers for extracting typed values from PropertyResultValue
// ---------------------------------------------------------------------------

/// Extract the first value as a [`BacnetValue`]. Signed integers and unsigned
/// integers wider than 32 bits become `Double` (there is no signed variant, and
/// wrapping a negative number into `Unsigned` would be wrong).
fn extract_bacnet_value(values: &[PropertyValue]) -> BacnetValue {
    match values.first() {
        Some(PropertyValue::Real(f)) => BacnetValue::Real(*f),
        Some(PropertyValue::Double(f)) => BacnetValue::Double(*f),
        Some(PropertyValue::Unsigned(u)) => match u32::try_from(*u) {
            Ok(v) => BacnetValue::Unsigned(v),
            Err(_) => BacnetValue::Double(*u as f64),
        },
        Some(PropertyValue::Signed(i)) => BacnetValue::Double(*i as f64),
        Some(PropertyValue::Boolean(b)) => BacnetValue::Boolean(*b),
        Some(PropertyValue::Enumerated(e)) => BacnetValue::Enum(*e),
        Some(PropertyValue::CharacterString(s)) => BacnetValue::String(s.clone()),
        _ => BacnetValue::Null,
    }
}

/// Extract a u32 from the first value if it is numeric/enumerated and fits.
fn extract_u32(value: &PropertyResultValue) -> Option<u32> {
    let PropertyResultValue::Value(values) = value else {
        return None;
    };
    match values.first()? {
        PropertyValue::Unsigned(u) => u32::try_from(*u).ok(),
        PropertyValue::Enumerated(e) => Some(*e),
        PropertyValue::Signed(i) => u32::try_from(*i).ok(),
        PropertyValue::Real(f) if *f >= 0.0 && *f <= u32::MAX as f32 => Some(*f as u32),
        _ => None,
    }
}

/// Extract a bool from the first value if it is Boolean.
fn extract_bool(value: &bacnet_rs::service::PropertyResultValue) -> Option<bool> {
    match value {
        bacnet_rs::service::PropertyResultValue::Value(values) => {
            if let Some(PropertyValue::Boolean(b)) = values.first() {
                Some(*b)
            } else {
                None
            }
        }
        bacnet_rs::service::PropertyResultValue::Error(_, _) => None,
    }
}

/// Status flags as words: `normal`, or the set bits (BACnetStatusFlags: in-alarm, fault, overridden, out-of-service).
fn status_flags_text(bits: &[bool]) -> String {
    const NAMES: [&str; 4] = ["in-alarm", "fault", "overridden", "out-of-service"];
    let set: Vec<&str> = NAMES.iter().zip(bits).filter(|(_, on)| **on).map(|(name, _)| *name).collect();
    if set.is_empty() { "normal".into() } else { set.join(", ") }
}

/// BACnetReliability as a word; unknown (vendor) values keep their number.
fn reliability_text(value: u32) -> String {
    const NAMES: [&str; 25] = [
        "no-fault-detected", "no-sensor", "over-range", "under-range", "open-loop", "shorted-loop", "no-output",
        "unreliable-other", "process-error", "multi-state-fault", "configuration-error", "", "communication-failure",
        "member-fault", "monitored-object-fault", "tripped", "lamp-failure", "activation-failed", "renew-dhcp-failure",
        "renew-fd-registration-failure", "restart-auto-negotiation-failure", "restart-failure",
        "proprietary-command-failure", "faults-listed", "referenced-object-fault",
    ];
    match NAMES.get(value as usize) {
        Some(name) if !name.is_empty() => (*name).into(),
        _ => format!("reliability {value}"),
    }
}

/// Title-case a BACnet engineering-unit bacnet_name, e.g. "degrees-celsius" -> "Degrees Celsius".
fn title_case_units(s: &str) -> String {
    s.split(|c: char| c == '-' || c == '_')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a BACnet property identifier number to a human-readable name.
fn property_id_name(id: u32) -> String {
    match id {
        77 => "objectName".into(),
        28 => "description".into(),
        85 => "presentValue".into(),
        111 => "statusFlags".into(),
        117 => "units".into(),
        103 => "reliability".into(),
        81 => "outOfService".into(),
        36 => "eventState".into(),
        17 => "notificationClass".into(),
        45 => "highLimit".into(),
        59 => "lowLimit".into(),
        22 => "covIncrement".into(),
        _ => format!("property-{id}"),
    }
}

// ---------------------------------------------------------------------------
// Confirmed reads: ReadProperty / ReadPropertyMultiple with fallbacks
// ---------------------------------------------------------------------------

/// Read the Object_List of `device`.
///
/// The whole array is requested first. A device that cannot segment its reply
/// aborts (segmentation-not-supported / buffer-overflow) or errors when the list
/// does not fit one APDU; the list is then read element by element: array index 0
/// is the element count (clause 12.11.34), indices 1..=N are the identifiers.
fn read_object_list(
    socket: &UdpSocket,
    addr: SocketAddr,
    device: ObjectIdentifier,
) -> Result<Vec<ObjectIdentifier>, ProtocolError> {
    match read_property(socket, addr, device, PropertyIdentifier::ObjectList, None) {
        Ok(ack) => return Ok(object_identifiers(&ack.values)),
        Err(e @ ProtocolError::Timeout(_)) => return Err(e),
        Err(_) => {}
    }

    let count_ack = read_property(socket, addr, device, PropertyIdentifier::ObjectList, Some(0))?;
    let Some(PropertyValue::Unsigned(count)) = count_ack.values.first() else {
        return Err(ProtocolError::ReadFailed(
            "Object_List[0] is not an Unsigned element count".into(),
        ));
    };

    let mut ids = Vec::new();
    for index in 1..=*count {
        let index = u32::try_from(index)
            .map_err(|_| ProtocolError::ReadFailed(format!("Object_List length {count} exceeds the array index range")))?;
        let ack = read_property(socket, addr, device, PropertyIdentifier::ObjectList, Some(index))?;
        ids.extend(object_identifiers(&ack.values));
    }
    Ok(ids)
}

fn object_identifiers(values: &[PropertyValue]) -> Vec<ObjectIdentifier> {
    values
        .iter()
        .filter_map(|v| match v {
            PropertyValue::ObjectIdentifier(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// Decoded ReadProperty-ACK (clause 15.5.2).
#[derive(Debug)]
pub(crate) struct ReadPropertyAck {
    pub(crate) object: ObjectIdentifier,
    pub(crate) property: u32,
    pub(crate) values: Vec<PropertyValue>,
}

fn read_property_request(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<Vec<u8>, ProtocolError> {
    let mut data = Vec::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: array_index,
    }
    .encode(&mut data)
    .map_err(|e| ProtocolError::ReadFailed(format!("ReadProperty encode: {e}")))?;
    Ok(data)
}

fn read_property(
    socket: &UdpSocket,
    addr: SocketAddr,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<ReadPropertyAck, ProtocolError> {
    let reply = confirmed_request(
        socket,
        addr,
        ConfirmedServiceChoice::ReadProperty,
        read_property_request(object, property, array_index)?,
        REQUEST_TIMEOUT,
    )?;
    let ack = decode_read_property_ack(&reply).ok_or_else(|| {
        ProtocolError::ReadFailed("ReadProperty ACK could not be decoded".into())
    })?;
    if ack.object != object || ack.property != u32::from(property) {
        return Err(ProtocolError::ReadFailed(
            "ReadProperty ACK answers a different object or property".into(),
        ));
    }
    Ok(ack)
}

fn rpm_request(
    object: ObjectIdentifier,
    refs: &[PropertyReference],
) -> Result<Vec<u8>, ProtocolError> {
    let mut data = Vec::new();
    ReadPropertyMultipleRequest::new(vec![ReadAccessSpecification::new(object, refs.to_vec())])
        .encode(&mut data)
        .map_err(|e| ProtocolError::ReadFailed(format!("RPM encode: {e}")))?;
    Ok(data)
}

/// Read `refs` of one object with ReadPropertyMultiple.
///
/// When the device answers with an Error, Reject or Abort (no RPM support, or a
/// reply too large for a device that cannot segment), every property is read with
/// its own ReadProperty instead; properties the device refuses are omitted. Only
/// when nothing could be read is the original RPM failure returned. A timeout is
/// returned immediately.
fn read_access_results(
    socket: &UdpSocket,
    addr: SocketAddr,
    object: ObjectIdentifier,
    refs: &[PropertyReference],
) -> Result<Vec<ReadAccessResult>, ProtocolError> {
    let rpm_attempt = confirmed_request(
        socket,
        addr,
        ConfirmedServiceChoice::ReadPropertyMultiple,
        rpm_request(object, refs)?,
        REQUEST_TIMEOUT,
    )
    .and_then(|reply| {
        decode_rpm_ack(&reply)
            .ok_or_else(|| ProtocolError::ReadFailed("RPM response could not be decoded".into()))
    });
    let rpm_error = match rpm_attempt {
        Ok(results) => return Ok(results),
        Err(e @ ProtocolError::Timeout(_)) => return Err(e),
        Err(e) => e,
    };
    tracing::warn!("ReadPropertyMultiple of {object:?} failed ({rpm_error}); reading properties one by one");

    let mut results = Vec::with_capacity(refs.len());
    for r in refs {
        match read_property(socket, addr, object, r.property_identifier, r.property_array_index) {
            Ok(ack) => results.push(PropertyResult {
                property_identifier: r.property_identifier,
                array_index: r.property_array_index,
                value: PropertyResultValue::Value(ack.values),
            }),
            Err(e @ ProtocolError::Timeout(_)) => return Err(e),
            Err(_) => {}
        }
    }
    if results.is_empty() {
        return Err(rpm_error);
    }
    Ok(vec![ReadAccessResult {
        object_identifier: object,
        results,
    }])
}

// ---------------------------------------------------------------------------
// ASN.1 (clause 20.2) decoding of ReadProperty / ReadPropertyMultiple ACKs.
//
// bacnet-rs 0.3 rejects a whole RPM-ACK when any character string is not valid
// UTF-8 (ISO 8859-1 / UCS-2 names are common on real devices) or when a value is
// constructed, so the ACKs are decoded here.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagClass {
    Application,
    Context,
    Opening,
    Closing,
}

#[derive(Debug, Clone, Copy)]
struct Tag {
    class: TagClass,
    number: u32,
    /// Content length in octets. For an application Boolean this is the value
    /// (0/1) and there is no content.
    len: usize,
    /// Octets taken by the tag header (initial octet plus extensions).
    header: usize,
}

/// Decode one tag header (clause 20.2.1): extended tag numbers (initial nibble
/// 0xF) and extended lengths (length field 5 followed by 1, 3 or 5 octets).
fn read_tag(data: &[u8]) -> Option<Tag> {
    let first = *data.first()?;
    let mut header = 1;
    let mut number = u32::from(first >> 4);
    if number == 0x0F {
        number = u32::from(*data.get(1)?);
        header = 2;
    }
    let lvt = usize::from(first & 0x07);
    let class = match (first & 0x08 != 0, lvt) {
        (false, _) => TagClass::Application,
        (true, 6) => TagClass::Opening,
        (true, 7) => TagClass::Closing,
        (true, _) => TagClass::Context,
    };
    let len = match class {
        TagClass::Opening | TagClass::Closing => 0,
        _ if lvt == 5 => {
            let marker = *data.get(header)?;
            header += 1;
            match marker {
                254 => {
                    let b = data.get(header..header + 2)?;
                    header += 2;
                    usize::from(u16::from_be_bytes([b[0], b[1]]))
                }
                255 => {
                    let b = data.get(header..header + 4)?;
                    header += 4;
                    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
                }
                n => usize::from(n),
            }
        }
        _ => lvt,
    };
    Some(Tag {
        class,
        number,
        len,
        header,
    })
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn peek(&self) -> Option<Tag> {
        read_tag(&self.data[self.pos..])
    }

    fn tag(&mut self) -> Option<Tag> {
        let tag = self.peek()?;
        self.pos += tag.header;
        Some(tag)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let data: &'a [u8] = self.data;
        let end = self.pos.checked_add(n)?;
        let slice = data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Consume a context tag numbered `number` and return its content.
    fn context(&mut self, number: u32) -> Option<&'a [u8]> {
        let tag = self.tag()?;
        if tag.class != TagClass::Context || tag.number != number {
            return None;
        }
        self.take(tag.len)
    }

    fn context_unsigned(&mut self, number: u32) -> Option<u32> {
        u32::try_from(unsigned_be(self.context(number)?)?).ok()
    }

    /// `None` if malformed, `Some(None)` if the optional element is absent.
    fn optional_context_unsigned(&mut self, number: u32) -> Option<Option<u32>> {
        match self.peek() {
            Some(t) if t.class == TagClass::Context && t.number == number => {
                self.context_unsigned(number).map(Some)
            }
            _ => Some(None),
        }
    }

    fn context_object_id(&mut self, number: u32) -> Option<ObjectIdentifier> {
        let raw = <[u8; 4]>::try_from(self.context(number)?).ok()?;
        Some(ObjectIdentifier::from(u32::from_be_bytes(raw)))
    }

    fn opening(&mut self, number: u32) -> Option<()> {
        let tag = self.tag()?;
        (tag.class == TagClass::Opening && tag.number == number).then_some(())
    }

    fn closing(&mut self, number: u32) -> Option<()> {
        let tag = self.tag()?;
        (tag.class == TagClass::Closing && tag.number == number).then_some(())
    }

    /// Application-tagged Enumerated (or Unsigned) value.
    fn enumerated(&mut self) -> Option<u32> {
        let tag = self.tag()?;
        if tag.class != TagClass::Application || !matches!(tag.number, 2 | 9) {
            return None;
        }
        u32::try_from(unsigned_be(self.take(tag.len)?)?).ok()
    }
}

fn unsigned_be(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 8 {
        return None;
    }
    Some(bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
}

/// Two's-complement big-endian signed integer of 1 to 8 octets.
fn signed_be(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() || bytes.len() > 8 {
        return None;
    }
    let fill = if bytes[0] & 0x80 != 0 { 0xFF } else { 0x00 };
    let mut buf = [fill; 8];
    buf[8 - bytes.len()..].copy_from_slice(bytes);
    Some(i64::from_be_bytes(buf))
}

/// BACnetCharacterString (clause 20.2.9): the first octet selects the character
/// set. 3 = UCS-4, 4 = UCS-2 (both big-endian), 5 = ISO 8859-1; 0 (UTF-8 /
/// ANSI X3.4) and the double-byte sets 1 and 2 are decoded as UTF-8, lossily.
fn decode_character_string(content: &[u8]) -> Option<String> {
    let (&charset, text) = content.split_first()?;
    Some(match charset {
        3 => text
            .chunks_exact(4)
            .map(|c| char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]])).unwrap_or('\u{FFFD}'))
            .collect(),
        4 => String::from_utf16_lossy(
            &text
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ),
        5 => text.iter().map(|&b| char::from(b)).collect(),
        _ => String::from_utf8_lossy(text).into_owned(),
    })
}

/// BACnetBitString (clause 20.2.10): first octet = unused bits in the last octet.
fn decode_bit_string(content: &[u8]) -> Option<Vec<bool>> {
    let (&unused, bytes) = content.split_first()?;
    if unused > 7 || (bytes.is_empty() && unused != 0) {
        return None;
    }
    let total = bytes.len() * 8 - usize::from(unused);
    Some((0..total).map(|i| bytes[i / 8] & (0x80 >> (i % 8)) != 0).collect())
}

/// Decode one application-tagged value of tag `number` (clause 20.2).
fn app_value(number: u32, len: usize, content: &[u8]) -> Option<PropertyValue> {
    Some(match number {
        0 => PropertyValue::Null,
        1 => PropertyValue::Boolean(len != 0),
        2 => PropertyValue::Unsigned(unsigned_be(content)?),
        3 => PropertyValue::Signed(signed_be(content)?),
        4 => PropertyValue::Real(f32::from_be_bytes(content.try_into().ok()?)),
        5 => PropertyValue::Double(f64::from_be_bytes(content.try_into().ok()?)),
        6 => PropertyValue::OctetString(content.to_vec()),
        7 => PropertyValue::CharacterString(decode_character_string(content)?),
        8 => PropertyValue::BitString(decode_bit_string(content)?),
        9 => PropertyValue::Enumerated(u32::try_from(unsigned_be(content)?).ok()?),
        10 => {
            let [year, month, day, weekday] = <[u8; 4]>::try_from(content).ok()?;
            let year = if year == 0xFF { 0xFF } else { 1900 + u16::from(year) };
            PropertyValue::Date(year, month, day, weekday)
        }
        11 => {
            let [hour, minute, second, hundredths] = <[u8; 4]>::try_from(content).ok()?;
            PropertyValue::Time(hour, minute, second, hundredths)
        }
        12 => PropertyValue::ObjectIdentifier(ObjectIdentifier::from(u32::from_be_bytes(
            content.try_into().ok()?,
        ))),
        _ => PropertyValue::Unknown(content.to_vec()),
    })
}

/// Decode property values up to the closing tag `closing` (the opening tag has
/// been consumed). Constructed data inside the value is flattened: opening and
/// closing tags are dropped, application values are decoded, and context-tagged
/// primitives (whose type needs the property's schema) are kept as raw bytes.
fn decode_values(r: &mut Reader<'_>, closing: u32) -> Option<Vec<PropertyValue>> {
    let mut values = Vec::new();
    let mut depth = 0u32;
    loop {
        let tag = r.tag()?;
        match tag.class {
            TagClass::Closing => {
                if depth == 0 {
                    return (tag.number == closing).then_some(values);
                }
                depth -= 1;
            }
            TagClass::Opening => depth += 1,
            TagClass::Application => {
                let content: &[u8] = if tag.number == 1 { &[] } else { r.take(tag.len)? };
                values.push(app_value(tag.number, tag.len, content)?);
            }
            TagClass::Context => values.push(PropertyValue::Unknown(r.take(tag.len)?.to_vec())),
        }
    }
}

/// Decode ReadPropertyMultiple-ACK service data (clause 15.7.2):
/// `[0] objectIdentifier, [1] { [2] property, [3] index?, [4] { value } | [5] { class, code } }...`
pub(crate) fn decode_rpm_ack(data: &[u8]) -> Option<Vec<ReadAccessResult>> {
    let mut r = Reader::new(data);
    let mut out = Vec::new();
    while !r.is_empty() {
        let object = r.context_object_id(0)?;
        r.opening(1)?;
        let mut results = Vec::new();
        loop {
            if let Some(t) = r.peek() {
                if t.class == TagClass::Closing && t.number == 1 {
                    r.tag()?;
                    break;
                }
            }
            let property = r.context_unsigned(2)?;
            let array_index = r.optional_context_unsigned(3)?;
            let value = match r.tag()? {
                Tag {
                    class: TagClass::Opening,
                    number: 4,
                    ..
                } => PropertyResultValue::Value(decode_values(&mut r, 4)?),
                Tag {
                    class: TagClass::Opening,
                    number: 5,
                    ..
                } => {
                    let class = r.enumerated()?;
                    let code = r.enumerated()?;
                    r.closing(5)?;
                    PropertyResultValue::Error(class, code)
                }
                _ => return None,
            };
            results.push(PropertyResult {
                property_identifier: PropertyIdentifier::from(property),
                array_index,
                value,
            });
        }
        out.push(ReadAccessResult {
            object_identifier: object,
            results,
        });
    }
    Some(out)
}

/// Decode ReadProperty-ACK service data (clause 15.5.2):
/// `[0] objectIdentifier, [1] property, [2] index?, [3] { value }`.
pub(crate) fn decode_read_property_ack(data: &[u8]) -> Option<ReadPropertyAck> {
    let mut r = Reader::new(data);
    let object = r.context_object_id(0)?;
    let property = r.context_unsigned(1)?;
    r.optional_context_unsigned(2)?;
    r.opening(3)?;
    let values = decode_values(&mut r, 3)?;
    Some(ReadPropertyAck {
        object,
        property,
        values,
    })
}

// Byte vectors below are derived by hand from the ASHRAE 135 encodings cited on
// each test (clause 6.2 NPDU, clause 16.10 Who-Is/I-Am, clause 20.2 ASN.1,
// clause 15.5.2 ReadProperty-ACK, clause 15.7.2 ReadPropertyMultiple-ACK,
// Annex J BVLC); the Who-Is / I-Am frames match the layout Wireshark's BACnet
// dissector shows for bacnet-stack's `bacwi` / `bacrp` exchanges.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{apdu_of_frame, unicast_frame};

    #[test]
    fn status_flags_read_as_words() {
        assert_eq!(status_flags_text(&[false, false, false, false]), "normal");
        assert_eq!(status_flags_text(&[true, false, false, true]), "in-alarm, out-of-service");
        assert_eq!(status_flags_text(&[false, true, true, false]), "fault, overridden");
    }

    #[test]
    fn reliability_reads_as_a_word_and_keeps_vendor_numbers() {
        assert_eq!(reliability_text(0), "no-fault-detected");
        assert_eq!(reliability_text(12), "communication-failure");
        assert_eq!(reliability_text(11), "reliability 11");
        assert_eq!(reliability_text(300), "reliability 300");
    }

    /// BVLC header (type 0x81, function, total length) + body.
    fn bvlc(function: u8, body: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x81, function];
        frame.extend_from_slice(&((body.len() + 4) as u16).to_be_bytes());
        frame.extend_from_slice(body);
        frame
    }

    /// I-Am APDU: Unconfirmed-Request (0x10), service 0, Device 8 (C4 02 00 00 08),
    /// max-APDU 1476 (22 05 C4), segmentation 3 = none (91 03), vendor 5 (21 05).
    const IAM_APDU: [u8; 14] = [
        0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x08, 0x22, 0x05, 0xC4, 0x91, 0x03, 0x21, 0x05,
    ];

    /// NPDU of a global broadcast: version 1, control 0x20 (DNET present),
    /// DNET 0xFFFF, DLEN 0, hop count 255.
    const NPDU_GLOBAL_BROADCAST: [u8; 6] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF];

    fn udp_source() -> SocketAddr {
        "192.168.1.50:47808".parse().unwrap()
    }

    // ---- Who-Is (clause 16.10.1, clause 6.2.2, Annex J.2.12) ----

    #[test]
    fn who_is_global_broadcast_frame() {
        // 81 0B 00 0C | 01 20 FF FF 00 FF | 10 08  (bacnet-stack bacwi default frame)
        let frame = who_is_frame(&WhoIsRequest::new()).unwrap();
        assert_eq!(
            frame,
            [0x81, 0x0B, 0x00, 0x0C, 0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08]
        );
    }

    #[test]
    fn who_is_with_device_range_uses_context_tags_0_and_1() {
        // Range 1..=3: [0] unsigned 1 = 09 01, [1] unsigned 3 = 19 03.
        let frame = who_is_frame(&WhoIsRequest::for_range(1, 3)).unwrap();
        assert_eq!(
            frame,
            [
                0x81, 0x0B, 0x00, 0x10, 0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08, 0x09,
                0x01, 0x19, 0x03
            ]
        );
        // Full range 0..=4194303: [0] 09 00, [1] 3-octet unsigned 3F FF FF = 1B 3F FF FF.
        let frame = who_is_frame(&WhoIsRequest::for_range(0, 0x3F_FFFF)).unwrap();
        assert_eq!(
            &frame[12..],
            [0x09, 0x00, 0x1B, 0x3F, 0xFF, 0xFF]
        );
        assert_eq!(&frame[2..4], [0x00, 0x12]);
    }

    #[test]
    fn who_is_limits_are_both_or_neither() {
        // A lone low limit is not a valid Who-Is (clause 16.10.1): nothing is encoded.
        let lone_low = WhoIsRequest {
            device_instance_range_low_limit: Some(5),
            device_instance_range_high_limit: None,
        };
        assert_eq!(
            who_is_frame(&lone_low).unwrap(),
            who_is_frame(&WhoIsRequest::new()).unwrap()
        );
    }

    #[test]
    fn directed_broadcast_comes_from_the_netmask() {
        let b = |ip: [u8; 4], mask: [u8; 4]| directed_broadcast(ip.into(), mask.into());
        assert_eq!(
            b([192, 168, 1, 37], [255, 255, 255, 0]),
            Some(Ipv4Addr::new(192, 168, 1, 255))
        );
        assert_eq!(
            b([10, 1, 2, 3], [255, 255, 0, 0]),
            Some(Ipv4Addr::new(10, 1, 255, 255))
        );
        assert_eq!(
            b([172, 16, 5, 9], [255, 255, 255, 252]),
            Some(Ipv4Addr::new(172, 16, 5, 11))
        );
        assert_eq!(b([10, 0, 0, 1], [255, 255, 255, 255]), None);
        assert_eq!(b([10, 0, 0, 1], [255, 255, 255, 254]), None);
        assert_eq!(b([127, 0, 0, 1], [255, 0, 0, 0]), None);
    }

    // ---- I-Am (clause 16.10.2) ----

    #[test]
    fn iam_in_global_broadcast_npdu() {
        let mut body = NPDU_GLOBAL_BROADCAST.to_vec();
        body.extend_from_slice(&IAM_APDU);
        let frame = bvlc(BVLC_ORIGINAL_BROADCAST, &body);
        assert_eq!(&frame[..4], [0x81, 0x0B, 0x00, 0x18]);

        let device = parse_iam_response(&frame, udp_source()).unwrap();
        assert_eq!(device.device_instance, 8);
        assert_eq!(device.ip, "192.168.1.50");
        assert_eq!(device.port, 47808);
        assert!(device.vendor_name.unwrap().contains("Johnson"));
    }

    #[test]
    fn iam_in_unicast_npdu() {
        let mut body = vec![0x01, 0x00];
        body.extend_from_slice(&IAM_APDU);
        let device =
            parse_iam_response(&bvlc(BVLC_ORIGINAL_UNICAST, &body), udp_source()).unwrap();
        assert_eq!(device.device_instance, 8);
        assert_eq!(device.ip, "192.168.1.50");
    }

    #[test]
    fn iam_multi_octet_fields() {
        // Device 4194302 = 0x023FFFFE (C4 02 3F FF FE), max-APDU 480 (22 01 E0),
        // segmentation 0 (91 00), vendor 260 (22 01 04).
        let apdu = [
            0x10, 0x00, 0xC4, 0x02, 0x3F, 0xFF, 0xFE, 0x22, 0x01, 0xE0, 0x91, 0x00, 0x22, 0x01,
            0x04,
        ];
        let mut body = vec![0x01, 0x00];
        body.extend_from_slice(&apdu);
        let device =
            parse_iam_response(&bvlc(BVLC_ORIGINAL_UNICAST, &body), udp_source()).unwrap();
        assert_eq!(device.device_instance, 4_194_302);
    }

    #[test]
    fn iam_forwarded_by_bbmd_reports_the_originating_device_address() {
        // Annex J.2.5: BVLC 81 04, 6-octet B/IP origin (192.168.1.100:47808 =
        // C0 A8 01 64 BA C0), then NPDU 01 00 and the I-Am. The UDP source is the BBMD.
        let mut body = vec![0xC0, 0xA8, 0x01, 0x64, 0xBA, 0xC0, 0x01, 0x00];
        body.extend_from_slice(&IAM_APDU);
        let bbmd: SocketAddr = "10.0.0.1:47808".parse().unwrap();
        let device = parse_iam_response(&bvlc(BVLC_FORWARDED_NPDU, &body), bbmd).unwrap();
        assert_eq!(device.device_instance, 8);
        assert_eq!(device.ip, "192.168.1.100");
        assert_eq!(device.port, 47808);
    }

    #[test]
    fn iam_from_a_device_behind_a_router_is_not_dropped() {
        // Router-forwarded global-broadcast I-Am (clause 6.2.2): control 0x28 =
        // DNET + SNET present; DNET FFFF DLEN 0; SNET 5, SLEN 1, SADR 07; hop count 255.
        let mut body = vec![0x01, 0x28, 0xFF, 0xFF, 0x00, 0x00, 0x05, 0x01, 0x07, 0xFF];
        body.extend_from_slice(&IAM_APDU);
        let router: SocketAddr = "192.168.1.1:47808".parse().unwrap();
        let device = parse_iam_response(&bvlc(BVLC_ORIGINAL_BROADCAST, &body), router).unwrap();
        assert_eq!(device.device_instance, 8);
        assert_eq!(device.ip, "192.168.1.1");
    }

    #[test]
    fn frames_that_are_not_an_iam_are_ignored() {
        // Who-Is looped back from our own broadcast.
        let who_is = who_is_frame(&WhoIsRequest::new()).unwrap();
        assert!(parse_iam_response(&who_is, udp_source()).is_none());

        // Network-layer message (control bit 7): Who-Is-Router-To-Network.
        let net_msg = bvlc(BVLC_ORIGINAL_BROADCAST, &[0x01, 0x80, 0x00]);
        assert!(parse_iam_response(&net_msg, udp_source()).is_none());

        // BVLC-Result (function 0x00) is not an NPDU carrier.
        assert!(parse_iam_response(&[0x81, 0x00, 0x00, 0x06, 0x00, 0x00], udp_source()).is_none());

        // BVLC length that disagrees with the datagram.
        let mut body = vec![0x01, 0x00];
        body.extend_from_slice(&IAM_APDU);
        let mut frame = bvlc(BVLC_ORIGINAL_UNICAST, &body);
        frame[3] += 1;
        assert!(parse_iam_response(&frame, udp_source()).is_none());

        // I-Am whose identifier is not a Device object (Analog Input 8).
        let mut apdu = IAM_APDU;
        apdu[3] = 0x00;
        let mut body = vec![0x01, 0x00];
        body.extend_from_slice(&apdu);
        assert!(parse_iam_response(&bvlc(BVLC_ORIGINAL_UNICAST, &body), udp_source()).is_none());
    }

    // ---- ASN.1 application values (clause 20.2) ----

    /// Decode `bytes` followed by the closing tag [3].
    fn decode(bytes: &[u8]) -> Option<Vec<PropertyValue>> {
        let mut data = bytes.to_vec();
        data.push(0x3F);
        decode_values(&mut Reader::new(&data), 3)
    }

    fn one(bytes: &[u8]) -> PropertyValue {
        let mut values = decode(bytes).expect("decodes");
        assert_eq!(values.len(), 1);
        values.remove(0)
    }

    #[test]
    fn primitive_application_values() {
        assert_eq!(one(&[0x00]), PropertyValue::Null);
        assert_eq!(one(&[0x11]), PropertyValue::Boolean(true));
        assert_eq!(one(&[0x10]), PropertyValue::Boolean(false));
        assert_eq!(one(&[0x21, 0x00]), PropertyValue::Unsigned(0));
        assert_eq!(one(&[0x22, 0x05, 0xC4]), PropertyValue::Unsigned(1476));
        // 5-octet unsigned: length field 5 is the extended-length marker (25 05 ...).
        assert_eq!(
            one(&[0x25, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00]),
            PropertyValue::Unsigned(4_294_967_296)
        );
        assert_eq!(one(&[0x31, 0xFB]), PropertyValue::Signed(-5));
        assert_eq!(one(&[0x32, 0xFC, 0x18]), PropertyValue::Signed(-1000));
        assert_eq!(one(&[0x31, 0x7F]), PropertyValue::Signed(127));
        assert_eq!(
            one(&[0x34, 0x80, 0x00, 0x00, 0x00]),
            PropertyValue::Signed(i64::from(i32::MIN))
        );
        // 0x42910000 = 72.5
        assert_eq!(one(&[0x44, 0x42, 0x91, 0x00, 0x00]), PropertyValue::Real(72.5));
        // 0x4059000000000000 = 100.0
        assert_eq!(
            one(&[0x55, 0x08, 0x40, 0x59, 0, 0, 0, 0, 0, 0]),
            PropertyValue::Double(100.0)
        );
        assert_eq!(one(&[0x91, 0x3E]), PropertyValue::Enumerated(62));
        assert_eq!(
            one(&[0x63, 0x01, 0x02, 0x03]),
            PropertyValue::OctetString(vec![1, 2, 3])
        );
    }

    #[test]
    fn extended_length_octet_string() {
        // Tag 6, length field 5, marker 254, 16-bit length 300 (clause 20.2.1.3.1).
        let mut bytes = vec![0x65, 0xFE, 0x01, 0x2C];
        bytes.extend(std::iter::repeat(0xAB).take(300));
        match one(&bytes) {
            PropertyValue::OctetString(v) => assert_eq!(v.len(), 300),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn character_strings_honour_the_charset_octet() {
        // charset 0 (UTF-8/ANSI X3.4), extended length 10: "Zone Temp".
        let mut bytes = vec![0x75, 0x0A, 0x00];
        bytes.extend_from_slice(b"Zone Temp");
        assert_eq!(one(&bytes), PropertyValue::CharacterString("Zone Temp".into()));
        // short form, length 3: charset 0 + "AB".
        assert_eq!(
            one(&[0x73, 0x00, 0x41, 0x42]),
            PropertyValue::CharacterString("AB".into())
        );
        // charset 5 (ISO 8859-1): 0xB0 is the degree sign, invalid as UTF-8.
        assert_eq!(
            one(&[0x75, 0x05, 0x05, 0x32, 0x31, 0xB0, 0x43]),
            PropertyValue::CharacterString("21\u{B0}C".into())
        );
        // charset 4 (UCS-2 big-endian): "Hi".
        assert_eq!(
            one(&[0x75, 0x05, 0x04, 0x00, 0x48, 0x00, 0x69]),
            PropertyValue::CharacterString("Hi".into())
        );
        // charset 3 (UCS-4 big-endian): "A".
        assert_eq!(
            one(&[0x75, 0x05, 0x03, 0x00, 0x00, 0x00, 0x41]),
            PropertyValue::CharacterString("A".into())
        );
    }

    #[test]
    fn bit_strings_drop_the_unused_bits() {
        // 4 used bits, unused 4: in-alarm + fault = 1100.
        assert_eq!(
            one(&[0x82, 0x04, 0xC0]),
            PropertyValue::BitString(vec![true, true, false, false])
        );
        // 13 used bits over two octets, unused 3: A5 80 -> 10100101 10000.
        let mut expected = vec![true, false, true, false, false, true, false, true];
        expected.extend([true, false, false, false, false]);
        assert_eq!(one(&[0x83, 0x03, 0xA5, 0x80]), PropertyValue::BitString(expected));
    }

    #[test]
    fn date_time_and_object_identifier() {
        // Year octet 124 = 2024, March 15, Friday (5).
        assert_eq!(
            one(&[0xA4, 0x7C, 0x03, 0x0F, 0x05]),
            PropertyValue::Date(2024, 3, 15, 5)
        );
        // Unspecified date: every octet 0xFF.
        assert_eq!(
            one(&[0xA4, 0xFF, 0xFF, 0xFF, 0xFF]),
            PropertyValue::Date(255, 255, 255, 255)
        );
        assert_eq!(
            one(&[0xB4, 0x0D, 0x2D, 0x1E, 0x19]),
            PropertyValue::Time(13, 45, 30, 25)
        );
        // 0x00C00001: type 3 (binary-input), instance 1.
        assert_eq!(
            one(&[0xC4, 0x00, 0xC0, 0x00, 0x01]),
            PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::BinaryInput, 1))
        );
    }

    #[test]
    fn constructed_values_are_flattened_not_rejected() {
        // [0] { time 08:00:00.00, real 1.0 } as in a weekly-schedule day.
        let values = decode(&[0x0E, 0xB4, 0x08, 0x00, 0x00, 0x00, 0x44, 0x3F, 0x80, 0, 0, 0x0F]).unwrap();
        assert_eq!(values, vec![PropertyValue::Time(8, 0, 0, 0), PropertyValue::Real(1.0)]);
        // Context-tagged primitive: kept raw.
        assert_eq!(decode(&[0x09, 0x05]).unwrap(), vec![PropertyValue::Unknown(vec![5])]);
    }

    #[test]
    fn malformed_values_are_errors_not_panics() {
        // Unsigned claiming 9 octets.
        let mut bytes = vec![0x25, 0x09];
        bytes.extend([1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(decode(&bytes).is_none());
        // Truncated Real.
        assert!(decode(&[0x44, 0x42, 0x91]).is_none());
        // Missing closing tag.
        assert!(decode_values(&mut Reader::new(&[0x21, 0x05]), 3).is_none());
    }

    // ---- ReadPropertyMultiple (clause 15.7) ----

    #[test]
    fn rpm_request_bytes() {
        // [0] AI 1 = 0C 00 00 00 01, [1] opening 1E, property refs 09 <id> [19 <index>], 1F.
        let object = ObjectIdentifier::new(ObjectType::AnalogInput, 1);
        let refs = [
            PropertyReference::new(PropertyIdentifier::ObjectName),
            PropertyReference::new(PropertyIdentifier::PresentValue),
            PropertyReference::with_array_index(PropertyIdentifier::ObjectList, 0),
        ];
        assert_eq!(
            rpm_request(object, &refs).unwrap(),
            [0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x09, 0x4D, 0x09, 0x55, 0x09, 0x4C, 0x19, 0x00, 0x1F]
        );
    }

    #[test]
    fn rpm_ack_with_values_and_property_errors() {
        // AI 33: present-value 72.0, status-flags 0000, object-name -> Error(property, unknown-property).
        let ack = [
            0x0C, 0x00, 0x00, 0x00, 0x21, 0x1E, //
            0x29, 0x55, 0x4E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x4F, //
            0x29, 0x6F, 0x4E, 0x82, 0x04, 0x00, 0x4F, //
            0x29, 0x4D, 0x5E, 0x91, 0x02, 0x91, 0x20, 0x5F, //
            0x1F,
        ];
        let results = decode_rpm_ack(&ack).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].object_identifier,
            ObjectIdentifier::new(ObjectType::AnalogInput, 33)
        );
        let props = &results[0].results;
        assert_eq!(props.len(), 3);
        assert_eq!(props[0].property_identifier, PropertyIdentifier::PresentValue);
        assert_eq!(props[0].value, PropertyResultValue::Value(vec![PropertyValue::Real(72.0)]));
        assert_eq!(props[1].property_identifier, PropertyIdentifier::StatusFlags);
        assert_eq!(
            props[1].value,
            PropertyResultValue::Value(vec![PropertyValue::BitString(vec![false; 4])])
        );
        assert_eq!(props[2].property_identifier, PropertyIdentifier::ObjectName);
        assert_eq!(props[2].value, PropertyResultValue::Error(2, 32));

        // Truncated reply is an error, not a partial result.
        assert!(decode_rpm_ack(&ack[..ack.len() - 1]).is_none());
    }

    #[test]
    fn rpm_ack_with_array_index_and_second_object() {
        // Device 8 object-list[0] = 5 ([3] index 0), then AI 34 present-value 1.0.
        let ack = [
            0x0C, 0x02, 0x00, 0x00, 0x08, 0x1E, 0x29, 0x4C, 0x39, 0x00, 0x4E, 0x21, 0x05, 0x4F,
            0x1F, //
            0x0C, 0x00, 0x00, 0x00, 0x22, 0x1E, 0x29, 0x55, 0x4E, 0x44, 0x3F, 0x80, 0x00, 0x00,
            0x4F, 0x1F,
        ];
        let results = decode_rpm_ack(&ack).unwrap();
        assert_eq!(results.len(), 2);
        let first = &results[0].results[0];
        assert_eq!(first.property_identifier, PropertyIdentifier::ObjectList);
        assert_eq!(first.array_index, Some(0));
        assert_eq!(first.value, PropertyResultValue::Value(vec![PropertyValue::Unsigned(5)]));
        assert_eq!(results[1].object_identifier.instance, 34);
        assert_eq!(results[1].results[0].array_index, None);
    }

    #[test]
    fn rpm_ack_with_iso_8859_1_name_still_decodes() {
        // object-name "21°C" in ISO 8859-1: bacnet-rs rejects the whole reply on this.
        let ack = [
            0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x29, 0x4D, 0x4E, 0x75, 0x05, 0x05, 0x32, 0x31,
            0xB0, 0x43, 0x4F, 0x1F,
        ];
        let results = decode_rpm_ack(&ack).unwrap();
        assert_eq!(
            results[0].results[0].value,
            PropertyResultValue::Value(vec![PropertyValue::CharacterString("21\u{B0}C".into())])
        );
    }

    #[test]
    fn rpm_ack_unknown_object_error() {
        // Per-property Error(object, unknown-object) as a device answers for a missing object.
        let ack = [
            0x0C, 0x02, 0x00, 0x00, 0x08, 0x1E, 0x29, 0x4D, 0x5E, 0x91, 0x01, 0x91, 0x1F, 0x5F,
            0x1F,
        ];
        let results = decode_rpm_ack(&ack).unwrap();
        assert_eq!(results[0].results[0].value, PropertyResultValue::Error(1, 31));
    }

    // ---- ReadProperty (clause 15.5) ----

    #[test]
    fn read_property_request_bytes() {
        let device = ObjectIdentifier::new(ObjectType::Device, 8);
        // [0] Device 8, [1] property 76 (object-list), optional [2] array index.
        assert_eq!(
            read_property_request(device, PropertyIdentifier::ObjectList, None).unwrap(),
            [0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 0x4C]
        );
        assert_eq!(
            read_property_request(device, PropertyIdentifier::ObjectList, Some(0)).unwrap(),
            [0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 0x4C, 0x29, 0x00]
        );
        // Two-octet index 300 = 2A 01 2C.
        assert_eq!(
            read_property_request(device, PropertyIdentifier::ObjectList, Some(300)).unwrap(),
            [0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 0x4C, 0x2A, 0x01, 0x2C]
        );
    }

    #[test]
    fn read_property_ack_object_list() {
        // Device 8's object-list: Device 8, AI 1, AV 2 between opening/closing tag 3.
        let ack = [
            0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 0x4C, 0x3E, //
            0xC4, 0x02, 0x00, 0x00, 0x08, //
            0xC4, 0x00, 0x00, 0x00, 0x01, //
            0xC4, 0x00, 0x80, 0x00, 0x02, //
            0x3F,
        ];
        let ack = decode_read_property_ack(&ack).unwrap();
        assert_eq!(ack.object, ObjectIdentifier::new(ObjectType::Device, 8));
        assert_eq!(ack.property, 76);
        assert_eq!(
            object_identifiers(&ack.values),
            vec![
                ObjectIdentifier::new(ObjectType::Device, 8),
                ObjectIdentifier::new(ObjectType::AnalogInput, 1),
                ObjectIdentifier::new(ObjectType::AnalogValue, 2),
            ]
        );

        // With [2] array index 0: the element count.
        let ack = [
            0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 0x4C, 0x29, 0x00, 0x3E, 0x21, 0x03, 0x3F,
        ];
        assert_eq!(
            decode_read_property_ack(&ack).unwrap().values,
            vec![PropertyValue::Unsigned(3)]
        );
    }

    // ---- Request flow against a scripted device on loopback ----

    /// Spawn a UDP device on loopback. `handler(invoke_id, service_choice,
    /// service_data)` returns the reply APDU for every confirmed request.
    fn spawn_fake_device(
        handler: impl Fn(u8, u8, &[u8]) -> Vec<u8> + Send + 'static,
    ) -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while let Ok((n, peer)) = socket.recv_from(&mut buf) {
                let Some(apdu) = apdu_of_frame(&buf[..n]) else {
                    continue;
                };
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

    /// Object-list ReadProperty-ACK for Device 8, optionally with `[2] index`.
    fn object_list_ack(index: Option<u8>, elements: &[u8]) -> Vec<u8> {
        let mut data = vec![0x0C, 0x02, 0x00, 0x00, 0x08, 0x19, 0x4C];
        if let Some(i) = index {
            data.extend_from_slice(&[0x29, i]);
        }
        data.push(0x3E);
        data.extend_from_slice(elements);
        data.push(0x3F);
        data
    }

    const DEVICE_8: [u8; 5] = [0xC4, 0x02, 0x00, 0x00, 0x08];
    const AI_1: [u8; 5] = [0xC4, 0x00, 0x00, 0x00, 0x01];
    const AV_2: [u8; 5] = [0xC4, 0x00, 0x80, 0x00, 0x02];

    #[test]
    fn object_list_is_read_whole_when_the_device_returns_it() {
        let addr = spawn_fake_device(|invoke, service, _| {
            assert_eq!(service, 12, "only ReadProperty expected");
            let elements = [DEVICE_8, AI_1, AV_2].concat();
            complex_ack(invoke, service, &object_list_ack(None, &elements))
        });
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let device = ObjectIdentifier::new(ObjectType::Device, 8);
        let ids = read_object_list(&socket, addr, device).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[2], ObjectIdentifier::new(ObjectType::AnalogValue, 2));
    }

    #[test]
    fn object_list_falls_back_to_element_reads_after_segmentation_abort() {
        // A non-segmenting device aborts the whole-array read with reason 4
        // (segmentation-not-supported), then serves index 0 (count) and 1..=3.
        let addr = spawn_fake_device(|invoke, service, data| {
            assert_eq!(service, 12);
            // Request data: 0C <4> 19 4C [29 <index>]
            if data.len() == 7 {
                return vec![0x71, invoke, 0x04];
            }
            let index = data[8];
            let ack = match index {
                0 => object_list_ack(Some(0), &[0x21, 0x03]),
                1 => object_list_ack(Some(1), &DEVICE_8),
                2 => object_list_ack(Some(2), &AI_1),
                3 => object_list_ack(Some(3), &AV_2),
                other => panic!("index {other} out of range"),
            };
            complex_ack(invoke, service, &ack)
        });
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let device = ObjectIdentifier::new(ObjectType::Device, 8);
        let ids = read_object_list(&socket, addr, device).unwrap();
        assert_eq!(
            ids,
            vec![
                ObjectIdentifier::new(ObjectType::Device, 8),
                ObjectIdentifier::new(ObjectType::AnalogInput, 1),
                ObjectIdentifier::new(ObjectType::AnalogValue, 2),
            ]
        );
    }

    #[test]
    fn rpm_reject_falls_back_to_read_property() {
        // The device rejects ReadPropertyMultiple (reject reason 9 =
        // unrecognized-service) and serves ReadProperty: object-name and
        // present-value succeed, any other property is an Error PDU
        // (property / unknown-property) and is omitted.
        let addr = spawn_fake_device(|invoke, service, data| match service {
            14 => vec![0x60, invoke, 0x09],
            12 => match data[6] {
                0x4D => complex_ack(
                    invoke,
                    service,
                    &[
                        0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x4D, 0x3E, 0x75, 0x05, 0x00, b'A',
                        b'I', b'-', b'1', 0x3F,
                    ],
                ),
                0x55 => complex_ack(
                    invoke,
                    service,
                    &[
                        0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x42, 0x90, 0x00,
                        0x00, 0x3F,
                    ],
                ),
                _ => vec![0x50, invoke, service, 0x91, 0x02, 0x91, 0x20],
            },
            other => panic!("unexpected service {other}"),
        });
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let object = ObjectIdentifier::new(ObjectType::AnalogInput, 1);
        let refs = [
            PropertyReference::new(PropertyIdentifier::ObjectName),
            PropertyReference::new(PropertyIdentifier::PresentValue),
            PropertyReference::new(PropertyIdentifier::Description),
        ];
        let results = read_access_results(&socket, addr, object, &refs).unwrap();
        assert_eq!(results.len(), 1);
        let props = &results[0].results;
        assert_eq!(props.len(), 2);
        assert_eq!(
            props[0].value,
            PropertyResultValue::Value(vec![PropertyValue::CharacterString("AI-1".into())])
        );
        assert_eq!(props[1].value, PropertyResultValue::Value(vec![PropertyValue::Real(72.0)]));
    }

    #[test]
    fn rpm_failure_with_nothing_readable_reports_the_rpm_error() {
        // Error(object, unknown-object) for every request.
        let addr = spawn_fake_device(|invoke, service, _| {
            vec![0x50, invoke, service, 0x91, 0x01, 0x91, 0x1F]
        });
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let object = ObjectIdentifier::new(ObjectType::AnalogInput, 99);
        let refs = [PropertyReference::new(PropertyIdentifier::PresentValue)];
        let err = read_access_results(&socket, addr, object, &refs).unwrap_err();
        assert!(err.to_string().contains("unknown-object"), "{err}");
    }

    // ---- Value mapping ----

    #[test]
    fn signed_and_wide_values_do_not_wrap() {
        assert!(matches!(
            extract_bacnet_value(&[PropertyValue::Signed(-5)]),
            BacnetValue::Double(v) if v == -5.0
        ));
        assert!(matches!(
            extract_bacnet_value(&[PropertyValue::Unsigned(4_294_967_296)]),
            BacnetValue::Double(v) if v == 4_294_967_296.0
        ));
        assert!(matches!(
            extract_bacnet_value(&[PropertyValue::Unsigned(1476)]),
            BacnetValue::Unsigned(1476)
        ));
        let negative = PropertyResultValue::Value(vec![PropertyValue::Signed(-1)]);
        assert_eq!(extract_u32(&negative), None);
    }
}
