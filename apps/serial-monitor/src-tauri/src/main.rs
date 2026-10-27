mod capture;
mod system;
mod virtual_port;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use mt_serial::{SerialConfig, list_ports, PortInfo};
use tauri::Emitter;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Types shared with the frontend
// ---------------------------------------------------------------------------

/// Payload emitted on the `serial-data` event for each chunk read.
#[derive(Debug, Clone, Serialize)]
struct SerialDataEvent {
    /// Session id the chunk belongs to (the id passed to `connect`).
    id: String,
    /// Hex-formatted representation of the received bytes.
    hex: String,
    /// Raw bytes as a Vec<u8>.
    bytes: Vec<u8>,
    /// UNIX epoch milliseconds when the chunk was captured.
    timestamp: u64,
}

/// Payload emitted on the `serial-disconnect` event when the port is lost.
#[derive(Debug, Clone, Serialize)]
struct SerialDisconnectEvent {
    id: String,
    reason: String,
}

/// Payload emitted on the `capture-stopped` event when writing the capture file fails.
#[derive(Debug, Clone, Serialize)]
struct CaptureStoppedEvent {
    reason: String,
}

/// Internal state held while a serial connection is active.
struct ActiveConnection {
    /// System name of the port (`/dev/cu.usbserial-1`), used to refuse opening it twice.
    port_name: String,
    /// Shared handle to the serial stream (reader task + writer command).
    port: Arc<Mutex<tokio_serial::SerialStream>>,
    /// Cancellation token to gracefully stop the reader task.
    cancel: CancellationToken,
    /// Join handle for the background reader task.
    read_task: tokio::task::JoinHandle<()>,
}

type Connections = Arc<Mutex<HashMap<String, ActiveConnection>>>;

struct AppState {
    connections: Connections,
    virtual_conn: Arc<Mutex<Option<virtual_port::VirtualPortState>>>,
    capture: Arc<capture::Capture>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Append a row to the capture file; a write failure closes the capture and tells the frontend.
fn record_capture(
    app: &tauri::AppHandle,
    capture: &capture::Capture,
    port: &str,
    direction: &str,
    bytes: &[u8],
    ts: u64,
) {
    if let Err(reason) = capture.record(port, direction, bytes, ts) {
        let _ = app.emit("capture-stopped", CaptureStoppedEvent { reason });
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    config: SerialConfig,
) -> Result<(), String> {
    // One session per id and one session per physical port.
    {
        let guard = state.connections.lock().await;
        if guard.contains_key(&id) {
            return Err("This session is already connected".into());
        }
        if guard.values().any(|c| c.port_name == config.port_name) {
            return Err(format!("{} is already open in another session", config.port_name));
        }
    }

    // Build the serial port from the shared config type but open it directly
    // instead of going through SerialClient (which prepends tag names).
    let builder = config
        .to_serial_port_builder()
        .map_err(|e| e.to_string())?;

    let stream = tokio_serial::SerialStream::open(&builder)
        .map_err(|e| format!("Failed to open {}: {e}", config.port_name))?;

    // Share the serial stream between the reader task and the send command.
    let shared_port: Arc<Mutex<tokio_serial::SerialStream>> =
        Arc::new(Mutex::new(stream));

    let port_for_reader = shared_port.clone();
    let app_for_reader = app.clone();
    let conns_for_reader = state.connections.clone();
    let capture_for_reader = state.capture.clone();
    let id_for_reader = id.clone();
    let port_name_for_reader = config.port_name.clone();
    let cancel_token = CancellationToken::new();
    let cancel_for_reader = cancel_token.clone();

    // Spawn a background task that continuously reads from the port and emits
    // Tauri events for each chunk of received data.
    let read_task = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            // Check cancellation before attempting read.
            if cancel_for_reader.is_cancelled() {
                break;
            }

            let mut port = match tokio::time::timeout(
                std::time::Duration::from_millis(100),
                port_for_reader.lock(),
            ).await {
                Ok(guard) => guard,
                Err(_) => {
                    // Could not acquire lock in 100ms — check cancel and retry.
                    if cancel_for_reader.is_cancelled() { break; }
                    continue;
                }
            };

            // Use a moderate timeout so we can periodically check cancellation.
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                port.read(&mut buf),
            )
            .await;

            // Check cancellation after each read attempt.
            if cancel_for_reader.is_cancelled() {
                break;
            }

            match result {
                Ok(Ok(0)) => {
                    // Port closed / EOF
                    drop(port);
                    let _ = app_for_reader.emit(
                        "serial-disconnect",
                        SerialDisconnectEvent {
                            id: id_for_reader.clone(),
                            reason: "Port closed (EOF)".into(),
                        },
                    );
                    // Clear connection state
                    conns_for_reader.lock().await.remove(&id_for_reader);
                    break;
                }
                Ok(Ok(n)) => {
                    let data = buf[..n].to_vec();
                    drop(port); // Release the lock before emitting.

                    let hex: String = data
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let ts = now_ms();

                    record_capture(
                        &app_for_reader,
                        &capture_for_reader,
                        &port_name_for_reader,
                        "RX",
                        &data,
                        ts,
                    );
                    let _ = app_for_reader.emit(
                        "serial-data",
                        SerialDataEvent {
                            id: id_for_reader.clone(),
                            hex,
                            bytes: data,
                            timestamp: ts,
                        },
                    );
                }
                Ok(Err(ref e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // Read timeout on the underlying serial port -- normal,
                    // just retry.
                    drop(port);
                    continue;
                }
                Ok(Err(e)) => {
                    // Real I/O error -- port likely disconnected.
                    drop(port);
                    let reason = format!("Read error: {e}");
                    let _ = app_for_reader.emit(
                        "serial-disconnect",
                        SerialDisconnectEvent { id: id_for_reader.clone(), reason },
                    );
                    conns_for_reader.lock().await.remove(&id_for_reader);
                    break;
                }
                Err(_) => {
                    // tokio::timeout elapsed -- no data within the window,
                    // loop and try again.
                    drop(port);
                    continue;
                }
            }
        }
    });

    // Store the connection.
    state.connections.lock().await.insert(
        id,
        ActiveConnection {
            port_name: config.port_name,
            port: shared_port,
            cancel: cancel_token,
            read_task,
        },
    );

    Ok(())
}

