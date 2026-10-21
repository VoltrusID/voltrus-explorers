//! Wire-format conformance tests.
//!
//! The byte vectors come from real-device traffic published in
//! cpppo's `server/enip_test.py` (https://github.com/pjkundert/cpppo — Wireshark
//! captures of ControlLogix/CompactLogix/PowerFlex/MicroLogix/ANC-120e targets;
//! quoted below as "cpppo capture <variable>") and from layouts in ODVA CIP
//! Vol 1/Vol 2 and Rockwell 1756-PM020, cross-checked with pycomm3 (`packets/`,
//! `cip/`). Vectors marked "spec-derived" were assembled by hand from the
//! layout tables because no public capture exists for them (Large_Forward_Open,
//! UDP implicit I/O framing, route paths with IP hops); their expected bytes
//! are written out literally, not computed with the code under test.

use crate::cip::*;
use crate::client::{parse_identity_attributes, parse_symbol_records, symbol_to_tag, Symbol};
use crate::packets::*;
use crate::types::*;

fn h(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

/// Overwrite the sender-context field and the SendRRData timeout so a frame
/// built by us (counter context, 10 s timeout) can be compared byte-for-byte
/// with a capture.
fn align_to(mut ours: Vec<u8>, capture: &[u8], timeout_at: Option<usize>) -> Vec<u8> {
    ours[12..20].copy_from_slice(&capture[12..20]);
    ours[4..8].copy_from_slice(&capture[4..8]);
    if let Some(t) = timeout_at {
        ours[t..t + 2].copy_from_slice(&capture[t..t + 2]);
    }
    ours
}

// ───────────────────────── Encapsulation ─────────────────────────

#[test]
fn register_session_request_matches_capture() {
    // cpppo capture rss_004_request
    let capture = h("65 00 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00");
    let ours = build_enip_header(CMD_REGISTER_SESSION, 0, 0, &[1, 0, 0, 0]);
    assert_eq!(align_to(ours, &capture, None), capture);
}

#[test]
fn send_rr_data_get_attributes_all_matches_capture() {
    // cpppo capture gaa_008_request: Get_Attributes_All class 0x66 instance 1.
    let capture = h("6f 00 16 00 01 1e 02 11 00 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 05 00 02 00 \
                     00 00 00 00 b2 00 06 00 01 02 20 66 24 01");
    let req = build_generic_cip_request(CIP_GET_ATTRIBUTES_ALL, 0x66, 1, None, &[]);
    assert_eq!(req, h("01 02 20 66 24 01"));
    let ours = build_send_rr_data(0x11021E01, &req, &[]);
    // The capture's originator used a 5 s timeout; we use RR_TIMEOUT_SECS.
    assert_eq!(u16::from_le_bytes([ours[28], ours[29]]), RR_TIMEOUT_SECS);
    assert_eq!(align_to(ours, &capture, Some(28)), capture);
}

#[test]
fn send_unit_data_matches_capture() {
    // cpppo capture snd_u01_req (connected message on connection 0x8DEE0016).
    let capture = h("70 00 23 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 02 00 \
                     a1 00 04 00 16 00 ee 8d b1 00 0f 00 01 00 00 00 01 00 00 00 00 00 06 00 4a 0a 03");
    let (conn, seq, data) = parse_send_unit_data(&capture).unwrap();
    assert_eq!(conn, 0x8DEE0016);
    assert_eq!(seq, 1);
    let ours = build_send_unit_data(1, conn, seq, &data);
    assert_eq!(align_to(ours, &capture, Some(28)), capture);
}

#[test]
fn register_session_reply_session_handle_offset() {
    // cpppo capture rss_004_reply: the session handle lives at header bytes 4..8.
    let capture = h("65 00 04 00 01 1e 02 11 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00");
    assert_eq!(u32::from_le_bytes(capture[4..8].try_into().unwrap()), 0x11021E01);
}

#[test]
fn encapsulation_status_names() {
    assert!(encapsulation_status_text(0x64).contains("session"));
    assert!(encapsulation_status_text(0x69).contains("protocol"));
}

// ───────────────────────── ListIdentity ─────────────────────────

#[test]
fn list_identity_request_is_empty_command_0x63() {
    // cpppo capture listident_1_req: `63 00 00 00 …` all-zero header.
    let req = build_list_identity_request();
    assert_eq!(&req[..4], &[0x63, 0x00, 0x00, 0x00]);
    assert_eq!(req.len(), 24);
}

#[test]
fn list_identity_real_logix_reply() {
    // cpppo capture listident_2_rpy — 1769-L24ER-QB1B/A LOGIX5324ER.
    // Item data starts with protocol version + sockaddr_in; `state` follows the
    // product name with no pad byte.
    let capture = h("63 00 45 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 0c 00 3f 00 01 00 \
                     00 02 af 12 0a a1 01 03 00 00 00 00 00 00 00 00 01 00 0e 00 95 00 1b 0b 30 00 5e 33 1e c0 1d 31 \
                     37 36 39 2d 4c 32 34 45 52 2d 51 42 31 42 2f 41 20 4c 4f 47 49 58 35 33 32 34 45 52 03");
    let dev = parse_list_identity_response(&capture).expect("parses");
    assert_eq!(dev.vendor_id, 1);
    assert_eq!(dev.vendor_name, "Rockwell Automation/Allen-Bradley");
    assert_eq!(dev.device_type, 0x000E);
    assert_eq!(dev.product_code, 0x0095);
    assert_eq!((dev.revision_major, dev.revision_minor), (27, 11));
    assert_eq!(dev.status, 0x0030);
    assert_eq!(dev.serial_number, 0xC01E335E);
    assert_eq!(dev.product_name, "1769-L24ER-QB1B/A LOGIX5324ER");
    assert_eq!(dev.state, 3);
    assert_eq!(dev.port, 44818);
}

#[test]
fn list_identity_real_powerflex_reply() {
    // cpppo capture listident_1_rpy (frame length corrected as in the test) —
    // PowerFlex 753, product name space-padded to 32 characters, state 0xFF.
    let capture = h("63 00 48 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 0c 00 42 00 01 00 \
                     00 02 af 12 0a a1 01 05 00 00 00 00 00 00 00 00 01 00 7b 00 90 04 0b 01 61 05 15 1d 49 80 20 50 \
                     6f 77 65 72 46 6c 65 78 20 37 35 33 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 ff");
    let dev = parse_list_identity_response(&capture).expect("parses");
    assert_eq!(dev.vendor_id, 1);
    assert_eq!(dev.device_type, 0x7B);
    assert_eq!(dev.product_code, 0x0490);
    assert_eq!((dev.revision_major, dev.revision_minor), (11, 1));
    assert_eq!(dev.status, 0x0561);
    assert_eq!(dev.serial_number, 0x80491D15);
    assert_eq!(dev.product_name, "PowerFlex 753");
    assert_eq!(dev.state, 0xFF);
}

#[test]
fn simulator_list_identity_reply_has_spec_layout() {
    let r = build_list_identity_response([10, 1, 2, 3], 44818);
    let items = parse_cpf(&r[24..]).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].type_id, 0x000C);
    let d = &items[0].data;
    assert_eq!(&d[0..2], &[0x01, 0x00], "encapsulation protocol version");
    assert_eq!(&d[2..4], &[0x00, 0x02], "sin_family AF_INET, big-endian");
    assert_eq!(&d[4..6], &[0xAF, 0x12], "sin_port 44818, big-endian");
    assert_eq!(&d[6..10], &[10, 1, 2, 3], "sin_addr");
    assert_eq!(&d[10..18], &[0u8; 8], "sin_zero");
    assert_eq!(&d[18..20], &[0x01, 0x00], "vendor id");
    let dev = parse_list_identity_response(&r).unwrap();
    assert_eq!(dev.product_name, "ENIP-Sim");
    assert_eq!(dev.state, 3);
    assert_eq!(dev.port, 44818);
}

