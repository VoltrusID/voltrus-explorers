use crate::state::AppState;
use crate::commands::target::RemoteForceValue;

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
            let map: serde_json::Map<String, serde_json::Value> = fields.iter()
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
            Ok(vars.iter().map(|v| WatchedVar {
                name: v.name.clone(),
                value: plc_value_to_json(&v.value),
                type_name: v.type_name.clone(),
                quality: v.quality.clone(),
            }).collect())
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
    // Remote mode: send force command to ESP32 via persistent WebSocket
    let connected = state.target.connected.load(std::sync::atomic::Ordering::SeqCst);
    if connected {
        let force_val = RemoteForceValue::from_json(&value);
        crate::commands::target::send_remote_force(&state.target.ws_sink, &name, &force_val).await?;
        return Ok(());
    }

    // Local softPLC mode
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
            runtime.force_variable(&name, plc_val)
        }
        None => Err("Runtime not running".to_string()),
    }
}

#[tauri::command]
pub async fn unforce_variable(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    // Remote mode: send unforce command to ESP32 via persistent WebSocket
    let connected = state.target.connected.load(std::sync::atomic::Ordering::SeqCst);
    if connected {
        crate::commands::target::send_remote_unforce(&state.target.ws_sink, &name).await?;
        return Ok(());
    }

    // Local softPLC mode
    let mut plc = state.runtime.plc.lock().await;
    match *plc {
        Some(ref mut runtime) => {
            runtime.unforce_variable(&name);
            Ok(())
        }
        None => Err("Runtime not running".to_string()),
    }
}
