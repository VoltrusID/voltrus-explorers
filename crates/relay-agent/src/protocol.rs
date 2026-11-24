//! Wire protocol shared between the relay session loop and the drivers.
//!
//! The agent speaks JSON text frames with the RelayRoom Durable Object:
//!   receives  {deviceId, id, op, payload}   (ide requests forwarded by the DO)
//!   sends     {id, ok, payload | error}     (responses, routed by id)
//!   sends     {event, payload}              (unsolicited broadcasts to ides)
//!   sends     {op: "hello", payload}        (one-shot greeting; ignored by the DO)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request frame as forwarded by the DO. `payload` is opaque, op-specific.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceFrame {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub id: String,
    pub op: String,
    #[serde(default)]
    pub payload: Option<Value>,
}

/// Response to a specific request id; the DO routes it back to the asking ide.
#[derive(Debug, Serialize)]
pub struct ResponseFrame {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ResponseFrame {
    pub fn ok(id: &str, payload: Value) -> Self {
        ResponseFrame {
            id: id.to_owned(),
            ok: true,
            payload: Some(payload),
            error: None,
        }
    }

    pub fn err(id: &str, error: impl Into<String>) -> Self {
        ResponseFrame {
            id: id.to_owned(),
            ok: false,
            payload: None,
            error: Some(error.into()),
        }
    }
}

/// Unsolicited event, broadcast by the DO to every connected ide.
#[derive(Debug, Clone, Serialize)]
pub struct EventFrame {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// One variable as reported by `browse`.
#[derive(Debug, Clone, Serialize)]
pub struct VarInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forced: Option<bool>,
}
