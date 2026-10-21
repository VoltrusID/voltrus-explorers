//! Sparkplug B payload decoding.
//!
//! The Sparkplug B payload is a protobuf message. Its schema is small and
//! stable, so the message types below are written out by hand with
//! `prost` derives (field numbers follow the Eclipse Tahu `sparkplug_b.proto`,
//! EPL-2.0). That keeps the build free of a `protoc` dependency.
//!
//! [`decode_payload`] turns a raw payload into a JSON tree for display.
//! [`SparkplugAliases`] remembers the name/alias pairs announced in BIRTH
//! messages so later DATA messages, which carry only the alias, still show
//! the metric name.

use std::collections::HashMap;

use prost::Message;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Protobuf types
// ---------------------------------------------------------------------------

/// Hand-written prost types for `org.eclipse.tahu.protobuf.Payload`.
pub mod pb {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Payload {
        #[prost(uint64, optional, tag = "1")]
        pub timestamp: Option<u64>,
        #[prost(message, repeated, tag = "2")]
        pub metrics: Vec<Metric>,
        #[prost(uint64, optional, tag = "3")]
        pub seq: Option<u64>,
        #[prost(string, optional, tag = "4")]
        pub uuid: Option<String>,
        #[prost(bytes = "vec", optional, tag = "5")]
        pub body: Option<Vec<u8>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Metric {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(uint64, optional, tag = "2")]
        pub alias: Option<u64>,
        #[prost(uint64, optional, tag = "3")]
        pub timestamp: Option<u64>,
        #[prost(uint32, optional, tag = "4")]
        pub datatype: Option<u32>,
        #[prost(bool, optional, tag = "5")]
        pub is_historical: Option<bool>,
        #[prost(bool, optional, tag = "6")]
        pub is_transient: Option<bool>,
        #[prost(bool, optional, tag = "7")]
        pub is_null: Option<bool>,
        #[prost(message, optional, tag = "8")]
        pub metadata: Option<MetaData>,
        #[prost(message, optional, tag = "9")]
        pub properties: Option<PropertySet>,
        #[prost(oneof = "metric::Value", tags = "10, 11, 12, 13, 14, 15, 16, 17, 18")]
        pub value: Option<metric::Value>,
    }

