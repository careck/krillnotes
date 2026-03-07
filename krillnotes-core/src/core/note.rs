// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Note data types for the Krillnotes workspace.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Custom deserializer for `created_by` / `modified_by` that accepts both
/// the legacy integer format (always `0`) and the new base64 string format.
fn deserialize_author_field<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or integer author field")
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<String, E> { Ok(v.to_string()) }
        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<String, E> { Ok(v) }
        // Legacy: old archives serialized created_by/modified_by as integer 0.
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<String, E> { Ok(String::new()) }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<String, E> { Ok(String::new()) }
    }
    deserializer.deserialize_any(Visitor)
}

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
    /// A reference to another note by UUID. `None` = not set, `Some(uuid)` = linked note ID.
    /// Serializes as JSON `null` or `"uuid-string"`.
    NoteLink(Option<String>),
    /// A reference to an attachment by UUID. `None` means "not set".
    File(Option<String>),
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
    /// Fractional sort order among siblings that share the same `parent_id`.
    pub position: f64,
    /// Unix timestamp (seconds) when this note was created.
    pub created_at: i64,
    /// Unix timestamp (seconds) of the most recent modification.
    pub modified_at: i64,
    /// Base64-encoded Ed25519 public key of the identity that created this note.
    /// Empty string for notes created before identity enforcement was added.
    #[serde(default, deserialize_with = "deserialize_author_field")]
    pub created_by: String,
    /// Base64-encoded Ed25519 public key of the identity that last modified this note.
    /// Empty string for notes modified before identity enforcement was added.
    #[serde(default, deserialize_with = "deserialize_author_field")]
    pub modified_by: String,
    /// Schema-defined field values keyed by field name.
    pub fields: BTreeMap<String, FieldValue>,
    /// Whether this node is currently expanded in the tree UI.
    pub is_expanded: bool,
    /// Sorted, lowercase tags attached to this note.
    /// `#[serde(default)]` allows importing archives from before the tags feature.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Schema version this note was created/migrated with.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

fn default_schema_version() -> u32 { 1 }

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
            position: 0.0,
            created_at: 1234567890,
            modified_at: 1234567890,
            created_by: String::new(),
            modified_by: String::new(),
            fields: BTreeMap::new(),
            is_expanded: true,
            tags: vec![], schema_version: 1,
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
    fn test_boolean_field_value_serde() {
        let t = FieldValue::Boolean(true);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#"{"Boolean":true}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);

        let f = FieldValue::Boolean(false);
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, r#"{"Boolean":false}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn test_email_field_value_serde() {
        let email = FieldValue::Email("test@example.com".to_string());
        let json = serde_json::to_string(&email).unwrap();
        assert_eq!(json, r#"{"Email":"test@example.com"}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, email);
    }

    #[test]
    fn test_note_link_field_value_serializes_to_null() {
        let v = FieldValue::NoteLink(None);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"NoteLink":null}"#);
    }

    #[test]
    fn test_note_link_field_value_serializes_to_string() {
        let v = FieldValue::NoteLink(Some("abc-123".into()));
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"NoteLink":"abc-123"}"#);
    }

    #[test]
    fn test_note_link_field_value_round_trips() {
        let v = FieldValue::NoteLink(Some("test-uuid-456".into()));
        let json = serde_json::to_string(&v).unwrap();
        let v2: FieldValue = serde_json::from_str(&json).unwrap();
        // Verify round-trip via re-serialization
        assert_eq!(serde_json::to_string(&v2).unwrap(), json);
    }

    #[test]
    fn test_field_value_file_roundtrip() {
        let v = FieldValue::File(Some("abc-123".to_string()));
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"File":"abc-123"}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        match back {
            FieldValue::File(Some(id)) => assert_eq!(id, "abc-123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_field_value_file_none_roundtrip() {
        let v = FieldValue::File(None);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"File":null}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, FieldValue::File(None)));
    }
}
