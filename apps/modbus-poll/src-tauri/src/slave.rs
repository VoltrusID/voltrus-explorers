use tauri::Emitter;
use crate::state::*;

#[tauri::command]
pub async fn slave_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: mt_modbus::SlaveConfig,
) -> Result<(), String> {
    let (slave_state, log_rx, shutdown_tx) = mt_modbus::start_slave_server(
        config.port,
        &config.bind_addr,
        config.register_count,
    )
    .await
    .map_err(|e| e.to_string())?;

    {
        let mut slave = state.slave.lock().await;
        slave.state = Some(slave_state);
        slave.shutdown_tx = Some(shutdown_tx);
        slave.log_rx = Some(log_rx);
    }

    // Forward slave logs to Tauri events
    let app_clone = app.clone();
    let slave_arc = state.slave.clone();
    tokio::spawn(async move {
        let mut rx = {
            let mut slave = slave_arc.lock().await;
            slave.log_rx.take()
        };
        if let Some(ref mut rx) = rx {
            while let Some(entry) = rx.recv().await {
                app_clone.emit("slave-log", &entry).ok();
            }
        }
    });

    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn slave_start_rtu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    serial_config: mt_modbus::RtuSerialConfig,
    register_count: u16,
) -> Result<(), String> {
    let (slave_state, log_rx, shutdown_tx) = mt_modbus::start_slave_server_rtu(
        &serial_config.port_name,
        serial_config.baud_rate,
        serial_config.data_bits,
        serial_config.stop_bits,
        serial_config.parity,
        register_count,
    )
    .await
    .map_err(|e| e.to_string())?;

    {
        let mut slave = state.slave.lock().await;
        slave.state = Some(slave_state);
        slave.shutdown_tx = Some(shutdown_tx);
        slave.log_rx = Some(log_rx);
    }

    let app_clone = app.clone();
    let slave_arc = state.slave.clone();
    tokio::spawn(async move {
        let mut rx = {
            let mut slave = slave_arc.lock().await;
            slave.log_rx.take()
        };
        if let Some(ref mut rx) = rx {
            while let Some(entry) = rx.recv().await {
                app_clone.emit("slave-log", &entry).ok();
            }
        }
    });

    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn slave_start_rtu_virtual(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    register_count: u16,
) -> Result<String, String> {
    let (slave_state, log_rx, shutdown_tx, slave_path, slave_fd) =
        mt_modbus::start_slave_server_rtu_virtual(register_count)
            .await
            .map_err(|e| e.to_string())?;

    {
        let mut slave = state.slave.lock().await;
        slave.state = Some(slave_state);
        slave.shutdown_tx = Some(shutdown_tx);
        slave.log_rx = Some(log_rx);
        slave.virtual_slave_fd = Some(slave_fd);
        slave.virtual_slave_path = Some(slave_path.clone());
    }

    let app_clone = app.clone();
    let slave_arc = state.slave.clone();
    tokio::spawn(async move {
        let mut rx = {
            let mut slave = slave_arc.lock().await;
            slave.log_rx.take()
        };
        if let Some(ref mut rx) = rx {
            while let Some(entry) = rx.recv().await {
                app_clone.emit("slave-log", &entry).ok();
            }
        }
    });

    Ok(slave_path)
}

#[tauri::command]
pub async fn slave_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slave = state.slave.lock().await;
    if let Some(tx) = slave.shutdown_tx.take() {
        let _ = tx.send(true);
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(fd) = slave.virtual_slave_fd.take() {
            unsafe { libc::close(fd); }
        }
        slave.virtual_slave_path = None;
    }
    slave.state = None;
    slave.log_rx = None;
    Ok(())
}

#[tauri::command]
pub async fn slave_read_registers(
    state: tauri::State<'_, AppState>,
    start: u16,
    count: u16,
) -> Result<Vec<u16>, String> {
    let slave = state.slave.lock().await;
    let data = slave.state.as_ref().ok_or("Slave not running")?;
    let data = data.lock().unwrap();
    let start = start as usize;
    let end = std::cmp::min(start + count as usize, data.registers.len());
    Ok(data.registers.get(start..end).unwrap_or(&[]).to_vec())
}

#[tauri::command]
pub async fn slave_write_register(
    state: tauri::State<'_, AppState>,
    address: u16,
    value: u16,
) -> Result<(), String> {
    let slave = state.slave.lock().await;
    let data = slave.state.as_ref().ok_or("Slave not running")?;
    let mut data = data.lock().unwrap();
    let addr = address as usize;
    if addr < data.registers.len() {
        data.registers[addr] = value;
        Ok(())
    } else {
        Err(format!("Address {} out of range", address))
    }
}
