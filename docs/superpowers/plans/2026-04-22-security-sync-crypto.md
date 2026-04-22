# Security: Sync & Crypto — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three security vulnerabilities in sync/crypto (C1 signature verification, H2 password zeroization, M1 sidecar manifest integrity) and add a persistent `sync_events` audit trail with UI.

**Architecture:** Add `verify_operation_signatures()` in `delta.rs` called inside `parse_delta_bundle` so all callers get automatic verification. Add `sync_events` table via migration, with `Workspace` methods for logging/querying. Wrap relay password zeroization in a custom `Drop` impl (simpler than `Zeroizing<String>` given serde constraints). UI is a second tab in the existing Operations Log dialog.

**Tech Stack:** Rust (rusqlite, ed25519-dalek, blake3, zeroize), React 19, Tailwind v4, i18next

**Issue:** #141
**Spec:** `docs/superpowers/specs/2026-04-22-pre-1.0-audit-remediation-design.md` (Batch 4)

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `krillnotes-core/Cargo.toml` | Add `zeroize` dependency |
| Modify | `krillnotes-core/src/core/storage.rs` | `sync_events` table migration |
| Create | `krillnotes-core/src/core/workspace/sync_events.rs` | `log_sync_event()`, `list_sync_events()`, `SyncEventRecord` type |
| Modify | `krillnotes-core/src/core/workspace/mod.rs` | `mod sync_events;` declaration |
| Modify | `krillnotes-core/src/core/lib.rs` | Re-export `SyncEventRecord` |
| Modify | `krillnotes-core/src/core/sync/relay/relay_account.rs` | Custom `Debug` + `Drop` for zeroization |
| Modify | `krillnotes-core/src/core/swarm/delta.rs` | Sidecar hashes in manifest + op signature verification |
| Modify | `krillnotes-core/src/core/swarm/sync.rs` | Log signature failures to `sync_events` in `apply_delta` |
| Modify | `krillnotes-core/src/core/sync/mod.rs` | Log signature failures to `sync_events` in `poll()` |
| Modify | `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs` | Log signature failures to `sync_events` |
| Modify | `krillnotes-desktop/src-tauri/src/lib.rs` | Add `list_sync_events` Tauri command |
| Modify | `krillnotes-desktop/src/types.ts` | Add `SyncEventRecord` TS type |
| Modify | `krillnotes-desktop/src/components/OperationsLogDialog.tsx` | Add Sync Events tab |
| Modify | `krillnotes-desktop/src/i18n/locales/*.json` | i18n keys for sync events (all 7 locales) |

---

## Task 1: `sync_events` Storage Layer

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs:467` (after last migration)
- Create: `krillnotes-core/src/core/workspace/sync_events.rs`
- Modify: `krillnotes-core/src/core/workspace/mod.rs` (add `mod sync_events;`)

### Steps

- [ ] **Step 1: Write failing test for `log_sync_event` and `list_sync_events`**

In new file `krillnotes-core/src/core/workspace/sync_events.rs`:

```rust
use crate::Result;
use crate::core::workspace::Workspace;
use serde::Serialize;

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
    /// Log a sync security event to the persistent audit trail.
    pub fn log_sync_event(
        &self,
        peer_pubkey: &str,
        event_type: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        todo!()
    }

    /// Query sync events, most recent first.
    pub fn list_sync_events(&self, limit: i64, offset: i64) -> Result<Vec<SyncEventRecord>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use crate::core::workspace::Workspace;

    fn test_workspace() -> Workspace {
        Workspace::create_empty("", "test-device").unwrap()
    }

    #[test]
    fn test_log_and_list_sync_events() {
        let ws = test_workspace();
        ws.log_sync_event("pk-alice", "signature_invalid", Some("op-123 failed")).unwrap();
        ws.log_sync_event("pk-bob", "bundle_rejected", None).unwrap();
        ws.log_sync_event("pk-alice", "sidecar_mismatch", Some("att-456 missing")).unwrap();

        let events = ws.list_sync_events(10, 0).unwrap();
        assert_eq!(events.len(), 3);
        // Most recent first
        assert_eq!(events[0].event_type, "sidecar_mismatch");
        assert_eq!(events[0].peer_pubkey, "pk-alice");
        assert_eq!(events[2].event_type, "signature_invalid");
    }

    #[test]
    fn test_list_sync_events_pagination() {
        let ws = test_workspace();
        for i in 0..5 {
            ws.log_sync_event("pk-peer", "bundle_rejected", Some(&format!("event {i}"))).unwrap();
        }

        let page1 = ws.list_sync_events(2, 0).unwrap();
        assert_eq!(page1.len(), 2);

        let page2 = ws.list_sync_events(2, 2).unwrap();
        assert_eq!(page2.len(), 2);

        let page3 = ws.list_sync_events(2, 4).unwrap();
        assert_eq!(page3.len(), 1);
    }

    #[test]
    fn test_list_sync_events_empty() {
        let ws = test_workspace();
        let events = ws.list_sync_events(10, 0).unwrap();
        assert!(events.is_empty());
    }
}
```

- [ ] **Step 2: Add `mod sync_events;` to workspace module**

In `krillnotes-core/src/core/workspace/mod.rs`, add near the other `mod` declarations:

```rust
mod sync_events;
pub use sync_events::SyncEventRecord;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_log_and_list_sync_events`
Expected: FAIL — `todo!()` panics

- [ ] **Step 4: Add `sync_events` migration to `storage.rs`**

In `krillnotes-core/src/core/storage.rs`, after the `is_checked` migration (line ~467), before `Ok(())`:

```rust
        // Migration: create sync_events table for sync security audit trail.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                peer_pubkey TEXT NOT NULL,
                event_type TEXT NOT NULL,
                detail TEXT
            )"
        )?;