/// Decode a data payload string based on the requested format.
fn decode_payload(data: &str, format: &str) -> Result<Vec<u8>, String> {
    match format {
        "hex" => {
            let cleaned: String = data.split_whitespace().collect();
            if cleaned.is_empty() {
                return Err("No data to send".into());
            }
            if cleaned.len() % 2 != 0 {
                return Err(format!(
                    "Invalid hex string: odd length ({})",
                    cleaned.len()
                ));
            }
            (0..cleaned.len())
                .step_by(2)
                .map(|i| {
                    u8::from_str_radix(&cleaned[i..i + 2], 16)
                        .map_err(|e| format!("Invalid hex at position {i}: {e}"))
                })
                .collect::<Result<Vec<_>, String>>()
        }
        _ => Ok(data.as_bytes().to_vec()),
    }
}

#[tauri::command]
async fn send(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    data: String,
    format: String,
) -> Result<(), String> {
    // Clone the Arc quickly so we don't hold the connection lock while writing.
    let (port_arc, port_name) = {
        let guard = state.connections.lock().await;
        let conn = guard.get(&id).ok_or("Not connected")?;
        (conn.port.clone(), conn.port_name.clone())
    };

    // Decode the data payload based on the requested format.
    let bytes = decode_payload(&data, &format)?;

    // Write with timeout to avoid blocking forever on misconfigured ports.
    let write_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            let mut port = port_arc.lock().await;
            port.write_all(&bytes).await?;
            port.flush().await?;
            Ok::<(), std::io::Error>(())
        },
    )
    .await;

    match write_result {
        Ok(Ok(())) => {
            record_capture(&app, &state.capture, &port_name, "TX", &bytes, now_ms());
            Ok(())
        }
        Ok(Err(e)) => Err(format!("Write failed: {e}")),
        Err(_) => Err("Write timed out (port may be misconfigured)".into()),
    }
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    // Take the connection out first so other sessions are not blocked while this one shuts down.
    let conn = state.connections.lock().await.remove(&id);
    if let Some(conn) = conn {
        // Signal the reader task to stop gracefully.
        conn.cancel.cancel();
        // Drop the port Arc to force-close the serial stream.
        drop(conn.port);
        // Wait briefly for the reader task to finish, then force-abort it.
        let mut task = conn.read_task;
        if tokio::time::timeout(std::time::Duration::from_millis(500), &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
    }
    Ok(())
}

/// Start writing every RX and TX chunk of every open session to a CSV file at `path`
/// (a path the user picked in the save panel). Replaces a capture already running.
#[tauri::command]
fn start_capture(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    state.capture.start(&path)
}

