//! Roundtrip tests: the embedded NATS server vs the real async-nats client.

use std::time::Duration;

use mt_nats::*;
use tokio::time::timeout;

/// Grab an ephemeral free TCP port.
async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

fn server_config(port: u16) -> NatsServerConfig {
    NatsServerConfig {
        enabled: true,
        port,
        max_connections: 10,
        auth: NatsAuthConfig::default(),
    }
}

async fn start_test_server() -> (NatsServerState, u16) {
    let port = free_port().await;
    let state = NatsServerState::new(server_config(port));
    start_server(state.clone(), None).await.expect("start server");
    // Give the listener a beat to accept.
    tokio::time::sleep(Duration::from_millis(100)).await;
    (state, port)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connect_publish_subscribe() {
    let (_state, port) = start_test_server().await;

    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    let client = NatsClient::connect(&config).await.expect("client connect");

    // Subscribe first, then publish.
    let mut sub = client
        .subscribe("demo.subject", None)
        .await
        .expect("subscribe");
    client
        .publish("demo.subject", b"hello nats")
        .await
        .expect("publish");

    let msg = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("timeout waiting for message")
        .expect("message");
    assert_eq!(msg.subject, "demo.subject");
    assert_eq!(msg.payload_bytes(), b"hello nats");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wildcard_subscription() {
    let (_state, port) = start_test_server().await;

    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    let client = NatsClient::connect(&config).await.expect("client connect");

    // `foo.*` matches `foo.bar` but not `foo.bar.baz`.
    let mut sub = client.subscribe("foo.*", None).await.expect("subscribe");
    client.publish("foo.bar", b"1").await.expect("pub1");
    client.publish("foo.bar.baz", b"2").await.expect("pub2");
    client.publish("foo.qux", b"3").await.expect("pub3");

    let msg1 = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("t1")
        .expect("m1");
    assert_eq!(msg1.subject, "foo.bar");
    let msg3 = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("t2")
        .expect("m2");
    assert_eq!(msg3.subject, "foo.qux");
    // foo.bar.baz must NOT arrive.
    let extra = timeout(Duration::from_millis(300), sub.next()).await;
    assert!(extra.is_err(), "wildcard must not match deeper subjects");

    // `foo.>` matches everything under foo.
    let mut sub2 = client.subscribe("foo.>", None).await.expect("sub2");
    client.publish("foo.a.b.c", b"deep").await.expect("pub deep");
    let deep = timeout(Duration::from_secs(2), sub2.next())
        .await
        .expect("t3")
        .expect("m4");
    assert_eq!(deep.subject, "foo.a.b.c");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_headers_roundtrip() {
    let (_state, port) = start_test_server().await;

    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    let client = NatsClient::connect(&config).await.expect("client connect");

    let mut sub = client.subscribe("hdr", None).await.expect("subscribe");
    let headers = vec![
        ("X-Trace".to_string(), "abc123".to_string()),
        ("Nats-Msg-Id".to_string(), "42".to_string()),
    ];
    client
        .publish_with_headers("hdr", &headers, b"body")
        .await
        .expect("publish headers");

    let msg = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("timeout")
        .expect("msg");
    assert_eq!(msg.payload_bytes(), b"body");
    let map = msg.headers_map();
    assert_eq!(map.get("X-Trace").and_then(|v| v.as_str()), Some("abc123"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ping_keepalive() {
    let (_state, port) = start_test_server().await;

    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    // async-nats sends PINGs automatically; ensure the connection stays alive.
    let client = NatsClient::connect(&config).await.expect("connect");
    for i in 0..5 {
        client
            .publish("ping.subject", format!("msg {}", i).as_bytes())
            .await
            .expect("publish");
    }
    // Connection stays alive through PING/PONG keepalive.
    let mut sub = client.subscribe("ping.subject", None).await.expect("sub");
    client.publish("ping.subject", b"still alive").await.expect("publish");
    let m = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("timeout")
        .expect("msg");
    assert_eq!(m.payload_bytes(), b"still alive");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_rejected() {
    let port = free_port().await;
    let state = NatsServerState::new(NatsServerConfig {
        enabled: true,
        port,
        max_connections: 5,
        auth: NatsAuthConfig {
            mode: "password".to_string(),
            users: vec![NatsUser {
                username: "admin".to_string(),
                password: "secret".to_string(),
            }],
        },
    });
    start_server(state.clone(), None).await.expect("start");

    let bad = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        username: Some("admin".to_string()),
        password: Some("wrong".to_string()),
        ..Default::default()
    };
    let res = timeout(Duration::from_secs(5), NatsClient::connect(&bad)).await;
    assert!(
        matches!(res, Err(_)) || matches!(res, Ok(Err(_))),
        "bad credentials must fail to connect"
    );

    // Good credentials work.
    let good = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        username: Some("admin".to_string()),
        password: Some("secret".to_string()),
        ..Default::default()
    };
    let client = NatsClient::connect(&good).await.expect("good creds connect");
    let mut sub = client.subscribe("auth.ok", None).await.expect("sub");
    client.publish("auth.ok", b"hi").await.expect("pub");
    let m = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("timeout")
        .expect("msg");
    assert_eq!(m.payload_bytes(), b"hi");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_queue_groups_deliver_once() {
    let (_state, port) = start_test_server().await;

    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    let c1 = NatsClient::connect(&config).await.expect("c1");
    let c2 = NatsClient::connect(&config).await.expect("c2");

    let mut s1 = c1
        .subscribe("jobs", Some("workers".to_string()))
        .await
        .expect("s1");
    let mut s2 = c2
        .subscribe("jobs", Some("workers".to_string()))
        .await
        .expect("s2");

    // Publish one message to a queue group with two members. Exactly one
    // member should receive it (queue-group semantics: one per group).
    c1.publish("jobs", b"single job").await.expect("pub");

    let m1 = timeout(Duration::from_secs(2), s1.next()).await;
    let m2 = timeout(Duration::from_secs(2), s2.next()).await;
    let got = match (m1, m2) {
        (Ok(Some(m)), _) => Some(m.payload_bytes()),
        (_, Ok(Some(m))) => Some(m.payload_bytes()),
        _ => None,
    };
    assert_eq!(got, Some(b"single job".to_vec()), "one queue member gets the message");

    // The other member must NOT receive the same message again.
    let extra1 = timeout(Duration::from_millis(300), s1.next()).await;
    let extra2 = timeout(Duration::from_millis(300), s2.next()).await;
    assert!(
        extra1.is_err() || extra2.is_err(),
        "queue group must deliver to only one member"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_request_reply() {
    let (_state, port) = start_test_server().await;

    let config = NatsClientConfig {
        host: "127.0.0.1".to_string(),
        port,
        ..Default::default()
    };
    let server_client = NatsClient::connect(&config).await.expect("svc connect");
    let client = NatsClient::connect(&config).await.expect("cli connect");

    // Service: subscribe to "calc.add", publish reply back.
    let mut sub = server_client.subscribe("calc.add", None).await.unwrap();
    // SUB travels on another connection than the request: let the server register it first.
    tokio::time::sleep(Duration::from_millis(200)).await;
    tokio::spawn(async move {
        while let Some(req) = sub.next().await {
            let reply = req.reply.clone().expect("request has reply");
            let payload = req.payload_bytes();
            let a: i64 = String::from_utf8_lossy(&payload).trim().parse().unwrap_or(0);
            server_client
                .publish(&reply, format!("{}", a + 1).as_bytes())
                .await
                .ok();
        }
    });

    let reply = client
        .request("calc.add", b"41", 2000)
        .await
        .expect("request");
    assert_eq!(reply.payload_bytes(), b"42");
}
