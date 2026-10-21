//! Conformance tests for the simulated device: known request byte sequences go
//! in, the exact response bytes are asserted. Expected bytes are derived by
//! hand from ASHRAE 135 (encoding rules clause 20.2, PDU formats clause 20.1,
//! NPDU clause 6.2, BVLC Annex J, object/service definitions clauses 12/13/15/16)
//! and cross-checked with the framing emitted by bacnet-stack
//! (`Send_WhoIs_Global`: `81 0B 00 0C 01 20 FF FF 00 FF 10 08`).
//! A few round-trip tests also run requests/responses through the independent
//! `bacnet-rs` encoder/decoder.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::service::handle_packet;
use super::wire::*;
use super::*;

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::object::{ObjectIdentifier, PropertyIdentifier};
use bacnet_rs::property::PropertyValue;
use bacnet_rs::service::{
    ConfirmedServiceChoice, PropertyReference, ReadAccessSpecification, ReadPropertyMultipleRequest,
    ReadPropertyMultipleResponse, ReadPropertyRequest, ReadPropertyResponse, WritePropertyRequest,
};

// ── Object identifier octets: (type << 22) | instance (clause 20.2.14) ──
const DEV: [u8; 4] = [0x02, 0x00, 0x03, 0xDF]; // device 991
const DEV_WILDCARD: [u8; 4] = [0x02, 0x3F, 0xFF, 0xFF]; // device 4194303
const AI1: [u8; 4] = [0x00, 0x00, 0x00, 0x01];
const AO1: [u8; 4] = [0x00, 0x40, 0x00, 0x01];
const AV1: [u8; 4] = [0x00, 0x80, 0x00, 0x01];
const BI1: [u8; 4] = [0x00, 0xC0, 0x00, 0x01];
const BV1: [u8; 4] = [0x01, 0x40, 0x00, 0x01];
const MSV1: [u8; 4] = [0x04, 0xC0, 0x00, 0x01]; // multi-state-value (19) << 22 | 1

const SVC_RP: u8 = 0x0C;
const SVC_RPM: u8 = 0x0E;
const SVC_WP: u8 = 0x0F;
const SVC_SUBSCRIBE_COV: u8 = 0x05;

fn src() -> SocketAddr {
    "127.0.0.1:47808".parse().unwrap()
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn bvlc(function: u8, payload: &[u8]) -> Vec<u8> {
    let len = (4 + payload.len()) as u16;
    let mut f = vec![0x81, function, (len >> 8) as u8, len as u8];
    f.extend_from_slice(payload);
    f
}

/// BVLC Original-Unicast + plain NPDU (`01 00`) + APDU.
fn unicast(apdu: &[u8]) -> Vec<u8> {
    bvlc(0x0A, &cat(&[&[0x01, 0x00], apdu]))
}

/// ConfirmedRequest, max-APDU 1476, invoke id 1.
fn confirmed(service: u8, body: &[u8]) -> Vec<u8> {
    cat(&[&[0x00, 0x05, 0x01, service], body])
}

/// ReadProperty request body: object, then the already-encoded property reference (`[1]` pid, `[2]` index).
fn rp_body(oid: [u8; 4], pid_and_index: &[u8]) -> Vec<u8> {
    cat(&[&[0x0C], &oid, pid_and_index])
}

/// WriteProperty body: object, `[1]` pid (+index), `[3]` { value }, optional priority.
fn wp_body(oid: [u8; 4], pid_and_index: &[u8], value: &[u8], priority: &[u8]) -> Vec<u8> {
    cat(&[&[0x0C], &oid, pid_and_index, &[0x3E], value, &[0x3F], priority])
}

fn pt(key: &str, ot: &str, inst: u32, base: f64, writable: bool, is_bool: bool) -> SimPointDef {
    SimPointDef {
        key: key.into(),
        label: key.into(),
        object_type: ot.into(),
        object_instance: inst,
        base,
        variance: 0.0,
        noise: 0.0,
        period: 1000,
        min: None,
        max: None,
        decimals: 1,
        unit: Some("°C".into()),
        is_bool,
        writable,
    }
}

/// The standard fixture: one point of every Present_Value datatype family.
fn points() -> Vec<SimPointDef> {
    let mut ao = pt("ao", "analogOutput", 1, 20.0, false, false);
    ao.min = Some(0.0);
    ao.max = Some(100.0);
    let mut ms = pt("ms", "multiStateValue", 1, 1.0, true, false);
    ms.max = Some(4.0);
    vec![
        pt("temp", "analogInput", 1, 22.5, false, false),
        pt("sp", "analogValue", 1, 23.0, true, false),
        ao,
        pt("bi", "binaryInput", 1, 1.0, false, true),
        pt("bv", "binaryValue", 1, 0.0, true, true),
        ms,
    ]
}

struct Rig {
    dev: DeviceInfo,
    objects: Vec<SimObj>,
    cov: CovState,
    now: Instant,
    writes: RefCell<Vec<(String, f64)>>,
}

impl Rig {
    fn new(points: &[SimPointDef]) -> Self {
        Self {
            dev: DeviceInfo { id: 991, vendor_id: 260 },
            objects: build_objects(points).unwrap(),
            cov: CovState::new(),
            now: Instant::now(),
            writes: RefCell::new(Vec::new()),
        }
    }

    fn standard() -> Self {
        Self::new(&points())
    }

    fn send_from(&mut self, from: SocketAddr, frame: &[u8]) -> Option<Reply> {
        let writes = &self.writes;
        handle_packet(frame, from, self.dev, &mut self.objects, &mut self.cov, self.now, &|k, v| {
            writes.borrow_mut().push((k, v))
        })
    }

    fn send(&mut self, frame: &[u8]) -> Option<Reply> {
        self.send_from(src(), frame)
    }

    /// Send an APDU (plain NPDU, unicast) and return the reply's APDU.
    fn apdu(&mut self, apdu: &[u8]) -> Vec<u8> {
        let reply = self.send(&unicast(apdu)).expect("the device must answer");
        reply_apdu(&reply)
    }

    fn check_cov(&mut self) {
        self.cov.check(&self.objects, self.dev.id, self.now);
    }

    fn advance(&mut self, secs: u64) {
        self.now += Duration::from_secs(secs);
    }
}

/// Validate BVLC framing of a reply and return its APDU.
fn reply_apdu(r: &Reply) -> Vec<u8> {
    assert_eq!(&r.frame[..2], &[0x81, 0x0A], "replies are Original-Unicast-NPDU");
    assert_eq!(usize::from(u16::from_be_bytes([r.frame[2], r.frame[3]])), r.frame.len(), "BVLC length");
    let h = parse_npdu(&r.frame[4..]).unwrap();
    r.frame[4 + h.len..].to_vec()
}

// ───────────────────────── BVLC ─────────────────────────

/// Who-Is (global broadcast) as sent by bacnet-stack / YABE.
fn who_is_global(range: &[u8]) -> Vec<u8> {
    bvlc(0x0B, &cat(&[&[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08], range]))
}

/// I-Am for device 991, max-APDU 1476, no-segmentation (3), vendor 260:
/// `10 00` I-Am, `C4` device id, `22 05 C4` Unsigned 1476, `91 03` Enumerated 3, `22 01 04` Unsigned 260.
const IAM: [u8; 15] = [0x10, 0x00, 0xC4, 0x02, 0x00, 0x03, 0xDF, 0x22, 0x05, 0xC4, 0x91, 0x03, 0x22, 0x01, 0x04];

#[test]
fn global_broadcast_who_is_is_answered_with_iam() {
    let mut rig = Rig::standard();
    let wire = who_is_global(&[]);
    assert_eq!(wire, [0x81, 0x0B, 0x00, 0x0C, 0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08]);
    let reply = rig.send(&wire).expect("I-Am");
    assert_eq!(reply.to, src());
    assert_eq!(reply.frame, cat(&[&[0x81, 0x0A, 0x00, 0x15, 0x01, 0x00], &IAM]));
}

#[test]
fn unicast_who_is_is_answered_too() {
    let mut rig = Rig::standard();
    let reply = rig.send(&unicast(&[0x10, 0x08])).expect("I-Am");
    assert_eq!(reply_apdu(&reply), IAM);
}

#[test]
fn who_is_range_limits_are_inclusive_and_both_required() {
    // Context tags [0] low / [1] high, minimal-length Unsigned (clause 16.10.1).
    let cases: [(&str, &[u8], bool); 8] = [
        ("[0,4194303]", &[0x09, 0x00, 0x1B, 0x3F, 0xFF, 0xFF], true),
        ("[3,1000]", &[0x09, 0x03, 0x1A, 0x03, 0xE8], true),
        ("[991,991] lower and upper bound inclusive", &[0x0A, 0x03, 0xDF, 0x1A, 0x03, 0xDF], true),
        ("[992,1000] id below range", &[0x0A, 0x03, 0xE0, 0x1A, 0x03, 0xE8], false),
        ("[0,990] id above range", &[0x09, 0x00, 0x1A, 0x03, 0xDE], false),
        ("[100000,4194303] 3-byte low limit", &[0x0B, 0x01, 0x86, 0xA0, 0x1B, 0x3F, 0xFF, 0xFF], false),
        ("low limit only", &[0x09, 0x00], false),
        ("high limit only", &[0x19, 0xFF], false),
    ];
    for (name, range, answered) in cases {
        let mut rig = Rig::standard();
        let reply = rig.send(&who_is_global(range));
        assert_eq!(reply.is_some(), answered, "Who-Is {name}");
    }
}

#[test]
fn who_is_with_out_of_range_limit_is_ignored() {
    let mut rig = Rig::standard();
    // high limit 4194304 (0x400000) is above the maximum instance number.
    let wire = who_is_global(&[0x09, 0x00, 0x1B, 0x40, 0x00, 0x00]);
    assert!(rig.send(&wire).is_none());
}

#[test]
fn forwarded_npdu_is_answered_to_the_originating_address() {
    let mut rig = Rig::standard();
    // Annex J.2.5: B/IP address of the originator (192.168.1.100:47808 = C0 A8 01 64 BA C0), then the NPDU.
    let wire = bvlc(0x04, &[0xC0, 0xA8, 0x01, 0x64, 0xBA, 0xC0, 0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08]);
    let reply = rig.send(&wire).expect("I-Am");
    assert_eq!(reply.to, "192.168.1.100:47808".parse().unwrap());
    assert_eq!(reply_apdu(&reply), IAM);
}

#[test]
fn bvlc_length_must_match_the_datagram() {
    let mut rig = Rig::standard();
    let good = who_is_global(&[]);
    let mut longer_field = good.clone();
    longer_field[3] += 1;
    assert!(rig.send(&longer_field).is_none());
    let mut trailing = good.clone();
    trailing.push(0x00);
    assert!(rig.send(&trailing).is_none());
    assert!(rig.send(&good[..good.len() - 1]).is_none());
    let mut wrong_type = good;
    wrong_type[0] = 0x82;
    assert!(rig.send(&wrong_type).is_none());
}

#[test]
fn bbmd_functions_are_nacked_by_an_end_device() {
    // Annex J.2: request → BVLC-Result NAK code of that function (J.2.1): Write-BDT 0010, Read-BDT 0020,
    // Register-Foreign-Device 0030, Read-FDT 0040, Delete-FDT-Entry 0050, Distribute-Broadcast-To-Network 0060.
    let cases: [(Vec<u8>, [u8; 2]); 6] = [
        (bvlc(0x01, &[]), [0x00, 0x10]),
        (bvlc(0x02, &[]), [0x00, 0x20]),
        (bvlc(0x05, &[0x00, 0x3C]), [0x00, 0x30]), // time-to-live 60 s
        (bvlc(0x06, &[]), [0x00, 0x40]),
        (bvlc(0x08, &[0xC0, 0xA8, 0x01, 0x64, 0xBA, 0xC0]), [0x00, 0x50]),
        (bvlc(0x09, &[0x01, 0x00, 0x10, 0x08]), [0x00, 0x60]),
    ];
    for (wire, code) in cases {
        let mut rig = Rig::standard();
        let reply = rig.send(&wire).expect("BVLC-Result NAK");
        assert_eq!(reply.to, src());
        assert_eq!(reply.frame, [0x81, 0x00, 0x00, 0x06, code[0], code[1]], "function {:#04x}", wire[1]);
    }
    // Responses and unknown functions are ignored.
    let mut rig = Rig::standard();
    for f in [0x00, 0x03, 0x07, 0x0C, 0x7F] {
        assert!(rig.send(&bvlc(f, &[0x00, 0x00])).is_none(), "function {f:#04x}");
    }
}

// ───────────────────────── NPDU ─────────────────────────

#[test]
fn network_layer_messages_and_foreign_versions_are_ignored() {
    let mut rig = Rig::standard();
    // Who-Is-Router-To-Network (network message type 0): control bit 7 set.
    assert!(rig.send(&bvlc(0x0B, &[0x01, 0x80, 0x00])).is_none());
    // NPDU version 2.
    assert!(rig.send(&bvlc(0x0A, &[0x02, 0x00, 0x10, 0x08])).is_none());
    // Truncated NPDU: DNET announced but missing.
    assert!(rig.send(&bvlc(0x0A, &[0x01, 0x20, 0x00])).is_none());
    // SNET present with SLEN = 0.
    assert!(rig.send(&bvlc(0x0A, &[0x01, 0x08, 0x00, 0x07, 0x00, 0x10, 0x08])).is_none());
}

#[test]
fn requests_addressed_to_another_network_are_not_answered() {
    let mut rig = Rig::standard();
    // DNET 5, DLEN 1, DADR 07, hop count 255.
    let npdu = [0x01, 0x24, 0x00, 0x05, 0x01, 0x07, 0xFF];
    let rp = confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55]));
    assert!(rig.send(&bvlc(0x0A, &cat(&[&npdu, &rp]))).is_none());
    let who_is = [0x01, 0x20, 0x00, 0x05, 0x01, 0x07, 0xFF, 0x10, 0x08];
    assert!(rig.send(&bvlc(0x0A, &who_is)).is_none());
}

