use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanError {
    #[error("{0}")]
    Frame(String),
    #[error("DBC line {line}: {msg}")]
    Dbc { line: usize, msg: String },
    #[error("log line {line}: {msg}")]
    Log { line: usize, msg: String },
    #[error("adapter: {0}")]
    Adapter(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the bus is closed")]
    Closed,
    #[error("{0}")]
    Unsupported(String),
}
