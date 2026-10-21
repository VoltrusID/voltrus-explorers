use std::sync::Arc;
use mt_modbus::{SimDeviceProfile, Simulation, start_simulation, stop_simulation};
use crate::state::*;

#[tauri::command]
pub async fn sim_start(
    state: tauri::State<'_, AppState>,
    profile: SimDeviceProfile,
    interval_ms: u64,
) -> Result<(), String> {
    let slave = state.slave.lock().await;
    let slave_state = slave.state.as_ref().ok_or("Slave not running")?;
    let sim = Simulation::new(profile);
    let (running, shared_values) = start_simulation(sim, slave_state.clone(), interval_ms);
    drop(slave);

    let mut slave = state.slave.lock().await;
    slave.sim_running = Some(running);
    slave.sim_values = Some(shared_values);
    slave.sim_profile = None;
    Ok(())
}

#[tauri::command]
pub async fn sim_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slave = state.slave.lock().await;
    if let Some(ref running) = slave.sim_running.take() {
        stop_simulation(running);
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
    if let Some(ref running) = slave.sim_running.take() {
        stop_simulation(running);
    }
    let slave_state = slave.state.as_ref().ok_or("Slave not running")?;
    let sim = Simulation::new(profile.clone());
    sim.write_to_slave(slave_state);
    slave.sim_values = Some(Arc::new(std::sync::Mutex::new(sim.values)));
    slave.sim_profile = Some(Arc::new(std::sync::Mutex::new(profile)));
    Ok(())
}

#[tauri::command]
pub async fn sim_lock_register(
    state: tauri::State<'_, AppState>,
    register_index: usize,
    locked: bool,
) -> Result<(), String> {
    let slave = state.slave.lock().await;
    let profile = slave.sim_profile.as_ref().ok_or("No profile loaded")?;
    let mut profile = profile.lock().unwrap();
    if register_index < profile.registers.len() {
        profile.registers[register_index].locked = locked;
        Ok(())
    } else {
        Err(format!("Register index {} out of range", register_index))
    }
}
