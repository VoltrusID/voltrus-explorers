//! Object/property model of the simulated device (ASHRAE 135 clause 12):
//! which properties each object supports, their values and datatypes, and how
//! WriteProperty is applied (including the priority array of commandable objects).

use bacnet_rs::object::{ObjectIdentifier, ObjectType};

use super::wire::*;
use super::{DeviceInfo, Kind, SimObj, COV_INCREMENT};

// ── BACnetPropertyIdentifier values (clause 21) ──
const P_ALL: u32 = 8;
const P_APDU_TIMEOUT: u32 = 11;
const P_APPLICATION_SOFTWARE_VERSION: u32 = 12;
const P_COV_INCREMENT: u32 = 22;
const P_DESCRIPTION: u32 = 28;
const P_DEVICE_ADDRESS_BINDING: u32 = 30;
const P_EVENT_STATE: u32 = 36;
const P_FIRMWARE_REVISION: u32 = 44;
const P_MAX_APDU_LENGTH_ACCEPTED: u32 = 62;
const P_MODEL_NAME: u32 = 70;
const P_NUMBER_OF_APDU_RETRIES: u32 = 73;
const P_NUMBER_OF_STATES: u32 = 74;
const P_OBJECT_IDENTIFIER: u32 = 75;
const P_OBJECT_LIST: u32 = 76;
const P_OBJECT_NAME: u32 = 77;
const P_OBJECT_TYPE: u32 = 79;
const P_OPTIONAL: u32 = 80;
const P_OUT_OF_SERVICE: u32 = 81;
const P_POLARITY: u32 = 84;
pub(super) const P_PRESENT_VALUE: u32 = 85;
const P_PRIORITY_ARRAY: u32 = 87;
const P_PROTOCOL_OBJECT_TYPES_SUPPORTED: u32 = 96;
const P_PROTOCOL_SERVICES_SUPPORTED: u32 = 97;
const P_PROTOCOL_VERSION: u32 = 98;
const P_RELINQUISH_DEFAULT: u32 = 104;
const P_REQUIRED: u32 = 105;
const P_SEGMENTATION_SUPPORTED: u32 = 107;
pub(super) const P_STATUS_FLAGS: u32 = 111;
const P_SYSTEM_STATUS: u32 = 112;
const P_UNITS: u32 = 117;
const P_VENDOR_IDENTIFIER: u32 = 120;
const P_VENDOR_NAME: u32 = 121;
const P_PROTOCOL_REVISION: u32 = 139;
const P_DATABASE_REVISION: u32 = 155;

/// Instance number a client may use to address "this device" (clause 12.11.1).
const DEVICE_WILDCARD_INSTANCE: u32 = 0x003F_FFFF;

const DEVICE_REQUIRED: [u32; 20] = [
    P_OBJECT_IDENTIFIER,
    P_OBJECT_NAME,
    P_OBJECT_TYPE,
    P_SYSTEM_STATUS,
    P_VENDOR_NAME,
    P_VENDOR_IDENTIFIER,
    P_MODEL_NAME,
    P_FIRMWARE_REVISION,
    P_APPLICATION_SOFTWARE_VERSION,
    P_PROTOCOL_VERSION,
    P_PROTOCOL_REVISION,
    P_PROTOCOL_SERVICES_SUPPORTED,
    P_PROTOCOL_OBJECT_TYPES_SUPPORTED,
    P_OBJECT_LIST,
    P_MAX_APDU_LENGTH_ACCEPTED,
    P_SEGMENTATION_SUPPORTED,
    P_APDU_TIMEOUT,
    P_NUMBER_OF_APDU_RETRIES,
    P_DEVICE_ADDRESS_BINDING,
    P_DATABASE_REVISION,
];
const DEVICE_OPTIONAL: [u32; 1] = [P_DESCRIPTION];

/// A resolved object: this device or one of its points.
enum Target<'a> {
    Device,
    Point(&'a SimObj),
}

fn is_device(dev: DeviceInfo, oid: ObjectIdentifier) -> bool {
    oid.object_type == ObjectType::Device && (oid.instance == dev.id || oid.instance == DEVICE_WILDCARD_INSTANCE)
}

fn find<'a>(dev: DeviceInfo, objects: &'a [SimObj], oid: ObjectIdentifier) -> Result<Target<'a>, SimError> {
    if oid.object_type == ObjectType::Device {
        return if is_device(dev, oid) { Ok(Target::Device) } else { Err(ERR_UNKNOWN_OBJECT) };
    }
    objects
        .iter()
        .find(|o| o.ot == oid.object_type && o.instance == oid.instance)
        .map(Target::Point)
        .ok_or(ERR_UNKNOWN_OBJECT)
}

