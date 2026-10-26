//! EtherNet/IP sessions over `mt-enip`: a client that reads a Logix
//! controller's tags and CIP attributes and sends explicit messages, an
//! adapter simulator playing a plant, and the ListIdentity behind the app's
//! device picker.

pub(crate) mod adapter;
pub(crate) mod client;
pub mod plants;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use mt_enip::{CipValue, ListIdentityReply, PacketDir, PacketRecord};
use vt_protocol::{CipType, Direction, EnipDeviceInfo, EnipDiscoverSpec, TrafficFrame};

/// The name of a CIP service code, as the inspector shows an explicit
/// message; `class` tells object-specific codes apart.
pub fn cip_service_name(service: u8, class: Option<u16>) -> &'static str {
    mt_enip::service_name(service, class)
}

/// Answer a [`vt_protocol::Command::DiscoverEnip`]: send ListIdentity and
/// list the devices that answer, once each.
pub(crate) async fn discover(spec: EnipDiscoverSpec) -> Result<Vec<EnipDeviceInfo>, String> {
    let mut directed = Vec::with_capacity(spec.directed.len());
    for target in &spec.directed {
        directed.push(resolve_target(target).await?);
    }
    let wait = Duration::from_millis(spec.wait_ms.clamp(200, 30_000));
    let replies = mt_enip::list_identity(
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        spec.broadcast_port,
        &directed,
        wait,
    )
    .await
    .map_err(|error| format!("ListIdentity could not be sent: {error}"))?;
    Ok(one_per_device(replies, &directed))
}

/// `host:port` to an IPv4 socket address: EtherNet/IP identities carry IPv4.
async fn resolve_target(target: &str) -> Result<SocketAddr, String> {
    tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host(target))
        .await
        .map_err(|_| format!("Resolving {target} timed out"))?
        .map_err(|error| format!("Cannot resolve {target}: {error}"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("{target} has no IPv4 address"))
}

/// One entry per identity (vendor, serial and session port): a device that
/// answered on several interfaces, or both the broadcast and a directed
/// request, is listed once, at an address a request was sent to if it answered
/// there, else on loopback, else the first; the others go in `also`.
fn one_per_device(replies: Vec<ListIdentityReply>, directed: &[SocketAddr]) -> Vec<EnipDeviceInfo> {
    let rank = |reply: &ListIdentityReply| {
        if directed.contains(&reply.from) {
            0
        } else if reply.from.ip().is_loopback() {
            1
        } else {
            2
        }
    };
    let mut groups: Vec<Vec<ListIdentityReply>> = Vec::new();
    for reply in replies {
        let key =
            |r: &ListIdentityReply| (r.device.vendor_id, r.device.serial_number, r.device.port);
        match groups
            .iter_mut()
            .find(|group| key(&group[0]) == key(&reply))
        {
            Some(group) => {
                if !group.iter().any(|seen| seen.from.ip() == reply.from.ip()) {
                    group.push(reply);
                }
            }
            None => groups.push(vec![reply]),
        }
    }
    groups
        .into_iter()
        .map(|mut group| {
            group.sort_by_key(rank);
            let first = group.remove(0);
            let device = first.device;
            EnipDeviceInfo {
                address: device.host.clone(),
                port: device.port,
                product: device.product_name.clone(),
                vendor: device.vendor_name.clone(),
                device_type: device_type_name(device.device_type),
                revision: format!("{}.{}", device.revision_major, device.revision_minor),
                serial: device.serial_number,
                also: group
                    .iter()
                    .map(|reply| host_port(reply.from.ip(), device.port))
                    .collect(),
            }
        })
        .collect()
}

fn host_port(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(ip) => format!("{ip}:{port}"),
        IpAddr::V6(ip) => format!("[{ip}]:{port}"),
    }
}

