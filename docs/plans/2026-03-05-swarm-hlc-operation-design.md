# Swarm Milestone 2 — HLC + Operation Enum + Signatures: Design

**Date:** 2026-03-05
**Status:** Approved — ready for implementation
**Branch:** `feat/swarm-hlc-operation-log` off `swarm`
**Scope:** Core protocol Phase 1 (minus gated function API, which is M3)

---

## Goals

Replace the current wall-clock `timestamp: i64` operation model with Hybrid Logical Clocks (HLC), update the Operation enum to match the swarm design spec, add Ed25519 operation signatures (taking advantage of the already-shipped identity system), and change tree positions from `i32` to `f64` for LWW conflict-free moves.

These changes make the operation log wire-compatible with the future `.swarm` sync format.

---

## 1. HlcTimestamp

A new type in `krillnotes-core/src/core/hlc.rs`.

```rust
/// Hybrid Logical Clock timestamp — 16 bytes / 128 bits.
/// Provides causal ordering with wall-clock readability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HlcTimestamp {
    /// Wall-clock ms since Unix epoch. Monotonically non-decreasing.
    pub wall_ms: u64,
    /// Disambiguates events within the same ms on the same node.
    /// Resets to 0 when wall_ms advances.
    pub counter: u32,
    /// Stable u32 derived from device_id. Deterministic tiebreak.
    pub node_id: u32,
}
```

Ordering: lexicographic on `(wall_ms, counter, node_id)`.

**Wire format** — compact JSON array:
```json
{ "timestamp": [1709550720000, 0, 2918374621] }
```

Implemented via a custom `Serialize`/`Deserialize` that writes/reads a 3-element array.

### 1.1 node_id derivation

Uses BLAKE3 (first 4 bytes of hash of device UUID bytes). New dependency: `blake3 = "1"` in `krillnotes-core/Cargo.toml`.

```rust
pub fn node_id_from_device(device_id: &uuid::Uuid) -> u32 {
    let hash = blake3::hash(device_id.as_bytes());
    u32::from_le_bytes(hash.as_bytes()[..4].try_into().unwrap())
}
```

### 1.2 HlcClock — state management

Stored as a field on `Workspace`: `hlc: HlcClock`.

```rust
pub struct HlcClock {
    wall_ms: u64,
    counter: u32,
    node_id: u32,
}

impl HlcClock {
    /// Issue a new timestamp for a local operation.
    pub fn now(&mut self) -> HlcTimestamp { ... }

    /// Observe an incoming remote timestamp (called during sync bundle apply).
    pub fn observe(&mut self, remote: HlcTimestamp) { ... }
}
```

**`now()` algorithm:**
1. `wall = system_time_ms()`
2. `wall_ms = max(wall, self.wall_ms)`
3. If `wall_ms > self.wall_ms`: `counter = 0`; else `counter += 1`
4. `self.wall_ms = wall_ms; self.counter = counter`
5. Return `HlcTimestamp { wall_ms, counter, node_id: self.node_id }`

**`observe()` algorithm:**
1. `wall_ms = max(system_time_ms(), self.wall_ms, remote.wall_ms)`
2. Set counter based on which max won (see spec §4.3)
3. Update `self.wall_ms`, `self.counter`

### 1.3 DB persistence of HLC state

New table added in the DB migration:

```sql
CREATE TABLE IF NOT EXISTS hlc_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    wall_ms INTEGER NOT NULL,
    counter INTEGER NOT NULL,
    node_id INTEGER NOT NULL
);
```

`HlcClock` is loaded from this table when the workspace opens, and updated (within the same transaction) every time `now()` is called. This ensures monotonicity across app restarts.

---

## 2. Updated Operation Enum

### 2.1 Common field changes across all variants

| Field | Before | After |
|---|---|---|
| `timestamp` | `i64` (Unix seconds) | `HlcTimestamp` |
| `operation_id` | `String` | `String` (UUID, unchanged) |
| `device_id` | `String` | `String` (UUID, unchanged) |
| `created_by` / `modified_by` | `i64` (stale/wrong) | `String` (base64 Ed25519 public key) |
| `signature` | absent | `String` (base64 Ed25519 signature) |

