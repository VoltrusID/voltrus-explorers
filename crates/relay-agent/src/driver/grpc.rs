//! `grpc` driver — bridges relay ops onto the local PLC runtime via mt-grpc.
//!
//! Mapping notes: the runtime exposes no plain-write RPC, so `write` is
//! implemented as a force (the strongest available write) and `browse` takes a
//! short watch snapshot (empty name list = "all variables" per mt-grpc docs).

use std::time::Duration;

use anyhow::{Result, bail};
use mt_grpc::client::PlcRuntimeClient;
use mt_grpc::variable_value::Value as PbValue;
use mt_grpc::{RuntimeState, VariableValue};
use mt_plc_types::{CompiledModule, crc32_le};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::{DeviceDriver, Subscription};
use crate::protocol::{EventFrame, VarInfo};

const BROWSE_IDLE: Duration = Duration::from_millis(400);
const WATCH_INTERVAL_MS: u32 = 250;

pub struct GrpcDriver {
    addr: String,
    watcher: Option<JoinHandle<()>>,
}

impl GrpcDriver {
    pub fn new(addr: impl Into<String>) -> Self {
        GrpcDriver {
            addr: addr.into(),
            watcher: None,
        }
    }

    async fn client(&self) -> Result<PlcRuntimeClient> {
        Ok(PlcRuntimeClient::connect(&self.addr).await?)
    }
}

impl Clone for GrpcDriver {
    /// The clone starts unsubscribed; each session subscribes fresh.
    fn clone(&self) -> Self {
        GrpcDriver::new(self.addr.clone())
    }
}

impl Drop for GrpcDriver {
    fn drop(&mut self) {
        if let Some(watcher) = self.watcher.take() {
            watcher.abort();
        }
    }
}

fn pb_value_to_json(value: &Option<VariableValue>) -> Option<Value> {
    let inner = match &value.as_ref()?.value {
        Some(PbValue::BoolVal(b)) => Value::Bool(*b),
        Some(PbValue::IntVal(n)) | Some(PbValue::DintVal(n)) | Some(PbValue::TimeVal(n)) => {
            json!(n)
        }
        Some(PbValue::RealVal(f)) => json!(f),
        Some(PbValue::StringVal(s)) => Value::String(s.clone()),
        Some(PbValue::WordVal(n)) | Some(PbValue::DwordVal(n)) | Some(PbValue::ByteVal(n)) => {
            json!(n)
        }
        None => return None,
    };
    Some(inner)
}

fn json_to_pb_value(value: Value) -> Result<VariableValue> {
    let inner = match value {
        Value::Bool(b) => PbValue::BoolVal(b),
        Value::String(s) => PbValue::StringVal(s),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                match i32::try_from(i) {
                    Ok(i) => PbValue::DintVal(i),
                    Err(_) => PbValue::DwordVal(u32::try_from(i)?),
                }
            } else if let Some(f) = n.as_f64() {
                PbValue::RealVal(f as f32)
            } else {
                bail!("unsupported number: {n}");
            }
        }
        other => bail!("unsupported value: {other}"),
    };
    Ok(VariableValue { value: Some(inner) })
}

fn state_to_json(state: RuntimeState) -> Value {
    json!({
        "status": mt_grpc::RuntimeStatus::try_from(state.status)
            .map(|s| s.as_str_name().trim_start_matches("RUNTIME_STATUS_").to_lowercase())
            .unwrap_or_else(|_| "unknown".into()),
        "cycle_count": state.cycle_count,
        "last_cycle_ms": state.last_cycle_ms,
        "avg_cycle_ms": state.avg_cycle_ms,
        "max_cycle_ms": state.max_cycle_ms,
        "error": if state.error_message.is_empty() { None } else { Some(state.error_message) },
        "deployed_bundle_len": state.deployed_bundle_len,
    })
}

