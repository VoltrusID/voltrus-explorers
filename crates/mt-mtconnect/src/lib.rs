//! `mt-mtconnect` - MTConnect client and simulator for the MacTools suite.
//!
//! - [`AgentClient`]: HTTP client for an MTConnect agent (`probe`, `current`, `sample`,
//!   `assets`, and a live [`SampleStream`] that uses the agent's `multipart` streaming with a
//!   polling fallback and survives agent restarts and `OUT_OF_RANGE`).
//! - [`parse`]: namespace-agnostic parsing of `MTConnectDevices`, `MTConnectStreams`,
//!   `MTConnectAssets` and `MTConnectError` documents (MTConnect 1.x and 2.x).
//! - [`sim`]: an agent simulator (HTTP agent + SHDR adapter) over a mutable [`SimModel`].
//!
//! All data types are serde-serializable so UIs can pass them straight to a front end.
//!
//! ```no_run
//! use mt_mtconnect::AgentClient;
//!
//! # async fn demo() -> Result<(), mt_mtconnect::MtConnectError> {
//! let client = AgentClient::from_url("https://demo.mtconnect.org")?;
//! let devices = client.probe().await?;
//! let current = client.current(None).await?;
//! println!("{} devices, {} observations", devices.devices.len(), current.len());
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod error;
pub mod multipart;
pub mod parse;
pub mod sim;
pub mod types;
mod util;
mod xml;

pub use client::{
    AgentClient, AgentConfig, ResyncReason, SampleQuery, SampleStream, StreamEvent, StreamOptions,
    StreamTransport,
};
pub use error::{AgentError, AgentErrorCode, AgentErrors, MtConnectError, Result};
pub use parse::{Document, parse_assets, parse_devices, parse_document, parse_streams};
pub use sim::{AgentSimulator, DeviceTemplate, SimConfig, SimItemState, SimModel, SimSnapshot};
pub use types::*;
pub use util::{iso_from_micros, now_iso};