`created_by`/`modified_by` carry the author's Ed25519 **public key** (32 bytes, base64-encoded). This is available from the unlocked identity at the time of operation creation. If no identity is unlocked (e.g. headless or test mode), these fields are set to an empty string and `signature` is also empty.

### 2.2 Variants added

**`UpdateNote`** — separates title updates from field updates (enables separate LWW for title):
```rust
UpdateNote {
    operation_id: String,
    timestamp: HlcTimestamp,
    device_id: String,
    note_id: String,
    title: String,
    modified_by: String,   // public key or ""
    signature: String,     // base64 or ""
},
```

All `workspace.rs` methods that update the title (currently via a `notes` table UPDATE) will emit this variant in addition to `UpdateField` variants.

**`SetTags`** — replaces direct DB writes to `note_tags` with a logged operation:
```rust
SetTags {
    operation_id: String,
    timestamp: HlcTimestamp,
    device_id: String,
    note_id: String,
    tags: Vec<String>,
    modified_by: String,
    signature: String,
},
```

Currently `update_note_tags` writes directly without logging. It will now emit this.

### 2.3 Position type changes

`CreateNote.position: i32` → `f64`
`MoveNote.new_position: i32` → `f64`

The `notes` table stores `position REAL` in SQLite (SQLite is flexible, but we update the Rust types and all call sites). Existing integer values survive as-is — SQLite will read them as `f64` naturally.

### 2.4 Removed fields

`CreateNote.created_by` was `i64` (device id as int — incorrect). Replaced with `String` public key. `UpdateField.modified_by` was similarly `i64`. Both become `String`.

### 2.5 Full updated enum signature (abbreviated)

```rust
pub enum Operation {
    CreateNote    { operation_id, timestamp: HlcTimestamp, device_id,
                    note_id, parent_id, position: f64, node_type, title, fields,
                    created_by: String, signature: String },
    UpdateNote    { operation_id, timestamp: HlcTimestamp, device_id,
                    note_id, title, modified_by: String, signature: String },
    UpdateField   { operation_id, timestamp: HlcTimestamp, device_id,
                    note_id, field, value, modified_by: String, signature: String },
    DeleteNote    { operation_id, timestamp: HlcTimestamp, device_id,
                    note_id, deleted_by: String, signature: String },
    MoveNote      { operation_id, timestamp: HlcTimestamp, device_id,
                    note_id, new_parent_id, new_position: f64,
                    moved_by: String, signature: String },
    SetTags       { operation_id, timestamp: HlcTimestamp, device_id,
                    note_id, tags: Vec<String>, modified_by: String, signature: String },
    CreateUserScript { ..., created_by: String, signature: String },
    UpdateUserScript { ..., modified_by: String, signature: String },
    DeleteUserScript { ..., deleted_by: String, signature: String },
    RetractOperation { operation_id, timestamp: HlcTimestamp, device_id,
                       retracted_ids, inverse, propagate },
}
```

`RetractOperation` carries no `*_by` or `signature` in M2 (undo is always local; signing undo operations is a future concern).

---

## 3. Operation Signing

### 3.1 What is signed

The **canonical signing payload** is the operation serialised to JSON with `signature` set to `""` (empty string). This is stable because serde field order is deterministic for structs. Specifically:

```
payload = serde_json::to_string(op_with_empty_signature)
sig = signing_key.sign(payload.as_bytes())
op.signature = base64::encode(sig.to_bytes())
```

### 3.2 Where signing happens

`workspace.rs` has a helper `make_hlc_timestamp()` that calls `self.hlc.now()` and persists the new HLC state. We add a parallel helper `sign_if_possible(op: &mut Operation)` that:
1. Checks `self.signing_key: Option<SigningKey>` (new field on `Workspace`)
2. If `Some(key)`: fills in `created_by`/`modified_by` with the public key (base64), signs the payload, sets `signature`
3. If `None`: leaves both fields as `""`

### 3.3 Where the signing key comes from

When a workspace is opened via `Workspace::open()`, the caller (Tauri `open_workspace` command in `lib.rs`) now passes in an `Option<&UnlockedIdentity>`. The workspace stores `signing_key: Option<SigningKey>` extracted from the unlocked identity.

### 3.4 Verification (future — not in M2 UI)

