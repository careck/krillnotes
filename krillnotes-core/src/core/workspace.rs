//! High-level workspace operations over a Krillnotes SQLite database.

use crate::core::user_script;
use crate::{
    get_device_id, DeleteResult, DeleteStrategy, FieldValue, KrillnotesError, Note,
    Operation, OperationLog, PurgeStrategy, QueryContext, Result, ScriptError, ScriptRegistry,
    Storage, UserScript,
};
use rhai::Dynamic;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// Controls where a new note is inserted relative to the currently selected note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddPosition {
    /// Insert as the first child of the selected note.
    AsChild,
    /// Insert immediately after the selected note within the same parent.
    AsSibling,
}

/// An open Krillnotes workspace backed by a SQLite database.
///
/// `Workspace` is the primary interface for all document mutations. It combines
/// a [`Storage`] connection, a [`ScriptRegistry`] for note-type validation and hooks,
/// and an [`OperationLog`] for durable change history.
///
/// Each instance is bound to a single window and protected by a `Mutex` in
/// the desktop application's state.
pub struct Workspace {
    storage: Storage,
    script_registry: ScriptRegistry,
    operation_log: OperationLog,
    device_id: String,
    current_user_id: i64,
}

impl Workspace {
    /// Creates a new workspace database at `path`, initialises the schema, and inserts
    /// a root note named after the file (e.g. `"My Notes"` for `my-notes.krillnotes`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::InvalidWorkspace`] if the device ID cannot be obtained.
    pub fn create<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 1000 });

        // Get hardware-based device ID
        let device_id = get_device_id()?;

        // Store metadata
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["device_id", &device_id],
        )?;
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["current_user_id", "0"],
        )?;

        // Seed the workspace with bundled starter scripts.
        let now = chrono::Utc::now().timestamp();
        let starters = ScriptRegistry::starter_scripts();
        {
            let tx = storage.connection_mut().transaction()?;
            for (load_order, starter) in starters.iter().enumerate() {
                let fm = user_script::parse_front_matter(&starter.source_code);
                let id = Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![id, fm.name, fm.description, &starter.source_code, load_order as i32, true, now, now],
                )?;
            }
            tx.commit()?;
        }

        // Load all scripts from the DB into the registry.
        let scripts = {
            let mut stmt = storage.connection().prepare(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at
                 FROM user_scripts ORDER BY load_order ASC, created_at ASC",
            )?;
            let results: Vec<UserScript> = stmt.query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        for script in scripts.iter().filter(|s| s.enabled) {
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load starter script '{}': {}", script.name, e);
            }
        }

        // Create root note from filename
        let filename = path
            .as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let title = humanize(filename);

        let root = Note {
            id: Uuid::new_v4().to_string(),
            title,
            node_type: "TextNote".to_string(),
            parent_id: None,
            position: 0,
            created_at: now,
            modified_at: now,
            created_by: 0,
            modified_by: 0,
            fields: script_registry.get_schema("TextNote")?.default_fields(),
            is_expanded: true,
            tags: vec![],
        };

        let tx = storage.connection_mut().transaction()?;
        tx.execute(
            "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                root.id,
                root.title,
                root.node_type,
                root.parent_id,
                root.position,
                root.created_at,
                root.modified_at,
                root.created_by,
                root.modified_by,
                serde_json::to_string(&root.fields)?,
                true,
            ],
        )?;
        tx.commit()?;

        Ok(Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            current_user_id: 0,
        })
    }

    /// Opens an existing workspace database at `path` and reads stored metadata.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::WrongPassword`] if the password is
    /// incorrect, [`crate::KrillnotesError::UnencryptedWorkspace`] if the file
    /// is a plain unencrypted SQLite database, or
    /// [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn open<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
        let storage = Storage::open(&path, password)?;
        let script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 1000 });

        // Read metadata from database
        let device_id = storage.connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'device_id'",
                [],
                |row| row.get::<_, String>(0)
            )?;

        let current_user_id = storage.connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'current_user_id'",
                [],
                |row| row.get::<_, String>(0)
            )?
            .parse::<i64>()
            .unwrap_or(0);

        let mut ws = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            current_user_id,
        };

        // Load enabled scripts from the workspace DB.
        let scripts = ws.list_user_scripts()?;
        for script in scripts.iter().filter(|s| s.enabled) {
            if let Err(e) = ws.script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load script '{}': {}", script.name, e);
            }
        }

        Ok(ws)
    }

    /// Returns a reference to the script registry for this workspace.
    pub fn script_registry(&self) -> &ScriptRegistry {
        &self.script_registry
    }

    /// Returns a mutable reference to the script registry for this workspace.
    #[cfg(test)]
    pub(crate) fn script_registry_mut(&mut self) -> &mut ScriptRegistry {
        &mut self.script_registry
    }

    /// Returns the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        self.storage.connection()
    }

    /// Fetches a single note by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// if `fields_json` cannot be deserialised.
    pub fn get_note(&self, note_id: &str) -> Result<Note> {
        let row = self.connection().query_row(
            "SELECT n.id, n.title, n.node_type, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded,
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
            AddPosition::AsChild => (Some(selected.id.clone()), 0),
            AddPosition::AsSibling => (selected.parent_id.clone(), selected.position + 1),
        };

        // Validate allowed_parent_types
        if !schema.allowed_parent_types.is_empty() {
            match &final_parent {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", note_type
                ))),
                Some(pid) => {
                    let parent_note = self.get_note(pid)?;
                    if !schema.allowed_parent_types.contains(&parent_note.node_type) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            note_type, parent_note.node_type
                        )));
                    }
                }
            }
        }

        // Validate allowed_children_types on the parent schema
        if let Some(pid) = &final_parent {
            let parent_note = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent_note.node_type)?;
            if !parent_schema.allowed_children_types.is_empty()
                && !parent_schema.allowed_children_types.contains(&note_type.to_string())
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    note_type, parent_note.node_type
                )));
            }
        }

        // Fetch parent note before opening the transaction (avoids borrow conflict with `tx`).
        let hook_parent = if let Some(ref pid) = final_parent {
            Some(self.get_note(pid)?)
        } else {
            None
        };

        let mut note = Note {
            id: Uuid::new_v4().to_string(),
            title: "Untitled".to_string(),
            node_type: note_type.to_string(),
            parent_id: final_parent,
            position: final_position,
            created_at: chrono::Utc::now().timestamp(),
            modified_at: chrono::Utc::now().timestamp(),
            created_by: self.current_user_id,
            modified_by: self.current_user_id,
            fields: schema.default_fields(),
            is_expanded: true,
            tags: vec![],
        };

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
            "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                note.id,
                note.title,
                note.node_type,
                note.parent_id,
                note.position,
                note.created_at,
                note.modified_at,
                note.created_by,
                note.modified_by,
                serde_json::to_string(&note.fields)?,
                true,
            ],
        )?;

        // Run on_add_child hook if the parent's schema defines one.
        // Allowed-parent and allowed-children checks have already passed above.
        if let Some(ref parent_note) = hook_parent {
            if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
                &parent_note.node_type,
                &parent_note.id, &parent_note.node_type, &parent_note.title, &parent_note.fields,
                &note.id, &note.node_type, &note.title, &note.fields,
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
        let op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: note.created_at,
            device_id: self.device_id.clone(),
            note_id: note.id.clone(),
            parent_id: note.parent_id.clone(),
            position: note.position,
            node_type: note.node_type.clone(),
            title: note.title.clone(),
            fields: note.fields.clone(),
            created_by: note.created_by,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        Ok(note.id)
    }

    /// Deep-copies the note at `source_id` and its entire descendant subtree,
    /// placing the copy at `target_id` with the given `position`.
    ///
    /// Returns the ID of the new root note.
    ///
    /// All notes in the subtree receive fresh UUIDs and current timestamps.
    /// Schema constraints (`allowed_parent_types`, `allowed_children_types`) are
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
        let root_schema = self.script_registry.get_schema(&root_source.node_type)?;
        let target_note = self.get_note(target_id)?;

        let (new_parent_id, new_position) = match position {
            AddPosition::AsChild => (Some(target_note.id.clone()), 0i32),
            AddPosition::AsSibling => (target_note.parent_id.clone(), target_note.position + 1),
        };

        // Validate allowed_parent_types for the root copy
        if !root_schema.allowed_parent_types.is_empty() {
            match &new_parent_id {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", root_source.node_type
                ))),
                Some(pid) => {
                    let parent = self.get_note(pid)?;
                    if !root_schema.allowed_parent_types.contains(&parent.node_type) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            root_source.node_type, parent.node_type
                        )));
                    }
                }
            }
        }

        // Validate allowed_children_types on the paste parent
        if let Some(pid) = &new_parent_id {
            let parent = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent.node_type)?;
            if !parent_schema.allowed_children_types.is_empty()
                && !parent_schema.allowed_children_types.contains(&root_source.node_type)
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    root_source.node_type, parent.node_type
                )));
            }
        }

        // 3. Build old_id → new_id remap table.
        let mut id_map: HashMap<String, String> = HashMap::new();
        for note in &subtree {
            id_map.insert(note.id.clone(), Uuid::new_v4().to_string());
        }

        let now = chrono::Utc::now().timestamp();

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

        for note in &subtree {
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
                "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    new_id,
                    note.title,
                    note.node_type,
                    new_parent,
                    this_position,
                    now,
                    now,
                    self.current_user_id,
                    self.current_user_id,
                    serde_json::to_string(&note.fields)?,
                    note.is_expanded,
                ],
            )?;

            // Log a CreateNote operation for each inserted note.
            let op = Operation::CreateNote {
                operation_id: Uuid::new_v4().to_string(),
                timestamp: now,
                device_id: self.device_id.clone(),
                note_id: new_id.clone(),
                parent_id: new_parent,
                position: this_position,
                node_type: note.node_type.clone(),
                title: note.title.clone(),
                fields: note.fields.clone(),
                created_by: self.current_user_id,
            };
            self.operation_log.log(&tx, &op)?;
        }

        self.operation_log.purge_if_needed(&tx)?;
        tx.commit()?;

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

        // Validate allowed_parent_types — root notes have no parent
        if !schema.allowed_parent_types.is_empty() {
            return Err(KrillnotesError::InvalidMove(format!(
                "Note type '{}' cannot be placed at root level", node_type
            )));
        }

        let new_note = Note {
            id: Uuid::new_v4().to_string(),
            title: "Untitled".to_string(),
            node_type: node_type.to_string(),
            parent_id: None,
            position: 0,
            created_at: now,
            modified_at: now,
            created_by: self.current_user_id,
            modified_by: self.current_user_id,
            fields: schema.default_fields(),
            is_expanded: true,
            tags: vec![],
        };

        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                new_note.id,
                new_note.title,
                new_note.node_type,
                new_note.parent_id,
                new_note.position,
                new_note.created_at,
                new_note.modified_at,
                new_note.created_by,
                new_note.modified_by,
                serde_json::to_string(&new_note.fields)?,
                true,
            ],
        )?;

        // Log operation
        let op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: new_note.created_at,
            device_id: self.device_id.clone(),
            note_id: new_note.id.clone(),
            parent_id: new_note.parent_id.clone(),
            position: new_note.position,
            node_type: new_note.node_type.clone(),
            title: new_note.title.clone(),
            fields: new_note.fields.clone(),
            created_by: new_note.created_by,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;
        Ok(new_note.id)
    }

    /// Updates the title of `note_id` and logs an `UpdateField` operation.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// the UPDATE fails.
    pub fn update_note_title(&mut self, note_id: &str, new_title: String) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE notes SET title = ?, modified_at = ?, modified_by = ? WHERE id = ?",
            rusqlite::params![new_title, now, self.current_user_id, note_id],
        )?;

        // Log operation
        let op = Operation::UpdateField {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            field: "title".to_string(),
            value: crate::FieldValue::Text(new_title),
            modified_by: self.current_user_id,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

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

        let tx = self.storage.connection_mut().transaction()?;
        tx.execute("DELETE FROM note_tags WHERE note_id = ?", [note_id])?;
        for tag in &normalised {
            tx.execute(
                "INSERT INTO note_tags (note_id, tag) VALUES (?, ?)",
                rusqlite::params![note_id, tag],
            )?;
        }
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
            "SELECT n.id, n.title, n.node_type, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded,
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
        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Returns all notes in the workspace, ordered by `parent_id` then `position`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::Json`] if any row's `fields_json` is corrupt.
    pub fn list_all_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.connection().prepare(
            "SELECT n.id, n.title, n.node_type, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             GROUP BY n.id
             ORDER BY n.parent_id, n.position",
        )?;

        let rows = stmt
            .query_map([], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Runs the `on_view` hook for the note's schema, falling back to a default
    /// HTML view when no hook is registered.
    ///
    /// The default view auto-renders `textarea` fields as CommonMark markdown.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Database`] if the note or any workspace note
    /// cannot be fetched, or [`KrillnotesError::Scripting`] if the hook fails.
    pub fn run_view_hook(&self, note_id: &str) -> Result<String> {
        let note = self.get_note(note_id)?;

        // No hook registered: generate the default view without fetching all notes.
        if !self.script_registry.has_view_hook(&note.node_type) {
            return Ok(self.script_registry.render_default_view(&note));
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

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.node_type.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
        }

        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag };
        // run_on_view_hook returns Some(...) since we've confirmed a hook exists above.
        Ok(self
            .script_registry
            .run_on_view_hook(&note, context)?
            .unwrap_or_default())
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
        let note = self.get_note(note_id)?;
        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: HashMap<String, Dynamic> = HashMap::new();
        let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_type: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_tag: HashMap<String, Vec<Dynamic>> = HashMap::new();
        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.node_type.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag };

        // invoke_tree_action_hook returns an error if the script throws — in that case
        // we propagate the error without touching the DB (implicit rollback).
        let result = self.script_registry.invoke_tree_action_hook(label, &note, context)?;

        // Apply creates and updates atomically if any were queued.
        if !result.creates.is_empty() || !result.updates.is_empty() {
            let now = chrono::Utc::now().timestamp();
            let tx = self.storage.connection_mut().transaction()?;

            // ── creates ────────────────────────────────────────────────────────
            for create in &result.creates {
                // Compute the next available position under the parent.
                let position: i32 = tx.query_row(
                    "SELECT COALESCE(MAX(position), -1) + 1 FROM notes WHERE parent_id = ?1",
                    rusqlite::params![create.parent_id],
                    |row| row.get(0),
                )?;

                let fields_json = serde_json::to_string(&create.fields)?;

                tx.execute(
                    "INSERT INTO notes (id, title, node_type, parent_id, position, \
                                        created_at, modified_at, created_by, modified_by, \
                                        fields_json, is_expanded) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        create.id,
                        create.title,
                        create.node_type,
                        create.parent_id,
                        position,
                        now,
                        now,
                        self.current_user_id,
                        self.current_user_id,
                        fields_json,
                        true,
                    ],
                )?;

                let op = Operation::CreateNote {
                    operation_id: Uuid::new_v4().to_string(),
                    timestamp: now,
                    device_id: self.device_id.clone(),
                    note_id: create.id.clone(),
                    parent_id: Some(create.parent_id.clone()),
                    position,
                    node_type: create.node_type.clone(),
                    title: create.title.clone(),
                    fields: create.fields.clone(),
                    created_by: self.current_user_id,
                };
                self.operation_log.log(&tx, &op)?;
            }

            // ── updates ────────────────────────────────────────────────────────
            for update in &result.updates {
                let fields_json = serde_json::to_string(&update.fields)?;

                tx.execute(
                    "UPDATE notes SET title = ?1, fields_json = ?2, \
                                      modified_at = ?3, modified_by = ?4 \
                     WHERE id = ?5",
                    rusqlite::params![
                        update.title,
                        fields_json,
                        now,
                        self.current_user_id,
                        update.note_id,
                    ],
                )?;

                // Log title update
                let title_op = Operation::UpdateField {
                    operation_id: Uuid::new_v4().to_string(),
                    timestamp: now,
                    device_id: self.device_id.clone(),
                    note_id: update.note_id.clone(),
                    field: "title".to_string(),
                    value: crate::FieldValue::Text(update.title.clone()),
                    modified_by: self.current_user_id,
                };
                self.operation_log.log(&tx, &title_op)?;

                // Log one UpdateField per field value
                for (field_key, field_value) in &update.fields {
                    let field_op = Operation::UpdateField {
                        operation_id: Uuid::new_v4().to_string(),
                        timestamp: now,
                        device_id: self.device_id.clone(),
                        note_id: update.note_id.clone(),
                        field: field_key.clone(),
                        value: field_value.clone(),
                        modified_by: self.current_user_id,
                    };
                    self.operation_log.log(&tx, &field_op)?;
                }
            }

            self.operation_log.purge_if_needed(&tx)?;
            tx.commit()?;
        }

        // ── reorder path (unchanged) ───────────────────────────────────────────
        if let Some(ids) = result.reorder {
            for (position, id) in ids.iter().enumerate() {
                self.move_note(id, Some(note_id), position as i32)?;
            }
        }

        Ok(())
    }

    /// Returns a map of `note_type → [action_label, …]` from the script registry.
    pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
        self.script_registry.tree_action_map()
    }

    // Note: toggle_note_expansion and set_selected_note intentionally do NOT write to the
    // operation log. These are transient UI state (not document mutations) and should not
    // participate in sync or undo. They are stored in workspace_meta / the notes table but
    // treated as per-device view state, not collaborative operations.
    /// Toggles the `is_expanded` flag of `note_id` in the database.
    ///
    /// This is a UI-state mutation and is intentionally excluded from the
    /// operation log — expansion state is per-device and should not sync.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found.
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
        new_position: i32,
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

        // 3. Allowed-parent-types check
        let note_to_move = self.get_note(note_id)?;
        let schema = self.script_registry.get_schema(&note_to_move.node_type)?;
        if !schema.allowed_parent_types.is_empty() {
            match new_parent_id {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", note_to_move.node_type
                ))),
                Some(pid) => {
                    let parent_note = self.get_note(pid)?;
                    if !schema.allowed_parent_types.contains(&parent_note.node_type) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            note_to_move.node_type, parent_note.node_type
                        )));
                    }
                }
            }
        }

        // 3b. Allowed-children-types check on the new parent
        if let Some(pid) = new_parent_id {
            let parent_note = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent_note.node_type)?;
            if !parent_schema.allowed_children_types.is_empty()
                && !parent_schema.allowed_children_types.contains(&note_to_move.node_type)
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    note_to_move.node_type, parent_note.node_type
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

        let now = chrono::Utc::now().timestamp();
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
                &parent_note.node_type,
                &parent_note.id, &parent_note.node_type, &parent_note.title, &parent_note.fields,
                &note_to_move.id, &note_to_move.node_type, &note_to_move.title, &note_to_move.fields,
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
        let op = Operation::MoveNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            new_parent_id: new_parent_id.map(|s| s.to_string()),
            new_position,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        // 9. Commit
        tx.commit()?;

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
        let mut stmt = self.connection().prepare(
            "SELECT n.id, n.title, n.node_type, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded,
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
        let tx = self.storage.connection_mut().transaction()?;
        let result = Self::delete_recursive_in_tx(&tx, note_id)?;
        tx.commit()?;
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
    fn delete_recursive_in_tx(
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

        // Delete the note itself after its children have been safely re-parented.
        tx.execute(
            "DELETE FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
        )?;

        tx.commit()?;

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
    /// Returns [`crate::KrillnotesError::NoteNotFound`] if no note with `note_id`
    /// exists in the database.  Returns [`crate::KrillnotesError::Json`] if
    /// `fields` cannot be serialised to JSON.  Returns
    /// [`crate::KrillnotesError::Database`] for any other SQLite failure.
    pub fn update_note(
        &mut self,
        note_id: &str,
        title: String,
        fields: HashMap<String, FieldValue>,
    ) -> Result<Note> {
        // Look up this note's schema so the pre-save hook can be dispatched.
        let node_type: String = self
            .storage
            .connection()
            .query_row(
                "SELECT node_type FROM notes WHERE id = ?1",
                rusqlite::params![note_id],
                |row| row.get(0),
            )
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Run the pre-save hook. If a hook is registered it may modify title and fields.
        let (title, fields) =
            match self
                .script_registry
                .run_on_save_hook(&node_type, note_id, &node_type, &title, &fields)?
            {
                Some((new_title, new_fields)) => (new_title, new_fields),
                None => (title, fields),
            };

        // Enforce required-field constraints defined in the schema.
        let schema = self.script_registry.get_schema(&node_type)?;
        schema.validate_required_fields(&fields)?;

        let now = chrono::Utc::now().timestamp();
        let fields_json = serde_json::to_string(&fields)?;

        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3, modified_by = ?4 WHERE id = ?5",
            rusqlite::params![title, fields_json, now, self.current_user_id, note_id],
        )?;

        // Detect nonexistent IDs: SQLite UPDATE on a missing row succeeds but
        // touches zero rows. Surface this as NoteNotFound rather than silently
        // returning stale data.
        if tx.changes() == 0 {
            return Err(KrillnotesError::NoteNotFound(note_id.to_string()));
        }

        // Log an UpdateField operation for the title, consistent with
        // update_note_title.
        let title_op = Operation::UpdateField {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            field: "title".to_string(),
            value: crate::FieldValue::Text(title.clone()),
            modified_by: self.current_user_id,
        };
        self.operation_log.log(&tx, &title_op)?;

        // Log one UpdateField operation per field value that was written.
        for (field_key, field_value) in &fields {
            let field_op = Operation::UpdateField {
                operation_id: Uuid::new_v4().to_string(),
                timestamp: now,
                device_id: self.device_id.clone(),
                note_id: note_id.to_string(),
                field: field_key.clone(),
                value: field_value.clone(),
                modified_by: self.current_user_id,
            };
            self.operation_log.log(&tx, &field_op)?;
        }

        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        // Re-use get_note to fetch the persisted row, keeping row-mapping logic
        // in a single place.
        self.get_note(note_id)
    }

    // ── User-script CRUD ──────────────────────────────────────────

    /// Returns all user scripts, ordered by `load_order` ascending.
    pub fn list_user_scripts(&self) -> Result<Vec<UserScript>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at
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
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(scripts)
    }

    /// Returns a single user script by ID.
    pub fn get_user_script(&self, script_id: &str) -> Result<UserScript> {
        self.connection()
            .query_row(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at
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
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        let now = chrono::Utc::now().timestamp();
        let id = Uuid::new_v4().to_string();

        // Pre-validation: try to load the script against the live registry.
        // Catches syntax errors and schema collisions before writing to the DB.
        if let Err(e) = self.script_registry.load_script(source_code, &fm.name) {
            // Restore the registry to its pre-validation state; ignore restoration errors.
            let _ = self.reload_scripts();
            return Err(e);
        }

        let tx = self.storage.connection_mut().transaction()?;

        // Determine next load_order
        let max_order: i32 = tx
            .query_row("SELECT COALESCE(MAX(load_order), -1) FROM user_scripts", [], |row| row.get(0))
            .unwrap_or(-1);
        let load_order = max_order + 1;

        tx.execute(
            "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, fm.name, fm.description, source_code, load_order, true, now, now],
        )?;

        // Log operation
        let op = Operation::CreateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            script_id: id.clone(),
            name: fm.name.clone(),
            description: fm.description.clone(),
            source_code: source_code.to_string(),
            load_order,
            enabled: true,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        // Full reload to ensure deterministic ordering and collect any load errors.
        let errors = self.reload_scripts()?;
        let script = self.get_user_script(&id)?;
        Ok((script, errors))
    }

    /// Updates an existing user script's source code, re-parsing front matter.
    ///
    /// Returns an error if `@name` is missing from the front matter, or if Rhai
    /// compilation fails. On failure nothing is written to the database.
    pub fn update_user_script(&mut self, script_id: &str, source_code: &str) -> Result<(UserScript, Vec<ScriptError>)> {
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        // Pre-validation: try to compile and evaluate the new source code.
        // The collision check allows same-script re-registration, so updating a script that
        // already owns some schemas will not falsely fire a collision error.
        if let Err(e) = self.script_registry.load_script(source_code, &fm.name) {
            let _ = self.reload_scripts(); // restore registry; ignore restoration errors
            return Err(e);
        }

        let now = chrono::Utc::now().timestamp();
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
        let op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name: fm.name.clone(),
            description: fm.description.clone(),
            source_code: source_code.to_string(),
            load_order,
            enabled,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        let errors = self.reload_scripts()?;
        let script = self.get_user_script(script_id)?;
        Ok((script, errors))
    }

    /// Deletes a user script by ID and reloads remaining scripts.
    pub fn delete_user_script(&mut self, script_id: &str) -> Result<Vec<ScriptError>> {
        let now = chrono::Utc::now().timestamp();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute("DELETE FROM user_scripts WHERE id = ?", [script_id])?;

        // Log operation
        let op = Operation::DeleteUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Toggles the enabled state of a user script and reloads.
    pub fn toggle_user_script(&mut self, script_id: &str, enabled: bool) -> Result<Vec<ScriptError>> {
        let now = chrono::Utc::now().timestamp();
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
        let op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name,
            description,
            source_code,
            load_order,
            enabled,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Changes the load order of a user script and reloads.
    pub fn reorder_user_script(&mut self, script_id: &str, new_load_order: i32) -> Result<Vec<ScriptError>> {
        let now = chrono::Utc::now().timestamp();
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
        let op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name,
            description,
            source_code,
            load_order: new_load_order,
            enabled,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Re-assigns sequential load_order (0-based) to all scripts given in `ids` order, then reloads.
    pub fn reorder_all_user_scripts(&mut self, ids: &[String]) -> Result<Vec<ScriptError>> {
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

    /// Deletes all operations from the log. Returns the number deleted.
    pub fn purge_all_operations(&self) -> Result<usize> {
        self.operation_log.purge_all(self.connection())
    }

    /// Clears all registered schemas/hooks and re-executes enabled scripts from the DB in order.
    ///
    /// Returns any errors that occurred during loading (e.g. schema collisions, Rhai errors).
    /// A failing script is skipped; subsequent scripts continue to load.
    fn reload_scripts(&mut self) -> Result<Vec<ScriptError>> {
        self.script_registry.clear_all();
        let scripts = self.list_user_scripts()?;
        let mut errors = Vec::new();
        for script in scripts.iter().filter(|s| s.enabled) {
            if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
                errors.push(ScriptError {
                    script_name: script.name.clone(),
                    message: e.to_string(),
                });
            }
        }
        Ok(errors)
    }
}

/// Raw 12-column tuple extracted from a `notes` + `note_tags` SQLite row.
type NoteRow = (String, String, String, Option<String>, i64, i64, i64, i64, i64, String, i64, Option<String>);

/// Row-mapping closure for `rusqlite::Row` → raw tuple.
///
/// Returns the 12-column tuple that `note_from_row_tuple` converts into a `Note`.
/// Extracted to avoid duplicating column-index logic across every query.
fn map_note_row(row: &rusqlite::Row) -> rusqlite::Result<NoteRow> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, Option<String>>(3)?,
        row.get::<_, i64>(4)?,
        row.get::<_, i64>(5)?,
        row.get::<_, i64>(6)?,
        row.get::<_, i64>(7)?,
        row.get::<_, i64>(8)?,
        row.get::<_, String>(9)?,
        row.get::<_, i64>(10)?,
        row.get::<_, Option<String>>(11)?,
    ))
}