/// The identifier to echo in a response: the device wildcard becomes the real device id.
pub(super) fn canonical(dev: DeviceInfo, oid: ObjectIdentifier) -> ObjectIdentifier {
    if is_device(dev, oid) {
        ObjectIdentifier::new(ObjectType::Device, dev.id)
    } else {
        oid
    }
}

pub(super) fn is_this_device(dev: DeviceInfo, oid: ObjectIdentifier) -> bool {
    is_device(dev, oid)
}

fn point_property_lists(o: &SimObj) -> (Vec<u32>, Vec<u32>) {
    let mut required = vec![
        P_OBJECT_IDENTIFIER,
        P_OBJECT_NAME,
        P_OBJECT_TYPE,
        P_PRESENT_VALUE,
        P_STATUS_FLAGS,
        P_EVENT_STATE,
        P_OUT_OF_SERVICE,
    ];
    let mut optional = vec![P_DESCRIPTION];
    match o.kind {
        Kind::Analog => {
            required.push(P_UNITS);
            optional.push(P_COV_INCREMENT);
        }
        // Binary_Value has no Polarity property.
        Kind::Binary if o.ot != ObjectType::BinaryValue => required.push(P_POLARITY),
        Kind::Binary => {}
        Kind::MultiState => required.push(P_NUMBER_OF_STATES),
    }
    if o.commandable {
        required.push(P_PRIORITY_ARRAY);
        required.push(P_RELINQUISH_DEFAULT);
    }
    (required, optional)
}

fn point_supports(o: &SimObj, pid: u32) -> bool {
    let (required, optional) = point_property_lists(o);
    required.contains(&pid) || optional.contains(&pid)
}

/// Properties to report for `pid` in a ReadPropertyMultiple request: the
/// special identifiers ALL / REQUIRED / OPTIONAL (clause 15.7.3.1.1) expand to
/// the object's property lists; anything else (or an unknown object) is
/// passed through unchanged.
pub(super) fn expand(dev: DeviceInfo, objects: &[SimObj], oid: ObjectIdentifier, pid: u32) -> Vec<u32> {
    if !matches!(pid, P_ALL | P_REQUIRED | P_OPTIONAL) {
        return vec![pid];
    }
    let (required, optional) = match find(dev, objects, oid) {
        Ok(Target::Device) => (DEVICE_REQUIRED.to_vec(), DEVICE_OPTIONAL.to_vec()),
        Ok(Target::Point(o)) => point_property_lists(o),
        Err(_) => return vec![pid],
    };
    match pid {
        P_REQUIRED => required,
        P_OPTIONAL => optional,
        _ => required.into_iter().chain(optional).collect(),
    }
}

/// Is `pid` one of the special identifiers that carries no array index?
pub(super) fn is_special(pid: u32) -> bool {
    matches!(pid, P_ALL | P_REQUIRED | P_OPTIONAL)
}

// ── Reading ──

pub(super) fn read_property(
    dev: DeviceInfo,
    objects: &[SimObj],
    oid: ObjectIdentifier,
    pid: u32,
    index: Option<u32>,
) -> Result<Vec<Val>, SimError> {
    match find(dev, objects, oid)? {
        Target::Device => device_property(dev, objects, pid, index),
        Target::Point(o) => point_property(o, pid, index),
    }
}

fn point_property(o: &SimObj, pid: u32, index: Option<u32>) -> Result<Vec<Val>, SimError> {
    if !point_supports(o, pid) {
        return Err(ERR_UNKNOWN_PROPERTY);
    }
    if pid == P_PRIORITY_ARRAY {
        let slot = |i: usize| o.slots[i].map_or(Val::Null, |v| o.typed(v));
        return match index {
            None => Ok((0..16).map(slot).collect()),
            Some(0) => Ok(vec![Val::Unsigned(16)]),
            Some(i @ 1..=16) => Ok(vec![slot(i as usize - 1)]),
            Some(_) => Err(ERR_INVALID_ARRAY_INDEX),
        };
    }
    if index.is_some() {
        return Err(ERR_NOT_AN_ARRAY);
    }
    let value = match pid {
        P_OBJECT_IDENTIFIER => Val::Oid(ObjectIdentifier::new(o.ot, o.instance)),
        P_OBJECT_NAME | P_DESCRIPTION => Val::Str(o.label.clone()),
        P_OBJECT_TYPE => Val::Enum(u32::from(o.ot)),
        P_PRESENT_VALUE => o.typed(o.current),
        // in-alarm, fault, overridden, out-of-service
        P_STATUS_FLAGS => Val::Bits(vec![false, false, false, o.out_of_service]),
        P_EVENT_STATE => Val::Enum(0), // normal
        P_OUT_OF_SERVICE => Val::Bool(o.out_of_service),
        P_UNITS => Val::Enum(o.units_enum),
        P_COV_INCREMENT => Val::Real(COV_INCREMENT as f32),
        P_POLARITY => Val::Enum(0), // normal
        P_NUMBER_OF_STATES => Val::Unsigned(u64::from(o.states)),
        P_RELINQUISH_DEFAULT => o.typed(o.base),
        _ => return Err(ERR_UNKNOWN_PROPERTY),
    };
    Ok(vec![value])
}

