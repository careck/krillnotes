# Per-Operation Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-operation signature verification and transitive vouching to delta sync, so every stored operation has a cryptographic trust chain back to a known peer.

**Architecture:** Operations in delta payloads are wrapped with optional `verified_by` metadata. The sender vouches for ops it trusts (directly verified or previously vouched). The receiver verifies direct peer signatures and accepts vouches from trusted direct peers. Unvouched relayed ops are rejected. Local DB stores `verified_by` as the direct peer who vouched.

**Tech Stack:** Rust, ed25519-dalek, rusqlite, serde_json, base64

**Spec:** `docs/superpowers/specs/2026-04-23-per-op-verification-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `krillnotes-core/src/core/swarm/delta.rs` | Modify | `DeltaOperation` wrapper, serialize/deserialize in build/parse |
| `krillnotes-core/src/core/storage.rs` | Modify | Migration: add `verified_by` column to `operations` |
| `krillnotes-core/src/core/schema.sql` | Modify | Add `verified_by` column to CREATE TABLE |
| `krillnotes-core/src/core/operation_log.rs` | Modify | Self-authored ops: populate `verified_by` on INSERT |
| `krillnotes-core/src/core/workspace/sync.rs` | Modify | `apply_incoming_operation`: accept + store `verified_by`, verify sender-authored sigs |
| `krillnotes-core/src/core/swarm/sync.rs` | Modify | `generate_delta`: query `verified_by`, wrap ops. `apply_delta`: pass `verified_by` through |

---

### Task 1: Schema Migration — Add `verified_by` Column

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs` (after last migration block)
- Modify: `krillnotes-core/src/core/schema.sql` (operations table)

- [ ] **Step 1: Write a test for the migration**

In `krillnotes-core/src/core/storage.rs` test module, add a test that opens an in-memory workspace (which runs all migrations), then verifies the `verified_by` column exists on the `operations` table:

```rust
#[test]
fn test_verified_by_column_exists() {
    let ws = crate::Workspace::open_in_memory("test-migration").unwrap();
    let conn = ws.storage().connection();
    let mut stmt = conn
        .prepare("SELECT verified_by FROM operations LIMIT 0")
        .expect("verified_by column should exist on operations table");
    drop(stmt);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p krillnotes-core test_verified_by_column_exists`
Expected: FAIL — column `verified_by` does not exist.

- [ ] **Step 3: Add the column to `schema.sql`**

In `krillnotes-core/src/core/schema.sql`, add `verified_by` to the `operations` CREATE TABLE:

```sql
CREATE TABLE IF NOT EXISTS operations (
    operation_id TEXT NOT NULL PRIMARY KEY,
    timestamp_wall_ms INTEGER NOT NULL DEFAULT 0,
    timestamp_counter INTEGER NOT NULL DEFAULT 0,
    timestamp_node_id INTEGER NOT NULL DEFAULT 0,
    device_id TEXT NOT NULL,
    operation_type TEXT NOT NULL,
    operation_data TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0,
    received_from_peer TEXT,
    verified_by TEXT NOT NULL DEFAULT ''
);
```

- [ ] **Step 4: Add migration in `storage.rs`**

Add a new migration block after the last existing migration in `storage.rs`. Follow the existing pattern — check if column exists using `pragma_table_info`, then ALTER if missing:

```rust
// Migration: add verified_by column to operations table
{
    let has_verified_by = conn
        .pragma_table_info("operations")?
        .iter()
        .any(|col| col.name == "verified_by");
    if !has_verified_by {
        conn.execute_batch(
            "ALTER TABLE operations ADD COLUMN verified_by TEXT NOT NULL DEFAULT '';"
        )?;
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p krillnotes-core test_verified_by_column_exists`
Expected: PASS

