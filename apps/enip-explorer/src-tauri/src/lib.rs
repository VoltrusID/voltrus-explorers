use std::sync::Arc;
use tokio::sync::Mutex;
use mt_enip::{
    EnipClient, EnipConfig, CipTag, CipTagDef, CipValue, CipDataType, CipTemplateDef,
    CipTemplateMember, EnipDevice, EnipSimDevice, IdentityInfo, PacketRecord, SimPointDef,
    browse_tags as browse_tags_fn, discover_devices as discover_devices_fn,
    start_enip_server, EnipServerConfig, EnipServerState, EnipServerLogEntry,
};
use mt_protocol::ProtocolClient;
use tauri::{Emitter, Manager};

// ── Scanner (client) state ──

struct ScannerState {
    client: Arc<Mutex<Option<EnipClient>>>,
    config: Arc<Mutex<Option<EnipConfig>>>,
}

// ── Adapter (server) state ──

struct AdapterServerState {
    state: Option<Arc<EnipServerState>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    log_rx: Option<tokio::sync::mpsc::UnboundedReceiver<EnipServerLogEntry>>,
}

/// Background tasks for an active scanner connection.
struct ScannerTasks {
    packet_fwd: Option<tokio::task::JoinHandle<()>>,
}

/// An active implicit/IO pump (ForwardOpen + cyclic reader).
struct IoState {
    reader: Option<tokio::task::JoinHandle<()>>,
}

struct AppState {
    scanner: ScannerState,
    adapter: Arc<Mutex<AdapterServerState>>,
    sim: Arc<Mutex<Option<EnipSimDevice>>>,
    sim_defs: Arc<Mutex<Vec<SimPointDef>>>,
    scanner_tasks: Arc<Mutex<ScannerTasks>>,
    io: Arc<Mutex<Option<IoState>>>,
}

// ── Scanner commands ──

#[tauri::command]
async fn connect(app: tauri::AppHandle, state: tauri::State<'_, AppState>, config: EnipConfig) -> Result<(), String> {
    let client = EnipClient::connect(config.clone()).await.map_err(|e| e.to_string())?;

    // Wire the raw-frame packet logger: a channel from the client into a
    // forwarder task that re-emits each frame as a Tauri event.
    let (pkt_tx, mut pkt_rx) = tokio::sync::mpsc::unbounded_channel::<PacketRecord>();
    client.set_packet_sink(pkt_tx).await;
    let app_fwd = app.clone();
    let fwd = tokio::spawn(async move {
        while let Some(rec) = pkt_rx.recv().await {
            let _ = app_fwd.emit("enip://packet", &rec);
        }
    });
    *state.scanner_tasks.lock().await = ScannerTasks { packet_fwd: Some(fwd) };

    *state.scanner.client.lock().await = Some(client);
    *state.scanner.config.lock().await = Some(config);
    Ok(())
}

#[tauri::command]
async fn read_tags(
    state: tauri::State<'_, AppState>,
    tags: Vec<CipTag>,
) -> Result<Vec<CipValue>, String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.read(&tags).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: CipTag,
    value: CipValue,
) -> Result<(), String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.write(&tag, value).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn browse_tags(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CipTag>, String> {
    let guard = state.scanner.config.lock().await;
    let config = guard.as_ref().ok_or("Not connected")?;
    browse_tags_fn(config).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        let mut guard = state.scanner.client.lock().await;
        if let Some(client) = guard.take() {
            client.disconnect().await.map_err(|e| e.to_string())?;
        }
    }
    // Stop background tasks (packet forwarder + any IO pump).
    if let Some(t) = state.scanner_tasks.lock().await.packet_fwd.take() {
        t.abort();
    }
    if let Some(io) = state.io.lock().await.take() {
        if let Some(r) = io.reader {
            r.abort();
        }
    }
    Ok(())
}

// ── Discovery & Identity (explorer parity) ──

#[tauri::command]
async fn discover_devices(timeout_ms: Option<u64>) -> Result<Vec<EnipDevice>, String> {
    let ms = timeout_ms.unwrap_or(3000);
    Ok(discover_devices_fn(ms).await)
}