#[test]
fn replies_to_routed_requests_carry_the_source_network_as_dnet() {
    let mut rig = Rig::standard();
    // Request from network 7, MAC 01 02 03 (SNET/SLEN/SADR), expecting a reply, priority 1.
    let npdu = [0x01, 0x0D, 0x00, 0x07, 0x03, 0x01, 0x02, 0x03];
    let rp = confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55]));
    let reply = rig.send(&bvlc(0x0A, &cat(&[&npdu, &rp]))).expect("ack");
    // Reply NPDU: DNET present (0x20), priority 1 preserved, DNET 7 / DLEN 3 / DADR, hop count 255.
    let ack = [0x30, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x3F];
    assert_eq!(
        reply.frame,
        bvlc(0x0A, &cat(&[&[0x01, 0x21, 0x00, 0x07, 0x03, 0x01, 0x02, 0x03, 0xFF], &ack]))
    );
    // I-Am to a routed Who-Is likewise.
    let who_is = bvlc(0x0A, &[0x01, 0x08, 0x00, 0x07, 0x03, 0x01, 0x02, 0x03, 0x10, 0x08]);
    let reply = rig.send(&who_is).expect("I-Am");
    assert_eq!(
        reply.frame,
        bvlc(0x0A, &cat(&[&[0x01, 0x20, 0x00, 0x07, 0x03, 0x01, 0x02, 0x03, 0xFF], &IAM]))
    );
}

#[test]
fn replies_preserve_the_request_priority() {
    let mut rig = Rig::standard();
    // Control 0x05: expecting reply + priority 1 → the reply has priority 1 and no expecting-reply bit.
    let rp = confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55]));
    let reply = rig.send(&bvlc(0x0A, &cat(&[&[0x01, 0x05], &rp]))).expect("ack");
    assert_eq!(&reply.frame[4..6], &[0x01, 0x01]);
}

#[test]
fn broadcast_confirmed_requests_are_dropped() {
    let mut rig = Rig::standard();
    let rp = confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55]));
    assert!(rig.send(&bvlc(0x0B, &cat(&[&[0x01, 0x04], &rp]))).is_none(), "Original-Broadcast");
    assert!(
        rig.send(&bvlc(0x0A, &cat(&[&[0x01, 0x24, 0xFF, 0xFF, 0x00, 0xFF], &rp]))).is_none(),
        "global broadcast DNET"
    );
}

// ───────────────────────── Confirmed PDU handling ─────────────────────────

