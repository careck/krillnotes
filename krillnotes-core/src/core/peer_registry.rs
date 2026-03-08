// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Per-workspace sync peer registry.
//!
//! Tracks devices we directly exchange `.swarm` bundles with.
//! Display names are resolved via the cross-workspace contacts address book
//! (matching `peer_identity_id` to a `Contact.public_key`).

use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::Result;

/// One row in the `sync_peers` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPeer {
    pub peer_device_id: String,
    /// Ed25519 public key (base64) — references the contact record.
    pub peer_identity_id: String,
    /// operation_id of the last operation we sent to this peer.
    pub last_sent_op: Option<String>,
    /// operation_id of the last operation we received from this peer.
    pub last_received_op: Option<String>,
    /// ISO 8601 timestamp of the last bundle exchange.
    pub last_sync: Option<String>,
}

/// Manages the `sync_peers` table for one workspace connection.
pub struct PeerRegistry<'a> {
    conn: &'a Connection,
}

impl<'a> PeerRegistry<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Register a new sync peer. Does nothing if the device_id already exists.
    pub fn add_peer(&self, peer_device_id: &str, peer_identity_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sync_peers (peer_device_id, peer_identity_id) VALUES (?1, ?2)",
            rusqlite::params![peer_device_id, peer_identity_id],
        )?;
        Ok(())
    }

    /// Retrieve a peer by device ID.
    pub fn get_peer(&self, peer_device_id: &str) -> Result<Option<SyncPeer>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync
             FROM sync_peers WHERE peer_device_id = ?1",
        )?;
        let result = stmt.query_row(rusqlite::params![peer_device_id], |row| {
            Ok(SyncPeer {
                peer_device_id: row.get(0)?,
                peer_identity_id: row.get(1)?,
                last_sent_op: row.get(2)?,
                last_received_op: row.get(3)?,
                last_sync: row.get(4)?,
            })
        });
        match result {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all registered peers.
    pub fn list_peers(&self) -> Result<Vec<SyncPeer>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync
             FROM sync_peers",
        )?;
        let peers = stmt
            .query_map([], |row| {
                Ok(SyncPeer {
                    peer_device_id: row.get(0)?,
                    peer_identity_id: row.get(1)?,
                    last_sent_op: row.get(2)?,
                    last_received_op: row.get(3)?,
                    last_sync: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(peers)
    }

    /// Update the last-sent operation marker for a peer.
    pub fn update_last_sent(&self, peer_device_id: &str, operation_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sync_peers SET last_sent_op = ?1, last_sync = ?2 WHERE peer_device_id = ?3",
            rusqlite::params![operation_id, now, peer_device_id],
        )?;
        Ok(())
    }

    /// Update the last-received operation marker for a peer.
    pub fn update_last_received(&self, peer_device_id: &str, operation_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sync_peers SET last_received_op = ?1, last_sync = ?2 WHERE peer_device_id = ?3",
            rusqlite::params![operation_id, now, peer_device_id],
        )?;
        Ok(())
    }

    /// Remove a peer from the registry.
    pub fn remove_peer(&self, peer_device_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM sync_peers WHERE peer_device_id = ?1",
            rusqlite::params![peer_device_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::Storage;

    fn open_db() -> (NamedTempFile, Storage) {
        let f = NamedTempFile::new().unwrap();
        let s = Storage::create(f.path(), "").unwrap();
        (f, s)
    }

    #[test]
    fn test_add_and_get_peer() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("device-abc", "pubkey-alice").unwrap();
        let peer = reg.get_peer("device-abc").unwrap().unwrap();
        assert_eq!(peer.peer_device_id, "device-abc");
        assert_eq!(peer.peer_identity_id, "pubkey-alice");
        assert!(peer.last_sent_op.is_none());
    }

    #[test]
    fn test_update_last_sent() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("device-abc", "pubkey-alice").unwrap();
        reg.update_last_sent("device-abc", "op-uuid-1").unwrap();
        let peer = reg.get_peer("device-abc").unwrap().unwrap();
        assert_eq!(peer.last_sent_op.as_deref(), Some("op-uuid-1"));
    }

    #[test]
    fn test_update_last_received() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("device-abc", "pubkey-alice").unwrap();
        reg.update_last_received("device-abc", "op-uuid-2").unwrap();
        let peer = reg.get_peer("device-abc").unwrap().unwrap();
        assert_eq!(peer.last_received_op.as_deref(), Some("op-uuid-2"));
    }

    #[test]
    fn test_list_peers() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("dev-1", "pk-1").unwrap();
        reg.add_peer("dev-2", "pk-2").unwrap();
        assert_eq!(reg.list_peers().unwrap().len(), 2);
    }

    #[test]
    fn test_remove_peer() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("dev-1", "pk-1").unwrap();
        reg.remove_peer("dev-1").unwrap();
        assert!(reg.get_peer("dev-1").unwrap().is_none());
    }
}
