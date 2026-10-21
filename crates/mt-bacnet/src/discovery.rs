use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use if_addrs::get_if_addrs;
use mt_protocol::ProtocolError;

use crate::transport::{confirmed_request, REQUEST_TIMEOUT};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::engineering_units::EngineeringUnits;
use bacnet_rs::object::{ObjectIdentifier, PropertyIdentifier};
use bacnet_rs::property::PropertyValue;
use bacnet_rs::service::{
    ConfirmedServiceChoice, IAmRequest, PropertyReference, ReadAccessSpecification,
    ReadPropertyMultipleRequest, ReadPropertyMultipleResponse, UnconfirmedServiceChoice, WhoIsRequest,
};
use bacnet_rs::vendor::get_vendor_name;

use crate::types::{
    BacnetDiscoveredDevice, BacnetObjectDetail, BacnetPropertyEntry, BacnetValue, DeviceInspection,
};
use crate::BacnetObjectInfo;

/// Default BACnet/IP UDP port (0xBAC0).
const BACNET_IP_PORT: u16 = 47808;

/// Broadcast a BACnet Who-Is and collect I-Am responses for the given timeout.
pub async fn discover_devices(
    timeout_secs: u64,
) -> Result<Vec<BacnetDiscoveredDevice>, ProtocolError> {
    tokio::task::spawn_blocking(move || {
        // Build the Who-Is APDU wrapped in a global-broadcast NPDU.
        let whois = WhoIsRequest::new();
        let mut service_data = Vec::new();
        whois
            .encode(&mut service_data)
            .map_err(|e| ProtocolError::Other(format!("WhoIs encode: {e}")))?;

        let mut apdu = vec![0x10, UnconfirmedServiceChoice::WhoIs as u8];
        apdu.extend_from_slice(&service_data);

        let npdu = Npdu::global_broadcast();
        let mut npdu_apdu = npdu.encode();
        npdu_apdu.extend_from_slice(&apdu);

        // Wrap in a BACnet/IP BVLC Original-Broadcast-NPDU (0x0B) frame.
        let total_len = (4 + npdu_apdu.len()) as u16;
        let mut frame = vec![0x81, 0x0B];
        frame.extend_from_slice(&total_len.to_be_bytes());
        frame.extend_from_slice(&npdu_apdu);

        // Bind the standard BACnet port so devices replying I-Am to :47808 reach us.
        // Fall back to an ephemeral port if 47808 is already in use on the host.
        let socket = UdpSocket::bind(("0.0.0.0", BACNET_IP_PORT))
            .or_else(|_| UdpSocket::bind("0.0.0.0:0"))
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
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

        // Collect each host interface's directed subnet broadcast address.
        // bacnet-rs computes `0.0.0.255` when bound to 0.0.0.0 (os error 65 on send),
        // and macOS limited-broadcast (255.255.255.255) does not reach vmnet subnets.
        // Directed broadcast (e.g. 192.168.64.255) routes to the correct interface.
        let mut broadcasts: Vec<Ipv4Addr> = Vec::new();
        if let Ok(ifaces) = get_if_addrs() {
            for iface in ifaces {
                if let if_addrs::IfAddr::V4(v4) = iface.addr {
                    if let Some(bcast) = v4.broadcast {
                        if !broadcasts.contains(&bcast) {
                            broadcasts.push(bcast);
                        }
                    }
                }
            }
        }

        // Always also send a limited broadcast (no-op if duplicate).
        let limited = Ipv4Addr::new(255, 255, 255, 255);
        if !broadcasts.contains(&limited) {
            broadcasts.push(limited);
        }

        tracing::info!("BACnet Who-Is targets: {:?}", broadcasts);
        for bcast in &broadcasts {
            if let Err(e) = socket.send_to(&frame, SocketAddr::from((*bcast, BACNET_IP_PORT))) {
                tracing::warn!("Who-Is send to {bcast} failed: {e}");
            }
        }

        let mut discovered = Vec::new();
        let start = std::time::Instant::now();

        while start.elapsed() < Duration::from_secs(timeout_secs) {
            let mut buf = [0u8; 1500];
            match socket.recv_from(&mut buf) {
                Ok((n, source)) => {
                    if let Some(device) = parse_iam_response(&buf[..n], source) {
                        if !discovered
                            .iter()
                            .any(|d: &BacnetDiscoveredDevice| d.device_instance == device.device_instance)
                        {
                            discovered.push(device);
                        }
                    }
                }
                Err(_) => continue,
            }
        }

        Ok(discovered)
    })
    .await
    .map_err(|e| ProtocolError::Other(format!("Task join: {e}")))?
}

