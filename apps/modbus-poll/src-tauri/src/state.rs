use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex, watch};
use mt_modbus::{ModbusClient, SimLocks, SlaveLogEntry, SlaveState};
use tauri::Emitter;

#[derive(serde::Serialize, Clone)]
pub struct TcpEvent {
    pub timestamp: u64,
    pub event_type: String,
    pub detail: String,
}

pub struct MasterState {
    /// The connected client. Commands clone the `Arc` and release the lock before
    /// doing I/O; the client serialises its own requests.
    pub client: Mutex<Option<Arc<ModbusClient>>>,
    /// Stops a running bus scan only; never implies a disconnect.
    #[cfg(target_os = "macos")]
    pub scan_cancel: AtomicBool,
    #[cfg(target_os = "macos")]
    pub scan_cancel_notify: tokio::sync::Notify,
    /// Bumped by `disconnect`; a `connect` still in flight that
    /// sees it change gives up instead of installing its client.
    pub connect_epoch: watch::Sender<u64>,
}

impl MasterState {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
            #[cfg(target_os = "macos")]
            scan_cancel: AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            scan_cancel_notify: tokio::sync::Notify::new(),
            connect_epoch: watch::channel(0).0,
        }
    }
}

#[derive(Default)]
pub struct SlaveServerState {
    pub state: Option<Arc<std::sync::Mutex<SlaveState>>>,
    pub shutdown_tx: Option<watch::Sender<bool>>,
    pub log_rx: Option<tokio::sync::mpsc::UnboundedReceiver<SlaveLogEntry>>,
    pub sim_running: Option<Arc<AtomicBool>>,
    pub sim_values: Option<Arc<std::sync::Mutex<Vec<f64>>>>,
    /// Register lock flags, shared with the running simulation.
    pub sim_locks: SimLocks,
    #[cfg(target_os = "macos")]
    pub virtual_slave_fd: Option<std::os::fd::RawFd>,
    #[cfg(target_os = "macos")]
    pub virtual_slave_path: Option<String>,
}

pub struct AppState {
    pub master: MasterState,
    pub slave: Arc<Mutex<SlaveServerState>>,
}

#[derive(serde::Serialize, Clone)]
pub struct ScanResult {
    pub ip: String,
    pub port_open: bool,
    pub hostname: Option<String>,
}

pub fn emit_tcp(app: &tauri::AppHandle, event_type: &str, detail: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    app.emit("tcp-event", TcpEvent {
        timestamp: ts,
        event_type: event_type.into(),
        detail: detail.into(),
    }).ok();
}

pub fn format_duration(ms: u128) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_milliseconds() {
        assert_eq!(format_duration(500), "500ms");
    }

    #[test]
    fn test_format_duration_seconds_exact() {
        assert_eq!(format_duration(2000), "2.0s");
    }

    #[test]
    fn test_format_duration_seconds_fractional() {
        assert_eq!(format_duration(1500), "1.5s");
    }
}