#[test]
fn list_identity_rejects_other_commands_and_errors() {
    let pkt = build_enip_header(CMD_REGISTER_SESSION, 0, 0, &[]);
    assert!(parse_list_identity_response(&pkt).is_none());
    let mut bad = build_list_identity_response([0; 4], 44818);
    bad[8] = 1; // encapsulation status
    assert!(parse_list_identity_response(&bad).is_none());
}

// ───────────────────────── Identity object ─────────────────────────

#[test]
fn identity_get_attributes_all_reply_1756_l61() {
    // cpppo capture cpf_2_rpy: Get_Attributes_All reply of a 1756-L61/B. It has
    // attributes 1–7 only (no state byte).
    let reply = h("81 00 00 00 01 00 0e 00 36 00 14 0b 60 31 1a 06 6c 00 14 31 37 35 36 2d 4c 36 31 2f 42 20 4c 4f 47 49 58 35 35 36 31");
    let data = CipReply::parse(&reply).unwrap().expect(CIP_GET_ATTRIBUTES_ALL, false).unwrap().data;
    let id = parse_identity_attributes(&data).expect("identity");
    assert_eq!(id.vendor_id, 1);
    assert_eq!(id.device_type, 0x0E);
    assert_eq!(id.product_code, 0x36);
    assert_eq!((id.revision_major, id.revision_minor), (20, 11));
    assert_eq!(id.status, 0x3160);
    assert_eq!(id.serial_number, 0x006C061A);
    assert_eq!(id.product_name, "1756-L61/B LOGIX5561");
}

#[test]
fn vendor_ids_follow_the_odva_registry() {
    // pycomm3 status_info._VENDORS (ODVA vendor list)
    assert_eq!(vendor_name(1), "Rockwell Automation/Allen-Bradley");
    assert_eq!(vendor_name(47), "Omron");
    assert_eq!(vendor_name(108), "Beckhoff Automation");
    assert_eq!(vendor_name(1251), "Siemens");
    // Previously mislabelled: 9 is not Yaskawa, 40 is not ABB, 114 is not Beckhoff.
    assert_ne!(vendor_name(9), "Yaskawa");
    assert_eq!(vendor_name(40), "WAGO");
}

// ───────────────────────── EPaths ─────────────────────────

#[test]
fn logical_epath_uses_smallest_segment_widths() {
    assert_eq!(build_epath(0x6B, 1, Some(1)), (h("20 6b 24 01 30 01"), 3));
    assert_eq!(build_epath(0x1234, 1, None), (h("21 00 34 12 24 01"), 3));
    // 16-bit instance: segment 0x25 + pad + LE word
    assert_eq!(build_epath(0x04, 0x0102, None), (h("20 04 25 00 02 01"), 3));
    // 32-bit instance (0x26): previously truncated to 16 bits
    assert_eq!(build_epath(0x04, 0x0001_0002, None), (h("20 04 26 00 02 00 01 00"), 4));
}

