use thiserror::Error;

#[derive(Debug, Error)]
pub enum LogicError {
    #[error("{0}")]
    Format(String),
    #[error("{0}")]
    Config(String),
    #[error("USB: {0}")]
    Usb(String),
    #[error("{0}")]
    Firmware(String),
    #[error("no logic analyzer found")]
    NoDevice,
    #[error("{0}")]
    Permission(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("capture stopped")]
    Stopped,
}

pub type Result<T> = std::result::Result<T, LogicError>;
