use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::cell::Cell;

use crate::server::SlaveState;

// Simple xorshift32 PRNG — no external dependency needed.
thread_local! {
    static RNG_STATE: Cell<u32> = const { Cell::new(0xDEAD_BEEF) };
}

fn xorshift32() -> u32 {
    RNG_STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        s.set(x);
        x
    })
}

/// A single register definition within a device profile.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimRegisterDef {
    pub name: String,
    pub unit: String,
    pub description: String,
    pub initial: f64,
    pub min: f64,
    pub max: f64,
    pub drift: f64,
    pub is_counter: bool,
    pub counter_rate: f64,
    #[serde(default)]
    pub locked: bool,
}

/// A device profile for simulation.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDeviceProfile {
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub description: String,
    pub base_address: u16,
    pub registers: Vec<SimRegisterDef>,
}

/// Running simulation state.
pub struct Simulation {
    pub profile: SimDeviceProfile,
    pub values: Vec<f64>,
    pub load_factor: f32,
}

impl Simulation {
    pub fn new(profile: SimDeviceProfile) -> Self {
        let values = profile.registers.iter().map(|r| r.initial).collect();
        Self {
            profile,
            values,
            load_factor: 0.0,
        }
    }

    #[allow(dead_code)]
    fn num_registers(&self) -> usize {
        self.profile.registers.iter()
            .map(|r| if r.is_counter { 4 } else { 2 })
            .sum()
    }

    pub fn write_to_slave(&self, state: &Arc<std::sync::Mutex<SlaveState>>) {
        let mut data = state.lock().unwrap();
        let base = self.profile.base_address as usize;
        let mut offset = 0usize;

        for (i, def) in self.profile.registers.iter().enumerate() {
            if def.is_counter {
                let raw = self.values[i] as u64;
                let hi32 = (raw >> 32) as u32;
                let lo32 = (raw & 0xFFFFFFFF) as u32;
                let regs = [
                    (hi32 >> 16) as u16,
                    (hi32 & 0xFFFF) as u16,
                    (lo32 >> 16) as u16,
                    (lo32 & 0xFFFF) as u16,
                ];
                for (j, &v) in regs.iter().enumerate() {
                    let addr = base + offset + j;
                    if addr < data.registers.len() {
                        data.registers[addr] = v;
                    }
                }
                offset += 4;
            } else {
                let f32_val = self.values[i] as f32;
                let bits = f32_val.to_bits();
                let regs = [
                    (bits >> 16) as u16,
                    (bits & 0xFFFF) as u16,
                ];
                for (j, &v) in regs.iter().enumerate() {
                    let addr = base + offset + j;
                    if addr < data.registers.len() {
                        data.registers[addr] = v;
                    }
                }
                offset += 2;
            }
        }
    }
}

fn rand_range(min: f64, max: f64) -> f64 {
    let r = xorshift32() as f64 / u32::MAX as f64;
    min + (max - min) * r
}

/// Tick all simulation values forward one step.
pub fn tick_values(sim: &mut Simulation) {
    // Slowly-varying load factor (sinusoidal with noise)
    sim.load_factor = (sim.load_factor + 2.0 * std::f32::consts::PI / 900.0
        + rand_range(-0.001, 0.001) as f32)
        % (2.0 * std::f32::consts::PI);
    let load = 1.0 + 0.15 * sim.load_factor.sin();

    for (i, def) in sim.profile.registers.iter().enumerate() {
        if def.locked { continue; }
        if def.is_counter {
            let variation = (xorshift32() as f64 / u32::MAX as f64 - 0.5) * def.counter_rate * 0.1;
            sim.values[i] += def.counter_rate + variation;
        } else {
            let drift = rand_range(-def.drift, def.drift);
            let load_effect = if def.unit == "W" || def.unit == "A" || def.unit == "VA" || def.unit == "var" {
                (load - 1.0) * def.initial.abs() as f32 * 0.1
            } else if def.unit == "V" || def.unit == "kV" {
                -(load - 1.0) * 0.5
            } else {
                0.0
            };
            sim.values[i] = (sim.values[i] + drift + load_effect as f64)
                .clamp(def.min, def.max);
        }
    }
}

/// Start the simulation loop as a background tokio task.
/// Returns a handle to stop it and shared values for reading.
pub fn start_simulation(
    mut sim: Simulation,
    slave_state: Arc<std::sync::Mutex<SlaveState>>,
    interval_ms: u64,
) -> (Arc<AtomicBool>, Arc<std::sync::Mutex<Vec<f64>>>) {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let shared_values = Arc::new(std::sync::Mutex::new(sim.values.clone()));

    // Write initial values immediately
    sim.write_to_slave(&slave_state);

    let sv = shared_values.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        while running_clone.load(Ordering::Relaxed) {
            interval.tick().await;
            tick_values(&mut sim);
            sim.write_to_slave(&slave_state);
            if let Ok(mut v) = sv.lock() {
                *v = sim.values.clone();
            }
        }
    });

    (running, shared_values)
}

