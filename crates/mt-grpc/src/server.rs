use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

use mt_plc_runtime::io_manager::{self, IoAddress, IoDriver, IoArea};
use mt_plc_runtime::opcode::{CompiledModule, Instruction};
use mt_plc_runtime::types::{CycleStats, RuntimeConfig};
use mt_plc_runtime::{PlcRuntime, PlcValue};
use mt_plc_types::crc32_le;

use crate::pb::plc_runtime_server::{PlcRuntime as PlcRuntimeTrait, PlcRuntimeServer};
use crate::pb::{self, *};

// Alias to disambiguate the protobuf RuntimeState from mt_plc_runtime::types::RuntimeState.
type ProtoRuntimeState = pb::RuntimeState;
type DomainRuntimeState = mt_plc_runtime::types::RuntimeState;

// ── Conversions between protobuf types and domain types ──

fn runtime_status_to_pb(state: &DomainRuntimeState) -> RuntimeStatus {
    match state {
        DomainRuntimeState::Stopped => RuntimeStatus::Stopped,
        DomainRuntimeState::Running => RuntimeStatus::Running,
        DomainRuntimeState::Paused => RuntimeStatus::Paused,
        DomainRuntimeState::Error(_) => RuntimeStatus::Error,
    }
}

fn runtime_state_to_pb(inner: &RuntimeInner) -> ProtoRuntimeState {
    ProtoRuntimeState {
        status: runtime_status_to_pb(&inner.state).into(),
        cycle_count: inner.stats.cycle_count,
        last_cycle_ms: inner.stats.last_cycle_ms,
        avg_cycle_ms: inner.stats.avg_cycle_ms,
        max_cycle_ms: inner.stats.max_cycle_ms,
        error_message: match &inner.state {
            DomainRuntimeState::Error(e) => e.clone(),
            _ => String::new(),
        },
        deployed_bundle_crc32: inner.deployed_bundle_crc32,
        deployed_bundle_len: inner.deployed_bundle_len,
    }
}

fn plc_value_to_variable_value(val: &PlcValue) -> VariableValue {
    match val {
        PlcValue::Bool(b) => VariableValue {
            value: Some(variable_value::Value::BoolVal(*b)),
        },
        PlcValue::Byte(b) => VariableValue {
            value: Some(variable_value::Value::ByteVal(*b as u32)),
        },
        PlcValue::Int(i) => VariableValue {
            value: Some(variable_value::Value::IntVal(*i as i32)),
        },
        PlcValue::Dint(i) => VariableValue {
            value: Some(variable_value::Value::DintVal(*i)),
        },
        PlcValue::Real(f) => VariableValue {
            value: Some(variable_value::Value::RealVal(*f)),
        },
        PlcValue::String(s) => VariableValue {
            value: Some(variable_value::Value::StringVal(s.clone())),
        },
        PlcValue::Word(w) => VariableValue {
            value: Some(variable_value::Value::WordVal(*w as u32)),
        },
        PlcValue::Dword(d) => VariableValue {
            value: Some(variable_value::Value::DwordVal(*d)),
        },
        PlcValue::Time(t) => VariableValue {
            value: Some(variable_value::Value::TimeVal(*t)),
        },
        // Enums and complex types fall back to string representation
        PlcValue::Enum(e) => VariableValue {
            value: Some(variable_value::Value::StringVal(e.clone())),
        },
        PlcValue::Array(_) | PlcValue::Struct(_) => VariableValue {
            value: Some(variable_value::Value::StringVal(val.to_string())),
        },
    }
}

