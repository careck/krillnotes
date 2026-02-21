//! Error types for the Krillnotes core library.

use thiserror::Error;

/// All errors that can occur within the Krillnotes core library.
#[derive(Debug, Error)]
pub enum KrillnotesError {
    /// A SQLite operation failed.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// A Rhai script failed to parse or execute.
    #[error("Scripting error: {0}")]
    Scripting(String),

    /// A schema was requested that has not been registered.
    #[error("Schema not found: {0}")]
    SchemaNotFound(String),

    /// A note ID was requested that does not exist in the database.
    #[error("Note not found: {0}")]
    NoteNotFound(String),

    /// A required field was empty when trying to save a note.
    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    /// The opened file is not a valid Krillnotes workspace.
    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),

    /// An I/O operation on the filesystem failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Stored note data could not be deserialized from JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience alias that pins the error type to [`KrillnotesError`].
pub type Result<T> = std::result::Result<T, KrillnotesError>;

impl KrillnotesError {
    /// Returns a short, human-readable message suitable for display to the end user.
    #[must_use]
    pub fn user_message(&self) -> String {
        match self {
            Self::Database(e) => format!("Failed to save: {e}"),
            Self::SchemaNotFound(name) => format!("Unknown note type: {name}"),
            Self::NoteNotFound(_) => "Note no longer exists".to_string(),
            Self::InvalidWorkspace(_) => "Could not open workspace file".to_string(),
            Self::Scripting(e) => format!("Script error: {e}"),
            Self::Io(e) => format!("File error: {e}"),
            Self::Json(e) => format!("Data format error: {e}"),
            Self::ValidationFailed(msg) => msg.clone(),
        }
    }
}
