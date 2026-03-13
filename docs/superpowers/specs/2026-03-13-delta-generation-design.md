# Delta Generation & Ingest — Design Spec

**Date:** 2026-03-13
**Phase:** A12 (Delta Bundle Generation) + A13 (Delta Bundle Ingest, partial stub)
**Prerequisite:** A1–A11 complete (contacts, peer registry, bundle codec, snapshot import)
**Status:** Approved

---

## Overview

With snapshot import (A11) complete, peers can onboard into a workspace. This phase adds the ongoing sync loop: generating delta `.swarm` bundles for known peers (A12) and applying received delta bundles to the local workspace (A13, stub — no RBAC or conflict resolution yet).

Delta bundles carry all workspace operations since the last sync watermark, encrypted per-recipient. Each peer requires a *separate* delta file (different encryption key, different `since_operation_id`). The primary UX surface is a batch export: one menu action generates delta files for all selected peers into a user-chosen directory.

### Sync is bidirectional and transitive

**Bidirectional:** After Alice sends Bob a snapshot, Bob can immediately generate a delta back to Alice with his own changes, then Alice generates a delta back to Bob, and so on. The watermark `last_sent_op` tracks what each side has sent, independently. See Section 1.3 — snapshot import must initialise `last_sent_op` to enable this.

**Transitive:** If Alice invites Bob and Bob invites George, George's operations travel to Alice via Bob — Alice never syncs directly with George. This works automatically: `operations_since` returns *all* operations in the log since the watermark, not just locally-authored ones. When Bob applies George's delta, George's operations enter Bob's log. Bob's next delta to Alice includes them. Alice can verify George's individual operation signatures once George's public key propagates via TOFU contact registration.

---

## Constraints (Regression Safety)

These constraints are inviolable. Any implementation that violates them is incorrect.

| Constraint | Rationale |
|------------|-----------|
| No storage outside `~/.config/krillnotes/` | Past regression: files accidentally written to `~/Library/Application Support`. Delta generation returns bytes; files are written only to a user-chosen directory. |
| Workspace always connected to exactly one identity | The `generate_delta` function requires a `&SigningKey` parameter — it cannot be called without a valid unlocked identity. The Tauri command fails early if the identity is locked. |
| Incoming operations must NOT be re-signed | `apply_incoming_operation` is a new, dedicated method. It never calls existing workspace mutation methods, which generate new operation IDs and new timestamps. |
| HLC must be advanced for every incoming operation | `self.hlc.observe(op.timestamp())` is called in `apply_incoming_operation` before returning. (`hlc` is a private field of `Workspace`; calling it directly inside the method is correct.) Forgetting this breaks future conflict resolution. |
| Incoming `RetractOperation` with `propagate = false` must not be applied | These are local-only undo markers. They must be filtered out both during generation (not sent) and during ingest (not applied). |

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  krillnotes-desktop (Tauri + React)                      │
│                                                          │
│  CreateDeltaDialog.tsx                                   │
│    → generate_deltas_for_peers (Tauri command)           │
│    → get_workspace_peers (Tauri command)                 │
│                                                          │
│  handle_swarm_open (existing handler, extended)          │
│    → apply_delta_bundle (Tauri command)                  │
└────────────────────┬────────────────────────────────────┘
                     │ invoke
