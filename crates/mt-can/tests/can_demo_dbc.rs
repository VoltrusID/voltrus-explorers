//! The CAN FD messages of the yocto image's can-demo (`tests/fixtures/can-demo.dbc`, copied from the
//! voltrus-yocto repo's docs/can-demo.dbc): parsing the FD attributes and decoding 64-byte payloads.

use std::collections::HashMap;

use mt_can::dbc::encode_message;
use mt_can::log::parse_frame;
use mt_can::{CanFrame, Dbc};

const DBC: &str = include_str!("fixtures/can-demo.dbc");

/// can-demo's telemetry payload (apps/can-demo/src/main.rs `telemetry_payload`).
fn telemetry(seq: u32, uptime_ms: u64, frames_rx: u64, start_count: u32) -> Vec<u8> {
    let mut p = vec![0u8; 64];
    p[0..4].copy_from_slice(&seq.to_le_bytes());
    p[4..8].copy_from_slice(b"FDT1");
    p[8..16].copy_from_slice(&uptime_ms.to_le_bytes());
    p[16..24].copy_from_slice(&frames_rx.to_le_bytes());
    p[24..28].copy_from_slice(&start_count.to_le_bytes());
    for (i, b) in p[32..].iter_mut().enumerate() {
        *b = (seq as u8).wrapping_add(i as u8);
    }
    p
}

fn values(d: &Dbc, f: &CanFrame) -> (String, HashMap<String, f64>) {
    let (m, sigs) = d.decode(f).expect("message in the DBC");
    (m.name.clone(), sigs.into_iter().map(|s| (s.name, s.value)).collect())
}

#[test]
fn parses_frame_formats() {
    let d = Dbc::parse(DBC).unwrap();
    assert_eq!(d.messages.len(), 6);
    let m = |name: &str| d.message_by_name(name).unwrap();
    // 64-byte messages are FD; BA_DEF_DEF_ "CANFD_BRS" "1" turns on the bit rate switch for them.
    for name in ["EdgeTelemetryFd", "EdgeTelemetryFdVcan", "HostFdPing"] {
        assert!(m(name).fd && m(name).brs, "{name}");
        assert_eq!(m(name).size, 64);
    }
    // Classic heartbeats stay classic, and BRS never applies to them.
    for name in ["EdgeHeartbeat", "EdgeHeartbeatVcan", "HostPing"] {
        assert!(!m(name).fd && !m(name).brs, "{name}");
    }
    assert!(m("EdgeTelemetryFd").comment.contains("FDT1"));
}

#[test]
fn decodes_fd_telemetry() {
    let d = Dbc::parse(DBC).unwrap();
    let data = telemetry(0x0102_03F0, 123_456_789, 4242, 7);
    let f = CanFrame::fd(0x201, false, true, &data).unwrap();
    let (name, v) = values(&d, &f);
    assert_eq!(name, "EdgeTelemetryFd");
    assert_eq!(v["Seq"], 0x0102_03F0 as f64);
    assert_eq!(v["Magic"], u32::from_le_bytes(*b"FDT1") as f64);
    assert_eq!(v["UptimeMs"], 123_456_789.0);
    assert_eq!(v["FramesRx"], 4242.0);
    assert_eq!(v["StartCount"], 7.0);
    // Bytes 32 and 63: the last signal ends at the last bit of a 64-byte frame.
    assert_eq!(v["PatternFirst"], 0xF0 as f64);
    assert_eq!(v["PatternLast"], ((0xF0 + 31) & 0xFF) as f64);
}

#[test]
fn decodes_from_candump_and_classic() {
    let d = Dbc::parse(DBC).unwrap();
    let hex: String = telemetry(5, 1000, 3, 1).iter().map(|b| format!("{b:02X}")).collect();
    let f = parse_frame(&format!("201##1{hex}")).unwrap();
    assert_eq!(values(&d, &f).1["PatternLast"], 36.0);
    // The heartbeat on the same bus: seq + "HBT1".
    let mut hb = 9u32.to_le_bytes().to_vec();
    hb.extend_from_slice(b"HBT1");
    let (name, v) = values(&d, &CanFrame::new(0x101, &hb).unwrap());
    assert_eq!((name.as_str(), v["Seq"]), ("EdgeHeartbeat", 9.0));
    // A truncated FD frame decodes what fits and leaves out signals past its end.
    let short = CanFrame::fd(0x201, false, true, &telemetry(5, 1000, 3, 1)[..32]).unwrap();
    let (_, v) = values(&d, &short);
    assert!(v.contains_key("StartCount") && !v.contains_key("PatternFirst"));
}

#[test]
fn encodes_host_fd_ping() {
    let d = Dbc::parse(DBC).unwrap();
    let m = d.message_by_name("HostFdPing").unwrap();
    let vals = HashMap::from([("Count".to_string(), 77.0), ("PatternFirst".to_string(), 0x4D as f64), ("PatternLast".to_string(), 0x80 as f64)]);
    let data = encode_message(m, &vals, None).unwrap();
    assert_eq!(data.len(), 64);
    assert_eq!(&data[8..12], &77u32.to_le_bytes());
    assert_eq!((data[12], data[63]), (0x4D, 0x80));
    let (_, back) = values(&d, &CanFrame::fd(0x322, false, true, &data).unwrap());
    assert_eq!(back["Count"], 77.0);
}
