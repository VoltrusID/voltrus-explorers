mod export;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

// ---------------------------------------------------------------------------
// Shared types (used by both Tauri commands and export module)
// ---------------------------------------------------------------------------

fn default_min() -> f64 {
    0.0
}
fn default_max() -> f64 {
    100.0
}
fn default_color() -> String {
    "#22c55e".to_string()
}
fn default_font_size() -> f64 {
    14.0
}
fn default_bg() -> String {
    "#1e1e2e".to_string()
}
fn default_false() -> bool {
    false
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HmiWidget {
    pub id: String,
    #[serde(rename = "type")]
    pub widget_type: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub label: String,
    #[serde(default)]
    pub variable_name: String,
    #[serde(default = "default_min")]
    pub min: f64,
    #[serde(default = "default_max")]
    pub max: f64,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(default)]
    pub required_role: Option<String>,
    #[serde(default)]
    pub z_index: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HmiScreen {
    pub id: String,
    pub name: String,
    #[serde(default = "default_bg")]
    pub background_color: String,
    pub widgets: Vec<HmiWidget>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AlarmRule {
    pub id: String,
    pub source: String, // "scada" | "plc"
    #[serde(default)]
    pub variable_name: String,
    pub severity: String, // "critical" | "warning"
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccessConfig {
    pub operator_pin_hash: String,
    pub supervisor_pin_hash: String,
    pub salt: String,
    #[serde(default = "default_auto_logout")]
    pub auto_logout_minutes: u32,
}

fn default_auto_logout() -> u32 {
    5
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSet {
    #[serde(flatten)]
    pub languages: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub screens: Vec<HmiScreen>,
    pub plc_ip: String,
    #[serde(default)]
    pub scada_url: Option<String>,
    #[serde(default)]
    pub alarms: Vec<AlarmRule>,
    #[serde(default)]
    pub access_config: Option<AccessConfig>,
    #[serde(default)]
    pub translations: Option<TranslationSet>,
    #[serde(default)]
    pub default_language: Option<String>,
    #[serde(default = "default_false")]
    pub kiosk_mode: bool,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn export_hmi(request: ExportRequest) -> Result<String, String> {
    export::build_export_html(&request)
}

#[tauri::command]
async fn deploy_to_esp32(ip: String, html: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://{}/api/hmi/upload", ip);
    let resp = client
        .post(&url)
        .header("Content-Type", "text/html")
        .body(html)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        Ok("Deployed successfully".into())
    } else {
        Err(format!("Deploy failed: HTTP {}", resp.status()))
    }
}

#[tauri::command]
async fn deploy_to_rpi5(ip: String, port: u16, html: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://{}:{}/api/deploy", ip, port);
    let resp = client
        .post(&url)
        .header("Content-Type", "text/html")
        .body(html)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        Ok("Deployed to RPi5 successfully".into())
    } else {
        Err(format!("Deploy failed: HTTP {}", resp.status()))
    }
}

#[tauri::command]
fn save_project_data(path: String, data: String) -> Result<(), String> {
    std::fs::write(&path, data).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_project_data(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            export_hmi,
            deploy_to_esp32,
            deploy_to_rpi5,
            save_project_data,
            load_project_data,
        ])
        .setup(|app| {
            #[cfg(desktop)]
            {
                app.handle().plugin(tauri_plugin_dialog::init())?;
                // plugin-fs: currently using custom save/load commands; plugin kept for future direct fs access
                app.handle().plugin(tauri_plugin_fs::init())?;
                app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
                app.handle().plugin(tauri_plugin_deep_link::init())?;

                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register("voltrus-hmi")?;
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    if let Some(url) = event.urls().first() {
                        let _ = handle.emit("deep-link", url.to_string());
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
