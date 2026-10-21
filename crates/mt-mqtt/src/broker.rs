use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::{RwLock, watch};

use crate::types::*;

// ---------------------------------------------------------------------------
// Topic tracker
// ---------------------------------------------------------------------------

const MAX_RECENT_MESSAGES: usize = 500;
const MAX_TOPIC_MESSAGES: usize = 50;

/// In-memory tracker of topics and messages passing through the broker.
#[derive(Clone)]
pub struct TopicTracker {
    topics: Arc<RwLock<HashMap<String, TopicInfo>>>,
    recent_messages: Arc<RwLock<VecDeque<RecentMessage>>>,
}

impl TopicTracker {
    pub fn new() -> Self {
        Self {
            topics: Arc::new(RwLock::new(HashMap::new())),
            recent_messages: Arc::new(RwLock::new(VecDeque::with_capacity(
                MAX_RECENT_MESSAGES,
            ))),
        }
    }

    pub async fn record_message(&self, topic: String, payload: Vec<u8>, retain: bool) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        {
            let mut topics = self.topics.write().await;
            let entry = topics.entry(topic.clone()).or_insert_with(|| TopicInfo {
                topic: topic.clone(),
                last_payload: Vec::new(),
                last_update: 0,
                message_count: 0,
                retain: false,
            });
            entry.last_payload = payload.clone();
            entry.last_update = now;
            entry.message_count += 1;
            entry.retain = retain;
        }

        {
            let mut msgs = self.recent_messages.write().await;
            if msgs.len() >= MAX_RECENT_MESSAGES {
                msgs.pop_front();
            }
            msgs.push_back(RecentMessage {
                topic,
                payload,
                timestamp: now,
                retain,
            });
        }
    }

    pub async fn get_topics(&self) -> Vec<TopicInfo> {
        let topics = self.topics.read().await;
        let mut list: Vec<TopicInfo> = topics.values().cloned().collect();
        list.sort_by(|a, b| a.topic.cmp(&b.topic));
        list
    }

    pub async fn get_topic_messages(&self, topic_prefix: &str) -> Vec<RecentMessage> {
        let msgs = self.recent_messages.read().await;
        let prefix_slash = format!("{}/", topic_prefix);
        let mut filtered: Vec<RecentMessage> = msgs
            .iter()
            .filter(|m| m.topic == topic_prefix || m.topic.starts_with(&prefix_slash))
            .rev()
            .take(MAX_TOPIC_MESSAGES)
            .cloned()
            .collect();
        filtered.reverse();
        filtered
    }

    pub async fn get_recent_messages(&self, limit: usize) -> Vec<RecentMessage> {
        let msgs = self.recent_messages.read().await;
        let mut result: Vec<RecentMessage> = msgs.iter().rev().take(limit).cloned().collect();
        result.reverse();
        result
    }

    pub async fn clear(&self) {
        self.topics.write().await.clear();
        self.recent_messages.write().await.clear();
    }
}

impl Default for TopicTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Broker state
// ---------------------------------------------------------------------------

/// Shared broker state accessible from callers.
#[derive(Clone)]
pub struct MqttBrokerState {
    pub status: Arc<watch::Sender<BrokerStatus>>,
    pub status_rx: watch::Receiver<BrokerStatus>,
    pub config: Arc<RwLock<Option<MqttBrokerConfig>>>,
    pub clients: Arc<RwLock<Vec<ConnectedClient>>>,
    pub shutdown_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    pub topic_tracker: TopicTracker,
    publish_tx: Arc<std::sync::Mutex<Option<rumqttd::local::LinkTx>>>,
}

