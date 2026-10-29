pub mod core;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::core::{
    AckView, AlarmView, CommandView, ConnInfo, Core, EcRow, EcWriteView, EstablishView, HostConnect, OnlineView, ParamIn,
    RawReply, SimAction, SimStart, SimView, SvRow, SvValue, WizardRequest, WizardStep, ControlView,
};

type State<'a> = tauri::State<'a, Arc<Core>>;

// ── Host (client) ──

#[tauri::command]
async fn gem_connect(core: State<'_>, config: HostConnect) -> Result<ConnInfo, String> {
    core.connect(config).await
}

#[tauri::command]
async fn gem_disconnect(core: State<'_>) -> Result<(), String> {
    core.disconnect().await;
    Ok(())
}

#[tauri::command]
async fn gem_are_you_there(core: State<'_>) -> Result<String, String> {
    core.are_you_there().await
}

#[tauri::command]
async fn gem_establish(core: State<'_>) -> Result<EstablishView, String> {
    core.establish().await
}

#[tauri::command]
async fn gem_online(core: State<'_>) -> Result<OnlineView, String> {
    core.online().await
}

#[tauri::command]
async fn gem_offline(core: State<'_>) -> Result<OnlineView, String> {
    core.offline().await
}

#[tauri::command]
async fn gem_control(core: State<'_>) -> Result<Option<ControlView>, String> {
    core.control().await
}

#[tauri::command]
async fn gem_sv_names(core: State<'_>) -> Result<Vec<SvRow>, String> {
    core.sv_names().await
}

#[tauri::command]
async fn gem_sv_read(core: State<'_>, ids: Vec<u64>) -> Result<Vec<SvValue>, String> {
    core.sv_read(ids).await
}

#[tauri::command]
async fn gem_ec_table(core: State<'_>) -> Result<Vec<EcRow>, String> {
    core.ec_table().await
}

#[tauri::command]
async fn gem_ec_write(core: State<'_>, id: u64, text: String) -> Result<EcWriteView, String> {
    core.ec_write(id, text).await
}

#[tauri::command]
async fn gem_report_wizard(core: State<'_>, request: WizardRequest) -> Result<Vec<WizardStep>, String> {
    core.report_wizard(request).await
}

#[tauri::command]
async fn gem_report_delete(core: State<'_>, rptid: u64) -> Result<AckView, String> {
    core.report_delete(rptid).await
}

#[tauri::command]
async fn gem_events_enable(core: State<'_>, enable: bool, ceids: Vec<u64>) -> Result<AckView, String> {
    core.events_enable(enable, ceids).await
}

#[tauri::command]
async fn gem_alarm_list(core: State<'_>) -> Result<Vec<AlarmView>, String> {
    core.alarm_list().await
}

#[tauri::command]
async fn gem_alarm_enable(core: State<'_>, enable: bool, alid: Option<u64>) -> Result<AckView, String> {
    core.alarm_enable(enable, alid).await
}

#[tauri::command]
async fn gem_remote_command(core: State<'_>, rcmd: String, params: Vec<ParamIn>) -> Result<CommandView, String> {
    core.remote_command(rcmd, params).await
}

#[tauri::command]
async fn gem_ppid_list(core: State<'_>) -> Result<Vec<String>, String> {
    core.ppid_list().await
}

#[tauri::command]
async fn gem_terminal_send(core: State<'_>, tid: u8, text: String) -> Result<AckView, String> {
    core.terminal_send(tid, text).await
}

#[tauri::command]
async fn gem_send_sml(core: State<'_>, sml: String) -> Result<RawReply, String> {
    core.send_sml(sml).await
}

// ── Equipment simulator ──

#[tauri::command]
async fn sim_start(core: State<'_>, config: SimStart) -> Result<u16, String> {
    core.sim_start(config).await
}

#[tauri::command]
async fn sim_stop(core: State<'_>) -> Result<(), String> {
    core.sim_stop().await;
    Ok(())
}

#[tauri::command]
async fn sim_view(core: State<'_>) -> Result<SimView, String> {
    core.sim_view().await
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
        // Native save panel + writes to the path the user picked (the only file access the sandboxed
        // App Store build has), used by the message log and CSV export.
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
            gem_connect, gem_disconnect, gem_are_you_there, gem_establish, gem_online, gem_offline, gem_control,
            gem_sv_names, gem_sv_read, gem_ec_table, gem_ec_write, gem_report_wizard, gem_report_delete,
            gem_events_enable, gem_alarm_list, gem_alarm_enable, gem_remote_command, gem_ppid_list,
            gem_terminal_send, gem_send_sml,
            sim_start, sim_stop, sim_view, sim_action,
            open_url,
            mt_store::request_review, mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_store::store_activate, mt_store::store_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
