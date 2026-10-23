use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::sync::Mutex;

use mt_protocol::ProtocolClient;
use mt_s7::{
    BlockListEntry, BlockType, CpuInfo, DiagBuffer, PlcMode, S7Client, S7Config, S7DataType,
    S7Server, S7Tag, S7Value,
};

// ── App state ──

/// Holds one live client per device id, plus at most one running simulator.
struct AppState {
    clients: Mutex<HashMap<String, Arc<S7Client>>>,
    sim: Mutex<Option<Arc<S7Server>>>,
}

impl AppState {
    async fn client(&self, device_id: &str) -> Result<Arc<S7Client>, String> {
        self.clients
            .lock()
            .await
            .get(device_id)
            .cloned()
            .ok_or_else(|| format!("Device '{device_id}' is not connected"))
    }
}

// ── Wire shapes ──

/// Frontend tag spec: an address plus the data type the user chose.
/// The address itself encodes the type (DB1.DBD0 → DWord); `data_type` is the
/// authoritative override when present.
#[derive(Debug, Clone, Deserialize)]
struct TagSpec {
    address: String,
    #[serde(default)]
    data_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

impl TagSpec {
    fn to_tag(&self) -> Result<S7Tag, String> {
        let mut tag = S7Tag::parse_address(&self.address)
            .map_err(|e| format!("Invalid address '{}': {e}", self.address))?;
        if let Some(dt) = self.data_type.as_deref() {
            tag.set_data_type_by_name(dt);
        }
        if let Some(name) = self.name.as_ref() {
            tag.name = Some(name.clone());
        }
        Ok(tag)
    }
}

#[derive(Debug, Clone, Serialize)]
struct DeviceInfo {
    id: String,
    host: String,
    port: u16,
    rack: u8,
    slot: u8,
}

/// A single simulated data point seeded into the S7 server image.
#[derive(Debug, Clone, Deserialize)]
struct SimPointDef {
    address: String,
    #[serde(default)]
    data_type: Option<String>,
    value: f64,
}

#[derive(Debug, Clone, Serialize)]
struct SimInfo {
    port: u16,
    point_count: usize,
}

// ── Value coercion ──

/// Build an [`S7Value`] for a given data type from a JSON scalar.
fn coerce_value(data_type: &str, value: &serde_json::Value) -> Result<S7Value, String> {
    let dtype = S7DataType::from_str_name(data_type)
        .ok_or_else(|| format!("Unknown data type: '{data_type}'"))?;
    Ok(match dtype {
        S7DataType::Bool => S7Value::Bool(value.as_bool().unwrap_or_else(|| {
            value.as_f64().map(|f| f != 0.0).unwrap_or(false)
        })),
        S7DataType::Byte => S7Value::Byte(value.as_f64().unwrap_or(0.0).round() as u8),
        S7DataType::Word => S7Value::Word(value.as_f64().unwrap_or(0.0).round() as u16),
        S7DataType::DWord => S7Value::DWord(value.as_f64().unwrap_or(0.0).round() as u32),
        S7DataType::Real => S7Value::Real(value.as_f64().unwrap_or(0.0) as f32),
        S7DataType::String => S7Value::String(
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string()),
        ),
        S7DataType::Timer => S7Value::Timer(value.as_f64().unwrap_or(0.0).round() as u32),
        S7DataType::Counter => S7Value::Counter(value.as_f64().unwrap_or(0.0).round() as u16),
    })
}

fn f64_for(dtype: &str, v: f64) -> S7Value {
    match S7DataType::from_str_name(dtype) {
        Some(S7DataType::Bool) => S7Value::Bool(v != 0.0),
        Some(S7DataType::Byte) => S7Value::Byte(v.round() as u8),
        Some(S7DataType::Word) => S7Value::Word(v.round() as u16),
        Some(S7DataType::DWord) => S7Value::DWord(v.round() as u32),
        Some(S7DataType::Timer) => S7Value::Timer(v.round() as u32),
        Some(S7DataType::Counter) => S7Value::Counter(v.round() as u16),
        _ => S7Value::Real(v as f32),
    }
}

// ── Device commands ──

#[tauri::command]
async fn connect(
    state: State<'_, AppState>,
    device_id: String,
    config: S7Config,
) -> Result<DeviceInfo, String> {
    let client = S7Client::connect(config.clone())
        .await
        .map_err(|e| e.to_string())?;
    state
        .clients
        .lock()
        .await
        .insert(device_id.clone(), Arc::new(client));
    Ok(DeviceInfo {
        id: device_id,
        host: config.host,
        port: config.port,
        rack: config.rack,
        slot: config.slot,
    })
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    if let Some(c) = state.clients.lock().await.remove(&device_id) {
        let _ = c.disconnect().await;
    }
    Ok(())
}

#[tauri::command]
async fn list_devices(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.clients.lock().await.keys().cloned().collect())
}

