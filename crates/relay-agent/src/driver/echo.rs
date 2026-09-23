//! `echo` driver — an in-agent simulated PLC. Accepts every op, so the agent
//! can be demoed and tested without hardware.

use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::{DeviceDriver, Subscription};
use crate::protocol::{EventFrame, VarInfo};

const TICK: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
struct EchoVar {
    name: &'static str,
    type_name: &'static str,
    value: Value,
    forced: Option<Value>,
}

/// What the last accepted deploy left behind: payload size plus a
/// deterministic fingerprint of the module JSON bytes.
#[derive(Debug, Clone)]
struct EchoDeploy {
    bytes: usize,
    crc32: u32,
}

/// FNV-1a (32-bit): the deploy fingerprint the echo driver reports. It only
/// needs to be deterministic so an ide can compare deploy vs status payloads.
fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Simulated device with a few fake variables; `cycle_count` advances on its
/// own so subscribers see movement.
#[derive(Debug)]
pub struct EchoDriver {
    vars: Vec<EchoVar>,
    deployed: Option<EchoDeploy>,
    ticker: Option<JoinHandle<()>>,
    started_at: std::time::Instant,
}

impl Default for EchoDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoDriver {
    pub fn new() -> Self {
        EchoDriver {
            vars: vec![
                EchoVar { name: "motor_running", type_name: "BOOL", value: Value::Bool(false), forced: None },
                EchoVar { name: "cycle_count", type_name: "DINT", value: json!(0), forced: None },
                EchoVar { name: "temperature", type_name: "REAL", value: json!(22.5), forced: None },
                EchoVar { name: "part_id", type_name: "STRING", value: json!("PART-0001"), forced: None },
            ],
            deployed: None,
            ticker: None,
            started_at: std::time::Instant::now(),
        }
    }

    fn index_of(&self, name: &str) -> Result<usize> {
        self.vars
            .iter()
            .position(|v| v.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown variable: {name}"))
    }

    fn snapshot(&self) -> Vec<VarInfo> {
        self.vars
            .iter()
            .map(|v| VarInfo {
                name: v.name.to_owned(),
                type_name: v.type_name.to_owned(),
                value: Some(v.forced.clone().unwrap_or_else(|| v.value.clone())),
                forced: Some(v.forced.is_some()),
            })
            .collect()
    }
}

impl Clone for EchoDriver {
    /// The clone starts unsubscribed; each session subscribes fresh.
    fn clone(&self) -> Self {
        EchoDriver {
            vars: self.vars.clone(),
            deployed: self.deployed.clone(),
            ticker: None,
            started_at: self.started_at,
        }
    }
}

impl Drop for EchoDriver {
    fn drop(&mut self) {
        if let Some(ticker) = self.ticker.take() {
            ticker.abort();
        }
    }
}

impl DeviceDriver for EchoDriver {
    async fn browse(&mut self) -> Result<Vec<VarInfo>> {
        Ok(self.snapshot())
    }

    async fn subscribe(&mut self, names: Subscription, tx: UnboundedSender<EventFrame>) -> Result<Value> {
        if let Some(prev) = self.ticker.take() {
            prev.abort();
        }
        let var_names: Vec<String> = if names.is_empty() {
            self.vars.iter().map(|v| v.name.to_owned()).collect()
        } else {
            for name in &names {
                self.index_of(name)?;
            }
            names
        };
        let baseline: Vec<(String, Value)> = var_names
            .iter()
            .map(|name| {
                let v = &self.vars[self.index_of(name).expect("checked above")];
                (name.clone(), v.forced.clone().unwrap_or_else(|| v.value.clone()))
            })
            .collect();

        self.ticker = Some(tokio::spawn(async move {
            let send = |name: &str, value: Value| {
                let frame = EventFrame { event: "variable_update".into(), payload: Some(json!({ "name": name, "value": value })) };
                tx.send(frame).is_ok()
            };
            // Baseline snapshot first, then only real changes per tick.
            for (name, value) in &baseline {
                if !send(name, value.clone()) {
                    return; // Session gone.
                }
            }
            let mut cycle: i64 = baseline
                .iter()
                .find(|(name, _)| name == "cycle_count")
                .and_then(|(_, v)| v.as_i64())
                .unwrap_or(0);
            loop {
                tokio::time::sleep(TICK).await;
                cycle += 1;
                if !send("cycle_count", json!(cycle)) {
                    return;
                }
            }
        }));

        Ok(json!({ "subscribed": var_names, "interval_ms": TICK.as_millis() as u64 }))
    }

    async fn write(&mut self, name: &str, value: Value) -> Result<Value> {
        let idx = self.index_of(name)?;
        if self.vars[idx].forced.is_some() {
            bail!("{name} is forced; unforce before writing");
        }
        self.vars[idx].value = value;
        Ok(json!({ "name": name, "written": true }))
    }

    async fn force(&mut self, name: &str, value: Value) -> Result<()> {
        let idx = self.index_of(name)?;
        self.vars[idx].forced = Some(value);
        Ok(())
    }

    async fn unforce(&mut self, name: &str) -> Result<()> {
        let idx = self.index_of(name)?;
        self.vars[idx].forced = None;
        Ok(())
    }

    async fn status(&mut self) -> Result<Value> {
        Ok(json!({
            "driver": "echo",
            "running": true,
            "variables": self.vars.len(),
            "forced": self.vars.iter().filter(|v| v.forced.is_some()).count(),
            "deployed_bytes": self.deployed.as_ref().map(|d| d.bytes),
            "deployed_crc32": self.deployed.as_ref().map(|d| d.crc32),
            "uptime_s": self.started_at.elapsed().as_secs(),
        }))
    }

    async fn deploy(&mut self, module_json: &str, config: Option<String>) -> Result<Value> {
        if module_json.is_empty() {
            bail!("empty module_json");
        }
        // Sanity gate: the payload must at least be JSON the runtime could
        // start deserializing.
        serde_json::from_str::<Value>(module_json)
            .map_err(|e| anyhow::anyhow!("module_json is not valid JSON: {e}"))?;
        let crc32 = fnv1a32(module_json.as_bytes());
        self.deployed = Some(EchoDeploy { bytes: module_json.len(), crc32 });
        Ok(json!({ "deployed": true, "bytes": module_json.len(), "crc32": crc32, "config": config }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE_JSON: &str = r#"{"name":"Main","instructions":[],"variable_slots":[["x",{"Dint":1}]]}"#;

    #[test]
    fn deploy_crc_is_deterministic() {
        let mut driver = EchoDriver::new();
        let first = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(driver.deploy(MODULE_JSON, None))
            .expect("deploy ok");
        let second = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(driver.deploy(MODULE_JSON, None))
            .expect("deploy ok");
        assert_eq!(first["crc32"], second["crc32"]);
        assert_eq!(first["crc32"], fnv1a32(MODULE_JSON.as_bytes()));
        // Same fingerprint surfaces from status, so an ide can diff them.
        let status = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(driver.status())
            .expect("status ok");
        assert_eq!(status["deployed_crc32"], first["crc32"]);
        assert_eq!(status["deployed_bytes"], MODULE_JSON.len());
    }

    #[test]
    fn deploy_rejects_non_json() {
        let mut driver = EchoDriver::new();
        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(driver.deploy("not json", None));
        assert!(result.is_err());
    }
}
