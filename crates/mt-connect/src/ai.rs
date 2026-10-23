//! The Connect AI assistant (console `/api/connect/ai/*`, contract: voltrus-console docs/connect-ai.md).
//!
//! The app never talks to a model provider and holds no provider key: it sends one task request with its
//! Connect token and gets validated JSON back. The assistant only reads and suggests; nothing here can
//! write to a device. The UI shows every result as a proposal the user confirms.

use std::time::Duration;

use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

use crate::error::ConnectError;
use crate::session::Session;

/// Image requests on the vision model can take half a minute.
const AI_TIMEOUT: Duration = Duration::from_secs(90);

/// UI task name -> console path. Anything else is refused, so a script bug can not reach other endpoints.
const TASKS: [(&str, &str); 5] = [
    ("register_map", "/api/connect/ai/register-map"),
    ("manual_blocks", "/api/connect/ai/manual-blocks"),
    ("diagnose", "/api/connect/ai/diagnose"),
    ("decode", "/api/connect/ai/decode"),
    ("ask", "/api/connect/ai/ask"),
];

/// What the UI needs to explain a failed request: the console's `error` code (`quota_exceeded`,
/// `entitlement_required`, `ai_unavailable`, ...), its message, and the quota when it sent one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AiFailure {
    /// HTTP status; 0 when the server could not be reached.
    pub status: u16,
    pub code: String,
    pub message: String,
    pub quota: Option<Value>,
}

impl AiFailure {
    fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self { status, code: code.into(), message: message.into(), quota: None }
    }

    fn from_error(e: ConnectError) -> Self {
        match e {
            ConnectError::NotSignedIn => Self::new(401, "not_signed_in", e.to_string()),
            ConnectError::Network(_) => Self::new(0, "network", e.to_string()),
            ConnectError::Api { status, code, message } => Self::new(status, code.as_deref().unwrap_or("error"), message),
            other => Self::new(0, "error", other.to_string()),
        }
    }
}

/// `GET /api/connect/ai/availability` (no sign-in): `{ available }`, true when the assistant is open to everyone.
/// Signed-out apps use it to decide whether to mention the Assistant at all.
pub async fn availability(session: &Session) -> Result<Value, AiFailure> {
    let res = session.api().send(Method::GET, "/api/connect/ai/availability", None, None).await.map_err(AiFailure::from_error)?;
    res.json().map_err(AiFailure::from_error)
}

/// `GET /api/connect/ai/status`: `{ enabled: false }` or `{ enabled, entitled, models, quota }`.
pub async fn status(session: &Session) -> Result<Value, AiFailure> {
    let res = session.request(Method::GET, "/api/connect/ai/status", None).await.map_err(AiFailure::from_error)?;
    res.json().map_err(AiFailure::from_error)
}

/// Runs one task with its JSON payload and returns the console's JSON (`result`, `quota`, `usage`, ...).
pub async fn run(session: &Session, task: &str, payload: &Value) -> Result<Value, AiFailure> {
    let Some((_, path)) = TASKS.iter().find(|(name, _)| *name == task) else {
        return Err(AiFailure::new(0, "unknown_task", format!("Unknown assistant task: {task}")));
    };
    let res = session.request_within(Method::POST, path, Some(payload), Some(AI_TIMEOUT)).await.map_err(AiFailure::from_error)?;
    if res.ok() {
        return res.json().map_err(AiFailure::from_error);
    }
    // The error body carries the quota (429) that the generic error type drops.
    let body: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    let code = body.get("error").and_then(Value::as_str).unwrap_or("error");
    let message = body.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| res.error().to_string());
    Err(AiFailure { quota: body.get("quota").cloned(), ..AiFailure::new(res.status, code, message) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use crate::testutil::{MockServer, json};
    use crate::vault::MemoryVault;
    use serde_json::json;

    fn session(base: &str) -> Session {
        let s = Session::with_clock(Api::new(base).unwrap(), Box::new(MemoryVault::default()), "modbus-poll", "voltrus-modbus-poll://connect", "test", || 10_000);
        s.seed_tokens_for_test("a1", "r1", 99_999);
        s
    }

    #[tokio::test]
    async fn status_and_tasks_use_the_bearer_and_the_whitelisted_paths() {
        let server = MockServer::start(|req| match req.path.as_str() {
            "/api/connect/ai/status" => json(200, json!({ "enabled": true, "entitled": true })),
            "/api/connect/ai/diagnose" => json(200, json!({ "task": "diagnose", "result": { "summary": "x" } })),
            _ => json(404, json!({ "error": "not_found" })),
        })
        .await;
        let s = session(&server.base);
        assert_eq!(status(&s).await.unwrap()["enabled"], true);
        let out = run(&s, "diagnose", &json!({ "protocol": "modbus" })).await.unwrap();
        assert_eq!(out["result"]["summary"], "x");
        let reqs = server.requests.lock();
        assert_eq!(reqs[1].authorization.as_deref(), Some("Bearer a1"));
        assert_eq!(reqs[1].json()["protocol"], "modbus");
    }

    #[tokio::test]
    async fn availability_needs_no_sign_in() {
        let server = MockServer::start(|req| {
            if req.authorization.is_some() {
                json(400, json!({ "error": "unexpected_auth" }))
            } else {
                json(200, json!({ "available": false }))
            }
        })
        .await;
        let s = Session::with_clock(Api::new(&server.base).unwrap(), Box::new(MemoryVault::default()), "modbus-poll", "voltrus-modbus-poll://connect", "test", || 1);
        assert_eq!(availability(&s).await.unwrap()["available"], false);
    }

    #[tokio::test]
    async fn an_unknown_task_never_leaves_the_app() {
        let server = MockServer::start(|_| json(200, json!({}))).await;
        let s = session(&server.base);
        let err = run(&s, "../../profiles", &json!({})).await.unwrap_err();
        assert_eq!(err.code, "unknown_task");
        assert!(server.requests.lock().is_empty());
    }

    #[tokio::test]
    async fn a_refusal_keeps_the_code_message_and_quota() {
        let server = MockServer::start(|_| {
            json(429, json!({ "error": "quota_exceeded", "message": "monthly text request limit reached", "quota": { "text": { "used": 300 } } }))
        })
        .await;
        let s = session(&server.base);
        let err = run(&s, "ask", &json!({})).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (429, "quota_exceeded"));
        assert_eq!(err.quota.unwrap()["text"]["used"], 300);
    }

    #[tokio::test]
    async fn signed_out_is_reported_as_such() {
        let server = MockServer::start(|_| json(200, json!({}))).await;
        let s = Session::with_clock(Api::new(&server.base).unwrap(), Box::new(MemoryVault::default()), "modbus-poll", "voltrus-modbus-poll://connect", "test", || 1);
        assert_eq!(status(&s).await.unwrap_err().code, "not_signed_in");
    }
}