#[test]
fn symbolic_path_single_tag() {
    // cpppo capture readfrag_1_req: `91 05 SCADA` + pad.
    assert_eq!(encode_symbolic_path("SCADA").unwrap(), h("91 05 53 43 41 44 41 00"));
    // even-length names get no pad byte
    assert_eq!(encode_symbolic_path("Test").unwrap(), h("91 04 54 65 73 74"));
}

#[test]
fn symbolic_path_array_element() {
    // cpppo capture writetag_1_req: SCADA[12] → 5 words.
    assert_eq!(encode_symbolic_path("SCADA[12]").unwrap(), h("91 05 53 43 41 44 41 00 28 0c"));
    // multi-dimensional + wide indices (0x28 / 0x29 / 0x2A member segments)
    assert_eq!(encode_symbolic_path("G[1,300]").unwrap(), h("91 01 47 00 28 01 29 00 2c 01"));
    assert_eq!(encode_symbolic_path("A[70000]").unwrap(), h("91 01 41 00 2a 00 70 11 01 00"));
}

#[test]
fn symbolic_path_splits_dotted_names_into_segments() {
    // pycomm3 tag_request_path: one 0x91 segment per dotted component.
    let p = encode_symbolic_path("Program:Main.Tank[1].Level").unwrap();
    let mut want = vec![0x91, 12];
    want.extend_from_slice(b"Program:Main");
    want.extend_from_slice(&[0x91, 0x04]);
    want.extend_from_slice(b"Tank");
    want.extend_from_slice(&[0x28, 0x01, 0x91, 0x05]);
    want.extend_from_slice(b"Level");
    want.push(0x00);
    assert_eq!(p, want);
    assert_eq!(decode_symbolic_path(&p).as_deref(), Some("Program:Main.Tank[1].Level"));
    // multi-dim round trip
    let g = encode_symbolic_path("G[1,300].X").unwrap();
    assert_eq!(decode_symbolic_path(&g).as_deref(), Some("G[1,300].X"));
}

#[test]
fn symbolic_path_rejects_garbage() {
    for bad in ["", ".A", "A.", "[1]", "A[", "A[x]", "A]"] {
        assert!(encode_symbolic_path(bad).is_err(), "{bad:?} should be rejected");
    }
    assert!(encode_symbolic_path("Tag.5").is_err(), "bit selectors are resolved by the client");
}

#[test]
fn bit_and_element_suffixes() {
    assert_eq!(split_bit_suffix("Status.5"), Some(("Status", 5)));
    assert_eq!(split_bit_suffix("A.B.31"), Some(("A.B", 31)));
    assert_eq!(split_bit_suffix("Status.Run"), None);
    assert_eq!(split_bit_suffix("Arr[2]"), None);
    assert_eq!(split_element_suffix("Arr[2]"), Some(("Arr", 2)));
    assert_eq!(split_element_suffix("Arr[1,2]"), None);
}

// ───────────────────────── Tag services ─────────────────────────

#[test]
fn read_tag_request_layout() {
    // 1756-PM020: service 0x4C, path size (words), symbolic path, element count.
    assert_eq!(build_read_tag_request("Test", 1).unwrap(), h("4c 03 91 04 54 65 73 74 01 00"));
    assert_eq!(build_read_tag_request("SCADA", 20).unwrap(), h("4c 04 91 05 53 43 41 44 41 00 14 00"));
}

#[test]
fn read_tag_fragmented_request_matches_capture() {
    // cpppo capture readfrag_1_req
    assert_eq!(
        build_read_tag_fragmented_request("SCADA", 20, 2).unwrap(),
        h("52 04 91 05 53 43 41 44 41 00 14 00 02 00 00 00")
    );
}

#[test]
fn read_tag_fragmented_reply_matches_capture() {
    // cpppo capture readfrag_1_rpy: INT[19] starting at the requested offset.
    let reply = h("d2 00 00 00 c3 00 4c 10 08 00 03 00 02 00 02 00 02 00 0e 00 00 00 00 00 e6 42 07 00 c8 40 c8 40 00 00 \
                   e4 00 00 00 64 00 b2 02 c8 80");
    let r = parse_tag_read_reply(&reply, CIP_READ_TAG_FRAGMENTED).unwrap();
    assert_eq!(r.ty, TagType::atomic(CIP_TYPE_INT));
    assert!(!r.partial);
    let v = decode_tag_value(&r.ty, &r.data, 19).unwrap();
    let ints = [
        0x104C, 8, 3, 2, 2, 2, 0xE, 0, 0, 0x42E6, 7, 0x40C8, 0x40C8, 0, 0xE4, 0, 0x64, 0x2B2, 0x80C8u16 as i16 as i32,
    ];
    let CipValue::Array(items) = v else { panic!("expected array") };
    assert_eq!(items.len(), 19);
    for (item, want) in items.iter().zip(ints) {
        assert_eq!(*item, CipValue::Int(want as u16 as i16));
    }
}

#[test]
fn read_tag_partial_status_is_flagged() {
    // status 0x06 with data present: caller must continue with 0x52.
    let reply = h("cc 00 06 00 c4 00 01 00 00 00 02 00 00 00");
    let r = parse_tag_read_reply(&reply, CIP_READ_TAG).unwrap();
    assert!(r.partial);
    assert_eq!(r.data.len(), 8);
}

