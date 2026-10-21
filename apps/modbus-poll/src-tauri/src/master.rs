use mt_modbus::{ModbusClient, ModbusConfig, ModbusTag, ModbusValue};
use mt_protocol::ProtocolClient;
use tauri::Emitter;
use crate::state::*;

#[tauri::command]
pub async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: ModbusConfig,
) -> Result<(), String> {
    // Reset cancel flag — disconnect sets it to true, must clear before reconnecting
    state.master.scan_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let addr = format!("{}:{}", config.ip, config.port);
    let connect_start = std::time::Instant::now();

    emit_tcp(&app, "connecting", &format!("{}:{}", config.ip, config.port));

    let client_result = ModbusClient::connect(config).await;
    let connect_ms = connect_start.elapsed().as_millis();

    match client_result {
        Ok(mut client) => {
            emit_tcp(&app, "connect", &format!("{} ({})", addr, format_duration(connect_ms)));

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<mt_modbus::TrafficLogEntry>();
            client.set_traffic_channel(tx);
            let app_clone = app.clone();
            tokio::spawn(async move {
                while let Some(entry) = rx.recv().await {
                    app_clone.emit("traffic-log", &entry).ok();
                }
            });

            *state.master.client.lock().await = Some(client);
            Ok(())
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("timeout") || err_str.contains("Timeout") {
                emit_tcp(&app, "timeout", &format!("{} ({}ms)", addr, connect_ms));
            } else if err_str.contains("refused") || err_str.contains("Connection refused") {
                emit_tcp(&app, "error", &format!("Connection refused: {}", addr));
            } else if err_str.contains("reset") {
                emit_tcp(&app, "error", &format!("Connection reset: {}", addr));
            } else {
                emit_tcp(&app, "error", &format!("{}: {}", addr, err_str));
            }
            Err(err_str)
        }
    }
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn connect_rtu(
    state: tauri::State<'_, AppState>,
    serial_config: mt_modbus::RtuSerialConfig,
    unit_id: u8,
    timeout_ms: Option<u64>,
) -> Result<(), String> {
    // Clean up any stale client first — await inline so port is freed before reconnecting
    {
        let result: Result<_, _> = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            state.master.client.lock(),
        ).await;
        if let Ok(mut guard) = result {
            if let Some(old) = guard.take() {
                let _ = old.disconnect().await;
            }
        }
    }

    // Reset cancel flag
    state.master.scan_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
    // Retry serial open — port may still be held by background disconnect cleanup
    let mut client = None;
    for attempt in 0..5 {
        match ModbusClient::connect_rtu(serial_config.clone(), unit_id, Default::default(), timeout).await {
            Ok(c) => { client = Some(c); break; }
            Err(e) if attempt < 9 => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("busy") || msg.contains("lock") || msg.contains("serial") {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                }
                return Err(e.to_string());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let client = client.unwrap();
    *state.master.client.lock().await = Some(client);
    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn connect_rtu_virtual(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    unit_id: u8,
) -> Result<(), String> {
    use std::os::fd::FromRawFd;

    // Get the slave fd from virtual slave state and dup it
    let slave_fd = {
        let slave = state.slave.lock().await;
        slave.virtual_slave_fd.ok_or("No virtual slave running")?
    };
    let fd = unsafe { libc::dup(slave_fd) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error().to_string());
    }

    // dup'd fd → TTYPort → SerialStream → RTU client
    let tty_port = unsafe { serialport::TTYPort::from_raw_fd(fd) };
    let stream = tokio_serial::SerialStream::try_from(tty_port)
        .map_err(|e| format!("SerialStream from fd failed: {}", e))?;

    let timeout = std::time::Duration::from_secs(5);
    let mut client = ModbusClient::connect_rtu_from_stream(stream, unit_id, Default::default(), timeout)
        .await
        .map_err(|e| format!("RTU virtual connect failed: {}", e))?;

    emit_tcp(&app, "connect", "virtual PTY");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<mt_modbus::TrafficLogEntry>();
    client.set_traffic_channel(tx);
    let app_clone = app.clone();
    tokio::spawn(async move {
        while let Some(entry) = rx.recv().await {
            app_clone.emit("traffic-log", &entry).ok();
        }
    });

    *state.master.client.lock().await = Some(client);
    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub fn list_serial_ports() -> Result<Vec<String>, String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_tags(
    state: tauri::State<'_, AppState>,
    tags: Vec<ModbusTag>,
) -> Result<Vec<ModbusValue>, String> {
    // Take client out — don't hold outer lock during slow serial I/O
    let client = {
        let mut guard = state.master.client.lock().await;
        guard.take().ok_or("Not connected")?
    };

    let result = client.read(&tags).await.map_err(|e| e.to_string());

    // If disconnect was requested during read, discard client (port freed)
    if state.master.scan_cancel.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = client.disconnect().await;
        return Err("Disconnected".into());
    }

    // Put client back for next read
    *state.master.client.lock().await = Some(client);
    result
}

#[tauri::command]
pub async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: ModbusTag,
    value: f64,
) -> Result<(), String> {
    let client = {
        let mut guard = state.master.client.lock().await;
        guard.take().ok_or("Not connected")?
    };

    let result = client.write(&tag, ModbusValue::Float64(value)).await.map_err(|e| e.to_string());

    if state.master.scan_cancel.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = client.disconnect().await;
        return Err("Disconnected".into());
    }

    *state.master.client.lock().await = Some(client);
    result
}