```

- [ ] **Step 5: Implement `log_sync_event` and `list_sync_events`**

Replace `todo!()` stubs in `sync_events.rs`:

```rust
impl Workspace {
    pub fn log_sync_event(
        &self,
        peer_pubkey: &str,
        event_type: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.storage.connection().execute(
            "INSERT INTO sync_events (timestamp, peer_pubkey, event_type, detail) VALUES (?, ?, ?, ?)",
            rusqlite::params![now, peer_pubkey, event_type, detail],
        )?;
        Ok(())
    }

    pub fn list_sync_events(&self, limit: i64, offset: i64) -> Result<Vec<SyncEventRecord>> {
        let conn = self.storage.connection();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, peer_pubkey, event_type, detail \
             FROM sync_events ORDER BY id DESC LIMIT ? OFFSET ?"
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
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core sync_events`
Expected: All 3 tests PASS

- [ ] **Step 7: Re-export from lib.rs**

In `krillnotes-core/src/core/lib.rs` (or wherever public types are re-exported), add:

```rust
pub use core::workspace::SyncEventRecord;
```

Verify the existing re-export pattern — follow the same style as other workspace types.

- [ ] **Step 8: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync_events.rs \
        krillnotes-core/src/core/workspace/mod.rs \
        krillnotes-core/src/core/storage.rs \
        krillnotes-core/src/core/lib.rs
git commit -m "feat: add sync_events audit trail table and Workspace methods"
```

---

## Task 2: H2 — Zeroize Relay Account Password

**Files:**
- Modify: `krillnotes-core/Cargo.toml` (add `zeroize`)
- Modify: `krillnotes-core/src/core/sync/relay/relay_account.rs:29-39` (struct + impls)

**Design note:** The spec suggests `Zeroizing<String>`, but `RelayAccount` needs `Serialize + Deserialize` (for AES-GCM encryption at rest) and `Clone` (for the `HashMap` cache). `Zeroizing<String>` lacks serde derives. A custom `Drop` impl that calls `password.zeroize()` achieves the same security goal with zero API disruption.

### Steps

- [ ] **Step 1: Write failing test for redacted Debug**

Add to `krillnotes-core/src/core/sync/relay/relay_account.rs` tests module:

```rust
    #[test]
    fn test_debug_redacts_password_and_token() {
        let account = RelayAccount {
            relay_account_id: Uuid::new_v4(),
            relay_url: "https://relay.example.com".to_string(),
            email: "test@test.com".to_string(),
            password: "super-secret-password".to_string(),
            session_token: "secret-token-value".to_string(),
            session_expires_at: Utc::now(),
            device_public_key: "pk-123".to_string(),
        };
        let debug_output = format!("{:?}", account);
        assert!(!debug_output.contains("super-secret-password"),
            "Debug output must not contain password");
        assert!(!debug_output.contains("secret-token-value"),
            "Debug output must not contain session_token");
        assert!(debug_output.contains("[REDACTED]"));
        assert!(debug_output.contains("relay.example.com"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_debug_redacts_password`
Expected: FAIL — derived `Debug` prints the password

- [ ] **Step 3: Add `zeroize` to Cargo.toml**

In `krillnotes-core/Cargo.toml`, in the `[dependencies]` section under the encryption group:

```toml
zeroize = "1"
```

- [ ] **Step 4: Replace derived Debug with custom impl, add Drop for zeroization**

In `krillnotes-core/src/core/sync/relay/relay_account.rs`:

Change the derive on `RelayAccount` from:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
```
to:
```rust
#[derive(Clone, Serialize, Deserialize)]
```

Add below the struct definition (after line 39):

```rust
impl std::fmt::Debug for RelayAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayAccount")
            .field("relay_account_id", &self.relay_account_id)
            .field("relay_url", &self.relay_url)
            .field("email", &self.email)
            .field("password", &"[REDACTED]")
            .field("session_token", &"[REDACTED]")
            .field("session_expires_at", &self.session_expires_at)
            .field("device_public_key", &self.device_public_key)
            .finish()
    }
}

impl Drop for RelayAccount {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
        self.session_token.zeroize();
    }
}
```

Also remove the existing manual `Drop` impl on `RelayAccountManager` (lines 243-249) — it manually zeroes `encryption_key` with `key.fill(0)`. Replace it with a proper zeroize call for consistency:

```rust
impl Drop for RelayAccountManager {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(key) = self.encryption_key.as_mut() {
            key.zeroize();
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core relay_account`
Expected: All relay_account tests PASS (including the new debug test + all existing tests)

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/Cargo.toml \
        krillnotes-core/src/core/sync/relay/relay_account.rs
git commit -m "fix(security): zeroize relay account password on drop, redact in Debug (H2)"
```

---

## Task 3: M1 — Sidecar Hashes in Bundle Manifest

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs:107-220`

**Design:** Encrypt all sidecars *before* computing the manifest hash, include `("attachments/{id}.enc", &ciphertext)` pairs in the `files` vec passed to `sign_manifest`/`verify_manifest`. The `manifest_hash` function already sorts by filename, so order is deterministic.

### Steps

- [ ] **Step 1: Write failing test — stripped sidecar must fail verification**

Add to `krillnotes-core/src/core/swarm/delta.rs` tests module:

```rust
    #[test]
    fn test_stripped_sidecar_fails_verification() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let recipient_vk = recipient_key.verifying_key();

