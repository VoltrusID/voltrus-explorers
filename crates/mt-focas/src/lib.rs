//! `mt-focas` - FANUC FOCAS/Ethernet client and CNC simulator in pure Rust.
//!
//! FANUC ships its FOCAS library only as a C library for Windows and Linux. This crate speaks the
//! protocol directly over TCP (port 8193), so it runs anywhere Rust does, including macOS. The
//! wire protocol is the one reverse-engineered by [pyfanuc](https://github.com/diohpix/pyfanuc)
//! (Unlicense); see the README for provenance, what is implemented, and what has and has not been
//! verified. It is independent of FANUC and is not verified against a real controller.
//!
//! - [`FocasClient`]: async client with typed results (status, axes, positions, feed, spindle,
//!   program, alarms, macro variables, parameters, diagnosis, PMC).
//! - [`FocasSimulator`]: a CNC that answers the same calls from a [`CncModel`] the host can change
//!   while it runs.
//! - [`codec`] and [`functions`]: the framing and payload layouts both sides share.
//!
//! # Example
//!
//! ```no_run
//! use mt_focas::{FocasClient, FocasConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = FocasClient::connect(FocasConfig::new("192.168.1.50")).await?;
//!     println!("{:?}", client.sysinfo().await?);
//!     let snapshot = client.snapshot().await?;
//!     println!("{:?}", snapshot.status);
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod codec;
pub mod error;
pub mod functions;
pub mod model;
pub mod simulator;
pub mod types;

pub use client::{DEFAULT_PORT, FocasClient, FocasConfig, MAX_SUBPACKETS};
pub use codec::{CodecError, Command, Frame, FrameType, SubRequest, SubResponse};
pub use error::{FocasError, ReadFailure, Reading, ew, ew_name};
pub use model::{CncModel, CncProfile, SimAlarm, SimAxis, SimBlock, SimNumbered, SimProgram};
pub use simulator::{FocasSimulator, SimulatorConfig};
pub use types::*;
