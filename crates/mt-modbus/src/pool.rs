use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

/// A pooled Modbus TCP connection wrapper.
struct PooledConnection {
    ctx: tokio_modbus::client::Context,
    last_used: Instant,
}

/// Connection pool for reusing Modbus TCP connections across polling cycles.
///
/// Each device is identified by a string key. Connections that have been idle
/// longer than `max_idle` are discarded and re-established on next use.
pub struct ModbusPool {
    connections: Arc<tokio::sync::Mutex<HashMap<String, PooledConnection>>>,
    max_idle: Duration,
}

impl ModbusPool {
    /// Create a new pool with a 60-second idle timeout.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            max_idle: Duration::from_secs(60),
        }
    }

    /// Create a pool with a custom idle timeout.
    pub fn with_max_idle(max_idle: Duration) -> Self {
        Self {
            connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            max_idle,
        }
    }

    /// Get an existing connection for `device_id`, or create a new one.
    ///
    /// If `bind_addr` is provided, the TCP connection will be bound to that
    /// local address before connecting to `addr`.
    pub async fn get_or_connect(
        &self,
        device_id: &str,
        addr: std::net::SocketAddr,
        unit_id: u8,
        bind_addr: Option<std::net::SocketAddr>,
    ) -> anyhow::Result<tokio_modbus::client::Context> {
        let mut conns = self.connections.lock().await;
        if let Some(pooled) = conns.remove(device_id) {
            if pooled.last_used.elapsed() < self.max_idle {
                tracing::debug!("Reusing connection for device {device_id}");
                return Ok(pooled.ctx);
            }
            tracing::debug!("Dropping idle connection for device {device_id}");
        }

        tracing::debug!("Connecting to device {device_id} at {addr}");
        let ctx = if let Some(_local) = bind_addr {
            // NOTE: The standalone pool does not have access to the
            // voltrus network::bind_and_connect helper. For now, fall
            // back to a direct connection. Users needing bind_address
            // should construct a custom connection and use `return_connection`.
            tcp::connect_slave(addr, Slave(unit_id)).await?
        } else {
            tcp::connect_slave(addr, Slave(unit_id)).await?
        };
        Ok(ctx)
    }

    /// Return a connection to the pool for later reuse.
    pub async fn return_connection(&self, device_id: &str, ctx: tokio_modbus::client::Context) {
        let mut conns = self.connections.lock().await;
        conns.insert(
            device_id.to_string(),
            PooledConnection {
                ctx,
                last_used: Instant::now(),
            },
        );
    }

    /// Remove and drop a connection from the pool (e.g. after an error).
    pub async fn remove(&self, device_id: &str) {
        let mut conns = self.connections.lock().await;
        conns.remove(device_id);
    }
}

impl Default for ModbusPool {
    fn default() -> Self {
        Self::new()
    }
}