#[test]
fn read_property_ack_bytes() {
    let mut rig = Rig::standard();
    // ReadProperty of analogInput 1 presentValue (Annex F.3 example shape); 22.5 = 41 B4 00 00.
    let req = confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55]));
    assert_eq!(req, [0x00, 0x05, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]);
    assert_eq!(
        rig.apdu(&req),
        [0x30, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x3F]
    );
}

#[test]
fn present_value_datatypes_follow_the_object_type() {
    let mut rig = Rig::standard();
    let cases: [([u8; 4], &[u8]); 6] = [
        (AI1, &[0x44, 0x41, 0xB4, 0x00, 0x00]), // analog → Real 22.5
        (AV1, &[0x44, 0x41, 0xB8, 0x00, 0x00]), // Real 23.0
        (AO1, &[0x44, 0x41, 0xA0, 0x00, 0x00]), // Real 20.0
        (BI1, &[0x91, 0x01]),                   // binary → Enumerated active
        (BV1, &[0x91, 0x00]),                   // Enumerated inactive
        (MSV1, &[0x21, 0x01]),                  // multi-state → Unsigned state 1
    ];
    for (oid, value) in cases {
        let ack = rig.apdu(&confirmed(SVC_RP, &rp_body(oid, &[0x19, 0x55])));
        let expected = cat(&[&[0x30, 0x01, 0x0C, 0x0C], &oid, &[0x19, 0x55, 0x3E], value, &[0x3F]]);
        assert_eq!(ack, expected, "object {oid:02X?}");
    }
}

#[test]
fn point_properties_have_the_standard_datatypes() {
    let mut rig = Rig::standard();
    let name = [0x75, 0x05, 0x00, b't', b'e', b'm', b'p']; // CharacterString "temp": ext. length 5, charset 0
    let cases: [(&[u8], Vec<u8>); 8] = [
        (&[0x19, 0x4D], name.to_vec()),                                  // objectName
        (&[0x19, 0x4B], vec![0xC4, 0x00, 0x00, 0x00, 0x01]),             // objectIdentifier
        (&[0x19, 0x4F], vec![0x91, 0x00]),                               // objectType analog-input
        (&[0x19, 0x6F], vec![0x82, 0x04, 0x00]),                         // statusFlags: 4 bits, 4 unused
        (&[0x19, 0x24], vec![0x91, 0x00]),                               // eventState normal
        (&[0x19, 0x51], vec![0x10]),                                     // outOfService FALSE
        (&[0x19, 0x75], vec![0x91, 0x3E]),                               // units degrees-celsius (62)
        (&[0x19, 0x16], vec![0x44, 0x3D, 0xCC, 0xCC, 0xCD]),             // covIncrement 0.1
    ];
    for (prop, value) in cases {
        let ack = rig.apdu(&confirmed(SVC_RP, &rp_body(AI1, prop)));
        assert_eq!(ack, cat(&[&[0x30, 0x01, 0x0C, 0x0C], &AI1, prop, &[0x3E], &value, &[0x3F]]), "property {prop:02X?}");
    }
}

#[test]
fn units_use_the_standard_enumeration_values() {
    // BACnetEngineeringUnits (clause 21).
    for (unit, value) in [
        ("°C", 62),
        ("°F", 64),
        ("%", 98),
        ("Pa", 53),
        ("kPa", 54),
        ("bar", 55),
        ("W", 47),
        ("kW", 48),
        ("RPM", 104),
        ("CFM", 84),
        ("m³/s", 85),
        ("Hz", 27),
    ] {
        assert_eq!(unit_enum(Some(unit)), value, "{unit}");
    }
    assert_eq!(unit_enum(None), 95, "no-units");
    assert_eq!(unit_enum(Some("furlongs")), 95);
}

#[test]
fn read_errors_use_the_standard_error_codes() {
    let mut rig = Rig::standard(); // device + 6 points = 7 objects
    let cases: [(Vec<u8>, [u8; 2], &str); 6] = [
        (rp_body([0x00, 0x00, 0x00, 0x63], &[0x19, 0x55]), [1, 31], "unknown-object"),
        (rp_body(AI1, &[0x19, 0x57]), [2, 32], "priorityArray on a non-commandable input: unknown-property"),
        (rp_body(AI1, &[0x19, 0x55, 0x29, 0x00]), [2, 50], "array index on a non-array: property-is-not-an-array"),
        (rp_body(DEV, &[0x19, 0x4C, 0x29, 0x08]), [2, 42], "object-list index 8 of 7: invalid-array-index"),
        (rp_body(DEV, &[0x19, 0x08]), [2, 32], "special identifier ALL is not valid in ReadProperty"),
        (rp_body(DEV, &[0x1A, 0x27, 0x0F]), [2, 32], "property 9999"),
    ];
    for (body, code, what) in cases {
        assert_eq!(rig.apdu(&confirmed(SVC_RP, &body)), [0x50, 0x01, 0x0C, 0x91, code[0], 0x91, code[1]], "{what}");
    }
}

#[test]
fn object_list_array_access() {
    let mut rig = Rig::standard(); // device + 6 points = 7 objects
    let ack = |index: &[u8], value: &[u8]| {
        cat(&[&[0x30, 0x01, 0x0C, 0x0C], &DEV, &[0x19, 0x4C], index, &[0x3E], value, &[0x3F]])
    };
    // index 0 → count (Unsigned 7)
    assert_eq!(rig.apdu(&confirmed(SVC_RP, &rp_body(DEV, &[0x19, 0x4C, 0x29, 0x00]))), ack(&[0x29, 0x00], &[0x21, 0x07]));
    // index 1 → the device itself
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(DEV, &[0x19, 0x4C, 0x29, 0x01]))),
        ack(&[0x29, 0x01], &[0xC4, 0x02, 0x00, 0x03, 0xDF])
    );
    // index 7 → last point (multiStateValue 1)
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(DEV, &[0x19, 0x4C, 0x29, 0x07]))),
        ack(&[0x29, 0x07], &[0xC4, 0x04, 0xC0, 0x00, 0x01])
    );
    // index 8 → invalid-array-index (property class, code 42)
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(DEV, &[0x19, 0x4C, 0x29, 0x08]))),
        [0x50, 0x01, 0x0C, 0x91, 0x02, 0x91, 0x2A]
    );
    // no index → all 7 identifiers
    let all = rig.apdu(&confirmed(SVC_RP, &rp_body(DEV, &[0x19, 0x4C])));
    assert_eq!(all.len(), 3 + 5 + 2 + 1 + 7 * 5 + 1);
}

#[test]
fn device_object_properties() {
    let mut rig = Rig::standard();
    let rp = |rig: &mut Rig, oid: [u8; 4], prop: &[u8]| rig.apdu(&confirmed(SVC_RP, &rp_body(oid, prop)));
    // vendorIdentifier is the vendor id announced in I-Am (260).
    assert_eq!(
        rp(&mut rig, DEV, &[0x19, 0x78]),
        [0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x03, 0xDF, 0x19, 0x78, 0x3E, 0x22, 0x01, 0x04, 0x3F]
    );
    // max-APDU-length-accepted 1476 and segmentation-supported no-segmentation (3) match I-Am.
    assert_eq!(&rp(&mut rig, DEV, &[0x19, 0x3E])[10..], [0x3E, 0x22, 0x05, 0xC4, 0x3F]);
    assert_eq!(&rp(&mut rig, DEV, &[0x19, 0x6B])[10..], [0x3E, 0x91, 0x03, 0x3F]);
    // The wildcard instance 4194303 addresses this device; the ACK carries the real identifier.
    assert_eq!(
        rp(&mut rig, DEV_WILDCARD, &[0x19, 0x4B]),
        [0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x03, 0xDF, 0x19, 0x4B, 0x3E, 0xC4, 0x02, 0x00, 0x03, 0xDF, 0x3F]
    );
    // protocolServicesSupported: 40 bits; set bits 1, 5, 12, 14, 15, 26, 28, 34.
    assert_eq!(
        &rp(&mut rig, DEV, &[0x19, 0x61])[10..],
        [0x3E, 0x85, 0x06, 0x00, 0x44, 0x0B, 0x00, 0x28, 0x20, 0x3F]
    );
    // deviceAddressBinding: empty list.
    assert_eq!(&rp(&mut rig, DEV, &[0x19, 0x1E])[10..], [0x3E, 0x3F]);
}