        let mut op = Operation::AddAttachment {
            operation_id: "op-att-strip-1".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
            device_id: "dev-1".to_string(),
            attachment_id: "att-strip-1".to_string(),
            note_id: "note-1".to_string(),
            filename: "photo.png".to_string(),
            mime_type: Some("image/png".to_string()),
            size_bytes: 5,
            hash_sha256: "fakehash".to_string(),
            added_by: String::new(),
            signature: String::new(),
        };
        op.sign(&sender_key);

        let bundle = create_delta_bundle(DeltaParams {
            protocol: "test".to_string(),
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
            attachment_blobs: vec![("att-strip-1".to_string(), b"PHOTO".to_vec())],
        }).unwrap();

        // Tamper: remove the sidecar from the ZIP
        let tampered = strip_sidecar_from_bundle(&bundle);
        let result = parse_delta_bundle(&tampered, &recipient_key);
        assert!(result.is_err(), "Bundle with stripped sidecar must fail verification");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("signature verification failed"),
            "Error should mention signature verification, got: {err_msg}");
    }

    /// Helper: rebuild a bundle ZIP with all `attachments/*.enc` entries removed.
    fn strip_sidecar_from_bundle(bundle: &[u8]) -> Vec<u8> {
        use std::io::{Cursor, Read, Write};
        use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

        let mut src = ZipArchive::new(Cursor::new(bundle)).unwrap();
        let mut buf = Vec::new();
        {
            let mut dst = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();
            for i in 0..src.len() {
                let mut entry = src.by_index(i).unwrap();
                let name = entry.name().to_string();
                if name.starts_with("attachments/") {
                    continue; // strip sidecar
                }
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                dst.start_file(&name, opts).unwrap();
                dst.write_all(&data).unwrap();
            }
            dst.finish().unwrap();
        }
        buf
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_stripped_sidecar_fails`
Expected: FAIL — sidecar not in manifest, so stripped bundle still passes verification

- [ ] **Step 3: Modify `create_delta_bundle` — encrypt sidecars before signing**

In `krillnotes-core/src/core/swarm/delta.rs`, replace lines 106-138 (from the NOTE comment through the ZIP writing) with:

```rust
    let header_bytes = serde_json::to_vec(&header)?;

    // Encrypt attachment sidecars before signing so their ciphertext is in the manifest.
    let encrypted_sidecars: Vec<(String, Vec<u8>)> = params
        .attachment_blobs
        .iter()
        .map(|(att_id, plaintext)| {
            let ct = encrypt_blob(&sym_key, plaintext)?;
            Ok((att_id.clone(), ct))
        })
        .collect::<Result<Vec<_>>>()?;

    // Build manifest over header + payload + all sidecar ciphertexts.
    let mut files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.enc", &ciphertext),
    ];
    let sidecar_names: Vec<String> = encrypted_sidecars
        .iter()
        .map(|(id, _)| format!("attachments/{id}.enc"))
        .collect();
    for (i, (_, ct)) in encrypted_sidecars.iter().enumerate() {
        files.push((&sidecar_names[i], ct));
    }
    let sig = sign_manifest(&files, params.sender_key);

    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);
        let opts = SimpleFileOptions::default();
        zip.start_file("header.json", opts)?;
        zip.write_all(&header_bytes)?;
        zip.start_file("payload.enc", opts)?;
        zip.write_all(&ciphertext)?;
        zip.start_file("signature.bin", opts)?;
        zip.write_all(&sig)?;
        for (att_id, ct) in &encrypted_sidecars {
            zip.start_file(format!("attachments/{att_id}.enc"), opts)?;
            zip.write_all(ct)?;
        }
        zip.finish()?;
    }
    Ok(buf)
```

- [ ] **Step 4: Modify `parse_delta_bundle` — include sidecars in verification**

In `parse_delta_bundle`, replace lines 158-169 (the verify block) and lines 192-208 (sidecar reading) with a new flow that reads sidecar ciphertext first, verifies manifest with sidecars included, then decrypts:

After reading `header_bytes`, `ciphertext`, `sig_bytes` (lines 151-153), and parsing the header (line 155), add sidecar ciphertext reading BEFORE verification:

```rust
    // Read all sidecar ciphertext BEFORE verification (needed for manifest hash).
    let mut sidecar_entries: Vec<(String, Vec<u8>)> = Vec::new();
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
            sidecar_entries.push((att_id.to_string(), ct));
        }
    }

    // Verify bundle signature (including sidecar ciphertexts).
    let vk_bytes = BASE64.decode(&header.source_identity)
        .map_err(|e| KrillnotesError::Swarm(format!("bad source_identity: {e}")))?;
    let vk_arr: [u8; 32] = vk_bytes.try_into()
        .map_err(|_| KrillnotesError::Swarm("source_identity wrong length".to_string()))?;
    let vk = VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid sender key: {e}")))?;
    let mut files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.enc", &ciphertext),
    ];
    let sidecar_names: Vec<String> = sidecar_entries
        .iter()
        .map(|(id, _)| format!("attachments/{id}.enc"))
        .collect();
    for (i, (_, ct)) in sidecar_entries.iter().enumerate() {
        files.push((&sidecar_names[i], ct));
    }
    verify_manifest(&files, &sig_bytes, &vk)?;
