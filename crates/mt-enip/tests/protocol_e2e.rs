//! End-to-end client ↔ simulated-target tests for the Logix tag services:
//! route paths, Multiple_Service_Packet batching, partial/fragmented
//! transfers, types (STRING structure, arrays, coercion, bit access), symbol
//! browsing, session handling and UDT templates.

use mt_enip::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};

fn def(name: &str, dt: CipDataType, count: u16, v: CipValue) -> CipTagDef {
    CipTagDef { name: name.into(), data_type: dt, element_count: count, initial_value: v, template_id: None }
}

fn tag(name: &str, dt: CipDataType, count: u16) -> CipTag {
    CipTag { name: name.into(), data_type: dt, element_count: count, template_id: None }
}

struct Target {
    state: Arc<EnipServerState>,
    port: u16,
    log: mpsc::UnboundedReceiver<EnipServerLogEntry>,
    _shutdown: watch::Sender<bool>,
}

impl Target {
    fn cfg(&self, route: Option<&str>) -> EnipConfig {
        EnipConfig {
            id: "e2e".into(),
            host: "127.0.0.1".into(),
            port: self.port,
            poll_interval_ms: 1000,
            route_path: route.map(str::to_string),
        }
    }

    /// Drain buffered server log entries.
    fn drain(&mut self) -> Vec<EnipServerLogEntry> {
        let mut v = Vec::new();
        while let Ok(e) = self.log.try_recv() {
            v.push(e);
        }
        v
    }
}

fn drive_template() -> CipTemplateDef {
    CipTemplateDef {
        id: 0x123,
        name: "DriveInfo".into(),
        members: vec![
            CipTemplateMember { name: "RatedSpeed".into(), data_type: CipDataType::Real, array_elements: 1 },
            CipTemplateMember { name: "Poles".into(), data_type: CipDataType::Dint, array_elements: 1 },
            CipTemplateMember { name: "Curve".into(), data_type: CipDataType::Int, array_elements: 3 },
        ],
    }
}

async fn target() -> Target {
    let mut tags = vec![
        def("Counter", CipDataType::Dint, 1, CipValue::Dint(7)),
        def("Speed", CipDataType::Real, 1, CipValue::Real(1.5)),
        def("Flag", CipDataType::Bool, 1, CipValue::Bool(true)),
        def("Level", CipDataType::Lint, 1, CipValue::Lint(1 << 40)),
        def("Status", CipDataType::Dint, 1, CipValue::Dint(0b1000)),
        def("Name", CipDataType::String, 1, CipValue::String("Hi".into())),
        def("Arr", CipDataType::Dint, 300, CipValue::Array((0..300).map(CipValue::Dint).collect())),
        def("Program:Main.Motor", CipDataType::Real, 1, CipValue::Real(2.5)),
    ];
    for i in 0..30 {
        tags.push(def(&format!("Mx{i:02}"), CipDataType::Dint, 1, CipValue::Dint(100 + i)));
    }
    let t = drive_template();
    let packed = t.pack_instance(&[1450.0, 4.0, 0.0]);
    tags.push(CipTagDef {
        name: "Drive".into(),
        data_type: CipDataType::Dint,
        element_count: 1,
        initial_value: CipValue::Bytes(packed),
        template_id: Some(t.id),
    });
    let (state, log, shutdown) = start_enip_server(
        EnipServerConfig { port: 0, bind_addr: "127.0.0.1".into() },
        tags,
        vec![t],
    )
    .await
    .expect("start server");
    Target { port: state.tcp_port(), state, log, _shutdown: shutdown }
}

#[tokio::test]
async fn route_path_wraps_requests_in_unconnected_send() {
    let t = target().await;
    let c = EnipClient::connect(t.cfg(Some("1,0"))).await.unwrap();
    assert_eq!(c.read(&[tag("Counter", CipDataType::Dint, 1)]).await.unwrap(), vec![CipValue::Dint(7)]);
    // Generic services are routed too.
    assert_eq!(c.generic_cip(0x0E, 1, 1, Some(1), vec![]).await.unwrap(), vec![1, 0]);

    // Empty slot: the Connection Manager answers 0x01 / 0x0312, surfaced verbatim.
    let c = EnipClient::connect(t.cfg(Some("1,5"))).await.unwrap();
    let e = c.read(&[tag("Counter", CipDataType::Dint, 1)]).await.unwrap_err().to_string();
    assert!(e.contains("0x0312") && e.contains("link address"), "{e}");
    let c = EnipClient::connect(t.cfg(Some("2,0"))).await.unwrap();
    let e = c.read(&[tag("Counter", CipDataType::Dint, 1)]).await.unwrap_err().to_string();
    assert!(e.contains("0x0311"), "{e}");

    // Garbage route text is rejected at connect time, not on the wire.
    assert!(EnipClient::connect(t.cfg(Some("1"))).await.is_err());
}

