//! BACnet/IP sessions over `mt-bacnet`: a client that discovers a device,
//! reads its object list and polls or subscribes to its points, and a device
//! simulator; plus the Who-Is behind the app's device picker.

pub(crate) mod client;
pub(crate) mod device;

use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use mt_bacnet::BacnetObject;
use mt_bacnet::transport::{
    abort_reason_name, error_class_name, error_code_name, reject_reason_name,
};
use vt_protocol::{BacnetDeviceInfo, BacnetDiscoverSpec, Direction, TrafficFrame};

use crate::emit::now_ms;

/// The BACnet object types read as points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    AnalogInput,
    AnalogOutput,
    AnalogValue,
    BinaryInput,
    BinaryOutput,
    BinaryValue,
    MultiStateInput,
    MultiStateOutput,
    MultiStateValue,
}

impl ObjectKind {
    const ALL: [ObjectKind; 9] = [
        Self::AnalogInput,
        Self::AnalogOutput,
        Self::AnalogValue,
        Self::BinaryInput,
        Self::BinaryOutput,
        Self::BinaryValue,
        Self::MultiStateInput,
        Self::MultiStateOutput,
        Self::MultiStateValue,
    ];

    /// The camelCase name `mt-bacnet` uses, e.g. `analogInput`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::AnalogInput => "analogInput",
            Self::AnalogOutput => "analogOutput",
            Self::AnalogValue => "analogValue",
            Self::BinaryInput => "binaryInput",
            Self::BinaryOutput => "binaryOutput",
            Self::BinaryValue => "binaryValue",
            Self::MultiStateInput => "multiStateInput",
            Self::MultiStateOutput => "multiStateOutput",
            Self::MultiStateValue => "multiStateValue",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The BACnetObjectType number (clause 21).
    fn from_number(number: u32) -> Option<Self> {
        Some(match number {
            0 => Self::AnalogInput,
            1 => Self::AnalogOutput,
            2 => Self::AnalogValue,
            3 => Self::BinaryInput,
            4 => Self::BinaryOutput,
            5 => Self::BinaryValue,
            13 => Self::MultiStateInput,
            14 => Self::MultiStateOutput,
            19 => Self::MultiStateValue,
            _ => return None,
        })
    }

    /// Engineers' short form, e.g. `AI`, `MSV`.
    pub(crate) fn short(self) -> &'static str {
        match self {
            Self::AnalogInput => "AI",
            Self::AnalogOutput => "AO",
            Self::AnalogValue => "AV",
            Self::BinaryInput => "BI",
            Self::BinaryOutput => "BO",
            Self::BinaryValue => "BV",
            Self::MultiStateInput => "MSI",
            Self::MultiStateOutput => "MSO",
            Self::MultiStateValue => "MSV",
        }
    }

    /// Type column of the points table: the present value's datatype, as
    /// the address already names the object type.
    pub(crate) fn label(self) -> &'static str {
        if self.is_analog() {
            "Real"
        } else if self.is_binary() {
            "Binary"
        } else {
            "Multi-state"
        }
    }

    pub(crate) fn is_analog(self) -> bool {
        matches!(
            self,
            Self::AnalogInput | Self::AnalogOutput | Self::AnalogValue
        )
    }

    pub(crate) fn is_binary(self) -> bool {
        matches!(
            self,
            Self::BinaryInput | Self::BinaryOutput | Self::BinaryValue
        )
    }

    pub(crate) fn is_multi_state(self) -> bool {
        matches!(
            self,
            Self::MultiStateInput | Self::MultiStateOutput | Self::MultiStateValue
        )
    }

    /// Outputs and values take present-value writes; inputs are sensors.
    pub(crate) fn writable(self) -> bool {
        !matches!(
            self,
            Self::AnalogInput | Self::BinaryInput | Self::MultiStateInput
        )
    }
}

/// One object of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Object {
    pub(crate) kind: ObjectKind,
    pub(crate) instance: u32,
}

impl Object {
    /// How engineers write it: `AI 1`.
    pub(crate) fn label(self) -> String {
        format!("{} {}", self.kind.short(), self.instance)
    }

    pub(crate) fn tag(self, property: &str) -> BacnetObject {
        BacnetObject {
            name: self.label(),
            object_type: self.kind.name().into(),
            object_instance: self.instance,
            property: property.into(),
        }
    }
}