```

Then replace the old sidecar decryption loop (lines 192-208) with decryption from the already-read ciphertext:

```rust
    // Decrypt attachment sidecars from already-read ciphertext.
    let mut attachment_blobs = Vec::new();
    for (att_id, ct) in &sidecar_entries {
        let pt = decrypt_blob(&sym_key, ct)?;
        attachment_blobs.push((att_id.clone(), pt));
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core -- delta`
Expected: ALL delta tests PASS (roundtrip, attachment roundtrip, empty, without-attachments, AND new stripped-sidecar test)

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/swarm/delta.rs
git commit -m "fix(security): include sidecar hashes in bundle manifest (M1)"
```

---

## Task 4: C1 — Per-Operation Signature Verification

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs:190-220` (add verification after decryption)
- Modify: `krillnotes-core/src/core/swarm/sync.rs:186` (log to sync_events on failure)
- Modify: `krillnotes-core/src/core/sync/mod.rs:260` (log to sync_events on failure)
- Modify: `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs:440,501` (log to sync_events on failure)

**Design:** Verify each operation's signature against its own `author_key()` (not the bundle sender's key), because operations may be forwarded from their original author by a different peer. Verification happens inside `parse_delta_bundle` so no caller can skip it. Callers with workspace access log failures to `sync_events`.

### Steps

- [ ] **Step 1: Write failing test — tampered operation must be rejected**

Add to `krillnotes-core/src/core/swarm/delta.rs` tests module:

```rust
    #[test]
    fn test_tampered_operation_rejected() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let recipient_vk = recipient_key.verifying_key();

        // Create a properly signed operation.
        let mut op = Operation::UpdateNote {
            operation_id: "op-tamper-1".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            title: "Original".to_string(),
            modified_by: String::new(),
            signature: String::new(),
        };
        op.sign(&sender_key);

        // Tamper: change the title after signing.
        if let Operation::UpdateNote { ref mut title, .. } = op {
            *title = "TAMPERED".to_string();
        }

        let bundle = create_delta_bundle(DeltaParams {
            protocol: "test".to_string(),
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
        }).unwrap();

        let result = parse_delta_bundle(&bundle, &recipient_key);
        assert!(result.is_err(), "Bundle with tampered operation must be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("operation signature"),
            "Error should mention operation signature, got: {err_msg}");
    }

    #[test]
    fn test_valid_signed_operations_accepted() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let recipient_vk = recipient_key.verifying_key();

        let mut op1 = Operation::UpdateNote {
            operation_id: "op-valid-1".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            title: "Valid".to_string(),
            modified_by: String::new(),
            signature: String::new(),
        };
        op1.sign(&sender_key);

        let mut op2 = Operation::CreateNote {
            operation_id: "op-valid-2".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 1001, counter: 0, node_id: 1 },
            device_id: "dev-1".to_string(),
            note_id: "note-2".to_string(),
            parent_id: None,
            title: "New Note".to_string(),
            schema: "TextNote".to_string(),
            position: 0.0,
            created_by: String::new(),
            signature: String::new(),
        };
        op2.sign(&sender_key);

        let bundle = create_delta_bundle(DeltaParams {
            protocol: "test".to_string(),
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: String::new(),
            operations: vec![op1, op2],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_vk],
            recipient_peer_ids: vec!["peer-1".to_string()],
            recipient_identity_id: "recip-id".to_string(),
            owner_pubkey: "owner-key".to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![],
        }).unwrap();

        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.operations.len(), 2);
    }
```

- [ ] **Step 2: Run tests to verify the tampered test fails**

Run: `cargo test -p krillnotes-core test_tampered_operation_rejected`
Expected: FAIL — tampered operation is currently accepted

- [ ] **Step 3: Add per-operation signature verification in `parse_delta_bundle`**

In `parse_delta_bundle`, after deserializing operations (`let operations: Vec<Operation> = ...`) and before the sidecar reading, add:

```rust
    // Verify per-operation signatures against each op's claimed author key.
    for op in &operations {
        let author_b64 = op.author_key();
        if author_b64.is_empty() {
            continue; // RetractOperation has no author — local-only, will be skipped later
        }
        let author_vk_bytes = BASE64.decode(author_b64)
            .map_err(|e| KrillnotesError::Swarm(format!(
                "operation signature invalid: op {} has bad author_key: {e}",
                op.operation_id()
            )))?;
        let author_vk_arr: [u8; 32] = author_vk_bytes.try_into()
            .map_err(|_| KrillnotesError::Swarm(format!(
                "operation signature invalid: op {} author_key wrong length",
                op.operation_id()
            )))?;
        let author_vk = VerifyingKey::from_bytes(&author_vk_arr)
            .map_err(|e| KrillnotesError::Swarm(format!(
                "operation signature invalid: op {} bad verifying key: {e}",
                op.operation_id()
            )))?;
        if !op.verify(&author_vk) {
            return Err(KrillnotesError::Swarm(format!(
                "operation signature invalid: op {} failed verification against author {}",
                op.operation_id(),
                &author_b64[..author_b64.len().min(8)],
            )));
        }
    }