#[tokio::test]
async fn reads_batch_into_multiple_service_packets() {
    let mut t = target().await;
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    let mut tags: Vec<CipTag> = (0..30).map(|i| tag(&format!("Mx{i:02}"), CipDataType::Dint, 1)).collect();
    tags.insert(5, tag("Speed", CipDataType::Real, 1));
    tags.insert(9, tag("Flag", CipDataType::Bool, 1));
    t.drain();
    let values = c.read(&tags).await.unwrap();
    assert_eq!(values.len(), 32);
    assert_eq!(values[0], CipValue::Dint(100));
    assert_eq!(values[5], CipValue::Real(1.5));
    assert_eq!(values[9], CipValue::Bool(true));
    assert_eq!(values[31], CipValue::Dint(129));
    let log = t.drain();
    let msp = log.iter().filter(|e| e.service == "multi_service").count();
    assert!(msp >= 1, "expected Multiple_Service_Packet, log: {:?}", log.iter().map(|e| &e.service).collect::<Vec<_>>());
    assert!(msp < 32, "must batch, not one packet per tag");

    // A bad tag inside a batch reports that tag's error, not a transport failure.
    let mut bad = tags.clone();
    bad.insert(3, tag("Missing", CipDataType::Dint, 1));
    let e = c.read(&bad).await.unwrap_err().to_string();
    assert!(e.contains("Missing") && e.contains("0x04"), "{e}");
}

#[tokio::test]
async fn routed_batches_work_too() {
    let t = target().await;
    let c = EnipClient::connect(t.cfg(Some("1,0"))).await.unwrap();
    let tags: Vec<CipTag> = (0..10).map(|i| tag(&format!("Mx{i:02}"), CipDataType::Dint, 1)).collect();
    let v = c.read(&tags).await.unwrap();
    assert_eq!(v[9], CipValue::Dint(109));
}

#[tokio::test]
async fn large_arrays_transfer_with_fragmented_services() {
    let mut t = target().await;
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    t.drain();
    let v = c.read(&[tag("Arr", CipDataType::Dint, 300)]).await.unwrap();
    let CipValue::Array(items) = &v[0] else { panic!("expected array, got {:?}", v[0]) };
    assert_eq!(items.len(), 300);
    assert_eq!(items[0], CipValue::Dint(0));
    assert_eq!(items[299], CipValue::Dint(299));
    let partials = t.drain().iter().filter(|e| e.message.contains("(partial)")).count();
    assert!(partials >= 2, "1200 bytes need several fragments, saw {partials}");

    // Write all 300 elements back (Write Tag Fragmented), verify element access.
    let new = CipValue::Array((0..300).map(|i| CipValue::Dint(i * 2)).collect());
    c.write(&tag("Arr", CipDataType::Dint, 300), new).await.unwrap();
    assert_eq!(c.read(&[tag("Arr[299]", CipDataType::Dint, 1)]).await.unwrap(), vec![CipValue::Dint(598)]);
    let v = c.read(&[tag("Arr[10]", CipDataType::Dint, 3)]).await.unwrap();
    assert_eq!(v[0], CipValue::Array(vec![CipValue::Dint(20), CipValue::Dint(22), CipValue::Dint(24)]));
    // Write a slice via an element path
    c.write(&tag("Arr[0]", CipDataType::Dint, 2), CipValue::Array(vec![CipValue::Dint(-1), CipValue::Dint(-2)]))
        .await
        .unwrap();
    let v = c.read(&[tag("Arr", CipDataType::Dint, 3)]).await.unwrap();
    assert_eq!(v[0], CipValue::Array(vec![CipValue::Dint(-1), CipValue::Dint(-2), CipValue::Dint(4)]));
}

#[tokio::test]
async fn strings_are_logix_structures() {
    let t = target().await;
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    let name = tag("Name", CipDataType::String, 1);
    assert_eq!(c.read(&[name.clone()]).await.unwrap(), vec![CipValue::String("Hi".into())]);
    c.write(&name, CipValue::String("Hello, world".into())).await.unwrap();
    assert_eq!(c.read(&[name.clone()]).await.unwrap(), vec![CipValue::String("Hello, world".into())]);
    let e = c.write(&name, CipValue::String("x".repeat(90))).await.unwrap_err().to_string();
    assert!(e.contains("82"), "{e}");
}

