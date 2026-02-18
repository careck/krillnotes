use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Boolean(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub node_type: String,
    pub parent_id: Option<String>,
    pub position: i32,
    pub created_at: i64,
    pub modified_at: i64,
    pub created_by: i64,
    pub modified_by: i64,
    pub fields: HashMap<String, FieldValue>,
    pub is_expanded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_note() {
        let note = Note {
            id: "test-id".to_string(),
            title: "Test Note".to_string(),
            node_type: "TextNote".to_string(),
            parent_id: None,
            position: 0,
            created_at: 1234567890,
            modified_at: 1234567890,
            created_by: 0,
            modified_by: 0,
            fields: HashMap::new(),
            is_expanded: true,
        };

        assert_eq!(note.title, "Test Note");
        assert_eq!(note.node_type, "TextNote");
        assert!(note.parent_id.is_none());
    }

    #[test]
    fn test_field_value_text() {
        let value = FieldValue::Text("Hello".to_string());
        match value {
            FieldValue::Text(s) => assert_eq!(s, "Hello"),
            _ => panic!("Wrong variant"),
        }
    }
}