```

Also remove the stub comment at line 143: `/// **STUB:** individual operation signatures are NOT verified yet — WP-C adds that.`

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core -- delta`
Expected: ALL delta tests PASS

**Note:** The existing `test_delta_roundtrip` and `test_empty_delta_allowed` tests use `dummy_op()` which creates operations with fake signatures (`"sig"` / `"pk"`). These will now fail because the verification will reject them. You need to update `dummy_op` to produce properly signed operations:

```rust
    fn dummy_op(id: &str, key: &SigningKey) -> Operation {
        let mut op = Operation::UpdateNote {
            operation_id: id.to_string(),
            timestamp: HlcTimestamp { wall_ms: 1, counter: 0, node_id: 0 },
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            title: "Updated".to_string(),
            modified_by: String::new(),
            signature: String::new(),
        };
        op.sign(key);
        op
    }
```

Update both `test_delta_roundtrip` and `test_empty_delta_allowed` to pass `&sender_key` to `dummy_op`.

- [ ] **Step 5: Wire up sync_events logging in `apply_delta` (swarm/sync.rs)**

In `krillnotes-core/src/core/swarm/sync.rs`, change line 186 from:

```rust
    let parsed = parse_delta_bundle(bundle_bytes, recipient_key)?;
```

to:

```rust
    let parsed = match parse_delta_bundle(bundle_bytes, recipient_key) {
        Ok(p) => p,
        Err(ref e) if e.to_string().contains("operation signature") => {
            // Extract sender info from the bundle header for audit logging.
            if let Ok(header) = super::header::read_header(bundle_bytes) {
                let _ = workspace.log_sync_event(
                    &header.source_identity,
                    "signature_invalid",
                    Some(&e.to_string()),
                );
            }
            return Err(e);
        }
        Err(e) => return Err(e),
    };
```

Wait — `Err(ref e)` then `return Err(e)` won't work because of the borrow. Use a different pattern:

```rust
    let parsed = match parse_delta_bundle(bundle_bytes, recipient_key) {
        Ok(p) => p,
        Err(e) => {
            if e.to_string().contains("operation signature") || e.to_string().contains("sidecar") {
                if let Ok(header) = super::header::read_header(bundle_bytes) {
                    let event_type = if e.to_string().contains("operation signature") {
                        "signature_invalid"
                    } else {
                        "sidecar_mismatch"
                    };
                    let _ = workspace.log_sync_event(
                        &header.source_identity,
                        event_type,
                        Some(&e.to_string()),
                    );
                }
            }
            return Err(e);
        }
    };
```

Also update the stub comment at line 164 to remove "Individual per-operation signatures are not verified."

- [ ] **Step 6: Wire up sync_events logging in `poll()` (sync/mod.rs)**

In `krillnotes-core/src/core/sync/mod.rs`, in the `Err(e)` arm of `parse_delta_bundle` (around line 280), add sync_events logging. The workspace is available as `workspace` in scope:

```rust
                            Err(e) => {
                                log::error!(target: "krillnotes::sync", "parse_delta_bundle failed for peer {}: {e}", header.source_device_id);
                                if e.to_string().contains("operation signature")
                                    || e.to_string().contains("signature verification failed")
                                {
                                    let _ = workspace.log_sync_event(
                                        &header.source_identity,
                                        "signature_invalid",
                                        Some(&e.to_string()),
                                    );
                                }
                                events.push(SyncEvent::IngestError {
                                    workspace_id: workspace_id.clone(),
                                    peer_device_id: header.source_device_id.clone(),
                                    error: format!("parse_delta_bundle: {e}"),
                                });
                            }
```

- [ ] **Step 7: Wire up sync_events logging in `receive_poll.rs`**

In `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs`, the parse phase (lines 440, 501) doesn't have workspace access. Add a collection for parse failures and log them during the apply phase.

After the `downloaded_deltas` vec declaration, add:

```rust
        let mut parse_failures: Vec<(String, String)> = Vec::new(); // (sender_pubkey, error)
```

In both parse error handlers (relay at ~line 455, folder at ~line 517), extract sender info from the header and collect:

```rust
                    Err(e) => {
                        log::warn!("poll_receive_workspace: parse relay delta {} failed: {e}", bundle_meta.bundle_id);
                        if let Ok(header) = read_header(&bundle_bytes) {
                            parse_failures.push((header.source_identity.clone(), e.to_string()));
                        }
                    }
```

Then in the apply phase (after `workspace` is locked, around line 580), log the collected failures:

```rust
        for (sender_pubkey, error) in &parse_failures {
            let event_type = if error.contains("operation signature") {
                "signature_invalid"
            } else if error.contains("signature verification failed") {
                "sidecar_mismatch"
            } else {
                "bundle_rejected"
            };
            let _ = workspace.log_sync_event(sender_pubkey, event_type, Some(error));
        }