/// CIP device profile names (ODVA Vol 1, Table 6-7.1).
pub(crate) fn device_type_name(code: u16) -> String {
    match code {
        0x00 => "Generic Device (deprecated)".into(),
        0x02 => "AC Drive".into(),
        0x03 => "Motor Overload".into(),
        0x04 => "Limit Switch".into(),
        0x05 => "Inductive Proximity Switch".into(),
        0x06 => "Photoelectric Sensor".into(),
        0x07 => "General Purpose Discrete I/O".into(),
        0x09 => "Resolver".into(),
        0x0C => "Communications Adapter".into(),
        0x0E => "Programmable Logic Controller".into(),
        0x10 => "Position Controller".into(),
        0x13 => "DC Drive".into(),
        0x15 => "Contactor".into(),
        0x16 => "Motor Starter".into(),
        0x17 => "Soft Start".into(),
        0x18 => "Human-Machine Interface".into(),
        0x1A => "Mass Flow Controller".into(),
        0x1B => "Pneumatic Valve".into(),
        0x1C => "Vacuum Pressure Gauge".into(),
        0x1D => "Process Control Value".into(),
        0x1E => "Residual Gas Analyzer".into(),
        0x1F => "DC Power Generator".into(),
        0x20 => "RF Power Generator".into(),
        0x21 => "Turbomolecular Vacuum Pump".into(),
        0x22 => "Encoder".into(),
        0x23 => "Safety Discrete I/O".into(),
        0x24 => "Fluid Flow Controller".into(),
        0x25 => "CIP Motion Drive".into(),
        0x26 => "CompoNet Repeater".into(),
        0x27 => "Mass Flow Controller, Enhanced".into(),
        0x28 => "CIP Modbus Device".into(),
        0x29 => "CIP Modbus Translator".into(),
        0x2A => "Safety Analog I/O".into(),
        0x2B => "Generic Device (keyable)".into(),
        0x2C => "Managed Switch".into(),
        0x32 => "ControlNet Physical Layer Component".into(),
        other => format!("Device type 0x{other:02X}"),
    }
}

/// The CIP type of a value read from a tag, if it is a number or bit.
pub(crate) fn cip_type_of(value: &CipValue) -> Option<CipType> {
    Some(match value {
        CipValue::Bool(_) => CipType::Bool,
        CipValue::Sint(_) => CipType::Sint,
        CipValue::Int(_) => CipType::Int,
        CipValue::Dint(_) => CipType::Dint,
        CipValue::Lint(_) => CipType::Lint,
        CipValue::Usint(_) => CipType::Usint,
        CipValue::Uint(_) => CipType::Uint,
        CipValue::Udint(_) => CipType::Udint,
        CipValue::Ulint(_) => CipType::Ulint,
        CipValue::Real(_) => CipType::Real,
        CipValue::Lreal(_) => CipType::Lreal,
        CipValue::String(_) | CipValue::Bytes(_) | CipValue::Array(_) => return None,
    })
}

/// Read a little-endian value of `data_type` at `offset` of an attribute's
/// bytes; `None` when the attribute is too short.
pub(crate) fn decode(data_type: CipType, bytes: &[u8], offset: usize) -> Option<f64> {
    let b = bytes.get(offset..offset.checked_add(data_type.size())?)?;
    Some(match data_type {
        CipType::Bool => f64::from(u8::from(b[0] != 0)),
        CipType::Sint => f64::from(i8::from_le_bytes([b[0]])),
        CipType::Usint => f64::from(b[0]),
        CipType::Int => f64::from(i16::from_le_bytes([b[0], b[1]])),
        CipType::Uint => f64::from(u16::from_le_bytes([b[0], b[1]])),
        CipType::Dint => f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        CipType::Udint => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        CipType::Real => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        CipType::Lint => i64::from_le_bytes(eight(b)) as f64,
        CipType::Ulint => u64::from_le_bytes(eight(b)) as f64,
        CipType::Lreal => f64::from_le_bytes(eight(b)),
    })
}

fn eight(b: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    out.copy_from_slice(&b[..8]);
    out
}

/// Little-endian bytes of `value` as `data_type`: whole numbers in range for
/// the integer types, 0 or 1 for BOOL.
pub(crate) fn encode(data_type: CipType, value: f64) -> Result<Vec<u8>, String> {
    if !value.is_finite() {
        return Err("Set a number".into());
    }
    let name = data_type.name();
    let whole = || -> Result<i128, String> {
        if value.fract() != 0.0 {
            return Err(format!("{name} takes whole numbers"));
        }
        Ok(value as i128)
    };
    let range = |_: std::num::TryFromIntError| format!("{value} is out of range for {name}");
    Ok(match data_type {
        CipType::Bool if value == 0.0 => vec![0],
        CipType::Bool if value == 1.0 => vec![1],
        CipType::Bool => return Err("BOOL takes 0 or 1".into()),
        CipType::Sint => i8::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Usint => u8::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Int => i16::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Uint => u16::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Dint => i32::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Udint => u32::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Lint => i64::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Ulint => u64::try_from(whole()?)
            .map_err(range)?
            .to_le_bytes()
            .to_vec(),
        CipType::Real => {
            if value.abs() > f64::from(f32::MAX) {
                return Err(format!("{value} is out of range for REAL"));
            }
            (value as f32).to_le_bytes().to_vec()
        }
        CipType::Lreal => value.to_le_bytes().to_vec(),
    })
}

