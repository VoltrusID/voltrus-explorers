//! An in-process OPC UA server for a described plant: folders and objects holding variables (with
//! engineering units and ranges) and methods, anonymous, user-name and X.509 users, and None,
//! Basic256Sha256 and Aes256Sha256RsaPss endpoints. Values are set through the node manager, so
//! subscriptions see every change; writes from clients land in the address space, where
//! [`PlantServer::value`] reads them back.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use opcua::crypto::SecurityPolicy;
use opcua::server::address_space::{
    AccessLevel, AddressSpace, MethodBuilder, NodeType, ObjectBuilder, VariableBuilder,
};
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::{
    InMemoryNodeManagerBuilder, SimpleNodeManager, SimpleNodeManagerBuilder,
};
use opcua::server::{ANONYMOUS_USER_TOKEN_ID, ServerBuilder, ServerHandle, ServerUserToken};
use opcua::types::{
    Argument, DataEncoding, DataTypeId, DataValue, DateTime, EUInformation,
    ExtensionObject, LocalizedText, MessageSecurityMode, NodeId, NumericRange, ObjectId,
    ObjectTypeId, Range, StatusCode, TimestampsToReturn, UAString, VariableTypeId, Variant,
};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::opcua_client::{json_variant, variant_text};

/// The namespace every plant node lives in.
pub const PLANT_NAMESPACE: &str = "urn:voltrus:plant";
/// User token ids of the server's users.
const USER_TOKEN: &str = "operator";
const CERTIFICATE_TOKEN: &str = "operator-certificate";
/// How long a stopping server gets to let go of its port.
const STOP_WAIT: Duration = Duration::from_secs(2);

/// One node of the plant's tree, under the Objects folder.
#[derive(Debug, Clone, PartialEq)]
pub enum PlantNode {
    /// A FolderType object.
    Folder { key: String, name: String, children: Vec<PlantNode> },
    /// A BaseObjectType object (a machine), its variables and methods as components.
    Object { key: String, name: String, children: Vec<PlantNode> },
    Variable(PlantVariable),
    Method(PlantMethod),
}

/// A variable. Its node id is `ns=<plant>;s=<key>`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlantVariable {
    pub key: String,
    pub name: String,
    /// A built-in type name: `Boolean`, `Double`, `UInt32`, `String`, ...
    pub data_type: String,
    /// Engineering unit shown by clients (EngineeringUnits property); empty for none.
    pub unit: String,
    /// The EURange property: analog items get both.
    pub range: Option<(f64, f64)>,
    pub writable: bool,
    pub initial: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlantMethod {
    pub key: String,
    pub name: String,
    pub inputs: Vec<PlantArgument>,
    pub outputs: Vec<PlantArgument>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlantArgument {
    pub name: String,
    pub data_type: String,
    pub description: String,
}

/// Who may sign in.
#[derive(Debug, Clone, PartialEq)]
pub struct PlantServerConfig {
    pub port: u16,
    pub application_name: String,
    pub anonymous: bool,
    /// A user-name user; `None` turns user-name sign-in off.
    pub user: Option<(String, String)>,
    /// A user certificate (DER) whose holder may sign in; `None` turns certificate sign-in off.
    pub user_certificate: Option<PathBuf>,
}

/// Answers a method call: the method's key and its input arguments as JSON; the output arguments
/// as JSON (converted to the method's declared output types), or why the call is refused.
pub type MethodHandler =
    Arc<dyn Fn(&str, &[serde_json::Value]) -> Result<Vec<serde_json::Value>, String> + Send + Sync>;

pub struct PlantServer {
    handle: ServerHandle,
    values: PlantValues,
    namespace: u16,
    url: String,
    task: JoinHandle<Result<(), String>>,
}

/// Reads and sets the plant's variables. Cheap to clone. Its calls take the address space's lock,
/// so async callers run them on a blocking thread.
#[derive(Clone)]
pub struct PlantValues {
    handle: ServerHandle,
    node_manager: Arc<SimpleNodeManager>,
    namespace: u16,
}

/// Make a user certificate and private key (PEM) at the given paths unless they exist: what a
/// client signs in with to the server's certificate user. Blocking: call it off the async workers.
pub fn create_user_certificate(certificate: &Path, private_key: &Path, common_name: &str) -> Result<()> {
    crate::pki::ensure_certificate(
        certificate,
        private_key,
        common_name,
        &format!("urn:voltrus:user:{}", common_name.replace(' ', "-")),
    )
    .map_err(|e| anyhow!("Cannot create the user certificate: {e}"))
}

fn data_type_id(name: &str) -> NodeId {
    match name {
        "Boolean" => DataTypeId::Boolean,
        "SByte" => DataTypeId::SByte,
        "Byte" => DataTypeId::Byte,
        "Int16" => DataTypeId::Int16,
        "UInt16" => DataTypeId::UInt16,
        "Int32" => DataTypeId::Int32,
        "UInt32" => DataTypeId::UInt32,
        "Int64" => DataTypeId::Int64,
        "UInt64" => DataTypeId::UInt64,
        "Float" => DataTypeId::Float,
        "String" => DataTypeId::String,
        "DateTime" => DataTypeId::DateTime,
        _ => DataTypeId::Double,
    }
    .into()
}

fn argument(arg: &PlantArgument) -> Argument {
    Argument {
        name: UAString::from(arg.name.as_str()),
        data_type: data_type_id(&arg.data_type),
        value_rank: -1,
        array_dimensions: None,
        description: LocalizedText::new("", &arg.description),
    }
}

fn read_write(writable: bool) -> AccessLevel {
    if writable {
        AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE
    } else {
        AccessLevel::CURRENT_READ
    }
}

struct Builder<'a> {
    space: &'a mut AddressSpace,
    namespace: u16,
    /// Method key → (node id, output types), for the call handler.
    methods: Vec<(NodeId, String, Vec<String>)>,
}

