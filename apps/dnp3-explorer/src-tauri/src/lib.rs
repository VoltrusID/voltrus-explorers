use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;

use mt_dnp3::{
    Dnp3Client, Dnp3Config, Dnp3Point, Dnp3Value, ControlObject, ControlRequest, Crob,
    OutstationConfig, OutstationEvent, OutstationLogEntry, OutstationState,
    Dnp3Simulation,
    start_outstation_server, start_simulation, stop_simulation,
};
use mt_dnp3::types::SimDnp3Profile;
use mt_protocol::ProtocolClient;
use tauri::Emitter;

struct MasterState {
    client: Arc<Mutex<Option<Dnp3Client>>>,
}

struct OutstationServerState {
    state: Option<Arc<std::sync::Mutex<OutstationState>>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    log_rx: Option<tokio::sync::mpsc::UnboundedReceiver<OutstationLogEntry>>,
    sim_running: Option<Arc<AtomicBool>>,
    sim_values: Option<Arc<std::sync::Mutex<Vec<f64>>>>,
}

struct AppState {
    master: MasterState,
    outstation: Arc<Mutex<OutstationServerState>>,
}

// ── Master commands ──

#[tauri::command]
async fn connect(state: tauri::State<'_, AppState>, config: Dnp3Config) -> Result<(), String> {
    let client = Dnp3Client::connect(config).await.map_err(|e| e.to_string())?;
    *state.master.client.lock().await = Some(client);
    Ok(())
}

