# RBAC Gate Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the permission gate into every workspace mutation, make it non-optional, add the protocol header to `.swarm` bundles, and add the `ProtocolMismatch` error variant.

**Architecture:** `AllowAllGate` (configurable protocol ID) lives in `krillnotes-core` alongside the trait. `Workspace.permission_gate` becomes `Box<dyn PermissionGate>` — always installed. Every mutating method calls `authorize()` before applying. `.swarm` bundle headers carry a `protocol` field checked on ingest.

**Tech Stack:** Rust, rusqlite, ed25519-dalek, serde_json, thiserror, log

**Specs:**
- Design: `docs/plans/2026-03-21-rbac-gate-wiring-design.md`
- Behavioral: `docs/swarm/Swarm_RBAC_Spec_v2_0.md`
- Plugin architecture: `docs/swarm/Permission_Gate_Spec_v1_0.md`

---

## File Structure

### New files

None.

### Modified files

| File | Change |
|------|--------|
| `krillnotes-core/src/core/permission.rs` | Add `AllowAllGate` struct + impl |
| `krillnotes-core/src/lib.rs` | Re-export `AllowAllGate` |
| `krillnotes-core/src/core/error.rs` | Add `ProtocolMismatch` variant |
| `krillnotes-core/src/core/workspace/mod.rs` | `permission_gate` → `Box<dyn PermissionGate>`, update `authorize()` + `apply_permission_op_via()`, all constructors |
| `krillnotes-core/src/core/workspace/undo.rs` | Add `authorize()` calls to `undo`, `redo`, `script_undo`, `script_redo` |
| `krillnotes-core/src/core/workspace/attachments.rs` | Add `authorize()` calls to `attach_file`, `delete_attachment`, `restore_attachment`, `set_attachment_max_size_bytes` |
| `krillnotes-core/src/core/workspace/scripts.rs` | Add `authorize()` to `reorder_all_user_scripts`, `purge_all_operations` |
| `krillnotes-core/src/core/workspace/notes.rs` | Add `authorize()` to `set_workspace_metadata` |
| `krillnotes-core/src/core/swarm/header.rs` | Add `protocol` field to `SwarmHeader`, update `sample_header()` test helper |
| `krillnotes-core/src/core/swarm/delta.rs` | Add `protocol` to `DeltaParams` + header construction |
| `krillnotes-core/src/core/swarm/snapshot.rs` | Add `protocol` to `SnapshotParams` + header construction |
| `krillnotes-core/src/core/swarm/invite.rs` | Add `protocol` to `InviteParams`, `AcceptParams` + header constructions |
| `krillnotes-core/src/core/swarm/sync.rs` | Add protocol check before processing delta; add `protocol` to `DeltaParams` construction |
| `krillnotes-core/src/core/swarm/mod.rs` (tests) | Add `protocol` to all Params construction sites in integration tests |
| `krillnotes-core/src/core/export.rs` | Pass `AllowAllGate` instead of `None` |
| `krillnotes-core/src/core/workspace/tests.rs` | Replace all `None` with `test_gate()` |
| `krillnotes-core/src/core/export_tests.rs` | Replace all `None` with `test_gate()` |
| `krillnotes-core/tests/relay_integration.rs` | Replace all `None` with `test_gate()` |
| `krillnotes-core/tests/watermark_recovery.rs` | Replace all `None` with `test_gate()` |
| `krillnotes-core/src/core/swarm/sync.rs` (tests) | Replace all `None` with `test_gate()` |
| `krillnotes-desktop/src-tauri/src/commands/workspace.rs` | Return `Box<dyn PermissionGate>` instead of `Option`, update `create_permission_gate` |
| `krillnotes-desktop/src-tauri/src/commands/swarm.rs` | Pass real gate instead of `None` at line 433; add `protocol` to `SnapshotParams` constructions |
| `krillnotes-desktop/src-tauri/src/commands/sync.rs` | Add `protocol` to `AcceptParams` construction at line 626 |

