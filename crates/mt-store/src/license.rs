//! Pro by licence key, for direct-download builds (`license` feature).
//!
//! The key is bought through Polar in the Voltrus console and activated against the app's download worker
//! (`dl-<app>.voltrus.id`, the same service the older MacTools builds used):
//!
//! ```text
//! POST /license/activate  { key, device_id, app_version, platform } -> { valid, activation_id?, error? }
//! POST /license/validate  { key, activation_id, device_id, ... }    -> { valid, error? }
//! ```
//!
//! The key, its activation id and the device id live in the macOS Keychain (the app's own generic-password
//! items, sandbox-safe) or, on other platforms, in files under the app data directory. The key is validated
//! once per launch. Only an explicit `{ "valid": false }` takes Pro away: an unreachable server, a 5xx or
//! an answer that is not the worker's JSON keeps Pro, so a licensed user is never locked out offline.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::LicenseConfig;

const KEY_ITEM: &str = "licence";
const DEVICE_ITEM: &str = "device-id";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

// ---- secret storage -------------------------------------------------------------------------------------------

/// Where the licence key and the device id are kept.
pub trait Secrets: Send + Sync {
    fn get(&self, item: &str) -> Result<Option<String>, String>;
    fn set(&self, item: &str, value: &str) -> Result<(), String>;
    fn delete(&self, item: &str) -> Result<(), String>;
}

/// One file per item in a private directory. Used where there is no Keychain.
pub struct FileSecrets {
    dir: PathBuf,
}

impl FileSecrets {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Secrets for FileSecrets {
    fn get(&self, item: &str) -> Result<Option<String>, String> {
        match std::fs::read_to_string(self.dir.join(item)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read the saved licence: {e}")),
        }
    }

    fn set(&self, item: &str, value: &str) -> Result<(), String> {
        let fail = |e: std::io::Error| format!("Could not save the licence: {e}");
        std::fs::create_dir_all(&self.dir).map_err(fail)?;
        let path = self.dir.join(item);
        std::fs::write(&path, value).map_err(fail)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(fail)?;
        }
        Ok(())
    }

    fn delete(&self, item: &str) -> Result<(), String> {
        match std::fs::remove_file(self.dir.join(item)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Could not remove the saved licence: {e}")),
        }
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use super::Secrets;
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    /// `errSecItemNotFound`
    const NOT_FOUND: i32 = -25300;

    /// Generic-password items in the app's own Keychain (no access group needed, so it works in the sandbox).
    pub struct KeychainSecrets {
        service: String,
    }

    impl KeychainSecrets {
        pub fn new(service: impl Into<String>) -> Self {
            Self { service: service.into() }
        }
    }

    fn fail(e: security_framework::base::Error) -> String {
        format!("Keychain: {e}")
    }

    impl Secrets for KeychainSecrets {
        fn get(&self, item: &str) -> Result<Option<String>, String> {
            match get_generic_password(&self.service, item) {
                Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
                Err(e) if e.code() == NOT_FOUND => Ok(None),
                Err(e) => Err(fail(e)),
            }
        }
        fn set(&self, item: &str, value: &str) -> Result<(), String> {
            set_generic_password(&self.service, item, value.as_bytes()).map_err(fail)
        }
        fn delete(&self, item: &str) -> Result<(), String> {
            match delete_generic_password(&self.service, item) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == NOT_FOUND => Ok(()),
                Err(e) => Err(fail(e)),
            }
        }
    }
}

/// The platform's secret store: the Keychain on macOS, files under `data_dir` elsewhere.
pub fn platform_secrets(service: &str, data_dir: PathBuf) -> Box<dyn Secrets> {
    #[cfg(target_os = "macos")]
    {
        let _ = data_dir;
        Box::new(keychain::KeychainSecrets::new(service))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = service;
        Box::new(FileSecrets::new(data_dir.join("licence")))
    }
}

// ---- licence client -------------------------------------------------------------------------------------------

/// What is saved after a successful activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    key: String,
    activation_id: String,
}

#[derive(Default)]
struct State {
    /// The saved key has been validated (or found absent) in this process.
    checked: bool,
    saved: Option<Saved>,
}

