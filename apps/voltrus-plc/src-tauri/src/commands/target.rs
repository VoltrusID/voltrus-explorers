//! # Drift detection (PLC-6)
//!
//! The IDE records the CRC32 of every bundle it deploys (`TargetState::last_deployed_crc32`)
//! and compares it against the CRC32 reported by the running target
//! (`target_status` → `deployed_crc32` vs `running_crc32`).
//!
//! CRC definition (must stay identical on every side):
//! - `crc32_le` (reflected IEEE, init 0, no final XOR — see `mt-plc-types`).
//! - Linux gRPC target: CRC over the full bincode payload sent as
//!   `DeployRequest.bytecode`; reported via `RuntimeState.deployed_bundle_crc32`.
//! - ESP32 (this file): CRC over the VLT1 bundle body WITHOUT its trailing
//!   4-byte CRC field — exactly the value the firmware already computes
//!   during deploy validation.
//!
//! Required ESP32-C firmware change (firmware lives outside this repo):
//! `GET /api/state` must include the loaded-bundle CRC32 in its JSON:
//!
//!      { "state": "running", "cycle_count": 1234,
//!        "loaded_bundle_crc32": 3149628632 }
//!
//! After a successful `/api/deploy` validation the firmware stores
//! `crc32_le(payload[0 .. len-4])` and reports it here. Older firmware that
//! omits the field deserializes as `None`, and the UI shows "unknown".

