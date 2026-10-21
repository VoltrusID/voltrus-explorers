use std::sync::Arc;

use mt_dnp3::{
    Dnp3Client, Dnp3Config, Dnp3Point, Dnp3Value, OutstationConfig, OutstationState,
    start_outstation_server,
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
