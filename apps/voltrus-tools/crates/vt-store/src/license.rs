//! The unlock by licence key, for direct-download builds (`license` feature).
//!
//! The key is bought through Polar in the Voltrus console and activated
//! against the app's download worker (`dl-voltrus-tools.voltrus.id`), with the
//! protocol of the explorer apps' `mt-store` licence feature:
//!
//! ```text
//! POST /license/activate   { key, device_id, app_version, platform } -> { valid, activation_id?, error? }
//! POST /license/validate   { key, activation_id, device_id, ... }    -> { valid, error? }
//! POST /license/move       { key, device_id, ... }                   -> { valid, activation_id?, error?, code? }
//! POST /license/deactivate { key, activation_id, device_id }         -> { ok }
//! ```
//!
//! The worker serves one app, so its host names the app; the bodies carry no
//! app name. The key, its activation id and a per-device id live in the
//! macOS Keychain (the app's own generic-password items) or, elsewhere, in
//! files beside the licence cache. The key is validated once per launch. Only
//! an explicit `{ "valid": false }` takes the unlock away: an unreachable
//! server, a 5xx or an answer that isn't the worker's JSON keeps it, so a
//! buyer is never locked out offline.
//!
//! [`Keys`] is the blocking front the app calls from a background thread;
//! builds without the feature get one that never has a key.

use std::path::PathBuf;

use serde::Deserialize;

/// The `license` section of `store.json`, as in the explorer apps.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LicenseConfig {
    /// The download worker that activates and validates keys.
    pub server: String,
    /// The console page that sells the key; the Buy button opens it.
    pub buy_url: String,
    /// The display price, e.g. `$49`.
    #[serde(default)]
    pub price: Option<String>,
}

#[derive(Deserialize)]
struct StoreJson {
    license: LicenseConfig,
}

impl LicenseConfig {
    /// The configuration built into the app from `store.json`.
    pub fn bundled() -> Self {
        Self::parse(include_str!("../store.json")).unwrap_or_else(|_| Self {
            server: "https://dl-voltrus-tools.voltrus.id".to_owned(),
            buy_url: "https://console.voltrus.id/app/catalog".to_owned(),
            price: None,
        })
    }

    fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<StoreJson>(json).map(|store| store.license)
    }
}

/// Why a key wasn't activated. The message is meant for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateError {
    pub message: String,
    /// The key is active on another Mac: offer "Move licence here".
    pub seat_limit: bool,
}

impl ActivateError {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            seat_limit: false,
        }
    }
}

impl std::fmt::Display for ActivateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ActivateError {}

/// The Keychain service of the key items (the app's bundle id).
#[cfg_attr(not(feature = "license"), allow(dead_code))]
const SERVICE: &str = "com.joyodigitama.voltrus-tools";

/// Licence keys for the app: blocking, so call it off the UI thread.
pub struct Keys {
    #[cfg(feature = "license")]
    runtime: tokio::runtime::Runtime,
    #[cfg(feature = "license")]
    client: client::Licence,
}

impl Keys {
    /// The app's keys, kept in the Keychain on macOS and under `data_dir`
    /// elsewhere. `None` in builds that don't sell keys, or when no HTTP
    /// runtime could start.
    pub fn open(config: &LicenseConfig, data_dir: PathBuf) -> Option<Self> {
        #[cfg(feature = "license")]
        {
            let secrets = client::platform_secrets(SERVICE, data_dir);
            Self::with(&config.server, env!("CARGO_PKG_VERSION"), secrets)
        }
        #[cfg(not(feature = "license"))]
        {
            let _ = (config, data_dir);
            None
        }
    }