use std::sync::Arc;
use crate::state::AppState;
use futures::{SinkExt, StreamExt};
use mt_plc_types::{crc32_le, serialize_bytecode, IoMapping, IoType};
use tauri::Emitter;
use tokio_tungstenite::tungstenite::Message;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct TargetStatusResult {
    pub connected: bool,
    pub ip: Option<String>,
    pub runtime_state: Option<String>,
    pub cycle_count: Option<u64>,
    /// CRC32 of the last bundle deployed from this project session.
    pub deployed_crc32: Option<u32>,
    /// CRC32 the running target reports for its loaded bundle.
    pub running_crc32: Option<u32>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DeployResult {
    pub success: bool,
    pub message: String,
    pub byte_count: Option<usize>,
    /// CRC32 recorded for this deploy (drift-detection fingerprint).
    pub crc32: Option<u32>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ScadaRegistration {
    pub scada_url: String,
    pub device_name: String,
    pub device_url: String,
    pub transport: String,
    pub interval_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DiscoveredTarget {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

/// Scan for Voltrus Runtime devices on the local network via mDNS.
#[tauri::command]
pub async fn scan_mdns_targets() -> Result<Vec<DiscoveredTarget>, String> {
    let daemon = mdns_sd::ServiceDaemon::new()
        .map_err(|e| format!("mDNS init failed: {e}"))?;
    let receiver = daemon.browse("_voltrus-runtime._tcp.local.")
        .map_err(|e| format!("mDNS browse failed: {e}"))?;

    let mut targets: Vec<DiscoveredTarget> = Vec::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(3);

    loop {
        match tokio::time::timeout_at(deadline, receiver.recv_async()).await {
            Ok(Ok(event)) => match event {
                mdns_sd::ServiceEvent::ServiceResolved(info) => {
                    let name = info.get_fullname().to_string();
                    for addr in info.get_addresses() {
                        targets.push(DiscoveredTarget {
                            name: name.clone(),
                            ip: addr.to_string(),
                            port: info.get_port(),
                        });
                    }
                }
                mdns_sd::ServiceEvent::SearchStarted(_) | mdns_sd::ServiceEvent::ServiceFound(_, _) => {}
                _ => {}
            },
            Ok(Err(_)) | Err(_) => break,
        }
    }
    let _ = daemon.stop_browse("_voltrus-runtime._tcp.local.");
    Ok(targets)
}

/// Frontend I/O mapping format (string-based, from project file).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct FrontendIoMapping {
    pub variable: String,
    pub direction: String,
    pub address: String,
    pub driver_id: String,
    #[serde(default)]
    pub protocol_address: String,
}

/// Deploy compiled bytecode to a remote ESP32 target via HTTP POST.
#[tauri::command]
pub async fn deploy_to_target(
    state: tauri::State<'_, AppState>,
    ip: String,
    io_mappings: Option<Vec<FrontendIoMapping>>,
) -> Result<DeployResult, String> {
    let module = {
        let compiler = state.compiler.lock().await;
        compiler
            .last_module
            .clone()
            .ok_or("No compiled program. Click Build first.")?
    };

    let mappings = match io_mappings {
        Some(ref frontend) if !frontend.is_empty() => {
            resolve_mappings(frontend, &module.variable_slots)
        }
        _ => auto_gpio_mappings(&module.variable_slots),
    };

    // Cache for `get_io_mappings`
    *state.io.mappings.lock().await = frontend_cache(io_mappings.as_deref());

    let bytecode = serialize_bytecode(&module, &mappings)
        .map_err(|e| format!("Bytecode serialization failed: {e}"))?;

    let byte_count = bytecode.len();
    // Drift fingerprint: CRC over the VLT1 body without the trailing CRC
    // field — the same value the firmware computes during deploy validation.
    let bundle_crc = (byte_count >= 4).then(|| crc32_le(&bytecode[..byte_count - 4]));
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

        *state.target.last_deployed_crc32.lock().await = bundle_crc;

        Ok(DeployResult {
            success: true,
            message: "Deployed successfully".into(),
            byte_count: Some(byte_count),
            crc32: bundle_crc,
        })
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Deploy failed (HTTP {status}): {body}"))
    }
}

/// Connect to ESP32 WebSocket for live variable streaming.
/// Splits the connection into a persistent sink (for commands) and a reader task (for events).
#[tauri::command]
pub async fn target_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    ip: String,
) -> Result<(), String> {
    let url = format!("ws://{ip}/ws/plc");

    let ws = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("WebSocket connection failed: {e}"))?
        .0;

    // Split into sink (for sending commands) and stream (for receiving events)
    let (sink, stream) = ws.split();

    // Store sink in state for persistent command channel
    {
        let mut ws_sink = state.target.ws_sink.lock().await;
        *ws_sink = Some(sink);
    }

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
    let ws_sink = state.target.ws_sink.clone();

    // Background reader: parse ESP32 binary frames and emit Tauri events
    let app_handle = app.clone();
    tokio::spawn(async move {
        let mut stream = stream;
        while let Some(msg) = stream.next().await {
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
                    let mut sink_guard = ws_sink.lock().await;
                    *sink_guard = None;
                    break;
                }
                Err(_) => {
                    connected.store(false, std::sync::atomic::Ordering::SeqCst);
                    let mut sink_guard = ws_sink.lock().await;
                    *sink_guard = None;
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
        let mut sink = state.target.ws_sink.lock().await;
        if let Some(mut s) = sink.take() {
            let _ = s.close().await;
        }
    }
    Ok(())
}

/// Start the remote runtime via the persistent WebSocket sink.
#[tauri::command]
pub async fn target_start(
    state: tauri::State<'_, AppState>,
    cycle_ms: u32,
) -> Result<(), String> {
    // [0x04] [4B cycle_time_ms LE]
    let mut payload = vec![0x04u8];
    payload.extend_from_slice(&cycle_ms.to_le_bytes());

    send_via_sink(&state.target.ws_sink, &payload).await
}

/// Stop the remote runtime via the persistent WebSocket sink.
#[tauri::command]
pub async fn target_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    send_via_sink(&state.target.ws_sink, &[0x05]).await
}

/// Step the remote runtime via the persistent WebSocket sink.
#[tauri::command]
pub async fn target_step(state: tauri::State<'_, AppState>) -> Result<(), String> {
    send_via_sink(&state.target.ws_sink, &[0x06]).await
}

/// Get current target connection status, including the drift-detection
/// fingerprints: `deployed_crc32` (what this project last deployed) and
/// `running_crc32` (what the target reports as loaded — `/api/state` for
/// ESP32, `GetRuntimeState` for gRPC targets). `None` means unknown.
#[tauri::command]
pub async fn target_status(state: tauri::State<'_, AppState>) -> Result<TargetStatusResult, String> {
    let connected = state
        .target
        .connected
        .load(std::sync::atomic::Ordering::SeqCst);
    let ip = state.target.ip.lock().await.clone();
    let kind = state.target.target_kind.lock().await.clone();

    let (runtime_state, cycle_count, running_crc32) = if connected {
        match kind {
            crate::state::TargetKind::Grpc => {
                let mut client = state.target.grpc_client.lock().await.clone();
                match client.as_mut().map(|c| c.get_runtime_state()) {
                    Some(fut) => match fut.await {
                        Ok(s) => (
                            Some(pb_status_name(s.status).to_string()),
                            Some(s.cycle_count),
                            (s.deployed_bundle_crc32 != 0).then_some(s.deployed_bundle_crc32),
                        ),
                        Err(_) => (None, None, None),
                    },
                    None => (None, None, None),
                }
            }
            crate::state::TargetKind::WebSocket => match ip.as_deref() {
                Some(ip) => match fetch_runtime_state(ip).await {
                    Ok(s) => (
                        Some(s.state),
                        Some(s.cycle_count),
                        s.loaded_bundle_crc32.filter(|crc| *crc != 0),
                    ),
                    Err(_) => (None, None, None),
                },
                None => (None, None, None),
            },
        }
    } else {
        (None, None, None)
    };

    let deployed_crc32 = state.target.last_deployed_crc32.lock().await.clone();

    Ok(TargetStatusResult {
        connected,
        ip,
        runtime_state,
        cycle_count,
        deployed_crc32,
        running_crc32,
    })
}

/// Human-readable name for the proto `RuntimeStatus` (mirrors the ESP32
/// status names used by `parse_status_msg`). The prost field is a raw `i32`.
fn pb_status_name(status: i32) -> &'static str {
    match mt_grpc::RuntimeStatus::try_from(status) {
        Ok(mt_grpc::RuntimeStatus::Stopped) => "stopped",
        Ok(mt_grpc::RuntimeStatus::Running) => "running",
        Ok(mt_grpc::RuntimeStatus::Paused) => "paused",
        Ok(mt_grpc::RuntimeStatus::Error) => "error",
        _ => "unknown",
    }
}