---

## Task 1: `AllowAllGate` + `ProtocolMismatch` Error

**Files:**
- Modify: `krillnotes-core/src/core/permission.rs`
- Modify: `krillnotes-core/src/lib.rs`
- Modify: `krillnotes-core/src/core/error.rs`

- [ ] **Step 1: Add `AllowAllGate` to permission.rs**

Append after the `PermissionError` enum (after line 79 of `permission.rs`):

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
    fn protocol_id(&self) -> &'static str {
        self.protocol
    }

    fn authorize(
        &self,
        _conn: &Connection,
        _actor: &str,
        _operation: &Operation,
    ) -> Result<(), PermissionError> {
        Ok(())
    }

    fn apply_permission_op(
        &self,
        _conn: &Connection,
        _operation: &Operation,
    ) -> Result<(), PermissionError> {
        Ok(())
    }

    fn ensure_schema(&self, _conn: &Connection) -> Result<(), PermissionError> {
        Ok(())
    }
}
```

- [ ] **Step 2: Re-export `AllowAllGate` from lib.rs**

In `krillnotes-core/src/lib.rs`, change the permission re-export (line 40) from:
```rust
    permission::{PermissionError, PermissionGate},
```
to:
```rust
    permission::{AllowAllGate, PermissionError, PermissionGate},
```

- [ ] **Step 3: Add `ProtocolMismatch` variant to `KrillnotesError`**

In `krillnotes-core/src/core/error.rs`, add after the `Permission` variant (after line 137):
```rust
    #[error("protocol mismatch: expected {expected}, found {found}")]
    ProtocolMismatch { expected: String, found: String },
```

- [ ] **Step 4: Add `ProtocolMismatch` case to `user_message()`**

In the `user_message()` method in `error.rs`, add the case:
```rust
    Self::ProtocolMismatch { expected, found } =>
        format!("Incompatible swarm protocol: expected {}, found {}", expected, found),
```

- [ ] **Step 5: Update module doc comment in permission.rs**

Change the module doc (line 10-12) from mentioning "optional gate" to reflect that the gate is always installed:
```rust
//! The [`PermissionGate`] trait defines the interface that permission backends
//! (such as the RBAC crate) must implement. The workspace holds a gate;
//! every mutating operation is checked via [`PermissionGate::authorize`]
//! before being applied. [`AllowAllGate`] is used for tests and fallback builds.
```

- [ ] **Step 6: Verify core crate compiles**

Run: `cargo check -p krillnotes-core`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/permission.rs krillnotes-core/src/lib.rs krillnotes-core/src/core/error.rs
git commit -m "feat(core): add AllowAllGate and ProtocolMismatch error variant"
```

---

## Task 2: Make `permission_gate` Non-Optional on Workspace

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Change the field type**

In `mod.rs` line 115-118, change:
```rust
    /// Optional pluggable permission gate (e.g. RBAC).
    /// When `Some`, every mutating operation is checked via `authorize()` before being applied.
    /// When `None` (single-user / test mode), all operations are permitted.
    permission_gate: Option<Box<dyn crate::core::permission::PermissionGate>>,
```
to:
```rust
    /// Pluggable permission gate (e.g. RBAC).
    /// Every mutating operation is checked via `authorize()` before being applied.
    /// Use `AllowAllGate` for tests or builds without a specific gate feature.
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
```

- [ ] **Step 2: Update all constructor signatures**

Change the `permission_gate` parameter type in all 5 constructors from `Option<Box<dyn crate::core::permission::PermissionGate>>` to `Box<dyn crate::core::permission::PermissionGate>`:

- `create` (line 130)
- `create_with_id` (lines 340-347)
- `create_empty` (line 559)
- `create_empty_with_id` (lines 705-712)
- `open` (line 818)

Use `replace_all` where possible — the old type string `permission_gate: Option<Box<dyn crate::core::permission::PermissionGate>>` should be replaced with `permission_gate: Box<dyn crate::core::permission::PermissionGate>` throughout `mod.rs`.

