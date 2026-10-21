//! Implicit IO (ForwardOpen + cyclic T→O) and GetAttributeSingle against the
//! device simulator over loopback. Proves the hardest new feature end-to-end
//! with no GUI and no hardware.

use mt_enip::*;
use std::time::Duration;

fn cfg(port: u16) -> EnipConfig {
    EnipConfig {
        id: "io".into(),
        host: "127.0.0.1".into(),
        port,
        poll_interval_ms: 1000,
    }
}

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
async fn io_pump_receives_cyclic_to_data() {
    let port: u16 = 44830;
    let points = vec![pt("a", "Program:Sim.A", "dint", 1.0), pt("b", "Program:Sim.B", "real", 2.0)];
    let dev = EnipSimDevice::start(port, "127.0.0.1".into(), points, vec![], vec![], |_| {}, |_, _| {}, |_| {})
        .await
        .expect("start sim");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let params = ForwardOpenParams {
        o_to_t_connection_id: 0x00000001,
        t_to_o_connection_id: 0x00000002,
        connection_serial: 1,
        vendor_id: 1,
        originator_serial: 0x12345678,
        timeout_ticks: 0x0A,
        timeout_multiplier: 1,
        o_to_t_rpi_us: 100_000,
        o_to_t_params: 0x0200,
        t_to_o_rpi_us: 100_000,
        t_to_o_params: 0x0200,
        transport_class_trigger: 0xA3,
        connection_path: vec![0x01, 0x00, 0x20, 0x04, 0x24, 100],
    };

    let mut conn = IoConnection::open(&cfg(port), params).await.expect("forward open");
    // A cyclic T→O frame should arrive within ~RPI + slack.
    let sample = tokio::time::timeout(Duration::from_millis(800), conn.next_sample())
        .await
        .expect("timed out waiting for cyclic T→O data")
        .expect("next_sample errored");

    assert_eq!(sample.connection_id, 0x00000002, "T→O connection id");
    // Assembly = encoded tag values (DINT 4B + REAL 4B).
    assert_eq!(sample.bytes.len(), 8, "assembly size: {:?}", sample.bytes);

    dev.stop();
}

#[tokio::test]
async fn get_attribute_single_reads_identity() {
    let port: u16 = 44831;
    let dev = EnipSimDevice::start(port, "127.0.0.1".into(), vec![pt("a", "Program:Sim.A", "dint", 1.0)], vec![], vec![], |_| {}, |_, _| {}, |_| {})
        .await
        .expect("start sim");
    tokio::time::sleep(Duration::from_millis(200)).await;

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

    client.disconnect().await.ok();
    dev.stop();
}