/// Parse a raw BVLC/NPDU/APDU payload into a [`BacnetDiscoveredDevice`].
fn parse_iam_response(data: &[u8], source: SocketAddr) -> Option<BacnetDiscoveredDevice> {
    if data.len() < 4 || data[0] != 0x81 {
        return None;
    }

    let bvlc_len = ((data[2] as usize) << 8) | data[3] as usize;
    if data.len() != bvlc_len {
        return None;
    }

    let npdu_start = 4;
    let (_npdu, npdu_len) = Npdu::decode(&data[npdu_start..]).ok()?;

    let apdu_start = npdu_start + npdu_len;
    let apdu = &data[apdu_start..];

    if apdu.len() < 2 || apdu[0] != 0x10 || apdu[1] != UnconfirmedServiceChoice::IAm as u8 {
        return None;
    }

    let iam = IAmRequest::decode(&apdu[2..]).ok()?;

    Some(BacnetDiscoveredDevice {
        device_instance: iam.device_identifier.instance,
        ip: source.ip().to_string(),
        port: source.port(),
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
        let client = bacnet_rs::client::BacnetClient::new()
            .map_err(|e| ProtocolError::ConnectionFailed(format!("BACnet client: {e}")))?;

        let obj_ids = client
            .read_object_list(addr, device_instance)
            .map_err(|e| ProtocolError::ReadFailed(format!("Object list: {e}")))?;

        let objects = obj_ids
            .iter()
            .filter(|id| id.object_type != bacnet_rs::object::ObjectType::Device)
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
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
        socket
            .set_read_timeout(Some(Duration::from_secs(6)))
            .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

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

        let read_spec = ReadAccessSpecification::new(obj_id, prop_refs);
        let rpm_request = ReadPropertyMultipleRequest::new(vec![read_spec]);

        let mut service_data = Vec::new();
        rpm_request
            .encode(&mut service_data)
            .map_err(|e| ProtocolError::ReadFailed(format!("RPM encode: {e}")))?;

        let response_data = confirmed_request(
            &socket,
            addr,
            ConfirmedServiceChoice::ReadPropertyMultiple,
            service_data,
            REQUEST_TIMEOUT,
        )?;

        // Parse response using bacnet-rs structured decoder
        let rpm_response = ReadPropertyMultipleResponse::decode(&response_data)
            .map_err(|e| ProtocolError::ReadFailed(format!("RPM response decode: {e}")))?;

        let mut details = Vec::new();
        for access_result in &rpm_response.read_access_results {
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
                        values
                            .iter()
                            .map(|v| v.as_display_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                    bacnet_rs::service::PropertyResultValue::Error(c, code) => {
                        format!("Error({c}, {code})")
                    }
                };

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
                    PropertyIdentifier::Units => {
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
                    PropertyIdentifier::StatusFlags => {
                        detail.status_flags = Some(raw_value);
                    }
                    PropertyIdentifier::Reliability => {
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
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Socket bind: {e}")))?;
        socket
            .set_read_timeout(Some(Duration::from_secs(6)))
            .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

        let device_obj = ObjectIdentifier::new(bacnet_rs::object::ObjectType::Device, device_instance);
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
        ];

        let read_spec = ReadAccessSpecification::new(device_obj, prop_refs);
        let rpm_request = ReadPropertyMultipleRequest::new(vec![read_spec]);

        let mut service_data = Vec::new();
        rpm_request
            .encode(&mut service_data)
            .map_err(|e| ProtocolError::ReadFailed(format!("RPM encode: {e}")))?;

        let response_data = confirmed_request(
            &socket,
            addr,
            ConfirmedServiceChoice::ReadPropertyMultiple,
            service_data,
            REQUEST_TIMEOUT,
        )?;

        let rpm_response = ReadPropertyMultipleResponse::decode(&response_data)
            .map_err(|e| ProtocolError::ReadFailed(format!("RPM response decode: {e}")))?;

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

        for access_result in &rpm_response.read_access_results {
            for prop_result in &access_result.results {
                let raw = match &prop_result.value {
                    bacnet_rs::service::PropertyResultValue::Value(values) => values
                        .iter()
                        .map(|v| v.as_display_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    bacnet_rs::service::PropertyResultValue::Error(c, code) => {
                        format!("Error({c}, {code})")
                    }
                };

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

/// Extract the first numeric BacnetValue from a list of PropertyValues.
fn extract_bacnet_value(values: &[PropertyValue]) -> BacnetValue {
    if values.is_empty() {
        return BacnetValue::Null;
    }
    match &values[0] {
        PropertyValue::Real(f) => BacnetValue::Real(*f),
        PropertyValue::Double(f) => BacnetValue::Double(*f),
        PropertyValue::Unsigned(u) => BacnetValue::Unsigned(*u as u32),
        PropertyValue::Signed(i) => BacnetValue::Unsigned(*i as u32),
        PropertyValue::Boolean(b) => BacnetValue::Boolean(*b),
        PropertyValue::Enumerated(e) => BacnetValue::Enum(*e),
        PropertyValue::CharacterString(s) => BacnetValue::String(s.clone()),
        PropertyValue::Null => BacnetValue::Null,
        _ => BacnetValue::Null,
    }
}

/// Extract a u32 from the first value if it is numeric/enumerated.
fn extract_u32(value: &bacnet_rs::service::PropertyResultValue) -> Option<u32> {
    match value {
        bacnet_rs::service::PropertyResultValue::Value(values) => {
            if let Some(v) = values.first() {
                match v {
                    PropertyValue::Unsigned(u) => Some(*u as u32),
                    PropertyValue::Enumerated(e) => Some(*e),
                    PropertyValue::Signed(i) => Some(*i as u32),
                    PropertyValue::Real(f) => Some(*f as u32),
                    _ => None,
                }
            } else {
                None
            }
        }
        bacnet_rs::service::PropertyResultValue::Error(_, _) => None,
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

/// Map a BACnet property identifier number to a human-readable name.
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
