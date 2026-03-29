// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Snapshot sync, delta operations, and peer registry management.

use super::*;
use crate::core::peer_registry::SyncPeer;
use crate::core::sync::channel::{ChannelType, PeerSyncInfo};

impl Workspace {
    // ── Snapshot (peer sync) ───────────────────────────────────────

    /// Serialise all notes, user scripts, attachment metadata, and permission
    /// operations to JSON bytes for a snapshot bundle.
    pub fn to_snapshot_json(&self) -> Result<Vec<u8>> {
        log::info!(target: "krillnotes::sync", "generating snapshot JSON");
        let notes = self.list_all_notes()?;
        let user_scripts = self.list_user_scripts()?;
        let attachments = self.list_all_attachments()?;
        let permission_ops = self.collect_permission_ops()?;
        log::debug!(target: "krillnotes::sync",
            "snapshot: {} notes, {} scripts, {} attachments, {} permission ops",
            notes.len(), user_scripts.len(), attachments.len(), permission_ops.len());
        let snapshot = WorkspaceSnapshot {
            version: 1,
            notes,
            user_scripts,
            attachments,
            permission_ops,
        };
        Ok(serde_json::to_vec(&snapshot)?)
    }

    /// Query all SetPermission / RevokePermission operations from the log,
    /// ordered by HLC timestamp (oldest first).
    fn collect_permission_ops(&self) -> Result<Vec<Operation>> {
        let conn = self.storage.connection();
        let mut stmt = conn.prepare(
            "SELECT operation_data FROM operations \
             WHERE operation_type IN ('SetPermission', 'RevokePermission') \
             ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, timestamp_node_id ASC"
        )?;
        let ops = stmt.query_map([], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?.filter_map(|r| {
            match r {
                Ok(json) => match serde_json::from_str::<Operation>(&json) {
                    Ok(op) => Some(op),
                    Err(e) => {
                        log::warn!(target: "krillnotes::sync", "skipping malformed permission op: {e}");
                        None
                    }
                },
                Err(e) => {
                    log::warn!(target: "krillnotes::sync", "skipping unreadable permission row: {e}");
                    None
                }
            }
        }).collect();
        Ok(ops)
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

        // Two exclusions per query:
        //   device_id != ?  — don't send a peer their own authored ops
        //   received_from_peer IS NULL OR received_from_peer != ?
        //     — don't echo back ops the peer already delivered to us
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
                     AND (received_from_peer IS NULL OR received_from_peer != ?5) \
                     ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                              timestamp_node_id ASC",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![wall_ms, counter, node_id, exclude_device_id, exclude_device_id],
                    |row| row.get::<_, String>(0),
                )?.collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?;
                rows
            } else {
                // Watermark op not in this workspace's log (e.g. freshly imported
                // from a snapshot whose operations were never inserted locally).
                // Fall back to sending everything — the recipient's INSERT OR IGNORE
                // handles any duplicates safely.
                let mut stmt = conn.prepare(
                    "SELECT operation_data FROM operations \
                     WHERE device_id != ?1 \
                     AND (received_from_peer IS NULL OR received_from_peer != ?2) \
                     ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                              timestamp_node_id ASC",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![exclude_device_id, exclude_device_id],
                    |row| row.get::<_, String>(0),
                )?.collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?;
                rows
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT operation_data FROM operations \
                 WHERE device_id != ?1 \
                 AND (received_from_peer IS NULL OR received_from_peer != ?2) \
                 ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                          timestamp_node_id ASC",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![exclude_device_id, exclude_device_id],
                |row| row.get::<_, String>(0),
            )?.collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?;
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
    pub fn apply_incoming_operation(
        &mut self,
        op: Operation,
        received_from_peer: &str,
        attachment_blobs: &[(String, Vec<u8>)],
    ) -> Result<bool> {
        // 1. Skip local-only retracts — they must never cross device boundaries.
        if matches!(op, Operation::RetractOperation { propagate: false, .. }) {
            log::debug!(target: "krillnotes::sync", "skipping local-only retract operation {}", op.operation_id());
            return Ok(false);
        }

        // No RBAC gate on inbound operations — all ops replicate unconditionally.
        // Access control is enforced at the visibility layer (visible_note_ids,
        // list_notes) not at the replication layer.

        log::debug!(target: "krillnotes::sync", "applying incoming operation {} ({})", op.operation_id(), Self::operation_type_str(&op));

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
                  device_id, operation_type, operation_data, synced, received_from_peer) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)",
                rusqlite::params![
                    op.operation_id(),
                    ts.wall_ms as i64,
                    ts.counter as i64,
                    ts.node_id as i64,
                    op.device_id(),
                    op_type,
                    op_json,
                    received_from_peer,
                ],
            )?;
            tx.commit()?;
            rows
        };

        // 4. Duplicate — already applied.
        if rows == 0 {
            log::debug!(target: "krillnotes::sync", "duplicate operation {}, skipping", op.operation_id());
            return Ok(false);
        }

        // 5. Apply the state change to working tables.
        let mut scripts_changed = false;
        // (attachment_id, note_id, filename, mime_type, blob)
        let mut pending_attachment: Option<(String, String, String, Option<String>, Vec<u8>)> = None;
        let mut pending_attachment_delete: Option<String> = None;
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
                created_by, script_id, name, description, source_code, load_order, enabled, ..
            } => {
                if created_by == &self.owner_pubkey {
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
                    scripts_changed = true;
                }
            }

            Operation::UpdateUserScript {
                modified_by, script_id, name, description, source_code, load_order, enabled, ..
            } => {
                if modified_by == &self.owner_pubkey {
                    let now_ms = ts.wall_ms as i64;
                    tx.execute(
                        "UPDATE user_scripts SET name = ?1, description = ?2, source_code = ?3, \
                         load_order = ?4, enabled = ?5, modified_at = ?6 WHERE id = ?7",
                        rusqlite::params![
                            name, description, source_code,
                            load_order, *enabled as i32, now_ms, script_id,
                        ],
                    )?;
                    scripts_changed = true;
                }
            }

            Operation::DeleteUserScript { deleted_by, script_id, .. } => {
                if deleted_by == &self.owner_pubkey {
                    tx.execute(
                        "DELETE FROM user_scripts WHERE id = ?1",
                        [script_id],
                    )?;
                    scripts_changed = true;
                }
            }

            // Permission-modifying operations: apply through the gate.
            Operation::SetPermission { .. }
            | Operation::RevokePermission { .. } => {
                Self::apply_permission_op_via(&*self.permission_gate, &tx, &op)?;
            }

            // Log-only variants — no working table change in this phase.
            Operation::JoinWorkspace { .. }
            | Operation::UpdateSchema { .. }
            | Operation::RetractOperation { .. }
            | Operation::RemovePeer { .. }
            | Operation::TransferRootOwnership { .. }
            | Operation::RegisterDevice { .. } => {}

            Operation::AddAttachment {
                attachment_id, note_id, filename, mime_type, ..
            } => {
                let note_exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1)",
                    [note_id],
                    |row| row.get(0),
                )?;
                if note_exists {
                    if let Some((_, blob)) = attachment_blobs.iter().find(|(id, _)| id == attachment_id) {
                        pending_attachment = Some((
                            attachment_id.clone(),
                            note_id.clone(),
                            filename.clone(),
                            mime_type.clone(),
                            blob.clone(),
                        ));
                    } else {
                        log::warn!(target: "krillnotes::sync",
                            "AddAttachment {} has no matching blob in delta, recording op only",
                            attachment_id);
                    }
                } else {
                    log::warn!(target: "krillnotes::sync",
                        "AddAttachment {} targets deleted note {}, skipping file write",
                        attachment_id, note_id);
                }
            }

            Operation::RemoveAttachment { attachment_id, .. } => {
                tx.execute("DELETE FROM attachments WHERE id = ?1", [attachment_id])?;
                pending_attachment_delete = Some(attachment_id.clone());
            }
        }
        tx.commit()?;

        // Deferred attachment file write (after state-mutation tx is committed).
        //
        // TODO: split-transaction window — `attach_file_with_id` both encrypts the file
        // (generating the per-file salt) and inserts the DB row.  The salt is only known
        // after encryption, so the DB insert cannot be moved inside the transaction above.
        // If the process crashes between `tx.commit()` and the completion of
        // `attach_file_with_id`, the operation is in the log but the attachment file and/or
        // DB row may be missing.  Recovery: re-apply the delta (idempotent via
        // INSERT OR IGNORE) or re-sync from the peer.
        if let Some((att_id, note_id, filename, mime_type, blob)) = pending_attachment {
            if let Err(e) = self.attach_file_with_id(&att_id, &note_id, &filename, mime_type.as_deref(), &blob) {
                log::error!(target: "krillnotes::sync",
                    "Failed to write attachment file {}: {e}", att_id);
            }
        }

        // NOTE: DB row deleted in transaction above; file deletion below is best-effort.
        // If process crashes between the two, an orphan .enc file may remain on disk.
        // This is acceptable: the operation is in the log and will be replayed; orphans
        // can be swept on next startup.
        if let Some(att_id) = pending_attachment_delete {
            let enc_path = self.workspace_root.join("attachments").join(format!("{att_id}.enc"));
            if enc_path.exists() {
                if let Err(e) = std::fs::remove_file(&enc_path) {
                    log::error!(target: "krillnotes::sync",
                        "Failed to delete attachment file {}: {e}", att_id);
                }
            }
            // Also clean up any .trash file
            let trash_path = self.workspace_root.join("attachments").join(format!("{att_id}.enc.trash"));
            if trash_path.exists() {
                let _ = std::fs::remove_file(&trash_path);
            }
        }

        // Re-register scripts with the Rhai engine after applying script ops.
        if scripts_changed {
            log::info!(target: "krillnotes::sync", "scripts changed, reloading Rhai engine");
            self.reload_scripts()?;
        }

        log::debug!(target: "krillnotes::sync", "operation {} applied successfully", op.operation_id());
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
            Operation::RemovePeer { .. } => "RemovePeer",
            Operation::TransferRootOwnership { .. } => "TransferRootOwnership",
            Operation::AddAttachment { .. } => "AddAttachment",
            Operation::RemoveAttachment { .. } => "RemoveAttachment",
            Operation::RegisterDevice { .. } => "RegisterDevice",
        }
    }

    /// Populate a workspace from snapshot JSON bytes.
    ///
    /// Notes and user scripts are inserted. Returns the number of notes imported.
    /// Designed for freshly created workspaces — duplicates will be skipped via INSERT OR IGNORE.
    pub fn import_snapshot_json(&mut self, data: &[u8]) -> Result<usize> {
        log::info!(target: "krillnotes::sync", "importing snapshot ({} bytes)", data.len());
        let snapshot: WorkspaceSnapshot = serde_json::from_slice(data)
            .map_err(|e| KrillnotesError::Json(e))?;

        let note_count = snapshot.notes.len();
        log::debug!(target: "krillnotes::sync", "snapshot contains {} notes, {} scripts", note_count, snapshot.user_scripts.len());

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

            // Re-register imported scripts with the Rhai engine.
            log::info!(target: "krillnotes::sync", "reloading scripts after snapshot import");
            self.reload_scripts()?;
        }

        // Replay permission operations through the gate so the recipient
        // can see the notes they've been granted access to.
        if !snapshot.permission_ops.is_empty() {
            log::info!(target: "krillnotes::sync",
                "replaying {} permission ops from snapshot", snapshot.permission_ops.len());
            let tx = self.storage.connection_mut().transaction()?;
            for op in &snapshot.permission_ops {
                // Log the operation so it can be forwarded via future delta syncs.
                let op_json = serde_json::to_string(op)?;
                let ts = op.timestamp();
                let op_type = Self::operation_type_str(op);
                tx.execute(
                    "INSERT OR IGNORE INTO operations \
                     (operation_id, timestamp_wall_ms, timestamp_counter, timestamp_node_id, \
                      device_id, operation_type, operation_data, synced, received_from_peer) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 1, 'snapshot')",
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
                // Apply through the permission gate.
                Self::apply_permission_op_via(&*self.permission_gate, &tx, op)?;
            }
            tx.commit()?;
        }

        log::info!(target: "krillnotes::sync", "snapshot import complete: {} notes", note_count);
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
                    peer_identity_id: peer.peer_identity_id.clone(),
                    display_name,
                    fingerprint,
                    trust_level,
                    contact_id: contact.map(|c| c.contact_id.to_string()),
                    last_sync: peer.last_sync,
                    is_owner: peer.peer_identity_id == self.owner_pubkey,
                    is_self_peer: peer.peer_identity_id == self.current_identity_pubkey,
                    channel_type: peer.channel_type,
                    channel_params: peer.channel_params,
                    sync_status: peer.sync_status,
                    sync_status_detail: peer.sync_status_detail,
                    last_sync_error: peer.last_sync_error,
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

    /// Update a peer's channel configuration.
    pub fn update_peer_channel(
        &self,
        peer_device_id: &str,
        channel_type: &str,
        channel_params: &str,
    ) -> Result<()> {
        PeerRegistry::new(self.storage.connection())
            .update_channel_config(peer_device_id, channel_type, channel_params)
    }

    /// Update a peer's sync status.
    pub fn update_peer_sync_status(
        &self,
        peer_device_id: &str,
        sync_status: &str,
        detail: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        PeerRegistry::new(self.storage.connection())
            .update_sync_status(peer_device_id, sync_status, detail, error)
    }

    /// List peers filtered by channel type.
    pub fn list_peers_with_channel(&self, channel_type: &str) -> Result<Vec<SyncPeer>> {
        PeerRegistry::new(self.storage.connection())
            .list_peers_by_channel(channel_type)
    }

    /// Reset `last_sent_op` for a peer to a specific op ID, or `None` to trigger full resend.
    pub fn reset_peer_watermark(&self, peer_device_id: &str, to_op: Option<&str>) -> Result<()> {
        PeerRegistry::new(self.storage.connection())
            .reset_last_sent(peer_device_id, to_op)
    }



    /// Returns true if `op_a` is strictly before `op_b` in HLC order.
    /// Returns false if either operation is not found in the log.
    pub fn is_operation_before(&self, op_a: &str, op_b: &str) -> Result<bool> {
        let conn = self.storage.connection();
        let get_hlc = |op_id: &str| -> Result<Option<(i64, i64, i64)>> {
            conn.query_row(
                "SELECT timestamp_wall_ms, timestamp_counter, timestamp_node_id \
                 FROM operations WHERE operation_id = ?1",
                [op_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            ).optional().map_err(KrillnotesError::Database)
        };
        let Some(hlc_a) = get_hlc(op_a)? else { return Ok(false) };
        let Some(hlc_b) = get_hlc(op_b)? else { return Ok(false) };
        Ok(hlc_a < hlc_b)
    }

    /// Returns true if the given operation_id exists in the operations log.
    pub fn operation_exists(&self, operation_id: &str) -> Result<bool> {
        let conn = self.storage.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id = ?1",
            [operation_id],
            |row| row.get(0),
        ).map_err(KrillnotesError::Database)?;
        Ok(count > 0)
    }

    /// Returns true if there are operations to send to at least one non-manual peer.
    ///
    /// Returns true if:
    /// - A peer has no watermark at all (needs a snapshot), OR
    /// - A peer's watermark is set but newer ops exist after it (needs a delta).
    pub fn has_pending_ops_for_any_peer(&self) -> Result<bool> {
        let peers = self.get_active_sync_peers()?;
        let conn = self.storage.connection();
        for peer in &peers {
            if peer.last_sent_op.is_none() {
                // Peer hasn't received a snapshot yet — work is needed.
                return Ok(true);
            }
            if let Some(ref op_id) = peer.last_sent_op {
                // Check if any ops exist after the watermark using HLC comparison.
                let hlc = conn.query_row(
                    "SELECT timestamp_wall_ms, timestamp_counter, timestamp_node_id \
                     FROM operations WHERE operation_id = ?1",
                    [op_id.as_str()],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
                ).optional().map_err(KrillnotesError::Database)?;
                if let Some((wall_ms, counter, node_id)) = hlc {
                    let count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM operations WHERE \
                         (timestamp_wall_ms > ?1 \
                          OR (timestamp_wall_ms = ?1 AND timestamp_counter > ?2) \
                          OR (timestamp_wall_ms = ?1 AND timestamp_counter = ?2 AND timestamp_node_id > ?3))",
                        rusqlite::params![wall_ms, counter, node_id],
                        |row| row.get(0),
                    ).unwrap_or(0);
                    if count > 0 {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    /// Get `PeerSyncInfo` for all non-manual peers (used by the SyncEngine).
    pub fn get_active_sync_peers(&self) -> Result<Vec<PeerSyncInfo>> {
        let peers = PeerRegistry::new(self.storage.connection())
            .list_peers_by_channel_not("manual")?;
        Ok(peers.into_iter().map(|p| PeerSyncInfo {
            peer_device_id: p.peer_device_id,
            peer_identity_id: p.peer_identity_id,
            channel_type: match p.channel_type.as_str() {
                "relay" => ChannelType::Relay,
                "folder" => ChannelType::Folder,
                _ => ChannelType::Manual,
            },
            channel_params: serde_json::from_str(&p.channel_params)
                .unwrap_or(serde_json::Value::Object(Default::default())),
            last_sent_op: p.last_sent_op,
            last_received_op: p.last_received_op,
        }).collect())
    }

}