#[tokio::test]
async fn writes_use_the_tags_real_type() {
    let t = target().await;
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    // The caller declares DINT for a REAL tag (the explorer's editor does this
    // for types it cannot represent); the old encoder sent type 0xC4 and the
    // controller answered 0xFF/0x2107. The value must be coerced to REAL.
    c.write(&tag("Speed", CipDataType::Dint, 1), CipValue::Dint(3)).await.unwrap();
    assert_eq!(c.read(&[tag("Speed", CipDataType::Real, 1)]).await.unwrap(), vec![CipValue::Real(3.0)]);
    // Same for LINT (declared DINT) and BOOL (declared SINT)
    c.write(&tag("Level", CipDataType::Dint, 1), CipValue::Dint(5)).await.unwrap();
    assert_eq!(c.read(&[tag("Level", CipDataType::Lint, 1)]).await.unwrap(), vec![CipValue::Lint(5)]);
    c.write(&tag("Flag", CipDataType::Sint, 1), CipValue::Sint(0)).await.unwrap();
    assert_eq!(c.read(&[tag("Flag", CipDataType::Bool, 1)]).await.unwrap(), vec![CipValue::Bool(false)]);
    // Out-of-range values fail locally with a clear message instead of wrapping.
    let e = c.write(&tag("Counter", CipDataType::Dint, 1), CipValue::Lint(1 << 40)).await.unwrap_err().to_string();
    assert!(e.contains("out of range"), "{e}");
}

#[tokio::test]
async fn bit_selectors_read_and_write_the_parent_word() {
    let t = target().await;
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    assert_eq!(c.read(&[tag("Status.3", CipDataType::Bool, 1)]).await.unwrap(), vec![CipValue::Bool(true)]);
    assert_eq!(c.read(&[tag("Status.2", CipDataType::Bool, 1)]).await.unwrap(), vec![CipValue::Bool(false)]);
    c.write(&tag("Status.0", CipDataType::Bool, 1), CipValue::Bool(true)).await.unwrap();
    c.write(&tag("Status.3", CipDataType::Bool, 1), CipValue::Bool(false)).await.unwrap();
    assert_eq!(c.read(&[tag("Status", CipDataType::Dint, 1)]).await.unwrap(), vec![CipValue::Dint(1)]);
}

#[tokio::test]
async fn browse_uses_get_instance_attribute_list_with_pagination() {
    let t = target().await;
    let tags = browse_tags(&t.cfg(None)).await.unwrap();
    let find = |n: &str| tags.iter().find(|x| x.name == n).unwrap_or_else(|| panic!("{n} not browsed: {:?}", tags.iter().map(|t| &t.name).collect::<Vec<_>>()));
    assert_eq!(find("Counter").data_type, CipDataType::Dint);
    assert_eq!(find("Counter").element_count, 1);
    assert_eq!(find("Speed").data_type, CipDataType::Real);
    assert_eq!(find("Flag").data_type, CipDataType::Bool);
    assert_eq!(find("Name").data_type, CipDataType::String);
    assert_eq!(find("Name").template_id, None);
    let arr = find("Arr");
    assert_eq!((arr.data_type, arr.element_count), (CipDataType::Dint, 300));
    assert_eq!(find("Drive").template_id, Some(0x123));
    // program scope tags are fully qualified; the Program object itself is not a tag
    assert_eq!(find("Program:Main.Motor").data_type, CipDataType::Real);
    assert!(!tags.iter().any(|x| x.name == "Program:Main"));
    // 30 Mx tags + the others overflow one reply → browsing continued across pages
    assert!(tags.len() >= 38, "{}", tags.len());

    // Browsed descriptors are directly readable (arrays come back whole)
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    let v = c.read(&[arr.clone(), find("Program:Main.Motor").clone()]).await.unwrap();
    assert!(matches!(&v[0], CipValue::Array(a) if a.len() == 300));
    assert_eq!(v[1], CipValue::Real(2.5));
}