fn device_property(dev: DeviceInfo, objects: &[SimObj], pid: u32, index: Option<u32>) -> Result<Vec<Val>, SimError> {
    if !DEVICE_REQUIRED.contains(&pid) && !DEVICE_OPTIONAL.contains(&pid) {
        return Err(ERR_UNKNOWN_PROPERTY);
    }
    if pid == P_OBJECT_LIST {
        // index 0 → count; index N → the Nth object id; no index → whole list.
        let ids: Vec<ObjectIdentifier> = std::iter::once(ObjectIdentifier::new(ObjectType::Device, dev.id))
            .chain(objects.iter().map(|o| ObjectIdentifier::new(o.ot, o.instance)))
            .collect();
        return match index {
            None => Ok(ids.into_iter().map(Val::Oid).collect()),
            Some(0) => Ok(vec![Val::Unsigned(ids.len() as u64)]),
            Some(i) if (i as usize) <= ids.len() => Ok(vec![Val::Oid(ids[i as usize - 1])]),
            Some(_) => Err(ERR_INVALID_ARRAY_INDEX),
        };
    }
    if index.is_some() {
        return Err(ERR_NOT_AN_ARRAY);
    }
    let value = match pid {
        P_OBJECT_IDENTIFIER => Val::Oid(ObjectIdentifier::new(ObjectType::Device, dev.id)),
        P_OBJECT_NAME => Val::Str(format!("SimDevice-{}", dev.id)),
        P_OBJECT_TYPE => Val::Enum(u32::from(ObjectType::Device)),
        P_DESCRIPTION => Val::Str("MacTools BACnet Simulator".into()),
        P_SYSTEM_STATUS => Val::Enum(0), // operational
        P_VENDOR_NAME => Val::Str("JoyoDigitama".into()),
        P_VENDOR_IDENTIFIER => Val::Unsigned(u64::from(dev.vendor_id)),
        P_MODEL_NAME => Val::Str("Sim".into()),
        P_FIRMWARE_REVISION | P_APPLICATION_SOFTWARE_VERSION => Val::Str("1.0.0".into()),
        P_PROTOCOL_VERSION => Val::Unsigned(1),
        P_PROTOCOL_REVISION => Val::Unsigned(5),
        P_PROTOCOL_SERVICES_SUPPORTED => Val::Bits(services_supported()),
        P_PROTOCOL_OBJECT_TYPES_SUPPORTED => Val::Bits(object_types_supported()),
        P_MAX_APDU_LENGTH_ACCEPTED => Val::Unsigned(MAX_APDU_ACCEPTED as u64),
        P_SEGMENTATION_SUPPORTED => Val::Enum(3), // no-segmentation
        P_APDU_TIMEOUT => Val::Unsigned(super::cov::APDU_TIMEOUT.as_millis() as u64),
        P_NUMBER_OF_APDU_RETRIES => Val::Unsigned(u64::from(super::cov::APDU_RETRIES)),
        P_DEVICE_ADDRESS_BINDING => return Ok(Vec::new()), // empty list
        P_DATABASE_REVISION => Val::Unsigned(1),
        _ => return Err(ERR_UNKNOWN_PROPERTY),
    };
    Ok(vec![value])
}

/// BACnetServicesSupported (clause 21): bit positions of the services the
/// simulator executes or initiates.
fn services_supported() -> Vec<bool> {
    const BITS: [usize; 8] = [
        1,  // confirmedCOVNotification (initiated)
        5,  // subscribeCOV
        12, // readProperty
        14, // readPropertyMultiple
        15, // writeProperty
        26, // i-Am
        28, // unconfirmedCOVNotification (initiated)
        34, // who-Is
    ];
    let mut v = vec![false; 40];
    for b in BITS {
        v[b] = true;
    }
    v
}

/// BACnetObjectTypesSupported: bit N = object type N (clause 21).
fn object_types_supported() -> Vec<bool> {
    const TYPES: [usize; 10] = [
        0,  // analog-input
        1,  // analog-output
        2,  // analog-value
        3,  // binary-input
        4,  // binary-output
        5,  // binary-value
        8,  // device
        13, // multi-state-input
        14, // multi-state-output
        19, // multi-state-value
    ];
    let mut v = vec![false; 32];
    for t in TYPES {
        v[t] = true;
    }
    v
}

// ── Writing ──

