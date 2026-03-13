// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Snapshot sync, delta operations, and peer registry management.

use super::*;

impl Workspace {
    // ── Snapshot (peer sync) ───────────────────────────────────────

    /// Serialise all notes, user scripts, and attachment metadata to JSON bytes for a snapshot bundle.
    pub fn to_snapshot_json(&self) -> Result<Vec<u8>> {
        let notes = self.list_all_notes()?;
        let user_scripts = self.list_user_scripts()?;
        let attachments = self.list_all_attachments()?;
        let snapshot = WorkspaceSnapshot {
            version: 1,
            notes,
            user_scripts,
            attachments,
        };
        Ok(serde_json::to_vec(&snapshot)?)
    }

    /// Returns the `operation_id` of the most recent logged operation, or `None` if log is empty.
    pub fn get_latest_operation_id(&self) -> Result<Option<String>> {
        let conn = self.storage.connection();
        let mut stmt = conn.prepare(
            "SELECT operation_id FROM operations ORDER BY timestamp_wall_ms DESC, timestamp_counter DESC LIMIT 1"
        )?;
        match stmt.query_row([], |row| row.get::<_, String>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(KrillnotesError::Database(e)),
        }
    }

    /// Returns all operations in HLC order that occurred strictly after `since_op_id`,
    /// excluding operations from `exclude_device_id` (echo prevention).
    ///
    /// Used by `swarm::sync::generate_delta` to build the operation list for a delta bundle.
    /// `RetractOperation { propagate: false }` is filtered out (local-only undo markers).
    ///
    /// If `since_op_id` is `None`, all operations except those from `exclude_device_id`
    /// are returned (used when peer has no watermark set).
    pub fn operations_since(
        &self,
        since_op_id: Option<&str>,
        exclude_device_id: &str,
    ) -> Result<Vec<Operation>> {
        let conn = self.storage.connection();

        let op_jsons: Vec<String> = if let Some(op_id) = since_op_id {
            // Look up HLC tuple for the watermark operation.
            let hlc_row: Option<(i64, i64, i64)> = conn.query_row(
                "SELECT timestamp_wall_ms, timestamp_counter, timestamp_node_id \
                 FROM operations WHERE operation_id = ?1",
                [op_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional().map_err(KrillnotesError::Database)?;

            if let Some((wall_ms, counter, node_id)) = hlc_row {
                // Three-column strictly-greater comparison (single-column > would silently
                // drop ops that share the same wall_ms as the watermark).
                let mut stmt = conn.prepare(
                    "SELECT operation_data FROM operations \
                     WHERE ((timestamp_wall_ms > ?1) \
                        OR  (timestamp_wall_ms = ?1 AND timestamp_counter > ?2) \
                        OR  (timestamp_wall_ms = ?1 AND timestamp_counter = ?2 \
                             AND timestamp_node_id > ?3)) \
                     AND device_id != ?4 \
                     ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                              timestamp_node_id ASC",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![wall_ms, counter, node_id, exclude_device_id],
                    |row| row.get::<_, String>(0),
                )?.collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?;
                rows
            } else {
                // Watermark op not in this workspace's log (e.g. freshly imported
                // from a snapshot whose operations were never inserted locally).
                // Fall back to sending everything — the recipient's INSERT OR IGNORE
                // handles any duplicates safely.
                let mut stmt = conn.prepare(
                    "SELECT operation_data FROM operations WHERE device_id != ?1 \
                     ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                              timestamp_node_id ASC",
                )?;
                let rows = stmt.query_map([exclude_device_id], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?;
                rows
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT operation_data FROM operations WHERE device_id != ?1 \
                 ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                          timestamp_node_id ASC",
            )?;
            let rows = stmt.query_map([exclude_device_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?;
            rows
        };

        let mut ops: Vec<Operation> = op_jsons
            .iter()
            .filter_map(|json| serde_json::from_str(json).ok())
            .collect();

        // Filter local-only retracts (propagate = false) in Rust
        // (the propagate flag is inside the JSON blob, not a SQL column).
        ops.retain(|op| !matches!(op, Operation::RetractOperation { propagate: false, .. }));

        Ok(ops)
    }

    /// Apply a single operation received from a remote peer.
    ///
    /// Returns `Ok(true)` if the operation was inserted and applied to the working tables,
    /// or `Ok(false)` if it was skipped (duplicate or local-only retract).
    ///
    /// Idempotent: calling this twice with the same operation is safe — the second call
    /// returns `Ok(false)` without modifying any data.
    pub fn apply_incoming_operation(&mut self, op: Operation) -> Result<bool> {
        // 1. Skip local-only retracts — they must never cross device boundaries.
        if matches!(op, Operation::RetractOperation { propagate: false, .. }) {
            return Ok(false);
        }

        // 2. Advance the local HLC by observing the incoming timestamp.
        self.hlc.observe(op.timestamp());

        // 3. Insert into the operations log with synced = 1.
        //    INSERT OR IGNORE gives 0 changed rows if the operation_id already exists.
        let op_json = serde_json::to_string(&op)?;
        let ts = op.timestamp();
        let op_type = Self::operation_type_str(&op);

        let rows = {
            let tx = self.storage.connection_mut().transaction()?;
            let rows = tx.execute(
                "INSERT OR IGNORE INTO operations \
                 (operation_id, timestamp_wall_ms, timestamp_counter, timestamp_node_id, \
                  device_id, operation_type, operation_data, synced) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 1)",
                rusqlite::params![
                    op.operation_id(),
                    ts.wall_ms as i64,
                    ts.counter as i64,
                    ts.node_id as i64,
                    op.device_id(),
                    op_type,
                    op_json,
                ],
            )?;
            tx.commit()?;
            rows
        };

        // 4. Duplicate — already applied.
        if rows == 0 {
            return Ok(false);
        }

        // 5. Apply the state change to working tables.
        let tx = self.storage.connection_mut().transaction()?;
        match &op {
            Operation::CreateNote {
                note_id, title, schema, parent_id, position,
                created_by, fields, ..
            } => {
                let fields_json = serde_json::to_string(fields)?;
                let now_ms = ts.wall_ms as i64;
                tx.execute(
                    "INSERT OR IGNORE INTO notes \
                     (id, title, schema, parent_id, position, created_at, modified_at, \
                      created_by, modified_by, fields_json, is_expanded, schema_version) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 1)",
                    rusqlite::params![
                        note_id, title, schema, parent_id, position,
                        now_ms, now_ms, created_by, created_by, fields_json,
                    ],
                )?;
            }

            Operation::UpdateNote { note_id, title, .. } => {
                let now_ms = ts.wall_ms as i64;
                tx.execute(
                    "UPDATE notes SET title = ?1, modified_at = ?2 WHERE id = ?3",
                    rusqlite::params![title, now_ms, note_id],
                )?;
            }

            Operation::UpdateField { note_id, field, value, modified_by, .. } => {
                // Read-modify-write the fields_json blob.
                let fields_json: Option<String> = tx.query_row(
                    "SELECT fields_json FROM notes WHERE id = ?1",
                    [note_id],
                    |row| row.get(0),
                ).optional().map_err(KrillnotesError::Database)?;

                if let Some(json) = fields_json {
                    let mut map: std::collections::BTreeMap<String, crate::FieldValue> =
                        serde_json::from_str(&json).unwrap_or_default();
                    map.insert(field.clone(), value.clone());
                    let new_json = serde_json::to_string(&map)?;
                    let now_ms = ts.wall_ms as i64;
                    tx.execute(
                        "UPDATE notes SET fields_json = ?1, modified_at = ?2, modified_by = ?3 WHERE id = ?4",
                        rusqlite::params![new_json, now_ms, modified_by, note_id],
                    )?;
                }
            }

            Operation::DeleteNote { note_id, .. } => {
                tx.execute(
                    "DELETE FROM notes WHERE id = ?1",
                    [note_id],
                )?;
            }

            Operation::MoveNote { note_id, new_parent_id, new_position, .. } => {
                tx.execute(
                    "UPDATE notes SET parent_id = ?1, position = ?2 WHERE id = ?3",
                    rusqlite::params![new_parent_id, new_position, note_id],
                )?;
            }

            Operation::SetTags { note_id, tags, .. } => {
                tx.execute(
                    "DELETE FROM note_tags WHERE note_id = ?1",
                    [note_id],
                )?;
                for tag in tags {
                    tx.execute(
                        "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?, ?)",
                        rusqlite::params![note_id, tag],
                    )?;
                }
            }

            Operation::CreateUserScript {
                script_id, name, description, source_code, load_order, enabled, ..
            } => {
                let now_ms = ts.wall_ms as i64;
                tx.execute(
                    "INSERT OR IGNORE INTO user_scripts \
                     (id, name, description, source_code, load_order, enabled, \
                      created_at, modified_at, category) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'user')",
                    rusqlite::params![
                        script_id, name, description, source_code,
                        load_order, *enabled as i32, now_ms, now_ms,
                    ],
                )?;
            }

            Operation::UpdateUserScript {
                script_id, name, description, source_code, load_order, enabled, ..
            } => {
                let now_ms = ts.wall_ms as i64;
                tx.execute(
                    "UPDATE user_scripts SET name = ?1, description = ?2, source_code = ?3, \
                     load_order = ?4, enabled = ?5, modified_at = ?6 WHERE id = ?7",
                    rusqlite::params![
                        name, description, source_code,
                        load_order, *enabled as i32, now_ms, script_id,
                    ],
                )?;
            }

            Operation::DeleteUserScript { script_id, .. } => {
                tx.execute(
                    "DELETE FROM user_scripts WHERE id = ?1",
                    [script_id],
                )?;
            }

            // Log-only variants — no working table change in this phase.
            Operation::JoinWorkspace { .. }
            | Operation::UpdateSchema { .. }
            | Operation::RetractOperation { .. }
            | Operation::SetPermission { .. }
            | Operation::RevokePermission { .. } => {}
        }
        tx.commit()?;

        Ok(true)
    }

    /// Returns the `operation_type` string for a given `Operation` variant.
    fn operation_type_str(op: &Operation) -> &'static str {
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
            Operation::UpdateSchema { .. } => "UpdateSchema",
            Operation::RetractOperation { .. } => "RetractOperation",
            Operation::SetPermission { .. } => "SetPermission",
            Operation::RevokePermission { .. } => "RevokePermission",
            Operation::JoinWorkspace { .. } => "JoinWorkspace",
        }
    }

    /// Populate a workspace from snapshot JSON bytes.
    ///
    /// Notes and user scripts are inserted. Returns the number of notes imported.
    /// Designed for freshly created workspaces — duplicates will be skipped via INSERT OR IGNORE.
    pub fn import_snapshot_json(&mut self, data: &[u8]) -> Result<usize> {
        let snapshot: WorkspaceSnapshot = serde_json::from_slice(data)
            .map_err(|e| KrillnotesError::Json(e))?;

        let note_count = snapshot.notes.len();

        // Bulk-insert notes preserving original IDs.
        // Defer foreign-key checks so children can be inserted before parents.
        {
            self.storage
                .connection_mut()
                .execute_batch("PRAGMA defer_foreign_keys = ON;")?;
            let tx = self.storage.connection_mut().transaction()?;
            for note in &snapshot.notes {
                let fields_json = serde_json::to_string(&note.fields)?;
                tx.execute(
                    "INSERT OR IGNORE INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        note.id,
                        note.title,
                        note.schema,
                        note.parent_id,
                        note.position,
                        note.created_at,
                        note.modified_at,
                        note.created_by,
                        note.modified_by,
                        fields_json,
                        note.is_expanded,
                        note.schema_version,
                    ],
                )?;
                for tag in &note.tags {
                    tx.execute(
                        "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?, ?)",
                        rusqlite::params![note.id, tag],
                    )?;
                }
            }
            tx.commit()?;
        }

        // Insert user scripts (preserve original IDs via INSERT OR IGNORE).
        if !snapshot.user_scripts.is_empty() {
            let tx = self.storage.connection_mut().transaction()?;
            for script in &snapshot.user_scripts {
                tx.execute(
                    "INSERT OR IGNORE INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        script.id,
                        script.name,
                        script.description,
                        script.source_code,
                        script.load_order,
                        script.enabled,
                        script.created_at,
                        script.modified_at,
                        script.category,
                    ],
                )?;
            }
            tx.commit()?;
        }

        Ok(note_count)
    }

    /// Returns a resolved view of all sync peers for this workspace, joining
    /// sync_peers with the given contact manager for name/trust resolution.
    /// Sorted by display_name ascending.
    pub fn list_peers_info(
        &self,
        contact_manager: &crate::core::contact::ContactManager,
    ) -> Result<Vec<PeerInfo>> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        let peers = registry.list_peers()?;
        let contacts = contact_manager.list_contacts()?;

        let mut result: Vec<PeerInfo> = peers
            .into_iter()
            .map(|peer| {
                let contact = contacts
                    .iter()
                    .find(|c| c.public_key == peer.peer_identity_id);

                let display_name = contact
                    .map(|c| c.local_name.clone().unwrap_or_else(|| c.declared_name.clone()))
                    .unwrap_or_else(|| {
                        let key = &peer.peer_identity_id;
                        format!("{}…", &key[..key.len().min(8)])
                    });

                let fingerprint = generate_fingerprint(&peer.peer_identity_id)
                    .unwrap_or_else(|_| format!("{}…", &peer.peer_identity_id[..peer.peer_identity_id.len().min(8)]));

                let trust_level = contact.map(|c| match c.trust_level {
                    TrustLevel::Tofu => "Tofu".to_string(),
                    TrustLevel::CodeVerified => "CodeVerified".to_string(),
                    TrustLevel::Vouched => "Vouched".to_string(),
                    TrustLevel::VerifiedInPerson => "VerifiedInPerson".to_string(),
                });

                PeerInfo {
                    peer_device_id: peer.peer_device_id,
                    peer_identity_id: peer.peer_identity_id,
                    display_name,
                    fingerprint,
                    trust_level,
                    contact_id: contact.map(|c| c.contact_id.to_string()),
                    last_sync: peer.last_sync,
                }
            })
            .collect();

        result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(result)
    }

