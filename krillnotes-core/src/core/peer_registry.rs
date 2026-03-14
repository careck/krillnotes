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
    /// How this peer is reached: "manual", "relay", etc.
    pub channel_type: String,
    /// JSON-encoded channel-specific parameters (e.g. relay URL, token).
    pub channel_params: String,
    /// Current sync status: "idle", "syncing", "error".
    pub sync_status: String,
    /// Optional human-readable detail about the current status.
    pub sync_status_detail: Option<String>,
    /// Last sync error message, if any.
    pub last_sync_error: Option<String>,
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
    /// True if this peer's identity is the workspace owner.
    pub is_owner: bool,
    /// How this peer is reached: "manual", "relay", etc.
    pub channel_type: String,
    /// JSON-encoded channel configuration (e.g. `{"path":"/shared/folder"}`).
    pub channel_params: String,
    /// Current sync status: "idle", "syncing", "error".
    pub sync_status: String,
    /// Optional human-readable detail about the current status.
    pub sync_status_detail: Option<String>,
    /// Last sync error message, if any.
    pub last_sync_error: Option<String>,
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
            "SELECT peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync,
                    channel_type, channel_params, sync_status, sync_status_detail, last_sync_error
             FROM sync_peers WHERE peer_device_id = ?1",
        )?;
        let result = stmt.query_row(rusqlite::params![peer_device_id], |row| {
            Ok(SyncPeer {
                peer_device_id: row.get(0)?,
                peer_identity_id: row.get(1)?,
                last_sent_op: row.get(2)?,
                last_received_op: row.get(3)?,
                last_sync: row.get(4)?,
                channel_type: row.get(5)?,
                channel_params: row.get(6)?,
                sync_status: row.get(7)?,
                sync_status_detail: row.get(8)?,
                last_sync_error: row.get(9)?,
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
            "SELECT peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync,
                    channel_type, channel_params, sync_status, sync_status_detail, last_sync_error
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
                    channel_type: row.get(5)?,
                    channel_params: row.get(6)?,
                    sync_status: row.get(7)?,
                    sync_status_detail: row.get(8)?,
                    last_sync_error: row.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(peers)
    }

    /// List all peers with a specific channel type.
    pub fn list_peers_by_channel(&self, channel_type: &str) -> Result<Vec<SyncPeer>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync,
                    channel_type, channel_params, sync_status, sync_status_detail, last_sync_error
             FROM sync_peers WHERE channel_type = ?1",
        )?;
        let peers = stmt
            .query_map(rusqlite::params![channel_type], |row| {
                Ok(SyncPeer {
                    peer_device_id: row.get(0)?,
                    peer_identity_id: row.get(1)?,
                    last_sent_op: row.get(2)?,
                    last_received_op: row.get(3)?,
                    last_sync: row.get(4)?,
                    channel_type: row.get(5)?,
                    channel_params: row.get(6)?,
                    sync_status: row.get(7)?,
                    sync_status_detail: row.get(8)?,
                    last_sync_error: row.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(peers)
    }

    /// List all peers NOT using the specified channel type.
    pub fn list_peers_by_channel_not(&self, channel_type: &str) -> Result<Vec<SyncPeer>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync,
                    channel_type, channel_params, sync_status, sync_status_detail, last_sync_error
             FROM sync_peers WHERE channel_type != ?1",
        )?;
        let peers = stmt
            .query_map(rusqlite::params![channel_type], |row| {
                Ok(SyncPeer {
                    peer_device_id: row.get(0)?,
                    peer_identity_id: row.get(1)?,
                    last_sent_op: row.get(2)?,
                    last_received_op: row.get(3)?,
                    last_sync: row.get(4)?,
                    channel_type: row.get(5)?,
                    channel_params: row.get(6)?,
                    sync_status: row.get(7)?,
                    sync_status_detail: row.get(8)?,
                    last_sync_error: row.get(9)?,
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

    /// Upsert a peer when processing a received delta bundle.
    ///
    /// Uses `peer_identity_id` (not `peer_device_id`) as the merge key so that
    /// placeholder rows (`"identity:{pk}"`) created at snapshot-send time are
    /// consolidated with the real device ID learned from the incoming bundle.
    ///
    /// Preserves the best `last_sent_op` from any existing row for this identity,
    /// then replaces all rows for that identity with a single clean row keyed by
    /// the real device ID.
    pub fn upsert_peer_from_delta(
        &self,
        real_device_id: &str,
        peer_identity_id: &str,
        last_received_op: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        // Carry forward the best last_sent_op from any existing row.
        let existing_last_sent: Option<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT last_sent_op FROM sync_peers \
                 WHERE peer_identity_id = ?1 AND last_sent_op IS NOT NULL \
                 LIMIT 1",
            )?;
            match stmt.query_row([peer_identity_id], |row| row.get::<_, String>(0)) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(crate::KrillnotesError::Database(e)),
            }
        };

        // Carry forward channel config if peer already has one configured.
        let existing_channel: Option<(String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT channel_type, channel_params FROM sync_peers \
                 WHERE peer_identity_id = ?1 AND channel_type != 'manual' \
                 LIMIT 1",
            )?;
            match stmt.query_row([peer_identity_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(crate::KrillnotesError::Database(e)),
            }
        };

        let (channel_type, channel_params) = existing_channel
            .unwrap_or_else(|| ("manual".to_string(), "{}".to_string()));

        // Drop all placeholder / stale rows for this identity, then insert one clean row.
        self.conn.execute(
            "DELETE FROM sync_peers WHERE peer_identity_id = ?1",
            [peer_identity_id],
        )?;
        self.conn.execute(
            "INSERT INTO sync_peers \
                 (peer_device_id, peer_identity_id, last_sent_op, last_received_op, last_sync, \
                  channel_type, channel_params) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                real_device_id,
                peer_identity_id,
                existing_last_sent,
                last_received_op,
                now,
                channel_type,
                channel_params,
            ],
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

    /// Update the channel configuration for a peer.
    pub fn update_channel_config(
        &self,
        peer_device_id: &str,
        channel_type: &str,
        channel_params: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_peers SET channel_type = ?1, channel_params = ?2 WHERE peer_device_id = ?3",
            rusqlite::params![channel_type, channel_params, peer_device_id],
        )?;
        Ok(())
    }

    /// Update the sync status for a peer.
    pub fn update_sync_status(
        &self,
        peer_device_id: &str,
        sync_status: &str,
        sync_status_detail: Option<&str>,
        last_sync_error: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_peers SET sync_status = ?1, sync_status_detail = ?2, last_sync_error = ?3 WHERE peer_device_id = ?4",
            rusqlite::params![sync_status, sync_status_detail, last_sync_error, peer_device_id],
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

    #[test]
    fn test_peer_registry_channel_fields() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        let peer_id = "test-device-id";
        let identity_id = "test-identity-key";

        // Add peer with default channel (manual)
        reg.add_peer(peer_id, identity_id).unwrap();

        // Read back — should have default channel_type = manual
        let peer = reg.get_peer(peer_id).unwrap().unwrap();
        assert_eq!(peer.channel_type, "manual");
        assert_eq!(peer.channel_params, "{}");
        assert_eq!(peer.sync_status, "idle");
        assert!(peer.sync_status_detail.is_none());
        assert!(peer.last_sync_error.is_none());
    }

    #[test]
    fn test_update_channel_config() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("dev-1", "pk-1").unwrap();
        reg.update_channel_config("dev-1", "relay", r#"{"url":"https://relay.example.com"}"#).unwrap();
        let peer = reg.get_peer("dev-1").unwrap().unwrap();
        assert_eq!(peer.channel_type, "relay");
        assert_eq!(peer.channel_params, r#"{"url":"https://relay.example.com"}"#);
    }

    #[test]
    fn test_update_sync_status() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("dev-1", "pk-1").unwrap();
        reg.update_sync_status("dev-1", "error", Some("connection refused"), Some("TLS handshake failed")).unwrap();
        let peer = reg.get_peer("dev-1").unwrap().unwrap();
        assert_eq!(peer.sync_status, "error");
        assert_eq!(peer.sync_status_detail.as_deref(), Some("connection refused"));
        assert_eq!(peer.last_sync_error.as_deref(), Some("TLS handshake failed"));
    }

    #[test]
    fn test_list_peers_by_channel() {
        let (_f, s) = open_db();
        let reg = PeerRegistry::new(s.connection());
        reg.add_peer("dev-1", "pk-1").unwrap();
        reg.add_peer("dev-2", "pk-2").unwrap();
        reg.update_channel_config("dev-2", "relay", "{}").unwrap();

        let manual_peers = reg.list_peers_by_channel("manual").unwrap();
        assert_eq!(manual_peers.len(), 1);
        assert_eq!(manual_peers[0].peer_device_id, "dev-1");

        let relay_peers = reg.list_peers_by_channel("relay").unwrap();
        assert_eq!(relay_peers.len(), 1);
        assert_eq!(relay_peers[0].peer_device_id, "dev-2");

        let non_manual = reg.list_peers_by_channel_not("manual").unwrap();
        assert_eq!(non_manual.len(), 1);
        assert_eq!(non_manual[0].peer_device_id, "dev-2");
    }
}
