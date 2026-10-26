//! Address-space exploration and typed I/O on top of [`OpcUaProtocolClient`]: connecting as an
//! anonymous, user-name or certificate identity, one browse level with every reference (continuation
//! points followed) and whether each child has children of its own, variables described in one round
//! trip, monitored items with their own sampling interval and per-item results, typed writes, and
//! method calls with their argument definitions.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use opcua::client::{DataChangeCallback, IdentityToken};
use opcua::types::{
    Argument, AttributeId, BrowseDescription, BrowseDirection, BrowseResult, BrowseResultMask,
    CallMethodRequest, DataTypeId, DataValue, DateTime, EUInformation, LocalizedText,
    MonitoredItemCreateRequest, MonitoringMode, MonitoringParameters, NodeClass, NodeClassMask,
    NodeId, NumericRange, ObjectId, ReadValueId, ReferenceDescription, ReferenceTypeId, StatusCode,
    TimestampsToReturn, UAString, Variant, WriteValue,
};

use super::{OpcUaProtocolClient, datetime_to_ms, parse_node_id};
use crate::types::OpcUaConfig;

/// References asked for per browse request; the rest come through continuation points.
const BROWSE_BATCH: u32 = 500;
/// How long a graceful close waits for the server before the connection is dropped.
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// Who the client authenticates as when it activates its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserIdentity {
    Anonymous,
    UserName { user: String, password: String },
    /// An X.509 user certificate (DER or PEM) and its private key (PEM).
    Certificate { certificate: PathBuf, private_key: PathBuf },
}

impl UserIdentity {
    fn token(&self) -> Result<IdentityToken, String> {
        match self {
            Self::Anonymous => Ok(IdentityToken::Anonymous),
            Self::UserName { user, password } => {
                Ok(IdentityToken::new_user_name(user.clone(), password.clone()))
            }
            Self::Certificate { certificate, private_key } => {
                IdentityToken::new_x509_path(certificate, private_key)
                    .map_err(|e| format!("Cannot use the user certificate: {e}"))
            }
        }
    }
}

/// One child found by [`OpcUaProtocolClient::browse_level`].
#[derive(Debug, Clone, PartialEq)]
pub struct BrowseNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    /// `Object`, `Variable`, `Method`, `ObjectType`, ...
    pub node_class: String,
    /// Variables: their data type's name (`Double`), or its node id for other types.
    pub data_type: String,
    /// Whether the child has hierarchical children of its own.
    pub has_children: bool,
}

/// A variable as the client reads it before monitoring it.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableInfo {
    pub node_id: String,
    pub display_name: String,
    pub data_type: String,
    pub writable: bool,
    /// The EngineeringUnits property's display name, if the variable has one.
    pub unit: String,
}

/// One input or output argument of a method.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentInfo {
    pub name: String,
    pub data_type: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UaQuality {
    Good,
    Uncertain,
    Bad,
}

/// One data change of a monitored item.
#[derive(Debug, Clone, PartialEq)]
pub struct UaValue {
    pub node_id: String,
    /// The value as a number: numbers as they are, booleans 1 and 0; `None` for text and others.
    pub number: Option<f64>,
    /// The value as text.
    pub text: String,
    pub quality: UaQuality,
    /// The status code's name, e.g. `Good` or `BadNodeIdUnknown`.
    pub status: String,
    /// Unix milliseconds; 0 when the server sent none.
    pub source_ms: u64,
    pub server_ms: u64,
}

/// A node id as typed (`ns=2;s=Pump.Speed`, `i=2258`), or `Objects` / `Root` / empty for the
/// standard folders.
fn node(text: &str) -> Result<NodeId, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("objects") {
        return Ok(ObjectId::ObjectsFolder.into());
    }
    if text.eq_ignore_ascii_case("root") {
        return Ok(ObjectId::RootFolder.into());
    }
    NodeId::from_str(text)
        .or_else(|_| parse_node_id(text))
        .map_err(|_| format!("“{text}” is not a node id (e.g. ns=2;s=Pump.Speed or i=2258)"))
}

/// The node id the server would print for `text`, so ids typed differently compare equal.
pub fn canonical_node_id(text: &str) -> Result<String, String> {
    node(text).map(|id| id.to_string())
}

