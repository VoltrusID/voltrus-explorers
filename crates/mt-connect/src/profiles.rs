//! Saved site profiles on the server: list, save with optimistic concurrency, delete.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ConnectError;
use crate::session::Session;

/// A profile as the server stores it. `data` is opaque to Connect: each app decides what a profile holds.
/// Tombstones (`deleted: true`) carry no `name` or `data`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub data: Value,
    /// Bumped by the server on every accepted save; a save must name the version it started from.
    pub version: i64,
    /// Global change counter, the sync cursor.
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub deleted: bool,
    /// False for a team member looking at someone else's profile.
    #[serde(default = "yes")]
    pub can_edit: bool,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListPage {
    #[serde(default)]
    pub items: Vec<Profile>,
    #[serde(default)]
    pub cursor: Option<i64>,
}

/// What a save wants to become, and the version it started from.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDraft {
    pub id: String,
    pub name: String,
    pub team_id: Option<String>,
    pub data: Value,
    /// Version the edit started from; 0 for a profile the server has never seen.
    pub base_version: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SaveOutcome {
    Saved { profile: Profile },
    /// Someone else saved first. `server` is their version when the server let us read it.
    Conflict { server: Option<Profile> },
}

/// A random version 4 UUID for a new profile id.
pub fn new_uuid() -> Result<String, ConnectError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| ConnectError::Internal(format!("no system randomness: {e}")))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `since: None` lists everything visible (no tombstones); `Some(cursor)` lists changes after it, with tombstones.
pub async fn list(session: &Session, app: &str, since: Option<i64>) -> Result<ListPage, ConnectError> {
    let mut path = format!("/api/connect/profiles?app={}", enc(app));
    if let Some(s) = since {
        path.push_str(&format!("&since={s}"));
    }
    session.request(Method::GET, &path, None).await?.json()
}

pub async fn save(session: &Session, app: &str, draft: &ProfileDraft) -> Result<SaveOutcome, ConnectError> {
    let body = json!({
        "app": app,
        "name": draft.name,
        "team_id": draft.team_id,
        "data": draft.data,
        "base_version": draft.base_version,
    });
    let res = session.request(Method::PUT, &format!("/api/connect/profiles/{}", enc(&draft.id)), Some(&body)).await?;
    if res.status == 409 {
        let parsed: Option<Value> = serde_json::from_str(&res.body).ok();
        let code = parsed.as_ref().and_then(|v| v.get("error")).and_then(Value::as_str);
        // `deleted`: the id is a tombstone, which the caller handles by saving under a new id.
        if code == Some("version_conflict") {
            let current = parsed.and_then(|v| v.get("current").cloned()).and_then(|c| serde_json::from_value(c).ok());
            return Ok(SaveOutcome::Conflict { server: current });
        }
    }
    Ok(SaveOutcome::Saved { profile: res.json()? })
}

pub async fn delete(session: &Session, id: &str) -> Result<(), ConnectError> {
    let res = session.request(Method::DELETE, &format!("/api/connect/profiles/{}", enc(id)), None).await?;
    // Already gone counts as deleted.
    if res.ok() || res.status == 404 { Ok(()) } else { Err(res.error()) }
}
