/// Cyclic scan scheduler
pub struct Scheduler {
    cycle_time_ms: u64,
    max_cycle_ms: u64,
}

impl Scheduler {
    pub fn new(cycle_time_ms: u64, max_cycle_ms: u64) -> Self {
        Self {
            cycle_time_ms,
            max_cycle_ms,
        }
    }

    pub fn cycle_time(&self) -> u64 {
        self.cycle_time_ms
    }

    pub fn max_cycle_time(&self) -> u64 {
        self.max_cycle_ms
    }

    pub fn set_cycle_time(&mut self, ms: u64) {
        self.cycle_time_ms = ms;
    }

    pub fn set_max_cycle_time(&mut self, ms: u64) {
        self.max_cycle_ms = ms;
    }

    /// Calculate sleep time for remaining cycle budget
    pub fn remaining_sleep(&self, elapsed_ms: f64) -> u64 {
        let budget = self.cycle_time_ms as f64;
        if elapsed_ms >= budget {
            0
        } else {
            (budget - elapsed_ms) as u64
        }
    }

    /// Check if elapsed time exceeds the watchdog
    pub fn is_watchdog_exceeded(&self, elapsed_ms: f64) -> bool {
        elapsed_ms > self.max_cycle_ms as f64
    }
}
