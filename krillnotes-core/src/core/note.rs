//! Note data types for the Krillnotes workspace.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A typed value stored in a note's schema-defined fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    /// A plain-text string.
    Text(String),
    /// A 64-bit floating-point number.
    Number(f64),
    /// A boolean flag.
    Boolean(bool),
    /// A calendar date. `None` represents "not set".
    /// Serializes as ISO 8601 `"YYYY-MM-DD"` or JSON `null`.
    Date(Option<NaiveDate>),
    /// An email address string. Format is validated client-side.
    Email(String),
}

/// A single node in the workspace hierarchy.
///
/// Notes form a tree via `parent_id`; siblings are ordered by `position`.
/// Each note has a `node_type` that maps to a [`crate::Schema`] and a set
/// of typed `fields` validated against that schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// Stable UUID identifying this note.
    pub id: String,
    /// Human-readable title shown in the tree view.
    pub title: String,
    /// Schema name governing this note's `fields` (e.g. `"TextNote"`).
    pub node_type: String,
    /// ID of the parent note, or `None` for root-level notes.
    pub parent_id: Option<String>,
    /// Zero-based sort order among siblings that share the same `parent_id`.
    pub position: i32,
    /// Unix timestamp (seconds) when this note was created.
    pub created_at: i64,
    /// Unix timestamp (seconds) of the most recent modification.
    pub modified_at: i64,
    /// Device ID that created this note.
    pub created_by: i64,
    /// Device ID that last modified this note.
    pub modified_by: i64,
    /// Schema-defined field values keyed by field name.
    pub fields: HashMap<String, FieldValue>,
    /// Whether this node is currently expanded in the tree UI.
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

    #[test]
    fn test_date_field_value_serde() {
        // None round-trips as null
        let none = FieldValue::Date(None);
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, r#"{"Date":null}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, none);

        // Some(date) round-trips as ISO string
        use chrono::NaiveDate;
        let date = NaiveDate::from_ymd_opt(2026, 2, 19).unwrap();
        let some = FieldValue::Date(Some(date));
        let json = serde_json::to_string(&some).unwrap();
        assert_eq!(json, r#"{"Date":"2026-02-19"}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, some);
    }

    #[test]
    fn test_email_field_value_serde() {
        let email = FieldValue::Email("test@example.com".to_string());
        let json = serde_json::to_string(&email).unwrap();
        assert_eq!(json, r#"{"Email":"test@example.com"}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, email);
    }
}
