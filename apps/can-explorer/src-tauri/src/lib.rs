pub mod core;

use std::collections::HashMap;
use std::sync::Arc;

use mt_can::protocols::Layer;
use mt_can::sim::{SimAction, SimState};
use mt_can::{InterfaceConfig, PortInfo};
use tauri::{Emitter, Manager};

use crate::core::{ConnInfo, Core, DbcView, FrameIn, FrameView, ReplayInfo, ReplayRequest};

type State<'a> = tauri::State<'a, Arc<Core>>;

#[tauri::command]
fn can_list_ports(core: State<'_>) -> Result<Vec<PortInfo>, String> {
    core.list_ports()
}

#[tauri::command]
async fn can_list_gsusb(core: State<'_>) -> Result<Vec<mt_can::gsusb::GsUsbDevice>, String> {
    core.list_gsusb().await
}

#[tauri::command]
fn can_list_socketcan(core: State<'_>) -> Vec<String> {
    core.list_socketcan()
}

#[tauri::command]
async fn can_connect(core: State<'_>, config: InterfaceConfig) -> Result<ConnInfo, String> {
    core.connect(config).await
}

#[tauri::command]
async fn can_disconnect(core: State<'_>) -> Result<(), String> {
    core.disconnect().await;
    Ok(())
}

#[tauri::command]
async fn can_send(core: State<'_>, frame: FrameIn) -> Result<(), String> {
    core.send(frame).await
}

#[tauri::command]
async fn can_send_text(core: State<'_>, text: String) -> Result<(), String> {
    core.send_text(text).await
}

#[tauri::command]
async fn can_periodic_start(core: State<'_>, key: String, frame: FrameIn, period_ms: u64, count: Option<u64>) -> Result<(), String> {
    core.periodic_start(key, frame, period_ms, count).await
}

#[tauri::command]
async fn can_periodic_stop(core: State<'_>, key: String) -> Result<(), String> {
    core.periodic_stop(&key).await;
    Ok(())
}

#[tauri::command]
async fn can_set_layer(core: State<'_>, layer: Layer) -> Result<(), String> {
    core.set_layer(layer).await;
    Ok(())
}

#[tauri::command]
async fn dbc_load(core: State<'_>, name: String, text: String) -> Result<DbcView, String> {
    core.dbc_load(name, text).await
}

#[tauri::command]
async fn dbc_load_sim(core: State<'_>) -> Result<DbcView, String> {
    core.dbc_load_sim().await
}

#[tauri::command]
fn dbc_sim_text(core: State<'_>) -> String {
    core.sim_dbc_text().to_string()
}

#[tauri::command]
async fn dbc_clear(core: State<'_>) -> Result<(), String> {
    core.dbc_clear().await;
    Ok(())
}

#[tauri::command]
async fn dbc_encode(core: State<'_>, message: String, values: HashMap<String, f64>) -> Result<FrameView, String> {
    core.dbc_encode(message, values).await
}

#[tauri::command]
async fn trace_decode(core: State<'_>, text: String) -> Result<Vec<FrameView>, String> {
    core.decode_frames(text).await
}

#[tauri::command]
async fn replay_start(core: State<'_>, request: ReplayRequest) -> Result<ReplayInfo, String> {
    core.replay_start(request).await
}

#[tauri::command]
async fn replay_stop(core: State<'_>) -> Result<(), String> {
    core.replay_stop().await;
    Ok(())
}

#[tauri::command]
async fn sim_start(core: State<'_>) -> Result<(), String> {
    core.sim_start().await
}

#[tauri::command]
async fn sim_stop(core: State<'_>) -> Result<(), String> {
    core.sim_stop().await;
    Ok(())
}

#[tauri::command]
async fn sim_state(core: State<'_>) -> Result<Option<SimState>, String> {
    core.sim_state().await
}

#[tauri::command]
async fn sim_action(core: State<'_>, action: SimAction) -> Result<(), String> {
    core.sim_action(action).await
}

/// Opens an https page (the privacy policy) in the system browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Only https links can be opened.".into());
    }
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(&url).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open and save panels plus reads and writes of the paths the user picked (the only file
        // access the sandboxed build has): DBC files, trace import, export and recording.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(Arc::new(Core::new(Arc::new(move |event, payload| {
                let _ = handle.emit(event, payload);
            }))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            can_list_ports, can_list_gsusb, can_list_socketcan, can_connect, can_disconnect, can_send, can_send_text,
            can_periodic_start, can_periodic_stop, can_set_layer,
            dbc_load, dbc_load_sim, dbc_sim_text, dbc_clear, dbc_encode, trace_decode,
            replay_start, replay_stop,
            sim_start, sim_stop, sim_state, sim_action,
            open_url,
            mt_store::request_review, mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_store::store_activate, mt_store::store_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