impl Builder<'_> {
    fn id(&self, key: &str) -> NodeId {
        NodeId::new(self.namespace, key)
    }

    fn add(&mut self, node: &PlantNode, parent: &NodeId, organized: bool) {
        match node {
            PlantNode::Folder { key, name, children } | PlantNode::Object { key, name, children } => {
                let id = self.id(key);
                let builder = ObjectBuilder::new(&id, name.as_str(), name.as_str());
                let builder = if matches!(node, PlantNode::Folder { .. }) {
                    builder.is_folder()
                } else {
                    builder.has_type_definition(ObjectTypeId::BaseObjectType)
                };
                let builder = if organized {
                    builder.organized_by(parent.clone())
                } else {
                    builder.component_of(parent.clone())
                };
                builder.insert(&mut *self.space);
                let folder = matches!(node, PlantNode::Folder { .. });
                for child in children {
                    self.add(child, &id, folder);
                }
            }
            PlantNode::Variable(variable) => self.variable(variable, parent, organized),
            PlantNode::Method(method) => {
                let id = self.id(&method.key);
                let inputs: Vec<Argument> = method.inputs.iter().map(argument).collect();
                let outputs: Vec<Argument> = method.outputs.iter().map(argument).collect();
                let mut builder = MethodBuilder::new(&id, method.name.as_str(), method.name.as_str())
                    .component_of(parent.clone())
                    .executable(true)
                    .user_executable(true);
                if !inputs.is_empty() {
                    let args = self.id(&format!("{}.InputArguments", method.key));
                    builder = builder.input_args(&mut *self.space, &args, &inputs);
                }
                if !outputs.is_empty() {
                    let args = self.id(&format!("{}.OutputArguments", method.key));
                    builder = builder.output_args(&mut *self.space, &args, &outputs);
                }
                builder.insert(&mut *self.space);
                self.methods.push((
                    id,
                    method.key.clone(),
                    method.outputs.iter().map(|arg| arg.data_type.clone()).collect(),
                ));
            }
        }
    }

    fn variable(&mut self, variable: &PlantVariable, parent: &NodeId, organized: bool) {
        let id = self.id(&variable.key);
        let initial = json_variant(&variable.data_type, &variable.initial).unwrap_or(Variant::Empty);
        let analog = variable.range.is_some();
        let builder = VariableBuilder::new(&id, variable.name.as_str(), variable.name.as_str())
            .data_type(data_type_id(&variable.data_type))
            .value(initial)
            .access_level(read_write(variable.writable))
            .user_access_level(read_write(variable.writable))
            .has_type_definition(if analog {
                VariableTypeId::AnalogItemType
            } else {
                VariableTypeId::BaseDataVariableType
            });
        let builder = if organized {
            builder.organized_by(parent.clone())
        } else {
            builder.component_of(parent.clone())
        };
        builder.insert(&mut *self.space);
        if let Some((low, high)) = variable.range {
            VariableBuilder::new(&self.id(&format!("{}.EURange", variable.key)), "EURange", "EURange")
                .property_of(id.clone())
                .has_type_definition(VariableTypeId::PropertyType)
                .data_type(DataTypeId::Range)
                .value(Variant::from(ExtensionObject::from_message(Range { low, high })))
                .insert(&mut *self.space);
        }
        if !variable.unit.is_empty() {
            VariableBuilder::new(
                &self.id(&format!("{}.EngineeringUnits", variable.key)),
                "EngineeringUnits",
                "EngineeringUnits",
            )
            .property_of(id)
            .has_type_definition(VariableTypeId::PropertyType)
            .data_type(DataTypeId::EUInformation)
            .value(Variant::from(ExtensionObject::from_message(EUInformation {
                namespace_uri: UAString::from("http://www.opcfoundation.org/UA/units/un/cefact"),
                unit_id: -1,
                display_name: LocalizedText::new("", &variable.unit),
                description: LocalizedText::new("", &variable.unit),
            })))
            .insert(&mut *self.space);
        }
    }
}

