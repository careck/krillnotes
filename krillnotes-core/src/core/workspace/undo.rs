// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Undo/redo stack management for workspace mutations.

use super::*;

impl Workspace {
    /// Returns `true` if there is at least one action to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns `true` if there is at least one action to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Returns `true` if there is at least one script action to undo.
    pub fn can_script_undo(&self) -> bool {
        !self.script_undo_stack.is_empty()
    }

    /// Returns `true` if there is at least one script action to redo.
    pub fn can_script_redo(&self) -> bool {
        !self.script_redo_stack.is_empty()
    }

    /// Undoes the most recent script mutation (create/update/delete script).
    ///
    /// Script undo is separate from note undo to prevent script saves from
    /// interleaving with note edits in the workspace undo stack.
    pub fn script_undo(&mut self) -> Result<UndoResult> {
        let entry = self.script_undo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to undo".into()))?;

        let redo_inverse = self.build_redo_inverse(&entry)?;

        // Authorize before mutating.
        let retract_op = Operation::RetractOperation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: self.hlc.now(),
            device_id: self.device_id.clone(),
            retracted_ids: entry.retracted_ids.clone(),
            inverse: entry.inverse.clone(),
            propagate: entry.propagate,
        };
        self.authorize(&retract_op)?;

        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        apply_result?;

