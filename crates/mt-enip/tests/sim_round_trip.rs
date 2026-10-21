//! End-to-end smoke test: the device simulator (a real ENIP TCP server) spoken
//! to by the real EnipClient over loopback. Exercises server CIP handling,
//! the tick engine, and the client path with no GUI.

use mt_enip::*;
use std::time::Duration;

fn pt(key: &str, name: &str, dt: &str, base: f64, writable: bool, is_bool: bool) -> SimPointDef {
    SimPointDef {
        key: key.into(),
        name: name.into(),
        data_type: dt.into(),
        element_count: 1,
        base,
        variance: 0.0,
        noise: 0.0,
        period_ms: 5000,
        min: None,
        max: None,
        decimals: None,
        unit: None,
        writable,
        is_bool,
        source: "wave".into(),
    }
}

fn cfg(port: u16) -> EnipConfig {
    EnipConfig {
        id: "smoke".into(),
        host: "127.0.0.1".into(),
        port,
        poll_interval_ms: 1000,
    }
}

#[tokio::test]
async fn sim_server_client_round_trip() {
    let port: u16 = 44820;
    let points = vec![
        pt("speed", "Program:Sim.Speed", "real", 50.0, false, false),
        pt("cmd", "Program:Sim.Cmd", "real", 0.0, true, false),
        pt("run", "Program:Sim.Run", "bool", 1.0, false, true),
    ];

    let dev = EnipSimDevice::start(port, "127.0.0.1".into(), points, vec![], vec![], |_| {}, |_, _| {}, |_| {})
        .await
        .expect("start sim");
    // Let the listener + accept loop come up.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(dev.is_running());

    let cfg = cfg(port);

    // Browse: server should expose all sim tags via the Symbol class.
    let tags = browse_tags(&cfg).await.expect("browse");
    assert!(tags.len() >= 3, "expected >=3 tags, got {}", tags.len());

    // Read the auto-ticked sensor (base 50, no variance -> ~50).
    let speed = CipTag { name: "Program:Sim.Speed".into(), data_type: CipDataType::Real, element_count: 1, template_id: None };
    let vals = read_tags(&cfg, &[speed.clone()]).await.expect("read speed");
    match &vals[0] {
        CipValue::Real(v) => assert!((v - 50.0).abs() < 0.5, "speed not ~50: {v}"),
        other => panic!("expected Real, got {other:?}"),
    }

    // Write a setpoint, read it back.
    let cmd = CipTag { name: "Program:Sim.Cmd".into(), data_type: CipDataType::Real, element_count: 1, template_id: None };
    write_tag(&cfg, &cmd, CipValue::Real(77.0)).await.expect("write cmd");
    let back = read_tags(&cfg, &[cmd]).await.expect("read cmd");
    match &back[0] {
        CipValue::Real(v) => assert!((v - 77.0).abs() < 0.01, "cmd not persisted: {v}"),
        other => panic!("expected Real, got {other:?}"),
    }

    // Read a bool sensor.
    let run = CipTag { name: "Program:Sim.Run".into(), data_type: CipDataType::Bool, element_count: 1, template_id: None };
    let rb = read_tags(&cfg, &[run]).await.expect("read run");
    assert!(matches!(rb[0], CipValue::Bool(_)), "expected Bool");

    dev.stop();
}

#[tokio::test]
async fn sim_set_by_name_overrides_writable() {
    let port: u16 = 44821;
    let points = vec![pt("cmd", "Program:Sim.Cmd", "real", 0.0, true, false)];
    let dev = EnipSimDevice::start(port, "127.0.0.1".into(), points, vec![], vec![], |_| {}, |_, _| {}, |_| {})
        .await
        .expect("start sim");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Manual override via the sim handle (control-panel path).
    dev.set_by_name("Program:Sim.Cmd", CipValue::Real(123.0)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let cfg = cfg(port);
    let cmd = CipTag { name: "Program:Sim.Cmd".into(), data_type: CipDataType::Real, element_count: 1, template_id: None };
    let back = read_tags(&cfg, &[cmd]).await.expect("read");
    match &back[0] {
        CipValue::Real(v) => assert!((v - 123.0).abs() < 0.01, "override not visible: {v}"),
        other => panic!("expected Real, got {other:?}"),
    }

    dev.stop();
}