- [ ] **Step 3: Update constructor bodies**

In each constructor body, where `permission_gate` is stored on the struct, remove any `Some()` wrapping. The field assignment should be just:
```rust
permission_gate,
```
or wherever it's assigned from a local variable.

Also update the `ensure_schema` call site. Change any:
```rust
if let Some(gate) = &self.permission_gate {
    gate.ensure_schema(...)?;
}
```
to:
```rust
self.permission_gate.ensure_schema(...)?;
```

Search for all `if let Some(gate)` or `if let Some(ref gate)` patterns in `mod.rs` and simplify them.

- [ ] **Step 4: Simplify `authorize()` helper** (line 1099-1108)

Change:
```rust
    fn authorize(&self, operation: &Operation) -> Result<()> {
        if let Some(gate) = &self.permission_gate {
            gate.authorize(
                self.storage.connection(),
                &self.current_identity_pubkey,
                operation,
            )?;
        }
        Ok(())
    }
```
to:
```rust
    fn authorize(&self, operation: &Operation) -> Result<()> {
        self.permission_gate.authorize(
            self.storage.connection(),
            &self.current_identity_pubkey,
            operation,
        )?;
        Ok(())
    }
```

- [ ] **Step 5: Simplify `apply_permission_op_via()` helper** (line 1114-1123)

Change:
```rust
    fn apply_permission_op_via(
        gate: &Option<Box<dyn crate::core::permission::PermissionGate>>,
        conn: &Connection,
        operation: &Operation,
    ) -> Result<()> {
        if let Some(gate) = gate {
            gate.apply_permission_op(conn, operation)?;
        }
        Ok(())
    }
```
to:
```rust
    fn apply_permission_op_via(
        gate: &dyn crate::core::permission::PermissionGate,
        conn: &Connection,
        operation: &Operation,
    ) -> Result<()> {
        gate.apply_permission_op(conn, operation)?;
        Ok(())
    }
```

- [ ] **Step 6: Update all call sites of `apply_permission_op_via`**

Search `mod.rs`, `notes.rs`, `scripts.rs`, `sync.rs` for calls to `apply_permission_op_via`. Change from:
```rust
Self::apply_permission_op_via(&self.permission_gate, &tx, &op)?;
```
to:
```rust
Self::apply_permission_op_via(&*self.permission_gate, &tx, &op)?;
```

- [ ] **Step 7: Add `protocol_id()` public accessor**

Add a public method to `Workspace` in `mod.rs` so that callers (sync.rs, desktop commands) can read the gate's protocol ID without accessing the private field:

```rust
    /// Returns the protocol identifier from the installed permission gate.
    /// Used to stamp outbound .swarm bundle headers and validate inbound ones.
    pub fn protocol_id(&self) -> &str {
        self.permission_gate.protocol_id()
    }
```

- [ ] **Step 8: Check for any remaining `Option` references**

Search `mod.rs` for any remaining references to `Option<Box<dyn` related to permission_gate and fix them.