    pub mod metric {
        use super::{DataSet, Template};

        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Value {
            #[prost(uint32, tag = "10")]
            IntValue(u32),
            #[prost(uint64, tag = "11")]
            LongValue(u64),
            #[prost(float, tag = "12")]
            FloatValue(f32),
            #[prost(double, tag = "13")]
            DoubleValue(f64),
            #[prost(bool, tag = "14")]
            BooleanValue(bool),
            #[prost(string, tag = "15")]
            StringValue(String),
            #[prost(bytes = "vec", tag = "16")]
            BytesValue(Vec<u8>),
            #[prost(message, tag = "17")]
            DatasetValue(DataSet),
            #[prost(message, tag = "18")]
            TemplateValue(Template),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MetaData {
        #[prost(bool, optional, tag = "1")]
        pub is_multi_part: Option<bool>,
        #[prost(string, optional, tag = "2")]
        pub content_type: Option<String>,
        #[prost(uint64, optional, tag = "3")]
        pub size: Option<u64>,
        #[prost(uint64, optional, tag = "4")]
        pub seq: Option<u64>,
        #[prost(string, optional, tag = "5")]
        pub file_name: Option<String>,
        #[prost(string, optional, tag = "6")]
        pub file_type: Option<String>,
        #[prost(string, optional, tag = "7")]
        pub md5: Option<String>,
        #[prost(string, optional, tag = "8")]
        pub description: Option<String>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PropertySet {
        #[prost(string, repeated, tag = "1")]
        pub keys: Vec<String>,
        #[prost(message, repeated, tag = "2")]
        pub values: Vec<PropertyValue>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PropertySetList {
        #[prost(message, repeated, tag = "1")]
        pub propertyset: Vec<PropertySet>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PropertyValue {
        #[prost(uint32, optional, tag = "1")]
        pub r#type: Option<u32>,
        #[prost(bool, optional, tag = "2")]
        pub is_null: Option<bool>,
        #[prost(oneof = "property_value::Value", tags = "3, 4, 5, 6, 7, 8, 9, 10")]
        pub value: Option<property_value::Value>,
    }

    pub mod property_value {
        use super::{PropertySet, PropertySetList};

        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Value {
            #[prost(uint32, tag = "3")]
            IntValue(u32),
            #[prost(uint64, tag = "4")]
            LongValue(u64),
            #[prost(float, tag = "5")]
            FloatValue(f32),
            #[prost(double, tag = "6")]
            DoubleValue(f64),
            #[prost(bool, tag = "7")]
            BooleanValue(bool),
            #[prost(string, tag = "8")]
            StringValue(String),
            #[prost(message, tag = "9")]
            PropertysetValue(PropertySet),
            #[prost(message, tag = "10")]
            PropertysetsValue(PropertySetList),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DataSet {
        #[prost(uint64, optional, tag = "1")]
        pub num_of_columns: Option<u64>,
        #[prost(string, repeated, tag = "2")]
        pub columns: Vec<String>,
        #[prost(uint32, repeated, packed = "false", tag = "3")]
        pub types: Vec<u32>,
        #[prost(message, repeated, tag = "4")]
        pub rows: Vec<data_set::Row>,
    }

    pub mod data_set {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Row {
            #[prost(message, repeated, tag = "1")]
            pub elements: Vec<DataSetValue>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct DataSetValue {
            #[prost(oneof = "data_set_value::Value", tags = "1, 2, 3, 4, 5, 6")]
            pub value: Option<data_set_value::Value>,
        }

        pub mod data_set_value {
            #[derive(Clone, PartialEq, prost::Oneof)]
            pub enum Value {
                #[prost(uint32, tag = "1")]
                IntValue(u32),
                #[prost(uint64, tag = "2")]
                LongValue(u64),
                #[prost(float, tag = "3")]
                FloatValue(f32),
                #[prost(double, tag = "4")]
                DoubleValue(f64),
                #[prost(bool, tag = "5")]
                BooleanValue(bool),
                #[prost(string, tag = "6")]
                StringValue(String),
            }
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Template {
        #[prost(string, optional, tag = "1")]
        pub version: Option<String>,
        #[prost(message, repeated, tag = "2")]
        pub metrics: Vec<Metric>,
        #[prost(message, repeated, tag = "3")]
        pub parameters: Vec<template::Parameter>,
        #[prost(string, optional, tag = "4")]
        pub template_ref: Option<String>,
        #[prost(bool, optional, tag = "5")]
        pub is_definition: Option<bool>,
    }

    pub mod template {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Parameter {
            #[prost(string, optional, tag = "1")]
            pub name: Option<String>,
            #[prost(uint32, optional, tag = "2")]
            pub r#type: Option<u32>,
            #[prost(oneof = "parameter::Value", tags = "3, 4, 5, 6, 7, 8")]
            pub value: Option<parameter::Value>,
        }

        pub mod parameter {
            #[derive(Clone, PartialEq, prost::Oneof)]
            pub enum Value {
                #[prost(uint32, tag = "3")]
                IntValue(u32),
                #[prost(uint64, tag = "4")]
                LongValue(u64),
                #[prost(float, tag = "5")]
                FloatValue(f32),
                #[prost(double, tag = "6")]
                DoubleValue(f64),
                #[prost(bool, tag = "7")]
                BooleanValue(bool),
                #[prost(string, tag = "8")]
                StringValue(String),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Sparkplug B `DataType` ids. Unknown ids render as `Unknown(<n>)`.
pub fn datatype_name(id: u32) -> String {
    match id {
        0 => "Unknown",
        1 => "Int8",
        2 => "Int16",
        3 => "Int32",
        4 => "Int64",
        5 => "UInt8",
        6 => "UInt16",
        7 => "UInt32",
        8 => "UInt64",
        9 => "Float",
        10 => "Double",
        11 => "Boolean",
        12 => "String",
        13 => "DateTime",
        14 => "Text",
        15 => "UUID",
        16 => "DataSet",
        17 => "Bytes",
        18 => "File",
        19 => "Template",
        20 => "PropertySet",
        21 => "PropertySetList",
        22 => "Int8Array",
        23 => "Int16Array",
        24 => "Int32Array",
        25 => "Int64Array",
        26 => "UInt8Array",
        27 => "UInt16Array",
        28 => "UInt32Array",
        29 => "UInt64Array",
        30 => "FloatArray",
        31 => "DoubleArray",
        32 => "BooleanArray",
        33 => "StringArray",
        34 => "DateTimeArray",
        other => return format!("Unknown({other})"),
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Topic
// ---------------------------------------------------------------------------

/// Namespace token of every Sparkplug B topic.
pub const NAMESPACE: &str = "spBv1.0";

/// A parsed `spBv1.0/...` topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparkplugTopic {
    /// `None` for `spBv1.0/STATE/<host_id>`.
    pub group_id: Option<String>,
    /// `NBIRTH`, `NDATA`, `DDEATH`, `STATE`, and so on.
    pub message_type: String,
    /// Edge node id, or the primary host id for `STATE`.
    pub edge_node_id: String,
    pub device_id: Option<String>,
}

impl SparkplugTopic {
    /// Parse a topic. Returns `None` when it is not in the Sparkplug B
    /// namespace or does not have a valid shape.
    pub fn parse(topic: &str) -> Option<Self> {
        let mut parts = topic.split('/');
        if parts.next()? != NAMESPACE {
            return None;
        }
        let second = parts.next()?;
        if second == "STATE" {
            let host = parts.next()?;
            if parts.next().is_some() || host.is_empty() {
                return None;
            }
            return Some(Self {
                group_id: None,
                message_type: "STATE".into(),
                edge_node_id: host.into(),
                device_id: None,
            });
        }
        let message_type = parts.next()?;
        if !matches!(
            message_type,
            "NBIRTH" | "NDEATH" | "NDATA" | "NCMD" | "DBIRTH" | "DDEATH" | "DDATA" | "DCMD"
        ) {
            return None;
        }
        let edge = parts.next()?;
        let device = parts.next();
        if parts.next().is_some() || edge.is_empty() {
            return None;
        }
        // Node-level messages carry no device id; device-level messages must.
        let is_device = message_type.starts_with('D');
        if is_device != device.is_some() {
            return None;
        }
        Some(Self {
            group_id: Some(second.into()),
            message_type: message_type.into(),
            edge_node_id: edge.into(),
            device_id: device.map(String::from),
        })
    }

    fn is_birth(&self) -> bool {
        self.message_type == "NBIRTH" || self.message_type == "DBIRTH"
    }

    fn alias_key(&self) -> String {
        format!(
            "{}/{}/{}",
            self.group_id.as_deref().unwrap_or(""),
            self.edge_node_id,
            self.device_id.as_deref().unwrap_or("")
        )
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("namespace".into(), json!(NAMESPACE));
        if let Some(g) = &self.group_id {
            m.insert("group_id".into(), json!(g));
        }
        m.insert("message_type".into(), json!(self.message_type));
        m.insert("edge_node_id".into(), json!(self.edge_node_id));
        if let Some(d) = &self.device_id {
            m.insert("device_id".into(), json!(d));
        }
        Value::Object(m)
    }
}

// ---------------------------------------------------------------------------
// Aliases
// ---------------------------------------------------------------------------

/// Metric name/alias pairs from BIRTH messages, keyed by
/// group/edge-node/device. DATA and CMD messages usually carry the alias only.
#[derive(Debug, Default)]
pub struct SparkplugAliases {
    by_source: HashMap<String, HashMap<u64, String>>,
}

impl SparkplugAliases {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget every learned alias (for example on reconnect).
    pub fn clear(&mut self) {
        self.by_source.clear();
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// JSON number for a u64. Values beyond 2^53 become strings so a JS client
/// does not silently round them.
fn u64_json(v: u64) -> Value {
    if v <= (1u64 << 53) { json!(v) } else { json!(v.to_string()) }
}

fn i64_json(v: i64) -> Value {
    if v.unsigned_abs() <= (1u64 << 53) { json!(v) } else { json!(v.to_string()) }
}

fn f64_json(v: f64) -> Value {
    // JSON has no NaN or infinity; show them as strings.
    match serde_json::Number::from_f64(v) {
        Some(n) => Value::Number(n),
        None => json!(v.to_string()),
    }
}

fn f32_json(v: f32) -> Value {
    // Round-trip through the shortest decimal that identifies the f32, so
    // 0.1f32 shows as 0.1 and not 0.10000000149011612.
    match v.to_string().parse::<f64>() {
        Ok(d) => f64_json(d),
        Err(_) => f64_json(v as f64),
    }
}

fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

/// Apply the metric's `datatype` to the raw `int_value` bits.
fn int_for_type(datatype: Option<u32>, v: u32) -> Value {
    match datatype {
        Some(1) => json!(v as u8 as i8),
        Some(2) => json!(v as u16 as i16),
        Some(3) => json!(v as i32),
        Some(5) => json!(v as u8),
        Some(6) => json!(v as u16),
        _ => json!(v),
    }
}

fn long_for_type(datatype: Option<u32>, v: u64) -> Value {
    match datatype {
        Some(4) => i64_json(v as i64),
        _ => u64_json(v),
    }
}

/// Decode a packed array stored in `bytes_value` (Sparkplug 3.0 section on
/// array types): little-endian fixed width numbers, NUL-terminated strings,
/// or a 4-byte count followed by MSB-first packed bits for booleans.
fn decode_array(datatype: u32, b: &[u8]) -> Result<Value, String> {
    fn fixed<const N: usize, T>(b: &[u8], f: impl Fn([u8; N]) -> T) -> Result<Vec<T>, String> {
        if b.len() % N != 0 {
            return Err(format!("array length {} is not a multiple of {N}", b.len()));
        }
        Ok(b.chunks_exact(N)
            .map(|c| f(c.try_into().expect("chunk size")))
            .collect())
    }
    Ok(match datatype {
        22 => json!(fixed::<1, _>(b, |c| i8::from_le_bytes(c))?),
        23 => json!(fixed::<2, _>(b, i16::from_le_bytes)?),
        24 => json!(fixed::<4, _>(b, i32::from_le_bytes)?),
        25 => Value::Array(fixed::<8, _>(b, i64::from_le_bytes)?.into_iter().map(i64_json).collect()),
        26 => json!(b),
        27 => json!(fixed::<2, _>(b, u16::from_le_bytes)?),
        28 => json!(fixed::<4, _>(b, u32::from_le_bytes)?),
        29 | 34 => Value::Array(fixed::<8, _>(b, u64::from_le_bytes)?.into_iter().map(u64_json).collect()),
        30 => Value::Array(fixed::<4, _>(b, f32::from_le_bytes)?.into_iter().map(f32_json).collect()),
        31 => Value::Array(fixed::<8, _>(b, f64::from_le_bytes)?.into_iter().map(f64_json).collect()),
        32 => {
            if b.len() < 4 {
                return Err("BooleanArray shorter than its 4-byte count".into());
            }
            let count = u32::from_le_bytes(b[..4].try_into().expect("4 bytes")) as usize;
            let bits = &b[4..];
            if bits.len() * 8 < count {
                return Err(format!("BooleanArray count {count} exceeds {} data bits", bits.len() * 8));
            }
            json!((0..count).map(|i| bits[i / 8] & (0x80 >> (i % 8)) != 0).collect::<Vec<_>>())
        }
        33 => {
            let mut out = Vec::new();
            let mut rest = b;
            while !rest.is_empty() {
                let end = rest.iter().position(|&c| c == 0).ok_or("StringArray element without NUL terminator")?;
                out.push(String::from_utf8_lossy(&rest[..end]).into_owned());
                rest = &rest[end + 1..];
            }
            json!(out)
        }
        other => return Err(format!("not an array datatype: {other}")),
    })
}

fn property_value_json(v: &pb::PropertyValue) -> Value {
    use pb::property_value::Value as V;
    let mut m = Map::new();
    if let Some(t) = v.r#type {
        m.insert("type".into(), json!(datatype_name(t)));
    }
    if v.is_null == Some(true) {
        m.insert("value".into(), Value::Null);
        return Value::Object(m);
    }
    let value = match &v.value {
        Some(V::IntValue(i)) => int_for_type(v.r#type, *i),
        Some(V::LongValue(l)) => long_for_type(v.r#type, *l),
        Some(V::FloatValue(f)) => f32_json(*f),
        Some(V::DoubleValue(d)) => f64_json(*d),
        Some(V::BooleanValue(b)) => json!(b),
        Some(V::StringValue(s)) => json!(s),
        Some(V::PropertysetValue(set)) => property_set_json(set),
        Some(V::PropertysetsValue(list)) => Value::Array(list.propertyset.iter().map(property_set_json).collect()),
        None => Value::Null,
    };
    m.insert("value".into(), value);
    Value::Object(m)
}

fn property_set_json(set: &pb::PropertySet) -> Value {
    let mut m = Map::new();
    for (i, key) in set.keys.iter().enumerate() {
        let value = set.values.get(i).map(property_value_json).unwrap_or(Value::Null);
        m.insert(key.clone(), value);
    }
    Value::Object(m)
}

fn dataset_json(ds: &pb::DataSet) -> Value {
    use pb::data_set::data_set_value::Value as V;
    let rows: Vec<Value> = ds
        .rows
        .iter()
        .map(|row| {
            Value::Array(
                row.elements
                    .iter()
                    .enumerate()
                    .map(|(i, el)| {
                        let ty = ds.types.get(i).copied();
                        match &el.value {
                            Some(V::IntValue(v)) => int_for_type(ty, *v),
                            Some(V::LongValue(v)) => long_for_type(ty, *v),
                            Some(V::FloatValue(v)) => f32_json(*v),
                            Some(V::DoubleValue(v)) => f64_json(*v),
                            Some(V::BooleanValue(v)) => json!(v),
                            Some(V::StringValue(v)) => json!(v),
                            None => Value::Null,
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    json!({
        "columns": ds.columns,
        "types": ds.types.iter().map(|t| datatype_name(*t)).collect::<Vec<_>>(),
        "rows": rows,
    })
}

fn template_json(t: &pb::Template) -> Value {
    use pb::template::parameter::Value as V;
    let mut m = Map::new();
    if let Some(v) = &t.version {
        m.insert("version".into(), json!(v));
    }
    if let Some(r) = &t.template_ref {
        m.insert("template_ref".into(), json!(r));
    }
    if let Some(d) = t.is_definition {
        m.insert("is_definition".into(), json!(d));
    }
    if !t.parameters.is_empty() {
        let params: Vec<Value> = t
            .parameters
            .iter()
            .map(|p| {
                let value = match &p.value {
                    Some(V::IntValue(v)) => int_for_type(p.r#type, *v),
                    Some(V::LongValue(v)) => long_for_type(p.r#type, *v),
                    Some(V::FloatValue(v)) => f32_json(*v),
                    Some(V::DoubleValue(v)) => f64_json(*v),
                    Some(V::BooleanValue(v)) => json!(v),
                    Some(V::StringValue(v)) => json!(v),
                    None => Value::Null,
                };
                json!({
                    "name": p.name,
                    "type": p.r#type.map(datatype_name),
                    "value": value,
                })
            })
            .collect();
        m.insert("parameters".into(), Value::Array(params));
    }
    m.insert(
        "metrics".into(),
        Value::Array(t.metrics.iter().map(|x| metric_json(x, None)).collect()),
    );
    Value::Object(m)
}

/// JSON for a metric's value according to its datatype.
fn metric_value_json(m: &pb::Metric) -> Value {
    use pb::metric::Value as V;
    if m.is_null == Some(true) {
        return Value::Null;
    }
    match &m.value {
        Some(V::IntValue(v)) => int_for_type(m.datatype, *v),
        Some(V::LongValue(v)) => long_for_type(m.datatype, *v),
        Some(V::FloatValue(v)) => f32_json(*v),
        Some(V::DoubleValue(v)) => f64_json(*v),
        Some(V::BooleanValue(v)) => json!(v),
        Some(V::StringValue(v)) => json!(v),
        Some(V::BytesValue(b)) => match m.datatype {
            Some(dt @ 22..=34) => decode_array(dt, b).unwrap_or_else(|e| json!({ "hex": hex(b), "error": e })),
            _ => json!(hex(b)),
        },
        Some(V::DatasetValue(d)) => dataset_json(d),
        Some(V::TemplateValue(t)) => template_json(t),
        None => Value::Null,
    }
}

fn metric_json(m: &pb::Metric, resolved_name: Option<&str>) -> Value {
    let mut o = Map::new();
    match (&m.name, resolved_name) {
        (Some(n), _) => {
            o.insert("name".into(), json!(n));
        }
        (None, Some(n)) => {
            o.insert("name".into(), json!(n));
            o.insert("name_from_alias".into(), json!(true));
        }
        (None, None) => {}
    }
    if let Some(a) = m.alias {
        o.insert("alias".into(), u64_json(a));
    }
    if let Some(ts) = m.timestamp {
        o.insert("timestamp".into(), u64_json(ts));
    }
    if let Some(dt) = m.datatype {
        o.insert("datatype".into(), json!(datatype_name(dt)));
    }
    for (key, flag) in [
        ("is_historical", m.is_historical),
        ("is_transient", m.is_transient),
        ("is_null", m.is_null),
    ] {
        if flag == Some(true) {
            o.insert(key.into(), json!(true));
        }
    }
    o.insert("value".into(), metric_value_json(m));
    if let Some(md) = &m.metadata {
        let mut mo = Map::new();
        if let Some(v) = md.is_multi_part {
            mo.insert("is_multi_part".into(), json!(v));
        }
        for (k, v) in [
            ("content_type", &md.content_type),
            ("file_name", &md.file_name),
            ("file_type", &md.file_type),
            ("md5", &md.md5),
            ("description", &md.description),
        ] {
            if let Some(s) = v {
                mo.insert(k.into(), json!(s));
            }
        }
        if let Some(v) = md.size {
            mo.insert("size".into(), u64_json(v));
        }
        if let Some(v) = md.seq {
            mo.insert("seq".into(), u64_json(v));
        }
        o.insert("metadata".into(), Value::Object(mo));
    }
    if let Some(p) = &m.properties {
        o.insert("properties".into(), property_set_json(p));
    }
    Value::Object(o)
}

/// Decode a Sparkplug B payload received on `topic`.
///
/// `aliases` is updated on BIRTH messages and consulted for every other
/// message to put names on alias-only metrics. The result is a JSON object
/// with the parsed `topic`, the payload header fields and a `metrics` array.
/// `STATE` topics carry JSON, not protobuf; pass those through unchanged
/// instead of calling this function.
pub fn decode_payload(
    topic: &SparkplugTopic,
    bytes: &[u8],
    aliases: &mut SparkplugAliases,
) -> Result<Value, String> {
    let payload = pb::Payload::decode(bytes).map_err(|e| format!("not a Sparkplug B payload: {e}"))?;

    let key = topic.alias_key();
    if topic.is_birth() {
        let learned: HashMap<u64, String> = payload
            .metrics
            .iter()
            .filter_map(|m| Some((m.alias?, m.name.clone()?)))
            .collect();
        aliases.by_source.insert(key.clone(), learned);
    }
    let known = aliases.by_source.get(&key);

    let metrics: Vec<Value> = payload
        .metrics
        .iter()
        .map(|m| {
            let resolved = if m.name.is_none() {
                m.alias.and_then(|a| known.and_then(|k| k.get(&a))).map(String::as_str)
            } else {
                None
            };
            metric_json(m, resolved)
        })
        .collect();

    let mut o = Map::new();
    o.insert("topic".into(), topic.to_json());
    if let Some(ts) = payload.timestamp {
        o.insert("timestamp".into(), u64_json(ts));
    }
    if let Some(seq) = payload.seq {
        o.insert("seq".into(), u64_json(seq));
    }
    if let Some(uuid) = &payload.uuid {
        o.insert("uuid".into(), json!(uuid));
    }
    if let Some(body) = &payload.body {
        o.insert("body".into(), json!(hex(body)));
    }
    o.insert("metrics".into(), Value::Array(metrics));
    Ok(Value::Object(o))
}

#[cfg(test)]
mod tests {
    use super::pb::metric::Value as MV;
    use super::*;

    fn metric(name: Option<&str>, alias: Option<u64>, datatype: u32, value: Option<MV>) -> pb::Metric {
        pb::Metric {
            name: name.map(String::from),
            alias,
            datatype: Some(datatype),
            value,
            ..Default::default()
        }
    }

    fn encode(metrics: Vec<pb::Metric>, seq: u64) -> Vec<u8> {
        pb::Payload {
            timestamp: Some(1_700_000_000_000),
            metrics,
            seq: Some(seq),
            ..Default::default()
        }
        .encode_to_vec()
    }

    fn topic(t: &str) -> SparkplugTopic {
        SparkplugTopic::parse(t).unwrap_or_else(|| panic!("topic {t} should parse"))
    }

    #[test]
    fn parses_node_and_device_topics() {
        let t = topic("spBv1.0/plant/NBIRTH/edge1");
        assert_eq!(t.group_id.as_deref(), Some("plant"));
        assert_eq!(t.message_type, "NBIRTH");
        assert_eq!(t.edge_node_id, "edge1");
        assert_eq!(t.device_id, None);

        let t = topic("spBv1.0/plant/DDATA/edge1/pump-7");
        assert_eq!(t.message_type, "DDATA");
        assert_eq!(t.device_id.as_deref(), Some("pump-7"));
    }

    #[test]
    fn parses_state_topic() {
        let t = topic("spBv1.0/STATE/scada-host");
        assert_eq!(t.message_type, "STATE");
        assert_eq!(t.group_id, None);
        assert_eq!(t.edge_node_id, "scada-host");
    }

    #[test]
    fn rejects_non_sparkplug_topics() {
        for t in [
            "sensors/temp",
            "spBv1.0",
            "spBv1.0/plant",
            "spBv1.0/plant/NDATA",
            "spAv1.0/plant/NDATA/e",
            "spBv1.0/plant/BOGUS/e",
            // Device messages need a device id, node messages must not have one.
            "spBv1.0/plant/DDATA/e",
            "spBv1.0/plant/NDATA/e/d",
            "spBv1.0/plant/NDATA/e/d/extra",
            "spBv1.0/STATE/a/b",
        ] {
            assert!(SparkplugTopic::parse(t).is_none(), "{t}");
        }
    }

    #[test]
    fn decodes_scalar_metrics() {
        let bytes = encode(
            vec![
                metric(Some("Temp"), None, 10, Some(MV::DoubleValue(21.5))),
                metric(Some("Count"), None, 7, Some(MV::IntValue(42))),
                metric(Some("On"), None, 11, Some(MV::BooleanValue(true))),
                metric(Some("Label"), None, 12, Some(MV::StringValue("pump".into()))),
                metric(Some("Ratio"), None, 9, Some(MV::FloatValue(0.1))),
                metric(Some("Big"), None, 8, Some(MV::LongValue(u64::MAX))),
            ],
            3,
        );
        let mut aliases = SparkplugAliases::new();
        let v = decode_payload(&topic("spBv1.0/g/NDATA/e"), &bytes, &mut aliases).unwrap();

        assert_eq!(v["timestamp"], 1_700_000_000_000u64);
        assert_eq!(v["seq"], 3);
        assert_eq!(v["topic"]["edge_node_id"], "e");
        let m = &v["metrics"];
        assert_eq!(m[0]["name"], "Temp");
        assert_eq!(m[0]["datatype"], "Double");
        assert_eq!(m[0]["value"], 21.5);
        assert_eq!(m[1]["value"], 42);
        assert_eq!(m[2]["value"], true);
        assert_eq!(m[3]["value"], "pump");
        // f32 0.1 must display as 0.1, not the widened f64 expansion.
        assert_eq!(m[4]["value"], 0.1);
        // Beyond 2^53 the value is a string so JS cannot round it.
        assert_eq!(m[5]["value"], u64::MAX.to_string());
    }

    #[test]
    fn signed_integers_follow_the_declared_datatype() {
        // Negative Int8/Int16/Int32 arrive as the low bits of a uint32.
        let bytes = encode(
            vec![
                metric(Some("i8"), None, 1, Some(MV::IntValue((-5i8) as u8 as u32))),
                metric(Some("i8-sign-extended"), None, 1, Some(MV::IntValue((-5i32) as u32))),
                metric(Some("i16"), None, 2, Some(MV::IntValue((-300i16) as u16 as u32))),
                metric(Some("i32"), None, 3, Some(MV::IntValue((-70000i32) as u32))),
                metric(Some("u8"), None, 5, Some(MV::IntValue(200))),
                metric(Some("i64"), None, 4, Some(MV::LongValue((-9i64) as u64))),
            ],
            0,
        );
        let v = decode_payload(&topic("spBv1.0/g/NDATA/e"), &bytes, &mut SparkplugAliases::new()).unwrap();
        let vals: Vec<i64> = v["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["value"].as_i64().unwrap())
            .collect();
        assert_eq!(vals, vec![-5, -5, -300, -70000, 200, -9]);
    }

    #[test]
    fn birth_aliases_name_later_data_metrics() {
        let mut aliases = SparkplugAliases::new();
        let birth = encode(
            vec![
                metric(Some("Motor/Speed"), Some(1), 10, Some(MV::DoubleValue(0.0))),
                metric(Some("Motor/Run"), Some(2), 11, Some(MV::BooleanValue(false))),
            ],
            0,
        );
        decode_payload(&topic("spBv1.0/g/DBIRTH/e/d"), &birth, &mut aliases).unwrap();

        let data = encode(vec![metric(None, Some(1), 10, Some(MV::DoubleValue(1450.0)))], 1);
        let v = decode_payload(&topic("spBv1.0/g/DDATA/e/d"), &data, &mut aliases).unwrap();
        let m = &v["metrics"][0];
        assert_eq!(m["name"], "Motor/Speed");
        assert_eq!(m["name_from_alias"], true);
        assert_eq!(m["alias"], 1);
        assert_eq!(m["value"], 1450.0);

        // Another device's DATA must not use these aliases.
        let v = decode_payload(&topic("spBv1.0/g/DDATA/e/other"), &data, &mut aliases).unwrap();
        assert!(v["metrics"][0].get("name").is_none());

        // A new BIRTH replaces the map.
        let rebirth = encode(vec![metric(Some("Other"), Some(1), 10, None)], 0);
        decode_payload(&topic("spBv1.0/g/DBIRTH/e/d"), &rebirth, &mut aliases).unwrap();
        let v = decode_payload(&topic("spBv1.0/g/DDATA/e/d"), &data, &mut aliases).unwrap();
        assert_eq!(v["metrics"][0]["name"], "Other");
    }

    #[test]
    fn null_and_flags() {
        let mut m = metric(Some("x"), None, 10, Some(MV::DoubleValue(9.0)));
        m.is_null = Some(true);
        m.is_historical = Some(true);
        m.timestamp = Some(1234);
        let v = decode_payload(
            &topic("spBv1.0/g/NDATA/e"),
            &encode(vec![m], 0),
            &mut SparkplugAliases::new(),
        )
        .unwrap();
        let m = &v["metrics"][0];
        assert!(m["value"].is_null());
        assert_eq!(m["is_null"], true);
        assert_eq!(m["is_historical"], true);
        assert_eq!(m["timestamp"], 1234);
    }

    #[test]
    fn decodes_datasets() {
        use pb::data_set::{DataSetValue, Row, data_set_value::Value as DV};
        let ds = pb::DataSet {
            num_of_columns: Some(2),
            columns: vec!["id".into(), "label".into()],
            types: vec![3, 12],
            rows: vec![Row {
                elements: vec![
                    DataSetValue { value: Some(DV::IntValue((-1i32) as u32)) },
                    DataSetValue { value: Some(DV::StringValue("a".into())) },
                ],
            }],
        };
        let bytes = encode(vec![metric(Some("tbl"), None, 16, Some(MV::DatasetValue(ds)))], 0);
        let v = decode_payload(&topic("spBv1.0/g/NDATA/e"), &bytes, &mut SparkplugAliases::new()).unwrap();
        let value = &v["metrics"][0]["value"];
        assert_eq!(value["columns"], json!(["id", "label"]));
        assert_eq!(value["types"], json!(["Int32", "String"]));
        assert_eq!(value["rows"], json!([[-1, "a"]]));
    }

    #[test]
    fn decodes_templates_and_properties() {
        let inner = metric(Some("rpm"), None, 10, Some(MV::DoubleValue(900.0)));
        let tmpl = pb::Template {
            version: Some("1".into()),
            metrics: vec![inner],
            template_ref: Some("Motor".into()),
            is_definition: Some(false),
            ..Default::default()
        };
        let mut m = metric(Some("M1"), None, 19, Some(MV::TemplateValue(tmpl)));
        m.properties = Some(pb::PropertySet {
            keys: vec!["Quality".into(), "EngUnit".into()],
            values: vec![
                pb::PropertyValue {
                    r#type: Some(3),
                    is_null: None,
                    value: Some(pb::property_value::Value::IntValue(192)),
                },
                pb::PropertyValue {
                    r#type: Some(12),
                    is_null: None,
                    value: Some(pb::property_value::Value::StringValue("rpm".into())),
                },
            ],
        });
        let v = decode_payload(
            &topic("spBv1.0/g/NBIRTH/e"),
            &encode(vec![m], 0),
            &mut SparkplugAliases::new(),
        )
        .unwrap();
        let m = &v["metrics"][0];
        assert_eq!(m["value"]["template_ref"], "Motor");
        assert_eq!(m["value"]["metrics"][0]["name"], "rpm");
        assert_eq!(m["value"]["metrics"][0]["value"], 900.0);
        assert_eq!(m["properties"]["Quality"]["value"], 192);
        assert_eq!(m["properties"]["EngUnit"]["value"], "rpm");
    }

    #[test]
    fn decodes_array_and_bytes_metrics() {
        let i32s: Vec<u8> = [1i32, -2, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
        let doubles: Vec<u8> = [1.5f64, -2.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        // 10 booleans: 1010 0000 01 -> count=10, bits MSB first.
        let mut bools = 10u32.to_le_bytes().to_vec();
        bools.extend([0b1010_0000, 0b0100_0000]);
        let strings = b"ab\0c\0".to_vec();

        let bytes = encode(
            vec![
                metric(Some("i32a"), None, 24, Some(MV::BytesValue(i32s))),
                metric(Some("f64a"), None, 31, Some(MV::BytesValue(doubles))),
                metric(Some("boola"), None, 32, Some(MV::BytesValue(bools))),
                metric(Some("stra"), None, 33, Some(MV::BytesValue(strings))),
                metric(Some("raw"), None, 17, Some(MV::BytesValue(vec![0xde, 0xad]))),
                metric(Some("bad"), None, 24, Some(MV::BytesValue(vec![1, 2, 3]))),
            ],
            0,
        );
        let v = decode_payload(&topic("spBv1.0/g/NDATA/e"), &bytes, &mut SparkplugAliases::new()).unwrap();
        let m = &v["metrics"];
        assert_eq!(m[0]["value"], json!([1, -2, 3]));
        assert_eq!(m[1]["value"], json!([1.5, -2.0]));
        assert_eq!(
            m[2]["value"],
            json!([true, false, true, false, false, false, false, false, false, true])
        );
        assert_eq!(m[3]["value"], json!(["ab", "c"]));
        assert_eq!(m[4]["value"], "dead");
        // A malformed array keeps the bytes and reports why.
        assert_eq!(m[5]["value"]["hex"], "010203");
        assert!(m[5]["value"]["error"].as_str().unwrap().contains("multiple of 4"));
    }

    #[test]
    fn payload_body_is_hex() {
        let bytes = pb::Payload {
            body: Some(vec![1, 2, 255]),
            uuid: Some("abc".into()),
            ..Default::default()
        }
        .encode_to_vec();
        let v = decode_payload(&topic("spBv1.0/g/NDATA/e"), &bytes, &mut SparkplugAliases::new()).unwrap();
        assert_eq!(v["body"], "0102ff");
        assert_eq!(v["uuid"], "abc");
        assert_eq!(v["metrics"], json!([]));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let mut aliases = SparkplugAliases::new();
        let t = topic("spBv1.0/g/NDATA/e");
        assert!(decode_payload(&t, b"{\"not\":\"protobuf\"}", &mut aliases).is_err());
        // Truncated mid-field.
        let good = encode(vec![metric(Some("a"), None, 10, Some(MV::DoubleValue(1.0)))], 0);
        assert!(decode_payload(&t, &good[..good.len() - 3], &mut aliases).is_err());
    }

    #[test]
    fn empty_payload_is_an_empty_message() {
        let v = decode_payload(&topic("spBv1.0/g/NDEATH/e"), &[], &mut SparkplugAliases::new()).unwrap();
        assert_eq!(v["metrics"], json!([]));
    }

    #[test]
    fn unknown_datatype_is_named() {
        assert_eq!(datatype_name(99), "Unknown(99)");
        assert_eq!(datatype_name(10), "Double");
    }
}
