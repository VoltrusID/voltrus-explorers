//! NATS Explorer Tauri backend.
//!
//! Thin bridge between the React frontend and [`mt_nats`]. Follows the
//! `mqtt-explorer` pattern: client connection lives in managed state, message
//! streams are forwarded to the webview via Tauri events, and the embedded
//! server runs in a background task.

mod state;
mod system;

use std::sync::Arc;

use futures_util::StreamExt;
use mt_nats::{
    detect, encode_payload, hex_dump, msgpack_to_json, pretty_json, proto_decode, NatsClient,
    NatsClientConfig, NatsConsumerInfo, NatsKvBucketInfo, NatsKvEntry, NatsMessage, NatsObjectInfo,
    NatsServerConfig, NatsServerState, NatsStoredMessage, NatsStreamInfo, PayloadFormat,
    ServerStatus,
};
use tauri::{AppHandle, Emitter, State};

use crate::state::{ActiveSubscription, AppState};

// ---------------------------------------------------------------------------
// Client mode commands
// ---------------------------------------------------------------------------

const MAX_RECENT: usize = 200;

#[tauri::command]
async fn client_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    config: NatsClientConfig,
) -> Result<(), String> {
    if state.client.client.lock().await.is_some() {
        return Err("Already connected".to_string());
    }
    let client = NatsClient::connect(&config).await?;
    *state.client.client.lock().await = Some(client.clone());
    *state.client.recent.lock().await = Vec::new();
    let _ = app.emit("client-status", serde_json::json!({ "connected": true }));
    Ok(())
}

#[tauri::command]
async fn client_disconnect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Abort all subscription tasks.
    let mut subs = state.client.subs.lock().await;
    for (_k, sub) in subs.drain() {
        sub.handle.abort();
    }
    drop(subs);

    let client = state.client.client.lock().await.take();
    if let Some(c) = client {
        // Closing the client is async-nats; there is no explicit close() in
        // async-nats 0.50 — dropping the last clone closes the connection.
        drop(c);
    }
    let _ = app.emit("client-disconnected", serde_json::json!({ "reason": "disconnected" }));
    Ok(())
}

#[tauri::command]
async fn client_subscribe(
    app: AppHandle,
    state: State<'_, AppState>,
    subject: String,
    queue: Option<String>,
) -> Result<String, String> {
    let client = state
        .client
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Not connected".to_string())?;

    let sub = client.subscribe(&subject, queue.clone()).await?;
    let sid = sub.sid().to_string();
    let recent = state.client.recent.clone();
    let app2 = app.clone();

    let handle = tokio::spawn(async move {
        let mut sub = sub;
        while let Some(msg) = sub.next().await {
            let mut msg = msg;
            msg.queue = queue.clone();
            {
                let mut r = recent.lock().await;
                if r.len() >= MAX_RECENT {
                    r.remove(0);
                }
                r.push(msg.clone());
            }
            let _ = app2.emit("client-message", msg);
        }
    });

    state.client.subs.lock().await.insert(
        sid.clone(),
        ActiveSubscription { handle },
    );
    Ok(sid)
}

#[tauri::command]
async fn client_unsubscribe(state: State<'_, AppState>, sid: String) -> Result<(), String> {
    if let Some(sub) = state.client.subs.lock().await.remove(&sid) {
        sub.handle.abort();
    }
    Ok(())
}

#[tauri::command]
async fn client_publish(state: State<'_, AppState>, message: NatsMessage) -> Result<(), String> {
    let client = state
        .client
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Not connected".to_string())?;
    client.publish_message(&message).await
}

#[tauri::command]
async fn client_request(
    state: State<'_, AppState>,
    subject: String,
    payload: String,
    timeout_ms: u64,
) -> Result<NatsMessage, String> {
    let client = state
        .client
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Not connected".to_string())?;
    let payload_bytes = mt_nats::decode_payload(&payload);
    client.request(&subject, &payload_bytes, timeout_ms).await
}

#[tauri::command]
async fn client_get_recent(state: State<'_, AppState>) -> Result<Vec<NatsMessage>, String> {
    Ok(state.client.recent.lock().await.clone())
}