- [ ] **Step 9: Verify it compiles (tests will fail — that's expected)**

Run: `cargo check -p krillnotes-core`
Expected: compiles, but tests will fail because they still pass `None`

- [ ] **Step 10: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor(core): make permission_gate non-optional on Workspace"
```

---

## Task 3: Update All Tests to Use `AllowAllGate`

**Files:**
- Modify: `krillnotes-core/src/core/workspace/tests.rs`
- Modify: `krillnotes-core/src/core/export_tests.rs`
- Modify: `krillnotes-core/tests/relay_integration.rs`
- Modify: `krillnotes-core/tests/watermark_recovery.rs`
- Modify: `krillnotes-core/src/core/swarm/sync.rs` (test section)

This is a mechanical replacement: every `None` passed as the `permission_gate` argument becomes `test_gate()`.

- [ ] **Step 1: Add test helper to `workspace/tests.rs`**

At the top of the test module (after the imports), add:
```rust
use crate::core::permission::{AllowAllGate, PermissionGate};

fn test_gate() -> Box<dyn PermissionGate> {
    Box::new(AllowAllGate::new("test"))
}
```

- [ ] **Step 2: Replace all `None` permission_gate args in `workspace/tests.rs`**

Use find-and-replace across the file. The pattern to match is the final `None` argument in `Workspace::create(...)`, `Workspace::open(...)`, `Workspace::create_with_id(...)`, `Workspace::create_empty(...)`, `Workspace::create_empty_with_id(...)` calls.

These appear as `, None)` at the end of constructor calls. Replace each with `, test_gate())`.

**Important:** Be careful not to replace `None` that is NOT a permission_gate argument (e.g. `parent_id: None`). Only replace the final `None)` in Workspace constructor calls. Verify by checking the parameter position — `permission_gate` is always the **last** parameter.

- [ ] **Step 3: Run workspace tests**

Run: `cargo test -p krillnotes-core workspace::tests`
Expected: all pass

- [ ] **Step 4: Add test helper and replace in `export_tests.rs`**

Add the same `test_gate()` helper and `use` imports at the top of the test module. Replace all `None` permission_gate args. The pattern is the same — `None` as the last arg to `Workspace::open(...)`.

- [ ] **Step 5: Run export tests**

Run: `cargo test -p krillnotes-core export_tests`
Expected: all pass

- [ ] **Step 6: Add test helper and replace in `tests/relay_integration.rs`**

Add at the top:
```rust
use krillnotes_core::core::permission::{AllowAllGate, PermissionGate};

fn test_gate() -> Box<dyn PermissionGate> {
    Box::new(AllowAllGate::new("test"))
}
```
Replace all `None` permission_gate args.

- [ ] **Step 7: Add test helper and replace in `tests/watermark_recovery.rs`**

Same pattern as step 6.

- [ ] **Step 8: Add test helper and replace in `swarm/sync.rs` tests**

Add the helper inside the `#[cfg(test)]` module. Replace all `None` permission_gate args in the test functions.

- [ ] **Step 9: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

- [ ] **Step 10: Commit**

```bash
git add krillnotes-core/src/core/workspace/tests.rs krillnotes-core/src/core/export_tests.rs krillnotes-core/tests/relay_integration.rs krillnotes-core/tests/watermark_recovery.rs krillnotes-core/src/core/swarm/sync.rs
git commit -m "test(core): replace None permission_gate with AllowAllGate in all tests"
```

---

## Task 4: Wire `authorize()` Into Remaining Mutating Methods

**Files:**
- Modify: `krillnotes-core/src/core/workspace/undo.rs`
- Modify: `krillnotes-core/src/core/workspace/attachments.rs`
- Modify: `krillnotes-core/src/core/workspace/scripts.rs`
- Modify: `krillnotes-core/src/core/workspace/notes.rs`
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

For each method, the `authorize()` call goes **before** any DB writes. These methods already build an `Operation` for logging — the authorize call uses that same operation. If the method doesn't build an Operation (e.g. `set_owner_pubkey`), check what it does and build the appropriate one.

**Important — undo/redo restructuring:** The undo methods currently construct the `RetractOperation` **after** the DB mutation. The authorize call must happen before the mutation, so you need to construct the `RetractOperation` earlier in the method body, authorize it, then proceed with the mutation. This requires restructuring — do not just insert `self.authorize()` at the current operation-construction point.

- [ ] **Step 1: Wire undo.rs — `undo()` (line 180) and `redo()` (line 237)**

For `undo()`: construct the `RetractOperation` before line 194 (`self.inside_undo = true`), call `self.authorize(&retract_op)?;`, then proceed with the existing mutation logic.

Same restructuring for `redo()`, `script_undo()` (line 36), and `script_redo()` (line 56).

- [ ] **Step 2: Wire attachments.rs — `attach_file()` (line 18)**