// ── Drift comparison (PLC-6) ──

/// Outcome of comparing the project's last-deployed CRC against the
/// running target's loaded CRC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftStatus {
    /// Both CRCs known and equal.
    Sync,
    /// Both CRCs known but different — the target runs another program.
    Drift,
    /// Either side unknown (never deployed, old firmware, disconnected).
    Unknown,
}

/// Compare the project-side CRC with the running CRC. A CRC of 0 counts as
/// unknown (proto3 default / "nothing loaded"), mirroring the frontend rule.
pub fn compare_crc(project_crc: Option<u32>, running_crc: Option<u32>) -> DriftStatus {
    match (project_crc, running_crc) {
        (Some(project), Some(running)) if project != 0 && running != 0 => {
            if project == running {
                DriftStatus::Sync
            } else {
                DriftStatus::Drift
            }
        }
        _ => DriftStatus::Unknown,
    }
}

#[cfg(test)]
mod drift_tests {
    use super::*;

    #[test]
    fn equal_crcs_are_in_sync() {
        assert_eq!(compare_crc(Some(0xDEADBEEF), Some(0xDEADBEEF)), DriftStatus::Sync);
    }

    #[test]
    fn different_crcs_drift() {
        assert_eq!(compare_crc(Some(0xDEADBEEF), Some(0x12345678)), DriftStatus::Drift);
    }

    #[test]
    fn missing_either_side_is_unknown() {
        assert_eq!(compare_crc(None, Some(0xDEADBEEF)), DriftStatus::Unknown);
        assert_eq!(compare_crc(Some(0xDEADBEEF), None), DriftStatus::Unknown);
        assert_eq!(compare_crc(None, None), DriftStatus::Unknown);
    }

    #[test]
    fn zero_crc_counts_as_unknown() {
        assert_eq!(compare_crc(Some(0), Some(0xDEADBEEF)), DriftStatus::Unknown);
        assert_eq!(compare_crc(Some(0xDEADBEEF), Some(0)), DriftStatus::Unknown);
    }

    #[test]
    fn drift_lifecycle_transitions() {
        let mut status = compare_crc(None, None);
        assert_eq!(status, DriftStatus::Unknown); // not connected / nothing deployed

        // Deploy succeeded, target reports the same bundle.
        status = compare_crc(Some(42), Some(42));
        assert_eq!(status, DriftStatus::Sync);

        // Someone else redeployed on the target.
        status = compare_crc(Some(42), Some(43));
        assert_eq!(status, DriftStatus::Drift);

        // Target went away (old firmware or disconnected).
        status = compare_crc(Some(42), None);
        assert_eq!(status, DriftStatus::Unknown);
    }
}