/// Stop a running simulation.
pub fn stop_simulation(running: &Arc<AtomicBool>) {
    running.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::SlaveState;

    fn make_profile(registers: Vec<SimRegisterDef>) -> SimDeviceProfile {
        SimDeviceProfile {
            id: "test".into(),
            manufacturer: "Test".into(),
            model: "Test".into(),
            description: "Test profile".into(),
            base_address: 0,
            registers,
        }
    }

    fn float_reg(name: &str, initial: f64, min: f64, max: f64, drift: f64) -> SimRegisterDef {
        SimRegisterDef {
            name: name.into(),
            unit: "V".into(),
            description: String::new(),
            initial,
            min,
            max,
            drift,
            is_counter: false,
            counter_rate: 0.0,
            locked: false,
        }
    }

    fn counter_reg(name: &str, initial: f64, rate: f64) -> SimRegisterDef {
        SimRegisterDef {
            name: name.into(),
            unit: "kWh".into(),
            description: String::new(),
            initial,
            min: 0.0,
            max: 999999.0,
            drift: 0.0,
            is_counter: true,
            counter_rate: rate,
            locked: false,
        }
    }

    #[test]
    fn test_simulation_new_initializes_from_profile() {
        let profile = make_profile(vec![
            float_reg("v1", 100.0, 0.0, 200.0, 1.0),
            float_reg("v2", 50.0, 0.0, 100.0, 0.5),
        ]);
        let sim = Simulation::new(profile);
        assert_eq!(sim.values, vec![100.0, 50.0]);
    }

    #[test]
    fn test_tick_counter_increments() {
        let profile = make_profile(vec![
            counter_reg("energy", 1000.0, 10.0),
        ]);
        let mut sim = Simulation::new(profile);
        let before = sim.values[0];
        tick_values(&mut sim);
        // Counter should increase by roughly counterRate
        assert!(sim.values[0] > before);
        assert!((sim.values[0] - before) > 5.0); // at least counterRate - some variation
    }

    #[test]
    fn test_tick_locked_register_unchanged() {
        let mut locked_reg = float_reg("locked", 50.0, 0.0, 100.0, 5.0);
        locked_reg.locked = true;
        let profile = make_profile(vec![locked_reg]);
        let mut sim = Simulation::new(profile);
        tick_values(&mut sim);
        assert_eq!(sim.values[0], 50.0);
    }

    #[test]
    fn test_tick_non_counter_stays_in_bounds() {
        let profile = make_profile(vec![
            float_reg("v", 50.0, 40.0, 60.0, 100.0), // huge drift
        ]);
        let mut sim = Simulation::new(profile);
        for _ in 0..100 {
            tick_values(&mut sim);
            assert!(sim.values[0] >= 40.0 && sim.values[0] <= 60.0,
                "value {} out of bounds [{}, {}]", sim.values[0], 40.0, 60.0);
        }
    }

    #[test]
    fn test_write_to_slave_float32() {
        let profile = make_profile(vec![
            float_reg("v", 220.5, 0.0, 300.0, 0.0),
        ]);
        let mut sim = Simulation::new(profile);
        sim.values[0] = 220.5;
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0u16; 10],
        }));
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        // 220.5f32 = 0x435C8000
        let bits = 220.5f32.to_bits();
        assert_eq!(data.registers[0], (bits >> 16) as u16); // 0x435C
        assert_eq!(data.registers[1], (bits & 0xFFFF) as u16); // 0x8000
    }

    #[test]
    fn test_write_to_slave_counter() {
        let profile = make_profile(vec![
            counter_reg("energy", 0.0, 10.0),
        ]);
        let mut sim = Simulation::new(profile);
        sim.values[0] = 1000000.0;
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0u16; 10],
        }));
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        // 1000000 as u64 -> hi32 = 0, lo32 = 0x000F4240
        // registers: hi16(hi32), lo16(hi32), hi16(lo32), lo16(lo32)
        assert_eq!(data.registers[0], 0);        // hi16(hi32) = 0
        assert_eq!(data.registers[1], 0);        // lo16(hi32) = 0
        assert_eq!(data.registers[2], 0x000F);   // hi16(lo32) = 0x000F
        assert_eq!(data.registers[3], 0x4240);   // lo16(lo32) = 0x4240
    }

    #[test]
    fn test_write_to_slave_no_panic_on_short_registers() {
        let mut profile = make_profile(vec![
            float_reg("v", 220.5, 0.0, 300.0, 0.0),
        ]);
        profile.base_address = 100; // past the SlaveState length
        let sim = Simulation::new(profile);
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0u16; 10], // only 10 registers, base is 100
        }));
        // Should not panic
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        // All registers should be unchanged
        assert!(data.registers.iter().all(|&r| r == 0));
    }

    #[test]
    fn test_num_registers_mixed() {
        let profile = make_profile(vec![
            float_reg("v1", 0.0, 0.0, 100.0, 1.0),   // 2 registers
            float_reg("v2", 0.0, 0.0, 100.0, 1.0),   // 2 registers
            counter_reg("energy", 0.0, 10.0),          // 4 registers
        ]);
        let sim = Simulation::new(profile);
        assert_eq!(sim.num_registers(), 8);
    }

    // ── Expanded coverage ──

    #[test]
    fn test_write_to_slave_with_base_address() {
        let mut profile = make_profile(vec![
            float_reg("v", 100.0, 0.0, 200.0, 0.0),
        ]);
        profile.base_address = 5;
        let mut sim = Simulation::new(profile);
        sim.values[0] = 100.0;
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0u16; 20],
        }));
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        // First 5 registers should be untouched
        assert!(data.registers[..5].iter().all(|&r| r == 0));
        // Registers 5-6 should contain 100.0f32
        let bits = 100.0f32.to_bits();
        assert_eq!(data.registers[5], (bits >> 16) as u16);
        assert_eq!(data.registers[6], (bits & 0xFFFF) as u16);
    }

    #[test]
    fn test_write_to_slave_float32_zero() {
        let profile = make_profile(vec![float_reg("v", 0.0, 0.0, 100.0, 0.0)]);
        let sim = Simulation::new(profile);
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0xFFFF; 10],
        }));
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        assert_eq!(data.registers[0], 0);
        assert_eq!(data.registers[1], 0);
    }

    #[test]
    fn test_write_to_slave_counter_zero() {
        let profile = make_profile(vec![counter_reg("energy", 0.0, 10.0)]);
        let mut sim = Simulation::new(profile);
        sim.values[0] = 0.0;
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0xFFFF; 10],
        }));
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        assert_eq!(data.registers[0], 0);
        assert_eq!(data.registers[1], 0);
        assert_eq!(data.registers[2], 0);
        assert_eq!(data.registers[3], 0);
    }

    #[test]
    fn test_tick_load_factor_advances() {
        let profile = make_profile(vec![float_reg("v", 50.0, 0.0, 100.0, 0.0)]);
        let mut sim = Simulation::new(profile);
        let initial = sim.load_factor;
        for _ in 0..10 {
            tick_values(&mut sim);
        }
        // load_factor should have changed from initial
        assert_ne!(sim.load_factor, initial);
    }

    #[test]
    fn test_tick_mixed_registers() {
        let profile = make_profile(vec![
            float_reg("v", 50.0, 0.0, 100.0, 1.0),
            counter_reg("energy", 1000.0, 10.0),
        ]);
        let mut sim = Simulation::new(profile);
        let counter_before = sim.values[1];
        tick_values(&mut sim);
        // Float register stays in bounds
        assert!(sim.values[0] >= 0.0 && sim.values[0] <= 100.0);
        // Counter incremented
        assert!(sim.values[1] > counter_before);
    }

    #[test]
    fn test_write_to_slave_multiple_registers() {
        let profile = make_profile(vec![
            float_reg("v1", 100.0, 0.0, 200.0, 0.0),
            float_reg("v2", 200.0, 0.0, 300.0, 0.0),
        ]);
        let mut sim = Simulation::new(profile);
        sim.values[0] = 100.0;
        sim.values[1] = 200.0;
        let state = Arc::new(std::sync::Mutex::new(SlaveState {
            registers: vec![0u16; 10],
        }));
        sim.write_to_slave(&state);
        let data = state.lock().unwrap();
        // v1 occupies regs 0-1, v2 occupies regs 2-3
        let bits1 = 100.0f32.to_bits();
        assert_eq!(data.registers[0], (bits1 >> 16) as u16);
        assert_eq!(data.registers[1], (bits1 & 0xFFFF) as u16);
        let bits2 = 200.0f32.to_bits();
        assert_eq!(data.registers[2], (bits2 >> 16) as u16);
        assert_eq!(data.registers[3], (bits2 & 0xFFFF) as u16);
    }
}