impl DeviceDriver for GrpcDriver {
    async fn browse(&mut self) -> Result<Vec<VarInfo>> {
        let mut client = self.client().await?;
        let mut stream = client.watch_variables(vec![], 0).await?;

        let mut out: Vec<VarInfo> = Vec::new();
        loop {
            match tokio::time::timeout(BROWSE_IDLE, stream.next()).await {
                Err(_idle) => break, // No more updates within the window: snapshot complete.
                Ok(None) => break,
                Ok(Some(Err(status))) => bail!("watch failed: {status}"),
                Ok(Some(Ok(update))) => {
                    if out.iter().any(|v| v.name == update.name) {
                        continue;
                    }
                    out.push(VarInfo {
                        name: update.name,
                        type_name: update.type_name,
                        value: pb_value_to_json(&update.current),
                        forced: None,
                    });
                }
            }
        }
        Ok(out)
    }

    async fn subscribe(
        &mut self,
        names: Subscription,
        tx: UnboundedSender<EventFrame>,
    ) -> Result<Value> {
        if let Some(prev) = self.watcher.take() {
            prev.abort();
        }
        let addr = self.addr.clone();
        self.watcher = Some(tokio::spawn(async move {
            let Ok(mut client) = PlcRuntimeClient::connect(&addr).await else {
                return;
            };
            let Ok(mut stream) = client.watch_variables(names, WATCH_INTERVAL_MS).await else {
                return;
            };
            while let Some(Ok(update)) = stream.next().await {
                let frame = EventFrame {
                    event: "variable_update".into(),
                    payload: Some(json!({
                        "name": update.name,
                        "type": update.type_name,
                        "value": pb_value_to_json(&update.current),
                        "quality": update.quality,
                    })),
                };
                if tx.send(frame).is_err() {
                    break; // Session gone.
                }
            }
        }));
        Ok(json!({ "interval_ms": WATCH_INTERVAL_MS }))
    }

    async fn write(&mut self, name: &str, value: Value) -> Result<Value> {
        // No plain-write RPC in the runtime protocol; force is the closest op.
        self.force(name, value).await?;
        Ok(json!({ "name": name, "written": true, "as": "force" }))
    }

    async fn force(&mut self, name: &str, value: Value) -> Result<()> {
        let mut client = self.client().await?;
        let response = client
            .force_variable(name, json_to_pb_value(value)?)
            .await?;
        if !response.success {
            bail!(
                "runtime rejected force of {name}: {}",
                response.error_message
            );
        }
        Ok(())
    }

    async fn unforce(&mut self, name: &str) -> Result<()> {
        let mut client = self.client().await?;
        let response = client.unforce_variable(name).await?;
        if !response.success {
            bail!(
                "runtime rejected unforce of {name}: {}",
                response.error_message
            );
        }
        Ok(())
    }

    async fn status(&mut self) -> Result<Value> {
        let mut client = self.client().await?;
        let state = client.get_runtime_state().await?;
        let crc32 = state.deployed_bundle_crc32;
        let io = client.get_io_status().await?;
        Ok(json!({
            "driver": "grpc",
            "addr": self.addr,
            "runtime": state_to_json(state),
            // Flat, so ides can diff deploy vs status fingerprints without
            // knowing the driver shape.
            "deployed_crc32": crc32,
            "connections": io.connections.len(),
        }))
    }

    async fn deploy(&mut self, module_json: &str, config: Option<String>) -> Result<Value> {
        // The mt-grpc runtime's Deploy handler deserializes
        // `DeployRequest.bytecode` with bincode as a CompiledModule, so the
        // module crosses the relay as serde JSON and is bincode-serialized
        // here, agent-side (see driver/mod.rs).
        let module: CompiledModule = serde_json::from_str(module_json)
            .map_err(|e| anyhow::anyhow!("module_json is not a CompiledModule: {e}"))?;
        let bytecode = bincode::serialize(&module)
            .map_err(|e| anyhow::anyhow!("bincode serialization failed: {e}"))?;
        // Fingerprint over the exact bytes shipped to the runtime — the same
        // value the runtime records from DeployRequest.bytecode.
        let crc32 = crc32_le(&bytecode);
        let mut client = self.client().await?;
        let response = client.deploy(&bytecode, config.as_deref()).await?;
        if !response.success {
            bail!("deploy failed: {}", response.error_message);
        }
        Ok(json!({
            "deployed": true,
            "bytes": bytecode.len(),
            "crc32": crc32,
            "state": state_to_json(response.new_state.unwrap_or_default()),
        }))
    }
}