/// The worker's answer to activate and validate.
#[derive(Deserialize)]
struct Reply {
    valid: bool,
    #[serde(default)]
    activation_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

enum Answer {
    Valid(Option<String>),
    /// The server looked at the key and refused it.
    Invalid(String),
    /// No verdict: network error, 5xx, or a body that is not the worker's JSON.
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    pub pro: bool,
}

pub struct Licence {
    server: String,
    app_version: String,
    secrets: Box<dyn Secrets>,
    http: reqwest::Client,
    state: Mutex<State>,
}

impl Licence {
    pub fn new(server: &str, app_version: &str, secrets: Box<dyn Secrets>) -> Self {
        Self {
            server: server.trim_end_matches('/').to_string(),
            app_version: app_version.to_string(),
            secrets,
            http: reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().expect("reqwest client"),
            state: Mutex::new(State::default()),
        }
    }

    /// Current entitlement. The first call of a process validates the saved key with the server; later calls
    /// answer from memory.
    pub async fn status(&self) -> Entitlement {
        let mut state = self.state.lock().await;
        if !state.checked {
            match self.load() {
                Ok(saved) => {
                    state.saved = match saved {
                        Some(saved) => self.revalidate(saved).await,
                        None => None,
                    };
                    state.checked = true;
                }
                // Keychain unavailable: free tier for now, asked again next time.
                Err(e) => eprintln!("licence: {e}"),
            }
        }
        entitlement(&state)
    }

    /// Activates `key` on this device and saves it. With `relocate` the licence is moved here: the server
    /// releases the device that holds it first.
    pub async fn activate(&self, key: &str, relocate: bool) -> Result<Entitlement, String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("Enter your licence key.".into());
        }
        let mut state = self.state.lock().await;
        let device_id = self.device_id()?;
        let body = json!({ "key": key, "device_id": device_id, "app_version": self.app_version, "platform": platform() });
        match self.ask(if relocate { "/license/move" } else { "/license/activate" }, &body).await {
            Answer::Valid(Some(activation_id)) => {
                let saved = Saved { key: key.to_string(), activation_id };
                self.store(&saved)?;
                state.saved = Some(saved);
                state.checked = true;
                Ok(entitlement(&state))
            }
            Answer::Valid(None) => Err("The licence server sent an incomplete answer. Try again.".into()),
            Answer::Invalid(reason) => Err(reason),
            Answer::Unreachable => Err("Could not reach the licence server. Check your internet connection.".into()),
        }
    }

    /// Forgets the key on this device.
    pub async fn remove(&self) -> Result<Entitlement, String> {
        let mut state = self.state.lock().await;
        self.secrets.delete(KEY_ITEM)?;
        state.saved = None;
        state.checked = true;
        Ok(entitlement(&state))
    }

    /// Validates a saved key. Returns it while the server accepts it or cannot be asked; `None` once refused.
    async fn revalidate(&self, saved: Saved) -> Option<Saved> {
        let device_id = self.device_id().unwrap_or_default();
        let body = json!({
            "key": saved.key,
            "activation_id": saved.activation_id,
            "device_id": device_id,
            "app_version": self.app_version,
            "platform": platform(),
        });
        match self.ask("/license/validate", &body).await {
            Answer::Valid(_) | Answer::Unreachable => Some(saved),
            Answer::Invalid(_) => {
                if let Err(e) = self.secrets.delete(KEY_ITEM) {
                    eprintln!("licence: {e}");
                }
                None
            }
        }
    }

    async fn ask(&self, path: &str, body: &Value) -> Answer {
        let res = match self.http.post(format!("{}{}", self.server, path)).json(body).send().await {
            Ok(res) => res,
            Err(_) => return Answer::Unreachable,
        };
        // A 5xx is the server's trouble, not a verdict on the key.
        if res.status().is_server_error() {
            return Answer::Unreachable;
        }
        match res.json::<Reply>().await {
            Ok(Reply { valid: true, activation_id, .. }) => Answer::Valid(activation_id),
            // The seat-limit text names the device holding the licence and says how to move it: keep it whole.
            Ok(Reply { error: Some(e), code: Some(c), .. }) if c == "seat_limit" => Answer::Invalid(e),
            Ok(Reply { error, .. }) => Answer::Invalid(friendly(error.as_deref().unwrap_or(""))),
            Err(_) => Answer::Unreachable,
        }
    }

    fn load(&self) -> Result<Option<Saved>, String> {
        Ok(self.secrets.get(KEY_ITEM)?.and_then(|raw| serde_json::from_str(&raw).ok()))
    }

    fn store(&self, saved: &Saved) -> Result<(), String> {
        self.secrets.set(KEY_ITEM, &serde_json::to_string(saved).map_err(|e| e.to_string())?)
    }

    /// Stable per-device id. A new id would use up another activation seat, so it is created once and kept
    /// even when the key is removed.
    fn device_id(&self) -> Result<String, String> {
        if let Some(id) = self.secrets.get(DEVICE_ITEM)?.filter(|id| !id.trim().is_empty()) {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.secrets.set(DEVICE_ITEM, &id)?;
        Ok(id)
    }
}

