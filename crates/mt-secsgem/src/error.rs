//! Error types.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The HSMS timers (E37). The name is what a timeout error reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timer {
    /// Reply timeout: no secondary message for a primary with the W-bit.
    T3,
    /// Connect separation timeout: minimum time between two connection attempts.
    T5,
    /// Control transaction timeout: no answer to Select, Deselect or Linktest.
    T6,
    /// Not-selected timeout: TCP connected but no Select within this time.
    T7,
    /// Network inter-character timeout: a message stopped arriving half way.
    T8,
}

impl fmt::Display for Timer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, what) = match self {
            Timer::T3 => ("T3", "reply timeout"),
            Timer::T5 => ("T5", "connect separation timeout"),
            Timer::T6 => ("T6", "control transaction timeout"),
            Timer::T7 => ("T7", "not-selected timeout"),
            Timer::T8 => ("T8", "network inter-character timeout"),
        };
        write!(f, "{name} {what}")
    }
}

/// Failures of the SECS-II item codec and the SML parser.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    #[error("message ends inside an item (need {needed} more bytes, have {have})")]
    Truncated { needed: usize, have: usize },
    #[error("unknown item format code {0:#04x}")]
    UnknownFormat(u8),
    #[error("item with zero length bytes (format byte {0:#04x})")]
    NoLengthBytes(u8),
    #[error("{kind} item of {len} bytes is not a multiple of {size}")]
    BadLength {
        kind: &'static str,
        len: usize,
        size: usize,
    },
    #[error("item of {0} elements does not fit in 3 length bytes")]
    TooLong(usize),
    #[error("lists nest deeper than {0} levels")]
    TooDeep(usize),
    #[error("{0} bytes follow the item")]
    TrailingBytes(usize),
    #[error("SML: {0}")]
    Sml(String),
}

/// Reason of a Reject.req (E37), byte 3 of its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// SType not supported.
    STypeNotSupported,
    /// PType not supported.
    PTypeNotSupported,
    /// Transaction not open: a reply arrived for no outstanding primary.
    TransactionNotOpen,
    /// Entity not selected: a data message arrived before Select.
    EntityNotSelected,
    Other(u8),
}

impl RejectReason {
    pub fn from_code(code: u8) -> Self {
        match code {
            1 => RejectReason::STypeNotSupported,
            2 => RejectReason::PTypeNotSupported,
            3 => RejectReason::TransactionNotOpen,
            4 => RejectReason::EntityNotSelected,
            c => RejectReason::Other(c),
        }
    }

    pub fn code(self) -> u8 {
        match self {
            RejectReason::STypeNotSupported => 1,
            RejectReason::PTypeNotSupported => 2,
            RejectReason::TransactionNotOpen => 3,
            RejectReason::EntityNotSelected => 4,
            RejectReason::Other(c) => c,
        }
    }
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RejectReason::STypeNotSupported => f.write_str("SType not supported"),
            RejectReason::PTypeNotSupported => f.write_str("PType not supported"),
            RejectReason::TransactionNotOpen => f.write_str("transaction not open"),
            RejectReason::EntityNotSelected => f.write_str("entity not selected"),
            RejectReason::Other(c) => write!(f, "reason {c}"),
        }
    }
}

/// Status byte of a Select.rsp (E37).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectStatus {
    Ok,
    AlreadyActive,
    NotReady,
    Exhausted,
    Other(u8),
}

impl SelectStatus {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => SelectStatus::Ok,
            1 => SelectStatus::AlreadyActive,
            2 => SelectStatus::NotReady,
            3 => SelectStatus::Exhausted,
            c => SelectStatus::Other(c),
        }
    }

    pub fn code(self) -> u8 {
        match self {
            SelectStatus::Ok => 0,
            SelectStatus::AlreadyActive => 1,
            SelectStatus::NotReady => 2,
            SelectStatus::Exhausted => 3,
            SelectStatus::Other(c) => c,
        }
    }
}

impl fmt::Display for SelectStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectStatus::Ok => f.write_str("communication established"),
            SelectStatus::AlreadyActive => f.write_str("communication already active"),
            SelectStatus::NotReady => f.write_str("connection not ready"),
            SelectStatus::Exhausted => f.write_str("connection exhausted"),
            SelectStatus::Other(c) => write!(f, "status {c}"),
        }
    }
}