    #[cfg(feature = "license")]
    fn with(server: &str, version: &str, secrets: Box<dyn client::Secrets>) -> Option<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("licence-key")
            .enable_all()
            .build()
            .ok()?;
        let client = client::Licence::new(server, version, secrets)?;
        Some(Self { runtime, client })
    }

    /// Whether a key is saved on this Mac, without asking the server: the
    /// unlock at launch, offline included.
    pub fn saved(&self) -> bool {
        #[cfg(feature = "license")]
        {
            self.client.saved()
        }
        #[cfg(not(feature = "license"))]
        {
            false
        }
    }

    /// Whether the key unlocks the app. The first call of a process validates
    /// the saved key with the server; later calls answer from memory.
    pub fn status(&self) -> bool {
        #[cfg(feature = "license")]
        {
            self.runtime.block_on(self.client.status())
        }
        #[cfg(not(feature = "license"))]
        {
            false
        }
    }

    /// Activate `key` on this Mac and save it. With `relocate` the licence
    /// moves here from the Mac that holds it.
    pub fn activate(&self, key: &str, relocate: bool) -> Result<(), ActivateError> {
        #[cfg(feature = "license")]
        {
            self.runtime.block_on(self.client.activate(key, relocate))
        }
        #[cfg(not(feature = "license"))]
        {
            let _ = (key, relocate);
            Err(ActivateError::plain(
                "This build of Voltrus Tools sells no licence keys.",
            ))
        }
    }

    /// Release this Mac's seat (best effort) and forget the key.
    pub fn remove(&self) -> Result<(), String> {
        #[cfg(feature = "license")]
        {
            self.runtime.block_on(self.client.remove())
        }
        #[cfg(not(feature = "license"))]
        {
            Ok(())
        }
    }
}

#[cfg(feature = "license")]
mod client {
    use std::path::PathBuf;
    use std::time::Duration;

    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use super::ActivateError;

    pub(super) const KEY_ITEM: &str = "licence";
    pub(super) const DEVICE_ITEM: &str = "device-id";
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

    // ── Secret storage ───────────────────────────────────────────────────

    /// Where the key and the device id are kept.
    pub trait Secrets: Send + Sync {
        fn get(&self, item: &str) -> Result<Option<String>, String>;
        fn set(&self, item: &str, value: &str) -> Result<(), String>;
        fn delete(&self, item: &str) -> Result<(), String>;
    }

    /// One file per item in a private folder, where there's no Keychain.
    /// macOS uses the Keychain, so there this is only used by the tests.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub struct FileSecrets {
        dir: PathBuf,
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    impl FileSecrets {
        pub fn new(dir: impl Into<PathBuf>) -> Self {
            Self { dir: dir.into() }
        }
    }

