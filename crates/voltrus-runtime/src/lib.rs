//! voltrus-runtime: standalone Linux runtime for Voltrus PLC programs.
//!
//! Boots a VLT1 bytecode bundle into the same `mt-grpc` runtime service the
//! IDE drives (deploy / watch / force behave unchanged), runs its scan loop,
//! and optionally publishes the VM's variables as Sparkplug B metrics with
//! report-by-exception.
//!
//! Composition lives here so `main.rs` stays a thin CLI shell; every step is
//! exercised in-process by the tests.

pub mod bridge;
pub mod bundle;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use mt_grpc::pb::plc_runtime_server::PlcRuntime as PlcRuntimeApi;
use mt_grpc::pb::{DeployRequest, StartConfig};
use mt_grpc::server::PlcRuntimeGrpcServer;
use mt_plc_types::CompiledModule;
use mt_sparkplug::{RumqttcTransport, SparkplugConfig, SparkplugPublisher};
use tonic::Request;

/// Deploy `module` into a fresh gRPC runtime server.
///
/// The IDE deploy path consumes bincode-encoded `CompiledModule` bytes plus a
/// JSON `RuntimeConfig` override; booting through the very same handler keeps
/// one deploy code path for the IDE and the headless runtime (module
/// decoding, config overrides, fingerprint bookkeeping).
///
/// `max_cycle_ms` watchdog is set to 100x the scan cycle (floor 1000 ms).
pub async fn boot(module: CompiledModule, scan_cycle_ms: u64) -> Result<Arc<PlcRuntimeGrpcServer>> {
    let bytecode = bincode::serialize(&module).context("encoding module for the runtime service")?;
    let config_json = format!(
        r#"{{"scan_cycle_ms": {}, "max_cycle_ms": {}}}"#,
        scan_cycle_ms.max(1),
        scan_cycle_ms.saturating_mul(100).max(1000),
    );
    let server = Arc::new(PlcRuntimeGrpcServer::new());
    let response = server
        .deploy(Request::new(DeployRequest { bytecode, config_json }))
        .await
        .context("deploying bundle into the runtime service")?
        .into_inner();
    anyhow::ensure!(response.success, "deploy rejected: {}", response.error_message);
    Ok(server)
}

/// Start the scan loop — the same `Start` RPC the IDE issues.
pub async fn start_scan(server: &PlcRuntimeGrpcServer) -> Result<()> {
    server
        .start(Request::new(StartConfig { config_json: String::new() }))
        .await
        .context("starting the scan loop")?;
    Ok(())
}

/// Sparkplug B publishing configuration for this binary.
#[derive(Debug, Clone)]
pub struct SparkplugSettings {
    /// Broker address, `tcp://host:port`.
    pub broker_url: String,
    pub group_id: String,
    pub edge_node_id: String,
    /// File persisting the bdSeq counter across restarts.
    pub bd_seq_path: PathBuf,
    /// Sampling cadence; also the RBE evaluation cadence.
    pub scan_cycle_ms: u64,
}

