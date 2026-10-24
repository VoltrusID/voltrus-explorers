use crate::commands::target::RemoteForceValue;
use crate::state::AppState;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct WatchedVar {
    pub name: String,
    pub value: serde_json::Value,
    pub type_name: String,
    pub quality: String,
}

fn plc_value_to_json(val: &mt_plc_runtime::PlcValue) -> serde_json::Value {
    match val {
        mt_plc_runtime::PlcValue::Bool(b) => serde_json::Value::Bool(*b),
        mt_plc_runtime::PlcValue::Byte(b) => serde_json::json!(*b),
        mt_plc_runtime::PlcValue::Int(i) => serde_json::json!(*i),
        mt_plc_runtime::PlcValue::Dint(i) => serde_json::json!(*i),
        mt_plc_runtime::PlcValue::Real(f) => serde_json::json!(*f),
        mt_plc_runtime::PlcValue::Time(t) => serde_json::json!(*t),
        mt_plc_runtime::PlcValue::String(s) => serde_json::json!(s),
        mt_plc_runtime::PlcValue::Word(w) => serde_json::json!(*w),
        mt_plc_runtime::PlcValue::Dword(d) => serde_json::json!(*d),
        mt_plc_runtime::PlcValue::Array(arr) => {
            let items: Vec<serde_json::Value> = arr.iter().map(|v| plc_value_to_json(v)).collect();
            serde_json::json!(items)
        }
        mt_plc_runtime::PlcValue::Struct(fields) => {
            let map: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), plc_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        mt_plc_runtime::PlcValue::Enum(e) => serde_json::json!(e),
    }
}

#[tauri::command]
pub async fn get_watched_variables(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WatchedVar>, String> {
    let plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref runtime) => {
            let vars = runtime.watch_variables();
            Ok(vars
                .iter()
                .map(|v| WatchedVar {
                    name: v.name.clone(),
                    value: plc_value_to_json(&v.value),
                    type_name: v.type_name.clone(),
                    quality: v.quality.clone(),
                })
                .collect())
        }
        None => Ok(vec![]),
    }
}

#[tauri::command]
pub async fn force_variable(
    state: tauri::State<'_, AppState>,
    name: String,
    value: serde_json::Value,
) -> Result<(), String> {
    // Remote mode: route by target kind — gRPC (Linux) or WebSocket (ESP32)
    let connected = state
        .target
        .connected
        .load(std::sync::atomic::Ordering::SeqCst);
    if connected {
        let kind = state.target.target_kind.lock().await.clone();
        match kind {
            crate::state::TargetKind::Grpc => {
                let resp = {
                    let guard = state.target.grpc_client.lock().await;
                    let Some(client) = guard.as_ref() else {
                        // No persistent gRPC connection — fall through to local below
                        return force_local(&state, &name, value).await;
                    };
                    let mut c = client.clone();
                    match &value {
                        serde_json::Value::Bool(b) => c.force_bool(&name, *b).await,
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                c.force_dint(&name, i as i32).await
                            } else if let Some(f) = n.as_f64() {
                                c.force_real(&name, f as f32).await
                            } else {
                                c.force_dint(&name, 0).await
                            }
                        }
                        serde_json::Value::String(s) => c.force_string(&name, s).await,
                        _ => c.force_dint(&name, 0).await,
                    }
                }
                .map_err(|e| format!("gRPC force failed: {e}"))?;
                if !resp.success {
                    return Err(resp.error_message);
                }
                return Ok(());
            }
            crate::state::TargetKind::WebSocket => {
                let force_val = RemoteForceValue::from_json(&value);
                crate::commands::target::send_remote_force(
                    &state.target.ws_sink,
                    &name,
                    &force_val,
                )
                .await?;
                return Ok(());
            }
        }
    }

    force_local(&state, &name, value).await
}

/// Force a variable on the local softPLC.
async fn force_local(
    state: &tauri::State<'_, AppState>,
    name: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let mut plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref mut runtime) => {
            let plc_val = match value {
                serde_json::Value::Bool(b) => mt_plc_runtime::PlcValue::Bool(b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        mt_plc_runtime::PlcValue::Dint(i as i32)
                    } else if let Some(f) = n.as_f64() {
                        mt_plc_runtime::PlcValue::Real(f as f32)
                    } else {
                        mt_plc_runtime::PlcValue::Dint(0)
                    }
                }
                serde_json::Value::String(s) => mt_plc_runtime::PlcValue::String(s),
                _ => mt_plc_runtime::PlcValue::Dint(0),
            };
            runtime.force_variable(name, plc_val)
        }
        None => Err("Runtime not running".to_string()),
    }
}

#[tauri::command]
pub async fn unforce_variable(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    // Remote mode: route by target kind — gRPC (Linux) or WebSocket (ESP32)
    let connected = state
        .target
        .connected
        .load(std::sync::atomic::Ordering::SeqCst);
    if connected {
        let kind = state.target.target_kind.lock().await.clone();
        match kind {
            crate::state::TargetKind::Grpc => {
                let resp = {
                    let guard = state.target.grpc_client.lock().await;
                    let Some(client) = guard.as_ref() else {
                        return unforce_local(&state, &name).await;
                    };
                    let mut c = client.clone();
                    c.unforce_variable(&name).await
                }
                .map_err(|e| format!("gRPC unforce failed: {e}"))?;
                if !resp.success {
                    return Err(resp.error_message);
                }
                return Ok(());
            }
            crate::state::TargetKind::WebSocket => {
                crate::commands::target::send_remote_unforce(&state.target.ws_sink, &name).await?;
                return Ok(());
            }
        }
    }

    unforce_local(&state, &name).await
}

/// Unforce a variable on the local softPLC.
async fn unforce_local(state: &tauri::State<'_, AppState>, name: &str) -> Result<(), String> {
    let mut plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref mut runtime) => {
            runtime.unforce_variable(name);
            Ok(())
        }
        None => Err("Runtime not running".to_string()),
    }
}