#[test]
fn malformed_requests_are_rejected_with_the_right_reason() {
    let mut rig = Rig::standard();
    let cases: [(Vec<u8>, u8, &str); 5] = [
        (confirmed(SVC_RP, &[0x0C, 0x00, 0x00, 0x00, 0x01]), 5, "missing property: missing-required-parameter"),
        (confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55, 0x00])), 7, "trailing application tag: too-many-arguments"),
        (confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x55, 0x29, 0x00, 0x29, 0x00])), 7, "too-many-arguments"),
        (confirmed(SVC_RP, &[0x1C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]), 4, "wrong context tag number: invalid-tag"),
        (confirmed(SVC_RP, &[0x0D, 0x05, 0x00, 0x00, 0x00, 0x01, 0x00, 0x19, 0x55]), 3, "5-octet object id: invalid-parameter-data-type"),
    ];
    for (req, reason, what) in cases {
        assert_eq!(rig.apdu(&req), [0x60, 0x01, reason], "{what}");
    }
}

#[test]
fn unsupported_services_are_rejected_as_unrecognized() {
    let mut rig = Rig::standard();
    // ReadRange (26), WritePropertyMultiple (16), ConfirmedCOVNotification (1), DeviceCommunicationControl (17).
    for service in [26u8, 16, 1, 17] {
        let req = confirmed(service, &rp_body(AI1, &[0x19, 0x55]));
        assert_eq!(rig.apdu(&req), [0x60, 0x01, 0x09], "service {service}");
    }
}

#[test]
fn segmented_requests_are_aborted() {
    let mut rig = Rig::standard();
    // SEG flag set, sequence number 0, window size 2: the device cannot reassemble.
    let req = [0x08, 0x05, 0x01, 0x00, 0x02, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55];
    assert_eq!(rig.apdu(&req), [0x71, 0x01, 0x04]); // Abort by server, segmentation-not-supported
}

#[test]
fn responses_larger_than_the_clients_max_apdu_are_aborted() {
    let many: Vec<SimPointDef> = (1..=100).map(|i| pt(&format!("p{i}"), "analogInput", i, 1.0, false, false)).collect();
    let mut rig = Rig::new(&many);
    // object-list: 101 identifiers × 5 octets + framing = 519 octets of APDU.
    let rp = |max: u8| cat(&[&[0x00, max, 0x07, SVC_RP], &rp_body(DEV, &[0x19, 0x4C])]);
    let rpm = |max: u8| cat(&[&[0x00, max, 0x08, SVC_RPM], &[0x0C], &DEV, &[0x1E, 0x09, 0x4C, 0x1F]]);
    // max-APDU codes: 0 → 50, 3 → 480, 4 → 1024, 5 → 1476.
    assert_eq!(rig.apdu(&rp(0)), [0x71, 0x07, 0x04]);
    assert_eq!(rig.apdu(&rp(3)), [0x71, 0x07, 0x04]);
    assert_eq!(rig.apdu(&rpm(3)), [0x71, 0x08, 0x04]);
    let ack = rig.apdu(&rp(4));
    assert_eq!(ack.len(), 3 + 5 + 2 + 1 + 101 * 5 + 1);
    assert_eq!(&ack[..3], &[0x30, 0x07, 0x0C]);
    assert_eq!(rig.apdu(&rpm(5))[..3], [0x30, 0x08, 0x0E]);
    // Small answers still fit the smallest APDU (50 octets).
    let small = cat(&[&[0x00, 0x00, 0x09, SVC_RP], &rp_body(AI1, &[0x19, 0x55])]);
    assert_eq!(rig.apdu(&small)[..3], [0x30, 0x09, 0x0C]);
}

// ───────────────────────── ReadPropertyMultiple ─────────────────────────

fn rpm_body(oid: [u8; 4], props: &[u8]) -> Vec<u8> {
    cat(&[&[0x0C], &oid, &[0x1E], props, &[0x1F]])
}

#[test]
fn rpm_two_properties_ack_bytes() {
    let mut rig = Rig::standard();
    // AI1 { presentValue, units }
    let req = confirmed(SVC_RPM, &rpm_body(AI1, &[0x09, 0x55, 0x09, 0x75]));
    assert_eq!(
        rig.apdu(&req),
        [
            0x30, 0x01, 0x0E, // ComplexAck, invoke 1, ReadPropertyMultiple
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object id
            0x1E, // [1] list of results
            0x29, 0x55, 0x4E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x4F, // [2] presentValue [4]{Real 22.5}
            0x29, 0x75, 0x4E, 0x91, 0x3E, 0x4F, // [2] units [4]{Enumerated 62}
            0x1F,
        ]
    );
}

#[test]
fn rpm_unknown_object_is_a_per_property_error() {
    let mut rig = Rig::standard();
    // Clause 15.7: no Error PDU, each requested property carries [5] { object, unknown-object }.
    let req = confirmed(SVC_RPM, &rpm_body([0x00, 0x00, 0x00, 0x63], &[0x09, 0x55, 0x09, 0x08]));
    assert_eq!(
        rig.apdu(&req),
        [
            0x30, 0x01, 0x0E, 0x0C, 0x00, 0x00, 0x00, 0x63, 0x1E, //
            0x29, 0x55, 0x5E, 0x91, 0x01, 0x91, 0x1F, 0x5F, //
            0x29, 0x08, 0x5E, 0x91, 0x01, 0x91, 0x1F, 0x5F, //
            0x1F,
        ]
    );
}

#[test]
fn rpm_unknown_property_and_array_index_echo() {
    let mut rig = Rig::standard();
    let req = confirmed(SVC_RPM, &rpm_body(AI1, &[0x09, 0x57]));
    assert_eq!(
        rig.apdu(&req),
        [0x30, 0x01, 0x0E, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x29, 0x57, 0x5E, 0x91, 0x02, 0x91, 0x20, 0x5F, 0x1F]
    );
    // Array index is echoed as [3] and reads object-list[0] = 7.
    let req = confirmed(SVC_RPM, &rpm_body(DEV, &[0x09, 0x4C, 0x19, 0x00]));
    assert_eq!(
        rig.apdu(&req),
        [0x30, 0x01, 0x0E, 0x0C, 0x02, 0x00, 0x03, 0xDF, 0x1E, 0x29, 0x4C, 0x39, 0x00, 0x4E, 0x21, 0x07, 0x4F, 0x1F]
    );
}

#[test]
fn rpm_special_property_identifiers_expand() {
    let mut rig = Rig::standard();
    let object_name = [0x29, 0x4D, 0x4E, 0x75, 0x05, 0x00, b't', b'e', b'm', b'p', 0x4F];
    let description = [0x29, 0x1C, 0x4E, 0x75, 0x05, 0x00, b't', b'e', b'm', b'p', 0x4F];
    let cov_increment = [0x29, 0x16, 0x4E, 0x44, 0x3D, 0xCC, 0xCC, 0xCD, 0x4F];
    let required: Vec<u8> = cat(&[
        &[0x29, 0x4B, 0x4E, 0xC4, 0x00, 0x00, 0x00, 0x01, 0x4F], // objectIdentifier
        &object_name,
        &[0x29, 0x4F, 0x4E, 0x91, 0x00, 0x4F], // objectType
        &[0x29, 0x55, 0x4E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x4F], // presentValue
        &[0x29, 0x6F, 0x4E, 0x82, 0x04, 0x00, 0x4F], // statusFlags
        &[0x29, 0x24, 0x4E, 0x91, 0x00, 0x4F], // eventState
        &[0x29, 0x51, 0x4E, 0x10, 0x4F], // outOfService
        &[0x29, 0x75, 0x4E, 0x91, 0x3E, 0x4F], // units
    ]);
    let optional = cat(&[&description, &cov_increment]);
    let head = [0x30, 0x01, 0x0E, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E];
    // REQUIRED (105), OPTIONAL (80), ALL (8) — clause 15.7.3.1.1.
    let cases: [(u8, Vec<u8>); 3] = [
        (0x69, required.clone()),
        (0x50, optional.clone()),
        (0x08, cat(&[&required, &optional])),
    ];
    for (pid, body) in cases {
        let ack = rig.apdu(&confirmed(SVC_RPM, &rpm_body(AI1, &[0x09, pid])));
        assert_eq!(ack, cat(&[&head, &body, &[0x1F]]), "special property {pid}");
    }
}

