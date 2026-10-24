use crate::io_manager::{IoAddress, IoArea, IoDriver, IoError, IoSize};
use crate::types::PlcValue;
use mt_opcua::opcua_client::OpcUaProtocolClient;
use mt_opcua::types::{OpcUaConfig, OpcUaNode};

pub struct OpcUaIoDriver {
    client: Option<OpcUaProtocolClient>,
    endpoint_url: String,
}

impl OpcUaIoDriver {
    pub fn new(endpoint_url: String) -> Self {
        Self {
            client: None,
            endpoint_url,
        }
    }

    pub async fn connect(&mut self) -> Result<(), IoError> {
        let config = OpcUaConfig {
            endpoint_url: self.endpoint_url.clone(),
            security_policy: String::new(),
            security_mode: String::new(),
            username: None,
            password: None,
            poll_interval_ms: 1000,
        };
        let mut client = OpcUaProtocolClient::new();
        client.connect(config).await.map_err(|e| IoError {
            message: format!("OPC-UA connect: {}", e),
            recoverable: true,
        })?;
        self.client = Some(client);
        Ok(())
    }

    fn io_to_node(&self, addr: &IoAddress) -> OpcUaNode {
        let area = match addr.area {
            IoArea::Input => "Input",
            IoArea::Output => "Output",
            IoArea::Memory => "Memory",
        };
        let node_id = match addr.size {
            IoSize::Bit => format!("ns=2;s={}.{}.{}", area, addr.offset, addr.bit.unwrap_or(0)),
            _ => format!("ns=2;s={}.{}", area, addr.offset),
        };
        OpcUaNode {
            node_id,
            name: format!("{}_{}", area, addr.offset),
            data_type: String::new(),
        }
    }

    fn value_to_plc(val: &serde_json::Value) -> PlcValue {
        match val {
            serde_json::Value::Bool(b) => PlcValue::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    PlcValue::Dint(i as i32)
                } else if let Some(f) = n.as_f64() {
                    PlcValue::Real(f as f32)
                } else {
                    PlcValue::Dint(0)
                }
            }
            _ => PlcValue::Dint(0),
        }
    }
}

impl IoDriver for OpcUaIoDriver {
    fn read_inputs(&mut self, addresses: &[IoAddress]) -> Result<Vec<PlcValue>, IoError> {
        let client = self.client.as_ref().ok_or(IoError {
            message: "OPC-UA not connected".into(),
            recoverable: true,
        })?;
        let nodes: Vec<OpcUaNode> = addresses.iter().map(|a| self.io_to_node(a)).collect();
        let rt = tokio::runtime::Handle::current();
        let values = rt
            .block_on(async { client.read_tags(&nodes).await })
            .map_err(|e| IoError {
                message: format!("OPC-UA read: {}", e),
                recoverable: true,
            })?;
        Ok(values
            .iter()
            .map(|v| Self::value_to_plc(&v.value))
            .collect())
    }

    fn write_outputs(&mut self, mappings: &[(IoAddress, PlcValue)]) -> Result<(), IoError> {
        let client = self.client.as_ref().ok_or(IoError {
            message: "OPC-UA not connected".into(),
            recoverable: true,
        })?;
        let rt = tokio::runtime::Handle::current();
        for (addr, plc_val) in mappings {
            let node = self.io_to_node(addr);
            let json_val = match plc_val {
                PlcValue::Bool(b) => serde_json::Value::Bool(*b),
                PlcValue::Int(i) => serde_json::json!(*i),
                PlcValue::Dint(i) => serde_json::json!(*i),
                PlcValue::Real(f) => serde_json::json!(*f),
                PlcValue::Word(w) => serde_json::json!(*w),
                PlcValue::Byte(b) => serde_json::json!(*b),
                _ => serde_json::Value::Null,
            };
            let opc_val = mt_opcua::types::OpcUaValue {
                node_id: node.node_id.clone(),
                name: node.name.clone(),
                value: json_val,
                timestamp: 0,
            };
            rt.block_on(async { client.write_tag(&node, &opc_val).await })
                .map_err(|e| IoError {
                    message: format!("OPC-UA write: {}", e),
                    recoverable: true,
                })?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "opcua"
    }

    fn is_connected(&self) -> bool {
        self.client.as_ref().map_or(false, |c| c.is_connected())
    }

    fn disconnect(&mut self) {
        if let Some(ref mut client) = self.client {
            let _ = client.disconnect();
        }
        self.client = None;
    }
}