    impl Secrets for FileSecrets {
        fn get(&self, item: &str) -> Result<Option<String>, String> {
            match std::fs::read_to_string(self.dir.join(item)) {
                Ok(value) => Ok(Some(value)),
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
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .map_err(fail)?;
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
        use security_framework::passwords::{
            delete_generic_password, get_generic_password, set_generic_password,
        };

        use super::Secrets;

        /// `errSecItemNotFound`
        const NOT_FOUND: i32 = -25300;

        /// Generic-password items in the app's own Keychain.
        pub struct KeychainSecrets {
            service: String,
        }

        impl KeychainSecrets {
            pub fn new(service: impl Into<String>) -> Self {
                Self {
                    service: service.into(),
                }
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

    /// The Keychain on macOS; files in `data_dir/licence-key` elsewhere.
    pub fn platform_secrets(service: &str, data_dir: PathBuf) -> Box<dyn Secrets> {
        #[cfg(target_os = "macos")]
        {
            let _ = data_dir;
            Box::new(keychain::KeychainSecrets::new(service))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = service;
            Box::new(FileSecrets::new(data_dir.join("licence-key")))
        }
    }

    // ── Client ───────────────────────────────────────────────────────────

    /// What is saved after a successful activation.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Saved {
        key: String,
        activation_id: String,
    }

    #[derive(Default)]
    struct State {
        /// The saved key has been validated (or found absent) in this
        /// process.
        checked: bool,
        saved: Option<Saved>,
    }

    /// The worker's answer to activate, validate and move.
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
        Invalid {
            reason: String,
            seat_limit: bool,
        },
        /// No verdict: network error, 5xx, or a body that isn't the
        /// worker's JSON.
        Unreachable,
    }

    pub struct Licence {
        server: String,
        app_version: String,
        secrets: Box<dyn Secrets>,
        http: reqwest::Client,
        state: Mutex<State>,
    }

    impl Licence {
        pub fn new(server: &str, app_version: &str, secrets: Box<dyn Secrets>) -> Option<Self> {
            let http = reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .ok()?;
            Some(Self {
                server: server.trim_end_matches('/').to_owned(),
                app_version: app_version.to_owned(),
                secrets,
                http,
                state: Mutex::new(State::default()),
            })
        }

        /// A key is saved, valid or not; no network.
        pub fn saved(&self) -> bool {
            matches!(self.load(), Ok(Some(_)))
        }

        /// Whether the key unlocks the app. The first call of a process
        /// validates the saved key with the server.
        pub async fn status(&self) -> bool {
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
                    // Keychain unavailable: the saved state is unknown, so
                    // nothing changes; asked again next time.
                    Err(e) => {
                        eprintln!("licence: {e}");
                        return self.saved();
                    }
                }
            }
            state.saved.is_some()
        }

        pub async fn activate(&self, key: &str, relocate: bool) -> Result<(), ActivateError> {
            let key = key.trim();
            if key.is_empty() {
                return Err(ActivateError::plain("Enter your licence key."));
            }
            let mut state = self.state.lock().await;
            let device_id = self.device_id().map_err(ActivateError::plain)?;
            let body = json!({
                "key": key,
                "device_id": device_id,
                "app_version": self.app_version,
                "platform": platform(),
            });
            let path = if relocate {
                "/license/move"
            } else {
                "/license/activate"
            };
            match self.ask(path, &body).await {
                Answer::Valid(Some(activation_id)) => {
                    let saved = Saved {
                        key: key.to_owned(),
                        activation_id,
                    };
                    self.store(&saved).map_err(ActivateError::plain)?;
                    state.saved = Some(saved);
                    state.checked = true;
                    Ok(())
                }
                Answer::Valid(None) => Err(ActivateError::plain(
                    "The licence server sent an incomplete answer. Try again.",
                )),
                Answer::Invalid { reason, seat_limit } => Err(ActivateError {
                    message: reason,
                    seat_limit,
                }),
                Answer::Unreachable => Err(ActivateError::plain(
                    "Could not reach the licence server. Check your internet connection.",
                )),
            }
        }

        /// Release this device's activation (best effort: the key is
        /// forgotten here even when the server can't be asked).
        pub async fn remove(&self) -> Result<(), String> {
            let mut state = self.state.lock().await;
            let saved = match state.saved.take() {
                Some(saved) => Some(saved),
                None => self.load().ok().flatten(),
            };
            if let Some(saved) = saved {
                let body = json!({
                    "key": saved.key,
                    "activation_id": saved.activation_id,
                    "device_id": self.device_id().unwrap_or_default(),
                });
                self.ask("/license/deactivate", &body).await;
            }
            self.secrets.delete(KEY_ITEM)?;
            state.checked = true;
            Ok(())
        }

        /// The saved key while the server accepts it or can't be asked;
        /// `None` once refused.
        async fn revalidate(&self, saved: Saved) -> Option<Saved> {
            let body = json!({
                "key": saved.key,
                "activation_id": saved.activation_id,
                "device_id": self.device_id().unwrap_or_default(),
                "app_version": self.app_version,
                "platform": platform(),
            });
            match self.ask("/license/validate", &body).await {
                Answer::Valid(_) | Answer::Unreachable => Some(saved),
                Answer::Invalid { .. } => {
                    if let Err(e) = self.secrets.delete(KEY_ITEM) {
                        eprintln!("licence: {e}");
                    }
                    None
                }
            }
        }

        async fn ask(&self, path: &str, body: &Value) -> Answer {
            let url = format!("{}{path}", self.server);
            let Ok(res) = self.http.post(url).json(body).send().await else {
                return Answer::Unreachable;
            };
            // A 5xx is the server's trouble, not a verdict on the key.
            if res.status().is_server_error() {
                return Answer::Unreachable;
            }
            match res.json::<Reply>().await {
                Ok(Reply {
                    valid: true,
                    activation_id,
                    ..
                }) => Answer::Valid(activation_id),
                // The seat-limit text names the Mac holding the licence and
                // says how to move it: keep it whole.
                Ok(Reply {
                    error: Some(error),
                    code: Some(code),
                    ..
                }) if code == "seat_limit" => Answer::Invalid {
                    reason: error,
                    seat_limit: true,
                },
                Ok(Reply { error, .. }) => Answer::Invalid {
                    reason: friendly(error.as_deref().unwrap_or("")),
                    seat_limit: false,
                },
                Err(_) => Answer::Unreachable,
            }
        }

        fn load(&self) -> Result<Option<Saved>, String> {
            Ok(self
                .secrets
                .get(KEY_ITEM)?
                .and_then(|raw| serde_json::from_str(&raw).ok()))
        }

        fn store(&self, saved: &Saved) -> Result<(), String> {
            let raw = serde_json::to_string(saved).map_err(|e| e.to_string())?;
            self.secrets.set(KEY_ITEM, &raw)
        }

        /// A stable per-device id. A new id would take another activation
        /// seat, so it is made once and kept when the key is removed.
        fn device_id(&self) -> Result<String, String> {
            if let Some(id) = self
                .secrets
                .get(DEVICE_ITEM)?
                .filter(|id| !id.trim().is_empty())
            {
                return Ok(id);
            }
            let id = uuid::Uuid::new_v4().to_string();
            self.secrets.set(DEVICE_ITEM, &id)?;
            Ok(id)
        }
    }

