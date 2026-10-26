use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use opcua::client::{Client, ClientBuilder, DataChangeCallback, IdentityToken, Session};
use opcua::types::{
    ApplicationDescription, AttributeId, BrowseDescription, BrowseDirection, BrowseResultMask,
    DataValue, DateTime, EndpointDescription, ExtensionObject, MessageSecurityMode,
    MonitoringMode, MonitoringParameters, MonitoredItemCreateRequest, NodeClass, NodeClassMask,
    NodeId, NumericRange, ObjectId, ReadValueId, ReferenceTypeId, StatusCode,
    TimestampsToReturn, UserTokenPolicy, Variant, WriteValue, DataTypeId,
    DataChangeFilter, DataChangeTrigger, EUInformation, Range,
};

use crate::types::{
    AttributeRead, BrowseChild, EndpointInfo, MonitoredItemDef, NamespaceInfo, NotificationValue,
    OpcUaConfig, OpcUaNode, OpcUaValue, ReferenceInfo, ServerInfo, SubscriptionConfig,
};

mod explore;

pub use explore::{
    ArgumentInfo, BrowseNode, UaQuality, UaValue, UserIdentity, VariableInfo, canonical_node_id, json_variant,
    type_name, variant_text,
};

/// The threads every client session's event loop runs on, made on first use.
static EVENT_LOOP_RUNTIME: std::sync::LazyLock<Option<tokio::runtime::Runtime>> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("opcua-client")
            .enable_all()
            .build()
            .ok()
    });

fn event_loop_runtime() -> Option<&'static tokio::runtime::Runtime> {
    EVENT_LOOP_RUNTIME.as_ref()
}

/// Receives live subscription data-change notifications out of the OPC UA session
/// thread. Implemented by the Tauri layer (which forwards to a webview event);
/// the crate stays free of any Tauri dependency.
pub trait NotificationSink: Send + Sync {
    fn notify(&self, value: NotificationValue);
}

fn parse_node_id(s: &str) -> Result<NodeId> {
    // Every form the standard prints (`i=85`, `ns=2;s=Pump`, `ns=1;g=…`, `ns=1;b=…`).
    if let Ok(id) = <NodeId as std::str::FromStr>::from_str(s.trim()) {
        return Ok(id);
    }
    let rest = s.strip_prefix("ns=").ok_or_else(|| anyhow!("missing ns="))?;
    let (ns_str, id_part) = rest.split_once(';').ok_or_else(|| anyhow!("missing ;"))?;
    let ns: u16 = ns_str.parse()?;
    if let Some(id_str) = id_part.strip_prefix("s=") {
        Ok(NodeId::new(ns, id_str))
    } else if let Some(id_str) = id_part.strip_prefix('s') {
        Ok(NodeId::new(ns, id_str))
    } else if let Some(id_str) = id_part.strip_prefix("i=") {
        let id: u32 = id_str.parse()?;
        Ok(NodeId::new(ns, id))
    } else if let Some(id_str) = id_part.strip_prefix('i') {
        let id: u32 = id_str.parse()?;
        Ok(NodeId::new(ns, id))
    } else {
        Ok(NodeId::new(ns, id_part))
    }
}

/// Resolve a root node id; empty / "Objects" maps to the ObjectsFolder.
fn parse_root_node_id(s: &str) -> Result<NodeId> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("objects") {
        return Ok(ObjectId::ObjectsFolder.into());
    }
    parse_node_id(trimmed)
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
        Variant::DateTime(dt) => serde_json::json!(dt.checked_ticks()),
        Variant::Empty => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    }
}

/// Format an attribute property value (EURange / EngineeringUnits / Hi-Lo limits etc.)
/// into a friendly JSON representation. Range and EUInformation arrive as
/// ExtensionObjects wrapping the generated struct types; downcast them so the
/// browser panel shows structured data instead of `null`.
fn format_property_value(name: &str, value: Option<&Variant>) -> serde_json::Value {
    let Some(v) = value else { return serde_json::Value::Null };
    match v {
        Variant::ExtensionObject(ext) => {
            if let Some(range) = ext.inner_as::<Range>() {
                serde_json::json!({ "low": range.low, "high": range.high })
            } else if let Some(eu) = ext.inner_as::<EUInformation>() {
                serde_json::json!({
                    "unit_id": eu.unit_id,
                    "display_name": eu.display_name.text.as_ref(),
                    "namespace_uri": eu.namespace_uri.as_ref(),
                    "description": eu.description.text.as_ref(),
                })
            } else if let Some(argument) = ext.inner_as::<opcua::types::Argument>() {
                // Input/OutputArguments: `Name: Type`.
                serde_json::json!(format!(
                    "{}: {}",
                    argument.name.as_ref(),
                    explore::type_name(&argument.data_type)
                ))
            } else {
                variant_to_json(v)
            }
        }
        Variant::Variant(inner) => format_property_value(name, Some(inner)),
        Variant::Array(arr) => serde_json::json!(
            arr.values.iter().map(|x| format_property_value(name, Some(x))).collect::<Vec<_>>()
        ),
        _ => variant_to_json(v),
    }
}

