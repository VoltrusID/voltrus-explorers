//! Account-level answers from the server.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entitlement {
    pub active: bool,
    /// "app_store" | "console" | "none"
    #[serde(default = "none_source")]
    pub source: String,
    /// Unix milliseconds; `None` for none or non-expiring.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

fn none_source() -> String {
    "none".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    /// "owner" | "member"
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub email_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub user: User,
    pub entitlement: Entitlement,
    #[serde(default)]
    pub teams: Vec<Team>,
    /// Pass to StoreKit as `appAccountToken` so a purchase is bound to this account.
    #[serde(default)]
    pub app_account_token: Option<String>,
}
