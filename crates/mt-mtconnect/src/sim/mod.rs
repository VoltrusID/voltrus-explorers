//! MTConnect agent simulator: an HTTP agent (`/probe`, `/current`, `/sample`, `/assets`) and an
//! SHDR adapter serving the same mutable [`SimModel`].

mod http;
mod model;
mod shdr;
mod template;
mod xmlout;
mod xpath;

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use model::{SimItemState, SimModel, SimSnapshot};
pub use shdr::{format_observation, pong_for};
pub use template::DeviceTemplate;

use crate::error::{MtConnectError, Result};

fn default_bind() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    5050
}
fn default_shdr_port() -> Option<u16> {
    Some(7878)
}
fn default_template() -> DeviceTemplate {
    DeviceTemplate::Vmc3Axis
}
fn default_buffer() -> usize {
    8192
}
fn default_tick() -> u64 {
    250
}
fn default_heartbeat() -> u64 {
    10_000
}

/// Simulator settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimConfig {
    /// Interface to bind both servers to (`0.0.0.0` to accept other machines).
    #[serde(default = "default_bind")]
    pub bind_addr: String,
    /// HTTP agent port (`0` picks a free one).
    #[serde(default = "default_port")]
    pub http_port: u16,
    /// SHDR adapter port (`Some(0)` picks a free one, `None` disables the adapter).
    #[serde(default = "default_shdr_port")]
    pub shdr_port: Option<u16>,
    #[serde(default = "default_template")]
    pub template: DeviceTemplate,
    /// Ring buffer capacity in observations (`bufferSize`).
    #[serde(default = "default_buffer")]
    pub buffer_size: usize,
    /// How often the simulated program advances.
    #[serde(default = "default_tick")]
    pub tick_ms: u64,
    /// Heartbeat announced to SHDR clients in `* PONG <ms>`.
    #[serde(default = "default_heartbeat")]
    pub shdr_heartbeat_ms: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind(),
            http_port: default_port(),
            shdr_port: default_shdr_port(),
            template: default_template(),
            buffer_size: default_buffer(),
            tick_ms: default_tick(),
            shdr_heartbeat_ms: default_heartbeat(),
        }
    }
}

/// A running simulator. Stop it with [`stop`](Self::stop) or by dropping it.
pub struct AgentSimulator {
    model: SimModel,
    http_addr: SocketAddr,
    shdr_addr: Option<SocketAddr>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl AgentSimulator {
    /// Bind the servers and start simulating.
    pub async fn start(config: SimConfig) -> Result<Self> {
        if config.tick_ms == 0 {
            return Err(MtConnectError::InvalidConfig("tickMs must be > 0".into()));
        }
        let bind = |port: u16| format!("{}:{port}", config.bind_addr);
        let http = TcpListener::bind(bind(config.http_port))
            .await
            .map_err(|e| MtConnectError::Simulator(format!("cannot bind HTTP {}:{}: {e}", config.bind_addr, config.http_port)))?;
        let http_addr = http.local_addr()?;
        let shdr = match config.shdr_port {
            Some(port) => Some(
                TcpListener::bind(bind(port))
                    .await
                    .map_err(|e| MtConnectError::Simulator(format!("cannot bind SHDR {}:{port}: {e}", config.bind_addr)))?,
            ),
            None => None,
        };
        let shdr_addr = shdr.as_ref().map(|l| l.local_addr()).transpose()?;

        let model = SimModel::new(config.template, config.buffer_size);
        let (shutdown, rx) = watch::channel(false);
        let mut tasks = Vec::new();

        tasks.push(tokio::spawn(http::serve(http, model.clone(), rx.clone())));
        if let Some(l) = shdr {
            tasks.push(tokio::spawn(shdr::serve(l, model.clone(), config.shdr_heartbeat_ms, rx.clone())));
        }

        let ticker = model.clone();
        let mut tick_rx = rx.clone();
        let period = std::time::Duration::from_millis(config.tick_ms);
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last = std::time::Instant::now();
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let now = std::time::Instant::now();
                        ticker.tick(now.duration_since(last).as_secs_f64());
                        last = now;
                    }
                    _ = tick_rx.changed() => return,
                }
            }
        }));

        tracing::info!(%http_addr, ?shdr_addr, "MTConnect simulator started");
        Ok(Self { model, http_addr, shdr_addr, shutdown, tasks })
    }

    /// The mutable model: change values, raise conditions, pause, restart the agent.
    pub fn model(&self) -> &SimModel {
        &self.model
    }

    /// Address the HTTP agent listens on.
    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// Address the SHDR adapter listens on, if enabled.
    pub fn shdr_addr(&self) -> Option<SocketAddr> {
        self.shdr_addr
    }

    /// Base URL for an [`AgentClient`](crate::AgentClient).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.http_addr)
    }

    /// Stop both servers and the simulation, closing open connections.
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        for t in self.tasks.drain(..) {
            let _ = t.await;
        }
    }
}

impl Drop for AgentSimulator {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for t in &self.tasks {
            t.abort();
        }
    }
}
