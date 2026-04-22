// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Sync event logging — persistent audit trail for sync security failures.

use crate::core::error::Result;
use crate::core::workspace::Workspace;
use chrono::Utc;
use serde::Serialize;

/// A single record from the `sync_events` audit table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEventRecord {
    pub id: i64,
    pub timestamp: i64,
    pub peer_pubkey: String,
    pub event_type: String,
    pub detail: Option<String>,
}

impl Workspace {
    /// Appends a sync security event to the persistent audit log.
    pub fn log_sync_event(
        &self,
        peer_pubkey: &str,
        event_type: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        let ts = Utc::now().timestamp();
        self.storage.connection().execute(
            "INSERT INTO sync_events (timestamp, peer_pubkey, event_type, detail) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, peer_pubkey, event_type, detail],
        )?;
        Ok(())
    }

    /// Returns sync events ordered most-recent-first with pagination.
    pub fn list_sync_events(&self, limit: i64, offset: i64) -> Result<Vec<SyncEventRecord>> {
        let conn = self.storage.connection();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, peer_pubkey, event_type, detail
             FROM sync_events
             ORDER BY id DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit, offset], |row| {
            Ok(SyncEventRecord {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                peer_pubkey: row.get(2)?,
                event_type: row.get(3)?,
                detail: row.get(4)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::permission::AllowAllGate;
    use tempfile::NamedTempFile;

    fn make_ws() -> (Workspace, NamedTempFile) {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(
            temp.path(),
            "",
            "test-identity",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
            Box::new(AllowAllGate::new("test")),
            None,
        )
        .expect("workspace");
        (ws, temp)
    }

    #[test]
    fn test_log_and_list_sync_events() {
        let (ws, _tmp) = make_ws();
        ws.log_sync_event("pubkey-a", "sig_invalid", Some("bad signature"))
            .unwrap();
        ws.log_sync_event("pubkey-b", "sig_invalid", None).unwrap();
        ws.log_sync_event("pubkey-c", "unknown_peer", Some("detail c"))
            .unwrap();

        let events = ws.list_sync_events(100, 0).unwrap();
        // The workspace is created with a root note — focus only on events we inserted.
        assert_eq!(events.len(), 3);
        // Most recent first (highest id first)
        assert_eq!(events[0].peer_pubkey, "pubkey-c");
        assert_eq!(events[1].peer_pubkey, "pubkey-b");
        assert_eq!(events[2].peer_pubkey, "pubkey-a");
        assert_eq!(events[2].detail, Some("bad signature".to_string()));
        assert_eq!(events[1].detail, None);
    }

    #[test]
    fn test_list_sync_events_pagination() {
        let (ws, _tmp) = make_ws();
        for i in 0..5 {
            ws.log_sync_event("pubkey", "test_event", Some(&format!("detail {i}")))
                .unwrap();
        }

        let page0 = ws.list_sync_events(2, 0).unwrap();
        assert_eq!(page0.len(), 2);

        let page1 = ws.list_sync_events(2, 2).unwrap();
        assert_eq!(page1.len(), 2);

        let page2 = ws.list_sync_events(2, 4).unwrap();
        assert_eq!(page2.len(), 1);

        // Ordering: id DESC, so page0[0] has the highest id
        assert!(page0[0].id > page0[1].id);
        assert!(page1[0].id > page1[1].id);
        assert!(page0[1].id > page1[0].id);
    }

    #[test]
    fn test_list_sync_events_empty() {
        let (ws, _tmp) = make_ws();
        let events = ws.list_sync_events(100, 0).unwrap();
        assert!(events.is_empty());
    }
}
