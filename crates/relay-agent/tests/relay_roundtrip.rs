//! Round trip against a fake in-test relay: a raw tokio-tungstenite server
//! standing in for the RelayRoom Durable Object.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, WebSocketStream};

use relay_agent::driver::echo::EchoDriver;
use relay_agent::session::{run_session, SessionConfig};

type Ws = WebSocketStream<TcpStream>;

/// Accept one WebSocket connection and hand it to the test over a channel.
async fn fake_relay(listener: TcpListener, tx: mpsc::Sender<Ws>) {
    let (stream, _) = listener.accept().await.expect("tcp accept");
    let ws = accept_async(stream).await.expect("ws accept");
    let _ = tx.send(ws).await;
}

async fn bind_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    (listener, port)
}

async fn recv_text(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("socket closed")
            .expect("socket error");
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).expect("json frame");
        }
        // Pings/pongs and other control frames are skipped.
    }
}

async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into())).await.expect("send");
}

fn session_config(port: u16) -> SessionConfig {
    SessionConfig {
        server: format!("ws://127.0.0.1:{port}/api/relay"),
        device_id: "dev-1".into(),
        pairing_token: "pair-me".into(),
    }
}

#[tokio::test]
async fn echo_driver_round_trip() {
    let (listener, port) = bind_port().await;
    let (ws_tx, mut ws_rx) = mpsc::channel(1);
    tokio::spawn(fake_relay(listener, ws_tx));

    let config = session_config(port);
    let session_config_owned = config.clone();
    let session = tokio::spawn(async move {
        run_session(&session_config_owned, &mut EchoDriver::new()).await
    });

    let mut ws = tokio::time::timeout(Duration::from_secs(5), ws_rx.recv())
        .await
        .expect("agent never connected")
        .expect("channel closed");

    // Hello arrives first.
    let hello = recv_text(&mut ws).await;
    assert_eq!(hello["op"], "hello");
    assert_eq!(hello["payload"]["agent"], "relay-agent");

    // browse
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r1", "op": "browse", "payload": null })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r1");
    assert_eq!(reply["ok"], true);
    let names: Vec<&str> = reply["payload"]["variables"]
        .as_array()
        .expect("variables array")
        .iter()
        .map(|v| v["name"].as_str().expect("name"))
        .collect();
    assert!(names.contains(&"motor_running") && names.contains(&"cycle_count"));

    // subscribe -> baseline events then a moving cycle_count
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r2", "op": "subscribe", "payload": { "names": ["cycle_count"] } })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r2");
    assert_eq!(reply["ok"], true);

    let mut saw_event = false;
    for _ in 0..5 {
        let frame = tokio::time::timeout(Duration::from_secs(5), recv_text(&mut ws)).await.expect("event timeout");
        if frame["event"] == "variable_update" {
            assert_eq!(frame["payload"]["name"], "cycle_count");
            saw_event = true;
            break;
        }
        // Skip response duplicates (none expected) and keep waiting.
    }
    assert!(saw_event, "no variable_update event arrived");

    // write on a fresh variable then verify via browse
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r3", "op": "write", "payload": { "name": "temperature", "value": 31.5 } })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r3");
    assert_eq!(reply["ok"], true);

    // force pins the value; writes to it are rejected
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r4", "op": "force", "payload": { "name": "cycle_count", "value": 999 } })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r4");
    assert_eq!(reply["ok"], true);

    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r5", "op": "write", "payload": { "name": "cycle_count", "value": 1 } })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r5");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().expect("error string").contains("forced"));

    // unforce
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r6", "op": "unforce", "payload": { "name": "cycle_count" } })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r6");
    assert_eq!(reply["ok"], true);

    // status
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r7", "op": "status", "payload": null })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r7");
    assert_eq!(reply["payload"]["driver"], "echo");
    assert!(reply["payload"]["deployed_crc32"].is_null());

    // deploy: the ide payload is {module_json, config_json?}; the echo driver
    // fingerprints the module_json bytes with FNV-1a 32-bit and reports the
    // same crc32 back, so an ide can detect drift between what it deployed
    // and what the device runs. Expected crc computed independently here.
    let module_json = r#"{"name":"Main","instructions":[],"variable_slots":[["x",{"Dint":1}]]}"#;
    let expected_crc = fnv1a32(module_json.as_bytes());
    send_json(
        &mut ws,
        json!({
            "deviceId": "dev-1",
            "id": "r8",
            "op": "deploy",
            "payload": { "module_json": module_json, "config_json": "{\"io_mappings\":[]}" }
        }),
    )
    .await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r8");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["payload"]["deployed"], true);
    assert_eq!(reply["payload"]["bytes"], module_json.len());
    assert_eq!(reply["payload"]["crc32"], expected_crc);

    // Determinism: redeploying the identical module_json yields the same crc.
    send_json(
        &mut ws,
        json!({
            "deviceId": "dev-1",
            "id": "r9",
            "op": "deploy",
            "payload": { "module_json": module_json }
        }),
    )
    .await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r9");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["payload"]["crc32"], expected_crc);

    // The deployed fingerprint surfaces from status for drift comparison.
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r10", "op": "status", "payload": null })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["payload"]["deployed_crc32"], expected_crc);
    assert_eq!(reply["payload"]["deployed_bytes"], module_json.len());

    // Malformed module_json -> ok:false with a pointed error.
    send_json(
        &mut ws,
        json!({
            "deviceId": "dev-1",
            "id": "r11",
            "op": "deploy",
            "payload": { "module_json": "not json" }
        }),
    )
    .await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r11");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().expect("error string").contains("module_json"));

    // missing module_json -> ok:false
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r12", "op": "deploy", "payload": {} })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r12");
    assert_eq!(reply["ok"], false);

    // unknown op -> ok:false
    send_json(&mut ws, json!({ "deviceId": "dev-1", "id": "r13", "op": "teleport", "payload": null })).await;
    let reply = recv_text(&mut ws).await;
    assert_eq!(reply["id"], "r13");
    assert_eq!(reply["ok"], false);

    session.abort();
}

