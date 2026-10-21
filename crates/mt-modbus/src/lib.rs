//! `mt-modbus` - Standalone Modbus TCP/RTU client crate for the MacTools suite.
//!
//! Extracted from the Voltrus SCADA platform, this crate provides:
//! - A [`ModbusClient`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - A connection [`ModbusPool`] for reusing TCP connections
//! - A [`scan_registers`] function for discovering register ranges
//! - A standalone [`write_tag`] function for one-shot writes
//!
//! # Example
//!
//! ```no_run
//! use mt_modbus::{ModbusClient, ModbusConfig, ModbusTag, ModbusValue, RegisterType};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ModbusConfig {
//!         id: Some("plc-1".into()),
//!         ip: "192.168.1.100".into(),
//!         port: 502,
//!         unit_id: 1,
//!         interval_ms: 1000,
//!         endianness: Default::default(),
//!         bind_addr: None,
//!     };
//!
//!     let client = ModbusClient::connect(config).await?;
//!     let tags = vec![ModbusTag {
//!         name: "voltage".into(),
//!         address: 40001,
//!         reg_type: RegisterType::Float32,
//!         scale: 1.0,
//!         register_area: "holding".into(),
//!     }];
//!     let values = client.read(&tags).await?;
//!     println!("{:?}", values);
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod pool;
pub mod scanner;
pub mod server;
pub mod simulator;
pub mod types;
pub mod write;

// Re-export the primary public API at the crate root.
pub use client::{ModbusClient, read_tags, write_tag as write_tag_client};
pub use pool::ModbusPool;
pub use scanner::scan_registers;
pub use server::{SlaveState, start_slave_server};
#[cfg(feature = "serial")]
pub use server::start_slave_server_rtu;
#[cfg(all(feature = "serial", unix))]
pub use server::start_slave_server_rtu_virtual;
pub use simulator::{Simulation, SimDeviceProfile, SimRegisterDef, start_simulation, stop_simulation, tick_values};
pub use types::{
    Backoff, BusScanResult, Endianness, ModbusConfig, ModbusTag, ModbusValue, RegisterType,
    ScannedRegister, SlaveConfig, SlaveLogEntry, TrafficLogEntry, WriteResult, combine_u16,
    parse_register, value_to_words,
};
#[cfg(feature = "serial")]
pub use types::{RtuParity, RtuSerialConfig};
pub use write::write_tag;