#[test]
fn rpm_malformed_requests_are_rejected() {
    let mut rig = Rig::standard();
    // empty request, empty property list, missing closing tag, property-reference with a bad tag
    let cases: [(Vec<u8>, u8); 4] = [
        (confirmed(SVC_RPM, &[]), 5),
        (confirmed(SVC_RPM, &rpm_body(AI1, &[])), 5),
        (confirmed(SVC_RPM, &cat(&[&[0x0C], &AI1, &[0x1E, 0x09, 0x55]])), 5),
        (confirmed(SVC_RPM, &rpm_body(AI1, &[0x19, 0x55])), 4),
    ];
    for (req, reason) in cases {
        assert_eq!(rig.apdu(&req), [0x60, 0x01, reason]);
    }
}

// ───────────────────────── WriteProperty ─────────────────────────

const REAL_50: [u8; 5] = [0x44, 0x42, 0x48, 0x00, 0x00];
const REAL_10: [u8; 5] = [0x44, 0x41, 0x20, 0x00, 0x00];

fn write_ao(rig: &mut Rig, value: &[u8], priority: &[u8]) -> Vec<u8> {
    rig.apdu(&confirmed(SVC_WP, &wp_body(AO1, &[0x19, 0x55], value, priority)))
}

#[test]
fn write_to_read_only_point_is_denied_with_error_pdu() {
    let mut rig = Rig::standard();
    let apdu = confirmed(SVC_WP, &wp_body(AI1, &[0x19, 0x55], &[0x44, 0x41, 0xA0, 0x00, 0x00], &[]));
    assert_eq!(rig.apdu(&apdu), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x28]); // property / write-access-denied
    assert!(rig.writes.borrow().is_empty(), "on_write must not fire for a rejected write");
    assert_eq!(rig.objects[0].current, 22.5);
}

#[test]
fn write_priority_array_semantics() {
    let mut rig = Rig::standard();
    // Priority 8: commanded value wins; reported to on_write.
    assert_eq!(write_ao(&mut rig, &REAL_50, &[0x49, 0x08]), [0x20, 0x01, 0x0F]);
    assert_eq!(rig.objects[2].current, 50.0);
    assert_eq!(rig.writes.borrow().last(), Some(&("ao".to_string(), 50.0)));
    // Priority array: 16 entries, slot 8 holds the Real, the others are Null; index 0 = 16.
    let pa = |tail: &[u8]| cat(&[&[0x30, 0x01, 0x0C, 0x0C], &AO1, &[0x19, 0x57], tail]);
    let mut slots = vec![0x3E];
    for slot in 1..=16 {
        if slot == 8 {
            slots.extend_from_slice(&REAL_50);
        } else {
            slots.push(0x00);
        }
    }
    slots.push(0x3F);
    assert_eq!(rig.apdu(&confirmed(SVC_RP, &rp_body(AO1, &[0x19, 0x57]))), pa(&slots));
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(AO1, &[0x19, 0x57, 0x29, 0x08]))),
        pa(&cat(&[&[0x29, 0x08, 0x3E], &REAL_50, &[0x3F]]))
    );
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(AO1, &[0x19, 0x57, 0x29, 0x00]))),
        pa(&[0x29, 0x00, 0x3E, 0x21, 0x10, 0x3F])
    );
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(AO1, &[0x19, 0x57, 0x29, 0x11]))),
        [0x50, 0x01, 0x0C, 0x91, 0x02, 0x91, 0x2A]
    );
    // A lower-priority (16) command does not change the effective value.
    assert_eq!(write_ao(&mut rig, &REAL_10, &[]), [0x20, 0x01, 0x0F]); // priority omitted = 16
    assert_eq!(rig.objects[2].current, 50.0);
    // Relinquish priority 8 (Null) → the priority-16 value takes over.
    assert_eq!(write_ao(&mut rig, &[0x00], &[0x49, 0x08]), [0x20, 0x01, 0x0F]);
    assert_eq!(rig.objects[2].current, 10.0);
    // Relinquish 16 → Relinquish_Default (the point's base, 20.0).
    assert_eq!(write_ao(&mut rig, &[0x00], &[]), [0x20, 0x01, 0x0F]);
    assert_eq!(rig.objects[2].current, 20.0);
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(AO1, &[0x19, 0x68]))),
        cat(&[&[0x30, 0x01, 0x0C, 0x0C], &AO1, &[0x19, 0x68, 0x3E, 0x44, 0x41, 0xA0, 0x00, 0x00, 0x3F]])
    );
}

#[test]
fn write_priority_validation() {
    let mut rig = Rig::standard();
    // Priority 6 is reserved for Minimum On/Off: write-access-denied.
    assert_eq!(write_ao(&mut rig, &REAL_50, &[0x49, 0x06]), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x28]);
    // Priority outside 1..16 is a parameter-out-of-range Reject.
    assert_eq!(write_ao(&mut rig, &REAL_50, &[0x49, 0x11]), [0x60, 0x01, 0x06]);
    assert_eq!(write_ao(&mut rig, &REAL_50, &[0x49, 0x00]), [0x60, 0x01, 0x06]);
    assert_eq!(rig.objects[2].current, 20.0);
    // Priority 1 and 16 are the valid extremes.
    assert_eq!(write_ao(&mut rig, &REAL_50, &[0x49, 0x01]), [0x20, 0x01, 0x0F]);
    assert_eq!(write_ao(&mut rig, &REAL_10, &[0x49, 0x10]), [0x20, 0x01, 0x0F]);
}

#[test]
fn write_value_validation() {
    let mut rig = Rig::standard();
    let denied_type = [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x09]; // invalid-data-type
    let out_of_range = [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x25]; // value-out-of-range
    // AO range is 0..=100: 150.0 = 43 16 00 00, NaN = 7F C0 00 00.
    assert_eq!(write_ao(&mut rig, &[0x44, 0x43, 0x16, 0x00, 0x00], &[]), out_of_range);
    assert_eq!(write_ao(&mut rig, &[0x44, 0x7F, 0xC0, 0x00, 0x00], &[]), out_of_range);
    // Wrong datatypes: Enumerated, Unsigned, Double, CharacterString, Boolean, two values, constructed.
    assert_eq!(write_ao(&mut rig, &[0x91, 0x01], &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &[0x21, 0x01], &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &[0x55, 0x08, 0x40, 0x34, 0, 0, 0, 0, 0, 0], &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &[0x72, 0x00, 0x41], &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &[0x11], &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &cat(&[&REAL_50, &REAL_50]), &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &cat(&[&[0x2E], &REAL_50, &[0x2F]]), &[]), denied_type);
    assert_eq!(write_ao(&mut rig, &[], &[]), denied_type);
    assert_eq!(rig.objects[2].current, 20.0, "no rejected write may change the value");

    // Binary: Enumerated 0/1 only.
    let bv = |rig: &mut Rig, value: &[u8]| rig.apdu(&confirmed(SVC_WP, &wp_body(BV1, &[0x19, 0x55], value, &[0x49, 0x10])));
    assert_eq!(bv(&mut rig, &[0x91, 0x01]), [0x20, 0x01, 0x0F]);
    assert_eq!(rig.objects[4].current, 1.0);
    assert_eq!(bv(&mut rig, &[0x91, 0x02]), out_of_range);
    assert_eq!(bv(&mut rig, &[0x44, 0x3F, 0x80, 0x00, 0x00]), denied_type); // Real 1.0 where Enumerated is required
    assert_eq!(bv(&mut rig, &[0x21, 0x01]), denied_type);

    // Multi-state (4 states): Unsigned 1..=4 only.
    let ms = |rig: &mut Rig, value: &[u8]| rig.apdu(&confirmed(SVC_WP, &wp_body(MSV1, &[0x19, 0x55], value, &[])));
    assert_eq!(ms(&mut rig, &[0x21, 0x03]), [0x20, 0x01, 0x0F]);
    assert_eq!(rig.objects[5].current, 3.0);
    assert_eq!(ms(&mut rig, &[0x21, 0x00]), out_of_range);
    assert_eq!(ms(&mut rig, &[0x21, 0x05]), out_of_range);
    assert_eq!(ms(&mut rig, &[0x91, 0x02]), denied_type);
    assert_eq!(ms(&mut rig, &[0x44, 0x40, 0x40, 0x00, 0x00]), denied_type);
}

