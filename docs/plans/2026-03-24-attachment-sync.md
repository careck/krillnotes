# Attachment Sync via Operations Log — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make attachment add/remove mutations flow through the operations log so they sync between peers via delta bundles.

**Architecture:** Two new Operation variants (`AddAttachment`, `RemoveAttachment`) carry attachment metadata. Delta bundles gain `attachments/<id>.enc` sidecar files mirroring the existing snapshot pattern. Workspace methods gain an optional signing key to control whether ops are logged.

**Tech Stack:** Rust, rusqlite, ed25519-dalek, aes-gcm, zip, serde_json

**Spec:** `docs/plans/2026-03-24-attachment-sync-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `krillnotes-core/src/core/operation.rs` | Modify | Add `AddAttachment`, `RemoveAttachment` variants + update all accessor/signing match arms |
| `krillnotes-core/src/core/operation_log.rs` | Modify | Add new variants to `operation_type_name()` helper |
| `krillnotes-core/src/core/workspace/attachments.rs` | Modify | Add `signing_key` param, log ops, add undo push to both methods |
| `krillnotes-core/src/core/workspace/notes.rs` | Modify | Update `delete_attachment` call at line 1745 to pass `None` |
| `krillnotes-core/src/core/workspace/tests.rs` | Modify | Update ~11 `attach_file` + 1 `delete_attachment` calls to pass `None` |
| `krillnotes-core/src/core/export_tests.rs` | Modify | Update 2 `attach_file` calls to pass `None` |
| `krillnotes-core/src/core/workspace/sync.rs` | Modify | Add `attachment_blobs` param to `apply_incoming_operation()`, new match arms, update `operation_type_str()` |
| `krillnotes-core/src/core/swarm/delta.rs` | Modify | Extend `DeltaParams`/`ParsedDelta` with `attachment_blobs`, switch crypto fns, add encrypt/decrypt loops |
| `krillnotes-core/src/core/swarm/sync.rs` | Modify | `generate_delta()` collects blobs, `apply_delta()` extracts and passes blobs |
| `krillnotes-core/src/core/sync/mod.rs` | Modify | `PendingDelta` carries blobs, phase 2 threads blobs to `apply_incoming_operation()` |
| `krillnotes-core/src/core/workspace/undo.rs` | Modify | Wire `AddAttachment`/`RemoveAttachment` into undo group logic |
| `krillnotes-desktop/src-tauri/src/commands/attachments.rs` | Modify | Pass signing key from `AppState` to workspace methods |
| `krillnotes-desktop/src-tauri/src/commands/swarm.rs` | Modify | Pass attachment blobs through delta create/apply paths |

---

### Task 1: Add `AddAttachment` and `RemoveAttachment` Operation Variants

**Files:**
- Modify: `krillnotes-core/src/core/operation.rs`
- Modify: `krillnotes-core/src/core/operation_log.rs` (the `operation_type_name()` function)
- Test: `krillnotes-core/src/core/operation_tests.rs`

- [ ] **Step 1: Write failing tests for the new variants**

Add to `operation_tests.rs`:

```rust
#[test]
fn test_add_attachment_sign_and_verify() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut op = Operation::AddAttachment {
        operation_id: "op-att-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        attachment_id: "att-uuid-1".to_string(),
        note_id: "note-1".to_string(),
        filename: "photo.jpg".to_string(),
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: 1024,
        hash_sha256: "abc123".to_string(),
        added_by: String::new(),
        signature: String::new(),
    };

    op.sign(&signing_key);
    assert!(!op.get_signature().is_empty());
    assert!(!op.author_key().is_empty());
    assert!(op.verify(&verifying_key));

    // Tamper test
    if let Operation::AddAttachment { ref mut filename, .. } = op {
        *filename = "tampered.jpg".to_string();
    }
    assert!(!op.verify(&verifying_key));
}

#[test]
fn test_remove_attachment_sign_and_verify() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut op = Operation::RemoveAttachment {
        operation_id: "op-ratt-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        attachment_id: "att-uuid-1".to_string(),
        note_id: "note-1".to_string(),
        removed_by: String::new(),
        signature: String::new(),
    };

    op.sign(&signing_key);
    assert!(op.verify(&verifying_key));
}

