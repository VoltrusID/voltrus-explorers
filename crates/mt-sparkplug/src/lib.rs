//! Sparkplug B edge publishers.
//!
//! [`EdgeNode`] (module [`edge`]) is the full edge node used by Voltrus
//! Gateway: devices (DBIRTH/DDATA/DDEATH), aliases, report by exception,
//! NCMD Rebirth and DCMD writes, driven over [`EdgeLink`].
//!
//! [`SparkplugPublisher`] below is the earlier node-only publisher used by
//! the PLC runtime:
//! Scope (Atur GW-3): publish-only. One edge node, one broker session:
//! connect → NBIRTH (all metrics + `bdSeq`) → NDATA with report-by-exception
//! → NDEATH as the MQTT Will and on graceful shutdown. No NCMD, no aliases,
//! no Templates, no primary-host STATE handling, no TCK conformance — see
//! README.md for the honest gap list.
//!
//! [`SparkplugPublisher`] is generic over [`MqttTransport`] so the state
//! machine can be driven against a mock in tests; [`RumqttcTransport`] is the
//! real transport over `rumqttc`.

mod bdseq;
pub mod edge;
pub mod link;
mod publisher;
mod topic;
mod transport;
mod value;

pub mod pb;

pub use edge::{CommandOutcome, DeviceWrite, EdgeNode, EdgeNodeConfig, Outgoing, SpValue};
pub use link::{BrokerConfig, EdgeLink, LinkEvent};
pub use pb::payload::Metric;
pub use pb::{DataType, Payload};
pub use publisher::{MetricSource, SparkplugConfig, SparkplugPublisher};
pub use topic::{SparkplugVerb, sparkplug_topic};
pub use transport::{MqttTransport, RumqttcTransport, Will};
pub use value::MetricValue;

/// `uuid` stamped on every payload (schema identifier expected by the TCK
/// and common host applications).
pub(crate) const PAYLOAD_UUID: &str = "Sparkplug B";

#[derive(Debug, thiserror::Error)]
pub enum SparkplugError {
    #[error("bdSeq store {path}: {source}")]
    BdSeqIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("bdSeq store {path}: contents are not a bdSeq number")]
    BdSeqCorrupt { path: std::path::PathBuf },
    #[error("mqtt transport: {0}")]
    Transport(String),
    #[error("not connected: rebirth requires an open session")]
    NotConnected,
}