/// BACnetStatusFlags (clause 12), as every point object reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StatusFlags {
    pub(crate) in_alarm: bool,
    pub(crate) fault: bool,
    pub(crate) overridden: bool,
    pub(crate) out_of_service: bool,
}

impl StatusFlags {
    /// Parse the named form `mt-bacnet` reports: `normal`, or the set flags
    /// separated by commas, e.g. `in-alarm, out-of-service`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let mut flags = Self::default();
        if text == "normal" {
            return Some(flags);
        }
        for name in text.split(',') {
            match name.trim() {
                "in-alarm" => flags.in_alarm = true,
                "fault" => flags.fault = true,
                "overridden" => flags.overridden = true,
                "out-of-service" => flags.out_of_service = true,
                _ => return None,
            }
        }
        Some(flags)
    }

    /// The named form, as [`Self::parse`] reads it.
    pub(crate) fn describe(self) -> String {
        let names: Vec<&str> = [
            (self.in_alarm, "in-alarm"),
            (self.fault, "fault"),
            (self.overridden, "overridden"),
            (self.out_of_service, "out-of-service"),
        ]
        .into_iter()
        .filter_map(|(set, name)| set.then_some(name))
        .collect();
        if names.is_empty() {
            "normal".into()
        } else {
            names.join(", ")
        }
    }
}

/// An IP address as typed, possibly in brackets (`[::1]`).
pub(crate) fn parse_ip(text: &str) -> Option<IpAddr> {
    let text = text.trim();
    text.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(text)
        .parse()
        .ok()
}

/// Resolve `host` (an address or a name) on `port`, IPv4 first.
pub(crate) async fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    let bare = host.trim();
    let bare = bare
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(bare);
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::lookup_host((bare, port)),
    )
    .await
    .map_err(|_| format!("Resolving {host} timed out"))?
    .map_err(|error| format!("Cannot resolve {host}: {error}"))?
    .min_by_key(SocketAddr::is_ipv6)
    .ok_or_else(|| format!("{host} has no address"))
}

/// Resolve `host:port` (`[v6]:port` for IPv6).
async fn resolve_target(target: &str) -> Result<SocketAddr, String> {
    let target = target.trim();
    let (host, port) = target
        .rsplit_once(':')
        .filter(|(host, _)| !host.is_empty())
        .ok_or_else(|| format!("{target}: give an address and port, e.g. 192.168.1.40:47808"))?;
    let port = port
        .parse()
        .map_err(|_| format!("{target}: the port must be a number"))?;
    resolve(host, port).await
}

/// Answer a [`vt_protocol::Command::DiscoverBacnet`]: send the Who-Is and
/// list the devices that answer, by instance.
pub(crate) async fn discover(spec: BacnetDiscoverSpec) -> Result<Vec<BacnetDeviceInfo>, String> {
    let ip = parse_ip(&spec.bind)
        .ok_or_else(|| format!("Send from an IP address, not {}", spec.bind))?;
    let mut directed = Vec::with_capacity(spec.directed.len());
    for target in &spec.directed {
        directed.push(resolve_target(target).await?);
    }
    let wait = Duration::from_millis(spec.wait_ms.clamp(200, 30_000));
    // Devices answer a broadcast Who-Is with a broadcast I-Am to the BACnet
    // port, so listen there when it is free; another BACnet application (or a
    // simulator in this app) may hold it, and then only unicast answers come.
    let found = match spec.broadcast_port {
        Some(port) => match mt_bacnet::who_is(
            SocketAddr::new(ip, port),
            Some(port),
            directed.clone(),
            wait,
        )
        .await
        {
            Ok(found) => found,
            Err(_) => mt_bacnet::who_is(SocketAddr::new(ip, 0), Some(port), directed.clone(), wait)
                .await
                .map_err(|error| error.to_string())?,
        },
        None => mt_bacnet::who_is(SocketAddr::new(ip, 0), None, directed.clone(), wait)
            .await
            .map_err(|error| error.to_string())?,
    };
    Ok(one_per_device(found, &directed))
}