#[tauri::command]
pub async fn disconnect(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Signal everything to stop
    state.master.scan_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    state.master.scan_cancel_notify.notify_waiters();

    // read_tags no longer holds the outer lock during I/O — it takes the client out.
    // So we can acquire the lock instantly.
    // If read_tags has the client out, we find None here — read_tags will see
    // scan_cancel and discard the client itself.
    let taken = {
        let mut guard = state.master.client.lock().await;
        guard.take()
    };

    if let Some(client) = taken {
        let _ = client.disconnect().await;
    }

    emit_tcp(&app, "disconnect", "Client disconnected");
    Ok(())
}

/// Scan Modbus RTU bus for devices by reading a register from each unit ID.
/// Reuses the existing connected client's serial port — no second connection opened.
/// Takes client out of state during scan so disconnect can run concurrently.
#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn scan_bus(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    register_address: u16,
    register_area: String,
    start_id: u8,
    end_id: u8,
) -> Result<Vec<mt_modbus::BusScanResult>, String> {
    let total = (end_id - start_id + 1) as usize;
    let app_clone = app.clone();

    // Reset cancel flag before starting
    state.master.scan_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    // Take client out of state — no lock held during scan, disconnect can run
    let client = {
        let mut guard = state.master.client.lock().await;
        guard.take().ok_or("Not connected to a serial port")?
    };

    let per_id_timeout = std::time::Duration::from_secs(1);

    let scan_result = client.scan_bus(
        register_address,
        &register_area,
        start_id,
        end_id,
        per_id_timeout,
        &state.master.scan_cancel,
        &state.master.scan_cancel_notify,
    )
    .await;

    let cancelled = state.master.scan_cancel.load(std::sync::atomic::Ordering::Relaxed);

    if cancelled {
        // Disconnect was requested during scan — discard client
        let _ = client.disconnect().await;
        let results = scan_result.map_err(|e| e.to_string())?;
        let msg = format!(
            "Bus scan cancelled: {}/{} devices found (stopped at ID {})",
            results.len(), total, results.last().map_or(start_id, |r| r.unit_id)
        );
        emit_tcp(&app_clone, "info", &msg);
        return Ok(results);
    }

    // Put client back
    *state.master.client.lock().await = Some(client);

    let results = scan_result.map_err(|e| e.to_string())?;

    // Emit summary
    let msg = format!("Bus scan complete: {}/{} devices found", results.len(), total);
    emit_tcp(&app_clone, "info", &msg);

    Ok(results)
}

/// Cancel an in-progress bus scan. Instant — wakes scan via Notify.
#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn stop_scan_bus(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.master.scan_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    state.master.scan_cancel_notify.notify_waiters();
    Ok(())
}
