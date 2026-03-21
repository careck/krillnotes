# RBAC Gate Wiring — Design Specification

**Date:** 2026-03-21
**Companion to:** Permission Gate Spec v1.0, RBAC Implementation Plan (2026-03-21)

This spec covers the three items deferred from the initial RBAC implementation: protocol headers on `.swarm` bundles, the `ProtocolMismatch` error, and wiring `authorize()` into every mutating workspace method. It also makes the permission gate non-optional and introduces `AllowAllGate` for tests and fallback builds.

---

## 1. Problem Statement

The RBAC PR shipped the `PermissionGate` trait, the `RbacGate` implementation, and `authorize()` calls on 14 mutating methods. Three gaps remain:

1. **No protocol header on `.swarm` bundles.** The Permission Gate Spec §7 requires a `"protocol"` field in every bundle header, and §4.4 requires rejecting bundles whose protocol doesn't match the local gate. Without this, a Krillnotes client could silently process an OPswarm ACL bundle (or vice versa), discarding permission data it doesn't understand.

2. **`permission_gate` is `Option`.** The initial PR made the gate optional to avoid touching 206 test call sites. This means it's possible to construct a workspace with no permission enforcement — a gap that will widen as more code is written.

3. **~47 mutating methods lack `authorize()` calls.** Undo, attachments, and several note/script methods bypass the gate entirely.

---

## 2. Scope

### In scope

- `AllowAllGate` with configurable protocol ID in `krillnotes-core`
- `permission_gate` becomes `Box<dyn PermissionGate>` (non-optional)
- `protocol` field on `SwarmHeader` — set on export, validated before ingest
- `ProtocolMismatch` error variant on `KrillnotesError`
- Wire `authorize()` into all remaining mutating workspace methods
- Update all core tests to pass `AllowAllGate`
- Update desktop `#[cfg(not(feature = "rbac"))]` fallback to use `AllowAllGate`
- Fix hardcoded `None` gate sites in `export.rs` and `swarm.rs` commands

### Out of scope

- Contested operation states (three-state model from RBAC Spec §5.1) — deferred to multi-peer work
- Permission-granting UI
- Frontend-specific error handling for `ProtocolMismatch`

---

## 3. `AllowAllGate`

Lives in `krillnotes-core/src/core/permission.rs` alongside the `PermissionGate` trait.

```rust
/// A no-op permission gate that permits all operations.
///
/// Used as the fallback when no gate feature (e.g. `rbac`) is enabled,
/// and in core tests that don't exercise permission logic.
///
/// The `protocol_id` is configurable so it remains decoupled from any
/// specific permission model (RBAC, ACL, etc.).
pub struct AllowAllGate {
    protocol: &'static str,
}

impl AllowAllGate {
    pub fn new(protocol_id: &'static str) -> Self {
        Self { protocol: protocol_id }
    }
}

impl PermissionGate for AllowAllGate {
    fn protocol_id(&self) -> &'static str { self.protocol }

    fn authorize(
        &self, _conn: &Connection, _actor: &str, _operation: &Operation,
    ) -> Result<(), PermissionError> {
        Ok(())
    }

    fn apply_permission_op(
        &self, _conn: &Connection, _operation: &Operation,
    ) -> Result<(), PermissionError> {
        Ok(())
    }

    fn ensure_schema(&self, _conn: &Connection) -> Result<(), PermissionError> {
        Ok(())
    }
}
```

---

## 4. Non-Optional Permission Gate

### 4.1 Workspace struct change

```rust
// Before
permission_gate: Option<Box<dyn PermissionGate>>,

// After
permission_gate: Box<dyn PermissionGate>,
```

### 4.2 Constructor signatures

`Workspace::create()`, `Workspace::open()`, and all `create_*` variants change their parameter from `Option<Box<dyn PermissionGate>>` to `Box<dyn PermissionGate>`.

### 4.3 Authorize calls simplify

```rust
// Before
if let Some(gate) = &self.permission_gate {
    gate.authorize(&self.conn(), actor, &operation)?;
}

// After
self.permission_gate.authorize(&self.conn(), actor, &operation)?;
```

### 4.4 `apply_permission_op` calls simplify similarly

The existing `apply_permission_op_via` helper method (if present) or inline calls are updated to remove the `Option` unwrapping.

### 4.5 Test updates

All 206 test functions across 5 test files pass `Box::new(AllowAllGate::new("test"))` instead of `None`.

A helper function in each test module keeps this concise:

```rust
fn test_gate() -> Box<dyn PermissionGate> {
    Box::new(AllowAllGate::new("test"))
}
```

### 4.6 Desktop feature flag