Find where the `AddAttachment` operation is built, add before DB write:
```rust
self.authorize(&operation)?;
```

Same pattern for:
- `delete_attachment()` (line 266)
- `restore_attachment()` (line 290)
- `set_attachment_max_size_bytes()` (line 344) — this may not build an Operation currently. If it doesn't, it should be gated by checking `is_owner()` or building a workspace-level operation.

**Skip `attach_file_with_id()` (line 76)** — this is an import-only internal method (doc comment: "Import-only: attach a file with a pre-specified ID"). It's called during snapshot import where `AllowAllGate` is used and the data is already trusted. No authorize gate needed.

- [ ] **Step 3: Wire scripts.rs — `reorder_all_user_scripts()` (line 460)**

This method already has an owner check (from the owner-only-scripts PR). Verify it exists. If not, add:
```rust
self.authorize(&operation)?;
```
If it doesn't build an Operation, guard with the existing `is_owner()` check pattern.

- [ ] **Step 4: Wire scripts.rs — `purge_all_operations()` (line 497)**

This is a destructive workspace-level action. Same pattern — either authorize against an appropriate operation or guard with `is_owner()`.

- [ ] **Step 5: Wire notes.rs — `set_workspace_metadata()` (line 916)**

This is a workspace-level config mutation. Add an authorize call or `is_owner()` guard. Check whether it builds an Operation — if not, guard with `is_owner()`.

- [ ] **Step 6: Verify mod.rs — `set_owner_pubkey()` (line 1021)**

This is the most sensitive mutation. Verify it is only called internally (from snapshot import, not exposed via Tauri command). If internal-only, add a comment documenting this:
```rust
// Internal only — called during snapshot import. Not exposed via Tauri.
// Authorization is handled by the caller's context (AllowAllGate during import).
```
If it's exposed via Tauri command, it must be gated with authorize.

- [ ] **Step 7: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass (AllowAllGate permits everything)

- [ ] **Step 8: Commit**

```bash
git add krillnotes-core/src/core/workspace/undo.rs krillnotes-core/src/core/workspace/attachments.rs krillnotes-core/src/core/workspace/scripts.rs krillnotes-core/src/core/workspace/notes.rs krillnotes-core/src/core/workspace/mod.rs
git commit -m "feat(core): wire authorize() into undo, attachments, scripts, and notes methods"
```

---

## Task 5: Add `protocol` Field to SwarmHeader

**Files:**
- Modify: `krillnotes-core/src/core/swarm/header.rs`
- Modify: `krillnotes-core/src/core/swarm/delta.rs`
- Modify: `krillnotes-core/src/core/swarm/snapshot.rs`
- Modify: `krillnotes-core/src/core/swarm/invite.rs`

- [ ] **Step 1: Add `protocol` field to `SwarmHeader`**

In `header.rs` line 39, add as the first field after the opening brace:
```rust
pub struct SwarmHeader {
    /// Protocol discriminator: "krillnotes/1" for RBAC, "opswarm/1" for ACL.
    /// Must match the receiving workspace's gate. Checked before decryption.
    pub protocol: String,
    pub format_version: u32,
    // ... rest unchanged ...
}
```

- [ ] **Step 2: Update `sample_header()` test helper**

In `header.rs` line 153-177, add the `protocol` field:
```rust
fn sample_header(mode: SwarmMode) -> SwarmHeader {
    SwarmHeader {
        protocol: "test".to_string(),
        format_version: 1,
        // ... rest unchanged ...
    }
}
```

- [ ] **Step 3: Add `protocol` to `DeltaParams` and header construction**

In `delta.rs`, add to `DeltaParams` struct (line 25):
```rust
pub struct DeltaParams<'a> {
    pub protocol: String,
    // ... existing fields ...
}
```

In the header construction (line 68), add:
```rust
let header = SwarmHeader {
    protocol: params.protocol,
    format_version: 1,
    // ... rest unchanged ...
};
```

