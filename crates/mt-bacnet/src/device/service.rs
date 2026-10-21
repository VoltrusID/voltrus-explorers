//! Datagram → response: BVLC (Annex J), NPDU (clause 6) and APDU (clause 5/20)
//! handling plus the services the simulated device implements: Who-Is / I-Am,
//! ReadProperty, ReadPropertyMultiple, WriteProperty and SubscribeCOV.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Instant;

use bacnet_rs::object::{ObjectIdentifier, ObjectType};

use super::cov::{CovState, Subscription};
use super::props;
use super::wire::*;
use super::{DeviceInfo, SimObj};

// Confirmed service choices (clause 21, BACnetConfirmedServiceChoice).
const SVC_SUBSCRIBE_COV: u8 = 5;
const SVC_READ_PROPERTY: u8 = 12;
const SVC_READ_PROPERTY_MULTIPLE: u8 = 14;
const SVC_WRITE_PROPERTY: u8 = 15;
// Unconfirmed service choices (BACnetUnconfirmedServiceChoice).
const UNCONF_I_AM: u8 = 0;
const UNCONF_WHO_IS: u8 = 8;

/// BVLC-Result codes for BBMD functions this (non-BBMD) device does not implement (Annex J.2.1).
const NAK_WRITE_BDT: u16 = 0x0010;
const NAK_READ_BDT: u16 = 0x0020;
const NAK_REGISTER_FOREIGN_DEVICE: u16 = 0x0030;
const NAK_READ_FDT: u16 = 0x0040;
const NAK_DELETE_FDT_ENTRY: u16 = 0x0050;
const NAK_DISTRIBUTE_BROADCAST_TO_NETWORK: u16 = 0x0060;

/// Highest valid object instance number (clause 20.2.14).
const MAX_INSTANCE: u32 = 0x003F_FFFF;

/// Segmentation_Supported = no-segmentation (BACnetSegmentation).
const SEGMENTATION_NONE: u32 = 3;

/// Outcome of a confirmed service, before it is framed.
enum Resp {
    SimpleAck,
    ComplexAck(Vec<u8>),
    Error(SimError),
    Reject(u8),
}

/// Mutable device state a request is processed against.
struct Env<'a> {
    dev: DeviceInfo,
    objects: &'a mut [SimObj],
    cov: &'a mut CovState,
    now: Instant,
    on_write: &'a dyn Fn(String, f64),
}

/// Process one received datagram and return the datagram to send back, if any.
pub(super) fn handle_packet(
    data: &[u8],
    src: SocketAddr,
    dev: DeviceInfo,
    objects: &mut [SimObj],
    cov: &mut CovState,
    now: Instant,
    on_write: &dyn Fn(String, f64),
) -> Option<Reply> {
    // BVLC header: type, function, total length. The length must match the datagram (J.2).
    if data.len() < 4 || data[0] != 0x81 || usize::from(u16::from_be_bytes([data[2], data[3]])) != data.len() {
        return None;
    }
    let nak = |code: u16| Some(Reply { to: src, frame: bvlc_result(code) });
    let (origin, npdu, broadcast) = match data[1] {
        BVLC_ORIGINAL_UNICAST => (src, &data[4..], false),
        BVLC_ORIGINAL_BROADCAST => (src, &data[4..], true),
        // Forwarded-NPDU: 6-octet B/IP address of the originating device; replies go there (Annex J.2.5).
        BVLC_FORWARDED_NPDU => {
            if data.len() < 10 {
                return None;
            }
            let ip = Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            let port = u16::from_be_bytes([data[8], data[9]]);
            (SocketAddr::from((ip, port)), &data[10..], false)
        }
        // BBMD / foreign-device functions: this device is not a BBMD, so it answers with the NAK (Annex J.2.1).
        BVLC_WRITE_BDT => return nak(NAK_WRITE_BDT),
        BVLC_READ_BDT => return nak(NAK_READ_BDT),
        BVLC_REGISTER_FOREIGN_DEVICE => return nak(NAK_REGISTER_FOREIGN_DEVICE),
        BVLC_READ_FDT => return nak(NAK_READ_FDT),
        BVLC_DELETE_FDT_ENTRY => return nak(NAK_DELETE_FDT_ENTRY),
        BVLC_DISTRIBUTE_BROADCAST_TO_NETWORK => return nak(NAK_DISTRIBUTE_BROADCAST_TO_NETWORK),
        _ => return None,
    };

    let hdr = parse_npdu(npdu)?;
    // Network-layer messages (router discovery, ...) are not for an end device.
    if hdr.network_message {
        return None;
    }
    // A message with a DNET other than the global broadcast is addressed to a
    // remote network; only routers act on those.
    let global_broadcast = match &hdr.dest {
        None => false,
        Some(d) if d.net == 0xFFFF && d.adr.is_empty() => true,
        Some(_) => return None,
    };
    let apdu = npdu.get(hdr.len..).filter(|a| !a.is_empty())?;
    let priority = hdr.priority;
    let peer = Peer { addr: origin, net: hdr.source };
    let mut env = Env { dev, objects, cov, now, on_write };

    match apdu[0] >> 4 {
        // Confirmed requests are directed; one that arrives as a broadcast is dropped, never answered.
        PDU_CONFIRMED_REQUEST if broadcast || global_broadcast => None,
        PDU_CONFIRMED_REQUEST => handle_confirmed(apdu, &peer, priority, &mut env),
        PDU_UNCONFIRMED_REQUEST => handle_unconfirmed(apdu, &peer, priority, dev),
        t @ (PDU_SIMPLE_ACK | PDU_ERROR | PDU_REJECT | PDU_ABORT) => {
            // Answers to our confirmed COV notifications.
            if let Some(&invoke_id) = apdu.get(1) {
                env.cov.on_response(peer.addr, invoke_id, t == PDU_SIMPLE_ACK);
            }
            None
        }
        _ => None,
    }
}

