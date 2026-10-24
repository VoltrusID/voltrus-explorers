//! Pro by licence key (direct-sale builds, `keys` feature).
//!
//! A Voltrus console licence (`VLT-XXXXX-XXXXX-XXXXX`, issued by the purchase of "Modbus Poll for Windows Pro")
//! is activated once against the licence server. The server binds it to this PC (one seat) and answers with an
//! activation id; both are stored in `license.json` in the app data directory together with a hash of the
//! PC's machine id. After that the app never contacts the server again: Pro is unlocked while the file is
//! present and its machine hash matches this PC, so a copied file does nothing on another PC.
//!
//! The code is open source, so this is an honest licence check, not copy protection.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LICENSE_FILE: &str = "license.json";

/// Same alphabet the console draws licence codes from (no I, L, O, 0, 1).
const CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// `keys.json` next to `store.json`: where this build sells and activates Pro.
#[derive(Debug, Clone, Deserialize)]
pub struct KeyConfig {
    /// Console app slug the licence is bound to; sent as `app` so the server only accepts keys of this app.
    pub app: String,
    /// Licence server, e.g. `https://dl-modbus-poll.voltrus.id`.
    pub server: String,
    /// Page where Pro is bought (https).
    pub buy_url: String,
    /// Price shown on the buy button, e.g. `$14.99`.
    #[serde(default)]
    pub price: Option<String>,
}

/// What `license.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    pub key: String,
    pub activation_id: String,
    /// Hash of this PC's machine id (see [`device_id`]).
    pub device: String,
    /// Unix seconds.
    pub activated_at: u64,
}

/// Upper-cases and checks the shape `VLT-XXXXX-XXXXX-XXXXX`.
pub fn normalize_key(raw: &str) -> Result<String, String> {
    let key = raw.trim().to_uppercase();
    let groups: Vec<&str> = key.split('-').collect();
    let shaped = groups.len() == 4
        && groups[0] == "VLT"
        && groups[1..].iter().all(|g| g.chars().count() == 5 && g.chars().all(|c| CODE_ALPHABET.contains(c)));
    if shaped {
        Ok(key)
    } else {
        Err("That is not a Voltrus licence key. It looks like VLT-XXXXX-XXXXX-XXXXX and is on your Licenses page at console.voltrus.id.".into())
    }
}

/// Stable id of this PC for the licence server and the local file: a hash, never the raw machine id.
pub fn device_id(app: &str, machine: &str) -> String {
    let digest = Sha256::digest(format!("voltrus:{app}:{}", machine.trim().to_lowercase()).as_bytes());
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// `MachineGuid` out of `reg query HKLM\SOFTWARE\Microsoft\Cryptography /v MachineGuid`.
pub fn parse_machine_guid(reg_output: &str) -> Option<String> {
    reg_output.lines().find(|l| l.contains("MachineGuid")).and_then(|l| l.split_whitespace().last()).filter(|g| g.len() >= 16 && *g != "MachineGuid").map(str::to_string)
}

/// The OS's machine id: the Windows `MachineGuid`, or `/etc/machine-id` on Linux (so the logic runs in tests).
pub fn machine_id() -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let out = std::process::Command::new(format!(r"{root}\System32\reg.exe"))
            .args(["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Could not read this PC's id: {e}"))?;
        parse_machine_guid(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "Could not read this PC's id (MachineGuid).".to_string())
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/etc/machine-id")
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Licence keys are only supported on Windows.".to_string())
    }
}

pub fn read_activation(dir: &Path) -> Option<Activation> {
    serde_json::from_slice(&std::fs::read(dir.join(LICENSE_FILE)).ok()?).ok()
}

pub fn write_activation(dir: &Path, activation: &Activation) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not save the licence: {e}"))?;
    let json = serde_json::to_vec_pretty(activation).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(LICENSE_FILE), json).map_err(|e| format!("Could not save the licence: {e}"))
}

