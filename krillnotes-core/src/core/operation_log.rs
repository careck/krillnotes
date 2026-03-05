// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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
    /// HLC wall clock timestamp in Unix milliseconds.
    pub timestamp_wall_ms: u64,
    pub device_id: String,
    pub operation_type: String,
    pub target_name: String,
    /// First 8 characters of the base64 public key of the operation author,
    /// or an empty string if the operation has no author (e.g. `RetractOperation`).
    pub author_key: String,
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
        let ts = op.timestamp();

        tx.execute(
            "INSERT INTO operations (operation_id, timestamp_wall_ms, timestamp_counter, timestamp_node_id, device_id, operation_type, operation_data, synced)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0)",
            rusqlite::params![
                op.operation_id(),
                ts.wall_ms as i64,
                ts.counter as i64,
                ts.node_id as i64,
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
                    "DELETE FROM operations WHERE operation_id NOT IN (
                        SELECT operation_id FROM operations
                        ORDER BY timestamp_wall_ms DESC, timestamp_counter DESC LIMIT ?
                    )",
                    [keep_last as i64],
                )?;
            }
            PurgeStrategy::WithSync { retention_days } => {
                // Convert retention cutoff from Unix seconds to wall_ms (milliseconds).
                let cutoff_ms = (chrono::Utc::now().timestamp()
                    - (retention_days as i64 * SECONDS_PER_DAY))
                    * 1000;
                tx.execute(
                    "DELETE FROM operations WHERE synced = 1 AND timestamp_wall_ms < ?",
                    [cutoff_ms],
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
            "SELECT operation_id, timestamp_wall_ms, device_id, operation_type, operation_data FROM operations",
        );
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(t) = type_filter {
            conditions.push("operation_type = ?".to_string());
            params.push(Box::new(t.to_string()));
        }
        if let Some(s) = since {
            // `since` is in milliseconds (HLC wall_ms scale).
            conditions.push("timestamp_wall_ms >= ?".to_string());
            params.push(Box::new(s));
        }
        if let Some(u) = until {
            // `until` is in milliseconds (HLC wall_ms scale).
            conditions.push("timestamp_wall_ms <= ?".to_string());
            params.push(Box::new(u));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(
            " ORDER BY timestamp_wall_ms DESC, timestamp_counter DESC, timestamp_node_id DESC, operation_id DESC",
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let wall_ms_raw: i64 = row.get(1)?;
            let operation_data: String = row.get(4)?;
            let target_name = Self::extract_target_name(&operation_data);
            let author_key = Self::extract_author_key(&operation_data);
            Ok(OperationSummary {
                operation_id: row.get(0)?,
                timestamp_wall_ms: wall_ms_raw.max(0) as u64,
                device_id: row.get(2)?,
                operation_type: row.get(3)?,
                target_name,
                author_key,
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
            Operation::UpdateNote { .. } => "UpdateNote",
            Operation::UpdateField { .. } => "UpdateField",
            Operation::DeleteNote { .. } => "DeleteNote",
            Operation::MoveNote { .. } => "MoveNote",
            Operation::SetTags { .. } => "SetTags",
            Operation::CreateUserScript { .. } => "CreateUserScript",
            Operation::UpdateUserScript { .. } => "UpdateUserScript",
            Operation::DeleteUserScript { .. } => "DeleteUserScript",
            Operation::RetractOperation { .. } => "RetractOperation",
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

    /// Extracts the first 8 characters of the base64 author public key from the
    /// operation's JSON data.
    ///
    /// Checks fields in order: `created_by`, `modified_by`, `deleted_by`, `moved_by`.
    /// Returns an empty string if none of these fields are present or non-empty
    /// (e.g. `RetractOperation`).
    fn extract_author_key(json: &str) -> String {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return String::new();
        };

        for field in &["created_by", "modified_by", "deleted_by", "moved_by"] {
            if let Some(key) = value.get(field).and_then(|v| v.as_str()) {
                if !key.is_empty() {
                    return key.to_string();
                }
            }
        }

        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hlc::HlcTimestamp;
    use crate::Storage;
    use std::collections::{BTreeMap, HashMap};
    use tempfile::NamedTempFile;

    fn ts(wall_ms: u64) -> HlcTimestamp {
        HlcTimestamp { wall_ms, counter: 0, node_id: 0 }
    }

    #[test]
    fn test_log_and_purge() {
        let temp = NamedTempFile::new().unwrap();
        let mut storage = Storage::create(temp.path(), "").unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 5 });

        let tx = storage.connection_mut().transaction().unwrap();

        for i in 0..10u64 {
            let op = Operation::CreateNote {
                operation_id: format!("op-{}", i),
                timestamp: ts(1_000_000 + i),
                device_id: "dev-1".to_string(),
                note_id: format!("note-{}", i),
                parent_id: None,
                position: i as f64,
                node_type: "TextNote".to_string(),
                title: format!("Note {}", i),
                fields: BTreeMap::new(),
                created_by: String::new(),
                signature: String::new(),
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
        let mut storage = Storage::create(temp.path(), "").unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Insert two operations with different types and timestamps.
        {
            let tx = storage.connection_mut().transaction().unwrap();

            let op1 = Operation::CreateNote {
                operation_id: "op-1".to_string(),
                timestamp: ts(1_000_000),
                device_id: "dev-1".to_string(),
                note_id: "note-1".to_string(),
                parent_id: None,
                position: 0.0,
                node_type: "TextNote".to_string(),
                title: "My Note".to_string(),
                fields: BTreeMap::new(),
                created_by: String::new(),
                signature: String::new(),
            };
            log.log(&tx, &op1).unwrap();

            let op2 = Operation::CreateUserScript {
                operation_id: "op-2".to_string(),
                timestamp: ts(2_000_000),
                device_id: "dev-1".to_string(),
                script_id: "script-1".to_string(),
                name: "My Script".to_string(),
                description: "A test script".to_string(),
                source_code: "print(42);".to_string(),
                load_order: 0,
                enabled: true,
                created_by: String::new(),
                signature: String::new(),
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

        // Filter by since (timestamp_wall_ms is in milliseconds).
        // op1 stored at wall_ms=1_000_000, op2 at wall_ms=2_000_000.
        let recent = log
            .list(storage.connection(), None, Some(1_500_000), None)
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].operation_id, "op-2");
    }

    #[test]
    fn test_purge_all() {
        let temp = NamedTempFile::new().unwrap();
        let mut storage = Storage::create(temp.path(), "").unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        {
            let tx = storage.connection_mut().transaction().unwrap();
            for i in 0..5u64 {
                let op = Operation::CreateNote {
                    operation_id: format!("op-{}", i),
                    timestamp: ts(1_000_000 + i),
                    device_id: "dev-1".to_string(),
                    note_id: format!("note-{}", i),
                    parent_id: None,
                    position: i as f64,
                    node_type: "TextNote".to_string(),
                    title: format!("Note {}", i),
                    fields: BTreeMap::new(),
                    created_by: String::new(),
                    signature: String::new(),
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