#[tauri::command]
async fn read_tags(
    state: tauri::State<'_, AppState>,
    tags: Vec<Dnp3Point>,
) -> Result<Vec<Dnp3Value>, String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.read(&tags).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn event_scan(
    state: tauri::State<'_, AppState>,
    class: u8,
    tags: Vec<Dnp3Point>,
) -> Result<Vec<Dnp3Value>, String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client
        .scan_events(class, &tags)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn time_sync(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.time_sync().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn cold_restart(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.cold_restart().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn warm_restart(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.warm_restart().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn freeze_counters(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.freeze_counters().await.map_err(|e| e.to_string())
}

/// Operate a control relay output block (Group 12 Var 1) on the connected outstation, either as a
/// DIRECT OPERATE or as SELECT followed by OPERATE.
#[tauri::command]
async fn control_crob(
    state: tauri::State<'_, AppState>,
    index: u16,
    code: u8,
    count: u8,
    on_time_ms: u32,
    off_time_ms: u32,
    select_before_operate: bool,
) -> Result<(), String> {
    let request = ControlRequest {
        index,
        object: ControlObject::Crob(Crob { code, count, on_time_ms, off_time_ms }),
    };
    run_control(&state, &request, select_before_operate).await
}

/// Operate an analog output (Group 41 Var 3, float32), as DIRECT OPERATE or SELECT then OPERATE.
#[tauri::command]
async fn control_analog(
    state: tauri::State<'_, AppState>,
    index: u16,
    value: f64,
    select_before_operate: bool,
) -> Result<(), String> {
    let request = ControlRequest {
        index,
        object: ControlObject::AnalogOutput(value as f32),
    };
    run_control(&state, &request, select_before_operate).await
}

async fn run_control(
    state: &tauri::State<'_, AppState>,
    request: &ControlRequest,
    select_before_operate: bool,
) -> Result<(), String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    let result = if select_before_operate {
        client.select_before_operate(request).await
    } else {
        client.direct_operate(request).await
    };
    result.map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: Dnp3Point,
    value: Dnp3Value,
) -> Result<(), String> {
    let guard = state.master.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.write(&tag, value).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.master.client.lock().await;
    if let Some(client) = guard.take() {
        client.disconnect().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Outstation commands ──

#[tauri::command]
async fn outstation_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: OutstationConfig,
) -> Result<(), String> {
    let (outstation_state, log_rx, shutdown_tx) = start_outstation_server(config)
        .await
        .map_err(|e| e.to_string())?;

    {
        let mut os = state.outstation.lock().await;
        os.state = Some(outstation_state);
        os.shutdown_tx = Some(shutdown_tx);
        os.log_rx = Some(log_rx);
    }

    // Forward outstation logs to Tauri events
    let app_clone = app.clone();
    let os_arc = state.outstation.clone();
    tokio::spawn(async move {
        let mut rx = {
            let mut os = os_arc.lock().await;
            os.log_rx.take()
        };
        if let Some(ref mut rx) = rx {
            while let Some(entry) = rx.recv().await {
                app_clone.emit("outstation-log", &entry).ok();
            }
        }
    });

    Ok(())
}

#[tauri::command]
async fn outstation_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut os = state.outstation.lock().await;
    if let Some(tx) = os.shutdown_tx.take() {
        let _ = tx.send(true);
    }
    os.state = None;
    os.log_rx = None;
    Ok(())
}

#[tauri::command]
async fn outstation_read_points(
    state: tauri::State<'_, AppState>,
) -> Result<(Vec<f64>, Vec<bool>, Vec<f64>), String> {
    let os = state.outstation.lock().await;
    let data = os.state.as_ref().ok_or("Outstation not running")?;
    let data = data.lock().unwrap();
    Ok((
        data.analog_values.clone(),
        data.binary_values.clone(),
        data.counter_values.clone(),
    ))
}

#[tauri::command]
async fn outstation_read_events(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<OutstationEvent>, String> {
    let os = state.outstation.lock().await;
    let data = os.state.as_ref().ok_or("Outstation not running")?;
    let data = data.lock().unwrap();
    Ok(data.events.iter().cloned().collect())
}

#[tauri::command]
async fn outstation_write_analog(
    state: tauri::State<'_, AppState>,
    index: usize,
    value: f64,
) -> Result<(), String> {
    let os = state.outstation.lock().await;
    let data = os.state.as_ref().ok_or("Outstation not running")?;
    let mut data = data.lock().unwrap();
    if index < data.analog_values.len() {
        data.analog_values[index] = value;
        data.detect_events();
        Ok(())
    } else {
        Err(format!("Index {} out of range", index))
    }
}

#[tauri::command]
async fn outstation_write_binary(
    state: tauri::State<'_, AppState>,
    index: usize,
    value: bool,
) -> Result<(), String> {
    let os = state.outstation.lock().await;
    let data = os.state.as_ref().ok_or("Outstation not running")?;
    let mut data = data.lock().unwrap();
    if index < data.binary_values.len() {
        data.binary_values[index] = value;
        data.detect_events();
        Ok(())
    } else {
        Err(format!("Index {} out of range", index))
    }
}

// ── Simulation commands ──

#[tauri::command]
async fn sim_start(
    state: tauri::State<'_, AppState>,
    profile: SimDnp3Profile,
    interval_ms: u64,
) -> Result<(), String> {
    let os = state.outstation.lock().await;
    let os_state = os.state.as_ref().ok_or("Outstation not running")?;
    let sim = Dnp3Simulation::new(profile);
    let (running, shared_values) = start_simulation(sim, os_state.clone(), interval_ms);
    drop(os);

    let mut os = state.outstation.lock().await;
    os.sim_running = Some(running);
    os.sim_values = Some(shared_values);
    Ok(())
}

#[tauri::command]
async fn sim_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut os = state.outstation.lock().await;
    if let Some(ref running) = os.sim_running.take() {
        stop_simulation(running);
    }
    Ok(())
}

#[tauri::command]
async fn sim_read_values(state: tauri::State<'_, AppState>) -> Result<Vec<f64>, String> {
    let os = state.outstation.lock().await;
    let sv = os.sim_values.as_ref().ok_or("No profile loaded")?;
    let v = sv.lock().unwrap();
    Ok(v.clone())
}

#[tauri::command]
async fn sim_load_profile(
    state: tauri::State<'_, AppState>,
    profile: SimDnp3Profile,
) -> Result<(), String> {
    let mut os = state.outstation.lock().await;
    if let Some(ref running) = os.sim_running.take() {
        stop_simulation(running);
    }
    let os_state = os.state.as_ref().ok_or("Outstation not running")?;
    let sim = Dnp3Simulation::new(profile);
    sim.write_to_outstation(os_state);
    os.sim_values = Some(Arc::new(std::sync::Mutex::new(sim.values)));
    Ok(())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native save panel + writes to the path the user picked (the only file access the sandboxed
        // App Store build has), used by the log export.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            master: MasterState {
                client: Arc::new(Mutex::new(None)),
            },
            outstation: Arc::new(Mutex::new(OutstationServerState {
                state: None,
                shutdown_tx: None,
                log_rx: None,
                sim_running: None,
                sim_values: None,
            })),
        })
        .invoke_handler(tauri::generate_handler![
            connect, read_tags, event_scan, time_sync, cold_restart, warm_restart,
            freeze_counters, write_tag, control_crob, control_analog, disconnect,
            outstation_start, outstation_stop, outstation_read_points, outstation_read_events,
            outstation_write_analog, outstation_write_binary,
            sim_start, sim_stop, sim_load_profile, sim_read_values,
            open_url,
            mt_store::request_review, mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