/// Run the Sparkplug publisher until `shutdown` fires.
///
/// Each tick: sample the server's variables into `cache`, then evaluate RBE
/// through the publisher. Broker loss is recovered by opening a fresh
/// session (bdSeq bump + NBIRTH); on shutdown the graceful NDEATH is
/// published first.
pub async fn run_sparkplug(
    server: Arc<PlcRuntimeGrpcServer>,
    cache: Arc<bridge::VariableCache>,
    settings: SparkplugSettings,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let client_id = format!("voltrus-runtime-{}", settings.edge_node_id);
    let tick = std::time::Duration::from_millis(settings.scan_cycle_ms.max(1));

    loop {
        let transport = match RumqttcTransport::new(&settings.broker_url, client_id.clone()) {
            Ok(transport) => transport,
            Err(e) => {
                tracing::warn!("invalid Sparkplug broker URL {:?}: {e}", settings.broker_url);
                tokio::time::sleep(tick).await;
                continue;
            }
        };
        let mut publisher = SparkplugPublisher::new(
            SparkplugConfig {
                broker_url: settings.broker_url.clone(),
                group_id: settings.group_id.clone(),
                edge_node_id: settings.edge_node_id.clone(),
                bd_seq_path: settings.bd_seq_path.clone(),
                metrics: cache.metric_sources(),
                deadband_by_prefix: Vec::new(),
                max_interval_ms: 60_000,
            },
            transport,
        );

        // Sample the VM before birthing: the NBIRTH must carry real values,
        // and a metric absent at birth stays unpublished for the session.
        let snapshot = server.variable_snapshot(cache.names()).await;
        cache.refresh(snapshot);

        match publisher.connect(now_ms()).await {
            Ok(()) => {
                tracing::info!(
                    "Sparkplug session established (group {}, node {})",
                    settings.group_id,
                    settings.edge_node_id
                );
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => {
                            let _ = publisher.death(now_ms()).await;
                            tracing::info!("Sparkplug session closed (NDEATH published)");
                            return;
                        }
                        _ = tokio::time::sleep(tick) => {
                            let snapshot = server.variable_snapshot(cache.names()).await;
                            cache.refresh(snapshot);
                            if let Err(e) = publisher.publish_changes(now_ms()).await {
                                tracing::warn!("Sparkplug publish failed, reconnecting: {e}");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("Sparkplug connect to {} failed: {e}", settings.broker_url),
        }

        // Session lost or never established: back off, then try a fresh one.
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_plc_types::{Instruction, Opcode, PlcValue};
    use mt_sparkplug::MetricValue;

    /// The acceptance chain: trivial program → real VLT1 bytes → bundle load
    /// → in-process boot → VM variables → Sparkplug metric sampling.
    #[tokio::test]
    async fn bundle_loads_boots_and_feeds_metric_sampling() {
        let module = CompiledModule {
            name: "Pump".into(),
            instructions: vec![
                Instruction { opcode: Opcode::PushBool, operand: Some(1) },
                Instruction { opcode: Opcode::StoreVar, operand: Some(0) },
                Instruction { opcode: Opcode::Halt, operand: None },
            ],
            variable_slots: vec![
                ("pump".to_string(), PlcValue::Bool(false)),
                ("target".to_string(), PlcValue::Dint(40)),
            ],
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        };
        let bytes = mt_plc_types::serialize_bytecode(&module, &[]).unwrap();
        let bundle = bundle::load_vlt1(&bytes).expect("valid bundle");

        let server = boot(bundle.module, 10).await.expect("boot");
        let snapshot = server
            .variable_snapshot(&["pump".to_string(), "target".to_string(), "missing".to_string()])
            .await;
        assert_eq!(
            snapshot,
            vec![
                Some(PlcValue::Bool(false)),
                Some(PlcValue::Dint(40)),
                None,
            ]
        );

        let cache = Arc::new(bridge::VariableCache::new(vec![
            "pump".to_string(),
            "target".to_string(),
            "missing".to_string(),
        ]));
        cache.refresh(server.variable_snapshot(cache.names()).await);
        let sources = cache.metric_sources();
        assert_eq!((sources[0].get)(), Some(MetricValue::Bool(false)));
        assert_eq!((sources[1].get)(), Some(MetricValue::Int(40)));
        assert_eq!((sources[2].get)(), None);
    }

    #[tokio::test]
    async fn start_scan_transitions_to_running() {
        let module = CompiledModule {
            name: "Main".into(),
            instructions: vec![Instruction { opcode: Opcode::Halt, operand: None }],
            variable_slots: vec![],
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        };
        let server = boot(module, 10).await.expect("boot");
        start_scan(&server).await.expect("start");
        let state = server
            .get_runtime_state(Request::new(mt_grpc::pb::Empty {}))
            .await
            .expect("state");
        let state = state.into_inner();
        assert_eq!(state.status, mt_grpc::pb::RuntimeStatus::Running as i32);
    }
}
