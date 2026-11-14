pub mod core;

use std::sync::Arc;

use mt_ethercat::link::NetInterface;
use mt_ethercat::master::{CycleStats, EntryView, IoImage, LinkSpec, ObjectView, PdoMapView, RegistersView, ScanResult, StateView};
use mt_ethercat::sii::Sii;
use mt_ethercat::sim::{SimAction, SimState};
use tauri::{Emitter, Manager};

use crate::core::{Core, RawAccess, SdoValue};

type State<'a> = tauri::State<'a, Arc<Core>>;

#[tauri::command]
fn ecat_interfaces(core: State<'_>) -> Vec<NetInterface> {
    core.interfaces()
}

#[tauri::command]
fn ecat_raw_access(core: State<'_>) -> RawAccess {
    core.raw_access()
}

#[tauri::command]
async fn ecat_grant_raw_access(core: State<'_>) -> Result<(), String> {
    let core = core.inner().clone();
    tokio::task::spawn_blocking(move || core.grant_raw_access()).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn ecat_connect(core: State<'_>, link: LinkSpec) -> Result<ScanResult, String> {
    core.connect(link).await
}

#[tauri::command]
async fn ecat_disconnect(core: State<'_>) -> Result<(), String> {
    core.disconnect().await;
    Ok(())
}

#[tauri::command]
async fn ecat_rescan(core: State<'_>) -> Result<ScanResult, String> {
    core.rescan().await
}

#[tauri::command]
async fn ecat_states(core: State<'_>) -> Result<Vec<StateView>, String> {
    core.states().await
}

#[tauri::command]
async fn ecat_request_state(core: State<'_>, state: String, positions: Vec<usize>) -> Result<Vec<StateView>, String> {
    core.request_state(state, positions).await
}

#[tauri::command]
async fn ecat_registers(core: State<'_>, position: usize) -> Result<RegistersView, String> {
    core.registers(position).await
}

#[tauri::command]
async fn ecat_write_register(core: State<'_>, position: usize, addr: u16, hex: String) -> Result<(), String> {
    core.write_register(position, addr, hex).await
}

#[tauri::command]
async fn ecat_reset_counters(core: State<'_>, position: usize) -> Result<(), String> {
    core.reset_counters(position).await
}

#[tauri::command]
async fn ecat_sii(core: State<'_>, position: usize) -> Result<Sii, String> {
    core.sii(position).await
}

#[tauri::command]
async fn ecat_eeprom(core: State<'_>, position: usize) -> Result<Vec<u8>, String> {
    core.eeprom(position).await
}

#[tauri::command]
async fn ecat_pdo_map(core: State<'_>, position: usize) -> Result<PdoMapView, String> {
    core.pdo_map(position).await
}

#[tauri::command]
async fn ecat_objects(core: State<'_>, position: usize) -> Result<Vec<ObjectView>, String> {
    core.objects(position).await
}

#[tauri::command]
async fn ecat_object_entries(core: State<'_>, position: usize, index: u16) -> Result<Vec<EntryView>, String> {
    core.object_entries(position, index).await
}

#[tauri::command]
async fn ecat_sdo_read(core: State<'_>, position: usize, index: u16, sub: u8, data_type: u16) -> Result<SdoValue, String> {
    core.sdo_read(position, index, sub, data_type).await
}

#[tauri::command]
async fn ecat_sdo_write(core: State<'_>, position: usize, index: u16, sub: u8, data_type: u16, bits: u16, text: String) -> Result<(), String> {
    core.sdo_write(position, index, sub, data_type, bits, text).await
}

#[tauri::command]
async fn ecat_write_output(core: State<'_>, position: usize, bit_offset: usize, bits: usize, data_type: u16, text: String) -> Result<(), String> {
    core.write_output(position, bit_offset, bits, data_type, text).await
}

#[tauri::command]
async fn ecat_set_cycle(core: State<'_>, ms: f64) -> Result<(), String> {
    core.set_cycle(ms).await
}

#[tauri::command]
async fn ecat_cycle(core: State<'_>, run: bool) -> Result<(), String> {
    core.cycle(run).await
}

#[tauri::command]
async fn ecat_io(core: State<'_>) -> Result<(CycleStats, Vec<IoImage>), String> {
    core.io().await
}

#[tauri::command]
async fn ecat_capture(core: State<'_>, frames: usize) -> Result<(), String> {
    core.capture(frames).await
}

#[tauri::command]
async fn sim_state(core: State<'_>) -> Result<Option<SimState>, String> {
    Ok(core.sim_state().await)
}

#[tauri::command]
async fn sim_action(core: State<'_>, action: SimAction) -> Result<(), String> {
    core.sim_action(action).await
}

/// Writes bytes to a path the user picked in a save panel (EEPROM backups, pcap captures).
#[tauri::command]
fn save_bytes(path: String, bytes: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, bytes).map_err(|e| format!("Could not write {path}: {e}"))
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(Arc::new(Core::new(Arc::new(move |event, payload| {
                let _ = handle.emit(event, payload);
            }))));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close the session (and the raw socket) with the window.
            if let tauri::WindowEvent::Destroyed = event
                && let Some(core) = window.try_state::<Arc<Core>>() {
                    let core = core.inner().clone();
                    tauri::async_runtime::block_on(core.disconnect());
                }
        })
        .invoke_handler(tauri::generate_handler![
            ecat_interfaces, ecat_raw_access, ecat_grant_raw_access, ecat_connect, ecat_disconnect, ecat_rescan,
            ecat_states, ecat_request_state, ecat_registers, ecat_write_register, ecat_reset_counters,
            ecat_sii, ecat_eeprom, ecat_pdo_map, ecat_objects, ecat_object_entries, ecat_sdo_read, ecat_sdo_write,
            ecat_write_output, ecat_set_cycle, ecat_cycle, ecat_io, ecat_capture,
            sim_state, sim_action, save_bytes, open_url,
            mt_store::request_review, mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_store::store_activate, mt_store::store_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