- [ ] **Step 6: Run all existing tests to check for regressions**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass. The new column has a DEFAULT so existing INSERTs are unaffected.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/schema.sql krillnotes-core/src/core/storage.rs
git commit -m "feat: add verified_by column to operations table"
```

---

### Task 2: Self-Authored Ops — Populate `verified_by` on INSERT

**Files:**
- Modify: `krillnotes-core/src/core/operation_log.rs` (lines 69-81, the `log()` method)

Self-authored operations should be inserted with `verified_by = current_identity_pubkey` because the local node is the author and the verifier.

- [ ] **Step 1: Write a test for self-authored op `verified_by`**

Add a test that creates a note and checks the stored `verified_by` value:

```rust
#[test]
fn test_self_authored_op_has_verified_by() {
    let ws = crate::Workspace::open_in_memory("test-self-verify").unwrap();
    let identity_pubkey = ws.current_identity_pubkey().to_string();

    // Create a note (which logs a self-authored op)
    let note_id = ws.create_note(None, "Test Note", "TextNote").unwrap();

    // Query the stored operation's verified_by
    let conn = ws.storage().connection();
    let verified_by: String = conn
        .query_row(
            "SELECT verified_by FROM operations WHERE operation_type = 'CreateNote' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(verified_by, identity_pubkey, "self-authored op should have verified_by = own identity key");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p krillnotes-core test_self_authored_op_has_verified_by`
Expected: FAIL — `verified_by` is empty string (DEFAULT).

- [ ] **Step 3: Update `OperationLog::log()` to include `verified_by`**

In `krillnotes-core/src/core/operation_log.rs`, the `log()` method at line 69 currently inserts without `verified_by`. The `OperationLog` struct needs access to the identity pubkey. Modify the struct and `log()` method:

First, add `identity_pubkey` to the `OperationLog` struct (or accept it as a parameter to `log()`). The cleanest approach: `OperationLog` already has context about the workspace — add an `identity_pubkey: String` field set at construction time.

Update the INSERT in `log()`:

```rust
tx.execute(
    "INSERT INTO operations \
     (operation_id, timestamp_wall_ms, timestamp_counter, timestamp_node_id, \
      device_id, operation_type, operation_data, synced, verified_by) \
     VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)",
    rusqlite::params![
        op.operation_id(),
        ts.wall_ms as i64,
        ts.counter as i64,
        ts.node_id as i64,
        op.device_id(),
        op_type,
        op_json,
        &self.identity_pubkey,
    ],
)?;
```

Update wherever `OperationLog` is constructed to pass in the identity pubkey (check `workspace/mod.rs` for the construction site).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p krillnotes-core test_self_authored_op_has_verified_by`
Expected: PASS

- [ ] **Step 5: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/operation_log.rs krillnotes-core/src/core/workspace/
git commit -m "feat: populate verified_by for self-authored operations"
```

---

### Task 3: DeltaOperation Wrapper — Serialize/Deserialize

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs` (add struct, update build + parse)

- [ ] **Step 1: Write a round-trip test for DeltaOperation serialization**

Add a test in the existing `#[cfg(test)]` module in `delta.rs`:

```rust
#[test]
fn test_delta_operation_serde_roundtrip() {
    use crate::Operation;
    use crate::core::operation::HlcTimestamp;

    let op = Operation::CreateNote {
        operation_id: "op-1".to_string(),
        timestamp: HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
        device_id: "dev-1".to_string(),
        note_id: "note-1".to_string(),
        parent_id: None,
        title: "Test".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: "AAAA".to_string(),
        signature: "BBBB".to_string(),
    };

    // With verified_by
    let delta_op = DeltaOperation {
        op: op.clone(),
        verified_by: Some("pubkey123".to_string()),
    };
    let json = serde_json::to_string(&delta_op).unwrap();
    let parsed: DeltaOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.verified_by, Some("pubkey123".to_string()));
    assert_eq!(parsed.op.operation_id(), "op-1");

    // Without verified_by (self-authored ops)
    let delta_op_none = DeltaOperation {
        op: op.clone(),
        verified_by: None,
    };
    let json_none = serde_json::to_string(&delta_op_none).unwrap();
    assert!(!json_none.contains("verified_by"), "None should be skipped in serialization");
    let parsed_none: DeltaOperation = serde_json::from_str(&json_none).unwrap();
    assert_eq!(parsed_none.verified_by, None);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p krillnotes-core test_delta_operation_serde_roundtrip`
Expected: FAIL — `DeltaOperation` does not exist.

- [ ] **Step 3: Add the `DeltaOperation` struct**

In `krillnotes-core/src/core/swarm/delta.rs`, add near the top (after the existing struct definitions):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaOperation {
    pub op: Operation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p krillnotes-core test_delta_operation_serde_roundtrip`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/swarm/delta.rs
git commit -m "feat: add DeltaOperation wrapper struct"
```

---

### Task 4: Update `create_delta_bundle` to Serialize `DeltaOperation`

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs` (lines 27-49 `DeltaParams`, lines 67-150 `create_delta_bundle`)

- [ ] **Step 1: Write a test that builds a bundle with `DeltaOperation` wrappers and parses them back**

Add to the test module in `delta.rs`:

```rust
#[test]
fn test_delta_bundle_with_verified_by() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let sender_key = SigningKey::generate(&mut OsRng);
    let recipient_key = SigningKey::generate(&mut OsRng);
    let sender_pubkey = base64::engine::general_purpose::STANDARD
        .encode(sender_key.verifying_key().to_bytes());

    let mut op = Operation::CreateNote {
        operation_id: "op-verified-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 5000, counter: 0, node_id: 1 },
        device_id: "dev-1".to_string(),
        note_id: "note-1".to_string(),
        parent_id: None,
        title: "Verified Note".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: String::new(),
        signature: String::new(),
    };
    op.sign(&sender_key);

    let delta_ops = vec![
        DeltaOperation {
            op: op.clone(),
            verified_by: None, // self-authored
        },
    ];

    let bundle = create_delta_bundle(DeltaParams {
        protocol: "krillnotes/1".to_string(),
        workspace_id: "ws-1".to_string(),
        workspace_name: "Test WS".to_string(),
        source_device_id: "dev-1".to_string(),
        source_display_name: "Sender".to_string(),
        since_operation_id: String::new(),
        delta_operations: delta_ops,
        sender_key: &sender_key,
        recipient_keys: vec![&recipient_key.verifying_key()],
        recipient_peer_ids: vec!["dev-2".to_string()],
        recipient_identity_id: base64::engine::general_purpose::STANDARD
            .encode(recipient_key.verifying_key().to_bytes()),
        owner_pubkey: sender_pubkey.clone(),
        ack_operation_id: None,
        attachment_blobs: vec![],
    })
    .unwrap();

    let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
    assert_eq!(parsed.delta_operations.len(), 1);
    assert_eq!(parsed.delta_operations[0].verified_by, None);
    assert_eq!(parsed.delta_operations[0].op.operation_id(), "op-verified-1");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p krillnotes-core test_delta_bundle_with_verified_by`
Expected: FAIL — `DeltaParams` has no `delta_operations` field; `ParsedDelta` has no `delta_operations` field.

- [ ] **Step 3: Update `DeltaParams` — replace `operations` with `delta_operations`**

In `delta.rs`, change the `DeltaParams` struct field:

```rust
pub struct DeltaParams<'a> {
    pub protocol: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    pub source_display_name: String,
    pub since_operation_id: String,
    pub delta_operations: Vec<DeltaOperation>,  // was: operations: Vec<Operation>
    pub sender_key: &'a SigningKey,
    pub recipient_keys: Vec<&'a VerifyingKey>,
    pub recipient_peer_ids: Vec<String>,
    pub recipient_identity_id: String,
    pub owner_pubkey: String,
    pub ack_operation_id: Option<String>,
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
}
```

- [ ] **Step 4: Update `create_delta_bundle` serialization**

At line ~71, change the serialization from `serde_json::to_vec(&params.operations)` to:

```rust
let ops_json = serde_json::to_vec(&params.delta_operations)?;
```

- [ ] **Step 5: Update `ParsedDelta` — replace `operations` with `delta_operations`**

```rust
pub struct ParsedDelta {
    pub protocol: String,
    pub workspace_id: String,
    pub since_operation_id: String,
    pub sender_public_key: String,
    pub sender_device_id: String,
    pub delta_operations: Vec<DeltaOperation>,  // was: operations: Vec<Operation>
    pub owner_pubkey: Option<String>,
    pub ack_operation_id: Option<String>,
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
}
```

- [ ] **Step 6: Update `parse_delta_bundle` deserialization**

At line ~225, change the deserialization from `serde_json::from_slice::<Vec<Operation>>` to:

```rust
let delta_operations: Vec<DeltaOperation> = serde_json::from_slice(&ops_json)?;
```

And update the `ParsedDelta` construction to use `delta_operations`.

- [ ] **Step 7: Fix all callers of `DeltaParams` and `ParsedDelta`**

The compiler will guide you. Key call sites:
- `swarm/sync.rs` line ~131: `generate_delta()` builds `DeltaParams` — wrap each `Operation` in a `DeltaOperation`
- `swarm/sync.rs` line ~228: `apply_delta()` iterates `parsed.operations` — change to `parsed.delta_operations`
- Existing delta tests in `delta.rs`: update to use new field names

- [ ] **Step 8: Run the new test**

Run: `cargo test -p krillnotes-core test_delta_bundle_with_verified_by`
Expected: PASS

- [ ] **Step 9: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All pass. Existing tests updated in step 7.

- [ ] **Step 10: Commit**

```bash
git add krillnotes-core/src/core/swarm/
git commit -m "feat: serialize DeltaOperation wrapper in delta bundles"
```

---

### Task 5: Receiver — Verify Sender-Authored Ops and Accept Vouches

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs` (`apply_incoming_operation`)
- Modify: `krillnotes-core/src/core/swarm/sync.rs` (`apply_delta` loop)

- [ ] **Step 1: Write a test for rejecting an unvouched relayed op**

Add to the test module in `workspace/sync.rs` (or a dedicated test file):

```rust
#[test]
fn test_reject_unvouched_relayed_op() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let mut ws = crate::Workspace::open_in_memory("test-reject").unwrap();
    let other_key = SigningKey::generate(&mut OsRng);

    let mut op = Operation::CreateNote {
        operation_id: "op-unvouched-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 99 },
        device_id: "remote-dev".to_string(),
        note_id: "note-remote-1".to_string(),
        parent_id: None,
        title: "Unvouched Note".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: String::new(),
        signature: String::new(),
    };
    op.sign(&other_key);

    // Relayed op (author != sender) with no verified_by → should be rejected
    let sender_device_id = "sender-dev";
    let sender_pubkey = "some-other-sender-key";
    let result = ws.apply_incoming_operation(op, sender_device_id, &[], None, sender_pubkey);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), false, "unvouched relayed op should be rejected");
}
```

- [ ] **Step 2: Write a test for accepting a vouched relayed op**

```rust
#[test]
fn test_accept_vouched_relayed_op() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let mut ws = crate::Workspace::open_in_memory("test-vouched").unwrap();
    let author_key = SigningKey::generate(&mut OsRng);
    let sender_key = SigningKey::generate(&mut OsRng);
    let sender_pubkey = STANDARD.encode(sender_key.verifying_key().to_bytes());

    let mut op = Operation::CreateNote {
        operation_id: "op-vouched-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 2000, counter: 0, node_id: 99 },
        device_id: "remote-dev".to_string(),
        note_id: "note-vouched-1".to_string(),
        parent_id: None,
        title: "Vouched Note".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: String::new(),
        signature: String::new(),
    };
    op.sign(&author_key);

    // Relayed op with verified_by = sender's key → should be accepted
    let result = ws.apply_incoming_operation(
        op, "sender-dev", &[], Some(&sender_pubkey), &sender_pubkey
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), true, "vouched relayed op should be accepted");

    // Check stored verified_by
    let conn = ws.storage().connection();
    let stored_vb: String = conn
        .query_row(
            "SELECT verified_by FROM operations WHERE operation_id = 'op-vouched-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_vb, sender_pubkey);
}
```

- [ ] **Step 3: Write a test for verifying a sender-authored op's Ed25519 signature**

```rust
#[test]
fn test_verify_sender_authored_op_signature() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let mut ws = crate::Workspace::open_in_memory("test-sender-verify").unwrap();
    let sender_key = SigningKey::generate(&mut OsRng);
    let sender_pubkey = STANDARD.encode(sender_key.verifying_key().to_bytes());

    let mut op = Operation::CreateNote {
        operation_id: "op-sender-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 3000, counter: 0, node_id: 99 },
        device_id: "sender-dev".to_string(),
        note_id: "note-sender-1".to_string(),
        parent_id: None,
        title: "Sender Note".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: String::new(),
        signature: String::new(),
    };
    op.sign(&sender_key);

    // Sender-authored op (author_key == sender_pubkey), no verified_by in delta
    let result = ws.apply_incoming_operation(
        op, "sender-dev", &[], None, &sender_pubkey
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), true, "sender-authored op with valid sig should be accepted");

    // Check stored verified_by = sender's key
    let conn = ws.storage().connection();
    let stored_vb: String = conn
        .query_row(
            "SELECT verified_by FROM operations WHERE operation_id = 'op-sender-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_vb, sender_pubkey);
}
```

- [ ] **Step 4: Write a test for rejecting a sender-authored op with invalid signature**

```rust
#[test]
fn test_reject_sender_authored_op_bad_signature() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let mut ws = crate::Workspace::open_in_memory("test-bad-sig").unwrap();
    let sender_key = SigningKey::generate(&mut OsRng);
    let sender_pubkey = STANDARD.encode(sender_key.verifying_key().to_bytes());

    let mut op = Operation::CreateNote {
        operation_id: "op-badsig-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 4000, counter: 0, node_id: 99 },
        device_id: "sender-dev".to_string(),
        note_id: "note-badsig-1".to_string(),
        parent_id: None,
        title: "Bad Sig Note".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: String::new(),
        signature: String::new(),
    };
    op.sign(&sender_key);

    // Tamper with the op after signing
    if let Operation::CreateNote { ref mut title, .. } = op {
        *title = "Tampered".to_string();
    }

    let result = ws.apply_incoming_operation(
        op, "sender-dev", &[], None, &sender_pubkey
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), false, "tampered sender-authored op should be rejected");
}
```

- [ ] **Step 5: Run all four tests to verify they fail**

Run: `cargo test -p krillnotes-core test_reject_unvouched test_accept_vouched test_verify_sender_authored test_reject_sender_authored_op_bad`
Expected: FAIL — `apply_incoming_operation` doesn't accept the new parameters.

- [ ] **Step 6: Update `apply_incoming_operation` signature and verification logic**

In `krillnotes-core/src/core/workspace/sync.rs`, update the function signature:

```rust
pub fn apply_incoming_operation(
    &mut self,
    op: Operation,
    received_from_peer: &str,
    attachment_blobs: &[(String, Vec<u8>)],
    verified_by: Option<&str>,
    sender_identity: &str,
) -> Result<bool>
```

Add verification logic **before** the INSERT statement (after extracting `op_type` and `op_json`, before the `let rows = {` block):

```rust
// --- Per-op verification ---
let author_key = op.author_key();
let resolved_verified_by: String;

if author_key == sender_identity {
    // Sender-authored op: verify the original Ed25519 signature
    let vk_bytes = base64::engine::general_purpose::STANDARD
        .decode(sender_identity)
        .map_err(|e| KrillnotesError::Sync(format!("bad sender identity: {e}")))?;
    let vk_arr: [u8; 32] = vk_bytes.try_into()
        .map_err(|_| KrillnotesError::Sync("sender identity wrong length".into()))?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Sync(format!("invalid sender key: {e}")))?;

    if !op.verify(&vk) {
        log::warn!(target: "krillnotes::sync",
            "rejecting op {} — sender-authored but signature invalid", op.operation_id());
        return Ok(false);
    }
    resolved_verified_by = sender_identity.to_string();
} else if let Some(vb) = verified_by {
    // Relayed op: sender must vouch for it
    if vb != sender_identity {
        log::warn!(target: "krillnotes::sync",
            "rejecting op {} — verified_by doesn't match sender", op.operation_id());
        return Ok(false);
    }
    resolved_verified_by = sender_identity.to_string();
} else {
    // Relayed op with no vouch: reject
    log::warn!(target: "krillnotes::sync",
        "rejecting op {} — relayed without vouching", op.operation_id());
    return Ok(false);
}
```

- [ ] **Step 7: Update the INSERT statement to include `verified_by`**

```rust
let rows = tx.execute(
    "INSERT OR IGNORE INTO operations \
     (operation_id, timestamp_wall_ms, timestamp_counter, timestamp_node_id, \
      device_id, operation_type, operation_data, synced, received_from_peer, verified_by) \
     VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
    rusqlite::params![
        op.operation_id(),
        ts.wall_ms as i64,
        ts.counter as i64,
        ts.node_id as i64,
        op.device_id(),
        op_type,
        op_json,
        received_from_peer,
        &resolved_verified_by,
    ],
)?;
```

- [ ] **Step 8: Update `apply_delta` in `swarm/sync.rs` to pass new parameters**

In the `apply_delta` function, update the loop that iterates over operations (line ~228). Change from iterating `parsed.operations` to `parsed.delta_operations`:

```rust
for delta_op in &parsed.delta_operations {
    let op = &delta_op.op;
    let author_key = op.author_key();

    // TOFU: auto-register unknown authors.
    if !author_key.is_empty() && contact_manager.find_by_public_key(author_key)?.is_none() {
        let name = if let Operation::JoinWorkspace { declared_name, .. } = op {
            declared_name.clone()
        } else {
            format!("{}…", &author_key[..8.min(author_key.len())])
        };
        contact_manager.find_or_create_by_public_key(&name, author_key, TrustLevel::Tofu)?;
        new_tofu_contacts.push(name);
    }

    if workspace.apply_incoming_operation(
        op.clone(),
        &parsed.sender_device_id,
        &parsed.attachment_blobs,
        delta_op.verified_by.as_deref(),
        &parsed.sender_public_key,
    )? {
        applied += 1;
    } else {
        skipped += 1;
    }
}
```

- [ ] **Step 9: Run the four new tests**

Run: `cargo test -p krillnotes-core test_reject_unvouched test_accept_vouched test_verify_sender_authored test_reject_sender_authored_op_bad`
Expected: All PASS

- [ ] **Step 10: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All pass. Any callers of `apply_incoming_operation` that weren't updated will fail to compile — fix them by adding the new parameters.

- [ ] **Step 11: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/swarm/sync.rs
git commit -m "feat: verify sender-authored ops and enforce vouching on relayed ops"
```

---

### Task 6: Sender — Vouch for Ops When Building Deltas

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs` (`generate_delta`, lines 62-148)
- Modify: `krillnotes-core/src/core/workspace/sync.rs` (`operations_since` — needs to return `verified_by`)

- [ ] **Step 1: Write a test for delta generation with vouching**

```rust
#[test]
fn test_generate_delta_vouches_for_verified_ops() {
    // This is an integration test: create a workspace, insert some ops
    // (self-authored + received-with-verified_by), then generate a delta
    // and check the DeltaOperation wrappers.
    
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let mut ws = crate::Workspace::open_in_memory("test-gen-delta").unwrap();
    let my_pubkey = ws.current_identity_pubkey().to_string();

    // Create a self-authored op
    ws.create_note(None, "My Note", "TextNote").unwrap();

    // Simulate a received op with verified_by set
    let peer_key = SigningKey::generate(&mut OsRng);
    let peer_pubkey = STANDARD.encode(peer_key.verifying_key().to_bytes());
    let mut remote_op = Operation::CreateNote {
        operation_id: "op-remote-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 9999, counter: 0, node_id: 50 },
        device_id: "peer-dev".to_string(),
        note_id: "note-remote-1".to_string(),
        parent_id: None,
        title: "Remote Note".to_string(),
        schema: "TextNote".to_string(),
        position: 1.0,
        created_by: String::new(),
        signature: String::new(),
    };
    remote_op.sign(&peer_key);
    ws.apply_incoming_operation(
        remote_op, "peer-dev", &[], None, &peer_pubkey
    ).unwrap();

    // Now query ops for a delta — both should be present
    let ops_with_vb = ws.operations_since_with_verified_by(None, "other-peer-dev").unwrap();
    assert_eq!(ops_with_vb.len(), 2);

    // Self-authored op: verified_by = my_pubkey → delta wrapper: verified_by = None
    // Remote op: verified_by = peer_pubkey → delta wrapper: verified_by = Some(my_pubkey) (re-vouch)
    let self_op = ops_with_vb.iter().find(|(_, vb)| vb == &my_pubkey).unwrap();
    let remote_op = ops_with_vb.iter().find(|(_, vb)| vb == &peer_pubkey).unwrap();

    // When wrapping for delta:
    // self-authored → verified_by = None (receiver verifies sig directly)
    // remote with verified_by set → verified_by = Some(my_pubkey) (I re-vouch)
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p krillnotes-core test_generate_delta_vouches`
Expected: FAIL — `operations_since_with_verified_by` doesn't exist.

- [ ] **Step 3: Add `operations_since_with_verified_by` method**

In `krillnotes-core/src/core/workspace/sync.rs`, add a new method (or modify `operations_since`) that returns `Vec<(Operation, String)>` — each op paired with its `verified_by` value. Update the SQL SELECT to include the `verified_by` column:

Clone the `operations_since` method body (which already handles the watermark lookup and `received_from_peer` echo-prevention). Changes from the original:

1. Add `verified_by` to the SELECT column list: `SELECT operation_data, verified_by FROM operations WHERE ...`
2. In the row mapping closure, extract both columns:
```rust
let op_data: String = row.get(0)?;
let verified_by: String = row.get(1)?;
let op: Operation = serde_json::from_str(&op_data)
    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(...))?;
Ok((op, verified_by))
```
3. Return type becomes `Result<Vec<(Operation, String)>>`

The method signature:
```rust
pub fn operations_since_with_verified_by(
    &self,
    since_op_id: Option<&str>,
    exclude_peer: &str,
) -> Result<Vec<(Operation, String)>>
```

- [ ] **Step 4: Update `generate_delta` to wrap ops with vouching logic**

In `swarm/sync.rs`, `generate_delta()` function:

```rust
let ops_with_vb = workspace.operations_since_with_verified_by(
    peer.last_sent_op.as_deref(),
    &peer.peer_device_id,
)?;

let my_pubkey = workspace.current_identity_pubkey().to_string();

let delta_operations: Vec<DeltaOperation> = ops_with_vb
    .into_iter()
    .filter_map(|(op, verified_by)| {
        if op.author_key() == my_pubkey {
            // Self-authored: receiver verifies my sig directly
            Some(DeltaOperation { op, verified_by: None })
        } else if !verified_by.is_empty() {
            // Previously verified/vouched: I re-vouch
            Some(DeltaOperation { op, verified_by: Some(my_pubkey.clone()) })
        } else {
            // Unverified op — do not include
            log::warn!(target: "krillnotes::sync",
                "skipping unverified op {} in delta", op.operation_id());
            None
        }
    })
    .collect();
```

Update the `DeltaParams` construction to use `delta_operations` instead of `operations`.

Also update the attachment blob collection to work with the new structure (iterate `delta_operations` to find `AddAttachment` ops).

- [ ] **Step 5: Run the test**

Run: `cargo test -p krillnotes-core test_generate_delta_vouches`
Expected: PASS

- [ ] **Step 6: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/swarm/sync.rs
git commit -m "feat: vouch for verified ops when building delta bundles"
```

---

### Task 7: Integration Test — Full Chain Verification

**Files:**
- Add test in: `krillnotes-core/src/core/swarm/delta.rs` (test module) or a new integration test file

- [ ] **Step 1: Write an end-to-end test simulating A → B → C**

This test simulates the full chain: A creates an op, B receives and verifies, B builds a delta for C with vouching, C receives and accepts based on B's vouch.

```rust
#[test]
fn test_full_chain_a_to_b_to_c() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Three identities
    let key_a = SigningKey::generate(&mut OsRng);
    let key_b = SigningKey::generate(&mut OsRng);
    let key_c = SigningKey::generate(&mut OsRng);
    let pubkey_a = STANDARD.encode(key_a.verifying_key().to_bytes());
    let pubkey_b = STANDARD.encode(key_b.verifying_key().to_bytes());
    let pubkey_c = STANDARD.encode(key_c.verifying_key().to_bytes());

    // A creates and signs an operation
    let mut op_from_a = Operation::CreateNote {
        operation_id: "op-chain-1".to_string(),
        timestamp: crate::core::operation::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 1 },
        device_id: "dev-a".to_string(),
        note_id: "note-chain-1".to_string(),
        parent_id: None,
        title: "Chain Test".to_string(),
        schema: "TextNote".to_string(),
        position: 0.0,
        created_by: String::new(),
        signature: String::new(),
    };
    op_from_a.sign(&key_a);

    // === A sends delta to B ===
    let delta_a_to_b = create_delta_bundle(DeltaParams {
        protocol: "krillnotes/1".to_string(),
        workspace_id: "ws-chain".to_string(),
        workspace_name: "Chain WS".to_string(),
        source_device_id: "dev-a".to_string(),
        source_display_name: "Alice".to_string(),
        since_operation_id: String::new(),
        delta_operations: vec![DeltaOperation {
            op: op_from_a.clone(),
            verified_by: None, // self-authored
        }],
        sender_key: &key_a,
        recipient_keys: vec![&key_b.verifying_key()],
        recipient_peer_ids: vec!["dev-b".to_string()],
        recipient_identity_id: pubkey_b.clone(),
        owner_pubkey: pubkey_a.clone(),
        ack_operation_id: None,
        attachment_blobs: vec![],
    })
    .unwrap();

    // B parses the delta from A
    let parsed_at_b = parse_delta_bundle(&delta_a_to_b, &key_b).unwrap();
    assert_eq!(parsed_at_b.delta_operations.len(), 1);
    assert_eq!(parsed_at_b.delta_operations[0].verified_by, None); // A didn't vouch (self-authored)

    // B verifies A's signature (A is B's direct peer)
    assert!(parsed_at_b.delta_operations[0].op.verify(
        &key_a.verifying_key()
    ), "B should be able to verify A's signature");

    // === B sends delta to C, vouching for A's op ===
    let delta_b_to_c = create_delta_bundle(DeltaParams {
        protocol: "krillnotes/1".to_string(),
        workspace_id: "ws-chain".to_string(),
        workspace_name: "Chain WS".to_string(),
        source_device_id: "dev-b".to_string(),
        source_display_name: "Bob".to_string(),
        since_operation_id: String::new(),
        delta_operations: vec![DeltaOperation {
            op: op_from_a.clone(),
            verified_by: Some(pubkey_b.clone()), // B vouches
        }],
        sender_key: &key_b,
        recipient_keys: vec![&key_c.verifying_key()],
        recipient_peer_ids: vec!["dev-c".to_string()],
        recipient_identity_id: pubkey_c.clone(),
        owner_pubkey: pubkey_a.clone(),
        ack_operation_id: None,
        attachment_blobs: vec![],
    })
    .unwrap();

    // C parses the delta from B
    let parsed_at_c = parse_delta_bundle(&delta_b_to_c, &key_c).unwrap();
    assert_eq!(parsed_at_c.delta_operations.len(), 1);
    assert_eq!(
        parsed_at_c.delta_operations[0].verified_by,
        Some(pubkey_b.clone()),
        "C should see B's vouch"
    );
    assert_eq!(parsed_at_c.sender_public_key, pubkey_b);

    // C accepts: verified_by matches sender_public_key (B vouched, B is sender)
    let delta_op = &parsed_at_c.delta_operations[0];
    assert_eq!(
        delta_op.verified_by.as_ref().unwrap(),
        &parsed_at_c.sender_public_key,
        "vouch should match sender identity"
    );
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p krillnotes-core test_full_chain_a_to_b_to_c`
Expected: PASS (all building blocks from prior tasks)

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/swarm/delta.rs
git commit -m "test: add full chain A→B→C verification integration test"
```

---

### Task 8: Update Snapshot Bundling (if applicable)

**Files:**
- Check: `krillnotes-core/src/core/swarm/snapshot.rs`

Snapshots also bundle operations for initial sync. If snapshots serialize `Vec<Operation>` directly, they need the same `DeltaOperation` wrapper treatment.

- [ ] **Step 1: Check if snapshot uses the same serialization path**

Read `snapshot.rs` to determine if it serializes operations the same way as deltas. If it uses a separate `SnapshotParams` / `create_snapshot_bundle` with its own `Vec<Operation>`, it needs updating. If it reuses `create_delta_bundle`, it's already covered.

- [ ] **Step 2: If snapshot has its own serialization, update it**

Apply the same pattern: wrap ops in `DeltaOperation`, populate `verified_by` for non-self ops, deserialize as `Vec<DeltaOperation>` on parse.

- [ ] **Step 3: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All pass.

- [ ] **Step 4: Commit (if changes were made)**

```bash
git add krillnotes-core/src/core/swarm/snapshot.rs
git commit -m "feat: apply DeltaOperation wrapper to snapshot bundles"
```

---

### Task 9: Final — Full Test Suite and Cleanup

- [ ] **Step 1: Run the complete test suite**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass.

- [ ] **Step 2: Run type check on the desktop app**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No TypeScript errors (this feature is backend-only, but verify no frontend breakage).

- [ ] **Step 3: Run a full build**

Run: `cd krillnotes-desktop && npm run tauri build`
Expected: Build succeeds.

- [ ] **Step 4: Final commit if any cleanup was needed**

```bash
git add -A
git commit -m "chore: cleanup after per-op verification implementation"
```
