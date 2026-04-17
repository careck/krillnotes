// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Gated operations model for Rhai write paths.
//!
//! [`SaveTransaction`] collects pending field/title writes and soft errors
//! from `set_field()`, `set_title()`, `reject()`, and `commit()` calls
//! during on_save hooks, tree actions, and on_add_child hooks.

use std::collections::BTreeMap;
use crate::core::note::FieldValue;

/// A soft validation error accumulated by `reject()`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoftError {
    /// `None` = note-level error, `Some(name)` = field-pinned error.
    pub field: Option<String>,
    pub message: String,
}

/// Pending state for a single note within a [`SaveTransaction`].
#[derive(Debug, Clone)]
pub struct PendingNote {
    pub note_id: String,
    /// True if created via `create_child()` within this transaction.
    pub is_new: bool,
    /// Parent ID (only meaningful when `is_new` is true).
    pub parent_id: Option<String>,
    pub schema: String,
    pub original_fields: BTreeMap<String, FieldValue>,
    pub pending_fields: BTreeMap<String, FieldValue>,
    pub original_title: String,
    pub pending_title: Option<String>,
    pub pending_checked: Option<bool>,
}

impl PendingNote {
    /// Returns the current effective title (pending or original).
    pub fn effective_title(&self) -> &str {
        self.pending_title.as_deref().unwrap_or(&self.original_title)
    }

    pub fn effective_checked(&self) -> Option<bool> {
        self.pending_checked
    }

    /// Returns the current effective fields (original merged with pending).
    pub fn effective_fields(&self) -> BTreeMap<String, FieldValue> {
        let mut fields = self.original_fields.clone();
        for (k, v) in &self.pending_fields {
            fields.insert(k.clone(), v.clone());
        }
        fields
    }
}

/// Collects pending writes and soft errors during a Rhai write-path hook.
///
/// Supports single-note (on_save) and multi-note (tree actions) transactions.
#[derive(Debug, Clone)]
pub struct SaveTransaction {
    pub pending_notes: BTreeMap<String, PendingNote>,
    pub soft_errors: Vec<SoftError>,
    pub committed: bool,
}

impl SaveTransaction {
    /// Creates an empty transaction.
    pub fn new() -> Self {
        Self {
            pending_notes: BTreeMap::new(),
            soft_errors: Vec::new(),
            committed: false,
        }
    }

    /// Creates a transaction pre-loaded with one existing note (for on_save).
    pub fn for_existing_note(
        note_id: String,
        schema: String,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) -> Self {
        let mut tx = Self::new();
        tx.pending_notes.insert(note_id.clone(), PendingNote {
            note_id,
            is_new: false,
            parent_id: None,
            schema,
            original_fields: fields,
            pending_fields: BTreeMap::new(),
            original_title: title,
            pending_title: None,
            pending_checked: None,
        });
        tx
    }

    /// Registers an existing note in this transaction (for multi-note hooks such as on_add_child).
    ///
    /// Unlike [`for_existing_note`](Self::for_existing_note), this method adds a note to an
    /// already-created transaction so that multiple existing notes can be pre-seeded at once.
    pub fn register_existing_note(
        &mut self,
        note_id: String,
        schema: String,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) {
        self.pending_notes.insert(note_id.clone(), PendingNote {
            note_id,
            is_new: false,
            parent_id: None,
            schema,
            original_fields: fields,
            pending_fields: BTreeMap::new(),
            original_title: title,
            pending_title: None,
            pending_checked: None,
        });
    }

    /// Registers a newly created child note in the transaction.
    pub fn add_new_note(
        &mut self,
        note_id: String,
        parent_id: String,
        schema: String,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) {
        self.pending_notes.insert(note_id.clone(), PendingNote {
            note_id,
            is_new: true,
            parent_id: Some(parent_id),
            schema,
            original_fields: fields.clone(),
            pending_fields: fields,
            original_title: title,
            pending_title: None,
            pending_checked: None,
        });
    }

    /// Queues a field write.
    ///
    /// # Errors
    ///
    /// Returns an error if `note_id` is not in this transaction.
    pub fn set_field(&mut self, note_id: &str, field: String, value: FieldValue) -> Result<(), String> {
        let pending = self.pending_notes.get_mut(note_id)
            .ok_or_else(|| format!("Note '{}' is not in this transaction", note_id))?;
        pending.pending_fields.insert(field, value);
        Ok(())
    }

    /// Queues a title write.
    ///
    /// # Errors
    ///
    /// Returns an error if `note_id` is not in this transaction.
    pub fn set_title(&mut self, note_id: &str, title: String) -> Result<(), String> {
        let pending = self.pending_notes.get_mut(note_id)
            .ok_or_else(|| format!("Note '{}' is not in this transaction", note_id))?;
        pending.pending_title = Some(title);
        Ok(())
    }

    /// Queues a checked-state write.
    ///
    /// # Errors
    ///
    /// Returns an error if `note_id` is not in this transaction.
    pub fn set_checked(&mut self, note_id: &str, checked: bool) -> Result<(), String> {
        let pending = self.pending_notes.get_mut(note_id)
            .ok_or_else(|| format!("Note '{}' is not in this transaction", note_id))?;
        pending.pending_checked = Some(checked);
        Ok(())
    }

    /// Accumulates a note-level soft error.
    pub fn reject_note(&mut self, message: String) {
        self.soft_errors.push(SoftError { field: None, message });
    }

    /// Accumulates a field-pinned soft error.
    pub fn reject_field(&mut self, field: String, message: String) {
        self.soft_errors.push(SoftError { field: Some(field), message });
    }

