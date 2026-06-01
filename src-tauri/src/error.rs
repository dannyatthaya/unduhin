//! Bridge `unduhin_core::CoreError` into a Tauri-command-friendly error
//! that serializes to a flat `{ "message": "…" }` JSON object.
//!
//! The Tauri command boundary requires `Serialize` errors; `CoreError`
//! contains non-`Serialize` variants (`sqlx::Error`, `io::Error`, …) so
//! this wrapper does the conversion once at the edge.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub message: String,
}

impl<E: std::fmt::Display> From<E> for CommandError {
    fn from(e: E) -> Self {
        CommandError {
            message: e.to_string(),
        }
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