fn node_class_name(class: NodeClass) -> &'static str {
    match class {
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

/// The name of a data type: the built-in ones by name, others by node id.
pub fn type_name(id: &NodeId) -> String {
    let name = match id.as_data_type_id() {
        Ok(DataTypeId::Boolean) => "Boolean",
        Ok(DataTypeId::SByte) => "SByte",
        Ok(DataTypeId::Byte) => "Byte",
        Ok(DataTypeId::Int16) => "Int16",
        Ok(DataTypeId::UInt16) => "UInt16",
        Ok(DataTypeId::Int32) => "Int32",
        Ok(DataTypeId::UInt32) => "UInt32",
        Ok(DataTypeId::Int64) => "Int64",
        Ok(DataTypeId::UInt64) => "UInt64",
        Ok(DataTypeId::Float) => "Float",
        Ok(DataTypeId::Double) => "Double",
        Ok(DataTypeId::String) => "String",
        Ok(DataTypeId::DateTime) => "DateTime",
        Ok(DataTypeId::Guid) => "Guid",
        Ok(DataTypeId::ByteString) => "ByteString",
        Ok(DataTypeId::NodeId) => "NodeId",
        Ok(DataTypeId::StatusCode) => "StatusCode",
        Ok(DataTypeId::QualifiedName) => "QualifiedName",
        Ok(DataTypeId::LocalizedText) => "LocalizedText",
        Ok(DataTypeId::Number) => "Number",
        Ok(DataTypeId::Integer) => "Integer",
        Ok(DataTypeId::UInteger) => "UInteger",
        Ok(DataTypeId::Enumeration) => "Enumeration",
        Ok(DataTypeId::BaseDataType) => "BaseDataType",
        _ => return id.to_string(),
    };
    name.to_owned()
}

fn type_of(value: Option<&Variant>) -> String {
    match value {
        Some(Variant::NodeId(id)) => type_name(id),
        Some(Variant::ExpandedNodeId(id)) => type_name(&id.node_id),
        _ => String::new(),
    }
}

fn text_of(value: Option<&Variant>) -> String {
    match value {
        Some(Variant::LocalizedText(text)) => text.text.as_ref().to_owned(),
        Some(Variant::QualifiedName(name)) => name.name.as_ref().to_owned(),
        Some(Variant::String(text)) => text.as_ref().to_owned(),
        _ => String::new(),
    }
}

/// A value as a number (if it is one) and as text.
pub fn variant_text(value: &Variant) -> (Option<f64>, String) {
    match value {
        Variant::Empty => (None, String::new()),
        Variant::Boolean(b) => (Some(if *b { 1.0 } else { 0.0 }), b.to_string()),
        Variant::SByte(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::Byte(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::Int16(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::UInt16(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::Int32(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::UInt32(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::Int64(n) => (Some(*n as f64), n.to_string()),
        Variant::UInt64(n) => (Some(*n as f64), n.to_string()),
        Variant::Float(n) => (Some(f64::from(*n)), n.to_string()),
        Variant::Double(n) => (Some(*n), n.to_string()),
        Variant::String(text) => (None, text.as_ref().to_owned()),
        Variant::LocalizedText(text) => (None, text.text.as_ref().to_owned()),
        Variant::QualifiedName(name) => {
            (None, format!("{}:{}", name.namespace_index, name.name.as_ref()))
        }
        Variant::DateTime(at) => (None, at.to_rfc3339()),
        Variant::NodeId(id) => (None, id.to_string()),
        Variant::StatusCode(status) => (None, format!("{status}")),
        Variant::ByteString(bytes) => (
            None,
            bytes
                .value
                .as_ref()
                .map(|bytes| bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
        ),
        Variant::Variant(inner) => variant_text(inner),
        Variant::Array(array) => {
            let items: Vec<String> = array.values.iter().map(|item| variant_text(item).1).collect();
            (None, format!("[{}]", items.join(", ")))
        }
        other => (None, format!("{other:?}")),
    }
}

fn status_text(status: StatusCode) -> String {
    format!("{status}")
}

fn quality(status: StatusCode) -> UaQuality {
    if status.is_bad() {
        UaQuality::Bad
    } else if status.is_uncertain() {
        UaQuality::Uncertain
    } else {
        UaQuality::Good
    }
}

/// The value of a data change as the app shows it.
fn ua_value(node_id: String, value: &DataValue) -> UaValue {
    let status = value.status.unwrap_or(StatusCode::Good);
    let (number, text) = value.value.as_ref().map(variant_text).unwrap_or((None, String::new()));
    let quality = if value.value.is_none() && status.is_good() { UaQuality::Bad } else { quality(status) };
    UaValue {
        node_id,
        number,
        text,
        quality,
        status: status_text(status),
        source_ms: value.source_timestamp.map(|at| datetime_to_ms(&at)).unwrap_or(0),
        server_ms: value.server_timestamp.map(|at| datetime_to_ms(&at)).unwrap_or(0),
    }
}

fn integer<T: TryFrom<i64> + TryFrom<u64>>(value: &serde_json::Value, name: &str) -> Result<T, String> {
    let fits = || format!("{value} does not fit a {name}");
    if let Some(n) = value.as_i64() {
        return T::try_from(n).map_err(|_| fits());
    }
    if let Some(n) = value.as_u64() {
        return T::try_from(n).map_err(|_| fits());
    }
    Err(format!("A {name} takes a whole number"))
}

/// A JSON value as the variant of a data type: what writes and method arguments send.
pub fn json_variant(data_type: &str, value: &serde_json::Value) -> Result<Variant, String> {
    let number = || value.as_f64().ok_or_else(|| format!("A {data_type} takes a number"));
    Ok(match data_type {
        "Boolean" => Variant::Boolean(match value {
            serde_json::Value::Bool(b) => *b,
            other => other.as_f64().map(|n| n != 0.0).ok_or("A Boolean takes true or false")?,
        }),
        "SByte" => Variant::SByte(integer(value, data_type)?),
        "Byte" => Variant::Byte(integer(value, data_type)?),
        "Int16" => Variant::Int16(integer(value, data_type)?),
        "UInt16" => Variant::UInt16(integer(value, data_type)?),
        "Int32" | "Enumeration" => Variant::Int32(integer(value, data_type)?),
        "UInt32" => Variant::UInt32(integer(value, data_type)?),
        "Int64" | "Integer" => Variant::Int64(integer(value, data_type)?),
        "UInt64" | "UInteger" => Variant::UInt64(integer(value, data_type)?),
        "Float" => Variant::Float(number()? as f32),
        "Double" | "Number" => Variant::Double(number()?),
        "String" => Variant::String(UAString::from(
            value.as_str().ok_or_else(|| format!("A {data_type} takes text"))?,
        )),
        "LocalizedText" => Variant::LocalizedText(Box::new(LocalizedText::new(
            "",
            value.as_str().ok_or_else(|| format!("A {data_type} takes text"))?,
        ))),
        "DateTime" => {
            let text = value.as_str().ok_or("A DateTime takes an RFC 3339 time")?;
            Variant::DateTime(Box::new(
                DateTime::parse_from_rfc3339(text)
                    .map_err(|_| format!("“{text}” is not an RFC 3339 time"))?,
            ))
        }
        other => return Err(format!("Writing {other} values is not supported")),
    })
}

fn property_reads(results: &[BrowseResult], name: &str) -> Vec<Option<NodeId>> {
    results
        .iter()
        .map(|result| {
            result.references.as_ref().and_then(|refs| {
                refs.iter()
                    .find(|r| r.browse_name.name.as_ref() == name)
                    .map(|r| r.node_id.node_id.clone())
            })
        })
        .collect()
}

fn property_browse(node_id: NodeId) -> BrowseDescription {
    BrowseDescription {
        node_id,
        browse_direction: BrowseDirection::Forward,
        reference_type_id: ReferenceTypeId::HasProperty.into(),
        include_subtypes: false,
        node_class_mask: NodeClassMask::VARIABLE.bits(),
        result_mask: BrowseResultMask::All as u32,
    }
}

fn hierarchical_browse(node_id: NodeId) -> BrowseDescription {
    BrowseDescription {
        node_id,
        browse_direction: BrowseDirection::Forward,
        reference_type_id: ReferenceTypeId::HierarchicalReferences.into(),
        include_subtypes: true,
        node_class_mask: NodeClassMask::all().bits(),
        result_mask: BrowseResultMask::All as u32,
    }
}

fn arguments(value: Option<&Variant>) -> Vec<ArgumentInfo> {
    let one = |item: &Variant| match item {
        Variant::ExtensionObject(ext) => ext.inner_as::<Argument>().map(|arg| ArgumentInfo {
            name: arg.name.as_ref().to_owned(),
            data_type: type_name(&arg.data_type),
            description: arg.description.text.as_ref().to_owned(),
        }),
        _ => None,
    };
    match value {
        Some(Variant::Array(array)) => array.values.iter().filter_map(one).collect(),
        Some(item) => one(item).into_iter().collect(),
        None => Vec::new(),
    }
}

impl OpcUaProtocolClient {
    /// Connect as `identity`: discovery picks the server endpoint matching the configured security
    /// policy and mode, and the session activates with the identity's token.
    pub async fn connect_as(&mut self, config: OpcUaConfig, identity: &UserIdentity) -> Result<(), String> {
        let token = identity.token()?;
        self.connect_with_token(config, token).await
    }

    /// Whether the session's connection is still up (its event loop has not ended).
    pub fn is_alive(&self) -> bool {
        self.session.is_some() && self._event_loop.as_ref().is_some_and(|handle| !handle.is_finished())
    }

    /// Close the session on the server (briefly waiting for it), then drop the connection.
    pub async fn close(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = tokio::time::timeout(CLOSE_WAIT, session.disconnect()).await;
        }
        if let Some(handle) = self._event_loop.take() {
            handle.abort();
        }
        self.subscription_id = None;
    }

    async fn browse_all(&self, descriptions: Vec<BrowseDescription>) -> Result<Vec<BrowseResult>, String> {
        let session = self.session()?;
        let mut results = session
            .browse(&descriptions, BROWSE_BATCH, None)
            .await
            .map_err(|e| format!("Browse failed: {e}"))?;
        for result in &mut results {
            let mut point = result.continuation_point.clone();
            while !point.is_null_or_empty() {
                let next = session
                    .browse_next(false, std::slice::from_ref(&point))
                    .await
                    .map_err(|e| format!("BrowseNext failed: {e}"))?;
                let Some(next) = next.into_iter().next() else { break };
                if let Some(more) = next.references {
                    result.references.get_or_insert_with(Vec::new).extend(more);
                }
                point = next.continuation_point;
            }
        }
        Ok(results)
    }

    /// The hierarchical children of `node_id` (`Objects` for the objects folder), with each
    /// variable's data type and whether each child has children itself.
    pub async fn browse_level(&self, node_id: &str) -> Result<Vec<BrowseNode>, String> {
        let parent = node(node_id)?;
        let mut results = self.browse_all(vec![hierarchical_browse(parent)]).await?;
        let result = results.pop().ok_or("The server sent no browse result")?;
        if result.status_code.is_bad() {
            return Err(format!("Browse failed: {}", status_text(result.status_code)));
        }
        let refs: Vec<ReferenceDescription> = result
            .references
            .unwrap_or_default()
            .into_iter()
            .filter(|r| !r.node_id.node_id.is_null())
            .collect();
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let session = self.session()?;

        // One read for every variable's data type.
        let variables: Vec<usize> =
            (0..refs.len()).filter(|&i| refs[i].node_class == NodeClass::Variable).collect();
        let mut types = vec![String::new(); refs.len()];
        if !variables.is_empty() {
            let reads: Vec<ReadValueId> = variables
                .iter()
                .map(|&i| ReadValueId {
                    node_id: refs[i].node_id.node_id.clone(),
                    attribute_id: AttributeId::DataType as u32,
                    ..Default::default()
                })
                .collect();
            if let Ok(values) = session.read(&reads, TimestampsToReturn::Neither, 0.0).await {
                for (&i, value) in variables.iter().zip(&values) {
                    types[i] = type_of(value.value.as_ref());
                }
            }
        }

        // One browse (a single reference each) for whether the children have children.
        let probes: Vec<BrowseDescription> =
            refs.iter().map(|r| hierarchical_browse(r.node_id.node_id.clone())).collect();
        let has_children: Vec<bool> = match session.browse(&probes, 1, None).await {
            Ok(found) => {
                let points: Vec<_> = found
                    .iter()
                    .map(|r| r.continuation_point.clone())
                    .filter(|p| !p.is_null_or_empty())
                    .collect();
                if !points.is_empty() {
                    let _ = session.browse_next(true, &points).await;
                }
                found.iter().map(|r| r.references.as_ref().is_some_and(|refs| !refs.is_empty())).collect()
            }
            // Unknown: let the user try to expand it.
            Err(_) => vec![true; refs.len()],
        };

        Ok(refs
            .into_iter()
            .enumerate()
            .map(|(i, r)| BrowseNode {
                node_id: r.node_id.node_id.to_string(),
                browse_name: r.browse_name.name.as_ref().to_owned(),
                display_name: r.display_name.text.as_ref().to_owned(),
                node_class: node_class_name(r.node_class).to_owned(),
                data_type: std::mem::take(&mut types[i]),
                has_children: has_children.get(i).copied().unwrap_or(true),
            })
            .collect())
    }

    /// Describe variables before monitoring them: display name, data type, whether the user may
    /// write them and their engineering unit. One result per id, in order.
    pub async fn describe_variables(
        &self,
        node_ids: &[String],
    ) -> Result<Vec<Result<VariableInfo, String>>, String> {
        let session = self.session()?;
        let parsed: Vec<Result<NodeId, String>> = node_ids.iter().map(|id| node(id)).collect();
        let valid: Vec<NodeId> = parsed.iter().filter_map(|id| id.as_ref().ok().cloned()).collect();
        const ATTRIBUTES: [AttributeId; 4] = [
            AttributeId::NodeClass,
            AttributeId::DisplayName,
            AttributeId::DataType,
            AttributeId::UserAccessLevel,
        ];
        let mut values: Vec<DataValue> = Vec::new();
        let mut units: Vec<String> = vec![String::new(); valid.len()];
        if !valid.is_empty() {
            let reads: Vec<ReadValueId> = valid
                .iter()
                .flat_map(|id| {
                    ATTRIBUTES.iter().map(move |attribute| ReadValueId {
                        node_id: id.clone(),
                        attribute_id: *attribute as u32,
                        ..Default::default()
                    })
                })
                .collect();
            values = session
                .read(&reads, TimestampsToReturn::Neither, 0.0)
                .await
                .map_err(|e| format!("Read failed: {e}"))?;
            let properties = self.browse_all(valid.iter().cloned().map(property_browse).collect()).await;
            if let Ok(properties) = properties {
                let eu = property_reads(&properties, "EngineeringUnits");
                let reads: Vec<(usize, ReadValueId)> = eu
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, id)| {
                        id.map(|node_id| {
                            (i, ReadValueId { node_id, attribute_id: AttributeId::Value as u32, ..Default::default() })
                        })
                    })
                    .collect();
                if !reads.is_empty() {
                    let ids: Vec<ReadValueId> = reads.iter().map(|(_, read)| read.clone()).collect();
                    if let Ok(found) = session.read(&ids, TimestampsToReturn::Neither, 0.0).await {
                        for ((i, _), value) in reads.iter().zip(&found) {
                            if let Some(Variant::ExtensionObject(ext)) = value.value.as_ref() {
                                if let Some(info) = ext.inner_as::<EUInformation>() {
                                    units[*i] = info.display_name.text.as_ref().to_owned();
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(parsed.len());
        let mut index = 0;
        for id in parsed {
            let id = match id {
                Ok(id) => id,
                Err(error) => {
                    out.push(Err(error));
                    continue;
                }
            };
            let at = |n: usize| values.get(index * ATTRIBUTES.len() + n);
            let class = at(0);
            let described = match class.and_then(|v| v.value.as_ref()) {
                Some(Variant::Int32(2)) => {
                    let access = match at(3).and_then(|v| v.value.as_ref()) {
                        Some(Variant::Byte(level)) => *level,
                        _ => 0,
                    };
                    Ok(VariableInfo {
                        node_id: id.to_string(),
                        display_name: text_of(at(1).and_then(|v| v.value.as_ref())),
                        data_type: type_of(at(2).and_then(|v| v.value.as_ref())),
                        // AccessLevel bit 1: CurrentWrite.
                        writable: access & 0x02 != 0,
                        unit: std::mem::take(&mut units[index]),
                    })
                }
                Some(Variant::Int32(other)) => Err(format!(
                    "{id} is not a variable but {}",
                    match other {
                        1 => "an object",
                        4 => "a method",
                        _ => "a type or view",
                    }
                )),
                _ => Err(format!(
                    "{id}: {}",
                    class.and_then(|v| v.status).map(status_text).unwrap_or_else(|| "no such node".into())
                )),
            };
            out.push(described);
            index += 1;
        }
        Ok(out)
    }

    /// Create a subscription whose data changes go to `on_value`. Replaces nothing: call once.
    pub async fn subscribe_values(
        &mut self,
        publishing_interval_ms: u32,
        on_value: Arc<dyn Fn(UaValue) + Send + Sync>,
    ) -> Result<u32, String> {
        let session = self.session()?.clone();
        let callback = DataChangeCallback::new(move |value: DataValue, item| {
            on_value(ua_value(item.item_to_monitor().node_id.to_string(), &value));
        });
        let id = session
            .create_subscription(
                Duration::from_millis(u64::from(publishing_interval_ms.max(10))),
                // Lifetime and keep-alive counts the spec recommends (lifetime ≥ 3 × keep-alive).
                60,
                10,
                0,
                0,
                true,
                callback,
            )
            .await
            .map_err(|e| format!("CreateSubscription failed: {e}"))?;
        self.subscription_id = Some(id);
        Ok(id)
    }

    /// Monitor `items` (node id, sampling interval in ms; 0 samples as fast as the server can,
    /// negative uses the publishing interval) in `subscription_id`. One result per item: the revised
    /// sampling interval, or why the server refused the item.
    pub async fn monitor(
        &self,
        subscription_id: u32,
        items: &[(String, f64)],
    ) -> Result<Vec<Result<f64, String>>, String> {
        let session = self.session()?;
        let parsed: Vec<Result<NodeId, String>> = items.iter().map(|(id, _)| node(id)).collect();
        let requests: Vec<MonitoredItemCreateRequest> = parsed
            .iter()
            .zip(items)
            .filter_map(|(id, (_, sampling))| {
                let node_id = id.as_ref().ok()?.clone();
                let mut params = MonitoringParameters::default();
                params.sampling_interval = *sampling;
                params.queue_size = 10;
                params.discard_oldest = true;
                Some(MonitoredItemCreateRequest::new(
                    ReadValueId { node_id, attribute_id: AttributeId::Value as u32, ..Default::default() },
                    MonitoringMode::Reporting,
                    params,
                ))
            })
            .collect();
        let created = if requests.is_empty() {
            Vec::new()
        } else {
            session
                .create_monitored_items(subscription_id, TimestampsToReturn::Both, requests)
                .await
                .map_err(|e| format!("CreateMonitoredItems failed: {e}"))?
        };
        let mut created = created.into_iter();
        Ok(parsed
            .into_iter()
            .map(|id| {
                id?;
                let item = created.next().ok_or("The server sent no result for the item")?;
                if item.result.status_code.is_bad() {
                    Err(status_text(item.result.status_code))
                } else {
                    Ok(item.result.revised_sampling_interval)
                }
            })
            .collect())
    }

    /// Read the current Value of each node, as data changes would report it.
    pub async fn read_values(&self, node_ids: &[String]) -> Result<Vec<UaValue>, String> {
        let session = self.session()?;
        let ids = node_ids.iter().map(|id| node(id)).collect::<Result<Vec<_>, _>>()?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let reads: Vec<ReadValueId> = ids
            .iter()
            .map(|id| ReadValueId { node_id: id.clone(), attribute_id: AttributeId::Value as u32, ..Default::default() })
            .collect();
        let values = session
            .read(&reads, TimestampsToReturn::Both, 0.0)
            .await
            .map_err(|e| format!("Read failed: {e}"))?;
        Ok(ids.iter().zip(&values).map(|(id, value)| ua_value(id.to_string(), value)).collect())
    }

    /// Write `value` (JSON, converted to `data_type`) to a variable's Value.
    pub async fn write_value(&self, node_id: &str, data_type: &str, value: &serde_json::Value) -> Result<(), String> {
        let session = self.session()?;
        let node_id = node(node_id)?;
        let variant = json_variant(data_type, value)?;
        let results = session
            .write(&[WriteValue {
                node_id,
                attribute_id: AttributeId::Value as u32,
                index_range: NumericRange::None,
                value: DataValue { value: Some(variant), ..Default::default() },
            }])
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        match results.first() {
            Some(status) if status.is_bad() => Err(format!("The server refused the write: {}", status_text(*status))),
            _ => Ok(()),
        }
    }

    /// The input and output arguments of a method.
    pub async fn method_arguments(&self, method_id: &str) -> Result<(Vec<ArgumentInfo>, Vec<ArgumentInfo>), String> {
        let session = self.session()?;
        let method = node(method_id)?;
        let properties = self.browse_all(vec![property_browse(method)]).await?;
        let inputs = property_reads(&properties, "InputArguments").pop().flatten();
        let outputs = property_reads(&properties, "OutputArguments").pop().flatten();
        let ids: Vec<NodeId> = [inputs.clone(), outputs.clone()].into_iter().flatten().collect();
        if ids.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let reads: Vec<ReadValueId> = ids
            .iter()
            .map(|id| ReadValueId { node_id: id.clone(), attribute_id: AttributeId::Value as u32, ..Default::default() })
            .collect();
        let values = session
            .read(&reads, TimestampsToReturn::Neither, 0.0)
            .await
            .map_err(|e| format!("Read failed: {e}"))?;
        let mut values = values.iter();
        let input = if inputs.is_some() { arguments(values.next().and_then(|v| v.value.as_ref())) } else { Vec::new() };
        let output = if outputs.is_some() { arguments(values.next().and_then(|v| v.value.as_ref())) } else { Vec::new() };
        Ok((input, output))
    }

    /// Call a method of `object_id` with typed arguments (data type, JSON value). Returns the output
    /// arguments as text.
    pub async fn call_method(
        &self,
        object_id: &str,
        method_id: &str,
        inputs: &[(String, serde_json::Value)],
    ) -> Result<Vec<String>, String> {
        let session = self.session()?;
        let arguments = inputs
            .iter()
            .enumerate()
            .map(|(i, (data_type, value))| {
                json_variant(data_type, value).map_err(|e| format!("Argument {}: {e}", i + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result = session
            .call_one(CallMethodRequest {
                object_id: node(object_id)?,
                method_id: node(method_id)?,
                input_arguments: Some(arguments),
            })
            .await
            .map_err(|e| format!("Call failed: {e}"))?;
        if result.status_code.is_bad() {
            let bad_inputs: Vec<String> = result
                .input_argument_results
                .unwrap_or_default()
                .iter()
                .enumerate()
                .filter(|(_, status)| status.is_bad())
                .map(|(i, status)| format!("argument {}: {}", i + 1, status_text(*status)))
                .collect();
            return Err(if bad_inputs.is_empty() {
                format!("The server refused the call: {}", status_text(result.status_code))
            } else {
                format!("The server refused the call: {} ({})", status_text(result.status_code), bad_inputs.join(", "))
            });
        }
        Ok(result.output_arguments.unwrap_or_default().iter().map(|v| variant_text(v).1).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_parse_in_every_namespace() {
        assert_eq!(canonical_node_id("ns=2;s=Plant.Reactor").unwrap(), "ns=2;s=Plant.Reactor");
        assert_eq!(canonical_node_id("i=2258").unwrap(), "i=2258");
        assert_eq!(canonical_node_id(" Objects ").unwrap(), "i=85");
        assert!(canonical_node_id("Plant").is_err());
    }

    #[test]
    fn json_values_become_the_declared_type() {
        assert_eq!(json_variant("Int16", &serde_json::json!(-5)).unwrap(), Variant::Int16(-5));
        assert!(json_variant("Byte", &serde_json::json!(256)).is_err());
        assert!(json_variant("UInt32", &serde_json::json!(1.5)).is_err());
        assert_eq!(json_variant("Boolean", &serde_json::json!(true)).unwrap(), Variant::Boolean(true));
        assert!(json_variant("Guid", &serde_json::json!("x")).is_err());
    }
}