    /// `macos-aarch64`: the platform names the download workers use.
    pub(super) fn platform() -> String {
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
    }

    /// Polar errors arrive as raw JSON text
    /// (`{"error":"ResourceNotFound","detail":"..."}`); show the readable
    /// part.
    pub(super) fn friendly(raw: &str) -> String {
        let text = serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|v| {
                ["detail", "message", "error"]
                    .iter()
                    .find_map(|k| v.get(*k)?.as_str().map(str::to_owned))
            })
            .unwrap_or_else(|| raw.trim().to_owned());
        match text.as_str() {
            "" | "ResourceNotFound" | "Not found" => "That licence key was not recognised.".into(),
            _ => text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_config_names_the_worker_and_the_price() {
        let config = LicenseConfig::bundled();
        assert_eq!(config.server, "https://dl-voltrus-tools.voltrus.id");
        assert!(config.buy_url.starts_with("https://console.voltrus.id/"));
        assert_eq!(config.price.as_deref(), Some("$49"));
        assert_eq!(
            LicenseConfig::parse(include_str!("../store.json")).ok(),
            Some(config)
        );
    }

    #[test]
    #[cfg(not(feature = "license"))]
    fn builds_without_the_feature_have_no_keys() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Keys::open(&LicenseConfig::bundled(), dir.path().to_owned()).is_none());
    }
}

