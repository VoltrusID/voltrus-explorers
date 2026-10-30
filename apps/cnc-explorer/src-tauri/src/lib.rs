pub mod capture;
pub mod core;

use std::sync::Arc;

use focas_proto::{
    AlarmMessage, AlarmType, FocasConfig, MacroValue, NumberedValues, PmcArea, PmcDataType, PmcValue, ProgramEntry,
    SysInfo,
};
use mt_mtconnect::{AgentConfig, AssetList, ConditionLevel, Devices, SimConfig, SimSnapshot, Streams};
use tauri::{Emitter, Manager};

use crate::capture::{CaptureBundle, MachineState};
use crate::core::{Core, FanucPoll, FocasSimAction, FocasSimConfig, FocasSimView, MtcSimInfo};

type State<'a> = tauri::State<'a, Arc<Core>>;

// ── MTConnect client ──

#[tauri::command]
async fn mtc_connect(core: State<'_>, config: AgentConfig) -> Result<Devices, String> {
    core.mtc_connect(config).await
}

#[tauri::command]
async fn mtc_disconnect(core: State<'_>) -> Result<(), String> {
    core.mtc_disconnect().await;
    Ok(())
}

#[tauri::command]
async fn mtc_current(core: State<'_>, path: Option<String>) -> Result<Streams, String> {
    core.mtc_current(path).await
}

#[tauri::command]
async fn mtc_assets(core: State<'_>) -> Result<AssetList, String> {
    core.mtc_assets().await
}

#[tauri::command]
async fn mtc_stream_start(core: State<'_>, interval_ms: u64) -> Result<(), String> {
    core.mtc_stream_start(interval_ms).await
}

#[tauri::command]
async fn mtc_stream_stop(core: State<'_>) -> Result<(), String> {
    core.mtc_stream_stop().await;
    Ok(())
}

// ── FANUC client ──

#[tauri::command]
async fn fanuc_connect(core: State<'_>, config: FocasConfig) -> Result<SysInfo, String> {
    core.fanuc_connect(config).await
}

#[tauri::command]
async fn fanuc_disconnect(core: State<'_>) -> Result<(), String> {
    core.fanuc_disconnect().await;
    Ok(())
}

#[tauri::command]
async fn fanuc_poll(core: State<'_>, macros: Option<Vec<u32>>) -> Result<FanucPoll, String> {
    core.fanuc_poll(macros.unwrap_or_default()).await
}

/// Records a read-only probe of the connected CNC and returns the capture, report and summary to save.
#[tauri::command]
async fn fanuc_capture(
    core: State<'_>,
    app: tauri::AppHandle,
    state: MachineState,
    macros: Option<Vec<u32>>,
) -> Result<CaptureBundle, String> {
    let version = app.package_info().version.to_string();
    core.fanuc_capture(state, macros.unwrap_or_default(), &version).await
}

#[tauri::command]
async fn fanuc_alarms(core: State<'_>) -> Result<Vec<AlarmMessage>, String> {
    core.fanuc_alarms().await
}

#[tauri::command]
async fn fanuc_programs(core: State<'_>) -> Result<Vec<ProgramEntry>, String> {
    core.fanuc_programs().await
}

#[tauri::command]
async fn fanuc_read_macro(core: State<'_>, first: u32, last: u32) -> Result<Vec<MacroValue>, String> {
    core.fanuc_read_macro(first, last).await
}

#[tauri::command]
async fn fanuc_write_macro(core: State<'_>, number: u32, value: f64) -> Result<(), String> {
    core.fanuc_write_macro(number, value).await
}

#[tauri::command]
async fn fanuc_read_params(core: State<'_>, first: u32, last: u32, axis: i32) -> Result<Vec<NumberedValues>, String> {
    core.fanuc_read_params(first, last, axis).await
}

#[tauri::command]
async fn fanuc_read_diag(core: State<'_>, first: u32, last: u32, axis: i32) -> Result<Vec<NumberedValues>, String> {
    core.fanuc_read_diag(first, last, axis).await
}

