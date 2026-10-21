use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpcUaConfig {
    pub endpoint_url: String,
    #[serde(default)]
    pub security_policy: String,
    #[serde(default)]
    pub security_mode: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_poll_interval_ms() -> u64 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpcUaNode {
    pub node_id: String,
    #[serde(alias = "browse_name")]
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpcUaValue {
    pub node_id: String,
    pub name: String,
    pub value: serde_json::Value,
    pub timestamp: u64,
}

// ── Discovery ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub url: String,
    pub security_policy_uri: String,
    pub security_mode: String,
    pub transport_profile_uri: String,
    pub security_level: u8,
    pub user_token_policies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub application_name: String,
    pub application_uri: String,
    pub application_type: String,
    pub product_uri: String,
    pub discovery_urls: Vec<String>,
}

// ── Browse / References / Namespaces ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseChild {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub node_class: String,
    pub data_type: String,
    pub is_variable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceInfo {
    pub reference_type: String,
    pub is_forward: bool,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub node_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceInfo {
    pub index: u16,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeRead {
    pub attribute: String,
    pub value: serde_json::Value,
    pub status: String,
}

// ── Subscriptions ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionConfig {
    #[serde(default = "default_publishing_interval_ms")]
    pub publishing_interval_ms: u32,
    #[serde(default = "default_lifetime_count")]
    pub lifetime_count: u32,
    #[serde(default = "default_max_keep_alive")]
    pub max_keep_alive_count: u32,
    #[serde(default)]
    pub max_notifications_per_publish: u32,
    #[serde(default)]
    pub priority: u8,
    #[serde(default = "default_true")]
    pub publishing_enabled: bool,
}

fn default_publishing_interval_ms() -> u32 {
    1000
}
fn default_lifetime_count() -> u32 {
    60
}
fn default_max_keep_alive() -> u32 {
    10
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredItemDef {
    pub node_id: String,
    #[serde(default = "default_attribute_value")]
    pub attribute_id: u32,
    #[serde(default = "default_sampling_ms")]
    pub sampling_interval_ms: f64,
    #[serde(default = "default_queue_size")]
    pub queue_size: u32,
    #[serde(default = "default_true")]
    pub discard_oldest: bool,
    #[serde(default = "default_monitoring_mode")]
    pub monitoring_mode: String,
    #[serde(default = "default_trigger")]
    pub trigger: String,
    #[serde(default)]
    pub deadband_type: u32,
    #[serde(default)]
    pub deadband_value: f64,
}

fn default_attribute_value() -> u32 {
    13 // AttributeId::Value
}
fn default_sampling_ms() -> f64 {
    1000.0
}
fn default_queue_size() -> u32 {
    10
}
fn default_monitoring_mode() -> String {
    "Reporting".to_string()
}
fn default_trigger() -> String {
    "Status".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationValue {
    pub node_id: String,
    pub value: serde_json::Value,
    pub source_ts: u64,
    pub server_ts: u64,
    pub status: String,
}

// ── Certificates ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertInfo {
    pub thumbprint: String,
    pub subject: String,
    pub issuer: String,
    pub valid_from: String,
    pub valid_to: String,
    pub bucket: String, // "own" | "trusted" | "rejected"
    pub kind: String,   // "client" | "server"
    pub is_ca: bool,
    pub path: String,
}

/// A data-change notification tagged with the client session it came from. This is the payload of the
/// `opcua://notification` event; the value fields sit next to `session` on the same object.
#[derive(Debug, Clone, Serialize)]
pub struct SessionNotification {
    pub session: String,
    #[serde(flatten)]
    pub value: NotificationValue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_notification_carries_session_next_to_the_value_fields() {
        let event = SessionNotification {
            session: "s2".to_string(),
            value: NotificationValue {
                node_id: "ns=2;s=Reactor.Temperature".to_string(),
                value: serde_json::json!(21.5),
                source_ts: 10,
                server_ts: 11,
                status: "Good".to_string(),
            },
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({
                "session": "s2",
                "node_id": "ns=2;s=Reactor.Temperature",
                "value": 21.5,
                "source_ts": 10,
                "server_ts": 11,
                "status": "Good",
            })
        );
    }
}
