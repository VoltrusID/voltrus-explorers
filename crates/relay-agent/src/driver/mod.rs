//! Driver abstraction: the relay agent is transport + routing; drivers speak PLC.

pub mod echo;
#[cfg(feature = "grpc")]
pub mod grpc;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{EventFrame, VarInfo};

/// Variable names passed to [`DeviceDriver::subscribe`] — empty means "all".
pub type Subscription = Vec<String>;

// AFIT keeps the trait object-safe-free and simple; the returned futures are
// Send in practice (the session spawns them), the lint just can't prove it.
#[allow(async_fn_in_trait)]
pub trait DeviceDriver {
    /// List variables with type and current value.
    async fn browse(&mut self) -> Result<Vec<VarInfo>>;

    /// Start pushing [`EventFrame`]s into `tx`; returns the ok-payload for the
    /// ide that asked. A second call replaces the previous subscription.
    async fn subscribe(&mut self, names: Subscription, tx: UnboundedSender<EventFrame>) -> Result<Value>;

    /// Write a value. Note: the mt-grpc runtime exposes no plain-write RPC, so
    /// the grpc driver implements this as a force — see driver/grpc.rs.
    async fn write(&mut self, name: &str, value: Value) -> Result<Value>;

    async fn force(&mut self, name: &str, value: Value) -> Result<()>;
    async fn unforce(&mut self, name: &str) -> Result<()>;

    /// Runtime status as a JSON object (driver-defined shape).
    async fn status(&mut self) -> Result<Value>;

    /// Deploy a compiled bytecode bundle; `config` is an optional JSON string.
    async fn deploy(&mut self, bytecode: &[u8], config: Option<String>) -> Result<Value>;
}