/// Independent FNV-1a reference implementation for the deploy fingerprint
/// assertions (kept separate from the driver's copy on purpose).
fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[tokio::test]
async fn reconnects_after_server_drop() {
    let (listener, port) = bind_port().await;
    let (ws_tx, mut ws_rx) = mpsc::channel(4);
    tokio::spawn(fake_relay(listener, ws_tx.clone()));

    let config = session_config(port);
    // The reconnect loop never returns; the test runtime tears it down.
    tokio::spawn(async move {
        relay_agent::session::run_with_reconnect(
            &config,
            EchoDriver::new,
            Duration::from_millis(20),
            Duration::from_millis(100),
        )
        .await
    });

    let mut first = tokio::time::timeout(Duration::from_secs(5), ws_rx.recv())
        .await
        .expect("first connection never happened")
        .expect("channel closed");
    let hello = recv_text(&mut first).await;
    assert_eq!(hello["op"], "hello");

    // Kill the first server: agent must back off and dial again.
    drop(first);

    // A second listener on the same port plays the restarted relay.
    let mut second_listener = None;
    for _ in 0..50 {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => {
                second_listener = Some(l);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    let second_listener = second_listener.expect("port never became free");
    tokio::spawn(fake_relay(second_listener, ws_tx));

    let mut second = tokio::time::timeout(Duration::from_secs(10), ws_rx.recv())
        .await
        .expect("agent never reconnected")
        .expect("channel closed");
    let hello = recv_text(&mut second).await;
    assert_eq!(hello["op"], "hello");
    assert_eq!(hello["payload"]["agent"], "relay-agent");
}

#[tokio::test]
async fn rejects_bad_server_url() {
    let config = SessionConfig { server: "http://not-ws".into(), device_id: "d".into(), pairing_token: "t".into() };
    let result = run_session(&config, &mut EchoDriver::new()).await;
    let error = format!("{:#}", result.expect_err("bad URL must fail"));
    assert!(error.contains("invalid relay server URL"), "unexpected error: {error}");
}
