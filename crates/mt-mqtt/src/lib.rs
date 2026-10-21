//! `mt-mqtt` - Standalone MQTT broker + client crate for the Voltrus explorer apps.
//!
//! Extracted from the Voltrus SCADA platform, this crate provides:
//! - An **embedded MQTT broker** ([`MqttBrokerState`], [`start_broker`], [`stop_broker`])
//! - A **topic tracker** ([`TopicTracker`]) for monitoring broker traffic
//! - An **MQTT client** ([`MqttPubClient`]) for publish/subscribe operations
//! - Auth helpers ([`validate_credentials`])
//! - Payload decoding: Sparkplug B ([`sparkplug`]) and user-schema protobuf ([`protodec`]),
//!   dispatched per topic by [`PayloadDecoder`]
//!
//! # Broker example
//!
//! ```no_run
//! use mt_mqtt::broker::{MqttBrokerState, start_broker, stop_broker};
//! use mt_mqtt::types::MqttBrokerConfig;
//!
//! #[tokio::main]
//! async fn main() {
//!     let state = MqttBrokerState::new();
//!     let config = MqttBrokerConfig::default();
//!     start_broker(state.clone(), config, None).await.unwrap();
//!     // ... later
//!     stop_broker(state).await.unwrap();
//! }
//! ```
//!
//! # Client example
//!
//! ```no_run
//! use mt_mqtt::client_pub::MqttPubClient;
//! use mt_mqtt::types::{MqttClientConfig, MqttMessage, MqttQos};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = MqttClientConfig {
//!         host: "127.0.0.1".into(),
//!         port: 1883,
//!         ..Default::default()
//!     };
//!     let mut client = MqttPubClient::connect(config).await?;
//!     client.subscribe("sensors/#", MqttQos::AtLeastOnce).await?;
//!     while let Some(event) = client.next_event().await {
//!         println!("{:?}", event);
//!     }
//!     Ok(())
//! }
//! ```

pub mod broker;
pub mod client_pub;
pub mod decode;
pub mod protodec;
pub mod sparkplug;
pub mod types;

// Re-export the primary public API at the crate root.
pub use broker::{
    MqttBrokerState, TopicTracker, build_rumqttd_config, build_topic_tree, get_topic_tree,
    start_broker, stop_broker, validate_credentials,
};
pub use client_pub::{MqttClientEvent, MqttPubClient, SubscribeOutcome};
pub use decode::{DecodeKind, Decoded, PayloadDecoder, ProtoConfig, ProtoRule, SharedProtoConfig, topic_matches};
pub use types::{
    BrokerStatus, ConnectedClient, MqttAuthConfig, MqttBrokerConfig, MqttClientConfig, MqttLastWill,
    MqttMessage, MqttQos, MqttServerTlsConfig, MqttTlsConfig, MqttUser, RecentMessage, TopicInfo,
};
