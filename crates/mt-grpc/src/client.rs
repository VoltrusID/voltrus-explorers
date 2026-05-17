use std::time::Duration;

use tokio_stream::StreamExt;
use tonic::transport::{Channel, Endpoint};

use crate::pb::plc_runtime_client::PlcRuntimeClient as TonicPlcRuntimeClient;
use crate::pb::*;

/// High-level gRPC client for the Voltrus PLC Runtime.
///
/// Wraps the generated tonic client with ergonomic methods.
pub struct PlcRuntimeClient {
    inner: TonicPlcRuntimeClient<Channel>,
}

impl PlcRuntimeClient {
    /// Connect to a PLC runtime at the given address (e.g. "http://127.0.0.1:50051").
    pub async fn connect(addr: &str) -> Result<Self, tonic::transport::Error> {
        let endpoint = Endpoint::from_shared(addr.to_string())?
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5));
        let client = TonicPlcRuntimeClient::connect(endpoint).await?;
        Ok(Self { inner: client })
    }

    /// Connect with a custom endpoint (for advanced configuration).
    pub async fn connect_with_endpoint(
        endpoint: Endpoint,
    ) -> Result<Self, tonic::transport::Error> {
        let client = TonicPlcRuntimeClient::connect(endpoint).await?;
        Ok(Self { inner: client })
    }

    /// Deploy a compiled bytecode bundle to the runtime.
    ///
    /// `bytecode` should be a serialized `CompiledModule` (via bincode/serde).
    /// `config_json` is an optional JSON string of `RuntimeConfig` overrides.
    pub async fn deploy(
        &mut self,
        bytecode: &[u8],
        config_json: Option<&str>,
    ) -> Result<DeployResponse, tonic::Status> {
        let req = DeployRequest {
            bytecode: bytecode.to_vec(),
            config_json: config_json.unwrap_or("").to_string(),
        };
        let resp = self.inner.deploy(req).await?;
        Ok(resp.into_inner())
    }

    /// Get the current runtime state and cycle statistics.
    pub async fn get_runtime_state(&mut self) -> Result<RuntimeState, tonic::Status> {
        let resp = self.inner.get_runtime_state(Empty {}).await?;
        Ok(resp.into_inner())
    }

    /// Start the runtime scan cycle loop.
    ///
    /// Optionally pass a JSON-encoded `RuntimeConfig` to override settings
    /// before starting.
    pub async fn start(&mut self, config_json: Option<&str>) -> Result<RuntimeState, tonic::Status> {
        let req = StartConfig {
            config_json: config_json.unwrap_or("").to_string(),
        };
        let resp = self.inner.start(req).await?;
        Ok(resp.into_inner())
    }

    /// Stop the runtime scan cycle loop.
    pub async fn stop(&mut self) -> Result<RuntimeState, tonic::Status> {
        let resp = self.inner.stop(Empty {}).await?;
        Ok(resp.into_inner())
    }

    /// Execute a single scan cycle.
    pub async fn step(&mut self) -> Result<RuntimeState, tonic::Status> {
        let resp = self.inner.step(Empty {}).await?;
        Ok(resp.into_inner())
    }

    /// Subscribe to variable updates.
    ///
    /// Returns a stream of `VariableUpdate` messages pushed from the runtime.
    /// If `variable_names` is empty, all variables are streamed.
    /// `interval_ms` controls the minimum push interval (0 = every cycle).
    pub async fn watch_variables(
        &mut self,
        variable_names: Vec<String>,
        interval_ms: u32,
    ) -> Result<VariableUpdateStream, tonic::Status> {
        let req = WatchRequest {
            variable_names,
            interval_ms,
        };
        let resp = self.inner.watch_variables(req).await?;
        Ok(VariableUpdateStream {
            inner: resp.into_inner(),
        })
    }

    /// Force a variable to a specific value.
    pub async fn force_variable(
        &mut self,
        name: &str,
        value: VariableValue,
    ) -> Result<ForceResponse, tonic::Status> {
        let req = ForceRequest {
            variable_name: name.to_string(),
            value: Some(value),
        };
        let resp = self.inner.force_variable(req).await?;
        Ok(resp.into_inner())
    }

    /// Force a boolean variable.
    pub async fn force_bool(
        &mut self,
        name: &str,
        val: bool,
    ) -> Result<ForceResponse, tonic::Status> {
        self.force_variable(name, VariableValue {
            value: Some(variable_value::Value::BoolVal(val)),
        })
        .await
    }

    /// Force an integer (DINT) variable.
    pub async fn force_dint(
        &mut self,
        name: &str,
        val: i32,
    ) -> Result<ForceResponse, tonic::Status> {
        self.force_variable(name, VariableValue {
            value: Some(variable_value::Value::DintVal(val)),
        })
        .await
    }

    /// Force a real (float) variable.
    pub async fn force_real(
        &mut self,
        name: &str,
        val: f32,
    ) -> Result<ForceResponse, tonic::Status> {
        self.force_variable(name, VariableValue {
            value: Some(variable_value::Value::RealVal(val)),
        })
        .await
    }

    /// Force a string variable.
    pub async fn force_string(
        &mut self,
        name: &str,
        val: &str,
    ) -> Result<ForceResponse, tonic::Status> {
        self.force_variable(name, VariableValue {
            value: Some(variable_value::Value::StringVal(val.to_string())),
        })
        .await
    }

    /// Remove a forced override from a variable.
    pub async fn unforce_variable(
        &mut self,
        name: &str,
    ) -> Result<ForceResponse, tonic::Status> {
        let req = UnforceRequest {
            variable_name: name.to_string(),
        };
        let resp = self.inner.unforce_variable(req).await?;
        Ok(resp.into_inner())
    }

    /// Get the I/O driver connection status.
    pub async fn get_io_status(&mut self) -> Result<IoStatus, tonic::Status> {
        let resp = self.inner.get_io_status(Empty {}).await?;
        Ok(resp.into_inner())
    }

    /// Configure I/O mappings.
    pub async fn configure_io(&mut self, config: IoConfig) -> Result<IoStatus, tonic::Status> {
        let resp = self.inner.configure_io(config).await?;
        Ok(resp.into_inner())
    }
}

/// Stream of variable updates from the runtime.
pub struct VariableUpdateStream {
    inner: tonic::Streaming<VariableUpdate>,
}

impl VariableUpdateStream {
    /// Wait for the next variable update.
    pub async fn next(&mut self) -> Option<Result<VariableUpdate, tonic::Status>> {
        self.inner.next().await
    }

    /// Collect all buffered updates until the stream ends.
    pub async fn collect_available(&mut self) -> Vec<Result<VariableUpdate, tonic::Status>> {
        let mut results = Vec::new();
        while let Some(item) = self.inner.next().await {
            results.push(item);
        }
        results
    }
}
