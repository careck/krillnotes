use crate::FieldValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Operation {
    CreateNote {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
        parent_id: Option<String>,
        position: i32,
        node_type: String,
        title: String,
        fields: HashMap<String, FieldValue>,
        created_by: i64,
    },
    UpdateField {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
        field: String,
        value: FieldValue,
        modified_by: i64,
    },
    DeleteNote {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
    },
    MoveNote {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
        new_parent_id: Option<String>,
        new_position: i32,
    },
}

impl Operation {
    pub fn operation_id(&self) -> &str {
        match self {
            Self::CreateNote { operation_id, .. } => operation_id,
            Self::UpdateField { operation_id, .. } => operation_id,
            Self::DeleteNote { operation_id, .. } => operation_id,
            Self::MoveNote { operation_id, .. } => operation_id,
        }
    }

    pub fn timestamp(&self) -> i64 {
        match self {
            Self::CreateNote { timestamp, .. } => *timestamp,
            Self::UpdateField { timestamp, .. } => *timestamp,
            Self::DeleteNote { timestamp, .. } => *timestamp,
            Self::MoveNote { timestamp, .. } => *timestamp,
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
