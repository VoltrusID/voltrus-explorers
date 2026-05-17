use std::sync::Arc;
use tokio::sync::Mutex;

use crate::commands::compile::Diagnostic;
use crate::commands::io::IoDriverConfig;
use mt_plc_types::CompiledModule;
use mt_plc_runtime::io_manager::IoDriver;

pub struct CompilerState {
    pub last_diagnostics: Vec<Diagnostic>,
    pub last_module: Option<CompiledModule>,
}

pub struct RuntimeState {
    pub plc: Arc<Mutex<Option<mt_plc_runtime::PlcRuntime>>>,
    pub running: Arc<std::sync::atomic::AtomicBool>,
}

pub struct IoState {
    pub driver: Arc<Mutex<Option<Box<dyn IoDriver>>>>,
    pub config: Arc<Mutex<Option<IoDriverConfig>>>,
}

pub struct TargetState {
    pub ip: Arc<Mutex<Option<String>>>,
    pub connected: Arc<std::sync::atomic::AtomicBool>,
}

pub struct AppState {
    pub compiler: Arc<Mutex<CompilerState>>,
    pub runtime: RuntimeState,
    pub io: IoState,
    pub target: TargetState,
}