- [ ] **Step 4: Add `protocol` to `SnapshotParams` and header construction**

In `snapshot.rs`, add to `SnapshotParams` struct (line 23):
```rust
pub struct SnapshotParams<'a> {
    pub protocol: String,
    // ... existing fields ...
}
```

In the header construction (line 82), add `protocol: params.protocol,`.

- [ ] **Step 5: Add `protocol` to `InviteParams` and both header constructions**

In `invite.rs`, add to `InviteParams` struct (line 52):
```rust
pub struct InviteParams<'a> {
    pub protocol: String,
    // ... existing fields ...
}
```

In the invite header construction (line 92), add `protocol: params.protocol.clone(),`.

Add to `AcceptParams` (around line 210):
```rust
pub protocol: String,
```

In the accept header construction (line 230), add `protocol: params.protocol,`.

- [ ] **Step 6: Update all production call sites that build `*Params` structs**

Complete enumeration of production call sites:

| File | Line | Struct | Protocol source |
|------|------|--------|-----------------|
| `krillnotes-core/src/core/swarm/sync.rs` | 107 | `DeltaParams` | `workspace.protocol_id().to_string()` |
| `krillnotes-desktop/src-tauri/src/commands/swarm.rs` | ~330 | `SnapshotParams` | get from workspace via `protocol_id()` accessor |
| `krillnotes-desktop/src-tauri/src/commands/swarm.rs` | ~654 | `SnapshotParams` | same |
| `krillnotes-desktop/src-tauri/src/commands/sync.rs` | ~626 | `AcceptParams` | same |

Note: `sync.rs`'s `generate_delta()` is a free function taking `workspace: &mut Workspace`, so use `workspace.protocol_id().to_string()` (uses the accessor added in Task 2 Step 7).

- [ ] **Step 7: Fix all test call sites in swarm modules**

Complete enumeration of test call sites:

| File | Lines | Struct |
|------|-------|--------|
| `krillnotes-core/src/core/swarm/mod.rs` | 49, 69, 91, 113 | `InviteParams`, `AcceptParams`, `SnapshotParams`, `DeltaParams` |
| `krillnotes-core/src/core/swarm/delta.rs` | 201, 228 | `DeltaParams` |
| `krillnotes-core/src/core/swarm/snapshot.rs` | 227, 254, 279, 305, 332 | `SnapshotParams` |
| `krillnotes-core/src/core/swarm/invite.rs` | 354, 376, 410 | `InviteParams` |

Add `protocol: "test".to_string(),` to each.

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p krillnotes-core`
Expected: compiles

- [ ] **Step 9: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: all pass

- [ ] **Step 10: Commit**

```bash
git add krillnotes-core/src/core/swarm/
git commit -m "feat(swarm): add protocol field to SwarmHeader and all bundle params"
```

---

## Task 6: Add Protocol Check on Bundle Ingest

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs`
- Modify: `krillnotes-core/src/core/swarm/delta.rs`
- Modify: `krillnotes-core/src/core/swarm/snapshot.rs`

**Approach:** The design spec says the check must happen "before decryption, before signature verification." Use `read_header()` from `header.rs` (line 119) to parse just the header from the raw bundle bytes, check the protocol field, then proceed to `parse_delta_bundle()` / `parse_snapshot_bundle()` which handle decryption and signature verification. This avoids wasting resources on rejected bundles.

- [ ] **Step 1: Write a failing test for protocol mismatch on delta ingest**

In the test section of `sync.rs`, add:
```rust
#[test]
fn test_protocol_mismatch_rejects_delta() {
    // Build a delta bundle with protocol "wrong/1".
    // Create a workspace with AllowAllGate::new("test").
    // Try to apply the delta.
    // Assert ProtocolMismatch error.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_protocol_mismatch`
Expected: FAIL (no protocol check exists yet)

- [ ] **Step 3: Add protocol check to delta ingest path**

