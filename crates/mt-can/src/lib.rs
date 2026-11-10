//! `mt-can`: CAN and CAN FD for CAN Explorer.
//!
//! - [`frame`]: frames, identifiers, DLC and hex helpers
//! - [`slcan`]: the SLCAN (Lawicel) ASCII protocol of USB-CAN adapters (CANable, CANUSB, USBtin)
//! - [`gsusb`]: gs_usb / candleLight USB adapters, driven from user space (no driver needed on macOS)
//! - [`bus`]: opening an SLCAN or gs_usb adapter, a Linux SocketCAN interface or the in-process virtual bus
//! - [`dbc`]: DBC parsing, signal decoding and encoding
//! - [`log`]: candump log and CSV traces
//! - [`protocols`]: J1939, CANopen and OBD-II labels
//! - [`sim`]: a simulated genset (J1939 engine, generator, pump drive, CANopen I/O node, OBD-II)

pub mod bus;
pub mod dbc;
pub mod error;
pub mod frame;
pub mod gsusb;
pub mod log;
pub mod protocols;
pub mod sim;
pub mod slcan;
#[cfg(target_os = "linux")]
mod socketcan;

pub use bus::{BusEvent, CanHandle, CanSender, InterfaceConfig, PortInfo, VirtualBus, list_ports, open};
pub use dbc::{Dbc, Message, Signal, SignalValue};
pub use error::CanError;
pub use frame::{CanFrame, Direction, TimedFrame};