/// Register the ESP32 target as a device in Voltrus SCADA.
#[tauri::command]
pub async fn register_scada_device(
    _state: tauri::State<'_, AppState>,
    registration: ScadaRegistration,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let base = registration.scada_url.trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err("SCADA URL must start with http:// or https://".into());
    }
    let url = format!("{base}/api/v1/voltrus-plc/devices");

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

/// Set a breakpoint on the remote target via WebSocket.
#[tauri::command]
pub async fn set_remote_breakpoint(
    state: tauri::State<'_, AppState>,
    line: u32,
) -> Result<(), String> {
    let mut payload = vec![0x09]; // MSG_SET_BREAKPOINT
    payload.extend_from_slice(&line.to_le_bytes());
    send_via_sink(&state.target.ws_sink, &payload).await
}

/// Clear a breakpoint on the remote target via WebSocket.
#[tauri::command]
pub async fn clear_remote_breakpoint(
    state: tauri::State<'_, AppState>,
    line: u32,
) -> Result<(), String> {
    let mut payload = vec![0x0A]; // MSG_CLEAR_BREAKPOINT
    payload.extend_from_slice(&line.to_le_bytes());
    send_via_sink(&state.target.ws_sink, &payload).await
}

/// Get/set the target kind (WebSocket for ESP32, gRPC for Linux).
#[tauri::command]
pub async fn get_target_kind(state: tauri::State<'_, AppState>) -> Result<crate::state::TargetKind, String> {
    let kind = state.target.target_kind.lock().await;
    Ok(kind.clone())
}

#[tauri::command]
pub async fn set_target_kind(
    state: tauri::State<'_, AppState>,
    kind: crate::state::TargetKind,
) -> Result<(), String> {
    *state.target.target_kind.lock().await = kind;
    Ok(())
}

/// Deploy compiled bytecode to a Linux target via gRPC.
///
/// The Linux runtime deserializes the payload with `bincode` (see the
/// `Deploy` handler in `mt-grpc` server), so the module is sent via
/// serde/bincode — the ESP32 `VLT1` format is only used by the HTTP
/// deploy path (`deploy_to_target`).
#[tauri::command]
pub async fn deploy_via_grpc(
    state: tauri::State<'_, AppState>,
    addr: String,
    io_mappings: Option<Vec<FrontendIoMapping>>,
) -> Result<DeployResult, String> {
    let module = {
        let compiler = state.compiler.lock().await;
        compiler.last_module.clone()
    };

    let module = module.ok_or("No compiled program. Build first.")?;

    // Cache for `get_io_mappings`
    *state.io.mappings.lock().await = frontend_cache(io_mappings.as_deref());

    let bytecode = bincode::serialize(&module)
        .map_err(|e| format!("Bytecode serialization failed: {e}"))?;
    // Drift fingerprint: CRC over the full bincode payload — the value the
    // mt-grpc server records from `DeployRequest.bytecode`.
    let bundle_crc = crc32_le(&bytecode);

    // I/O mappings travel in the config JSON (proto IoMapping shape) so the
    // Linux runtime can wire its IoManager.
    let io_json: Vec<serde_json::Value> = match io_mappings {
        Some(ref frontend) if !frontend.is_empty() => frontend
            .iter()
            .map(|m| {
                serde_json::json!({
                    "variable_name": m.variable,
                    "address": m.address,
                    "driver": m.driver_id,
                    "connection_json": m.protocol_address,
                })
            })
            .collect(),
        _ => vec![],
    };
    let config = serde_json::json!({ "io_mappings": io_json }).to_string();

    let mut client = mt_grpc::client::PlcRuntimeClient::connect(&addr)
        .await
        .map_err(|e| format!("gRPC connection to {addr} failed: {e}"))?;

    let resp = client.deploy(&bytecode, Some(config.as_str()))
        .await
        .map_err(|e| format!("gRPC deploy failed: {e}"))?;

    let msg = if resp.error_message.is_empty() {
        "Deployed via gRPC".into()
    } else {
        resp.error_message
    };

    let success = resp.success;
    if success {
        *state.target.ip.lock().await = Some(addr.clone());
        *state.target.target_kind.lock().await = crate::state::TargetKind::Grpc;
        *state.target.grpc_addr.lock().await = Some(addr);
        *state.target.grpc_client.lock().await = Some(client);
        state
            .target
            .connected
            .store(true, std::sync::atomic::Ordering::SeqCst);
        *state.target.last_deployed_crc32.lock().await = Some(bundle_crc);
    }

    Ok(DeployResult {
        success,
        message: msg,
        byte_count: Some(bytecode.len()),
        crc32: Some(bundle_crc),
    })
}

