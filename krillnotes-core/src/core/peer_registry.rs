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

/// A resolved view of a sync peer, joining sync_peers with the contact book.
/// This is the type returned to callers (Tauri, future frontends).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    /// Raw device ID from sync_peers (PRIMARY KEY). May be `identity:<pubkey>` for
    /// pre-authorised contacts who have never synced yet.
    pub peer_device_id: String,
    /// Ed25519 public key (base64) — the peer's identity.
    pub peer_identity_id: String,
    /// Resolved display name: local_name || declared_name || first 8 chars of key + "…"
    pub display_name: String,
    /// 4-word BIP-39 fingerprint derived from BLAKE3(peer_identity_id).
    pub fingerprint: String,
    /// Trust level string if peer is in the contact book ("Tofu", "CodeVerified",
    /// "Vouched", "VerifiedInPerson"). None if not in contacts.
    pub trust_level: Option<String>,
    /// Contact UUID (as String) if peer is in the contact book. None otherwise.
    pub contact_id: Option<String>,
    /// ISO 8601 timestamp of last .swarm bundle exchange. None if never synced.
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

    /// Update the last-sent operation marker for a peer (peer row must already exist).
    pub fn update_last_sent(&self, peer_device_id: &str, operation_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sync_peers SET last_sent_op = ?1, last_sync = ?2 WHERE peer_device_id = ?3",
            rusqlite::params![operation_id, now, peer_device_id],
        )?;
        Ok(())
    }

    /// Upsert last-sent marker: inserts a peer row if absent, always updates last_sent_op.
    ///
    /// Use this when the peer may not yet have a row (e.g. after first snapshot send).
    pub fn upsert_last_sent(
        &self,
        peer_device_id: &str,
        peer_identity_id: &str,
        operation_id: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sync_peers (peer_device_id, peer_identity_id, last_sent_op, last_sync)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(peer_device_id) DO UPDATE SET
                 last_sent_op = excluded.last_sent_op,
                 last_sync = excluded.last_sync",
            rusqlite::params![peer_device_id, peer_identity_id, operation_id, now],
        )?;
        Ok(())
    }

    /// Insert or update a sync peer row with optional watermark fields.
    ///
    /// On conflict, only non-null incoming values overwrite existing ones
    /// (preserves existing watermarks when called with `None`).
    pub fn upsert_sync_peer(
        &self,
        peer_device_id: &str,
        peer_identity_id: &str,
        last_sent_op: Option<&str>,
        last_received_op: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sync_peers (peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(peer_device_id) DO UPDATE SET
                 peer_identity_id = excluded.peer_identity_id,
                 last_sent_op = COALESCE(excluded.last_sent_op, last_sent_op),
                 last_received_op = COALESCE(excluded.last_received_op, last_received_op),
                 last_sync = excluded.last_sync",
            rusqlite::params![peer_device_id, peer_identity_id, last_sent_op, last_received_op, now],
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