fn entitlement(state: &State) -> Entitlement {
    Entitlement { pro: state.saved.is_some() }
}

/// `macos-aarch64`: the platform names the download workers use.
fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Polar errors arrive as raw JSON text (`{"error":"ResourceNotFound","detail":"..."}`); show the readable part.
fn friendly(raw: &str) -> String {
    let text = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| ["detail", "message", "error"].iter().find_map(|k| v.get(*k)?.as_str().map(str::to_owned)))
        .unwrap_or_else(|| raw.trim().to_string());
    match text.as_str() {
        "" | "ResourceNotFound" | "Not found" => "That licence key was not recognised.".into(),
        _ => text,
    }
}

// ---- Tauri side -----------------------------------------------------------------------------------------------

/// Managed state: the configuration plus the client, created on first use because the app data directory and
/// version come from the running app.
pub struct Slot {
    pub config: LicenseConfig,
    client: tokio::sync::OnceCell<Licence>,
}

impl Slot {
    pub fn new(config: LicenseConfig) -> Self {
        Self { config, client: tokio::sync::OnceCell::new() }
    }

    pub async fn licence<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) -> Result<&Licence, String> {
        use tauri::Manager;
        self.client
            .get_or_try_init(|| async {
                let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
                let secrets = platform_secrets(&app.config().identifier, data_dir);
                Ok(Licence::new(&self.config.server, &app.package_info().version.to_string(), secrets))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Default, Clone)]
    struct MemorySecrets(Arc<std::sync::Mutex<HashMap<String, String>>>);

    impl Secrets for MemorySecrets {
        fn get(&self, item: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(item).cloned())
        }
        fn set(&self, item: &str, value: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(item.into(), value.into());
            Ok(())
        }
        fn delete(&self, item: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(item);
            Ok(())
        }
    }

    struct Request {
        path: String,
        body: Value,
    }

    /// A tiny in-process HTTP/1.1 server, so the tests exercise the real reqwest path.
    struct MockServer {
        base: String,
        requests: Arc<std::sync::Mutex<Vec<Request>>>,
    }

    impl MockServer {
        async fn start(handler: impl Fn(&Request) -> (u16, String) + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let (handler, log) = (Arc::new(handler), requests.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((mut sock, _)) = listener.accept().await else { return };
                    let (handler, log) = (handler.clone(), log.clone());
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let (head_end, content_length) = loop {
                            let n = sock.read(&mut chunk).await.unwrap_or(0);
                            if n == 0 {
                                return;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                                let len = head
                                    .lines()
                                    .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                                    .unwrap_or(0);
                                break (pos + 4, len);
                            }
                        };
                        while buf.len() < head_end + content_length {
                            let n = sock.read(&mut chunk).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                        let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                        let body = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
                        let req = Request { path, body };
                        let (status, reply) = handler(&req);
                        log.lock().unwrap().push(req);
                        let res = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                            reply.len()
                        );
                        let _ = sock.write_all(res.as_bytes()).await;
                        let _ = sock.shutdown().await;
                    });
                }
            });
            Self { base, requests }
        }

        fn count(&self, path: &str) -> usize {
            self.requests.lock().unwrap().iter().filter(|r| r.path == path).count()
        }

        fn last_body(&self) -> Value {
            self.requests.lock().unwrap().last().unwrap().body.clone()
        }
    }

    fn ok_activation(_: &Request) -> (u16, String) {
        (200, r#"{"valid":true,"activation_id":"act-1"}"#.into())
    }

    /// A base URL nothing listens on: port 1 is privileged and unused, so connects are refused.
    const DEAD: &str = "http://127.0.0.1:1";

    fn client(server: &str, secrets: &MemorySecrets) -> Licence {
        Licence::new(server, "1.6.0", Box::new(secrets.clone()))
    }

    /// A process that already holds an activated key (as after an earlier launch).
    fn with_saved_key(secrets: &MemorySecrets) {
        secrets.set(KEY_ITEM, r#"{"key":"VLT-AAAAA-BBBBB-CCCCC","activation_id":"act-1"}"#).unwrap();
        secrets.set(DEVICE_ITEM, "device-1").unwrap();
    }

    #[tokio::test]
    async fn activation_unlocks_pro_and_saves_the_key() {
        let server = MockServer::start(ok_activation).await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);

        assert_eq!(licence.status().await, Entitlement { pro: false });
        let got = licence.activate("  VLT-AAAAA-BBBBB-CCCCC ", false).await.unwrap();
        assert_eq!(got, Entitlement { pro: true });

        let body = server.last_body();
        assert_eq!(body["key"], "VLT-AAAAA-BBBBB-CCCCC", "the key is trimmed");
        assert_eq!(body["app_version"], "1.6.0");
        assert_eq!(body["platform"], platform());
        assert!(!body["device_id"].as_str().unwrap().is_empty());

        // A fresh process finds the key in the secret store.
        let relaunched = client(&server.base, &secrets);
        assert!(relaunched.status().await.pro);
        assert_eq!(server.count("/license/validate"), 1);
    }

    #[tokio::test]
    async fn device_id_is_created_once_and_reused() {
        let server = MockServer::start(ok_activation).await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);
        licence.activate("K1", false).await.unwrap();
        let first = server.last_body()["device_id"].clone();
        licence.remove().await.unwrap();
        licence.activate("K2", false).await.unwrap();
        assert_eq!(server.last_body()["device_id"], first, "removing a key keeps the device id");
    }

    #[tokio::test]
    async fn refused_key_is_reported_and_nothing_is_saved() {
        let server = MockServer::start(|_| (200, r#"{"valid":false,"error":"{\"error\":\"NotPermitted\",\"detail\":\"License key reached its activation limit\"}"}"#.into())).await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);
        let err = licence.activate("K", false).await.unwrap_err();
        assert_eq!(err, "License key reached its activation limit");
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
        assert!(!licence.status().await.pro);
    }

    #[tokio::test]
    async fn unknown_key_gets_a_plain_message() {
        let server = MockServer::start(|_| (200, r#"{"valid":false,"error":"ResourceNotFound"}"#.into())).await;
        let licence = client(&server.base, &MemorySecrets::default());
        assert_eq!(licence.activate("nope", false).await.unwrap_err(), "That licence key was not recognised.");
    }

    #[tokio::test]
    async fn seat_limit_text_is_kept_and_relocate_uses_the_move_endpoint() {
        let text = "This licence is active on another Mac. Move licence here to use it on this one.";
        let server = MockServer::start(move |r| match r.path.as_str() {
            "/license/move" => ok_activation(r),
            _ => (200, format!(r#"{{"valid":false,"code":"seat_limit","error":"{text}"}}"#)),
        })
        .await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);

        assert_eq!(licence.activate("K", false).await.unwrap_err(), text);
        assert_eq!(server.count("/license/move"), 0);
        assert!(licence.activate("K", true).await.unwrap().pro);
        assert_eq!(server.count("/license/move"), 1);
    }

    #[tokio::test]
    async fn activation_needs_a_reachable_server_and_a_key() {
        let secrets = MemorySecrets::default();
        let licence = client(DEAD, &secrets);
        assert_eq!(
            licence.activate("K", false).await.unwrap_err(),
            "Could not reach the licence server. Check your internet connection."
        );
        assert_eq!(licence.activate("   ", false).await.unwrap_err(), "Enter your licence key.");
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());

        let broken = MockServer::start(|_| (503, "down".into())).await;
        assert!(client(&broken.base, &secrets).activate("K", false).await.unwrap_err().starts_with("Could not reach"));
    }

    #[tokio::test]
    async fn valid_answer_without_activation_id_is_not_trusted() {
        let server = MockServer::start(|_| (200, r#"{"valid":true}"#.into())).await;
        let secrets = MemorySecrets::default();
        assert!(client(&server.base, &secrets).activate("K", false).await.is_err());
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
    }

    #[tokio::test]
    async fn launch_validation_keeps_a_valid_key() {
        let server = MockServer::start(|_| (200, r#"{"valid":true}"#.into())).await;
        let secrets = MemorySecrets::default();
        with_saved_key(&secrets);
        let licence = client(&server.base, &secrets);
        assert!(licence.status().await.pro);
        let body = server.last_body();
        assert_eq!(body["key"], "VLT-AAAAA-BBBBB-CCCCC");
        assert_eq!(body["activation_id"], "act-1");
        assert_eq!(body["device_id"], "device-1");
        // Validated once per process.
        assert!(licence.status().await.pro);
        assert_eq!(server.count("/license/validate"), 1);
    }

    #[tokio::test]
    async fn explicit_invalid_drops_pro_and_the_saved_key() {
        let server = MockServer::start(|_| (200, r#"{"valid":false,"error":"License key is revoked"}"#.into())).await;
        let secrets = MemorySecrets::default();
        with_saved_key(&secrets);
        let licence = client(&server.base, &secrets);
        assert!(!licence.status().await.pro);
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
        assert!(!client(&server.base, &secrets).status().await.pro, "stays dropped after a relaunch");
    }

    #[tokio::test]
    async fn network_failure_keeps_pro() {
        let secrets = MemorySecrets::default();
        with_saved_key(&secrets);
        assert!(client(DEAD, &secrets).status().await.pro);
        assert!(secrets.get(KEY_ITEM).unwrap().is_some(), "the key survives for the next launch");
    }

    #[tokio::test]
    async fn server_errors_and_garbage_keep_pro() {
        for (status, body) in [(500, r#"{"valid":false,"error":"License server not configured"}"#), (502, "bad gateway"), (404, "Not found"), (200, "<html>")] {
            let server = MockServer::start(move |_| (status, body.into())).await;
            let secrets = MemorySecrets::default();
            with_saved_key(&secrets);
            assert!(client(&server.base, &secrets).status().await.pro, "{status} {body}");
            assert!(secrets.get(KEY_ITEM).unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn without_a_key_the_server_is_never_asked() {
        let server = MockServer::start(ok_activation).await;
        let licence = client(&server.base, &MemorySecrets::default());
        assert!(!licence.status().await.pro);
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_the_key_locks_pro() {
        let server = MockServer::start(ok_activation).await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);
        licence.activate("K", false).await.unwrap();
        assert_eq!(licence.remove().await.unwrap(), Entitlement { pro: false });
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
        assert!(!client(&server.base, &secrets).status().await.pro);
    }

    #[test]
    fn polar_errors_are_unwrapped() {
        assert_eq!(friendly(r#"{"detail":"Too many activations"}"#), "Too many activations");
        assert_eq!(friendly(r#"{"error":"Expired"}"#), "Expired");
        assert_eq!(friendly("License is expired"), "License is expired");
        assert_eq!(friendly(""), "That licence key was not recognised.");
    }

    #[test]
    fn file_secrets_round_trip() {
        let dir = std::env::temp_dir().join(format!("mt-store-licence-{}", uuid::Uuid::new_v4()));
        let secrets = FileSecrets::new(&dir);
        assert_eq!(secrets.get("a").unwrap(), None);
        secrets.set("a", "1").unwrap();
        assert_eq!(secrets.get("a").unwrap().as_deref(), Some("1"));
        secrets.delete("a").unwrap();
        secrets.delete("a").unwrap();
        assert_eq!(secrets.get("a").unwrap(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn license_section_parses() {
        let c: LicenseConfig = serde_json::from_str(r#"{"server":"https://x","buy_url":"https://y"}"#).unwrap();
        assert_eq!(c.price, None);
    }
}
