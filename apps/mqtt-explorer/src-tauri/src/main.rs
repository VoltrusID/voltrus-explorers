mod system;

use std::collections::HashMap;
use std::sync::Arc;

use mt_mqtt::{
    BrokerStatus, DecodeKind, MqttBrokerConfig, MqttBrokerState, MqttClientConfig, MqttClientEvent,
    MqttMessage, MqttPubClient, MqttQos, PayloadDecoder, ProtoConfig, ProtoRule, RecentMessage,
    SharedProtoConfig, TopicInfo, start_broker, stop_broker,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::Emitter;
use tokio::sync::Mutex;

// ── Payload views ──

/// Fields every message event and topic row carries for its payload:
///
/// - `payload`: lossy UTF-8 text, what the Raw and JSON views show.
/// - `raw_hex`: the exact bytes, only when the payload is not valid UTF-8
///   (valid UTF-8 round-trips through `payload` unchanged).
/// - `decoded`: Sparkplug B / user-protobuf result, when the topic calls for it.
fn payload_fields(decoder: &mut PayloadDecoder, topic: &str, bytes: &[u8]) -> Map<String, Value> {
    let mut m = Map::new();
    match std::str::from_utf8(bytes) {
        Ok(s) => {
            m.insert("payload".into(), json!(s));
        }
        Err(_) => {
            m.insert("payload".into(), json!(String::from_utf8_lossy(bytes)));
            m.insert("raw_hex".into(), json!(hex_encode(bytes)));
        }
    }
    if let Some(d) = decoder.decode(topic, bytes) {
        m.insert("decoded".into(), d.to_json());
    }
    m
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ── Client: one background task per connection ──

/// Emit a received message to the UI, tagged with its connection id.
fn emit_message(
    app: &tauri::AppHandle,
    conn: &str,
    decoder: &mut PayloadDecoder,
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool,
) {
    let mut ev = payload_fields(decoder, topic, payload);
    ev.insert("conn".into(), json!(conn));
    ev.insert("topic".into(), json!(topic));
    ev.insert("qos".into(), json!(qos));
    ev.insert("retain".into(), json!(retain));
    ev.insert("timestamp".into(), json!(now_ms()));
    if let Err(e) = app.emit("client-message", Value::Object(ev)) {
        eprintln!("[mqtt-client] emit client-message failed: {}", e);
    }
}

enum ClientAction {
    Subscribe { topic: String, qos: MqttQos, reply: tokio::sync::oneshot::Sender<Result<u8, String>> },
    Unsubscribe { topic: String, reply: tokio::sync::oneshot::Sender<Result<(), String>> },
    Publish { message: MqttMessage, reply: tokio::sync::oneshot::Sender<Result<(), String>> },
    Disconnect { reply: tokio::sync::oneshot::Sender<Result<(), String>> },
}

/// Drop guard that ensures `client-disconnected` is emitted even if the
/// event-loop thread panics or the tokio runtime crashes.
struct DisconnectGuard {
    app: tauri::AppHandle,
    conn: String,
    did_disconnect: bool,
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        if !self.did_disconnect {
            eprintln!("[mqtt-client] Thread for {} exited unexpectedly, emitting client-disconnected", self.conn);
            let _ = self.app.emit(
                "client-disconnected",
                json!({ "conn": self.conn, "reason": "connection_lost" }),
            );
        }
    }
}

struct BrokerStateHolder {
    state: MqttBrokerState,
}

/// Latest decoded payload per topic seen by the embedded broker, so the topic
/// poll can return it alongside the payload.
type BrokerDecoded = Arc<parking_lot::Mutex<HashMap<String, Value>>>;

struct AppState {
    /// Command channel of every live client connection, by connection id.
    clients: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<ClientAction>>>>,
    broker: Arc<Mutex<Option<BrokerStateHolder>>>,
    broker_decoded: BrokerDecoded,
    /// User protobuf schema and topic rules, shared by every decoder.
    proto: SharedProtoConfig,
}

impl AppState {
    async fn client_tx(
        &self,
        conn: &str,
    ) -> Result<tokio::sync::mpsc::UnboundedSender<ClientAction>, String> {
        // Clone the sender and drop the lock before awaiting so concurrent
        // commands aren't serialized (and we never hold the mutex across `.await`).
        self.clients
            .lock()
            .await
            .get(conn)
            .cloned()
            .ok_or_else(|| "Not connected".to_string())
    }
}

// ── Client commands ──

#[tauri::command]
async fn client_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    conn: String,
    config: MqttClientConfig,
) -> Result<(), String> {
    // Gracefully stop any existing client task with this id first. Sending
    // Disconnect (and awaiting the reply) marks its guard as a clean
    // disconnect so it does not emit a stale `client-disconnected` that would
    // race our new connection.
    let old = state.clients.lock().await.remove(&conn);
    if let Some(old_tx) = old {
        let (r_tx, r_rx) = tokio::sync::oneshot::channel();
        let _ = old_tx.send(ClientAction::Disconnect { reply: r_tx });
        let _ = r_rx.await;
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ClientAction>();
    let mut decoder = PayloadDecoder::new(state.proto.clone());
    let app_clone = app.clone();
    let conn_id = conn.clone();
    // Signal the connect result back to this command so it can return a proper
    // error if the broker refuses / is unreachable.
    let (connect_tx, connect_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

    // Run the client on a dedicated OS thread with its OWN current-thread
    // tokio runtime. rumqttc's EventLoop is `!Send`, so it cannot live on a
    // multi-thread runtime's worker pool — and critically, the connection MUST
    // be created AND polled on the same runtime. The previous implementation
    // connected on Tauri's runtime then polled on a different one, so the TCP
    // socket was bound to the wrong reactor and no incoming Publishes were ever
    // delivered. Creating + polling inside this one thread fixes live delivery.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create MQTT client runtime");

        rt.block_on(async move {
            let mut client = match MqttPubClient::connect(config).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = connect_tx.send(Err(e));
                    return;
                }
            };
            // Connect succeeded — release the command.
            let _ = connect_tx.send(Ok(()));

            let mut guard = DisconnectGuard {
                app: app_clone.clone(),
                conn: conn_id.clone(),
                did_disconnect: false,
            };

            loop {
                tokio::select! {
                    biased;
                    // Drain actions with priority; commands never starve on traffic.
                    action = rx.recv() => {
                        let action = match action {
                            Some(a) => a,
                            // All senders dropped (client_disconnect took the tx).
                            None => break,
                        };
                        match action {
                            ClientAction::Subscribe { topic, qos, reply } => {
                                let outcome = client.subscribe_confirm(&topic, qos).await;
                                // Replay messages that arrived while waiting for
                                // the SUBACK so they are not lost.
                                for ev in outcome.buffered {
                                    if let MqttClientEvent::Message(m) = ev {
                                        emit_message(
                                            &app_clone, &conn_id, &mut decoder, &m.topic, &m.payload,
                                            m.qos as u8, m.retained,
                                        );
                                    }
                                }
                                let r = match outcome.error {
                                    Some(e) => Err(e),
                                    None => Ok(outcome.granted_qos.map(|q| q as u8).unwrap_or(0)),
                                };
                                let _ = reply.send(r);
                            }
                            ClientAction::Unsubscribe { topic, reply } => {
                                let r = client.unsubscribe(&topic).await.map_err(|e| e.to_string());
                                let _ = reply.send(r);
                            }
                            ClientAction::Publish { message, reply } => {
                                let r = client.publish(&message).await.map_err(|e| e.to_string());
                                let _ = reply.send(r);
                            }
                            ClientAction::Disconnect { reply } => {
                                let r = client.disconnect().await.map_err(|e| e.to_string());
                                let _ = reply.send(r);
                                guard.did_disconnect = true;
                                break;
                            }
                        }
                    }
                    // Receive published messages. next_event drives the eventloop,
                    // reconnects, and re-establishes subscriptions internally.
                    event = client.next_event() => match event {
                        Some(MqttClientEvent::Message(m)) => {
                            emit_message(
                                &app_clone, &conn_id, &mut decoder, &m.topic, &m.payload,
                                m.qos as u8, m.retained,
                            );
                        }
                        Some(MqttClientEvent::Rtt(ms)) => {
                            let _ = app_clone.emit("client-rtt", json!({ "conn": conn_id, "rtt": ms }));
                        }
                        None => break,
                    }
                }
            }
            // guard.drop() emits client-disconnected unless this was a clean one.
        });
    });

    // Wait for the connect attempt so we can surface broker errors to the UI.
    match connect_rx.await {
        Ok(Ok(())) => {
            state.clients.lock().await.insert(conn, tx);
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Client task died during connect".to_string()),
    }
}

