# Swarm Milestone 2 — HLC + Operation Enum + Signatures: Implementation Plan

**Date:** 2026-03-05
**Design doc:** `2026-03-05-swarm-hlc-operation-design.md`
**Branch:** `feat/swarm-hlc-operation-log` off `swarm`
**Target:** `swarm` branch (PR → swarm; swarm → master when full sync is ready)

---

## Phase 1 — Core HLC Infrastructure

### Task 1: Add blake3 dependency
- File: `krillnotes-core/Cargo.toml`
- Add: `blake3 = "1"`

### Task 2: Create `hlc.rs` module
- File: `krillnotes-core/src/core/hlc.rs`
- Implement:
  - `HlcTimestamp { wall_ms: u64, counter: u32, node_id: u32 }`
  - `Ord` + `PartialOrd` impl (lexicographic on wall_ms, counter, node_id)
  - Custom `Serialize`/`Deserialize` using compact 3-element JSON array `[u64, u32, u32]`
  - `node_id_from_device(device_id: &Uuid) -> u32` (BLAKE3 first 4 bytes)
  - `HlcClock { wall_ms, counter, node_id }` with `new(node_id)`, `now(&mut self) -> HlcTimestamp`, `observe(&mut self, remote: HlcTimestamp)`
  - `HlcClock::load_from_db(conn: &Connection) -> Result<HlcClock>` — reads `hlc_state` table
- Tests:
  - `now()` is monotonic
  - `observe()` updates clock (all three cases)
  - `node_id_from_device()` is stable
  - Serde round-trip produces `[wall_ms, counter, node_id]` array

### Task 3: Register `hlc.rs` in the module tree
- File: `krillnotes-core/src/core/mod.rs`
- Add: `pub mod hlc;`
- File: `krillnotes-core/src/lib.rs`
- Re-export: `pub use core::hlc::{HlcTimestamp, HlcClock};`

---

## Phase 2 — DB Migration

### Task 4: Write migration SQL
- File: `krillnotes-core/src/core/storage.rs` (migration runner)
- Add migration N+1 that:
  1. Creates `hlc_state` table
  2. Recreates `operations` table with three HLC columns instead of one `timestamp`
  3. Copies data: `timestamp_wall_ms = old_timestamp * 1000`, counter=0, node_id=0
  4. Recreates `notes` table with `position REAL` instead of `INTEGER`
  5. Seeds `hlc_state` from `MAX(timestamp_wall_ms)` of existing operations
- The recreation pattern (CREATE new → INSERT INTO SELECT → DROP old → RENAME) is already used in existing migrations — follow the same pattern

### Task 5: Update `schema.sql`
- File: `krillnotes-core/src/core/schema.sql`
- Update `operations` table DDL: replace `timestamp INTEGER` with three HLC columns
- Add `hlc_state` table DDL
- Update `notes` table DDL: `position REAL NOT NULL DEFAULT 0.0`

---

## Phase 3 — Operation Enum

### Task 6: Update `operation.rs` — field types
- File: `krillnotes-core/src/core/operation.rs`
- For every variant:
  - Change `timestamp: i64` → `timestamp: HlcTimestamp`
  - Change `created_by: i64` / `modified_by: i64` → `created_by: String` / `modified_by: String` / `deleted_by: String`
  - Add `signature: String` to all mutating variants (not `RetractOperation`)
- Change `CreateNote.position: i32` → `f64`
- Change `MoveNote.new_position: i32` → `f64`
- Add `UpdateNote { operation_id, timestamp, device_id, note_id, title, modified_by, signature }` variant
- Add `SetTags { operation_id, timestamp, device_id, note_id, tags: Vec<String>, modified_by, signature }` variant
- Update `operation_id()`, `timestamp()`, `device_id()` accessor methods — add new variants to each match arm
- Add `author_key(&self) -> &str` accessor (returns `created_by`/`modified_by`/etc. for applicable variants, `""` for others)
- Add `sign(&mut self, key: &SigningKey)` method — serialises self with `signature = ""`, signs, sets field
- Add `verify(&self, pubkey: &VerifyingKey) -> bool` method — same canonical payload, verifies
- Update tests: fix `CreateNote` construction (position `f64`, timestamp `HlcTimestamp`, new fields)

### Task 7: Update `operation_log.rs`
- File: `krillnotes-core/src/core/operation_log.rs`
- `log()`: write to `timestamp_wall_ms`, `timestamp_counter`, `timestamp_node_id` columns
- `list()`: ORDER BY the three columns DESC; update `since`/`until` filter to treat param as wall_ms (ms)
- `OperationSummary`: rename `timestamp: i64` → `timestamp_wall_ms: u64`, add `author_key: String`
- `operation_type_name()`: add `UpdateNote`, `SetTags` arms
- `WithSync` purge: use `timestamp_wall_ms` with ms cutoff
- Update tests: construct operations with `HlcTimestamp`; check ordering; update since/until ms values

---

## Phase 4 — Workspace Integration

