# Owner-Only Script Enforcement — Design Spec

**Date:** 2026-03-14
**Status:** Approved

## Problem

Scripts define note schemas and hooks — they control what data types exist and how mutations behave. Currently, any peer who receives a workspace can create, update, or delete scripts both locally and via sync. A malicious or careless peer could push a broken schema that corrupts the workspace for everyone.

## Goal

Only the workspace **owner** (the identity that created the workspace) can mutate scripts. This is enforced at three levels:

1. **Core API** — All five script mutation methods (`create_user_script`, `update_user_script`, `delete_user_script`, `toggle_user_script`, `reorder_user_script`) reject non-owner callers with `KrillnotesError::NotOwner`. (`reorder_all_user_scripts` is also guarded, though it does not log operations.)
2. **Sync ingest** — `apply_incoming_operation()` silently skips script operations signed by anyone other than the owner.
3. **Frontend** — Script Manager disables all mutation controls for non-owners with an explanatory banner.

Additionally, the `owner_pubkey` is embedded in every `.swarm` bundle header (invite, accept, snapshot, delta) so that peers can cross-check ownership claims against the signed transport layer.

## Design Decisions

- **Single owner, no delegation.** There is exactly one owner per workspace — the creator. Ownership transfer is a future feature (not in scope).
- **`workspace_meta` KV storage.** Owner pubkey is stored as a `workspace_meta` row (`key = 'owner_pubkey'`), consistent with `device_id`, `identity_uuid`, and `workspace_id`.
- **Silent skip on unauthorized sync ops.** Rejecting a delta because of one bad operation would break the entire sync exchange. Instead, unauthorized script ops are skipped (logged as warning), and the rest of the delta proceeds normally.
- **No new DB tables or migrations.** All changes fit within existing schema. Existing workspaces created before this feature will not have an `owner_pubkey` row — `open()` handles this defensively (see Section 2).

## Security Model

The attack vector: a peer modifies their local `workspace_meta` to set their own pubkey as `owner_pubkey`, then creates script operations signed with their key.

**Why this is safe:**
1. Script operations are **signed** with the author's Ed25519 key.
2. The **receiving** peer checks the operation's signer (`created_by` / `modified_by` / `deleted_by`) against their own locally-stored `owner_pubkey`.
3. The attacker can't forge the real owner's signature — the operation arrives signed by the wrong key and gets rejected.
4. Honest peers receive the correct `owner_pubkey` through the **signed invite/snapshot exchange**, which the attacker also can't forge.
5. Every `.swarm` bundle header includes `owner_pubkey`, providing a cross-check: if the header's `owner_pubkey` doesn't match the local value, the bundle is rejected.

## Changes

### 1. Core — Error Variant

**File:** `krillnotes-core/src/core/error.rs`

Add variant to `KrillnotesError`:

```rust
#[error("Only the workspace owner can modify scripts")]
NotOwner,
```

Also add a `Self::NotOwner` arm to `user_message()`:
```rust
Self::NotOwner => "Only the workspace owner can modify scripts".to_string(),
```

### 2. Core — Workspace Ownership Storage & Accessors

**File:** `krillnotes-core/src/core/workspace/mod.rs`

**New field on `Workspace` struct:**
```rust
owner_pubkey: String,
```

**Set at creation** — all four `create*` methods (`create`, `create_with_id`, `create_empty`, `create_empty_with_id`) insert into `workspace_meta`:
```sql
INSERT INTO workspace_meta (key, value) VALUES ('owner_pubkey', <creator's base64 Ed25519 pubkey>)
```

**Read at open** — `open()` reads from `workspace_meta`:
```sql
SELECT value FROM workspace_meta WHERE key = 'owner_pubkey'
```

**Defensive fallback for pre-existing workspaces:** If `owner_pubkey` is absent from `workspace_meta` when `open()` is called, insert the current identity's pubkey as the owner. This makes the first post-upgrade opener the owner, which is correct for single-user workspaces. For shared workspaces, the true owner will be established via the next snapshot/invite exchange.

**New public methods:**
```rust
pub fn owner_pubkey(&self) -> &str { &self.owner_pubkey }
pub fn is_owner(&self) -> bool { self.current_identity_pubkey == self.owner_pubkey }
```

### 3. Core — Script Mutation Enforcement

**File:** `krillnotes-core/src/core/workspace/scripts.rs`

Add a guard at the top of each mutation method:

- `create_user_script()` — `if !self.is_owner() { return Err(KrillnotesError::NotOwner); }`
- `update_user_script()` — same
- `delete_user_script()` — same
- `toggle_user_script()` — same (this logs an `UpdateUserScript` op that propagates via sync)
- `reorder_user_script()` — same (this also logs an `UpdateUserScript` op)
- `reorder_all_user_scripts()` — same (this does NOT log ops, but is still a local mutation that should be owner-only)

### 4. Core — Sync Ingest Enforcement

**File:** `krillnotes-core/src/core/workspace/sync.rs` (note: this is `workspace/sync.rs`, not `swarm/sync.rs`)

In `apply_incoming_operation()`, the three script match arms gain a signer check. The check goes inside the `match &op` block (within the second transaction that applies working-table changes). If the signer doesn't match, the SQL statements for that arm are simply skipped — no early return or explicit commit needed, since the transaction commits at the end for all branches:

```rust
Operation::CreateUserScript { created_by, script_id, name, description, source_code, load_order, enabled, .. } => {
    if created_by == &self.owner_pubkey {
        let now_ms = ts.wall_ms as i64;
        tx.execute(/* ... existing INSERT ... */)?;
    }
    // else: unauthorized — skip working table mutation, op stays in log for audit
}
```

