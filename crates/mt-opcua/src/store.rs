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

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
