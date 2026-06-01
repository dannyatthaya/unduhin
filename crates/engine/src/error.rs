//! Engine error types.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("server returned terminal status {status}")]
    TerminalStatus { status: u16 },

    #[error("server returned transient status {status}")]
    TransientStatus { status: u16 },

    #[error("I/O error at {path:?}: {source}")]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: io::Error,
    },

    #[error("sidecar metadata error: {0}")]
    Meta(String),

    #[error("remote file changed since last attempt (etag/last-modified mismatch)")]
    RemoteChanged,

    #[error("download cancelled")]
    Cancelled,

    #[error("retry budget exhausted: {last}")]
    RetryExhausted { last: String },

    #[error("integrity error: {0}")]
    Integrity(String),

    #[error("response body ended early: got {actual} of {expected} expected bytes")]
    BodyTruncated { expected: u64, actual: u64 },

    #[error("other: {0}")]
    Other(String),
}

impl EngineError {
    pub fn io(path: impl Into<Option<PathBuf>>, source: io::Error) -> Self {
        EngineError::Io {
            path: path.into(),
            source,
        }
    }

    pub fn meta(msg: impl Into<String>) -> Self {
        EngineError::Meta(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        EngineError::Other(msg.into())
    }
}
