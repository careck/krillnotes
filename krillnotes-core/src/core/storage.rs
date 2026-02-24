//! SQLite connection management and schema migration for Krillnotes workspaces.

use crate::Result;
use rusqlite::Connection;
use std::path::Path;

/// Manages the SQLite connection for a Krillnotes workspace file.
///
/// `Storage` validates the database structure on open and applies
/// any pending column-level migrations before handing off the connection.
#[derive(Debug)]
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Creates a new workspace database at `path` and initialises the schema.
    ///
    /// The schema is loaded from the bundled `schema.sql` file. If a file
    /// already exists at `path` it will be opened and the schema re-applied
    /// (SQLite `CREATE TABLE IF NOT EXISTS` semantics).
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the file cannot be
    /// created or the schema SQL fails to execute.
    pub fn create<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let escaped = password.replace('\'', "''");
        conn.execute_batch(&format!("PRAGMA key = '{escaped}';\n"))?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Self { conn })
    }

    /// Opens an existing workspace database at `path` and runs pending migrations.
    ///
    /// Validates that the file contains all three required tables (`notes`,
    /// `operations`, `workspace_meta`) before returning. If the password is
    /// wrong, returns [`crate::KrillnotesError::WrongPassword`]. If the file is
    /// a plain (unencrypted) SQLite database, returns
    /// [`crate::KrillnotesError::UnencryptedWorkspace`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::WrongPassword`] if the password is
    /// incorrect or the file is not a valid Krillnotes database,
    /// [`crate::KrillnotesError::UnencryptedWorkspace`] if the file is a plain
    /// unencrypted SQLite database, or [`crate::KrillnotesError::Database`] for
    /// any other SQLite error.
    pub fn open<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
        let conn = Connection::open(path.as_ref())?;
        let escaped = password.replace('\'', "''");
        conn.execute_batch(&format!("PRAGMA key = '{escaped}';\n"))?;

        // Attempt to read the schema. With a wrong password, SQLCipher returns
        // garbage bytes and the query either errors or returns zero matching tables.
        let table_count: std::result::Result<i64, rusqlite::Error> = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table'
             AND name IN ('notes', 'operations', 'workspace_meta')",
            [],
            |row| row.get(0),
        );

        match table_count {
            Ok(3) => {
                // Correct password and valid workspace — run migrations.
                Self::run_migrations(&conn)?;
                Ok(Self { conn })
            }
            Ok(_) | Err(_) => {
                // Either wrong password or not a Krillnotes workspace.
                // Check if the file is a plain (unencrypted) SQLite database.
                let plain_conn = Connection::open(path.as_ref())?;
                // No PRAGMA key — opens as plaintext
                let plain_count: std::result::Result<i64, rusqlite::Error> = plain_conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table'
                     AND name IN ('notes', 'operations', 'workspace_meta')",
                    [],
                    |row| row.get(0),
                );
                match plain_count {
                    Ok(3) => Err(crate::KrillnotesError::UnencryptedWorkspace),
                    _ => Err(crate::KrillnotesError::WrongPassword),
                }
            }
        }
    }

    fn run_migrations(conn: &Connection) -> Result<()> {
        // Migration: add is_expanded column if absent.
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !column_exists {
            conn.execute("ALTER TABLE notes ADD COLUMN is_expanded INTEGER DEFAULT 1", [])?;
        }

        // Migration: add user_scripts table if absent.
        let user_scripts_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='user_scripts'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !user_scripts_exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS user_scripts (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL DEFAULT '',
                    description TEXT NOT NULL DEFAULT '',
                    source_code TEXT NOT NULL,
                    load_order INTEGER NOT NULL DEFAULT 0,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL
                )",
            )?;
        }
        Ok(())
    }

    /// Returns a shared reference to the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Returns an exclusive reference to the underlying SQLite connection.
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_encrypted_storage() {
        let temp = NamedTempFile::new().unwrap();
        let storage = Storage::create(temp.path(), "hunter2").unwrap();

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
        Storage::create(temp.path(), "testpass").unwrap();
        let storage = Storage::open(temp.path(), "testpass").unwrap();

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
        std::fs::write(temp.path(), "not a database").unwrap();
        let result = Storage::open(temp.path(), "any_password");
        assert!(
            matches!(result, Err(crate::KrillnotesError::WrongPassword)),
            "Expected WrongPassword, got: {:?}",
            result
        );
    }

    #[test]
    fn test_open_encrypted_storage_correct_password() {
        let temp = NamedTempFile::new().unwrap();
        Storage::create(temp.path(), "correct").unwrap();
        let storage = Storage::open(temp.path(), "correct").unwrap();
        let count: i64 = storage
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('notes','operations','workspace_meta')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_open_encrypted_storage_wrong_password() {
        let temp = NamedTempFile::new().unwrap();
        Storage::create(temp.path(), "correct").unwrap();
        let result = Storage::open(temp.path(), "wrong");
        assert!(matches!(result, Err(crate::KrillnotesError::WrongPassword)));
    }

    #[test]
    fn test_open_unencrypted_workspace_returns_specific_error() {
        let temp = NamedTempFile::new().unwrap();
        // Create a plain (unencrypted) SQLite database with the expected tables
        {
            let conn = rusqlite::Connection::open(temp.path()).unwrap();
            conn.execute_batch(
                "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, node_type TEXT NOT NULL, parent_id TEXT, position INTEGER NOT NULL, created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL, created_by INTEGER NOT NULL DEFAULT 0, modified_by INTEGER NOT NULL DEFAULT 0, fields_json TEXT NOT NULL DEFAULT '{}', is_expanded INTEGER DEFAULT 1);
                 CREATE TABLE operations (id INTEGER PRIMARY KEY AUTOINCREMENT, operation_id TEXT UNIQUE NOT NULL, timestamp INTEGER NOT NULL, device_id TEXT NOT NULL, operation_type TEXT NOT NULL, operation_data TEXT NOT NULL, synced INTEGER DEFAULT 0);
                 CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE user_scripts (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '', source_code TEXT NOT NULL, load_order INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL);",
            ).unwrap();
        }
        let result = Storage::open(temp.path(), "any_password");
        assert!(
            matches!(result, Err(crate::KrillnotesError::UnencryptedWorkspace)),
            "Expected UnencryptedWorkspace, got: {:?}",
            result
        );
    }

    #[test]
    fn test_migration_adds_is_expanded_column() {
        let temp = NamedTempFile::new().unwrap();

        // Create an encrypted old-schema DB (no is_expanded column) to simulate
        // a workspace created before this column was added.
        {
            let conn = Connection::open(temp.path()).unwrap();
            conn.execute_batch("PRAGMA key = 'testpass';").unwrap();
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

        let storage = Storage::open(temp.path(), "testpass").unwrap();

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

    #[test]
    fn test_migration_creates_user_scripts_table() {
        let temp = NamedTempFile::new().unwrap();

        // Create an encrypted old-schema DB (no user_scripts table) to simulate
        // a workspace created before this table was added.
        {
            let conn = Connection::open(temp.path()).unwrap();
            conn.execute_batch("PRAGMA key = 'testpass';").unwrap();
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
                    fields_json TEXT NOT NULL,
                    is_expanded INTEGER DEFAULT 1
                )",
                [],
            ).unwrap();
            conn.execute("CREATE TABLE operations (id INTEGER PRIMARY KEY)", []).unwrap();
            conn.execute("CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        }

        let storage = Storage::open(temp.path(), "testpass").unwrap();

        let table_exists: bool = storage
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='user_scripts'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0),
            )
            .unwrap();

        assert!(table_exists, "user_scripts table should exist after migration");
    }
}