/// One entry per device instance: a device on several interfaces (or that
/// answered both the broadcast and a directed Who-Is) is listed once, at an
/// address the Who-Is was sent to if it answered there, else on loopback,
/// else the first; its other addresses go in `also`.
fn one_per_device(
    mut found: Vec<mt_bacnet::BacnetDiscoveredDevice>,
    directed: &[SocketAddr],
) -> Vec<BacnetDeviceInfo> {
    let rank = |device: &mt_bacnet::BacnetDiscoveredDevice| {
        let ip = device.ip.parse::<IpAddr>().ok();
        let address = ip.map(|ip| SocketAddr::new(ip, device.port));
        if address.is_some_and(|address| directed.contains(&address)) {
            0
        } else if ip.is_some_and(|ip| ip.is_loopback()) {
            1
        } else {
            2
        }
    };
    found.sort_by(|a, b| {
        (a.device_instance, rank(a), &a.ip, a.port).cmp(&(
            b.device_instance,
            rank(b),
            &b.ip,
            b.port,
        ))
    });
    let mut devices: Vec<BacnetDeviceInfo> = Vec::new();
    for device in found {
        match devices.last_mut() {
            Some(last) if last.instance == device.device_instance => {
                last.also.push(format!("{}:{}", device.ip, device.port));
            }
            _ => devices.push(BacnetDeviceInfo {
                instance: device.device_instance,
                address: device.ip,
                port: device.port,
                vendor: device.vendor_name.unwrap_or_default(),
                also: Vec::new(),
            }),
        }
    }
    devices
}

pub(crate) fn traffic(
    direction: Direction,
    summary: impl Into<String>,
    detail: impl Into<String>,
    bytes: usize,
) -> TrafficFrame {
    TrafficFrame {
        at_ms: now_ms(),
        direction,
        summary: summary.into(),
        detail: detail.into(),
        bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
    }
}

fn confirmed_service(choice: u8) -> String {
    match choice {
        0 => "AcknowledgeAlarm".into(),
        1 => "ConfirmedCOVNotification".into(),
        2 => "ConfirmedEventNotification".into(),
        3 => "GetAlarmSummary".into(),
        5 => "SubscribeCOV".into(),
        6 => "AtomicReadFile".into(),
        7 => "AtomicWriteFile".into(),
        12 => "ReadProperty".into(),
        14 => "ReadPropertyMultiple".into(),
        15 => "WriteProperty".into(),
        16 => "WritePropertyMultiple".into(),
        17 => "DeviceCommunicationControl".into(),
        20 => "ReinitializeDevice".into(),
        26 => "ReadRange".into(),
        28 => "SubscribeCOVProperty".into(),
        29 => "GetEventInformation".into(),
        n => format!("Service {n}"),
    }
}

fn unconfirmed_service(choice: u8) -> String {
    match choice {
        0 => "I-Am".into(),
        1 => "I-Have".into(),
        2 => "UnconfirmedCOVNotification".into(),
        3 => "UnconfirmedEventNotification".into(),
        6 => "TimeSynchronization".into(),
        7 => "Who-Has".into(),
        8 => "Who-Is".into(),
        9 => "UTCTimeSynchronization".into(),
        n => format!("Unconfirmed {n}"),
    }
}

pub(crate) fn property_name(id: u32) -> String {
    match id {
        28 => "description".into(),
        36 => "eventState".into(),
        62 => "maxApduLengthAccepted".into(),
        74 => "numberOfStates".into(),
        75 => "objectIdentifier".into(),
        76 => "objectList".into(),
        77 => "objectName".into(),
        79 => "objectType".into(),
        81 => "outOfService".into(),
        85 => "presentValue".into(),
        87 => "priorityArray".into(),
        104 => "relinquishDefault".into(),
        111 => "statusFlags".into(),
        112 => "systemStatus".into(),
        117 => "units".into(),
        n => format!("property {n}"),
    }
}

/// An object identifier's label: `AI 3`, `Device 1001`, or `type 20 #4`.
fn object_label(type_number: u32, instance: u32) -> String {
    match ObjectKind::from_number(type_number) {
        Some(kind) => Object { kind, instance }.label(),
        None if type_number == 8 => format!("Device {instance}"),
        None => format!("type {type_number} #{instance}"),
    }
}

/// One primitive or constructed tag of ASN.1 BACnet data (clause 20.2.1).
struct Tag<'a> {
    number: u8,
    context: bool,
    opening: bool,
    closing: bool,
    content: &'a [u8],
}

