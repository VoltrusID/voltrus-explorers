use tauri::Emitter;
use crate::state::*;

/// Record a freshly started server in the app state and forward its log as `slave-log` events.
async fn install_server(
    app: &tauri::AppHandle,
    state: &AppState,
    handles: mt_modbus::SlaveHandles,
) {
    let (slave_state, log_rx, shutdown_tx) = handles;
    {
        let mut slave = state.slave.lock().await;
        slave.state = Some(slave_state);
        slave.shutdown_tx = Some(shutdown_tx);
        slave.log_rx = Some(log_rx);
        slave.sim_locks = Default::default();
    }

    let app = app.clone();
    let slave_arc = state.slave.clone();
    tokio::spawn(async move {
        let mut rx = {
            let mut slave = slave_arc.lock().await;
            slave.log_rx.take()
        };
        if let Some(rx) = &mut rx {
            while let Some(entry) = rx.recv().await {
                app.emit("slave-log", &entry).ok();
            }
        }
    });
}

#[tauri::command]
pub async fn slave_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: mt_modbus::SlaveConfig,
) -> Result<(), String> {
    let handles = mt_modbus::start_slave_server(
        config.port,
        &config.bind_addr,
        config.register_count,
        config.unit_id,
    )
    .await
    .map_err(|e| e.to_string())?;
    install_server(&app, &state, handles).await;
    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn slave_start_rtu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    serial_config: mt_modbus::RtuSerialConfig,
    register_count: u16,
    unit_id: u8,
) -> Result<(), String> {
    let handles = mt_modbus::start_slave_server_rtu(
        &serial_config.port_name,
        serial_config.baud_rate,
        serial_config.data_bits,
        serial_config.stop_bits,
        serial_config.parity,
        register_count,
        unit_id,
    )
    .await
    .map_err(|e| e.to_string())?;
    install_server(&app, &state, handles).await;
    Ok(())
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn slave_start_rtu_virtual(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    register_count: u16,
    unit_id: u8,
) -> Result<String, String> {
    let (slave_state, log_rx, shutdown_tx, slave_path, slave_fd) =
        mt_modbus::start_slave_server_rtu_virtual(register_count, unit_id)
            .await
            .map_err(|e| e.to_string())?;

    {
        let mut slave = state.slave.lock().await;
        slave.virtual_slave_fd = Some(slave_fd);
        slave.virtual_slave_path = Some(slave_path.clone());
    }
    install_server(&app, &state, (slave_state, log_rx, shutdown_tx)).await;

    Ok(slave_path)
}

#[tauri::command]
pub async fn slave_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slave = state.slave.lock().await;
    // The simulation task writes into this server's registers; it must not outlive it.
    if let Some(running) = slave.sim_running.take() {
        mt_modbus::stop_simulation(&running);
    }
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
    slave.sim_values = None;
    slave.sim_locks = Default::default();
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
