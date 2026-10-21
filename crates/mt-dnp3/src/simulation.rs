use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::server::OutstationState;
use crate::types::SimDnp3Profile;

/// Running simulation state.
pub struct Dnp3Simulation {
    pub profile: SimDnp3Profile,
    pub values: Vec<f64>,
}

impl Dnp3Simulation {
    pub fn new(profile: SimDnp3Profile) -> Self {
        let values = profile.points.iter().map(|p| p.base).collect();
        Self { profile, values }
    }

    pub fn write_to_outstation(&self, state: &Arc<std::sync::Mutex<OutstationState>>) {
        let mut st = state.lock().unwrap();

        for (i, p) in self.profile.points.iter().enumerate() {
            let idx = p.index as usize;
            if p.is_bool {
                if idx < st.binary_values.len() {
                    st.binary_values[idx] = self.values[i] >= 0.5;
                }
            } else if p.group == 20 {
                // Counter input
                if idx < st.counter_values.len() {
                    st.counter_values[idx] = self.values[i];
                }
            } else {
                if idx < st.analog_values.len() {
                    st.analog_values[idx] = self.values[i];
                }
            }
        }

        // Buffer any value changes as events (class assignment inside).
        st.detect_events();
    }
}

/// Tick all simulation values forward one step.
pub fn tick_values(sim: &mut Dnp3Simulation, elapsed_ms: u64) {
    for (i, p) in sim.profile.points.iter().enumerate() {
        if p.is_bool {
            continue;
        }

        let t = elapsed_ms as f64;

        if p.group == 20 || p.group == 21 {
            // Counter inputs accumulate pulses over time.
            let rate = p.variance.max(0.1); // counts per period
            let increment = rate * (t / p.period_ms as f64).fract().abs();
            let mut v = sim.values[i] + increment.max(0.001);
            if let Some(max) = p.max {
                if v > max {
                    v = p.base;
                }
            }
            sim.values[i] = v;
            continue;
        }

        let variation = (t / p.period_ms as f64 * std::f64::consts::PI * 2.0).sin() * p.variance;
        let noise = (rand_f64() - 0.5) * p.noise;
        let mut v = sim.values[i] + variation * 0.01 + noise;

        // Slowly drift back towards base
        v = v * 0.99 + p.base * 0.01;

        if let Some(min) = p.min {
            v = v.max(min);
        }
        if let Some(max) = p.max {
            v = v.min(max);
        }

        sim.values[i] = v;
    }
}

fn rand_f64() -> f64 {
    use std::cell::Cell;
    thread_local! {
        static RNG: Cell<u32> = const { Cell::new(0xDEAD_BEEF) };
    }
    RNG.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        s.set(x);
        x as f64 / u32::MAX as f64
    })
}

/// Start the simulation loop as a background tokio task.
pub fn start_simulation(
    mut sim: Dnp3Simulation,
    outstation_state: Arc<std::sync::Mutex<OutstationState>>,
    interval_ms: u64,
) -> (Arc<AtomicBool>, Arc<std::sync::Mutex<Vec<f64>>>) {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let shared_values = Arc::new(std::sync::Mutex::new(sim.values.clone()));

    // Write initial values
    sim.write_to_outstation(&outstation_state);

    let sv = shared_values.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        let mut elapsed: u64 = 0;
        while running_clone.load(Ordering::Relaxed) {
            interval.tick().await;
            elapsed += interval_ms;
            tick_values(&mut sim, elapsed);
            sim.write_to_outstation(&outstation_state);
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