impl MqttBrokerState {
    pub fn new() -> Self {
        let (status_tx, status_rx) = watch::channel(BrokerStatus::Stopped);
        Self {
            status: Arc::new(status_tx),
            status_rx,
            config: Arc::new(RwLock::new(None)),
            clients: Arc::new(RwLock::new(Vec::new())),
            shutdown_tx: Arc::new(tokio::sync::Mutex::new(None)),
            topic_tracker: TopicTracker::new(),
            publish_tx: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub async fn current_status(&self) -> BrokerStatus {
        self.status_rx.borrow().clone()
    }

    pub fn publish_sync(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        let mut guard = self.publish_tx.lock().map_err(|e| e.to_string())?;
        if let Some(ref mut link_tx) = *guard {
            link_tx
                .publish(topic.to_string(), payload.to_vec())
                .map_err(|e| format!("Publish failed: {}", e))?;
            Ok(())
        } else {
            Err("Broker is not running".to_string())
        }
    }
}

impl Default for MqttBrokerState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Broker config builder
// ---------------------------------------------------------------------------

/// Build a `rumqttd::Config` from our `MqttBrokerConfig`.
///
/// `tls` is an optional pre-built TLS acceptor config (see
/// [`tls_config_from`] — it writes PEM bodies to temp files because rumqttd
/// requires file paths). Pass `None` for a plaintext broker.
pub fn build_rumqttd_config(
    config: &MqttBrokerConfig,
    tls: Option<rumqttd::TlsConfig>,
) -> rumqttd::Config {
    use std::net::SocketAddr;

    let name = format!("mt-mqtt-{}", config.port);
    let listen: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .expect("valid socket address");

    // Build the auth map (username → password). rumqttd enforces it on every
    // CONNECT. `auth.mode == "none"` or no valid users → open broker. Users
    // with empty usernames are skipped to avoid creating a key collision.
    let valid_users: Vec<&MqttUser> = config
        .auth
        .users
        .iter()
        .filter(|u| !u.username.trim().is_empty())
        .collect();
    let auth_map: Option<HashMap<String, String>> =
        if config.auth.mode == "none" || valid_users.is_empty() {
            None
        } else {
            let mut m = HashMap::new();
            for u in valid_users {
                m.insert(u.username.clone(), u.password.clone());
            }
            Some(m)
        };

    // Shared connection settings across all listeners (v4 + ws).
    let conn_settings = || rumqttd::ConnectionSettings {
        connection_timeout_ms: 60_000u16,
        max_payload_size: 268_435_456,
        max_inflight_count: 1000,
        auth: auth_map.clone(),
        external_auth: None,
        dynamic_filters: true,
    };

    let server = rumqttd::ServerSettings {
        name,
        listen,
        tls: tls.clone(),
        next_connection_delay_ms: 1,
        connections: conn_settings(),
    };

    let mut v4 = HashMap::new();
    v4.insert("v4-1".to_string(), server);

    // Optional WebSocket listener (MQTT over WS) for browser-based clients.
    let ws = if let Some(ws_port) = config.ws_port {
        let ws_listen: SocketAddr = format!("0.0.0.0:{}", ws_port)
            .parse()
            .expect("valid ws socket address");
        let ws_server = rumqttd::ServerSettings {
            name: format!("mt-mqtt-ws-{}", ws_port),
            listen: ws_listen,
            tls: None,
            next_connection_delay_ms: 1,
            connections: conn_settings(),
        };
        let mut ws_map = HashMap::new();
        ws_map.insert("ws-1".to_string(), ws_server);
        Some(ws_map)
    } else {
        None
    };

    rumqttd::Config {
        id: 0,
        router: rumqttd::RouterConfig {
            max_connections: config.max_connections as usize,
            max_outgoing_packet_count: 1000,
            max_segment_size: 104_857_600,
            max_segment_count: 10,
            custom_segment: None,
            initialized_filters: None,
            shared_subscriptions_strategy: rumqttd::Strategy::RoundRobin,
        },
        v4: Some(v4),
        v5: None,
        ws,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    }
}

/// Prepare a rumqttd `TlsConfig::Rustls` from our server TLS settings.
///
/// rumqttd reads cert/key/CA from file paths, so the PEM bodies are written
/// to a unique per-call temp directory (returned so the caller can clean up).
/// When `ca_pem` is present, the broker requires client certificates (mTLS);
/// rumqttd only enforces this when its `verify-client-cert` feature is on,
/// which we don't enable — so `ca_pem` is accepted but currently only used for
/// path validation. Returns `(TlsConfig, temp_dir_guard)` or an error string.
pub fn tls_config_from(
    tls: &MqttServerTlsConfig,
) -> Result<(rumqttd::TlsConfig, Option<std::path::PathBuf>), String> {
    use std::io::Write;

    let cert = tls.cert_pem.as_deref().unwrap_or("");
    let key = tls.key_pem.as_deref().unwrap_or("");
    if cert.trim().is_empty() || key.trim().is_empty() {
        return Err("Broker TLS requires both a server certificate and a key".to_string());
    }

    // Unique per-call dir avoids collisions between concurrent brokers.
    let dir = std::env::temp_dir().join(format!(
        "mt-mqtt-broker-tls-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create TLS temp dir: {}", e))?;

    let cert_path = dir.join("server.crt");
    let key_path = dir.join("server.key");
    let mut f = std::fs::File::create(&cert_path)
        .map_err(|e| format!("Failed to write server cert: {}", e))?;
    f.write_all(cert.trim().as_bytes())
        .map_err(|e| format!("Failed to write server cert: {}", e))?;
    drop(f);
    let mut f = std::fs::File::create(&key_path)
        .map_err(|e| format!("Failed to write server key: {}", e))?;
    f.write_all(key.trim().as_bytes())
        .map_err(|e| format!("Failed to write server key: {}", e))?;
    drop(f);

    // Optional CA enables client-certificate verification (mTLS).
    let capath = match tls.ca_pem.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(ca) => {
            let ca_path = dir.join("ca.crt");
            let mut f = std::fs::File::create(&ca_path)
                .map_err(|e| format!("Failed to write CA cert: {}", e))?;
            f.write_all(ca.as_bytes())
                .map_err(|e| format!("Failed to write CA cert: {}", e))?;
            drop(f);
            Some(ca_path.to_string_lossy().into_owned())
        }
        None => None,
    };

    let config = rumqttd::TlsConfig::Rustls {
        capath,
        certpath: cert_path.to_string_lossy().into_owned(),
        keypath: key_path.to_string_lossy().into_owned(),
    };

    Ok((config, Some(dir)))
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

/// Validate MQTT credentials against the configured auth settings.
pub fn validate_credentials(
    auth_config: &MqttAuthConfig,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    if auth_config.mode == "none" || auth_config.users.is_empty() {
        return Ok(());
    }

    let username = username.ok_or_else(|| "Username required when auth is enabled".to_string())?;
    let password = password.ok_or_else(|| "Password required when auth is enabled".to_string())?;

    let user = auth_config
        .users
        .iter()
        .find(|u| u.username == username)
        .ok_or_else(|| format!("Unknown MQTT user: {}", username))?;

    if user.password != password {
        return Err("Invalid MQTT password".to_string());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Start / stop broker
// ---------------------------------------------------------------------------

/// Verify that the broker's TCP (and optional WS) ports are free to bind.
///
/// rumqttd 0.20 swallows bind errors inside its server threads, so we pre-bind
/// (and immediately release) the listener here to surface port conflicts to the
/// caller. `0.0.0.0` is used to match rumqttd's own bind address.
fn check_ports_free(config: &MqttBrokerConfig) -> Result<(), String> {
    for (label, port) in [
        ("TCP", config.port),
        ("WebSocket", config.ws_port.unwrap_or(0)),
    ] {
        if port == 0 {
            continue;
        }
        match std::net::TcpListener::bind(("0.0.0.0", port)) {
            Ok(l) => drop(l), // released immediately; rumqttd takes it next
            Err(e) => {
                return Err(format!(
                    "Broker {label} port {port} is already in use: {e}"
                ));
            }
        }
    }
    Ok(())
}

/// Start the embedded MQTT broker as a background task.
///
/// If `on_message` is provided, it will be called for every message passing through
/// the broker (topic, payload as string, timestamp millis, retain).
pub async fn start_broker(
    state: MqttBrokerState,
    config: MqttBrokerConfig,
    on_message: Option<Arc<dyn Fn(String, String, i64, bool) + Send + Sync + 'static>>,
) -> Result<(), String> {
    if state.current_status().await == BrokerStatus::Running {
        return Err("Broker is already running".to_string());
    }

    let _ = state.status.send(BrokerStatus::Starting);

    {
        let mut cfg = state.config.write().await;
        *cfg = Some(config.clone());
    }

    // Pre-flight: verify the TCP/WS ports are bindable before handing the
    // config to rumqttd. rumqttd 0.20 swallows bind errors inside its server
    // threads (it joins them and returns Ok), so this is the only way to
    // surface a port conflict to the caller.
    if let Err(e) = check_ports_free(&config) {
        let _ = state.status.send(BrokerStatus::Error(e.clone()));
        return Err(e);
    }

    // Prepare TLS listener config (writes PEM bodies to temp files).
    let (tls_config, tls_temp_dir) = if config.tls.enabled {
        let (c, d) = tls_config_from(&config.tls)?;
        (Some(c), d)
    } else {
        (None, None)
    };

    let broker_config = build_rumqttd_config(&config, tls_config);
    let port = config.port;
    let status_tx = state.status.clone();
    let tracker = state.topic_tracker.clone();
    let publish_tx = state.publish_tx.clone();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut handle = state.shutdown_tx.lock().await;
        *handle = Some(shutdown_tx);
    }

    let status_tx_clone = status_tx.clone();
    let status_tx_final = status_tx.clone();

    tokio::task::spawn_blocking(move || {
        let mut broker = rumqttd::Broker::new(broker_config);
        // Remove the TLS temp files once the broker shuts down.
        let _tls_guard = tls_temp_dir.as_ref().map(|d| {
            struct CleanupDir(std::path::PathBuf);
            impl Drop for CleanupDir {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            CleanupDir(d.clone())
        });

        // Create local link to intercept all messages
        let (mut link_tx, mut link_rx) = broker
            .link("mt-mqtt-tracker")
            .expect("failed to create local link");

        link_tx
            .subscribe("#")
            .expect("failed to subscribe to wildcard");

        // Store the link_tx for publish capability
        {
            let mut px = publish_tx.lock().unwrap();
            *px = Some(link_tx);
        }

        let tracker_clone = tracker.clone();
        let rt = tokio::runtime::Handle::current();
        let on_msg = on_message.clone();
        let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag_clone = stop_flag.clone();

        // Spawn notification processor thread
        let _notif_handle = std::thread::Builder::new()
            .name("mqtt-tracker".into())
            .spawn(move || {
                eprintln!("[mqtt-tracker] Notification processor started, waiting for messages...");
                loop {
                    if stop_flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    // Wake up periodically so we can observe the stop flag; the
                    // router channel stays open after the broker shuts down.
                    let deadline = std::time::Instant::now()
                        + std::time::Duration::from_millis(500);
                    match link_rx.recv_deadline(deadline) {
                        Ok(Some(rumqttd::Notification::Forward(forward))) => {
                            let topic =
                                String::from_utf8_lossy(&forward.publish.topic).to_string();
                            let payload = forward.publish.payload.to_vec();
                            let retain = forward.publish.retain;

                            eprintln!("[mqtt-tracker] Forward: topic={}", topic);

                            let payload_str = String::from_utf8_lossy(&payload).to_string();
                            let ts = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64;

                            if let Some(ref cb) = on_msg {
                                eprintln!("[mqtt-tracker] Calling on_message callback");
                                cb(topic.clone(), payload_str, ts, retain);
                            } else {
                                eprintln!("[mqtt-tracker] on_message is None, no callback");
                            }

                            let t = tracker_clone.clone();
                            rt.block_on(t.record_message(topic, payload, retain));
                        }
                        Ok(None) | Ok(Some(rumqttd::Notification::Unschedule)) => {}
                        Ok(Some(_)) => {}
                        // Deadline elapsed (no traffic) — keep polling so we can
                        // observe the stop flag. Only a closed channel ends us.
                        Err(e) if matches!(e, rumqttd::local::LinkError::Elapsed(_) | rumqttd::local::LinkError::RecvTimeout(_)) => {}
                        Err(_) => break,
                    }
                }
                eprintln!("[mqtt-tracker] Notification processor exiting");
            });

        // Bind the port and start serving. rumqttd 0.20 never returns Err here
        // on a bind failure (it swallows it inside its server threads), so port
        // conflicts are caught by `check_ports_free` before this task spawns.
        if let Err(e) = broker.start() {
            tracing::error!("MQTT broker error: {}", e);
            stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = status_tx_clone.send(BrokerStatus::Error(e.to_string()));
            {
                let mut px = publish_tx.lock().unwrap();
                *px = None;
            }
            return;
        }

        let _ = shutdown_rx.blocking_recv();

        tracing::info!("MQTT broker shutting down on port {}", port);
        stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = status_tx_clone.send(BrokerStatus::Stopped);

        {
            let mut px = publish_tx.lock().unwrap();
            *px = None;
        }
    });

    // Wait for publish_tx to be ready (the spawn_blocking task sets it).
    {
        let publish_tx = state.publish_tx.clone();
        for i in 0..30 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if publish_tx.lock().map(|g| g.is_some()).unwrap_or(false) {
                break;
            }
            if i == 29 {
                tracing::warn!("MQTT broker publish_tx not ready after 3s");
            }
        }
    }

    let _ = status_tx_final.send(BrokerStatus::Running);
    tracing::info!("MQTT broker started on port {}", port);

    Ok(())
}

/// Stop the embedded MQTT broker.
pub async fn stop_broker(state: MqttBrokerState) -> Result<(), String> {
    if state.current_status().await != BrokerStatus::Running {
        return Err("Broker is not running".to_string());
    }

    let mut handle = state.shutdown_tx.lock().await;
    if let Some(tx) = handle.take() {
        let _ = tx.send(());
    }

    {
        let mut clients = state.clients.write().await;
        clients.clear();
    }

    state.topic_tracker.clear().await;

    let _ = state.status.send(BrokerStatus::Stopped);
    tracing::info!("MQTT broker stopped");

    Ok(())
}

// ---------------------------------------------------------------------------
// Topic tree helper
// ---------------------------------------------------------------------------

/// Build a hierarchical JSON tree from a flat list of topic paths.
pub fn build_topic_tree(topics: &[TopicInfo]) -> serde_json::Value {
    use serde_json::{Map, Value};

    let mut root = Map::new();

    for info in topics {
        let parts: Vec<&str> = info.topic.split('/').collect();
        let node = &mut root;

        for (i, part) in parts.iter().enumerate() {
            let is_leaf = i == parts.len() - 1;
            let children = node
                .entry(*part)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .unwrap();

            if is_leaf {
                children.insert("_leaf".to_string(), Value::Bool(true));
                children.insert(
                    "_message_count".to_string(),
                    Value::Number(info.message_count.into()),
                );
                children.insert(
                    "_last_update".to_string(),
                    Value::Number(info.last_update.into()),
                );
            }
        }
    }

    Value::Object(root)
}

/// Get the topic tree for the API layer.
pub async fn get_topic_tree(tracker: &TopicTracker) -> serde_json::Value {
    let topics = tracker.get_topics().await;
    let tree = build_topic_tree(&topics);
    serde_json::json!({
        "topics": topics,
        "tree": tree,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_config_default() {
        let config = MqttBrokerConfig {
            enabled: true,
            port: 1883,
            max_connections: 500,
            auth: MqttAuthConfig {
                mode: "none".to_string(),
                users: vec![],
            },
            ws_port: None,
            tls: MqttServerTlsConfig::default(),
        };
        let _rumqttd = build_rumqttd_config(&config, None);
    }

    #[test]
    fn test_build_config_with_auth_and_ws() {
        let config = MqttBrokerConfig {
            enabled: true,
            port: 1883,
            max_connections: 100,
            auth: MqttAuthConfig {
                mode: "password".to_string(),
                users: vec![MqttUser {
                    username: "admin".to_string(),
                    password: "secret".to_string(),
                }],
            },
            ws_port: Some(9001),
            tls: MqttServerTlsConfig::default(),
        };
        // Should not panic; auth map + WS listener are wired into rumqttd config.
        let rumqttd_cfg = build_rumqttd_config(&config, None);
        let v4 = rumqttd_cfg.v4.expect("v4 server present");
        let server = v4.get("v4-1").expect("v4-1 server");
        assert!(server.connections.auth.is_some(), "auth map wired");
        assert_eq!(
            server.connections.auth.as_ref().unwrap().get("admin"),
            Some(&"secret".to_string())
        );
        let ws = rumqttd_cfg.ws.expect("ws server present");
        assert!(ws.contains_key("ws-1"), "ws-1 server present");
    }

    #[test]
    fn test_auth_disabled() {
        let config = MqttAuthConfig {
            mode: "none".to_string(),
            users: vec![],
        };
        let result = validate_credentials(&config, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_auth_enabled_valid() {
        let config = MqttAuthConfig {
            mode: "password".to_string(),
            users: vec![MqttUser {
                username: "admin".to_string(),
                password: "secret".to_string(),
            }],
        };
        let result = validate_credentials(&config, Some("admin"), Some("secret"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_auth_enabled_invalid_password() {
        let config = MqttAuthConfig {
            mode: "password".to_string(),
            users: vec![MqttUser {
                username: "admin".to_string(),
                password: "secret".to_string(),
            }],
        };
        let result = validate_credentials(&config, Some("admin"), Some("wrong"));
        assert!(result.is_err());
    }

    /// Grab an ephemeral free TCP port (tiny TOCTOU window, fine for tests).
    async fn free_port() -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    }

    /// Run `start_broker` on a dedicated runtime thread and report whether it
    /// returned within `timeout`. The broker task keeps running (rumqttd joins
    /// its server threads forever), so this must live outside the test runtime
    /// which would otherwise wait on the blocking pool at teardown.
    fn start_broker_on_thread(
        port: u16,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("runtime");
            let state = MqttBrokerState::new();
            let config = MqttBrokerConfig {
                enabled: true,
                port,
                max_connections: 10,
                ..Default::default()
            };
            let result = rt.block_on(async {
                match tokio::time::timeout(timeout, start_broker(state.clone(), config, None)).await
                {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err("start_broker timed out".to_string()),
                }
            });
            let _ = tx.send(result);
            // Keep this thread's runtime alive so the broker keeps serving;
            // the test only cares that `start_broker` returned.
            std::thread::sleep(std::time::Duration::from_secs(60));
        });
        rx.recv_timeout(timeout + std::time::Duration::from_secs(2))
            .unwrap_or_else(|_| Err("start_broker did not report".to_string()))
    }

    #[test]
    fn test_broker_start_returns_promptly() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let port = rt.block_on(free_port());
        // The regression: start_broker used to block forever (the error await
        // never resolved while broker.start() ran). It must return Ok quickly.
        match start_broker_on_thread(port, std::time::Duration::from_secs(5)) {
            Ok(()) => {}
            Err(e) => panic!("start_broker failed or hung: {}", e),
        }
    }

    #[test]
    fn test_publish_sync_reaches_on_message_once() {
        let port = {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(free_port())
        };
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let (state_tx, state_rx) = std::sync::mpsc::sync_channel::<MqttBrokerState>(1);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("runtime");
            let state = MqttBrokerState::new();
            let config = MqttBrokerConfig { enabled: true, port, max_connections: 10, ..Default::default() };
            let tx = std::sync::Mutex::new(tx);
            let cb: Arc<dyn Fn(String, String, i64, bool) + Send + Sync + 'static> =
                Arc::new(move |topic, _payload, _ts, _retain| {
                    let _ = tx.lock().unwrap().send(topic);
                });
            rt.block_on(start_broker(state.clone(), config, Some(cb))).expect("start");
            state_tx.send(state).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(10));
        });
        let state = state_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("broker up");
        state.publish_sync("test/once", b"hello").expect("publish");
        let first = rx.recv_timeout(std::time::Duration::from_secs(3)).expect("callback fired");
        assert_eq!(first, "test/once");
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(800)).is_err(),
            "a single publish must reach the on_message callback exactly once"
        );
    }

    #[test]
    fn test_broker_start_port_conflict_reports_error() {
        // Occupy the port so the broker cannot bind to it. Bind 0.0.0.0 so the
        // broker's own wildcard bind fails (127.0.0.1 alone wouldn't block it).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let port = rt.block_on(free_port());
        let _listener = std::net::TcpListener::bind(("0.0.0.0", port))
            .expect("test listener bind");

        match start_broker_on_thread(port, std::time::Duration::from_secs(5)) {
            Err(e) => {
                assert!(
                    e.contains("already in use"),
                    "expected a port-in-use error, got: {}",
                    e
                );
            }
            Ok(()) => panic!("broker started on an occupied port"),
        }
    }
}
