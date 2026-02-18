use crate::{
    get_device_id, Note, Operation, OperationLog, PurgeStrategy, Result, SchemaRegistry, Storage,
};
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddPosition {
    AsChild,
    AsSibling,
}

#[allow(dead_code)]
pub struct Workspace {
    storage: Storage,
    registry: SchemaRegistry,
    operation_log: OperationLog,
    device_id: String,
    current_user_id: i64,
}

// SAFETY: Workspace contains SchemaRegistry with rhai::Engine that has Rc pointers.
// However, we ensure thread-safety by only accessing Workspace through Mutex in AppState.
// Each Workspace instance is associated with a single window and protected by a Mutex.
unsafe impl Send for Workspace {}
unsafe impl Sync for Workspace {}

impl Workspace {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut storage = Storage::create(&path)?;
        let registry = SchemaRegistry::new()?;
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
            fields: registry.get_schema("TextNote")?.default_fields(),
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
            registry,
            operation_log,
            device_id,
            current_user_id: 0,
        })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let storage = Storage::open(&path)?;
        let registry = SchemaRegistry::new()?;
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
            registry,
            operation_log,
            device_id,
            current_user_id,
        })
    }

    pub fn registry(&self) -> &SchemaRegistry {
        &self.registry
    }

    pub fn connection(&self) -> &Connection {
        self.storage.connection()
    }

    pub fn get_note(&self, note_id: &str) -> Result<Note> {
        let row = self.connection().query_row(
            "SELECT id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded
             FROM notes WHERE id = ?",
            [note_id],
            |row| {
                Ok(Note {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    node_type: row.get(2)?,
                    parent_id: row.get(3)?,
                    position: row.get(4)?,
                    created_at: row.get(5)?,
                    modified_at: row.get(6)?,
                    created_by: row.get(7)?,
                    modified_by: row.get(8)?,
                    fields: serde_json::from_str(&row.get::<_, String>(9)?).unwrap(),
                    is_expanded: row.get::<_, i64>(10)? == 1,
                })
            },
        )?;
        Ok(row)
    }

    pub fn create_note(
        &mut self,
        selected_note_id: &str,
        position: AddPosition,
        note_type: &str,
    ) -> Result<String> {
        let schema = self.registry.get_schema(note_type)?;
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

    pub fn list_all_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded
             FROM notes ORDER BY parent_id, position",
        )?;

        let notes = stmt
            .query_map([], |row| {
                Ok(Note {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    node_type: row.get(2)?,
                    parent_id: row.get(3)?,
                    position: row.get(4)?,
                    created_at: row.get(5)?,
                    modified_at: row.get(6)?,
                    created_by: row.get(7)?,
                    modified_by: row.get(8)?,
                    fields: serde_json::from_str(&row.get::<_, String>(9)?).unwrap(),
                    is_expanded: row.get::<_, i64>(10)? == 1,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(notes)
    }

    pub fn list_node_types(&self) -> Result<Vec<String>> {
        self.registry.list_types()
    }

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
}

fn humanize(filename: &str) -> String {
    filename
        .replace('-', " ")
        .replace('_', " ")
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
}