#[test]
fn write_tag_fragmented_request_matches_capture() {
    // cpppo capture writetag_1_req: SCADA[12], INT, 1 element, offset 0, data C9 40.
    let req = build_write_tag_fragmented_request("SCADA[12]", &TagType::atomic(CIP_TYPE_INT), 1, 0, &[0xC9, 0x40]).unwrap();
    assert_eq!(req, h("53 05 91 05 53 43 41 44 41 00 28 0c c3 00 01 00 00 00 00 00 c9 40"));
    // cpppo capture writetag_1_rpy
    parse_tag_write_reply(&h("d3 00 00 00"), CIP_WRITE_TAG_FRAGMENTED).unwrap();
}

#[test]
fn write_tag_request_layout() {
    // 4D | path | type (2 bytes) | count | data
    let req = build_write_tag_request("Test", &TagType::atomic(CIP_TYPE_DINT), 1, &5i32.to_le_bytes()).unwrap();
    assert_eq!(req, h("4d 03 91 04 54 65 73 74 c4 00 01 00 05 00 00 00"));
}

#[test]
fn logix_atomic_write_encodings() {
    // BOOL: ONE byte, 0xFF = true (pycomm3 BOOL.encode). The old encoder sent 4 bytes.
    let t = TagType::atomic(CIP_TYPE_BOOL);
    assert_eq!(encode_tag_value(&t, &CipValue::Bool(true)).unwrap(), (1, vec![0xFF]));
    assert_eq!(encode_tag_value(&t, &CipValue::Bool(false)).unwrap(), (1, vec![0x00]));
    // SINT: ONE byte
    let t = TagType::atomic(CIP_TYPE_SINT);
    assert_eq!(encode_tag_value(&t, &CipValue::Sint(-2)).unwrap(), (1, vec![0xFE]));
    // numeric coercion to the tag's type
    let t = TagType::atomic(CIP_TYPE_REAL);
    assert_eq!(encode_tag_value(&t, &CipValue::Dint(3)).unwrap(), (1, 3.0f32.to_le_bytes().to_vec()));
    let t = TagType::atomic(CIP_TYPE_DINT);
    assert_eq!(encode_tag_value(&t, &CipValue::Real(2.6)).unwrap(), (1, 3i32.to_le_bytes().to_vec()));
    // range checks
    assert!(encode_tag_value(&TagType::atomic(CIP_TYPE_SINT), &CipValue::Dint(300)).is_err());
    assert!(encode_tag_value(&TagType::atomic(CIP_TYPE_UINT), &CipValue::Dint(-1)).is_err());
    // arrays
    let arr = CipValue::Array(vec![CipValue::Int(1), CipValue::Int(2), CipValue::Int(-1)]);
    assert_eq!(
        encode_tag_value(&TagType::atomic(CIP_TYPE_INT), &arr).unwrap(),
        (3, h("01 00 02 00 ff ff"))
    );
}

#[test]
fn logix_string_is_a_structure_with_handle_0fce() {
    // 1756-PM020: STRING tags are structures: type `A0 02 CE 0F`, data = DINT LEN + 82 chars.
    let t = TagType::logix_string();
    assert_eq!(t.encode(), h("a0 02 ce 0f"));
    let (count, data) = encode_tag_value(&t, &CipValue::String("Hello".into())).unwrap();
    assert_eq!(count, 1);
    assert_eq!(data.len(), 88);
    assert_eq!(&data[..9], &h("05 00 00 00 48 65 6c 6c 6f")[..]);
    assert!(data[9..].iter().all(|&b| b == 0));
    assert_eq!(decode_tag_value(&t, &data, 1).unwrap(), CipValue::String("Hello".into()));
    assert!(encode_tag_value(&t, &CipValue::String("x".repeat(83))).is_err(), "longer than DATA[82]");

    // Reply type field parse: A0 02 <handle> then the 88-byte image.
    let mut reply = h("cc 00 00 00 a0 02 ce 0f");
    reply.extend_from_slice(&data);
    let r = parse_tag_read_reply(&reply, CIP_READ_TAG).unwrap();
    assert!(r.ty.is_logix_string());
    assert_eq!(r.data.len(), 88);
}

#[test]
fn unknown_structures_decode_to_raw_bytes() {
    let reply = h("cc 00 00 00 a0 02 34 12 01 02 03 04");
    let r = parse_tag_read_reply(&reply, CIP_READ_TAG).unwrap();
    assert_eq!(r.ty, TagType::structure(0x1234));
    assert_eq!(decode_tag_value(&r.ty, &r.data, 1).unwrap(), CipValue::Bytes(vec![1, 2, 3, 4]));
}

