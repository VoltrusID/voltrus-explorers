use std::sync::Arc;
use tokio::sync::Mutex;
use futures::stream::SplitSink;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio::net::TcpStream;

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

pub type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum TargetKind {
    WebSocket,
    Grpc,
}

impl Default for TargetKind {
    fn default() -> Self { Self::WebSocket }
}

pub struct TargetState {
    pub ip: Arc<Mutex<Option<String>>>,
    pub connected: Arc<std::sync::atomic::AtomicBool>,
    pub ws_sink: Arc<Mutex<Option<WsSink>>>,
    pub target_kind: Arc<Mutex<TargetKind>>,
}

pub struct AppState {
    pub compiler: Arc<Mutex<CompilerState>>,
    pub runtime: RuntimeState,
    pub io: IoState,
    pub target: TargetState,
}
