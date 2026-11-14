//! `mt-ethercat`: EtherCAT for EtherCAT Explorer.
//!
//! - [`wire`], [`esc`], [`sii`], [`coe`]: frames and datagrams, the ESC register map, SII EEPROM images, and CoE
//!   mailbox messages (SDO, SDO Information), each with encoders and decoders.
//! - [`sim`]: a simulated segment of EtherCAT devices (couplers, I/O terminals, CiA 402 servo drives) that answers
//!   frames at register level, so the explorer works with no hardware.
pub mod coe;
pub mod esc;
pub mod link;
pub mod master;
pub mod sii;
pub mod sim;
pub mod wire;
