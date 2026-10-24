//! JetStream roundtrip tests: the embedded server's `$JS.API` vs the real
//! async-nats JetStream client.

use std::time::Duration;

use futures_util::StreamExt;
use mt_nats::*;
use tokio::time::timeout;

async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

async fn start_test_server() -> (NatsServerState, u16) {
    let port = free_port().await;
    let state = NatsServerState::new(NatsServerConfig {
        enabled: true,
        port,
        max_connections: 10,
        auth: NatsAuthConfig::default(),
    });
    start_server(state.clone(), None).await.expect("start server");
    tokio::time::sleep(Duration::from_millis(100)).await;
    (state, port)
}

async fn client(port: u16) -> NatsClient {
    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    NatsClient::connect(&config).await.expect("connect")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_js_create_stream_and_list() {
    let (_state, port) = start_test_server().await;
    let c = client(port).await;

    let info = c
        .js_create_stream("events", &["events.>".to_string()], 100, 0)
        .await
        .expect("create stream");
    assert_eq!(info.name, "events");
    assert_eq!(info.subjects, vec!["events.>".to_string()]);
    assert_eq!(info.messages, 0);

    let names = c.js_list_streams().await.expect("list streams");
    assert!(names.contains(&"events".to_string()));

    let again = c.js_stream_info("events").await.expect("info");
    assert!(again.is_some());
    assert_eq!(again.unwrap().name, "events");

    let missing = c.js_stream_info("nope").await.expect("info missing");
    assert!(missing.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_js_publish_stores_and_fetch() {
    let (_state, port) = start_test_server().await;
    let c = client(port).await;

    c.js_create_stream("events", &["events.>".to_string()], 1000, 0)
        .await
        .expect("create stream");

    // Publish several messages to a matching subject.
    for i in 0..5 {
        c.publish("events.user.created", format!("user-{}", i).as_bytes())
            .await
            .expect("publish");
    }

    // Stream info should reflect stored count.
    let info = c.js_stream_info("events").await.expect("info").unwrap();
    assert_eq!(info.messages, 5);

    // Fetch the stored messages.
    let msgs = c
        .js_stream_messages("events", 10)
        .await
        .expect("stream messages");
    assert_eq!(msgs.len(), 5);
    assert_eq!(msgs[0].sequence, 1);
    assert_eq!(msgs[0].subject, "events.user.created");
    assert_eq!(payload_utf8(&msgs[4]), "user-4");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_js_pull_consumer_full_flow() {
    let (_state, port) = start_test_server().await;
    let c = client(port).await;

    c.js_create_stream("orders", &["orders.>".to_string()], 100, 0)
        .await
        .expect("create stream");
    for i in 0..5 {
        c.publish("orders.new", format!("order-{}", i).as_bytes())
            .await
            .expect("publish");
    }

    let consumer = c
        .js_create_consumer("orders", "worker")
        .await
        .expect("create consumer");
    assert_eq!(consumer.stream, "orders");
    assert_eq!(consumer.name, "worker");
    assert_eq!(consumer.deliver_kind, "pull");

    let info = c
        .js_consumer_info("orders", "worker")
        .await
        .expect("consumer info");
    assert_eq!(info.name, "worker");
    assert!(info.pending >= 5);

    // Fetch a batch of 3 — should get order-0..2.
    let batch = c
        .js_consumer_next("orders", "worker", 3, 3000)
        .await
        .expect("fetch batch");
    assert_eq!(batch.len(), 3, "got {:?}", batch.iter().map(|m| m.sequence).collect::<Vec<_>>());
    assert_eq!(payload_utf8(&batch[0]), "order-0");

    // Fetch the rest.
    let rest = c
        .js_consumer_next("orders", "worker", 10, 3000)
        .await
        .expect("fetch rest");
    assert_eq!(rest.len(), 2);
    assert_eq!(payload_utf8(&rest[0]), "order-3");

    // Nothing left.
    let none = c
        .js_consumer_next("orders", "worker", 5, 1500)
        .await
        .expect("fetch none");
    assert!(none.is_empty(), "no more messages expected");

    // Cleanup.
    c.js_delete_consumer("orders", "worker").await.expect("delete consumer");
    c.js_purge_stream("orders").await.expect("purge stream");
    let info = c.js_stream_info("orders").await.expect("info").unwrap();
    assert_eq!(info.messages, 0);
    c.js_delete_stream("orders").await.expect("delete stream");
    let missing = c.js_stream_info("orders").await.expect("info");
    assert!(missing.is_none(), "stream should be gone after delete");
}

// Small helper: interpret a stored message payload as UTF-8.
fn payload_utf8(m: &NatsStoredMessage) -> String {
    if let Some(hex_part) = m.payload.strip_prefix("0x") {
        String::from_utf8_lossy(&hex::decode(hex_part).unwrap_or_default()).into_owned()
    } else {
        m.payload.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_js_raw_client_compat() {
    // Use the raw async-nats JetStream client (not our wrapper) to prove the
    // wire protocol is compatible with the official client.
    let (_state, port) = start_test_server().await;
    let client = async_nats::connect(format!("127.0.0.1:{}", port))
        .await
        .expect("raw connect");
    let js = async_nats::jetstream::new(client.clone());

    js.create_stream(async_nats::jetstream::stream::Config {
        name: "raw".to_string(),
        subjects: vec!["raw.>".to_string()],
        ..Default::default()
    })
    .await
    .expect("raw create stream");

    client.publish("raw.msg", "hello-raw".into()).await.expect("raw publish");
    client.flush().await.expect("flush");

    let stream = js.get_stream("raw").await.expect("raw get stream");
    let info = stream.cached_info().clone();
    assert_eq!(info.state.messages, 1);

    let consumer = stream
        .get_or_create_consumer(
            "c1",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("c1".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("raw create consumer");

    let mut msgs = consumer
        .fetch()
        .max_messages(5)
        .expires(Duration::from_secs(2))
        .messages()
        .await
        .expect("raw fetch");
    let msg = timeout(Duration::from_secs(3), msgs.next())
        .await
        .expect("timeout")
        .expect("some message")
        .expect("ok message");
    assert_eq!(String::from_utf8_lossy(&msg.payload), "hello-raw");
    let seq = msg.info().expect("info").stream_sequence;
    assert_eq!(seq, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_js_create_stream_with_message_callback() {
    // The app starts the server with an on_message callback that emits UI events.
    let port = free_port().await;
    let state = NatsServerState::new(NatsServerConfig {
        enabled: true,
        port,
        max_connections: 10,
        auth: NatsAuthConfig::default(),
    });
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen2 = seen.clone();
    let cb: OnMessage = std::sync::Arc::new(move |_s, _p, _t, _h| {
        seen2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    start_server(state.clone(), Some(cb)).await.expect("start server");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let c = client(port).await;

    let started = std::time::Instant::now();
    let info = timeout(
        Duration::from_secs(5),
        c.js_create_stream("EVENTS", &["events.>".to_string()], 10000, 0),
    )
    .await
    .expect("create stream timed out")
    .expect("create stream");
    eprintln!("create_stream took {:?}", started.elapsed());
    assert_eq!(info.name, "EVENTS");
}
