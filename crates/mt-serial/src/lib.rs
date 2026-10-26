//! `mt-serial` - RS232/RS485 serial communication crate for the MacTools suite.
//!
//! Provides:
//! - A [`SerialClient`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - A [`list_ports`] function for enumerating available serial ports
//! - One-shot [`read_tags`] / [`write_tag`] helpers
//!
//! # Example
//!
//! ```no_run
//! use mt_serial::{SerialClient, SerialConfig, SerialTag, SerialValue, list_ports};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // List available ports
//!     for port in list_ports()? {
//!         println!("{} ({})", port.port_name, port.port_type);
//!     }
//!
//!     let config = SerialConfig {
//!         port_name: "/dev/ttyUSB0".into(),
//!         baud_rate: 9600,
//!         data_bits: 8,
//!         stop_bits: 1,
//!         parity: mt_serial::Parity::None,
//!         flow_control: mt_serial::FlowControl::None,
//!         timeout_ms: 1000,
//!     };
//!
//!     let client = SerialClient::connect(config).await?;
//!     let tags = vec![SerialTag {
//!         name: "READ".into(),
//!         address: Some(1),
//!     }];
//!     let values = client.read(&tags).await?;
//!     for v in &values {
//!         println!("{}: {}", v.formatted_hex, v.timestamp);
//!     }
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod client;
#[cfg(unix)]
pub mod pty;
pub mod stream;
pub mod types;

// Re-export the primary public API at the crate root.
pub use client::{SerialClient, list_ports, read_tags, write_tag};
pub use types::{
    FlowControl, Parity, PortInfo, SerialConfig, SerialTag, SerialValue,
};
pub use stream::{is_pseudo_terminal, open_stream};
/// The async stream [`open_stream`] returns.
pub use tokio_serial::SerialStream;
