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

/// Simulated device with a few fake variables; `cycle_count` advances on its
/// own so subscribers see movement.
#[derive(Debug)]
pub struct EchoDriver {
    vars: Vec<EchoVar>,
    deployed: Option<usize>,
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
            deployed: self.deployed,
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
            "deployed_bytes": self.deployed,
            "uptime_s": self.started_at.elapsed().as_secs(),
        }))
    }

    async fn deploy(&mut self, bytecode: &[u8], config: Option<String>) -> Result<Value> {
        if bytecode.is_empty() {
            bail!("empty bytecode bundle");
        }
        self.deployed = Some(bytecode.len());
        Ok(json!({ "deployed": true, "bytes": bytecode.len(), "config": config }))
    }
}
