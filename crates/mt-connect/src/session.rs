//! Signed-in state: PKCE sign-in, token storage, refresh and authorized requests.

use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use url::Url;

use crate::api::{Api, RawResponse, TokenResponse};
use crate::error::ConnectError;
use crate::pkce::{PendingLogin, is_callback};
use crate::vault::Vault;

/// Tokens are refreshed this long before they expire.
pub const EXPIRY_SKEW_SECS: u64 = 60;

const TOKENS_KEY: &str = "tokens";
const PENDING_KEY: &str = "pending";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
}

impl TokenSet {
    pub fn usable_at(&self, now: u64) -> bool {
        now + EXPIRY_SKEW_SECS < self.expires_at
    }

    /// Builds the stored set from a token response. Servers that rotate refresh tokens send a new one;
    /// otherwise the previous one stays valid.
    pub fn from_response(res: TokenResponse, previous_refresh: Option<&str>, now: u64) -> Result<Self, ConnectError> {
        let refresh_token = res
            .refresh_token
            .or_else(|| previous_refresh.map(str::to_string))
            .ok_or_else(|| ConnectError::Internal("Voltrus sent no refresh token.".into()))?;
        Ok(Self { access_token: res.access_token, refresh_token, expires_at: now + res.expires_in })
    }
}

pub fn system_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Default)]
struct Cache {
    loaded: bool,
    tokens: Option<TokenSet>,
}

pub struct Session {
    api: Api,
    vault: Box<dyn Vault>,
    client_id: String,
    redirect_uri: String,
    device_name: String,
    /// Doubles as the single-flight lock: only one refresh runs, the others wait and reuse its result.
    cache: Mutex<Cache>,
    clock: fn() -> u64,
}

impl Session {
    pub fn new(api: Api, vault: Box<dyn Vault>, client_id: &str, redirect_uri: &str, device_name: &str) -> Self {
        Self::with_clock(api, vault, client_id, redirect_uri, device_name, system_now)
    }

    pub fn with_clock(api: Api, vault: Box<dyn Vault>, client_id: &str, redirect_uri: &str, device_name: &str, clock: fn() -> u64) -> Self {
        Self {
            api,
            vault,
            client_id: client_id.to_string(),
            redirect_uri: redirect_uri.to_string(),
            device_name: device_name.chars().take(80).collect(),
            cache: Mutex::new(Cache::default()),
            clock,
        }
    }

