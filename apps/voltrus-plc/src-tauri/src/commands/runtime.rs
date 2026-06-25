use crate::state::AppState;
use mt_plc_runtime::{PlcRuntime, RuntimeConfig as PlcRuntimeConfig};
use tauri::Emitter;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct RuntimeConfig {
    pub scan_cycle_ms: u64,
    pub max_cycle_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct RuntimeStateInfo {
    pub state: String,
    pub cycle_count: u64,
    pub last_cycle_ms: f64,
    pub avg_cycle_ms: f64,
    pub max_cycle_ms: f64,
    pub error_message: Option<String>,
    #[serde(default)]
    pub cycle_history: Vec<f64>,
}

#[tauri::command]
pub async fn plc_run(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: RuntimeConfig,
) -> Result<(), String> {
    let running = state.runtime.running.clone();
    running.store(true, std::sync::atomic::Ordering::SeqCst);

    let plc_arc = state.runtime.plc.clone();

    // Create runtime from compiled module, preserving variable values
    {
        let compiler = state.compiler.lock().await;
        let module = compiler.last_module.as_ref()
            .ok_or("No compiled program. Click Build first.")?;

        let mut plc = plc_arc.lock().await;

        // Snapshot current variable values and FB state before replacing the runtime
        let snapshot = plc.as_ref().map(|r| r.snapshot_variables()).unwrap_or_default();
        let fb_snapshot = plc.as_ref().map(|r| r.snapshot_fb_state());

        *plc = Some(PlcRuntime::new(
            PlcRuntimeConfig {
                scan_cycle_ms: config.scan_cycle_ms,
                max_cycle_ms: config.max_cycle_ms,
            },
            module.clone(),
        ));

        // Restore matching variable values from snapshot
        if !snapshot.is_empty() {
            if let Some(ref mut runtime) = *plc {
                runtime.restore_variables(&snapshot);
            }
        }

        // Restore FB state (timer ET, counter CV, edge detection prev)
        if let Some(ref snap) = fb_snapshot {
            if let Some(ref mut runtime) = *plc {
                runtime.restore_fb_state(snap);
            }
        }
    }

    let app_handle = app.clone();
    tokio::spawn(async move {
        let cycle_time = config.scan_cycle_ms;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(cycle_time));

        loop {
            if !running.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            interval.tick().await;

            let mut plc = plc_arc.lock().await;
            if let Some(ref mut runtime) = *plc {
                match runtime.execute_cycle() {
                    Ok(stats) => {
                        // Check if stop was requested during this cycle
                        if !running.load(std::sync::atomic::Ordering::SeqCst) {
                            let _ = app_handle.emit("runtime-state", RuntimeStateInfo {
                                state: "stopped".to_string(),
                                cycle_count: stats.cycle_count,
                                last_cycle_ms: stats.last_cycle_ms,
                                avg_cycle_ms: stats.avg_cycle_ms,
                                max_cycle_ms: stats.max_cycle_ms,
                                error_message: None,
                                cycle_history: stats.cycle_history.clone(),
                            });
                            break;
                        }
                        let watch = runtime.watch_variables();
                        let state_name = runtime.state().name().to_string();
                        let _ = app_handle.emit("watch-update", &watch);
                        let _ = app_handle.emit("runtime-state", RuntimeStateInfo {
                            state: state_name,
                            cycle_count: stats.cycle_count,
                            last_cycle_ms: stats.last_cycle_ms,
                            avg_cycle_ms: stats.avg_cycle_ms,
                            max_cycle_ms: stats.max_cycle_ms,
                            error_message: None,
                            cycle_history: stats.cycle_history.clone(),
                        });
                    }
                    Err(e) => {
                        running.store(false, std::sync::atomic::Ordering::SeqCst);
                        let _ = app_handle.emit("runtime-state", RuntimeStateInfo {
                            state: "error".to_string(),
                            cycle_count: 0,
                            last_cycle_ms: 0.0,
                            avg_cycle_ms: 0.0,
                            max_cycle_ms: 0.0,
                            error_message: Some(e),
                            cycle_history: vec![],
                        });
                        break;
                    }
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn plc_stop(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.runtime.running.store(false, std::sync::atomic::Ordering::SeqCst);

    // Emit stopped state so UI updates immediately
    let plc = state.runtime.plc.lock().await;
    let stats = plc.as_ref().map(|r| r.stats().clone()).unwrap_or_default();
    let _ = app.emit("runtime-state", RuntimeStateInfo {
        state: "stopped".to_string(),
        cycle_count: stats.cycle_count,
        last_cycle_ms: stats.last_cycle_ms,
        avg_cycle_ms: stats.avg_cycle_ms,
        max_cycle_ms: stats.max_cycle_ms,
        error_message: None,
        cycle_history: stats.cycle_history.clone(),
    });

    Ok(())
}

#[tauri::command]
pub async fn plc_step(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Ensure runtime exists from compiled module
    {
        let compiler = state.compiler.lock().await;
        if compiler.last_module.is_none() {
            return Err("No compiled program. Click Build first.".into());
        }

        let mut plc = state.runtime.plc.lock().await;
        if plc.is_none() {
            let module = compiler.last_module.as_ref().unwrap();
            *plc = Some(PlcRuntime::new(
                PlcRuntimeConfig { scan_cycle_ms: 10, max_cycle_ms: 1000 },
                module.clone(),
            ));
        }
    }

    let mut plc = state.runtime.plc.lock().await;
    if let Some(ref mut runtime) = *plc {
        runtime.execute_cycle()?;
        let watch = runtime.watch_variables();
        let stats = runtime.stats().clone();
        let _ = app.emit("watch-update", &watch);
        let _ = app.emit("runtime-state", RuntimeStateInfo {
            state: "paused".to_string(),
            cycle_count: stats.cycle_count,
            last_cycle_ms: stats.last_cycle_ms,
            avg_cycle_ms: stats.avg_cycle_ms,
            max_cycle_ms: stats.max_cycle_ms,
            error_message: None,
            cycle_history: stats.cycle_history.clone(),
        });
    }

    Ok(())
}

#[tauri::command]
pub async fn get_runtime_state(
    state: tauri::State<'_, AppState>,
) -> Result<RuntimeStateInfo, String> {
    let plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref runtime) => {
            let stats = runtime.stats();
            let s = runtime.state();
            Ok(RuntimeStateInfo {
                state: s.name().to_string(),
                cycle_count: stats.cycle_count,
                last_cycle_ms: stats.last_cycle_ms,
                avg_cycle_ms: stats.avg_cycle_ms,
                max_cycle_ms: stats.max_cycle_ms,
                error_message: match s {
                    mt_plc_runtime::RuntimeState::Error(e) => Some(e.clone()),
                    _ => None,
                },
                cycle_history: stats.cycle_history.clone(),
            })
        }
        None => Ok(RuntimeStateInfo {
            state: "stopped".into(),
            cycle_count: 0,
            last_cycle_ms: 0.0,
            avg_cycle_ms: 0.0,
            max_cycle_ms: 0.0,
            error_message: None,
            cycle_history: vec![],
        }),
    }
}

// ── Debug / Breakpoint commands ──

#[tauri::command]
pub async fn set_breakpoint(
    state: tauri::State<'_, AppState>,
    line: u32,
) -> Result<(), String> {
    let mut plc = state.runtime.plc.lock().await;
    if let Some(ref mut runtime) = *plc {
        runtime.set_breakpoint_line(line as usize);
        Ok(())
    } else {
        Err("No runtime. Compile first.".into())
    }
}

#[tauri::command]
pub async fn clear_breakpoint(
    state: tauri::State<'_, AppState>,
    line: u32,
) -> Result<(), String> {
    let mut plc = state.runtime.plc.lock().await;
    if let Some(ref mut runtime) = *plc {
        runtime.clear_breakpoint_line(line as usize);
        Ok(())
    } else {
        Err("No runtime. Compile first.".into())
    }
}

#[tauri::command]
pub async fn get_breakpoints(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<u32>, String> {
    let plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref runtime) => Ok(runtime.get_breakpoint_lines().into_iter().map(|l| l as u32).collect()),
        None => Ok(vec![]),
    }
}

#[derive(serde::Serialize, Clone)]
pub struct DebugStepResult {
    pub state: String,
    pub current_line: Option<u32>,
}

#[tauri::command]
pub async fn step_instruction(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<DebugStepResult, String> {
    let mut plc = state.runtime.plc.lock().await;
    if let Some(ref mut runtime) = *plc {
        let exec_state = runtime.step_instruction()?;
        let watch = runtime.watch_variables();
        let _ = app.emit("watch-update", &watch);
        let current_line = runtime.current_pc_line().map(|l| l as u32);
        let state_name = match &exec_state {
            mt_plc_runtime::ExecutionState::Paused => "paused",
            mt_plc_runtime::ExecutionState::BreakpointHit(_) => "paused",
            mt_plc_runtime::ExecutionState::Done => "stopped",
            _ => "running",
        };
        let _ = app.emit("breakpoint-hit", DebugStepResult {
            state: state_name.to_string(),
            current_line,
        });
        Ok(DebugStepResult {
            state: state_name.to_string(),
            current_line,
        })
    } else {
        Err("No runtime. Compile first.".into())
    }
}

#[tauri::command]
pub async fn continue_execution(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<DebugStepResult, String> {
    let mut plc = state.runtime.plc.lock().await;
    if let Some(ref mut runtime) = *plc {
        let exec_state = runtime.continue_from_breakpoint()?;
        let watch = runtime.watch_variables();
        let _ = app.emit("watch-update", &watch);
        let current_line = runtime.current_pc_line().map(|l| l as u32);
        let state_name = match &exec_state {
            mt_plc_runtime::ExecutionState::Paused => "paused",
            mt_plc_runtime::ExecutionState::BreakpointHit(_pc) => {
                // emit breakpoint event
                let line = runtime.current_pc_line().map(|l| l as u32);
                let _ = app.emit("breakpoint-hit", DebugStepResult {
                    state: "paused".to_string(),
                    current_line: line,
                });
                "paused"
            }
            mt_plc_runtime::ExecutionState::Done => "stopped",
            _ => "running",
        };
        Ok(DebugStepResult {
            state: state_name.to_string(),
            current_line,
        })
    } else {
        Err("No runtime. Compile first.".into())
    }
}

#[tauri::command]
pub async fn get_current_pc_line(
    state: tauri::State<'_, AppState>,
) -> Result<Option<u32>, String> {
    let plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref runtime) => Ok(runtime.current_pc_line().map(|l| l as u32)),
        None => Ok(None),
    }
}