/// Bind every interface: IPv6 and IPv4 where the system has both (macOS resolves `localhost` to
/// `::1` first), IPv4 alone otherwise.
async fn bind(port: u16) -> Result<TcpListener> {
    match TcpListener::bind(("::", port)).await {
        Ok(listener) => Ok(listener),
        Err(v6) => TcpListener::bind(("0.0.0.0", port)).await.map_err(|v4| {
            if v4.kind() == std::io::ErrorKind::AddrInUse || v6.kind() == std::io::ErrorKind::AddrInUse {
                anyhow!("TCP port {port} is in use by another program")
            } else {
                anyhow!("Cannot listen on TCP port {port}: {v4}")
            }
        }),
    }
}

impl PlantServer {
    /// Build the address space and listen on `config.port` (every interface). Fails, without
    /// panicking, when the port is taken or the PKI folder cannot be written.
    pub async fn start(config: PlantServerConfig, nodes: &[PlantNode], on_call: MethodHandler) -> Result<Self> {
        let listener = bind(config.port).await?;
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(config.port);
        // The server's own certificate, made here without resolving the host name.
        crate::pki::ensure_own_certificate(
            crate::pki::server_dir(port),
            &config.application_name,
            "urn:voltrus:tools:plant",
        )
        .await
        .map_err(|e| anyhow!(e))?;
        let url = format!("opc.tcp://localhost:{port}");

        let mut tokens: Vec<&str> = Vec::new();
        if config.anonymous {
            tokens.push(ANONYMOUS_USER_TOKEN_ID);
        }
        let mut builder = ServerBuilder::new()
            .application_name(config.application_name.as_str())
            .application_uri("urn:voltrus:tools:plant")
            .product_uri("urn:voltrus:tools")
            .create_sample_keypair(true)
            .pki_dir(crate::pki::server_dir(port))
            .host("localhost")
            .port(port)
            .trust_client_certs(true)
            .discovery_urls(vec![url.clone()]);
        if let Some((user, password)) = &config.user {
            builder = builder.add_user_token(USER_TOKEN, ServerUserToken::user_pass(user.as_str(), password.as_str()));
            tokens.push(USER_TOKEN);
        }
        if let Some(certificate) = &config.user_certificate {
            builder = builder.add_user_token(CERTIFICATE_TOKEN, ServerUserToken::x509("operator", certificate));
            tokens.push(CERTIFICATE_TOKEN);
        }
        if tokens.is_empty() {
            return Err(anyhow!("Allow anonymous clients or give the server a user"));
        }
        let endpoints = [
            ("none", SecurityPolicy::None, MessageSecurityMode::None),
            ("basic256sha256_sign", SecurityPolicy::Basic256Sha256, MessageSecurityMode::Sign),
            ("basic256sha256_sign_encrypt", SecurityPolicy::Basic256Sha256, MessageSecurityMode::SignAndEncrypt),
            ("aes256sha256rsapss_sign_encrypt", SecurityPolicy::Aes256Sha256RsaPss, MessageSecurityMode::SignAndEncrypt),
        ];
        for (id, policy, mode) in endpoints {
            builder = builder.add_endpoint(id, ("/", policy, mode, tokens.as_slice()));
        }
        let (server, handle) = builder
            .with_node_manager(InMemoryNodeManagerBuilder::new(SimpleNodeManagerBuilder::new(
                NamespaceMetadata { namespace_uri: PLANT_NAMESPACE.to_owned(), ..Default::default() },
                "plant",
            )))
            .build()
            .map_err(|e| anyhow!("Cannot build the server: {e}"))?;
        let node_manager: Arc<SimpleNodeManager> = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .ok_or_else(|| anyhow!("The server has no node manager"))?;
        let namespace = handle
            .get_namespace_index(PLANT_NAMESPACE)
            .ok_or_else(|| anyhow!("The server has no plant namespace"))?;

        let methods = {
            let address_space = node_manager.address_space().clone();
            let mut space = address_space.write();
            let mut builder = Builder { space: &mut *space, namespace, methods: Vec::new() };
            let objects: NodeId = ObjectId::ObjectsFolder.into();
            for node in nodes {
                builder.add(node, &objects, true);
            }
            builder.methods
        };
        for (id, key, outputs) in methods {
            let on_call = on_call.clone();
            node_manager.inner().add_method_callback(id, move |inputs: &[Variant]| {
                let inputs: Vec<serde_json::Value> = inputs.iter().map(variant_json).collect();
                let outputs_json =
                    on_call(key.as_str(), inputs.as_slice()).map_err(|_| StatusCode::BadInvalidArgument)?;
                outputs
                    .iter()
                    .zip(&outputs_json)
                    .map(|(data_type, value)| json_variant(data_type, value).map_err(|_| StatusCode::BadInternalError))
                    .collect()
            });
        }

        let task = tokio::spawn(server.run_with(listener));
        let values = PlantValues {
            handle: handle.clone(),
            node_manager,
            namespace,
        };
        Ok(Self { handle, values, namespace, url, task })
    }