The feature flag stays — it controls which gate implementation is linked.

```rust
#[cfg(feature = "rbac")]
fn create_permission_gate(owner_pubkey: String) -> Box<dyn krillnotes_core::PermissionGate> {
    Box::new(krillnotes_rbac::RbacGate::new(owner_pubkey))
}

#[cfg(not(feature = "rbac"))]
fn create_permission_gate(_owner_pubkey: String) -> Box<dyn krillnotes_core::PermissionGate> {
    Box::new(krillnotes_core::AllowAllGate::new("krillnotes/1"))
}
```

### 4.7 Export and swarm command call sites

- `export.rs` (`import_workspace`): Pass `AllowAllGate::new("krillnotes/1")` (imported workspaces have no gate context; AllowAll is correct since the data is already trusted).
- `swarm.rs` (`apply_swarm_bundle` / `create_empty_with_id`): Pass the real gate derived from the owner pubkey, same as other desktop call sites.

---

## 5. Protocol Header on `.swarm` Bundles

### 5.1 SwarmHeader change

Add a required `protocol` field to `SwarmHeader`:

```rust
pub struct SwarmHeader {
    pub protocol: String,    // e.g. "krillnotes/1", "opswarm/1"
    // ... all existing fields unchanged ...
}
```

### 5.2 Outbound bundles (export)

When building a swarm bundle header, `protocol` is set from the workspace's gate:

```rust
header.protocol = self.permission_gate.protocol_id().to_string();
```

This applies to all bundle types: `Delta`, `Snapshot`, `Invite`, `Accept`.

### 5.3 Inbound bundles (ingest)

Before decryption, before signature verification, before any operation processing:

```rust
if bundle.header.protocol != self.permission_gate.protocol_id() {
    log::error!(
        "Rejecting swarm bundle: protocol mismatch (expected '{}', found '{}')",
        self.permission_gate.protocol_id(),
        bundle.header.protocol,
    );
    return Err(KrillnotesError::ProtocolMismatch {
        expected: self.permission_gate.protocol_id().to_string(),
        found: bundle.header.protocol.clone(),
    });
}
```

### 5.4 Backward compatibility

Existing `.swarm` files lack the `protocol` field. Since only test swarms exist and backward compatibility is not required, the field is required (no `#[serde(default)]`). Old bundles fail deserialization with a clear serde error.

---

## 6. `ProtocolMismatch` Error Variant

Added to `KrillnotesError` in `error.rs`:

```rust
#[error("protocol mismatch: expected {expected}, found {found}")]
ProtocolMismatch { expected: String, found: String },
```

**Note:** The Permission Gate Spec v1.0 §8 uses `&'static str` for `expected`. This spec uses `String` instead to avoid lifetime constraints on the error enum (e.g. when storing or sending errors across threads). The Permission Gate Spec should be updated to match.

Added to `user_message()`:

```rust
Self::ProtocolMismatch { expected, found } =>
    format!("Incompatible swarm protocol: expected {}, found {}", expected, found),
```

---

## 7. Wiring `authorize()` Into All Mutating Methods

### 7.1 Actor identity

Two sources of actor identity, depending on operation origin:

| Origin | Actor | Source |
|--------|-------|--------|
| Local (UI) | `self.current_identity_pubkey` | The identity bound to this workspace |
| Swarm (inbound) | `operation.author_key()` | The peer who signed the operation |

### 7.2 Methods already wired (15)

**notes.rs (9):** `create_note`, `deep_copy_note`, `create_note_root`, `update_note_title`, `update_note_tags`, `move_note`, `delete_note_recursive`, `delete_note_promote`, `update_note`

**scripts.rs (5):** `create_user_script_with_category`, `update_user_script`, `delete_user_script`, `toggle_user_script`, `reorder_user_script`

**sync.rs (1):** `apply_incoming_operation` (uses `op.author_key()` as actor)

### 7.3 Methods to wire

Each mutating method is classified as **wire** (needs authorize), **skip** (not a permission-gated action), or **covered** (delegates to an already-wired method).

#### undo.rs — wire

| Method | Gate? | Notes |
|--------|-------|-------|
| `undo` | Wire | Actor: `current_identity_pubkey`. Authorize against the `RetractOperation` variant being applied. RBAC Spec §3.1: user can retract own ops, Owner can retract any in subtree. |
| `redo` | Wire | Same as undo — re-applies a retracted operation. |
| `script_undo` | Wire | Same pattern, script-scoped undo. |
| `script_redo` | Wire | Same pattern, script-scoped redo. |

#### attachments.rs — wire

