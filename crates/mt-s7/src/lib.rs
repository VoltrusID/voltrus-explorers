//! `mt-s7` - Siemens S7 protocol client crate for the MacTools suite.
//!
//! Implements ISO-on-TCP (TPKT + COTP + S7) directly over TCP with no external
//! S7 library dependency.
//!
//! Provides:
//! - [`S7Client`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - [`S7Tag::parse_address`] for parsing tag address strings like `"DB1.DBD0"`
//! - Standalone [`read_tags`] / [`write_tag`] helpers for one-shot operations
//!
//! # Example
//!
//! ```no_run
//! use mt_s7::{S7Client, S7Config, S7Tag, S7Area, S7DataType, S7Value};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = S7Config {
//!         id: "plc-1".into(),
//!         host: "192.168.1.100".into(),
//!         port: 102,
//!         rack: 0,
//!         slot: 1,
//!         poll_interval_ms: 1000,
//!     };
//!
//!     let client = S7Client::connect(config).await?;
//!     let tags = vec![S7Tag {
//!         area: S7Area::DB,
//!         db_number: 1,
//!         offset: 0,
//!         bit_offset: 0,
//!         data_type: S7DataType::DWord,
//!         length: 1,
//!         name: Some("voltage".into()),
//!     }];
//!     let values = client.read(&tags).await?;
//!     println!("{:?}", values);
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod server;
pub mod types;

// Re-export the primary public API at the crate root.
pub use client::{S7Client, parse_area_string, read_tags, write_tag};
pub use server::S7Server;
pub use types::{
    BlockInfo, BlockListEntry, BlockType, CpuInfo, DiagBuffer, DiagEntry, PlcMode,
    S7Area, S7Config, S7DataType, S7Tag, S7Value, DEFAULT_PDU_SIZE, MAX_PDU_SIZE,
};