fn variable_value_to_plc(val: VariableValue) -> PlcValue {
    match val.value {
        Some(variable_value::Value::BoolVal(b)) => PlcValue::Bool(b),
        Some(variable_value::Value::ByteVal(b)) => PlcValue::Byte(b as u8),
        Some(variable_value::Value::IntVal(i)) => PlcValue::Int(i as i16),
        Some(variable_value::Value::DintVal(i)) => PlcValue::Dint(i),
        Some(variable_value::Value::RealVal(f)) => PlcValue::Real(f),
        Some(variable_value::Value::StringVal(s)) => PlcValue::String(s),
        Some(variable_value::Value::WordVal(w)) => PlcValue::Word(w as u16),
        Some(variable_value::Value::DwordVal(d)) => PlcValue::Dword(d),
        Some(variable_value::Value::TimeVal(t)) => PlcValue::Time(t),
        None => PlcValue::Dint(0),
    }
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Internal state shared between the gRPC handlers and the scan-cycle task.
struct RuntimeInner {
    runtime: Option<PlcRuntime>,
    instructions: Vec<Instruction>,
    state: DomainRuntimeState,
    stats: CycleStats,
    /// Fingerprint of the deployed bundle (drift detection, PLC-6).
    /// CRC32 is `crc32_le` over the received bytecode; both stay 0 until a
    /// Deploy succeeds.
    deployed_bundle_crc32: u32,
    deployed_bundle_len: u64,
}

/// Handle to the background scan-cycle task, used to start/stop it.
struct ScanHandle {
    cancel: watch::Sender<bool>,
}

/// Fired after a successful Deploy — see
/// [`PlcRuntimeGrpcServer::deploy_events`]. Capacity 1: a lagged receiver
/// missed deploys that a newer one supersedes anyway.
#[derive(Debug, Clone, Copy)]
pub struct DeployEvent {
    pub deployed_bundle_crc32: u32,
    pub deployed_bundle_len: u64,
}

/// The gRPC server wrapping a PLC runtime.
pub struct PlcRuntimeGrpcServer {
    inner: Arc<RwLock<RuntimeInner>>,
    scan_handle: Arc<Mutex<Option<ScanHandle>>>,
    scan_loop_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    watch_tx: broadcast::Sender<Result<VariableUpdate, Status>>,
    /// Fires [`DeployEvent`] after each successful Deploy.
    deploy_tx: broadcast::Sender<DeployEvent>,
    /// Filter for which variables to stream.
    watch_filter: Arc<RwLock<Vec<String>>>,
    watch_interval_ms: Arc<RwLock<u32>>,
    /// Field-device I/O: one primary driver plus variable↔address mappings.
    io: Arc<std::sync::Mutex<ServerIoState>>,
}

/// A resolved I/O mapping: PLC variable ↔ parsed direct address.
struct ServerIoMapping {
    variable_name: String,
    address: IoAddress,
}

/// Driver connection parameters, parsed from a mapping's `connection_json`.
#[derive(serde::Deserialize, Default)]
struct ConnectionParams {
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    unit_id: u8,
    #[serde(default)]
    rack: u8,
    #[serde(default)]
    slot: u8,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    topic_prefix: String,
    #[serde(default)]
    client_id: String,
}

/// I/O state shared between the gRPC handlers and the scan-cycle task.
/// Sync mutex: `IoDriver` methods are synchronous (they internally `block_on`).
#[derive(Default)]
struct ServerIoState {
    driver: Option<Box<dyn IoDriver>>,
    mappings: Vec<ServerIoMapping>,
    connection_label: String,
    last_error: Option<String>,
}

/// Serde mirror of the proto `IoMapping` (prost types have no serde impls).
#[derive(serde::Deserialize, Clone)]
struct IoMappingJson {
    #[serde(default)]
    variable_name: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    driver: String,
    #[serde(default)]
    connection_json: String,
}

impl From<IoMappingJson> for pb::IoMapping {
    fn from(m: IoMappingJson) -> Self {
        pb::IoMapping {
            variable_name: m.variable_name,
            address: m.address,
            driver: m.driver,
            connection_json: m.connection_json,
        }
    }
}

impl PlcRuntimeGrpcServer {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(256);
        let (deploy_tx, _) = broadcast::channel(1);
        Self {
            inner: Arc::new(RwLock::new(RuntimeInner {
                runtime: None,
                instructions: Vec::new(),
                state: DomainRuntimeState::Stopped,
                stats: CycleStats::default(),
                deployed_bundle_crc32: 0,
                deployed_bundle_len: 0,
            })),
            scan_handle: Arc::new(Mutex::new(None)),
            scan_loop_handle: Arc::new(Mutex::new(None)),
            watch_tx,
            deploy_tx,
            watch_filter: Arc::new(RwLock::new(Vec::new())),
            watch_interval_ms: Arc::new(RwLock::new(0)),
            io: Arc::new(std::sync::Mutex::new(ServerIoState::default())),
        }
    }

    /// Build a `PlcRuntimeServer` ready to be served via tonic.
    pub fn into_service(self) -> PlcRuntimeServer<Self> {
        PlcRuntimeServer::new(self)
    }

    /// Read-only snapshot of the named variables from the currently deployed
    /// runtime — `None` per name that is absent (or while nothing is
    /// deployed). Lets in-process observers, e.g. a Sparkplug publisher,
    /// sample VM state without owning the scan loop.
    pub async fn variable_snapshot(&self, names: &[String]) -> Vec<Option<PlcValue>> {
        let guard = self.inner.read().await;
        let Some(rt) = guard.runtime.as_ref() else {
            return vec![None; names.len()];
        };
        // Force-aware: report the values the Watch stream reports (forced
        // overrides honored), not the raw memory slots.
        let watched_vars = rt.watch_variables();
        let watched: std::collections::HashMap<&str, &PlcValue> = watched_vars
            .iter()
            .map(|w| (w.name.as_str(), &w.value))
            .collect();
        names
            .iter()
            .map(|name| watched.get(name.as_str()).map(|value| (*value).clone()))
            .collect()
    }

    /// Subscribe to deploy events, fired after each successful Deploy.
    /// Capacity 1: a `RecvError::Lagged` means newer deploys superseded the
    /// missed ones — re-read the live state rather than replaying.
    pub fn deploy_events(&self) -> broadcast::Receiver<DeployEvent> {
        self.deploy_tx.subscribe()
    }

    /// Variable names of the currently deployed runtime (empty while nothing
    /// is deployed). Complements [`Self::variable_snapshot`] for observers
    /// that must discover the catalog after a redeploy.
    pub async fn variable_names(&self) -> Vec<String> {
        let guard = self.inner.read().await;
        guard
            .runtime
            .as_ref()
            .map(|rt| rt.watch_variables().into_iter().map(|v| v.name).collect())
            .unwrap_or_default()
    }

    /// Start the background scan-cycle loop.
    ///
    /// If a scan loop is already running, it is stopped first (Fix 2: guard
    /// against multiple concurrent loops).
    async fn start_scan_loop(&self) {
        // Stop any existing scan loop before starting a new one.
        self.stop_scan_loop().await;

        let (cancel_tx, mut cancel_rx) = watch::channel(false);

        let inner = self.inner.clone();
        let broadcast_tx = self.watch_tx.clone();
        let watch_filter = self.watch_filter.clone();
        let watch_interval_ms = self.watch_interval_ms.clone();
        let io_state = self.io.clone();

        // Store the cancel handle
        {
            let mut handle = self.scan_handle.lock().await;
            *handle = Some(ScanHandle { cancel: cancel_tx });
        }

        let loop_handle = tokio::spawn(async move {
            let mut last_push = std::time::Instant::now();

            loop {
                // Pace the loop by the deployed program's configured scan
                // cycle (RuntimeConfig.scan_cycle_ms); re-read each iteration
                // so Deploy/Start config overrides take effect live. The
                // 1ms floor avoids a busy loop on a zero config.
                let cycle_ms = {
                    let guard = inner.read().await;
                    guard
                        .runtime
                        .as_ref()
                        .map(|rt| rt.config().scan_cycle_ms)
                        .unwrap_or(10)
                        .max(1)
                };
                // Check cancellation
                tokio::select! {
                    result = cancel_rx.changed() => {
                        if result.is_err() || *cancel_rx.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(cycle_ms)) => {}
                }

                let _cycle_result = {
                    let mut guard = inner.write().await;
                    if let Some(ref mut rt) = guard.runtime {
                        // Read field inputs → force into the VM
                        io_cycle_read(&io_state, rt);

                        let cycle = rt.execute_cycle();
                        let result = cycle.map_err(|e| e.clone());

                        // Write VM outputs → field devices
                        io_cycle_write(&io_state, rt);

                        match result {
                            Ok(stats) => {
                                guard.state = DomainRuntimeState::Running;
                                guard.stats = stats;
                                Ok(())
                            }
                            Err(e) => {
                                guard.state = DomainRuntimeState::Error(e.clone());
                                Err(e)
                            }
                        }
                    } else {
                        Err("No program deployed".to_string())
                    }
                };

                // Push variable updates via broadcast channel
                let interval = {
                    let ms = watch_interval_ms.read().await;
                    *ms
                };
                let elapsed_since_push = last_push.elapsed().as_millis() as u64;
                if interval == 0 || elapsed_since_push >= interval as u64 {
                    let should_push = {
                        let guard = inner.read().await;
                        guard.state == DomainRuntimeState::Running
                            || matches!(guard.state, DomainRuntimeState::Error(_))
                    };

                    if should_push {
                        let updates = {
                            let guard = inner.read().await;
                            let filter = watch_filter.read().await;
                            guard.runtime.as_ref().map(|rt| {
                                let watched = rt.watch_variables();
                                if filter.is_empty() {
                                    watched
                                } else {
                                    watched
                                    .into_iter()
                                    .filter(|v| filter.contains(&v.name))
                                    .collect()
                                }
                            })
                        };

                        if let Some(watched) = updates {
                            for var in &watched {
                                let update = VariableUpdate {
                                    name: var.name.clone(),
                                    type_name: var.type_name.clone(),
                                    current: Some(plc_value_to_variable_value(&var.value)),
                                    quality: var.quality.clone(),
                                    timestamp_us: now_us(),
                                };
                                match broadcast_tx.send(Ok(update)) {
                                    Ok(_) => {}
                                    Err(tokio::sync::broadcast::error::SendError(_)) => {
                                        // No active receivers — skip.
                                    }
                                }
                            }
                        }
                        last_push = std::time::Instant::now();
                    }
                }
            }
        });

        // Store the JoinHandle so we can await/abort it later.
        {
            let mut handle = self.scan_loop_handle.lock().await;
            *handle = Some(loop_handle);
        }
    }

    /// Stop the background scan-cycle loop and wait for the task to finish.
    async fn stop_scan_loop(&self) {
        // Signal cancellation via the watch channel.
        {
            let mut handle = self.scan_handle.lock().await;
            if let Some(h) = handle.take() {
                let _ = h.cancel.send(true);
            }
        }

        // Await the task completion so we know it has fully stopped.
        let task_handle = {
            let mut h = self.scan_loop_handle.lock().await;
            h.take()
        };
        if let Some(jh) = task_handle {
            let _ = jh.await;
        }
    }
}

