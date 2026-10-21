//! `mt-modbus` - Standalone Modbus TCP/RTU client crate for the MacTools suite.
//!
//! This crate provides:
//! - A [`ModbusClient`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - A Modbus slave ([`start_slave_server`], plus RTU variants with the `serial` feature)
//! - A device [`Simulation`] that drives a slave's register image
//!
//! # Example
//!
//! ```no_run
//! use mt_modbus::{ModbusClient, ModbusConfig, ModbusTag, RegisterType};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ModbusConfig {
//!         ip: "192.168.1.100".into(),
//!         port: 502,
//!         unit_id: 1,
//!         interval_ms: 1000,
//!         timeout_ms: 5000,
//!         endianness: Default::default(),
//!     };
//!
//!     let client = ModbusClient::connect(config).await?;
//!     // Addresses are 0-based protocol addresses: this reads holding registers 0 and 1
//!     // (the register shown as 40001 in 1-based documentation).
//!     let tags = vec![ModbusTag {
//!         name: "voltage".into(),
//!         address: 0,
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
mod link;
pub mod server;
pub mod simulator;
pub mod types;

// Re-export the primary public API at the crate root.
pub use client::ModbusClient;
pub use server::{SlaveHandles, SlaveState, start_slave_server};
#[cfg(feature = "serial")]
pub use server::start_slave_server_rtu;
#[cfg(all(feature = "serial", unix))]
pub use server::start_slave_server_rtu_virtual;
pub use simulator::{
    SimDeviceProfile, SimLocks, SimRegisterDef, Simulation, start_simulation, stop_simulation,
    tick_values,
};
pub use types::{
    BusScanResult, Endianness, ModbusConfig, ModbusTag, ModbusValue, RegisterType, SlaveConfig,
    SlaveLogEntry, TrafficLogEntry, check_writable, parse_register, value_to_words,
};
#[cfg(feature = "serial")]
pub use types::{RtuParity, RtuSerialConfig};