    /// `opc.tcp://localhost:<port>`.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn namespace_index(&self) -> u16 {
        self.namespace
    }

    /// The node id of a plant key, as clients see it.
    pub fn node_id(&self, key: &str) -> String {
        NodeId::new(self.namespace, key).to_string()
    }

    /// A handle on the variables, for a blocking thread.
    pub fn values(&self) -> PlantValues {
        self.values.clone()
    }

    /// Stop serving and wait (briefly) for the port to be free again.
    pub async fn stop(self) {
        self.handle.cancel();
        let _ = tokio::time::timeout(STOP_WAIT, self.task).await;
    }
}

impl PlantValues {
    /// Set variables (key, JSON value converted to the given type) in one go; subscriptions are
    /// notified.
    pub fn set_values(&self, values: &[(&str, &str, serde_json::Value)]) -> Result<()> {
        let now = DateTime::now();
        let mut updates = Vec::with_capacity(values.len());
        for (key, data_type, value) in values {
            let variant = json_variant(data_type, value).map_err(|e| anyhow!("{key}: {e}"))?;
            updates.push((
                NodeId::new(self.namespace, *key),
                DataValue {
                    value: Some(variant),
                    status: Some(StatusCode::Good),
                    source_timestamp: Some(now),
                    server_timestamp: Some(now),
                    ..Default::default()
                },
            ));
        }
        self.node_manager
            .set_values(
                self.handle.subscriptions(),
                updates.iter().map(|(id, value)| (id, None, value.clone())),
            )
            .map_err(|e| anyhow!("Set failed: {e}"))
    }

    /// A variable's current value as a number (if it is one) and as text.
    pub fn value(&self, key: &str) -> Option<(Option<f64>, String)> {
        let address_space = self.node_manager.address_space().clone();
        let space = address_space.read();
        let id = NodeId::new(self.namespace, key);
        let found = match space.find(&id) {
            Some(NodeType::Variable(variable)) => variable
                .value(TimestampsToReturn::Neither, &NumericRange::None, &DataEncoding::Binary, 0.0)
                .value
                .map(|value| variant_text(&value)),
            _ => None,
        };
        found
    }
}

fn variant_json(value: &Variant) -> serde_json::Value {
    match value {
        Variant::Boolean(b) => serde_json::Value::Bool(*b),
        Variant::String(text) => serde_json::Value::String(text.as_ref().to_owned()),
        Variant::LocalizedText(text) => serde_json::Value::String(text.text.as_ref().to_owned()),
        other => match variant_text(other) {
            (Some(number), _) => serde_json::json!(number),
            (None, text) => serde_json::Value::String(text),
        },
    }
}