#[tauri::command]
async fn client_status(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.client.client.lock().await.is_some())
}

// ---------------------------------------------------------------------------
// JetStream commands
// ---------------------------------------------------------------------------

fn js_client(state: &State<'_, AppState>) -> Result<NatsClient, String> {
    state
        .client
        .client
        .blocking_lock()
        .clone()
        .ok_or_else(|| "Not connected".to_string())
}

#[tauri::command]
async fn js_list_streams(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let c = js_client(&state)?;
    c.js_list_streams().await
}

#[tauri::command]
async fn js_stream_info(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<NatsStreamInfo>, String> {
    let c = js_client(&state)?;
    c.js_stream_info(&name).await
}

#[tauri::command]
async fn js_create_stream(
    state: State<'_, AppState>,
    name: String,
    subjects: Vec<String>,
    max_messages: i64,
    max_age_ns: i64,
) -> Result<NatsStreamInfo, String> {
    let c = js_client(&state)?;
    c.js_create_stream(&name, &subjects, max_messages, max_age_ns).await
}

#[tauri::command]
async fn js_delete_stream(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let c = js_client(&state)?;
    c.js_delete_stream(&name).await
}

#[tauri::command]
async fn js_purge_stream(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let c = js_client(&state)?;
    c.js_purge_stream(&name).await
}

#[tauri::command]
async fn js_stream_messages(
    state: State<'_, AppState>,
    name: String,
    limit: u64,
) -> Result<Vec<NatsStoredMessage>, String> {
    let c = js_client(&state)?;
    c.js_stream_messages(&name, limit).await
}

#[tauri::command]
async fn js_create_consumer(
    state: State<'_, AppState>,
    stream: String,
    durable: String,
) -> Result<NatsConsumerInfo, String> {
    let c = js_client(&state)?;
    c.js_create_consumer(&stream, &durable).await
}

#[tauri::command]
async fn js_delete_consumer(
    state: State<'_, AppState>,
    stream: String,
    durable: String,
) -> Result<(), String> {
    let c = js_client(&state)?;
    c.js_delete_consumer(&stream, &durable).await
}

#[tauri::command]
async fn js_consumer_info(
    state: State<'_, AppState>,
    stream: String,
    durable: String,
) -> Result<NatsConsumerInfo, String> {
    let c = js_client(&state)?;
    c.js_consumer_info(&stream, &durable).await
}

#[tauri::command]
async fn js_consumer_next(
    state: State<'_, AppState>,
    stream: String,
    durable: String,
    batch: u64,
    wait_ms: u64,
) -> Result<Vec<NatsStoredMessage>, String> {
    let c = js_client(&state)?;
    c.js_consumer_next(&stream, &durable, batch, wait_ms).await
}

// ---------------------------------------------------------------------------
// Server mode commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn server_start(
    app: AppHandle,
    state: State<'_, AppState>,
    config: NatsServerConfig,
) -> Result<(), String> {
    if state.server.server.lock().await.is_some() {
        return Err("Server is already running".to_string());
    }

    let server_state = NatsServerState::new(config.clone());
    let cb: mt_nats::OnMessage = Arc::new(move |subject, payload, ts, headers| {
        let _ = app.emit(
            "server-message",
            NatsMessage {
                subject,
                reply: None,
                headers,
                payload: mt_nats::encode_payload(&payload),
                timestamp_ms: ts,
                queue: None,
            },
        );
    });
    mt_nats::start_server(server_state.clone(), Some(cb)).await?;

    *state.server.server.lock().await = Some(server_state);
    Ok(())
}

#[tauri::command]
async fn server_stop(state: State<'_, AppState>) -> Result<(), String> {
    let server = state.server.server.lock().await.take();
    if let Some(s) = server {
        mt_nats::stop_server(&s).await?;
    }
    Ok(())
}

#[tauri::command]
async fn server_get_status(state: State<'_, AppState>) -> Result<ServerStatus, String> {
    let server = state.server.server.lock().await.clone();
    match server {
        Some(s) => Ok(s.current_status().await),
        None => Ok(ServerStatus::Stopped),
    }
}