```

- [ ] **Step 8: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: ALL tests PASS

- [ ] **Step 9: Commit**

```bash
git add krillnotes-core/src/core/swarm/delta.rs \
        krillnotes-core/src/core/swarm/sync.rs \
        krillnotes-core/src/core/sync/mod.rs \
        krillnotes-desktop/src-tauri/src/commands/receive_poll.rs
git commit -m "fix(security): verify per-operation signatures on delta ingest (C1)"
```

---

## Task 5: Tauri Command + TypeScript Types

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (add `list_sync_events` command)
- Modify: `krillnotes-desktop/src/types.ts` (add `SyncEventRecord` type)

### Steps

- [ ] **Step 1: Add `list_sync_events` Tauri command**

In `krillnotes-desktop/src-tauri/src/lib.rs`, add the command function (follow the pattern of existing commands like `list_operations`):

```rust
#[tauri::command]
async fn list_sync_events(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    limit: i64,
    offset: i64,
) -> Result<Vec<krillnotes_core::SyncEventRecord>, String> {
    let label = window.label().to_string();
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let workspace = workspaces
        .get(&label)
        .ok_or_else(|| format!("No workspace for window {label}"))?;
    workspace
        .list_sync_events(limit, offset)
        .map_err(|e| e.to_string())
}
```

Add `list_sync_events` to the `tauri::generate_handler![...]` macro invocation.

- [ ] **Step 2: Add `SyncEventRecord` TypeScript type**

In `krillnotes-desktop/src/types.ts`:

```typescript
export interface SyncEventRecord {
  id: number;
  timestamp: number;
  peerPubkey: string;
  eventType: string;
  detail: string | null;
}
```

- [ ] **Step 3: Verify compilation**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No type errors

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs \
        krillnotes-desktop/src/types.ts
git commit -m "feat: add list_sync_events Tauri command and TS type"
```

---

## Task 6: Frontend — Sync Events Tab + i18n

**Files:**
- Modify: `krillnotes-desktop/src/components/OperationsLogDialog.tsx`
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`
- Modify: `krillnotes-desktop/src/i18n/locales/de.json`
- Modify: `krillnotes-desktop/src/i18n/locales/es.json`
- Modify: `krillnotes-desktop/src/i18n/locales/fr.json`
- Modify: `krillnotes-desktop/src/i18n/locales/ja.json`
- Modify: `krillnotes-desktop/src/i18n/locales/ko.json`
- Modify: `krillnotes-desktop/src/i18n/locales/zh.json`

### Steps

- [ ] **Step 1: Add i18n keys to all 7 locale files**

Add to the `"log"` section of each locale file:

**en.json:**
```json
    "operationsTab": "Operations",
    "syncEventsTab": "Sync Events",
    "syncNoEvents": "No sync events recorded.",
    "syncPeer": "Peer",
    "syncEventType": "Event",
    "syncDetail": "Detail",
    "syncCount_one": "{{count}} event",
    "syncCount_other": "{{count}} events",
    "syncBundleRejected": "Bundle Rejected",
    "syncSignatureInvalid": "Signature Invalid",
    "syncSidecarMismatch": "Sidecar Mismatch"
```

**de.json:**
```json
    "operationsTab": "Operationen",
    "syncEventsTab": "Sync-Ereignisse",
    "syncNoEvents": "Keine Sync-Ereignisse aufgezeichnet.",
    "syncPeer": "Peer",
    "syncEventType": "Ereignis",
    "syncDetail": "Detail",
    "syncCount_one": "{{count}} Ereignis",
    "syncCount_other": "{{count}} Ereignisse",
    "syncBundleRejected": "Paket abgelehnt",
    "syncSignatureInvalid": "Signatur ungültig",
    "syncSidecarMismatch": "Sidecar-Abweichung"
```

**es.json:**
```json
    "operationsTab": "Operaciones",
    "syncEventsTab": "Eventos de sincronización",
    "syncNoEvents": "No se han registrado eventos de sincronización.",
    "syncPeer": "Par",
    "syncEventType": "Evento",
    "syncDetail": "Detalle",
    "syncCount_one": "{{count}} evento",
    "syncCount_other": "{{count}} eventos",
    "syncBundleRejected": "Paquete rechazado",
    "syncSignatureInvalid": "Firma inválida",
    "syncSidecarMismatch": "Discrepancia de sidecar"
```

**fr.json:**
```json
    "operationsTab": "Opérations",
    "syncEventsTab": "Événements de synchronisation",
    "syncNoEvents": "Aucun événement de synchronisation enregistré.",
    "syncPeer": "Pair",
    "syncEventType": "Événement",
    "syncDetail": "Détail",
    "syncCount_one": "{{count}} événement",
    "syncCount_other": "{{count}} événements",
    "syncBundleRejected": "Paquet rejeté",
    "syncSignatureInvalid": "Signature invalide",
    "syncSidecarMismatch": "Décalage de sidecar"
```

**ja.json:**
```json
    "operationsTab": "操作",
    "syncEventsTab": "同期イベント",
    "syncNoEvents": "同期イベントは記録されていません。",
    "syncPeer": "ピア",
    "syncEventType": "イベント",
    "syncDetail": "詳細",
    "syncCount_one": "{{count}} 件のイベント",
    "syncCount_other": "{{count}} 件のイベント",
    "syncBundleRejected": "バンドル拒否",
    "syncSignatureInvalid": "署名無効",
    "syncSidecarMismatch": "サイドカー不一致"