    /// Pre-authorises a contact as a workspace sync peer before any .swarm exchange.
    /// Uses `identity:<peer_identity_id>` as a placeholder device ID.
    pub fn add_contact_as_peer(
        &self,
        peer_identity_id: &str,
    ) -> Result<()> {
        let placeholder_device_id = format!("identity:{}", peer_identity_id);
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        registry.add_peer(&placeholder_device_id, peer_identity_id)
    }

    /// Removes a peer from this workspace's sync peer list by device ID.
    pub fn remove_peer(
        &self,
        peer_device_id: &str,
    ) -> Result<()> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        registry.remove_peer(peer_device_id)
    }

    /// Update last_sent_op for a peer identified by their identity public key.
    /// Peers added via invite use placeholder device_id = "identity:<pubkey>".
    /// Uses upsert semantics: inserts a peer row if none exists yet.
    pub fn update_peer_last_sent_by_identity(&self, identity_pk: &str, op_id: &str) -> Result<()> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        let placeholder_device_id = format!("identity:{identity_pk}");
        registry.upsert_last_sent(&placeholder_device_id, identity_pk, op_id)
    }

    /// Retrieve a sync peer by device ID.
    pub fn get_sync_peer(&self, peer_device_id: &str) -> Result<Option<crate::core::peer_registry::SyncPeer>> {
        crate::core::peer_registry::PeerRegistry::new(self.storage.connection())
            .get_peer(peer_device_id)
    }

    /// Upsert a peer received via a delta bundle, consolidating any placeholder row.
    pub fn upsert_peer_from_delta(
        &self,
        real_device_id: &str,
        peer_identity_id: &str,
        last_received_op: Option<&str>,
    ) -> Result<()> {
        crate::core::peer_registry::PeerRegistry::new(self.storage.connection())
            .upsert_peer_from_delta(real_device_id, peer_identity_id, last_received_op)
    }

    /// Insert or update a sync peer row. Pass `None` for watermark fields that
    /// should not overwrite an existing value.
    pub fn upsert_sync_peer(
        &self,
        device_id: &str,
        identity_id: &str,
        last_sent_op: Option<&str>,
        last_received_op: Option<&str>,
    ) -> Result<()> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        registry.upsert_sync_peer(device_id, identity_id, last_sent_op, last_received_op)
    }

}
