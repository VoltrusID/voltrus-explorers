//! `mt-enip` - Standalone EtherNet/IP (Rockwell) client crate for the MacTools suite.
//!
//! Extracted from the Voltrus SCADA platform, this crate provides:
//! - An [`EnipClient`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - CIP tag read/write operations over EtherNet/IP TCP (port 44818)
//! - Tag browsing via the Symbol class
//! - Packet-level ENIP/CIP encoding and decoding
//!
//! # Example
//!
//! ```no_run
//! use mt_enip::{EnipClient, EnipConfig, CipTag, CipDataType, CipValue};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = EnipConfig {
//!         id: "plc-1".into(),
//!         host: "192.168.1.100".into(),
//!         port: 44818,
//!         poll_interval_ms: 1000,
//!     };
//!
//!     let client = EnipClient::connect(config).await?;
//!     let tags = vec![CipTag {
//!         name: "MyTag".into(),
//!         data_type: CipDataType::Dint,
//!         element_count: 1,
//!         template_id: None,
//!     }];
//!     let values = client.read(&tags).await?;
//!     println!("{:?}", values);
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub use mt_protocol::ProtocolClient;

pub mod client;
pub mod codegen;
pub mod packets;
pub mod server;
pub mod sim;
pub mod templates;
pub mod types;

// Re-export the primary public API at the crate root.
pub use client::{EnipClient, IoConnection, IoSample, browse_tags, discover_devices, read_tags, write_tag};
pub use codegen::{iec_type_from_cip_code, member_type_name, udt_templates_to_st};
pub use templates::read_templates;
pub use server::{start_enip_server, EnipServerState, CipTagEntry};
pub use sim::{EnipSimDevice, SimPointDef, SimSnapPoint, SimSnapshot};
pub use types::{
    Backoff, CipDataType, CipTag, CipTagDef, CipTemplateDef, CipTemplateMember, CipValue, EnipConfig, EnipDevice,
    EnipServerConfig, EnipServerLogEntry, ForwardOpenParams, ForwardOpenResult, IdentityInfo,
    PacketDir, PacketRecord,
    IDENTITY_CLASS_ID, validate_tag_name,
    vendor_name,
};