/// Stop the capture; returns the number of rows written, or `None` if none was running.
#[tauri::command]
fn stop_capture(state: tauri::State<'_, AppState>) -> Option<u64> {
    state.capture.stop()
}

/// Path and row count of the running capture, if any.
#[tauri::command]
fn capture_status(state: tauri::State<'_, AppState>) -> Option<capture::CaptureStatus> {
    state.capture.status()
}

#[tauri::command]
fn list_available_ports() -> Result<Vec<PortInfo>, String> {
    list_ports().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Application entry point
// ---------------------------------------------------------------------------

pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            connections: Arc::new(Mutex::new(HashMap::new())),
            virtual_conn: Arc::new(Mutex::new(None)),
            capture: Arc::new(capture::Capture::default()),
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            send,
            disconnect,
            list_available_ports,
            start_capture,
            stop_capture,
            capture_status,
            virtual_port::create_virtual_port,
            virtual_port::destroy_virtual_port,
            virtual_port::send_virtual,
            virtual_port::virtual_port_status,
            system::open_url, system::connect_open_url,
            mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_store::store_activate, mt_store::store_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_hex_basic() {
        let result = decode_payload("01 03 00 00 00 0A C5 CD", "hex").unwrap();
        assert_eq!(result, vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]);
    }

    #[test]
    fn decode_hex_no_spaces() {
        let result = decode_payload("010300000A", "hex").unwrap();
        assert_eq!(result, vec![0x01, 0x03, 0x00, 0x00, 0x0A]);
    }

    #[test]
    fn decode_hex_extra_whitespace() {
        let result = decode_payload("  01   03   FF  ", "hex").unwrap();
        assert_eq!(result, vec![0x01, 0x03, 0xFF]);
    }

    #[test]
    fn decode_hex_lowercase() {
        let result = decode_payload("ff ab cd", "hex").unwrap();
        assert_eq!(result, vec![0xFF, 0xAB, 0xCD]);
    }

    #[test]
    fn decode_hex_mixed_case() {
        let result = decode_payload("Ff Ab cD", "hex").unwrap();
        assert_eq!(result, vec![0xFF, 0xAB, 0xCD]);
    }

    #[test]
    fn decode_hex_empty() {
        let result = decode_payload("", "hex");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No data to send");
    }

    #[test]
    fn decode_hex_whitespace_only() {
        let result = decode_payload("   ", "hex");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No data to send");
    }

    #[test]
    fn decode_hex_odd_length() {
        let result = decode_payload("ABC", "hex");
        assert!(result.is_err());
    }

    #[test]
    fn decode_hex_invalid_char() {
        let result = decode_payload("ZZ", "hex");
        assert!(result.is_err());
    }

    #[test]
    fn decode_text_basic() {
        let result = decode_payload("Hello", "text").unwrap();
        assert_eq!(result, b"Hello".to_vec());
    }

    #[test]
    fn decode_text_empty() {
        let result = decode_payload("", "text").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn decode_text_utf8() {
        // Verify multi-byte characters round-trip correctly through as_bytes().
        let result = decode_payload("héllo wörld 日本語", "text").unwrap();
        assert_eq!(result, "héllo wörld 日本語".as_bytes().to_vec());
    }

    #[test]
    fn decode_hex_single_byte() {
        let result = decode_payload("FF", "hex").unwrap();
        assert_eq!(result, vec![0xFF]);
    }

    #[test]
    fn decode_hex_zero_byte() {
        let result = decode_payload("00", "hex").unwrap();
        assert_eq!(result, vec![0x00]);
    }

    #[test]
    fn serial_data_event_serialization() {
        let event = SerialDataEvent {
            id: "s1".into(),
            hex: "01 03 FF".into(),
            bytes: vec![0x01, 0x03, 0xFF],
            timestamp: 1700000000000,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"id\":\"s1\""));
        assert!(json.contains("\"hex\":\"01 03 FF\""));
        assert!(json.contains("\"timestamp\":1700000000000"));
        // bytes serialize as array of numbers
        assert!(json.contains("[1,3,255]"));
    }

    #[test]
    fn serial_disconnect_event_serialization() {
        let event = SerialDisconnectEvent {
            id: "s2".into(),
            reason: "Port closed (EOF)".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"id\":\"s2\""));
        assert!(json.contains("\"reason\":\"Port closed (EOF)\""));
    }

    #[test]
    fn decode_hex_max_byte() {
        let result = decode_payload("FF FF FF", "hex").unwrap();
        assert_eq!(result, vec![255, 255, 255]);
    }
}