#[test]
fn write_to_non_writable_and_unknown_targets() {
    let mut rig = Rig::standard();
    let wp = |rig: &mut Rig, oid, prop: &[u8], value: &[u8]| rig.apdu(&confirmed(SVC_WP, &wp_body(oid, prop, value, &[])));
    let denied = [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x28];
    // objectName (known, read-only here), priorityArray, with and without index.
    assert_eq!(wp(&mut rig, AV1, &[0x19, 0x4D], &[0x72, 0x00, 0x41]), denied);
    assert_eq!(wp(&mut rig, AO1, &[0x19, 0x57], &[0x00]), denied);
    assert_eq!(wp(&mut rig, AO1, &[0x19, 0x57, 0x29, 0x03], &[0x00]), denied);
    // Device properties are read-only.
    assert_eq!(wp(&mut rig, DEV, &[0x19, 0x79], &[0x72, 0x00, 0x41]), denied); // vendorName (121)
    // Unknown property / object, property-is-not-an-array.
    assert_eq!(wp(&mut rig, AV1, &[0x1A, 0x27, 0x0F], &[0x00]), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x20]);
    assert_eq!(wp(&mut rig, AI1, &[0x19, 0x57], &[0x00]), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x20]); // inputs have no priorityArray
    assert_eq!(
        wp(&mut rig, [0x00, 0x80, 0x00, 0x63], &[0x19, 0x55], &REAL_50),
        [0x50, 0x01, 0x0F, 0x91, 0x01, 0x91, 0x1F]
    );
    assert_eq!(
        wp(&mut rig, AV1, &[0x19, 0x55, 0x29, 0x00], &REAL_50),
        [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x32]
    );
}

#[test]
fn write_real_containing_closing_tag_byte_is_applied() {
    let mut rig = Rig::standard();
    // Real 1.0 = 3F 80 00 00: the first content octet equals the [3] closing tag byte.
    let apdu = confirmed(SVC_WP, &wp_body(AV1, &[0x19, 0x55], &[0x44, 0x3F, 0x80, 0x00, 0x00], &[0x49, 0x08]));
    assert_eq!(rig.apdu(&apdu), [0x20, 0x01, 0x0F]);
    assert_eq!(rig.objects[1].current, 1.0);
}

#[test]
fn out_of_service_unlocks_input_present_value() {
    let mut rig = Rig::standard();
    let write_pv = |rig: &mut Rig, value: &[u8]| rig.apdu(&confirmed(SVC_WP, &wp_body(AI1, &[0x19, 0x55], value, &[])));
    let set_oos = |rig: &mut Rig, v: u8| rig.apdu(&confirmed(SVC_WP, &wp_body(AI1, &[0x19, 0x51], &[v], &[])));
    // Present_Value of an input is writable only while Out_Of_Service (clause 12.1).
    assert_eq!(write_pv(&mut rig, &[0x44, 0x42, 0x04, 0x00, 0x00]), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x28]);
    assert_eq!(set_oos(&mut rig, 0x11), [0x20, 0x01, 0x0F]);
    assert!(rig.objects[0].out_of_service);
    // Null is not a valid datatype for a non-commandable property.
    assert_eq!(write_pv(&mut rig, &[0x00]), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x09]);
    assert_eq!(write_pv(&mut rig, &[0x44, 0x42, 0x04, 0x00, 0x00]), [0x20, 0x01, 0x0F]); // 33.0
    assert_eq!(rig.objects[0].current, 33.0);
    // The simulation no longer moves it, and statusFlags reports out-of-service.
    rig.objects[0].base = 50.0;
    tick_values(&mut rig.objects, &Instant::now(), true);
    assert_eq!(rig.objects[0].current, 33.0);
    assert_eq!(
        &rig.apdu(&confirmed(SVC_RP, &rp_body(AI1, &[0x19, 0x6F])))[10..],
        [0x3E, 0x82, 0x04, 0x10, 0x3F]
    );
    // Boolean required for outOfService.
    assert_eq!(set_oos(&mut rig, 0x10), [0x20, 0x01, 0x0F]);
    assert_eq!(
        rig.apdu(&confirmed(SVC_WP, &wp_body(AI1, &[0x19, 0x51], &[0x21, 0x01], &[]))),
        [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x09]
    );
    assert_eq!(write_pv(&mut rig, &[0x44, 0x42, 0x04, 0x00, 0x00]), [0x50, 0x01, 0x0F, 0x91, 0x02, 0x91, 0x28]);
}

#[test]
fn outputs_are_always_commandable_and_values_only_when_writable() {
    let mut rig = Rig::standard();
    // AV "sp" is writable → commandable; its Relinquish_Default is the base 23.0.
    assert_eq!(
        &rig.apdu(&confirmed(SVC_RP, &rp_body(AV1, &[0x19, 0x68])))[10..],
        [0x3E, 0x44, 0x41, 0xB8, 0x00, 0x00, 0x3F]
    );
    // A binary output without `writable` is still commandable (Priority_Array is required for outputs).
    let mut with_bo = points();
    with_bo.push(pt("bo", "binaryOutput", 1, 0.0, false, true));
    let mut rig = Rig::new(&with_bo);
    let bo1 = [0x01, 0x00, 0x00, 0x01];
    let ack = rig.apdu(&confirmed(SVC_RP, &rp_body(bo1, &[0x19, 0x57, 0x29, 0x00])));
    assert_eq!(&ack[ack.len() - 4..], [0x3E, 0x21, 0x10, 0x3F]);
    // Binary_Output has Polarity (normal = 0), Binary_Value does not.
    let ack = rig.apdu(&confirmed(SVC_RP, &rp_body(bo1, &[0x19, 0x54])));
    assert_eq!(&ack[ack.len() - 4..], [0x3E, 0x91, 0x00, 0x3F]);
    assert_eq!(
        rig.apdu(&confirmed(SVC_RP, &rp_body(BV1, &[0x19, 0x54]))),
        [0x50, 0x01, 0x0C, 0x91, 0x02, 0x91, 0x20]
    );
}

// ───────────────────────── SubscribeCOV and notifications ─────────────────────────

/// SubscribeCOV body: `[0]` process id, `[1]` object, optional `[2]` confirmed, optional `[3]` lifetime.
fn subscribe_body(proc_id: u8, oid: [u8; 4], confirmed_flag: Option<u8>, lifetime: Option<u8>) -> Vec<u8> {
    let mut b = cat(&[&[0x09, proc_id, 0x1C], &oid]);
    if let Some(c) = confirmed_flag {
        b.extend_from_slice(&[0x29, c]);
    }
    if let Some(l) = lifetime {
        b.extend_from_slice(&[0x39, l]);
    }
    b
}

fn subscribe(rig: &mut Rig, proc_id: u8, oid: [u8; 4], confirmed_flag: Option<u8>, lifetime: Option<u8>) -> Vec<u8> {
    rig.apdu(&confirmed(SVC_SUBSCRIBE_COV, &subscribe_body(proc_id, oid, confirmed_flag, lifetime)))
}

/// UnconfirmedCOVNotification of analogInput 1 at 22.5, process 7, indefinite lifetime (clause 13.7).
const COV_AI1_22_5: [u8; 40] = [
    0x81, 0x0A, 0x00, 0x28, // BVLC original-unicast, 40 octets
    0x01, 0x00, // NPDU version 1, no flags
    0x10, 0x02, // UnconfirmedRequest, UnconfirmedCOVNotification(2)
    0x09, 0x07, // [0] subscriberProcessIdentifier 7
    0x1C, 0x02, 0x00, 0x03, 0xDF, // [1] initiatingDeviceIdentifier: device 991
    0x2C, 0x00, 0x00, 0x00, 0x01, // [2] monitoredObjectIdentifier: analogInput 1
    0x39, 0x00, // [3] timeRemaining 0 (indefinite)
    0x4E, // [4] opening: listOfValues
    0x09, 0x55, // [0] propertyIdentifier presentValue
    0x2E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x2F, // [2] { Real 22.5 }
    0x09, 0x6F, // [0] propertyIdentifier statusFlags
    0x2E, 0x82, 0x04, 0x00, 0x2F, // [2] { BitString, 4 unused bits, 0000 }
    0x4F, // [4] closing
];

#[test]
fn subscribe_sends_initial_unconfirmed_notification() {
    let mut rig = Rig::standard();
    assert_eq!(subscribe(&mut rig, 7, AI1, Some(0), Some(0)), [0x20, 0x01, 0x05]); // SimpleAck, SubscribeCOV
    assert_eq!(rig.cov.outbox.len(), 1, "initial notification expected");
    assert_eq!(rig.cov.outbox[0].to, src());
    assert_eq!(rig.cov.outbox[0].frame, COV_AI1_22_5);
}

