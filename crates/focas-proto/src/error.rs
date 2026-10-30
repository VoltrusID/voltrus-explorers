//! Error types.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// FOCAS data-window return codes (`EW_*`), as listed in FANUC's FOCAS documentation.
///
/// The CNC puts one of these in every sub-response; 0 is success.
pub mod ew {
    pub const OK: i16 = 0;
    /// Function not executed or not available.
    pub const FUNC: i16 = 1;
    /// Data block length error / error of number of data.
    pub const LENGTH: i16 = 2;
    /// Data number error.
    pub const NUMBER: i16 = 3;
    /// Data attribute error.
    pub const ATTRIB: i16 = 4;
    /// Data error.
    pub const DATA: i16 = 5;
    /// No option: the CNC lacks the corresponding option.
    pub const NOOPT: i16 = 6;
    /// Write protection.
    pub const PROT: i16 = 7;
    /// Memory overflow.
    pub const OVRFLOW: i16 = 8;
    /// CNC parameter error.
    pub const PARAM: i16 = 9;
    /// Buffer empty/full.
    pub const BUFFER: i16 = 10;
    /// Path number error.
    pub const PATH: i16 = 11;
    /// CNC mode error.
    pub const MODE: i16 = 12;
    /// CNC execution rejection.
    pub const REJECT: i16 = 13;
    /// Busy.
    pub const BUSY: i16 = -1;
    /// Reset or stop request.
    pub const RESET: i16 = -2;
}

/// FOCAS name of a return code, e.g. `EW_NOOPT` for 6.
pub fn ew_name(code: i16) -> &'static str {
    match code {
        0 => "EW_OK",
        1 => "EW_FUNC",
        2 => "EW_LENGTH",
        3 => "EW_NUMBER",
        4 => "EW_ATTRIB",
        5 => "EW_DATA",
        6 => "EW_NOOPT",
        7 => "EW_PROT",
        8 => "EW_OVRFLOW",
        9 => "EW_PARAM",
        10 => "EW_BUFFER",
        11 => "EW_PATH",
        12 => "EW_MODE",
        13 => "EW_REJECT",
        -1 => "EW_BUSY",
        -2 => "EW_RESET",
        -6 => "EW_UNEXP",
        -7 => "EW_VERSION",
        -8 => "EW_HANDLE",
        _ => "EW_?",
    }
}

/// Everything a client call can fail with.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum FocasError {
    #[error("Not connected")]
    NotConnected,
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("I/O error: {0}")]
    Io(String),
    /// The bytes on the wire do not follow the framing.
    #[error("Protocol error: {0}")]
    Protocol(String),
    /// The CNC answered the call with a non-zero `EW_*` code.
    #[error("{call}: CNC returned {} ({code})", ew_name(*.code))]
    Cnc { call: &'static str, code: i16 },
    /// The CNC answered successfully but the payload is not what the reference documents.
    #[error("{call}: unexpected payload: {detail}")]
    UnexpectedPayload { call: &'static str, detail: String },
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

impl Serialize for FocasError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<crate::codec::CodecError> for FocasError {
    fn from(e: crate::codec::CodecError) -> Self {
        FocasError::Protocol(e.to_string())
    }
}

/// Why one item of a multi-request (see [`Snapshot`](crate::Snapshot)) has no value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadFailure {
    /// The `EW_*` code when the CNC refused the item; `None` for a payload that did not parse.
    pub code: Option<i16>,
    pub message: String,
}

impl From<&FocasError> for ReadFailure {
    fn from(e: &FocasError) -> Self {
        let code = match e {
            FocasError::Cnc { code, .. } => Some(*code),
            _ => None,
        };
        ReadFailure {
            code,
            message: e.to_string(),
        }
    }
}

/// One value of a multi-request: what the CNC answered, or why it did not.
///
/// A CNC refuses single items of a multi-request (for example an option it lacks); the other
/// items still carry data, so a failed item must not fail the whole poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum Reading<T> {
    Value(T),
    Failed(ReadFailure),
}

impl<T> Reading<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Reading::Value(v) => Some(v),
            Reading::Failed(_) => None,
        }
    }

    pub fn into_result(self) -> Result<T, ReadFailure> {
        match self {
            Reading::Value(v) => Ok(v),
            Reading::Failed(f) => Err(f),
        }
    }

    pub fn is_value(&self) -> bool {
        matches!(self, Reading::Value(_))
    }
}

impl<T> From<Result<T, FocasError>> for Reading<T> {
    fn from(r: Result<T, FocasError>) -> Self {
        match r {
            Ok(v) => Reading::Value(v),
            Err(e) => Reading::Failed(ReadFailure::from(&e)),
        }
    }
}