```

**ko.json:**
```json
    "operationsTab": "작업",
    "syncEventsTab": "동기화 이벤트",
    "syncNoEvents": "기록된 동기화 이벤트가 없습니다.",
    "syncPeer": "피어",
    "syncEventType": "이벤트",
    "syncDetail": "세부 정보",
    "syncCount_one": "{{count}}개 이벤트",
    "syncCount_other": "{{count}}개 이벤트",
    "syncBundleRejected": "번들 거부됨",
    "syncSignatureInvalid": "서명 무효",
    "syncSidecarMismatch": "사이드카 불일치"
```

**zh.json:**
```json
    "operationsTab": "操作",
    "syncEventsTab": "同步事件",
    "syncNoEvents": "没有记录的同步事件。",
    "syncPeer": "对等节点",
    "syncEventType": "事件",
    "syncDetail": "详情",
    "syncCount_one": "{{count}} 个事件",
    "syncCount_other": "{{count}} 个事件",
    "syncBundleRejected": "包已拒绝",
    "syncSignatureInvalid": "签名无效",
    "syncSidecarMismatch": "附件不匹配"
```

- [ ] **Step 2: Add tab state and sync events fetching to OperationsLogDialog**

In `OperationsLogDialog.tsx`, add to imports:

```typescript
import type { OperationSummary, SyncEventRecord } from '../types';
```

Add state variables inside the component function (near other state):

```typescript
  const [activeTab, setActiveTab] = useState<'operations' | 'syncEvents'>('operations');
  const [syncEvents, setSyncEvents] = useState<SyncEventRecord[]>([]);
  const [syncError, setSyncError] = useState<string | null>(null);
```

Add a fetch function for sync events (near the existing `fetchOperations`):

```typescript
  const fetchSyncEvents = useCallback(async () => {
    try {
      setSyncError(null);
      const events = await invoke<SyncEventRecord[]>('list_sync_events', {
        limit: 200,
        offset: 0,
      });
      setSyncEvents(events);
    } catch (err) {
      setSyncError(String(err));
    }
  }, []);
```

Add to the existing `useEffect` that fetches data on open:

```typescript
  useEffect(() => {
    if (isOpen) {
      fetchOperations();
      fetchSyncEvents();
    }
  }, [isOpen, fetchOperations, fetchSyncEvents]);
