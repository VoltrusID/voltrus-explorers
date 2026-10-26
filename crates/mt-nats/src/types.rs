use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Client configuration
// ---------------------------------------------------------------------------

fn default_nats_port() -> u16 {
    4222
}

fn default_connect_timeout_ms() -> u64 {
    5000
}

/// Authentication method for a NATS client connection.
///
/// Externally tagged (serde `tag = "type"`) so the frontend sends e.g.
/// `{"type":"user_pass","username":"...","password":"..."}` or
/// `{"type":"credentials_file","path":"/path/to.creds"}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NatsAuth {
    /// Anonymous (server permits unauthenticated connections).
    #[default]
    None,
    /// Username + password ("simple" auth).
    UserPass {
        username: String,
        password: String,
    },
    /// Token-only auth.
    Token {
        token: String,
    },
    /// Path to a decentralized-auth `.creds` file (NATS v2+).
    CredentialsFile {
        path: PathBuf,
    },
    /// Inline decentralized-auth `.creds` body.
    Credentials {
        creds: String,
    },
    /// NKey seed (decentralized auth).
    Nkey {
        seed: String,
    },
    /// User JWT + nkey seed (operator/account-scoped decentralized auth).
    /// The seed signs the server's connect nonce via `nkeys`.
    Jwt {
        user_jwt: String,
        nkey_seed: String,
    },
}

/// Configuration for an NATS client connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsClientConfig {
    pub host: String,
    #[serde(default = "default_nats_port")]
    pub port: u16,
    /// Client connection name (shown in server connections).
    #[serde(default)]
    pub name: String,
    /// Preferred auth method. When `None`, the legacy `username`/`password`/
    /// `token` fields below are consulted for backward compatibility.
    #[serde(default)]
    pub auth: NatsAuth,
    /// Legacy username (used only when `auth == None`).
    #[serde(default)]
    pub username: Option<String>,
    /// Legacy password (used only when `auth == None`).
    #[serde(default)]
    pub password: Option<String>,
    /// Legacy token (used only when `auth == None`).
    #[serde(default)]
    pub token: Option<String>,
    /// TLS (NATS over TLS, commonly port 4222 with `tls://`).
    #[serde(default)]
    pub tls: NatsClientTlsConfig,
    /// Connection timeout in milliseconds.
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
}

impl Default for NatsClientConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: default_nats_port(),
            name: "nats-explorer".to_string(),
            auth: NatsAuth::None,
            username: None,
            password: None,
            token: None,
            tls: NatsClientTlsConfig::default(),
            connect_timeout_ms: default_connect_timeout_ms(),
        }
    }
}

/// TLS settings for the NATS client.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NatsClientTlsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// PEM CA bundle. When set, it pins this CA; otherwise the platform roots.
    #[serde(default)]
    pub ca_pem: Option<String>,
    /// Client certificate PEM (mTLS).
    #[serde(default)]
    pub client_cert_pem: Option<String>,
    /// Client private key PEM (mTLS).
    #[serde(default)]
    pub client_key_pem: Option<String>,
}

// ---------------------------------------------------------------------------
// Server configuration
// ---------------------------------------------------------------------------

fn default_max_connections() -> u32 {
    100
}

/// Configuration for the embedded NATS server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_nats_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// When `mode == "password"`, only these users may connect.
    #[serde(default)]
    pub auth: NatsAuthConfig,
}

impl Default for NatsServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_nats_port(),
            max_connections: default_max_connections(),
            auth: NatsAuthConfig::default(),
        }
    }
}

fn default_auth_mode() -> String {
    "none".to_string()
}

/// Authentication configuration for the embedded server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsAuthConfig {
    #[serde(default = "default_auth_mode")]
    pub mode: String,
    #[serde(default)]
    pub users: Vec<NatsUser>,
}

impl Default for NatsAuthConfig {
    fn default() -> Self {
        Self {
            mode: default_auth_mode(),
            users: vec![],
        }
    }
}

