// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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

    /// Contact encryption or decryption failed.
    #[error("Contact encryption error: {0}")]
    ContactEncryption(String),

    /// Attachment exceeds the workspace size limit.
    #[error("Attachment too large: {size} bytes (limit: {limit} bytes)")]
    AttachmentTooLarge { size: u64, limit: u64 },

    #[error("Identity not found: {0}")]
    IdentityNotFound(String),

    #[error("Identity already exists: {0}")]
    IdentityAlreadyExists(String),

    #[error("Wrong passphrase for identity")]
    IdentityWrongPassphrase,

    #[error("Identity file corrupt: {0}")]
    IdentityCorrupt(String),

    #[error("Cannot delete identity with bound workspaces: {0}")]
    IdentityHasBoundWorkspaces(String),

    #[error("Workspace not bound to any identity: {0}")]
    WorkspaceNotBound(String),

    #[error("Invalid .swarmid file: {0}")]
    SwarmIdInvalidFormat(String),

    #[error("Unsupported .swarmid version: {0}")]
    SwarmIdVersionUnsupported(u32),

    /// A `.swarm` bundle operation failed.
    #[error("swarm: {0}")]
    Swarm(String),

    /// A low-level cryptographic operation failed (e.g. blob encrypt/decrypt).
    #[error("crypto: {0}")]
    Crypto(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Invite expired or revoked")]
    InviteExpiredOrRevoked,

    /// A zip archive operation failed (bundle encoding/decoding).
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Only the workspace owner can modify scripts")]
    NotOwner,

    /// Relay session has expired or token is invalid (HTTP 401).
    #[error("Relay auth expired: {0}")]
    RelayAuthExpired(String),

    /// Relay rate limit exceeded (HTTP 429).
    #[error("Relay rate limited: {0}")]
    RelayRateLimited(String),

    /// Relay resource not found or expired (HTTP 404/410).
    #[error("Relay not found: {0}")]
    RelayNotFound(String),

    /// Relay server unreachable or returned a server error.
    #[error("Relay unavailable: {0}")]
    RelayUnavailable(String),
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
            Self::ContactEncryption(msg) => msg.clone(),
            Self::AttachmentTooLarge { size, limit } => {
                format!("File too large ({} bytes). This workspace limits attachments to {} bytes.", size, limit)
            }
            Self::IdentityNotFound(id) => format!("Identity not found: {id}"),
            Self::IdentityAlreadyExists(name) => {
                format!("An identity named \"{name}\" already exists.")
            }
            Self::IdentityWrongPassphrase => {
                "Wrong passphrase. Please check your passphrase and try again.".to_string()
            }
            Self::IdentityCorrupt(msg) => {
                format!("Identity file is corrupt or unreadable: {msg}")
            }
            Self::IdentityHasBoundWorkspaces(id) => {
                format!("Cannot delete identity {id} — it still has workspaces bound to it. Unbind all workspaces first.")
            }
            Self::WorkspaceNotBound(id) => {
                format!("Workspace {id} is not bound to any identity.")
            }
            Self::SwarmIdInvalidFormat(msg) => {
                format!("The .swarmid file is invalid: {msg}")
            }
            Self::SwarmIdVersionUnsupported(v) => {
                format!("This .swarmid file uses version {v}, which is not supported by this version of Krillnotes.")
            }
            Self::Swarm(msg) => format!("Swarm bundle error: {msg}"),
            Self::Crypto(msg) => format!("Cryptography error: {msg}"),
            Self::InvalidSignature => "Invalid signature — the file may be tampered or corrupted.".to_string(),
            Self::InviteExpiredOrRevoked => "This invite has expired or been revoked.".to_string(),
            Self::Zip(e) => format!("Bundle archive error: {e}"),
            Self::NotOwner => "Only the workspace owner can modify scripts".to_string(),
            Self::RelayAuthExpired(_) => "Relay session expired. Please log in again.".to_string(),
            Self::RelayRateLimited(_) => "Relay is rate limiting requests. Please try again later.".to_string(),
            Self::RelayNotFound(_) => "The requested relay resource was not found or has expired.".to_string(),
            Self::RelayUnavailable(msg) => format!("Relay server unavailable: {msg}"),
        }
    }
}
