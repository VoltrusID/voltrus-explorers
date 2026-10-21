// Release builds on Windows: GUI subsystem, no console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod system;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use mt_opcua::{
    BrowseChild, EndpointInfo, MonitoredItemDef, NamespaceInfo, NotificationSink, NotificationValue,
    OpcUaClient, OpcUaConfig, OpcUaNode, OpcUaProtocolClient, OpcUaServer, OpcUaValue, OpcUaStore,
    ReferenceInfo, ServerInfo, ServerNodeDef, SimDeviceProfile, SubscriptionConfig, AttributeRead,
    CertInfo, start_simulation,
};
use mt_protocol::ProtocolClient;
use tauri::{AppHandle, Emitter, Manager};

struct AppState {
    client: Arc<Mutex<Option<OpcUaClient>>>,
    server: Arc<Mutex<OpcUaServer>>,
    store: OpcUaStore,
    sim_running: Arc<AtomicBool>,
    sim_values: Arc<Mutex<Vec<f64>>>,
}

// ── Client commands ──

#[tauri::command]
async fn connect(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
    config: OpcUaConfig,
) -> Result<(), String> {
    let client = OpcUaClient::connect(config).await.map_err(|e| e.to_string())?;
    {
        // Wire subscription notifications → webview event before any use.
        let inner = client.inner();
        let mut g = inner.lock().await;
        g.set_notification_sink(Arc::new(NotificationForwarder { app }));
    }
    *state.client.lock().await = Some(client);
    Ok(())
}

/// Forwards OPC UA subscription data-change notifications to the webview.
struct NotificationForwarder {
    app: AppHandle,
}

impl NotificationSink for NotificationForwarder {
    fn notify(&self, value: NotificationValue) {
        let _ = self.app.emit("opcua://notification", value);
    }
}

#[tauri::command]
async fn read_tags(
    state: tauri::State<'_, AppState>,
    tags: Vec<OpcUaNode>,
) -> Result<Vec<OpcUaValue>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.read(&tags).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: OpcUaNode,
    value: OpcUaValue,
) -> Result<(), String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.write(&tag, value).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.client.lock().await;
    if let Some(client) = guard.take() {
        client.disconnect().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn browse_nodes(state: tauri::State<'_, AppState>) -> Result<Vec<OpcUaNode>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let inner_guard = inner.lock().await;
    inner_guard.browse_nodes().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_endpoints(url: String) -> Result<Vec<EndpointInfo>, String> {
    OpcUaProtocolClient::get_endpoints(&url).await
}

#[tauri::command]
async fn find_servers(url: String) -> Result<Vec<ServerInfo>, String> {
    OpcUaProtocolClient::find_servers(&url).await
}

#[tauri::command]
async fn browse_children(
    state: tauri::State<'_, AppState>,
    node_id: String,
) -> Result<Vec<BrowseChild>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let g = inner.lock().await;
    g.browse_children(&node_id).await
}

#[tauri::command]
async fn browse_references(
    state: tauri::State<'_, AppState>,
    node_id: String,
) -> Result<Vec<ReferenceInfo>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let g = inner.lock().await;
    g.browse_references(&node_id).await
}

#[tauri::command]
async fn read_namespaces(state: tauri::State<'_, AppState>) -> Result<Vec<NamespaceInfo>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let g = inner.lock().await;
    g.read_namespaces().await
}

#[tauri::command]
async fn read_attributes(
    state: tauri::State<'_, AppState>,
    node_id: String,
) -> Result<Vec<AttributeRead>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let g = inner.lock().await;
    g.read_attributes(&node_id).await
}

#[tauri::command]
async fn create_subscription(
    state: tauri::State<'_, AppState>,
    cfg: SubscriptionConfig,
) -> Result<u32, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let mut g = inner.lock().await;
    g.create_subscription(cfg).await
}

#[tauri::command]
async fn create_monitored_items(
    state: tauri::State<'_, AppState>,
    subscription_id: u32,
    items: Vec<MonitoredItemDef>,
) -> Result<usize, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let g = inner.lock().await;
    g.create_monitored_items(subscription_id, items).await
}

#[tauri::command]
async fn delete_subscription(
    state: tauri::State<'_, AppState>,
    subscription_id: u32,
) -> Result<(), String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let inner = client.inner();
    let mut g = inner.lock().await;
    g.delete_subscription(subscription_id).await
}

// ── Server commands ──

#[tauri::command]
async fn server_start(state: tauri::State<'_, AppState>, port: u16) -> Result<String, String> {
    let mut server = state.server.lock().await;
    let nodes = ServerNodeDef::default_nodes();
    let url = server.start(port, nodes).await.map_err(|e| e.to_string())?;
    for &(nid, name, dtype) in DEFAULT_NODES {
        let val = if dtype == "Boolean" { serde_json::json!(false) } else { serde_json::json!(0.0) };
        state.store.init_node(nid, name, dtype, val).await;
    }
    state.store.server_running.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(url)
}

#[tauri::command]
async fn server_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut server = state.server.lock().await;
    server.stop().map_err(|e| e.to_string())?;
    state.store.server_running.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
async fn server_read_nodes(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let server = state.server.lock().await;
    let node_defs = ServerNodeDef::default_nodes();
    let values = server.read_all_nodes(&node_defs);
    Ok(values.iter().map(|v| {
        let dtype = DEFAULT_NODES.iter()
            .find(|(nid, _, _)| *nid == v.node_id)
            .map(|(_, _, dt)| *dt)
            .unwrap_or("Double");
        serde_json::json!({
            "node_id": v.node_id,
            "name": v.name,
            "data_type": dtype,
            "value": v.value,
            "timestamp": v.timestamp,
        })
    }).collect())
}

