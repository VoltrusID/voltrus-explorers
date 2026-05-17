use crate::state::AppState;
use futures::{SinkExt, StreamExt};
use mt_plc_types::{serialize_bytecode, IoMapping};
use tauri::Emitter;
use tokio_tungstenite::tungstenite::Message;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct TargetStatusResult {
    pub connected: bool,
    pub ip: Option<String>,
    pub runtime_state: Option<String>,
    pub cycle_count: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DeployResult {
    pub success: bool,
    pub message: String,
    pub byte_count: Option<usize>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ScadaRegistration {
    pub scada_url: String,
    pub device_name: String,
    pub device_url: String,
    pub transport: String,
    pub interval_ms: u64,
}

/// Deploy compiled bytecode to a remote ESP32 target via HTTP POST.
#[tauri::command]
pub async fn deploy_to_target(
    state: tauri::State<'_, AppState>,
    ip: String,
    io_mappings: Option<Vec<IoMapping>>,
) -> Result<DeployResult, String> {
    let module = {
        let compiler = state.compiler.lock().await;
        compiler
            .last_module
            .clone()
            .ok_or("No compiled program. Click Build first.")?
    };

    let mappings = io_mappings.unwrap_or_default();
    let bytecode = serialize_bytecode(&module, &mappings)
        .map_err(|e| format!("Bytecode serialization failed: {e}"))?;

    let byte_count = bytecode.len();
    let url = format!("http://{ip}/api/deploy");

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/octet-stream")
        .body(bytecode)
        .send()
        .await
        .map_err(|e| format!("Deploy failed: {e}"))?;

    if resp.status().is_success() {
        // Store the IP as current target
        {
            let mut target_ip = state.target.ip.lock().await;
            *target_ip = Some(ip);
        }
        state
            .target
            .connected
            .store(true, std::sync::atomic::Ordering::SeqCst);

        Ok(DeployResult {
            success: true,
            message: "Deployed successfully".into(),
            byte_count: Some(byte_count),
        })
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Deploy failed (HTTP {status}): {body}"))
    }
}

/// Connect to ESP32 WebSocket for live variable streaming.
/// Spawns a background reader task that emits `watch-update` events.
#[tauri::command]
pub async fn target_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    ip: String,
) -> Result<(), String> {
    let url = format!("ws://{ip}/ws/plc");

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("WebSocket connection failed: {e}"))?;

    // Store IP
    {
        let mut target_ip = state.target.ip.lock().await;
        *target_ip = Some(ip.clone());
    }
    state
        .target
        .connected
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let connected = state.target.connected.clone();

    // Background reader: parse ESP32 binary frames and emit Tauri events
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(msg) = ws.next().await {
            if !connected.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            match msg {
                Ok(Message::Binary(data)) => {
                    if data.len() < 1 {
                        continue;
                    }
                    let msg_type = data[0];

                    match msg_type {
                        0x01 => {
                            // VAR_UPDATE: parse variable snapshot
                            if let Ok(vars) = parse_var_update(&data[1..]) {
                                let _ = app_handle.emit("watch-update", &vars);
                            }
                        }
                        0x07 => {
                            // STATUS
                            if let Ok(status) = parse_status_msg(&data[1..]) {
                                let _ = app_handle.emit("runtime-state", &status);
                            }
                        }
                        0x08 => {
                            // ERROR
                            if let Ok(error_msg) = parse_error_msg(&data[1..]) {
                                let _ = app_handle.emit(
                                    "runtime-state",
                                    &serde_json::json!({
                                        "state": "error",
                                        "error_message": error_msg,
                                    }),
                                );
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Message::Close(_)) => {
                    connected.store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                Err(_) => {
                    connected.store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                _ => {}
            }
        }

        let _ = app_handle.emit(
            "target-disconnected",
            serde_json::json!({"ip": ip}),
        );
    });

    Ok(())
}

/// Disconnect from the ESP32 target.
#[tauri::command]
pub async fn target_disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .target
        .connected
        .store(false, std::sync::atomic::Ordering::SeqCst);
    {
        let mut ip = state.target.ip.lock().await;
        *ip = None;
    }
    Ok(())
}

/// Start the remote runtime via WebSocket.
#[tauri::command]
pub async fn target_start(
    state: tauri::State<'_, AppState>,
    cycle_ms: u32,
) -> Result<(), String> {
    let ip = state
        .target
        .ip
        .lock()
        .await
        .clone()
        .ok_or("Not connected to target")?;

    // [0x04] [4B cycle_time_ms LE]
    let mut payload = vec![0x04u8];
    payload.extend_from_slice(&cycle_ms.to_le_bytes());

    send_ws_command(&ip, &payload).await
}

/// Stop the remote runtime via WebSocket.
#[tauri::command]
pub async fn target_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let ip = state
        .target
        .ip
        .lock()
        .await
        .clone()
        .ok_or("Not connected to target")?;

    send_ws_command(&ip, &[0x05]).await
}

/// Step the remote runtime via WebSocket.
#[tauri::command]
pub async fn target_step(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let ip = state
        .target
        .ip
        .lock()
        .await
        .clone()
        .ok_or("Not connected to target")?;

    send_ws_command(&ip, &[0x06]).await
}

/// Get current target connection status.
#[tauri::command]
pub async fn target_status(state: tauri::State<'_, AppState>) -> Result<TargetStatusResult, String> {
    let connected = state
        .target
        .connected
        .load(std::sync::atomic::Ordering::SeqCst);
    let ip = state.target.ip.lock().await.clone();

    // Try to fetch state from ESP32 if connected
    let (runtime_state, cycle_count) = if connected {
        if let Some(ref ip) = ip {
            match fetch_runtime_state(ip).await {
                Ok(s) => (Some(s.state), Some(s.cycle_count)),
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    Ok(TargetStatusResult {
        connected,
        ip,
        runtime_state,
        cycle_count,
    })
}

/// Register the ESP32 target as a device in Voltrus SCADA.
#[tauri::command]
pub async fn register_scada_device(
    state: tauri::State<'_, AppState>,
    registration: ScadaRegistration,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/voltrus-plc/devices", registration.scada_url.trim_end_matches('/'));

    let payload = serde_json::json!({
        "name": registration.device_name,
        "url": registration.device_url,
        "transport": registration.transport,
        "interval_ms": registration.interval_ms,
    });

    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("SCADA registration failed: {e}"))?;

    if resp.status().is_success() {
        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid SCADA response: {e}"))?;
        Ok(result)
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("SCADA registration failed (HTTP {status}): {body}"))
    }
}

// ── Internal helpers ──

#[derive(serde::Serialize)]
struct WatchedVar {
    name: String,
    value: serde_json::Value,
    display_value: String,
    type_name: String,
    quality: String,
}

#[derive(serde::Deserialize)]
struct RemoteState {
    state: String,
    cycle_count: u64,
}

/// Parse ESP32 VAR_UPDATE frame (msg_type 0x01 already stripped).
/// Format: [2B var_count] [for each: 2B name_len + name + 1B type + 4B value + 4B timestamp]
fn parse_var_update(data: &[u8]) -> Result<Vec<WatchedVar>, String> {
    if data.len() < 2 {
        return Err("VAR_UPDATE too short".into());
    }
    let var_count = u16::from_le_bytes([data[0], data[1]]) as usize;
    let mut ptr = 2;
    let mut vars = Vec::with_capacity(var_count);

    for _ in 0..var_count {
        if ptr + 2 > data.len() {
            break;
        }
        let name_len = u16::from_le_bytes([data[ptr], data[ptr + 1]]) as usize;
        ptr += 2;

        if ptr + name_len + 9 > data.len() {
            break;
        }
        let name = String::from_utf8_lossy(&data[ptr..ptr + name_len]).into_owned();
        ptr += name_len;

        let type_tag = data[ptr];
        ptr += 1;

        let raw_val = u32::from_le_bytes([
            data[ptr],
            data[ptr + 1],
            data[ptr + 2],
            data[ptr + 3],
        ]);
        ptr += 4;
        ptr += 4; // skip timestamp

        let (value, type_name) = decode_esp_value(type_tag, raw_val);
        let display_value = format_value(&value);

        vars.push(WatchedVar {
            name,
            value,
            display_value,
            type_name,
            quality: "good".into(),
        });
    }

    Ok(vars)
}

fn decode_esp_value(type_tag: u8, raw: u32) -> (serde_json::Value, String) {
    match type_tag {
        0 => (serde_json::Value::Bool(raw != 0), "BOOL".into()),
        1 => (serde_json::json!(raw as u8), "BYTE".into()),
        2 => (serde_json::json!(raw as i32 as i16), "INT".into()),
        3 => (serde_json::json!(raw as i32), "DINT".into()),
        4 => {
            let f: f32 = f32::from_bits(raw);
            (serde_json::json!(f), "REAL".into())
        }
        5 => (serde_json::json!(raw as i32), "TIME".into()),
        6 => (serde_json::json!(raw), "STRING".into()),
        7 => (serde_json::json!(raw as u16), "WORD".into()),
        8 => (serde_json::json!(raw), "DWORD".into()),
        _ => (serde_json::Value::Null, "UNKNOWN".into()),
    }
}

fn format_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f == (f as i64) as f64 {
                    (f as i64).to_string()
                } else {
                    format!("{f:.3}")
                }
            } else {
                n.to_string()
            }
        }
        _ => val.to_string(),
    }
}