/// Start the remote runtime via gRPC.
#[tauri::command]
pub async fn grpc_start(
    state: tauri::State<'_, AppState>,
    addr: String,
    cycle_ms: u32,
) -> Result<String, String> {
    let mut client = grpc_client_for(&state, &addr).await?;
    let config = serde_json::json!({"scan_cycle_ms": cycle_ms}).to_string();
    let resp = client.start(Some(config.as_str())).await
        .map_err(|e| format!("gRPC start failed: {e}"))?;
    Ok(format!("{:?}", resp.status))
}

/// Stop the remote runtime via gRPC.
#[tauri::command]
pub async fn grpc_stop(state: tauri::State<'_, AppState>, addr: String) -> Result<String, String> {
    let mut client = grpc_client_for(&state, &addr).await?;
    let resp = client.stop().await
        .map_err(|e| format!("gRPC stop failed: {e}"))?;
    Ok(format!("{:?}", resp.status))
}

/// Step the remote runtime via gRPC.
#[tauri::command]
pub async fn grpc_step(state: tauri::State<'_, AppState>, addr: String) -> Result<String, String> {
    let mut client = grpc_client_for(&state, &addr).await?;
    let resp = client.step().await
        .map_err(|e| format!("gRPC step failed: {e}"))?;
    Ok(format!("{:?}", resp.status))
}

/// Return the persistent gRPC client when it matches `addr`; otherwise
/// connect, store it, and mark the target connected.
async fn grpc_client_for(
    state: &tauri::State<'_, AppState>,
    addr: &str,
) -> Result<mt_grpc::client::PlcRuntimeClient, String> {
    {
        let stored_addr = state.target.grpc_addr.lock().await;
        if stored_addr.as_deref() == Some(addr) {
            let client = state.target.grpc_client.lock().await;
            if let Some(c) = client.as_ref() {
                return Ok(c.clone());
            }
        }
    }

    let client = mt_grpc::client::PlcRuntimeClient::connect(addr)
        .await
        .map_err(|e| format!("gRPC connect failed: {e}"))?;
    *state.target.grpc_addr.lock().await = Some(addr.to_string());
    *state.target.grpc_client.lock().await = Some(client.clone());
    state
        .target
        .connected
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(client)
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
    /// Drift fingerprint reported by the firmware (`loaded_bundle_crc32` in
    /// `/api/state`). Absent on older firmware → `None` → UI shows "unknown".
    #[serde(default)]
    loaded_bundle_crc32: Option<u32>,
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

/// Send a binary command through the persistent WebSocket sink.
async fn send_via_sink(
    ws_sink: &Arc<tokio::sync::Mutex<Option<crate::state::WsSink>>>,
    payload: &[u8],
) -> Result<(), String> {
    use futures::SinkExt;
    let mut sink_guard = ws_sink.lock().await;
    let sink: &mut crate::state::WsSink = sink_guard
        .as_mut()
        .ok_or("Not connected to target")?;

    sink.send(Message::Binary(payload.to_vec().into()))
        .await
        .map_err(|e| format!("Send failed: {e}"))?;

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

/// Value type tag + raw u32 for remote force commands.
pub struct RemoteForceValue {
    pub type_tag: u8,
    pub raw: u32,
}

impl RemoteForceValue {
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Bool(b) => Self { type_tag: 0, raw: if *b { 1 } else { 0 } },
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self { type_tag: 3, raw: (i as i32) as u32 }
                } else if let Some(f) = n.as_f64() {
                    Self { type_tag: 4, raw: (f as f32).to_bits() }
                } else {
                    Self { type_tag: 3, raw: 0 }
                }
            }
            _ => Self { type_tag: 3, raw: 0 },
        }
    }
}

/// Build a FORCE command payload.
/// Format: [0x02] [2B name_len] [name_bytes] [1B type] [4B value]
fn build_force_payload(name: &str, val: &RemoteForceValue) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len() as u16;
    let mut payload = vec![0x02u8];
    payload.extend_from_slice(&name_len.to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload.push(val.type_tag);
    payload.extend_from_slice(&val.raw.to_le_bytes());
    payload
}