    /// Returns true if any soft errors have been accumulated.
    pub fn has_errors(&self) -> bool {
        !self.soft_errors.is_empty()
    }

    /// Marks the transaction as committed (caller must still apply to DB).
    ///
    /// # Errors
    ///
    /// Returns an error if soft errors exist (commit blocked).
    pub fn commit(&mut self) -> Result<(), Vec<SoftError>> {
        if self.has_errors() {
            Err(self.soft_errors.clone())
        } else {
            self.committed = true;
            Ok(())
        }
    }
}

impl Default for SaveTransaction {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of the save pipeline returned to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveResult {
    /// Save succeeded — returns the updated note.
    Ok(crate::core::note::Note),
    /// Validation or reject errors blocked the save.
    ValidationErrors {
        /// Field-pinned errors: field_name -> error message.
        #[serde(rename = "fieldErrors")]
        field_errors: BTreeMap<String, String>,
        /// Note-level errors from reject().
        #[serde(rename = "noteErrors")]
        note_errors: Vec<String>,
        /// Preview title from set_title() (if any).
        #[serde(rename = "previewTitle")]
        preview_title: Option<String>,
        /// Preview fields from set_field() calls.
        #[serde(rename = "previewFields")]
        preview_fields: BTreeMap<String, FieldValue>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_transaction_is_empty() {
        let tx = SaveTransaction::new();
        assert!(tx.pending_notes.is_empty());
        assert!(tx.soft_errors.is_empty());
        assert!(!tx.committed);
    }

    #[test]
    fn test_for_existing_note_populates_pending() {
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), FieldValue::Text("hello".to_string()));
        let tx = SaveTransaction::for_existing_note(
            "n1".to_string(), "TextNote".to_string(), "Title".to_string(), fields,
        );
        assert_eq!(tx.pending_notes.len(), 1);
        let pn = tx.pending_notes.get("n1").unwrap();
        assert!(!pn.is_new);
        assert_eq!(pn.effective_title(), "Title");
    }

    #[test]
    fn test_set_field_updates_pending() {
        let tx_fields = BTreeMap::new();
        let mut tx = SaveTransaction::for_existing_note(
            "n1".to_string(), "T".to_string(), "T".to_string(), tx_fields,
        );
        tx.set_field("n1", "x".to_string(), FieldValue::Number(42.0)).unwrap();
        let eff = tx.pending_notes.get("n1").unwrap().effective_fields();
        assert_eq!(eff.get("x"), Some(&FieldValue::Number(42.0)));
    }

    #[test]
    fn test_set_title_updates_pending() {
        let mut tx = SaveTransaction::for_existing_note(
            "n1".to_string(), "T".to_string(), "Old".to_string(), BTreeMap::new(),
        );
        tx.set_title("n1", "New".to_string()).unwrap();
        assert_eq!(tx.pending_notes.get("n1").unwrap().effective_title(), "New");
    }

    #[test]
    fn test_set_field_unknown_note_errors() {
        let mut tx = SaveTransaction::new();
        let result = tx.set_field("missing", "x".to_string(), FieldValue::Number(1.0));
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_accumulates_errors() {
        let mut tx = SaveTransaction::new();
        tx.reject_note("bad".to_string());
        tx.reject_field("f".to_string(), "invalid".to_string());
        assert_eq!(tx.soft_errors.len(), 2);
        assert!(tx.has_errors());
    }

    #[test]
    fn test_commit_blocked_by_errors() {
        let mut tx = SaveTransaction::new();
        tx.reject_note("nope".to_string());
        let result = tx.commit();
        assert!(result.is_err());
        assert!(!tx.committed);
    }

    #[test]
    fn test_commit_succeeds_when_clean() {
        let mut tx = SaveTransaction::new();
        let result = tx.commit();
        assert!(result.is_ok());
        assert!(tx.committed);
    }

    #[test]
    fn test_add_new_note() {
        let mut tx = SaveTransaction::new();
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), FieldValue::Text(String::new()));
        tx.add_new_note("c1".to_string(), "p1".to_string(), "TextNote".to_string(), "".to_string(), fields);
        assert_eq!(tx.pending_notes.len(), 1);
        let pn = tx.pending_notes.get("c1").unwrap();
        assert!(pn.is_new);
        assert_eq!(pn.parent_id.as_deref(), Some("p1"));
    }

    #[test]
    fn test_set_checked_updates_pending() {
        let mut tx = SaveTransaction::for_existing_note(
            "n1".to_string(), "T".to_string(), "T".to_string(), BTreeMap::new(),
        );
        assert_eq!(tx.pending_notes.get("n1").unwrap().effective_checked(), None);
        tx.set_checked("n1", true).unwrap();
        assert_eq!(tx.pending_notes.get("n1").unwrap().effective_checked(), Some(true));
    }

    #[test]
    fn test_effective_fields_merges_original_and_pending() {
        let mut orig = BTreeMap::new();
        orig.insert("a".to_string(), FieldValue::Text("original".to_string()));
        orig.insert("b".to_string(), FieldValue::Text("keep".to_string()));
        let mut tx = SaveTransaction::for_existing_note(
            "n1".to_string(), "T".to_string(), "T".to_string(), orig,
        );
        tx.set_field("n1", "a".to_string(), FieldValue::Text("updated".to_string())).unwrap();
        let eff = tx.pending_notes.get("n1").unwrap().effective_fields();
        assert_eq!(eff.get("a"), Some(&FieldValue::Text("updated".to_string())));
        assert_eq!(eff.get("b"), Some(&FieldValue::Text("keep".to_string())));
    }
}