┌────────────────────▼────────────────────────────────────┐
│  krillnotes-desktop/src-tauri/src/lib.rs                 │
│                                                          │
│  get_workspace_peers      → PeerRegistry::list_peers_info│
│  generate_deltas_for_peers → swarm::sync::generate_delta │
│  apply_delta_bundle        → swarm::sync::apply_delta    │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  krillnotes-core                                         │
│                                                          │
│  swarm/sync.rs  (NEW — orchestration)                    │
│    generate_delta(workspace, peer_id, key, contacts)     │
│    apply_delta(bundle, workspace, key, contacts)         │
│                                                          │
│  swarm/delta.rs (EXISTING — codec only)                  │
│    create_delta_bundle / parse_delta_bundle              │
│                                                          │
│  workspace.rs  (two new primitive methods)               │
│    operations_since(since_op_id) → Vec<Operation>        │
│    apply_incoming_operation(op)  → bool                  │
│                                                          │
│  peer_registry.rs  (EXISTING — no changes)               │
│  contact.rs        (EXISTING — no changes)               │
│  operation_log.rs  (EXISTING — no changes)               │
└─────────────────────────────────────────────────────────┘
```

---

## Section 1: Core Layer

### 1.1 New file: `krillnotes-core/src/core/swarm/sync.rs`

All orchestration for A12 and A13 lives here. The existing `delta.rs` is codec-only and is not modified.

#### `generate_delta`

```rust
pub fn generate_delta(
    workspace: &mut Workspace,
    peer_device_id: &str,
    signing_key: &SigningKey,
    contact_manager: &ContactManager,
) -> Result<Vec<u8>>
```

**Steps:**

1. Look up peer in peer registry — return `KrillnotesError::NotFound` if absent.
2. Return `KrillnotesError::Swarm("snapshot must precede delta — no last_sent_op for peer")` if `last_sent_op` is `None`.
3. Call `workspace.operations_since(peer.last_sent_op.as_deref(), peer_device_id)` to get the operation list. An empty list is valid (produces an empty delta bundle — useful as a "heartbeat" that the peer knows you're alive).
4. Resolve peer's `VerifyingKey` from `contact_manager` using `peer.peer_identity_id`.
5. Call `create_delta_bundle(DeltaParams { workspace_id, workspace_name, source_device_id, since_operation_id: peer.last_sent_op.unwrap(), operations, sender_key: signing_key, recipient_keys, recipient_peer_ids })`. The `.unwrap()` is safe here because step 2 has already asserted `last_sent_op` is `Some`.
6. If the operation list is non-empty: update `last_sent_op` to the `operation_id` of the last (HLC-latest) operation in the list.
7. Return bundle bytes.

#### `apply_delta`

```rust
pub fn apply_delta(
    bundle_bytes: &[u8],
    workspace: &mut Workspace,
    recipient_key: &SigningKey,
    contact_manager: &mut ContactManager,
) -> Result<ApplyResult>

pub struct ApplyResult {
    pub operations_applied: usize,
    pub operations_skipped: usize,        // duplicate operation_ids
    pub sender_device_id: String,
    pub sender_public_key: String,
    pub new_tofu_contacts: Vec<String>,   // display names of auto-registered contacts
}
```

**Steps:**

1. `parse_delta_bundle(bundle_bytes, recipient_key)` — decrypts and verifies bundle-level signature.
2. Assert `parsed.workspace_id == workspace.workspace_id()` — return `KrillnotesError::Swarm("workspace_id mismatch")` if not.
3. For each operation in chronological order:
   - Extract the author's public key from the operation.
   - If the author is not in `contact_manager`, auto-register them with `TrustLevel::Tofu`. For the `declared_name`: use `declared_name` from `Operation::JoinWorkspace` if the variant carries one; for all other variants, use the first 8 characters of the base64 public key followed by `"…"` as a synthetic placeholder (consistent with the fallback already used in `list_peers_info`). Record in `new_tofu_contacts`.
   - Call `workspace.apply_incoming_operation(op)` → returns `true` (applied) or `false` (skipped — duplicate).
4. Upsert the sender in the peer registry: `workspace.upsert_sync_peer(sender_device_id, sender_public_key, None, last_op_id)`. `sender_device_id` comes from `ParsedDelta.sender_device_id` — see note below.
5. Return `ApplyResult`.

**Note — `ParsedDelta` extension required:** `sender_device_id` is present in `SwarmHeader.source_device_id` but is currently discarded by `parse_delta_bundle`. `ParsedDelta` must be extended with a `sender_device_id: String` field populated from `header.source_device_id`. This is an additive, backwards-compatible change to the existing codec struct. The "codec-only, not modified" comment in the spec's architecture section is relaxed for this one field addition.

---

### 1.2 New workspace primitives

#### `Workspace::operations_since`

```rust
pub fn operations_since(
    &self,
    since_op_id: Option<&str>,
    exclude_device_id: &str,
) -> Result<Vec<Operation>>
```

Queries the `operations` table in ascending HLC order (`timestamp_wall_ms, timestamp_counter, timestamp_node_id ASC`).

**Filtering rules:**

The HLC "strictly greater than" comparison cannot use a simple single-column `>` in SQLite. The correct three-column expansion is:

```sql
WHERE (timestamp_wall_ms > ?)
   OR (timestamp_wall_ms = ? AND timestamp_counter > ?)
   OR (timestamp_wall_ms = ? AND timestamp_counter = ? AND timestamp_node_id > ?)
