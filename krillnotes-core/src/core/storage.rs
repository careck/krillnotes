// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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
        if !password.is_empty() {
            let escaped = password.replace('\'', "''");
            conn.execute_batch(&format!("PRAGMA key = '{escaped}';\n"))?;
        }
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
        if !password.is_empty() {
            let escaped = password.replace('\'', "''");
            conn.execute_batch(&format!("PRAGMA key = '{escaped}';\n"))?;
        }

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

        // Migration: add note_tags table if absent.
        let note_tags_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_tags'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !note_tags_exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS note_tags (
                    note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
                    tag     TEXT NOT NULL,
                    PRIMARY KEY (note_id, tag)
                );
                CREATE INDEX IF NOT EXISTS idx_note_tags_tag ON note_tags(tag);"
            )?;
        }

        // Migration: add note_links table
        let note_links_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_links'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !note_links_exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS note_links (
                    source_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
                    field_name TEXT NOT NULL,
                    target_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE RESTRICT,
                    PRIMARY KEY (source_id, field_name)
                );
                CREATE INDEX IF NOT EXISTS idx_note_links_target ON note_links(target_id);",
            )?;
        }

        // Migration: add attachments table if absent.
        let attachments_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !attachments_exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS attachments (
                    id          TEXT PRIMARY KEY,
                    note_id     TEXT NOT NULL,
                    filename    TEXT NOT NULL,
                    mime_type   TEXT,
                    size_bytes  INTEGER NOT NULL,
                    hash_sha256 TEXT NOT NULL,
                    salt        BLOB NOT NULL,
                    created_at  INTEGER NOT NULL,
                    FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_attachments_note_id ON attachments(note_id);",
            )?;
        }

        // Migration: add hlc_state table, replace operations.timestamp with HLC columns,
        // and change notes.position from INTEGER to REAL.
        let hlc_state_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='hlc_state'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !hlc_state_exists {
            // Step 1: Create hlc_state table.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS hlc_state (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    wall_ms INTEGER NOT NULL,
                    counter INTEGER NOT NULL,
                    node_id INTEGER NOT NULL
                );",
            )?;

            // Step 2: Recreate operations table with HLC timestamp columns.
            // Check whether the old operations table has the standard `operation_id` column
            // before migrating data (very old test-schema tables may only have `id`).
            let ops_has_operation_id: bool = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name='operation_id'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )?;

            conn.execute_batch(
                "CREATE TABLE operations_new (
                    operation_id TEXT NOT NULL PRIMARY KEY,
                    timestamp_wall_ms INTEGER NOT NULL DEFAULT 0,
                    timestamp_counter INTEGER NOT NULL DEFAULT 0,
                    timestamp_node_id INTEGER NOT NULL DEFAULT 0,
                    device_id TEXT NOT NULL,
                    operation_type TEXT NOT NULL,
                    operation_data TEXT NOT NULL,
                    synced INTEGER NOT NULL DEFAULT 0
                );",
            )?;

            if ops_has_operation_id {
                // Standard schema: copy data, converting Unix seconds → milliseconds.
                conn.execute_batch(
                    "INSERT INTO operations_new
                        SELECT operation_id,
                               COALESCE(timestamp, 0) * 1000,
                               0,
                               0,
                               device_id,
                               operation_type,
                               operation_data,
                               synced
                        FROM operations;",
                )?;
            }
            // If old table had no operation_id, it held no real data — skip data migration.

            conn.execute_batch(
                "DROP TABLE operations;
                ALTER TABLE operations_new RENAME TO operations;
                CREATE INDEX IF NOT EXISTS idx_operations_timestamp_wall_ms ON operations(timestamp_wall_ms);
                CREATE INDEX IF NOT EXISTS idx_operations_synced ON operations(synced);",
            )?;

            // Step 3: Recreate notes table with position REAL.
            // Check whether the notes table has the standard columns (test helpers may be minimal).
            let notes_has_created_at: bool = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='created_at'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )?;

            conn.execute_batch(
                "CREATE TABLE notes_new (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    node_type TEXT NOT NULL,
                    parent_id TEXT,
                    position REAL NOT NULL DEFAULT 0.0,
                    created_at INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL,
                    created_by INTEGER NOT NULL DEFAULT 0,
                    modified_by INTEGER NOT NULL DEFAULT 0,
                    fields_json TEXT NOT NULL DEFAULT '{}',
                    is_expanded INTEGER DEFAULT 1,
                    FOREIGN KEY (parent_id) REFERENCES notes(id) ON DELETE CASCADE
                );",
            )?;

            if notes_has_created_at {
                // Full-schema notes table: copy all rows.
                conn.execute_batch("INSERT INTO notes_new SELECT * FROM notes;")?;
            }
            // Minimal test notes tables (no created_at) have no data worth migrating.

            conn.execute_batch(
                "DROP TABLE notes;
                ALTER TABLE notes_new RENAME TO notes;
                CREATE INDEX IF NOT EXISTS idx_notes_parent ON notes(parent_id, position);",
            )?;

            // Step 4: Seed hlc_state from existing max timestamp.
            conn.execute_batch(
                "INSERT OR IGNORE INTO hlc_state (id, wall_ms, counter, node_id)
                    SELECT 1, COALESCE(MAX(timestamp_wall_ms), 0), 0, 0 FROM operations;",
            )?;
        }

        // Migration: add category column to user_scripts if absent.
        let category_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('user_scripts') WHERE name='category'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !category_exists {
            conn.execute(
                "ALTER TABLE user_scripts ADD COLUMN category TEXT NOT NULL DEFAULT 'presentation'",
                [],
            )?;
        }

        // Migration: add schema_version column to notes if absent.
        let schema_version_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='schema_version'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !schema_version_exists {
            conn.execute(
                "ALTER TABLE notes ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1",
                [],
            )?;
        }

        // Migration: change created_by/modified_by columns from INTEGER to TEXT.
        // SQLite requires a full table rebuild to change column types.
        let created_by_type: String = conn
            .query_row(
                "SELECT type FROM pragma_table_info('notes') WHERE name='created_by'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "TEXT".to_string());
        if created_by_type.to_uppercase() == "INTEGER" {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS notes_identity_migration (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    node_type TEXT NOT NULL,
                    parent_id TEXT,
                    position REAL NOT NULL DEFAULT 0.0,
                    created_at INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL,
                    created_by TEXT NOT NULL DEFAULT '',
                    modified_by TEXT NOT NULL DEFAULT '',
                    fields_json TEXT NOT NULL DEFAULT '{}',
                    is_expanded INTEGER DEFAULT 1,
                    schema_version INTEGER NOT NULL DEFAULT 1,
                    FOREIGN KEY (parent_id) REFERENCES notes(id) ON DELETE CASCADE
                );
                INSERT INTO notes_identity_migration
                    SELECT id, title, node_type, parent_id, position,
                           created_at, modified_at,
                           CAST(created_by AS TEXT), CAST(modified_by AS TEXT),
                           fields_json, is_expanded, schema_version
                    FROM notes;
                DROP TABLE notes;
                ALTER TABLE notes_identity_migration RENAME TO notes;
                CREATE INDEX IF NOT EXISTS idx_notes_parent ON notes(parent_id, position);",
            )?;
        }

        // Migration: add sync_peers table if absent.
        let sync_peers_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sync_peers'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !sync_peers_exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS sync_peers (
                    peer_device_id   TEXT PRIMARY KEY,
                    peer_identity_id TEXT NOT NULL,
                    last_sent_op     TEXT,
                    last_received_op TEXT,
                    last_sync        TEXT
                )",
            )?;
        }

        // Migration: sync engine channel and status columns on sync_peers.
        let has_channel_type: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sync_peers') WHERE name = 'channel_type'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);
        if !has_channel_type {
            conn.execute_batch(
                "ALTER TABLE sync_peers ADD COLUMN channel_type TEXT NOT NULL DEFAULT 'manual';
                 ALTER TABLE sync_peers ADD COLUMN channel_params TEXT NOT NULL DEFAULT '{}';
                 ALTER TABLE sync_peers ADD COLUMN sync_status TEXT NOT NULL DEFAULT 'idle';
                 ALTER TABLE sync_peers ADD COLUMN sync_status_detail TEXT;
                 ALTER TABLE sync_peers ADD COLUMN last_sync_error TEXT;",
            )?;
        }

        // Migration: rename node_type column to schema (if the old column name still exists).
        let node_type_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='node_type'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if node_type_exists {
            conn.execute_batch(
                "ALTER TABLE notes RENAME COLUMN node_type TO schema;",
            )?;
        }

        // Migration: add HLC covering index for operations_since queries.
        let hlc_index_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_operations_hlc'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !hlc_index_exists {
            conn.execute(
                "CREATE INDEX idx_operations_hlc \
                 ON operations(timestamp_wall_ms, timestamp_counter, timestamp_node_id)",
                [],
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
#[path = "storage_tests.rs"]
mod tests;
