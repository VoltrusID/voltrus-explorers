use std::sync::Arc;
use mt_modbus::{SimDeviceProfile, Simulation, start_simulation, stop_simulation};
use crate::state::*;

#[tauri::command]
pub async fn sim_start(
    state: tauri::State<'_, AppState>,
    profile: SimDeviceProfile,
    interval_ms: u64,
) -> Result<(), String> {
    let mut slave = state.slave.lock().await;
    let slave_state = slave.state.clone().ok_or("Slave not running")?;
    if let Some(previous) = slave.sim_running.take() {
        stop_simulation(&previous);
    }
    // Locks are shared with the lock button, so toggling one affects the running simulation.
    let sim = Simulation::new(profile).with_locks(slave.sim_locks.clone());
    let (running, shared_values) = start_simulation(sim, slave_state, interval_ms);
    slave.sim_running = Some(running);
    slave.sim_values = Some(shared_values);
    Ok(())
}

#[tauri::command]
pub async fn sim_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slave = state.slave.lock().await;
    if let Some(running) = slave.sim_running.take() {
        stop_simulation(&running);
    }
    Ok(())
}

#[tauri::command]
pub async fn sim_read_values(state: tauri::State<'_, AppState>) -> Result<Vec<f64>, String> {
    let slave = state.slave.lock().await;
    let sv = slave.sim_values.as_ref().ok_or("No profile loaded")?;
    let v = sv.lock().unwrap();
    Ok(v.clone())
}

#[tauri::command]
pub async fn sim_load_profile(
    state: tauri::State<'_, AppState>,
    profile: SimDeviceProfile,
) -> Result<(), String> {
    let mut slave = state.slave.lock().await;
    if let Some(running) = slave.sim_running.take() {
        stop_simulation(&running);
    }
    let slave_state = slave.state.as_ref().ok_or("Slave not running")?;
    let sim = Simulation::new(profile);
    sim.write_to_slave(slave_state);
    // A new profile starts with its own locks.
    slave.sim_locks = sim.locks.clone();
    slave.sim_values = Some(Arc::new(std::sync::Mutex::new(sim.values)));
    Ok(())
}

#[tauri::command]
pub async fn sim_lock_register(
    state: tauri::State<'_, AppState>,
    register_index: usize,
    locked: bool,
) -> Result<(), String> {
    let slave = state.slave.lock().await;
    if slave.state.is_none() {
        return Err("Slave not running".into());
    }
    if let Some(values) = &slave.sim_values {
        let count = values.lock().unwrap().len();
        if register_index >= count {
            return Err(format!("Register index {register_index} out of range ({count} registers)"));
        }
    }
    slave.sim_locks.set(register_index, locked);
    Ok(())
}