pub fn remove_activation(dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(dir.join(LICENSE_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Could not remove the licence: {e}")),
    }
}

/// Whether the stored activation unlocks Pro on the PC with device id `device`.
pub fn unlocks(activation: Option<&Activation>, device: &str) -> bool {
    activation.is_some_and(|a| !a.key.is_empty() && !a.activation_id.is_empty() && a.device == device)
}

#[derive(Deserialize)]
struct ServerAnswer {
    #[serde(default)]
    valid: bool,
    activation_id: Option<String>,
    error: Option<String>,
}

/// The licence server's answer to an activation as a user-facing result: the activation id, or a message.
pub fn read_answer(http_status: u16, body: &str) -> Result<String, String> {
    let answer: Option<ServerAnswer> = serde_json::from_str(body).ok();
    match answer {
        Some(ServerAnswer { valid: true, activation_id: Some(id), .. }) if !id.is_empty() => Ok(id),
        Some(ServerAnswer { error: Some(e), .. }) => Err(explain(&e)),
        _ if http_status >= 500 => Err("The licence server is not available right now. Try again in a few minutes.".into()),
        _ => Err(format!("Unexpected answer from the licence server (HTTP {http_status}).")),
    }
}

fn explain(server_error: &str) -> String {
    let lower = server_error.to_lowercase();
    if lower.contains("notfound") || lower.contains("not found") {
        "This key was not found. Check it for typos, and that it is a Modbus Poll for Windows Pro key.".into()
    } else if lower.contains("not active") {
        "This licence is not active (it may have been refunded). Contact support@voltrus.id.".into()
    } else if lower.contains("expired") {
        "This licence has expired. Contact support@voltrus.id.".into()
    } else if lower.contains("seat") {
        "This key is already activated on another PC. Release that PC on the Licenses page at console.voltrus.id, then activate again.".into()
    } else {
        server_error.to_string()
    }
}

/// Activates `key` for this PC and stores the result in `dir`.
pub async fn activate(config: &KeyConfig, dir: &Path, raw_key: &str, app_version: &str) -> Result<Activation, String> {
    let key = normalize_key(raw_key)?;
    let device = device_id(&config.app, &machine_id()?);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let response = client
        .post(format!("{}/license/activate", config.server.trim_end_matches('/')))
        .json(&serde_json::json!({
            "key": key,
            "app": config.app,
            "device_id": device,
            "app_version": app_version,
            "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        }))
        .send()
        .await
        .map_err(|_| "Could not reach the licence server. Check your internet connection and try again.".to_string())?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let activation_id = read_answer(status, &body)?;
    let activation = Activation {
        key,
        activation_id,
        device,
        activated_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    write_activation(dir, &activation)?;
    Ok(activation)
}

/// The directory `license.json` lives in.
pub fn license_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    use tauri::Manager;
    app.path().app_data_dir().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_normalized_and_checked() {
        assert_eq!(normalize_key("  vlt-abcde-fghjk-23456 \n").unwrap(), "VLT-ABCDE-FGHJK-23456");
        assert!(normalize_key("VLT-ABCDE-FGHJK").is_err());
        assert!(normalize_key("VOLTRUS-ABCDE-FGHJK-23456").is_err());
        assert!(normalize_key("VLT-ABCDE-FGHJK-2345O").is_err(), "O is not in the alphabet");
        assert!(normalize_key("VLT-ABCDE-FGHJK-234567").is_err());
        assert!(normalize_key("").is_err());
    }

    #[test]
    fn device_id_is_a_stable_hash_per_app_and_pc() {
        let a = device_id("modbus-poll-windows", "ABCD-1234-ABCD-1234");
        assert_eq!(a, device_id("modbus-poll-windows", " abcd-1234-abcd-1234\r\n"));
        assert_eq!(a.len(), 32);
        assert!(!a.contains("abcd"));
        assert_ne!(a, device_id("other", "ABCD-1234-ABCD-1234"));
        assert_ne!(a, device_id("modbus-poll-windows", "ABCD-1234-ABCD-1235"));
    }

    #[test]
    fn machine_guid_is_read_from_reg_output() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    8a4e5b1c-1111-4c2d-9f00-0123456789ab\r\n\r\n";
        assert_eq!(parse_machine_guid(out).as_deref(), Some("8a4e5b1c-1111-4c2d-9f00-0123456789ab"));
        assert_eq!(parse_machine_guid("ERROR: The system was unable to find the specified registry key or value."), None);
    }

    #[test]
    fn server_answers_become_messages() {
        assert_eq!(read_answer(200, r#"{"valid":true,"activation_id":"lic:act"}"#).unwrap(), "lic:act");
        let nf = read_answer(200, r#"{"valid":false,"error":"ResourceNotFound"}"#).unwrap_err();
        assert!(nf.contains("not found"), "{nf}");
        let seats = read_answer(200, r#"{"valid":false,"error":"All 1 seat(s) are in use — release a device first"}"#).unwrap_err();
        assert!(seats.contains("another PC"), "{seats}");
        assert!(read_answer(200, r#"{"valid":false,"error":"License is expired"}"#).unwrap_err().contains("expired"));
        assert!(read_answer(200, r#"{"valid":false,"error":"License is not active"}"#).unwrap_err().contains("not active"));
        assert!(read_answer(503, "upstream").unwrap_err().contains("not available"));
        assert!(read_answer(200, r#"{"valid":true}"#).is_err(), "valid without an activation id is not an activation");
    }

    #[test]
    fn activation_unlocks_only_on_the_pc_it_was_made_on() {
        let a = Activation { key: "VLT-ABCDE-FGHJK-23456".into(), activation_id: "x".into(), device: "dev1".into(), activated_at: 1 };
        assert!(unlocks(Some(&a), "dev1"));
        assert!(!unlocks(Some(&a), "dev2"));
        assert!(!unlocks(None, "dev1"));
        assert!(!unlocks(Some(&Activation { activation_id: String::new(), ..a }), "dev1"));
    }

    #[test]
    fn activation_file_round_trips_and_is_removable() {
        let dir = std::env::temp_dir().join(format!("mt-store-key-test-{}", std::process::id()));
        let a = Activation { key: "VLT-ABCDE-FGHJK-23456".into(), activation_id: "x".into(), device: "dev1".into(), activated_at: 7 };
        assert_eq!(read_activation(&dir), None);
        write_activation(&dir, &a).unwrap();
        assert_eq!(read_activation(&dir), Some(a));
        remove_activation(&dir).unwrap();
        remove_activation(&dir).unwrap();
        assert_eq!(read_activation(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_activation_file_is_no_activation() {
        let dir = std::env::temp_dir().join(format!("mt-store-key-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LICENSE_FILE), b"{not json").unwrap();
        assert_eq!(read_activation(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Against the live licence server: `MT_LIVE_KEY=VLT-... cargo test -p mt-store --features keys -- --ignored live`.
    /// Activates the key for this machine, so use a test licence and release its seat afterwards.
    #[tokio::test]
    #[ignore]
    async fn live_activation_round_trip() {
        let key = std::env::var("MT_LIVE_KEY").expect("set MT_LIVE_KEY");
        let config = KeyConfig {
            app: "modbus-poll-windows".into(),
            server: "https://dl-modbus-poll.voltrus.id".into(),
            buy_url: "https://voltrus.id/modbus-poll/windows/".into(),
            price: None,
        };
        let dir = std::env::temp_dir().join(format!("mt-store-live-{}", std::process::id()));
        let activation = activate(&config, &dir, &key, "0.0.0-test").await.unwrap();
        assert!(activation.activation_id.contains(':'));
        assert!(unlocks(read_activation(&dir).as_ref(), &device_id(&config.app, &machine_id().unwrap())));
        // The same PC may activate again.
        activate(&config, &dir, &key, "0.0.0-test").await.unwrap();
        let wrong = activate(&config, &dir, "VLT-AAAAA-BBBBB-CCCCC", "0.0.0-test").await.unwrap_err();
        assert!(wrong.contains("not found"), "{wrong}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
