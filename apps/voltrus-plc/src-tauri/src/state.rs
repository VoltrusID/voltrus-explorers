use futures::stream::SplitSink;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::commands::compile::Diagnostic;
use crate::commands::io::IoDriverConfig;
use mt_plc_runtime::io_manager::IoDriver;
use mt_plc_types::CompiledModule;

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
    /// Last I/O mappings handed to a deploy — surfaced by `get_io_mappings`.
    pub mappings: Arc<Mutex<Vec<crate::commands::io::IoMapping>>>,
}

pub type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum TargetKind {
    WebSocket,
    Grpc,
}

impl Default for TargetKind {
    fn default() -> Self {
        Self::WebSocket
    }
}

pub struct TargetState {
    pub ip: Arc<Mutex<Option<String>>>,
    pub connected: Arc<std::sync::atomic::AtomicBool>,
    pub ws_sink: Arc<Mutex<Option<WsSink>>>,
    pub target_kind: Arc<Mutex<TargetKind>>,
    /// Persistent gRPC client to a Linux runtime (set on successful gRPC deploy/connect).
    pub grpc_client: Arc<Mutex<Option<mt_grpc::client::PlcRuntimeClient>>>,
    /// Address of the persistent gRPC connection.
    pub grpc_addr: Arc<Mutex<Option<String>>>,
    /// CRC32 of the last successfully deployed bundle payload (drift
    /// detection, PLC-6). `crc32_le` over the VLT1 body without its trailing
    /// 4-byte CRC for ESP32 HTTP deploys, over the full bincode payload for
    /// gRPC deploys. `None` until a deploy succeeds in this session.
    pub last_deployed_crc32: Arc<Mutex<Option<u32>>>,
}

pub struct AppState {
    pub compiler: Arc<Mutex<CompilerState>>,
    pub runtime: RuntimeState,
    pub io: IoState,
    pub target: TargetState,
    pub siemens: crate::commands::siemens::SiemensState,
}
