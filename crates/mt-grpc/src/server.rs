use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

use mt_plc_runtime::opcode::{CompiledModule, Instruction};
use mt_plc_runtime::types::{CycleStats, RuntimeConfig};
use mt_plc_runtime::{PlcRuntime, PlcValue};

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

fn runtime_state_to_pb(state: &DomainRuntimeState, stats: &CycleStats) -> ProtoRuntimeState {
    ProtoRuntimeState {
        status: runtime_status_to_pb(state).into(),
        cycle_count: stats.cycle_count,
        last_cycle_ms: stats.last_cycle_ms,
        avg_cycle_ms: stats.avg_cycle_ms,
        max_cycle_ms: stats.max_cycle_ms,
        error_message: match state {
            DomainRuntimeState::Error(e) => e.clone(),
            _ => String::new(),
        },
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
}

/// Handle to the background scan-cycle task, used to start/stop it.
struct ScanHandle {
    cancel: watch::Sender<bool>,
}

/// The gRPC server wrapping a PLC runtime.
pub struct PlcRuntimeGrpcServer {
    inner: Arc<RwLock<RuntimeInner>>,
    scan_handle: Arc<Mutex<Option<ScanHandle>>>,
    scan_loop_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    watch_tx: broadcast::Sender<Result<VariableUpdate, Status>>,
    /// Filter for which variables to stream.
    watch_filter: Arc<RwLock<Vec<String>>>,
    watch_interval_ms: Arc<RwLock<u32>>,
}

impl PlcRuntimeGrpcServer {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(RwLock::new(RuntimeInner {
                runtime: None,
                instructions: Vec::new(),
                state: DomainRuntimeState::Stopped,
                stats: CycleStats::default(),
            })),
            scan_handle: Arc::new(Mutex::new(None)),
            scan_loop_handle: Arc::new(Mutex::new(None)),
            watch_tx,
            watch_filter: Arc::new(RwLock::new(Vec::new())),
            watch_interval_ms: Arc::new(RwLock::new(0)),
        }
    }

    /// Build a `PlcRuntimeServer` ready to be served via tonic.
    pub fn into_service(self) -> PlcRuntimeServer<Self> {
        PlcRuntimeServer::new(self)
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

        // Store the cancel handle
        {
            let mut handle = self.scan_handle.lock().await;
            *handle = Some(ScanHandle { cancel: cancel_tx });
        }

        let loop_handle = tokio::spawn(async move {
            let mut last_push = std::time::Instant::now();

            loop {
                // Check cancellation
                tokio::select! {
                    result = cancel_rx.changed() => {
                        if result.is_err() || *cancel_rx.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {}
                }

                let _cycle_result = {
                    let mut guard = inner.write().await;
                    if let Some(ref mut rt) = guard.runtime {
                        match rt.execute_cycle() {
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

        // Parse optional config overrides.
        let config = if !req.config_json.is_empty() {
            serde_json::from_str::<RuntimeConfig>(&req.config_json)
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {}", e)))?
        } else {
            RuntimeConfig::default()
        };

        let instructions = module.instructions.clone();

        let mut guard = self.inner.write().await;
        guard.instructions = instructions;
        guard.runtime = Some(PlcRuntime::new(config, module));
        guard.state = DomainRuntimeState::Stopped;
        guard.stats = CycleStats::default();

        let new_state = runtime_state_to_pb(&guard.state, &guard.stats);

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
        let pb_state = runtime_state_to_pb(&guard.state, &guard.stats);
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
        let pb_state = runtime_state_to_pb(&guard.state, &guard.stats);
        Ok(Response::new(pb_state))
    }

    async fn stop(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ProtoRuntimeState>, Status> {
        self.stop_scan_loop().await;

        let mut guard = self.inner.write().await;
        guard.state = DomainRuntimeState::Stopped;

        let pb_state = runtime_state_to_pb(&guard.state, &guard.stats);
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

        let pb_state = runtime_state_to_pb(&guard.state, &guard.stats);
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
        // Stub: I/O drivers are not yet connected through gRPC.
        // Returns empty status; will be filled in Phase 2+.
        Ok(Response::new(IoStatus {
            connections: Vec::new(),
        }))
    }

    async fn configure_io(
        &self,
        _request: Request<IoConfig>,
    ) -> Result<Response<IoStatus>, Status> {
        // Stub: I/O configuration will be implemented in Phase 2+.
        Ok(Response::new(IoStatus {
            connections: Vec::new(),
        }))
    }
}