| Method | Gate? | Notes |
|--------|-------|-------|
| `attach_file` | Wire | `AddAttachment` operation. |
| `attach_file_with_id` | Wire | Same, with explicit attachment ID. |
| `delete_attachment` | Wire | `RemoveAttachment` operation. |
| `restore_attachment` | Wire | Re-adds a deleted attachment. |
| `set_attachment_max_size_bytes` | Wire | Workspace-level config — Owner only. |

#### scripts.rs — wire

| Method | Gate? | Notes |
|--------|-------|-------|
| `reorder_all_user_scripts` | Wire | Workspace-level batch reorder — Owner only. |
| `purge_all_operations` | Wire | Destructive — deletes operation log entries. Owner only. |

#### mod.rs — wire

| Method | Gate? | Notes |
|--------|-------|-------|
| `set_owner_pubkey` | Wire | **Critical** — transfers workspace ownership. Maps to `TransferRootOwnership`. Root Owner only. |
| `set_workspace_metadata` | Wire | Workspace-level config mutation. Owner only. |

#### notes.rs — skip (not permission-gated)

| Method | Gate? | Notes |
|--------|-------|-------|
| `toggle_note_expansion` | Skip | UI-local state (tree expansion), not a data mutation. |
| `set_selected_note` | Skip | UI-local state, not persisted as an operation. |
| `rebuild_note_links_index` | Skip | Maintenance/reindex, not user-initiated mutation. |

#### scripts.rs — skip

| Method | Gate? | Notes |
|--------|-------|-------|
| `reload_all_scripts` | Skip | Re-evaluates Rhai scripts, no data mutation. |

#### sync.rs — skip (transport layer)

| Method | Gate? | Notes |
|--------|-------|-------|
| `add_contact_as_peer` | Skip | Peer registry — transport concern, not content permission. |
| `remove_peer` | Skip | Same. |
| `upsert_peer_from_delta` | Skip | Same. |
| `upsert_sync_peer` | Skip | Same. |
| `update_peer_channel` | Skip | Same. |
| `update_peer_sync_status` | Skip | Same. |
| `reset_peer_watermark` | Skip | Same. |
| `update_peer_last_sent_by_identity` | Skip | Same. |

#### hooks.rs — covered by delegation

| Method | Gate? | Notes |
|--------|-------|-------|
| `run_tree_action` | Covered | Delegates to `create_note`/`update_note` which already have authorize calls. No additional gate needed. |

### 7.4 Undo/redo authorization semantics

Undo and redo authorize against the `RetractOperation` variant. Per RBAC Spec §3.1:
- A user can retract their own operations.
- An Owner can retract any operation on notes within their subtree.

The gate receives the `RetractOperation` which references the original operation ID. The `RbacGate` implementation resolves the target note from the original operation and checks the actor's role on that note.

### 7.5 Pattern

Every mutating method follows the same pattern:

```rust
pub fn some_mutation(&mut self, ...) -> Result<...> {
    // Build the operation
    let operation = Operation::SomeVariant { ... };

    // Authorize
    self.permission_gate.authorize(
        self.storage.conn(),
        &self.current_identity_pubkey,
        &operation,
    )?;

    // Apply mutation + log
    // ...
}
```

For `apply_incoming_operation` (swarm ingest), the existing pattern is preserved — it uses `op.author_key()` instead of `self.current_identity_pubkey`.

---

## 8. Testing Strategy

### 8.1 Core tests (AllowAllGate)

All 206 existing tests pass `AllowAllGate::new("test")`. No behavioral change — these tests validate core logic, not permissions.

### 8.2 RBAC crate tests (RbacGate)

The `krillnotes-rbac` crate's existing tests exercise the real gate with tree-walk resolution, role-capped delegation, and cascade revocation. No changes needed.

### 8.3 New tests for this PR

- **Protocol check test:** Build a bundle with protocol `"wrong/1"`, attempt ingest, assert `ProtocolMismatch` error.
- **Root owner end-to-end:** Create a workspace with `RbacGate`, verify the root owner (current identity = owner) can perform all operations: create notes, move, delete, undo, add attachments, manage scripts.
- **Non-optional gate test:** Verify `AllowAllGate` passes all operations.

---

## 9. Migration and Breaking Changes

- **`.swarm` bundle format:** The new `protocol` field is required. Existing test bundles will fail to deserialize. No migration path — this is acceptable per user confirmation.
- **API change:** `Workspace::create/open` signatures change from `Option<Box<dyn PermissionGate>>` to `Box<dyn PermissionGate>`. All call sites updated in this PR.
- **Feature flag:** No change — `rbac` feature continues to control which gate is linked.
