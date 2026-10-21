//! Device discovery vs the simulator: the adapter must answer a ListIdentity
//! UDP request so the explorer's discovery (broadcast to :44818) finds it.

use mt_enip::packets::{build_list_identity_request, parse_list_identity_response};
use mt_enip::*;
use std::time::Duration;

fn pt(key: &str, name: &str, dt: &str, base: f64) -> SimPointDef {
    SimPointDef {
        key: key.into(),
        name: name.into(),
        data_type: dt.into(),
        element_count: 1,
        base,
        variance: 0.0,
        noise: 0.0,
        period_ms: 60_000,
        min: None,
        max: None,
        decimals: None,
        unit: None,
        writable: false,
        is_bool: false,
        source: "wave".into(),
    }
}

#[tokio::test]
async fn sim_responds_to_list_identity() {
    let port: u16 = 44850;
    let dev = EnipSimDevice::start(
        port,
        "127.0.0.1".into(),
        vec![pt("a", "Program:Sim.A", "dint", 1.0)],
        vec![],
        vec![],
        |_| {},
        |_, _| {},
        |_| {},
    )
    .await
    .expect("start sim");

    // Give the UDP identity responder a moment to bind.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client udp");
    sock.send_to(&build_list_identity_request(), format!("127.0.0.1:{port}"))
        .await
        .expect("send list identity");

    let mut buf = vec![0u8; 1024];
    let (n, _peer) = tokio::time::timeout(Duration::from_millis(1000), sock.recv_from(&mut buf))
        .await
        .expect("timed out waiting for identity reply")
        .expect("recv");

    let parsed = parse_list_identity_response(&buf[..n]).expect("parse identity reply");
    assert_eq!(parsed.vendor_id, 1);
    assert_eq!(parsed.serial_number, 0x12345678);
    assert_eq!(parsed.product_name, "ENIP-Sim");
    // Reply source port == the sim's bind port (44818 in the app) == TCP port.
    assert_eq!(_peer.port(), port);
    // The identity item advertises the TCP port to connect to.
    assert_eq!(parsed.port, port);

    dev.stop();
}
