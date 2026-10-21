//! Connection Manager / I/O behaviour against the simulated target over
//! loopback: Forward_Open and Large_Forward_Open, class 3 connected explicit
//! messaging over SendUnitData, class 1 implicit I/O over UDP with the O→T
//! watchdog, and the error replies a real target gives for bad requests.

use mt_enip::*;
use std::time::Duration;

fn cfg(port: u16) -> EnipConfig {
    EnipConfig {
        id: "io".into(),
        host: "127.0.0.1".into(),
        port,
        poll_interval_ms: 1000,
        route_path: None,
    }
}

fn dint(name: &str, v: i32) -> CipTagDef {
    CipTagDef {
        name: name.into(),
        data_type: CipDataType::Dint,
        element_count: 1,
        initial_value: CipValue::Dint(v),
        template_id: None,
    }
}

async fn start(tags: Vec<CipTagDef>) -> (std::sync::Arc<EnipServerState>, u16, tokio::sync::watch::Sender<bool>) {
    let (state, rx, shutdown) = start_enip_server(
        EnipServerConfig { port: 0, bind_addr: "127.0.0.1".into() },
        tags,
        vec![],
    )
    .await
    .expect("start server");
    // Keep draining the log so the unbounded channel does not grow.
    tokio::spawn(async move {
        let mut rx = rx;
        while rx.recv().await.is_some() {}
    });
    let port = state.tcp_port();
    (state, port, shutdown)
}

fn base_params() -> ForwardOpenParams {
    ForwardOpenParams {
        o_to_t_connection_id: 0,
        t_to_o_connection_id: 0x0000_0042,
        connection_serial: 1,
        vendor_id: 1,
        originator_serial: 0x1234_5678,
        priority_time_tick: 0x0A,
        timeout_ticks: 0x05,
        timeout_multiplier: 1,
        o_to_t_rpi_us: 20_000,
        o_to_t_params: 0,
        t_to_o_rpi_us: 20_000,
        t_to_o_params: 0,
        transport_class_trigger: 0xA3,
        large: false,
        o_to_t_run_idle: true,
        connection_path: vec![0x20, 0x02, 0x24, 0x01],
    }
}

/// Class 3 explicit messaging to the Message Router (path `20 02 24 01`).
fn explicit_params(large: bool) -> ForwardOpenParams {
    let size = if large { 4000 } else { 500 };
    let p = network_connection_params(size, ConnectionType::PointToPoint, true, 0, large);
    ForwardOpenParams { large, o_to_t_params: p, t_to_o_params: p, ..base_params() }
}

/// Class 1 cyclic I/O to an assembly: config 0x66, output 0x96, input 0x64.
fn implicit_params(o_size: u16, t_size: u16) -> ForwardOpenParams {
    ForwardOpenParams {
        o_to_t_params: network_connection_params(o_size, ConnectionType::PointToPoint, false, 0, false),
        t_to_o_params: network_connection_params(t_size, ConnectionType::PointToPoint, false, 0, false),
        transport_class_trigger: 0x01,
        connection_path: vec![0x20, 0x04, 0x24, 0x66, 0x2C, 0x96, 0x2C, 0x64],
        ..base_params()
    }
}

#[tokio::test]
async fn class3_connected_messaging_over_send_unit_data() {
    let (state, port, _sd) = start(vec![dint("Counter", 7)]).await;
    for large in [false, true] {
        let params = explicit_params(large);
        let mut conn = IoConnection::open(&cfg(port), params.clone()).await.expect("forward open");
        // Target-assigned O→T id (not our proposal), originator's T→O id echoed.
        assert_ne!(conn.result.o_to_t_connection_id, 0);
        assert_eq!(conn.result.t_to_o_connection_id, 0x42);
        assert_eq!(conn.result.o_to_t_api_us, 20_000);
        // Read Tag "Counter" over the connected transport.
        let reply = conn
            .produce(0, &[0x4C, 0x05, 0x91, 0x07, b'C', b'o', b'u', b'n', b't', b'e', b'r', 0x00, 0x01, 0x00])
            .await
            .expect("connected request");
        assert_eq!(reply, vec![0xCC, 0x00, 0x00, 0x00, 0xC4, 0x00, 0x07, 0x00, 0x00, 0x00], "large={large}");
        conn.close(&params).await.expect("close");
    }
    assert_eq!(state.open_io_connections().await, 0);
}

