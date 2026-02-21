//! Durable operation log and purge strategies for the Krillnotes workspace.

use crate::{Operation, Result};
use rusqlite::Connection;
use rusqlite::Transaction;

/// Seconds in one day; used to convert `retention_days` to a Unix timestamp cutoff.
const SECONDS_PER_DAY: i64 = 86_400;

/// Controls which old operations are removed from the log.
pub enum PurgeStrategy {
    /// Retain only the most recent `keep_last` operations.
    ///
    /// Used when sync is disabled and the log is local-only.
    LocalOnly { keep_last: usize },
    /// Retain synced operations for up to `retention_days` before removing them.
    ///
    /// Used when sync is enabled and remote peers may still need older operations.
    WithSync { retention_days: u32 },
}

/// Lightweight summary of an operation for display in the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSummary {
    pub operation_id: String,
    pub timestamp: i64,
    pub device_id: String,
    pub operation_type: String,
    pub target_name: String,
}

/// Records document mutations to the `operations` table and purges stale entries.
pub struct OperationLog {
    strategy: PurgeStrategy,
}

impl OperationLog {
    /// Creates a new `OperationLog` with the given purge strategy.
    pub fn new(strategy: PurgeStrategy) -> Self {
        Self { strategy }
    }

    /// Serialises `op` and appends it to the `operations` table within `tx`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the INSERT fails, or
    /// [`crate::KrillnotesError::Json`] if `op` cannot be serialised.
    pub fn log(&self, tx: &Transaction, op: &Operation) -> Result<()> {
        let op_json = serde_json::to_string(op)?;

        tx.execute(
            "INSERT INTO operations (operation_id, timestamp, device_id, operation_type, operation_data, synced)
             VALUES (?, ?, ?, ?, ?, 0)",
            rusqlite::params![
                op.operation_id(),
                op.timestamp(),
                op.device_id(),
                self.operation_type_name(op),
                op_json,
            ],
        )?;

        Ok(())
    }