#[test]
fn confirmed_notification_wire_bytes() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(1), Some(60));
    let expected: Vec<u8> = vec![
        0x81, 0x0A, 0x00, 0x2A, // BVLC original-unicast, 42 octets
        0x01, 0x04, // NPDU: expecting reply
        0x00, 0x05, 0x00, 0x01, // ConfirmedRequest, max-APDU 1476, invoke 0, ConfirmedCOVNotification(1)
        0x09, 0x07, // [0] process id 7
        0x1C, 0x02, 0x00, 0x03, 0xDF, // [1] device 991
        0x2C, 0x00, 0x00, 0x00, 0x01, // [2] analogInput 1
        0x39, 0x3C, // [3] timeRemaining 60
        0x4E, 0x09, 0x55, 0x2E, 0x44, 0x41, 0xB4, 0x00, 0x00, 0x2F, // presentValue { Real 22.5 }
        0x09, 0x6F, 0x2E, 0x82, 0x04, 0x00, 0x2F, 0x4F, // statusFlags { all clear }, [4] closing
    ];
    assert_eq!(rig.cov.outbox[0].frame, expected);
}

#[test]
fn notification_time_remaining_counts_down_and_binary_values_are_enumerated() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(0), Some(10));
    rig.cov.outbox.clear();
    rig.advance(4);
    rig.objects[0].current = 30.0;
    rig.check_cov();
    let frame = &rig.cov.outbox[0].frame;
    assert_eq!(&frame[20..22], &[0x39, 0x06], "10 s lifetime, 4 s later");
    assert_eq!(&frame[23..33], &[0x09, 0x55, 0x2E, 0x44, 0x41, 0xF0, 0x00, 0x00, 0x2F, 0x09]); // Real 30.0

    // binaryInput 1: Enumerated present value; any state change notifies.
    subscribe(&mut rig, 8, BI1, Some(0), Some(0));
    let initial = rig.cov.outbox.pop().unwrap().frame;
    assert_eq!(&initial[23..31], &[0x09, 0x55, 0x2E, 0x91, 0x01, 0x2F, 0x09, 0x6F]);
    rig.cov.outbox.clear();
    rig.objects[3].current = 0.0;
    rig.check_cov();
    assert_eq!(rig.cov.outbox.len(), 1);
    assert_eq!(&rig.cov.outbox[0].frame[23..28], &[0x09, 0x55, 0x2E, 0x91, 0x00]);
}

#[test]
fn analog_notifications_need_a_cov_increment_change() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(0), Some(0));
    rig.cov.outbox.clear();
    rig.check_cov();
    assert!(rig.cov.outbox.is_empty(), "no change, no notification");
    rig.objects[0].current = 22.55; // below the 0.1 increment
    rig.check_cov();
    assert!(rig.cov.outbox.is_empty());
    rig.objects[0].current = 22.7; // 0.2 ≥ 0.1
    rig.check_cov();
    assert_eq!(rig.cov.outbox.len(), 1);
}

#[test]
fn out_of_service_change_is_notified_through_status_flags() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(0), Some(0));
    rig.cov.outbox.clear();
    rig.objects[0].out_of_service = true;
    rig.check_cov();
    assert_eq!(rig.cov.outbox.len(), 1);
    let frame = &rig.cov.outbox[0].frame;
    assert_eq!(&frame[frame.len() - 8..], &[0x09, 0x6F, 0x2E, 0x82, 0x04, 0x10, 0x2F, 0x4F]);
}

#[test]
fn unacknowledged_confirmed_notifications_are_retransmitted_then_the_subscription_is_dropped() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(1), Some(60));
    let first = rig.cov.outbox.remove(0).frame;
    // Number_Of_APDU_Retries = 3, APDU_Timeout = 3 s: three identical retransmissions.
    for _ in 0..3 {
        rig.advance(2);
        rig.check_cov();
        assert!(rig.cov.outbox.is_empty(), "not before the APDU timeout");
        rig.advance(1);
        rig.check_cov();
        assert_eq!(rig.cov.outbox.len(), 1);
        assert_eq!(rig.cov.outbox.remove(0).frame, first, "same invoke id on retransmission");
    }
    rig.advance(3);
    rig.check_cov();
    assert!(rig.cov.outbox.is_empty());
    assert!(rig.cov.subs.is_empty(), "subscriber never acknowledged: subscription dropped");
}

#[test]
fn confirmed_notification_ack_stops_retransmission() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(1), Some(60));
    rig.cov.outbox.clear();
    // SimpleAck, invoke 0 (the notification's), ConfirmedCOVNotification(1).
    assert!(rig.send(&unicast(&[0x20, 0x00, 0x01])).is_none());
    rig.advance(10);
    rig.check_cov();
    assert!(rig.cov.outbox.is_empty());
    assert_eq!(rig.cov.subs.len(), 1);
}

#[test]
fn error_reject_or_abort_from_the_subscriber_cancels_the_subscription() {
    for answer in [vec![0x50, 0x00, 0x01, 0x91, 0x05, 0x91, 0x00], vec![0x60, 0x00, 0x09], vec![0x70, 0x00, 0x04]] {
        let mut rig = Rig::standard();
        subscribe(&mut rig, 7, AI1, Some(1), Some(60));
        assert!(rig.send(&unicast(&answer)).is_none());
        assert!(rig.cov.subs.is_empty(), "answer {answer:02X?}");
    }
    // An acknowledgement from somebody else does not count.
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(1), Some(60));
    rig.cov.outbox.clear();
    let other: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    assert!(rig.send_from(other, &unicast(&[0x20, 0x00, 0x01])).is_none());
    rig.advance(3);
    rig.check_cov();
    assert_eq!(rig.cov.outbox.len(), 1, "still retransmitting");
}

#[test]
fn confirmed_notifications_use_distinct_invoke_ids() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 1, AI1, Some(1), Some(60));
    subscribe(&mut rig, 2, AI1, Some(1), Some(60));
    let ids: Vec<u8> = rig.cov.outbox.iter().map(|r| r.frame[8]).collect();
    assert_eq!(ids, [0, 1]);
}

#[test]
fn subscriptions_expire_after_their_lifetime() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(0), Some(2));
    rig.advance(1);
    rig.check_cov();
    assert_eq!(rig.cov.subs.len(), 1);
    rig.advance(1);
    rig.check_cov();
    assert!(rig.cov.subs.is_empty());
}

#[test]
fn subscribe_renews_and_cancel_matches_process_and_object() {
    let mut rig = Rig::standard();
    subscribe(&mut rig, 7, AI1, Some(0), Some(0));
    subscribe(&mut rig, 7, AI1, Some(0), Some(0)); // same key: renewed, not duplicated
    assert_eq!(rig.cov.subs.len(), 1);
    subscribe(&mut rig, 8, AI1, Some(0), Some(0));
    assert_eq!(rig.cov.subs.len(), 2);
    // Neither optional parameter = cancellation of exactly (subscriber, process 7, object).
    assert_eq!(subscribe(&mut rig, 7, AI1, None, None), [0x20, 0x01, 0x05]);
    assert_eq!(rig.cov.subs.len(), 1);
    assert_eq!(rig.cov.subs[0].proc_id, 8);
    // Cancelling an unknown object is an error like any other request for it.
    assert_eq!(subscribe(&mut rig, 7, [0x00, 0x00, 0x00, 0x63], None, None), [0x50, 0x01, 0x05, 0x91, 0x01, 0x91, 0x1F]);
}

#[test]
fn subscribe_request_validation() {
    let mut rig = Rig::standard();
    // Unknown object / the device object (no COV on the device).
    assert_eq!(
        subscribe(&mut rig, 7, [0x00, 0x00, 0x00, 0x63], Some(0), Some(0)),
        [0x50, 0x01, 0x05, 0x91, 0x01, 0x91, 0x1F]
    );
    assert_eq!(subscribe(&mut rig, 7, DEV, Some(0), Some(0)), [0x50, 0x01, 0x05, 0x91, 0x05, 0x91, 0x1D]);
    // Context Boolean must be 0 or 1; trailing data; missing object.
    let bad_bool = confirmed(SVC_SUBSCRIBE_COV, &subscribe_body(7, AI1, Some(2), None));
    assert_eq!(rig.apdu(&bad_bool), [0x60, 0x01, 0x03]);
    let trailing = confirmed(SVC_SUBSCRIBE_COV, &cat(&[&subscribe_body(7, AI1, Some(0), Some(0)), &[0x49, 0x01]]));
    assert_eq!(rig.apdu(&trailing), [0x60, 0x01, 0x07]);
    assert_eq!(rig.apdu(&confirmed(SVC_SUBSCRIBE_COV, &[0x09, 0x07])), [0x60, 0x01, 0x05]);
    assert!(rig.cov.subs.is_empty());
}

