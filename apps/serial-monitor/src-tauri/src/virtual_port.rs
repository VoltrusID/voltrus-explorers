//! Virtual serial port via Unix PTY (pseudo-terminal) pairs.
//!
//! Uses `libc::openpty()` to create a master/slave pair. The app holds the
//! master FD and the slave path (e.g. `/dev/ttys012`) can be used by external
//! programs or the app's own client side to connect.

// PTY creation is macOS-only; elsewhere the command is a stub and the reader pieces are unused.
#![cfg_attr(not(target_os = "macos"), allow(unused_imports, dead_code))]

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tauri::Emitter;
use serde::Serialize;

// ── Events ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct VirtualDataEvent {
    pub hex: String,
    pub bytes: Vec<u8>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct VirtualDisconnectEvent {
    pub reason: String,
}

// ── State ───────────────────────────────────────────────────────────────

pub struct VirtualPortState {
    pub port: Arc<Mutex<tokio_serial::SerialStream>>,
    pub slave_path: String,
    /// Held open so the master does not see a hang-up; closed when the port is destroyed.
    pub slave_fd: std::os::raw::c_int,
    pub read_task: tokio::task::JoinHandle<()>,
}

#[cfg(unix)]
fn close_fd(fd: std::os::raw::c_int) {
    mt_serial::pty::close_fd(fd);
}

#[cfg(not(unix))]
fn close_fd(_fd: std::os::raw::c_int) {}

// ── Commands ────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn create_virtual_port(
    app: tauri::AppHandle,
    state: tauri::State<'_, super::AppState>,
) -> Result<String, String> {
    use std::os::unix::io::FromRawFd;

    let pair = mt_serial::pty::open_pair()
        .map_err(|e| format!("openpty failed: {e}"))?;
    if let Err(e) = pair.make_raw() {
        mt_serial::pty::close_fd(pair.master_fd);
        mt_serial::pty::close_fd(pair.slave_fd);
        return Err(e.to_string());
    }
    let (master_fd, slave_fd, slave_path) = (pair.master_fd, pair.slave_fd, pair.slave_path);

    // Keep the slave open for as long as the port lives. Once the last slave is closed the master
    // reads end-of-file, so closing it here made the port die before any other program opened it.

    // master fd → TTYPort → SerialStream
    let tty_port = unsafe { serialport::TTYPort::from_raw_fd(master_fd) };
    let stream = match tokio_serial::SerialStream::try_from(tty_port) {
        Ok(stream) => stream,
        Err(e) => {
            // The failed conversion closed the master; the slave is still ours.
            mt_serial::pty::close_fd(slave_fd);
            return Err(format!("Failed to create SerialStream from PTY: {e}"));
        }
    };

    let shared_port: Arc<Mutex<tokio_serial::SerialStream>> =
        Arc::new(Mutex::new(stream));
    let port_for_reader = shared_port.clone();
    let app_for_reader = app.clone();
    let virtual_conn_for_reader = state.virtual_conn.clone();

    let read_task = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            let mut port = port_for_reader.lock().await;
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                port.read(&mut buf),
            )
            .await;

            match result {
                Ok(Ok(0)) => {
                    drop(port);
                    let _ = app_for_reader.emit(
                        "virtual-disconnect",
                        VirtualDisconnectEvent {
                            reason: "PTY closed (EOF)".into(),
                        },
                    );
                    let mut guard = virtual_conn_for_reader.lock().await;
                    if let Some(conn) = guard.take() {
                        conn.read_task.abort();
                        close_fd(conn.slave_fd);
                    }
                    break;
                }
                Ok(Ok(n)) => {
                    let data = buf[..n].to_vec();
                    drop(port);
                    let hex: String = data
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let _ = app_for_reader.emit(
                        "virtual-data",
                        VirtualDataEvent {
                            bytes: data,
                            hex,
                            timestamp: ts,
                        },
                    );
                }
                Ok(Err(ref e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Ok(Err(e)) => {
                    let reason = format!("PTY read error: {e}");
                    drop(port);
                    let _ = app_for_reader.emit(
                        "virtual-disconnect",
                        VirtualDisconnectEvent { reason },
                    );
                    let mut guard = virtual_conn_for_reader.lock().await;
                    if let Some(conn) = guard.take() {
                        conn.read_task.abort();
                        close_fd(conn.slave_fd);
                    }
                    break;
                }
                Err(_) => {
                    // timeout elapsed, retry
                }
            }
        }
    });

    let slave_path_clone = slave_path.clone();
    *state.virtual_conn.lock().await = Some(VirtualPortState {
        port: shared_port,
        slave_path: slave_path_clone,
        slave_fd,
        read_task,
    });

    Ok(slave_path)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn create_virtual_port(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, super::AppState>,
) -> Result<String, String> {
    Err("Virtual serial ports are only supported on macOS".into())
}

#[tauri::command]
pub async fn destroy_virtual_port(
    state: tauri::State<'_, super::AppState>,
) -> Result<(), String> {
    let mut guard = state.virtual_conn.lock().await;
    if let Some(conn) = guard.take() {
        conn.read_task.abort();
        close_fd(conn.slave_fd);
    }
    Ok(())
}

#[tauri::command]
pub async fn send_virtual(
    state: tauri::State<'_, super::AppState>,
    data: String,
    format: String,
) -> Result<(), String> {
    let guard = state.virtual_conn.lock().await;
    let conn = guard.as_ref().ok_or("Virtual port not active")?;

    let bytes = super::decode_payload(&data, &format)?;

    let mut port = conn.port.lock().await;
    port.write_all(&bytes)
        .await
        .map_err(|e| format!("Write failed: {e}"))?;
    port.flush()
        .await
        .map_err(|e| format!("Flush failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn virtual_port_status(
    state: tauri::State<'_, super::AppState>,
) -> Result<Option<String>, String> {
    let guard = state.virtual_conn.lock().await;
    Ok(guard.as_ref().map(|c| c.slave_path.clone()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_data_event_serialization() {
        let event = VirtualDataEvent {
            hex: "AA BB CC".into(),
            bytes: vec![0xAA, 0xBB, 0xCC],
            timestamp: 1700000000123,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"hex\":\"AA BB CC\""));
        assert!(json.contains("\"timestamp\":1700000000123"));
        assert!(json.contains("[170,187,204]"));
    }

    #[test]
    fn virtual_disconnect_event_serialization() {
        let event = VirtualDisconnectEvent {
            reason: "PTY closed (EOF)".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"reason\":\"PTY closed (EOF)\""));
    }
}
