//! Durable operation log and purge strategies for the Krillnotes workspace.

use crate::{Operation, Result};
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
                self.extract_device_id(op),
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

    fn extract_device_id<'a>(&self, op: &'a Operation) -> &'a str {
        match op {
            Operation::CreateNote { device_id, .. } => device_id,
            Operation::UpdateField { device_id, .. } => device_id,
            Operation::DeleteNote { device_id, .. } => device_id,
            Operation::MoveNote { device_id, .. } => device_id,
        }
    }

    fn operation_type_name(&self, op: &Operation) -> &str {
        match op {
            Operation::CreateNote { .. } => "CreateNote",
            Operation::UpdateField { .. } => "UpdateField",
            Operation::DeleteNote { .. } => "DeleteNote",
            Operation::MoveNote { .. } => "MoveNote",
        }
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
}