fn json_to_variant(value: &serde_json::Value, data_type: &str) -> Result<Variant> {
    match data_type {
        "Boolean" => Ok(Variant::Boolean(value.as_bool().ok_or_else(|| anyhow!("expected bool"))?)),
        "Double" => Ok(Variant::Double(value.as_f64().ok_or_else(|| anyhow!("expected number"))?)),
        "Float" => Ok(Variant::Float(value.as_f64().ok_or_else(|| anyhow!("expected number"))? as f32)),
        "Int32" => Ok(Variant::Int32(value.as_i64().ok_or_else(|| anyhow!("expected integer"))? as i32)),
        "UInt32" => Ok(Variant::UInt32(value.as_u64().ok_or_else(|| anyhow!("expected unsigned integer"))? as u32)),
        "Int16" => Ok(Variant::Int16(value.as_i64().ok_or_else(|| anyhow!("expected integer"))? as i16)),
        "UInt16" => Ok(Variant::UInt16(value.as_u64().ok_or_else(|| anyhow!("expected unsigned integer"))? as u16)),
        "Int64" => Ok(Variant::Int64(value.as_i64().ok_or_else(|| anyhow!("expected integer"))?)),
        "UInt64" => Ok(Variant::UInt64(value.as_u64().ok_or_else(|| anyhow!("expected unsigned integer"))?)),
        "String" => Ok(Variant::String(opcua::types::UAString::from(
            value.as_str().ok_or_else(|| anyhow!("expected string"))?,
        ))),
        _ => Ok(Variant::Double(value.as_f64().ok_or_else(|| anyhow!("expected number"))?)),
    }
}

fn data_type_from_node_id(node_id: &NodeId) -> &'static str {
    match node_id.as_data_type_id() {
        Ok(DataTypeId::Boolean) => "Boolean",
        Ok(DataTypeId::Double) => "Double",
        Ok(DataTypeId::Float) => "Float",
        Ok(DataTypeId::Int32) => "Int32",
        Ok(DataTypeId::UInt32) => "UInt32",
        Ok(DataTypeId::Int16) => "Int16",
        Ok(DataTypeId::UInt16) => "UInt16",
        Ok(DataTypeId::Int64) => "Int64",
        Ok(DataTypeId::UInt64) => "UInt64",
        Ok(DataTypeId::String) => "String",
        Ok(DataTypeId::SByte) => "SByte",
        Ok(DataTypeId::Byte) => "Byte",
        Ok(DataTypeId::DateTime) => "DateTime",
        _ => "Unknown",
    }
}

fn node_class_to_string(nc: NodeClass) -> &'static str {
    match nc {
        NodeClass::Object => "Object",
        NodeClass::Variable => "Variable",
        NodeClass::Method => "Method",
        NodeClass::ObjectType => "ObjectType",
        NodeClass::VariableType => "VariableType",
        NodeClass::ReferenceType => "ReferenceType",
        NodeClass::DataType => "DataType",
        NodeClass::View => "View",
        _ => "Unspecified",
    }
}

fn node_class_from_i32(v: i32) -> &'static str {
    match v {
        1 => "Object",
        2 => "Variable",
        4 => "Method",
        8 => "ObjectType",
        16 => "VariableType",
        32 => "ReferenceType",
        64 => "DataType",
        128 => "View",
        _ => "Unspecified",
    }
}

/// Map a human security-policy name (or canonical URI) to its canonical URI string.
fn policy_uri(s: &str) -> &'static str {
    let lower = s.trim().to_ascii_lowercase();
    match lower.as_str() {
        "" | "none" => opcua::types::constants::SECURITY_POLICY_NONE_URI,
        "basic128rsa15" | "http://opcfoundation.org/ua/securitypolicy#basic128rsa15" => {
            "http://opcfoundation.org/UA/SecurityPolicy#Basic128Rsa15"
        }
        "basic256" | "http://opcfoundation.org/ua/securitypolicy#basic256" => {
            "http://opcfoundation.org/UA/SecurityPolicy#Basic256"
        }
        "basic256sha256" | "http://opcfoundation.org/ua/securitypolicy#basic256sha256" => {
            "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256"
        }
        "aes128sha256rsaoaep" | "aes128_sha256_rsaoaep"
            | "http://opcfoundation.org/ua/securitypolicy#aes128_sha256_rsaoaep" => {
            "http://opcfoundation.org/UA/SecurityPolicy#Aes128_Sha256_RsaOaep"
        }
        "aes256sha256rsapss" | "aes256_sha256_rsapss"
            | "http://opcfoundation.org/ua/securitypolicy#aes256_sha256_rsapss" => {
            "http://opcfoundation.org/UA/SecurityPolicy#Aes256_Sha256_RsaPss"
        }
        _ => opcua::types::constants::SECURITY_POLICY_NONE_URI,
    }
}