/// The tags of `data`, in order; stops at the first malformed one.
fn tags(data: &[u8]) -> impl Iterator<Item = Tag<'_>> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let first = *data.get(at)?;
        let mut header = 1;
        let number = if first >> 4 == 0x0F {
            header += 1;
            *data.get(at + 1)?
        } else {
            first >> 4
        };
        let context = first & 0x08 != 0;
        let lvt = first & 0x07;
        let (opening, closing) = (context && lvt == 6, context && lvt == 7);
        let len = if opening || closing {
            0
        } else if !context && number == 1 {
            // An application Boolean carries its value in the length bits.
            0
        } else if lvt == 5 {
            let extended = usize::from(*data.get(at + header)?);
            header += 1;
            match extended {
                254 => {
                    let bytes = data.get(at + header..at + header + 2)?;
                    header += 2;
                    usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
                }
                255 => return None,
                n => n,
            }
        } else {
            usize::from(lvt)
        };
        let content = data.get(at + header..at + header + len)?;
        at += header + len;
        Some(Tag {
            number,
            context,
            opening,
            closing,
            content,
        })
    })
}

fn unsigned(content: &[u8]) -> Option<u32> {
    (!content.is_empty() && content.len() <= 4)
        .then(|| content.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)))
}

/// `(type, instance)` of a 4-octet object identifier.
fn object_id(content: &[u8]) -> Option<(u32, u32)> {
    let id = u32::from_be_bytes(content.try_into().ok()?);
    Some((id >> 22, id & 0x3F_FFFF))
}

/// What a confirmed request asks for, from its service data.
fn request_detail(service: u8, data: &[u8]) -> String {
    let top: Vec<Tag<'_>> = {
        // Only the outermost tags: skip whatever sits inside opening/closing pairs.
        let mut depth = 0usize;
        tags(data)
            .filter(|tag| {
                if tag.opening {
                    depth += 1;
                    false
                } else if tag.closing {
                    depth = depth.saturating_sub(1);
                    false
                } else {
                    depth == 0
                }
            })
            .collect()
    };
    let context = |number: u8| {
        top.iter()
            .find(|tag| tag.context && tag.number == number)
            .map(|tag| tag.content)
    };
    match service {
        // ReadProperty / WriteProperty: [0] object, [1] property, [4] priority (WP).
        12 | 15 => {
            let mut text = String::new();
            if let Some((kind, instance)) = context(0).and_then(object_id) {
                text.push_str(&object_label(kind, instance));
            }
            if let Some(property) = context(1).and_then(unsigned) {
                let _ = write!(text, " {}", property_name(property));
            }
            if service == 15
                && let Some(priority) = context(4).and_then(unsigned)
            {
                let _ = write!(text, " @ priority {priority}");
            }
            text
        }
        // ReadPropertyMultiple: one [0] object per access specification.
        14 => {
            let objects = top
                .iter()
                .filter(|tag| tag.context && tag.number == 0)
                .count();
            format!("{objects} object{}", if objects == 1 { "" } else { "s" })
        }
        // SubscribeCOV: [0] process, [1] object, [2] confirmed, [3] lifetime.
        5 => {
            let mut text = context(1)
                .and_then(object_id)
                .map(|(kind, instance)| object_label(kind, instance))
                .unwrap_or_default();
            match context(3).and_then(unsigned) {
                Some(lifetime) => {
                    let _ = write!(text, " · lifetime {lifetime} s");
                }
                None if context(2).is_none() => text.push_str(" · cancel"),
                None => {}
            }
            text
        }
        _ => String::new(),
    }
}

/// The values of a COV notification, e.g. `presentValue 22.4 · statusFlags normal`.
fn notification_detail(frame: &[u8]) -> Option<(String, String)> {
    let packet = mt_bacnet::parse_cov_packet(frame)?;
    let notification = packet.notification;
    let object = ObjectKind::from_name(&notification.monitored_object_type).map_or_else(
        || {
            format!(
                "{} {}",
                notification.monitored_object_type, notification.monitored_object_instance
            )
        },
        |kind| {
            Object {
                kind,
                instance: notification.monitored_object_instance,
            }
            .label()
        },
    );
    let values = notification
        .values
        .iter()
        .map(|value| format!("{} {}", value.property_name, value.raw_value))
        .collect::<Vec<_>>()
        .join(" · ");
    Some((object, values))
}

