use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct NodeValue {
    pub node_id: String,
    pub name: String,
    pub data_type: String,
    pub value: serde_json::Value,
    pub timestamp: u64,
}

#[derive(Clone)]
pub struct OpcUaStore {
    pub nodes: Arc<RwLock<HashMap<String, NodeValue>>>,
    pub server_running: Arc<AtomicBool>,
}

impl OpcUaStore {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            server_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn read_all(&self) -> Vec<NodeValue> {
        self.nodes.read().await.values().cloned().collect()
    }

    pub async fn read(&self, node_id: &str) -> Option<NodeValue> {
        self.nodes.read().await.get(node_id).cloned()
    }

    pub async fn write(&self, node_id: &str, value: serde_json::Value) {
        let mut nodes = self.nodes.write().await;
        match nodes.get_mut(node_id) {
            Some(existing) => {
                existing.value = value;
                existing.timestamp = now_ms();
            }
            None => {
                nodes.insert(node_id.to_string(), NodeValue {
                    node_id: node_id.to_string(),
                    name: node_id.to_string(),
                    data_type: "Variant".to_string(),
                    value,
                    timestamp: now_ms(),
                });
            }
        }
    }

    pub async fn write_named(&self, node_id: &str, name: &str, data_type: &str, value: serde_json::Value) {
        let mut nodes = self.nodes.write().await;
        match nodes.get_mut(node_id) {
            Some(existing) => {
                existing.value = value;
                existing.timestamp = now_ms();
            }
            None => {
                nodes.insert(node_id.to_string(), NodeValue {
                    node_id: node_id.to_string(),
                    name: name.to_string(),
                    data_type: data_type.to_string(),
                    value,
                    timestamp: now_ms(),
                });
            }
        }
    }

    pub async fn init_node(&self, node_id: &str, name: &str, data_type: &str, value: serde_json::Value) {
        let mut nodes = self.nodes.write().await;
        nodes.entry(node_id.to_string()).or_insert(NodeValue {
            node_id: node_id.to_string(),
            name: name.to_string(),
            data_type: data_type.to_string(),
            value,
            timestamp: now_ms(),
        });
    }

    pub async fn clear(&self) {
        self.nodes.write().await.clear();
    }
}

/// Milliseconds between the OPC UA epoch (1601-01-01, what `DateTime` ticks count from) and the Unix epoch.
const OPC_UA_TO_UNIX_MS: i64 = 11_644_473_600_000;

/// An OPC UA `DateTime`'s 100 ns ticks since 1601 as Unix milliseconds; 0 for a null or pre-1970 time.
pub fn opc_ticks_to_unix_ms(ticks: i64) -> u64 {
    (ticks / 10_000 - OPC_UA_TO_UNIX_MS).max(0) as u64
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod epoch_tests {
    use super::opc_ticks_to_unix_ms;

    #[test]
    fn opc_ticks_count_from_1601_not_1970() {
        // 2026-01-01T00:00:00Z is 1_767_225_600 s after the Unix epoch; as OPC UA ticks (100 ns since 1601):
        let ticks = (1_767_225_600i64 + 11_644_473_600) * 10_000_000;
        assert_eq!(opc_ticks_to_unix_ms(ticks), 1_767_225_600_000);
        assert_eq!(opc_ticks_to_unix_ms(11_644_473_600 * 10_000_000), 0);
        // A null DateTime (ticks 0) and anything before 1970 are 0, not a wrapped number.
        assert_eq!(opc_ticks_to_unix_ms(0), 0);
        assert_eq!(opc_ticks_to_unix_ms(-5), 0);
    }
}