#[tauri::command]
async fn client_subscribe(
    state: tauri::State<'_, AppState>,
    conn: String,
    topic: String,
    qos: MqttQos,
) -> Result<u8, String> {
    let tx = state.client_tx(&conn).await?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(ClientAction::Subscribe { topic, qos, reply: reply_tx })
        .map_err(|_| "Client disconnected")?;
    reply_rx.await.map_err(|_| "Client task died")?
}

#[tauri::command]
async fn client_unsubscribe(
    state: tauri::State<'_, AppState>,
    conn: String,
    topic: String,
) -> Result<(), String> {
    let tx = state.client_tx(&conn).await?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(ClientAction::Unsubscribe { topic, reply: reply_tx })
        .map_err(|_| "Client disconnected")?;
    reply_rx.await.map_err(|_| "Client task died")?
}

#[tauri::command]
async fn client_publish(
    state: tauri::State<'_, AppState>,
    conn: String,
    message: MqttMessage,
) -> Result<(), String> {
    let tx = state.client_tx(&conn).await?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(ClientAction::Publish { message, reply: reply_tx })
        .map_err(|_| "Client disconnected")?;
    reply_rx.await.map_err(|_| "Client task died")?
}

#[tauri::command]
async fn client_disconnect(state: tauri::State<'_, AppState>, conn: String) -> Result<(), String> {
    // Take the sender (releases the lock immediately) so the client task sees
    // all senders gone and exits without holding the mutex across `.await`.
    let tx = state.clients.lock().await.remove(&conn);
    if let Some(tx) = tx {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(ClientAction::Disconnect { reply: reply_tx })
            .map_err(|_| "Client disconnected")?;
        reply_rx.await.map_err(|_| "Client task died")??;
    }
    Ok(())
}