/// Summary and detail of one BACnet/IP datagram for the traffic monitor, the
/// way a protocol analyser lists it.
pub(crate) fn describe_frame(frame: &[u8]) -> (String, String) {
    let Some(apdu) = mt_bacnet::transport::apdu_of_frame(frame) else {
        return match frame {
            [0x81, 0x00, _, _, hi, lo, ..] => (
                "BVLC-Result".into(),
                format!("code {}", u16::from_be_bytes([*hi, *lo])),
            ),
            _ => ("Not BACnet/IP".into(), format!("{} bytes", frame.len())),
        };
    };
    let byte = |index: usize| apdu.get(index).copied().unwrap_or_default();
    let segmented = byte(0) & 0x08 != 0;
    match byte(0) >> 4 {
        0 if apdu.len() < if segmented { 6 } else { 4 } => {
            ("Confirmed-Request".into(), "truncated".into())
        }
        0 => {
            let (service, data) = if segmented {
                (byte(5), apdu.get(6..).unwrap_or_default())
            } else {
                (byte(3), apdu.get(4..).unwrap_or_default())
            };
            if service == 1
                && let Some((object, values)) = notification_detail(frame)
            {
                return (format!("{} {object}", confirmed_service(service)), values);
            }
            let mut detail = request_detail(service, data);
            if !detail.is_empty() {
                detail.push_str(" · ");
            }
            let _ = write!(detail, "invoke {}", byte(2));
            (confirmed_service(service), detail)
        }
        1 => {
            let service = byte(1);
            let data = apdu.get(2..).unwrap_or_default();
            match service {
                0 => {
                    let device = tags(data)
                        .next()
                        .and_then(|tag| object_id(tag.content))
                        .map(|(_, instance)| format!("device {instance}"))
                        .unwrap_or_default();
                    (format!("I-Am {device}"), String::new())
                }
                2 => match notification_detail(frame) {
                    Some((object, values)) => {
                        (format!("{} {object}", unconfirmed_service(service)), values)
                    }
                    None => (unconfirmed_service(service), String::new()),
                },
                8 => {
                    let limits: Vec<u32> =
                        tags(data).filter_map(|tag| unsigned(tag.content)).collect();
                    let detail = match limits.as_slice() {
                        [low, high] => format!("instances {low}–{high}"),
                        _ => "every device".into(),
                    };
                    (unconfirmed_service(service), detail)
                }
                _ => (unconfirmed_service(service), String::new()),
            }
        }
        2 => (
            format!("SimpleAck {}", confirmed_service(byte(2))),
            format!("invoke {}", byte(1)),
        ),
        3 => {
            let service = if segmented { byte(4) } else { byte(2) };
            let detail = if segmented {
                format!("invoke {} · segment {}", byte(1), byte(2))
            } else {
                format!("invoke {}", byte(1))
            };
            (format!("ComplexAck {}", confirmed_service(service)), detail)
        }
        4 => ("SegmentAck".into(), format!("invoke {}", byte(1))),
        5 => {
            let codes: Vec<u32> = tags(apdu.get(3..).unwrap_or_default())
                .filter_map(|tag| unsigned(tag.content))
                .collect();
            let detail = match codes.as_slice() {
                [class, code, ..] => {
                    format!("{} / {}", error_class_name(*class), error_code_name(*code))
                }
                _ => String::new(),
            };
            (format!("Error {}", confirmed_service(byte(2))), detail)
        }
        6 => ("Reject".into(), reject_reason_name(byte(2))),
        7 => ("Abort".into(), abort_reason_name(byte(2))),
        _ => ("APDU".into(), format!("{} bytes", apdu.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_flags_read_the_named_form() {
        assert_eq!(StatusFlags::parse("normal"), Some(StatusFlags::default()));
        let flags = StatusFlags::parse("in-alarm, out-of-service").expect("two flags");
        assert!(flags.in_alarm && flags.out_of_service);
        assert!(!flags.fault && !flags.overridden);
        assert_eq!(flags.describe(), "in-alarm, out-of-service");
        assert_eq!(StatusFlags::parse(&flags.describe()), Some(flags));
        // A bit-string display is not the named form.
        assert_eq!(StatusFlags::parse("Bits(0100)"), None);
        assert_eq!(StatusFlags::parse(""), None);
    }

    fn answer(instance: u32, ip: &str, port: u16) -> mt_bacnet::BacnetDiscoveredDevice {
        mt_bacnet::BacnetDiscoveredDevice {
            device_instance: instance,
            ip: ip.into(),
            port,
            vendor_name: Some("ASHRAE".into()),
            model_name: None,
            firmware_version: None,
            object_list_count: None,
        }
    }

    #[test]
    fn each_device_is_listed_once_at_the_address_asked() {
        let found = vec![
            answer(1001, "192.168.64.2", 47808),
            answer(7, "10.0.0.9", 47808),
            answer(1001, "127.0.0.1", 47808),
            answer(1001, "10.0.0.5", 47808),
        ];
        // Nothing asked directly: loopback wins over the LAN addresses.
        let devices = one_per_device(found.clone(), &[]);
        assert_eq!(devices.len(), 2);
        assert_eq!((devices[0].instance, devices[1].instance), (7, 1001));
        assert_eq!(devices[1].address, "127.0.0.1");
        assert_eq!(devices[1].also, ["10.0.0.5:47808", "192.168.64.2:47808"]);
        // The typed host wins over loopback.
        let typed: SocketAddr = "192.168.64.2:47808".parse().unwrap();
        let devices = one_per_device(found, &[typed]);
        assert_eq!(devices[1].address, "192.168.64.2");
        assert_eq!(devices[1].also, ["127.0.0.1:47808", "10.0.0.5:47808"]);
        assert!(devices[0].also.is_empty());
    }

    #[test]
    fn object_kinds_cover_the_point_types_only() {
        for kind in ObjectKind::ALL {
            assert_eq!(ObjectKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ObjectKind::from_name("device"), None);
        assert_eq!(ObjectKind::from_name("trendLog"), None);
        assert_eq!(
            ObjectKind::from_number(19),
            Some(ObjectKind::MultiStateValue)
        );
        assert_eq!(object_label(8, 1001), "Device 1001");
        assert_eq!(object_label(20, 4), "type 20 #4");
    }

    #[test]
    fn frames_read_like_an_analyser() {
        // WriteProperty AV 1 presentValue = Real 21.5 @ priority 8, invoke 7.
        let write = mt_bacnet::transport::unicast_frame(
            &[
                0x00, 0x05, 0x07, 0x0F, // confirmed request, invoke 7, WriteProperty
                0x0C, 0x00, 0x80, 0x00, 0x01, // [0] analog-value 1
                0x19, 0x55, // [1] presentValue
                0x3E, 0x44, 0x41, 0xAC, 0x00, 0x00, 0x3F, // [3] Real 21.5
                0x49, 0x08, // [4] priority 8
            ],
            true,
        );
        assert_eq!(
            describe_frame(&write),
            (
                "WriteProperty".into(),
                "AV 1 presentValue @ priority 8 · invoke 7".into()
            )
        );

        // An Error answer: property / write-access-denied.
        let error =
            mt_bacnet::transport::unicast_frame(&[0x50, 0x07, 0x0F, 0x91, 0x02, 0x91, 0x28], false);
        assert_eq!(
            describe_frame(&error),
            (
                "Error WriteProperty".into(),
                "property / write-access-denied".into()
            )
        );

        // A ReadPropertyMultiple of two objects: tags inside the lists don't count.
        let rpm = mt_bacnet::transport::unicast_frame(
            &[
                0x00, 0x05, 0x02, 0x0E, //
                0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x09, 0x55, 0x09, 0x6F, 0x1F, //
                0x0C, 0x00, 0xC0, 0x00, 0x02, 0x1E, 0x09, 0x55, 0x1F,
            ],
            true,
        );
        assert_eq!(
            describe_frame(&rpm),
            ("ReadPropertyMultiple".into(), "2 objects · invoke 2".into())
        );

        // An I-Am from device 1001, and a Who-Is with a range.
        let i_am = mt_bacnet::transport::unicast_frame(
            &[
                0x10, 0x00, 0xC4, 0x02, 0x00, 0x03, 0xE9, 0x22, 0x05, 0xC4, 0x91, 0x03, 0x21, 0x00,
            ],
            false,
        );
        assert_eq!(describe_frame(&i_am).0, "I-Am device 1001");
        let who_is =
            mt_bacnet::transport::unicast_frame(&[0x10, 0x08, 0x09, 0x01, 0x1A, 0x03, 0xE8], false);
        assert_eq!(
            describe_frame(&who_is),
            ("Who-Is".into(), "instances 1–1000".into())
        );

        // Truncated or foreign datagrams never panic.
        assert_eq!(describe_frame(&[0x81]).0, "Not BACnet/IP");
        assert_eq!(
            describe_frame(&[0x81, 0x00, 0x00, 0x06, 0x00, 0x30]).0,
            "BVLC-Result"
        );
        let cut = mt_bacnet::transport::unicast_frame(&[0x00, 0x05], true);
        assert_eq!(describe_frame(&cut).0, "Confirmed-Request");
    }
}
