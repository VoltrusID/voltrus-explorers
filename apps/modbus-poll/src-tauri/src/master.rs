use std::sync::Arc;
use std::time::Instant;

use mt_modbus::{ModbusClient, ModbusConfig, ModbusTag, ModbusValue};
use mt_protocol::ProtocolClient;
use tauri::Emitter;

use crate::state::*;

/// Forward the client's frame log to the frontend as `traffic-log` events.
/// The forwarder ends by itself when the client is dropped.
fn forward_traffic(app: &tauri::AppHandle, client: &mut ModbusClient) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<mt_modbus::TrafficLogEntry>();
    client.set_traffic_channel(tx);
    let app = app.clone();
    tokio::spawn(async move {
        while let Some(entry) = rx.recv().await {
            app.emit("traffic-log", &entry).ok();
        }
    });
}

/// Close and forget the active connection, if any.
async fn drop_client(state: &AppState) {
    let old = state.master.client.lock().await.take();
    if let Some(old) = old {
        let _ = old.disconnect().await;
    }
}

/// The active connection. The state lock is released before any I/O: the client
/// serialises its own requests, so overlapping polls and writes queue instead of failing.
async fn current_client(state: &AppState) -> Result<Arc<ModbusClient>, String> {
    state.master.client.lock().await.clone().ok_or_else(|| "Not connected".to_string())
}

#[tauri::command]
pub async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: ModbusConfig,
) -> Result<(), String> {
    let addr = format!("{}:{}", config.ip, config.port);
    let mut cancelled = state.master.connect_epoch.subscribe();
    let epoch = *cancelled.borrow_and_update();

    drop_client(&state).await;
    emit_tcp(&app, "connecting", &addr);
    let started = Instant::now();

    let outcome = tokio::select! {
        result = ModbusClient::connect(config) => Some(result),
        _ = cancelled.wait_for(|e| *e != epoch) => None,
    };
    let connect_ms = started.elapsed().as_millis();

    let mut client = match outcome {
        None => {
            emit_tcp(&app, "info", &format!("Connection to {addr} cancelled"));
            return Err("Connection cancelled".into());
        }
        Some(Err(e)) => {
            let err_str = e.to_string();
            if err_str.contains("imed out") || err_str.contains("Timeout") {
                emit_tcp(&app, "timeout", &format!("{} ({}ms)", addr, connect_ms));
            } else if err_str.contains("refused") {
                emit_tcp(&app, "error", &format!("Connection refused: {}", addr));
            } else if err_str.contains("reset") {
                emit_tcp(&app, "error", &format!("Connection reset: {}", addr));
            } else {
                emit_tcp(&app, "error", &format!("{}: {}", addr, err_str));
            }
            return Err(err_str);
        }
        Some(Ok(client)) => client,
    };

    forward_traffic(&app, &mut client);
    let mut slot = state.master.client.lock().await;
    // A cancel or disconnect that landed after the connect finished still wins.
    if *state.master.connect_epoch.borrow() != epoch {
        drop(slot);
        let _ = client.disconnect().await;
        emit_tcp(&app, "info", &format!("Connection to {addr} cancelled"));
        return Err("Connection cancelled".into());
    }
    *slot = Some(Arc::new(client));
    drop(slot);
    emit_tcp(&app, "connect", &format!("{} ({})", addr, format_duration(connect_ms)));
    Ok(())
}

/// The serial port may still be held by the previous connection's teardown.
#[cfg(target_os = "macos")]
fn is_port_busy(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("busy") || message.contains("lock")
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn connect_rtu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    serial_config: mt_modbus::RtuSerialConfig,
    unit_id: u8,
    endianness: mt_modbus::Endianness,
    timeout_ms: Option<u64>,
) -> Result<(), String> {
    // Free the port of any previous connection before reopening it.
    drop_client(&state).await;

    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
    let port = serial_config.port_name.clone();
    let baud = serial_config.baud_rate;

    let mut attempt = 0;
    let mut client = loop {
        match ModbusClient::connect_rtu(serial_config.clone(), unit_id, endianness, timeout).await {
            Ok(client) => break client,
            Err(e) if attempt < 4 && is_port_busy(&e.to_string()) => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => return Err(e.to_string()),
        }
    };

    forward_traffic(&app, &mut client);
    *state.master.client.lock().await = Some(Arc::new(client));
    emit_tcp(&app, "connect", &format!("{port} @ {baud} baud"));
    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn connect_rtu_virtual(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    unit_id: u8,
    endianness: mt_modbus::Endianness,
) -> Result<(), String> {
    use std::os::fd::FromRawFd;

    drop_client(&state).await;

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
    let mut client = ModbusClient::connect_rtu_from_stream(stream, unit_id, endianness, timeout)
        .await
        .map_err(|e| format!("RTU virtual connect failed: {}", e))?;

    forward_traffic(&app, &mut client);
    *state.master.client.lock().await = Some(Arc::new(client));
    emit_tcp(&app, "connect", "virtual PTY");
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
    let client = current_client(&state).await?;
    client.read(&tags).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: ModbusTag,
    value: f64,
) -> Result<(), String> {
    let client = current_client(&state).await?;
    client.write(&tag, ModbusValue::Float64(value)).await.map_err(|e| e.to_string())
}

/// Address subsequent requests to another unit ID on the open connection.
#[tauri::command]
pub async fn set_unit_id(state: tauri::State<'_, AppState>, unit_id: u8) -> Result<(), String> {
    current_client(&state).await?.set_unit_id(unit_id).await;
    Ok(())
}

#[tauri::command]
pub async fn disconnect(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // A connect still waiting for the device must not install its client afterwards.
    state.master.connect_epoch.send_modify(|epoch| *epoch += 1);
    // Closing the client aborts a request or bus scan in flight, so this does not wait for the bus.
    drop_client(&state).await;
    emit_tcp(&app, "disconnect", "Client disconnected");
    Ok(())
}

/// Scan Modbus RTU bus for devices by reading a register from each unit ID.
/// Reuses the existing connected client's serial port — no second connection opened.
/// The client stays connected afterwards, back on its configured unit ID, also when the scan is stopped.
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
    if start_id > end_id {
        return Err(format!("Start ID {start_id} is above end ID {end_id}"));
    }
    let total = (end_id - start_id) as usize + 1;

    let client = current_client(&state)
        .await
        .map_err(|_| "Not connected to a serial port".to_string())?;

    state.master.scan_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let results = client
        .scan_bus(
            register_address,
            &register_area,
            start_id,
            end_id,
            std::time::Duration::from_secs(1),
            &state.master.scan_cancel,
            &state.master.scan_cancel_notify,
        )
        .await
        .map_err(|e| e.to_string())?;

    let msg = if state.master.scan_cancel.load(std::sync::atomic::Ordering::Relaxed) {
        format!("Bus scan stopped: {} device(s) found", results.len())
    } else {
        format!("Bus scan complete: {}/{} devices found", results.len(), total)
    };
    emit_tcp(&app, "info", &msg);

    Ok(results)
}

/// Stop an in-progress bus scan. Instant — wakes the scan via Notify. The connection stays up.
#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn stop_scan_bus(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.master.scan_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    state.master.scan_cancel_notify.notify_waiters();
    Ok(())
}
