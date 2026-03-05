// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Undo/redo types for the Krillnotes workspace.
//!
//! `RetractInverse` captures the "before-state" needed to reverse any
//! workspace mutation. It is carried by `Operation::RetractOperation` so
//! that retract entries can be synced to peers via `.swarm` diffs.

use crate::{AttachmentMeta, FieldValue, Note};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The inverse data needed to reverse one or more workspace mutations.
///
/// Applied by [`crate::Workspace::undo`] to restore previous state.
/// Serialised into `Operation::RetractOperation.inverse` for the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RetractInverse {
    /// Inverse of `CreateNote` — delete the created note.
    DeleteNote { note_id: String },

    /// Inverse of `DeleteNote` (recursive) — restore the full subtree.
    ///
    /// `notes` are ordered parent-first (root of deleted subtree is index 0).
    /// Attachment `.enc` files remain on disk after a note-level delete, so
    /// only the `attachments` DB rows need to be re-inserted.
    SubtreeRestore {
        notes: Vec<Note>,
        attachments: Vec<AttachmentMeta>,
    },

    /// Inverse of `UpdateNote` — restore full note state atomically.
    NoteRestore {
        note_id: String,
        old_title: String,
        old_fields: BTreeMap<String, FieldValue>,
        old_tags: Vec<String>,
    },

    /// Inverse of `MoveNote` — return note to its previous position.
    PositionRestore {
        note_id: String,
        old_parent_id: Option<String>,
        old_position: f64,
    },

    /// Inverse of `CreateUserScript` — delete the created script.
    DeleteScript { script_id: String },

    /// Inverse of `UpdateUserScript` or `DeleteUserScript` — restore script.
    ScriptRestore {
        script_id: String,
        name: String,
        description: String,
        source_code: String,
        load_order: i32,
        enabled: bool,
    },

    /// Inverse of `DeleteAttachment` — restores a soft-deleted attachment.
    ///
    /// The `.enc.trash` file is renamed back to `.enc` and the DB row is re-inserted.
    AttachmentRestore {
        meta: AttachmentMeta,
    },

    /// Inverse of `AttachmentRestore` (i.e. redo of an undone DeleteAttachment).
    ///
    /// Re-soft-deletes the attachment: renames `.enc` → `.enc.trash` and removes the DB row.
    AttachmentSoftDelete {
        attachment_id: String,
    },

    /// Inverse of a compound action (tree hook, batch import).
    /// Items are applied in **reverse** order (LIFO — children before parent).
    Batch(Vec<RetractInverse>),
}

/// Returned by [`crate::Workspace::undo`] and [`crate::Workspace::redo`].
///
/// The frontend uses `affected_note_id` to re-select the relevant note
/// after applying the inverse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    /// Note to select/highlight after undo/redo. `None` for script operations.
    pub affected_note_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn test_retract_inverse_batch_serializes() {
        let batch = RetractInverse::Batch(vec![
            RetractInverse::DeleteNote { note_id: "n1".into() },
            RetractInverse::DeleteNote { note_id: "n2".into() },
        ]);
        let json = serde_json::to_string(&batch).unwrap();
        let back: RetractInverse = serde_json::from_str(&json).unwrap();
        match back {
            RetractInverse::Batch(items) => assert_eq!(items.len(), 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_undo_result_no_note() {
        let r = UndoResult { affected_note_id: None };
        assert!(r.affected_note_id.is_none());
    }
}
