//! CRDT-style operation types for the Krillnotes operation log.

use crate::FieldValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single document mutation recorded in the workspace operation log.
///
/// Operations capture the full intent of each change so they can be
/// replayed, merged, or synced across devices in a future sync phase.
/// Every variant carries a stable `operation_id`, a wall-clock `timestamp`,
/// and the `device_id` of the originating machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Operation {
    /// A new note was inserted into the workspace hierarchy.
    CreateNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID assigned to the new note.
        note_id: String,
        /// Parent note ID, or `None` for a root note.
        parent_id: Option<String>,
        /// Zero-based position among siblings.
        position: i32,
        /// Schema type of the new note.
        node_type: String,
        /// Initial title of the new note.
        title: String,
        /// Initial field values of the new note.
        fields: HashMap<String, FieldValue>,
        /// Device ID logged as the creator.
        created_by: i64,
    },
    /// A single schema field on an existing note was updated.
    UpdateField {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note whose field was updated.
        note_id: String,
        /// Name of the field that changed.
        field: String,
        /// New value for the field.
        value: FieldValue,
        /// Device ID logged as the modifier.
        modified_by: i64,
    },
    /// A note (and all its descendants) was deleted.
    DeleteNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the deleted note.
        note_id: String,
    },
    /// A note was relocated to a new parent or position.
    MoveNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note that was moved.
        note_id: String,
        /// New parent note ID, or `None` to move to root level.
        new_parent_id: Option<String>,
        /// New zero-based position among siblings.
        new_position: i32,
    },
    /// A new user script was created.
    CreateUserScript {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID assigned to the new script.
        script_id: String,
        /// Script name (from front matter).
        name: String,
        /// Script description (from front matter).
        description: String,
        /// Full Rhai source code.
        source_code: String,
        /// Position in load order.
        load_order: i32,
        /// Whether the script is active.
        enabled: bool,
    },
    /// An existing user script was modified (source, enabled state, or load order).
    UpdateUserScript {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the script that was modified.
        script_id: String,
        /// Updated script name.
        name: String,
        /// Updated script description.
        description: String,
        /// Updated full source code.
        source_code: String,
        /// Updated load order.
        load_order: i32,
        /// Updated enabled state.
        enabled: bool,
    },
    /// A user script was deleted.
    DeleteUserScript {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the deleted script.
        script_id: String,
    },
}

impl Operation {
    /// Returns the stable identifier for this operation.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        match self {
            Self::CreateNote { operation_id, .. }
            | Self::UpdateField { operation_id, .. }
            | Self::DeleteNote { operation_id, .. }
            | Self::MoveNote { operation_id, .. }
            | Self::CreateUserScript { operation_id, .. }
            | Self::UpdateUserScript { operation_id, .. }
            | Self::DeleteUserScript { operation_id, .. } => operation_id,
        }
    }

    /// Returns the wall-clock Unix timestamp (seconds) when this operation was created.
    #[must_use]
    pub fn timestamp(&self) -> i64 {
        match self {
            Self::CreateNote { timestamp, .. }
            | Self::UpdateField { timestamp, .. }
            | Self::DeleteNote { timestamp, .. }
            | Self::MoveNote { timestamp, .. }
            | Self::CreateUserScript { timestamp, .. }
            | Self::UpdateUserScript { timestamp, .. }
            | Self::DeleteUserScript { timestamp, .. } => *timestamp,
        }
    }

    /// Returns the device identifier of the machine that created this operation.
    #[must_use]
    pub fn device_id(&self) -> &str {
        match self {
            Self::CreateNote { device_id, .. }
            | Self::UpdateField { device_id, .. }
            | Self::DeleteNote { device_id, .. }
            | Self::MoveNote { device_id, .. }
            | Self::CreateUserScript { device_id, .. }
            | Self::UpdateUserScript { device_id, .. }
            | Self::DeleteUserScript { device_id, .. } => device_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_serialization() {
        let op = Operation::CreateNote {
            operation_id: "op-123".to_string(),
            timestamp: 1234567890,
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            parent_id: None,
            position: 0,
            node_type: "TextNote".to_string(),
            title: "Test".to_string(),
            fields: HashMap::new(),
            created_by: 0,
        };

        let json = serde_json::to_string(&op).unwrap();
        let deserialized: Operation = serde_json::from_str(&json).unwrap();

        assert_eq!(op.operation_id(), deserialized.operation_id());
    }
}
