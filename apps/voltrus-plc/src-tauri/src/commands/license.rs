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

fn generate_device_id() -> String {
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
        id = format!("voltrus-{:x}", std::process::id());
    }
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
    let device_id = generate_device_id();
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
pub fn deactivate_license() -> Result<(), String> {
    let path = license_file_path();
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