#[test]
fn atomic_types_decode_by_code() {
    use CipValue::*;
    let cases: &[(u8, &str, CipValue)] = &[
        (CIP_TYPE_BOOL, "ff", Bool(true)),
        (CIP_TYPE_SINT, "fe", Sint(-2)),
        (CIP_TYPE_INT, "fe ff", Int(-2)),
        (CIP_TYPE_DINT, "fe ff ff ff", Dint(-2)),
        (CIP_TYPE_LINT, "fe ff ff ff ff ff ff ff", Lint(-2)),
        (CIP_TYPE_USINT, "fe", Usint(254)),
        (CIP_TYPE_UINT, "fe ff", Uint(65534)),
        (CIP_TYPE_UDINT, "fe ff ff ff", Udint(4294967294)),
        (CIP_TYPE_ULINT, "fe ff ff ff ff ff ff ff", Ulint(u64::MAX - 1)),
        (CIP_TYPE_REAL, "00 00 c0 3f", Real(1.5)),
        (CIP_TYPE_LREAL, "00 00 00 00 00 00 f8 3f", Lreal(1.5)),
        (CIP_TYPE_BYTE, "81", Usint(0x81)),
        (CIP_TYPE_WORD, "34 12", Uint(0x1234)),
        (CIP_TYPE_DWORD, "78 56 34 12", Bytes(vec![0x78, 0x56, 0x34, 0x12])),
    ];
    for (code, hex, want) in cases {
        assert_eq!(decode_tag_value(&TagType::atomic(*code), &h(hex), 1).unwrap(), *want, "code 0x{code:02X}");
    }
    // CIP STRING (UINT length) and SHORT_STRING (USINT length) attribute values
    assert_eq!(
        decode_tag_value(&TagType::atomic(CIP_TYPE_STRING), &h("02 00 68 69"), 1).unwrap(),
        CipValue::String("hi".into())
    );
    assert_eq!(
        decode_tag_value(&TagType::atomic(CIP_TYPE_SHORT_STRING), &h("02 68 69"), 1).unwrap(),
        CipValue::String("hi".into())
    );
}

#[test]
fn multiple_service_reply_capture_splits_into_17_replies() {
    // cpppo capture multiple_1_rpy
    let reply = h("8a 00 00 00 11 00 24 00 28 00 2c 00 30 00 34 00 40 00 44 00 52 00 62 00 66 00 6a 00 74 00 78 00 7c 00 80 00 \
                   8e 00 9c 00 cd 00 00 00 d3 00 00 00 cd 00 00 00 cd 00 00 00 cc 00 00 00 c3 00 00 00 00 00 00 00 cd 00 \
                   00 00 cc 00 00 00 c3 00 00 00 00 00 00 00 00 00 cc 00 00 00 c3 00 00 00 61 00 62 00 63 00 00 00 cd 00 \
                   00 00 cd 00 00 00 cc 00 00 00 c3 00 00 00 00 00 cd 00 00 00 cd 00 00 00 cd 00 00 00 d2 00 00 00 c3 00 \
                   61 03 62 03 00 00 00 00 cc 00 00 00 c3 00 00 00 00 00 00 00 00 00 cc 00 00 00 c3 00 00 00");
    let r = CipReply::parse(&reply).unwrap().expect(CIP_MULTIPLE_SERVICE_PACKET, false).unwrap();
    let parts = split_multiple_service_data(&r.data).unwrap();
    assert_eq!(parts.len(), 17);
    assert_eq!(parts[0], h("cd 00 00 00"), "first reply is a Write Tag ack");
    assert_eq!(parts[1], h("d3 00 00 00"), "second reply is a Write Tag Fragmented ack");
    assert_eq!(parts[4], h("cc 00 00 00 c3 00 00 00 00 00 00 00"), "Read Tag reply: INT[3]");
    for p in &parts {
        let rep = CipReply::parse(p).unwrap();
        assert_eq!(rep.status, 0);
        assert!(rep.service & 0x80 != 0);
    }
    // And the builder produces the layout the capture's offsets imply: offsets are
    // relative to the count field.
    let msp = build_multiple_service_packet(&[h("4c 03 91 04 54 65 73 74 01 00"), h("4c 02 91 01 41 00 01 00")]);
    assert_eq!(&msp[..6], &h("0a 02 20 02 24 01")[..]);
    assert_eq!(&msp[6..12], &h("02 00 06 00 10 00")[..]);
}

// ───────────────────────── Unconnected_Send / route path ─────────────────────────

#[test]
fn unconnected_send_matches_capture_layout() {
    // cpppo capture cpf_1: Unconnected_Send of Read Tag Fragmented SCADA, route `01 00`.
    let capture = h("52 02 20 06 24 01 05 9d 10 00 52 04 91 05 53 43 41 44 41 00 14 00 02 00 00 00 01 00 01 00");
    let (msg, route) = parse_unconnected_send(&capture).expect("parses");
    assert_eq!(msg, &h("52 04 91 05 53 43 41 44 41 00 14 00 02 00 00 00")[..]);
    assert_eq!(route, &[0x01, 0x00]);
    assert_eq!(msg, &build_read_tag_fragmented_request("SCADA", 20, 2).unwrap()[..]);

    // Our builder differs from the capture only in the priority/tick + timeout bytes.
    let mut ours = build_unconnected_send(msg, route);
    assert_eq!(&ours[6..8], &[0x0A, 0x05], "pycomm3 PRIORITY / TIMEOUT_TICKS");
    ours[6..8].copy_from_slice(&capture[6..8]);
    assert_eq!(ours, capture);
}

#[test]
fn unconnected_send_pads_odd_messages_before_the_route() {
    // 5-byte embedded message → pad byte, then route size / reserved / route.
    let msg = h("01 02 20 66 24");
    let wrapped = build_unconnected_send(&msg, &[0x01, 0x03]);
    assert_eq!(wrapped, h("52 02 20 06 24 01 0a 05 05 00 01 02 20 66 24 00 01 00 01 03"));
    let (m, r) = parse_unconnected_send(&wrapped).unwrap();
    assert_eq!(m, &msg[..]);
    assert_eq!(r, &[0x01, 0x03]);
}

