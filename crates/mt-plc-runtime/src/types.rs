pub use mt_plc_types::PlcValue;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub scan_cycle_ms: u64,
    pub max_cycle_ms: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            scan_cycle_ms: 10,
            max_cycle_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuntimeState {
    Stopped,
    Running,
    Paused,
    Error(String),
}

impl RuntimeState {
    pub fn name(&self) -> &str {
        match self {
            RuntimeState::Stopped => "stopped",
            RuntimeState::Running => "running",
            RuntimeState::Paused => "paused",
            RuntimeState::Error(_) => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CycleStats {
    pub cycle_count: u64,
    pub last_cycle_ms: f64,
    pub avg_cycle_ms: f64,
    pub max_cycle_ms: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycle_history: Vec<f64>,
}