#[tauri::command]
async fn get_device_identity(state: tauri::State<'_, AppState>) -> Result<IdentityInfo, String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.get_identity().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn cip_get_attribute(
    state: tauri::State<'_, AppState>,
    class_id: u16,
    instance_id: u32,
    attribute_id: u16,
) -> Result<Vec<u8>, String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client
        .generic_cip(0x0E, class_id, instance_id, Some(attribute_id), Vec::new())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cip_set_attribute(
    state: tauri::State<'_, AppState>,
    class_id: u16,
    instance_id: u32,
    attribute_id: u16,
    data: Vec<u8>,
) -> Result<(), String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client
        .set_attribute_single(class_id, instance_id, attribute_id, data)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cip_generic(
    state: tauri::State<'_, AppState>,
    service: u8,
    class_id: u16,
    instance_id: u32,
    attribute_id: Option<u16>,
    data: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client
        .generic_cip(service, class_id, instance_id, attribute_id, data)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn forward_open(
    state: tauri::State<'_, AppState>,
    params: mt_enip::ForwardOpenParams,
) -> Result<mt_enip::ForwardOpenResult, String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.forward_open(params).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn forward_close(
    state: tauri::State<'_, AppState>,
    connection_serial: u16,
    vendor_id: u16,
    originator_serial: u32,
    connection_path: Vec<u8>,
) -> Result<(), String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client
        .forward_close(connection_serial, vendor_id, originator_serial, connection_path)
        .await
        .map_err(|e| e.to_string())
}

// ── Implicit/IO pump (ForwardOpen + cyclic T→O reader) ──

#[tauri::command]
async fn io_open(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    params: mt_enip::ForwardOpenParams,
) -> Result<mt_enip::ForwardOpenResult, String> {
    // Stop any existing pump first.
    if let Some(io) = state.io.lock().await.take() {
        if let Some(r) = io.reader {
            r.abort();
        }
    }
    let config = state
        .scanner
        .config
        .lock()
        .await
        .clone()
        .ok_or("Not connected")?;
    let conn = mt_enip::IoConnection::open(&config, params)
        .await
        .map_err(|e| e.to_string())?;
    let result = conn.result.clone();
    let app_io = app.clone();
    let reader = tokio::spawn(async move {
        let mut conn = conn;
        loop {
            match conn.next_sample().await {
                Ok(sample) => {
                    let _ = app_io.emit("enip://io", &sample);
                }
                Err(_) => break,
            }
        }
    });
    *state.io.lock().await = Some(IoState { reader: Some(reader) });
    Ok(result)
}

#[tauri::command]
async fn io_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Some(io) = state.io.lock().await.take() {
        if let Some(r) = io.reader {
            r.abort();
        }
    }
    Ok(())
}

// ── Device Simulator (drive-style control panel window) ──

#[derive(Clone, serde::Serialize)]
struct SimInfo {
    port: u16,
}

/// One member of a frontend-described UDT (name + atomic type + initial value).
#[derive(Debug, Clone, serde::Deserialize)]
struct SimUdtMember {
    name: String,
    data_type: String,
    #[serde(default)]
    value: f64,
}

/// A UDT to expose: a struct tag plus its template definition.
#[derive(Debug, Clone, serde::Deserialize)]
struct SimUdtDef {
    tag: String,
    template_id: u16,
    template_name: String,
    members: Vec<SimUdtMember>,
}

/// Map a lowercase type name ("real"/"dint"/...) to a CipDataType.
fn udt_data_type(s: &str) -> CipDataType {
    match s.to_ascii_lowercase().as_str() {
        "bool" => CipDataType::Bool,
        "sint" => CipDataType::Sint,
        "int" => CipDataType::Int,
        "lint" => CipDataType::Lint,
        "dint" => CipDataType::Dint,
        _ => CipDataType::Real,
    }
}

