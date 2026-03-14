// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! View hooks, hover hooks, tree actions, and query context building.

use super::*;

impl Workspace {
    fn build_query_context(&self) -> Result<QueryContext> {
        let all_notes = self.list_all_notes()?;
        let mut notes_by_id: HashMap<String, Dynamic> = HashMap::new();
        let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_type: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_tag: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_link_target: HashMap<String, Vec<Dynamic>> = HashMap::new();

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }

        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }

        Ok(QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id })
    }

    /// # Errors
    ///
    /// Returns [`KrillnotesError::Database`] if the note or any workspace note
    /// cannot be fetched, or [`KrillnotesError::Scripting`] if the hook fails.
    pub fn run_view_hook(&self, note_id: &str) -> Result<String> {
        let note = self.get_note(note_id)?;

        // No hook registered: generate the default view without fetching all notes.
        if !self.script_registry.has_views(&note.schema) {
            // Pre-resolve NoteLink field targets to titles for the default renderer.
            let mut resolved_titles: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            for value in note.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    if let Ok(linked) = self.get_note(target_id) {
                        resolved_titles.insert(target_id.clone(), linked.title);
                    }
                }
            }
            let attachments = self.get_attachments(&note.id).unwrap_or_default();
            return Ok(self.script_registry.render_default_view(&note, &resolved_titles, &attachments));
        }

        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: std::collections::HashMap<String, Dynamic> =
            std::collections::HashMap::new();
        let mut children_by_id: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_type: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_tag: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_link_target: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }

        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id };

        // Set per-run context so markdown() and other helpers can resolve attachments.
        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        self.script_registry.set_run_context(note.clone(), attachments);
        // RAII guard: ensures run_context is cleared even if hook panics
        struct RunContextGuard<'a>(&'a crate::core::scripting::ScriptRegistry);
        impl Drop for RunContextGuard<'_> {
            fn drop(&mut self) { self.0.clear_run_context(); }
        }
        let _guard = RunContextGuard(&self.script_registry);
        // run_on_view_hook returns Some(...) since we've confirmed a hook exists above.
        self
            .script_registry
            .run_on_view_hook(&note, context)
            .map(|opt| opt.unwrap_or_default())
            .map(|html| self.embed_attachment_images(html))
    }

    /// Runs the `on_hover` hook for the given note, if one is registered.
    ///
    /// Returns `Ok(None)` when no hook is registered for the note's schema type.
    /// Returns `Ok(Some(html))` with the generated HTML on success.
    pub fn run_hover_hook(&self, note_id: &str) -> Result<Option<String>> {
        let note = self.get_note(note_id)?;

        if !self.script_registry.has_hover(&note.schema) {
            return Ok(None);
        }

        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: std::collections::HashMap<String, Dynamic> =
            std::collections::HashMap::new();
        let mut children_by_id: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_type: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_tag: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_link_target: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }

        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id };

        // Set per-run context so markdown() and other helpers can resolve attachments.
        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        self.script_registry.set_run_context(note.clone(), attachments);
        // RAII guard: ensures run_context is cleared even if hook panics
        struct RunContextGuard<'a>(&'a crate::core::scripting::ScriptRegistry);
        impl Drop for RunContextGuard<'_> {
            fn drop(&mut self) { self.0.clear_run_context(); }
        }
        let _guard = RunContextGuard(&self.script_registry);
        self.script_registry
            .run_on_hover_hook(&note, context)
            .map(|opt| opt.map(|html| self.embed_attachment_images(html)))
    }

    /// Returns the names of all registered note types (schema names).
    ///
    /// # Errors
    ///
    /// This method currently does not fail, but returns `Result` for consistency.
    pub fn list_node_types(&self) -> Result<Vec<String>> {
        self.script_registry.list_types()
    }

    /// Runs the tree action named `label` on the note identified by `note_id`.
    ///
    /// Builds a full `QueryContext` (same as `run_view_hook`), calls the registered
    /// callback, and — if the callback returns an array of note IDs — reorders
    /// those notes by calling `move_note` in the given order.
    ///
    /// # Errors
    ///
    /// Returns an error if the note or any workspace note cannot be fetched, if
    /// no action is registered under `label`, or if the callback throws.
    pub fn run_tree_action(&mut self, note_id: &str, label: &str) -> Result<()> {
        self.begin_undo_group();
        let result = self.run_tree_action_inner(note_id, label);
        self.end_undo_group();
        result
    }

    fn run_tree_action_inner(&mut self, note_id: &str, label: &str) -> Result<()> {
        let note = self.get_note(note_id)?;
        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: HashMap<String, Dynamic> = HashMap::new();
        let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_type: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_tag: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_link_target: HashMap<String, Vec<Dynamic>> = HashMap::new();
        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }
        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id };

        // invoke_tree_action_hook returns an error if the script throws — in that case
        // we propagate the error without touching the DB (implicit rollback).
        let result = self.script_registry.invoke_tree_action_hook(label, &note, context)?;

        // Apply pending notes from the SaveTransaction atomically, if any were queued.
        let tx_pending = result.transaction;
        // Separate the acted-upon note (is_new == false) from new child notes.
        // New notes are sorted topologically so parents are inserted before children —
        // this is required to satisfy the FK constraint when the parent itself is a new note.
        let all_pending: Vec<_> = tx_pending.pending_notes.into_values().collect();
        let (existing_updates, mut new_creates): (Vec<_>, Vec<_>) =
            all_pending.into_iter().partition(|p| !p.is_new);
        // Topological sort for new creates: a note whose parent_id is also a new note must
        // come after its parent. IDs of new notes collected for quick look-up.
        let new_ids: std::collections::HashSet<String> =
            new_creates.iter().map(|p| p.note_id.clone()).collect();
        let mut ordered_creates: Vec<_> = Vec::with_capacity(new_creates.len());
        let mut remaining = new_creates.len();
        let mut iters = 0usize;
        while !new_creates.is_empty() {
            iters += 1;
            if iters > new_creates.len() * new_creates.len() + 1 {
                // Cycle guard — should never happen in practice; break to avoid infinite loop.
                ordered_creates.extend(new_creates.drain(..));
                break;
            }
            let mut next = Vec::with_capacity(new_creates.len());
            for pending in new_creates.drain(..) {
                let parent_is_new = pending.parent_id.as_ref()
                    .map(|pid| new_ids.contains(pid.as_str()))
                    .unwrap_or(false);
                let parent_already_emitted = pending.parent_id.as_ref()
                    .map(|pid| ordered_creates.iter().any(|e: &crate::core::save_transaction::PendingNote| &e.note_id == pid))
                    .unwrap_or(true);
                if !parent_is_new || parent_already_emitted {
                    ordered_creates.push(pending);
                } else {
                    next.push(pending);
                }
            }
            new_creates = next;
            if new_creates.len() == remaining {
                // No progress — break to avoid infinite loop.
                ordered_creates.extend(new_creates.drain(..));
                break;
            }
            remaining = new_creates.len();
        }
        // Combine: existing updates first (or last — order doesn't matter between them and creates)
        // then topologically sorted creates.
        let pending_notes: Vec<_> = existing_updates.into_iter().chain(ordered_creates).collect();

        if !pending_notes.is_empty() {
            let now = chrono::Utc::now().timestamp();

            // Pre-advance HLC for each pending note before borrowing self.storage.
            // Creates need one timestamp; updates need one for title + one per field.
            let timestamps: Vec<(HlcTimestamp, Vec<HlcTimestamp>)> = pending_notes.iter()
                .map(|p| {
                    let main_ts = self.advance_hlc();
                    let field_tss: Vec<HlcTimestamp> = if p.is_new {
                        vec![]
                    } else {
                        p.effective_fields().keys().map(|_| self.advance_hlc()).collect()
                    };
                    (main_ts, field_tss)
                })
                .collect();
            let signing_key = self.signing_key.clone();

            let tx_db = self.storage.connection_mut().transaction()?;

            for (pending, (main_ts, field_tss)) in pending_notes.iter().zip(timestamps.iter()) {
                if pending.is_new {
                    // ── INSERT new note ──────────────────────────────────────────
                    let parent_id = pending.parent_id.as_deref().unwrap_or("");
                    let position: i32 = tx_db.query_row(
                        "SELECT COALESCE(MAX(position), -1) + 1 FROM notes WHERE parent_id = ?1",
                        rusqlite::params![parent_id],
                        |row| row.get(0),
                    )?;
                    let effective_fields = pending.effective_fields();
                    let fields_json = serde_json::to_string(&effective_fields)?;
                    let effective_title = pending.effective_title();

                    let schema_ver = self.script_registry.get_schema(&pending.schema)
                        .map(|s| s.version).unwrap_or(1);
                    tx_db.execute(
                        "INSERT INTO notes (id, title, schema, parent_id, position, \
                                            created_at, modified_at, created_by, modified_by, \
                                            fields_json, is_expanded, schema_version) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        rusqlite::params![
                            pending.note_id, effective_title, pending.schema,
                            parent_id, position, now, now,
                            self.current_identity_pubkey.clone(), self.current_identity_pubkey.clone(), fields_json, true,
                            schema_ver,
                        ],
                    )?;

                    Self::save_hlc(main_ts, &tx_db)?;
                    let mut op = Operation::CreateNote {
                        operation_id: Uuid::new_v4().to_string(),
                        timestamp: *main_ts,
                        device_id: self.device_id.clone(),
                        note_id: pending.note_id.clone(),
                        parent_id: Some(parent_id.to_string()),
                        position: position as f64,
                        schema: pending.schema.clone(),
                        title: effective_title.to_string(),
                        fields: effective_fields,
                        created_by: String::new(),
                        signature: String::new(),
                    };
                    Self::sign_op_with(&signing_key, &mut op);
                    Self::log_op(&self.operation_log, &tx_db, &op)?;
                } else {
                    // ── UPDATE existing note ─────────────────────────────────────
                    let effective_fields = pending.effective_fields();
                    let fields_json = serde_json::to_string(&effective_fields)?;
                    let effective_title = pending.effective_title();

                    tx_db.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, \
                                          modified_at = ?3, modified_by = ?4 \
                         WHERE id = ?5",
                        rusqlite::params![
                            effective_title, fields_json, now,
                            self.current_identity_pubkey.clone(), pending.note_id,
                        ],
                    )?;

                    Self::save_hlc(main_ts, &tx_db)?;
                    let mut title_op = Operation::UpdateNote {
                        operation_id: Uuid::new_v4().to_string(),
                        timestamp: *main_ts,
                        device_id: self.device_id.clone(),
                        note_id: pending.note_id.clone(),
                        title: effective_title.to_string(),
                        modified_by: String::new(),
                        signature: String::new(),
                    };
                    Self::sign_op_with(&signing_key, &mut title_op);
                    Self::log_op(&self.operation_log, &tx_db, &title_op)?;

                    for ((field_key, field_value), field_ts) in effective_fields.iter().zip(field_tss.iter()) {
                        Self::save_hlc(field_ts, &tx_db)?;
                        let mut field_op = Operation::UpdateField {
                            operation_id: Uuid::new_v4().to_string(),
                            timestamp: *field_ts,
                            device_id: self.device_id.clone(),
                            note_id: pending.note_id.clone(),
                            field: field_key.clone(),
                            value: field_value.clone(),
                            modified_by: String::new(),
                            signature: String::new(),
                        };
                        Self::sign_op_with(&signing_key, &mut field_op);
                        Self::log_op(&self.operation_log, &tx_db, &field_op)?;
                    }
                }
            }

            Self::purge_ops_if_needed(&self.operation_log, &tx_db)?;
            tx_db.commit()?;
        }

        // ── reorder path (unchanged) ───────────────────────────────────────────
        if let Some(ids) = result.reorder {
            for (position, id) in ids.iter().enumerate() {
                self.move_note(id, Some(note_id), position as f64)?;
            }
        }

        Ok(())
    }

    /// Returns a map of `note_type → [action_label, …]` from the script registry.
    pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
        self.script_registry.menu_action_map()
    }

    pub fn get_views_for_type(&self, schema_name: &str) -> Vec<crate::core::scripting::ViewRegistration> {
        self.script_registry.get_views_for_type(schema_name)
    }

    pub fn get_script_warnings(&self) -> Vec<crate::core::scripting::ScriptWarning> {
        self.script_registry.get_script_warnings()
    }

    /// Renders a specific registered view tab for a note.
    pub fn render_view(&self, note_id: &str, view_label: &str) -> Result<String> {
        let note = self.get_note(note_id)?;
        let context = self.build_query_context()?;

        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        self.script_registry.set_run_context(note.clone(), attachments);
        struct RunContextGuard<'a>(&'a crate::core::scripting::ScriptRegistry);
        impl Drop for RunContextGuard<'_> {
            fn drop(&mut self) { self.0.clear_run_context(); }
        }
        let _guard = RunContextGuard(&self.script_registry);

        self.script_registry
            .run_view(&note, view_label, context)
            .map(|html| self.embed_attachment_images(html))
    }

    /// Renders a single textarea field value as markdown HTML with attachment images embedded.
    pub fn render_markdown_field(&self, note_id: &str, text: &str) -> Result<String> {
        use crate::core::scripting::display_helpers;
        let note = self.get_note(note_id)?;
        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        let after_images = display_helpers::preprocess_image_blocks(text, &note.fields, &attachments);
        let preprocessed = display_helpers::preprocess_media_embeds(&after_images);
        let html = format!(
            "<div class=\"kn-view-markdown\">{}</div>",
            display_helpers::render_markdown_to_html(&preprocessed)
        );
        Ok(self.embed_attachment_images(html))
    }

}