AND device_id != ?
ORDER BY timestamp_wall_ms ASC, timestamp_counter ASC, timestamp_node_id ASC
```

The three values for the HLC tuple are obtained by first querying the `since_op_id` row. A naive `WHERE timestamp_wall_ms > ?` would silently drop operations that share the same wall clock millisecond as the watermark — this must not be used.

**Schema change required:** A covering index must be added to the `operations` table migration:

```sql
CREATE INDEX IF NOT EXISTS idx_operations_hlc
    ON operations(timestamp_wall_ms, timestamp_counter, timestamp_node_id);
```

Without this index, `operations_since` degrades to a full table scan on large workspaces.

**Post-query Rust filter:** After deserializing, filter out `Operation::RetractOperation { propagate: false, .. }` (the `propagate` flag is inside the `operation_data` JSON blob, not a SQL column).

Returns the full deserialized `Operation` structs in chronological order.

#### `Workspace::apply_incoming_operation`

```rust
pub fn apply_incoming_operation(&mut self, op: &Operation) -> Result<bool>
```

Applies a single operation received from a remote peer. Returns `true` if applied, `false` if skipped (already present).

**Behaviour:**

1. Advance local HLC: `self.hlc.observe(op.timestamp())`.
2. Attempt to insert into `operations` table with `synced = 1`, preserving all original fields (`operation_id`, `timestamp_*`, `device_id`, `operation_data`). Use `INSERT OR IGNORE` — if the row already exists, return `Ok(false)`.
3. Apply the state change to the working tables (`notes`, `note_tags`, `note_permissions`, `user_scripts`) using the same SQL as the corresponding local mutation, but driven by the incoming operation's data rather than new inputs. This is a `match op { ... }` over all Operation variants.
4. `RetractOperation { propagate: false }` — skip entirely (return `Ok(false)`); never apply incoming local-only retracts.
5. `RetractOperation { propagate: true }` — insert the operation row into the log (step 2 handles this), then return `Ok(true)` with no further state change. State revert (un-applying the retracted operation's effect) is deferred entirely to WP-C. No additional columns or schema changes are needed here.
6. Return `Ok(true)`.

**Important:** This method never calls any existing workspace mutation method (e.g. `create_note`, `update_note_title`). Those methods generate new `operation_id`, new `HlcTimestamp`, and a new signature — all wrong for incoming operations. The state-change SQL is written directly in this method.

---

### 1.3 A11 fix — snapshot import must initialise both watermarks

**Bug in current implementation (lib.rs line ~3726):** `upsert_sync_peer` is called with `last_sent_op = None` during snapshot import. This leaves the importing peer's `last_sent_op` as `None` for the snapshot sender, which causes `generate_delta` (step 2) to return an error — permanently blocking bidirectional exchange.

**Fix:** When importing a snapshot, set **both** `last_received_op` and `last_sent_op` to `as_of_operation_id`:

```rust
// Current (broken for bidirectional sync):
ws.upsert_sync_peer(&placeholder_device_id, &parsed.sender_public_key,
    None,                              // last_sent_op
    Some(&parsed.as_of_operation_id),  // last_received_op
);

