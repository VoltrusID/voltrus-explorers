use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex, watch};
use mt_modbus::{ModbusClient, SimLocks, SlaveLogEntry, SlaveState};
use tauri::Emitter;

#[derive(serde::Serialize, Clone)]
pub struct TcpEvent {
    /// Id of the connection the event belongs to.
    pub conn: String,
    pub timestamp: u64,
    pub event_type: String,
    pub detail: String,
}

/// One master connection, addressed by the id the frontend gave it.
pub struct Conn {
    /// The connected client. Commands clone the `Arc` and release the lock before
    /// doing I/O; the client serialises its own requests.
    pub client: Mutex<Option<Arc<ModbusClient>>>,
    /// Stops a running bus scan only; never implies a disconnect.
    #[cfg(serial)]
    pub scan_cancel: AtomicBool,
    #[cfg(serial)]
    pub scan_cancel_notify: tokio::sync::Notify,
    /// Bumped by `disconnect`; a `connect` still in flight that
    /// sees it change gives up instead of installing its client.
    pub connect_epoch: watch::Sender<u64>,
}

impl Conn {
    fn new() -> Self {
        Self {
            client: Mutex::new(None),
            #[cfg(serial)]
            scan_cancel: AtomicBool::new(false),
            #[cfg(serial)]
            scan_cancel_notify: tokio::sync::Notify::new(),
            connect_epoch: watch::channel(0).0,
        }
    }
}

/// All master connections by id. Connections are independent: each has its own client,
/// traffic feed, connect cancellation and bus scan.
pub struct MasterState {
    conns: Mutex<HashMap<String, Arc<Conn>>>,
}

impl MasterState {
    pub fn new() -> Self {
        Self { conns: Mutex::new(HashMap::new()) }
    }

    /// The connection with this id, created empty on first use.
    pub async fn conn(&self, id: &str) -> Arc<Conn> {
        self.conns.lock().await.entry(id.to_string()).or_insert_with(|| Arc::new(Conn::new())).clone()
    }

    /// The connection with this id, if it was ever used.
    pub async fn existing(&self, id: &str) -> Option<Arc<Conn>> {
        self.conns.lock().await.get(id).cloned()
    }

    /// Forget a connection. A connect still in flight on it keeps its own handle and gives up
    /// because `disconnect` bumped the epoch first.
    pub async fn remove(&self, id: &str) {
        self.conns.lock().await.remove(id);
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

pub fn emit_tcp(app: &tauri::AppHandle, conn: &str, event_type: &str, detail: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    app.emit("tcp-event", TcpEvent {
        conn: conn.into(),
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

    #[tokio::test]
    async fn connections_are_independent_and_created_on_first_use() {
        let master = MasterState::new();
        assert!(master.existing("c1").await.is_none());
        let a = master.conn("c1").await;
        let b = master.conn("c2").await;
        assert!(!Arc::ptr_eq(&a, &b));
        // The same id gives back the same connection.
        assert!(Arc::ptr_eq(&a, &master.conn("c1").await));
        // Cancelling one connect leaves the other's epoch alone.
        a.connect_epoch.send_modify(|e| *e += 1);
        assert_eq!(*a.connect_epoch.borrow(), 1);
        assert_eq!(*b.connect_epoch.borrow(), 0);
    }

    #[tokio::test]
    async fn removed_connection_is_forgotten_but_old_handles_keep_working() {
        let master = MasterState::new();
        let old = master.conn("c1").await;
        master.remove("c1").await;
        assert!(master.existing("c1").await.is_none());
        // A reconnect on the same id starts from a fresh connection.
        let fresh = master.conn("c1").await;
        assert!(!Arc::ptr_eq(&old, &fresh));
        assert!(old.client.lock().await.is_none());
    }
}