#[test]
fn test_attachment_op_accessors() {
    let ts = dummy_timestamp();
    let op = Operation::AddAttachment {
        operation_id: "op-acc-1".to_string(),
        timestamp: ts,
        device_id: "dev-acc".to_string(),
        attachment_id: "att-1".to_string(),
        note_id: "note-1".to_string(),
        filename: "f.txt".to_string(),
        mime_type: None,
        size_bytes: 100,
        hash_sha256: "hash".to_string(),
        added_by: "key123".to_string(),
        signature: String::new(),
    };

    assert_eq!(op.operation_id(), "op-acc-1");
    assert_eq!(op.timestamp(), ts);
    assert_eq!(op.device_id(), "dev-acc");
    assert_eq!(op.author_key(), "key123");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_add_attachment_sign_and_verify test_remove_attachment_sign_and_verify test_attachment_op_accessors 2>&1 | head -30`
Expected: Compile error — `AddAttachment` and `RemoveAttachment` don't exist yet.

- [ ] **Step 3: Add the two new variants to the `Operation` enum**

In `operation.rs`, add after the `RetractOperation` variant (around line 238):

```rust
    AddAttachment {
        operation_id: String,
        timestamp: HlcTimestamp,
        device_id: String,
        attachment_id: String,
        note_id: String,
        filename: String,
        mime_type: Option<String>,
        size_bytes: i64,
        hash_sha256: String,
        added_by: String,
        signature: String,
    },
    RemoveAttachment {
        operation_id: String,
        timestamp: HlcTimestamp,
        device_id: String,
        attachment_id: String,
        note_id: String,
        removed_by: String,
        signature: String,
    },
```

- [ ] **Step 4: Update all accessor match arms**

In `operation.rs`, add `Self::AddAttachment` and `Self::RemoveAttachment` pattern arms to each of these methods:

1. `operation_id()` (~line 312–333) — bind `operation_id` field
2. `timestamp()` (~line 335–356) — bind `timestamp` field
3. `device_id()` (~line 358–379) — bind `device_id` field
4. `author_key()` (~line 381–404) — `AddAttachment { added_by, .. } => added_by`, `RemoveAttachment { removed_by, .. } => removed_by`
5. `set_author_key()` (~line 408–427) — same pattern, `*added_by = key` / `*removed_by = key`
6. `set_signature()` (~line 429–448) — bind and set `signature` field
7. `get_signature()` (~line 450–469) — bind and return `signature` field

Follow the existing pattern: add each new variant to the `|`-separated match arm lists for methods 1–3, 6–7. Methods 4–5 need dedicated arms because author field names differ.

- [ ] **Step 5: Update `operation_type_name()` in `operation_log.rs`**

Add to the match in `operation_type_name()` (~line 218–237):

```rust
Operation::AddAttachment { .. } => "AddAttachment",
Operation::RemoveAttachment { .. } => "RemoveAttachment",
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p krillnotes-core test_add_attachment_sign_and_verify test_remove_attachment_sign_and_verify test_attachment_op_accessors -- --nocapture 2>&1 | tail -20`
Expected: All 3 pass. If compile fails, check for missing match arms — the compiler will tell you exactly which function has non-exhaustive patterns.

- [ ] **Step 7: Run full test suite to check nothing broke**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All existing tests still pass. Any failures indicate a missing match arm in an existing function.

- [ ] **Step 8: Commit**

```bash
git add krillnotes-core/src/core/operation.rs krillnotes-core/src/core/operation_log.rs krillnotes-core/src/core/operation_tests.rs
git commit -m "feat(core): add AddAttachment and RemoveAttachment operation variants

Signed operations with metadata fields for attachment sync.
Includes accessor methods, signing/verification, and type discriminators."
```

---

### Task 2: Update `apply_incoming_operation()` with New Match Arms

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs`

**Why before Task 3?** The method signature gains an `attachment_blobs` parameter. Doing this early lets us compile-check and wire the new match arms before touching the delta codec.

- [ ] **Step 1: Add `attachment_blobs` parameter and update signature**

In `workspace/sync.rs`, change `apply_incoming_operation` signature (~line 179):

From:
```rust
pub fn apply_incoming_operation(&mut self, op: Operation, received_from_peer: &str) -> Result<bool> {
```

To:
```rust
pub fn apply_incoming_operation(
    &mut self,
    op: Operation,
    received_from_peer: &str,
    attachment_blobs: &[(String, Vec<u8>)],
) -> Result<bool> {
```

- [ ] **Step 2: Add the `operation_type_str()` arms**

In `workspace/sync.rs`, find `operation_type_str()` (~line 378–398) and add:

```rust
Operation::AddAttachment { .. } => "AddAttachment",
Operation::RemoveAttachment { .. } => "RemoveAttachment",
```

- [ ] **Step 3: Add `AddAttachment` match arm**

In the `match &op { ... }` block inside `apply_incoming_operation` (~line 230), add before the log-only catch-all:

```rust
Operation::AddAttachment {
    attachment_id, note_id, filename, mime_type, size_bytes, hash_sha256, ..
} => {
    // Check if the target note still exists.
    let note_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1)",
        [note_id],
        |row| row.get(0),
    )?;
    if note_exists {
        // Find the matching blob from the delta bundle.
        if let Some((_, blob)) = attachment_blobs.iter().find(|(id, _)| id == attachment_id) {
            // attach_file_with_id handles encryption + disk + DB insert.
            // Drop the transaction first since attach_file_with_id uses its own.
            // We commit the op log insert, then do the file write.
            // NOTE: We handle this after the tx.commit() below.
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
```

**Important:** The actual `attach_file_with_id` call needs to happen AFTER the transaction commits, because `attach_file_with_id` opens its own transaction. Add a flag variable before the match block:

```rust
let mut pending_attachment: Option<(String, String, String, Option<String>, Vec<u8>)> = None;
```

Inside the match arm, instead of calling `attach_file_with_id`, populate the flag:

```rust
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
```

Then after `tx.commit()` (~line 370), before the final `Ok(true)`:

```rust
// Deferred attachment file write (after op-log tx is committed).
if let Some((att_id, note_id, filename, mime_type, blob)) = pending_attachment {
    if let Err(e) = self.attach_file_with_id(&att_id, &note_id, &filename, mime_type.as_deref(), &blob) {
        log::error!(target: "krillnotes::sync",
            "Failed to write attachment file {}: {e}", att_id);
    }
}
```

- [ ] **Step 4: Add `RemoveAttachment` match arm**

In the same match block:

```rust
Operation::RemoveAttachment { attachment_id, .. } => {
    // Hard-delete: remove DB row. File deletion deferred after tx.
    tx.execute(
        "DELETE FROM attachments WHERE id = ?1",
        [attachment_id],
    )?;
}
```

After `tx.commit()`, add hard-delete of the `.enc` file (no soft-delete for remote ops):

Add a flag before the match:
```rust
let mut pending_attachment_delete: Option<String> = None;
```

In the match arm:
```rust
Operation::RemoveAttachment { attachment_id, .. } => {
    tx.execute("DELETE FROM attachments WHERE id = ?1", [attachment_id])?;
    pending_attachment_delete = Some(attachment_id.clone());
}
```

After `tx.commit()`:
```rust
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
```

- [ ] **Step 5: Fix all callers to pass `attachment_blobs`**

The signature change breaks existing callers. Temporarily pass `&[]` to all call sites so the project compiles:

1. In `swarm/sync.rs` `apply_delta()` (~line 217):
   ```rust
   // Before:
   workspace.apply_incoming_operation(op.clone(), &parsed.sender_device_id)?
   // After:
   workspace.apply_incoming_operation(op.clone(), &parsed.sender_device_id, &[])?
   ```

2. In `sync/mod.rs` phase 2 loop (~line 388):
   ```rust
   // Before:
   workspace.apply_incoming_operation(entry.op.clone(), &entry.sender_device_id)
   // After:
   workspace.apply_incoming_operation(entry.op.clone(), &entry.sender_device_id, &[])
   ```

- [ ] **Step 6: Compile and run tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass. The `&[]` stubs mean no blobs are passed yet, but existing behavior is preserved.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/swarm/sync.rs krillnotes-core/src/core/sync/mod.rs
git commit -m "feat(core): add AddAttachment/RemoveAttachment match arms to apply_incoming_operation

Handles deferred file writes after op-log transaction commits.
Callers temporarily pass empty blobs slice."
```

---

### Task 3: Update Workspace Attachment Methods to Log Operations

**Files:**
- Modify: `krillnotes-core/src/core/workspace/attachments.rs`
- Modify (callers): `krillnotes-core/src/core/workspace/notes.rs`, `krillnotes-core/src/core/workspace/tests.rs`, `krillnotes-core/src/core/export_tests.rs`, `krillnotes-desktop/src-tauri/src/commands/attachments.rs`

**Note:** `apply_incoming_operation` has two separate transactions — one for the op-log insert (first tx), and a second for state changes (the `match &op` block). The new match arms in Task 2 go in the second transaction; deferred file I/O runs after the second tx commits.

- [ ] **Step 1: Add `signing_key` parameter to `attach_file()` with op logging + undo**

Change signature (~line 19) and add op logging + undo push:

From:
```rust
pub fn attach_file(
    &mut self,
    note_id: &str,
    filename: &str,
    mime_type: Option<&str>,
    data: &[u8],
) -> Result<AttachmentMeta>
```

To:
```rust
pub fn attach_file(
    &mut self,
    note_id: &str,
    filename: &str,
    mime_type: Option<&str>,
    data: &[u8],
    signing_key: Option<&ed25519_dalek::SigningKey>,
) -> Result<AttachmentMeta>
```

After the DB insert and before the `Ok(meta)` return (~line 63), add operation logging and undo:

```rust
if let Some(key) = signing_key {
    let op_id = uuid::Uuid::new_v4().to_string();
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
    self.op_log.log(self.storage.connection(), &op)?;
    self.push_undo(crate::core::undo::UndoEntry {
        retracted_ids: vec![op_id],
        inverse: crate::RetractInverse::AttachmentSoftDelete {
            attachment_id: meta.id.clone(),
        },
        propagate: true,
    });
}
```

Add `use crate::Operation;` at the top of the file if not already imported.

- [ ] **Step 2: Rewrite `delete_attachment()` with signing key, op logging, and undo**

The full revised method should have this structure — query metadata BEFORE the delete, then soft-delete, then log op, then push undo:

```rust
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

    // 2. Soft-delete: rename .enc → .enc.trash, delete DB row.
    let enc_path = self.workspace_root.join("attachments").join(format!("{attachment_id}.enc"));
    let trash_path = self.workspace_root.join("attachments").join(format!("{attachment_id}.enc.trash"));
    if enc_path.exists() {
        std::fs::rename(&enc_path, &trash_path)?;
    }
    self.storage.connection().execute(
        "DELETE FROM attachments WHERE id = ?",
        [attachment_id],
    )?;

    // 3. Log RemoveAttachment op + push undo (only when signing key provided).
    if let Some(key) = signing_key {
        if let Some(ref m) = meta {
            let op_id = uuid::Uuid::new_v4().to_string();
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
            self.op_log.log(self.storage.connection(), &op)?;
            self.push_undo(crate::core::undo::UndoEntry {
                retracted_ids: vec![op_id],
                inverse: crate::RetractInverse::AttachmentRestore { meta: m.clone() },
                propagate: true,
            });
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Fix ALL callers of `attach_file()` and `delete_attachment()`**

The signature changes break callers. Pass `None` to every call site (Task 7 wires the real signing key later):

**`attach_file()` callers — add `, None` as last argument:**
1. `krillnotes-desktop/src-tauri/src/commands/attachments.rs` — `attach_file` command (~line 33)
2. `krillnotes-desktop/src-tauri/src/commands/attachments.rs` — `attach_file_bytes` command
3. `krillnotes-core/src/core/workspace/tests.rs` — ~11 calls (lines 1793, 1809, 1821, 1822, 1838, 1862, 1876, 1884, 1920, 2833)
4. `krillnotes-core/src/core/export_tests.rs` — 2 calls (lines 510, 532)

**`delete_attachment()` callers — add `, None` as last argument:**
5. `krillnotes-desktop/src-tauri/src/commands/attachments.rs` — `delete_attachment` command (~line 138)
6. `krillnotes-core/src/core/workspace/notes.rs` — line 1745: `let _ = self.delete_attachment(old_uuid, None);`
7. `krillnotes-core/src/core/workspace/tests.rs` — line 1846

**Verify no other callers:** `cargo build -p krillnotes-core -p krillnotes-desktop 2>&1 | grep "error"` — fix any remaining.

- [ ] **Step 4: Compile and run tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass. Operations are logged when signing key is provided, but existing paths pass `None` and behave as before.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/attachments.rs krillnotes-core/src/core/workspace/notes.rs krillnotes-core/src/core/workspace/tests.rs krillnotes-core/src/core/export_tests.rs krillnotes-desktop/src-tauri/src/commands/attachments.rs
git commit -m "feat(core): log AddAttachment/RemoveAttachment ops from workspace methods

attach_file() and delete_attachment() accept optional signing key.
When provided, operations are signed and logged. Undo push for both add and delete."
```

---

### Task 4: Extend Delta Bundle Codec

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs`

- [ ] **Step 1: Write test for delta roundtrip with attachments**

Add a test in `delta.rs` (at the bottom, inside `#[cfg(test)]`):

```rust
#[test]
fn test_delta_with_attachments_roundtrip() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let sender_key = SigningKey::generate(&mut OsRng);
    let recipient_key = SigningKey::generate(&mut OsRng);
    let recipient_vk = recipient_key.verifying_key();

    let mut op = Operation::AddAttachment {
        operation_id: "op-att-delta-1".to_string(),
        timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
        device_id: "dev-1".to_string(),
        attachment_id: "att-uuid-1".to_string(),
        note_id: "note-1".to_string(),
        filename: "test.png".to_string(),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4,
        hash_sha256: "fakehash".to_string(),
        added_by: String::new(),
        signature: String::new(),
    };
    op.sign(&sender_key);

    let blob_data = b"BLOB".to_vec();
    let params = DeltaParams {
        protocol: "krillnotes/1".to_string(),
        workspace_id: "ws-1".to_string(),
        workspace_name: "Test".to_string(),
        source_device_id: "dev-1".to_string(),
        source_display_name: "Alice".to_string(),
        since_operation_id: String::new(),
        operations: vec![op],
        sender_key: &sender_key,
        recipient_keys: vec![&recipient_vk],
        recipient_peer_ids: vec!["peer-1".to_string()],
        recipient_identity_id: "recip-id".to_string(),
        owner_pubkey: "owner-key".to_string(),
        ack_operation_id: None,
        attachment_blobs: vec![("att-uuid-1".to_string(), blob_data.clone())],
    };

    let bundle = create_delta_bundle(params).unwrap();
    let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();

    assert_eq!(parsed.operations.len(), 1);
    assert_eq!(parsed.attachment_blobs.len(), 1);
    assert_eq!(parsed.attachment_blobs[0].0, "att-uuid-1");
    assert_eq!(parsed.attachment_blobs[0].1, blob_data);
}

#[test]
fn test_delta_without_attachments_roundtrip() {
    // Existing deltas with no attachment ops should still work.
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let sender_key = SigningKey::generate(&mut OsRng);
    let recipient_key = SigningKey::generate(&mut OsRng);
    let recipient_vk = recipient_key.verifying_key();

    let mut op = Operation::UpdateNote {
        operation_id: "op-un-1".to_string(),
        timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
        device_id: "dev-1".to_string(),
        note_id: "note-1".to_string(),
        title: "Updated".to_string(),
        modified_by: String::new(),
        signature: String::new(),
    };
    op.sign(&sender_key);

    let params = DeltaParams {
        protocol: "krillnotes/1".to_string(),
        workspace_id: "ws-1".to_string(),
        workspace_name: "Test".to_string(),
        source_device_id: "dev-1".to_string(),
        source_display_name: "Alice".to_string(),
        since_operation_id: String::new(),
        operations: vec![op],
        sender_key: &sender_key,
        recipient_keys: vec![&recipient_vk],
        recipient_peer_ids: vec!["peer-1".to_string()],
        recipient_identity_id: "recip-id".to_string(),
        owner_pubkey: "owner-key".to_string(),
        ack_operation_id: None,
        attachment_blobs: vec![],
    };

    let bundle = create_delta_bundle(params).unwrap();
    let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();

    assert_eq!(parsed.operations.len(), 1);
    assert!(parsed.attachment_blobs.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_delta_with_attachments_roundtrip test_delta_without_attachments_roundtrip 2>&1 | head -20`
Expected: Compile error — `DeltaParams` has no `attachment_blobs` field.

- [ ] **Step 3: Add `attachment_blobs` field to `DeltaParams`**

In `delta.rs`, add to `DeltaParams` struct (~line 43):

```rust
    /// Plaintext attachment bytes keyed by attachment_id.
    /// Each blob corresponds to an AddAttachment operation in the batch.
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
```

- [ ] **Step 4: Add `attachment_blobs` field to `ParsedDelta`**

In `delta.rs`, add to `ParsedDelta` struct (~line 56):

```rust
    /// Decrypted attachment blobs from the delta bundle sidecar files.
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
```

- [ ] **Step 5: Fix existing callers of `create_delta_bundle` FIRST (must compile before next steps)**

In `swarm/sync.rs` `generate_delta()`, add `attachment_blobs: vec![]` to the `DeltaParams` struct literal (~line 107). This is temporary — Task 5 will wire real blobs. Without this, the project won't compile after adding the field to `DeltaParams`.

- [ ] **Step 6: Update `create_delta_bundle()` to encrypt and write attachment blobs**

Three changes in `create_delta_bundle()`:

**5a.** Switch from `encrypt_for_recipients` to `encrypt_for_recipients_with_key` (~line 66–67):

From:
```rust
let (ciphertext, mut entries) =
    encrypt_for_recipients(&prefixed, &params.recipient_keys)?;
```

To:
```rust
let (ciphertext, sym_key, mut entries) =
    encrypt_for_recipients_with_key(&prefixed, &params.recipient_keys)?;
```

**5b.** Set `has_attachments` dynamically (~line 94):

From:
```rust
has_attachments: false,
```

To:
```rust
has_attachments: !params.attachment_blobs.is_empty(),
```

**5c.** Add attachment encrypt loop and ZIP writing. After `zip.write_all(&sig)?;` (~line 116), add:

```rust
// Write encrypted attachment sidecar files.
for (att_id, plaintext) in &params.attachment_blobs {
    let ct = encrypt_blob(&sym_key, plaintext)?;
    zip.start_file(format!("attachments/{att_id}.enc"), opts)?;
    zip.write_all(&ct)?;
}
```

Add `use super::crypto::{encrypt_blob, decrypt_blob};` to the imports at top of file if not already present. Also ensure `encrypt_for_recipients_with_key` is imported instead of (or in addition to) `encrypt_for_recipients`.

- [ ] **Step 7: Update `parse_delta_bundle()` to decrypt attachment blobs**

Two changes:

**6a.** Switch from `decrypt_payload` to `decrypt_payload_with_key` (~line 157):

From:
```rust
if let Ok(pt) = decrypt_payload(&ciphertext, entry, recipient_key) {
    plaintext = Some(pt);
    break;
}
```

To:
```rust
let mut sym_key_found = None;
// ... (move this declaration before the for loop)
if let Ok((pt, key)) = decrypt_payload_with_key(&ciphertext, entry, recipient_key) {
    plaintext = Some(pt);
    sym_key_found = Some(key);
    break;
}
```

Add `let mut sym_key_found: Option<[u8; 32]> = None;` before the for loop (~line 155).

After the for loop, recover the sym_key:
```rust
let sym_key = sym_key_found.expect("sym_key set iff decryption succeeded");
```

**6b.** Add attachment decryption loop. Before the `Ok(ParsedDelta { ... })` return (~line 170), add:

```rust
// Decrypt attachment sidecar blobs.
let mut attachment_blobs = Vec::new();
for i in 0..zip.len() {
    let mut file = zip.by_index(i)
        .map_err(|e| KrillnotesError::Swarm(format!("zip index {i}: {e}")))?;
    let name = file.name().to_string();
    if let Some(att_id) = name
        .strip_prefix("attachments/")
        .and_then(|n| n.strip_suffix(".enc"))
    {
        let mut ct = Vec::new();
        file.read_to_end(&mut ct)
            .map_err(|e| KrillnotesError::Swarm(format!("read att {att_id}: {e}")))?;
        let pt = decrypt_blob(&sym_key, &ct)?;
        attachment_blobs.push((att_id.to_string(), pt));
    }
}
```

Add `use std::io::Read;` to imports if not already present.

**6c.** Include `attachment_blobs` in the return struct:

```rust
Ok(ParsedDelta {
    protocol,
    workspace_id: header.workspace_id,
    since_operation_id: header.since_operation_id.unwrap_or_default(),
    sender_public_key: header.source_identity,
    sender_device_id: header.source_device_id,
    operations,
    owner_pubkey: header.owner_pubkey,
    ack_operation_id: header.ack_operation_id,
    attachment_blobs,
})
```

- [ ] **Step 8: Run the tests**

Run: `cargo test -p krillnotes-core test_delta_with_attachments_roundtrip test_delta_without_attachments_roundtrip -- --nocapture 2>&1 | tail -20`
Expected: Both pass.

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add krillnotes-core/src/core/swarm/delta.rs krillnotes-core/src/core/swarm/sync.rs
git commit -m "feat(core): extend delta bundle codec with attachment sidecar files

DeltaParams and ParsedDelta gain attachment_blobs field.
Delta creation encrypts blobs with shared symmetric key.
Delta parsing decrypts attachment sidecar files from ZIP."
```

---

### Task 5: Wire Attachment Blobs Through `generate_delta()` and `apply_delta()`

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs`

- [ ] **Step 1: Update `generate_delta()` to collect attachment blobs**

In `generate_delta()` (~line 100), after `let ops = workspace.operations_since(...)`:

```rust
// Collect plaintext attachment blobs for any AddAttachment ops in the batch.
let mut attachment_blobs: Vec<(String, Vec<u8>)> = Vec::new();
for op in &ops {
    if let Operation::AddAttachment { attachment_id, .. } = op {
        match workspace.get_attachment_bytes(attachment_id) {
            Ok(bytes) => attachment_blobs.push((attachment_id.clone(), bytes)),
            Err(e) => {
                log::warn!(target: "krillnotes::sync",
                    "Could not read attachment {} for delta, skipping blob: {e}",
                    attachment_id);
            }
        }
    }
}
```

Then update the `DeltaParams` struct literal to pass them:

```rust
// Change from:
attachment_blobs: vec![],
// To:
attachment_blobs,
```

- [ ] **Step 2: Update `apply_delta()` to pass blobs to `apply_incoming_operation()`**

In `apply_delta()`, the for loop over `parsed.operations` (~line 222) currently calls:
```rust
workspace.apply_incoming_operation(op.clone(), &parsed.sender_device_id, &[])?
```

Change to:
```rust
workspace.apply_incoming_operation(op.clone(), &parsed.sender_device_id, &parsed.attachment_blobs)?
```

- [ ] **Step 3: Compile and run tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/swarm/sync.rs
git commit -m "feat(core): wire attachment blobs through generate_delta and apply_delta

generate_delta collects plaintext blobs for AddAttachment ops.
apply_delta passes parsed blobs to apply_incoming_operation."
```

---

### Task 6: Update SyncEngine Poll Loop

**Files:**
- Modify: `krillnotes-core/src/core/sync/mod.rs`

- [ ] **Step 1: Add `attachment_blobs` to `PendingDelta` and `OpEntry`**

The `PendingDelta` struct wraps `ParsedDelta`, which now has `attachment_blobs`. No change needed to `PendingDelta` itself.

However, the phase 2 loop flattens ops from all pending deltas into `OpEntry` structs, losing the blob association. Use `Arc` to share blobs across ops from the same delta (avoids cloning multi-MB attachment data per op):

```rust
use std::sync::Arc;

struct OpEntry {
    op: Operation,
    sender_device_id: String,
    bundle_ack: Option<String>,
    attachment_blobs: Arc<Vec<(String, Vec<u8>)>>,  // shared ref to parent delta's blobs
}
```

In the `flat_map` that creates `OpEntry` instances, wrap blobs in Arc:

```rust
let mut op_entries: Vec<OpEntry> = pending_deltas
    .iter()
    .flat_map(|pd| {
        let sender = pd.parsed.sender_device_id.clone();
        let ack = pd.parsed.ack_operation_id.clone();
        let blobs = Arc::new(std::mem::take(&mut pd.parsed.attachment_blobs));
        pd.parsed.operations.iter().map(move |op| OpEntry {
            op: op.clone(),
            sender_device_id: sender.clone(),
            bundle_ack: ack.clone(),
            attachment_blobs: Arc::clone(&blobs),
        })
    })
    .collect();
```

**Note:** `Arc<Vec<...>>` dereferences to `&[(String, Vec<u8>)]` which is what `apply_incoming_operation` expects. If `pending_deltas` is not mutable, use `Arc::new(pd.parsed.attachment_blobs.clone())` instead of `std::mem::take`.

- [ ] **Step 2: Pass blobs to `apply_incoming_operation()` in the loop**

In the phase 2 for loop:

```rust
// Before:
workspace.apply_incoming_operation(entry.op.clone(), &entry.sender_device_id, &[])
// After:
workspace.apply_incoming_operation(entry.op.clone(), &entry.sender_device_id, &entry.attachment_blobs)
```

- [ ] **Step 3: Compile and run tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/sync/mod.rs
git commit -m "feat(core): thread attachment blobs through SyncEngine poll loop

OpEntry carries parent delta's attachment blobs.
Blobs passed to apply_incoming_operation during phase 2."
```

---

### Task 7: Update Tauri Command Layer

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/attachments.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/swarm.rs` (if delta paths are exposed)

- [ ] **Step 1: Pass signing key to `attach_file()` and `delete_attachment()` in Tauri commands**

The signing key is obtained via a two-lock pattern (see `commands/swarm.rs` ~line 273 for reference):

1. Lock `state.workspace_identities` → get the `identity_uuid` for this window label
2. Lock `state.unlocked_identities` → get the `UnlockedIdentity` → clone the signing key
3. Release both locks, THEN lock `state.workspaces`

**Critical lock ordering:** Clone the signing key BEFORE locking `workspaces` to avoid deadlocks.

Add a helper at the top of `commands/attachments.rs`:

```rust
use ed25519_dalek::SigningKey as Ed25519SigningKey;

/// Get the signing key for the workspace associated with this window label.
/// Returns None if no identity is loaded (e.g., pre-identity workspaces).
fn get_signing_key_for_window(state: &AppState, label: &str) -> Option<Ed25519SigningKey> {
    let identity_uuid = {
        let m = state.workspace_identities.lock().expect("Mutex poisoned");
        m.get(label).cloned()?
    };
    let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
    let id = ids.get(&identity_uuid)?;
    Some(Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()))
}
```

Then update the `attach_file` command:

```rust
#[tauri::command]
pub fn attach_file(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    file_path: String,
) -> std::result::Result<crate::AttachmentMeta, String> {
    let label = window.label();

    // Get signing key BEFORE locking workspaces (lock ordering)
    let signing_key = get_signing_key_for_window(&state, label);

    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    let path = std::path::Path::new(&file_path);
    let filename = path.file_name().and_then(|n| n.to_str())
        .ok_or("Invalid file path")?.to_string();
    let mime_type = mime_guess::from_path(path).first().map(|m| m.to_string());
    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;

    workspace
        .attach_file(&note_id, &filename, mime_type.as_deref(), &data, signing_key.as_ref())
        .map_err(|e| { log::error!("attach_file failed: {e}"); e.to_string() })
}
```

Apply the same pattern to `attach_file_bytes` and `delete_attachment` commands — get signing key before locking workspaces.

- [ ] **Step 2: Update `delete_attachment` command similarly**

```rust
#[tauri::command]
pub fn delete_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let signing_key = get_signing_key_for_window(&state, label);

    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .delete_attachment(&attachment_id, signing_key.as_ref())
        .map_err(|e| { log::error!("delete_attachment failed: {e}"); e.to_string() })
}
```

- [ ] **Step 3: Verify swarm delta paths pass blobs correctly**

Check `commands/swarm.rs` for any manual delta creation/application paths that need updating. The core `generate_delta()` and `apply_delta()` in `swarm/sync.rs` already handle blobs (Task 5), so Tauri commands that call those functions should work without changes.

Verify no Tauri command directly calls `create_delta_bundle` or `parse_delta_bundle` — if they do, ensure `attachment_blobs` is wired through.

- [ ] **Step 4: Build the desktop app to verify compilation**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop 2>&1 | tail -20`
Expected: Compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/attachments.rs krillnotes-desktop/src-tauri/src/commands/swarm.rs
git commit -m "feat(desktop): wire signing key through Tauri attachment commands

attach_file and delete_attachment now log operations when identity is loaded."
```

---

### Task 8: Verify Undo Wiring for Attachment Operations

**Files:**
- Read (verify only): `krillnotes-core/src/core/workspace/undo.rs`

**Note:** The undo push for both `attach_file()` and `delete_attachment()` was already added in Task 3. This task verifies the existing undo infrastructure handles the new operations correctly.

- [ ] **Step 1: Verify undo inverse variants exist and have apply logic**

Read `undo.rs` and confirm:
1. `RetractInverse::AttachmentRestore { meta }` apply logic exists (~line 553) — restores a soft-deleted attachment
2. `RetractInverse::AttachmentSoftDelete { attachment_id }` apply logic exists (~line 559) — re-soft-deletes an attachment
3. When processing `AttachmentRestore`, the redo stack gets an `AttachmentSoftDelete` (and vice versa)

If the undo/redo inversion is missing (i.e., undoing a restore doesn't push a soft-delete for redo), add it. Otherwise, no code changes needed.

- [ ] **Step 2: Write a test for undo of attachment add and delete**

```rust
#[test]
fn test_undo_add_attachment() {
    // Setup workspace with signing key
    // attach_file() with signing key → logs AddAttachment + pushes undo
    // Verify attachment exists
    // Call undo → should soft-delete the attachment
    // Verify attachment is gone
    // Call redo → should restore the attachment
    // Verify attachment is back
}

#[test]
fn test_undo_delete_attachment() {
    // Setup workspace with signing key, attach a file
    // delete_attachment() with signing key → logs RemoveAttachment + pushes undo
    // Verify attachment is gone
    // Call undo → should restore from .trash
    // Verify attachment is back
}
```

- [ ] **Step 3: Run full test suite**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 4: Commit (only if code changes were needed)**

```bash
git add krillnotes-core/src/core/workspace/undo.rs
git commit -m "test(core): verify undo for AddAttachment and RemoveAttachment operations"
```

---

### Task 9: Integration Test — Full Delta Sync with Attachments

**Files:**
- Modify: `krillnotes-core/src/core/swarm/` (test file, or add to existing integration tests)

- [ ] **Step 1: Write an integration test for end-to-end attachment delta sync**

This test should:
1. Create two in-memory workspaces (Alice and Bob)
2. Alice attaches a file to a note
3. Generate a delta from Alice → Bob
4. Apply the delta on Bob's workspace
5. Verify Bob now has the attachment (metadata in DB + file bytes readable)

```rust
#[test]
fn test_attachment_delta_sync_end_to_end() {
    // Setup: two workspaces with identities
    // ... (follow existing integration test patterns in the codebase)

    // Alice creates a note
    // Alice attaches a file with signing key
    // Generate delta from Alice to Bob
    // Apply delta on Bob
    // Verify: Bob's workspace has the attachment metadata
    // Verify: Bob can read the attachment bytes and they match
}

#[test]
fn test_remove_attachment_delta_sync() {
    // Setup: two workspaces, Alice has an attachment
    // Alice deletes the attachment with signing key
    // Generate delta from Alice to Bob (Bob already had the attachment from a snapshot)
    // Apply delta on Bob
    // Verify: Bob no longer has the attachment
}
```

The exact test setup depends on how existing sync integration tests create workspace pairs. Follow the patterns in existing test files (check `krillnotes-core/src/core/swarm/` for `*_test*` or `#[cfg(test)]` modules).

- [ ] **Step 2: Run the integration tests**

Run: `cargo test -p krillnotes-core test_attachment_delta_sync 2>&1 | tail -30`
Expected: Both pass.

- [ ] **Step 3: Run the full test suite one final time**

Run: `cargo test -p krillnotes-core 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add -A krillnotes-core/src/core/
git commit -m "test(core): add integration tests for attachment delta sync

Covers end-to-end add and remove attachment sync between two workspaces."
```

---

## Dependency Graph

```
Task 1 (Operation variants)
    ↓
Task 2 (apply_incoming_operation match arms)
    ↓
Task 3 (Workspace methods + op logging)
    ↓
Task 4 (Delta codec extension)
    ↓
Task 5 (generate_delta / apply_delta wiring)
    ↓
Task 6 (SyncEngine poll loop)  ←── can run in parallel with Task 7
    ↓
Task 7 (Tauri commands)
    ↓
Task 8 (Undo wiring)
    ↓
Task 9 (Integration tests)
```

Tasks 6 and 7 are independent and can be implemented in parallel.