#[tonic::async_trait]
impl PlcRuntimeTrait for PlcRuntimeGrpcServer {
    async fn deploy(
        &self,
        request: Request<DeployRequest>,
    ) -> Result<Response<DeployResponse>, Status> {
        // Stop any running scan loop before replacing the runtime (Fix 3).
        self.stop_scan_loop().await;

        let req = request.into_inner();

        // Deserialize bytecode into a CompiledModule.
        let module: CompiledModule = bincode::deserialize(&req.bytecode)
            .map_err(|e| Status::invalid_argument(format!("Invalid bytecode: {}", e)))?;

        // Parse optional config overrides + I/O mappings.
        // JSON shape: { scan_cycle_ms, max_cycle_ms, io_mappings: [...] }
        let (config, io_mappings) = if !req.config_json.is_empty() {
            let value: serde_json::Value = serde_json::from_str(&req.config_json)
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {}", e)))?;
            let config: RuntimeConfig = serde_json::from_value(value.clone())
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {}", e)))?;
            let io_mappings: Vec<pb::IoMapping> = value
                .get("io_mappings")
                .cloned()
                .and_then(|v| serde_json::from_value::<Vec<IoMappingJson>>(v).ok())
                .map(|v| v.into_iter().map(Into::into).collect())
                .unwrap_or_default();
            (config, io_mappings)
        } else {
            (RuntimeConfig::default(), Vec::new())
        };

        // Wire field-device I/O when mappings were provided.
        if !io_mappings.is_empty() {
            self.apply_io_config(&io_mappings);
        }

        let instructions = module.instructions.clone();

        let mut guard = self.inner.write().await;
        guard.instructions = instructions;
        guard.runtime = Some(PlcRuntime::new(config, module));
        guard.state = DomainRuntimeState::Stopped;
        guard.stats = CycleStats::default();
        guard.deployed_bundle_crc32 = crc32_le(&req.bytecode);
        guard.deployed_bundle_len = req.bytecode.len() as u64;

        let new_state = runtime_state_to_pb(&guard);
        let event = DeployEvent {
            deployed_bundle_crc32: guard.deployed_bundle_crc32,
            deployed_bundle_len: guard.deployed_bundle_len,
        };
        drop(guard);
        let _ = self.deploy_tx.send(event);

        Ok(Response::new(DeployResponse {
            success: true,
            error_message: String::new(),
            new_state: Some(new_state),
        }))
    }