/// A single user credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsUser {
    pub username: String,
    pub password: String,
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// A NATS message surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsMessage {
    pub subject: String,
    /// Reply subject (set when the message is a request).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    /// Ordered header key/value pairs. Empty when no headers.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Payload as a UTF-8 string, hex-encoded when not valid UTF-8.
    #[serde(default)]
    pub payload: String,
    /// Milliseconds since the UNIX epoch.
    #[serde(default)]
    pub timestamp_ms: i64,
    /// Queue group name the message was received on (client subscriptions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

impl NatsMessage {
    pub fn payload_bytes(&self) -> Vec<u8> {
        decode_payload(&self.payload)
    }

    pub fn headers_map(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.headers {
            m.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
        m
    }
}

/// A tracked subject (server mode).
#[derive(Debug, Clone, Serialize)]
pub struct SubjectInfo {
    pub subject: String,
    pub message_count: u64,
    pub last_update: i64,
}

/// A single recent message stored in the server tracker.
#[derive(Debug, Clone, Serialize)]
pub struct ServerRecentMessage {
    pub subject: String,
    pub payload: String,
    pub timestamp: i64,
    pub headers: Vec<(String, String)>,
}

/// Runtime status of the embedded NATS server.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Error(String),
}

// ---------------------------------------------------------------------------
// JetStream UI types (mirror async-nats shapes)
// ---------------------------------------------------------------------------

/// A JetStream stream, as shown in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsStreamInfo {
    pub name: String,
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub messages: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub max_messages: i64,
    /// Max age in nanoseconds (-1 = unlimited).
    #[serde(default)]
    pub max_age_ns: i64,
    #[serde(default)]
    pub storage: String,
    #[serde(default)]
    pub consumers: u64,
    #[serde(default)]
    pub created: Option<String>,
    /// Sequence of the oldest stored message (0 when empty).
    #[serde(default)]
    pub first_seq: u64,
    /// Sequence of the newest stored message (0 when empty).
    #[serde(default)]
    pub last_seq: u64,
}

/// A JetStream consumer (durable, pull) as shown in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConsumerInfo {
    pub stream: String,
    pub name: String,
    pub durable: bool,
    /// "pull" | "push"
    pub deliver_kind: String,
    #[serde(default)]
    pub ack_policy: String,
    #[serde(default)]
    pub pending: u64,
    #[serde(default)]
    pub num_ack_pending: u64,
    #[serde(default)]
    pub num_redelivered: u64,
}

/// A single stored message read from a JetStream stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsStoredMessage {
    pub sequence: u64,
    pub subject: String,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub timestamp_ms: i64,
}

/// Summary of a NATS KV bucket (backed by a `KV_<bucket>` JetStream stream).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsKvBucketInfo {
    pub bucket: String,
    #[serde(default)]
    pub values: u64,
    #[serde(default)]
    pub bytes: u64,
    /// Kept-history depth per key (-1 = default/unlimited).
    #[serde(default)]
    pub history: i64,
}

/// A single KV entry (current value or a history revision).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsKvEntry {
    pub bucket: String,
    pub key: String,
    /// [`encode_payload`] of the value bytes.
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub revision: u64,
    /// `"put"` | `"delete"` | `"purge"`.
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub created_ms: i64,
}

/// Metadata for an Object Store object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsObjectInfo {
    pub bucket: String,
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub chunks: u64,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub modified_ms: i64,
    #[serde(default)]
    pub deleted: bool,
}

// ---------------------------------------------------------------------------
// Payload codec helpers
// ---------------------------------------------------------------------------

/// Encode bytes as a UI string: UTF-8 when possible, otherwise hex.
pub fn encode_payload(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => format!("0x{}", hex::encode(data)),
    }
}

/// Decode a UI payload string back to bytes. `0x...` / bare hex is decoded
/// as hex when it is not valid UTF-8 text.
pub fn decode_payload(s: &str) -> Vec<u8> {
    if let Some(hex_part) = s.strip_prefix("0x") {
        if let Ok(bytes) = hex::decode(hex_part) {
            return bytes;
        }
    }
    s.as_bytes().to_vec()
}