In `sync.rs`, before the `parse_delta_bundle()` call (around line 146), add:

```rust
// Protocol isolation — reject bundles from incompatible products before decryption.
let header = crate::core::swarm::header::read_header(bundle_bytes)?;
if header.protocol != workspace.protocol_id() {
    log::error!(
        "Rejecting swarm bundle: protocol mismatch (expected '{}', found '{}')",
        workspace.protocol_id(),
        header.protocol,
    );
    return Err(KrillnotesError::ProtocolMismatch {
        expected: workspace.protocol_id().to_string(),
        found: header.protocol,
    });
}
```

- [ ] **Step 4: Add protocol check to snapshot ingest path**

The snapshot apply path (in desktop `swarm.rs` `apply_swarm_snapshot` around line 396) also needs a protocol check. Add the same pattern there using `read_header()` before `parse_snapshot_bundle()`.

Alternatively, if the desktop swarm commands already call `peek_swarm_header()` before applying, add the check there at the command level — before dispatching to snapshot or delta apply functions.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_protocol_mismatch`
Expected: PASS

- [ ] **Step 6: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/swarm/
git commit -m "feat(swarm): add protocol mismatch check on bundle ingest"
```

---

## Task 7: Update Desktop Crate

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/workspace.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/swarm.rs`
- Modify: `krillnotes-core/src/core/export.rs`

- [ ] **Step 1: Update `create_permission_gate` return type**

In `workspace.rs` lines 14-22, change:
```rust
#[cfg(feature = "rbac")]
fn create_permission_gate(owner_pubkey: String) -> Option<Box<dyn krillnotes_core::PermissionGate>> {
    Some(Box::new(krillnotes_rbac::RbacGate::new(owner_pubkey)))
}

#[cfg(not(feature = "rbac"))]
fn create_permission_gate(_owner_pubkey: String) -> Option<Box<dyn krillnotes_core::PermissionGate>> {
    None
}
```
to:
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

- [ ] **Step 2: Fix swarm.rs `create_empty_with_id` call** (line 427-434)

Change from:
```rust
    let mut ws = Workspace::create_empty_with_id(
        &db_path,
        &workspace_password,
        &identity_uuid,
        Ed25519SigningKey::from_bytes(&import_seed),
        &parsed.workspace_id,
        None,
    )
```
to pass the real gate. Derive `owner_pubkey` from the signing key (same pattern as other call sites in `workspace.rs`):
```rust
    let owner_pubkey = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .encode(Ed25519SigningKey::from_bytes(&import_seed).verifying_key().as_bytes())
    };
    let mut ws = Workspace::create_empty_with_id(
        &db_path,
        &workspace_password,
        &identity_uuid,
        Ed25519SigningKey::from_bytes(&import_seed),
        &parsed.workspace_id,
        create_permission_gate(owner_pubkey),
    )