/// Build an UNFORCE command payload.
/// Format: [0x03] [2B name_len] [name_bytes]
fn build_unforce_payload(name: &str) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len() as u16;
    let mut payload = vec![0x03u8];
    payload.extend_from_slice(&name_len.to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload
}

/// Send a FORCE command via the persistent WebSocket sink.
pub async fn send_remote_force(
    ws_sink: &Arc<tokio::sync::Mutex<Option<crate::state::WsSink>>>,
    name: &str,
    val: &RemoteForceValue,
) -> Result<(), String> {
    let payload = build_force_payload(name, val);
    send_via_sink(ws_sink, &payload).await
}

/// Send an UNFORCE command via the persistent WebSocket sink.
pub async fn send_remote_unforce(
    ws_sink: &Arc<tokio::sync::Mutex<Option<crate::state::WsSink>>>,
    name: &str,
) -> Result<(), String> {
    let payload = build_unforce_payload(name);
    send_via_sink(ws_sink, &payload).await
}

/// Snapshot frontend mappings for `get_io_mappings` (lossless, same shape).
fn frontend_cache(frontend: Option<&[FrontendIoMapping]>) -> Vec<crate::commands::io::IoMapping> {
    frontend
        .unwrap_or(&[])
        .iter()
        .map(|m| crate::commands::io::IoMapping {
            variable: m.variable.clone(),
            direction: m.direction.clone(),
            address: m.address.clone(),
            driver_id: m.driver_id.clone(),
            protocol_address: m.protocol_address.clone(),
        })
        .collect()
}

/// Resolve frontend string-based mappings to index-based bytecode mappings.
fn resolve_mappings(
    frontend: &[FrontendIoMapping],
    variable_slots: &[(String, mt_plc_types::PlcValue)],
) -> Vec<IoMapping> {
    use mt_plc_types::{IoMapping, IoProtocol, IoType};

    frontend
        .iter()
        .filter_map(|fm| {
            let variable_index = variable_slots
                .iter()
                .position(|(name, _)| name == &fm.variable)? as u32;

            let io_type = match fm.direction.as_str() {
                "input" => IoType::Input,
                _ => IoType::Output,
            };

            let protocol = match fm.driver_id.as_str() {
                "modbus" => IoProtocol::Modbus,
                "s7" => IoProtocol::S7,
                "enip" => IoProtocol::Enip,
                "gpio" => IoProtocol::Gpio,
                "mqtt" => IoProtocol::Mqtt,
                "opcua" => IoProtocol::Opcua,
                _ => IoProtocol::None,
            };

            let address: u32 = fm.address.parse().unwrap_or(0);

            Some(IoMapping {
                variable_index,
                io_type,
                protocol,
                address,
            })
        })
        .collect()
}

/// Auto-create GPIO I/O mappings for known ESP32 variable names.
/// Waveshare ESP32-S3-Relay-6ch board:
///   relay0-5 → GPIO 1, 2, 41, 42, 45, 46 (via board_set_relay)
///   buzzer   → GPIO 21 (LEDC PWM via board_set_buzzer)
///   red/green/blue → GPIO 38 (WS2812 via board_set_rgb)
///
/// Address indices in I/O bridge: 0-5=relay, 6=buzzer, 7=red, 8=green, 9=blue
fn auto_gpio_mappings(variable_slots: &[(String, mt_plc_types::PlcValue)]) -> Vec<IoMapping> {
    use mt_plc_types::{IoMapping, IoProtocol, IoType};

    let mut mappings = Vec::new();
    for (i, (name, _)) in variable_slots.iter().enumerate() {
        let (io_type, address) = match name.as_str() {
            // Relays: relay0-relay5 → I/O bridge index 0-5
            n if n.starts_with("relay") && n[5..].parse::<u32>().is_ok() => {
                let idx: u32 = n[5..].parse().unwrap();
                if idx < 6 {
                    (IoType::Output, idx)
                } else {
                    continue;
                }
            }
            // Buzzer → I/O bridge index 6
            "buzzer" => (IoType::Output, 6),
            // RGB LED → I/O bridge index 7 (red), 8 (green), 9 (blue)
            "red" => (IoType::Output, 7),
            "green" => (IoType::Output, 8),
            "blue" => (IoType::Output, 9),
            _ => continue,
        };
        mappings.push(IoMapping {
            variable_index: i as u32,
            io_type,
            protocol: IoProtocol::Gpio,
            address,
        });
    }
    mappings
}
