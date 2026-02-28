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

    /// A move operation would create a cycle or is otherwise invalid.
    #[error("Invalid move: {0}")]
    InvalidMove(String),

    /// The opened file is not a valid Krillnotes workspace.
    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),

    /// The supplied password is wrong for this workspace.
    #[error("Wrong password for this workspace")]
    WrongPassword,

    /// The file is a valid but unencrypted (pre-encryption) workspace.
    #[error("This workspace was created with an older version of Krillnotes and cannot be opened here")]
    UnencryptedWorkspace,

    /// An I/O operation on the filesystem failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Stored note data could not be deserialized from JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Attachment encryption or decryption failed.
    #[error("Attachment encryption error: {0}")]
    AttachmentEncryption(String),

    /// Attachment exceeds the workspace size limit.
    #[error("Attachment too large: {size} bytes (limit: {limit} bytes)")]
    AttachmentTooLarge { size: u64, limit: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrong_password_variant_exists() {
        let e = KrillnotesError::WrongPassword;
        assert!(e.to_string().contains("password") || e.to_string().contains("Password"));
    }

    #[test]
    fn test_unencrypted_workspace_variant_exists() {
        let e = KrillnotesError::UnencryptedWorkspace;
        assert!(e.to_string().contains("encrypted") || e.to_string().contains("older version"));
    }

    #[test]
    fn test_attachment_error_variants_exist() {
        let e = KrillnotesError::AttachmentEncryption("bad key".to_string());
        assert!(e.to_string().contains("encryption") || e.to_string().contains("Encryption"));

        let e2 = KrillnotesError::AttachmentTooLarge { size: 200, limit: 100 };
        assert!(e2.to_string().contains("200"));
    }
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
            Self::InvalidMove(msg) => msg.clone(),
            Self::WrongPassword => "Wrong password — please try again".to_string(),
            Self::UnencryptedWorkspace => "This workspace was created with an older version of Krillnotes. Please open it in the previous version, export it via File → Export Workspace, then import it here.".to_string(),
            Self::AttachmentEncryption(_) => "Could not encrypt or decrypt the attachment".to_string(),
            Self::AttachmentTooLarge { size, limit } => {
                format!("File too large ({} bytes). This workspace limits attachments to {} bytes.", size, limit)
            }
        }
    }
}