fn parse_mode(s: &str) -> MessageSecurityMode {
    match s.trim().to_ascii_lowercase().as_str() {
        "sign" => MessageSecurityMode::Sign,
        "signandencrypt" | "sign_and_encrypt" | "signencrypt" | "encrypt" => {
            MessageSecurityMode::SignAndEncrypt
        }
        _ => MessageSecurityMode::None,
    }
}

/// Best-effort readable name for a well-known reference-type node id (ns=0 numeric).
fn reference_type_name(node_id: &NodeId) -> String {
    let fallback = node_id.to_string();
    if let Some(id) = node_id.as_u32() {
        if node_id.namespace == 0 {
            let name = match id {
                31 => "References",
                32 => "NonHierarchicalReferences",
                33 => "HierarchicalReferences",
                34 => "HasChild",
                35 => "Organizes",
                36 => "HasEventSource",
                37 => "HasModellingRule",
                38 => "HasEncoding",
                39 => "HasDescription",
                40 => "HasTypeDefinition",
                41 => "GeneratesEvent",
                44 => "Aggregates",
                45 => "HasSubtype",
                46 => "HasProperty",
                47 => "HasComponent",
                48 => "HasOrderedComponent",
                49 => "HasNotifier",
                56 => "HasInterface",
                _ => return fallback,
            };
            return name.to_string();
        }
    }
    fallback
}

fn datetime_to_ms(dt: &DateTime) -> u64 {
    crate::store::opc_ticks_to_unix_ms(dt.checked_ticks())
}

pub struct OpcUaProtocolClient {
    session: Option<Arc<Session>>,
    _event_loop: Option<tokio::task::JoinHandle<StatusCode>>,
    config: Option<OpcUaConfig>,
    notification_sink: Option<Arc<dyn NotificationSink>>,
    subscription_id: Option<u32>,
}

/// A client dropped mid-connect or after a failed step stops its event loop, so the server's
/// session does not outlive it.
impl Drop for OpcUaProtocolClient {
    fn drop(&mut self) {
        if let Some(handle) = self._event_loop.take() {
            handle.abort();
        }
    }
}

impl OpcUaProtocolClient {
    pub fn new() -> Self {
        Self {
            session: None,
            _event_loop: None,
            config: None,
            notification_sink: None,
            subscription_id: None,
        }
    }

    pub fn set_notification_sink(&mut self, sink: Arc<dyn NotificationSink>) {
        self.notification_sink = Some(sink);
    }

    /// Build a throw-away client used for discovery (no session).
    fn discovery_client() -> Result<Client, String> {
        let pki_dir = crate::pki::client_dir();
        ClientBuilder::new()
            .application_name("OPC UA Explorer")
            .application_uri("urn:opcua-explorer-client")
            .pki_dir(pki_dir)
            .create_sample_keypair(true)
            .trust_server_certs(true)
            .session_retry_limit(0)
            .client()
            .map_err(|e| format!("Failed to create discovery client: {e:?}"))
    }

    pub async fn connect(&mut self, config: OpcUaConfig) -> Result<(), String> {
        let identity_token = match (&config.username, &config.password) {
            (Some(user), Some(pass)) if !user.trim().is_empty() => {
                IdentityToken::new_user_name(user.clone(), pass.clone())
            }
            _ => IdentityToken::Anonymous,
        };
        self.connect_with_token(config, identity_token).await
    }