```

`create_permission_gate` is a private function in `workspace.rs`. Make it `pub(crate)` so `swarm.rs` and `sync.rs` can use it:
```rust
#[cfg(feature = "rbac")]
pub(crate) fn create_permission_gate(...) -> Box<dyn krillnotes_core::PermissionGate> {
```

- [ ] **Step 3: Fix export.rs `Workspace::open` call** (line 509)

In `krillnotes-core/src/core/export.rs`, change:
```rust
    let mut workspace = Workspace::open(db_path, workspace_password, identity_uuid, signing_key, None)
```
to:
```rust
    let mut workspace = Workspace::open(
        db_path,
        workspace_password,
        identity_uuid,
        signing_key,
        Box::new(crate::core::permission::AllowAllGate::new("krillnotes/1")),
    )
```

Import workspaces are local archives, not swarm bundles — `AllowAllGate` is correct since the data is trusted.

- [ ] **Step 4: Update Params struct construction in desktop commands**

Add `protocol` field to all `*Params` constructions in the desktop crate. Use `ws.protocol_id().to_string()` (the accessor added in Task 2 Step 7):

| File | Approx line | Struct |
|------|-------------|--------|
| `commands/swarm.rs` | ~330 | `SnapshotParams` |
| `commands/swarm.rs` | ~654 | `SnapshotParams` |
| `commands/sync.rs` | ~626 | `AcceptParams` |

- [ ] **Step 5: Verify desktop compilation**

Run: `cd krillnotes-desktop && cargo check -p krillnotes-desktop --features rbac`
Expected: compiles

- [ ] **Step 6: Run core tests one final time**

Run: `cargo test -p krillnotes-core`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/workspace.rs krillnotes-desktop/src-tauri/src/commands/swarm.rs krillnotes-desktop/src-tauri/src/commands/sync.rs krillnotes-core/src/core/export.rs
git commit -m "feat(desktop): update all call sites for non-optional permission gate"
```

---

## Task 8: Root Owner End-to-End Verification

**Files:**
- Modify: `krillnotes-rbac/src/tests/gate_tests.rs` (or create integration test)

- [ ] **Step 1: Write end-to-end test with real `RbacGate`**

Create a test that:
1. Creates a workspace with `RbacGate` as the gate
2. Uses the owner's pubkey as both `owner_pubkey` and `current_identity_pubkey`
3. Performs every category of operation: create note, update note, move note, delete note, undo, redo, attach file, delete attachment, create/update/delete script
4. Asserts all succeed (root owner has full access)

```rust
#[test]
fn test_root_owner_can_do_everything() {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
    let owner_pubkey = base64::engine::general_purpose::STANDARD
        .encode(signing_key.verifying_key().as_bytes());
    let gate = Box::new(RbacGate::new(owner_pubkey.clone()));

    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(), "", "test-identity", signing_key, gate,
    ).unwrap();

    // Create note
    let root = ws.list_all_notes().unwrap()[0].clone();
    let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

    // Update note
    ws.update_note_title(&child_id, "Test Note").unwrap();

    // Move note (create another child, move under it)
    let child2_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
    ws.move_note(&child_id, &child2_id, 0).unwrap();

    // Delete note
    ws.delete_note_recursive(&child_id).unwrap();

    // Undo + redo
    ws.undo().unwrap();
    ws.redo().unwrap();

    // Scripts (create, update, delete)
    let script_id = ws.create_user_script_with_category(
        "TestScript", "schema(\"Test\", #{});", None,
    ).unwrap();
    ws.update_user_script(&script_id, "TestScript2", "schema(\"Test2\", #{});", None).unwrap();
    ws.delete_user_script(&script_id).unwrap();

    // All operations succeeded — root owner has full access
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p krillnotes-rbac test_root_owner_can_do_everything`
Expected: PASS

- [ ] **Step 3: Write test for non-owner denial**

Create a test where the workspace has `RbacGate` but `current_identity_pubkey` does NOT match `owner_pubkey` and has no grants. Assert that operations are denied.

```rust
#[test]
fn test_non_owner_without_grants_is_denied() {
    let owner_key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
    let owner_pubkey = base64::engine::general_purpose::STANDARD
        .encode(owner_key.verifying_key().as_bytes());
    let gate = Box::new(RbacGate::new(owner_pubkey));

    // Create workspace as owner
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(), "", "test-identity", owner_key, gate,
    ).unwrap();

    // Manually change current_identity_pubkey to a non-owner
    // (This may require a test helper or direct field access)
    // Then attempt an operation and assert Permission error.
}
```

Note: The exact approach depends on whether `current_identity_pubkey` can be changed after construction. If not, create the workspace and re-open with a different identity.

- [ ] **Step 4: Run the test**

Run: `cargo test -p krillnotes-rbac test_non_owner_without_grants`
Expected: PASS

- [ ] **Step 5: Run full test suite across all crates**

Run: `cargo test --workspace`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add krillnotes-rbac/
git commit -m "test(rbac): add root owner end-to-end and non-owner denial tests"
```