`pub fn verify_operation(op: &Operation, public_key: &VerifyingKey) -> bool` is implemented in `operation.rs` but not yet called anywhere. It will be called during `.swarm` bundle application in a future milestone.

---

## 4. DB Schema Migration

Migration is number **N+1** (whichever the next migration index is). Applied automatically on workspace open via the existing migration runner in `storage.rs`.

### 4.1 `operations` table

```sql
-- Remove single timestamp column, add three HLC columns
ALTER TABLE operations RENAME COLUMN timestamp TO timestamp_wall_ms_old;
ALTER TABLE operations ADD COLUMN timestamp_wall_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE operations ADD COLUMN timestamp_counter INTEGER NOT NULL DEFAULT 0;
ALTER TABLE operations ADD COLUMN timestamp_node_id INTEGER NOT NULL DEFAULT 0;

-- Migrate: convert seconds → milliseconds, counter=0, node_id=0
UPDATE operations SET
    timestamp_wall_ms = timestamp_wall_ms_old * 1000,
    timestamp_counter = 0,
    timestamp_node_id = 0;

-- Drop old column (requires SQLite 3.35+; use CREATE TABLE AS workaround if needed)
-- We recreate the table to drop the column cleanly
```

In practice, SQLite < 3.35 doesn't support `DROP COLUMN`. We use the recreate-copy-drop pattern that the existing migration runner already does for other migrations.

The `operation_data` JSON column is preserved as-is. Old serialised operations still deserialise because `timestamp` (old field) is embedded in `operation_data` — but we don't re-read it from JSON; we use the three new DB columns for ordering.

> **Important:** `operation_data` JSON is the canonical operation record. The three timestamp columns are **indexed copies** for SQL ordering. They must always be kept in sync.

### 4.2 New `hlc_state` table

```sql
CREATE TABLE IF NOT EXISTS hlc_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    wall_ms INTEGER NOT NULL,
    counter INTEGER NOT NULL,
    node_id INTEGER NOT NULL
);

-- Seed from max timestamp in existing operations
INSERT OR IGNORE INTO hlc_state (id, wall_ms, counter, node_id)
    SELECT 1, COALESCE(MAX(timestamp_wall_ms), 0), 0, 0 FROM operations;
```

### 4.3 `notes` table position column

`notes.position` is currently `INTEGER` in schema. SQLite stores it as integer. We change the schema type to `REAL` and cast all existing values:

```sql
-- In the recreation of the notes table during migration:
-- position REAL NOT NULL DEFAULT 0.0
```

No data is lost — integers become floats with the same value (e.g. `2` → `2.0`).

---

## 5. Updated `operation_log.rs`

### 5.1 `log()` method

Now writes to three timestamp columns:

```rust
tx.execute(
    "INSERT INTO operations (operation_id, timestamp_wall_ms, timestamp_counter,
     timestamp_node_id, device_id, operation_type, operation_data, synced)
     VALUES (?, ?, ?, ?, ?, ?, ?, 0)",
    params![
        op.operation_id(),
        op.timestamp().wall_ms as i64,
        op.timestamp().counter as i64,
        op.timestamp().node_id as i64,
        op.device_id(),
        self.operation_type_name(op),
        op_json,
    ],
)?;
```

### 5.2 `list()` method

Ordering changes:
```sql
ORDER BY timestamp_wall_ms DESC, timestamp_counter DESC, timestamp_node_id DESC, id DESC
```

Filters: `since`/`until` now operate on `timestamp_wall_ms` (milliseconds). The `OperationSummary.timestamp` field is renamed to `timestamp_wall_ms: u64` and the frontend's date display updated accordingly.

### 5.3 `WithSync` purge

Now uses `timestamp_wall_ms` (as ms) instead of `timestamp` (seconds):
```sql
DELETE FROM operations WHERE synced = 1 AND timestamp_wall_ms < ?
-- cutoff = (Utc::now().timestamp_millis() - retention_days * 86_400_000)
```

### 5.4 `OperationSummary`

```rust
pub struct OperationSummary {
    pub operation_id: String,
    pub timestamp_wall_ms: u64,   // was: timestamp: i64
    pub device_id: String,
    pub operation_type: String,
    pub target_name: String,
    pub author_key: String,       // new: first 8 chars of base64 public key, or ""
}
```