/// Converts a raw 12-column tuple into a [`Note`], parsing `fields_json` and `tags_csv`.
fn note_from_row_tuple(
    (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded_int, tags_csv): NoteRow,
) -> Result<Note> {
    let mut tags: Vec<String> = tags_csv
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    tags.sort();
    Ok(Note {
        id,
        title,
        node_type,
        parent_id,
        position: position as i32,
        created_at,
        modified_at,
        created_by,
        modified_by,
        fields: serde_json::from_str(&fields_json)?,
        is_expanded: is_expanded_int == 1,
        tags,
    })
}

/// Converts a [`Note`] into a Rhai `Dynamic` map for use in `on_view` query functions.
///
/// Produces the same `{ id, node_type, title, fields }` shape as the map passed to
/// `on_save` hooks, so scripts can use a consistent note representation.
fn note_to_rhai_dynamic(note: &Note) -> Dynamic {
    use crate::core::scripting::field_value_to_dynamic;
    let mut fields_map = rhai::Map::new();
    for (k, v) in &note.fields {
        fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
    }
    let tags_array: rhai::Array = note.tags.iter()
        .map(|t| Dynamic::from(t.clone()))
        .collect();
    let mut note_map = rhai::Map::new();
    note_map.insert("id".into(), Dynamic::from(note.id.clone()));
    note_map.insert("node_type".into(), Dynamic::from(note.node_type.clone()));
    note_map.insert("title".into(), Dynamic::from(note.title.clone()));
    note_map.insert("fields".into(), Dynamic::from(fields_map));
    note_map.insert("tags".into(), Dynamic::from(tags_array));
    Dynamic::from(note_map)
}