    pub fn api(&self) -> &Api {
        &self.api
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    fn load(&self, cache: &mut Cache) -> Result<(), ConnectError> {
        if !cache.loaded {
            cache.tokens = self.vault.get(TOKENS_KEY)?.and_then(|s| serde_json::from_str(&s).ok());
            cache.loaded = true;
        }
        Ok(())
    }

    fn store(&self, cache: &mut Cache, tokens: Option<TokenSet>) -> Result<(), ConnectError> {
        match &tokens {
            Some(t) => self.vault.set(TOKENS_KEY, &serde_json::to_string(t).map_err(|e| ConnectError::Internal(e.to_string()))?)?,
            None => self.vault.delete(TOKENS_KEY)?,
        }
        cache.tokens = tokens;
        cache.loaded = true;
        Ok(())
    }

    pub async fn is_signed_in(&self) -> Result<bool, ConnectError> {
        let mut cache = self.cache.lock().await;
        self.load(&mut cache)?;
        Ok(cache.tokens.is_some())
    }

    /// Starts a sign-in: remembers the PKCE verifier and returns the URL for the system browser.
    pub fn begin_sign_in(&self) -> Result<Url, ConnectError> {
        let pending = PendingLogin::new(&self.redirect_uri, (self.clock)())?;
        let url = pending.authorize_url(self.api.base(), &self.client_id)?;
        self.vault.set(PENDING_KEY, &serde_json::to_string(&pending).map_err(|e| ConnectError::Internal(e.to_string()))?)?;
        Ok(url)
    }

    /// Whether `url` is this app's sign-in redirect.
    pub fn owns_callback(&self, url: &Url) -> bool {
        is_callback(url, &self.redirect_uri)
    }

    /// Finishes a sign-in from the redirect URL. The pending attempt is consumed either way, so a
    /// replayed or forged redirect can not be tried twice.
    pub async fn complete_sign_in(&self, callback: &Url) -> Result<(), ConnectError> {
        let pending: PendingLogin = self
            .vault
            .get(PENDING_KEY)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .ok_or_else(|| ConnectError::Auth("No sign-in is in progress.".into()))?;
        let now = (self.clock)();
        let code = match pending.code_from_callback(callback, now) {
            Ok(code) => code,
            Err(e) => {
                // A mismatching state may be noise from another app; keep the real attempt alive.
                if !matches!(e, ConnectError::ForeignCallback) {
                    self.vault.delete(PENDING_KEY)?;
                }
                return Err(e);
            }
        };
        self.vault.delete(PENDING_KEY)?;
        let res = self
            .api
            .token(&[
                ("grant_type", "authorization_code"),
                ("client_id", &self.client_id),
                ("code", &code),
                ("code_verifier", &pending.verifier),
                ("redirect_uri", &pending.redirect_uri),
                ("device_name", &self.device_name),
            ])
            .await?;
        let tokens = TokenSet::from_response(res, None, (self.clock)())?;
        let mut cache = self.cache.lock().await;
        self.store(&mut cache, Some(tokens))
    }

    /// Signs out on this device and, best effort, revokes the grant on the server (offline sign-out
    /// still clears the local session).
    pub async fn sign_out(&self) -> Result<(), ConnectError> {
        let mut cache = self.cache.lock().await;
        self.load(&mut cache)?;
        let refresh = cache.tokens.as_ref().map(|t| t.refresh_token.clone());
        self.vault.delete(PENDING_KEY)?;
        self.store(&mut cache, None)?;
        if let Some(rt) = refresh {
            let _ = self.api.revoke(&rt, &self.client_id).await;
        }
        Ok(())
    }

    /// A usable access token. With `rejected`, the token the server just refused: a refresh is forced
    /// unless another task already replaced it.
    pub async fn access_token(&self, rejected: Option<&str>) -> Result<String, ConnectError> {
        let mut cache = self.cache.lock().await;
        self.load(&mut cache)?;
        let current = cache.tokens.clone().ok_or(ConnectError::NotSignedIn)?;
        let now = (self.clock)();
        let stale = rejected.is_some_and(|r| r == current.access_token);
        if current.usable_at(now) && !stale {
            return Ok(current.access_token);
        }
        let res = self
            .api
            .token(&[
                ("grant_type", "refresh_token"),
                ("client_id", &self.client_id),
                ("refresh_token", &current.refresh_token),
            ])
            .await;
        match res {
            Ok(res) => {
                let next = TokenSet::from_response(res, Some(&current.refresh_token), (self.clock)())?;
                let token = next.access_token.clone();
                self.store(&mut cache, Some(next))?;
                Ok(token)
            }
            // Revoked in the console, or reused after rotation: the session is over.
            Err(e) if e.is_grant_rejected() => {
                self.store(&mut cache, None)?;
                Err(ConnectError::NotSignedIn)
            }
            // Offline and the like: keep the tokens, try again later.
            Err(e) => Err(e),
        }
    }

    /// Authorized request; one refresh-and-retry when the server answers 401.
    pub async fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Result<RawResponse, ConnectError> {
        let token = self.access_token(None).await?;
        let first = self.api.send(method.clone(), path, Some(&token), body).await?;
        if first.status != 401 {
            return Ok(first);
        }
        let token = self.access_token(Some(&token)).await?;
        let second = self.api.send(method, path, Some(&token), body).await?;
        if second.status == 401 {
            let mut cache = self.cache.lock().await;
            self.store(&mut cache, None)?;
            return Err(ConnectError::NotSignedIn);
        }
        Ok(second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pkce::challenge_for;
    use crate::testutil::{MockServer, Reply, dead_base, json};
    use crate::vault::MemoryVault;
    use serde_json::json;
    use std::sync::Arc;

    const NOW: u64 = 10_000;
    fn now() -> u64 {
        NOW
    }

    fn session(base: &str) -> Session {
        Session::with_clock(Api::new(base).unwrap(), Box::new(MemoryVault::default()), "modbus-poll", "voltrus-modbus-poll://connect", "test", now)
    }

    fn seed(s: &Session, access: &str, refresh: &str, expires_at: u64) {
        let t = TokenSet { access_token: access.into(), refresh_token: refresh.into(), expires_at };
        s.vault.set(TOKENS_KEY, &serde_json::to_string(&t).unwrap()).unwrap();
    }

    fn stored(s: &Session) -> Option<TokenSet> {
        s.vault.get(TOKENS_KEY).unwrap().and_then(|v| serde_json::from_str(&v).ok())
    }

    fn token_reply(access: &str, refresh: Option<&str>) -> Reply {
        let mut v = json!({ "access_token": access, "token_type": "Bearer", "expires_in": 900 });
        if let Some(r) = refresh {
            v["refresh_token"] = json!(r);
        }
        json(200, v)
    }

    #[test]
    fn expiry_skew_refreshes_a_little_early() {
        let t = TokenSet { access_token: "a".into(), refresh_token: "r".into(), expires_at: 1000 };
        assert!(t.usable_at(900));
        assert!(!t.usable_at(1000 - EXPIRY_SKEW_SECS));
        assert!(!t.usable_at(1001));
    }

    #[tokio::test]
    async fn fresh_token_is_used_without_touching_the_server() {
        let server = MockServer::start(|_| token_reply("never", None)).await;
        let s = session(&server.base);
        seed(&s, "fresh", "r1", NOW + 600);
        assert_eq!(s.access_token(None).await.unwrap(), "fresh");
        assert!(server.requests.lock().is_empty());
    }

    #[tokio::test]
    async fn expired_token_refreshes_and_stores_the_rotated_refresh_token() {
        let server = MockServer::start(|_| token_reply("a2", Some("r2"))).await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW - 1);
        assert_eq!(s.access_token(None).await.unwrap(), "a2");
        let form = server.requests.lock()[0].form();
        assert_eq!(form["grant_type"], "refresh_token");
        assert_eq!(form["refresh_token"], "r1");
        assert_eq!(form["client_id"], "modbus-poll");
        assert_eq!(stored(&s), Some(TokenSet { access_token: "a2".into(), refresh_token: "r2".into(), expires_at: NOW + 900 }));
    }

    #[tokio::test]
    async fn a_refresh_that_sends_no_new_refresh_token_keeps_the_old_one() {
        let server = MockServer::start(|_| token_reply("a2", None)).await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW - 1);
        s.access_token(None).await.unwrap();
        assert_eq!(stored(&s).unwrap().refresh_token, "r1");
    }

    #[tokio::test]
    async fn rejected_refresh_token_ends_the_session() {
        let server =
            MockServer::start(|_| json(400, json!({ "error": "invalid_grant", "error_description": "refresh token reused" }))).await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW - 1);
        assert!(matches!(s.access_token(None).await, Err(ConnectError::NotSignedIn)));
        assert!(stored(&s).is_none(), "tokens are wiped from the vault");
        assert!(!s.is_signed_in().await.unwrap());
    }