// ── Unconfirmed: Who-Is → I-Am ──

fn handle_unconfirmed(apdu: &[u8], peer: &Peer, priority: u8, dev: DeviceInfo) -> Option<Reply> {
    if apdu.len() < 2 || apdu[1] != UNCONF_WHO_IS {
        return None;
    }
    // A malformed Who-Is cannot be rejected (unconfirmed): it is ignored.
    let range = decode_who_is(&apdu[2..]).ok()?;
    if let Some((low, high)) = range {
        if !(low..=high).contains(&dev.id) {
            return None;
        }
    }
    // Answer directly to the requester (its address is known from the datagram).
    let mut iam = vec![PDU_UNCONFIRMED_REQUEST << 4, UNCONF_I_AM];
    Val::Oid(ObjectIdentifier::new(ObjectType::Device, dev.id)).encode(&mut iam);
    Val::Unsigned(MAX_APDU_ACCEPTED as u64).encode(&mut iam);
    Val::Enum(SEGMENTATION_NONE).encode(&mut iam);
    Val::Unsigned(u64::from(dev.vendor_id)).encode(&mut iam);
    Some(Reply { to: peer.addr, frame: frame_to(peer, &iam, false, priority) })
}

/// Who-Is parameters (clause 16.10.1): both range limits or neither, inclusive.
fn decode_who_is(data: &[u8]) -> Decoded<Option<(u32, u32)>> {
    if data.is_empty() {
        return Ok(None);
    }
    let mut r = Reader::new(data);
    let low = r.ctx_unsigned(0)?;
    let high = r.ctx_unsigned(1)?;
    r.finish()?;
    if low > MAX_INSTANCE || high > MAX_INSTANCE {
        return Err(REJ_PARAMETER_OUT_OF_RANGE);
    }
    Ok(Some((low, high)))
}

// ── Confirmed requests ──

/// Largest APDU a client can take, from the low nibble of the second octet of
/// a ConfirmedRequest (clause 20.1.2.5). Reserved codes are treated as the minimum.
fn client_max_apdu(code: u8) -> usize {
    match code {
        1 => 128,
        2 => 206,
        3 => 480,
        4 => 1024,
        5 => 1476,
        _ => 50,
    }
}

