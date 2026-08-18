use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const LICENSE_SERVER: &str = "https://dl-voltrus-plc.voltrus.id";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LicenseState {
    pub activated: bool,
    pub license_key: String,
    pub activation_id: String,
    pub device_id: String,
}

#[derive(Serialize, Deserialize)]
struct ActivateRequest {
    key: String,
    device_id: String,
}

#[derive(Serialize, Deserialize)]
struct ValidateRequest {
    key: String,
    activation_id: String,
}

#[derive(Serialize, Deserialize)]
struct LicenseResponse {
    valid: bool,
    activation_id: Option<String>,
    error: Option<String>,
}

fn license_file_path() -> PathBuf {
    let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.join("voltrus-plc").join("license.json")
}

/// Stable per-installation device identifier.
///
/// 1. Cached `device_id` file next to `license.json` (stable across launches)
/// 2. macOS: `IOPlatformUUID` via `ioreg` (stable across reinstalls)
/// 3. Fresh random UUID, persisted for next launch
///
/// The PID fallback that used to live here rotated on every launch and
/// burned a Polar seat per app start — never fall back to it.
fn generate_device_id() -> String {
    let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let cache = dir.join("voltrus-plc").join("device_id");

    if let Ok(cached) = fs::read_to_string(&cache) {
        let trimmed = cached.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    let mut id = String::new();
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("IOPlatformUUID") {
                    if let Some(start) = line.find('"') {
                        let rest = &line[start + 1..];
                        if let Some(end) = rest.find('"') {
                            id = rest[..end].to_string();
                        }
                    }
                }
            }
        }
    }
    if id.is_empty() {
        id = uuid::Uuid::new_v4().to_string();
    }

    // Persist so non-macOS installs (and ioreg failures) stay stable.
    if let Some(parent) = cache.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&cache, &id);
    id
}

fn load_license() -> Option<LicenseState> {
    let path = license_file_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            serde_json::from_str(&data).ok()
        } else {
            None
        }
    } else {
        None
    }
}

fn save_license(state: &LicenseState) -> Result<(), String> {
    let path = license_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn activate_license(license_key: String) -> Result<LicenseState, String> {
    // ioreg is a subprocess — keep it off the async worker
    let device_id = tauri::async_runtime::spawn_blocking(generate_device_id)
        .await
        .map_err(|e| format!("Device ID generation failed: {e}"))?;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/license/activate", LICENSE_SERVER))
        .json(&ActivateRequest {
            key: license_key.clone(),
            device_id: device_id.clone(),
        })
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    let result: LicenseResponse = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    if !result.valid {
        return Err(result.error.unwrap_or_else(|| "Activation failed".to_string()));
    }

    let state = LicenseState {
        activated: true,
        license_key,
        activation_id: result.activation_id.unwrap_or_default(),
        device_id,
    };

    save_license(&state)?;
    Ok(state)
}

#[tauri::command]
pub async fn validate_license() -> Result<LicenseState, String> {
    let state = load_license().ok_or("No license found")?;

    if !state.activated {
        return Err("License not activated".to_string());
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/license/validate", LICENSE_SERVER))
        .json(&ValidateRequest {
            key: state.license_key.clone(),
            activation_id: state.activation_id.clone(),
        })
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    let result: LicenseResponse = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    if !result.valid {
        let mut invalid_state = state;
        invalid_state.activated = false;
        save_license(&invalid_state)?;
        return Err(result.error.unwrap_or_else(|| "License invalid".to_string()));
    }

    Ok(state)
}

#[tauri::command]
pub fn get_license_status() -> LicenseState {
    load_license().unwrap_or(LicenseState {
        activated: false,
        license_key: String::new(),
        activation_id: String::new(),
        device_id: String::new(),
    })
}

#[tauri::command]
pub async fn deactivate_license() -> Result<(), String> {
    let state = load_license();

    // Release the Polar activation seat first, so it doesn't leak when the
    // user moves to another machine. Local file removal follows on success.
    if let Some(state) = state.filter(|s| s.activated && !s.activation_id.is_empty()) {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/license/deactivate", LICENSE_SERVER))
            .json(&serde_json::json!({
                "key": state.license_key,
                "activation_id": state.activation_id,
            }))
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        #[derive(serde::Deserialize)]
        struct DeactivateResponse {
            ok: bool,
            #[serde(default)]
            error: Option<String>,
        }
        let result: DeactivateResponse = resp
            .json()
            .await
            .map_err(|e| format!("Invalid response: {}", e))?;
        if !result.ok {
            return Err(result
                .error
                .unwrap_or_else(|| "Deactivation failed".to_string()));
        }
    }

    let path = license_file_path();
    let remove = move || {
        if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    tauri::async_runtime::spawn_blocking(remove)
        .await
        .map_err(|e| e.to_string())?
}
