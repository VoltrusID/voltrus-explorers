//! The path an App Review tester takes: start the built-in server, connect the client to it, browse, read, write.
//! Also covers the PKI folders, which must live under the configured root (the app data directory in the app).

use std::net::TcpListener;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use mt_opcua::{
    MonitoredItemDef, NotificationSink, NotificationValue, OpcUaConfig, OpcUaNode, OpcUaProtocolClient, OpcUaServer,
    OpcUaValue, ServerNodeDef, SubscriptionConfig, cert, pki,
};

static PKI: Once = Once::new();

fn init_pki() {
    PKI.call_once(|| {
        let root = std::env::temp_dir().join(format!("mt-opcua-loopback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        pki::set_root(root);
    });
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn config(url: &str, policy: &str, mode: &str) -> OpcUaConfig {
    OpcUaConfig {
        endpoint_url: url.to_string(),
        security_policy: policy.to_string(),
        security_mode: mode.to_string(),
        username: None,
        password: None,
        poll_interval_ms: 1000,
    }
}

async fn started_server() -> (OpcUaServer, String) {
    init_pki();
    let mut server = OpcUaServer::new();
    let url = server.start(free_port(), ServerNodeDef::default_nodes()).await.expect("server start");
    (server, url)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_browses_reads_and_writes_the_builtin_server() {
    let (mut server, url) = started_server().await;

    let mut client = OpcUaProtocolClient::new();
    client.connect(config(&url, "None", "None")).await.expect("connect");

    let nodes = client.browse_nodes().await.expect("browse");
    assert!(
        nodes.iter().any(|n| n.node_id == "ns=2;s=Reactor.Temperature"),
        "default nodes missing from browse: {nodes:?}"
    );

    let temperature = OpcUaNode {
        node_id: "ns=2;s=Reactor.Temperature".into(),
        name: "Temperature".into(),
        data_type: "Double".into(),
    };
    let before = client.read_tags(&[temperature.clone()]).await.expect("read");
    assert_eq!(before.len(), 1);

    client
        .write_tag(
            &temperature,
            &OpcUaValue {
                node_id: temperature.node_id.clone(),
                name: String::new(),
                value: serde_json::json!(123.5),
                timestamp: 0,
            },
        )
        .await
        .expect("write");
    let after = client.read_tags(&[temperature]).await.expect("read back");
    assert_eq!(after[0].value, serde_json::json!(123.5));

    let namespaces = client.read_namespaces().await.expect("namespaces");
    assert!(namespaces.iter().any(|n| n.uri == "urn:opcua-explorer:nodes"), "{namespaces:?}");

    client.disconnect().unwrap();
    server.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_and_encrypted_session_and_pki_lives_under_the_root() {
    let (mut server, url) = started_server().await;

    let mut client = OpcUaProtocolClient::new();
    client
        .connect(config(&url, "Basic256Sha256", "SignAndEncrypt"))
        .await
        .expect("secure connect");
    assert!(client.is_connected());
    client.disconnect().unwrap();
    server.stop().unwrap();

    let dirs = pki::all_dirs();
    assert!(dirs.iter().all(|d| d.starts_with(pki::root())), "{dirs:?}");
    let certs = cert::cert_list(&dirs);
    assert!(
        certs.iter().any(|c| c.bucket == "own" && c.kind == "client"),
        "client certificate was not created under the PKI root: {certs:?}"
    );
    assert!(
        certs.iter().any(|c| c.bucket == "own" && c.kind == "server"),
        "server certificate was not created under the PKI root: {certs:?}"
    );
}

struct Collect(Mutex<Vec<NotificationValue>>);

impl NotificationSink for Collect {
    fn notify(&self, value: NotificationValue) {
        self.0.lock().unwrap().push(value);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discovery_attributes_and_live_subscription() {
    let (mut server, url) = started_server().await;

    let endpoints = OpcUaProtocolClient::get_endpoints(&url).await.expect("get endpoints");
    assert!(endpoints.iter().any(|e| e.security_policy_uri.ends_with("#None")), "{endpoints:?}");
    assert!(endpoints.iter().any(|e| e.security_policy_uri.ends_with("#Basic256Sha256")), "{endpoints:?}");

    let sink = Arc::new(Collect(Mutex::new(Vec::new())));
    let mut client = OpcUaProtocolClient::new();
    client.set_notification_sink(sink.clone());
    client.connect(config(&url, "None", "None")).await.expect("connect");

    let children = client.browse_children("Objects").await.expect("browse children");
    assert!(!children.is_empty(), "Objects folder has no children");

    let attrs = client.read_attributes("ns=2;s=Reactor.Temperature").await.expect("attributes");
    assert!(attrs.iter().any(|a| a.attribute.contains("BrowseName")), "{attrs:?}");

    let sub = client
        .create_subscription(SubscriptionConfig {
            publishing_interval_ms: 100,
            lifetime_count: 60,
            max_keep_alive_count: 10,
            max_notifications_per_publish: 0,
            priority: 0,
            publishing_enabled: true,
        })
        .await
        .expect("create subscription");
    let created = client
        .create_monitored_items(
            sub,
            vec![MonitoredItemDef {
                node_id: "ns=2;s=Reactor.Temperature".into(),
                attribute_id: 13,
                sampling_interval_ms: 100.0,
                queue_size: 10,
                discard_oldest: true,
                monitoring_mode: "Reporting".into(),
                trigger: "StatusValue".into(),
                deadband_type: 0,
                deadband_value: 0.0,
            }],
        )
        .await
        .expect("create monitored items");
    assert_eq!(created, 1);

    // The first notification carries the current value; a server-side write must follow.
    tokio::time::sleep(Duration::from_millis(500)).await;
    server.write_node("ns=2;s=Reactor.Temperature", serde_json::json!(42.0), "Double").unwrap();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let seen = sink.0.lock().unwrap().clone();
    assert!(
        seen.iter().any(|n| n.node_id == "ns=2;s=Reactor.Temperature" && n.value == serde_json::json!(42.0)),
        "no data-change notification with the written value: {seen:?}"
    );

    client.delete_subscription(sub).await.expect("delete subscription");
    client.disconnect().unwrap();
    server.stop().unwrap();
}
