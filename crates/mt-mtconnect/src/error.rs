use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::Header;

/// Error code carried by an `MTConnectError` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "raw")]
pub enum AgentErrorCode {
    Unauthorized,
    NoDevice,
    OutOfRange,
    TooManyDevices,
    Unsupported,
    InvalidRequest,
    InvalidXPath,
    InvalidUri,
    /// A query parameter has a value of the wrong type (2.x agents).
    InvalidParameterValue,
    AssetNotFound,
    InternalError,
    /// Any code this crate does not know, kept verbatim.
    Other(String),
}

impl AgentErrorCode {
    /// Map the `errorCode` attribute of an `Error` element.
    pub fn from_code(code: &str) -> Self {
        match code {
            "UNAUTHORIZED" => Self::Unauthorized,
            "NO_DEVICE" => Self::NoDevice,
            "OUT_OF_RANGE" => Self::OutOfRange,
            "TOO_MANY_DEVICES" => Self::TooManyDevices,
            "UNSUPPORTED" => Self::Unsupported,
            "INVALID_REQUEST" => Self::InvalidRequest,
            "INVALID_XPATH" => Self::InvalidXPath,
            "INVALID_URI" => Self::InvalidUri,
            "INVALID_PARAMETER_VALUE" => Self::InvalidParameterValue,
            "ASSET_NOT_FOUND" => Self::AssetNotFound,
            "INTERNAL_ERROR" => Self::InternalError,
            other => Self::Other(other.to_string()),
        }
    }

    /// The `errorCode` attribute value for this code.
    pub fn as_code(&self) -> &str {
        match self {
            Self::Unauthorized => "UNAUTHORIZED",
            Self::NoDevice => "NO_DEVICE",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::TooManyDevices => "TOO_MANY_DEVICES",
            Self::Unsupported => "UNSUPPORTED",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::InvalidXPath => "INVALID_XPATH",
            Self::InvalidUri => "INVALID_URI",
            Self::InvalidParameterValue => "INVALID_PARAMETER_VALUE",
            Self::AssetNotFound => "ASSET_NOT_FOUND",
            Self::InternalError => "INTERNAL_ERROR",
            Self::Other(s) => s,
        }
    }
}

/// One `Error` entry of an `MTConnectError` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentError {
    pub code: AgentErrorCode,
    pub message: String,
}

/// A parsed `MTConnectError` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentErrors {
    pub header: Header,
    pub errors: Vec<AgentError>,
}

impl AgentErrors {
    pub fn has_code(&self, code: &AgentErrorCode) -> bool {
        self.errors.iter().any(|e| &e.code == code)
    }
}

impl std::fmt::Display for AgentErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{}: {}", e.code.as_code(), e.message)?;
        }
        Ok(())
    }
}

/// Errors from the MTConnect client, parser and simulator.
#[derive(Error, Debug)]
pub enum MtConnectError {
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Agent returned HTTP {status}")]
    Status { status: u16 },
    /// The agent answered with an `MTConnectError` document.
    #[error("Agent error: {0}")]
    Agent(Box<AgentErrors>),
    #[error("XML error: {0}")]
    Xml(String),
    #[error("Unexpected document: expected {expected}, got {found}")]
    UnexpectedDocument { expected: &'static str, found: String },
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Simulator error: {0}")]
    Simulator(String),
    #[error("I/O error: {0}")]
    Io(String),
}

impl MtConnectError {
    /// True if the agent reported `OUT_OF_RANGE` (the requested sequence left the buffer).
    pub fn is_out_of_range(&self) -> bool {
        matches!(self, Self::Agent(e) if e.has_code(&AgentErrorCode::OutOfRange))
    }
}

impl From<std::io::Error> for MtConnectError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl serde::Serialize for MtConnectError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MtConnectError>;