#[cfg(test)]
#[cfg(feature = "license")]
mod client_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::client::{DEVICE_ITEM, FileSecrets, KEY_ITEM, Licence, Secrets, friendly, platform};
    use super::*;

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

    type Handler = dyn Fn(&Request) -> (u16, String) + Send + Sync;

    /// A tiny HTTP/1.1 server, so the tests exercise the real reqwest path.
    struct MockServer {
        base: String,
        requests: Arc<std::sync::Mutex<Vec<Request>>>,
    }

    impl MockServer {
        async fn start(
            handler: impl Fn(&Request) -> (u16, String) + Send + Sync + 'static,
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let handler: Arc<Handler> = Arc::new(handler);
            let log = requests.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut sock, _)) = listener.accept().await else {
                        return;
                    };
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
                                let head =
                                    String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                                let len = head
                                    .lines()
                                    .find_map(|l| {
                                        l.strip_prefix("content-length:")
                                            .map(|v| v.trim().parse().unwrap_or(0))
                                    })
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
                        let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                        let body = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
                        let request = Request { path, body };
                        let (status, reply) = handler(&request);
                        log.lock().unwrap().push(request);
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
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.path == path)
                .count()
        }

        fn last(&self) -> (String, Value) {
            let requests = self.requests.lock().unwrap();
            let last = requests.last().unwrap();
            (last.path.clone(), last.body.clone())
        }
    }

    fn ok_activation(_: &Request) -> (u16, String) {
        (200, r#"{"valid":true,"activation_id":"act-1"}"#.into())
    }

    /// Nothing listens on port 1, so connects are refused.
    const DEAD: &str = "http://127.0.0.1:1";

    fn client(server: &str, secrets: &MemorySecrets) -> Licence {
        Licence::new(server, "1.0.1", Box::new(secrets.clone())).unwrap()
    }

    /// A Mac that activated a key at an earlier launch.
    fn with_saved_key(secrets: &MemorySecrets) {
        secrets
            .set(
                KEY_ITEM,
                r#"{"key":"VLT-AAAAA-BBBBB-CCCCC","activation_id":"act-1"}"#,
            )
            .unwrap();
        secrets.set(DEVICE_ITEM, "device-1").unwrap();
    }

    #[tokio::test]
    async fn activation_unlocks_and_saves_the_key() {
        let server = MockServer::start(ok_activation).await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);

        assert!(!licence.status().await);
        licence
            .activate("  VLT-AAAAA-BBBBB-CCCCC ", false)
            .await
            .unwrap();
        assert!(licence.status().await);

        let (path, body) = server.last();
        assert_eq!(path, "/license/activate");
        assert_eq!(body["key"], "VLT-AAAAA-BBBBB-CCCCC", "the key is trimmed");
        assert_eq!(body["app_version"], "1.0.1");
        assert_eq!(body["platform"], platform());
        assert!(!body["device_id"].as_str().unwrap().is_empty());

        // The next launch finds the key and validates it once.
        let relaunched = client(&server.base, &secrets);
        assert!(relaunched.saved());
        assert!(relaunched.status().await);
        assert!(relaunched.status().await);
        assert_eq!(server.count("/license/validate"), 1);
        let (path, body) = server.last();
        assert_eq!(path, "/license/validate");
        assert_eq!(body["activation_id"], "act-1");
    }

    #[tokio::test]
    async fn the_device_id_is_made_once_and_kept() {
        let server = MockServer::start(ok_activation).await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);
        licence.activate("K1", false).await.unwrap();
        let first = server.last().1["device_id"].clone();
        licence.remove().await.unwrap();
        licence.activate("K2", false).await.unwrap();
        assert_eq!(server.last().1["device_id"], first);
    }

    #[tokio::test]
    async fn a_refused_key_is_reported_and_nothing_is_saved() {
        let server = MockServer::start(|_| {
            (
                200,
                r#"{"valid":false,"error":"{\"error\":\"NotPermitted\",\"detail\":\"License key reached its activation limit\"}"}"#.into(),
            )
        })
        .await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);
        let error = licence.activate("K", false).await.unwrap_err();
        assert_eq!(error.message, "License key reached its activation limit");
        assert!(!error.seat_limit);
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
        assert!(!licence.status().await);
    }

    #[tokio::test]
    async fn an_unknown_key_gets_a_plain_message() {
        let server =
            MockServer::start(|_| (200, r#"{"valid":false,"error":"ResourceNotFound"}"#.into()))
                .await;
        let licence = client(&server.base, &MemorySecrets::default());
        assert_eq!(
            licence.activate("nope", false).await.unwrap_err().message,
            "That licence key was not recognised."
        );
    }

    #[tokio::test]
    async fn seat_limit_then_move_licence_here() {
        let text =
            "This licence is active on another Mac. Move licence here to use it on this one.";
        let server = MockServer::start(move |r| match r.path.as_str() {
            "/license/move" => ok_activation(r),
            _ => (
                200,
                format!(r#"{{"valid":false,"code":"seat_limit","error":"{text}"}}"#),
            ),
        })
        .await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);

        let error = licence.activate("K", false).await.unwrap_err();
        assert_eq!(error.message, text, "the seat-limit text is kept whole");
        assert!(error.seat_limit);
        assert_eq!(server.count("/license/move"), 0);

        licence.activate("K", true).await.unwrap();
        assert!(licence.status().await);
        assert_eq!(server.count("/license/move"), 1);
        let (_, body) = server.last();
        assert_eq!(body["key"], "K");
        assert!(body["device_id"].is_string());
    }

    #[tokio::test]
    async fn remove_releases_the_seat_and_forgets_the_key_even_offline() {
        let server = MockServer::start(|r| match r.path.as_str() {
            "/license/activate" => ok_activation(r),
            _ => (200, r#"{"ok":true,"valid":false}"#.into()),
        })
        .await;
        let secrets = MemorySecrets::default();
        let licence = client(&server.base, &secrets);
        licence.activate("K", false).await.unwrap();
        licence.remove().await.unwrap();
        assert!(!licence.status().await);
        assert_eq!(server.count("/license/deactivate"), 1);
        let (_, body) = server.last();
        assert_eq!(body["activation_id"], "act-1");

        client(&server.base, &secrets)
            .activate("K", false)
            .await
            .unwrap();
        let offline = client(DEAD, &secrets);
        offline.remove().await.unwrap();
        assert!(!offline.status().await);
        assert!(!client(&server.base, &secrets).saved());
    }

    #[tokio::test]
    async fn activation_needs_a_reachable_server_and_a_key() {
        let secrets = MemorySecrets::default();
        let licence = client(DEAD, &secrets);
        assert_eq!(
            licence.activate("K", false).await.unwrap_err().message,
            "Could not reach the licence server. Check your internet connection."
        );
        assert_eq!(
            licence.activate("   ", false).await.unwrap_err().message,
            "Enter your licence key."
        );
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());

        let broken = MockServer::start(|_| (503, "down".into())).await;
        let error = client(&broken.base, &secrets)
            .activate("K", false)
            .await
            .unwrap_err();
        assert!(error.message.starts_with("Could not reach"));
    }

    #[tokio::test]
    async fn a_valid_answer_without_an_activation_id_is_not_trusted() {
        let server = MockServer::start(|_| (200, r#"{"valid":true}"#.into())).await;
        let secrets = MemorySecrets::default();
        assert!(
            client(&server.base, &secrets)
                .activate("K", false)
                .await
                .is_err()
        );
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
    }

    #[tokio::test]
    async fn launch_validation_keeps_a_valid_key() {
        let server = MockServer::start(|_| (200, r#"{"valid":true}"#.into())).await;
        let secrets = MemorySecrets::default();
        with_saved_key(&secrets);
        let licence = client(&server.base, &secrets);
        assert!(licence.status().await);
        let (path, body) = server.last();
        assert_eq!(path, "/license/validate");
        assert_eq!(body["key"], "VLT-AAAAA-BBBBB-CCCCC");
        assert_eq!(body["activation_id"], "act-1");
        assert_eq!(body["device_id"], "device-1");
        assert_eq!(body["app_version"], "1.0.1");
    }

    #[tokio::test]
    async fn an_explicit_invalid_removes_the_key() {
        let server = MockServer::start(|_| {
            (
                200,
                r#"{"valid":false,"error":"License key is revoked"}"#.into(),
            )
        })
        .await;
        let secrets = MemorySecrets::default();
        with_saved_key(&secrets);
        let licence = client(&server.base, &secrets);
        assert!(licence.saved());
        assert!(!licence.status().await);
        assert!(secrets.get(KEY_ITEM).unwrap().is_none());
        assert!(!client(&server.base, &secrets).status().await);
    }

    #[tokio::test]
    async fn a_network_failure_keeps_the_unlock() {
        let secrets = MemorySecrets::default();
        with_saved_key(&secrets);
        assert!(client(DEAD, &secrets).status().await);
        assert!(
            secrets.get(KEY_ITEM).unwrap().is_some(),
            "kept for the next launch"
        );
    }

    #[tokio::test]
    async fn server_errors_and_garbage_keep_the_unlock() {
        for (status, body) in [
            (
                500,
                r#"{"valid":false,"error":"License server not configured"}"#,
            ),
            (502, "bad gateway"),
            (404, "Not found"),
            (200, "<html>"),
        ] {
            let server = MockServer::start(move |_| (status, body.into())).await;
            let secrets = MemorySecrets::default();
            with_saved_key(&secrets);
            assert!(
                client(&server.base, &secrets).status().await,
                "{status} {body}"
            );
            assert!(secrets.get(KEY_ITEM).unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn without_a_key_the_server_is_never_asked() {
        let server = MockServer::start(ok_activation).await;
        let licence = client(&server.base, &MemorySecrets::default());
        assert!(!licence.status().await);
        assert!(server.requests.lock().unwrap().is_empty());
    }

    /// The blocking front the app uses, from a plain thread, against a
    /// server on another runtime.
    #[test]
    fn keys_block_on_their_own_runtime() {
        let server_runtime = tokio::runtime::Runtime::new().unwrap();
        let server = server_runtime.block_on(MockServer::start(|r| match r.path.as_str() {
            "/license/validate" => (200, r#"{"valid":false}"#.into()),
            _ => ok_activation(r),
        }));
        let secrets = MemorySecrets::default();
        let keys = Keys::with(&server.base, "1.0.1", Box::new(secrets.clone())).unwrap();
        assert!(!keys.saved());
        assert!(!keys.status());
        keys.activate("K", false).unwrap();
        assert!(keys.saved() && keys.status());

        // A new launch: the key is saved, and an explicit refusal removes it.
        let keys = Keys::with(&server.base, "1.0.1", Box::new(secrets.clone())).unwrap();
        assert!(keys.saved());
        assert!(!keys.status());
        assert!(!keys.saved());
        keys.remove().unwrap();
    }

    #[test]
    fn polar_errors_are_unwrapped() {
        assert_eq!(
            friendly(r#"{"detail":"Too many activations"}"#),
            "Too many activations"
        );
        assert_eq!(friendly(r#"{"error":"Expired"}"#), "Expired");
        assert_eq!(friendly("License is expired"), "License is expired");
        assert_eq!(friendly(""), "That licence key was not recognised.");
    }

    #[test]
    fn file_secrets_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = FileSecrets::new(dir.path().join("licence-key"));
        assert_eq!(secrets.get("a").unwrap(), None);
        secrets.set("a", "1").unwrap();
        assert_eq!(secrets.get("a").unwrap().as_deref(), Some("1"));
        secrets.delete("a").unwrap();
        secrets.delete("a").unwrap();
        assert_eq!(secrets.get("a").unwrap(), None);
    }
}