#[tauri::command]
async fn server_write_node(
    state: tauri::State<'_, AppState>,
    node_id: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let server = state.server.lock().await;
    let data_type = DEFAULT_NODES.iter()
        .find(|(nid, _, _)| *nid == node_id)
        .map(|(_, _, dt)| *dt)
        .unwrap_or("Double");
    server.write_node(&node_id, value.clone(), data_type).map_err(|e| e.to_string())?;
    state.store.write(&node_id, value).await;
    Ok(())
}

// ── Simulation commands ──

#[tauri::command]
async fn sim_start(
    state: tauri::State<'_, AppState>,
    profile: SimDeviceProfile,
    interval_ms: u64,
) -> Result<(), String> {
    if state.sim_running.load(std::sync::atomic::Ordering::SeqCst) {
        sim_stop_inner(state.inner()).await;
    }

    // Writer that pushes simulated values through the server node manager so
    // active subscriptions get data-change notifications (direct address-space
    // writes would leave the main table showing static values).
    let writer: Option<Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>> = {
        let server = state.server.lock().await;
        server.node_writer()
    };

    start_simulation(
        state.store.clone(),
        profile,
        interval_ms,
        state.sim_running.clone(),
        state.sim_values.clone(),
        writer,
    ).await;
    Ok(())
}

#[tauri::command]
async fn sim_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    sim_stop_inner(state.inner()).await;
    Ok(())
}

#[tauri::command]
async fn sim_read_values(state: tauri::State<'_, AppState>) -> Result<Vec<f64>, String> {
    Ok(state.sim_values.lock().await.clone())
}

async fn sim_stop_inner(state: &AppState) {
    state.sim_running.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut vals = state.sim_values.lock().await;
    vals.clear();
}

// ── Certificate commands ──

#[tauri::command]
fn cert_list() -> Vec<CertInfo> {
    mt_opcua::cert::cert_list(&mt_opcua::pki::all_dirs())
}

#[tauri::command]
fn cert_trust(thumbprint: String) -> Result<(), String> {
    mt_opcua::cert::cert_trust(&mt_opcua::pki::all_dirs(), &thumbprint).map_err(|e| e.to_string())
}

#[tauri::command]
fn cert_reject(thumbprint: String) -> Result<(), String> {
    mt_opcua::cert::cert_reject(&mt_opcua::pki::all_dirs(), &thumbprint).map_err(|e| e.to_string())
}

#[tauri::command]
fn cert_generate() -> Result<(), String> {
    mt_opcua::cert::cert_generate(&mt_opcua::pki::all_dirs()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            client: Arc::new(Mutex::new(None)),
            server: Arc::new(Mutex::new(OpcUaServer::new())),
            store: OpcUaStore::new(),
            sim_running: Arc::new(AtomicBool::new(false)),
            sim_values: Arc::new(Mutex::new(Vec::new())),
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            read_tags,
            write_tag,
            disconnect,
            browse_nodes,
            browse_children,
            browse_references,
            read_namespaces,
            read_attributes,
            get_endpoints,
            find_servers,
            create_subscription,
            create_monitored_items,
            delete_subscription,
            server_start,
            server_stop,
            server_read_nodes,
            server_write_node,
            sim_start,
            sim_stop,
            sim_read_values,
            cert_list,
            cert_trust,
            cert_reject,
            cert_generate,
            system::open_url,
            mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
        ])
        .setup(|app| {
            // Certificates and keys live in the app data directory: inside the container in the sandboxed
            // App Store build, and they survive restarts (the temp dir does not).
            let pki_root = app.path().app_data_dir()?.join("pki");
            std::fs::create_dir_all(&pki_root)?;
            mt_opcua::pki::set_root(pki_root);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}

const DEFAULT_NODES: &[(&str, &str, &str)] = &[
    ("ns=2;s=Reactor.Temperature", "Temperature", "Double"),
    ("ns=2;s=Reactor.Pressure", "Pressure", "Double"),
    ("ns=2;s=Reactor.Level", "Level", "Double"),
    ("ns=2;s=Reactor.AgitatorSpeed", "AgitatorSpeed", "Double"),
    ("ns=2;s=FeedFlow_Rate", "FeedFlow", "Double"),
    ("ns=2;s=ProductFlow_Rate", "ProductFlow", "Double"),
    ("ns=2;s=pH_Value", "pH", "Double"),
    ("ns=2;s=Reactor.Status.Running", "Running", "Boolean"),
    ("ns=2;s=Boiler.SteamPressure", "SteamPressure", "Double"),
    ("ns=2;s=Boiler.WaterLevel", "WaterLevel", "Double"),
    ("ns=2;s=Boiler.FuelFlow", "FuelFlow", "Double"),
    ("ns=2;s=Boiler.FeedWaterTemp", "FeedWaterTemp", "Double"),
    ("ns=2;s=Boiler.FlueGasTemp", "FlueGasTemp", "Double"),
    ("ns=2;s=Boiler.BurnerActive", "BurnerActive", "Boolean"),
    ("ns=2;s=Boiler.Efficiency", "Efficiency", "Double"),
    ("ns=2;s=Boiler.SafetyValve", "SafetyValve", "Boolean"),
];
