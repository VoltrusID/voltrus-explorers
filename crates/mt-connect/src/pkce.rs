//! OAuth 2.0 authorization code + PKCE (RFC 7636) for a native app.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::ConnectError;

/// Sign-in attempts older than this are dropped (the user never came back from the browser).
pub const PENDING_TTL_SECS: u64 = 10 * 60;

/// What the app remembers between opening the browser and receiving the redirect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingLogin {
    pub state: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub created_at: u64,
}

fn random_b64(bytes: usize) -> Result<String, ConnectError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| ConnectError::Internal(format!("no system randomness: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// 43-character code verifier from 32 random bytes (RFC 7636 section 4.1).
pub fn new_verifier() -> Result<String, ConnectError> {
    random_b64(32)
}

/// `BASE64URL(SHA256(verifier))` (the `S256` method).
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl PendingLogin {
    pub fn new(redirect_uri: &str, now: u64) -> Result<Self, ConnectError> {
        Ok(Self {
            state: random_b64(24)?,
            verifier: new_verifier()?,
            redirect_uri: redirect_uri.to_string(),
            created_at: now,
        })
    }

    pub fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.created_at) > PENDING_TTL_SECS
    }

    /// The URL the system browser opens.
    pub fn authorize_url(&self, api_base: &str, client_id: &str) -> Result<Url, ConnectError> {
        let mut url = Url::parse(&format!("{}/connect/authorize", api_base.trim_end_matches('/')))
            .map_err(|e| ConnectError::Internal(format!("bad API base: {e}")))?;
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("state", &self.state)
            .append_pair("code_challenge", &challenge_for(&self.verifier))
            .append_pair("code_challenge_method", "S256");
        Ok(url)
    }

    /// Validates a redirect back into the app and returns the authorization code.
    pub fn code_from_callback(&self, callback: &Url, now: u64) -> Result<String, ConnectError> {
        if self.expired(now) {
            return Err(ConnectError::Auth("The sign-in took too long. Start again.".into()));
        }
        let mut code = None;
        let mut state = None;
        let mut error = None;
        let mut error_description = None;
        for (k, v) in callback.query_pairs() {
            match k.as_ref() {
                "code" => code = Some(v.into_owned()),
                "state" => state = Some(v.into_owned()),
                "error" => error = Some(v.into_owned()),
                "error_description" => error_description = Some(v.into_owned()),
                _ => {}
            }
        }
        // Check state first: an error carried by a forged redirect must not be shown as the server's.
        if state.as_deref() != Some(self.state.as_str()) {
            return Err(ConnectError::ForeignCallback);
        }
        if let Some(e) = error {
            return Err(ConnectError::Auth(match (e.as_str(), error_description) {
                ("access_denied", _) => "Sign-in was cancelled.".into(),
                (_, Some(d)) => d,
                (e, None) => format!("Sign-in failed ({e})."),
            }));
        }
        code.filter(|c| !c.is_empty()).ok_or_else(|| ConnectError::Auth("The sign-in response had no code.".into()))
    }
}

/// Whether `url` is this app's Connect redirect (`voltrus-<slug>://connect`).
pub fn is_callback(url: &Url, redirect_uri: &str) -> bool {
    Url::parse(redirect_uri).is_ok_and(|r| {
        r.scheme() == url.scheme() && r.host_str().map(str::to_ascii_lowercase) == url.host_str().map(str::to_ascii_lowercase)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636_appendix_b() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_is_unreserved_43_chars_and_unique() {
        let a = new_verifier().unwrap();
        let b = new_verifier().unwrap();
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, b);
    }

    fn pending() -> PendingLogin {
        PendingLogin {
            state: "st4te".into(),
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            redirect_uri: "voltrus-modbus-poll://connect".into(),
            created_at: 1000,
        }
    }

    #[test]
    fn authorize_url_carries_pkce_parameters() {
        let url = pending().authorize_url("https://console.voltrus.id/", "modbus-poll").unwrap();
        assert_eq!(url.path(), "/connect/authorize");
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "modbus-poll");
        assert_eq!(q["redirect_uri"], "voltrus-modbus-poll://connect");
        assert_eq!(q["state"], "st4te");
        assert_eq!(q["code_challenge"], "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(q["code_challenge_method"], "S256");
        assert!(!url.as_str().contains("dBjftJeZ4CVP"), "the verifier never leaves the device");
    }

    #[test]
    fn callback_returns_code_when_state_matches() {
        let url = Url::parse("voltrus-modbus-poll://connect?code=abc123&state=st4te").unwrap();
        assert_eq!(pending().code_from_callback(&url, 1100).unwrap(), "abc123");
    }

    #[test]
    fn callback_rejects_wrong_or_missing_state() {
        let p = pending();
        for u in [
            "voltrus-modbus-poll://connect?code=abc&state=other",
            "voltrus-modbus-poll://connect?code=abc",
            "voltrus-modbus-poll://connect?error=access_denied&state=other",
        ] {
            let err = p.code_from_callback(&Url::parse(u).unwrap(), 1100).unwrap_err();
            assert!(err.to_string().contains("did not match"), "{u}: {err}");
        }
    }

    #[test]
    fn callback_surfaces_server_errors_and_cancel() {
        let p = pending();
        let cancelled = Url::parse("voltrus-modbus-poll://connect?error=access_denied&state=st4te").unwrap();
        assert!(p.code_from_callback(&cancelled, 1100).unwrap_err().to_string().contains("cancelled"));
        let other = Url::parse("voltrus-modbus-poll://connect?error=server_error&error_description=Try+later&state=st4te").unwrap();
        assert!(p.code_from_callback(&other, 1100).unwrap_err().to_string().contains("Try later"));
        let empty = Url::parse("voltrus-modbus-poll://connect?state=st4te").unwrap();
        assert!(p.code_from_callback(&empty, 1100).is_err());
    }

    #[test]
    fn stale_attempts_are_rejected() {
        let url = Url::parse("voltrus-modbus-poll://connect?code=abc&state=st4te").unwrap();
        let err = pending().code_from_callback(&url, 1000 + PENDING_TTL_SECS + 1).unwrap_err();
        assert!(err.to_string().contains("too long"));
    }

    #[test]
    fn callback_detection_matches_scheme_and_host() {
        let redirect = "voltrus-modbus-poll://connect";
        assert!(is_callback(&Url::parse("voltrus-modbus-poll://connect?code=1").unwrap(), redirect));
        assert!(!is_callback(&Url::parse("voltrus-bacnet-explorer://connect?code=1").unwrap(), redirect));
        assert!(!is_callback(&Url::parse("voltrus-modbus-poll://other").unwrap(), redirect));
    }
}