// Fixed:
ws.upsert_sync_peer(&placeholder_device_id, &parsed.sender_public_key,
    Some(&parsed.as_of_operation_id),  // last_sent_op — snapshot covers up to here
    Some(&parsed.as_of_operation_id),  // last_received_op
);
```

**Rationale:** The snapshot represents everything the sender had up to `as_of_operation_id`. The importer acknowledges this as the shared baseline. Setting `last_sent_op = as_of_operation_id` means "my first delta to this peer covers only what happened after the snapshot" — which is exactly correct.

This fix is part of the A12/A13 implementation scope (it is a prerequisite for delta generation to work at all) and must be applied to the existing snapshot import handler in `lib.rs`.

---

## Section 2: Tauri Commands

Three commands added to `lib.rs`:

### `get_workspace_peers`

```rust
#[tauri::command]
async fn get_workspace_peers(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Vec<PeerInfo>, String>
```

Returns the peer list with resolved display names and fingerprints. Drives the `CreateDeltaDialog` peer checklist. Uses the existing `PeerRegistry` and contact manager lookup already implemented.

### `generate_deltas_for_peers`

```rust
#[tauri::command]
async fn generate_deltas_for_peers(
    window: Window,
    state: State<'_, AppState>,
    dir_path: String,
    peer_device_ids: Vec<String>,
) -> Result<GenerateDeltasResult, String>

pub struct GenerateDeltasResult {
    pub succeeded: Vec<String>,          // peer_device_ids
    pub failed: Vec<(String, String)>,   // (peer_device_id, error message)
    pub files_written: Vec<String>,      // absolute paths of written .swarm files
}
```

For each `peer_device_id`:
1. Call `swarm::sync::generate_delta(workspace, peer_device_id, signing_key, contact_manager)`.
2. Write bytes to `{dir_path}/delta-{peer_display_name}-{YYYY-MM-DD}.swarm`. If a file with that name already exists, append `-2`, `-3`, etc.
3. Record success or failure per-peer.

The identity's `SigningKey` is obtained from `AppState` (already unlocked when workspace is open). The workspace is identified by `window.label()`.

### `apply_delta_bundle` (delta ingest path)

The existing `handle_swarm_open` handler already dispatches on `SwarmMode::Delta`. It is extended to call `swarm::sync::apply_delta(bundle_bytes, workspace, recipient_key, contact_manager)` and emit a `workspace-updated` event so the frontend refreshes the tree view.

No new Tauri command is required for ingest — it flows through the existing `.swarm` file open handler.

---

## Section 3: Menu + UI

### Menu (`menu.rs` + locale JSON files)

New item in the **Edit** menu after the existing workspace actions:

```rust
// menu.rs
MenuItem::with_id(app, "create_delta_swarm", menu_strings.create_delta_swarm, true, None::<&str>)
```

All 7 locale files gain a `"createDeltaSwarm"` key. English: `"Create delta Swarm"`.

The menu event is caught in `App.tsx` and sets `showCreateDeltaDialog = true`.

### `CreateDeltaDialog.tsx`

New component, follows the pattern of existing dialogs (`AddNoteDialog`, `DeleteConfirmDialog`).

```
┌─ Create Delta Swarm ───────────────────────────────┐
│                                                     │
│  Save to directory:                                 │
│  [ /Users/alice/Sync/krillnotes  ] [Browse…]        │
│                                                     │
│  Generate delta for:                                │
│  ☑  Bob Chen              Writer  •••• zest moon   │
│  ☑  Carol – Field Lead    Reader  •••• oak rim      │
│  ☐  Dave                  Reader  — never synced    │
│                                                     │
│           [Cancel]          [Generate]              │
└─────────────────────────────────────────────────────┘
```

**Behaviour:**

- On open: calls `get_workspace_peers` to populate the list. Peers with `last_sent_op = null` are shown with "— never synced" and their checkbox is disabled (must send snapshot first).
- Directory field: empty on open. "Browse…" calls Tauri's `open({ directory: true })` dialog. Generate button is disabled until a directory is chosen and at least one peer is checked.
- On Generate: calls `generate_deltas_for_peers`. Shows inline per-peer status (✓ written / ✗ error message). Dialog stays open on partial failure so the user can see which peers failed.
- On full success: shows a brief success state with the output path, then closes after 2 seconds (or on "Close" button click).

---

## Section 4: Test Scenarios

These complement the existing T1–T11 tests in the WP-A spec.

### T-A12-1: Basic delta generation

1. Alice has workspace with Bob in peer registry; `last_sent_op` set from prior snapshot.
2. Alice creates two notes → two `CreateNote` operations.
3. Call `generate_delta(workspace, bob_device_id, alice_key, contact_manager)`.
4. Assert: bundle bytes non-empty; `parse_delta_bundle` succeeds; 2 operations in bundle.
5. Assert: `last_sent_op[Bob]` updated to the second operation's ID.

### T-A12-2: Empty delta (no new ops)

1. Same setup; no new operations since `last_sent_op`. Record the current `last_sent_op` value.
2. `generate_delta` returns a valid empty bundle (0 operations).
3. Assert: `last_sent_op[Bob]` equals the value recorded in step 1 (not `None`, not a new value — unchanged).

### T-A12-3: No snapshot precondition

1. Peer exists in registry but `last_sent_op = None`.
2. `generate_delta` returns `Err(...)` — must not produce a bundle.

### T-A12-4: RetractOperation filtering

1. Alice creates note → `CreateNote` op-1.
2. Alice undoes (local only, `propagate = false`) → `RetractOperation` op-2.
3. `generate_delta` → bundle contains only op-1. op-2 is excluded.

### T-A12-5: Echo prevention

1. Bob's `CreateNote` arrives via `apply_incoming_operation` (stored with `device_id = "dev-bob"`).
2. Alice generates delta for Bob.
3. Assert: Bob's own operation is not in the bundle.

### T-A13-1: Basic delta ingest

1. Bundle with 3 operations (CreateNote, UpdateField, SetTags) from Alice.
2. `apply_delta(bundle, bob_workspace, bob_key, bob_contacts)`.
3. Assert: all 3 operations present in Bob's workspace with correct state.
4. Assert: Alice's operations in Bob's `operations` table have `synced = 1`.
5. Assert: Bob's HLC advanced past all three operations' timestamps.

### T-A13-2: Duplicate delivery

1. Apply same bundle twice.
2. Second apply: all operations skipped (`operations_skipped = 3`). No DB error.
3. State unchanged (idempotent).

### T-A13-3: TOFU contact registration

1. Bundle contains an operation signed by an unknown identity Carol.
2. `apply_delta` auto-registers Carol in `contact_manager` with `TrustLevel::Tofu`.
3. `ApplyResult.new_tofu_contacts` contains Carol's name.

### T-A13-4 (partial): Incoming `UpdateSchema` operation

`UpdateSchema` modifies many notes in a batch and has a `notes_migrated` counter.

1. Apply a delta containing an `UpdateSchema` operation that renames a field on all notes.
2. Assert: the `UpdateSchema` operation is stored in the log with `synced = 1`.
3. Assert: notes in the workspace have the updated field structure.

Note: full semantics (handling schema conflicts, the `notes_migrated` counter) are deferred to WP-C. The A13 stub applies unconditionally.

### T-A13-6: Workspace ID mismatch

1. Bundle has `workspace_id` different from the target workspace.
2. `apply_delta` returns `Err(...)` before touching any state.

### T-A13-7: HLC advancement

1. Incoming operation has `wall_ms` far in the future.
2. After `apply_incoming_operation`, local HLC's `wall_ms` is at least that value.
3. Next local operation has a timestamp ≥ the incoming operation's timestamp.

### T-A13-8: Full round-trip (end-to-end)

1. Alice workspace has peer Bob (snapshot already sent, `last_sent_op` set).
2. Alice creates 3 notes, calls `generate_deltas_for_peers([bob_device_id])`.
3. Delta file written to temp dir.
4. Bob opens the file via `apply_delta`.
5. Assert Bob's workspace has same 3 notes.
6. Bob creates 1 note, generates delta for Alice.
7. Alice applies Bob's delta.
8. Both workspaces have 4 notes, identical state.

### T-A13-9: Bidirectional exchange after snapshot import (regression test for Section 1.3 fix)

1. Alice generates snapshot for Bob. Bob imports it. Both watermarks set to `as_of_operation_id`.
2. Assert: Bob's `last_sent_op[Alice]` is `Some(as_of_operation_id)` — not `None`.
3. Bob creates a note immediately after import.
4. Assert: `generate_delta(bob_workspace, alice_device_id, ...)` succeeds (no "snapshot must precede delta" error).
5. Bundle contains exactly 1 operation (Bob's new note — the snapshot baseline is excluded).

### T-A13-10: Transitive operation forwarding

1. Alice invites Bob (snapshot exchange). Bob invites George (snapshot exchange).
2. George creates a note → `CreateNote` operation in George's log.
3. George generates delta for Bob. Bob applies it → George's op now in Bob's log.
4. Bob generates delta for Alice.
5. Assert: Alice's delta from Bob contains George's `CreateNote` operation.
6. Assert: Alice can parse and apply it (George's op stored with `synced = 1` in Alice's log).
7. Assert: George auto-registered as TOFU contact in Alice's contact manager.

---

## Out of Scope

- RBAC enforcement during ingest — stub: apply all operations unconditionally (WP-B)
- Conflict resolution — stub: last-write-wins (WP-C)
- Individual per-operation signature verification during ingest — stub (WP-C)
- File attachments in delta bundles (WP-10)
- Watch folder / automatic sync (later phase)
- Subtree-scoped deltas (SA-005)