fn handle_confirmed(apdu: &[u8], peer: &Peer, priority: u8, env: &mut Env) -> Option<Reply> {
    // type/flags, max-segments/max-APDU, invoke id, service choice
    if apdu.len() < 4 {
        return None;
    }
    let invoke_id = apdu[2];
    let respond = |pdu: Vec<u8>| Some(Reply { to: peer.addr, frame: frame_to(peer, &pdu, false, priority) });
    // The device does not support segmentation (Segmentation_Supported = no-segmentation).
    if apdu[0] & 0x08 != 0 {
        return respond(abort_pdu(invoke_id, ABORT_SEGMENTATION_NOT_SUPPORTED));
    }
    let service = apdu[3];
    let data = &apdu[4..];
    let resp = match service {
        SVC_READ_PROPERTY => read_property(data, env.dev, env.objects),
        SVC_READ_PROPERTY_MULTIPLE => read_property_multiple(data, env.dev, env.objects),
        SVC_WRITE_PROPERTY => write_property(data, env.dev, env.objects, env.on_write),
        SVC_SUBSCRIBE_COV => subscribe_cov(data, peer, env),
        _ => Resp::Reject(REJ_UNRECOGNIZED_SERVICE),
    };
    let pdu = match resp {
        Resp::SimpleAck => simple_ack_pdu(invoke_id, service),
        Resp::ComplexAck(body) => {
            let ack = complex_ack_pdu(invoke_id, service, &body);
            // A response that exceeds the client's max-APDU cannot be sent (no segmentation support).
            if ack.len() > client_max_apdu(apdu[1] & 0x0F).min(MAX_APDU_ACCEPTED) {
                abort_pdu(invoke_id, ABORT_SEGMENTATION_NOT_SUPPORTED)
            } else {
                ack
            }
        }
        Resp::Error(e) => error_pdu(invoke_id, service, e),
        Resp::Reject(reason) => reject_pdu(invoke_id, reason),
    };
    respond(pdu)
}

// ── ReadProperty ──

fn decode_read_property(data: &[u8]) -> Decoded<(ObjectIdentifier, u32, Option<u32>)> {
    let mut r = Reader::new(data);
    let oid = r.ctx_object_id(0)?;
    let pid = r.ctx_unsigned(1)?;
    let index = r.opt_ctx_unsigned(2)?;
    r.finish()?;
    Ok((oid, pid, index))
}

fn read_property(data: &[u8], dev: DeviceInfo, objects: &[SimObj]) -> Resp {
    let (oid, pid, index) = match decode_read_property(data) {
        Ok(v) => v,
        Err(reason) => return Resp::Reject(reason),
    };
    let values = match props::read_property(dev, objects, oid, pid, index) {
        Ok(v) => v,
        Err(e) => return Resp::Error(e),
    };
    // ReadProperty-ACK: [0] object id, [1] property, [2] array index (echoed), [3] value.
    let mut body = Vec::new();
    ctx_object_id(&mut body, 0, props::canonical(dev, oid));
    ctx_unsigned(&mut body, 1, u64::from(pid));
    if let Some(i) = index {
        ctx_unsigned(&mut body, 2, u64::from(i));
    }
    put_open(&mut body, 3);
    for v in &values {
        v.encode(&mut body);
    }
    put_close(&mut body, 3);
    Resp::ComplexAck(body)
}

// ── ReadPropertyMultiple ──

struct PropRef {
    pid: u32,
    index: Option<u32>,
}

struct ReadSpec {
    oid: ObjectIdentifier,
    props: Vec<PropRef>,
}

fn decode_read_property_multiple(data: &[u8]) -> Decoded<Vec<ReadSpec>> {
    let mut r = Reader::new(data);
    if r.is_empty() {
        return Err(REJ_MISSING_REQUIRED_PARAMETER);
    }
    let mut specs = Vec::new();
    while !r.is_empty() {
        let oid = r.ctx_object_id(0)?;
        r.open(1)?;
        let mut props = Vec::new();
        while !r.at_close(1)? {
            let pid = r.ctx_unsigned(0)?;
            let index = r.opt_ctx_unsigned(1)?;
            props.push(PropRef { pid, index });
        }
        r.close(1)?;
        if props.is_empty() {
            return Err(REJ_MISSING_REQUIRED_PARAMETER);
        }
        specs.push(ReadSpec { oid, props });
    }
    Ok(specs)
}

