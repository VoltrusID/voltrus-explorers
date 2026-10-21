use std::sync::Arc;

use mt_dnp3::{
    ControlObject, ControlRequest, Crob, Dnp3Client, Dnp3Config, Dnp3Point, Dnp3Value,
    OutstationConfig, OutstationState, crob_code, start_outstation_server,
};
use mt_protocol::ProtocolClient;

async fn setup(port: u16) -> Arc<std::sync::Mutex<OutstationState>> {
    let (state, mut _log_rx, _shutdown) = start_outstation_server(OutstationConfig {
        port,
        bind_addr: "127.0.0.1".into(),
        master_address: 1,
        outstation_address: 10,
        analog_count: 4,
        binary_count: 4,
        counter_count: 4,
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    state
}

fn config(port: u16) -> Dnp3Config {
    Dnp3Config {
        id: "test-master".into(),
        host: "127.0.0.1".into(),
        port,
        master_address: 1,
        outstation_address: 10,
        poll_interval_ms: 100,
    }
}

fn points() -> Vec<Dnp3Point> {
    vec![
        Dnp3Point { name: "a0".into(), group: 30, variation: 4, index: 0, scale: 1.0 },
        Dnp3Point { name: "b0".into(), group: 1, variation: 2, index: 0, scale: 1.0 },
        Dnp3Point { name: "c0".into(), group: 20, variation: 1, index: 0, scale: 1.0 },
    ]
}

#[tokio::test]
async fn integrity_poll_reads_all_types() {
    let state = setup(20010).await;
    let client = Dnp3Client::connect(config(20010)).await.unwrap();

    // Write a binary and counter via outstation state before reading
    {
        let mut st = state.lock().unwrap();
        st.binary_values[0] = true;
        st.counter_values[0] = 42.0;
        st.analog_values[0] = 220.5;
        st.detect_events();
    }

    let vals = client.read(&points()).await.unwrap();
    assert_eq!(vals.len(), 3);
    assert!((vals[0].value - 220.5).abs() < 0.01, "analog: {}", vals[0].value);
    assert_eq!(vals[1].value, 1.0, "binary");
    assert!((vals[2].value - 42.0).abs() < 0.01, "counter: {}", vals[2].value);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn binary_write_updates_outstation_and_buffers_event() {
    let state = setup(20011).await;
    let client = Dnp3Client::connect(config(20011)).await.unwrap();

    let pt = Dnp3Point { name: "b0".into(), group: 1, variation: 2, index: 0, scale: 1.0 };
    client
        .write(&pt, Dnp3Value { point: pt.clone(), value: 1.0 })
        .await
        .unwrap();

    let st = state.lock().unwrap();
    assert_eq!(st.binary_values[0], true);
    assert!(st.events.iter().any(|e| e.group == 1 && e.index == 0 && e.class == 3));
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn time_sync_and_restart() {
    let state = setup(20012).await;
    let client = Dnp3Client::connect(config(20012)).await.unwrap();

    client.time_sync().await.unwrap();
    {
        let st = state.lock().unwrap();
        assert!(st.time_synced);
    }

    client.cold_restart().await.unwrap();
    {
        let st = state.lock().unwrap();
        assert!(st.device_restart);
        assert!(!st.time_synced);
    }

    client.warm_restart().await.unwrap();
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn event_scan_returns_buffered_events() {
    let state = setup(20013).await;
    let client = Dnp3Client::connect(config(20013)).await.unwrap();

    // Buffer events before the scan
    {
        let mut st = state.lock().unwrap();
        st.analog_values[1] = 100.0; // class 1 event
        st.binary_values[2] = true; // class 3 event
        st.counter_values[1] = 7.0; // class 2 event
        st.detect_events();
    }

    let ev = client.scan_events(1, &points()).await.unwrap();
    assert!(!ev.is_empty(), "class 1 should return the analog event");

    // After draining class 1, class 3 should still be pending
    let ev3 = client.scan_events(3, &points()).await.unwrap();
    assert!(!ev3.is_empty(), "class 3 should return the binary event");

    let ev2 = client.scan_events(2, &points()).await.unwrap();
    assert!(!ev2.is_empty(), "class 2 should return the counter event");

    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn analog_write_roundtrip() {
    let state = setup(20014).await;
    let client = Dnp3Client::connect(config(20014)).await.unwrap();

    client.write_analog(0, 88.5).await.unwrap();

    let st = state.lock().unwrap();
    assert!((st.analog_values[0] - 88.5).abs() < 0.01, "analog: {}", st.analog_values[0]);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn large_event_scan_uses_transport_segmentation() {
    let state = setup(20015).await;
    let client = Dnp3Client::connect(config(20015)).await.unwrap();

    // Buffer 100 analog events — well past the 249-byte single-frame limit.
    {
        let mut st = state.lock().unwrap();
        for i in 0..100u16 {
            st.push_event(1, 30, i, (i as f64) * 10.0, 1_000_000 + i as u64);
        }
    }

    let ev = client.scan_events(1, &points()).await.unwrap();
    assert!(!ev.is_empty(), "class 1 should return the buffered events");

    // Subsequent scan must still be in sync (no stream desync).
    let again = client.scan_events(1, &points()).await.unwrap();
    assert!(again.is_empty(), "events should have been drained");

    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn binary_read_survives_variation_mismatch() {
    // Requested variation 1, outstation answers variation 2 (integrity poll
    // encodes its own variation). The lookup must match by group+index.
    let state = setup(20016).await;
    {
        let mut st = state.lock().unwrap();
        st.binary_values[0] = true;
        st.analog_values[0] = 55.5;
    }
    let client = Dnp3Client::connect(config(20016)).await.unwrap();
    let mismatched = vec![
        Dnp3Point { name: "b0".into(), group: 1, variation: 1, index: 0, scale: 1.0 },
        Dnp3Point { name: "a0".into(), group: 30, variation: 1, index: 0, scale: 1.0 },
    ];
    let vals = client.read(mismatched.as_slice()).await.unwrap();
    assert_eq!(vals[0].value, 1.0, "binary via var mismatch: {}", vals[0].value);
    assert!((vals[1].value - 55.5).abs() < 0.01, "analog via var mismatch: {}", vals[1].value);
    client.disconnect().await.unwrap();
}

fn crob_request(index: u16, code: u8) -> ControlRequest {
    ControlRequest {
        index,
        object: ControlObject::Crob(Crob { code, count: 1, on_time_ms: 1000, off_time_ms: 0 }),
    }
}

#[tokio::test]
async fn select_before_operate_crob_round_trip() {
    let state = setup(20020).await;
    let client = Dnp3Client::connect(config(20020)).await.unwrap();

    client
        .select_before_operate(&crob_request(2, crob_code::LATCH_ON))
        .await
        .unwrap();
    {
        let st = state.lock().unwrap();
        assert!(st.binary_values[2]);
        assert!(st.events.iter().any(|e| e.group == 1 && e.index == 2 && e.class == 3));
    }
    client
        .select_before_operate(&crob_request(2, crob_code::LATCH_OFF))
        .await
        .unwrap();
    assert!(!state.lock().unwrap().binary_values[2]);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn select_before_operate_analog_round_trip() {
    let state = setup(20021).await;
    let client = Dnp3Client::connect(config(20021)).await.unwrap();

    let req = ControlRequest { index: 1, object: ControlObject::AnalogOutput(73.25) };
    client.select_before_operate(&req).await.unwrap();
    assert!((state.lock().unwrap().analog_values[1] - 73.25).abs() < 0.001);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn select_before_operate_rejects_unknown_point_and_leaves_state() {
    let state = setup(20022).await;
    let client = Dnp3Client::connect(config(20022)).await.unwrap();

    // The outstation has 4 binary points; index 9 is out of range.
    let err = client
        .select_before_operate(&crob_request(9, crob_code::LATCH_ON))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("NOT_SUPPORTED"), "{err}");
    assert!(state.lock().unwrap().binary_values.iter().all(|b| !b));
    // The session keeps working afterwards.
    client.direct_operate(&crob_request(0, crob_code::LATCH_ON)).await.unwrap();
    assert!(state.lock().unwrap().binary_values[0]);
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn poll_after_select_before_operate_sees_new_value() {
    let state = setup(20023).await;
    let client = Dnp3Client::connect(config(20023)).await.unwrap();

    client.read(&points()).await.unwrap();
    client
        .select_before_operate(&crob_request(0, crob_code::PULSE_ON))
        .await
        .unwrap();
    let vals = client.read(&points()).await.unwrap();
    assert_eq!(vals[1].value, 1.0, "binary 0 after the operate");
    assert!(state.lock().unwrap().binary_values[0]);
    client.disconnect().await.unwrap();
}
