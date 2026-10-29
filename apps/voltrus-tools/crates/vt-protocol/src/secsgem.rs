//! SECS/GEM sessions over HSMS: a host (client) of a piece of equipment, and a
//! simulated equipment (a wafer processing chamber).
//!
//! SECS/GEM, SECS-II, HSMS and the SEMI standards behind them belong to SEMI.
//! This is an independent implementation, not affiliated with SEMI.

use serde::{Deserialize, Serialize};

/// The port HSMS uses by convention. macOS's AirPlay Receiver listens on 5000,
/// so the simulator defaults to another port.
pub const SECSGEM_PORT: u16 = 5000;

/// The simulator's default port.
pub const SECSGEM_SIM_PORT: u16 = 5600;

/// Most ids one list (status variables, equipment constants, events) holds.
pub const SECSGEM_MAX_IDS: usize = 256;

/// Most status variables a report of the events subscription carries.
pub const SECSGEM_MAX_REPORT_VIDS: usize = 32;

/// Act as the GEM host of a piece of equipment: read its status variables
/// and equipment constants, write the constants, and receive its events and
/// alarms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecsGemClientSpec {
    pub host: String,
    pub port: u16,
    /// HSMS device (session) id.
    pub device_id: u16,
    pub poll_ms: u64,
    /// T3, how long a reply is waited for.
    pub timeout_ms: u64,
    /// Status variables to read; none means every one the equipment names.
    pub svids: Vec<u64>,
    /// Equipment constants to read (and write); none means every one.
    pub ecids: Vec<u64>,
    /// Collection events to subscribe to: a report of the status variables is
    /// linked to each and the events are enabled.
    pub events: Vec<u64>,
    /// Enable the equipment's alarm reports.
    pub alarms: bool,
}

impl Default for SecsGemClientSpec {
    fn default() -> Self {
        Self {
            host: "192.168.1.60".into(),
            port: SECSGEM_PORT,
            device_id: 0,
            poll_ms: 1000,
            timeout_ms: 10_000,
            svids: Vec::new(),
            ecids: Vec::new(),
            events: Vec::new(),
            alarms: true,
        }
    }
}

/// Play a wafer processing chamber behind a GEM interface, for any GEM host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecsGemSimulatorSpec {
    /// Address to listen on: `0.0.0.0` for every interface.
    pub bind: String,
    pub port: u16,
    /// HSMS device (session) id.
    pub device_id: u16,
    /// Keep processing wafers: start a process whenever the chamber is idle.
    pub run: bool,
}

impl Default for SecsGemSimulatorSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: SECSGEM_SIM_PORT,
            device_id: 0,
            run: true,
        }
    }
}