// ── Payload decoding commands ──

/// Install the user's `.proto` schema and the topic filters that use it.
/// A blank `source` clears both. Returns the message names the schema defines.
#[tauri::command]
fn proto_configure(
    state: tauri::State<'_, AppState>,
    source: String,
    rules: Vec<ProtoRule>,
) -> Result<Vec<String>, String> {
    state.proto.write().set(&source, rules)
}

/// Compile a schema without installing it, to list its message types.
#[tauri::command]
fn proto_inspect(source: String) -> Result<Vec<String>, String> {
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut scratch = ProtoConfig::default();
    scratch.set(&source, Vec::new())
}

#[derive(Deserialize)]
struct DecodeItem {
    id: u32,
    topic: String,
    /// Exact payload bytes, hex encoded.
    hex: String,
}

#[derive(Serialize)]
struct DecodeResult {
    id: u32,
    decoded: Option<Value>,
}

/// Decode already received messages again, for example after the schema
/// changed. Items are decoded in the order given with one fresh decoder, so
/// pass them oldest first (Sparkplug aliases come from earlier BIRTH messages).
#[tauri::command]
fn decode_payloads(state: tauri::State<'_, AppState>, items: Vec<DecodeItem>) -> Vec<DecodeResult> {
    let mut decoder = PayloadDecoder::new(state.proto.clone());
    items
        .into_iter()
        .map(|it| {
            let decoded = hex_decode(&it.hex)
                .and_then(|bytes| decoder.decode(&it.topic, &bytes))
                .map(|d| d.to_json());
            DecodeResult { id: it.id, decoded }
        })
        .collect()
}

// ── Broker commands ──

#[tauri::command]
async fn broker_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: MqttBrokerConfig,
) -> Result<(), String> {
    let broker_state = MqttBrokerState::new();
    state.broker_decoded.lock().clear();

    // Wire the local-link notification handler to emit Tauri events directly.
    // This is more reliable than a separate TCP client because the local link
    // receives ALL messages (including those published via LinkTx::publish).
    let app_emit = app.clone();
    let decoded_by_topic = state.broker_decoded.clone();
    let decoder = parking_lot::Mutex::new(PayloadDecoder::new(state.proto.clone()));
    let on_message: Arc<dyn Fn(String, Vec<u8>, i64, bool) + Send + Sync + 'static> =
        Arc::new(move |topic, payload, ts, retain| {
            let mut ev = payload_fields(&mut decoder.lock(), &topic, &payload);
            match ev.get("decoded") {
                Some(d) => {
                    decoded_by_topic.lock().insert(topic.clone(), d.clone());
                }
                // A later plain message on the topic replaces an older decode.
                None => {
                    decoded_by_topic.lock().remove(&topic);
                }
            }
            ev.insert("topic".into(), json!(topic));
            ev.insert("timestamp".into(), json!(ts));
            ev.insert("retain".into(), json!(retain));
            if let Err(e) = app_emit.emit("broker-message", Value::Object(ev)) {
                eprintln!("[broker-message] emit failed: {}", e);
            }
        });

    start_broker(broker_state.clone(), config.clone(), Some(on_message)).await?;

    {
        let mut broker = state.broker.lock().await;
        *broker = Some(BrokerStateHolder {
            state: broker_state,
        });
    }

    Ok(())
}

