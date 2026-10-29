//! FANUC CNC sessions over FOCAS/Ethernet: a client of a controller, and a
//! simulated controller.
//!
//! FANUC and FOCAS are trademarks of FANUC CORPORATION. This is independent
//! software, not affiliated with FANUC.

use serde::{Deserialize, Serialize};

/// The port of FOCAS/Ethernet.
pub const FOCAS_PORT: u16 = 8193;

/// Most macro variables one connection reads.
pub const FOCAS_MAX_MACROS: usize = 64;

/// The controller family the simulator presents itself as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocasProfile {
    /// Series 31i-B.
    #[default]
    Series31iB,
    /// Series 0i-F.
    Series0iF,
}

impl FocasProfile {
    pub const ALL: &'static [FocasProfile] = &[Self::Series31iB, Self::Series0iF];

    pub fn name(self) -> &'static str {
        match self {
            Self::Series31iB => "31i-B",
            Self::Series0iF => "0i-F",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Series31iB => {
                "A three-axis mill reporting a 31i-B: status, axes, spindle, programs, alarms and macros"
            }
            Self::Series0iF => "The same mill reporting a 0i-F (only its identity differs)",
        }
    }
}

/// Poll a FANUC controller: identity, status, axis positions, feed, spindle,
/// the running program, alarms, and the macro variables you list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FocasClientSpec {
    pub host: String,
    pub port: u16,
    pub poll_ms: u64,
    pub timeout_ms: u64,
    /// Common macro variable numbers to read (and write), e.g. 500.
    pub macros: Vec<u32>,
}

impl Default for FocasClientSpec {
    fn default() -> Self {
        Self {
            host: "192.168.1.50".into(),
            port: FOCAS_PORT,
            poll_ms: 500,
            timeout_ms: 5000,
            macros: vec![500],
        }
    }
}

/// Play a FANUC controller on this computer for any FOCAS/Ethernet client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FocasSimulatorSpec {
    /// Address to listen on: `0.0.0.0` for every interface.
    pub bind: String,
    pub port: u16,
    pub profile: FocasProfile,
    /// Start the program cycle (and repeat it) as the simulator starts.
    pub run: bool,
}

impl Default for FocasSimulatorSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: FOCAS_PORT,
            profile: FocasProfile::Series31iB,
            run: true,
        }
    }
}
