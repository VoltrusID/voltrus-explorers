use std::sync::Arc;

use anyhow::{Result, anyhow};
use opcua::server::{
    ServerBuilder, ServerHandle,
    address_space::{AccessLevel, AddressSpace, NodeType, VariableBuilder},
    node_manager::memory::{InMemoryNodeManagerBuilder, SimpleNodeManager, SimpleNodeManagerBuilder},
    diagnostics::NamespaceMetadata,
};
use opcua::core::sync::RwLock;
use opcua::crypto::SecurityPolicy;
use opcua::nodes::NodeBase;
use opcua::types::{
    DataEncoding, DataTypeId, DataValue, DateTime, NodeId, NumericRange, ObjectId,
    ReferenceTypeId, TimestampsToReturn, VariableTypeId, Variant,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::types::OpcUaValue;

const DEFAULT_NODES: &[(&str, &str, &str)] = &[
    ("ns=2;s=Reactor.Temperature", "Temperature", "Double"),
    ("ns=2;s=Reactor.Pressure", "Pressure", "Double"),
    ("ns=2;s=Reactor.Level", "Level", "Double"),
    ("ns=2;s=Reactor.AgitatorSpeed", "AgitatorSpeed", "Double"),
    ("ns=2;s=FeedFlow_Rate", "FeedFlow", "Double"),
    ("ns=2;s=ProductFlow_Rate", "ProductFlow", "Double"),
    ("ns=2;s=pH_Value", "pH", "Double"),
    ("ns=2;s=Reactor.Status.Running", "Running", "Boolean"),
    ("ns=2;s=Boiler.SteamPressure", "SteamPressure", "Double"),
    ("ns=2;s=Boiler.WaterLevel", "WaterLevel", "Double"),
    ("ns=2;s=Boiler.FuelFlow", "FuelFlow", "Double"),
    ("ns=2;s=Boiler.FeedWaterTemp", "FeedWaterTemp", "Double"),
    ("ns=2;s=Boiler.FlueGasTemp", "FlueGasTemp", "Double"),
    ("ns=2;s=Boiler.BurnerActive", "BurnerActive", "Boolean"),
    ("ns=2;s=Boiler.Efficiency", "Efficiency", "Double"),
    ("ns=2;s=Boiler.SafetyValve", "SafetyValve", "Boolean"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerNodeDef {
    pub node_id: String,
    pub browse_name: String,
    pub data_type: String,
}

impl ServerNodeDef {
    pub fn default_nodes() -> Vec<Self> {
        DEFAULT_NODES
            .iter()
            .map(|(id, name, dt)| Self {
                node_id: id.to_string(),
                browse_name: name.to_string(),
                data_type: dt.to_string(),
            })
            .collect()
    }
}

fn parse_node_id(s: &str, ns_index: u16) -> Result<NodeId> {
    if let Some(rest) = s.strip_prefix("ns=") {
        if let Some((_ns_str, id_part)) = rest.split_once(';') {
            if let Some(id_str) = id_part.strip_prefix("s=") {
                return Ok(NodeId::new(ns_index, id_str));
            }
            if let Some(id_str) = id_part.strip_prefix("i=") {
                if let Ok(id) = id_str.parse::<u32>() {
                    return Ok(NodeId::new(ns_index, id));
                }
            }
        }
    }
    Ok(NodeId::new(ns_index, s))
}

fn default_variant(data_type: &str) -> Variant {
    match data_type {
        "Boolean" => Variant::Boolean(false),
        "Double" => Variant::Double(0.0),
        "Float" => Variant::Float(0.0),
        "Int32" => Variant::Int32(0),
        "UInt32" => Variant::UInt32(0),
        "String" => Variant::String(opcua::types::UAString::from("")),
        _ => Variant::Double(0.0),
    }
}

/// Write a node value through the node manager so active subscriptions receive
/// a data-change notification. Direct address-space mutation bypasses the
/// subscription cache, leaving monitored values static on clients.
fn write_node_value(
    node_manager: &Arc<SimpleNodeManager>,
    subscriptions: &opcua::server::SubscriptionCache,
    node_id: &NodeId,
    variant: Variant,
) -> Result<()> {
    node_manager
        .set_value(
            subscriptions,
            node_id,
            Some(&NumericRange::None),
            DataValue {
                value: Some(variant),
                status: Some(opcua::types::StatusCode::Good),
                source_timestamp: Some(DateTime::now()),
                ..Default::default()
            },
        )
        .map_err(|e| anyhow!("Write failed: {e}"))
}

fn data_type_id(data_type: &str) -> NodeId {
    match data_type {
        "Boolean" => DataTypeId::Boolean.into(),
        "Double" => DataTypeId::Double.into(),
        "Float" => DataTypeId::Float.into(),
        "Int32" => DataTypeId::Int32.into(),
        "UInt32" => DataTypeId::UInt32.into(),
        "String" => DataTypeId::String.into(),
        _ => DataTypeId::Double.into(),
    }
}

fn variant_to_json(v: &Variant) -> serde_json::Value {
    match v {
        Variant::Boolean(b) => serde_json::Value::Bool(*b),
        Variant::Double(d) => serde_json::json!(*d),
        Variant::Float(f) => serde_json::json!(*f),
        Variant::Int32(i) => serde_json::json!(*i),
        Variant::UInt32(u) => serde_json::json!(*u),
        Variant::String(s) => serde_json::json!(s.as_ref()),
        Variant::Int64(i) => serde_json::json!(*i),
        Variant::UInt64(u) => serde_json::json!(*u),
        Variant::Int16(i) => serde_json::json!(*i),
        Variant::UInt16(u) => serde_json::json!(*u),
        Variant::SByte(b) => serde_json::json!(*b),
        Variant::Byte(b) => serde_json::json!(*b),
        Variant::Empty => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    }
}

pub struct OpcUaServer {
    handle: Option<ServerHandle>,
    node_manager: Option<Arc<SimpleNodeManager>>,
    address_space: Option<Arc<RwLock<AddressSpace>>>,
    ns_index: u16,
}

impl OpcUaServer {
    pub fn new() -> Self {
        Self {
            handle: None,
            node_manager: None,
            address_space: None,
            ns_index: 2,
        }
    }

    pub fn is_running(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(|h| !h.token().is_cancelled())
    }

    /// Return a writer closure that pushes node values through the node manager
    /// so active subscriptions get data-change notifications. Safe to use from
    /// a background task; the server handle stays valid via the captured Arc's.
    pub fn node_writer(
        &self,
    ) -> Option<Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>> {
        let (node_manager, subscriptions) = self.writer_handle()?;
        let ns_index = self.ns_index;
        Some(Arc::new(move |node_id_str, value| {
            let Ok(node_id) = parse_node_id(node_id_str, ns_index) else { return };
            let variant = match value {
                serde_json::Value::Bool(b) => Variant::Boolean(b),
                v => Variant::Double(v.as_f64().unwrap_or(0.0)),
            };
            let _ = write_node_value(&node_manager, &subscriptions, &node_id, variant);
        }))
    }

    /// Clone the pieces needed to write a node value from outside the lock:
    /// the node manager (subscription-aware writes) and the subscription cache.
    pub fn writer_handle(&self) -> Option<(Arc<SimpleNodeManager>, Arc<opcua::server::SubscriptionCache>)> {
        let node_manager = self.node_manager.as_ref()?.clone();
        let subscriptions = self.handle.as_ref()?.subscriptions().clone();
        Some((node_manager, subscriptions))
    }

    pub async fn start(&mut self, port: u16, nodes: Vec<ServerNodeDef>) -> Result<String> {
        if self.is_running() {
            return Err(anyhow!("Server already running"));
        }

        let endpoint_url = format!("opc.tcp://localhost:{port}");

        let ns_meta = NamespaceMetadata {
            namespace_uri: "urn:opcua-explorer:nodes".to_owned(),
            ..Default::default()
        };

        let pki_dir = crate::pki::server_dir(port);
        let user_token_ids = vec![opcua::server::ANONYMOUS_USER_TOKEN_ID];

        let server_builder = ServerBuilder::new()
            .application_name("OPC UA Explorer")
            .application_uri("urn:opcua-explorer")
            .create_sample_keypair(true)
            .pki_dir(pki_dir)
            .host("localhost")
            .port(port)
            .trust_client_certs(true)
            .discovery_urls(vec![endpoint_url.clone()])
            .add_endpoint(
                "none",
                (
                    "/",
                    SecurityPolicy::None,
                    opcua::types::MessageSecurityMode::None,
                    &user_token_ids as &[&str],
                ),
            )
            .add_endpoint(
                "sign",
                (
                    "/",
                    SecurityPolicy::Basic256Sha256,
                    opcua::types::MessageSecurityMode::Sign,
                    &user_token_ids as &[&str],
                ),
            )
            .add_endpoint(
                "sign_encrypt",
                (
                    "/",
                    SecurityPolicy::Basic256Sha256,
                    opcua::types::MessageSecurityMode::SignAndEncrypt,
                    &user_token_ids as &[&str],
                ),
            )
            .with_node_manager(
                InMemoryNodeManagerBuilder::new(
                    SimpleNodeManagerBuilder::new(ns_meta, "explorer"),
                ),
            );

        let (server, handle) = server_builder
            .build()
            .map_err(|e| anyhow!("Server build failed: {e}"))?;

        let node_manager: Arc<SimpleNodeManager> = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .ok_or_else(|| anyhow!("Failed to find SimpleNodeManager"))?;

        let address_space = node_manager.address_space().clone();

        let ns_index = handle
            .get_namespace_index("urn:opcua-explorer:nodes")
            .ok_or_else(|| anyhow!("Failed to find namespace index"))?;

        self.ns_index = ns_index;

        populate_nodes(&address_space, ns_index, &handle, &nodes);

        self.address_space = Some(address_space);
        self.node_manager = Some(node_manager);
        self.handle = Some(handle);

        // Bind IPv6 wildcard (dual-stack): accepts both ::1 and 127.0.0.1.
        // macOS resolves "localhost" to ::1 first, so an IPv4-only 0.0.0.0 bind
        // would reject discovery/connection attempts through that name.
        let listener = TcpListener::bind(format!("[::]:{port}"))
            .await
            .map_err(|e| anyhow!("Bind failed: {e}"))?;

        let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let actual_url = format!("opc.tcp://localhost:{actual_port}");

        eprintln!("[OPCUA-SERVER] Listening on [::]:{actual_port}, endpoint URL: {actual_url}");
        eprintln!("[OPCUA-SERVER] Host: localhost, NS index: {ns_index}");

        tokio::task::spawn(server.run_with(listener));

        Ok(actual_url)
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(handle) = &self.handle {
            handle.cancel();
            self.handle = None;
            self.node_manager = None;
            self.address_space = None;
            Ok(())
        } else {
            Err(anyhow!("Server not running"))
        }
    }

    /// Write a node value through the node manager so active subscriptions get
    /// a data-change notification (direct address-space mutation would not).
    pub fn write_node(&self, node_id_str: &str, value: serde_json::Value, data_type: &str) -> Result<()> {
        let (node_manager, subscriptions) = self
            .writer_handle()
            .ok_or_else(|| anyhow!("Server not running"))?;
        let node_id = parse_node_id(node_id_str, self.ns_index)?;
        let variant = json_to_variant(value, data_type)?;
        write_node_value(&node_manager, &subscriptions, &node_id, variant)
    }

    pub fn read_all_nodes(&self, node_defs: &[ServerNodeDef]) -> Vec<OpcUaValue> {
        let Some(address_space) = &self.address_space else {
            return Vec::new();
        };

        let sp = address_space.read();
        let now = crate::store::opc_ticks_to_unix_ms(DateTime::now().checked_ticks());

        node_defs
            .iter()
            .filter_map(|def| {
                let node_id = parse_node_id(&def.node_id, self.ns_index).ok()?;
                let node = sp.find(&node_id)?;

                match node {
                    NodeType::Variable(v) => {
                        let dv = v.value(
                            TimestampsToReturn::Source,
                            &NumericRange::None,
                            &DataEncoding::Binary,
                            0.0,
                        );
                        let val = dv
                            .value
                            .as_ref()
                            .map(|v| variant_to_json(v))
                            .unwrap_or(serde_json::Value::Null);
                        Some(OpcUaValue {
                            node_id: def.node_id.clone(),
                            name: def.browse_name.clone(),
                            value: val,
                            timestamp: now,
                        })
                    }
                    _ => None,
                }
            })
            .collect()
    }

    pub fn address_space(&self) -> Option<Arc<RwLock<AddressSpace>>> {
        self.address_space.clone()
    }

    pub fn namespace_index(&self) -> u16 {
        self.ns_index
    }
}

fn json_to_variant(value: serde_json::Value, data_type: &str) -> Result<Variant> {
    let variant = match data_type {
        "Boolean" => Variant::Boolean(value.as_bool().ok_or_else(|| anyhow!("expected bool"))?),
        "Double" => Variant::Double(
            value
                .as_f64()
                .ok_or_else(|| anyhow!("expected number"))?,
        ),
        "Float" => Variant::Float(
            value
                .as_f64()
                .ok_or_else(|| anyhow!("expected number"))? as f32,
        ),
        "Int32" => Variant::Int32(
            value
                .as_i64()
                .ok_or_else(|| anyhow!("expected integer"))? as i32,
        ),
        "UInt32" => Variant::UInt32(
            value
                .as_u64()
                .ok_or_else(|| anyhow!("expected unsigned integer"))? as u32,
        ),
        "String" => Variant::String(opcua::types::UAString::from(
            value
                .as_str()
                .ok_or_else(|| anyhow!("expected string"))?,
        )),
        _ => Variant::Double(
            value
                .as_f64()
                .ok_or_else(|| anyhow!("expected number"))?,
        ),
    };
    Ok(variant)
}

fn populate_nodes(
    address_space: &Arc<RwLock<AddressSpace>>,
    ns_index: u16,
    handle: &ServerHandle,
    nodes: &[ServerNodeDef],
) {
    let mut sp = address_space.write();
    let type_tree = handle.type_tree().read();

    for node_def in nodes {
        let node_id = match parse_node_id(&node_def.node_id, ns_index) {
            Ok(id) => id,
            Err(_) => continue,
        };

        let dt_id = data_type_id(&node_def.data_type);
        let default_val = default_variant(&node_def.data_type);

        let var = VariableBuilder::new(&node_id, &node_def.browse_name, &node_def.browse_name)
            .data_type(dt_id)
            .value(default_val)
            .access_level(AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE)
            .user_access_level(AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE)
            .organized_by(ObjectId::ObjectsFolder)
            .has_type_definition(VariableTypeId::BaseDataVariableType)
            .build();

        let var_id = var.node_id().clone();
        sp.insert::<_, NodeId>(var, None);
        sp.insert_reference(
            &ObjectId::ObjectsFolder.into(),
            &var_id,
            ReferenceTypeId::Organizes,
        );
    }

    drop(type_tree);
}
