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
#[cfg(feature = "ide")]
pub mod ide;

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
    let bytecode =
        bincode::serialize(&module).context("encoding module for the runtime service")?;
    let config_json = format!(
        r#"{{"scan_cycle_ms": {}, "max_cycle_ms": {}}}"#,
        scan_cycle_ms.max(1),
        scan_cycle_ms.saturating_mul(100).max(1000),
    );
    let server = Arc::new(PlcRuntimeGrpcServer::new());
    let response = server
        .deploy(Request::new(DeployRequest {
            bytecode,
            config_json,
        }))
        .await
        .context("deploying bundle into the runtime service")?
        .into_inner();
    anyhow::ensure!(
        response.success,
        "deploy rejected: {}",
        response.error_message
    );
    Ok(server)
}

/// Start the scan loop — the same `Start` RPC the IDE issues.
pub async fn start_scan(server: &PlcRuntimeGrpcServer) -> Result<()> {
    server
        .start(Request::new(StartConfig {
            config_json: String::new(),
        }))
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
///
/// Redeploys are observed through the server's deploy events: when the
/// deployed variable catalog changed, the publisher is reborn with a fresh
/// catalog (Rebirth semantics — seq resets, bdSeq unchanged); otherwise only
/// the value snapshot is refreshed. A lagged event stream is treated the
/// same — the live catalog read is authoritative.
pub async fn run_sparkplug(
    server: Arc<PlcRuntimeGrpcServer>,
    mut cache: Arc<bridge::VariableCache>,
    settings: SparkplugSettings,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let client_id = format!("voltrus-runtime-{}", settings.edge_node_id);
    let tick = std::time::Duration::from_millis(settings.scan_cycle_ms.max(1));
    let mut deploy_events = server.deploy_events();
    // Catalog of the last birth; a redeploy that changes it triggers rebirth.
    // Assigned on the first successful connect before any read.
    let mut birthed_names: Vec<String>;

    loop {
        let transport = match RumqttcTransport::new(&settings.broker_url, client_id.clone()) {
            Ok(transport) => transport,
            Err(e) => {
                tracing::warn!(
                    "invalid Sparkplug broker URL {:?}: {e}",
                    settings.broker_url
                );
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
                birthed_names = cache.names().to_vec();
                tracing::info!(
                    "Sparkplug session established (group {}, node {}, {} metrics)",
                    settings.group_id,
                    settings.edge_node_id,
                    cache.names().len()
                );
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => {
                            let _ = publisher.death(now_ms()).await;
                            tracing::info!("Sparkplug session closed (NDEATH published)");
                            return;
                        }
                        event = deploy_events.recv() => {
                            if event.is_err() {
                                tracing::warn!("deploy event stream lagged; re-reading catalog");
                            }
                            let names = server.variable_names().await;
                            if names.is_empty() || names == birthed_names {
                                // Same program (or nothing deployed): refresh values only.
                                let snapshot = server.variable_snapshot(cache.names()).await;
                                cache.refresh(snapshot);
                                continue;
                            }
                            tracing::info!(
                                "redeploy detected ({} → {} variables): rebirthing with fresh catalog",
                                birthed_names.len(),
                                names.len()
                            );
                            birthed_names = names;
                            cache = Arc::new(bridge::VariableCache::new(birthed_names.clone()));
                            let snapshot = server.variable_snapshot(cache.names()).await;
                            cache.refresh(snapshot);
                            if let Err(e) = publisher.rebirth_with(cache.metric_sources(), now_ms()).await {
                                tracing::warn!("Sparkplug rebirth failed, reconnecting: {e}");
                                break;
                            }
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
    use bridge::VariableCache;
    use mt_plc_types::{Instruction, Opcode, PlcValue};
    use mt_sparkplug::MetricValue;

    /// The acceptance chain: trivial program → real VLT1 bytes → bundle load
    /// → in-process boot → VM variables → Sparkplug metric sampling.
    #[tokio::test]
    async fn bundle_loads_boots_and_feeds_metric_sampling() {
        let module = CompiledModule {
            name: "Pump".into(),
            instructions: vec![
                Instruction {
                    opcode: Opcode::PushBool,
                    operand: Some(1),
                },
                Instruction {
                    opcode: Opcode::StoreVar,
                    operand: Some(0),
                },
                Instruction {
                    opcode: Opcode::Halt,
                    operand: None,
                },
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
            .variable_snapshot(&[
                "pump".to_string(),
                "target".to_string(),
                "missing".to_string(),
            ])
            .await;
        assert_eq!(
            snapshot,
            vec![Some(PlcValue::Bool(false)), Some(PlcValue::Dint(40)), None,]
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
            instructions: vec![Instruction {
                opcode: Opcode::Halt,
                operand: None,
            }],
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

    /// Transport recording publishes, for driving the publisher without a
    /// broker (the crate's own mock is private to its tests).
    struct RecordingTransport {
        publishes: Arc<parking_lot::Mutex<Vec<(String, Vec<u8>)>>>,
    }

    impl mt_sparkplug::MqttTransport for RecordingTransport {
        async fn connect(
            &mut self,
            _will: mt_sparkplug::Will,
        ) -> Result<(), mt_sparkplug::SparkplugError> {
            Ok(())
        }

        async fn publish(
            &mut self,
            topic: &str,
            payload: &[u8],
        ) -> Result<(), mt_sparkplug::SparkplugError> {
            self.publishes
                .lock()
                .push((topic.to_string(), payload.to_vec()));
            Ok(())
        }
    }

    fn module_with_variables(variables: Vec<(&str, PlcValue)>) -> CompiledModule {
        CompiledModule {
            name: "Main".into(),
            instructions: vec![Instruction {
                opcode: Opcode::Halt,
                operand: None,
            }],
            variable_slots: variables
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        }
    }

    /// The GW-4 acceptance chain: boot v1, NDATA flowing, redeploy v2 with an
    /// extra variable through the server's deploy path, deploy event fires,
    /// rebirth republishes NBIRTH (seq reset, same bdSeq) including the new
    /// metric, and NDATA continues from the reset.
    #[tokio::test]
    async fn redeploy_rebirths_sparkplug_with_new_catalog() {
        use mt_grpc::pb::{DeployRequest, ForceRequest, VariableValue, variable_value};
        use mt_plc_types::crc32_le;
        use prost::Message as _;

        let server = boot(
            module_with_variables(vec![
                ("pump", PlcValue::Dint(1)),
                ("target", PlcValue::Dint(0)),
            ]),
            10,
        )
        .await
        .expect("boot v1");
        let names_v1 = server.variable_names().await;
        assert_eq!(names_v1, vec!["pump".to_string(), "target".to_string()]);

        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(VariableCache::new(names_v1));
        cache.refresh(server.variable_snapshot(cache.names()).await);
        let recorded = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut publisher = SparkplugPublisher::new(
            SparkplugConfig {
                broker_url: String::new(),
                group_id: "plant".into(),
                edge_node_id: "gw1".into(),
                bd_seq_path: dir.path().join("bd_seq"),
                metrics: cache.metric_sources(),
                deadband_by_prefix: vec![],
                max_interval_ms: 60_000,
            },
            RecordingTransport {
                publishes: recorded.clone(),
            },
        );
        publisher.connect(1_000).await.unwrap();

        // NDATA flowing: force pump 1 → 5 through the IDE force path.
        server
            .force_variable(Request::new(ForceRequest {
                variable_name: "pump".to_string(),
                value: Some(VariableValue {
                    value: Some(variable_value::Value::DintVal(5)),
                }),
            }))
            .await
            .unwrap();
        cache.refresh(server.variable_snapshot(cache.names()).await);
        assert!(publisher.publish_changes(1_500).await.unwrap());

        // Redeploy v2 (extra variable) through the same deploy handler the
        // IDE uses; the deploy event must fire with the new fingerprint.
        let mut events = server.deploy_events();
        let module_v2 = module_with_variables(vec![
            ("pump", PlcValue::Dint(1)),
            ("target", PlcValue::Dint(0)),
            ("extra", PlcValue::Dint(0)),
        ]);
        let v2_bytes = bincode::serialize(&module_v2).unwrap();
        server
            .deploy(Request::new(DeployRequest {
                bytecode: v2_bytes.clone(),
                config_json: String::new(),
            }))
            .await
            .unwrap();
        let event = events.recv().await.expect("deploy event");
        assert_eq!(event.deployed_bundle_crc32, crc32_le(&v2_bytes));
        assert_eq!(
            server.variable_names().await,
            vec![
                "pump".to_string(),
                "target".to_string(),
                "extra".to_string()
            ]
        );

        // The bridge's event handling: catalog changed → rebirth with a
        // fresh catalog.
        let new_cache = Arc::new(VariableCache::new(server.variable_names().await));
        new_cache.refresh(server.variable_snapshot(new_cache.names()).await);
        publisher
            .rebirth_with(new_cache.metric_sources(), 2_000)
            .await
            .unwrap();

        let payloads = recorded.lock().clone();
        let births = payloads
            .iter()
            .filter(|(topic, _)| topic.ends_with("/NBIRTH/gw1"))
            .map(|(_, bytes)| mt_sparkplug::Payload::decode(bytes.as_slice()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(births.len(), 2);
        assert_eq!(births[1].seq, Some(0), "rebirth resets seq");
        let bd_seq = |payload: &mt_sparkplug::Payload| {
            payload
                .metrics
                .iter()
                .find(|m| m.name.as_deref() == Some("bdSeq"))
                .map(|m| m.value.clone())
        };
        assert_eq!(
            bd_seq(&births[0]),
            bd_seq(&births[1]),
            "same session: bdSeq unchanged"
        );
        let extra = births[1]
            .metrics
            .iter()
            .find(|m| m.name.as_deref() == Some("extra"))
            .and_then(|m| m.value.clone());
        assert_eq!(
            extra,
            Some(mt_sparkplug::pb::payload::metric::Value::LongValue(0))
        );

        // NDATA continues from the reset: force extra 0 → 7.
        server
            .force_variable(Request::new(ForceRequest {
                variable_name: "extra".to_string(),
                value: Some(VariableValue {
                    value: Some(variable_value::Value::DintVal(7)),
                }),
            }))
            .await
            .unwrap();
        new_cache.refresh(server.variable_snapshot(new_cache.names()).await);
        assert!(publisher.publish_changes(2_500).await.unwrap());
        // Read the live recordings: `payloads` was cloned before this publish.
        let datas = recorded
            .lock()
            .iter()
            .filter(|(topic, _)| topic.ends_with("/NDATA/gw1"))
            .map(|(_, bytes)| mt_sparkplug::Payload::decode(bytes.as_slice()).unwrap())
            .collect::<Vec<_>>();
        let last = datas.last().expect("post-rebirth NDATA");
        assert_eq!(last.seq, Some(1), "NDATA continues from the reset");
        assert_eq!(last.metrics.len(), 1);
        assert_eq!(last.metrics[0].name.as_deref(), Some("extra"));
        assert_eq!(
            last.metrics[0].value,
            Some(mt_sparkplug::pb::payload::metric::Value::LongValue(7))
        );
    }
}