        self.script_redo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: redo_inverse,
            propagate: entry.propagate,
        });
        Ok(UndoResult { affected_note_id: None })
    }

    /// Re-applies the most recently undone script mutation.
    pub fn script_redo(&mut self) -> Result<UndoResult> {
        let entry = self.script_redo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to redo".into()))?;

        let new_undo_inverse = self.build_redo_inverse(&entry)?;

        // Authorize before mutating.
        let retract_op = Operation::RetractOperation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: self.hlc.now(),
            device_id: self.device_id.clone(),
            retracted_ids: entry.retracted_ids.clone(),
            inverse: entry.inverse.clone(),
            propagate: entry.propagate,
        };
        self.authorize(&retract_op)?;

        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        apply_result?;

        self.script_undo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: new_undo_inverse,
            propagate: entry.propagate,
        });
        // Trim to undo_limit.
        if self.script_undo_stack.len() > self.undo_limit {
            self.script_undo_stack.drain(0..1);
        }
        Ok(UndoResult { affected_note_id: None })
    }

    /// Returns the current undo stack depth limit.
    pub fn get_undo_limit(&self) -> usize {
        self.undo_limit
    }

    /// Sets the undo stack depth limit, persisting it to `workspace_meta`.
    ///
    /// The value is clamped to `[1, 500]`. If the new limit is smaller than
    /// the current stack depth, the oldest entries are dropped.
    pub fn set_undo_limit(&mut self, limit: usize) -> Result<()> {
        let limit = limit.max(1).min(500);
        self.storage.connection().execute(
            "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('undo_limit', ?)",
            [limit.to_string()],
        )?;
        self.undo_limit = limit;
        if self.undo_stack.len() > limit {
            let excess = self.undo_stack.len() - limit;
            self.undo_stack.drain(0..excess);
        }
        Ok(())
    }

    /// Pushes an entry onto the undo stack (or into the group buffer if a group
    /// is open). Clears the redo stack. Trims to `undo_limit`.
    ///
    /// When `inside_undo` is `true` (i.e. we are executing `apply_retract_inverse_internal`
    /// on behalf of an undo or redo call), this is a no-op so that mutations
    /// invoked internally (e.g. `move_note` called from `PositionRestore`) do not
    /// push spurious entries onto the stack.
    pub(crate) fn push_undo(&mut self, entry: UndoEntry) {
        if self.inside_undo {
            return;
        }
        if let Some(buf) = &mut self.undo_group_buffer {
            buf.push(entry);
            return;
        }
        self.redo_stack.clear();
        self.undo_stack.push(entry);
        if self.undo_stack.len() > self.undo_limit {
            self.undo_stack.drain(0..1);
        }
    }

    /// Pushes an entry onto the script undo stack. Clears the script redo stack.
    /// No-op while `inside_undo` is `true`.
    pub(crate) fn push_script_undo(&mut self, entry: UndoEntry) {
        if self.inside_undo {
            return;
        }
        self.script_redo_stack.clear();
        self.script_undo_stack.push(entry);
        if self.script_undo_stack.len() > self.undo_limit {
            self.script_undo_stack.drain(0..1);
        }
    }

    /// Opens an undo group. Subsequent mutations accumulate in a staging buffer
    /// until `end_undo_group` is called, at which point they are collapsed into
    /// a single `UndoEntry` with a `RetractInverse::Batch` inverse.
    ///
    /// Nested calls are ignored — the outermost begin/end pair wins.
    pub fn begin_undo_group(&mut self) {
        if self.undo_group_buffer.is_none() {
            self.undo_group_buffer = Some(Vec::new());
        }
    }

    /// Closes the undo group and pushes a single batched `UndoEntry`.
    /// If the buffer is empty or no group is open, this is a no-op.
    pub fn end_undo_group(&mut self) {
        let Some(mut buf) = self.undo_group_buffer.take() else { return };
        if buf.is_empty() { return; }

        let retracted_ids: Vec<String> = buf.iter()
            .flat_map(|e| e.retracted_ids.iter().cloned())
            .collect();
        let propagate = buf.iter().any(|e| e.propagate);
        // Build Batch in original order; undo will apply LIFO.
        let inverses: Vec<RetractInverse> = buf.drain(..).map(|e| e.inverse).collect();

        self.redo_stack.clear();
        self.undo_stack.push(UndoEntry {
            retracted_ids,
            inverse: RetractInverse::Batch(inverses),
            propagate,
        });
        if self.undo_stack.len() > self.undo_limit {
            self.undo_stack.drain(0..1);
        }
    }

    /// Undoes the most recent operation on the undo stack.
    ///
    /// Returns an [`UndoResult`] indicating which note (if any) should be
    /// re-selected in the UI.
    ///
    /// # Errors
    ///
    /// Returns an error if the undo stack is empty or if applying the inverse
    /// operation fails.
    pub fn undo(&mut self) -> Result<UndoResult> {
        let entry = self.undo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to undo".into()))?;

        // Build the redo inverse BEFORE applying the undo so that the current
        // DB state can be captured. For example, for DeleteNote (which is the
        // inverse of CreateNote), we need to snapshot the note's data into a
        // SubtreeRestore while the note still exists in the DB.
        let redo_inverse = self.build_redo_inverse(&entry)?;

        // Construct and authorize the RetractOperation before any DB mutation.
        let retract_ts = self.advance_hlc();
        let retract_op_id = uuid::Uuid::new_v4().to_string();
        let retract_op = Operation::RetractOperation {
            operation_id: retract_op_id,
            timestamp: retract_ts,
            device_id: self.device_id.clone(),
            retracted_ids: entry.retracted_ids.clone(),
            inverse: entry.inverse.clone(),
            propagate: entry.propagate,
        };
        self.authorize(&retract_op)?;

        // Apply the inverse to the DB. Set inside_undo so that any mutations
        // called from within apply_retract_inverse_internal (e.g. move_note
        // called from PositionRestore) do not push spurious undo entries.
        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        let affected_note_id = apply_result?;

        // Log the RetractOperation.
        {
            let tx = self.storage.connection_mut().transaction()?;
            Self::save_hlc(&retract_ts, &tx)?;
            Self::log_op(&self.operation_log, &tx, &retract_op)?;
            Self::purge_ops_if_needed(&self.operation_log, &tx)?;
            tx.commit()?;
        }

        // Push onto redo stack using the pre-captured redo inverse so that
        // redo() can re-apply the forward operation (e.g. re-insert the note).
        self.redo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: redo_inverse,
            propagate: entry.propagate,
        });

        Ok(UndoResult { affected_note_id })
    }

    /// Re-applies the most recently undone operation from the redo stack.
    ///
    /// Returns an [`UndoResult`] indicating which note (if any) should be
    /// re-selected in the UI.
    ///
    /// # Errors
    ///
    /// Returns an error if the redo stack is empty or if re-applying the
    /// operation fails.
    pub fn redo(&mut self) -> Result<UndoResult> {
        let entry = self.redo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to redo".into()))?;

        // Build the new undo inverse BEFORE applying so that the current DB state
        // can be captured for the "undo of redo" entry.
        let new_undo_inverse = self.build_redo_inverse(&entry)?;

        // Construct and authorize the RetractOperation before any DB mutation.
        let redo_ts = self.advance_hlc();
        let new_op_id = uuid::Uuid::new_v4().to_string();
        let redo_op = Operation::RetractOperation {
            operation_id: new_op_id,
            timestamp: redo_ts,
            device_id: self.device_id.clone(),
            retracted_ids: entry.retracted_ids.clone(),
            inverse: entry.inverse.clone(),
            propagate: entry.propagate,
        };
        self.authorize(&redo_op)?;

        // Apply the redo entry's inverse to the DB.
        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        let affected_note_id = apply_result?;

        // Log redo as a new RetractOperation.
        {
            let tx = self.storage.connection_mut().transaction()?;
            Self::save_hlc(&redo_ts, &tx)?;
            Self::log_op(&self.operation_log, &tx, &redo_op)?;
            Self::purge_ops_if_needed(&self.operation_log, &tx)?;
            tx.commit()?;
        }

        // Push a new undo entry carrying the new_undo_inverse so the redo can
        // itself be undone.
        self.undo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: new_undo_inverse,
            propagate: entry.propagate,
        });

        Ok(UndoResult { affected_note_id })
    }

    /// Builds the inverse needed to reverse a redo operation — i.e. captures
    /// the current DB state so that the redo can itself be undone.
    ///
    /// For each `RetractInverse` variant this determines what "un-doing the redo"
    /// would require:
    ///
    /// - `DeleteNote`     (undo was: un-do a CreateNote) → redo re-deletes.
    ///                    Build `SubtreeRestore` from current state.
    /// - `SubtreeRestore` (undo was: un-do a DeleteNote) → redo re-deletes root.
    ///                    Build `DeleteNote`.
    /// - `NoteRestore`    → redo re-updates. Capture current state as `NoteRestore`.
    /// - `PositionRestore`→ redo re-moves. Capture current position as `PositionRestore`.
    /// - `DeleteScript`   → redo re-deletes. Script no longer exists post-undo;
    ///                    use a stub `ScriptRestore` (deletion needs no data).
    /// - `ScriptRestore`  → redo re-restores. Build `DeleteScript`.
    /// - `Batch`          → recurse in reverse LIFO order.
    fn build_redo_inverse(&self, undo_entry: &UndoEntry) -> Result<RetractInverse> {
        match &undo_entry.inverse {
            RetractInverse::DeleteNote { note_id } => {
                // Undo was DeleteNote (undoing CreateNote). Redo = re-delete.
                // Current state: note exists. Capture subtree for redo's undo.
                let notes = self.collect_subtree_notes(note_id)?;
                let attachments = self.get_attachments(note_id).unwrap_or_default();
                Ok(RetractInverse::SubtreeRestore { notes, attachments })
            }
            RetractInverse::SubtreeRestore { notes, .. } => {
                // Undo was SubtreeRestore (undoing DeleteNote). Redo = re-delete root.
                let root_id = notes.first().map(|n| n.id.clone())
                    .ok_or_else(|| KrillnotesError::ValidationFailed("empty subtree in redo inverse".into()))?;
                Ok(RetractInverse::DeleteNote { note_id: root_id })
            }
            RetractInverse::NoteRestore { note_id, .. } => {
                let current = self.get_note(note_id)?;
                Ok(RetractInverse::NoteRestore {
                    note_id: note_id.clone(),
                    old_title: current.title,
                    old_fields: current.fields,
                    old_tags: current.tags,
                    old_is_checked: current.is_checked,
                })
            }
            RetractInverse::PositionRestore { note_id, .. } => {
                let current = self.get_note(note_id)?;
                Ok(RetractInverse::PositionRestore {
                    note_id: note_id.clone(),
                    old_parent_id: current.parent_id,
                    old_position: current.position,
                })
            }
            RetractInverse::DeleteScript { script_id } => {
                // Undo of CreateScript: redo should re-delete. Capture the
                // script's current state so that a subsequent undo-of-redo can
                // restore it fully (rather than using an empty placeholder).
                if let Ok(current) = self.get_user_script(script_id) {
                    Ok(RetractInverse::ScriptRestore {
                        script_id: script_id.clone(),
                        name: current.name,
                        description: current.description,
                        source_code: current.source_code,
                        load_order: current.load_order,
                        enabled: current.enabled,
                    })
                } else {
                    // Script already absent — redo entry is a no-op placeholder.
                    Ok(RetractInverse::ScriptRestore {
                        script_id: script_id.clone(),
                        name: String::new(),
                        description: String::new(),
                        source_code: String::new(),
                        load_order: 0,
                        enabled: false,
                    })
                }
            }
            RetractInverse::ScriptRestore { script_id, .. } => {
                // If the script exists now (undo of UpdateUserScript), redo must
                // restore it to its current (pre-undo) state, not delete it.
                // If it doesn't exist (undo of DeleteUserScript — script absent),
                // redo should delete it again.
                if let Ok(current) = self.get_user_script(script_id) {
                    Ok(RetractInverse::ScriptRestore {
                        script_id: script_id.clone(),
                        name: current.name,
                        description: current.description,
                        source_code: current.source_code,
                        load_order: current.load_order,
                        enabled: current.enabled,
                    })
                } else {
                    Ok(RetractInverse::DeleteScript { script_id: script_id.clone() })
                }
            }
            RetractInverse::AttachmentRestore { meta } => {
                // Undo was AttachmentRestore (undoing a DeleteAttachment).
                // build_redo_inverse is called BEFORE undo is applied, so the .enc.trash
                // file exists and the DB row is absent. Redo should soft-delete again.
                Ok(RetractInverse::AttachmentSoftDelete { attachment_id: meta.id.clone() })
            }
            RetractInverse::AttachmentSoftDelete { attachment_id } => {
                // Undo was AttachmentSoftDelete (redoing a DeleteAttachment).
                // build_redo_inverse is called BEFORE undo is applied, so the .enc file
                // exists and the DB row is present. Redo should restore to prior state.
                // Capture current meta from DB to populate the restore entry.
                let meta = self.get_attachment_meta(attachment_id)?;
                Ok(RetractInverse::AttachmentRestore { meta })
            }
            RetractInverse::Batch(items) => {
                // Build redo inverses in reverse order (LIFO mirror).
                let mut redo_items = Vec::with_capacity(items.len());
                for item in items.iter().rev() {
                    let entry = UndoEntry {
                        retracted_ids: vec![],
                        inverse: item.clone(),
                        propagate: undo_entry.propagate,
                    };
                    redo_items.push(self.build_redo_inverse(&entry)?);
                }
                Ok(RetractInverse::Batch(redo_items))
            }
        }
    }

    /// Applies `inverse` to the database without touching undo/redo stacks.
    ///
    /// Returns the note ID most relevant for UI re-selection, if any.
    pub(crate) fn apply_retract_inverse_internal(
        &mut self,
        inverse: &RetractInverse,
    ) -> Result<Option<String>> {
        match inverse {
            RetractInverse::DeleteNote { note_id } => {
                // Undo of CreateNote: delete the note (no children expected).
                let all_ids = self.collect_subtree_ids(note_id)?;
                for id in &all_ids {
                    self.clear_links_to(id)?;
                }
                let tx = self.storage.connection_mut().transaction()?;
                Self::delete_recursive_in_tx(&tx, note_id)?;
                tx.commit()?;
                Ok(None)
            }

            RetractInverse::SubtreeRestore { notes, attachments } => {
                // Undo of DeleteNote: re-insert notes (parent-first) and attachment rows.
                let root_id = notes.first().map(|n| n.id.clone());
                let conn = self.storage.connection_mut();
                let tx = conn.transaction()?;
                for note in notes {
                    let fields_json = serde_json::to_string(&note.fields)
                        .map_err(KrillnotesError::Json)?;
                    tx.execute(
                        "INSERT OR IGNORE INTO notes
                         (id, title, schema, parent_id, position,
                          created_at, modified_at, created_by, modified_by,
                          fields_json, is_expanded, schema_version)
                         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                        rusqlite::params![
                            note.id, note.title, note.schema, note.parent_id,
                            note.position, note.created_at, note.modified_at,
                            note.created_by, note.modified_by, fields_json,
                            note.is_expanded as i32, note.schema_version,
                        ],
                    )?;
                    for tag in &note.tags {
                        tx.execute(
                            "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?,?)",
                            rusqlite::params![note.id, tag],
                        )?;
                    }
                }
                for att in attachments {
                    // salt is hex-encoded in AttachmentMeta; DB stores raw bytes.
                    let salt_bytes = hex::decode(&att.salt)
                        .unwrap_or_else(|_| att.salt.as_bytes().to_vec());
                    tx.execute(
                        "INSERT OR IGNORE INTO attachments
                         (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
                         VALUES (?,?,?,?,?,?,?,?)",
                        rusqlite::params![
                            att.id, att.note_id, att.filename, att.mime_type,
                            att.size_bytes as i64, att.hash_sha256,
                            salt_bytes.as_slice(), att.created_at,
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(root_id)
            }

            RetractInverse::NoteRestore { note_id, old_title, old_fields, old_tags, old_is_checked } => {
                // Restore title + fields + tags + is_checked atomically.
                let fields_json = serde_json::to_string(old_fields)
                    .map_err(KrillnotesError::Json)?;
                let now = chrono::Utc::now().timestamp();
                let conn = self.storage.connection_mut();
                let tx = conn.transaction()?;
                tx.execute(
                    "UPDATE notes SET title=?, fields_json=?, modified_at=?, is_checked=? WHERE id=?",
                    rusqlite::params![old_title, fields_json, now, old_is_checked, note_id],
                )?;
                tx.execute("DELETE FROM note_tags WHERE note_id=?", [note_id])?;
                for tag in old_tags {
                    tx.execute(
                        "INSERT INTO note_tags (note_id, tag) VALUES (?,?)",
                        rusqlite::params![note_id, tag],
                    )?;
                }
                tx.commit()?;
                Ok(Some(note_id.clone()))
            }

            RetractInverse::PositionRestore { note_id, old_parent_id, old_position } => {
                self.move_note(note_id, old_parent_id.as_deref(), *old_position)?;
                Ok(Some(note_id.clone()))
            }

            RetractInverse::DeleteScript { script_id } => {
                self.storage.connection().execute(
                    "DELETE FROM user_scripts WHERE id=?",
                    [script_id],
                )?;
                self.reload_scripts()?;
                Ok(None)
            }

            RetractInverse::ScriptRestore {
                script_id, name, description,
                source_code, load_order, enabled,
            } => {
                let now = chrono::Utc::now().timestamp();
                self.storage.connection().execute(
                    "INSERT OR REPLACE INTO user_scripts
                     (id, name, description, source_code, load_order, enabled,
                      created_at, modified_at, category)
                     VALUES (?,?,?,?,?,?,?,?,?)",
                    rusqlite::params![
                        script_id, name, description, source_code,
                        load_order, enabled, now, now, "library",
                    ],
                )?;
                self.reload_scripts()?;
                Ok(None)
            }

            RetractInverse::AttachmentRestore { meta } => {
                let note_id = meta.note_id.clone();
                self.restore_attachment(meta)?;
                Ok(Some(note_id))
            }

            RetractInverse::AttachmentSoftDelete { attachment_id } => {
                // Redo of DeleteAttachment: rename .enc → .enc.trash, delete DB row.
                let note_id: Option<String> = self.storage.connection()
                    .query_row(
                        "SELECT note_id FROM attachments WHERE id = ?",
                        [attachment_id],
                        |row| row.get(0),
                    )
                    .ok();
                let enc_path = self.workspace_root.join("attachments")
                    .join(format!("{attachment_id}.enc"));
                let trash_path = self.workspace_root.join("attachments")
                    .join(format!("{attachment_id}.enc.trash"));
                if enc_path.exists() {
                    std::fs::rename(&enc_path, &trash_path)?;
                }
                self.storage.connection().execute(
                    "DELETE FROM attachments WHERE id = ?",
                    [attachment_id],
                )?;
                Ok(note_id)
            }

            RetractInverse::Batch(items) => {
                // Apply in reverse order (LIFO).
                let mut last_note = None;
                for item in items.iter().rev() {
                    if let Some(id) = self.apply_retract_inverse_internal(item)? {
                        last_note = Some(id);
                    }
                }
                Ok(last_note)
            }
        }
    }
}
