mod state;
mod master;
mod slave;
mod simulation;
mod network;
mod scan_targets;
mod smoke;
mod system;

use state::*;

/// `--smoke [file]` or `--smoke-license [file]`: run a headless self-test instead of the app (see [`smoke`]).
/// Returns the exit code.
pub fn smoke_from_args(args: &[String]) -> Option<i32> {
    let (at, check) = args.iter().enumerate().find_map(|(i, a)| match a.as_str() {
        "--smoke" => Some((i, smoke::Check::Local)),
        "--smoke-license" => Some((i, smoke::Check::License)),
        _ => None,
    })?;
    let out = args.get(at + 1).filter(|a| !a.starts_with("--")).map(String::as_str);
    Some(smoke::run(check, &app_version(), out))
}

/// The version shown to users: `tauri.conf.json`, or on Windows `tauri.windows.conf.json` when it sets one (the Windows
/// build has its own version; the crate version is the workspace's).
fn app_version() -> String {
    let version = |json: &str| serde_json::from_str::<serde_json::Value>(json).ok().and_then(|conf| conf["version"].as_str().map(str::to_string));
    let windows = if cfg!(windows) { version(include_str!("../tauri.windows.conf.json")) } else { None };
    windows.or_else(|| version(include_str!("../tauri.conf.json"))).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Must be the first plugin: a second launch (such as the sign-in redirect) is handed to this window.
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));
    #[cfg(feature = "keys")]
    let builder = mt_store::setup_keys(builder, include_str!("../keys.json"));
    let builder = mt_connect::setup(
        mt_store::setup(builder, include_str!("../store.json")),
        include_str!("../connect.json"),
    )
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            master: MasterState::new(),
            slave: std::sync::Arc::new(tokio::sync::Mutex::new(SlaveServerState::default())),
        });

    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        master::connect, master::connect_rtu, master::connect_rtu_virtual, master::list_serial_ports,
        master::read_tags, master::write_tag, master::disconnect, master::set_unit_id,
        master::scan_bus, master::stop_scan_bus,
        slave::slave_start, slave::slave_start_rtu, slave::slave_start_rtu_virtual, slave::slave_stop,
        slave::slave_read_registers, slave::slave_write_register,
        simulation::sim_start, simulation::sim_stop,
        simulation::sim_load_profile, simulation::sim_read_values,
        network::scan_network,
        simulation::sim_lock_register, network::auto_detect_rtu, system::platform, system::open_url,
        mt_store::store_status, mt_store::store_purchase, mt_store::store_restore, mt_store::store_activate, mt_store::store_deactivate,
        mt_connect::connect_info, mt_connect::connect_signed_in, mt_connect::connect_sign_in, mt_connect::connect_sign_out,
        mt_connect::connect_me, mt_connect::connect_sync, mt_connect::connect_profile_save, mt_connect::connect_new_id,
        mt_connect::connect_ai_availability, mt_connect::connect_ai_status, mt_connect::connect_ai_run, mt_connect::connect_open_url, mt_connect::connect_teams, mt_connect::connect_team_create, mt_connect::connect_team_invite,
        mt_connect::connect_invite_accept, mt_connect::connect_team_remove_member, mt_connect::connect_remote_sites,
        mt_connect::connect_remote_join, mt_connect::connect_store_products, mt_connect::connect_subscribe,
        mt_connect::connect_link_purchases,
    ]);

    // Windows: serial RTU like macOS, without the virtual (pty) port.
    #[cfg(target_os = "windows")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        master::connect, master::connect_rtu, master::list_serial_ports,
        master::read_tags, master::write_tag, master::disconnect, master::set_unit_id,
        master::scan_bus, master::stop_scan_bus,
        slave::slave_start, slave::slave_start_rtu, slave::slave_stop,
        slave::slave_read_registers, slave::slave_write_register,
        simulation::sim_start, simulation::sim_stop,
        simulation::sim_load_profile, simulation::sim_read_values,
        network::scan_network,
        simulation::sim_lock_register, network::auto_detect_rtu, system::platform, system::open_url,
        mt_store::store_status, mt_store::store_purchase, mt_store::store_restore, mt_store::store_activate, mt_store::store_deactivate,
        mt_connect::connect_info, mt_connect::connect_signed_in, mt_connect::connect_sign_in, mt_connect::connect_sign_out,
        mt_connect::connect_me, mt_connect::connect_sync, mt_connect::connect_profile_save, mt_connect::connect_new_id,
        mt_connect::connect_ai_availability, mt_connect::connect_ai_status, mt_connect::connect_ai_run, mt_connect::connect_open_url, mt_connect::connect_teams, mt_connect::connect_team_create, mt_connect::connect_team_invite,
        mt_connect::connect_invite_accept, mt_connect::connect_team_remove_member, mt_connect::connect_remote_sites,
        mt_connect::connect_remote_join, mt_connect::connect_store_products, mt_connect::connect_subscribe,
        mt_connect::connect_link_purchases,
    ]);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        master::connect,
        master::read_tags, master::write_tag, master::disconnect, master::set_unit_id,
        slave::slave_start, slave::slave_stop,
        slave::slave_read_registers, slave::slave_write_register,
        simulation::sim_start, simulation::sim_stop,
        simulation::sim_load_profile, simulation::sim_read_values,
        network::scan_network,
        simulation::sim_lock_register, system::open_url, system::platform,
        mt_store::store_status, mt_store::store_purchase, mt_store::store_restore, mt_store::store_activate, mt_store::store_deactivate,
        mt_connect::connect_info, mt_connect::connect_signed_in, mt_connect::connect_sign_in, mt_connect::connect_sign_out,
        mt_connect::connect_me, mt_connect::connect_sync, mt_connect::connect_profile_save, mt_connect::connect_new_id,
        mt_connect::connect_ai_availability, mt_connect::connect_ai_status, mt_connect::connect_ai_run, mt_connect::connect_open_url, mt_connect::connect_teams, mt_connect::connect_team_create, mt_connect::connect_team_invite,
        mt_connect::connect_invite_accept, mt_connect::connect_team_remove_member, mt_connect::connect_remote_sites,
        mt_connect::connect_remote_join, mt_connect::connect_store_products, mt_connect::connect_subscribe,
        mt_connect::connect_link_purchases,
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