#[tokio::test]
async fn udt_templates_are_read_with_offset_and_size_requests() {
    let t = target().await;
    let defs = read_templates(&t.cfg(None), &[0x123]).await.unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "DriveInfo");
    let names: Vec<&str> = defs[0].members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["RatedSpeed", "Poles", "Curve"]);
    assert_eq!(defs[0].members[2].data_type, CipDataType::Int);
    assert_eq!(defs[0].members[2].array_elements, 3, "array length travels in the member info word");

    // Whole-structure read returns the raw image; members read as atomics.
    let c = EnipClient::connect(t.cfg(None)).await.unwrap();
    let drive = CipTag { name: "Drive".into(), data_type: CipDataType::Dint, element_count: 1, template_id: Some(0x123) };
    let CipValue::Bytes(img) = c.read(&[drive.clone()]).await.unwrap().remove(0) else { panic!() };
    assert_eq!(img.len(), 14, "REAL@0, DINT@4, INT[3]@8");
    assert_eq!(c.read(&[tag("Drive.Poles", CipDataType::Dint, 1)]).await.unwrap(), vec![CipValue::Dint(4)]);
    // The structure can be written back using the handle the controller reported.
    c.write(&drive, CipValue::Bytes(img.clone())).await.unwrap();
}

// ── Raw session-level behaviour ──

fn frame(cmd: u16, session: u32, ctx: u64, body: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&cmd.to_le_bytes());
    f.extend_from_slice(&(body.len() as u16).to_le_bytes());
    f.extend_from_slice(&session.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&ctx.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(body);
    f
}

/// (command, session, status, sender context, body)
async fn read_frame(s: &mut TcpStream) -> (u16, u32, u32, u64, Vec<u8>) {
    let mut h = [0u8; 24];
    tokio::time::timeout(std::time::Duration::from_secs(2), s.read_exact(&mut h)).await.expect("reply").unwrap();
    let len = u16::from_le_bytes([h[2], h[3]]) as usize;
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).await.unwrap();
    (
        u16::from_le_bytes([h[0], h[1]]),
        u32::from_le_bytes(h[4..8].try_into().unwrap()),
        u32::from_le_bytes(h[8..12].try_into().unwrap()),
        u64::from_le_bytes(h[12..20].try_into().unwrap()),
        body,
    )
}

fn rr(cip: &[u8]) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0, 10, 0, 2, 0, 0, 0, 0, 0, 0xB2, 0];
    b.extend_from_slice(&(cip.len() as u16).to_le_bytes());
    b.extend_from_slice(cip);
    b
}

#[tokio::test]
async fn session_handling_follows_the_encapsulation_spec() {
    let t = target().await;
    let mut s = TcpStream::connect(("127.0.0.1", t.port)).await.unwrap();
    let get_identity_vendor = [0x0E, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01];

    // Requests before RegisterSession: invalid session handle (0x64)
    s.write_all(&frame(0x6F, 0, 0xAA, &rr(&get_identity_vendor))).await.unwrap();
    let (cmd, _, status, ctx, _) = read_frame(&mut s).await;
    assert_eq!((cmd, status, ctx), (0x6F, 0x64, 0xAA));

    // Unsupported protocol version: 0x69
    s.write_all(&frame(0x65, 0, 1, &[2, 0, 0, 0])).await.unwrap();
    let (_, session, status, _, _) = read_frame(&mut s).await;
    assert_eq!((status, session), (0x69, 0));

    // Malformed RegisterSession length: 0x65
    s.write_all(&frame(0x65, 0, 1, &[1, 0])).await.unwrap();
    assert_eq!(read_frame(&mut s).await.2, 0x65);

    // Proper registration: non-zero handle, version echoed, context echoed
    s.write_all(&frame(0x65, 0, 0x1122_3344_5566_7788, &[1, 0, 0, 0])).await.unwrap();
    let (cmd, session, status, ctx, body) = read_frame(&mut s).await;
    assert_eq!((cmd, status, ctx), (0x65, 0, 0x1122_3344_5566_7788));
    assert_ne!(session, 0);
    assert_eq!(body, vec![1, 0, 0, 0]);

    // A second RegisterSession on the same TCP connection is refused
    s.write_all(&frame(0x65, 0, 2, &[1, 0, 0, 0])).await.unwrap();
    assert_eq!(read_frame(&mut s).await.2, 0x01);

    // Wrong handle: 0x64; right handle: CIP reply inside the CPF with echoed context
    s.write_all(&frame(0x6F, session ^ 0xFF, 3, &rr(&get_identity_vendor))).await.unwrap();
    assert_eq!(read_frame(&mut s).await.2, 0x64);
    s.write_all(&frame(0x6F, session, 4, &rr(&get_identity_vendor))).await.unwrap();
    let (cmd, _, status, ctx, body) = read_frame(&mut s).await;
    assert_eq!((cmd, status, ctx), (0x6F, 0, 4));
    // interface(4) timeout(2) count(2) null item(4) B2 item header(4) + reply
    assert_eq!(&body[..8], &[0, 0, 0, 0, 0, 0, 2, 0]);
    assert_eq!(&body[8..12], &[0, 0, 0, 0]);
    assert_eq!(&body[12..14], &[0xB2, 0x00]);
    assert_eq!(&body[16..], &[0x8E, 0x00, 0x00, 0x00, 0x01, 0x00]);

    // Unknown encapsulation command: 0x01
    s.write_all(&frame(0x00AB, session, 5, &[])).await.unwrap();
    assert_eq!(read_frame(&mut s).await.2, 0x01);

    // ListServices advertises CIP over TCP + UDP class 0/1
    s.write_all(&frame(0x64, 0, 6, &[])).await.unwrap();
    let (_, _, status, _, body) = read_frame(&mut s).await;
    assert_eq!(status, 0);
    assert_eq!(&body[..2], &[1, 0], "one item");
    assert_eq!(&body[2..4], &[0x00, 0x01], "type 0x0100 Communications");
    assert_eq!(&body[6..8], &[1, 0], "version");
    assert_eq!(&body[8..10], &[0x20, 0x01], "capability flags 0x0120");
    assert!(body[10..].starts_with(b"Communications"));

    // ListIdentity over TCP: same Identity item as UDP, context echoed
    s.write_all(&frame(0x63, 0, 7, &[])).await.unwrap();
    let (cmd, _, status, ctx, body) = read_frame(&mut s).await;
    assert_eq!((cmd, status, ctx), (0x63, 0, 7));
    let mut full = frame(0x63, 0, 7, &body);
    full[2..4].copy_from_slice(&(body.len() as u16).to_le_bytes());
    let dev = mt_enip::packets::parse_list_identity_response(&full).expect("identity");
    assert_eq!(dev.port, t.port);

    // UnregisterSession closes the connection
    s.write_all(&frame(0x66, session, 8, &[])).await.unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(std::time::Duration::from_secs(2), s.read(&mut buf)).await.unwrap().unwrap();
    assert_eq!(n, 0);
    let _ = t.state;
}

