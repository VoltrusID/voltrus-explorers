mod state;
mod master;
mod slave;
mod simulation;
mod network;
mod scan_targets;
mod system;

use state::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = mt_connect::setup(
        mt_store::setup(tauri::Builder::default(), include_str!("../store.json")),
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
        mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
        mt_connect::connect_info, mt_connect::connect_signed_in, mt_connect::connect_sign_in, mt_connect::connect_sign_out,
        mt_connect::connect_me, mt_connect::connect_sync, mt_connect::connect_profile_save, mt_connect::connect_new_id,
        mt_connect::connect_ai_availability, mt_connect::connect_ai_status, mt_connect::connect_ai_run, mt_connect::connect_open_url, mt_connect::connect_teams, mt_connect::connect_team_create, mt_connect::connect_team_invite,
        mt_connect::connect_invite_accept, mt_connect::connect_team_remove_member, mt_connect::connect_remote_sites,
        mt_connect::connect_remote_join, mt_connect::connect_store_products, mt_connect::connect_subscribe,
        mt_connect::connect_link_purchases,
    ]);

    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        master::connect,
        master::read_tags, master::write_tag, master::disconnect, master::set_unit_id,
        slave::slave_start, slave::slave_stop,
        slave::slave_read_registers, slave::slave_write_register,
        simulation::sim_start, simulation::sim_stop,
        simulation::sim_load_profile, simulation::sim_read_values,
        network::scan_network,
        simulation::sim_lock_register, system::open_url, system::platform,
        mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
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
