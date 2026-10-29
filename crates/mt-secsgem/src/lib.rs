//! `mt-secsgem` - SECS/GEM host client and equipment simulator in pure Rust.
//!
//! SECS/GEM is the protocol stack semiconductor and electronics-manufacturing equipment speaks to
//! a factory host: HSMS (SEMI E37) carries SECS-II messages (SEMI E5) over TCP, and GEM
//! (SEMI E30) defines what the messages mean. This crate is an independent implementation of the
//! public parts of that stack; see the README for sources, what is implemented, what was
//! cross-checked and what was not.
//!
//! - [`hsms`]: framing, Select/Deselect/Linktest/Separate/Reject, timers T3 T5 T6 T7 T8, active
//!   and passive connections.
//! - [`item`], [`sml`]: the SECS-II item codec and the SML text notation.
//! - [`GemHost`]: async host with one typed helper per GEM stream/function plus raw sends.
//! - [`EquipmentSimulator`]: a wafer processing chamber behind a GEM interface, over a mutable
//!   [`EquipmentModel`].
//!
//! # Example
//!
//! ```no_run
//! use mt_secsgem::{GemHost, GemHostConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let host = GemHost::connect(GemHostConfig::new("192.168.1.60:5000")).await?;
//!     let (ack, tool) = host.establish_communications().await?;
//!     println!("{ack:?} {tool:?}");
//!     println!("{:?}", host.status_variables(&[]).await?);
//!     host.disconnect();
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod gem;
pub mod host;
pub mod hsms;
pub mod item;
pub mod message;
pub mod sim;
mod sml;

pub use error::{
    CodecError, RejectReason, Result, S9Error, S9Kind, SecsError, SelectStatus, Timer,
};
pub use gem::*;
pub use host::{GemHost, GemHostConfig, HostEvent};
pub use hsms::{
    Connector, DEFAULT_PORT, Direction, Frame, Header, HsmsConfig, HsmsConnection, HsmsLink,
    HsmsListener, HsmsState, Incoming, LogEntry, SType,
};
pub use item::Item;
pub use message::SecsMessage;
pub use sim::{
    EquipmentModel, EquipmentSimulator, ProcessState, SimMode, SimulatorConfig, TerminalMessage,
};