/// The SECS-II stream 9 system error messages (E5). `S9F1`..`S9F13` carry the 10-byte header of
/// the message that caused them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum S9Kind {
    /// S9F1: the device id of the message is not the equipment's.
    UnrecognizedDeviceId,
    /// S9F3: no such stream.
    UnrecognizedStream,
    /// S9F5: no such function in a known stream.
    UnrecognizedFunction,
    /// S9F7: the data of the message do not follow its definition.
    IllegalData,
    /// S9F9: the transaction timer ran out.
    TransactionTimerTimeout,
    /// S9F11: message too long.
    DataTooLong,
    /// S9F13: conversation timeout.
    ConversationTimeout,
}

impl S9Kind {
    /// The function number of the S9 message.
    pub fn function(self) -> u8 {
        match self {
            S9Kind::UnrecognizedDeviceId => 1,
            S9Kind::UnrecognizedStream => 3,
            S9Kind::UnrecognizedFunction => 5,
            S9Kind::IllegalData => 7,
            S9Kind::TransactionTimerTimeout => 9,
            S9Kind::DataTooLong => 11,
            S9Kind::ConversationTimeout => 13,
        }
    }

    pub fn from_function(function: u8) -> Option<Self> {
        Some(match function {
            1 => S9Kind::UnrecognizedDeviceId,
            3 => S9Kind::UnrecognizedStream,
            5 => S9Kind::UnrecognizedFunction,
            7 => S9Kind::IllegalData,
            9 => S9Kind::TransactionTimerTimeout,
            11 => S9Kind::DataTooLong,
            13 => S9Kind::ConversationTimeout,
            _ => return None,
        })
    }
}

impl fmt::Display for S9Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            S9Kind::UnrecognizedDeviceId => "unrecognized device id",
            S9Kind::UnrecognizedStream => "unrecognized stream",
            S9Kind::UnrecognizedFunction => "unrecognized function",
            S9Kind::IllegalData => "illegal data",
            S9Kind::TransactionTimerTimeout => "transaction timer timeout",
            S9Kind::DataTooLong => "data too long",
            S9Kind::ConversationTimeout => "conversation timeout",
        };
        write!(f, "S9F{} {text}", self.function())
    }
}

/// A received S9Fx message: what went wrong and the header of the message that caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct S9Error {
    pub kind: S9Kind,
    /// The 10-byte header of the offending message (`MHEAD`).
    pub mhead: [u8; 10],
}

impl S9Error {
    /// System bytes of the offending message; they identify the transaction that failed.
    pub fn system_bytes(&self) -> u32 {
        u32::from_be_bytes([self.mhead[6], self.mhead[7], self.mhead[8], self.mhead[9]])
    }

    /// Stream of the offending message.
    pub fn stream(&self) -> u8 {
        self.mhead[2] & 0x7f
    }

    /// Function of the offending message.
    pub fn function(&self) -> u8 {
        self.mhead[3]
    }
}

impl fmt::Display for S9Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (for S{}F{}, system bytes {:#010x})",
            self.kind,
            self.stream(),
            self.function(),
            self.system_bytes()
        )
    }
}

/// Everything a connection or a GEM call can fail with.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum SecsError {
    #[error("Not connected")]
    NotConnected,
    #[error("HSMS session is not selected")]
    NotSelected,
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("I/O error: {0}")]
    Io(String),
    /// A timer ran out. T3 and T6 fail one call; T7 and T8 (and T6 on a Select or Linktest)
    /// also drop the connection.
    #[error("Timeout: {0}")]
    Timeout(Timer),
    #[error("The peer rejected the message: {reason}")]
    Rejected { reason: RejectReason, code: u8 },
    #[error("Select refused: {0}")]
    SelectRefused(SelectStatus),
    #[error("Deselect refused, status {0}")]
    DeselectRefused(u8),
    /// The peer answered with `S{stream}F0`.
    #[error("The peer aborted the transaction (S{stream}F0)")]
    Aborted { stream: u8 },
    /// The peer answered with an S9Fx for this transaction.
    #[error("The peer answered with a stream 9 error: {0}")]
    S9(S9Error),
    #[error("Malformed data: {0}")]
    Codec(#[from] CodecError),
    #[error("HSMS protocol violation: {0}")]
    Protocol(String),
    /// A reply of the right stream and function whose item does not have the documented shape.
    #[error("Unexpected reply to {call}: {detail}")]
    UnexpectedReply { call: &'static str, detail: String },
    #[error("The connection closed")]
    Closed,
}

impl SecsError {
    /// The timer behind a timeout error.
    pub fn timer(&self) -> Option<Timer> {
        match self {
            SecsError::Timeout(t) => Some(*t),
            _ => None,
        }
    }
}

pub type Result<T, E = SecsError> = std::result::Result<T, E>;
