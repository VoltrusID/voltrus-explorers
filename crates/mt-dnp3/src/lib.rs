//! `mt-dnp3` - Standalone DNP3 client crate for the MacTools suite.
//!
//! Extracted from the Voltrus SCADA platform, this crate provides:
//! - A [`Dnp3Client`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - A custom DNP3 protocol stack (data-link, application, object parsing)
//! - Network discovery helpers
//!
//! # Example
//!
//! ```no_run
//! use mt_dnp3::{Dnp3Client, Dnp3Config, Dnp3Point, Dnp3Value};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Dnp3Config {
//!         id: "rtu-1".into(),
//!         host: "192.168.1.100".into(),
//!         port: 20000,
//!         master_address: 1,
//!         outstation_address: 10,
//!         poll_interval_ms: 1000,
//!     };
//!
//!     let client = Dnp3Client::connect(config).await?;
//!     let points = vec![Dnp3Point {
//!         name: "voltage".into(),
//!         group: 30,
//!         variation: 4,
//!         index: 0,
//!         scale: 1.0,
//!     }];
//!     let values = client.read(&points).await?;
//!     println!("{:?}", values);
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod application;
pub mod client;
pub mod datalink;
pub mod objects;
pub mod server;
pub mod simulation;
pub mod transport;
pub mod types;

// Re-export the primary public API at the crate root.
pub use client::{Dnp3Client, SequenceTracker, build_link_reset_frame, event_scan, read_points, write_point};
pub use datalink::{ControlByte, Dnp3Frame, crc16, describe_frame};
pub use objects::{Dnp3PointValue, common_group_variations, group_variation_name, parse_objects};
pub use server::{OutstationState, start_outstation_server};
pub use simulation::{Dnp3Simulation, start_simulation, stop_simulation, tick_values};
pub use transport::{MAX_SEGMENT_PAYLOAD, TransportHeader, TransportReassembler, TransportSegment, segment_asdu};
pub use types::{Backoff, Dnp3Config, Dnp3Point, Dnp3Value, Dnp3WriteResult, OutstationConfig, OutstationEvent, OutstationLogEntry, SimDnp3PointDef};