```

- [ ] **Step 3: Add tab switcher and sync events table to JSX**

Replace the `{/* Filters */}` section (lines 264-292) with a combined tab bar + filters section:

```tsx
        {/* Tab bar */}
        <div className="flex items-center gap-1 px-4 pt-2 border-b border-border shrink-0">
          <button
            onClick={() => setActiveTab('operations')}
            className={`px-3 py-1.5 text-sm font-medium rounded-t border border-b-0 ${
              activeTab === 'operations'
                ? 'bg-background border-border text-foreground'
                : 'bg-transparent border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('log.operationsTab')}
          </button>
          <button
            onClick={() => setActiveTab('syncEvents')}
            className={`px-3 py-1.5 text-sm font-medium rounded-t border border-b-0 ${
              activeTab === 'syncEvents'
                ? 'bg-background border-border text-foreground'
                : 'bg-transparent border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('log.syncEventsTab')}
          </button>
        </div>

        {/* Operations filters — only show when operations tab active */}
        {activeTab === 'operations' && (
          <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-muted/30 shrink-0">
            {/* ... existing filter controls ... */}
          </div>
        )}
```

Replace the `{/* Content area */}` section (lines 301-361) with tab-conditional rendering:

```tsx
        {/* Content area */}
        <div className="flex flex-1 overflow-hidden">
          {activeTab === 'operations' ? (
            <>
              {/* ... existing operations list + detail panel ... */}
            </>
          ) : (
            /* Sync Events tab */
            <div className="flex-1 overflow-y-auto">
              {syncError && (
                <div className="px-4 py-2 text-sm text-red-600 bg-red-50 border-b border-border">
                  {syncError}
                </div>
              )}
              {syncEvents.length === 0 ? (
                <div className="px-4 py-8 text-center text-muted-foreground text-sm">
                  {t('log.syncNoEvents')}
                </div>
              ) : (
                <table className="w-full text-sm">
                  <thead className="bg-muted/30 sticky top-0">
                    <tr>
                      <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.dateTime')}</th>
                      <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.syncPeer')}</th>
                      <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.syncEventType')}</th>
                      <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.syncDetail')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {syncEvents.map((evt) => (
                      <tr key={evt.id} className="border-b border-border/50 hover:bg-muted/20">
                        <td className="px-4 py-2 text-muted-foreground whitespace-nowrap">
                          {new Date(evt.timestamp * 1000).toLocaleString(i18n.language)}
                        </td>
                        <td className="px-4 py-2">
                          <span className="text-xs font-mono text-muted-foreground">
                            {evt.peerPubkey.length > 12
                              ? `${evt.peerPubkey.slice(0, 8)}…${evt.peerPubkey.slice(-4)}`
                              : evt.peerPubkey}
                          </span>
                        </td>
                        <td className="px-4 py-2">
                          <span className="inline-block bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400 rounded px-2 py-0.5 text-xs font-mono">
                            {evt.eventType === 'bundle_rejected' && t('log.syncBundleRejected')}
                            {evt.eventType === 'signature_invalid' && t('log.syncSignatureInvalid')}
                            {evt.eventType === 'sidecar_mismatch' && t('log.syncSidecarMismatch')}
                            {!['bundle_rejected', 'signature_invalid', 'sidecar_mismatch'].includes(evt.eventType) && evt.eventType}
                          </span>
                        </td>
                        <td className="px-4 py-2 text-xs text-muted-foreground truncate max-w-[300px]" title={evt.detail ?? undefined}>
                          {evt.detail ?? '—'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>
```

Update the footer to show the correct count for the active tab:

```tsx
        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-3 border-t border-border shrink-0">
          <span className="text-sm text-muted-foreground">
            {activeTab === 'operations'
              ? t('log.count', { count: operations.length })
              : t('log.syncCount', { count: syncEvents.length })}
          </span>
          {activeTab === 'operations' && (
            <div className="flex items-center gap-2">
              {/* ... existing purge button ... */}
            </div>
          )}
        </div>
```

- [ ] **Step 4: Verify TypeScript compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/OperationsLogDialog.tsx \
        krillnotes-desktop/src/i18n/locales/*.json
git commit -m "feat: add Sync Events tab to Operations Log dialog with i18n"
```

---

## Task 7: Integration Test + Final Verification

**Files:**
- Test: `krillnotes-core/src/core/swarm/sync.rs` (integration test for C1 + sync_events)

### Steps

- [ ] **Step 1: Write integration test — rejected bundle creates sync_event**

Add to `krillnotes-core/src/core/swarm/sync.rs` tests module:

```rust
    #[test]
    fn test_apply_delta_rejects_tampered_ops_and_logs_sync_event() {
        // Setup: two workspaces (Alice + Bob)
        let alice_key = SigningKey::generate(&mut OsRng);
        let bob_key = SigningKey::generate(&mut OsRng);

        let mut alice_ws = Workspace::create_empty("", "alice-device").unwrap();
        let mut bob_ws = Workspace::create_empty("", "bob-device").unwrap();
        let mut bob_contacts = crate::core::contacts::ContactManager::new(
            bob_ws.storage.connection(),
        );

        // Create a signed operation, then tamper with it.
        let mut op = Operation::UpdateNote {
            operation_id: "op-tamper-integ".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp { wall_ms: 2000, counter: 0, node_id: 1 },
            device_id: "alice-device".to_string(),
            note_id: "note-1".to_string(),
            title: "Original".to_string(),
            modified_by: String::new(),
            signature: String::new(),
        };
        op.sign(&alice_key);
        if let Operation::UpdateNote { ref mut title, .. } = op {
            *title = "TAMPERED".to_string();
        }

        // Build a bundle with the tampered op.
        let bundle = create_delta_bundle(DeltaParams {
            protocol: bob_ws.protocol_id().to_string(),
            workspace_id: bob_ws.workspace_id().to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "alice-device".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: String::new(),
            operations: vec![op],
            sender_key: &alice_key,
            recipient_keys: vec![&bob_key.verifying_key()],
            recipient_peer_ids: vec!["bob-device".to_string()],
            recipient_identity_id: "bob-pk".to_string(),
            owner_pubkey: bob_ws.owner_pubkey().to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![],
        }).unwrap();

        // Apply should fail.
        let result = apply_delta(
            &bundle,
            &mut bob_ws,
            &bob_key,
            &mut bob_contacts,
        );
        assert!(result.is_err());

        // A sync_event should have been logged.
        let events = bob_ws.list_sync_events(10, 0).unwrap();
        assert!(!events.is_empty(), "sync_event should be logged for signature failure");
        assert_eq!(events[0].event_type, "signature_invalid");
    }
```

**Note:** This test may need adjustment based on the actual module visibility and `ContactManager` constructor. Follow patterns from existing `swarm/sync.rs` tests.

- [ ] **Step 2: Run the integration test**

Run: `cargo test -p krillnotes-core test_apply_delta_rejects_tampered`
Expected: PASS

- [ ] **Step 3: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: ALL tests PASS

- [ ] **Step 4: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 5: Final commit (if any fixes needed)**

```bash
git add -A
git commit -m "test: integration test for C1 signature rejection + sync_events logging"
```

---

## Parallelization Notes

Tasks 1-3 are independent and can be dispatched as parallel subagents:
- **Agent 1:** Task 1 (sync_events storage)
- **Agent 2:** Task 2 (H2 zeroize)
- **Agent 3:** Task 3 (M1 sidecar hashes)

Task 4 depends on Task 1 (sync_events logging) and Task 3 (sidecar verification in parse_delta_bundle).

Task 5 depends on Task 1 (re-exported type).

Task 6 depends on Task 5 (Tauri command).

Task 7 depends on Tasks 1, 3, 4 (integration test).

```
Tasks 1, 2, 3  (parallel)
      │
      ▼
   Task 4  (C1 depends on 1+3)
      │
      ▼
   Task 5  (Tauri command)
      │
      ▼
   Task 6  (Frontend)
      │
      ▼
   Task 7  (Integration test)
```
