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
        )
        .unwrap();
        conn.execute("CREATE TABLE operations (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
    }

    let storage = Storage::open(temp.path(), "testpass").unwrap();

    let column_exists: bool = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
            [],
            |row| row.get::<_, i64>(0).map(|count| count > 0),
        )
        .unwrap();

    assert!(
        column_exists,
        "is_expanded column should exist after migration"
    );
}

#[test]
fn test_note_tags_table_created_on_new_workspace() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Storage::create(temp.path(), "").unwrap();
    let count: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_tags'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_note_tags_table_migration_on_existing_workspace() {
    // Simulate an old workspace that has no note_tags table.
    let temp = NamedTempFile::new().unwrap();
    // Create raw DB without note_tags
    {
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL,
             node_type TEXT NOT NULL, parent_id TEXT, position INTEGER NOT NULL,
             created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL,
             created_by INTEGER NOT NULL DEFAULT 0, modified_by INTEGER NOT NULL DEFAULT 0,
             fields_json TEXT NOT NULL DEFAULT '{}', is_expanded INTEGER DEFAULT 1);
             CREATE TABLE operations (id INTEGER PRIMARY KEY AUTOINCREMENT,
             operation_id TEXT UNIQUE NOT NULL, timestamp INTEGER NOT NULL,
             device_id TEXT NOT NULL, operation_type TEXT NOT NULL,
             operation_data TEXT NOT NULL, synced INTEGER DEFAULT 0);
             CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE user_scripts (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
             description TEXT NOT NULL DEFAULT '', source_code TEXT NOT NULL,
             load_order INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1,
             created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL);",
        )
        .unwrap();
    }
    // Open via Storage — should run migration
    let storage = Storage::open(temp.path(), "").unwrap();
    let count: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_tags'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_note_links_table_exists_after_migration() {
    // Simulate an old workspace that has no note_links table.
    let temp = NamedTempFile::new().unwrap();
    // Create raw DB without note_links
    {
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL,
             node_type TEXT NOT NULL, parent_id TEXT, position INTEGER NOT NULL,
             created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL,
             created_by INTEGER NOT NULL DEFAULT 0, modified_by INTEGER NOT NULL DEFAULT 0,
             fields_json TEXT NOT NULL DEFAULT '{}', is_expanded INTEGER DEFAULT 1);
             CREATE TABLE operations (id INTEGER PRIMARY KEY AUTOINCREMENT,
             operation_id TEXT UNIQUE NOT NULL, timestamp INTEGER NOT NULL,
             device_id TEXT NOT NULL, operation_type TEXT NOT NULL,
             operation_data TEXT NOT NULL, synced INTEGER DEFAULT 0);
             CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE user_scripts (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
             description TEXT NOT NULL DEFAULT '', source_code TEXT NOT NULL,
             load_order INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1,
             created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL);
             CREATE TABLE note_tags (
                 note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
                 tag TEXT NOT NULL,
                 PRIMARY KEY (note_id, tag)
             );",
        )
        .unwrap();
    }
    // Open via Storage::open — should run migrations and create note_links
    let storage = Storage::open(temp.path(), "").unwrap();
    let count: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_links'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
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
        )
        .unwrap();
        conn.execute("CREATE TABLE operations (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
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

    assert!(
        table_exists,
        "user_scripts table should exist after migration"
    );
}

#[test]
fn test_attachments_table_exists_on_new_workspace() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Storage::create(temp.path(), "").unwrap();
    let count: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_attachments_table_migration_on_existing_workspace() {
    let temp = NamedTempFile::new().unwrap();
    // Create raw DB without attachments table
    {
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, node_type TEXT NOT NULL,
             parent_id TEXT, position INTEGER NOT NULL, created_at INTEGER NOT NULL,
             modified_at INTEGER NOT NULL, created_by INTEGER NOT NULL DEFAULT 0,
             modified_by INTEGER NOT NULL DEFAULT 0, fields_json TEXT NOT NULL DEFAULT '{}',
             is_expanded INTEGER DEFAULT 1);
             CREATE TABLE operations (id INTEGER PRIMARY KEY AUTOINCREMENT,
             operation_id TEXT UNIQUE NOT NULL, timestamp INTEGER NOT NULL,
             device_id TEXT NOT NULL, operation_type TEXT NOT NULL,
             operation_data TEXT NOT NULL, synced INTEGER DEFAULT 0);
             CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
    }
    let storage = Storage::open(temp.path(), "").unwrap();
    let count: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_verified_by_column_exists_on_new_workspace() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Storage::create(temp.path(), "").unwrap();
    let exists: bool = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name='verified_by'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .unwrap();
    assert!(exists, "verified_by column should exist on new workspace");
}

#[test]
fn test_verified_by_column_migration_on_existing_workspace() {
    let temp = NamedTempFile::new().unwrap();
    // Create a workspace, then verify the migration adds verified_by on open.
    {
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        // Remove the verified_by column by recreating operations without it.
        conn.execute_batch(
            "CREATE TABLE operations_tmp (
                operation_id TEXT NOT NULL PRIMARY KEY,
                timestamp_wall_ms INTEGER NOT NULL DEFAULT 0,
                timestamp_counter INTEGER NOT NULL DEFAULT 0,
                timestamp_node_id INTEGER NOT NULL DEFAULT 0,
                device_id TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                operation_data TEXT NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0,
                received_from_peer TEXT
            );
            INSERT INTO operations_tmp SELECT operation_id, timestamp_wall_ms,
                timestamp_counter, timestamp_node_id, device_id, operation_type,
                operation_data, synced, received_from_peer FROM operations;
            DROP TABLE operations;
            ALTER TABLE operations_tmp RENAME TO operations;",
        )
        .unwrap();
        // Confirm verified_by is gone
        let gone: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name='verified_by'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap();
        assert!(!gone, "verified_by should not exist before migration");
    }
    // Open via Storage — migration should add verified_by
    let storage = Storage::open(temp.path(), "").unwrap();
    let exists: bool = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name='verified_by'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .unwrap();
    assert!(
        exists,
        "verified_by column should exist after migration on open"
    );
}

#[test]
fn test_hlc_index_exists_after_migration() {
    let f = tempfile::NamedTempFile::new().unwrap();
    let s = Storage::create(f.path(), "").unwrap();
    let count: i64 = s
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_operations_hlc'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "HLC index should exist after create");
}
