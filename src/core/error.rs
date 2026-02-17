use thiserror::Error;

#[derive(Debug, Error)]
pub enum KrillnotesError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Scripting error: {0}")]
    Scripting(String),

    #[error("Schema not found: {0}")]
    SchemaNotFound(String),

    #[error("Note not found: {0}")]
    NoteNotFound(String),

    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, KrillnotesError>;

impl KrillnotesError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Database(e) => format!("Failed to save: {}", e),
            Self::SchemaNotFound(name) => format!("Unknown note type: {}", name),
            Self::NoteNotFound(_) => "Note no longer exists".to_string(),
            Self::InvalidWorkspace(_) => "Could not open workspace file".to_string(),
            Self::Scripting(e) => format!("Script error: {}", e),
            Self::Io(e) => format!("File error: {}", e),
            Self::Json(e) => format!("Data format error: {}", e),
        }
    }
}
