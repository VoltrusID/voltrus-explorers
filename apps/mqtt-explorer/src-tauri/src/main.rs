mod system;

use std::sync::Arc;
use mt_mqtt::{
    MqttPubClient, MqttBrokerState, MqttBrokerConfig, MqttClientConfig, MqttClientEvent, MqttMessage,
    MqttQos, BrokerStatus, TopicInfo, RecentMessage,
    start_broker, stop_broker,
};
use tokio::sync::Mutex;
use tauri::Emitter;

// ── Client: use channels to communicate with a background task ──

struct ClientMessage {
    topic: String,
    payload: String,
    timestamp: i64,
    retain: bool,
    qos: u8,
}

/// Push a received message into the recent buffer and emit it to the UI.
/// Shared by the live event path and the SUBACK-wait replay path.
async fn store_and_emit(
    app: &tauri::AppHandle,
    recent: &Arc<Mutex<Vec<ClientMessage>>>,
    topic: &str,
    payload: &str,
    qos: u8,
    retain: bool,
) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    {
        let mut msgs = recent.lock().await;
        msgs.push(ClientMessage {
            topic: topic.to_string(),
            payload: payload.to_string(),
            timestamp: ts,
            retain,
            qos,
        });
        if msgs.len() > 500 {
            let excess = msgs.len() - 500;
            msgs.drain(0..excess);
        }
    }
    if let Err(e) = app.emit(
        "client-message",
        serde_json::json!({
            "topic": topic,
            "payload": payload,
            "qos": qos,
            "retain": retain,
            "timestamp": ts,
        }),
    ) {
        eprintln!("[mqtt-client] emit client-message failed: {}", e);
    }
}

struct ClientState {
    publish_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<ClientAction>>>>,
    recent_messages: Arc<Mutex<Vec<ClientMessage>>>,
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
    did_disconnect: bool,
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        if !self.did_disconnect {
            eprintln!("[mqtt-client] Thread exited unexpectedly, emitting client-disconnected");
            let _ = self.app.emit(
                "client-disconnected",
                serde_json::json!({"reason": "connection_lost"}),
            );
        }
    }
}

struct BrokerStateHolder {
    state: MqttBrokerState,
}

struct AppState {
    client: ClientState,
    broker: Arc<Mutex<Option<BrokerStateHolder>>>,
}

// ── Client commands ──

#[tauri::command]
async fn client_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: MqttClientConfig,
) -> Result<(), String> {
    // Gracefully stop any existing client task first. Sending Disconnect (and
    // awaiting the reply) marks its guard as a clean disconnect so it does not
    // emit a stale `client-disconnected` that would race our new connection.
    {
        let mut tx_guard = state.client.publish_tx.lock().await;
        if let Some(old_tx) = tx_guard.take() {
            let (r_tx, r_rx) = tokio::sync::oneshot::channel();
            let _ = old_tx.send(ClientAction::Disconnect { reply: r_tx });
            let _ = r_rx.await;
        }
    }

    // Clear previous client messages.
    state.client.recent_messages.lock().await.clear();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ClientAction>();
    *state.client.publish_tx.lock().await = Some(tx);

    let recent = state.client.recent_messages.clone();
    let app_clone = app.clone();
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
                                        let payload_str = String::from_utf8_lossy(&m.payload).to_string();
                                        store_and_emit(
                                            &app_clone, &recent, &m.topic, &payload_str,
                                            m.qos as u8, m.retained,
                                        )
                                        .await;
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
                            let payload_str = String::from_utf8_lossy(&m.payload).to_string();
                            store_and_emit(
                                &app_clone, &recent, &m.topic, &payload_str,
                                m.qos as u8, m.retained,
                            )
                            .await;
                        }
                        Some(MqttClientEvent::Rtt(ms)) => {
                            let _ = app_clone.emit("client-rtt", serde_json::json!({ "rtt": ms }));
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
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Client task died during connect".to_string()),
    }
}

#[tauri::command]
async fn client_subscribe(
    state: tauri::State<'_, AppState>,
    topic: String,
    qos: MqttQos,
) -> Result<u8, String> {
    // Clone the sender and drop the lock before awaiting so concurrent
    // commands aren't serialized (and we never hold the mutex across `.await`).
    let tx = {
        let guard = state.client.publish_tx.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(ClientAction::Subscribe { topic, qos, reply: reply_tx })
        .map_err(|_| "Client disconnected")?;
    reply_rx.await.map_err(|_| "Client task died")?
}

#[tauri::command]
async fn client_unsubscribe(
    state: tauri::State<'_, AppState>,
    topic: String,
) -> Result<(), String> {
    let tx = {
        let guard = state.client.publish_tx.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(ClientAction::Unsubscribe { topic, reply: reply_tx })
        .map_err(|_| "Client disconnected")?;
    reply_rx.await.map_err(|_| "Client task died")?
}

#[tauri::command]
async fn client_publish(
    state: tauri::State<'_, AppState>,
    message: MqttMessage,
) -> Result<(), String> {
    let tx = {
        let guard = state.client.publish_tx.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(ClientAction::Publish { message, reply: reply_tx })
        .map_err(|_| "Client disconnected")?;
    reply_rx.await.map_err(|_| "Client task died")?
}

#[tauri::command]
async fn client_get_recent(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let msgs = state.client.recent_messages.lock().await;
    Ok(msgs.iter().map(|m| serde_json::json!({
        "topic": m.topic,
        "payload": m.payload,
        "timestamp": m.timestamp,
        "retain": m.retain,
        "qos": m.qos,
    })).collect())
}

#[tauri::command]
async fn client_disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Take the sender (releases the lock immediately) so the client task sees
    // all senders gone and exits without holding the mutex across `.await`.
    let tx = state.client.publish_tx.lock().await.take();
    if let Some(tx) = tx {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(ClientAction::Disconnect { reply: reply_tx })
            .map_err(|_| "Client disconnected")?;
        reply_rx.await.map_err(|_| "Client task died")??;
    }
    Ok(())
}

// ── Broker commands ──

#[tauri::command]
async fn broker_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: MqttBrokerConfig,
) -> Result<(), String> {
    let broker_state = MqttBrokerState::new();

    // Wire the local-link notification handler to emit Tauri events directly.
    // This is more reliable than a separate TCP client because the local link
    // receives ALL messages (including those published via LinkTx::publish).
    let app_emit = app.clone();
    let on_message: Arc<dyn Fn(String, String, i64, bool) + Send + Sync + 'static> =
        Arc::new(move |topic, payload, ts, retain| {
            if let Err(e) = app_emit.emit("broker-message", serde_json::json!({
                "topic": topic,
                "payload": payload,
                "timestamp": ts,
                "retain": retain,
            })) {
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
async fn broker_get_topics(state: tauri::State<'_, AppState>) -> Result<Vec<TopicInfo>, String> {
    let broker = state.broker.lock().await;
    match broker.as_ref() {
        Some(holder) => Ok(holder.state.topic_tracker.get_topics().await),
        None => Ok(vec![]),
    }
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
            client: ClientState {
                publish_tx: Arc::new(Mutex::new(None)),
                recent_messages: Arc::new(Mutex::new(Vec::new())),
            },
            broker: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            client_connect, client_subscribe, client_unsubscribe, client_publish, client_disconnect,
            client_get_recent,
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