#[tokio::test]
async fn explicit_connection_keeps_itself_alive_by_polling_identity() {
    let (_state, port, _sd) = start(vec![dint("A", 1)]).await;
    let mut params = explicit_params(false);
    params.o_to_t_rpi_us = 100_000;
    let mut conn = IoConnection::open(&cfg(port), params).await.expect("forward open");
    let a = tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.unwrap().unwrap();
    let b = tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.unwrap().unwrap();
    // Connected reply to Get_Attribute_Single(Identity vendor id) = Rockwell (1).
    assert_eq!(a.bytes, vec![0x8E, 0x00, 0x00, 0x00, 0x01, 0x00]);
    assert_eq!(b.sequence, a.sequence.wrapping_add(1));
    assert_eq!(a.connection_id, 0x42);
}

#[tokio::test]
async fn implicit_io_class1_over_udp() {
    let (state, port, _sd) = start(vec![dint("A", 1), dint("B", 2)]).await;
    // O→T: seq(2) + run/idle header(4) + 4 data bytes; T→O: seq(2) + 8 data bytes.
    let params = implicit_params(10, 10);
    let mut conn = IoConnection::open(&cfg(port), params.clone()).await.expect("forward open");
    assert_eq!(state.open_io_connections().await, 1);

    let s1 = tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.expect("sample").unwrap();
    let s2 = tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.expect("sample").unwrap();
    assert_eq!(s1.connection_id, 0x42, "T→O connection id");
    assert_eq!(s1.bytes, vec![1, 0, 0, 0, 2, 0, 0, 0], "input assembly = tag image");
    assert_eq!(s2.sequence, s1.sequence.wrapping_add(1), "16-bit sequence count increments");

    // O→T heartbeats carry the run/idle header + output data.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.last_output.lock().await.clone(), Some(vec![1, 0, 0, 0, 0, 0, 0, 0]));
    conn.produce(0, &[9, 8, 7, 6]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.last_output.lock().await.clone(), Some(vec![1, 0, 0, 0, 9, 8, 7, 6]));

    // Tag changes show up in later cyclic frames (older frames may still be queued).
    state.tags.lock().await.get_mut("A").unwrap().value = CipValue::Dint(0x0102_0304);
    let mut seen = false;
    for _ in 0..200 {
        let s = tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.expect("sample").unwrap();
        if s.bytes.starts_with(&[4, 3, 2, 1]) {
            seen = true;
            break;
        }
    }
    assert!(seen, "updated tag value never appeared in the T→O data");
    conn.close(&params).await.expect("forward close");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.open_io_connections().await, 0, "Forward_Close releases the connection");
}

#[tokio::test]
async fn t_to_o_is_resized_to_the_negotiated_connection_size() {
    let (_state, port, _sd) = start(vec![dint("A", 1), dint("B", 2)]).await;
    // Variable-size connection asking for 2 + 12 bytes: image padded with zeros.
    let mut conn = IoConnection::open(&cfg(port), implicit_params(6, 14)).await.expect("forward open");
    let s = tokio::time::timeout(Duration::from_secs(2), conn.next_sample()).await.unwrap().unwrap();
    assert_eq!(s.bytes.len(), 12);
    assert_eq!(&s.bytes[8..], &[0, 0, 0, 0]);
}