#[tauri::command]
async fn server_get_subjects(
    state: State<'_, AppState>,
) -> Result<Vec<mt_nats::SubjectInfo>, String> {
    let server = state.server.server.lock().await.clone();
    match server {
        Some(s) => Ok(s.tracker.get_subjects().await),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
async fn server_get_recent(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<mt_nats::ServerRecentMessage>, String> {
    let server = state.server.server.lock().await.clone();
    match server {
        Some(s) => Ok(s.tracker.get_recent(limit).await),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
async fn server_publish(
    state: State<'_, AppState>,
    subject: String,
    payload: String,
    headers: Vec<(String, String)>,
) -> Result<(), String> {
    let server = state.server.server.lock().await.clone();
    match server {
        Some(s) => {
            let payload = mt_nats::decode_payload(&payload);
            // Route via the server's router so local subscribers get it, then
            // re-emit the callback so the app's own UI sees it too (matches
            // broker_publish behavior in mqtt-explorer).
            s.publish(subject.clone(), None, headers.clone(), payload.clone()).await?;
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if let Some(cb) = s.on_message().await.as_ref() {
                cb(subject, payload, ts, headers);
            }
            Ok(())
        }
        None => Err("Server is not running".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Payload decode + schema registry
// ---------------------------------------------------------------------------

/// Auto-detect a payload's format. `payload` is an encoded payload string
/// (UTF-8 or `0x`-prefixed hex), decoded before detection.
#[tauri::command]
async fn decode_detect(payload: String) -> Result<PayloadFormat, String> {
    let bytes = mt_nats::decode_payload(&payload);
    Ok(detect(&bytes))
}

/// Decode a payload to a display string in the requested format.
#[tauri::command]
async fn decode_format(
    payload: String,
    format: String,
    message_full_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let bytes = mt_nats::decode_payload(&payload);
    match format.as_str() {
        "json" => pretty_json(&bytes),
        "msgpack" => msgpack_to_json(&bytes),
        "hex" => Ok(hex_dump(&bytes)),
        "utf8" => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        "protobuf" => {
            let name = message_full_name
                .ok_or("message_full_name is required for protobuf decode")?;
            let pool = state.schema.lock().await;
            proto_decode(&bytes, &pool, &name)
        }
        other => Err(format!("unknown format '{}'", other)),
    }
}

/// Decode a payload using the schema binding for `subject` (wildcard match).
#[tauri::command]
async fn schema_decode(
    subject: String,
    payload: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let bytes = mt_nats::decode_payload(&payload);
    let pool = state.schema.lock().await;
    let msg = pool.decode_for_subject(&subject, &bytes)?;
    serde_json::to_string_pretty(&msg).map_err(|e| e.to_string())
}

#[tauri::command]
async fn schema_add_proto_source(src: String, state: State<'_, AppState>) -> Result<(), String> {
    state.schema.lock().await.add_proto_source(&src)
}

#[tauri::command]
async fn schema_add_proto_file(path: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .schema
        .lock()
        .await
        .add_proto_file(std::path::Path::new(&path))
}

#[tauri::command]
async fn schema_bind(
    pattern: String,
    message_full_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.schema.lock().await.bind(pattern, message_full_name);
    Ok(())
}

#[tauri::command]
async fn schema_unbind(pattern: String, state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.schema.lock().await.unbind(&pattern))
}

#[tauri::command]
async fn schema_bindings(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(state.schema.lock().await.bindings().clone())
}

#[tauri::command]
async fn schema_message_names(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.schema.lock().await.message_names())
}

// ---------------------------------------------------------------------------
// KV store
// ---------------------------------------------------------------------------

#[tauri::command]
async fn kv_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    js_client(&state)?.kv_list().await
}

#[tauri::command]
async fn kv_create(
    state: State<'_, AppState>,
    bucket: String,
    history: i64,
    max_age_ns: i64,
) -> Result<(), String> {
    js_client(&state)?.kv_create(&bucket, history, max_age_ns).await
}

#[tauri::command]
async fn kv_delete_bucket(state: State<'_, AppState>, bucket: String) -> Result<(), String> {
    js_client(&state)?.kv_delete_bucket(&bucket).await
}

#[tauri::command]
async fn kv_status(state: State<'_, AppState>, bucket: String) -> Result<NatsKvBucketInfo, String> {
    js_client(&state)?.kv_status(&bucket).await
}

#[tauri::command]
async fn kv_keys(state: State<'_, AppState>, bucket: String) -> Result<Vec<String>, String> {
    js_client(&state)?.kv_keys(&bucket).await
}

#[tauri::command]
async fn kv_get(
    state: State<'_, AppState>,
    bucket: String,
    key: String,
) -> Result<Option<NatsKvEntry>, String> {
    js_client(&state)?.kv_get(&bucket, &key).await
}

#[tauri::command]
async fn kv_put(
    state: State<'_, AppState>,
    bucket: String,
    key: String,
    payload: String,
) -> Result<u64, String> {
    let bytes = mt_nats::decode_payload(&payload);
    js_client(&state)?.kv_put(&bucket, &key, &bytes).await
}

#[tauri::command]
async fn kv_delete_key(state: State<'_, AppState>, bucket: String, key: String) -> Result<(), String> {
    js_client(&state)?.kv_delete_key(&bucket, &key).await
}

#[tauri::command]
async fn kv_history(
    state: State<'_, AppState>,
    bucket: String,
    key: String,
) -> Result<Vec<NatsKvEntry>, String> {
    js_client(&state)?.kv_history(&bucket, &key).await
}

/// Subscribe to a KV bucket (all keys, or one key) for live updates. Emits a
/// `kv-update` event per entry. Reuse `client_unsubscribe` with the returned
/// watch id to stop.
#[tauri::command]
async fn kv_watch_subscribe(
    app: AppHandle,
    state: State<'_, AppState>,
    bucket: String,
    key: Option<String>,
) -> Result<String, String> {
    let c = js_client(&state)?;
    let watch = c.kv_open_watch(&bucket, key.as_deref()).await?;
    let wid = format!("kv:{}:{}", bucket, key.as_deref().unwrap_or("*"));
    let subs = state.client.subs.clone();
    let app2 = app.clone();
    let bucket2 = bucket.clone();
    let handle = tokio::spawn(async move {
        let mut watch = watch;
        while let Some(res) = watch.next().await {
            match res {
                Ok(e) => {
                    let _ = app2.emit("kv-update", mt_nats::client::kv_entry_from(&bucket2, e));
                }
                Err(_) => break,
            }
        }
    });
    subs.lock().await.insert(wid.clone(), ActiveSubscription { handle });
    Ok(wid)
}

// ---------------------------------------------------------------------------
// Object Store
// ---------------------------------------------------------------------------

#[tauri::command]
async fn obj_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    js_client(&state)?.obj_list().await
}

#[tauri::command]
async fn obj_create(state: State<'_, AppState>, bucket: String) -> Result<(), String> {
    js_client(&state)?.obj_create(&bucket).await
}

#[tauri::command]
async fn obj_delete_bucket(state: State<'_, AppState>, bucket: String) -> Result<(), String> {
    js_client(&state)?.obj_delete_bucket(&bucket).await
}

#[tauri::command]
async fn obj_list_objects(
    state: State<'_, AppState>,
    bucket: String,
) -> Result<Vec<NatsObjectInfo>, String> {
    js_client(&state)?.obj_list_objects(&bucket).await
}

#[tauri::command]
async fn obj_info(
    state: State<'_, AppState>,
    bucket: String,
    name: String,
) -> Result<NatsObjectInfo, String> {
    js_client(&state)?.obj_info(&bucket, &name).await
}

/// Get an object's bytes as an encoded payload string (UTF-8 or `0x` hex),
/// for inline preview.
#[tauri::command]
async fn obj_get(
    state: State<'_, AppState>,
    bucket: String,
    name: String,
) -> Result<String, String> {
    let bytes = js_client(&state)?.obj_get(&bucket, &name).await?;
    Ok(encode_payload(&bytes))
}

/// Put an object from an inline payload string (textarea).
#[tauri::command]
async fn obj_put_payload(
    state: State<'_, AppState>,
    bucket: String,
    name: String,
    payload: String,
) -> Result<(), String> {
    let bytes = mt_nats::decode_payload(&payload);
    js_client(&state)?.obj_put(&bucket, &name, &bytes).await
}

/// Upload a local file as an object.
#[tauri::command]
async fn obj_upload(
    state: State<'_, AppState>,
    bucket: String,
    name: String,
    path: String,
) -> Result<(), String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("Read file failed: {}", e))?;
    js_client(&state)?.obj_put(&bucket, &name, &bytes).await
}

/// Download an object to a local file.
#[tauri::command]
async fn obj_download(
    state: State<'_, AppState>,
    bucket: String,
    name: String,
    path: String,
) -> Result<(), String> {
    let bytes = js_client(&state)?.obj_get(&bucket, &name).await?;
    std::fs::write(&path, &bytes).map_err(|e| format!("Write file failed: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn obj_delete(state: State<'_, AppState>, bucket: String, name: String) -> Result<(), String> {
    js_client(&state)?.obj_delete(&bucket, &name).await
}

// ---------------------------------------------------------------------------
// $SYS monitoring
// ---------------------------------------------------------------------------

/// Subscribe to a `$SYS.*` subject for live server/account events. Emits a
/// `sys-message` event per message: `{ subject, payload, timestamp_ms }`.
/// Reuse `client_unsubscribe` with the returned sid to stop.
#[tauri::command]
async fn sys_subscribe(
    app: AppHandle,
    state: State<'_, AppState>,
    subject: String,
) -> Result<String, String> {
    let client = state
        .client
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Not connected".to_string())?;
    let sub = client.subscribe(&subject, None).await?;
    let sid = sub.sid().to_string();
    let app2 = app.clone();
    let handle = tokio::spawn(async move {
        let mut sub = sub;
        while let Some(msg) = sub.next().await {
            let _ = app2.emit(
                "sys-message",
                serde_json::json!({
                    "subject": msg.subject,
                    "payload": msg.payload,
                    "timestamp_ms": msg.timestamp_ms,
                }),
            );
        }
    });
    state
        .client
        .subs
        .lock()
        .await
        .insert(sid.clone(), ActiveSubscription { handle });
    Ok(sid)
}

/// Send a `$SYS.REQ.*` request/reply (e.g. `...PING.STATSZ`) and return the
/// JSON payload string.
#[tauri::command]
async fn sys_request(
    state: State<'_, AppState>,
    subject: String,
    timeout_ms: u64,
) -> Result<String, String> {
    let c = js_client(&state)?;
    let msg = c.request(&subject, &[], timeout_ms).await?;
    Ok(msg.payload)
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            client_connect,
            client_disconnect,
            client_subscribe,
            client_unsubscribe,
            client_publish,
            client_request,
            client_get_recent,
            client_status,
            js_list_streams,
            js_stream_info,
            js_create_stream,
            js_delete_stream,
            js_purge_stream,
            js_stream_messages,
            js_create_consumer,
            js_delete_consumer,
            js_consumer_info,
            js_consumer_next,
            server_start,
            server_stop,
            server_get_status,
            server_get_subjects,
            server_get_recent,
            server_publish,
            decode_detect,
            decode_format,
            schema_decode,
            schema_add_proto_source,
            schema_add_proto_file,
            schema_bind,
            schema_unbind,
            schema_bindings,
            schema_message_names,
            kv_list,
            kv_create,
            kv_delete_bucket,
            kv_status,
            kv_keys,
            kv_get,
            kv_put,
            kv_delete_key,
            kv_history,
            kv_watch_subscribe,
            obj_list,
            obj_create,
            obj_delete_bucket,
            obj_list_objects,
            obj_info,
            obj_get,
            obj_put_payload,
            obj_upload,
            obj_download,
            obj_delete,
            sys_subscribe,
            sys_request,
            system::open_url,
            mt_store::store_status,
            mt_store::store_purchase,
            mt_store::store_restore,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
