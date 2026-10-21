mod state;
mod master;
mod slave;
mod simulation;
mod network;
mod system;

use state::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .manage(AppState {
            master: MasterState {
                client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                scan_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                scan_cancel_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
            },
            slave: std::sync::Arc::new(tokio::sync::Mutex::new(SlaveServerState {
                state: None,
                shutdown_tx: None,
                log_rx: None,
                sim_running: None,
                sim_values: None,
                sim_profile: None,
                #[cfg(target_os = "macos")]
                virtual_slave_fd: None,
                #[cfg(target_os = "macos")]
                virtual_slave_path: None,
            })),
        });

    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        master::connect, master::connect_rtu, master::connect_rtu_virtual, master::list_serial_ports,
        master::read_tags, master::write_tag, master::disconnect, master::scan_bus, master::stop_scan_bus,
        slave::slave_start, slave::slave_start_rtu, slave::slave_start_rtu_virtual, slave::slave_stop,
        slave::slave_read_registers, slave::slave_write_register,
        simulation::sim_start, simulation::sim_stop,
        simulation::sim_load_profile, simulation::sim_read_values,
        network::scan_network,
        simulation::sim_lock_register, network::auto_detect_rtu, system::platform, system::open_url,
    ]);

    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        master::connect,
        master::read_tags, master::write_tag, master::disconnect,
        slave::slave_start, slave::slave_stop,
        slave::slave_read_registers, slave::slave_write_register,
        simulation::sim_start, simulation::sim_stop,
        simulation::sim_load_profile, simulation::sim_read_values,
        network::scan_network,
        simulation::sim_lock_register, system::open_url, system::platform,
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}
