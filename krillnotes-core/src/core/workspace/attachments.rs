// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! File attachment operations: encrypt, store, retrieve, and delete.

use super::*;

impl Workspace {

    // -------------------------------------------------------------------------
    // Attachment methods
    // -------------------------------------------------------------------------

    /// Attaches a file to a note. Encrypts the bytes and writes them to
    /// `<workspace_root>/attachments/<uuid>.enc`, then inserts a DB metadata row.
    ///
    /// If `signing_key` is `Some`, an `AddAttachment` operation is signed, logged,
    /// and an undo entry is pushed so the attachment can be removed.
    pub fn attach_file(
        &mut self,
        note_id: &str,
        filename: &str,
        mime_type: Option<&str>,
        data: &[u8],
        signing_key: Option<&ed25519_dalek::SigningKey>,
    ) -> Result<AttachmentMeta> {
        // Enforce workspace size limit
        if let Some(limit) = self.attachment_max_size_bytes()? {
            if data.len() as u64 > limit {
                return Err(KrillnotesError::AttachmentTooLarge {
                    size: data.len() as u64,
                    limit,
                });
            }
        }

        // SHA-256 hash for integrity
        let hash = {
            let mut h = Sha256::new();
            h.update(data);
            format!("{:x}", h.finalize())
        };

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let (encrypted_bytes, file_salt) =
            encrypt_attachment(data, self.attachment_key.as_ref())?;

        // Write to disk
        let enc_path = self.workspace_root.join("attachments").join(format!("{id}.enc"));
        std::fs::write(&enc_path, &encrypted_bytes)?;

        let meta = AttachmentMeta {
            id,
            note_id: note_id.to_string(),
            filename: filename.to_string(),
            mime_type: mime_type.map(|s| s.to_string()),
            size_bytes: data.len() as i64,
            hash_sha256: hash,
            salt: hex::encode(file_salt),
            created_at: now,
        };

        // Build op and sign before opening transaction (avoids borrow conflict).
        let signed_op: Option<(String, Operation)> = if let Some(key) = signing_key {
            let op_id = Uuid::new_v4().to_string();
            let mut op = Operation::AddAttachment {
                operation_id: op_id.clone(),
                timestamp: self.hlc.now(),
                device_id: self.device_id().to_string(),
                attachment_id: meta.id.clone(),
                note_id: note_id.to_string(),
                filename: filename.to_string(),
                mime_type: mime_type.map(|s| s.to_string()),
                size_bytes: meta.size_bytes,
                hash_sha256: meta.hash_sha256.clone(),
                added_by: String::new(),
                signature: String::new(),
            };
            op.sign(key);
            Some((op_id, op))
        } else {
            None
        };

        // Insert DB row (and op log row) in one transaction.
        {
            let tx = self.storage.connection_mut().transaction()?;
            tx.execute(
                "INSERT INTO attachments (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    meta.id, meta.note_id, meta.filename, meta.mime_type.as_deref(),
                    meta.size_bytes, meta.hash_sha256, file_salt.as_slice(), meta.created_at
                ],
            )?;
            if let Some((_, ref op)) = signed_op {
                self.operation_log.log(&tx, op)?;
            }
            tx.commit()?;
        }

        if let Some((op_id, _)) = signed_op {
            self.push_undo(UndoEntry {
                retracted_ids: vec![op_id],
                inverse: RetractInverse::AttachmentSoftDelete {
                    attachment_id: meta.id.clone(),
                },
                propagate: true,
            });
        }