    /// Deletes old operations from the log according to the purge strategy.
    ///
    /// Call this after every [`log`](Self::log) call to keep the table bounded in size.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the DELETE fails.
    pub fn purge_if_needed(&self, tx: &Transaction) -> Result<()> {
        match self.strategy {
            PurgeStrategy::LocalOnly { keep_last } => {
                tx.execute(
                    "DELETE FROM operations WHERE id NOT IN (
                        SELECT id FROM operations ORDER BY id DESC LIMIT ?
                    )",
                    [keep_last as i64],
                )?;
            }
            PurgeStrategy::WithSync { retention_days } => {
                let cutoff = chrono::Utc::now().timestamp()
                    - (retention_days as i64 * SECONDS_PER_DAY);
                tx.execute(
                    "DELETE FROM operations WHERE synced = 1 AND timestamp < ?",
                    [cutoff],
                )?;
            }
        }
        Ok(())
    }

    /// Queries the operations table and returns lightweight summaries.
    ///
    /// Results are ordered newest-first (`timestamp DESC, id DESC`).
    /// All three filter parameters are optional and combined with AND.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the query fails.
    pub fn list(
        &self,
        conn: &Connection,
        type_filter: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<OperationSummary>> {
        let mut sql = String::from(
            "SELECT operation_id, timestamp, device_id, operation_type, operation_data FROM operations",
        );
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(t) = type_filter {
            conditions.push("operation_type = ?".to_string());
            params.push(Box::new(t.to_string()));
        }
        if let Some(s) = since {
            conditions.push("timestamp >= ?".to_string());
            params.push(Box::new(s));
        }
        if let Some(u) = until {
            conditions.push("timestamp <= ?".to_string());
            params.push(Box::new(u));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(" ORDER BY timestamp DESC, id DESC");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let operation_data: String = row.get(4)?;
            let target_name = Self::extract_target_name(&operation_data);
            Ok(OperationSummary {
                operation_id: row.get(0)?,
                timestamp: row.get(1)?,
                device_id: row.get(2)?,
                operation_type: row.get(3)?,
                target_name,
            })
        })?;

        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row?);
        }
        Ok(summaries)
    }

    /// Deletes all operations from the log, returning the number of rows removed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the DELETE fails.
    pub fn purge_all(&self, conn: &Connection) -> Result<usize> {
        let count = conn.execute("DELETE FROM operations", [])?;
        Ok(count)
    }

    fn operation_type_name(&self, op: &Operation) -> &str {
        match op {
            Operation::CreateNote { .. } => "CreateNote",
            Operation::UpdateField { .. } => "UpdateField",
            Operation::DeleteNote { .. } => "DeleteNote",
            Operation::MoveNote { .. } => "MoveNote",
            Operation::CreateUserScript { .. } => "CreateUserScript",
            Operation::UpdateUserScript { .. } => "UpdateUserScript",
            Operation::DeleteUserScript { .. } => "DeleteUserScript",
        }
    }

    /// Extracts a human-readable target name from the operation's JSON data.
    ///
    /// Checks fields in order: `title`, `name`, `note_id`, `script_id`.
    /// Returns an empty string if none of these fields are present.
    fn extract_target_name(json: &str) -> String {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return String::new();
        };

        // CreateNote has "title"
        if let Some(title) = value.get("title").and_then(|v| v.as_str()) {
            return title.to_string();
        }
        // CreateUserScript / UpdateUserScript have "name"
        if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
            return name.to_string();
        }
        // UpdateField / DeleteNote / MoveNote have "note_id"
        if let Some(note_id) = value.get("note_id").and_then(|v| v.as_str()) {
            return note_id.to_string();
        }
        // DeleteUserScript has "script_id"
        if let Some(script_id) = value.get("script_id").and_then(|v| v.as_str()) {
            return script_id.to_string();
        }

        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

    #[test]
    fn test_log_and_purge() {
        let temp = NamedTempFile::new().unwrap();
        let mut storage = Storage::create(temp.path()).unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 5 });

        let tx = storage.connection_mut().transaction().unwrap();

        for i in 0..10 {
            let op = Operation::CreateNote {
                operation_id: format!("op-{}", i),
                timestamp: 1000 + i,
                device_id: "dev-1".to_string(),
                note_id: format!("note-{}", i),
                parent_id: None,
                position: i as i32,
                node_type: "TextNote".to_string(),
                title: format!("Note {}", i),
                fields: HashMap::new(),
                created_by: 0,
            };
            log.log(&tx, &op).unwrap();
        }

        log.purge_if_needed(&tx).unwrap();
        tx.commit().unwrap();

        let count: i64 = storage
            .connection()
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 5);
    }

    #[test]
    fn test_list_operations() {
        let temp = NamedTempFile::new().unwrap();
        let mut storage = Storage::create(temp.path()).unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Insert two operations with different types and timestamps.
        {
            let tx = storage.connection_mut().transaction().unwrap();

            let op1 = Operation::CreateNote {
                operation_id: "op-1".to_string(),
                timestamp: 1000,
                device_id: "dev-1".to_string(),
                note_id: "note-1".to_string(),
                parent_id: None,
                position: 0,
                node_type: "TextNote".to_string(),
                title: "My Note".to_string(),
                fields: HashMap::new(),
                created_by: 0,
            };
            log.log(&tx, &op1).unwrap();

            let op2 = Operation::CreateUserScript {
                operation_id: "op-2".to_string(),
                timestamp: 2000,
                device_id: "dev-1".to_string(),
                script_id: "script-1".to_string(),
                name: "My Script".to_string(),
                description: "A test script".to_string(),
                source_code: "print(42);".to_string(),
                load_order: 0,
                enabled: true,
            };
            log.log(&tx, &op2).unwrap();

            tx.commit().unwrap();
        }

        // List all — should return newest first.
        let all = log.list(storage.connection(), None, None, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].operation_id, "op-2"); // newest
        assert_eq!(all[1].operation_id, "op-1");

        // Verify target_name extraction.
        assert_eq!(all[0].target_name, "My Script"); // from "name" field
        assert_eq!(all[1].target_name, "My Note"); // from "title" field

        // Filter by type.
        let notes_only = log
            .list(storage.connection(), Some("CreateNote"), None, None)
            .unwrap();
        assert_eq!(notes_only.len(), 1);
        assert_eq!(notes_only[0].operation_id, "op-1");

        // Filter by since.
        let recent = log
            .list(storage.connection(), None, Some(1500), None)
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].operation_id, "op-2");
    }

    #[test]
    fn test_purge_all() {
        let temp = NamedTempFile::new().unwrap();
        let mut storage = Storage::create(temp.path()).unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        {
            let tx = storage.connection_mut().transaction().unwrap();
            for i in 0..5 {
                let op = Operation::CreateNote {
                    operation_id: format!("op-{}", i),
                    timestamp: 1000 + i,
                    device_id: "dev-1".to_string(),
                    note_id: format!("note-{}", i),
                    parent_id: None,
                    position: i as i32,
                    node_type: "TextNote".to_string(),
                    title: format!("Note {}", i),
                    fields: HashMap::new(),
                    created_by: 0,
                };
                log.log(&tx, &op).unwrap();
            }
            tx.commit().unwrap();
        }

        let count = log.purge_all(storage.connection()).unwrap();
        assert_eq!(count, 5);

        let remaining = log.list(storage.connection(), None, None, None).unwrap();
        assert!(remaining.is_empty());
    }
}