    /// Connect with an explicit identity token (anonymous, user name or X.509).
    async fn connect_with_token(
        &mut self,
        config: OpcUaConfig,
        identity_token: IdentityToken,
    ) -> Result<(), String> {
        let pki_dir = crate::pki::client_dir();
        Self::prepare_certificate().await?;
        let endpoint_url = config.endpoint_url.clone();
        let policy_uri = policy_uri(&config.security_policy);
        let security_mode = parse_mode(&config.security_mode);

        eprintln!(
            "[OPCUA-CLIENT] Connecting to {endpoint_url} ({policy_uri} / {security_mode:?})"
        );

        let mut client = ClientBuilder::new()
            .application_name("OPC UA Explorer Client")
            .application_uri("urn:opcua-explorer-client")
            .pki_dir(pki_dir)
            .create_sample_keypair(true)
            .trust_server_certs(true)
            .session_retry_limit(0)
            .client()
            .map_err(|e| {
                eprintln!("[OPCUA-CLIENT] Failed to create client: {e:?}");
                format!("Failed to create client: {e:?}")
            })?;

        eprintln!("[OPCUA-CLIENT] Client created, resolving matching endpoint...");

        // Carry the user-token policy explicitly. The bare-URL `From<&str>`
        // shortcut omits `user_identity_tokens`, so the client cannot match its
        // identity token at ActivateSession time and rejects with
        // BadSecurityPolicyRejected. `connect_to_matching_endpoint` then fetches
        // the server's actual endpoints and selects the one matching our
        // requested security policy + mode.
        let endpoint_desc = EndpointDescription::from((
            endpoint_url.as_str(),
            policy_uri,
            security_mode,
            UserTokenPolicy::anonymous(),
        ));

        let (session, event_loop) = client
            .connect_to_matching_endpoint(endpoint_desc, identity_token)
            .await
            .map_err(|e| {
                eprintln!("[OPCUA-CLIENT] Failed to connect: {e}");
                format!("Failed to connect: {e}")
            })?;

        eprintln!("[OPCUA-CLIENT] Session built, spawning event loop...");

        // The session's I/O runs on threads of its own, so it never shares a worker with the
        // caller's tasks.
        let _handle = match event_loop_runtime() {
            Some(runtime) => runtime.spawn(event_loop.run()),
            None => event_loop.spawn(),
        };

        eprintln!("[OPCUA-CLIENT] Event loop spawned, waiting for connection (10s timeout)...");

        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            session.wait_for_connection(),
        )
        .await
        {
            Ok(true) => {
                eprintln!("[OPCUA-CLIENT] Connected successfully!");
            }
            Ok(false) => {
                eprintln!("[OPCUA-CLIENT] wait_for_connection returned false");
                // The event loop ends with the status that stopped it, e.g. a refused identity.
                let reason = match tokio::time::timeout(std::time::Duration::from_secs(1), _handle).await {
                    Ok(Ok(status)) if !status.is_good() => format!(" ({status})"),
                    _ => String::new(),
                };
                return Err(format!("Connection failed — session reported not connected{reason}"));
            }
            Err(_) => {
                _handle.abort();
                eprintln!("[OPCUA-CLIENT] Connection timeout after 10s");
                return Err(
                    "Connection timeout after 10s — ensure server is reachable and the \
                     security policy/mode matches an endpoint it offers"
                        .to_string(),
                );
            }
        }

        self.session = Some(session);
        self._event_loop = Some(_handle);
        self.config = Some(config);