#[tokio::test]
async fn forward_open_rejections_carry_the_real_extended_status() {
    let (_state, port, _sd) = start(vec![dint("A", 1)]).await;
    let open = |p: ForwardOpenParams| {
        let c = cfg(port);
        async move { IoConnection::open(&c, p).await.err().map(|e| format!("{e:#}")).unwrap_or_default() }
    };

    // Class 3 to an assembly: transport class/trigger not supported (0x0103)
    let mut p = implicit_params(10, 10);
    p.transport_class_trigger = 0xA3;
    let e = open(p).await;
    assert!(e.contains("0x0103") && e.contains("Transport class"), "{e}");

    // Class 1 to the Message Router: same
    let mut p = explicit_params(false);
    p.transport_class_trigger = 0x01;
    let e = open(p).await;
    assert!(e.contains("0x0103"), "{e}");

    // Routing through a port that does not exist: port not available (0x0311)
    let mut p = explicit_params(false);
    p.connection_path = vec![0x02, 0x00, 0x20, 0x02, 0x24, 0x01];
    let e = open(p).await;
    assert!(e.contains("0x0311") && e.contains("port not available"), "{e}");

    // Slot with nothing in it: link address not valid (0x0312)
    let mut p = explicit_params(false);
    p.connection_path = vec![0x01, 0x07, 0x20, 0x02, 0x24, 0x01];
    let e = open(p).await;
    assert!(e.contains("0x0312"), "{e}");

    // No class segment at all: invalid segment type (0x0315)
    let mut p = explicit_params(false);
    p.connection_path = vec![];
    let e = open(p).await;
    assert!(e.contains("0x0315"), "{e}");

    // Explicit connections must be point-to-point on both directions (0x0108)
    let mut p = explicit_params(false);
    p.o_to_t_params = network_connection_params(500, ConnectionType::Multicast, true, 0, false);
    let e = open(p).await;
    assert!(e.contains("0x0108"), "{e}");

    // 32-bit network parameters need Large_Forward_Open
    let mut p = explicit_params(false);
    p.o_to_t_params = 0x4200_01F4;
    let e = open(p).await;
    assert!(e.contains("Large_Forward_Open"), "{e}");
}

#[tokio::test]
async fn get_attribute_single_reads_identity() {
    let (_state, port, _sd) = start(vec![dint("A", 1)]).await;
    let client = EnipClient::connect(cfg(port)).await.expect("connect");

    // Identity vendor id (class 1, instance 1, attribute 1) = Rockwell 0x0001.
    let vendor = client.generic_cip(0x0E, 0x01, 1, Some(1), Vec::new()).await.expect("get vendor");
    assert_eq!(vendor, vec![0x01, 0x00]);

    // Serial number attribute 6 = 0x12345678 (little-endian).
    let serial = client.generic_cip(0x0E, 0x01, 1, Some(6), Vec::new()).await.expect("get serial");
    assert_eq!(serial, vec![0x78, 0x56, 0x34, 0x12]);

    // Product name attribute 7 = SHORT_STRING "ENIP-Sim": len(1) + bytes.
    let name = client.generic_cip(0x0E, 0x01, 1, Some(7), Vec::new()).await.expect("get name");
    assert_eq!(name[0], 8);
    assert_eq!(&name[1..], b"ENIP-Sim");

    // Identity via one Get_Attributes_All round trip.
    let id = client.get_identity().await.expect("identity");
    assert_eq!(id.vendor_id, 1);
    assert_eq!(id.product_name, "ENIP-Sim");
    assert_eq!((id.revision_major, id.revision_minor), (20, 11));

    // Typed CIP errors: unknown attribute → 0x14, unknown class → 0x05.
    let e = client.generic_cip(0x0E, 0x01, 1, Some(99), Vec::new()).await.unwrap_err().to_string();
    assert!(e.contains("0x14"), "{e}");
    let e = client.generic_cip(0x0E, 0x99, 1, Some(1), Vec::new()).await.unwrap_err().to_string();
    assert!(e.contains("0x05"), "{e}");

    client.disconnect().await.ok();
}
