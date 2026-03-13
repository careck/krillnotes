# Delta Generation & Ingest Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement A12 (delta bundle generation) and A13 (delta bundle ingest stub) so peers who share a snapshot can exchange ongoing changes via `.swarm` delta files.

**Architecture:** A new `swarm/sync.rs` orchestration module bridges the existing delta codec, peer registry, and contact manager. Two new workspace primitives (`operations_since`, `apply_incoming_operation`) are added to `workspace.rs`. A batch Tauri command (`generate_deltas_for_peers`) drives a new `CreateDeltaDialog.tsx`. A one-line A11 snapshot-import bug fix is a prerequisite for bidirectional exchange.

**Tech Stack:** Rust (rusqlite, ed25519-dalek, serde_json, base64), Tauri v2, React 19, TypeScript, Tailwind v4

**Spec:** `docs/superpowers/specs/2026-03-13-delta-generation-design.md`

---

## Worktree setup (do this first)

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/delta-generation -b feat/delta-generation
cd /Users/careck/Source/Krillnotes/.worktrees/feat/delta-generation
```

Test commands:
- Core: `cargo test -p krillnotes-core 2>&1 | tail -20`
- TypeScript: `cd krillnotes-desktop && npx tsc --noEmit`

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `krillnotes-core/src/core/swarm/delta.rs` | Modify | Add `sender_device_id` to `ParsedDelta` |
| `krillnotes-core/src/core/swarm/sync.rs` | **Create** | Orchestration: `generate_delta`, `apply_delta`, `ApplyResult` |
| `krillnotes-core/src/core/swarm/mod.rs` | Modify | Export `pub mod sync` |
| `krillnotes-core/src/core/storage.rs` | Modify | Add HLC covering index migration |
| `krillnotes-core/src/core/schema.sql` | Modify | Add HLC index for new databases |
| `krillnotes-core/src/core/workspace.rs` | Modify | Add `operations_since`, `apply_incoming_operation` |
| `krillnotes-core/src/lib.rs` | Modify | Re-export `sync::ApplyResult` |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Modify | Fix A11 bug, add 2 commands, extend `handle_swarm_open` |
| `krillnotes-desktop/src-tauri/src/menu.rs` | Modify | Add "Create delta Swarm" menu item |
| `krillnotes-desktop/src/i18n/locales/*.json` | Modify | Add `createDeltaSwarm` key (7 files) |
| `krillnotes-desktop/src/App.tsx` | Modify | Wire menu event → dialog state |
| `krillnotes-desktop/src/types.ts` | Modify | Add `GenerateDeltasResult` TS interface (verify `PeerInfo` exists) |
| `krillnotes-desktop/src/components/CreateDeltaDialog.tsx` | **Create** | Batch delta export UI |

---

## Chunk 1: Core Primitives

### Task 1: Extend `ParsedDelta` with `sender_device_id`

**Files:** `krillnotes-core/src/core/swarm/delta.rs`

- [ ] **Write the failing test** — extend `test_delta_roundtrip` to assert the new field:

```rust
// In the existing test_delta_roundtrip, after:
let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
// Add:
assert_eq!(parsed.sender_device_id, "dev-1");
```

- [ ] **Run to confirm it fails**

```bash
cargo test -p krillnotes-core swarm::delta::tests::test_delta_roundtrip 2>&1 | tail -10
```

Expected: compile error — field `sender_device_id` does not exist

- [ ] **Implement** — add the field to the struct and populate it in `parse_delta_bundle`:

In `ParsedDelta`:
```rust
pub struct ParsedDelta {
    pub workspace_id: String,
    pub since_operation_id: String,
    pub sender_public_key: String,
    pub sender_device_id: String,   // NEW — from header.source_device_id
    pub operations: Vec<Operation>,
}
```

In `parse_delta_bundle`, after `let header: SwarmHeader = ...`:
```rust
// Already parsed. Now extract sender_device_id.
// (later in the Ok return):
Ok(ParsedDelta {
    workspace_id: header.workspace_id,
    since_operation_id: header.since_operation_id.unwrap_or_default(),
    sender_public_key: header.source_identity,
    sender_device_id: header.source_device_id,  // NEW
    operations,
})
```

- [ ] **Run to confirm it passes**

```bash
cargo test -p krillnotes-core swarm::delta 2>&1 | tail -10
```

Expected: all delta tests pass

- [ ] **Commit**

```bash
git add krillnotes-core/src/core/swarm/delta.rs
git commit -m "feat(swarm): add sender_device_id to ParsedDelta"
```

---

### Task 2: HLC covering index

**Files:** `krillnotes-core/src/core/storage.rs`, `krillnotes-core/src/core/schema.sql`

- [ ] **Write a failing test** — add to the existing test block in `storage.rs` (or near the end of the file):

```rust
#[test]
fn test_hlc_index_exists_after_migration() {
    let f = tempfile::NamedTempFile::new().unwrap();
    let s = Storage::create(f.path(), "").unwrap();
    let count: i64 = s.connection().query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_operations_hlc'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1, "HLC index should exist after create");
}
```

- [ ] **Run to confirm it fails**

```bash
cargo test -p krillnotes-core test_hlc_index_exists 2>&1 | tail -10
```

- [ ] **Implement** — add to `schema.sql` (at the end, after the last `CREATE TABLE`):

```sql
CREATE INDEX IF NOT EXISTS idx_operations_hlc
    ON operations(timestamp_wall_ms, timestamp_counter, timestamp_node_id);
```

Add to `run_migrations` in `storage.rs` (copy the pattern from the note_tags migration):

```rust
// Migration: add HLC covering index for operations_since queries.
let hlc_index_exists: bool = conn.query_row(
    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_operations_hlc'",
    [],
    |row| row.get::<_, i64>(0).map(|c| c > 0),
)?;
if !hlc_index_exists {
    conn.execute(
        "CREATE INDEX idx_operations_hlc \
         ON operations(timestamp_wall_ms, timestamp_counter, timestamp_node_id)",
        [],
    )?;
}
```

- [ ] **Run to confirm it passes**

```bash
cargo test -p krillnotes-core test_hlc_index 2>&1 | tail -10
```

- [ ] **Commit**

```bash
git add krillnotes-core/src/core/storage.rs krillnotes-core/src/core/schema.sql
git commit -m "feat(storage): add HLC covering index for delta generation"
```

---

### Task 3: `Workspace::operations_since`

**Files:** `krillnotes-core/src/core/workspace.rs`

- [ ] **Write failing tests** — add inside the existing `#[cfg(test)]` block (look for it near the bottom of workspace.rs with `use super::*`):

```rust
#[test]
fn test_operations_since_empty() {
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
    // No operations yet, so operations_since(None, "other-device") returns empty
    let ops = ws.operations_since(None, "other-device").unwrap();
    assert!(ops.is_empty());
}

#[test]
fn test_operations_since_watermark() {
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
    let root = ws.list_all_notes().unwrap()[0].clone();
    // Create two child notes to generate two CreateNote operations
    let id1 = ws.create_note(Some(&root.id), "Note 1", crate::AddPosition::AsChild).unwrap();
    let id2 = ws.create_note(Some(&root.id), "Note 2", crate::AddPosition::AsChild).unwrap();

    // Get the operation_id for id1's creation (it was the first op)
    let all_ops = ws.operations_since(None, "nonexistent-device").unwrap();
    assert_eq!(all_ops.len(), 2);
    let first_op_id = all_ops[0].operation_id().to_string();

    // Only id2's op should be returned when watermark = first_op_id
    let since_ops = ws.operations_since(Some(&first_op_id), "nonexistent-device").unwrap();
    assert_eq!(since_ops.len(), 1);
    assert_eq!(since_ops[0].operation_id(), all_ops[1].operation_id());
}

#[test]
fn test_operations_since_excludes_device() {
    // ops_since should never return ops from the excluded device
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
    let root = ws.list_all_notes().unwrap()[0].clone();
    let _id = ws.create_note(Some(&root.id), "Note 1", crate::AddPosition::AsChild).unwrap();
    // The workspace's own device_id is stored in workspace_meta.
    // For this test, we check that passing the current device_id excludes the op.
    let current_device_id = ws.connection().query_row(
        "SELECT value FROM workspace_meta WHERE key='device_id'", [],
        |row| row.get::<_, String>(0)).unwrap();
    let ops = ws.operations_since(None, &current_device_id).unwrap();
    assert!(ops.is_empty(), "ops from own device should be excluded");
}

#[test]
fn test_operations_since_filters_local_retract() {
    use crate::Operation;
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
    let root = ws.list_all_notes().unwrap()[0].clone();
    let _id = ws.create_note(Some(&root.id), "Note 1", crate::AddPosition::AsChild).unwrap();
    // Undo should create a RetractOperation with propagate=false
    ws.undo().unwrap();

    let ops = ws.operations_since(None, "other-device").unwrap();
    // RetractOperation(propagate=false) must be absent
    for op in &ops {
        if let Operation::RetractOperation { propagate, .. } = op {
            assert!(propagate, "local-only retract must be filtered from delta");
        }
    }
}
```

- [ ] **Run to confirm they fail**

```bash
cargo test -p krillnotes-core test_operations_since 2>&1 | tail -20
```

Expected: method `operations_since` not found

- [ ] **Implement** — find a logical place near `get_latest_operation_id` in `workspace.rs` (~line 4719) and add:

```rust
/// Returns all operations in HLC order that occurred strictly after `since_op_id`,
/// excluding operations from `exclude_device_id` (echo prevention).
///
/// Used by `swarm::sync::generate_delta` to build the operation list for a delta bundle.
/// `RetractOperation { propagate: false }` is filtered out (local-only undo markers).
///
/// If `since_op_id` is `None`, all operations except those from `exclude_device_id`
/// are returned (used when peer has no watermark set — should not happen after A11 fix).
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
            stmt.query_map(
                rusqlite::params![wall_ms, counter, node_id, exclude_device_id],
                |row| row.get::<_, String>(0),
            )?.collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?
        } else {
            vec![] // watermark op not found in this workspace — nothing to send
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT operation_data FROM operations WHERE device_id != ?1 \
             ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, \
                      timestamp_node_id ASC",
        )?;
        stmt.query_map([exclude_device_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>().map_err(KrillnotesError::Database)?
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
```

- [ ] **Run to confirm tests pass**

```bash
cargo test -p krillnotes-core test_operations_since 2>&1 | tail -20
```

- [ ] **Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(workspace): add operations_since for delta generation"
```

---

### Task 4: `Workspace::apply_incoming_operation`

**Files:** `krillnotes-core/src/core/workspace.rs`

- [ ] **Write failing tests** — add inside the `#[cfg(test)]` block:

```rust
#[test]
fn test_apply_incoming_create_note() {
    use crate::core::hlc::HlcTimestamp;
    use crate::Operation;
    use std::collections::BTreeMap;

    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

    let op = Operation::CreateNote {
        operation_id: "op-incoming-1".to_string(),
        timestamp: HlcTimestamp { wall_ms: 9_000_000, counter: 0, node_id: 99 },
        device_id: "dev-remote".to_string(),
        note_id: "note-remote-1".to_string(),
        parent_id: None,
        position: 1.0,
        schema: "TextNote".to_string(),
        title: "Remote Title".to_string(),
        fields: BTreeMap::new(),
        created_by: "pk-remote".to_string(),
        signature: "sig".to_string(),
    };

    let applied = ws.apply_incoming_operation(&op).unwrap();
    assert!(applied);

    // Note should now exist
    let note = ws.get_note("note-remote-1").unwrap();
    assert_eq!(note.title, "Remote Title");

    // Operation should be in the log with synced=1
    let synced: i64 = ws.connection().query_row(
        "SELECT synced FROM operations WHERE operation_id='op-incoming-1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(synced, 1);
}

#[test]
fn test_apply_incoming_duplicate_is_idempotent() {
    use crate::core::hlc::HlcTimestamp;
    use crate::Operation;
    use std::collections::BTreeMap;

    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

    let op = Operation::CreateNote {
        operation_id: "op-dup".to_string(),
        timestamp: HlcTimestamp { wall_ms: 9_000_000, counter: 0, node_id: 99 },
        device_id: "dev-remote".to_string(),
        note_id: "note-dup".to_string(),
        parent_id: None, position: 1.0,
        schema: "TextNote".to_string(), title: "Dup".to_string(),
        fields: BTreeMap::new(), created_by: "pk".to_string(), signature: "sig".to_string(),
    };

    assert!(ws.apply_incoming_operation(&op).unwrap());   // first: applied
    assert!(!ws.apply_incoming_operation(&op).unwrap());  // second: skipped
}

#[test]
fn test_apply_incoming_retract_propagate_false_skipped() {
    use crate::core::hlc::HlcTimestamp;
    use crate::{Operation, RetractInverse};

    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

    let op = Operation::RetractOperation {
        operation_id: "op-retract-local".to_string(),
        timestamp: HlcTimestamp { wall_ms: 1, counter: 0, node_id: 0 },
        device_id: "dev-remote".to_string(),
        retracted_ids: vec!["op-x".to_string()],
        inverse: RetractInverse::None,
        propagate: false,
    };

    let applied = ws.apply_incoming_operation(&op).unwrap();
    assert!(!applied, "propagate=false retract must be skipped");
    let count: i64 = ws.connection().query_row(
        "SELECT COUNT(*) FROM operations WHERE operation_id='op-retract-local'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0, "no row should be inserted for local-only retract");
}

#[test]
fn test_apply_incoming_hlc_advances() {
    use crate::core::hlc::HlcTimestamp;
    use crate::Operation;
    use std::collections::BTreeMap;

    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

    let far_future_ms = 9_999_999_999u64;
    let op = Operation::CreateNote {
        operation_id: "op-future".to_string(),
        timestamp: HlcTimestamp { wall_ms: far_future_ms, counter: 0, node_id: 99 },
        device_id: "dev-remote".to_string(),
        note_id: "note-future".to_string(),
        parent_id: None, position: 1.0,
        schema: "TextNote".to_string(), title: "Future".to_string(),
        fields: BTreeMap::new(), created_by: "pk".to_string(), signature: "sig".to_string(),
    };

    ws.apply_incoming_operation(&op).unwrap();

    // Next local operation must have wall_ms >= far_future_ms
    let root = ws.list_all_notes().unwrap()[0].clone();
    let _new_id = ws.create_note(Some(&root.id), "After future", crate::AddPosition::AsChild).unwrap();
    let latest_op: HlcTimestamp = ws.connection().query_row(
        "SELECT timestamp_wall_ms, timestamp_counter, timestamp_node_id \
         FROM operations ORDER BY timestamp_wall_ms DESC, timestamp_counter DESC LIMIT 1",
        [],
        |row| Ok(HlcTimestamp { wall_ms: row.get::<_,i64>(0)? as u64,
                                 counter: row.get::<_,i64>(1)? as u32,
                                 node_id: row.get::<_,i64>(2)? as u16 }),
    ).unwrap();
    assert!(latest_op.wall_ms >= far_future_ms,
        "local op must have wall_ms >= incoming op's wall_ms");
}
```

- [ ] **Run to confirm they fail**

```bash
cargo test -p krillnotes-core test_apply_incoming 2>&1 | tail -20
```

Expected: method not found

- [ ] **Implement** — add after `operations_since` in `workspace.rs`:

```rust
/// Applies a single operation received from a remote peer.
///
/// Returns `true` if the operation was applied, `false` if skipped (already present).
///
/// **A13 stub behaviour:**
/// - RBAC is NOT enforced (allow all) — WP-B adds enforcement.
/// - Conflict resolution is NOT applied (last-write-wins) — WP-C adds it.
/// - Individual operation signatures are NOT verified — WP-C adds verification.
///
/// Constraints upheld:
/// - HLC is observed (advanced) for every incoming operation.
/// - Original operation_id, timestamp, and device_id are preserved (no re-signing).
/// - `RetractOperation { propagate: false }` is skipped entirely.
/// - Operations are recorded with `synced = 1`.
pub fn apply_incoming_operation(&mut self, op: &Operation) -> Result<bool> {
    use crate::Operation;

    // Guard: skip local-only retracts before doing anything.
    if let Operation::RetractOperation { propagate: false, .. } = op {
        return Ok(false);
    }

    // 1. Advance local HLC.
    self.hlc.observe(op.timestamp());

    // 2. Insert into operation log with synced = 1. INSERT OR IGNORE ensures idempotency.
    let op_json = serde_json::to_string(op)?;
    let ts = op.timestamp();
    let inserted = {
        let conn = self.storage.connection_mut();
        let rows = conn.execute(
            "INSERT OR IGNORE INTO operations \
             (operation_id, timestamp_wall_ms, timestamp_counter, timestamp_node_id, \
              device_id, operation_type, operation_data, synced) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            rusqlite::params![
                op.operation_id(),
                ts.wall_ms as i64,
                ts.counter as i64,
                ts.node_id as i64,
                op.device_id(),
                Self::operation_type_str(op),
                op_json,
            ],
        )?;
        rows > 0
    };

    if !inserted {
        return Ok(false); // duplicate — already in log, skip state change
    }

    // 3. Apply state change to working tables.
    // Uses the same SQL patterns as the local mutation methods, but driven by
    // the incoming operation's data. Never calls existing mutation methods
    // (those generate new operation_id, new HLC, new signature).
    match op {
        Operation::CreateNote { note_id, parent_id, position, schema, title, fields,
                                created_by, timestamp, .. } => {
            let fields_json = serde_json::to_string(fields)?;
            let now = chrono::Utc::now().to_rfc3339();
            self.storage.connection_mut().execute(
                "INSERT OR IGNORE INTO notes \
                 (id, title, schema, parent_id, position, created_at, modified_at, \
                  created_by, modified_by, fields_json, is_expanded, schema_version) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, 1, 0)",
                rusqlite::params![
                    note_id, title, schema, parent_id, position, now, now,
                    created_by, fields_json,
                ],
            )?;
        }

        Operation::UpdateNote { note_id, title, modified_by, .. } => {
            let now = chrono::Utc::now().to_rfc3339();
            self.storage.connection_mut().execute(
                "UPDATE notes SET title=?1, modified_by=?2, modified_at=?3 WHERE id=?4",
                rusqlite::params![title, modified_by, now, note_id],
            )?;
        }

        Operation::UpdateField { note_id, field, value, modified_by, .. } => {
            let conn = self.storage.connection_mut();
            // Read-modify-write the fields_json column.
            let fields_json: Option<String> = conn.query_row(
                "SELECT fields_json FROM notes WHERE id=?1", [note_id],
                |row| row.get(0),
            ).optional().map_err(KrillnotesError::Database)?;
            if let Some(json) = fields_json {
                let mut fields: std::collections::BTreeMap<String, crate::FieldValue> =
                    serde_json::from_str(&json)?;
                fields.insert(field.clone(), value.clone());
                let new_json = serde_json::to_string(&fields)?;
                let now = chrono::Utc::now().to_rfc3339();
                conn.execute(
                    "UPDATE notes SET fields_json=?1, modified_by=?2, modified_at=?3 WHERE id=?4",
                    rusqlite::params![new_json, modified_by, now, note_id],
                )?;
            }
        }

        Operation::DeleteNote { note_id, .. } => {
            // A13 stub: delete the note (SQLite CASCADE removes note_tags via FK).
            self.storage.connection_mut().execute(
                "DELETE FROM notes WHERE id=?1", [note_id],
            )?;
        }

        Operation::MoveNote { note_id, new_parent_id, new_position, moved_by, .. } => {
            let now = chrono::Utc::now().to_rfc3339();
            self.storage.connection_mut().execute(
                "UPDATE notes SET parent_id=?1, position=?2, modified_by=?3, modified_at=?4 \
                 WHERE id=?5",
                rusqlite::params![new_parent_id, new_position, moved_by, now, note_id],
            )?;
        }

        Operation::SetTags { note_id, tags, .. } => {
            let conn = self.storage.connection_mut();
            conn.execute("DELETE FROM note_tags WHERE note_id=?1", [note_id])?;
            for tag in tags {
                conn.execute(
                    "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?1, ?2)",
                    rusqlite::params![note_id, tag],
                )?;
            }
        }

        Operation::SetPermission { note_id, user_id, role, granted_by, .. } => {
            self.storage.connection_mut().execute(
                "INSERT OR REPLACE INTO note_permissions (note_id, user_id, role, granted_by) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![note_id, user_id, role, granted_by],
            )?;
        }

        Operation::RevokePermission { note_id, user_id, .. } => {
            self.storage.connection_mut().execute(
                "DELETE FROM note_permissions WHERE note_id IS ?1 AND user_id=?2",
                rusqlite::params![note_id, user_id],
            )?;
        }

        Operation::JoinWorkspace { .. } => {
            // Log only — no working-state change needed for A13 stub.
        }

        Operation::CreateUserScript { script_id, name, description, source_code,
                                       load_order, enabled, .. } => {
            let now = chrono::Utc::now().timestamp_millis();
            self.storage.connection_mut().execute(
                "INSERT OR IGNORE INTO user_scripts \
                 (id, name, description, source_code, load_order, enabled, created_at, modified_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                rusqlite::params![script_id, name, description, source_code,
                                  load_order, *enabled as i64, now],
            )?;
        }

        Operation::UpdateUserScript { script_id, name, description, source_code,
                                       load_order, enabled, .. } => {
            let now = chrono::Utc::now().timestamp_millis();
            self.storage.connection_mut().execute(
                "UPDATE user_scripts SET name=?1, description=?2, source_code=?3, \
                 load_order=?4, enabled=?5, modified_at=?6 WHERE id=?7",
                rusqlite::params![name, description, source_code,
                                  load_order, *enabled as i64, now, script_id],
            )?;
        }

        Operation::DeleteUserScript { script_id, .. } => {
            self.storage.connection_mut().execute(
                "DELETE FROM user_scripts WHERE id=?1", [script_id],
            )?;
        }

        Operation::UpdateSchema { .. } => {
            // A13 stub: log only — full batch migration deferred to WP-C.
            // (State change would require running the Rhai migration script,
            // which requires the schema registry to be consistent.)
        }

        Operation::RetractOperation { propagate: true, .. } => {
            // Log-only for A13 stub. State revert deferred to WP-C.
        }

        Operation::RetractOperation { propagate: false, .. } => {
            // Already handled at the top of this function — unreachable here.
            unreachable!("propagate=false retract should have been caught above");
        }
    }

    Ok(true)
}

/// Returns the operation type name string used in the `operations` table.
/// (Mirrors `OperationLog::operation_type_name` — keep in sync.)
fn operation_type_str(op: &Operation) -> &'static str {
    use crate::Operation;
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
```

> **Note:** The `note_permissions` table may not exist yet (it's part of RBAC, WP-B). If the `SetPermission`/`RevokePermission` SQL fails because the table doesn't exist, wrap those arms with a check: `self.storage.connection().query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_permissions'", [], ...).unwrap_or(0) > 0`.

> **Note:** Check if `operation_type_str` duplicates `OperationLog::operation_type_name` — if that method is accessible, call it instead of adding a new one.

- [ ] **Run to confirm tests pass**

```bash
cargo test -p krillnotes-core test_apply_incoming 2>&1 | tail -20
```

- [ ] **Run full test suite to check for regressions**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: all tests pass

- [ ] **Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(workspace): add apply_incoming_operation for delta ingest"
```

---

## Chunk 2: `swarm/sync.rs` Orchestration

### Task 5: Create `swarm/sync.rs` with `generate_delta`

**Files:** `krillnotes-core/src/core/swarm/sync.rs` (new), `krillnotes-core/src/core/swarm/mod.rs`, `krillnotes-core/src/lib.rs`

- [ ] **Wire up the module** — add to `mod.rs`:

```rust
pub mod sync;
```

- [ ] **Write failing test** — add to `sync.rs` (creating the file):

```rust
// krillnotes-core/src/core/swarm/sync.rs
// ... (module code to be added in the implement step)

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_key() -> SigningKey { SigningKey::generate(&mut OsRng) }

    fn setup_alice_with_bob_peer() -> (tempfile::NamedTempFile, crate::core::workspace::Workspace,
                                        SigningKey, crate::core::contact::ContactManager,
                                        SigningKey) {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(bob_key.verifying_key().as_bytes());

        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = crate::core::workspace::Workspace::create(
            temp.path(), "", "alice-id",
            ed25519_dalek::SigningKey::from_bytes(&alice_key.to_bytes()),
        ).unwrap();

        // Simulate that we already sent Bob a snapshot with last_sent_op = "snap-op"
        // First create a real operation to use as watermark
        let root = ws.list_all_notes().unwrap()[0].clone();
        let _ = ws.create_note(Some(&root.id), "Pre-snapshot note",
            crate::AddPosition::AsChild).unwrap();
        let snap_op = ws.get_latest_operation_id().unwrap().unwrap();
        ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64,
            Some(&snap_op), Some(&snap_op)).unwrap();

        // Bob must be in contacts for key lookup
        let cm_dir = tempfile::tempdir().unwrap();
        let cm_key = [2u8; 32];
        let cm = crate::core::contact::ContactManager::new(cm_dir.path(), cm_key).unwrap();
        cm.find_or_create_by_public_key(
            "Bob", &bob_pubkey_b64,
            crate::core::contact::TrustLevel::Tofu,
        ).unwrap();

        (temp, ws, alice_key, cm, bob_key)
    }

    #[test]
    fn test_generate_delta_basic() {
        let (_temp, mut ws, alice_key, cm, bob_key) = setup_alice_with_bob_peer();
        let bob_pubkey = base64::engine::general_purpose::STANDARD
            .encode(bob_key.verifying_key().as_bytes());

        // Add a new note AFTER the snapshot watermark
        let root = ws.list_all_notes().unwrap()[0].clone();
        let _new_id = ws.create_note(Some(&root.id), "Post-snapshot note",
            crate::AddPosition::AsChild).unwrap();

        let bundle = generate_delta(&mut ws, "dev-bob", "TestWorkspace",
            &alice_key, &cm).unwrap();

        // Parse and verify
        let parsed = crate::core::swarm::delta::parse_delta_bundle(&bundle, &bob_key).unwrap();
        assert_eq!(parsed.operations.len(), 1);
        assert_eq!(parsed.workspace_id, ws.workspace_id());
    }

    #[test]
    fn test_generate_delta_empty_no_watermark_update() {
        let (_temp, mut ws, alice_key, cm, _bob_key) = setup_alice_with_bob_peer();
        let snap_op_before = ws.connection().query_row(
            "SELECT last_sent_op FROM sync_peers WHERE peer_device_id='dev-bob'",
            [], |row| row.get::<_, Option<String>>(0),
        ).unwrap().unwrap();

        // No new ops since snapshot
        let bundle = generate_delta(&mut ws, "dev-bob", "TestWorkspace",
            &alice_key, &cm).unwrap();
        let parsed = crate::core::swarm::delta::parse_delta_bundle(&bundle, &_bob_key).unwrap();
        assert_eq!(parsed.operations.len(), 0);

        let snap_op_after = ws.connection().query_row(
            "SELECT last_sent_op FROM sync_peers WHERE peer_device_id='dev-bob'",
            [], |row| row.get::<_, Option<String>>(0),
        ).unwrap().unwrap();
        assert_eq!(snap_op_before, snap_op_after, "empty delta must not update watermark");
    }

    #[test]
    fn test_generate_delta_no_snapshot_errors() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(bob_key.verifying_key().as_bytes());

        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = crate::core::workspace::Workspace::create(
            temp.path(), "", "alice-id",
            ed25519_dalek::SigningKey::from_bytes(&alice_key.to_bytes()),
        ).unwrap();
        ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64, None, None).unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let cm = crate::core::contact::ContactManager::new(cm_dir.path(), [2u8; 32]).unwrap();

        let result = generate_delta(&mut ws, "dev-bob", "TestWorkspace", &alice_key, &cm);
        assert!(result.is_err(), "must error when last_sent_op is None");
    }
}
```

- [ ] **Run to confirm it fails**

```bash
cargo test -p krillnotes-core swarm::sync 2>&1 | tail -10
```

Expected: `generate_delta` not found

- [ ] **Implement `generate_delta`** — write the full file content:

```rust
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! High-level delta sync orchestration.
//!
//! `generate_delta` and `apply_delta` sit above the codec (`swarm/delta.rs`)
//! and workspace primitives (`workspace.rs`), orchestrating:
//!   - peer watermark lookup
//!   - operation list assembly
//!   - encryption key resolution from the contact manager
//!   - codec invocation
//!   - watermark and peer registry updates

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::Serialize;

use crate::core::contact::{ContactManager, TrustLevel};
use crate::core::swarm::delta::{create_delta_bundle, parse_delta_bundle, DeltaParams};
use crate::core::workspace::Workspace;
use crate::{KrillnotesError, Operation, Result};

/// Result of applying a received delta bundle.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub operations_applied: usize,
    pub operations_skipped: usize,
    pub sender_device_id: String,
    pub sender_public_key: String,
    /// Display names of contacts auto-registered via TOFU during this apply.
    pub new_tofu_contacts: Vec<String>,
}

/// Generate a delta `.swarm` bundle for a specific peer.
///
/// Queries all operations since `last_sent_op` for `peer_device_id`, encrypts them
/// for the peer's public key, and updates the `last_sent_op` watermark.
///
/// # Errors
/// - `KrillnotesError::NotFound` if the peer is not registered.
/// - `KrillnotesError::Swarm("snapshot must precede delta")` if `last_sent_op` is `None`.
pub fn generate_delta(
    workspace: &mut Workspace,
    peer_device_id: &str,
    workspace_name: &str,
    signing_key: &SigningKey,
    contact_manager: &ContactManager,
) -> Result<Vec<u8>> {
    // 1. Look up peer.
    let peer = workspace.get_sync_peer(peer_device_id)?
        .ok_or_else(|| KrillnotesError::NotFound(
            format!("peer {peer_device_id} not found in registry")))?;

    // 2. Require snapshot baseline.
    let last_sent_op = peer.last_sent_op.as_deref()
        .ok_or_else(|| KrillnotesError::Swarm(
            "snapshot must precede delta — no last_sent_op for this peer".to_string()))?;

    // 3. Collect operations since watermark, excluding this peer's own ops.
    let ops = workspace.operations_since(Some(last_sent_op), &peer.peer_device_id)?;

    // 4. Resolve peer's public key from contacts.
    let contact = contact_manager.find_by_public_key(&peer.peer_identity_id)?
        .ok_or_else(|| KrillnotesError::NotFound(
            format!("no contact for peer identity {}", peer.peer_identity_id)))?;
    let recipient_key_bytes = BASE64.decode(&contact.public_key)
        .map_err(|e| KrillnotesError::Swarm(format!("bad contact public key: {e}")))?;
    let recipient_key_arr: [u8; 32] = recipient_key_bytes.try_into()
        .map_err(|_| KrillnotesError::Swarm("contact public key wrong length".to_string()))?;
    let recipient_vk = VerifyingKey::from_bytes(&recipient_key_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid recipient key: {e}")))?;

    // 5. Build delta bundle (empty ops list is valid — acts as a heartbeat).
    let source_device_id = crate::core::device::get_device_id()
        .map_err(|e| KrillnotesError::Swarm(format!("cannot get device id: {e}")))?;

    let bundle = create_delta_bundle(DeltaParams {
        workspace_id: workspace.workspace_id().to_string(),
        workspace_name: workspace_name.to_string(),
        source_device_id,
        since_operation_id: last_sent_op.to_string(), // safe: checked in step 2
        operations: ops.clone(),
        sender_key: signing_key,
        recipient_keys: vec![&recipient_vk],
        recipient_peer_ids: vec![peer_device_id.to_string()],
    })?;

    // 6. Update watermark only if we sent at least one operation.
    if let Some(last_op) = ops.last() {
        workspace.upsert_sync_peer(
            peer_device_id,
            &peer.peer_identity_id,
            Some(last_op.operation_id()),
            None,
        )?;
    }

    Ok(bundle)
}
```

- [ ] **Run to confirm tests pass**

```bash
cargo test -p krillnotes-core swarm::sync::tests::test_generate_delta 2>&1 | tail -20
```

- [ ] **Commit**

```bash
git add krillnotes-core/src/core/swarm/sync.rs krillnotes-core/src/core/swarm/mod.rs
git commit -m "feat(swarm/sync): implement generate_delta"
```

---

> **Check:** Before Task 5, confirm `Workspace::get_sync_peer` exists. If it doesn't, add it to `workspace.rs` following the `upsert_sync_peer` pattern:
> ```rust
> pub fn get_sync_peer(&self, peer_device_id: &str) -> Result<Option<crate::core::peer_registry::SyncPeer>> {
>     crate::core::peer_registry::PeerRegistry::new(self.storage.connection())
>         .get_peer(peer_device_id)
> }
> ```

---

### Task 6: `apply_delta` in `swarm/sync.rs`

**Files:** `krillnotes-core/src/core/swarm/sync.rs`

- [ ] **Write failing tests** — add to the `tests` module in `sync.rs`:

```rust
    #[test]
    fn test_apply_delta_basic() {
        let alice_key = make_key();
        let bob_key = make_key();
        let alice_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(alice_key.verifying_key().as_bytes());

        // Alice's workspace with a note
        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(), "", "alice-id",
            ed25519_dalek::SigningKey::from_bytes(&alice_key.to_bytes()),
        ).unwrap();

        // Bob's workspace (peer of Alice, watermarks set)
        let bob_temp = tempfile::NamedTempFile::new().unwrap();
        let mut bob_ws = crate::core::workspace::Workspace::create(
            bob_temp.path(), "", "bob-id",
            ed25519_dalek::SigningKey::from_bytes(&bob_key.to_bytes()),
        ).unwrap();

        // Alice creates a note
        let root = alice_ws.list_all_notes().unwrap()[0].clone();
        let _ = alice_ws.create_note(Some(&root.id), "Alice's note",
            crate::AddPosition::AsChild).unwrap();
        let snap_op = alice_ws.get_latest_operation_id().unwrap().unwrap();

        // Set up sync state: Bob has Alice as a peer, watermarks at snap_op
        bob_ws.upsert_sync_peer("dev-alice", &alice_pubkey_b64,
            Some(&snap_op), Some(&snap_op)).unwrap();

        // Alice creates another note (this is the delta content)
        let _ = alice_ws.create_note(Some(&root.id), "Delta note",
            crate::AddPosition::AsChild).unwrap();

        // Alice sets up her contact manager with Bob
        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::new(
            alice_cm_dir.path(), [10u8; 32]).unwrap();
        let bob_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(bob_key.verifying_key().as_bytes());
        alice_ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64,
            Some(&snap_op), None).unwrap();
        alice_cm.find_or_create_by_public_key(
            "Bob", &bob_pubkey_b64, crate::core::contact::TrustLevel::Tofu).unwrap();

        // Generate delta from Alice to Bob
        let bundle = generate_delta(&mut alice_ws, "dev-bob", "Test",
            &alice_key, &alice_cm).unwrap();

        // Bob applies it
        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::new(
            bob_cm_dir.path(), [11u8; 32]).unwrap();
        let result = apply_delta(&bundle, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();

        assert_eq!(result.operations_applied, 1);
        assert_eq!(result.operations_skipped, 0);

        // "Delta note" should now exist in Bob's workspace
        let notes = bob_ws.list_all_notes().unwrap();
        assert!(notes.iter().any(|n| n.title == "Delta note"),
            "Bob should have Alice's new note");
    }

    #[test]
    fn test_apply_delta_idempotent() {
        // apply same bundle twice — second apply should be all-skipped, no error
        let alice_key = make_key();
        let bob_key = make_key();
        let alice_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(alice_key.verifying_key().as_bytes());
        let bob_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(bob_key.verifying_key().as_bytes());

        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(), "", "alice-id",
            ed25519_dalek::SigningKey::from_bytes(&alice_key.to_bytes()),
        ).unwrap();
        let snap_op = alice_ws.get_latest_operation_id().unwrap().unwrap_or_default();
        alice_ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None).unwrap();

        let root = alice_ws.list_all_notes().unwrap()[0].clone();
        let _ = alice_ws.create_note(Some(&root.id), "Note for dup test",
            crate::AddPosition::AsChild).unwrap();

        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::new(
            alice_cm_dir.path(), [12u8; 32]).unwrap();
        alice_cm.find_or_create_by_public_key(
            "Bob", &bob_pubkey_b64, crate::core::contact::TrustLevel::Tofu).unwrap();

        let bundle = generate_delta(&mut alice_ws, "dev-bob", "Test",
            &alice_key, &alice_cm).unwrap();

        let bob_temp = tempfile::NamedTempFile::new().unwrap();
        let mut bob_ws = crate::core::workspace::Workspace::create(
            bob_temp.path(), "", "bob-id",
            ed25519_dalek::SigningKey::from_bytes(&bob_key.to_bytes()),
        ).unwrap();
        bob_ws.upsert_sync_peer("dev-alice", &alice_pubkey_b64, Some(&snap_op), None).unwrap();

        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::new(
            bob_cm_dir.path(), [13u8; 32]).unwrap();

        let r1 = apply_delta(&bundle, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();
        let r2 = apply_delta(&bundle, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();
        assert_eq!(r1.operations_applied, 1);
        assert_eq!(r2.operations_applied, 0);
        assert_eq!(r2.operations_skipped, 1);
    }

    #[test]
    fn test_apply_delta_workspace_id_mismatch() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(bob_key.verifying_key().as_bytes());

        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(), "", "alice-id",
            ed25519_dalek::SigningKey::from_bytes(&alice_key.to_bytes()),
        ).unwrap();
        let snap_op = alice_ws.get_latest_operation_id().unwrap().unwrap_or_default();
        alice_ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None).unwrap();
        let root = alice_ws.list_all_notes().unwrap()[0].clone();
        let _ = alice_ws.create_note(Some(&root.id), "Some note",
            crate::AddPosition::AsChild).unwrap();

        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::new(
            alice_cm_dir.path(), [14u8; 32]).unwrap();
        alice_cm.find_or_create_by_public_key(
            "Bob", &bob_pubkey_b64, crate::core::contact::TrustLevel::Tofu).unwrap();
        let bundle = generate_delta(&mut alice_ws, "dev-bob", "Test",
            &alice_key, &alice_cm).unwrap();

        // Bob's workspace has a completely different workspace_id
        let bob_temp = tempfile::NamedTempFile::new().unwrap();
        let mut bob_ws = crate::core::workspace::Workspace::create(
            bob_temp.path(), "", "bob-id",
            ed25519_dalek::SigningKey::from_bytes(&bob_key.to_bytes()),
        ).unwrap();
        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::new(
            bob_cm_dir.path(), [15u8; 32]).unwrap();

        let result = apply_delta(&bundle, &mut bob_ws, &bob_key, &mut bob_cm);
        assert!(result.is_err(), "workspace_id mismatch must be an error");
    }
```

- [ ] **Run to confirm they fail**

```bash
cargo test -p krillnotes-core swarm::sync::tests::test_apply_delta 2>&1 | tail -10
```

- [ ] **Implement `apply_delta`** — add to `sync.rs` after `generate_delta`:

```rust
/// Apply a received delta `.swarm` bundle to the local workspace.
///
/// Decrypts, verifies bundle signature, applies each operation in order.
/// Auto-registers unknown operation authors as TOFU contacts.
///
/// Returns an `ApplyResult` summarising what was applied / skipped.
///
/// **A13 stub:** RBAC and conflict resolution are not enforced.
/// Individual per-operation signatures are not verified.
pub fn apply_delta(
    bundle_bytes: &[u8],
    workspace: &mut Workspace,
    recipient_key: &SigningKey,
    contact_manager: &mut ContactManager,
) -> Result<ApplyResult> {
    // 1. Decrypt and verify bundle-level signature.
    let parsed = parse_delta_bundle(bundle_bytes, recipient_key)?;

    // 2. Assert workspace_id matches.
    if parsed.workspace_id != workspace.workspace_id() {
        return Err(KrillnotesError::Swarm(format!(
            "workspace_id mismatch: bundle has '{}', this workspace is '{}'",
            parsed.workspace_id,
            workspace.workspace_id()
        )));
    }

    let mut applied = 0usize;
    let mut skipped = 0usize;
    let mut new_tofu_contacts: Vec<String> = Vec::new();
    let mut last_applied_op_id = String::new();

    // 3. Apply each operation in chronological order.
    for op in &parsed.operations {
        // TOFU: auto-register unknown authors.
        let author_key = op.author_key();
        if !author_key.is_empty()
            && contact_manager.find_by_public_key(author_key)?.is_none()
        {
            let name = if let Operation::JoinWorkspace { declared_name, .. } = op {
                declared_name.clone()
            } else {
                // Synthetic fallback: first 8 chars of base64 key + ellipsis
                format!("{}…", &author_key[..8.min(author_key.len())])
            };
            contact_manager.find_or_create_by_public_key(&name, author_key, TrustLevel::Tofu)?;
            new_tofu_contacts.push(name);
        }

        if workspace.apply_incoming_operation(op)? {
            applied += 1;
            last_applied_op_id = op.operation_id().to_string();
        } else {
            skipped += 1;
        }
    }

    // 4. Upsert sender in peer registry.
    let last_received = if last_applied_op_id.is_empty() {
        None
    } else {
        Some(last_applied_op_id.as_str())
    };
    workspace.upsert_sync_peer(
        &parsed.sender_device_id,
        &parsed.sender_public_key,
        None,           // don't touch last_sent_op
        last_received,
    )?;

    Ok(ApplyResult {
        operations_applied: applied,
        operations_skipped: skipped,
        sender_device_id: parsed.sender_device_id,
        sender_public_key: parsed.sender_public_key,
        new_tofu_contacts,
    })
}
```

- [ ] **Re-export `ApplyResult` from the crate root** — in `krillnotes-core/src/lib.rs`, find the existing `pub use` block and add:

```rust
pub use core::swarm::sync::ApplyResult;
```

- [ ] **Run tests**

```bash
cargo test -p krillnotes-core swarm::sync 2>&1 | tail -20
cargo test -p krillnotes-core 2>&1 | tail -20  # full suite
```

- [ ] **Commit**

```bash
git add krillnotes-core/src/core/swarm/sync.rs krillnotes-core/src/lib.rs
git commit -m "feat(swarm/sync): implement apply_delta"
```

---

## Chunk 3: Tauri Layer + A11 Fix

### Task 7: Fix A11 snapshot import watermark

**Files:** `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Locate the bug** — find line ~3726 (the `upsert_sync_peer` call inside the snapshot import handler). It currently passes `None` for `last_sent_op`:

```rust
let _ = ws.upsert_sync_peer(
    &placeholder_device_id,
    &parsed.sender_public_key,
    None,                              // ← BUG: blocks bidirectional sync
    Some(&parsed.as_of_operation_id),
);
```

- [ ] **Fix it** — set both watermarks to `as_of_operation_id`:

```rust
let _ = ws.upsert_sync_peer(
    &placeholder_device_id,
    &parsed.sender_public_key,
    Some(&parsed.as_of_operation_id),  // last_sent_op — snapshot is the baseline
    Some(&parsed.as_of_operation_id),  // last_received_op
);
```

- [ ] **Verify with compile check**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "error|warning.*unused" | head -20
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "fix(lib): set both watermarks on snapshot import (A11 bidirectional fix)"
```

---

### Task 8: Tauri command `get_workspace_peers`

**Files:** `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Check if command already exists** — grep for `get_workspace_peers`:

```bash
grep -n "get_workspace_peers" krillnotes-desktop/src-tauri/src/lib.rs
```

If it exists, skip this task. If not, continue.

- [ ] **Implement** — find the Tauri commands section and add before the `generate_handler!` macro:

```rust
/// Returns resolved peer info (display name, fingerprint, trust level) for the current workspace.
/// Used to populate the CreateDeltaDialog peer checklist.
#[tauri::command]
async fn get_workspace_peers(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<krillnotes_core::core::peer_registry::PeerInfo>, String> {
    let identity_uuid = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        ws.identity_uuid().to_string()
    };
    let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cm_guard.get(&identity_uuid).ok_or("Contact manager not available")?;
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
    ws.list_peers_info(cm).map_err(|e| e.to_string())
}
```

- [ ] **Register** — add `get_workspace_peers` to the `generate_handler![...]` list.

- [ ] **Build check**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep "^error" | head -10
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(tauri): add get_workspace_peers command"
```

---

### Task 9: Tauri command `generate_deltas_for_peers`

**Files:** `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Add the result type** — find where other result structs are defined and add:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateDeltasResult {
    succeeded: Vec<String>,          // peer_device_ids that worked
    failed: Vec<(String, String)>,   // (peer_device_id, error_message)
    files_written: Vec<String>,      // absolute paths of written .swarm files
}
```

- [ ] **Implement the command** — add before `generate_handler!`:

```rust
/// Batch-generates one delta .swarm per selected peer into `dir_path`.
///
/// Continues on per-peer errors so a single failure doesn't block the others.
#[tauri::command]
async fn generate_deltas_for_peers(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    dir_path: String,
    peer_device_ids: Vec<String>,
) -> Result<GenerateDeltasResult, String> {
    use krillnotes_core::core::swarm::sync::generate_delta;

    let (signing_key, workspace_name, identity_uuid) = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let paths   = state.workspace_paths.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;

        let identity_uuid = ws.identity_uuid().to_string();
        let workspace_name = paths
            .get(window.label())
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid).ok_or("Identity not unlocked")?;
        let key = ed25519_dalek::SigningKey::from_bytes(&id.signing_key.to_bytes());
        (key, workspace_name, identity_uuid)
    };

    let dir = std::path::Path::new(&dir_path);
    if !dir.exists() {
        return Err(format!("Directory does not exist: {dir_path}"));
    }

    let mut result = GenerateDeltasResult {
        succeeded: Vec::new(),
        failed: Vec::new(),
        files_written: Vec::new(),
    };

    for peer_id in &peer_device_ids {
        // Resolve display name for file naming
        let display_name = {
            let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
            let workspaces = state.workspaces.lock().expect("Mutex poisoned");
            let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
            if let Some(cm) = cm_guard.get(&identity_uuid) {
                ws.list_peers_info(cm)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|p| &p.peer_device_id == peer_id)
                    .map(|p| p.display_name)
                    .unwrap_or_else(|| peer_id[..8.min(peer_id.len())].to_string())
            } else {
                peer_id[..8.min(peer_id.len())].to_string()
            }
        };

        // Sanitise display name for use in file path
        let safe_name: String = display_name.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let base_name = format!("delta-{safe_name}-{date}.swarm");

        // Avoid overwriting existing files
        let file_path = {
            let mut p = dir.join(&base_name);
            let mut n = 2u32;
            while p.exists() {
                let stem = format!("delta-{safe_name}-{date}-{n}.swarm");
                p = dir.join(stem);
                n += 1;
            }
            p
        };

        // Generate the delta
        let bundle_result = {
            let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
            let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
            let ws = workspaces.get_mut(window.label()).ok_or("Workspace not open")?;
            if let Some(cm) = cm_guard.get(&identity_uuid) {
                generate_delta(ws, peer_id, &workspace_name, &signing_key, cm)
                    .map_err(|e| e.to_string())
            } else {
                Err("Contact manager not available".to_string())
            }
        };

        match bundle_result {
            Ok(bytes) => {
                match std::fs::write(&file_path, &bytes) {
                    Ok(()) => {
                        result.succeeded.push(peer_id.clone());
                        result.files_written.push(
                            file_path.to_string_lossy().to_string());
                    }
                    Err(e) => result.failed.push((peer_id.clone(), e.to_string())),
                }
            }
            Err(e) => result.failed.push((peer_id.clone(), e)),
        }
    }

    Ok(result)
}
```

- [ ] **Register** — add `generate_deltas_for_peers` to `generate_handler![...]`.

- [ ] **Build check**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep "^error" | head -10
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(tauri): add generate_deltas_for_peers command"
```

---

### Task 10: Extend `handle_swarm_open` for delta ingest

**Files:** `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Find the delta handling stub** — grep for `SwarmMode::Delta` or `Delta =>` in lib.rs:

```bash
grep -n "Delta\|delta" krillnotes-desktop/src-tauri/src/lib.rs | head -20
```

- [ ] **Replace the stub** with real `apply_delta` call. Look for a match arm that handles `SwarmMode::Delta` and find where it returns a "not implemented" error or does nothing. Replace with:

```rust
SwarmMode::Delta => {
    use krillnotes_core::core::swarm::sync::apply_delta;

    let identity_uuid = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("No workspace open")?;
        ws.identity_uuid().to_string()
    };

    let recipient_key = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid).ok_or("Identity not unlocked")?;
        ed25519_dalek::SigningKey::from_bytes(&id.signing_key.to_bytes())
    };

    let apply_result = {
        let mut cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
        let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get_mut(window.label()).ok_or("Workspace not open")?;
        let cm = cm_guard.get_mut(&identity_uuid).ok_or("Contact manager not available")?;
        apply_delta(&bundle_bytes, ws, &recipient_key, cm).map_err(|e| e.to_string())?
    };

    // Emit workspace-updated so the frontend refreshes the tree view.
    let _ = window.emit("workspace-updated", ());

    Ok(serde_json::json!({
        "mode": "delta",
        "operationsApplied": apply_result.operations_applied,
        "operationsSkipped": apply_result.operations_skipped,
        "newTofu": apply_result.new_tofu_contacts,
    }).to_string())
}
```

> If `bundle_bytes` is not in scope at this point, check how other mode arms access the bundle data and follow the same pattern.

- [ ] **Build check**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep "^error" | head -10
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(tauri): implement delta ingest in handle_swarm_open"
```

---

## Chunk 4: Menu + UI

### Task 11: Menu item + locale strings

**Files:** `krillnotes-desktop/src-tauri/src/menu.rs`, all 7 locale JSON files in `krillnotes-desktop/src/i18n/locales/`

- [ ] **Add locale key to all 7 JSON files** — for each locale file, add `"createDeltaSwarm"` with an appropriate translation. Locales to update:

```bash
ls krillnotes-desktop/src/i18n/locales/
```

English (`en.json`):
```json
"createDeltaSwarm": "Create delta Swarm"
```

For other languages, use the same English string as a placeholder (mark with `// TODO: translate` in a comment if the format allows; otherwise leave as-is for now).

- [ ] **Add menu string** — in `menu.rs`, find where the menu strings struct is defined (look for `create_delta_swarm` field or similar). Add the new field following the existing pattern. If the struct uses a derive macro loading from locale, the key `createDeltaSwarm` should be picked up automatically.

- [ ] **Add menu item** — in `menu.rs` find the Edit menu builder and add the new item after the last workspace action item:

```rust
MenuItem::with_id(app, "create_delta_swarm", menu_strings.create_delta_swarm, true, None::<&str>)?
```

Follow the exact same pattern as the other Edit menu items.

- [ ] **Build check**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep "^error" | head -10
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs \
        krillnotes-desktop/src/i18n/locales/
git commit -m "feat(menu): add Create delta Swarm menu item and locale strings"
```

---

### Task 12: `CreateDeltaDialog.tsx`

**Files:** `krillnotes-desktop/src/components/CreateDeltaDialog.tsx` (new)

- [ ] **Verify TypeScript types** — check `krillnotes-desktop/src/types.ts` for `PeerInfo` and add `GenerateDeltasResult` if missing:

```typescript
// In types.ts — add if not already present:

export interface PeerInfo {
  peerDeviceId: string;
  peerIdentityId: string;
  displayName: string;
  fingerprint: string;
  trustLevel: string | null;
  contactId: string | null;
  lastSync: string | null;
}

export interface GenerateDeltasResult {
  succeeded: string[];
  failed: [string, string][];
  filesWritten: string[];
}
```

- [ ] **Create the component**:

```tsx
// krillnotes-desktop/src/components/CreateDeltaDialog.tsx
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import type { PeerInfo, GenerateDeltasResult } from "../types";

interface Props {
  onClose: () => void;
}

export function CreateDeltaDialog({ onClose }: Props) {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [dirPath, setDirPath] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<GenerateDeltasResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<PeerInfo[]>("get_workspace_peers")
      .then(setPeers)
      .catch((e) => setError(String(e)));
  }, []);

  const handleBrowse = async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string") setDirPath(selected);
  };

  const togglePeer = (id: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // A peer can only be selected if it has a last_sent_op (i.e., snapshot was sent)
  const canSync = (p: PeerInfo) => p.lastSync !== null || p.peerDeviceId.startsWith("identity:");

  const handleGenerate = async () => {
    if (!dirPath || checked.size === 0) return;
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<GenerateDeltasResult>("generate_deltas_for_peers", {
        dirPath,
        peerDeviceIds: Array.from(checked),
      });
      setResult(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const canGenerate = dirPath.length > 0 && checked.size > 0 && !loading;
  const allDone = result !== null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-lg shadow-xl w-[480px] max-h-[80vh] flex flex-col p-6 gap-4">
        <h2 className="text-lg font-semibold">Create delta Swarm</h2>

        {/* Directory picker */}
        <div className="flex flex-col gap-1">
          <label className="text-sm text-zinc-500">Save to directory</label>
          <div className="flex gap-2">
            <input
              type="text"
              readOnly
              value={dirPath}
              placeholder="Choose a directory…"
              className="flex-1 border rounded px-2 py-1 text-sm bg-zinc-50 dark:bg-zinc-800"
            />
            <button
              onClick={handleBrowse}
              className="px-3 py-1 text-sm border rounded hover:bg-zinc-100 dark:hover:bg-zinc-700"
            >
              Browse…
            </button>
          </div>
        </div>

        {/* Peer list */}
        <div className="flex flex-col gap-1 overflow-y-auto max-h-48">
          <label className="text-sm text-zinc-500">Generate delta for</label>
          {peers.length === 0 && !error && (
            <p className="text-sm text-zinc-400 italic">Loading peers…</p>
          )}
          {peers.map((p) => {
            const syncable = canSync(p);
            return (
              <label
                key={p.peerDeviceId}
                className={`flex items-center gap-2 px-2 py-1 rounded cursor-pointer
                  ${syncable ? "hover:bg-zinc-50 dark:hover:bg-zinc-800" : "opacity-40 cursor-not-allowed"}`}
              >
                <input
                  type="checkbox"
                  disabled={!syncable || allDone}
                  checked={checked.has(p.peerDeviceId)}
                  onChange={() => togglePeer(p.peerDeviceId)}
                />
                <span className="flex-1 text-sm font-medium">{p.displayName}</span>
                <span className="text-xs text-zinc-400">{p.fingerprint}</span>
                {!syncable && (
                  <span className="text-xs text-orange-400 ml-1">— never synced</span>
                )}
                {/* Per-peer result feedback */}
                {result?.succeeded.includes(p.peerDeviceId) && (
                  <span className="text-xs text-green-500">✓</span>
                )}
                {result?.failed.find(([id]) => id === p.peerDeviceId) && (
                  <span className="text-xs text-red-500">
                    ✗ {result.failed.find(([id]) => id === p.peerDeviceId)![1]}
                  </span>
                )}
              </label>
            );
          })}
        </div>

        {error && <p className="text-xs text-red-500">{error}</p>}

        {allDone && result.failed.length === 0 && (
          <p className="text-sm text-green-600">
            ✓ {result.filesWritten.length} file(s) written to {dirPath}
          </p>
        )}

        {/* Buttons */}
        <div className="flex justify-end gap-2 pt-2">
          <button
            onClick={onClose}
            className="px-4 py-1.5 text-sm border rounded hover:bg-zinc-100 dark:hover:bg-zinc-700"
          >
            {allDone ? "Close" : "Cancel"}
          </button>
          {!allDone && (
            <button
              onClick={handleGenerate}
              disabled={!canGenerate}
              className="px-4 py-1.5 text-sm bg-blue-600 text-white rounded
                         hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? "Generating…" : "Generate"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src/components/CreateDeltaDialog.tsx \
        krillnotes-desktop/src/types.ts
git commit -m "feat(ui): add CreateDeltaDialog component"
```

---

### Task 13: Wire dialog to `App.tsx`

**Files:** `krillnotes-desktop/src/App.tsx`

- [ ] **Import the dialog** — at the top of `App.tsx`:

```tsx
import { CreateDeltaDialog } from "./components/CreateDeltaDialog";
```

- [ ] **Add state** — find the other dialog state declarations (like `showAddNote`, etc.) and add:

```tsx
const [showCreateDeltaDialog, setShowCreateDeltaDialog] = useState(false);
```

- [ ] **Handle menu event** — find the existing menu event handler (look for `menu-action` or Tauri menu event listener in App.tsx) and add a case for `"create_delta_swarm"`:

```tsx
case "create_delta_swarm":
  setShowCreateDeltaDialog(true);
  break;
```

- [ ] **Render the dialog** — find where other dialogs are rendered (likely at the bottom of the JSX return statement) and add:

```tsx
{showCreateDeltaDialog && (
  <CreateDeltaDialog onClose={() => setShowCreateDeltaDialog(false)} />
)}
```

- [ ] **TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

- [ ] **Full build check**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep "^error" | head -10
```

- [ ] **Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat(ui): wire Create delta Swarm menu to dialog"
```

---

## Final Verification

- [ ] **Run all core tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: all pass

- [ ] **TypeScript type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors

- [ ] **Create a branch PR**

```bash
git push github-https feat/delta-generation
# Then open PR via GitHub or gh CLI targeting master
```
