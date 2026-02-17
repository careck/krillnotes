use crate::{
    Note, OperationLog, PurgeStrategy, Result, SchemaRegistry, Storage,
};
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

pub struct Workspace {
    storage: Storage,
    registry: SchemaRegistry,
    operation_log: OperationLog,
    device_id: String,
    current_user_id: i64,
}

impl Workspace {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut storage = Storage::create(&path)?;
        let registry = SchemaRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 1000 });

        // Generate device ID
        let device_id = Uuid::new_v4().to_string();

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
        };

        let tx = storage.connection_mut().transaction()?;
        tx.execute(
            "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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

    pub fn registry(&self) -> &SchemaRegistry {
        &self.registry
    }

    pub fn connection(&self) -> &Connection {
        self.storage.connection()
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
}