    #[tokio::test]
    async fn offline_refresh_keeps_the_session() {
        let s = session(&dead_base().await);
        seed(&s, "a1", "r1", NOW - 1);
        assert!(matches!(s.access_token(None).await, Err(ConnectError::Network(_))));
        assert_eq!(stored(&s).unwrap().refresh_token, "r1");
        assert!(s.is_signed_in().await.unwrap());
    }

    #[tokio::test]
    async fn server_errors_during_refresh_do_not_sign_out() {
        let server = MockServer::start(|_| json(500, json!({ "error": "server_error" }))).await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW - 1);
        assert!(matches!(s.access_token(None).await, Err(ConnectError::Api { status: 500, .. })));
        assert!(stored(&s).is_some());
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_refresh() {
        let server = MockServer::start(|_| token_reply("a2", Some("r2"))).await;
        let s = Arc::new(session(&server.base));
        seed(&s, "a1", "r1", NOW - 1);
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let s = s.clone();
                tokio::spawn(async move { s.access_token(None).await.unwrap() })
            })
            .collect();
        for t in tasks {
            assert_eq!(t.await.unwrap(), "a2");
        }
        assert_eq!(server.count("POST", "/api/connect/token"), 1, "rotating refresh tokens are single use");
    }

    #[tokio::test]
    async fn a_401_refreshes_once_and_retries_the_request() {
        let server = MockServer::start(|r| match (r.path.as_str(), r.authorization.as_deref()) {
            ("/api/connect/token", _) => token_reply("a2", Some("r2")),
            (_, Some("Bearer a2")) => json(200, json!({ "ok": true })),
            _ => json(401, json!({ "error": "invalid_token", "message": "expired" })),
        })
        .await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW + 600);
        let res = s.request(Method::GET, "/api/connect/me", None).await.unwrap();
        assert_eq!(res.status, 200);
        assert_eq!(server.count("GET", "/api/connect/me"), 2);
        assert_eq!(server.count("POST", "/api/connect/token"), 1);
    }

    #[tokio::test]
    async fn a_401_after_refresh_signs_out() {
        let server = MockServer::start(|r| match r.path.as_str() {
            "/api/connect/token" => token_reply("a2", Some("r2")),
            _ => json(401, json!({ "error": "invalid_token", "message": "revoked" })),
        })
        .await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW + 600);
        assert!(matches!(s.request(Method::GET, "/api/connect/me", None).await, Err(ConnectError::NotSignedIn)));
        assert!(stored(&s).is_none());
    }

    #[tokio::test]
    async fn requests_without_a_session_fail_without_network() {
        let server = MockServer::start(|_| json(200, json!({}))).await;
        let s = session(&server.base);
        assert!(matches!(s.request(Method::GET, "/api/connect/me", None).await, Err(ConnectError::NotSignedIn)));
        assert!(server.requests.lock().is_empty());
    }

    fn query(url: &Url, key: &str) -> String {
        url.query_pairs().find(|(k, _)| k == key).unwrap().1.into_owned()
    }

    #[tokio::test]
    async fn sign_in_exchanges_the_code_with_the_pkce_verifier() {
        let server = MockServer::start(|_| token_reply("a1", Some("r1"))).await;
        let s = session(&server.base);
        let url = s.begin_sign_in().unwrap();
        assert_eq!(url.path(), "/connect/authorize");
        let (state, challenge) = (query(&url, "state"), query(&url, "code_challenge"));

        let callback = Url::parse(&format!("voltrus-modbus-poll://connect?code=the-code&state={state}")).unwrap();
        assert!(s.owns_callback(&callback));
        s.complete_sign_in(&callback).await.unwrap();

        let form = server.requests.lock()[0].form();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "the-code");
        assert_eq!(form["redirect_uri"], "voltrus-modbus-poll://connect");
        assert_eq!(form["client_id"], "modbus-poll");
        assert_eq!(challenge_for(&form["code_verifier"]), challenge, "the verifier matches the challenge sent to the browser");
        assert_eq!(stored(&s).unwrap().access_token, "a1");
        assert!(s.vault.get(PENDING_KEY).unwrap().is_none(), "the attempt is consumed");

        // Replaying the same redirect finds nothing pending.
        assert!(matches!(s.complete_sign_in(&callback).await, Err(ConnectError::Auth(_))));
        assert_eq!(server.count("POST", "/api/connect/token"), 1);
    }

    #[tokio::test]
    async fn a_foreign_redirect_leaves_the_real_attempt_alive() {
        let server = MockServer::start(|_| token_reply("a1", Some("r1"))).await;
        let s = session(&server.base);
        let url = s.begin_sign_in().unwrap();
        let forged = Url::parse("voltrus-modbus-poll://connect?code=evil&state=nope").unwrap();
        assert!(matches!(s.complete_sign_in(&forged).await, Err(ConnectError::ForeignCallback)));
        assert!(server.requests.lock().is_empty());

        let real = Url::parse(&format!("voltrus-modbus-poll://connect?code=ok&state={}", query(&url, "state"))).unwrap();
        s.complete_sign_in(&real).await.unwrap();
        assert!(stored(&s).is_some());
    }

    #[tokio::test]
    async fn a_cancelled_sign_in_stores_nothing() {
        let s = session("http://127.0.0.1:1");
        let url = s.begin_sign_in().unwrap();
        let cancelled = Url::parse(&format!("voltrus-modbus-poll://connect?error=access_denied&state={}", query(&url, "state"))).unwrap();
        let err = s.complete_sign_in(&cancelled).await.unwrap_err();
        assert!(err.to_string().contains("cancelled"));
        assert!(stored(&s).is_none());
    }

    #[tokio::test]
    async fn a_rejected_code_surfaces_the_server_message() {
        let server = MockServer::start(|_| {
            json(400, json!({ "error": "invalid_grant", "error_description": "authorization code is invalid, expired or already used" }))
        })
        .await;
        let s = session(&server.base);
        let url = s.begin_sign_in().unwrap();
        let cb = Url::parse(&format!("voltrus-modbus-poll://connect?code=old&state={}", query(&url, "state"))).unwrap();
        let err = s.complete_sign_in(&cb).await.unwrap_err();
        assert!(err.to_string().contains("already used"), "{err}");
        assert!(stored(&s).is_none());
    }

    #[tokio::test]
    async fn sign_out_clears_tokens_and_revokes_the_grant() {
        let server = MockServer::start(|_| json(200, json!({}))).await;
        let s = session(&server.base);
        seed(&s, "a1", "r1", NOW + 600);
        s.sign_out().await.unwrap();
        assert!(stored(&s).is_none());
        let revoke = server.requests.lock().iter().find(|r| r.path == "/api/connect/revoke").cloned().unwrap();
        assert_eq!(revoke.form()["token"], "r1");
    }

    #[tokio::test]
    async fn sign_out_works_offline() {
        let s = session(&dead_base().await);
        seed(&s, "a1", "r1", NOW + 600);
        s.sign_out().await.unwrap();
        assert!(stored(&s).is_none());
    }
}