    async fn get_runtime_state(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ProtoRuntimeState>, Status> {
        let guard = self.inner.read().await;
        let pb_state = runtime_state_to_pb(&guard);
        Ok(Response::new(pb_state))
    }

    async fn start(
        &self,
        request: Request<StartConfig>,
    ) -> Result<Response<ProtoRuntimeState>, Status> {
        let req = request.into_inner();

        // Apply optional config overrides.
        if !req.config_json.is_empty() {
            let config: RuntimeConfig = serde_json::from_str(&req.config_json)
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {}", e)))?;
            let mut guard = self.inner.write().await;
            if let Some(ref mut rt) = guard.runtime {
                rt.set_config(config);
            }
        }

        // Update state
        {
            let mut guard = self.inner.write().await;
            if guard.runtime.is_none() {
                return Err(Status::failed_precondition("No program deployed"));
            }
            guard.state = DomainRuntimeState::Running;
        }

        // Start the scan loop
        self.start_scan_loop().await;

        let guard = self.inner.read().await;
        let pb_state = runtime_state_to_pb(&guard);
        Ok(Response::new(pb_state))
    }

    async fn stop(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ProtoRuntimeState>, Status> {
        self.stop_scan_loop().await;

        let mut guard = self.inner.write().await;
        guard.state = DomainRuntimeState::Stopped;

        let pb_state = runtime_state_to_pb(&guard);
        Ok(Response::new(pb_state))
    }

    async fn step(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ProtoRuntimeState>, Status> {
        let mut guard = self.inner.write().await;
        if let Some(ref mut rt) = guard.runtime {
            match rt.execute_cycle() {
                Ok(stats) => {
                    guard.stats = stats;
                    // Keep current state but update stats
                }
                Err(e) => {
                    guard.state = DomainRuntimeState::Error(e.clone());
                }
            }
        } else {
            return Err(Status::failed_precondition("No program deployed"));
        }

        let pb_state = runtime_state_to_pb(&guard);
        Ok(Response::new(pb_state))
    }

    type WatchVariablesStream = Pin<Box<dyn Stream<Item = Result<VariableUpdate, Status>> + Send>>;

    async fn watch_variables(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchVariablesStream>, Status> {
        let req = request.into_inner();

        // Update watch filter (applies globally to all watchers)
        {
            let mut filter = self.watch_filter.write().await;
            *filter = req.variable_names;
        }
        {
            let mut interval = self.watch_interval_ms.write().await;
            *interval = req.interval_ms;
        }

        // Subscribe to the broadcast channel — each caller gets its own receiver.
        let rx = self.watch_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(update_result) => Some(update_result),
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                // A lagged receiver means some messages were dropped; skip silently.
                None
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    async fn force_variable(
        &self,
        request: Request<ForceRequest>,
    ) -> Result<Response<ForceResponse>, Status> {
        let req = request.into_inner();
        let value = req.value.map(variable_value_to_plc).unwrap_or(PlcValue::Dint(0));

        let mut guard = self.inner.write().await;
        if let Some(ref mut rt) = guard.runtime {
            match rt.force_variable(&req.variable_name, value) {
                Ok(()) => {
                    // Read back the forced value
                    let watched = rt.watch_variables();
                    if let Some(var) = watched.into_iter().find(|v| v.name == req.variable_name) {
                        Ok(Response::new(ForceResponse {
                            success: true,
                            error_message: String::new(),
                            current: Some(VariableUpdate {
                                name: var.name,
                                type_name: var.type_name,
                                current: Some(plc_value_to_variable_value(&var.value)),
                                quality: var.quality,
                                timestamp_us: now_us(),
                            }),
                        }))
                    } else {
                        Ok(Response::new(ForceResponse {
                            success: true,
                            error_message: String::new(),
                            current: None,
                        }))
                    }
                }
                Err(e) => Ok(Response::new(ForceResponse {
                    success: false,
                    error_message: e,
                    current: None,
                })),
            }
        } else {
            Err(Status::failed_precondition("No program deployed"))
        }
    }

    async fn unforce_variable(
        &self,
        request: Request<UnforceRequest>,
    ) -> Result<Response<ForceResponse>, Status> {
        let req = request.into_inner();

        let mut guard = self.inner.write().await;
        if let Some(ref mut rt) = guard.runtime {
            rt.unforce_variable(&req.variable_name);

            let watched = rt.watch_variables();
            if let Some(var) = watched.into_iter().find(|v| v.name == req.variable_name) {
                Ok(Response::new(ForceResponse {
                    success: true,
                    error_message: String::new(),
                    current: Some(VariableUpdate {
                        name: var.name,
                        type_name: var.type_name,
                        current: Some(plc_value_to_variable_value(&var.value)),
                        quality: var.quality,
                        timestamp_us: now_us(),
                    }),
                }))
            } else {
                Ok(Response::new(ForceResponse {
                    success: true,
                    error_message: String::new(),
                    current: None,
                }))
            }
        } else {
            Err(Status::failed_precondition("No program deployed"))
        }
    }

    async fn get_io_status(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<IoStatus>, Status> {
        let guard = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let connection = match (&guard.driver, &guard.last_error) {
            (Some(driver), _) => IoConnectionStatus {
                driver: driver.name().to_string(),
                address: guard.connection_label.clone(),
                connected: driver.is_connected(),
                error_message: String::new(),
            },
            (None, Some(err)) => IoConnectionStatus {
                driver: guard.connection_label.clone(),
                address: String::new(),
                connected: false,
                error_message: err.clone(),
            },
            (None, None) => IoConnectionStatus {
                driver: String::new(),
                address: String::new(),
                connected: false,
                error_message: if guard.mappings.is_empty() {
                    "No I/O configured".to_string()
                } else {
                    String::new()
                },
            },
        };
        Ok(Response::new(IoStatus {
            connections: vec![connection],
        }))
    }

    async fn configure_io(
        &self,
        request: Request<IoConfig>,
    ) -> Result<Response<IoStatus>, Status> {
        let req = request.into_inner();
        self.apply_io_config(&req.mappings);

        // Report the result of the (re)configuration.
        let guard = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let connection = match (&guard.driver, &guard.last_error) {
            (Some(driver), _) => IoConnectionStatus {
                driver: driver.name().to_string(),
                address: guard.connection_label.clone(),
                connected: driver.is_connected(),
                error_message: String::new(),
            },
            (None, Some(err)) => IoConnectionStatus {
                driver: guard.connection_label.clone(),
                address: String::new(),
                connected: false,
                error_message: err.clone(),
            },
            (None, None) => IoConnectionStatus {
                driver: String::new(),
                address: String::new(),
                connected: false,
                error_message: "No usable mapping found".to_string(),
            },
        };
        Ok(Response::new(IoStatus {
            connections: vec![connection],
        }))
    }
}

impl PlcRuntimeGrpcServer {
    /// Apply proto I/O mappings: resolve direct addresses, (re)connect the
    /// primary driver. Uses the first mapping with a driver as the connection
    /// source; mappings without a parseable address are skipped.
    fn apply_io_config(&self, mappings: &[pb::IoMapping]) {
        let mut state = self.io.lock().unwrap_or_else(|e| e.into_inner());

        // Disconnect any previous driver first.
        if let Some(mut old) = state.driver.take() {
            old.disconnect();
        }
        state.last_error = None;

        state.mappings = mappings
            .iter()
            .filter_map(|m| {
                io_manager::parse_direct_address(&m.address).map(|address| ServerIoMapping {
                    variable_name: m.variable_name.clone(),
                    address,
                })
            })
            .collect();

        // Pick the first mapping that names a driver and try to connect.
        let Some(source) = mappings.iter().find(|m| !m.driver.is_empty()) else {
            state.connection_label = String::new();
            return;
        };

        let params = parse_connection(&source.connection_json);
        state.connection_label = format!(
            "{}://{}:{}",
            source.driver, params.host, params.port
        );

        match io_manager::create_driver(
            &source.driver,
            &params.host,
            params.port,
            params.unit_id,
            params.rack,
            params.slot,
            &params.endpoint,
            &params.topic_prefix,
            &params.client_id,
        ) {
            Ok(driver) => state.driver = Some(driver),
            Err(e) => state.last_error = Some(e.to_string()),
        }
    }
}

/// Parse connection parameters from a mapping's `connection_json`.
/// Accepts a JSON object, or a plain `host:port` string.
fn parse_connection(raw: &str) -> ConnectionParams {
    let raw = raw.trim();
    if raw.is_empty() {
        return ConnectionParams::default();
    }
    if let Ok(params) = serde_json::from_str::<ConnectionParams>(raw) {
        return params;
    }
    // Fallback: "host:port"
    if let Some((host, port)) = raw.rsplit_once(':') {
        if let Ok(port) = port.trim().parse::<u16>() {
            let mut params = ConnectionParams::default();
            params.host = host.trim().to_string();
            params.port = port;
            return params;
        }
    }
    ConnectionParams::default()
}

/// Read mapped field inputs and force them into the VM before a scan cycle.
fn io_cycle_read(io: &Arc<std::sync::Mutex<ServerIoState>>, rt: &mut PlcRuntime) {
    let mut guard = io.lock().unwrap_or_else(|e| e.into_inner());
    // Destructure so `driver` and `mappings` borrow disjoint fields through
    // the MutexGuard deref.
    let ServerIoState { driver, mappings, last_error, .. } = &mut *guard;
    let Some(driver) = driver.as_mut() else { return };

    let input_addrs: Vec<IoAddress> = mappings
        .iter()
        .filter(|m| m.address.area == IoArea::Input)
        .map(|m| m.address.clone())
        .collect();
    if input_addrs.is_empty() {
        return;
    }

    let names: Vec<&str> = mappings
        .iter()
        .filter(|m| m.address.area == IoArea::Input)
        .map(|m| m.variable_name.as_str())
        .collect();

    match driver.read_inputs(&input_addrs) {
        Ok(values) => {
            for ((name, value)) in names.into_iter().zip(values) {
                let _ = rt.force_variable(name, value);
            }
        }
        Err(e) => *last_error = Some(e.to_string()),
    }
}

/// Write mapped VM outputs to field devices after a scan cycle.
fn io_cycle_write(io: &Arc<std::sync::Mutex<ServerIoState>>, rt: &PlcRuntime) {
    let mut guard = io.lock().unwrap_or_else(|e| e.into_inner());
    let ServerIoState { driver, mappings, last_error, .. } = &mut *guard;
    let Some(driver) = driver.as_mut() else { return };

    let outputs: Vec<&ServerIoMapping> = mappings
        .iter()
        .filter(|m| m.address.area != IoArea::Input)
        .collect();
    if outputs.is_empty() {
        return;
    }

    let watched = rt.watch_variables();
    let pairs: Vec<(IoAddress, PlcValue)> = outputs
        .iter()
        .filter_map(|m| {
            watched
                .iter()
                .find(|v| v.name == m.variable_name)
                .map(|v| (m.address.clone(), v.value.clone()))
        })
        .collect();

    if let Err(e) = driver.write_outputs(&pairs) {
        *last_error = Some(e.to_string());
    }
}

#[cfg(test)]
mod drift_tests {
    use super::*;

    use mt_plc_types::{Instruction, Opcode};

    use crate::pb::{DeployRequest, Empty};

    fn sample_module(name: &str) -> CompiledModule {
        CompiledModule {
            name: name.to_string(),
            instructions: vec![Instruction { opcode: Opcode::PushBool, operand: Some(1) }],
            variable_slots: vec![("motor".to_string(), PlcValue::Bool(false))],
            pou_table: vec![("Main".to_string(), 0)],
            name_constants: vec![],
            line_map: vec![(1, 0)],
        }
    }

    async fn deploy_bytes(server: &PlcRuntimeGrpcServer, bytes: &[u8]) {
        server
            .deploy(Request::new(DeployRequest {
                bytecode: bytes.to_vec(),
                config_json: String::new(),
            }))
            .await
            .expect("deploy");
    }

    async fn fingerprint(server: &PlcRuntimeGrpcServer) -> (u32, u64) {
        let state = server
            .get_runtime_state(Request::new(Empty {}))
            .await
            .expect("get_runtime_state")
            .into_inner();
        (state.deployed_bundle_crc32, state.deployed_bundle_len)
    }

    #[tokio::test]
    async fn fresh_runtime_reports_zero_fingerprint() {
        let server = PlcRuntimeGrpcServer::new();
        assert_eq!(fingerprint(&server).await, (0, 0));
    }

    #[tokio::test]
    async fn deploy_records_bundle_fingerprint() {
        let server = PlcRuntimeGrpcServer::new();
        let bytes = bincode::serialize(&sample_module("Main")).expect("serialize");

        deploy_bytes(&server, &bytes).await;
        assert_eq!(fingerprint(&server).await, (crc32_le(&bytes), bytes.len() as u64));
    }

    #[tokio::test]
    async fn redeploy_updates_fingerprint() {
        let server = PlcRuntimeGrpcServer::new();
        let first = bincode::serialize(&sample_module("Main")).expect("serialize");
        let mut second = first.clone();
        second.push(0xFF); // any byte difference must move the CRC

        deploy_bytes(&server, &first).await;
        let (crc_before, len_before) = fingerprint(&server).await;

        deploy_bytes(&server, &second).await;
        let (crc_after, len_after) = fingerprint(&server).await;

        assert_ne!(crc_before, crc_after);
        assert_ne!(len_before, len_after);
        assert_eq!(crc_after, crc32_le(&second));
    }

    #[tokio::test]
    async fn start_stop_preserves_fingerprint() {
        let server = PlcRuntimeGrpcServer::new();
        let bytes = bincode::serialize(&sample_module("Main")).expect("serialize");
        deploy_bytes(&server, &bytes).await;

        server
            .start(Request::new(StartConfig { config_json: String::new() }))
            .await
            .expect("start");
        assert_eq!(fingerprint(&server).await, (crc32_le(&bytes), bytes.len() as u64));

        server.stop(Request::new(Empty {})).await.expect("stop");
        assert_eq!(fingerprint(&server).await, (crc32_le(&bytes), bytes.len() as u64));
    }

    #[tokio::test]
    async fn deploy_fires_event_with_fingerprint() {
        let server = PlcRuntimeGrpcServer::new();
        let mut events = server.deploy_events();
        assert!(events.try_recv().is_err(), "no event before deploy");

        let bytes = bincode::serialize(&sample_module("Main")).expect("serialize");
        deploy_bytes(&server, &bytes).await;

        let event = events.recv().await.expect("deploy event");
        assert_eq!(event.deployed_bundle_crc32, crc32_le(&bytes));
        assert_eq!(event.deployed_bundle_len, bytes.len() as u64);
    }
}