fn read_property_multiple(data: &[u8], dev: DeviceInfo, objects: &[SimObj]) -> Resp {
    let specs = match decode_read_property_multiple(data) {
        Ok(s) => s,
        Err(reason) => return Resp::Reject(reason),
    };
    // ReadAccessResult: [0] object id, [1] { [2] property, [3] index?, [4] value | [5] error }.
    let mut body = Vec::new();
    for spec in &specs {
        ctx_object_id(&mut body, 0, props::canonical(dev, spec.oid));
        put_open(&mut body, 1);
        for pr in &spec.props {
            let special = props::is_special(pr.pid);
            for pid in props::expand(dev, objects, spec.oid, pr.pid) {
                ctx_unsigned(&mut body, 2, u64::from(pid));
                let index = if special { None } else { pr.index };
                if let Some(i) = index {
                    ctx_unsigned(&mut body, 3, u64::from(i));
                }
                match props::read_property(dev, objects, spec.oid, pid, index) {
                    Ok(values) => {
                        put_open(&mut body, 4);
                        for v in &values {
                            v.encode(&mut body);
                        }
                        put_close(&mut body, 4);
                    }
                    Err(e) => {
                        put_open(&mut body, 5);
                        Val::Enum(e.class).encode(&mut body);
                        Val::Enum(e.code).encode(&mut body);
                        put_close(&mut body, 5);
                    }
                }
            }
        }
        put_close(&mut body, 1);
    }
    Resp::ComplexAck(body)
}

// ── WriteProperty ──

struct WriteReq<'a> {
    oid: ObjectIdentifier,
    pid: u32,
    index: Option<u32>,
    /// Raw octets between the [3] opening and closing tags.
    value: &'a [u8],
    priority: Option<u8>,
}

fn decode_write_property(data: &[u8]) -> Decoded<WriteReq<'_>> {
    let mut r = Reader::new(data);
    let oid = r.ctx_object_id(0)?;
    let pid = r.ctx_unsigned(1)?;
    let index = r.opt_ctx_unsigned(2)?;
    r.open(3)?;
    let value = r.capture_constructed()?;
    let priority = r.opt_ctx_unsigned(4)?;
    r.finish()?;
    // Priority is Unsigned (1..16) (clause 15.9).
    let priority = match priority {
        None => None,
        Some(p @ 1..=16) => Some(p as u8),
        Some(_) => return Err(REJ_PARAMETER_OUT_OF_RANGE),
    };
    Ok(WriteReq { oid, pid, index, value, priority })
}

fn write_property(data: &[u8], dev: DeviceInfo, objects: &mut [SimObj], on_write: &dyn Fn(String, f64)) -> Resp {
    let req = match decode_write_property(data) {
        Ok(r) => r,
        Err(reason) => return Resp::Reject(reason),
    };
    match props::write_property(dev, objects, req.oid, req.pid, req.index, req.value, req.priority) {
        Ok(changed) => {
            if let Some((key, value)) = changed {
                on_write(key, value);
            }
            Resp::SimpleAck
        }
        Err(e) => Resp::Error(e),
    }
}

// ── SubscribeCOV ──

struct SubscribeReq {
    proc_id: u32,
    object: ObjectIdentifier,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
}

fn decode_subscribe_cov(data: &[u8]) -> Decoded<SubscribeReq> {
    let mut r = Reader::new(data);
    let proc_id = r.ctx_unsigned(0)?;
    let object = r.ctx_object_id(1)?;
    let confirmed = r.opt_ctx_bool(2)?;
    let lifetime = r.opt_ctx_unsigned(3)?;
    r.finish()?;
    Ok(SubscribeReq { proc_id, object, confirmed, lifetime })
}

fn subscribe_cov(data: &[u8], peer: &Peer, env: &mut Env) -> Resp {
    let req = match decode_subscribe_cov(data) {
        Ok(r) => r,
        Err(reason) => return Resp::Reject(reason),
    };
    if props::is_this_device(env.dev, req.object) {
        return Resp::Error(ERR_SERVICE_REQUEST_DENIED);
    }
    let Some(obj) = env
        .objects
        .iter()
        .find(|o| o.ot == req.object.object_type && o.instance == req.object.instance)
    else {
        return Resp::Error(ERR_UNKNOWN_OBJECT);
    };
    // Both optional parameters absent = cancellation (clause 13.14).
    if req.confirmed.is_none() && req.lifetime.is_none() {
        env.cov.remove(peer, req.proc_id, req.object);
        return Resp::SimpleAck;
    }
    // A subscription is identified by (subscriber, process id, monitored object); resubscribing renews it.
    let params = Subscription {
        subscriber: peer.clone(),
        proc_id: req.proc_id,
        confirmed: req.confirmed.unwrap_or(false),
        lifetime: req.lifetime,
    };
    env.cov.add(env.now, env.dev.id, obj, params);
    Resp::SimpleAck
}