Same pattern for `UpdateUserScript` (check `modified_by`) and `DeleteUserScript` (check `deleted_by`).

The operation is still recorded in the operations log (it passed the `INSERT OR IGNORE` in the first transaction) but the working table mutation is not applied. This preserves the full operation history for audit while preventing unauthorized state changes.

### 5. SwarmHeader — `owner_pubkey` Field

**File:** `krillnotes-core/src/core/swarm/header.rs`

Add to `SwarmHeader`:
```rust
/// Ed25519 public key of the workspace owner (base64). Present in all new bundles.
pub owner_pubkey: Option<String>,
```

`Option<String>` for backward compatibility with bundles created before this change.

### 6. Swarm Bundle Generation — Set `owner_pubkey`

**Invite** (`swarm/invite.rs`): Set `header.owner_pubkey = Some(inviter's pubkey)`. (In the current design, workspace owners are the ones who generate invites. This spec does not add enforcement preventing non-owners from creating invites — that is a separate concern for a future RBAC pass.)

**Accept** (`swarm/invite.rs`): Echo `header.owner_pubkey` from the original invite's parsed `owner_pubkey`.

**Snapshot** (`swarm/sync.rs`): Set `header.owner_pubkey = Some(workspace.owner_pubkey().to_string())`.

**Delta** (`swarm/delta.rs`): Set `header.owner_pubkey = Some(workspace.owner_pubkey().to_string())`.

### 7. Swarm Bundle Validation — Cross-Check on Receive

**File:** `swarm/sync.rs` (where `apply_delta` and snapshot application happen)

On receiving any bundle with `owner_pubkey` set in the header:
- If the local workspace also has `owner_pubkey`, compare them. **Mismatch → reject the entire bundle** with `KrillnotesError::Swarm("owner_pubkey mismatch")`.
- If the local workspace has no `owner_pubkey` (pre-existing workspace that hasn't been opened since upgrade): accept the header's value and store it.

### 8. Tauri Command

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

```rust
#[tauri::command]
pub async fn is_workspace_owner(window: Window, state: State<'_, AppState>) -> Result<bool, String> {
    // Look up workspace by window label, return workspace.is_owner()
}
```

Register in `tauri::generate_handler![...]`.

### 9. Frontend — PeerInfo Type

**File:** `krillnotes-desktop/src/types.ts`

```typescript
export interface PeerInfo {
  peerDeviceId: string;
  peerIdentityId: string;
  displayName: string;
  fingerprint: string;
  trustLevel?: string;
  contactId?: string;
  lastSync?: string;
  isOwner?: boolean;  // NEW — true if this peer is the workspace owner
}
```

### 10. Frontend — Owner Badge in Peers Dialog

**File:** `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

Next to the existing trust badge, render an amber "Owner" pill when `peer.isOwner`:

```tsx
{peer.isOwner && (
  <span className="text-xs px-1.5 py-0.5 rounded-full font-medium bg-amber-500/20 text-amber-400">
    Owner
  </span>
)}
```

### 11. Frontend — Script Manager Lockdown

**File:** `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`

On mount, call `invoke<boolean>('is_workspace_owner')` and store the result.

When `!isOwner`:
- **Disable** Save, Replace, Delete, New Script, Undo, Redo buttons (add `disabled` prop)
- **Disable** the enable/disable toggle (checkbox) for each script
- **Disable** any reorder controls (drag handles, up/down buttons)
- **Show info banner** at the top of the dialog: *"Only the workspace owner can modify scripts."* Styled as a muted info box (not an error).
- Script list and code editor remain **readable** — non-owners can view and browse scripts.

### 12. Rust `PeerInfo` Struct

**File:** `krillnotes-core/src/core/peer_registry.rs` (where `PeerInfo` is defined)

Add `is_owner: bool` to the `PeerInfo` struct. Computed in `list_peers_info()` (in `workspace/sync.rs`) inside the `into_iter().map(|peer| { ... })` closure:
```rust
is_owner: peer.peer_identity_id == self.owner_pubkey(),
```

With `#[serde(rename_all = "camelCase")]` this serializes as `isOwner` for the frontend.

## Testing Strategy

- **Unit tests in `krillnotes-core`:**
  - Create workspace → verify `owner_pubkey()` matches creator's pubkey
  - `is_owner()` returns `true` for creator, `false` for a different identity
  - `create_user_script` succeeds for owner, returns `NotOwner` for non-owner
  - `toggle_user_script` returns `NotOwner` for non-owner
  - `reorder_user_script` returns `NotOwner` for non-owner
  - `reorder_all_user_scripts` returns `NotOwner` for non-owner
  - `apply_incoming_operation` with `CreateUserScript` from owner → applied (returns `Ok(true)`)
  - `apply_incoming_operation` with `CreateUserScript` from non-owner → skipped (returns `Ok(true)` — logged but not applied to working tables)
  - SwarmHeader round-trip with `owner_pubkey` field present
  - SwarmHeader round-trip with `owner_pubkey` field absent (backward compat)
  - Open pre-existing workspace without `owner_pubkey` → verify opener becomes owner

- **Integration (manual):**
  - Open workspace as owner → Script Manager fully functional
  - Open workspace as non-owner → Save/Delete/New/Toggle/Reorder buttons disabled, banner shown
  - Peers dialog shows "Owner" badge on the correct peer

## Out of Scope

- Ownership transfer
- Multi-owner / co-owner model
- RBAC enforcement for note operations (future WP-C)
- Permission materialization from `SetPermission` / `RevokePermission` ops
- Non-owner invite prevention (future RBAC pass)