#[test]
fn notifications_to_a_routed_subscriber_carry_its_network_address() {
    let mut rig = Rig::standard();
    let npdu = [0x01, 0x08, 0x00, 0x07, 0x03, 0x01, 0x02, 0x03];
    let req = confirmed(SVC_SUBSCRIBE_COV, &subscribe_body(7, AI1, Some(0), Some(0)));
    let reply = rig.send(&bvlc(0x0A, &cat(&[&npdu, &req]))).expect("SimpleAck");
    assert_eq!(reply_apdu(&reply), [0x20, 0x01, 0x05]);
    let notification = &rig.cov.outbox[0].frame;
    // NPDU: DNET present, DNET 7, DLEN 3, DADR 01 02 03, hop count 255; then UnconfirmedCOVNotification.
    assert_eq!(&notification[4..15], &[0x01, 0x20, 0x00, 0x07, 0x03, 0x01, 0x02, 0x03, 0xFF, 0x10, 0x02]);
}

// ───────────────────────── Control panel and model ─────────────────────────

#[test]
fn control_panel_values_respect_the_priority_array() {
    let mut rig = Rig::standard();
    // Analog output: the panel commands at priority 16.
    rig.objects[2].ui_set(70.0);
    assert_eq!(rig.objects[2].current, 70.0);
    assert_eq!(rig.objects[2].slots[15], Some(70.0));
    // A client command at priority 8 outranks the panel.
    assert_eq!(write_ao(&mut rig, &REAL_50, &[0x49, 0x08]), [0x20, 0x01, 0x0F]);
    rig.objects[2].ui_set(10.0);
    assert_eq!(rig.objects[2].current, 50.0);
    assert_eq!(rig.objects[2].slots[15], Some(10.0));
    // Inputs have no priority array: the panel forces the sensor value and the tick keeps its hands off.
    rig.objects[0].ui_set(31.5);
    tick_values(&mut rig.objects, &Instant::now(), true);
    assert_eq!(rig.objects[0].current, 31.5);
    // Binary / multi-state values are normalised to their datatype.
    rig.objects[4].ui_set(0.7);
    assert_eq!(rig.objects[4].current, 1.0);
    rig.objects[5].ui_set(9.0);
    assert_eq!(rig.objects[5].current, 4.0, "clamped to Number_Of_States");
    rig.objects[5].ui_set(0.0);
    assert_eq!(rig.objects[5].current, 1.0, "multi-state values start at 1");
}

#[test]
fn point_definitions_are_validated() {
    let bad = |p: SimPointDef| build_objects(&[p]).err();
    assert!(bad(pt("x", "flux", 1, 0.0, false, false)).is_some(), "unknown object type");
    assert!(bad(pt("x", "device", 1, 0.0, false, false)).is_some(), "not a point type");
    assert!(bad(pt("x", "analogValue", 1, 0.0, false, true)).is_some(), "is_bool on a non-binary object");
    assert!(bad(pt("x", "binaryValue", 1, 0.0, false, false)).is_some(), "binary object without is_bool");
    assert!(bad(pt("x", "analogInput", 1, 0.0, true, false)).is_some(), "writable input");
    let dup = [pt("a", "analogInput", 1, 0.0, false, false), pt("b", "analogInput", 1, 0.0, false, false)];
    assert!(build_objects(&dup).err().is_some(), "duplicate object");
    let all = [
        pt("ai", "analogInput", 1, 0.0, false, false),
        pt("ao", "analogOutput", 1, 0.0, true, false),
        pt("av", "analogValue", 1, 0.0, true, false),
        pt("bi", "binaryInput", 1, 0.0, false, true),
        pt("bo", "binaryOutput", 1, 0.0, true, true),
        pt("bv", "binaryValue", 1, 0.0, true, true),
        pt("mi", "multiStateInput", 1, 1.0, false, false),
        pt("mo", "multiStateOutput", 1, 1.0, true, false),
        pt("mv", "multiStateValue", 1, 1.0, true, false),
    ];
    assert!(build_objects(&all).is_ok());
}

// ───────────────────────── Cross-check with the bacnet-rs codec ─────────────────────────

fn rs_request(service_choice: ConfirmedServiceChoice, service_data: Vec<u8>) -> Vec<u8> {
    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_data,
    };
    let mut msg = Npdu::new().encode();
    msg.extend_from_slice(&apdu.encode());
    bvlc(0x0A, &msg)
}

fn rs_ack_data(r: &Reply) -> Vec<u8> {
    match Apdu::decode(&reply_apdu(r)).unwrap() {
        Apdu::ComplexAck { service_data, .. } => service_data,
        other => panic!("expected ComplexAck, got {other:?}"),
    }
}

fn as_real(v: &PropertyValue) -> Option<f32> {
    if let PropertyValue::Real(f) = v { Some(*f) } else { None }
}

#[test]
fn bacnet_rs_can_talk_to_the_simulator() {
    use bacnet_rs::service::PropertyResultValue;
    let mut rig = Rig::standard();

    // ReadProperty request encoded by bacnet-rs, ACK decoded by bacnet-rs.
    let temp = ObjectIdentifier::new(ObjectType::AnalogInput, 1);
    let mut sd = Vec::new();
    ReadPropertyRequest::new(temp, PropertyIdentifier::PresentValue).encode(&mut sd).unwrap();
    let reply = rig.send(&rs_request(ConfirmedServiceChoice::ReadProperty, sd)).unwrap();
    let parsed = ReadPropertyResponse::decode(&rs_ack_data(&reply)).unwrap();
    assert_eq!(as_real(&parsed.property_values[0]), Some(22.5));

    // ReadPropertyMultiple over two objects.
    let sp = ObjectIdentifier::new(ObjectType::AnalogValue, 1);
    let specs = vec![
        ReadAccessSpecification::new(temp, vec![PropertyReference::new(PropertyIdentifier::PresentValue)]),
        ReadAccessSpecification::new(sp, vec![PropertyReference::new(PropertyIdentifier::PresentValue)]),
    ];
    let mut sd = Vec::new();
    ReadPropertyMultipleRequest::new(specs).encode(&mut sd).unwrap();
    let reply = rig.send(&rs_request(ConfirmedServiceChoice::ReadPropertyMultiple, sd)).unwrap();
    let rpm = ReadPropertyMultipleResponse::decode(&rs_ack_data(&reply)).unwrap();
    assert_eq!(rpm.read_access_results.len(), 2);
    for (result, expected) in rpm.read_access_results.iter().zip([22.5, 23.0]) {
        match &result.results[0].value {
            PropertyResultValue::Value(vs) => assert_eq!(as_real(&vs[0]), Some(expected)),
            other => panic!("unexpected result {other:?}"),
        }
    }

    // WriteProperty then read back.
    let pid: u32 = PropertyIdentifier::PresentValue.into();
    let mut value = vec![0x44];
    value.extend_from_slice(&21.0f32.to_be_bytes());
    let mut sd = Vec::new();
    WritePropertyRequest::new(sp, pid, value).encode(&mut sd).unwrap();
    let reply = rig.send(&rs_request(ConfirmedServiceChoice::WriteProperty, sd)).unwrap();
    assert!(matches!(Apdu::decode(&reply_apdu(&reply)).unwrap(), Apdu::SimpleAck { .. }));
    let mut sd = Vec::new();
    ReadPropertyRequest::new(sp, PropertyIdentifier::PresentValue).encode(&mut sd).unwrap();
    let reply = rig.send(&rs_request(ConfirmedServiceChoice::ReadProperty, sd)).unwrap();
    let parsed = ReadPropertyResponse::decode(&rs_ack_data(&reply)).unwrap();
    assert_eq!(as_real(&parsed.property_values[0]), Some(21.0));
}
