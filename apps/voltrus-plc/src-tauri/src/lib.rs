mod state;
mod commands {
    pub mod ai;
    pub mod compile;
    pub mod files;
    pub mod runtime;
    pub mod watch;
    pub mod io;
    pub mod license;
    pub mod target;
}

use state::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            compiler: std::sync::Arc::new(tokio::sync::Mutex::new(CompilerState {
                last_diagnostics: vec![],
                last_module: None,
            })),
            runtime: RuntimeState {
                plc: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
            io: IoState {
                driver: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                config: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            },
            target: TargetState {
                ip: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                connected: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        })
        .invoke_handler(tauri::generate_handler![
            commands::compile::compile_source,
            commands::compile::get_diagnostics,
            commands::files::new_file,
            commands::files::open_file,
            commands::files::save_file,
            commands::files::list_project_files,
            commands::files::get_sample_templates,
            commands::files::save_project,
            commands::files::load_project,

            commands::runtime::plc_run,
            commands::runtime::plc_stop,
            commands::runtime::plc_step,
            commands::runtime::get_runtime_state,
            commands::runtime::set_breakpoint,
            commands::runtime::clear_breakpoint,
            commands::runtime::get_breakpoints,
            commands::runtime::step_instruction,
            commands::runtime::continue_execution,
            commands::runtime::get_current_pc_line,
            commands::watch::get_watched_variables,
            commands::watch::force_variable,
            commands::watch::unforce_variable,
            commands::io::configure_io_driver,
            commands::io::disconnect_io_driver,
            commands::io::get_io_status,
            commands::io::io_test_read,
            commands::io::io_test_write,
            commands::io::get_io_mappings,
            commands::ai::chat_claude_stream,
            commands::ai::chat_ollama_stream,

            commands::license::activate_license,
            commands::license::validate_license,
            commands::license::get_license_status,
            commands::license::deactivate_license,

            commands::target::deploy_to_target,
            commands::target::target_connect,
            commands::target::target_disconnect,
            commands::target::target_start,
            commands::target::target_stop,
            commands::target::target_step,
            commands::target::target_status,
            commands::target::register_scada_device,
        ])
        .setup(|app| {
            #[cfg(desktop)]
            {
                app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
                app.handle().plugin(tauri_plugin_dialog::init())?;
                app.handle().plugin(tauri_plugin_fs::init())?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