---

## 6. `workspace.rs` Changes

### 6.1 New `Workspace` fields

```rust
pub struct Workspace {
    // ... existing fields ...
    hlc: HlcClock,
    signing_key: Option<SigningKey>,  // None when no identity is unlocked
}
```

`hlc` is loaded from `hlc_state` on open; `signing_key` is passed in from the caller.

### 6.2 New helpers

```rust
fn next_timestamp(&mut self, tx: &Transaction) -> Result<HlcTimestamp> {
    let ts = self.hlc.now();
    tx.execute(
        "INSERT OR REPLACE INTO hlc_state (id, wall_ms, counter, node_id) VALUES (1, ?, ?, ?)",
        [ts.wall_ms as i64, ts.counter as i64, ts.node_id as i64],
    )?;
    Ok(ts)
}

fn sign_op(&self, op: &mut Operation) {
    if let Some(key) = &self.signing_key {
        // set created_by/modified_by/deleted_by field from key.verifying_key()
        // sign canonical payload, set signature field
    }
}
```

Every existing mutation method gets two call-sites updated:
1. `timestamp: chrono::Utc::now().timestamp()` → `timestamp: self.next_timestamp(&tx)?`
2. After constructing the operation: `self.sign_op(&mut op);`

### 6.3 Position type changes

`create_note(parent_id, position: f64, ...)` and `move_note(..., new_position: f64)`. All call sites in `workspace.rs` that compute positions with `i32` arithmetic switch to `f64`. The tree position assignment logic (gapless integer insertion) stays the same semantically — positions are still integers in the DB (stored as REAL), just the type is `f64` now. Fractional positions are not yet assigned in M2 (that's a conflict resolution concern for sync).

---

## 7. `undo.rs` Changes

`RetractInverse` contains positions (`i32`) in `MoveNote` inverse. These change to `f64`:

```rust
RetractInverse::MoveNote {
    note_id: String,
    old_parent_id: Option<String>,
    old_position: f64,   // was i32
    // ...
}
```

All undo inverse construction in `workspace.rs` that reads `notes.position` as `i32` → `f64`.

---

## 8. Tauri / Frontend

### 8.1 `lib.rs` — pass signing key to workspace

`open_workspace` command (and `create_workspace`) now extracts the unlocked identity's signing key and passes it to `Workspace::open()`/`create()`. The workspace method signatures gain `signing_key: Option<SigningKey>`.

The unlocked identity is looked up from `state.unlocked_identities` by the workspace's bound identity UUID (available via `state.identity_manager`).

### 8.2 `OperationsLogDialog.tsx` — HLC timestamp display

`OperationSummary.timestamp` (seconds) → `timestamp_wall_ms` (milliseconds). Update the date display:
```ts
new Date(summary.timestampWallMs)  // instead of new Date(summary.timestamp * 1000)
```

Add an `authorKey` column (first 8 chars of base64 key) as a compact attribution indicator.

---

## 9. Dependencies

New in `krillnotes-core/Cargo.toml`:
```toml
blake3 = "1"
```

`ed25519-dalek` is already present (from identity system). No other new deps.

---

## 10. Testing

- `HlcClock::now()` is monotonic across rapid successive calls
- `HlcClock::observe()` updates clock correctly for all three cases (local > remote, remote > local, tie)
- `node_id_from_device` is stable (same input → same output)
- DB migration: workspace with old operations opens without error; operations still readable; timestamps converted correctly
- `Operation` round-trips through serde correctly with new types
- `sign_op` + `verify_operation` round-trip: valid sig passes, corrupted sig fails
- `OperationLog::list()` ordering: newer HLC timestamps sort first
- All existing tests continue to pass (position `i32` → `f64` changes affect test construction)

---

## 11. Out of Scope for M2

- Gated function API (scripting v2) → M3
- `.swarm` bundle format → M4
- `SetPermission` / `JoinWorkspace` / `RevokePermission` operations → M5 (RBAC)
- `AddAttachment` / `RemoveAttachment` operations (attachments are logged separately in M2; full sync integration in M6)
- Signature verification on incoming operations → bundle apply in M4
- Frontend display of identity attribution in operations log → polish pass