/// Parse STATUS frame (msg_type 0x07 already stripped).
fn parse_status_msg(data: &[u8]) -> Result<crate::commands::runtime::RuntimeStateInfo, String> {
    // [1B state] [4B cycle_count] [4B scan_time_us] [4B max_scan_time_us] [4B var_count]
    if data.len() < 17 {
        return Err("STATUS too short".into());
    }
    let state_code = data[0];
    let state_name = match state_code {
        0 => "stopped",
        1 => "running",
        2 => "paused",
        3 => "error",
        _ => "unknown",
    };

    let cycle_count = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as u64;
    let scan_time_us = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as f64;
    let max_scan_time_us = u32::from_le_bytes([data[9], data[10], data[11], data[12]]) as f64;

    Ok(crate::commands::runtime::RuntimeStateInfo {
        state: state_name.into(),
        cycle_count,
        last_cycle_ms: scan_time_us / 1000.0,
        avg_cycle_ms: scan_time_us / 1000.0,
        max_cycle_ms: max_scan_time_us / 1000.0,
        error_message: None,
        cycle_history: vec![],
    })
}

/// Parse ERROR frame (msg_type 0x08 already stripped).
fn parse_error_msg(data: &[u8]) -> Result<String, String> {
    // [2B msg_len] [msg_bytes]
    if data.len() < 2 {
        return Err("ERROR too short".into());
    }
    let msg_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + msg_len {
        return Err("ERROR truncated".into());
    }
    Ok(String::from_utf8_lossy(&data[2..2 + msg_len]).into_owned())
}

/// Send a binary command via a short-lived WebSocket connection.
async fn send_ws_command(ip: &str, payload: &[u8]) -> Result<(), String> {
    let url = format!("ws://{ip}/ws/plc");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("Command failed: {e}"))?;

    use futures::SinkExt;
    ws.send(Message::Binary(payload.to_vec().into()))
        .await
        .map_err(|e| format!("Send failed: {e}"))?;

    let _ = ws.close(None).await;
    Ok(())
}

/// Fetch runtime state from ESP32 REST API.
async fn fetch_runtime_state(ip: &str) -> Result<RemoteState, String> {
    let url = format!("http://{ip}/api/state");
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("State fetch failed: {e}"))?;

    resp.json()
        .await
        .map_err(|e| format!("State parse failed: {e}"))
}