/// Apply a WriteProperty request. `value` is the raw content of the [3]
/// opening/closing pair. On a Present_Value write returns the point's key and
/// resulting effective value so the caller can report it.
pub(super) fn write_property(
    dev: DeviceInfo,
    objects: &mut [SimObj],
    oid: ObjectIdentifier,
    pid: u32,
    index: Option<u32>,
    value: &[u8],
    priority: Option<u8>,
) -> Result<Option<(String, f64)>, SimError> {
    if oid.object_type == ObjectType::Device {
        if !is_device(dev, oid) {
            return Err(ERR_UNKNOWN_OBJECT);
        }
        if !DEVICE_REQUIRED.contains(&pid) && !DEVICE_OPTIONAL.contains(&pid) {
            return Err(ERR_UNKNOWN_PROPERTY);
        }
        if index.is_some() && pid != P_OBJECT_LIST {
            return Err(ERR_NOT_AN_ARRAY);
        }
        return Err(ERR_WRITE_ACCESS_DENIED);
    }
    let o = objects
        .iter_mut()
        .find(|o| o.ot == oid.object_type && o.instance == oid.instance)
        .ok_or(ERR_UNKNOWN_OBJECT)?;
    if !point_supports(o, pid) {
        return Err(ERR_UNKNOWN_PROPERTY);
    }
    if index.is_some() {
        // Priority_Array is the only array here and is read-only.
        return Err(if pid == P_PRIORITY_ARRAY { ERR_WRITE_ACCESS_DENIED } else { ERR_NOT_AN_ARRAY });
    }
    match pid {
        P_PRESENT_VALUE => {
            write_present_value(o, value, priority)?;
            Ok(Some((o.key.clone(), o.current)))
        }
        P_OUT_OF_SERVICE => match single_app_value(value) {
            Some((1, v)) => {
                o.out_of_service = v[0] == 1;
                Ok(None)
            }
            _ => Err(ERR_INVALID_DATA_TYPE),
        },
        _ => Err(ERR_WRITE_ACCESS_DENIED),
    }
}

fn write_present_value(o: &mut SimObj, value: &[u8], priority: Option<u8>) -> Result<(), SimError> {
    // Inputs only accept a Present_Value write while Out_Of_Service (clause 12.1).
    if !o.commandable && !o.out_of_service {
        return Err(ERR_WRITE_ACCESS_DENIED);
    }
    let (tag, content) = single_app_value(value).ok_or(ERR_INVALID_DATA_TYPE)?;
    // Null relinquishes a command, which only exists for commandable objects.
    let new = if tag == 0 {
        if !o.commandable {
            return Err(ERR_INVALID_DATA_TYPE);
        }
        None
    } else {
        Some(decode_value(o, tag, &content)?)
    };
    if !o.commandable {
        if let Some(v) = new {
            o.current = v;
        }
        return Ok(());
    }
    let prio = priority.unwrap_or(16);
    // Priority 6 is reserved for Minimum_On/Off_Time and may not be commanded by clients.
    if prio == 6 {
        return Err(ERR_WRITE_ACCESS_DENIED);
    }
    o.slots[usize::from(prio) - 1] = new;
    o.resolve();
    Ok(())
}

/// Decode a written Present_Value of application tag `tag`: analog objects take
/// a Real within the object's min/max, binary objects an Enumerated 0/1,
/// multi-state objects an Unsigned 1..=Number_Of_States.
fn decode_value(o: &SimObj, tag: u32, content: &[u8]) -> Result<f64, SimError> {
    let as_u32 = |c: &[u8]| -> Option<u32> {
        (!c.is_empty() && c.len() <= 4).then(|| c.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b)))
    };
    match o.kind {
        Kind::Analog => {
            let raw: [u8; 4] = content.try_into().map_err(|_| ERR_INVALID_DATA_TYPE)?;
            if tag != 4 {
                return Err(ERR_INVALID_DATA_TYPE);
            }
            let v = f64::from(f32::from_be_bytes(raw));
            if !v.is_finite() || o.min.is_some_and(|m| v < m) || o.max.is_some_and(|m| v > m) {
                return Err(ERR_VALUE_OUT_OF_RANGE);
            }
            Ok(v)
        }
        Kind::Binary => {
            let v = as_u32(content).filter(|_| tag == 9).ok_or(ERR_INVALID_DATA_TYPE)?;
            if v > 1 {
                return Err(ERR_VALUE_OUT_OF_RANGE);
            }
            Ok(f64::from(v))
        }
        Kind::MultiState => {
            let v = as_u32(content).filter(|_| tag == 2).ok_or(ERR_INVALID_DATA_TYPE)?;
            if v == 0 || v > o.states {
                return Err(ERR_VALUE_OUT_OF_RANGE);
            }
            Ok(f64::from(v))
        }
    }
}
