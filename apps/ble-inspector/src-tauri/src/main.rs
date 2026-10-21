use mt_ble::{BleDevice, BleManager, BleService};

mod system;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

struct AppState {
    manager: Arc<BleManager>,
}

#[tauri::command]
async fn init_ble(state: State<'_, AppState>) -> Result<(), String> {
    state.manager.init().await
}

#[tauri::command]
async fn start_scan(state: State<'_, AppState>) -> Result<(), String> {
    state.manager.start_scan().await
}

#[tauri::command]
async fn stop_scan(state: State<'_, AppState>) -> Result<(), String> {
    state.manager.stop_scan().await
}

#[tauri::command]
async fn is_scanning(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.manager.is_scanning().await)
}

#[tauri::command]
async fn get_devices(state: State<'_, AppState>) -> Result<Vec<BleDevice>, String> {
    Ok(state.manager.get_devices().await)
}

#[tauri::command]
async fn connect_device(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.manager.connect(&id).await
}

#[tauri::command]
async fn disconnect_device(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.manager.disconnect(&id).await
}

#[tauri::command]
async fn discover_services(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<BleService>, String> {
    state.manager.discover_services(&id).await
}

#[tauri::command]
async fn read_characteristic(
    state: State<'_, AppState>,
    device_id: String,
    char_uuid: String,
) -> Result<String, String> {
    state
        .manager
        .read_characteristic(&device_id, &char_uuid)
        .await
}

#[tauri::command]
async fn write_characteristic(
    state: State<'_, AppState>,
    device_id: String,
    char_uuid: String,
    value: String,
) -> Result<(), String> {
    state
        .manager
        .write_characteristic(&device_id, &char_uuid, &value)
        .await
}

#[tauri::command]
async fn subscribe_characteristic(
    state: State<'_, AppState>,
    device_id: String,
    char_uuid: String,
) -> Result<(), String> {
    state
        .manager
        .subscribe_characteristic(&device_id, &char_uuid)
        .await
}

#[tauri::command]
async fn unsubscribe_characteristic(
    state: State<'_, AppState>,
    device_id: String,
    char_uuid: String,
) -> Result<(), String> {
    state
        .manager
        .unsubscribe_characteristic(&device_id, &char_uuid)
        .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(256);

    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            manager: Arc::new(BleManager::new(event_tx)),
        })
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let _ = handle.emit("ble-event", &event);
                }
            });

            let state = app.state::<AppState>();
            let manager = state.manager.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = manager.init().await {
                    log::warn!("BLE init failed: {}", e);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            init_ble,
            start_scan,
            stop_scan,
            is_scanning,
            get_devices,
            connect_device,
            disconnect_device,
            discover_services,
            read_characteristic,
            write_characteristic,
            subscribe_characteristic,
            unsubscribe_characteristic,
            system::open_url,
            mt_store::store_status,
            mt_store::store_purchase,
            mt_store::store_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}