#[tauri::command]
async fn fanuc_read_pmc(
    core: State<'_>,
    area: PmcArea,
    data_type: PmcDataType,
    first: u32,
    count: u32,
) -> Result<Vec<PmcValue>, String> {
    core.fanuc_read_pmc(area, data_type, first, count).await
}

// ── MTConnect agent simulator ──

#[tauri::command]
async fn mtc_sim_start(core: State<'_>, config: SimConfig) -> Result<MtcSimInfo, String> {
    core.mtc_sim_start(config).await
}

#[tauri::command]
async fn mtc_sim_stop(core: State<'_>) -> Result<(), String> {
    core.mtc_sim_stop().await;
    Ok(())
}

#[tauri::command]
async fn mtc_sim_snapshot(core: State<'_>) -> Result<SimSnapshot, String> {
    core.mtc_sim_snapshot().await
}

#[tauri::command]
async fn mtc_sim_set_value(core: State<'_>, id: String, text: String) -> Result<(), String> {
    core.mtc_sim_set_value(id, text).await
}

#[tauri::command]
async fn mtc_sim_release(core: State<'_>, id: String) -> Result<(), String> {
    core.mtc_sim_release(id).await
}

#[tauri::command]
async fn mtc_sim_raise(
    core: State<'_>,
    id: String,
    level: ConditionLevel,
    code: String,
    message: String,
) -> Result<(), String> {
    core.mtc_sim_raise(id, level, code, message).await
}

#[tauri::command]
async fn mtc_sim_clear(core: State<'_>, id: String) -> Result<(), String> {
    core.mtc_sim_clear(id).await
}

#[tauri::command]
async fn mtc_sim_set_paused(core: State<'_>, paused: bool) -> Result<(), String> {
    core.mtc_sim_set_paused(paused).await
}

#[tauri::command]
async fn mtc_sim_restart_agent(core: State<'_>) -> Result<(), String> {
    core.mtc_sim_restart_agent().await
}

// ── FANUC CNC simulator ──

#[tauri::command]
async fn focas_sim_start(core: State<'_>, config: FocasSimConfig) -> Result<u16, String> {
    core.focas_sim_start(config).await
}

#[tauri::command]
async fn focas_sim_stop(core: State<'_>) -> Result<(), String> {
    core.focas_sim_stop().await;
    Ok(())
}

#[tauri::command]
async fn focas_sim_view(core: State<'_>) -> Result<FocasSimView, String> {
    core.focas_sim_view().await
}

#[tauri::command]
async fn focas_sim_action(core: State<'_>, action: FocasSimAction) -> Result<(), String> {
    core.focas_sim_action(action).await
}

#[tauri::command]
async fn focas_sim_raise_alarm(core: State<'_>, number: i32, alarm_type: AlarmType, message: String) -> Result<(), String> {
    core.focas_sim_raise_alarm(number, alarm_type, message).await
}

#[tauri::command]
async fn focas_sim_clear_alarm(core: State<'_>, number: Option<i32>) -> Result<(), String> {
    core.focas_sim_clear_alarm(number).await
}

#[tauri::command]
async fn focas_sim_set_macro(core: State<'_>, number: u32, value: f64) -> Result<(), String> {
    core.focas_sim_set_macro(number, value).await
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
        // Native save panel + writes to the path the user picked (the only file access the sandboxed
        // App Store build has), used by the log and CSV export.
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
            mtc_connect, mtc_disconnect, mtc_current, mtc_assets, mtc_stream_start, mtc_stream_stop,
            fanuc_connect, fanuc_disconnect, fanuc_poll, fanuc_capture, fanuc_alarms, fanuc_programs, fanuc_read_macro,
            fanuc_write_macro, fanuc_read_params, fanuc_read_diag, fanuc_read_pmc,
            mtc_sim_start, mtc_sim_stop, mtc_sim_snapshot, mtc_sim_set_value, mtc_sim_release, mtc_sim_raise,
            mtc_sim_clear, mtc_sim_set_paused, mtc_sim_restart_agent,
            focas_sim_start, focas_sim_stop, focas_sim_view, focas_sim_action, focas_sim_raise_alarm,
            focas_sim_clear_alarm, focas_sim_set_macro,
            open_url,
            mt_store::request_review, mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_store::store_activate, mt_store::store_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
