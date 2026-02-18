use crate::Result;
use rusqlite::Connection;
use std::path::Path;

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Self { conn })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Validate database structure
        let table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table'
             AND name IN ('notes', 'operations', 'workspace_meta')",
            [],
            |row| row.get(0)
        )?;

        if table_count != 3 {
            return Err(crate::KrillnotesError::InvalidWorkspace(
                "Not a valid Krillnotes database".to_string()
            ));
        }

        // Migrate: add is_expanded column if it doesn't exist
        let column_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0)
            )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE notes ADD COLUMN is_expanded INTEGER DEFAULT 1",
                []
            )?;
        }

        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_storage() {
        let temp = NamedTempFile::new().unwrap();
        let storage = Storage::create(temp.path()).unwrap();

        // Verify tables exist
        let tables: Vec<String> = storage
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"notes".to_string()));
        assert!(tables.contains(&"operations".to_string()));
        assert!(tables.contains(&"workspace_meta".to_string()));
    }

    #[test]
    fn test_open_existing_storage() {
        let temp = NamedTempFile::new().unwrap();

        // Create database first
        Storage::create(temp.path()).unwrap();

        // Open it
        let storage = Storage::open(temp.path()).unwrap();

        // Verify tables exist
        let tables: Vec<String> = storage
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"notes".to_string()));
        assert!(tables.contains(&"operations".to_string()));
        assert!(tables.contains(&"workspace_meta".to_string()));
    }

    #[test]
    fn test_open_invalid_database() {
        let temp = NamedTempFile::new().unwrap();

        // Create empty file (not a valid Krillnotes DB)
        std::fs::write(temp.path(), "not a database").unwrap();

        let result = Storage::open(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_migration_adds_is_expanded_column() {
        let temp = NamedTempFile::new().unwrap();

        // Create database with old schema (without is_expanded)
        {
            let conn = Connection::open(temp.path()).unwrap();
            conn.execute(
                "CREATE TABLE notes (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    node_type TEXT NOT NULL,
                    parent_id TEXT,
                    position INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL,
                    created_by INTEGER NOT NULL,
                    modified_by INTEGER NOT NULL,
                    fields_json TEXT NOT NULL
                )",
                [],
            ).unwrap();
            conn.execute("CREATE TABLE operations (id INTEGER PRIMARY KEY)", []).unwrap();
            conn.execute("CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        }

        // Open storage (should trigger migration)
        let storage = Storage::open(temp.path()).unwrap();

        // Verify is_expanded column exists
        let column_exists: bool = storage
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0)
            )
            .unwrap();

        assert!(column_exists, "is_expanded column should exist after migration");
    }
}