### Task 8: Update `Workspace` struct
- File: `krillnotes-core/src/core/workspace.rs`
- Add fields: `hlc: HlcClock`, `signing_key: Option<ed25519_dalek::SigningKey>`
- Add helper: `fn next_timestamp(&mut self, tx: &Transaction) -> Result<HlcTimestamp>` — calls `self.hlc.now()`, persists to `hlc_state`, returns timestamp
- Add helper: `fn sign_op(&self, op: &mut Operation)` — sets `created_by`/etc and `signature` from `self.signing_key`
- Update `Workspace::create(path, password, signing_key: Option<SigningKey>)` — add parameter, initialise `hlc` with seeded clock
- Update `Workspace::open(path, password, signing_key: Option<SigningKey>)` — load HLC from DB; pass signing key
- Load `hlc` from `hlc_state` table in both `create` and `open`

### Task 9: Update all mutation methods in `workspace.rs`
For every method that currently writes `timestamp: chrono::Utc::now().timestamp()`:
- Replace with `timestamp: self.next_timestamp(&tx)?`
- After operation construction, call `self.sign_op(&mut op)`
- Methods to update: `create_note`, `update_note` (title updates → emit `UpdateNote`), `update_field` → `UpdateField`, `delete_note`, `move_note`, `update_note_tags` (emit `SetTags`), `create_user_script`, `update_user_script`, `delete_user_script`

**Title update split:** Currently saving a note title writes directly to `notes.title`. After this change, `workspace.update_note_title()` (or the title-update path inside `save_note()`) must also emit an `UpdateNote` operation.

**Tag update:** `update_note_tags()` currently writes to `note_tags` without logging. After this change it emits a `SetTags` operation.

### Task 10: Update `undo.rs`
- File: `krillnotes-core/src/core/undo.rs`
- Change all `position: i32` / `old_position: i32` fields in `RetractInverse` variants to `f64`
- Update all inverse-construction code in `workspace.rs` that reads position from the DB

---

## Phase 5 — Tauri Integration

### Task 11: Update `Workspace::create` and `Workspace::open` call sites
- File: `krillnotes-desktop/src-tauri/src/lib.rs`
- For `create_workspace` command: look up the unlocked identity from `state.unlocked_identities` using the identity UUID passed in; extract `signing_key`; pass to `Workspace::create()`
- For `open_workspace` command: look up the bound identity UUID from `identity_manager.get_binding(workspace_id)`; find in `unlocked_identities`; pass signing key
- When no identity is unlocked (e.g. workspace opened without unlock): pass `None`

### Task 12: Update `OperationsLogDialog.tsx`
- File: `krillnotes-desktop/src/components/OperationsLogDialog.tsx`
- Update `OperationSummary` TypeScript type: `timestampWallMs: number` (ms, not seconds)
- Fix date display: `new Date(summary.timestampWallMs)` instead of `new Date(summary.timestamp * 1000)`
- Add `authorKey` column: display `summary.authorKey.slice(0, 8)` or `"—"` if empty
- Update `since`/`until` filter values that are currently passed as Unix seconds → milliseconds

---

## Phase 6 — Tests & Validation

### Task 13: Fix all existing tests
All tests that construct `Operation::CreateNote` or similar need updating:
- `timestamp: 1000` → `timestamp: HlcTimestamp { wall_ms: 1_000_000, counter: 0, node_id: 0 }`
- `position: 0` → `position: 0.0`
- `created_by: 0` → `created_by: String::new()`
- `signature: String::new()` (new field)

Files to check: `operation.rs`, `operation_log.rs`, `workspace.rs`, `undo.rs`, any integration tests.

### Task 14: Add HLC-specific tests
- Workspace open after migration: old operations readable, timestamps in ms
- Two rapid operations: HLC counter increments correctly
- `sign_op` + `verify_operation` round-trip
- `SetTags` operation appears in log after `update_note_tags`
- `UpdateNote` operation appears in log after title save

### Task 15: TypeScript type check
- `cd krillnotes-desktop && npx tsc --noEmit`
- Fix any type errors from `OperationSummary` shape change

---

## Phase 7 — Documentation & PR

### Task 16: Update DEVELOPER.md operation log section
- Update the `Operation` enum example in the doc to show `HlcTimestamp`, `f64` positions, `UpdateNote`, `SetTags`
- Update the "Key Types Quick Reference" table
- Add `hlc.rs` to the repository layout

### Task 17: Commit and open PR
- Target branch: `swarm`
- PR title: `feat: HLC timestamps, signed operations, f64 positions (swarm milestone 2)`

---

## Sequence summary

```
Task 1-3   : HLC infrastructure (hlc.rs + module wiring)
Task 4-5   : DB migration
Task 6-7   : Operation enum + log
Task 8-10  : Workspace integration
Task 11-12 : Tauri + frontend
Task 13-15 : Tests + type check
Task 16-17 : Docs + PR
```

Total estimated task count: 17. Each is independently commitable.
