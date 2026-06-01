//! Core error type.

use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("engine error: {0}")]
    Engine(#[from] engine::EngineError),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("download id {0} not found")]
    DownloadNotFound(i64),

    #[error("category id {0} not found")]
    CategoryNotFound(i64),

    #[error("category not found: {0}")]
    CategoryNameNotFound(String),

    #[error("invalid status transition for id {id}: {from} -> {to}")]
    InvalidTransition { id: i64, from: String, to: String },

    #[error("invalid status string: {0}")]
    InvalidStatus(String),

    #[error("invalid setting value for {key}: {message}")]
    InvalidSetting { key: String, message: String },

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("download does not support resumable byte ranges")]
    NotResumable,

    #[error("control channel closed before request could be acknowledged")]
    ControlClosed,
}

impl CoreError {
    pub fn io(e: io::Error) -> Self {
        CoreError::Io(e)
    }
}
