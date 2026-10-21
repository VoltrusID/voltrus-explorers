use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Broker configuration
// ---------------------------------------------------------------------------

fn default_mqtt_port() -> u16 {
    1883
}
fn default_mqtt_max_connections() -> u32 {
    100
}
fn default_mqtt_auth_mode() -> String {
    "none".to_string()
}

/// Configuration for the embedded MQTT broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttBrokerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default = "default_mqtt_max_connections")]
    pub max_connections: u32,
    #[serde(default)]
    pub auth: MqttAuthConfig,
    /// Optional WebSocket listener port (MQTT over WS, for browser clients).
    #[serde(default)]
    pub ws_port: Option<u16>,
    /// Optional TLS listener settings (MQTTS).
    #[serde(default)]
    pub tls: MqttServerTlsConfig,
}

impl Default for MqttBrokerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_mqtt_port(),
            max_connections: default_mqtt_max_connections(),
            auth: MqttAuthConfig::default(),
            ws_port: None,
            tls: MqttServerTlsConfig::default(),
        }
    }
}

/// TLS configuration for the embedded broker's listener (MQTTS).
///
/// - `enabled` toggles the TLS listener.
/// - `cert_pem` / `key_pem` are the server certificate + private key (PEM).
/// - `ca_pem` optionally enables client-certificate verification (mTLS).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MqttServerTlsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cert_pem: Option<String>,
    #[serde(default)]
    pub key_pem: Option<String>,
    #[serde(default)]
    pub ca_pem: Option<String>,
}

/// Authentication configuration for the MQTT broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttAuthConfig {
    #[serde(default = "default_mqtt_auth_mode")]
    pub mode: String,
    #[serde(default)]
    pub users: Vec<MqttUser>,
}

impl Default for MqttAuthConfig {
    fn default() -> Self {
        Self {
            mode: default_mqtt_auth_mode(),
            users: vec![],
        }
    }
}

/// A single user credential for broker authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttUser {
    pub username: String,
    pub password: String,
}

// ---------------------------------------------------------------------------
// Client configuration
// ---------------------------------------------------------------------------

fn default_client_port() -> u16 {
    1883
}

fn default_keep_alive() -> u64 {
    30
}

/// TLS configuration for an MQTT client connection.
///
/// - `enabled` toggles TLS (MQTTS, typically port 8883).
/// - `ca_pem` pins a private/self-signed CA (PEM body). When omitted, the
///   platform trust store is used.
/// - `client_cert_pem` / `client_key_pem` enable mutual TLS (mTLS).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MqttTlsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ca_pem: Option<String>,
    #[serde(default)]
    pub client_cert_pem: Option<String>,
    #[serde(default)]
    pub client_key_pem: Option<String>,
}

/// Last Will & Testament published by the broker if the client disconnects
/// uncleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttLastWill {
    pub topic: String,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub qos: MqttQos,
    #[serde(default)]
    pub retain: bool,
}

/// Configuration for an MQTT client connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttClientConfig {
    pub host: String,
    #[serde(default = "default_client_port")]
    pub port: u16,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// TLS settings (port 8883 / mTLS / private CA).
    #[serde(default)]
    pub tls: MqttTlsConfig,
    /// Keep-alive interval in seconds.
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
    /// Optional Last Will & Testament.
    #[serde(default)]
    pub last_will: Option<MqttLastWill>,
}

impl Default for MqttClientConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: default_client_port(),
            client_id: String::new(),
            username: None,
            password: None,
            tls: MqttTlsConfig::default(),
            keep_alive_secs: default_keep_alive(),
            last_will: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// QoS level for MQTT messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MqttQos {
    #[default]
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

impl From<MqttQos> for rumqttc::QoS {
    fn from(qos: MqttQos) -> Self {
        match qos {
            MqttQos::AtMostOnce => rumqttc::QoS::AtMostOnce,
            MqttQos::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
            MqttQos::ExactlyOnce => rumqttc::QoS::ExactlyOnce,
        }
    }
}

impl From<rumqttc::QoS> for MqttQos {
    fn from(qos: rumqttc::QoS) -> Self {
        match qos {
            rumqttc::QoS::AtMostOnce => MqttQos::AtMostOnce,
            rumqttc::QoS::AtLeastOnce => MqttQos::AtLeastOnce,
            rumqttc::QoS::ExactlyOnce => MqttQos::ExactlyOnce,
        }
    }
}

/// An MQTT publish message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttMessage {
    pub topic: String,
    #[serde(with = "bytes_to_string")]
    pub payload: Vec<u8>,
    #[serde(default)]
    pub qos: MqttQos,
    #[serde(default)]
    pub retained: bool,
}

mod bytes_to_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        match String::from_utf8(data.clone()) {
            Ok(st) => s.serialize_str(&st),
            Err(_) => s.serialize_str(&hex::encode(data)),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        // Always treat the incoming string as raw UTF-8 bytes. We intentionally
        // do NOT hex-decode: a publish payload like "deadbeef" or "1234" is
        // valid hex and would otherwise be silently corrupted into binary.
        // (Binary round-trips are not needed on the client publish path.)
        Ok(String::deserialize(d)?.into_bytes())
    }
}

// ---------------------------------------------------------------------------
// Broker introspection types
// ---------------------------------------------------------------------------

/// Runtime status of the embedded MQTT broker.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BrokerStatus {
    Stopped,
    Starting,
    Running,
    Error(String),
}

/// Connected client info.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectedClient {
    pub client_id: String,
    pub connected_at: i64,
    pub subscriptions: Vec<String>,
}

/// Tracked information for a single MQTT topic.
#[derive(Debug, Clone, Serialize)]
pub struct TopicInfo {
    pub topic: String,
    #[serde(with = "bytes_to_string")]
    pub last_payload: Vec<u8>,
    pub last_update: i64,
    pub message_count: u64,
    pub retain: bool,
}

/// A single recent message for the message stream.
#[derive(Debug, Clone, Serialize)]
pub struct RecentMessage {
    pub topic: String,
    #[serde(with = "bytes_to_string")]
    pub payload: Vec<u8>,
    pub timestamp: i64,
    pub retain: bool,
}
