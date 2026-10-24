pub mod counter;
#[cfg(feature = "io")]
pub mod io_enip;
#[cfg(feature = "io")]
pub mod io_manager;
#[cfg(feature = "io")]
pub mod io_mqtt;
#[cfg(feature = "io")]
pub mod io_opcua;
#[cfg(feature = "io")]
pub mod io_s7;
pub mod memory;
pub mod opcode;
pub mod scheduler;
pub mod timer;
pub mod types;
pub mod vm;

#[cfg(feature = "io")]
pub use io_manager::{
    IoAddress, IoArea, IoDriver, IoSize, ModbusIoDriver, create_driver, parse_direct_address,
};
#[cfg(feature = "io")]
pub use io_mqtt::MqttIoDriver;
#[cfg(feature = "io")]
pub use io_opcua::OpcUaIoDriver;
#[cfg(feature = "io")]
pub use io_s7::S7IoDriver;
pub use memory::MemoryModel;
pub use mt_plc_types::PlcValue;
pub use opcode::{CompiledModule, Instruction, Opcode};
pub use scheduler::Scheduler;
pub use types::{CycleStats, RuntimeConfig, RuntimeState};
pub use vm::{ExecutionState, FbSnapshot, Vm};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedVariable {
    pub name: String,
    pub value: PlcValue,
    pub display_value: String,
    pub type_name: String,
    pub quality: String,
}

pub struct PlcRuntime {
    vm: Vm,
    instructions: Vec<Instruction>,
    line_map: Vec<(usize, usize)>,
    scheduler: Scheduler,
    state: RuntimeState,
    config: RuntimeConfig,
    cycle_count: u64,
    stats: CycleStats,
}

impl PlcRuntime {
    pub fn new(config: RuntimeConfig, module: CompiledModule) -> Self {
        let instructions = module.instructions.clone();
        let line_map = module.line_map.clone();
        let vm = Vm::from_instructions(module);

        let scheduler = Scheduler::new(config.scan_cycle_ms, config.max_cycle_ms);

        Self {
            vm,
            instructions,
            line_map,
            scheduler,
            state: RuntimeState::Stopped,
            config,
            cycle_count: 0,
            stats: CycleStats::default(),
        }
    }

    pub fn state(&self) -> &RuntimeState {
        &self.state
    }

    pub fn cycle_count(&self) -> u64 {
        self.cycle_count
    }

    pub fn stats(&self) -> &CycleStats {
        &self.stats
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: RuntimeConfig) {
        self.scheduler.set_cycle_time(config.scan_cycle_ms);
        self.config = config;
    }

    // ── Debug / Breakpoint methods ──

    pub fn set_breakpoint_line(&mut self, line: usize) {
        for &(instr_idx, src_line) in &self.line_map {
            if src_line == line {
                self.vm.set_breakpoint(instr_idx);
            }
        }
    }

    pub fn clear_breakpoint_line(&mut self, line: usize) {
        for &(instr_idx, src_line) in &self.line_map {
            if src_line == line {
                self.vm.clear_breakpoint(instr_idx);
            }
        }
    }

    pub fn clear_all_breakpoints(&mut self) {
        self.vm.clear_all_breakpoints();
    }

    pub fn get_breakpoint_lines(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = self
            .vm
            .breakpoints
            .iter()
            .filter_map(|&pc| {
                self.line_map
                    .iter()
                    .find(|&&(idx, _)| idx == pc)
                    .map(|&(_, line)| line)
            })
            .collect();
        lines.sort();
        lines.dedup();
        lines
    }

    pub fn step_instruction(&mut self) -> Result<ExecutionState, String> {
        self.vm.step_into();
        self.vm.execute_all(&self.instructions)
    }

    pub fn continue_from_breakpoint(&mut self) -> Result<ExecutionState, String> {
        self.vm.continue_execution();
        self.vm.execute_all(&self.instructions)
    }

    pub fn current_pc_line(&self) -> Option<usize> {
        let pc = self.vm.last_pc;
        self.line_map
            .iter()
            .find(|&&(idx, _)| idx == pc)
            .map(|&(_, line)| line)
    }

    // ── End debug methods ──

    pub fn execute_cycle(&mut self) -> Result<CycleStats, String> {
        let start = now_ms();

        self.state = RuntimeState::Running;
        self.vm.execute(&self.instructions)?;

        let elapsed = now_ms() - start;

        self.cycle_count += 1;
        self.stats.last_cycle_ms = elapsed;
        self.stats.cycle_count = self.cycle_count;
        self.stats.avg_cycle_ms = (self.stats.avg_cycle_ms * (self.cycle_count - 1) as f64
            + elapsed)
            / self.cycle_count as f64;
        if elapsed > self.stats.max_cycle_ms {
            self.stats.max_cycle_ms = elapsed;
        }

        self.stats.cycle_history.push(elapsed);
        if self.stats.cycle_history.len() > 200 {
            self.stats.cycle_history.remove(0);
        }

        if elapsed > self.config.max_cycle_ms as f64 {
            self.state = RuntimeState::Error(format!(
                "Cycle time {:.2}ms exceeded watchdog {}ms",
                elapsed, self.config.max_cycle_ms
            ));
            return Err(format!("Watchdog timeout: {:.2}ms", elapsed));
        }

        Ok(self.stats.clone())
    }

    pub fn read_variable(&self, name: &str) -> Option<&PlcValue> {
        self.vm.memory().get_variable(name)
    }

    pub fn write_variable(&mut self, name: &str, value: PlcValue) -> Result<(), String> {
        self.vm.memory_mut().set_variable(name, value)
    }

    pub fn force_variable(&mut self, name: &str, value: PlcValue) -> Result<(), String> {
        self.vm.memory_mut().force_variable(name, value)
    }

    pub fn unforce_variable(&mut self, name: &str) {
        self.vm.memory_mut().unforce_variable(name)
    }

    /// Snapshot current variable values for state persistence across recompile
    pub fn snapshot_variables(&self) -> Vec<(String, PlcValue)> {
        self.vm.memory().variables_snapshot()
    }

    /// Restore variable values from a previous snapshot, matching by name
    pub fn restore_variables(&mut self, snapshot: &[(String, PlcValue)]) {
        for (name, value) in snapshot {
            let _ = self.write_variable(name, value.clone());
        }
    }

    pub fn snapshot_fb_state(&self) -> vm::FbSnapshot {
        self.vm.snapshot_fb_state()
    }

    pub fn restore_fb_state(&mut self, snapshot: &vm::FbSnapshot) {
        self.vm.restore_fb_state(snapshot);
    }

    pub fn watch_variables(&self) -> Vec<WatchedVariable> {
        self.vm
            .memory()
            .variable_names()
            .iter()
            .map(|name| {
                let (value, type_name, quality) = self.vm.memory().get_watch_info(name);
                WatchedVariable {
                    display_value: value.to_string(),
                    name: name.clone(),
                    value,
                    type_name,
                    quality,
                }
            })
            .collect()
    }
}

/// Monotonic wall clock in milliseconds for cycle timing.
/// `Instant` is unavailable on wasm32-unknown-unknown, so wasm uses `js_sys::Date::now()`
/// and native uses `Instant` anchored at first call.
fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::LazyLock;
        static EPOCH: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
        EPOCH.elapsed().as_secs_f64() * 1000.0
    }
}
