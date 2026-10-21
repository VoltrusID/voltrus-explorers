//! HTTP layer: one reqwest client, form-encoded token endpoint, JSON everywhere else.

use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::error::ConnectError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    pub status: u16,
    pub body: String,
}

impl RawResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Deserializes a 2xx body, or turns the server's error body into a [`ConnectError::Api`].
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConnectError> {
        if !self.ok() {
            return Err(self.error());
        }
        serde_json::from_str(&self.body).map_err(|e| ConnectError::Internal(format!("Unexpected answer from Voltrus: {e}")))
    }

    pub fn error(&self) -> ConnectError {
        #[derive(Deserialize)]
        struct Body {
            error: Option<Value>,
            error_description: Option<String>,
            message: Option<String>,
        }
        let parsed: Option<Body> = serde_json::from_str(&self.body).ok();
        let (code, message) = match parsed {
            Some(b) => {
                let (code, text) = match b.error {
                    Some(Value::String(s)) => (Some(s.clone()), Some(s)),
                    Some(Value::Object(o)) => (
                        o.get("code").and_then(Value::as_str).map(str::to_string),
                        o.get("message").and_then(Value::as_str).map(str::to_string),
                    ),
                    _ => (None, None),
                };
                (code, b.error_description.or(b.message).or(text))
            }
            None => (None, None),
        };
        ConnectError::Api {
            status: self.status,
            message: message.unwrap_or_else(|| format!("Voltrus answered with HTTP {}.", self.status)),
            code,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds until the access token expires.
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

fn default_expires_in() -> u64 {
    300
}

pub struct Api {
    http: reqwest::Client,
    base: String,
}

impl Api {
    pub fn new(base: &str) -> Result<Self, ConnectError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("voltrus-connect/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ConnectError::Internal(format!("HTTP client: {e}")))?;
        Ok(Self { http, base: base.trim_end_matches('/').to_string() })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn send(&self, method: Method, path: &str, bearer: Option<&str>, body: Option<&Value>) -> Result<RawResponse, ConnectError> {
        let mut req = self.http.request(method, format!("{}{}", self.base, path)).header("accept", "application/json");
        if let Some(token) = bearer {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let res = req.send().await.map_err(|e| ConnectError::Network(e.without_url().to_string()))?;
        let status = res.status().as_u16();
        let body = res.text().await.map_err(|e| ConnectError::Network(e.without_url().to_string()))?;
        Ok(RawResponse { status, body })
    }

    async fn post_form(&self, path: &str, params: &[(&str, &str)]) -> Result<RawResponse, ConnectError> {
        let form = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish();
        let res = self
            .http
            .post(format!("{}{}", self.base, path))
            .header("accept", "application/json")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|e| ConnectError::Network(e.without_url().to_string()))?;
        let status = res.status().as_u16();
        let body = res.text().await.map_err(|e| ConnectError::Network(e.without_url().to_string()))?;
        Ok(RawResponse { status, body })
    }

    /// `POST /api/connect/token` with an `application/x-www-form-urlencoded` body (RFC 6749).
    pub async fn token(&self, params: &[(&str, &str)]) -> Result<TokenResponse, ConnectError> {
        self.post_form("/api/connect/token", params).await?.json()
    }

    /// `POST /api/connect/revoke` (RFC 7009): the server answers 200 whatever the token was.
    pub async fn revoke(&self, refresh_token: &str, client_id: &str) -> Result<(), ConnectError> {
        let res = self.post_form("/api/connect/revoke", &[("token", refresh_token), ("client_id", client_id)]).await?;
        if res.ok() { Ok(()) } else { Err(res.error()) }
    }
}
