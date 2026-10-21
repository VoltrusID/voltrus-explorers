mod system;
mod virtual_port;

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
    reason: String,
}

/// Internal state held while a serial connection is active.
struct ActiveConnection {
    /// Shared handle to the serial stream (reader task + writer command).
    port: Arc<Mutex<tokio_serial::SerialStream>>,
    /// Cancellation token to gracefully stop the reader task.
    cancel: CancellationToken,
    /// Join handle for the background reader task.
    read_task: tokio::task::JoinHandle<()>,
}

struct AppState {
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    virtual_conn: Arc<Mutex<Option<virtual_port::VirtualPortState>>>,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: SerialConfig,
) -> Result<(), String> {
    // Build the serial port from the shared config type but open it directly
    // instead of going through SerialClient (which prepends tag names).
    let builder = config
        .to_serial_port_builder()
        .map_err(|e| e.to_string())?;

    let stream = tokio_serial::SerialStream::open(&builder)
        .map_err(|e| format!("Failed to open {}: {e}", config.port_name))?;

    // Clear any previous connection (should not happen, but be safe).
    {
        let mut guard = state.connection.lock().await;
        if let Some(old) = guard.take() {
            old.cancel.cancel();
            old.read_task.abort();
        }
    }

    // Share the serial stream between the reader task and the send command.
    let shared_port: Arc<Mutex<tokio_serial::SerialStream>> =
        Arc::new(Mutex::new(stream));

    let port_for_reader = shared_port.clone();
    let app_for_reader = app.clone();
    let conn_for_reader = state.connection.clone();
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
                            reason: "Port closed (EOF)".into(),
                        },
                    );
                    // Clear connection state
                    let mut guard = conn_for_reader.lock().await;
                    guard.take();
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
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    let _ = app_for_reader.emit(
                        "serial-data",
                        SerialDataEvent {
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
                        SerialDisconnectEvent { reason },
                    );
                    let mut guard = conn_for_reader.lock().await;
                    guard.take();
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
    *state.connection.lock().await = Some(ActiveConnection {
        port: shared_port,
        cancel: cancel_token,
        read_task,
    });

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
    state: tauri::State<'_, AppState>,
    data: String,
    format: String,
) -> Result<(), String> {
    // Clone the Arc quickly so we don't hold the connection lock while writing.
    let port_arc = {
        let guard = state.connection.lock().await;
        let conn = guard.as_ref().ok_or("Not connected")?;
        conn.port.clone()
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
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("Write failed: {e}")),
        Err(_) => Err("Write timed out (port may be misconfigured)".into()),
    }
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Cancel + abort with timeout to prevent hanging on misconfigured ports.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        async {
            let mut guard = state.connection.lock().await;
            if let Some(conn) = guard.take() {
                // Signal the reader task to stop gracefully.
                conn.cancel.cancel();
                // Drop the port Arc to force-close the serial stream.
                drop(conn.port);
                // Wait briefly for the reader task to finish.
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    conn.read_task,
                )
                .await;
            }
        },
    )
    .await;

    if result.is_err() {
        // Timed out — force-abort by re-acquiring the lock.
        let mut guard = state.connection.lock().await;
        if let Some(conn) = guard.take() {
            conn.read_task.abort();
        }
    }

    Ok(())
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
            connection: Arc::new(Mutex::new(None)),
            virtual_conn: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            send,
            disconnect,
            list_available_ports,
            virtual_port::create_virtual_port,
            virtual_port::destroy_virtual_port,
            virtual_port::send_virtual,
            virtual_port::virtual_port_status,
            system::open_url,
            mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
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
            hex: "01 03 FF".into(),
            bytes: vec![0x01, 0x03, 0xFF],
            timestamp: 1700000000000,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"hex\":\"01 03 FF\""));
        assert!(json.contains("\"timestamp\":1700000000000"));
        // bytes serialize as array of numbers
        assert!(json.contains("[1,3,255]"));
    }

    #[test]
    fn serial_disconnect_event_serialization() {
        let event = SerialDisconnectEvent {
            reason: "Port closed (EOF)".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"reason\":\"Port closed (EOF)\""));
    }

    #[test]
    fn decode_hex_max_byte() {
        let result = decode_payload("FF FF FF", "hex").unwrap();
        assert_eq!(result, vec![255, 255, 255]);
    }
}