/// A minimal target: registers sessions and answers every SendRRData with a
/// DINT read reply carrying a per-request counter; the first answer is delayed.
async fn mock_target() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut n = 0i32;
        loop {
            let mut h = [0u8; 24];
            if s.read_exact(&mut h).await.is_err() {
                return;
            }
            let len = u16::from_le_bytes([h[2], h[3]]) as usize;
            let mut body = vec![0u8; len];
            if s.read_exact(&mut body).await.is_err() {
                return;
            }
            let ctx = u64::from_le_bytes(h[12..20].try_into().unwrap());
            let cmd = u16::from_le_bytes([h[0], h[1]]);
            let reply = if cmd == 0x65 {
                frame(0x65, 0x5151, ctx, &[1, 0, 0, 0])
            } else {
                n += 1;
                if n == 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                let mut cip = vec![0xCC, 0, 0, 0, 0xC4, 0];
                cip.extend_from_slice(&(n * 111).to_le_bytes());
                frame(0x6F, 0x5151, ctx, &rr(&cip)[..])
            };
            if s.write_all(&reply).await.is_err() {
                return;
            }
        }
    });
    port
}

#[tokio::test]
async fn late_replies_after_a_cancelled_request_are_discarded() {
    let port = mock_target().await;
    let cfg = EnipConfig { id: "m".into(), host: "127.0.0.1".into(), port, poll_interval_ms: 1000, route_path: None };
    let c = EnipClient::connect(cfg).await.unwrap();
    let t = tag("X", CipDataType::Dint, 1);
    // First request: the caller gives up after 100 ms (future dropped mid-read).
    assert!(tokio::time::timeout(std::time::Duration::from_millis(100), c.read(&[t.clone()])).await.is_err());
    // The stale answer (111) reaches the socket before the answer to this request (222);
    // it must be skipped, not returned for the wrong request.
    assert_eq!(c.read(&[t]).await.unwrap(), vec![CipValue::Dint(222)]);
}

#[tokio::test]
async fn concurrent_users_of_one_client_are_serialised() {
    let t = target().await;
    let c = Arc::new(EnipClient::connect(t.cfg(None)).await.unwrap());
    let mut handles = Vec::new();
    for i in 0..8 {
        let c = c.clone();
        handles.push(tokio::spawn(async move {
            let name = format!("Mx{i:02}");
            c.read(&[tag(&name, CipDataType::Dint, 1)]).await.unwrap()
        }));
    }
    for (i, h) in handles.into_iter().enumerate() {
        assert_eq!(h.await.unwrap(), vec![CipValue::Dint(100 + i as i32)]);
    }
}