#[tauri::command]
async fn read_tags(
    state: State<'_, AppState>,
    device_id: String,
    tags: Vec<TagSpec>,
) -> Result<Vec<S7Value>, String> {
    let client = state.client(&device_id).await?;
    let parsed: Vec<S7Tag> = tags.iter().map(TagSpec::to_tag).collect::<Result<_, _>>()?;
    client.read(&parsed).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_tag(
    state: State<'_, AppState>,
    device_id: String,
    address: String,
    data_type: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let client = state.client(&device_id).await?;
    let mut tag = S7Tag::parse_address(&address)
        .map_err(|e| format!("Invalid address '{address}': {e}"))?;
    tag.set_data_type_by_name(&data_type);
    // Coerce to the type the tag will actually be written as.
    let v = coerce_value(&format!("{:?}", tag.data_type), &value)?;
    client.write(&tag, v).await.map_err(|e| e.to_string())
}

// ── Diagnostics ──

#[tauri::command]
async fn get_cpu_info(state: State<'_, AppState>, device_id: String) -> Result<CpuInfo, String> {
    let client = state.client(&device_id).await?;
    client.read_cpu_info().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_plc_mode(state: State<'_, AppState>, device_id: String) -> Result<PlcMode, String> {
    let client = state.client(&device_id).await?;
    client.read_plc_mode().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_diag_buffer(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<DiagBuffer, String> {
    let client = state.client(&device_id).await?;
    client.read_diag_buffer().await.map_err(|e| e.to_string())
}

// ── Block management ──

#[tauri::command]
async fn list_blocks(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<Vec<BlockListEntry>, String> {
    let client = state.client(&device_id).await?;
    client.list_blocks().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn upload_block(
    state: State<'_, AppState>,
    device_id: String,
    block_type: String,
    block_number: u16,
) -> Result<Vec<u8>, String> {
    let bt = BlockType::from_str_name(&block_type).ok_or("Invalid block type")?;
    let client = state.client(&device_id).await?;
    client
        .upload_block(&bt, block_number)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_plc_block(
    state: State<'_, AppState>,
    device_id: String,
    block_type: String,
    block_number: u16,
) -> Result<(), String> {
    let bt = BlockType::from_str_name(&block_type).ok_or("Invalid block type")?;
    let client = state.client(&device_id).await?;
    client
        .delete_block(&bt, block_number)
        .await
        .map_err(|e| e.to_string())
}

// ── Simulator (real S7 server) ──

#[tauri::command]
async fn sim_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    points: Vec<SimPointDef>,
    port: u16,
) -> Result<SimInfo, String> {
    let mut initial: Vec<(S7Tag, S7Value)> = Vec::with_capacity(points.len());
    for p in &points {
        let mut tag = S7Tag::parse_address(&p.address)
            .map_err(|e| format!("Invalid sim address '{}': {e}", p.address))?;
        if let Some(dt) = p.data_type.as_deref() {
            tag.set_data_type_by_name(dt);
        }
        let value = f64_for(&format!("{:?}", tag.data_type), p.value);
        initial.push((tag, value));
    }

    let server = S7Server::start(port, initial)
        .await
        .map_err(|e| format!("Failed to start simulator: {e}"))?;
    let bound_port = server.port();
    let info = SimInfo {
        port: bound_port,
        point_count: points.len(),
    };
    *state.sim.lock().await = Some(Arc::new(server));
    let _ = app.emit(
        "sim-log",
        serde_json::json!({
            "level": "info",
            "message": format!("Simulator listening on 0.0.0.0:{bound_port} ({} points)", info.point_count)
        }),
    );
    Ok(info)
}

#[tauri::command]
async fn sim_stop(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(server) = state.sim.lock().await.take() {
        server.stop().await;
        let _ = app.emit(
            "sim-log",
            serde_json::json!({ "level": "info", "message": "Simulator stopped" }),
        );
    }
    Ok(())
}

#[tauri::command]
async fn sim_status(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.sim.lock().await.is_some())
}

#[tauri::command]
async fn sim_set_value(
    state: State<'_, AppState>,
    address: String,
    data_type: String,
    value: f64,
) -> Result<(), String> {
    let guard = state.sim.lock().await;
    let server = guard.as_ref().ok_or("Simulator is not running")?;
    let mut tag = S7Tag::parse_address(&address)
        .map_err(|e| format!("Invalid address '{address}': {e}"))?;
    tag.set_data_type_by_name(&data_type);
    server
        .set_value(&tag, f64_for(&format!("{:?}", tag.data_type), value))
        .await;
    Ok(())
}

#[tauri::command]
async fn sim_set_bool(
    state: State<'_, AppState>,
    address: String,
    active: bool,
) -> Result<(), String> {
    let guard = state.sim.lock().await;
    let server = guard.as_ref().ok_or("Simulator is not running")?;
    let mut tag = S7Tag::parse_address(&address)
        .map_err(|e| format!("Invalid address '{address}': {e}"))?;
    tag.data_type = S7DataType::Bool;
    server.set_bool(&tag, active).await;
    Ok(())
}

// ── Shell ──

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            clients: Mutex::new(HashMap::new()),
            sim: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            list_devices,
            read_tags,
            write_tag,
            get_cpu_info,
            get_plc_mode,
            get_diag_buffer,
            list_blocks,
            upload_block,
            delete_plc_block,
            sim_start,
            sim_stop,
            sim_status,
            sim_set_value,
            sim_set_bool,
            open_url,
            mt_store::store_status,
            mt_store::store_purchase,
            mt_store::store_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
