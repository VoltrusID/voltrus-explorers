mod state;
mod commands {
    pub mod ai;
    pub mod compile;
    pub mod eip_import;
    pub mod files;
    pub mod runtime;
    pub mod watch;
    pub mod io;
    pub mod license;
    pub mod target;
    pub mod tests;
}

use state::*;
use tauri::Emitter;

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
                mappings: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
            },
            target: TargetState {
                ip: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                connected: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ws_sink: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                target_kind: std::sync::Arc::new(tokio::sync::Mutex::new(crate::state::TargetKind::WebSocket)),
                grpc_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                grpc_addr: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                last_deployed_crc32: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
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
            commands::eip_import::eip_import_udts,
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
            commands::target::scan_mdns_targets,
            commands::target::set_remote_breakpoint,
            commands::target::clear_remote_breakpoint,
            commands::target::get_target_kind,
            commands::target::set_target_kind,
            commands::target::deploy_via_grpc,
            commands::target::grpc_start,
            commands::target::grpc_stop,
            commands::target::grpc_step,

            commands::tests::run_project_tests,
            commands::tests::generate_test_prompt,
        ])
        .setup(|app| {
            #[cfg(desktop)]
            {
                app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
                app.handle().plugin(tauri_plugin_dialog::init())?;
                app.handle().plugin(tauri_plugin_fs::init())?;
                app.handle().plugin(tauri_plugin_deep_link::init())?;

                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register("voltrus-plc");
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    if let Some(url) = event.urls().first() {
                        let _ = handle.emit("deep-link", url.to_string());
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
