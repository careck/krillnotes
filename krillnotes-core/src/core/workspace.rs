//! High-level workspace operations over a Krillnotes SQLite database.

use crate::{
    get_device_id, DeleteResult, DeleteStrategy, FieldValue, KrillnotesError, Note, Operation,
    OperationLog, PurgeStrategy, Result, ScriptRegistry, Storage,
};
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
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut storage = Storage::create(&path)?;
        let script_registry = ScriptRegistry::new()?;
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
            created_at: chrono::Utc::now().timestamp(),
            modified_at: chrono::Utc::now().timestamp(),
            created_by: 0,
            modified_by: 0,
            fields: script_registry.get_schema("TextNote")?.default_fields(),
            is_expanded: true,
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
    /// Returns [`crate::KrillnotesError::InvalidWorkspace`] if the file is not a
    /// valid Krillnotes database, or [`crate::KrillnotesError::Database`] for
    /// any SQLite failure.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let storage = Storage::open(&path)?;
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

        Ok(Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            current_user_id,
        })
    }

    /// Returns a reference to the script registry for this workspace.
    pub fn script_registry(&self) -> &ScriptRegistry {
        &self.script_registry
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
            "SELECT id, title, node_type, parent_id, position,
                    created_at, modified_at, created_by, modified_by,
                    fields_json, is_expanded
             FROM notes WHERE id = ?",
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

        let note = Note {
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

    /// Returns all notes in the workspace, ordered by `parent_id` then `position`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::Json`] if any row's `fields_json` is corrupt.
    pub fn list_all_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, title, node_type, parent_id, position,
                    created_at, modified_at, created_by, modified_by,
                    fields_json, is_expanded
             FROM notes ORDER BY parent_id, position",
        )?;

        let rows = stmt
            .query_map([], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Returns the names of all registered note types (schema names).
    ///
    /// # Errors
    ///
    /// This method currently does not fail, but returns `Result` for consistency.
    pub fn list_node_types(&self) -> Result<Vec<String>> {
        self.script_registry.list_types()
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
            "SELECT id, title, node_type, parent_id, position,
                    created_at, modified_at, created_by, modified_by,
                    fields_json, is_expanded
             FROM notes WHERE parent_id = ?1 ORDER BY position",
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
}

/// Raw 11-column tuple extracted from a `notes` SQLite row.
type NoteRow = (String, String, String, Option<String>, i64, i64, i64, i64, i64, String, i64);

/// Row-mapping closure for `rusqlite::Row` → raw tuple.
///
/// Returns the 11-column tuple that `note_from_row_tuple` converts into a `Note`.
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
    ))
}

/// Converts a raw 11-column tuple into a [`Note`], parsing `fields_json`.
fn note_from_row_tuple(
    (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded_int): NoteRow,
) -> Result<Note> {
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
    })
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
        let ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
            let ws = Workspace::create(temp.path()).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            assert_eq!(root.node_type, "TextNote");
        }

        // Open it
        let ws = Workspace::open(temp.path()).unwrap();

        // Verify we can read notes
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].node_type, "TextNote");
    }

    #[test]
    fn test_is_expanded_defaults_to_true() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
            let mut ws = Workspace::create(temp.path()).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                .unwrap();
        }

        // Open and verify is_expanded is true
        let ws = Workspace::open(temp.path()).unwrap();
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes[0].is_expanded, "Root note should be expanded");
        assert!(notes[1].is_expanded, "Child note should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

        // Try to toggle a note that doesn't exist
        let result = ws.toggle_note_expansion("nonexistent-id");
        assert!(result.is_err(), "Should error for nonexistent note");
    }

    #[test]
    fn test_set_and_get_selected_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
            let mut ws = Workspace::create(temp.path()).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.set_selected_note(Some(&root.id)).unwrap();
        }

        // Open workspace and verify selection persists
        let ws = Workspace::open(temp.path()).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id), "Selection should persist across open");
    }

    #[test]
    fn test_set_selected_note_overwrites_previous() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();
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
        let mut ws = Workspace::create(temp.path()).unwrap();
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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

        let result = ws.update_note("nonexistent-id", "Title".to_string(), HashMap::new());
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_count_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();
        let result = ws.delete_note_recursive("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_delete_note_promote() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        let contact_id = ws
            .create_note(&root_id, AddPosition::AsChild, "Contact")
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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

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
        let mut ws = Workspace::create(temp.path()).unwrap();

        // Create a root note to act as parent
        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        let contact_id = ws
            .create_note(&root_id, AddPosition::AsChild, "Contact")
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
    #[test]
    fn test_delete_note_with_strategy() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

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
}