#[test]
fn unconnected_send_reply_is_the_embedded_reply() {
    // cpppo capture cpf_1_rpy: the reply to an Unconnected_Send is the embedded
    // message's own reply (here service 0xD2 = Read Tag Fragmented reply).
    let reply = h("d2 00 00 00 c3 00 4c 10");
    let ok = CipReply::parse(&reply).unwrap().expect(CIP_READ_TAG_FRAGMENTED, true).unwrap();
    assert_eq!(ok.status, 0);
    // A routing failure comes back as a 0xD2 reply with extended status and must
    // surface as a typed error for *any* embedded service.
    let fail = h("d2 00 01 01 04 02");
    let err = CipReply::parse(&fail).unwrap().expect(CIP_READ_TAG, false).unwrap_err();
    let ce = err.downcast_ref::<CipError>().expect("typed CipError");
    assert_eq!((ce.status, ce.ext.clone()), (0x01, vec![0x0204]));
    assert!(ce.to_string().contains("timed out"));
}

#[test]
fn route_paths() {
    assert_eq!(parse_route_path("1,0").unwrap(), h("01 00"));
    assert_eq!(parse_route_path("1/2").unwrap(), h("01 02"));
    assert_eq!(parse_route_path("").unwrap(), Vec::<u8>::new());
    // spec-derived: Ethernet hop (port 2) to 10.0.0.5 as an extended link address
    // string, then backplane slot 0. Segment 0x12 = port 2 | extended-link bit.
    assert_eq!(
        parse_route_path("1,2,2,10.0.0.5,1,0").unwrap(),
        h("01 02 12 08 31 30 2e 30 2e 30 2e 35 01 00")
    );
    // odd-length address string is padded to a word boundary
    assert_eq!(parse_route_path("2,10.0.0.55").unwrap(), h("12 09 31 30 2e 30 2e 30 2e 35 35 00"));
    assert!(parse_route_path("1").is_err());
    assert!(parse_route_path("x,0").is_err());
    assert!(parse_route_path("0,0").is_err());
}

// ───────────────────────── Status codes ─────────────────────────

#[test]
fn cip_error_messages_carry_extended_status() {
    // Logix type mismatch: 0xFF with ext 0x2107 (1756-RM003).
    let reply = h("cd 00 ff 01 07 21");
    let e = CipReply::parse(&reply).unwrap().expect(CIP_WRITE_TAG, false).unwrap_err().to_string();
    assert!(e.contains("0xFF"), "{e}");
    assert!(e.contains("0x2107"), "{e}");
    assert!(e.contains("does not match"), "{e}");
    // Tag not found: 0x04 with one extended status word.
    let reply = h("cc 00 04 01 00 00");
    let c = CipReply::parse(&reply).unwrap();
    assert_eq!((c.status, c.ext.clone()), (0x04, vec![0]));
    // No extended status: still a typed error.
    let reply = h("cc 00 05 00");
    let e = CipReply::parse(&reply).unwrap().expect(CIP_READ_TAG, false).unwrap_err();
    assert_eq!(e.downcast_ref::<CipError>().unwrap().status, 0x05);
    // Truncated inside the extended status
    assert!(CipReply::parse(&h("cc 00 04 02 00 00")).is_err());
}

// ───────────────────────── Forward_Open / Forward_Close ─────────────────────────

fn rslinx_params() -> ForwardOpenParams {
    // Parameters of cpppo capture fwd_o02_request (RSLinx → ANC-120e).
    ForwardOpenParams {
        o_to_t_connection_id: 0x8000_0005,
        t_to_o_connection_id: 0x8000_0004,
        connection_serial: 0x0005,
        vendor_id: 0x004D,
        originator_serial: 0x385A_5837,
        priority_time_tick: 0x07,
        timeout_ticks: 0x9B,
        timeout_multiplier: 1,
        o_to_t_rpi_us: 4_000_000,
        o_to_t_params: 0x4302,
        t_to_o_rpi_us: 4_000_000,
        t_to_o_params: 0x4302,
        transport_class_trigger: 0xA3,
        large: false,
        o_to_t_run_idle: true,
        connection_path: h("01 01 20 a6 24 01 2c 01"),
    }
}

#[test]
fn forward_open_request_matches_rslinx_capture() {
    // cpppo capture fwd_o02_request, CIP part (CPF data item payload, 50 bytes).
    let capture = h("54 02 20 06 24 01 07 9b 05 00 00 80 04 00 00 80 05 00 4d 00 37 58 5a 38 01 00 00 00 00 09 3d 00 02 43 \
                     00 09 3d 00 02 43 a3 04 01 01 20 a6 24 01 2c 01");
    assert_eq!(capture.len(), 50);
    assert_eq!(build_forward_open_request(&rslinx_params()).unwrap(), capture);
}

#[test]
fn forward_open_request_parses_back() {
    let p = rslinx_params();
    let parsed = parse_forward_open_request(&build_forward_open_request(&p).unwrap()).unwrap();
    assert_eq!(parsed.o_to_t_connection_id, p.o_to_t_connection_id);
    assert_eq!(parsed.t_to_o_connection_id, p.t_to_o_connection_id);
    assert_eq!(parsed.connection_serial, p.connection_serial);
    assert_eq!(parsed.vendor_id, p.vendor_id);
    assert_eq!(parsed.originator_serial, p.originator_serial);
    assert_eq!(parsed.o_to_t_rpi_us, p.o_to_t_rpi_us);
    assert_eq!(parsed.o_to_t_params, p.o_to_t_params);
    assert_eq!(parsed.transport_class_trigger, p.transport_class_trigger);
    assert_eq!(parsed.connection_path, p.connection_path);
    assert!(!parsed.large);
}