/// A captured frame of the client's session, for the traffic view.
pub(crate) fn traffic_frame(record: &PacketRecord) -> TrafficFrame {
    let (summary, detail) = mt_enip::describe_frame(&record.bytes);
    TrafficFrame {
        at_ms: record.timestamp_ms,
        direction: match record.direction {
            PacketDir::Tx => Direction::Tx,
            PacketDir::Rx => Direction::Rx,
        },
        summary,
        detail,
        bytes: u32::try_from(record.bytes.len()).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_enip::EnipDevice;

    #[test]
    fn attribute_values_decode_little_endian_at_their_offset() {
        let bytes = [0xFF, 0x34, 0x12, 0x00, 0x00, 0xC0, 0x3F, 0x9C, 0xFF];
        assert_eq!(decode(CipType::Bool, &bytes, 0), Some(1.0));
        assert_eq!(decode(CipType::Sint, &bytes, 0), Some(-1.0));
        assert_eq!(decode(CipType::Usint, &bytes, 0), Some(255.0));
        assert_eq!(decode(CipType::Uint, &bytes, 1), Some(f64::from(0x1234u16)));
        assert_eq!(decode(CipType::Real, &bytes, 3), Some(1.5));
        assert_eq!(decode(CipType::Int, &bytes, 7), Some(-100.0));
        // Past the end of the attribute: no value, not a panic.
        assert_eq!(decode(CipType::Dint, &bytes, 6), None);
        assert_eq!(decode(CipType::Lreal, &bytes, usize::MAX), None);
    }

    #[test]
    fn values_encode_in_range_or_say_why() {
        for data_type in CipType::ALL {
            let bytes = encode(data_type, 1.0).expect("1 fits every type");
            assert_eq!(bytes.len(), data_type.size());
            assert_eq!(decode(data_type, &bytes, 0), Some(1.0));
        }
        assert_eq!(encode(CipType::Int, -2.0), Ok(vec![0xFE, 0xFF]));
        assert_eq!(
            encode(CipType::Real, 1.5),
            Ok(1.5f32.to_le_bytes().to_vec())
        );
        assert_eq!(
            encode(CipType::Sint, 128.0),
            Err("128 is out of range for SINT".into())
        );
        assert_eq!(
            encode(CipType::Uint, -1.0),
            Err("-1 is out of range for UINT".into())
        );
        assert_eq!(
            encode(CipType::Dint, 2.5),
            Err("DINT takes whole numbers".into())
        );
        assert_eq!(encode(CipType::Bool, 2.0), Err("BOOL takes 0 or 1".into()));
        assert!(encode(CipType::Real, 1e39).is_err());
        assert!(encode(CipType::Lreal, f64::NAN).is_err());
    }

    fn reply(from: &str, serial: u32, port: u16) -> ListIdentityReply {
        let from: SocketAddr = from.parse().expect("an address");
        ListIdentityReply {
            from,
            device: EnipDevice {
                host: from.ip().to_string(),
                port,
                vendor_id: 1,
                vendor_name: "Rockwell Automation/Allen-Bradley".into(),
                device_type: 0x0E,
                product_code: 0x50,
                revision_major: 20,
                revision_minor: 11,
                status: 0x20,
                serial_number: serial,
                product_name: "ENIP-Sim".into(),
                state: 3,
            },
        }
    }

    #[test]
    fn a_device_answering_twice_is_listed_once_where_it_was_asked() {
        let directed: Vec<SocketAddr> = vec!["127.0.0.1:44830".parse().expect("an address")];
        let found = one_per_device(
            vec![
                reply("192.168.1.9:44830", 7, 44830),
                reply("127.0.0.1:44830", 7, 44830),
                // Same serial, another session port: another simulator.
                reply("192.168.1.9:44831", 7, 44831),
                reply("10.0.0.5:44818", 9, 44818),
            ],
            &directed,
        );
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].address, "127.0.0.1");
        assert_eq!(found[0].also, ["192.168.1.9:44830"]);
        assert_eq!(found[1].port, 44831);
        assert_eq!(found[2].device_type, "Programmable Logic Controller");
        assert_eq!(found[2].revision, "20.11");
    }
}
