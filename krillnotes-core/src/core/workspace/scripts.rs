// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! User-script CRUD, reload, and operation-log queries.

use super::*;

impl Workspace {
    // ── User-script CRUD ──────────────────────────────────────────

    /// Returns all user scripts, ordered by `load_order` ascending.
    pub fn list_user_scripts(&self) -> Result<Vec<UserScript>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
             FROM user_scripts ORDER BY load_order ASC, created_at ASC",
        )?;
        let scripts = stmt
            .query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(scripts)
    }

    /// Returns a single user script by ID.
    pub fn get_user_script(&self, script_id: &str) -> Result<UserScript> {
        self.connection()
            .query_row(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
                 FROM user_scripts WHERE id = ?",
                [script_id],
                |row| {
                    Ok(UserScript {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        source_code: row.get(3)?,
                        load_order: row.get(4)?,
                        enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                        created_at: row.get(6)?,
                        modified_at: row.get(7)?,
                        category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                    })
                },
            )
            .map_err(|_| KrillnotesError::NoteNotFound(format!("User script {script_id} not found")))
    }

    /// Creates a new user script from its source code, parsing front matter for name/description.
    ///
    /// Returns an error if `@name` is missing from the front matter, or if Rhai
    /// compilation fails. On failure nothing is written to the database.
    pub fn create_user_script(&mut self, source_code: &str) -> Result<(UserScript, Vec<ScriptError>)> {
        // Auto-detect category: if the source calls schema(), it's a schema script.
        let category = if source_code.contains("schema(") { "schema" } else { "presentation" };
        self.create_user_script_with_category(source_code, category)
    }

    /// Creates a user script with an explicit category ("schema" or "presentation").
    pub fn create_user_script_with_category(
        &mut self,
        source_code: &str,
        category: &str,
    ) -> Result<(UserScript, Vec<ScriptError>)> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        let now = chrono::Utc::now().timestamp();
        let id = Uuid::new_v4().to_string();

        // Pre-validation
        self.script_registry.set_loading_category(Some(category.to_string()));
        if let Err(e) = self.script_registry.load_script(source_code, &fm.name) {
            let _ = self.reload_scripts();
            return Err(e);
        }

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        let max_order: i32 = tx
            .query_row("SELECT COALESCE(MAX(load_order), -1) FROM user_scripts", [], |row| row.get(0))
            .unwrap_or(-1);
        let load_order = max_order + 1;

        tx.execute(
            "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, fm.name, fm.description, source_code, load_order, true, now, now, category],
        )?;

        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::CreateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: id.clone(),
            name: fm.name.clone(),
            description: fm.description.clone(),
            source_code: source_code.to_string(),
            load_order,
            enabled: true,
            created_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        let op_id = op.operation_id().to_string();
        self.push_script_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::DeleteScript { script_id: id.clone() },
            propagate: true,
        });

        let errors = self.reload_scripts()?;
        let script = self.get_user_script(&id)?;
        Ok((script, errors))
    }

    /// Updates an existing user script's source code, re-parsing front matter.
    ///
    /// Returns an error if `@name` is missing from the front matter, or if Rhai
    /// compilation fails. On failure nothing is written to the database.
    pub fn update_user_script(&mut self, script_id: &str, source_code: &str) -> Result<(UserScript, Vec<ScriptError>)> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        // Pre-validation: try to compile and evaluate the new source code.
        // The collision check allows same-script re-registration, so updating a script that
        // already owns some schemas will not falsely fire a collision error.
        // We must set the loading category so schema scripts get library sources prepended.
        let existing_category = self
            .get_user_script(script_id)
            .map(|s| s.category)
            .unwrap_or_else(|_| "presentation".to_string());
        self.script_registry.set_loading_category(Some(existing_category));
        if let Err(e) = self.script_registry.load_script(source_code, &fm.name) {
            let _ = self.reload_scripts(); // restore registry; ignore restoration errors
            return Err(e);
        }

        // Capture old script state BEFORE the update for undo.
        let old_script = self.get_user_script(script_id)?;

        let now = chrono::Utc::now().timestamp();
        // Advance HLC and capture signing key before the transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        let changes = tx.execute(
            "UPDATE user_scripts SET name = ?, description = ?, source_code = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![fm.name, fm.description, source_code, now, script_id],
        )?;

        if changes == 0 {
            return Err(KrillnotesError::NoteNotFound(format!("User script {script_id} not found")));
        }

        // Read current full state for the operation log
        let (load_order, enabled): (i32, bool) = tx.query_row(
            "SELECT load_order, enabled FROM user_scripts WHERE id = ?",
            [script_id],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1).map(|v| v != 0)?)),
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name: fm.name.clone(),
            description: fm.description.clone(),
            source_code: source_code.to_string(),
            load_order,
            enabled,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        // Push onto the script-specific undo stack (isolated from note undo).
        let op_id = op.operation_id().to_string();
        self.push_script_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::ScriptRestore {
                script_id: script_id.to_string(),
                name: old_script.name,
                description: old_script.description,
                source_code: old_script.source_code,
                load_order: old_script.load_order,
                enabled: old_script.enabled,
            },
            propagate: true,
        });

        let errors = self.reload_scripts()?;
        let script = self.get_user_script(script_id)?;
        Ok((script, errors))
    }

    /// Deletes a user script by ID and reloads remaining scripts.
    pub fn delete_user_script(&mut self, script_id: &str) -> Result<Vec<ScriptError>> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        // Capture old script state BEFORE deletion for undo.
        let old_script = self.get_user_script(script_id)?;

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute("DELETE FROM user_scripts WHERE id = ?", [script_id])?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::DeleteUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            deleted_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        // Push onto the script-specific undo stack (isolated from note undo).
        let op_id = op.operation_id().to_string();
        self.push_script_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::ScriptRestore {
                script_id: script_id.to_string(),
                name: old_script.name,
                description: old_script.description,
                source_code: old_script.source_code,
                load_order: old_script.load_order,
                enabled: old_script.enabled,
            },
            propagate: true,
        });

        self.reload_scripts()
    }

    /// Toggles the enabled state of a user script and reloads.
    pub fn toggle_user_script(&mut self, script_id: &str, enabled: bool) -> Result<Vec<ScriptError>> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE user_scripts SET enabled = ? WHERE id = ?",
            rusqlite::params![enabled, script_id],
        )?;

        // Read full current state for the operation log
        let (name, description, source_code, load_order): (String, String, String, i32) = tx.query_row(
            "SELECT name, description, source_code, load_order FROM user_scripts WHERE id = ?",
            [script_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name,
            description,
            source_code,
            load_order,
            enabled,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Changes the load order of a user script and reloads.
    pub fn reorder_user_script(&mut self, script_id: &str, new_load_order: i32) -> Result<Vec<ScriptError>> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE user_scripts SET load_order = ? WHERE id = ?",
            rusqlite::params![new_load_order, script_id],
        )?;

        // Read full current state for the operation log
        let (name, description, source_code, enabled): (String, String, String, bool) = tx.query_row(
            "SELECT name, description, source_code, enabled FROM user_scripts WHERE id = ?",
            [script_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3).map(|v| v != 0)?)),
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name,
            description,
            source_code,
            load_order: new_load_order,
            enabled,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Re-assigns sequential load_order (0-based) to all scripts given in `ids` order, then reloads.
    pub fn reorder_all_user_scripts(&mut self, ids: &[String]) -> Result<Vec<ScriptError>> {
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
        // Bulk reorder is not logged to the operation log — it's a UI ordering gesture, not a sync-relevant change.
        {
            let conn = self.storage.connection_mut();
            let tx = conn.transaction()?;
            for (i, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE user_scripts SET load_order = ? WHERE id = ?",
                    rusqlite::params![i as i32, id],
                )?;
            }
            tx.commit()?;
        }
        self.reload_scripts()
    }

    // ── Operations log queries ───────────────────────────────────────

    /// Returns operation summaries matching the given filters, newest first.
    pub fn list_operations(
        &self,
        type_filter: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<crate::OperationSummary>> {
        self.operation_log.list(self.connection(), type_filter, since, until)
    }

    /// Returns the full JSON detail for a single operation by ID.
    pub fn get_operation_detail(&self, operation_id: &str) -> Result<serde_json::Value> {
        self.operation_log.get_detail(self.connection(), operation_id)
    }

    /// Deletes all operations from the log. Returns the number deleted.
    pub fn purge_all_operations(&self) -> Result<usize> {
        self.operation_log.purge_all(self.connection())
    }

    /// Clears all registered schemas/hooks and re-executes enabled scripts from the DB in order.
    ///
    /// Returns any errors that occurred during loading (e.g. schema collisions, Rhai errors).
    /// A failing script is skipped; subsequent scripts continue to load.
    pub(crate) fn reload_scripts(&mut self) -> Result<Vec<ScriptError>> {
        self.script_registry.clear_all();
        let scripts = self.list_user_scripts()?;
        Ok(self.load_scripts_two_phase(&scripts))
    }

    /// Two-phase script loading: presentation/library first, then schema, then resolve bindings.
    fn load_scripts_two_phase(&mut self, scripts: &[UserScript]) -> Vec<ScriptError> {
        let mut errors = Vec::new();

        // Phase A: load presentation/library scripts first
        for script in scripts.iter().filter(|s| s.enabled && s.category == "presentation") {
            self.script_registry.set_loading_category(Some("presentation".to_string()));
            if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
                errors.push(ScriptError {
                    script_name: script.name.clone(),
                    message: e.to_string(),
                });
            }
        }

        // Phase B: load schema scripts
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            self.script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
                errors.push(ScriptError {
                    script_name: script.name.clone(),
                    message: e.to_string(),
                });
            }
        }

        // Phase C: resolve deferred bindings (views, hovers, menus)
        self.script_registry.resolve_bindings();

        errors
    }

    /// Public wrapper for reloading all scripts (e.g. from tests).
    pub fn reload_all_scripts(&mut self) -> Result<Vec<ScriptError>> {
        self.reload_scripts()
    }

}