#[test]
fn large_forward_open_uses_service_0x5b_and_32bit_parameters() {
    // spec-derived (ODVA Vol 1 Table 3-5.17): same layout as Forward_Open but the
    // two network connection parameter words are 32 bits wide.
    let mut p = rslinx_params();
    p.large = true;
    p.o_to_t_params = network_connection_params(4000, ConnectionType::PointToPoint, true, 0, true);
    p.t_to_o_params = network_connection_params(4000, ConnectionType::PointToPoint, true, 0, true);
    assert_eq!(p.o_to_t_params, 0x4200_0FA0);
    let req = build_forward_open_request(&p).unwrap();
    assert_eq!(req[0], 0x5B);
    assert_eq!(
        req,
        h("5b 02 20 06 24 01 07 9b 05 00 00 80 04 00 00 80 05 00 4d 00 37 58 5a 38 01 00 00 00 \
           00 09 3d 00 a0 0f 00 42 00 09 3d 00 a0 0f 00 42 a3 04 01 01 20 a6 24 01 2c 01")
    );
    let parsed = parse_forward_open_request(&req).unwrap();
    assert!(parsed.large);
    assert_eq!(parsed.o_to_t_params, 0x4200_0FA0);
    assert_eq!(parsed.connection_path, p.connection_path);
    // 16-bit Forward_Open cannot carry 32-bit parameters.
    p.large = false;
    assert!(build_forward_open_request(&p).is_err());
}

#[test]
fn network_connection_parameter_bits() {
    // cpppo/RSLinx captures use 0x43F4 for a 500-byte variable point-to-point connection;
    // pycomm3 `init_net_params = 0b0100_0010_0000_0000` (0x4200) | size.
    assert_eq!(network_connection_params(500, ConnectionType::PointToPoint, true, 0, false), 0x43F4);
    assert_eq!(network_connection_params(2, ConnectionType::PointToPoint, true, 0, false), 0x4202);
    // multicast fixed, scheduled priority: type 1 → bit 13, priority 2 → bit 11
    assert_eq!(network_connection_params(8, ConnectionType::Multicast, false, 2, false), 0x2808);
}

#[test]
fn forward_open_reply_matches_captures() {
    // cpppo capture fwd_o01_reply (CPF data item payload): ControlLogix grants 8 s API.
    let reply = h("d4 00 00 00 26 40 a3 ff 10 00 fe 80 11 00 4d 00 0f 7f 3d 1e 00 12 7a 00 00 12 7a 00 00 00");
    let r = parse_forward_open_response(&reply, false).unwrap();
    assert_eq!(r.o_to_t_connection_id, 0xFFA3_4026);
    assert_eq!(r.t_to_o_connection_id, 0x80FE_0010);
    assert_eq!(r.connection_serial, 0x0011);
    assert_eq!(r.vendor_id, 0x004D);
    assert_eq!(r.originator_serial, 0x1E3D_7F0F);
    assert_eq!(r.o_to_t_api_us, 8_000_000);
    assert_eq!(r.t_to_o_api_us, 8_000_000);

    // cpppo capture fwd_o02_reply (no trailing application data)
    let reply = h("d4 00 00 00 16 00 ee 8d 04 00 00 80 05 00 4d 00 37 58 5a 38 00 09 3d 00 00 09 3d 00 00 00");
    let r = parse_forward_open_response(&reply, false).unwrap();
    assert_eq!(r.o_to_t_connection_id, 0x8DEE_0016);
    assert_eq!(r.t_to_o_connection_id, 0x8000_0004);
}

#[test]
fn forward_open_failure_reports_extended_status() {
    // cpppo capture fwd_f01_reply (CPF payload): status 1, ext 0x0311 (port not
    // available), then connection serial/vendor/originator + remaining path size.
    let reply = h("d4 00 01 01 11 03 00 00 ff ff 78 56 34 12 01 00");
    let err = parse_forward_open_response(&reply, false).unwrap_err();
    let ce = err.downcast_ref::<CipError>().expect("typed CipError");
    assert_eq!(ce.status, 0x01);
    assert_eq!(ce.ext, vec![0x0311]);
    assert!(ce.to_string().contains("port not available"), "{ce}");
    // …and the same without the optional remaining-path-size bytes (…_no_remain).
    let reply = h("d4 00 01 01 11 03 00 00 ff ff 78 56 34 12");
    assert!(parse_forward_open_response(&reply, false).is_err());
}

#[test]
fn forward_close_request_matches_capture() {
    // cpppo capture fwd_c01_request, CIP part. Path size 3 words + reserved byte.
    let capture = h("4e 02 20 06 24 01 07 f9 01 00 4d 00 0f 7f 3d 1e 03 00 01 00 20 02 24 01");
    let mut ours = build_forward_close_request(0x0001, 0x004D, 0x1E3D_7F0F, &h("01 00 20 02 24 01"));
    assert_eq!(&ours[6..8], &[0x0A, 0x05]);
    ours[6..8].copy_from_slice(&capture[6..8]);
    assert_eq!(ours, capture);
}

// ───────────────────────── Implicit I/O (UDP 2222) ─────────────────────────