fn humanize(filename: &str) -> String {
    filename
        .replace(['-', '_'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FieldValue;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_workspace() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();

        // Verify root note exists
        let count: i64 = ws
            .connection()
            .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn test_humanize() {
        assert_eq!(humanize("my-project"), "My Project");
        assert_eq!(humanize("hello_world"), "Hello World");
        assert_eq!(humanize("test-case-123"), "Test Case 123");
    }

    #[test]
    fn test_create_and_get_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        let child = ws.get_note(&child_id).unwrap();
        assert_eq!(child.title, "Untitled");
        assert_eq!(child.parent_id, Some(root.id));
    }

    #[test]
    fn test_update_note_title() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "New Title".to_string())
            .unwrap();

        let updated = ws.get_note(&root.id).unwrap();
        assert_eq!(updated.title, "New Title");
    }

    #[test]
    fn test_open_existing_workspace() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace first
        {
            let ws = Workspace::create(temp.path(), "").unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            assert_eq!(root.node_type, "TextNote");
        }

        // Open it
        let ws = Workspace::open(temp.path(), "").unwrap();

        // Verify we can read notes
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].node_type, "TextNote");
    }

    #[test]
    fn test_is_expanded_defaults_to_true() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Check root note is expanded by default
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.is_expanded, "Root note should be expanded by default");

        // Create a child note and verify it's expanded by default
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        let child = ws.get_note(&child_id).unwrap();
        assert!(child.is_expanded, "New child note should be expanded by default");
    }

    #[test]
    fn test_is_expanded_persists_across_open() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace with notes
        {
            let mut ws = Workspace::create(temp.path(), "").unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                .unwrap();
        }

        // Open and verify is_expanded is true
        let ws = Workspace::open(temp.path(), "").unwrap();
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes[0].is_expanded, "Root note should be expanded");
        assert!(notes[1].is_expanded, "Child note should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.is_expanded, "Root should start expanded");

        // Toggle to collapsed
        ws.toggle_note_expansion(&root.id).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert!(!note.is_expanded, "Root should now be collapsed");

        // Toggle back to expanded
        ws.toggle_note_expansion(&root.id).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert!(note.is_expanded, "Root should be expanded again");
    }

    #[test]
    fn test_toggle_note_expansion_with_child_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Toggle child note
        ws.toggle_note_expansion(&child_id).unwrap();
        let child = ws.get_note(&child_id).unwrap();
        assert!(!child.is_expanded, "Child should be collapsed");

        // Toggle back
        ws.toggle_note_expansion(&child_id).unwrap();
        let child = ws.get_note(&child_id).unwrap();
        assert!(child.is_expanded, "Child should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion_nonexistent_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Try to toggle a note that doesn't exist
        let result = ws.toggle_note_expansion("nonexistent-id");
        assert!(result.is_err(), "Should error for nonexistent note");
    }

    #[test]
    fn test_set_and_get_selected_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Initially no selection
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, None, "Should have no selection initially");

        // Set selection
        ws.set_selected_note(Some(&root.id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id.clone()), "Should return selected note ID");

        // Clear selection
        ws.set_selected_note(None).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, None, "Should have no selection after clearing");
    }

    #[test]
    fn test_selected_note_persists_across_open() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace and set selection
        {
            let mut ws = Workspace::create(temp.path(), "").unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.set_selected_note(Some(&root.id)).unwrap();
        }

        // Open workspace and verify selection persists
        let ws = Workspace::open(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id), "Selection should persist across open");
    }

    #[test]
    fn test_set_selected_note_overwrites_previous() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Set first selection
        ws.set_selected_note(Some(&root.id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id.clone()));

        // Set second selection (should overwrite)
        ws.set_selected_note(Some(&child_id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(child_id.clone()), "Should overwrite previous selection");
    }

    #[test]
    fn test_create_note_root() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Delete existing root note to simulate empty workspace
        let existing_root = ws.list_all_notes().unwrap()[0].clone();
        ws.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&existing_root.id],
        ).unwrap();

        // Create a new root note
        let new_root_id = ws.create_note_root("TextNote").unwrap();
        let new_root = ws.get_note(&new_root_id).unwrap();

        assert_eq!(new_root.title, "Untitled");
        assert_eq!(new_root.node_type, "TextNote");
        assert_eq!(new_root.parent_id, None, "Root note should have no parent");
        assert_eq!(new_root.position, 0, "Root note should be at position 0");
        assert!(new_root.is_expanded, "Root note should be expanded");
    }

    #[test]
    fn test_create_note_root_invalid_type() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Delete existing root note
        let existing_root = ws.list_all_notes().unwrap()[0].clone();
        ws.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&existing_root.id],
        ).unwrap();

        // Try to create a root note with invalid type
        let result = ws.create_note_root("InvalidType");
        assert!(result.is_err(), "Should fail with invalid node type");
    }

    #[test]
    fn test_sibling_insertion_does_not_create_duplicate_positions() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Create child1 at position 0 under root
        let child1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        // Create child2 as sibling after child1 → gets position 1
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();
        // Create child3 as sibling after child1 → should push child2 to position 2, child3 at position 1
        let child3_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        let child1 = ws.get_note(&child1_id).unwrap();
        let child2 = ws.get_note(&child2_id).unwrap();
        let child3 = ws.get_note(&child3_id).unwrap();

        // All siblings should have unique positions
        assert_ne!(child1.position, child2.position, "child1 and child2 should not share a position");
        assert_ne!(child2.position, child3.position, "child2 and child3 should not share a position");
        assert_ne!(child1.position, child3.position, "child1 and child3 should not share a position");
    }

    #[test]
    fn test_get_note_with_corrupt_fields_json_returns_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        // Corrupt the stored JSON directly.
        ws.storage.connection_mut().execute(
            "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
            [&root.id],
        ).unwrap();

        // Should return Err, not panic.
        let result = ws.get_note(&root.id);
        assert!(result.is_err(), "get_note should return Err for corrupt fields_json");
    }

    #[test]
    fn test_list_all_notes_with_corrupt_fields_json_returns_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.storage.connection_mut().execute(
            "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
            [&root.id],
        ).unwrap();

        let result = ws.list_all_notes();
        assert!(result.is_err(), "list_all_notes should return Err for corrupt fields_json");
    }

    #[test]
    fn test_sibling_insertion_preserves_correct_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Create child1 (position 0), child2 as sibling (position 1)
        let child1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();
        // Insert child3 as sibling after child1 — should land between child1 and child2
        let child3_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        let child1 = ws.get_note(&child1_id).unwrap();
        let child2 = ws.get_note(&child2_id).unwrap();
        let child3 = ws.get_note(&child3_id).unwrap();

        // Expected order: child1 (0), child3 (1), child2 (2)
        assert_eq!(child1.position, 0, "child1 should remain at position 0");
        assert_eq!(child3.position, 1, "child3 (inserted after child1) should be at position 1");
        assert_eq!(child2.position, 2, "child2 should be bumped to position 2");
    }

    #[test]
    fn test_update_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Get the root note
        let notes = ws.list_all_notes().unwrap();
        let note_id = notes[0].id.clone();
        let original_modified = notes[0].modified_at;

        // Timestamp resolution is 1 s; sleep ensures modified_at advances.
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Update the note
        let new_title = "Updated Title".to_string();
        let mut new_fields = HashMap::new();
        new_fields.insert("body".to_string(), FieldValue::Text("Updated body".to_string()));

        let updated = ws.update_note(&note_id, new_title.clone(), new_fields.clone()).unwrap();

        // Verify changes
        assert_eq!(updated.title, new_title);
        assert_eq!(updated.fields.get("body"), Some(&FieldValue::Text("Updated body".to_string())));
        assert!(updated.modified_at > original_modified);
    }

    #[test]
    fn test_update_note_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let result = ws.update_note("nonexistent-id", "Title".to_string(), HashMap::new());
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_count_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Get root note
        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        // Initially has 0 children
        let count = ws.count_children(&root_id).unwrap();
        assert_eq!(count, 0);

        // Create 3 child notes
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Now has 3 children
        let count = ws.count_children(&root_id).unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_delete_note_recursive() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Get root note
        let root = ws.list_all_notes().unwrap()[0].clone();
        let root_id = root.id.clone();

        // Create tree: root -> child1 -> grandchild1
        //                   -> child2
        let child1_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let grandchild1_id = ws.create_note(&child1_id, AddPosition::AsChild, "TextNote").unwrap();

        // Count: root + child1 + child2 + grandchild1 = 4 notes
        assert_eq!(ws.list_all_notes().unwrap().len(), 4);

        // Delete child1 (should delete child1 + grandchild1)
        let result = ws.delete_note_recursive(&child1_id).unwrap();
        assert_eq!(result.deleted_count, 2);
        assert!(result.affected_ids.contains(&child1_id));
        assert!(result.affected_ids.contains(&grandchild1_id));

        // Now only root + child2 remain
        let remaining = ws.list_all_notes().unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|n| n.id == root_id));
        assert!(remaining.iter().any(|n| n.id == child2_id));
    }

    #[test]
    fn test_delete_note_recursive_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let result = ws.delete_note_recursive("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_delete_note_promote() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Get root note
        let root = ws.list_all_notes().unwrap()[0].clone();
        let root_id = root.id.clone();

        // Create tree: root -> middle -> child1
        //                              -> child2
        let middle_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let child1_id = ws.create_note(&middle_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&middle_id, AddPosition::AsChild, "TextNote").unwrap();

        // Count: 4 notes total
        assert_eq!(ws.list_all_notes().unwrap().len(), 4);

        // Delete middle (promote children)
        let result = ws.delete_note_promote(&middle_id).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.affected_ids, vec![middle_id.clone()]);

        // Now: root, child1, child2 (3 notes)
        let remaining = ws.list_all_notes().unwrap();
        assert_eq!(remaining.len(), 3);

        // Verify child1 and child2 now have root as parent
        let child1_updated = remaining.iter().find(|n| n.id == child1_id).unwrap();
        let child2_updated = remaining.iter().find(|n| n.id == child2_id).unwrap();
        assert_eq!(child1_updated.parent_id, Some(root_id.clone()));
        assert_eq!(child2_updated.parent_id, Some(root_id.clone()));
    }

    #[test]
    fn test_update_contact_rejects_empty_required_fields() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        // Contact schema is already loaded from starter scripts.

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        // Contact must be created under a ContactsFolder (allowed_parent_types constraint).
        let folder_id = ws
            .create_note(&root_id, AddPosition::AsChild, "ContactsFolder")
            .unwrap();
        let contact_id = ws
            .create_note(&folder_id, AddPosition::AsChild, "Contact")
            .unwrap();

        // first_name is required but empty — save must fail.
        let mut fields = HashMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Smith".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let result = ws.update_note(&contact_id, "".to_string(), fields);
        assert!(
            matches!(result, Err(KrillnotesError::ValidationFailed(_))),
            "Expected ValidationFailed, got {:?}", result
        );
    }

    /// Verify that `delete_note_promote` returns `NoteNotFound` when the given ID does not exist.
    #[test]
    fn test_delete_note_promote_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let result = ws.delete_note_promote("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    /// Verifies that positions do not collide when children are promoted by
    /// `delete_note_promote`. Specifically, when a node with two children (sib1,
    /// sib2) is deleted, and sib1 itself has children (child1, child2), those
    /// grandchildren should receive sequential positions with no duplicates.
    #[test]
    fn test_delete_note_promote_no_position_collision() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Build tree: root -> sib1 (pos 0) -> child1 (pos 0)
        //                                   -> child2 (pos 1)
        //                  -> sib2 (pos 1)
        let root = ws.list_all_notes().unwrap()[0].clone();
        let sib1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let sib2_id = ws.create_note(&sib1_id, AddPosition::AsSibling, "TextNote").unwrap();
        let child1_id = ws.create_note(&sib1_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        // Delete sib1 with promote — child1 and child2 move up to root level
        ws.delete_note_promote(&sib1_id).unwrap();

        // Collect remaining notes at root level
        let notes = ws.list_all_notes().unwrap();

        // sib1 must be gone
        assert!(notes.iter().all(|n| n.id != sib1_id), "sib1 should be deleted");

        // Gather positions of the surviving root-level notes
        let root_level: Vec<_> = notes.iter().filter(|n| n.parent_id == Some(root.id.clone())).collect();
        let mut positions: Vec<i32> = root_level.iter().map(|n| n.position).collect();
        positions.sort();

        // All positions must be unique
        let unique_count = {
            let mut deduped = positions.clone();
            deduped.dedup();
            deduped.len()
        };
        assert_eq!(
            positions.len(), unique_count,
            "Positions after promote must be unique, got: {:?}", positions
        );

        // sib2, child1, child2 should all be at root level
        let surviving_ids: Vec<_> = root_level.iter().map(|n| n.id.clone()).collect();
        assert!(surviving_ids.contains(&sib2_id), "sib2 should remain at root level");
        assert!(surviving_ids.contains(&child1_id), "child1 should be promoted to root level");
        assert!(surviving_ids.contains(&child2_id), "child2 should be promoted to root level");
    }

    #[test]
    fn test_update_contact_derives_title_from_hook() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        // Contact schema is already loaded from starter scripts.

        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        // Contact must be created under a ContactsFolder (allowed_parent_types constraint).
        let folder_id = ws
            .create_note(&root_id, AddPosition::AsChild, "ContactsFolder")
            .unwrap();
        let contact_id = ws
            .create_note(&folder_id, AddPosition::AsChild, "Contact")
            .unwrap();

        let mut fields = HashMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("Alice".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Walker".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let updated = ws
            .update_note(&contact_id, "ignored title".to_string(), fields)
            .unwrap();

        assert_eq!(updated.title, "Walker, Alice");
    }

    /// Verifies that `delete_note` dispatches correctly to both deletion strategies.
    ///
    /// - `DeleteAll` removes the target note and all descendants.
    /// - `PromoteChildren` removes only the target, re-parenting its children to
    ///   the grandparent.
    // ── User-script CRUD tests ──────────────────────────────────

    #[test]
    fn test_workspace_created_with_starter_scripts() {
        let temp = NamedTempFile::new().unwrap();
        let workspace = Workspace::create(temp.path(), "").unwrap();
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(!scripts.is_empty(), "New workspace should have starter scripts");
        // Verify first starter script is TextNote
        assert_eq!(scripts[0].name, "Text Note");
        assert!(scripts[0].enabled);
        assert_eq!(scripts[0].load_order, 0);
    }

    #[test]
    fn test_create_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "").unwrap();
        let starter_count = workspace.list_user_scripts().unwrap().len();
        let source = "// @name: Test Script\n// @description: A test\nschema(\"TestType\", #{ fields: [] });";
        let (script, errors) = workspace.create_user_script(source).unwrap();
        assert!(errors.is_empty());
        assert_eq!(script.name, "Test Script");
        assert_eq!(script.description, "A test");
        assert!(script.enabled);
        assert_eq!(script.load_order, starter_count as i32);
    }

    #[test]
    fn test_create_user_script_missing_name_fails() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "").unwrap();
        let source = "// no name here\nschema(\"X\", #{ fields: [] });";
        let result = workspace.create_user_script(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "").unwrap();
        let source = "// @name: Original\nschema(\"Orig\", #{ fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();

        let new_source = "// @name: Updated\nschema(\"Updated\", #{ fields: [] });";
        let (updated, errors) = workspace.update_user_script(&script.id, new_source).unwrap();
        assert!(errors.is_empty());
        assert_eq!(updated.name, "Updated");
    }

    #[test]
    fn test_delete_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "").unwrap();
        let initial_count = workspace.list_user_scripts().unwrap().len();
        let source = "// @name: ToDelete\nschema(\"Del\", #{ fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        assert_eq!(workspace.list_user_scripts().unwrap().len(), initial_count + 1);

        workspace.delete_user_script(&script.id).unwrap();
        assert_eq!(workspace.list_user_scripts().unwrap().len(), initial_count);
    }

    #[test]
    fn test_toggle_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "").unwrap();
        let source = "// @name: Toggle\nschema(\"Tog\", #{ fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        assert!(script.enabled);

        workspace.toggle_user_script(&script.id, false).unwrap();
        let updated = workspace.get_user_script(&script.id).unwrap();
        assert!(!updated.enabled);
    }

    #[test]
    fn test_user_scripts_sorted_by_load_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "").unwrap();
        let starter_count = workspace.list_user_scripts().unwrap().len();

        let s1 = "// @name: Second\nschema(\"S2\", #{ fields: [] });";
        let s2 = "// @name: First\nschema(\"S1\", #{ fields: [] });";
        workspace.create_user_script(s1).unwrap();
        let (second, _) = workspace.create_user_script(s2).unwrap();
        // Move "First" before all starters
        workspace.reorder_user_script(&second.id, -1).unwrap();

        let scripts = workspace.list_user_scripts().unwrap();
        assert_eq!(scripts[0].name, "First", "Reordered script should come first");
        // "Second" should come after all starters
        assert_eq!(scripts[starter_count + 1].name, "Second");
    }

    #[test]
    fn test_user_scripts_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path(), "").unwrap();
            workspace.create_user_script(
                "// @name: TestOpen\nschema(\"OpenType\", #{ fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap(); // (UserScript, Vec<ScriptError>) — result not inspected here
        }

        let workspace = Workspace::open(temp.path(), "").unwrap();
        assert!(workspace.script_registry().get_schema("OpenType").is_ok());
    }

    #[test]
    fn test_disabled_user_scripts_not_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path(), "").unwrap();
            let (script, _) = workspace.create_user_script(
                "// @name: Disabled\nschema(\"DisType\", #{ fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap();
            workspace.toggle_user_script(&script.id, false).unwrap();
        }

        let workspace = Workspace::open(temp.path(), "").unwrap();
        assert!(workspace.script_registry().get_schema("DisType").is_err());
    }

    #[test]
    fn test_delete_note_with_strategy() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Test DeleteAll strategy
        let result = ws.delete_note(&child_id, DeleteStrategy::DeleteAll).unwrap();
        assert_eq!(result.deleted_count, 1);

        // Create new child for PromoteChildren test
        let child2_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let grandchild_id = ws.create_note(&child2_id, AddPosition::AsChild, "TextNote").unwrap();

        let result = ws.delete_note(&child2_id, DeleteStrategy::PromoteChildren).unwrap();
        assert_eq!(result.deleted_count, 1);

        // Verify grandchild promoted
        let notes = ws.list_all_notes().unwrap();
        let gc = notes.iter().find(|n| n.id == grandchild_id).unwrap();
        assert_eq!(gc.parent_id, Some(root.id));
    }

    // ── move_note tests ──────────────────────────────────────────

    /// Helper: create a workspace with a root note and N children under it.
    ///
    /// The first child is created with `AsChild` (position 0). Subsequent
    /// children are created with `AsSibling` relative to the previous child,
    /// giving them sequential positions 0, 1, 2, .... The returned `Vec`
    /// preserves that order: `child_ids[0]` is at position 0, etc.
    fn setup_with_children(n: usize) -> (Workspace, String, Vec<String>, NamedTempFile) {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let mut child_ids: Vec<String> = Vec::new();
        for i in 0..n {
            let id = if i == 0 {
                ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                    .unwrap()
            } else {
                ws.create_note(&child_ids[i - 1], AddPosition::AsSibling, "TextNote")
                    .unwrap()
            };
            child_ids.push(id);
        }
        (ws, root.id, child_ids, temp)
    }

    #[test]
    fn test_move_note_reorder_siblings() {
        let (mut ws, root_id, children, _temp) = setup_with_children(3);
        ws.move_note(&children[2], Some(&root_id), 0).unwrap();
        let kids = ws.get_children(&root_id).unwrap();
        assert_eq!(kids[0].id, children[2]);
        assert_eq!(kids[1].id, children[0]);
        assert_eq!(kids[2].id, children[1]);
        for (i, kid) in kids.iter().enumerate() {
            assert_eq!(kid.position, i as i32, "Position mismatch at index {i}");
        }
    }

    #[test]
    fn test_move_note_to_different_parent() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[1], Some(&children[0]), 0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[0]);
        assert_eq!(root_kids[0].position, 0);
        let grandkids = ws.get_children(&children[0]).unwrap();
        assert_eq!(grandkids.len(), 1);
        assert_eq!(grandkids[0].id, children[1]);
        assert_eq!(grandkids[0].position, 0);
    }

    #[test]
    fn test_move_note_to_root() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[0], None, 1).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[1]);
        assert_eq!(root_kids[0].position, 0);
        let moved = ws.get_note(&children[0]).unwrap();
        assert_eq!(moved.parent_id, None);
        assert_eq!(moved.position, 1);
    }

    #[test]
    fn test_move_note_prevents_cycle() {
        let (mut ws, _root_id, children, _temp) = setup_with_children(1);
        let grandchild_id = ws
            .create_note(&children[0], AddPosition::AsChild, "TextNote")
            .unwrap();
        let result = ws.move_note(&children[0], Some(&grandchild_id), 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cycle"), "Expected cycle error, got: {err}");
    }

    #[test]
    fn test_move_note_prevents_self_move() {
        let (mut ws, _root_id, children, _temp) = setup_with_children(1);
        let result = ws.move_note(&children[0], Some(&children[0]), 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_move_note_logs_operation() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[1], Some(&root_id), 0).unwrap();
        let ops = ws.list_operations(None, None, None).unwrap();
        let move_ops: Vec<_> = ops.iter().filter(|o| o.operation_type == "MoveNote").collect();
        assert_eq!(move_ops.len(), 1, "Expected exactly one MoveNote operation");
    }

    #[test]
    fn test_move_note_positions_gapless_after_cross_parent_move() {
        let (mut ws, root_id, children, _temp) = setup_with_children(4);
        ws.move_note(&children[1], Some(&children[0]), 0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 3);
        for (i, kid) in root_kids.iter().enumerate() {
            assert_eq!(kid.position, i as i32, "Gap at index {i}");
        }
    }

    #[test]
    fn test_run_view_hook_returns_html_without_hook() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Load a schema with a textarea field but no on_view hook.
        ws.create_user_script(
            r#"// @name: Memo
schema("Memo", #{
    fields: [
        #{ name: "body", type: "textarea", required: false }
    ]
});
"#,
        )
        .unwrap();

        // Create a Memo note under the root.
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws
            .create_note(&root.id, AddPosition::AsChild, "Memo")
            .unwrap();

        // Update the note's body field with Markdown content.
        let mut fields = HashMap::new();
        fields.insert("body".into(), FieldValue::Text("**hello**".into()));
        ws.update_note(&note_id, "My Memo".into(), fields).unwrap();

        let html = ws.run_view_hook(&note_id).unwrap();
        assert!(!html.is_empty(), "default view must return non-empty HTML");
        assert!(
            html.contains("<strong>hello</strong>"),
            "textarea body should be markdown-rendered, got: {html}"
        );
    }

    #[test]
    fn test_create_user_script_rejects_compile_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let initial_count = ws.list_user_scripts().unwrap().len();

        // Clearly invalid Rhai: assignment with no identifier
        let bad_script = "// @name: Bad Script\n\nlet = 5;";
        let result = ws.create_user_script(bad_script);

        assert!(result.is_err(), "Should return error for invalid Rhai");
        // Confirm nothing was saved
        let scripts = ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), initial_count, "No script should be saved on compile error");
    }

    #[test]
    fn test_update_user_script_rejects_compile_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let initial_count = ws.list_user_scripts().unwrap().len();

        // Create a valid script first
        let valid_script = "// @name: Good Script\n\n// valid empty body";
        let (created, _) = ws.create_user_script(valid_script).unwrap();

        // Attempt update with invalid Rhai
        let bad_script = "// @name: Good Script\n\nlet = 5;";
        let result = ws.update_user_script(&created.id, bad_script);

        assert!(result.is_err(), "Should return error for invalid Rhai on update");

        // Original source code must be preserved
        let scripts = ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), initial_count + 1, "Script count must be unchanged after failed update");
        let saved = scripts.iter().find(|s| s.id == created.id).unwrap();
        assert_eq!(
            saved.source_code, valid_script,
            "Source code must be unchanged after failed update"
        );
    }

    #[test]
    fn test_create_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "secret").unwrap();
        // Should have at least one note (the root note)
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret").unwrap();
        let ws = Workspace::open(temp.path(), "secret").unwrap();
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_wrong_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret").unwrap();
        let result = Workspace::open(temp.path(), "wrong");
        assert!(matches!(result, Err(KrillnotesError::WrongPassword)));
    }

    #[test]
    fn test_deep_copy_note_as_child() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // root → child
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&child_id, "Original Child".to_string())
            .unwrap();

        // Copy child as another child of root
        let copy_id = ws
            .deep_copy_note(&child_id, &root.id, AddPosition::AsChild)
            .unwrap();

        // Copy has a new ID
        assert_ne!(copy_id, child_id);

        // Copy has same title and node_type
        let copy = ws.get_note(&copy_id).unwrap();
        assert_eq!(copy.title, "Original Child");
        assert_eq!(copy.node_type, "TextNote");

        // Original is unchanged
        let original = ws.get_note(&child_id).unwrap();
        assert_eq!(original.title, "Original Child");
        assert_eq!(original.parent_id, Some(root.id.clone()));
    }

    #[test]
    fn test_deep_copy_note_recursive() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // root → note_a → note_b
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_a_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&note_a_id, "Note A".to_string())
            .unwrap();
        let note_b_id = ws
            .create_note(&note_a_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&note_b_id, "Note B".to_string())
            .unwrap();

        // Copy note_a (with note_b inside) as a child of root
        let copy_a_id = ws
            .deep_copy_note(&note_a_id, &root.id, AddPosition::AsChild)
            .unwrap();

        // copy of note_a exists with a new ID and correct title
        assert_ne!(copy_a_id, note_a_id);
        let copy_a = ws.get_note(&copy_a_id).unwrap();
        assert_eq!(copy_a.title, "Note A");

        // A copy of note_b also exists — find it by parent = copy_a
        let all_notes = ws.list_all_notes().unwrap();
        let copy_b = all_notes
            .iter()
            .find(|n| n.parent_id.as_deref() == Some(&copy_a_id) && n.title == "Note B")
            .expect("copy of note_b should exist under copy_a");

        // copy of note_b has a new ID (not the original)
        assert_ne!(copy_b.id, note_b_id);

        // originals are untouched
        let orig_a = ws.get_note(&note_a_id).unwrap();
        assert_eq!(orig_a.parent_id, Some(root.id.clone()));
        let orig_b = ws.get_note(&note_b_id).unwrap();
        assert_eq!(orig_b.parent_id, Some(note_a_id.clone()));
    }

    #[test]
    fn test_on_add_child_hook_fires_on_create() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                    parent_note.title = "Folder (1)";
                    #{ parent: parent_note, child: child_note }
                }
            });
            schema("Item", #{
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();

        // Create an Item under the Folder — this should trigger the hook
        ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.title, "Folder (1)");
        assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
    }

    #[test]
    fn test_on_add_child_hook_fires_for_sibling_under_hooked_parent() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                    #{ parent: parent_note, child: child_note }
                }
            });
            schema("Item", #{
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
        // First child created as child of Folder (hook fires, count=1)
        let first_item_id = ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();
        // Second item created as sibling of first (still a child of Folder, hook should fire again, count=2)
        ws.create_note(&first_item_id, AddPosition::AsSibling, "Item").unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.fields["count"], FieldValue::Number(2.0));
    }

    #[test]
    fn test_on_add_child_hook_does_not_fire_for_root_level_creation() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // No on_add_child hook registered — creating a sibling of root should work silently
        let root = ws.list_all_notes().unwrap()[0].clone();
        // This creates a sibling of root, which has no parent — should not panic or error
        let result = ws.create_note(&root.id, AddPosition::AsSibling, "TextNote");
        assert!(result.is_ok(), "sibling of root should succeed without hook");
    }

    #[test]
    fn test_on_add_child_hook_fires_on_move() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                    parent_note.title = "Folder (1)";
                    #{ parent: parent_note, child: child_note }
                }
            });
            schema("Item", #{
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        // Create Folder and Item as siblings (both children of root)
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
        let item_id   = ws.create_note(&root.id, AddPosition::AsChild, "Item").unwrap();

        // Move Item under Folder — hook should fire
        ws.move_note(&item_id, Some(&folder_id), 0).unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.title, "Folder (1)");
        assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
    }

    // ── tree actions ─────────────────────────────────────────────────────────

    #[test]
    fn test_run_tree_action_reorders_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let parent_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Create first child: "B Note" (position 0)
        let child_b_id = ws.create_note(&parent_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_title(&child_b_id, "B Note".to_string()).unwrap();

        // Create second child as sibling: "A Note" (position 1)
        let child_a_id = ws.create_note(&child_b_id, AddPosition::AsSibling, "TextNote").unwrap();
        ws.update_note_title(&child_a_id, "A Note".to_string()).unwrap();

        // Verify initial order: B Note first, A Note second
        let kids_before = ws.get_children(&parent_id).unwrap();
        assert_eq!(kids_before[0].title, "B Note");
        assert_eq!(kids_before[1].title, "A Note");

        // Load a script that sorts children alphabetically
        ws.create_user_script(r#"
// @name: SortTest
add_tree_action("Sort A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
        "#).unwrap();

        ws.run_tree_action(&parent_id, "Sort A→Z").unwrap();

        let kids = ws.get_children(&parent_id).unwrap();
        assert_eq!(kids[0].title, "A Note");
        assert_eq!(kids[1].title, "B Note");
    }

    // ── tree action creates / updates ─────────────────────────────────────────

    #[test]
    fn test_tree_action_create_note_writes_to_db() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.create_user_script(r#"
// @name: CreateAction
schema("TaFolder", #{ fields: [] });
schema("TaItem", #{ fields: [#{ name: "tag", type: "text", required: false }] });
add_tree_action("Add Item", ["TaFolder"], |folder| {
    let item = create_note(folder.id, "TaItem");
    item.title = "My Item";
    item.fields.tag = "hello";
    update_note(item);
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TaFolder").unwrap();

        ws.run_tree_action(&folder_id, "Add Item").unwrap();

        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 1, "one child should have been created");
        assert_eq!(children[0].title, "My Item");
        assert_eq!(
            children[0].fields.get("tag"),
            Some(&FieldValue::Text("hello".into()))
        );
    }

    #[test]
    fn test_tree_action_update_note_writes_to_db() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.create_user_script(r#"
// @name: UpdateAction
schema("TaTask", #{ fields: [#{ name: "status", type: "text", required: false }] });
add_tree_action("Mark Done", ["TaTask"], |note| {
    note.title = "Done Task";
    note.fields.status = "done";
    update_note(note);
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let task_id = ws.create_note(&root.id, AddPosition::AsChild, "TaTask").unwrap();

        ws.run_tree_action(&task_id, "Mark Done").unwrap();

        let updated = ws.get_note(&task_id).unwrap();
        assert_eq!(updated.title, "Done Task");
        assert_eq!(
            updated.fields.get("status"),
            Some(&FieldValue::Text("done".into()))
        );
    }

    #[test]
    fn test_tree_action_nested_create_builds_subtree() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.create_user_script(r#"
// @name: NestedCreate
schema("TaSprint", #{ fields: [] });
schema("TaSubTask", #{ fields: [] });
add_tree_action("Add Sprint With Task", ["TaSprint"], |sprint| {
    let child_sprint = create_note(sprint.id, "TaSprint");
    child_sprint.title = "Child Sprint";
    update_note(child_sprint);
    let task = create_note(child_sprint.id, "TaSubTask");
    task.title = "Sprint Task";
    update_note(task);
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let sprint_id = ws.create_note(&root.id, AddPosition::AsChild, "TaSprint").unwrap();

        ws.run_tree_action(&sprint_id, "Add Sprint With Task").unwrap();

        // The child sprint should be under sprint_id
        let sprint_children = ws.get_children(&sprint_id).unwrap();
        assert_eq!(sprint_children.len(), 1, "one child sprint expected");
        assert_eq!(sprint_children[0].title, "Child Sprint");

        // The task should be under the child sprint
        let task_children = ws.get_children(&sprint_children[0].id).unwrap();
        assert_eq!(task_children.len(), 1, "one task expected under child sprint");
        assert_eq!(task_children[0].title, "Sprint Task");
    }

    #[test]
    fn test_tree_action_error_rolls_back_all_writes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        ws.create_user_script(r#"
// @name: ErrorAction
schema("TaErrFolder", #{ fields: [] });
schema("TaErrItem", #{ fields: [] });
add_tree_action("Create Then Fail", ["TaErrFolder"], |folder| {
    let item = create_note(folder.id, "TaErrItem");
    item.title = "Orphan";
    update_note(item);
    throw "deliberate error";
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TaErrFolder").unwrap();

        let result = ws.run_tree_action(&folder_id, "Create Then Fail");
        assert!(result.is_err(), "action should propagate the thrown error");

        // No note should have been created — the creates are not applied when the action errors
        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 0, "rollback: no child note should exist");
    }

    #[test]
    fn test_note_tags_round_trip() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.tags.is_empty());

        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert_eq!(note.tags, vec!["design", "rust"]); // sorted
    }

    #[test]
    fn test_get_all_tags_empty() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();
        assert!(ws.get_all_tags().unwrap().is_empty());
    }

    #[test]
    fn test_get_all_tags_sorted_distinct() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();
        ws.update_note_tags(&child_id, vec!["rust".into(), "testing".into()]).unwrap();
        let tags = ws.get_all_tags().unwrap();
        assert_eq!(tags, vec!["design", "rust", "testing"]);
    }

    #[test]
    fn test_get_notes_for_tag() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_tags(&root.id, vec!["rust".into()]).unwrap();
        ws.update_note_tags(&child_id, vec!["design".into()]).unwrap();

        let rust_notes = ws.get_notes_for_tag(&["rust".into()]).unwrap();
        assert_eq!(rust_notes.len(), 1);
        assert_eq!(rust_notes[0].id, root.id);

        // OR logic: both notes returned when both tags queried
        let both = ws.get_notes_for_tag(&["rust".into(), "design".into()]).unwrap();
        assert_eq!(both.len(), 2);

        // Unknown tag returns empty
        let none = ws.get_notes_for_tag(&["unknown".into()]).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_update_note_tags_replaces_existing() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["old".into()]).unwrap();
        ws.update_note_tags(&root.id, vec!["new".into()]).unwrap();
        let tags = ws.get_all_tags().unwrap();
        assert_eq!(tags, vec!["new"]); // "old" removed
    }

    #[test]
    fn test_update_note_tags_normalises() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["  Rust  ".into(), "RUST".into(), "rust".into()]).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert_eq!(note.tags, vec!["rust"]); // deduped, lowercased, trimmed
    }
}
