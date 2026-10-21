use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::interval;

use crate::store::OpcUaStore;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimRegisterDef {
    pub node_id: String,
    pub name: String,
    pub data_type: String,
    pub base: f64,
    pub variance: f64,
    pub noise: f64,
    pub period: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub is_bool: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDeviceProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub registers: Vec<SimRegisterDef>,
}

pub struct Simulation {
    pub profile: SimDeviceProfile,
    pub values: Vec<f64>,
    /// Writes a node value through the server node manager so active
    /// subscriptions receive data-change notifications.
    writer: Option<Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>>,
}

impl Simulation {
    pub fn new(profile: SimDeviceProfile) -> Self {
        let values = profile.registers.iter().map(|r| r.base).collect();
        Self { profile, values, writer: None }
    }

    pub fn with_writer(
        profile: SimDeviceProfile,
        writer: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>,
    ) -> Self {
        let values = profile.registers.iter().map(|r| r.base).collect();
        Self { profile, values, writer: Some(writer) }
    }

    pub fn tick_values(&mut self, t_ms: f64) {
        for (i, reg) in self.profile.registers.iter().enumerate() {
            if reg.is_bool {
                self.values[i] = if reg.base >= 0.5 { 1.0 } else { 0.0 };
            } else {
                let mut v = reg.base
                    + ((t_ms / reg.period) * std::f64::consts::PI * 2.0).sin() * reg.variance
                    + (xorshift32_f64() - 0.5) * reg.noise;
                if let Some(min) = reg.min {
                    v = v.max(min);
                }
                if let Some(max) = reg.max {
                    v = v.min(max);
                }
                self.values[i] = v;
            }
        }
    }

    pub fn write_to_store(&self, store: &OpcUaStore) {
        for (i, reg) in self.profile.registers.iter().enumerate() {
            let v = if reg.is_bool {
                serde_json::Value::Bool(self.values[i] >= 0.5)
            } else {
                serde_json::json!(self.values[i])
            };

            if let Ok(mut nodes) = store.nodes.try_write() {
                if let Some(existing) = nodes.get_mut(&reg.node_id) {
                    existing.value = v;
                    existing.timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                }
            }

            // Write through the server node manager so subscriptions are
            // notified of the data change (direct address-space mutation
            // bypasses the subscription cache and values appear static).
            if let Some(writer) = &self.writer {
                let value = if reg.is_bool {
                    serde_json::json!(self.values[i] >= 0.5)
                } else {
                    serde_json::json!(self.values[i])
                };
                writer(&reg.node_id, value);
            }
        }
    }
}

pub async fn start_simulation(
    store: OpcUaStore,
    profile: SimDeviceProfile,
    interval_ms: u64,
    running: Arc<AtomicBool>,
    sim_values: Arc<Mutex<Vec<f64>>>,
    writer: Option<Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>>,
) {
    let mut sim = if let Some(w) = writer {
        Simulation::with_writer(profile, w)
    } else {
        Simulation::new(profile)
    };

    for reg in &sim.profile.registers {
        let v = if reg.is_bool {
            serde_json::Value::Bool(reg.base >= 0.5)
        } else {
            serde_json::json!(reg.base)
        };
        store.init_node(&reg.node_id, &reg.name, &reg.data_type, v).await;
    }

    running.store(true, Ordering::SeqCst);
    let start = Instant::now();

    tokio::spawn(async move {
        let mut tick = interval(Duration::from_millis(interval_ms));

        loop {
            tick.tick().await;
            if !running.load(Ordering::SeqCst) {
                break;
            }

            let t_ms = start.elapsed().as_millis() as f64;
            sim.tick_values(t_ms);
            sim.write_to_store(&store);

            if let Ok(mut vals) = sim_values.try_lock() {
                *vals = sim.values.clone();
            }
        }
    });
}

fn xorshift32_f64() -> f64 {
    thread_local! {
        static STATE: std::cell::Cell<u32> = const { std::cell::Cell::new(0xDEAD_BEEF) };
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        s.set(x);
        x as f64 / u32::MAX as f64
    })
}