#[test]
fn io_packet_layout() {
    // spec-derived (ODVA Vol 2 §3-3.2.2): bare CPF, no encapsulation header.
    //   count 2 | 0x8002 len 8: connection id, sequence number |
    //   0x00B1 len 2+n: 16-bit sequence count, data
    let pkt = build_io_packet(0x1122_3344, 7, 9, &[0xAA, 0xBB, 0xCC]);
    assert_eq!(pkt, h("02 00 02 80 08 00 44 33 22 11 07 00 00 00 b1 00 05 00 09 00 aa bb cc"));
    let (conn, seq32, seq16, data) = parse_io_packet(&pkt).unwrap();
    assert_eq!((conn, seq32, seq16, data), (0x1122_3344, 7, 9, vec![0xAA, 0xBB, 0xCC]));
    assert!(parse_io_packet(&h("01 00 b1 00 02 00 01 00")).is_err(), "needs the sequenced address item");
}

#[test]
fn sockaddr_info_item_layout() {
    // sockaddr_in in network byte order: family 2, port, address, 8 zero bytes.
    let d = sockaddr_item_data([192, 168, 1, 20], 2222);
    assert_eq!(d, h("00 02 08 ae c0 a8 01 14 00 00 00 00 00 00 00 00"));
    assert_eq!(parse_sockaddr_item(&d), Some(([192, 168, 1, 20], 2222)));
}

#[test]
fn cpf_round_trip_and_truncation() {
    let items = vec![CpfItem::new(0, vec![]), CpfItem::new(0xB2, vec![1, 2, 3])];
    let b = build_cpf(&items);
    assert_eq!(b, h("02 00 00 00 00 00 b2 00 03 00 01 02 03"));
    assert_eq!(parse_cpf(&b).unwrap(), items);
    assert!(parse_cpf(&b[..b.len() - 1]).is_err());
}

// ───────────────────────── Browse (Symbol object) ─────────────────────────

fn symbol_record(id: u32, name: &str, type_word: u16, dims: [u32; 3]) -> Vec<u8> {
    // Get_Instance_Attribute_List reply record for attributes 1, 2, 8 (pycomm3
    // `_parse_instance_attribute_list` layout: UDINT id, STRING name, UINT type, UDINT[3]).
    let mut r = id.to_le_bytes().to_vec();
    r.extend_from_slice(&(name.len() as u16).to_le_bytes());
    r.extend_from_slice(name.as_bytes());
    r.extend_from_slice(&type_word.to_le_bytes());
    for d in dims {
        r.extend_from_slice(&d.to_le_bytes());
    }
    r
}

#[test]
fn symbol_records_parse() {
    let mut data = symbol_record(0x31, "Count", 0x00C4, [0; 3]);
    data.extend(symbol_record(0x35, "Arr", 0x20CA, [10, 0, 0]));
    let (syms, last) = parse_symbol_records(&data).unwrap();
    assert_eq!(syms.len(), 2);
    assert_eq!(syms[0].name, "Count");
    assert_eq!(syms[1].dims, [10, 0, 0]);
    assert_eq!(last, Some(0x35));
    assert!(parse_symbol_records(&data[..data.len() - 1]).is_err());
}

fn sym(name: &str, type_word: u16, dims: [u32; 3]) -> Symbol {
    Symbol { instance: 1, name: name.into(), type_word, dims }
}

#[test]
fn symbol_type_words_decode_like_pycomm3() {
    // atomic
    let t = symbol_to_tag("A", &sym("A", 0x00C4, [0; 3])).unwrap();
    assert_eq!((t.data_type, t.element_count, t.template_id), (CipDataType::Dint, 1, None));
    // 1-D array of REAL: dimension count in bits 13-14 (the old code read the
    // *high byte* of the type word as an element count)
    let t = symbol_to_tag("A", &sym("A", 0x20CA, [10, 0, 0])).unwrap();
    assert_eq!((t.data_type, t.element_count), (CipDataType::Real, 10));
    // 2-D array
    let t = symbol_to_tag("A", &sym("A", 0x40C4, [3, 4, 0])).unwrap();
    assert_eq!(t.element_count, 12);
    // BOOL with bit position 5 in bits 8-10 must still decode as BOOL
    let t = symbol_to_tag("A", &sym("A", 0x05C1, [0; 3])).unwrap();
    assert_eq!((t.data_type, t.element_count), (CipDataType::Bool, 1));
    // structure: bit 15 + 12-bit template instance id
    let t = symbol_to_tag("A", &sym("A", 0x8123, [0; 3])).unwrap();
    assert_eq!(t.template_id, Some(0x123));
    // built-in STRING (template 0x0FCE) is a string, not an anonymous struct
    let t = symbol_to_tag("A", &sym("A", 0x8FCE, [0; 3])).unwrap();
    assert_eq!((t.data_type, t.template_id), (CipDataType::String, None));
    // system tag bit 12, internal names
    assert!(symbol_to_tag("A", &sym("A", 0x10C4, [0; 3])).is_none());
    assert!(symbol_to_tag("A", &sym("__Hidden", 0x00C4, [0; 3])).is_none());
    assert!(symbol_to_tag("A", &sym("Routine:Main", 0x0000, [0; 3])).is_none());
    assert!(symbol_to_tag("A", &sym("Task:T1", 0x0000, [0; 3])).is_none());
    assert!(symbol_to_tag("A", &sym("Map:Foo", 0x00C4, [0; 3])).is_none());
}

// ───────────────────────── Misc ─────────────────────────

#[test]
fn tag_name_validation_accepts_logix_syntax() {
    assert!(validate_tag_name("Program:Main.Tank[1].Level").is_ok());
    assert!(validate_tag_name("G[1,2]").is_ok());
    assert!(validate_tag_name("1abc").is_err());
    assert!(validate_tag_name("a b").is_err());
}
