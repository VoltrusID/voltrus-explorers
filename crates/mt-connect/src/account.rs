//! Account, teams, Remote sites and App Store transaction linking.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ConnectError;
use crate::model::{Entitlement, Me};
use crate::session::Session;

pub async fn me(session: &Session) -> Result<Me, ConnectError> {
    session.request(Method::GET, "/api/connect/me", None).await?.json()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDetail {
    pub id: String,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub invites: Vec<Invite>,
}

#[derive(Deserialize)]
struct Items<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub async fn teams(session: &Session) -> Result<Vec<TeamDetail>, ConnectError> {
    Ok(session.request(Method::GET, "/api/connect/teams", None).await?.json::<Items<TeamDetail>>()?.items)
}

pub async fn create_team(session: &Session, name: &str) -> Result<TeamDetail, ConnectError> {
    session.request(Method::POST, "/api/connect/teams", Some(&json!({ "name": name }))).await?.json()
}

pub async fn invite(session: &Session, team_id: &str, email: &str) -> Result<(), ConnectError> {
    let res = session
        .request(Method::POST, &format!("/api/connect/teams/{}/invites", enc(team_id)), Some(&json!({ "email": email })))
        .await?;
    if res.ok() { Ok(()) } else { Err(res.error()) }
}

/// Accepts an invite token from the emailed link. Returns the team name.
pub async fn accept_invite(session: &Session, token: &str) -> Result<String, ConnectError> {
    #[derive(Deserialize)]
    struct Accepted {
        name: String,
    }
    Ok(session.request(Method::POST, "/api/connect/invites/accept", Some(&json!({ "token": token }))).await?.json::<Accepted>()?.name)
}

/// Removes `user_id` from the team; a member removing themselves is leaving.
pub async fn remove_member(session: &Session, team_id: &str, user_id: &str) -> Result<(), ConnectError> {
    let res = session
        .request(Method::DELETE, &format!("/api/connect/teams/{}/members/{}", enc(team_id), enc(user_id)), None)
        .await?;
    if res.ok() { Ok(()) } else { Err(res.error()) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSite {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub team_id: Option<String>,
    pub login_server: String,
    #[serde(default)]
    pub dashboard_url: Option<String>,
    #[serde(default)]
    pub subnets: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn remote_sites(session: &Session) -> Result<Vec<RemoteSite>, ConnectError> {
    Ok(session.request(Method::GET, "/api/connect/remote/sites", None).await?.json::<Items<RemoteSite>>()?.items)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteJoin {
    pub site_id: String,
    pub name: String,
    pub login_server: String,
    #[serde(default)]
    pub needs_enrolment_key: bool,
    pub tailscale_command: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub limits: Vec<String>,
}

pub async fn remote_join(session: &Session, site_id: &str) -> Result<RemoteJoin, ConnectError> {
    session.request(Method::GET, &format!("/api/connect/remote/sites/{}/join", enc(site_id)), None).await?.json()
}

#[cfg_attr(not(feature = "store"), allow(dead_code))]
/// Links a StoreKit 2 signed transaction to the signed-in account; returns the resulting entitlement.
pub async fn link_transaction(session: &Session, signed_transaction: &str) -> Result<Entitlement, ConnectError> {
    #[derive(Deserialize)]
    struct Linked {
        entitlement: Entitlement,
    }
    let body: Value = json!({ "signed_transaction": signed_transaction });
    Ok(session.request(Method::POST, "/api/connect/app-store/transactions", Some(&body)).await?.json::<Linked>()?.entitlement)
}
