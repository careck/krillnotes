// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Note CRUD, tree operations, search, links, metadata, and expansion.

use super::*;

impl Workspace {
    /// Fetches a single note by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// if `fields_json` cannot be deserialised.
    pub fn get_note(&self, note_id: &str) -> Result<Note> {
        self.check_read_access(note_id)?;
        let row = self.connection().query_row(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             WHERE n.id = ?
             GROUP BY n.id",
            [note_id],
            map_note_row,
        )?;
        note_from_row_tuple(row)
    }

    /// Creates a new note of `note_type` relative to `selected_note_id`.
    ///
    /// The new note is inserted as a child or sibling according to `position`.
    /// Sibling insertion bumps the positions of all following siblings to make room.
    ///
    /// Returns the ID of the newly created note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::SchemaNotFound`] if `note_type` is unknown,
    /// or [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn create_note(
        &mut self,
        selected_note_id: &str,
        position: AddPosition,
        note_type: &str,
    ) -> Result<String> {
        let schema = self.script_registry.get_schema(note_type)?;
        let selected = self.get_note(selected_note_id)?;

        // Determine final parent and position
        let (final_parent, final_position) = match position {
            AddPosition::AsChild => (Some(selected.id.clone()), 0.0_f64),
            AddPosition::AsSibling => (selected.parent_id.clone(), selected.position + 1.0),
        };

        // Validate allowed_parent_schemas
        if !schema.allowed_parent_schemas.is_empty() {
            match &final_parent {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", note_type
                ))),
                Some(pid) => {
                    let parent_note = self.get_note(pid)?;
                    if !schema.allowed_parent_schemas.contains(&parent_note.schema) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            note_type, parent_note.schema
                        )));
                    }
                }
            }
        }

        // Validate allowed_children_schemas on the parent schema
        if let Some(pid) = &final_parent {
            let parent_note = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent_note.schema)?;
            if parent_schema.is_leaf {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Cannot add children to a leaf note (schema: '{}')",
                    parent_note.schema
                )));
            }
            if !parent_schema.allowed_children_schemas.is_empty()
                && !parent_schema.allowed_children_schemas.contains(&note_type.to_string())
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    note_type, parent_note.schema
                )));
            }
        }

        // Fetch parent note before opening the transaction (avoids borrow conflict with `tx`).
        let hook_parent = if let Some(ref pid) = final_parent {
            Some(self.get_note(pid)?)
        } else {
            None
        };

        let now = chrono::Utc::now().timestamp();
        let mut note = Note {
            id: Uuid::new_v4().to_string(),
            title: "Untitled".to_string(),
            schema: note_type.to_string(),
            parent_id: final_parent,
            position: final_position,
            created_at: now,
            modified_at: now,
            created_by: self.current_identity_pubkey.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            fields: schema.default_fields(),
            is_expanded: true,
            tags: vec![],
            schema_version: schema.version,
        };

        // Authorize before opening the transaction.
        let auth_op = Operation::CreateNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note.id.clone(),
            parent_id: note.parent_id.clone(),
            position: note.position,
            schema: note.schema.clone(),
            title: note.title.clone(),
            fields: note.fields.clone(),
            created_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        // Advance HLC and capture signing key before the transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        // For sibling insertion, bump positions of all following siblings to make room
        if let AddPosition::AsSibling = position {
            tx.execute(
                "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ?",
                rusqlite::params![note.parent_id, note.position],
            )?;
        }

        // Insert note
        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                note.id,
                note.title,
                note.schema,
                note.parent_id,
                note.position,
                note.created_at,
                note.modified_at,
                note.created_by,
                note.modified_by,
                serde_json::to_string(&note.fields)?,
                true,
                note.schema_version,
            ],
        )?;

        // Run on_add_child hook if the parent's schema defines one.
        // Allowed-parent and allowed-children checks have already passed above.
        if let Some(ref parent_note) = hook_parent {
            if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
                &parent_note.schema,
                &parent_note.id, &parent_note.schema, &parent_note.title, &parent_note.fields,
                &note.id, &note.schema, &note.title, &note.fields,
            )? {
                let now = chrono::Utc::now().timestamp();
                if let Some((new_title, new_fields)) = hook_result.child {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, now, note.id],
                    )?;
                    // Keep note in sync with what was persisted so the operation log
                    // records the final stored values, not the pre-hook defaults.
                    note.title  = new_title;
                    note.fields = new_fields;
                }
                if let Some((new_title, new_fields)) = hook_result.parent {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, now, parent_note.id],
                    )?;
                }
            }
        }

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note.id.clone(),
            parent_id: note.parent_id.clone(),
            position: note.position,
            schema: note.schema.clone(),
            title: note.title.clone(),
            fields: note.fields.clone(),
            created_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // Keep the note_links junction table in sync (no-op for default fields, correct for future use).
        // Must run inside the transaction so the link update is atomic with the note write.
        sync_note_links(&tx, &note.id, &note.fields)?;

        tx.commit()?;

        // Push undo entry — inverse of CreateNote is DeleteNote.
        let op_id = op.operation_id().to_string();
        let note_id = note.id.clone();
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::DeleteNote { note_id },
            propagate: true,
        });

        Ok(note.id)
    }

    /// Deep-copies the note at `source_id` and its entire descendant subtree,
    /// placing the copy at `target_id` with the given `position`.
    ///
    /// Returns the ID of the new root note.
    ///
    /// All notes in the subtree receive fresh UUIDs and current timestamps.
    /// Schema constraints (`allowed_parent_schemas`, `allowed_children_schemas`) are
    /// validated only for the root of the copy against the paste target.
    /// Children's internal parent/child relationships are trusted and not re-validated.
    pub fn deep_copy_note(
        &mut self,
        source_id: &str,
        target_id: &str,
        position: AddPosition,
    ) -> Result<String> {
        // 1. Load the full subtree rooted at source_id using an iterative BFS.
        let mut subtree: Vec<Note> = Vec::new();
        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        queue.push_back(source_id.to_string());
        while let Some(current_id) = queue.pop_front() {
            let note = self.get_note(&current_id)?;
            // Enqueue children
            let child_ids: Vec<String> = self
                .connection()
                .prepare("SELECT id FROM notes WHERE parent_id = ? ORDER BY position")?
                .query_map([&current_id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for cid in child_ids {
                queue.push_back(cid);
            }
            subtree.push(note);
        }

        if subtree.is_empty() {
            return Err(KrillnotesError::NoteNotFound(source_id.to_string()));
        }

        // 2. Validate the paste location for the root note only.
        let root_source = subtree[0].clone();
        let root_schema = self.script_registry.get_schema(&root_source.schema)?;
        let target_note = self.get_note(target_id)?;

        let (new_parent_id, new_position) = match position {
            AddPosition::AsChild => (Some(target_note.id.clone()), 0.0_f64),
            AddPosition::AsSibling => (target_note.parent_id.clone(), target_note.position + 1.0),
        };

        // Validate allowed_parent_schemas for the root copy
        if !root_schema.allowed_parent_schemas.is_empty() {
            match &new_parent_id {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", root_source.schema
                ))),
                Some(pid) => {
                    let parent = self.get_note(pid)?;
                    if !root_schema.allowed_parent_schemas.contains(&parent.schema) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            root_source.schema, parent.schema
                        )));
                    }
                }
            }
        }

        // Validate allowed_children_schemas on the paste parent
        if let Some(pid) = &new_parent_id {
            let parent = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent.schema)?;
            if parent_schema.is_leaf {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Cannot add children to a leaf note (schema: '{}')",
                    parent.schema
                )));
            }
            if !parent_schema.allowed_children_schemas.is_empty()
                && !parent_schema.allowed_children_schemas.contains(&root_source.schema)
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    root_source.schema, parent.schema
                )));
            }
        }

        // 3. Build old_id → new_id remap table.
        let mut id_map: HashMap<String, String> = HashMap::new();
        for note in &subtree {
            id_map.insert(note.id.clone(), Uuid::new_v4().to_string());
        }

        // Authorize the deep copy (as a CreateNote for the root of the copy).
        let auth_op = Operation::CreateNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: id_map[source_id].clone(),
            parent_id: new_parent_id.clone(),
            position: new_position,
            schema: root_source.schema.clone(),
            title: root_source.title.clone(),
            fields: root_source.fields.clone(),
            created_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        let now = chrono::Utc::now().timestamp();

        // Pre-advance HLC once per note in the subtree, and capture signing key,
        // before the transaction borrows self.storage mutably.
        let subtree_timestamps: Vec<HlcTimestamp> = subtree.iter()
            .map(|_| self.advance_hlc())
            .collect();
        let signing_key = self.signing_key.clone();

        // 4. Insert all cloned notes in a single transaction.
        let tx = self.storage.connection_mut().transaction()?;

        // If pasting as sibling, bump positions of following siblings to make room.
        if let AddPosition::AsSibling = position {
            tx.execute(
                "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ?",
                rusqlite::params![new_parent_id, new_position],
            )?;
        }

        let root_new_id = id_map[source_id].clone();

        for (note, ts) in subtree.iter().zip(subtree_timestamps.iter()) {
            let new_id = id_map[&note.id].clone();
            let new_parent = if note.id == source_id {
                // Root of the copy gets the paste target as parent
                new_parent_id.clone()
            } else {
                // Children remap their parent_id through the id_map
                note.parent_id.as_ref().and_then(|pid| id_map.get(pid).cloned())
            };
            let this_position = if note.id == source_id { new_position } else { note.position };

            tx.execute(
                "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    new_id,
                    note.title,
                    note.schema,
                    new_parent,
                    this_position,
                    now,
                    now,
                    self.current_identity_pubkey.clone(),
                    self.current_identity_pubkey.clone(),
                    serde_json::to_string(&note.fields)?,
                    note.is_expanded,
                    note.schema_version,
                ],
            )?;

            // Log a CreateNote operation for each inserted note.
            Self::save_hlc(ts, &tx)?;
            let mut op = Operation::CreateNote {
                operation_id: Uuid::new_v4().to_string(),
                timestamp: *ts,
                device_id: self.device_id.clone(),
                note_id: new_id.clone(),
                parent_id: new_parent,
                position: this_position as f64,
                schema: note.schema.clone(),
                title: note.title.clone(),
                fields: note.fields.clone(),
                created_by: String::new(),
                signature: String::new(),
            };
            Self::sign_op_with(&signing_key, &mut op);
            Self::log_op(&self.operation_log, &tx, &op)?;
        }

        Self::purge_ops_if_needed(&self.operation_log, &tx)?;
        tx.commit()?;

        self.push_undo(UndoEntry {
            retracted_ids: vec![],
            inverse: RetractInverse::DeleteNote { note_id: root_new_id.clone() },
            propagate: true,
        });

        Ok(root_new_id)
    }

    /// Creates a new root-level note of `node_type` with no parent.
    ///
    /// Returns the ID of the newly created note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::SchemaNotFound`] if `node_type` is unknown,
    /// or [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn create_note_root(&mut self, node_type: &str) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let schema = self.script_registry.get_schema(node_type)?;

        // Validate allowed_parent_schemas — root notes have no parent
        if !schema.allowed_parent_schemas.is_empty() {
            return Err(KrillnotesError::InvalidMove(format!(
                "Note type '{}' cannot be placed at root level", node_type
            )));
        }

        let new_note = Note {
            id: Uuid::new_v4().to_string(),
            title: "Untitled".to_string(),
            schema: node_type.to_string(),
            parent_id: None,
            position: 0.0,
            created_at: now,
            modified_at: now,
            created_by: self.current_identity_pubkey.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            fields: schema.default_fields(),
            is_expanded: true,
            tags: vec![], schema_version: 1,
        };

        // Authorize before opening the transaction.
        let auth_op = Operation::CreateNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: new_note.id.clone(),
            parent_id: new_note.parent_id.clone(),
            position: new_note.position,
            schema: new_note.schema.clone(),
            title: new_note.title.clone(),
            fields: new_note.fields.clone(),
            created_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                new_note.id,
                new_note.title,
                new_note.schema,
                new_note.parent_id,
                new_note.position,
                new_note.created_at,
                new_note.modified_at,
                new_note.created_by,
                new_note.modified_by,
                serde_json::to_string(&new_note.fields)?,
                true,
                new_note.schema_version,
            ],
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: new_note.id.clone(),
            parent_id: new_note.parent_id.clone(),
            position: new_note.position,
            schema: new_note.schema.clone(),
            title: new_note.title.clone(),
            fields: new_note.fields.clone(),
            created_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // Keep the note_links junction table in sync (no-op for default fields, correct for future use).
        // Must run inside the transaction so the link update is atomic with the note write.
        sync_note_links(&tx, &new_note.id, &new_note.fields)?;

        tx.commit()?;

        // Push undo entry — inverse of CreateNote is DeleteNote.
        let op_id = op.operation_id().to_string();
        let note_id = new_note.id.clone();
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::DeleteNote { note_id },
            propagate: true,
        });

        Ok(new_note.id)
    }

    /// Updates the title of `note_id` and logs an `UpdateNote` operation.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// the UPDATE fails.
    pub fn update_note_title(&mut self, note_id: &str, new_title: String) -> Result<()> {
        // Authorize before opening the transaction.
        let auth_op = Operation::UpdateNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            title: new_title.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        let now = chrono::Utc::now().timestamp();
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE notes SET title = ?, modified_at = ?, modified_by = ? WHERE id = ?",
            rusqlite::params![new_title, now, self.current_identity_pubkey.clone(), note_id],
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            title: new_title,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;
        Ok(())
    }

    /// Replaces all tags for `note_id` with the provided list.
    ///
    /// Tags are normalised (lowercased, trimmed, deduplicated) before storage.
    /// Deletes existing tags and re-inserts in a single transaction.
    pub fn update_note_tags(&mut self, note_id: &str, tags: Vec<String>) -> Result<()> {
        let mut normalised: Vec<String> = tags
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        normalised.sort();
        normalised.dedup();

        // Authorize before opening the transaction.
        let auth_op = Operation::SetTags {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            tags: normalised.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;
        tx.execute("DELETE FROM note_tags WHERE note_id = ?", [note_id])?;
        for tag in &normalised {
            tx.execute(
                "INSERT INTO note_tags (note_id, tag) VALUES (?, ?)",
                rusqlite::params![note_id, tag],
            )?;
        }

        // Log a SetTags operation so peers can replicate tag changes.
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::SetTags {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            tags: normalised,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;
        Ok(())
    }

    /// Returns all distinct tags used across the workspace, sorted alphabetically.
    pub fn get_all_tags(&self) -> Result<Vec<String>> {
        let mut stmt = self.connection().prepare(
            "SELECT DISTINCT tag FROM note_tags ORDER BY tag"
        )?;
        let tags = stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tags)
    }

    /// Returns all notes that have any of the provided tags (OR logic).
    ///
    /// Returns an empty vec if `tags` is empty.
    pub fn get_notes_for_tag(&self, tags: &[String]) -> Result<Vec<Note>> {
        if tags.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = tags.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt2.tag, ',') AS tags_csv
             FROM notes n
             JOIN note_tags nt ON nt.note_id = n.id AND nt.tag IN ({placeholders})
             LEFT JOIN note_tags nt2 ON nt2.note_id = n.id
             GROUP BY n.id
             ORDER BY n.parent_id, n.position"
        );
        let mut stmt = self.connection().prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = tags.iter()
            .map(|t| t as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let notes: Vec<Note> = rows.into_iter().map(note_from_row_tuple).collect::<Result<_>>()?;

        // Filter by read access
        if let Some(visible) = self.visible_note_ids()? {
            Ok(notes.into_iter().filter(|n| visible.contains(&n.id)).collect())
        } else {
            Ok(notes)
        }
    }

    /// Returns all notes whose `note_link` fields point to `target_id`.
    ///
    /// Queries the `note_links` junction table for every source note that
    /// currently references `target_id`, then fetches each full `Note`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn get_notes_with_link(&self, target_id: &str) -> Result<Vec<Note>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT nl.source_id FROM note_links nl WHERE nl.target_id = ?1",
        )?;
        let source_ids: Vec<String> = stmt
            .query_map([target_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        // Filter by read access before fetching full notes.
        let visible = self.visible_note_ids()?;
        let mut notes = Vec::new();
        for id in source_ids {
            if let Some(ref vis) = visible {
                if !vis.contains(&id) {
                    continue;
                }
            }
            notes.push(self.get_note(&id)?);
        }
        Ok(notes)
    }

    /// Searches for notes whose title or text-like field values contain `query`
    /// (case-insensitive substring match).
    ///
    /// If `target_type` is `Some`, only notes of that schema type are included.
    /// Returns an empty vec when `query` is blank.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] or
    /// [`crate::KrillnotesError::Json`] if the underlying note fetch fails.
    pub fn search_notes(
        &self,
        query: &str,
        target_schema: Option<&str>,
    ) -> Result<Vec<NoteSearchResult>> {
        let query_lower = query.to_lowercase();
        if query_lower.is_empty() {
            return Ok(vec![]);
        }

        let all_notes = self.list_all_notes()?;

        let results = all_notes
            .into_iter()
            .filter(|n| {
                if let Some(t) = target_schema {
                    n.schema == t
                } else {
                    true
                }
            })
            .filter(|n| {
                if n.title.to_lowercase().contains(&query_lower) {
                    return true;
                }
                for value in n.fields.values() {
                    match value {
                        FieldValue::Text(s) | FieldValue::Email(s) => {
                            if s.to_lowercase().contains(&query_lower) {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
                false
            })
            .map(|n| NoteSearchResult { id: n.id, title: n.title })
            .collect();

        Ok(results)
    }

    /// Rebuilds the `note_links` junction table from scratch by scanning all
    /// `fields_json` values for `NoteLink` entries.
    ///
    /// This is idempotent and safe to call at any time.  It is called
    /// automatically after a workspace import to restore link data that was not
    /// stored in the junction table at export time.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] or
    /// [`crate::KrillnotesError::Json`] if any note cannot be fetched.
    pub fn rebuild_note_links_index(&mut self) -> Result<()> {
        let all_notes = self.list_all_notes()?;
        let conn = self.storage.connection_mut();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM note_links", [])?;
        for note in &all_notes {
            for (field_name, value) in &note.fields {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    let exists: bool = tx.query_row(
                        "SELECT COUNT(*) FROM notes WHERE id = ?1",
                        [target_id],
                        |row| row.get::<_, i64>(0).map(|c| c > 0),
                    )?;
                    if exists {
                        tx.execute(
                            "INSERT INTO note_links (source_id, field_name, target_id)
                             VALUES (?1, ?2, ?3)",
                            [&note.id, field_name, target_id],
                        )?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Returns all notes in the workspace, ordered by `parent_id` then `position`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::Json`] if any row's `fields_json` is corrupt.
    pub fn list_all_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.connection().prepare(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             GROUP BY n.id
             ORDER BY n.parent_id, n.position",
        )?;

        let rows = stmt
            .query_map([], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let notes: Vec<Note> = rows.into_iter().map(note_from_row_tuple).collect::<Result<_>>()?;

        // Filter by read access
        if let Some(visible) = self.visible_note_ids()? {
            Ok(notes.into_iter().filter(|n| visible.contains(&n.id)).collect())
        } else {
            Ok(notes)
        }
    }

    /// Runs the `on_view` hook for the note's schema, falling back to a default
    /// HTML view when no hook is registered.
    ///
    /// The default view auto-renders `textarea` fields as CommonMark markdown.
    ///

    pub fn toggle_note_expansion(&mut self, note_id: &str) -> Result<()> {
        let tx = self.storage.connection_mut().transaction()?;

        // Get current value
        let current: i64 = tx.query_row(
            "SELECT is_expanded FROM notes WHERE id = ?",
            [note_id],
            |row| row.get(0)
        )?;

        // Toggle
        let new_value = if current == 1 { 0 } else { 1 };

        tx.execute(
            "UPDATE notes SET is_expanded = ? WHERE id = ?",
            rusqlite::params![new_value, note_id],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Persists the selected note ID to `workspace_meta`.
    ///
    /// Pass `None` to clear the selection. Like expansion state, selection is
    /// per-device UI state and is not written to the operation log.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn set_selected_note(&mut self, note_id: Option<&str>) -> Result<()> {
        let tx = self.storage.connection_mut().transaction()?;

        // Delete existing entry
        tx.execute(
            "DELETE FROM workspace_meta WHERE key = 'selected_note_id'",
            [],
        )?;

        // Insert new value if provided
        if let Some(id) = note_id {
            tx.execute(
                "INSERT INTO workspace_meta (key, value) VALUES ('selected_note_id', ?)",
                [id],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Returns the persisted selected note ID, or `None` if no selection is stored.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite error other
    /// than "no rows returned".
    pub fn get_selected_note(&self) -> Result<Option<String>> {
        let result = self.storage.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key = 'selected_note_id'",
            [],
            |row| row.get::<_, String>(0)
        );

        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Returns the workspace-level metadata (author, license, description, etc.).
    ///
    /// Returns a default (all-empty) [`WorkspaceMetadata`] when no metadata has been
    /// stored yet, so callers can always treat the result as present.
    pub fn get_workspace_metadata(&self) -> Result<WorkspaceMetadata> {
        let result = self.storage.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key = 'workspace_metadata'",
            [],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(WorkspaceMetadata::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persists workspace-level metadata (author, license, description, etc.).
    pub fn set_workspace_metadata(&mut self, metadata: &WorkspaceMetadata) -> Result<()> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        let json = serde_json::to_string(metadata).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(e))
        })?;
        self.storage.connection().execute(
            "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('workspace_metadata', ?)",
            [&json],
        )?;
        Ok(())
    }

    /// Moves a note to a new parent and/or position within the tree.
    ///
    /// The move is performed inside a single SQLite transaction. Positions in
    /// the old sibling group are closed (decremented) and positions in the new
    /// sibling group are opened (incremented) before the note itself is
    /// relocated. A `MoveNote` operation is logged for sync/undo.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::InvalidMove`] if the move would make a note
    /// its own parent or create an ancestor cycle. Returns
    /// [`KrillnotesError::NoteNotFound`] if `note_id` does not exist. Returns
    /// [`KrillnotesError::Database`] for any SQLite failure.
    pub fn move_note(
        &mut self,
        note_id: &str,
        new_parent_id: Option<&str>,
        new_position: f64,
    ) -> Result<()> {
        // 1. Self-move check
        if new_parent_id == Some(note_id) {
            return Err(KrillnotesError::InvalidMove(
                "A note cannot be its own parent".to_string(),
            ));
        }

        // 2. Cycle check: walk ancestor chain of new_parent_id
        if let Some(target_parent) = new_parent_id {
            let mut current = target_parent.to_string();
            loop {
                let parent: Option<String> = self
                    .connection()
                    .query_row(
                        "SELECT parent_id FROM notes WHERE id = ?",
                        [&current],
                        |row| row.get(0),
                    )
                    .map_err(|_| {
                        KrillnotesError::NoteNotFound(current.clone())
                    })?;
                match parent {
                    Some(pid) => {
                        if pid == note_id {
                            return Err(KrillnotesError::InvalidMove(
                                "Move would create a cycle".to_string(),
                            ));
                        }
                        current = pid;
                    }
                    None => break,
                }
            }
        }

        // 3. Allowed-parent-schemas check
        let note_to_move = self.get_note(note_id)?;
        let schema = self.script_registry.get_schema(&note_to_move.schema)?;
        if !schema.allowed_parent_schemas.is_empty() {
            match new_parent_id {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", note_to_move.schema
                ))),
                Some(pid) => {
                    let parent_note = self.get_note(pid)?;
                    if !schema.allowed_parent_schemas.contains(&parent_note.schema) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            note_to_move.schema, parent_note.schema
                        )));
                    }
                }
            }
        }

        // 3b. Allowed-children-schemas check on the new parent
        if let Some(pid) = new_parent_id {
            let parent_note = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent_note.schema)?;
            if parent_schema.is_leaf {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Cannot add children to a leaf note (schema: '{}')",
                    parent_note.schema
                )));
            }
            if !parent_schema.allowed_children_schemas.is_empty()
                && !parent_schema.allowed_children_schemas.contains(&note_to_move.schema)
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    note_to_move.schema, parent_note.schema
                )));
            }
        }

        // Fetch the new parent note before opening the transaction (avoids borrow conflict with `tx`).
        let hook_new_parent = if let Some(pid) = new_parent_id {
            Some(self.get_note(pid)?)
        } else {
            None
        };

        // 4. Get the note's current parent_id and position
        let note = self.get_note(note_id)?;
        let old_parent_id = note.parent_id.clone();
        let old_position = note.position;

        // Authorize before opening the transaction.
        let auth_op = Operation::MoveNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            new_parent_id: new_parent_id.map(|s| s.to_string()),
            new_position,
            moved_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        let now = chrono::Utc::now().timestamp();
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        // 5. Close the gap in the old sibling group
        // Exclude the note itself: during a same-parent move it still occupies
        // old_position in the DB until step 7.
        tx.execute(
            "UPDATE notes SET position = position - 1 WHERE parent_id IS ? AND position > ? AND id != ?",
            rusqlite::params![old_parent_id, old_position, note_id],
        )?;

        // 6. Open a gap in the new sibling group
        tx.execute(
            "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ? AND id != ?",
            rusqlite::params![new_parent_id, new_position, note_id],
        )?;

        // 7. Update the note itself
        tx.execute(
            "UPDATE notes SET parent_id = ?, position = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![new_parent_id, new_position, now, note_id],
        )?;

        // Run on_add_child hook if the new parent's schema defines one.
        if let Some(ref parent_note) = hook_new_parent {
            if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
                &parent_note.schema,
                &parent_note.id, &parent_note.schema, &parent_note.title, &parent_note.fields,
                &note_to_move.id, &note_to_move.schema, &note_to_move.title, &note_to_move.fields,
            )? {
                let hook_now = chrono::Utc::now().timestamp();
                if let Some((new_title, new_fields)) = hook_result.child {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, hook_now, note_to_move.id],
                    )?;
                }
                if let Some((new_title, new_fields)) = hook_result.parent {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, hook_now, parent_note.id],
                    )?;
                }
            }
        }

        // 8. Log a MoveNote operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::MoveNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            new_parent_id: new_parent_id.map(|s| s.to_string()),
            new_position,
            moved_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // 9. Commit
        tx.commit()?;

        // Push undo entry — inverse of MoveNote is PositionRestore.
        let op_id = op.operation_id().to_string();
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::PositionRestore {
                note_id: note_id.to_string(),
                old_parent_id,
                old_position,
            },
            propagate: true,
        });

        Ok(())
    }

    /// Returns the direct children of `parent_id` as a [`Vec<Note>`], ordered
    /// by `position`.
    ///
    /// Only immediate children are returned; grandchildren and deeper
    /// descendants are not included.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError`] if the database query fails.
    pub fn get_children(&self, parent_id: &str) -> Result<Vec<Note>> {
        self.check_read_access(parent_id)?;

        let mut stmt = self.connection().prepare(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             WHERE n.parent_id = ?1
             GROUP BY n.id
             ORDER BY n.position",
        )?;

        let rows = stmt
            .query_map(rusqlite::params![parent_id], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Deletes `note_id` and all of its descendants recursively.
    ///
    /// The entire subtree rooted at `note_id` is removed within a single
    /// SQLite transaction, so a mid-subtree failure leaves the database
    /// unchanged. Every note in the subtree is deleted from the `notes`
    /// table; no re-parenting occurs. The returned [`DeleteResult`] reports
    /// the total count of removed notes and every deleted ID.
    ///
    /// This operation is intentionally excluded from the operation log:
    /// destructive bulk deletes are not currently part of the collaborative
    /// sync model and would require tombstone handling to be safe.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if any SQLite operation
    /// fails, including when `note_id` does not exist (the DELETE silently
    /// affects zero rows, but child queries will return empty results rather
    /// than errors in that case). The transaction is rolled back automatically
    /// on any failure.
    pub fn delete_note_recursive(&mut self, note_id: &str) -> Result<DeleteResult> {
        // Authorize before opening any transaction.
        let auth_op = Operation::DeleteNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            deleted_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        // Capture full subtree for undo before any deletion.
        let subtree_notes = self.collect_subtree_notes(note_id)?;
        let subtree_ids: Vec<&str> = subtree_notes.iter().map(|n| n.id.as_str()).collect();
        let attachments = self.list_all_attachments()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| subtree_ids.contains(&a.note_id.as_str()))
            .collect::<Vec<_>>();

        // Generate a stable operation ID before the deletion transaction.
        let op_id = Uuid::new_v4().to_string();

        // Collect all IDs in the subtree that will be deleted, then clear any
        // incoming NoteLink fields from other notes before the deletion transaction
        // opens (satisfies the note_links.target_id ON DELETE RESTRICT constraint).
        let all_ids = self.collect_subtree_ids(note_id)?;
        for id in &all_ids {
            self.clear_links_to(id)?;
        }

        let tx = self.storage.connection_mut().transaction()?;

        // Clean up any permission grants anchored on deleted notes.
        // The note_permissions table may not exist (created by RbacGate),
        // so silently ignore errors.
        let _ = tx.execute(
            "DELETE FROM note_permissions WHERE note_id IN (
                WITH RECURSIVE subtree(id) AS (
                    SELECT ?1
                    UNION ALL
                    SELECT n.id FROM notes n JOIN subtree s ON n.parent_id = s.id
                )
                SELECT id FROM subtree
            )",
            [&note_id],
        );

        let result = Self::delete_recursive_in_tx(&tx, note_id)?;
        tx.commit()?;

        // Log a DeleteNote operation for the root of the deleted subtree.
        // Uses a separate transaction since the deletion tx was already committed.
        // Advance HLC and capture signing key before the second transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        {
            let tx = self.storage.connection_mut().transaction()?;
            Self::save_hlc(&ts, &tx)?;
            let mut op = Operation::DeleteNote {
                operation_id: op_id.clone(),
                timestamp: ts,
                device_id: self.device_id.clone(),
                note_id: note_id.to_string(),
                deleted_by: String::new(),
                signature: String::new(),
            };
            Self::sign_op_with(&signing_key, &mut op);
            Self::log_op(&self.operation_log, &tx, &op)?;
            Self::purge_ops_if_needed(&self.operation_log, &tx)?;
            tx.commit()?;
        }

        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::SubtreeRestore { notes: subtree_notes, attachments },
            propagate: true,
        });

        Ok(result)
    }

    /// Recursively deletes `note_id` and all descendants within an existing transaction.
    ///
    /// Only child IDs are fetched (not full `Note` structs) to keep the query
    /// minimal. Deletion proceeds depth-first: children are removed before
    /// their parent so that any future foreign-key constraint can be satisfied.
    ///
    /// This helper must not open its own transaction; callers are responsible
    /// for wrapping the call in a transaction, as SQLite does not support
    /// nested transactions.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub(crate) fn delete_recursive_in_tx(
        tx: &rusqlite::Transaction,
        note_id: &str,
    ) -> Result<DeleteResult> {
        let mut affected_ids = vec![note_id.to_string()];

        // Fetch only the IDs of direct children — avoids deserialising full
        // Note structs and keeps the recursive helper lightweight.
        let mut stmt = tx.prepare("SELECT id FROM notes WHERE parent_id = ?1")?;
        let child_ids: Vec<String> = stmt
            .query_map(rusqlite::params![note_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Recurse into children before deleting this node (leaves-first order).
        for child_id in child_ids {
            let child_result = Self::delete_recursive_in_tx(tx, &child_id)?;
            affected_ids.extend(child_result.affected_ids);
        }

        // Delete this note after all descendants have been removed.
        tx.execute(
            "DELETE FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
        )?;

        // Detect nonexistent root IDs: SQLite DELETE silently affects zero rows
        // when the ID does not exist. Surface this as NoteNotFound.
        if tx.changes() == 0 {
            return Err(KrillnotesError::NoteNotFound(note_id.to_string()));
        }

        Ok(DeleteResult {
            deleted_count: affected_ids.len(),
            affected_ids,
        })
    }

    /// Deletes `note_id` and promotes its children to its grandparent.
    ///
    /// The note identified by `note_id` is removed from the `notes` table while
    /// all of its direct children are re-parented to the deleted note's own
    /// parent. Children of children (grandchildren of the deleted note) are not
    /// affected — they retain their existing parent. The entire operation runs
    /// inside a single SQLite transaction, so any failure leaves the database
    /// unchanged.
    ///
    /// The returned [`DeleteResult`] always has `deleted_count == 1` and
    /// `affected_ids` containing only `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::NoteNotFound`] if no note with
    /// `note_id` exists in the database. Returns
    /// [`crate::KrillnotesError::Database`] for any other SQLite failure.
    /// The transaction is rolled back automatically on any failure.
    pub fn delete_note_promote(&mut self, note_id: &str) -> Result<DeleteResult> {
        // Authorize before opening any transaction.
        let auth_op = Operation::DeleteNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            deleted_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        // Capture before-state for undo before any mutations.
        // Map Database error (QueryReturnedNoRows) to NoteNotFound for a missing ID.
        let deleted_note = self.get_note(note_id)
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
        let children = self.get_children(note_id)?;
        let deleted_attachments = self.get_attachments(note_id).unwrap_or_default();

        // Generate a stable operation ID before the deletion transaction.
        let op_id = Uuid::new_v4().to_string();

        // Clear incoming NoteLink fields from other notes before opening the
        // deletion transaction (satisfies note_links.target_id ON DELETE RESTRICT).
        self.clear_links_to(note_id)?;

        // Advance HLC and capture signing key before the transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        // Fetch the note's parent — surfaces NoteNotFound for missing IDs.
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                rusqlite::params![note_id],
                |row| row.get(0),
            )
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Re-parent all direct children to the grandparent (may be NULL).
        tx.execute(
            "UPDATE notes SET parent_id = ?1 WHERE parent_id = ?2",
            rusqlite::params![parent_id, note_id],
        )?;

        // Renumber all children of the new parent to avoid position collisions
        let child_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM notes WHERE parent_id IS ?1 ORDER BY position, id",
            )?;
            let ids = stmt.query_map(rusqlite::params![parent_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids
        };
        for (position, id) in child_ids.iter().enumerate() {
            tx.execute(
                "UPDATE notes SET position = ?1 WHERE id = ?2",
                rusqlite::params![position as i64, id],
            )?;
        }

        // Clean up permission grants on the deleted note only (children survive).
        // The note_permissions table may not exist (created by RbacGate),
        // so silently ignore errors.
        let _ = tx.execute(
            "DELETE FROM note_permissions WHERE note_id = ?1",
            rusqlite::params![note_id],
        );

        // Delete the note itself after its children have been safely re-parented.
        tx.execute(
            "DELETE FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
        )?;

        // Log a DeleteNote operation for the promoted note.
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::DeleteNote {
            operation_id: op_id.clone(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            deleted_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        // Build the Batch undo entry.
        //
        // `apply_retract_inverse_internal` applies Batch items with `.iter().rev()`
        // (LIFO), so the last item pushed is applied first.
        //
        // Required execution order on undo:
        //   1. SubtreeRestore — recreates the deleted note (must exist before
        //      children can point to it).
        //   2. PositionRestore for each child — moves them back to point at the
        //      restored note (each child's old_parent_id was note_id).
        //
        // To achieve that with LIFO: push PositionRestores FIRST, SubtreeRestore LAST.
        let mut batch_items: Vec<RetractInverse> = Vec::new();
        for child in &children {
            batch_items.push(RetractInverse::PositionRestore {
                note_id: child.id.clone(),
                old_parent_id: Some(deleted_note.id.clone()),
                old_position: child.position,
            });
        }
        batch_items.push(RetractInverse::SubtreeRestore {
            notes: vec![deleted_note.clone()],
            attachments: deleted_attachments,
        });
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::Batch(batch_items),
            propagate: true,
        });

        Ok(DeleteResult {
            deleted_count: 1,
            affected_ids: vec![note_id.to_string()],
        })
    }

    /// Deletes `note_id` using the specified [`DeleteStrategy`].
    ///
    /// This is the single public entry-point for note deletion. It dispatches
    /// to one of two internal methods:
    ///
    /// - [`DeleteStrategy::DeleteAll`] — calls [`Self::delete_note_recursive`],
    ///   which removes the note and every descendant in a single atomic
    ///   transaction.
    /// - [`DeleteStrategy::PromoteChildren`] — calls [`Self::delete_note_promote`],
    ///   which removes only the note itself and re-parents its direct children
    ///   to the deleted note's former parent.
    ///
    /// The returned [`DeleteResult`] reports the total count of deleted notes
    /// and the IDs of every affected note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::NoteNotFound`] (for `PromoteChildren`)
    /// or [`crate::KrillnotesError::Database`] (for either strategy) if the
    /// underlying operation fails. All database mutations are transactional;
    /// a failure leaves the workspace unchanged.
    pub fn delete_note(
        &mut self,
        note_id: &str,
        strategy: DeleteStrategy,
    ) -> Result<DeleteResult> {
        match strategy {
            DeleteStrategy::DeleteAll => self.delete_note_recursive(note_id),
            DeleteStrategy::PromoteChildren => self.delete_note_promote(note_id),
        }
    }

    /// Returns the number of direct children of `note_id`.
    ///
    /// Counts rows in the `notes` table whose `parent_id` equals `note_id`.
    /// Grandchildren and deeper descendants are not included.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure,
    /// including when `note_id` does not exist (the count will be zero in
    /// that case rather than an error, but connection failures are surfaced).
    pub fn count_children(&self, note_id: &str) -> Result<usize> {
        self.check_read_access(note_id)?;

        let count: i64 = self.storage.connection().query_row(
            "SELECT COUNT(*) FROM notes WHERE parent_id = ?1",
            rusqlite::params![note_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Updates the `title` and `fields` of an existing note, refreshing `modified_at`.
    ///
    /// Both the title and the full fields map are replaced atomically within a
    /// single SQLite transaction. The `modified_at` timestamp is set to the
    /// current UTC second and `modified_by` is set to the active user ID.
    ///
    /// # Errors
    ///
    /// Full 7-step save pipeline with validation:
    ///
    /// 1. Evaluate group visibility
    /// 2. Run field `validate` closures (only on visible fields)
    /// 3. Check required constraints (only on visible fields)
    /// 4-7. Delegate to `update_note` (on_save hook + DB write)
    ///
    /// Returns `SaveResult::ValidationErrors` when any step produces errors.
    /// Returns `SaveResult::Ok(note)` on success.
    pub fn save_note_with_pipeline(
        &mut self,
        note_id: &str,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<SaveResult> {
        let note = self.get_note(note_id)
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
        let schema = self.script_registry.get_schema(&note.schema)?;

        // Step 1: Evaluate group visibility.
        let visibility = self.script_registry.evaluate_group_visibility(
            &note.schema, &fields,
        )?;

        // Collect visible field names (top-level + fields from visible groups).
        let visible_field_names: std::collections::HashSet<String> = schema.fields.iter()
            .map(|f| f.name.clone())
            .chain(
                schema.field_groups.iter()
                    .filter(|g| visibility.get(&g.name).copied().unwrap_or(true))
                    .flat_map(|g| g.fields.iter().map(|f| f.name.clone()))
            )
            .collect();

        // Step 2: Run validate closures on visible fields.
        let all_errors = self.script_registry.validate_fields(&note.schema, &fields)?;
        let mut field_errors: BTreeMap<String, String> = all_errors.into_iter()
            .filter(|(k, _)| visible_field_names.contains(k))
            .collect();

        // Step 3: Required check on visible required fields.
        for field_def in schema.all_fields() {
            if field_def.required && visible_field_names.contains(&field_def.name) {
                let empty = match fields.get(&field_def.name) {
                    None => true,
                    Some(FieldValue::Text(s))   => s.is_empty(),
                    Some(FieldValue::Email(s))  => s.is_empty(),
                    Some(FieldValue::Date(None))
                    | Some(FieldValue::NoteLink(None))
                    | Some(FieldValue::File(None)) => true,
                    _ => false,
                };
                if empty && !field_errors.contains_key(&field_def.name) {
                    field_errors.insert(field_def.name.clone(), "Required".to_string());
                }
            }
        }

        if !field_errors.is_empty() {
            return Ok(SaveResult::ValidationErrors {
                field_errors,
                note_errors: vec![],
                preview_title: None,
                preview_fields: BTreeMap::new(),
            });
        }

        // Build final_fields: start from schema defaults, overlay existing note values,
        // then apply user-provided visible-field values. Hidden-group required fields
        // retain their existing/default values so update_note's validate_required_fields
        // doesn't reject them (that check is visibility-unaware).
        let mut final_fields = schema.default_fields();
        for (k, v) in &note.fields {
            final_fields.insert(k.clone(), v.clone());
        }
        for (k, v) in &fields {
            final_fields.insert(k.clone(), v.clone());
        }

        // Steps 4-7: update_note (runs on_save hook + writes to DB).
        match self.update_note(note_id, title, final_fields) {
            Ok(updated) => Ok(SaveResult::Ok(updated)),
            Err(KrillnotesError::ValidationFailed(msg)) => {
                Ok(SaveResult::ValidationErrors {
                    field_errors: BTreeMap::new(),
                    note_errors: vec![msg],
                    preview_title: None,
                    preview_fields: BTreeMap::new(),
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Returns [`crate::KrillnotesError::NoteNotFound`] if no note with `note_id`
    /// exists in the database.  Returns [`crate::KrillnotesError::Json`] if
    /// `fields` cannot be serialised to JSON.  Returns
    /// [`crate::KrillnotesError::Database`] for any other SQLite failure.
    pub fn update_note(
        &mut self,
        note_id: &str,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<Note> {
        // Authorize before opening any transaction.
        let auth_op = Operation::UpdateNote {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            title: title.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        // Capture before-state for undo.
        // Map Database errors (e.g. QueryReturnedNoRows) to NoteNotFound so that
        // callers see a consistent error type when the note does not exist.
        let old_note = self.get_note(note_id)
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Look up this note's schema so the pre-save hook can be dispatched.
        let note_schema: String = self
            .storage
            .connection()
            .query_row(
                "SELECT schema FROM notes WHERE id = ?1",
                rusqlite::params![note_id],
                |row| row.get(0),
            )
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Run the pre-save hook via the gated SaveTransaction model.
        // - hook not registered         → no-op (keep passed-in title/fields)
        // - hook called commit()        → apply effective_title / effective_fields
        // - hook called reject(…)       → return ValidationFailed error
        // - hook returned Map (old API) → hard Scripting error with migration message
        let (title, fields) =
            match self
                .script_registry
                .run_on_save_hook(&note_schema, note_id, &note_schema, &title, &fields)?
            {
                None => (title, fields),
                Some(tx) if tx.committed => {
                    let pn = tx.pending_notes.get(note_id)
                        .ok_or_else(|| KrillnotesError::Scripting(
                            format!("on_save hook committed but pending note '{}' not found", note_id)
                        ))?;
                    (pn.effective_title().to_string(), pn.effective_fields())
                }
                Some(tx) if tx.has_errors() => {
                    let msgs: Vec<String> = tx.soft_errors.iter().map(|e| {
                        match &e.field {
                            Some(f) => format!("{}: {}", f, e.message),
                            None => e.message.clone(),
                        }
                    }).collect();
                    return Err(KrillnotesError::ValidationFailed(msgs.join("; ")));
                }
                Some(_) => (title, fields),  // hook ran but didn't commit → no-op
            };

        // Enforce required-field constraints defined in the schema.
        let schema = self.script_registry.get_schema(&note_schema)?;
        schema.validate_required_fields(&fields)?;

        let now = chrono::Utc::now().timestamp();
        let fields_json = serde_json::to_string(&fields)?;

        // Clean up replaced or cleared File field attachments before the note UPDATE.
        // Must run before connection_mut() is borrowed for the transaction below,
        // since delete_attachment uses connection() (shared ref) which conflicts with
        // an active connection_mut() Transaction.
        //
        // Note: if delete_attachment succeeds but the tx.commit() below fails, the
        // note row still references old_uuid while the attachment is already gone,
        // leaving a dangling File field reference. This is an accepted trade-off
        // for a single-writer local store where commit failures are rare.
        {
            let old_fields_json: String = self
                .storage
                .connection()
                .query_row(
                    "SELECT fields_json FROM notes WHERE id = ?1",
                    rusqlite::params![note_id],
                    |row| row.get(0),
                )
                .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
            let old_fields: BTreeMap<String, FieldValue> =
                serde_json::from_str(&old_fields_json).unwrap_or_default();

            for (key, old_val) in &old_fields {
                if let FieldValue::File(Some(old_uuid)) = old_val {
                    let still_same = matches!(
                        fields.get(key),
                        Some(FieldValue::File(Some(u))) if u == old_uuid
                    );
                    if !still_same {
                        let _ = self.delete_attachment(old_uuid, None); // best-effort
                    }
                }
            }
        }

        // Collector for all operation IDs emitted during this update,
        // used to populate the undo entry's retracted_ids.
        let mut emitted_op_ids: Vec<String> = Vec::new();

        // Pre-advance HLC for title op + one per field, and capture signing key,
        // before the transaction borrows self.storage mutably.
        let title_ts = self.advance_hlc();
        let field_timestamps: Vec<HlcTimestamp> = fields.keys()
            .map(|_| self.advance_hlc())
            .collect();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        let current_schema_version = self.script_registry
            .get_schema(&note_schema)
            .map(|s| s.version)
            .unwrap_or(1);
        tx.execute(
            "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3, modified_by = ?4, schema_version = ?5 WHERE id = ?6",
            rusqlite::params![title, fields_json, now, self.current_identity_pubkey.clone(), current_schema_version, note_id],
        )?;

        // Detect nonexistent IDs: SQLite UPDATE on a missing row succeeds but
        // touches zero rows. Surface this as NoteNotFound rather than silently
        // returning stale data.
        if tx.changes() == 0 {
            return Err(KrillnotesError::NoteNotFound(note_id.to_string()));
        }

        // Log an UpdateNote operation for the title, consistent with
        // update_note_title.
        Self::save_hlc(&title_ts, &tx)?;
        let title_op_id = Uuid::new_v4().to_string();
        emitted_op_ids.push(title_op_id.clone());
        let mut title_op = Operation::UpdateNote {
            operation_id: title_op_id,
            timestamp: title_ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            title: title.clone(),
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut title_op);
        Self::log_op(&self.operation_log, &tx, &title_op)?;

        // Log one UpdateField operation per field value that was written.
        for ((field_key, field_value), field_ts) in fields.iter().zip(field_timestamps.iter()) {
            Self::save_hlc(field_ts, &tx)?;
            let field_op_id = Uuid::new_v4().to_string();
            emitted_op_ids.push(field_op_id.clone());
            let mut field_op = Operation::UpdateField {
                operation_id: field_op_id,
                timestamp: *field_ts,
                device_id: self.device_id.clone(),
                note_id: note_id.to_string(),
                field: field_key.clone(),
                value: field_value.clone(),
                modified_by: String::new(),
                signature: String::new(),
            };
            Self::sign_op_with(&signing_key, &mut field_op);
            Self::log_op(&self.operation_log, &tx, &field_op)?;
        }

        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // Keep the note_links junction table in sync with the written field values.
        // Must run inside the transaction so the link update is atomic with the note write.
        sync_note_links(&tx, note_id, &fields)?;

        tx.commit()?;

        // Push undo entry — inverse of UpdateNote is NoteRestore.
        // textarea fields use CRDT on peers; mark as non-propagating for v1.
        self.push_undo(UndoEntry {
            retracted_ids: emitted_op_ids,
            inverse: RetractInverse::NoteRestore {
                note_id: note_id.to_string(),
                old_title: old_note.title,
                old_fields: old_note.fields,
                old_tags: old_note.tags,
            },
            propagate: false,
        });

        // Re-use get_note to fetch the persisted row, keeping row-mapping logic
        // in a single place.
        self.get_note(note_id)
    }


    ///
    /// Returns a flat `Vec<String>` containing the root ID plus all descendant
    /// IDs in an unspecified order.
    pub(crate) fn collect_subtree_ids(&self, note_id: &str) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "WITH RECURSIVE subtree AS (
                SELECT id FROM notes WHERE id = ?1
                UNION ALL
                SELECT n.id FROM notes n JOIN subtree s ON n.parent_id = s.id
            )
            SELECT id FROM subtree",
        )?;
        let ids: Vec<String> = stmt
            .query_map([note_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Finds all notes that have a `NoteLink` field pointing to `target_id`,
    /// sets those fields to `NoteLink(None)` in `fields_json`, and removes
    /// the corresponding rows from the `note_links` junction table.
    ///
    /// This must be called BEFORE the target note is deleted so that the
    /// `note_links.target_id REFERENCES notes(id) ON DELETE RESTRICT`
    /// constraint is satisfied.
    ///
    /// All changes (field patches + junction-table delete) are committed in a
    /// single transaction. If no notes link to `target_id` the function
    /// returns immediately without touching the database.
    pub fn clear_links_to(&mut self, target_id: &str) -> Result<()> {
        // Find all notes linking to this target (read-only, uses shared ref).
        let links: Vec<(String, String)> = {
            let conn = self.connection();
            let mut stmt = conn.prepare(
                "SELECT source_id, field_name FROM note_links WHERE target_id = ?1",
            )?;
            let rows = stmt.query_map([target_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };

        if links.is_empty() {
            return Ok(());
        }

        // For each linking note: load fields_json, patch the field to NoteLink(None), save back.
        let conn = self.storage.connection_mut();
        let tx = conn.transaction()?;
        for (source_id, field_name) in &links {
            let fields_json: String = tx.query_row(
                "SELECT fields_json FROM notes WHERE id = ?1",
                [source_id],
                |row| row.get(0),
            )?;
            let mut json_val: serde_json::Value = serde_json::from_str(&fields_json)?;
            if let Some(obj) = json_val.as_object_mut() {
                // NoteLink(None) serializes as {"NoteLink":null} under serde external tagging.
                obj.insert(field_name.clone(), serde_json::json!({"NoteLink": null}));
            }
            let updated_json = serde_json::to_string(&json_val)?;
            tx.execute(
                "UPDATE notes SET fields_json = ?1 WHERE id = ?2",
                [&updated_json, source_id],
            )?;
        }
        tx.execute("DELETE FROM note_links WHERE target_id = ?1", [target_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Returns full `Note` data for every node in the subtree rooted at `note_id`,
    /// ordered parent-first (root at index 0) via a recursive CTE.
    pub(crate) fn collect_subtree_notes(&self, note_id: &str) -> Result<Vec<Note>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "WITH RECURSIVE subtree AS (
                SELECT n.id, 0 AS depth FROM notes n WHERE n.id = ?1
                UNION ALL
                SELECT n.id, s.depth + 1 FROM notes n JOIN subtree s ON n.parent_id = s.id
            )
            SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                   n.created_at, n.modified_at, n.created_by, n.modified_by,
                   n.fields_json, n.is_expanded, n.schema_version,
                   GROUP_CONCAT(nt.tag, ',') AS tags_csv
            FROM notes n
            JOIN subtree s ON n.id = s.id
            LEFT JOIN note_tags nt ON nt.note_id = n.id
            GROUP BY n.id
            ORDER BY s.depth ASC",
        )?;
        let rows = stmt.query_map([note_id], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(note_from_row_tuple).collect()
    }

}