#[tauri::command]
async fn broker_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut broker = state.broker.lock().await;
    if let Some(holder) = broker.take() {
        stop_broker(holder.state).await?;
    }
    Ok(())
}

#[tauri::command]
async fn broker_get_status(state: tauri::State<'_, AppState>) -> Result<BrokerStatus, String> {
    let broker = state.broker.lock().await;
    match broker.as_ref() {
        Some(holder) => Ok(holder.state.current_status().await),
        None => Ok(BrokerStatus::Stopped),
    }
}

#[tauri::command]
async fn broker_get_topics(state: tauri::State<'_, AppState>) -> Result<Vec<Value>, String> {
    let topics: Vec<TopicInfo> = {
        let broker = state.broker.lock().await;
        match broker.as_ref() {
            Some(holder) => holder.state.topic_tracker.get_topics().await,
            None => return Ok(vec![]),
        }
    };
    let decoded = state.broker_decoded.lock();
    Ok(topics
        .into_iter()
        .map(|t| {
            let mut m = Map::new();
            match std::str::from_utf8(&t.last_payload) {
                Ok(s) => {
                    m.insert("last_payload".into(), json!(s));
                }
                Err(_) => {
                    m.insert("last_payload".into(), json!(String::from_utf8_lossy(&t.last_payload)));
                    m.insert("raw_hex".into(), json!(hex_encode(&t.last_payload)));
                }
            }
            if let Some(d) = decoded.get(&t.topic) {
                m.insert("decoded".into(), d.clone());
            }
            m.insert("topic".into(), json!(t.topic));
            m.insert("last_update".into(), json!(t.last_update));
            m.insert("message_count".into(), json!(t.message_count));
            m.insert("retain".into(), json!(t.retain));
            Value::Object(m)
        })
        .collect())
}

#[tauri::command]
async fn broker_get_recent(
    state: tauri::State<'_, AppState>,
    limit: usize,
) -> Result<Vec<RecentMessage>, String> {
    let broker = state.broker.lock().await;
    match broker.as_ref() {
        Some(holder) => Ok(holder.state.topic_tracker.get_recent_messages(limit).await),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn broker_publish(
    state: tauri::State<'_, AppState>,
    topic: String,
    payload: String,
) -> Result<(), String> {
    // The broker's local link is subscribed to "#", so this publish comes back through the
    // `broker-message` handler wired in `broker_start`; emitting here too would show it twice.
    let broker = state.broker.lock().await;
    match broker.as_ref() {
        Some(holder) => holder.state.publish_sync(&topic, payload.as_bytes()),
        None => Err("Broker not running".to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            clients: Arc::new(Mutex::new(HashMap::new())),
            broker: Arc::new(Mutex::new(None)),
            broker_decoded: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            proto: SharedProtoConfig::default(),
        })
        .invoke_handler(tauri::generate_handler![
            client_connect, client_subscribe, client_unsubscribe, client_publish, client_disconnect,
            proto_configure, proto_inspect, decode_payloads,
            broker_start, broker_stop, broker_get_status, broker_get_topics, broker_get_recent, broker_publish,
            system::open_url,
            mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoder() -> PayloadDecoder {
        PayloadDecoder::new(SharedProtoConfig::default())
    }

    #[test]
    fn utf8_payload_has_no_raw_hex() {
        let f = payload_fields(&mut decoder(), "a/b", "héllo".as_bytes());
        assert_eq!(f["payload"], "héllo");
        assert!(!f.contains_key("raw_hex"));
        assert!(!f.contains_key("decoded"));
    }

    #[test]
    fn binary_payload_keeps_exact_bytes() {
        let f = payload_fields(&mut decoder(), "a/b", &[0x00, 0xff, 0xfe, 0x41]);
        assert_eq!(f["raw_hex"], "00fffe41");
        assert_eq!(hex_decode("00fffe41").unwrap(), vec![0x00, 0xff, 0xfe, 0x41]);
    }

    #[test]
    fn sparkplug_topic_is_decoded() {
        // Payload { seq: 5 }
        let f = payload_fields(&mut decoder(), "spBv1.0/g/NDATA/e", &[0x18, 0x05]);
        assert_eq!(f["decoded"]["kind"], "sparkplug");
        assert_eq!(f["decoded"]["value"]["seq"], 5);
    }

    #[test]
    fn hex_decode_rejects_bad_input() {
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        assert!(hex_decode("é1").is_none());
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn decode_kind_names_are_stable() {
        assert_eq!(DecodeKind::Sparkplug.as_str(), "sparkplug");
        assert_eq!(DecodeKind::Protobuf.as_str(), "protobuf");
    }
}