        Ok(meta)
    }

    /// Import-only: attach a file with a pre-specified ID (preserves IDs from export).
    /// Does NOT enforce size limits (the size was already validated at export time).
    pub fn attach_file_with_id(
        &mut self,
        id: &str,
        note_id: &str,
        filename: &str,
        mime_type: Option<&str>,
        data: &[u8],
    ) -> Result<()> {
        let hash = {
            let mut h = Sha256::new();
            h.update(data);
            format!("{:x}", h.finalize())
        };
        let now = chrono::Utc::now().timestamp();
        let (encrypted_bytes, file_salt) = encrypt_attachment(data, self.attachment_key.as_ref())?;
        let enc_path = self.workspace_root.join("attachments").join(format!("{id}.enc"));
        std::fs::write(&enc_path, &encrypted_bytes)?;
        let _ = self.storage.connection().execute(
            "INSERT OR IGNORE INTO attachments (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, note_id, filename, mime_type, data.len() as i64, hash, file_salt.as_slice(), now],
        );
        Ok(())
    }

    /// Returns all attachment metadata for a note (no file I/O).
    pub fn get_attachments(&self, note_id: &str) -> Result<Vec<AttachmentMeta>> {
        let mut stmt = self.storage.connection().prepare(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
             FROM attachments WHERE note_id = ? ORDER BY created_at ASC",
        )?;
        let results = stmt.query_map([note_id], |row| {
            let salt_bytes: Vec<u8> = row.get(6)?;
            Ok(AttachmentMeta {
                id: row.get(0)?,
                note_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                hash_sha256: row.get(5)?,
                salt: hex::encode(&salt_bytes),
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    /// Returns all attachments in the workspace (used for export).
    pub fn list_all_attachments(&self) -> Result<Vec<AttachmentMeta>> {
        let mut stmt = self.storage.connection().prepare(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
             FROM attachments ORDER BY created_at ASC",
        )?;
        let results = stmt.query_map([], |row| {
            let salt_bytes: Vec<u8> = row.get(6)?;
            Ok(AttachmentMeta {
                id: row.get(0)?,
                note_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                hash_sha256: row.get(5)?,
                salt: hex::encode(&salt_bytes),
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    /// Decrypts and returns the plaintext bytes for an attachment.
    pub fn get_attachment_bytes(&self, attachment_id: &str) -> Result<Vec<u8>> {
        let (salt_bytes, _): (Vec<u8>, i64) = self.storage.connection().query_row(
            "SELECT salt, size_bytes FROM attachments WHERE id = ?",
            [attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| KrillnotesError::NoteNotFound(attachment_id.to_string()))?;

        let enc_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        let encrypted_bytes = std::fs::read(&enc_path)?;
        decrypt_attachment(&encrypted_bytes, self.attachment_key.as_ref(), &salt_bytes)
    }

    /// Returns decrypted attachment bytes together with the stored MIME type.
    pub fn get_attachment_bytes_and_mime(
        &self,
        attachment_id: &str,
    ) -> Result<(Vec<u8>, Option<String>)> {
        let (salt_bytes, _, mime_type): (Vec<u8>, i64, Option<String>) =
            self.storage.connection().query_row(
                "SELECT salt, size_bytes, mime_type FROM attachments WHERE id = ?",
                [attachment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).map_err(|_| KrillnotesError::NoteNotFound(attachment_id.to_string()))?;

        let enc_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        let encrypted_bytes = std::fs::read(&enc_path)?;
        let bytes = decrypt_attachment(&encrypted_bytes, self.attachment_key.as_ref(), &salt_bytes)?;
        Ok((bytes, mime_type))
    }

    /// Replaces `<img data-kn-attach-id="UUID">` sentinels in `html` with real
    /// `src="data:mime;base64,..."` attributes and converts `data-kn-width="N"`
    /// to an inline `style="max-width:Npx;height:auto"`.
    ///
    /// Called after running `on_view` and `on_hover` hooks so the frontend
    /// receives fully-embedded HTML without needing client-side hydration.
    /// Sentinels whose attachment cannot be read are left in place so the
    /// client-side fallback can show an error message.
    pub(crate) fn embed_attachment_images(&self, html: String) -> String {
        use base64::Engine as _;
        use std::sync::OnceLock;

        static ID_RE: OnceLock<regex::Regex> = OnceLock::new();
        static WIDTH_RE: OnceLock<regex::Regex> = OnceLock::new();

        let id_re = ID_RE.get_or_init(|| {
            regex::Regex::new(r#"data-kn-attach-id="([^"]+)""#).expect("valid regex")
        });
        let width_re = WIDTH_RE.get_or_init(|| {
            regex::Regex::new(r#"data-kn-width="(\d+)""#).expect("valid regex")
        });

        // Collect unique attachment IDs present in the HTML.
        let ids: Vec<String> = id_re
            .captures_iter(&html)
            .map(|cap| cap[1].to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if ids.is_empty() {
            return html;
        }

        // Replace each sentinel with a real data URL.
        let mut result = html;
        for id in ids {
            if let Ok((bytes, mime_opt)) = self.get_attachment_bytes_and_mime(&id) {
                let mime = mime_opt.as_deref().unwrap_or("image/png");
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let src = format!(r#"src="data:{mime};base64,{encoded}""#);
                result = result.replace(&format!(r#"data-kn-attach-id="{id}""#), &src);
            }
            // If the attachment cannot be read, leave the sentinel; the client
            // hydration fallback will display an "Image not found" error.
        }

        // Convert data-kn-width="N" → style="max-width:Npx;height:auto".
        let result = width_re.replace_all(&result, |caps: &regex::Captures| {
            format!(r#"style="max-width:{}px;height:auto""#, &caps[1])
        });
        result.into_owned()
    }

    /// Deletes an attachment: removes the `.enc` file and the DB row.
    /// Returns the metadata for a single attachment by ID.
    pub(crate) fn get_attachment_meta(&self, attachment_id: &str) -> Result<AttachmentMeta> {
        let row = self.storage.connection().query_row(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
             FROM attachments WHERE id = ?",
            [attachment_id],
            |row| {
                let salt_bytes: Vec<u8> = row.get(6)?;
                Ok(AttachmentMeta {
                    id: row.get(0)?,
                    note_id: row.get(1)?,
                    filename: row.get(2)?,
                    mime_type: row.get(3)?,
                    size_bytes: row.get(4)?,
                    hash_sha256: row.get(5)?,
                    salt: hex::encode(&salt_bytes),
                    created_at: row.get(7)?,
                })
            },
        )?;
        Ok(row)
    }

    /// Soft-deletes an attachment: renames `{id}.enc` → `{id}.enc.trash` and removes the
    /// DB row.
    ///
    /// If `signing_key` is `Some`, a `RemoveAttachment` operation is signed, logged,
    /// and an `AttachmentRestore` undo entry is pushed so the deletion can be reversed.
    /// The `.enc.trash` file is cleaned up when the undo entry is discarded (workspace
    /// close or stack overflow past the limit).
    pub fn delete_attachment(
        &mut self,
        attachment_id: &str,
        signing_key: Option<&ed25519_dalek::SigningKey>,
    ) -> Result<()> {
        // 1. Query full metadata BEFORE deletion (needed for undo + op logging).
        let meta: Option<AttachmentMeta> = self.storage.connection().query_row(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, hex(salt), created_at \
             FROM attachments WHERE id = ?",
            [attachment_id],
            |row| {
                Ok(AttachmentMeta {
                    id: row.get(0)?,
                    note_id: row.get(1)?,
                    filename: row.get(2)?,
                    mime_type: row.get(3)?,
                    size_bytes: row.get(4)?,
                    hash_sha256: row.get(5)?,
                    salt: row.get(6)?,
                    created_at: row.get(7)?,
                })
            },
        ).optional()?;

        // Build the signed op before opening the transaction (avoids borrow conflict).
        let signed_op: Option<(String, Operation)> = if let Some(key) = signing_key {
            if let Some(ref m) = meta {
                let op_id = Uuid::new_v4().to_string();
                let mut op = Operation::RemoveAttachment {
                    operation_id: op_id.clone(),
                    timestamp: self.hlc.now(),
                    device_id: self.device_id().to_string(),
                    attachment_id: attachment_id.to_string(),
                    note_id: m.note_id.clone(),
                    removed_by: String::new(),
                    signature: String::new(),
                };
                op.sign(key);
                Some((op_id, op))
            } else {
                None
            }
        } else {
            None
        };

        // 2. Soft-delete: rename .enc → .enc.trash, delete DB row (+ op log in one tx).
        let enc_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        let trash_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc.trash"));
        if enc_path.exists() {
            std::fs::rename(&enc_path, &trash_path)?;
        }
        {
            let tx = self.storage.connection_mut().transaction()?;
            tx.execute("DELETE FROM attachments WHERE id = ?", [attachment_id])?;
            if let Some((_, ref op)) = signed_op {
                self.operation_log.log(&tx, op)?;
            }
            tx.commit()?;
        }

        // 3. Push undo entry (only when signing key was provided and meta was found).
        if let Some((op_id, _)) = signed_op {
            if let Some(ref m) = meta {
                self.push_undo(UndoEntry {
                    retracted_ids: vec![op_id],
                    inverse: RetractInverse::AttachmentRestore { meta: m.clone() },
                    propagate: true,
                });
            }
        }
        Ok(())
    }

    /// Restores a soft-deleted attachment: renames `.enc.trash` → `.enc` (if the
    /// trash file exists) and re-inserts the DB row. Used by the in-section "Restore"
    /// button. Safe to call even if the session ended and the trash file was purged —
    /// only the DB row is re-inserted in that case.
    pub fn restore_attachment(&mut self, meta: &AttachmentMeta) -> Result<()> {
        let trash_path = self.workspace_root.join("attachments")
            .join(format!("{}.enc.trash", meta.id));
        let enc_path = self.workspace_root.join("attachments")
            .join(format!("{}.enc", meta.id));
        if trash_path.exists() {
            std::fs::rename(&trash_path, &enc_path)?;
        }
        let salt_bytes = hex::decode(&meta.salt)
            .unwrap_or_else(|_| meta.salt.as_bytes().to_vec());
        self.storage.connection().execute(
            "INSERT OR IGNORE INTO attachments
             (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
             VALUES (?,?,?,?,?,?,?,?)",
            rusqlite::params![
                meta.id, meta.note_id, meta.filename, meta.mime_type,
                meta.size_bytes as i64, meta.hash_sha256,
                salt_bytes.as_slice(), meta.created_at,
            ],
        )?;
        Ok(())
    }

    /// Purges any `.enc.trash` files left over from a previous session.
    ///
    /// Should be called once on workspace open. Since undo stacks are in-session
    /// only, all `.enc.trash` files from prior sessions are safe to remove.
    pub(crate) fn purge_attachment_trash(&self) {
        let trash_dir = self.workspace_root.join("attachments");
        if let Ok(entries) = std::fs::read_dir(&trash_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("trash") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// Returns the workspace-level max attachment size in bytes, or `None` if unlimited.
    pub fn attachment_max_size_bytes(&self) -> Result<Option<u64>> {
        let val: std::result::Result<String, rusqlite::Error> = self.storage.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key = 'attachment_max_size_bytes'",
            [],
            |row| row.get(0),
        );
        match val {
            Ok(s) => Ok(s.parse::<u64>().ok()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Sets or clears the workspace-level max attachment size.
    pub fn set_attachment_max_size_bytes(&mut self, limit: Option<u64>) -> Result<()> {
        match limit {
            Some(n) => {
                self.storage.connection().execute(
                    "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('attachment_max_size_bytes', ?)",
                    [n.to_string()],
                )?;
            }
            None => {
                self.storage.connection().execute(
                    "DELETE FROM workspace_meta WHERE key = 'attachment_max_size_bytes'",
                    [],
                )?;
            }
        }
        Ok(())
    }
}