        Ok(())
    }

    /// The client's application instance certificate, made off the async workers and without
    /// resolving the host name (the stack's own sample certificate does both). Making its RSA key
    /// can take seconds (far longer in a debug build), so a caller that bounds discovery or a
    /// connection by a timeout calls this first: the wait is then for the server alone.
    pub async fn prepare_certificate() -> Result<(), String> {
        crate::pki::ensure_own_certificate(
            crate::pki::client_dir(),
            "OPC UA Explorer Client",
            "urn:opcua-explorer-client",
        )
        .await
    }

    pub async fn get_endpoints(url: &str) -> Result<Vec<EndpointInfo>, String> {
        Self::prepare_certificate().await?;
        let client = Self::discovery_client()?;
        let endpoints = client
            .get_server_endpoints_from_url(url)
            .await
            .map_err(|e| format!("GetEndpoints failed: {e}"))?;

        Ok(endpoints
            .iter()
            .map(|e| EndpointInfo {
                url: e.endpoint_url.as_ref().to_string(),
                security_policy_uri: e.security_policy_uri.as_ref().to_string(),
                security_mode: format!("{:?}", e.security_mode),
                transport_profile_uri: e.transport_profile_uri.as_ref().to_string(),
                security_level: e.security_level,
                user_token_policies: e
                    .user_identity_tokens
                    .as_ref()
                    .map(|tokens| tokens.iter().map(|t| format!("{:?}", t.token_type)).collect())
                    .unwrap_or_default(),
            })
            .collect())
    }

    pub async fn find_servers(url: &str) -> Result<Vec<ServerInfo>, String> {
        Self::prepare_certificate().await?;
        let client = Self::discovery_client()?;
        let servers = client
            .find_servers(url, None, None)
            .await
            .map_err(|e| format!("FindServers failed: {e}"))?;

        Ok(servers
            .iter()
            .map(|s: &ApplicationDescription| ServerInfo {
                application_name: s.application_name.text.as_ref().to_string(),
                application_uri: s.application_uri.as_ref().to_string(),
                application_type: format!("{:?}", s.application_type),
                product_uri: s.product_uri.as_ref().to_string(),
                discovery_urls: s
                    .discovery_urls
                    .as_ref()
                    .map(|urls| urls.iter().map(|u| u.as_ref().to_string()).collect())
                    .unwrap_or_default(),
            })
            .collect())
    }

    fn session(&self) -> Result<&Arc<Session>, String> {
        self.session.as_ref().ok_or_else(|| "Not connected".to_string())
    }

    pub async fn browse_nodes(&self) -> Result<Vec<OpcUaNode>, String> {
        let session = self.session()?;

        let desc = BrowseDescription {
            node_id: ObjectId::ObjectsFolder.into(),
            browse_direction: BrowseDirection::Forward,
            reference_type_id: ReferenceTypeId::HierarchicalReferences.into(),
            include_subtypes: true,
            node_class_mask: NodeClassMask::VARIABLE.bits(),
            result_mask: BrowseResultMask::All as u32,
        };

        let browse_results = session
            .browse(&[desc], 1000, None)
            .await
            .map_err(|e| format!("Browse failed: {e}"))?;

        let mut nodes = Vec::new();

        for result in &browse_results {
            if let Some(refs) = &result.references {
                for r in refs {
                    let node_id = r.node_id.node_id.clone();
                    if node_id.is_null() {
                        continue;
                    }

                    let browse_name = r.browse_name.name.as_ref().to_string();

                    let dt_node_id = match session
                        .read(
                            &[ReadValueId {
                                node_id: node_id.clone(),
                                attribute_id: AttributeId::DataType as u32,
                                ..Default::default()
                            }],
                            TimestampsToReturn::Neither,
                            0.0,
                        )
                        .await
                    {
                        Ok(results) => results
                            .first()
                            .and_then(|dv| dv.value.as_ref())
                            .and_then(|v| match v {
                                Variant::NodeId(id) => Some(data_type_from_node_id(id)),
                                Variant::ExpandedNodeId(eid) => {
                                    Some(data_type_from_node_id(&eid.node_id))
                                }
                                _ => None,
                            })
                            .unwrap_or("Unknown"),
                        Err(_) => "Unknown",
                    };

                    nodes.push(OpcUaNode {
                        node_id: node_id.to_string(),
                        name: browse_name,
                        data_type: dt_node_id.to_string(),
                    });
                }
            }
        }

        Ok(nodes)
    }

    /// Single-level browse of all node classes under a node (recursive UI drives expansion).
    pub async fn browse_children(&self, node_id: &str) -> Result<Vec<BrowseChild>, String> {
        let session = self.session()?;
        let root = parse_root_node_id(node_id).map_err(|e| e.to_string())?;

        let desc = BrowseDescription {
            node_id: root,
            browse_direction: BrowseDirection::Forward,
            reference_type_id: ReferenceTypeId::HierarchicalReferences.into(),
            include_subtypes: true,
            node_class_mask: NodeClassMask::all().bits(),
            result_mask: BrowseResultMask::All as u32,
        };

        let browse_results = session
            .browse(&[desc], 1000, None)
            .await
            .map_err(|e| format!("Browse failed: {e}"))?;

        let mut out = Vec::new();
        if let Some(result) = browse_results.first() {
            if let Some(refs) = &result.references {
                for r in refs {
                    let nid = r.node_id.node_id.clone();
                    if nid.is_null() {
                        continue;
                    }
                    let is_variable = r.node_class == NodeClass::Variable;
                    let data_type = if is_variable {
                        match session
                            .read(
                                &[ReadValueId {
                                    node_id: nid.clone(),
                                    attribute_id: AttributeId::DataType as u32,
                                    ..Default::default()
                                }],
                                TimestampsToReturn::Neither,
                                0.0,
                            )
                            .await
                        {
                            Ok(results) => results
                                .first()
                                .and_then(|dv| dv.value.as_ref())
                                .and_then(|v| match v {
                                    Variant::NodeId(id) => Some(data_type_from_node_id(id)),
                                    Variant::ExpandedNodeId(eid) => {
                                        Some(data_type_from_node_id(&eid.node_id))
                                    }
                                    _ => None,
                                })
                                .unwrap_or("Unknown")
                                .to_string(),
                            Err(_) => "Unknown".to_string(),
                        }
                    } else {
                        String::new()
                    };

                    out.push(BrowseChild {
                        node_id: nid.to_string(),
                        browse_name: r.browse_name.name.as_ref().to_string(),
                        display_name: r.display_name.text.as_ref().to_string(),
                        node_class: node_class_to_string(r.node_class).to_string(),
                        data_type,
                        is_variable,
                    });
                }
            }
        }

        Ok(out)
    }

    /// Forward + inverse references for any node.
    pub async fn browse_references(&self, node_id: &str) -> Result<Vec<ReferenceInfo>, String> {
        let session = self.session()?;
        let root = parse_node_id(node_id).map_err(|e| format!("Invalid node ID: {e}"))?;

        let desc = BrowseDescription {
            node_id: root,
            browse_direction: BrowseDirection::Both,
            reference_type_id: ReferenceTypeId::References.into(),
            include_subtypes: true,
            node_class_mask: NodeClassMask::all().bits(),
            result_mask: BrowseResultMask::All as u32,
        };

        let browse_results = session
            .browse(&[desc], 1000, None)
            .await
            .map_err(|e| format!("Browse references failed: {e}"))?;

        let mut out = Vec::new();
        if let Some(result) = browse_results.first() {
            if let Some(refs) = &result.references {
                for r in refs {
                    let nid = r.node_id.node_id.clone();
                    if nid.is_null() {
                        continue;
                    }
                    out.push(ReferenceInfo {
                        reference_type: reference_type_name(&r.reference_type_id),
                        is_forward: r.is_forward,
                        node_id: nid.to_string(),
                        browse_name: r.browse_name.name.as_ref().to_string(),
                        display_name: r.display_name.text.as_ref().to_string(),
                        node_class: node_class_to_string(r.node_class).to_string(),
                    });
                }
            }
        }

        Ok(out)
    }

    /// Read the server NamespaceArray (ns=0;i=2255, Server_NamespaceArray) and map index → URI.
    pub async fn read_namespaces(&self) -> Result<Vec<NamespaceInfo>, String> {
        let session = self.session()?;
        let namespace_array = NodeId::new(0, 2255u32);

        let results = session
            .read(
                &[ReadValueId {
                    node_id: namespace_array,
                    attribute_id: AttributeId::Value as u32,
                    ..Default::default()
                }],
                TimestampsToReturn::Neither,
                0.0,
            )
            .await
            .map_err(|e| format!("Read NamespaceArray failed: {e}"))?;

        let mut out = vec![NamespaceInfo {
            index: 0,
            uri: "http://opcfoundation.org/UA/".to_string(),
        }];

        if let Some(dv) = results.first() {
            if let Some(v) = dv.value.as_ref() {
                if let Some(arr) = v.as_array() {
                    for (i, item) in arr.iter().enumerate() {
                        if let Variant::String(s) = item {
                            if i == 0 {
                                out[0].uri = s.as_ref().to_string();
                            } else {
                                out.push(NamespaceInfo {
                                    index: i as u16,
                                    uri: s.as_ref().to_string(),
                                });
                            }
                        }
                    }
                } else if let Variant::String(s) = v {
                    out[0].uri = s.as_ref().to_string();
                }
            }
        }

        Ok(out)
    }

    /// Read the common attributes of a node (Value, AccessLevel, BrowseName,
    /// DisplayName, Description, NodeClass, DataType).
    pub async fn read_attributes(&self, node_id: &str) -> Result<Vec<AttributeRead>, String> {
        let session = self.session()?;
        let nid = parse_node_id(node_id).map_err(|e| format!("Invalid node ID: {e}"))?;

        const ATTRS: &[(u32, &str)] = &[
            (AttributeId::Value as u32, "Value"),
            (AttributeId::AccessLevel as u32, "AccessLevel"),
            (AttributeId::UserAccessLevel as u32, "UserAccessLevel"),
            (AttributeId::BrowseName as u32, "BrowseName"),
            (AttributeId::DisplayName as u32, "DisplayName"),
            (AttributeId::Description as u32, "Description"),
            (AttributeId::NodeClass as u32, "NodeClass"),
            (AttributeId::DataType as u32, "DataType"),
        ];

        let nodes_to_read: Vec<ReadValueId> = ATTRS
            .iter()
            .map(|(id, _)| ReadValueId {
                node_id: nid.clone(),
                attribute_id: *id,
                ..Default::default()
            })
            .collect();

        let results = session
            .read(&nodes_to_read, TimestampsToReturn::Neither, 0.0)
            .await
            .map_err(|e| format!("Read attributes failed: {e}"))?;

        let mut out = Vec::with_capacity(ATTRS.len());
        for ((_, name), dv) in ATTRS.iter().zip(results.iter()) {
            let status = dv
                .status
                .map(|s| if s.is_bad() { format!("{s}") } else { "Good".to_string() })
                .unwrap_or_else(|| "Good".to_string());
            let value = match (*name, dv.value.as_ref()) {
                (_, None) => serde_json::Value::Null,
                ("BrowseName", Some(Variant::QualifiedName(qn))) => {
                    serde_json::json!(format!("{}:{}", qn.namespace_index, qn.name.as_ref()))
                }
                ("DisplayName", Some(Variant::LocalizedText(lt)))
                | ("Description", Some(Variant::LocalizedText(lt))) => {
                    serde_json::json!(lt.text.as_ref())
                }
                ("NodeClass", Some(Variant::Int32(i))) => {
                    serde_json::json!(node_class_from_i32(*i))
                }
                ("DataType", Some(Variant::NodeId(id))) => {
                    serde_json::json!(data_type_from_node_id(id))
                }
                ("DataType", Some(Variant::ExpandedNodeId(eid))) => {
                    serde_json::json!(data_type_from_node_id(&eid.node_id))
                }
                (_, Some(v)) => variant_to_json(v),
            };
            out.push(AttributeRead {
                attribute: (*name).to_string(),
                value,
                status,
            });
        }

        // Append HasProperty children: EURange, EngineeringUnits, Hi/Lo limits, etc.
        let prop_desc = BrowseDescription {
            node_id: nid.clone(),
            browse_direction: BrowseDirection::Forward,
            reference_type_id: ReferenceTypeId::HasProperty.into(),
            include_subtypes: false,
            node_class_mask: NodeClassMask::VARIABLE.bits(),
            result_mask: BrowseResultMask::All as u32,
        };
        if let Ok(results) = session.browse(&[prop_desc], 200, None).await {
            if let Some(result) = results.into_iter().next() {
                if let Some(refs) = result.references {
                    let prop_reads: Vec<ReadValueId> = refs.iter().filter_map(|r| {
                        let cid = r.node_id.node_id.clone();
                        if cid.is_null() { return None; }
                        Some(ReadValueId {
                            node_id: cid,
                            attribute_id: AttributeId::Value as u32,
                            ..Default::default()
                        })
                    }).collect();
                    let names: Vec<String> =
                        refs.iter().map(|r| r.browse_name.name.as_ref().to_string()).collect();
                    if let Ok(pvals) = session
                        .read(&prop_reads, TimestampsToReturn::Neither, 0.0)
                        .await
                    {
                        for (name, dv) in names.iter().zip(pvals.iter()) {
                            let pstatus = dv
                                .status
                                .map(|s| if s.is_bad() { format!("{s}") } else { "Good".to_string() })
                                .unwrap_or_else(|| "Good".to_string());
                            out.push(AttributeRead {
                                attribute: name.clone(),
                                value: format_property_value(name, dv.value.as_ref()),
                                status: pstatus,
                            });
                        }
                    }
                }
            }
        }

        Ok(out)
    }

    pub async fn read_tags(&self, tags: &[OpcUaNode]) -> Result<Vec<OpcUaValue>, String> {
        let session = self.session()?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let nodes_to_read: Vec<ReadValueId> = tags
            .iter()
            .filter_map(|tag| {
                let node_id = parse_node_id(&tag.node_id).ok()?;
                Some(ReadValueId {
                    node_id,
                    attribute_id: AttributeId::Value as u32,
                    ..Default::default()
                })
            })
            .collect();

        if nodes_to_read.is_empty() {
            return Ok(tags
                .iter()
                .map(|tag| OpcUaValue {
                    node_id: tag.node_id.clone(),
                    name: tag.name.clone(),
                    value: serde_json::Value::Null,
                    timestamp: now_ms,
                })
                .collect());
        }

        let results = session
            .read(&nodes_to_read, TimestampsToReturn::Both, 0.0)
            .await
            .map_err(|e| format!("Read failed: {e}"))?;

        let mut values = Vec::with_capacity(tags.len());
        let mut read_idx = 0;

        for tag in tags {
            if parse_node_id(&tag.node_id).is_err() {
                values.push(OpcUaValue {
                    node_id: tag.node_id.clone(),
                    name: tag.name.clone(),
                    value: serde_json::Value::Null,
                    timestamp: now_ms,
                });
                continue;
            }

            if read_idx < results.len() {
                let dv = &results[read_idx];
                let val = dv
                    .value
                    .as_ref()
                    .map(variant_to_json)
                    .unwrap_or(serde_json::Value::Null);
                values.push(OpcUaValue {
                    node_id: tag.node_id.clone(),
                    name: tag.name.clone(),
                    value: val,
                    timestamp: now_ms,
                });
                read_idx += 1;
            } else {
                values.push(OpcUaValue {
                    node_id: tag.node_id.clone(),
                    name: tag.name.clone(),
                    value: serde_json::Value::Null,
                    timestamp: now_ms,
                });
            }
        }

        Ok(values)
    }

    pub async fn write_tag(&self, tag: &OpcUaNode, value: &OpcUaValue) -> Result<(), String> {
        let session = self.session()?;

        let node_id = parse_node_id(&tag.node_id)
            .map_err(|e| format!("Invalid node ID: {e}"))?;

        let variant = json_to_variant(&value.value, &tag.data_type)
            .map_err(|e| format!("Value conversion failed: {e}"))?;

        let write_value = WriteValue {
            node_id,
            attribute_id: AttributeId::Value as u32,
            index_range: NumericRange::None,
            value: DataValue {
                value: Some(variant),
                status: Some(StatusCode::Good),
                source_timestamp: Some(DateTime::now()),
                ..Default::default()
            },
        };

        let results = session
            .write(&[write_value])
            .await
            .map_err(|e| format!("Write failed: {e}"))?;

        if let Some(status) = results.first() {
            if status.is_bad() {
                return Err(format!("Write rejected: {status}"));
            }
        }

        Ok(())
    }

    // ── Subscriptions & monitored items ──

    pub async fn create_subscription(&mut self, cfg: SubscriptionConfig) -> Result<u32, String> {
        let session = self.session()?.clone();
        let sink = self
            .notification_sink
            .clone()
            .ok_or_else(|| "No notification sink registered".to_string())?;

        let callback = DataChangeCallback::new(move |dv, item| {
            let node_id = item.item_to_monitor().node_id.to_string();
            let value = dv.value.as_ref().map(variant_to_json).unwrap_or_default();
            let status = dv
                .status
                .map(|s| if s.is_bad() { format!("{s}") } else { "Good".to_string() })
                .unwrap_or_else(|| "Good".to_string());
            let source_ts = dv.source_timestamp.map(|dt| datetime_to_ms(&dt)).unwrap_or(0);
            let server_ts = dv.server_timestamp.map(|dt| datetime_to_ms(&dt)).unwrap_or(0);
            sink.notify(NotificationValue {
                node_id,
                value,
                source_ts,
                server_ts,
                status,
            });
        });

        let sub_id = session
            .create_subscription(
                std::time::Duration::from_millis(cfg.publishing_interval_ms as u64),
                cfg.lifetime_count,
                cfg.max_keep_alive_count,
                cfg.max_notifications_per_publish,
                cfg.priority,
                cfg.publishing_enabled,
                callback,
            )
            .await
            .map_err(|e| format!("create_subscription failed: {e}"))?;

        self.subscription_id = Some(sub_id);
        Ok(sub_id)
    }

    pub async fn create_monitored_items(
        &self,
        subscription_id: u32,
        items: Vec<MonitoredItemDef>,
    ) -> Result<usize, String> {
        let session = self.session()?;

        let reqs: Vec<MonitoredItemCreateRequest> = items
            .into_iter()
            .filter_map(|m| {
                let node_id = parse_node_id(&m.node_id).ok()?;
                let mode = match m.monitoring_mode.trim().to_ascii_lowercase().as_str() {
                    "sampling" => MonitoringMode::Sampling,
                    "disabled" => MonitoringMode::Disabled,
                    _ => MonitoringMode::Reporting,
                };
                let mut params = MonitoringParameters::default();
                params.sampling_interval = m.sampling_interval_ms;
                params.queue_size = m.queue_size;
                params.discard_oldest = m.discard_oldest;
                params.client_handle = 0; // session assigns
                if m.trigger != "Status" || m.deadband_type != 0 {
                    let trigger = match m.trigger.trim().to_ascii_lowercase().as_str() {
                        "statusvalue" => DataChangeTrigger::StatusValue,
                        "statusvaluetimestamp" => DataChangeTrigger::StatusValueTimestamp,
                        _ => DataChangeTrigger::Status,
                    };
                    params.filter = ExtensionObject::new(DataChangeFilter {
                        trigger,
                        deadband_type: m.deadband_type,
                        deadband_value: m.deadband_value,
                    });
                }
                Some(MonitoredItemCreateRequest::new(
                    ReadValueId {
                        node_id,
                        attribute_id: m.attribute_id,
                        ..Default::default()
                    },
                    mode,
                    params,
                ))
            })
            .collect();

        if reqs.is_empty() {
            return Err("No valid monitored items".to_string());
        }

        let created = session
            .create_monitored_items(subscription_id, TimestampsToReturn::Both, reqs)
            .await
            .map_err(|e| format!("create_monitored_items failed: {e}"))?;

        Ok(created.len())
    }

    pub async fn delete_subscription(&mut self, subscription_id: u32) -> Result<(), String> {
        let session = self.session()?;
        session
            .delete_subscriptions(&[subscription_id])
            .await
            .map_err(|e| format!("delete_subscription failed: {e}"))?;
        if self.subscription_id == Some(subscription_id) {
            self.subscription_id = None;
        }
        Ok(())
    }

    pub fn subscription_id(&self) -> Option<u32> {
        self.subscription_id
    }

    pub fn disconnect(&mut self) -> Result<(), String> {
        self.session = None;
        // A detached event loop would keep the session open on the server.
        if let Some(handle) = self._event_loop.take() {
            handle.abort();
        }
        self.subscription_id = None;
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.session.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in event loop that reports, through `alive`, when it stops running.
    fn event_loop() -> (tokio::task::JoinHandle<StatusCode>, tokio::sync::oneshot::Receiver<()>) {
        let (alive, stopped) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _alive = alive;
            std::future::pending::<()>().await;
            StatusCode::Good
        });
        (handle, stopped)
    }

    #[tokio::test]
    async fn dropping_the_client_stops_its_event_loop() {
        let (handle, stopped) = event_loop();
        let mut client = OpcUaProtocolClient::new();
        client._event_loop = Some(handle);
        drop(client);
        // The aborted task drops its future, and with it the sender.
        tokio::time::timeout(std::time::Duration::from_secs(5), stopped)
            .await
            .expect("the event loop is aborted")
            .expect_err("the event loop never finishes on its own");
    }

    #[tokio::test]
    async fn disconnect_stops_the_event_loop() {
        let (handle, stopped) = event_loop();
        let mut client = OpcUaProtocolClient::new();
        client._event_loop = Some(handle);
        client.disconnect().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), stopped)
            .await
            .expect("the event loop is aborted")
            .expect_err("the event loop never finishes on its own");
    }
}