#[tauri::command]
async fn sim_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    points: Vec<SimPointDef>,
    udts: Vec<SimUdtDef>,
    port: u16,
) -> Result<SimInfo, String> {
    // Stop any existing simulator first.
    if let Some(prev) = state.sim.lock().await.take() {
        prev.stop();
    }

    let app_tick = app.clone();
    let app_write = app.clone();
    let app_log = app.clone();

    let on_tick = Box::new(move |snap: mt_enip::SimSnapshot| {
        let _ = app_tick.emit("sim-tick", &snap);
    });
    let on_write = Box::new(move |key: String, value: f64| {
        let _ = app_write.emit("sim-written", &serde_json::json!({ "key": key, "value": value }));
    });
    let on_log = Box::new(move |entry: EnipServerLogEntry| {
        let _ = app_log.emit("server-log", &entry);
    });

    // Build UDT templates + structure tags from the frontend descriptors.
    let mut templates: Vec<CipTemplateDef> = Vec::new();
    let mut udt_tags: Vec<CipTagDef> = Vec::new();
    for u in &udts {
        let tmpl = CipTemplateDef {
            id: u.template_id,
            name: u.template_name.clone(),
            members: u.members.iter().map(|m| CipTemplateMember {
                name: m.name.clone(),
                data_type: udt_data_type(&m.data_type),
                array_elements: 1,
            }).collect(),
        };
        let values: Vec<f64> = u.members.iter().map(|m| m.value).collect();
        let packed = tmpl.pack_instance(&values);
        udt_tags.push(CipTagDef {
            name: u.tag.clone(),
            data_type: CipDataType::Dint,
            element_count: 1,
            initial_value: CipValue::Bytes(packed),
            template_id: Some(u.template_id),
        });
        templates.push(tmpl);
    }

    let device = mt_enip::EnipSimDevice::start(
        port, "0.0.0.0".into(), points.clone(), templates, udt_tags, on_tick, on_write, on_log,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Share the sim's server tag state so existing server_read_tags / server_write_tag
    // helpers work against the running simulator.
    {
        let mut adapter = state.adapter.lock().await;
        adapter.state = Some(device.server_state());
    }
    *state.sim_defs.lock().await = points;
    let info = SimInfo { port: device.port() };
    *state.sim.lock().await = Some(device);
    Ok(info)
}

#[tauri::command]
async fn sim_stop(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Some(dev) = state.sim.lock().await.take() {
        dev.stop();
    }
    {
        let mut adapter = state.adapter.lock().await;
        adapter.state = None;
        adapter.log_rx = None;
    }
    *state.sim_defs.lock().await = Vec::new();
    if let Some(w) = app.get_webview_window("device-panel") {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
async fn sim_set_value(state: tauri::State<'_, AppState>, key: String, value: f64) -> Result<(), String> {
    let (name, is_bool, dtype) = {
        let defs = state.sim_defs.lock().await;
        defs.iter()
            .find(|d| d.key == key)
            .map(|d| (d.name.clone(), d.is_bool, d.data_type.clone()))
            .ok_or_else(|| format!("Unknown point key: {key}"))?
    };
    let guard = state.sim.lock().await;
    let dev = guard.as_ref().ok_or("Simulator not running")?;
    let cv = if is_bool {
        CipValue::Bool(value >= 0.5)
    } else if dtype == "dint" {
        CipValue::Dint(value.round() as i32)
    } else {
        CipValue::Real(value as f32)
    };
    dev.set_by_name(&name, cv).await;
    Ok(())
}

#[tauri::command]
async fn sim_set_bool(state: tauri::State<'_, AppState>, key: String, active: bool) -> Result<(), String> {
    let name = {
        let defs = state.sim_defs.lock().await;
        defs.iter()
            .find(|d| d.key == key)
            .map(|d| d.name.clone())
            .ok_or_else(|| format!("Unknown point key: {key}"))?
    };
    let guard = state.sim.lock().await;
    let dev = guard.as_ref().ok_or("Simulator not running")?;
    dev.set_by_name(&name, CipValue::Bool(active)).await;
    Ok(())
}

#[tauri::command]
async fn sim_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let guard = state.sim.lock().await;
    Ok(guard.as_ref().map(|d| d.is_running()).unwrap_or(false))
}

#[tauri::command]
async fn open_device_panel(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("device-panel") {
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::webview::WebviewWindowBuilder::new(
        &app,
        "device-panel",
        tauri::WebviewUrl::App("index.html#/sim".into()),
    )
    .title("Device Control Panel")
    .inner_size(440.0, 700.0)
    .min_inner_size(360.0, 480.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("create window: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn list_class_instances(
    state: tauri::State<'_, AppState>,
    class_id: u16,
    max: Option<u32>,
) -> Result<Vec<serde_json::Value>, String> {
    let guard = state.scanner.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let list = client
        .list_instances(class_id, max.unwrap_or(256))
        .await
        .map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|(inst, data)| serde_json::json!({ "instance": inst, "data": data }))
        .collect())
}

// ── Adapter commands ──

#[tauri::command]
async fn server_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: EnipServerConfig,
    tags: Vec<CipTagDef>,
) -> Result<(), String> {
    let (server_state, log_rx, shutdown_tx) = start_enip_server(config, tags, Vec::new())
        .await
        .map_err(|e| e.to_string())?;

    {
        let mut adapter = state.adapter.lock().await;
        adapter.state = Some(server_state);
        adapter.shutdown_tx = Some(shutdown_tx);
        adapter.log_rx = Some(log_rx);
    }

    // Forward server logs to Tauri events
    let app_clone = app.clone();
    let adapter_arc = state.adapter.clone();
    tokio::spawn(async move {
        let mut rx = {
            let mut adapter = adapter_arc.lock().await;
            adapter.log_rx.take()
        };
        if let Some(ref mut rx) = rx {
            while let Some(entry) = rx.recv().await {
                app_clone.emit("server-log", &entry).ok();
            }
        }
    });

    Ok(())
}

#[tauri::command]
async fn server_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut adapter = state.adapter.lock().await;
    if let Some(tx) = adapter.shutdown_tx.take() {
        let _ = tx.send(true);
    }
    adapter.state = None;
    adapter.log_rx = None;
    Ok(())
}

#[tauri::command]
async fn server_read_tags(state: tauri::State<'_, AppState>) -> Result<Vec<(String, String, CipValue)>, String> {
    let adapter = state.adapter.lock().await;
    let server_state = adapter.state.as_ref().ok_or("Server not running")?;
    let tags = server_state.tags.lock().await;
    let result: Vec<(String, String, CipValue)> = tags.iter()
        .map(|(name, entry)| (name.clone(), entry.data_type.as_str().to_string(), entry.value.clone()))
        .collect();
    Ok(result)
}

#[tauri::command]
async fn server_write_tag(
    state: tauri::State<'_, AppState>,
    name: String,
    value: CipValue,
) -> Result<(), String> {
    let adapter = state.adapter.lock().await;
    let server_state = adapter.state.as_ref().ok_or("Server not running")?;
    let mut tags = server_state.tags.lock().await;
    let entry = tags.get_mut(&name).ok_or(format!("Tag not found: {name}"))?;
    entry.value = value;
    Ok(())
}

#[tauri::command]
async fn server_set_tags(
    state: tauri::State<'_, AppState>,
    new_tags: Vec<CipTagDef>,
) -> Result<(), String> {
    let adapter = state.adapter.lock().await;
    let server_state = adapter.state.as_ref().ok_or("Server not running")?;
    let mut tags = server_state.tags.lock().await;
    tags.clear();
    for t in new_tags {
        tags.insert(t.name.clone(), mt_enip::CipTagEntry {
            data_type: t.data_type,
            element_count: t.element_count,
            value: t.initial_value,
            template_id: None,
        });
    }
    Ok(())
}

// ── Utility ──

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

// ── Entry point ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            scanner: ScannerState {
                client: Arc::new(Mutex::new(None)),
                config: Arc::new(Mutex::new(None)),
            },
            adapter: Arc::new(Mutex::new(AdapterServerState {
                state: None,
                shutdown_tx: None,
                log_rx: None,
            })),
            sim: Arc::new(Mutex::new(None)),
            sim_defs: Arc::new(Mutex::new(Vec::new())),
            scanner_tasks: Arc::new(Mutex::new(ScannerTasks { packet_fwd: None })),
            io: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            connect, read_tags, write_tag, browse_tags, disconnect,
            discover_devices, get_device_identity, cip_get_attribute, cip_set_attribute, cip_generic,
            forward_open, forward_close, io_open, io_stop,
            list_class_instances,
            server_start, server_stop, server_read_tags, server_write_tag, server_set_tags,
            sim_start, sim_stop, sim_set_value, sim_set_bool, sim_status, open_device_panel,
            open_url,
            mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
        ])
        .setup(|app| {
            // Default the window to 1280 logical wide × full screen height, centered.
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = window.primary_monitor() {
                    let scale = monitor.scale_factor();
                    let width_phys = (1280.0 * scale).round() as u32;
                    let height_phys = monitor.size().height;
                    let _ = window.set_size(tauri::PhysicalSize::new(width_phys, height_phys));
                    let _ = window.center();
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
