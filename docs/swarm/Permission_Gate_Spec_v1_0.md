# SWARM PROTOCOL — Permission Gate Architecture

**Version 1.0 — March 2026**

*Companion to Swarm Protocol Unified Design v0.7 and RBAC Specification v1.0*

This document specifies the plugin architecture for the permission layer in Krillnotes. It describes the `PermissionGate` trait, protocol isolation, operation log integration, and the crate structure that allows different permission models to be linked into different product tiers.

The behavioural rules for the open-source RBAC model (roles, inheritance, sub-delegation, revocation) remain in the **RBAC Specification v1.0**. This document specifies only the plugin seam — the interface between `krillnotes-core` and any permission implementation.

---

## 1. Motivation

The Swarm protocol requires permission enforcement, but different product tiers need different permission models:

| Product | Permission model | Crate |
|---|---|---|
| Krillnotes (open source) | RBAC — 4 roles, tree inheritance | `krillnotes-rbac` |
| OPswarm Notes (enterprise) | ACL — bitmasks, groups, workspace-level | `opswarm-acl` (separate repo) |

`krillnotes-core` defines the plugin interface. It has no knowledge of any specific implementation. Each implementation is a separate crate that links against core and provides a concrete type implementing `PermissionGate`.

**Isolation requirement:** Krillnotes swarm bundles and OPswarm swarm bundles must never be mutually processable. A Krillnotes client receiving an OPswarm bundle must reject it outright — not attempt to interpret it through the RBAC model, which would silently discard ACL data. Protocol isolation is enforced via the `protocol_id()` method and a corresponding field in the `.swarm` bundle header.

---

## 2. The `PermissionGate` Trait

Defined in `krillnotes-core/src/core/permission.rs`.

```rust
use rusqlite::Connection;
use crate::core::{operation::Operation, identity::IdentityId};

/// A pluggable permission enforcement backend.
///
/// The workspace holds exactly one gate, set at construction time.
/// The RBAC gate is always installed — even in single-user workspaces.
/// In single-user mode all operations originate from the workspace owner,
/// who always holds the Owner role, so all checks pass trivially.
pub trait PermissionGate: Send + Sync {
    /// Protocol discriminator embedded in every outbound .swarm bundle header.
    ///
    /// The receiving workspace calls this method on its own gate and rejects
    /// any bundle whose `protocol` header field does not match.
    ///
    /// Krillnotes RBAC:  "krillnotes/1"
    /// OPswarm ACL:      "opswarm/1"
    fn protocol_id(&self) -> &'static str;

    /// Authorise an operation before it is applied.
    ///
    /// Called for every operation — both locally generated and inbound from
    /// a .swarm bundle — before the operation is written to the database.
    ///
    /// `actor` is the identity that claims to be performing the operation.
    /// `operation` is the full, already-signature-verified operation object.
    ///
    /// Returns `Ok(())` if the operation is permitted.
    /// Returns `Err(PermissionError)` if it is denied; the workspace
    /// rejects the operation and does not apply it.
    fn authorize(
        &self,
        conn: &Connection,
        actor: &IdentityId,
        operation: &Operation,
    ) -> Result<(), PermissionError>;

    /// Apply a permission-modifying operation to the gate's own tables.
    ///
    /// Called after `authorize()` has returned `Ok(())` for a
    /// `SetPermission` or `RevokePermission` operation.
    /// The gate validates the grant chain and writes its own DB rows.
    ///
    /// Calling this with any other operation variant is a logic error
    /// and implementations should return `PermissionError::NotAPermissionOp`.
    fn apply_permission_op(
        &self,
        conn: &Connection,
        operation: &Operation,
    ) -> Result<(), PermissionError>;

    /// Create or migrate the gate's own database tables.
    ///
    /// Called once when the workspace is opened. The gate is responsible
    /// for its own schema — `krillnotes-core` does not know or manage
    /// the gate's tables.
    fn ensure_schema(&self, conn: &Connection) -> Result<(), PermissionError>;
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("operation denied: {0}")]
    Denied(String),
    #[error("invalid permission chain: {0}")]
    InvalidChain(String),
    #[error("operation is not a permission operation")]
    NotAPermissionOp,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}
```

---

## 3. Operation Enum Changes

Three new variants are added to `Operation` in `krillnotes-core/src/core/operation.rs`.

The `payload` field carries gate-specific data as an opaque JSON value. Core signs and logs the payload without interpreting it. Each gate deserialises the payload it knows how to read.

```rust
/// Grant a permission on a note to an identity.
/// The payload is gate-specific:
///   RBAC:  { "role": "writer" }
///   ACL:   { "bits": 63 }
SetPermission {
    note_id:    NoteId,
    actor:      IdentityId,       // the identity receiving the permission
    granted_by: IdentityId,       // the identity issuing the grant
    payload:    serde_json::Value, // gate-specific permission data
},

/// Revoke all permissions on a note from an identity.
/// No payload — the meaning is universal across all gate implementations.
RevokePermission {
    note_id:    NoteId,
    actor:      IdentityId,       // the identity losing access
    revoked_by: IdentityId,       // the identity issuing the revocation
},

/// Gate-specific operation with no core semantics.
///
/// Used by enterprise gate implementations for operations that have no
/// equivalent in the RBAC model (e.g. group management in ACL).
/// Core signs, logs, and forwards the payload to the gate unchanged.
///
/// The RBAC gate denies all Extension operations — they are enterprise-only.
/// Unknown `kind` values must also be denied (fail-closed).
Extension {
    kind:    String,              // namespaced identifier, e.g. "opswarm/create_group"
    payload: serde_json::Value,
},
```

All three variants are signed as part of the normal operation signing pipeline (see Operation Envelope Spec). The `granted_by` / `revoked_by` fields are redundant with the operation signature but are included explicitly for chain-walking during cascade revocation evaluation, which must be performable without re-verifying the signature on every step.

**Exhaustive matching as enforcement.** The `Operation` enum is matched exhaustively in every gate's `authorize()` implementation. When a new variant is added to core, every gate crate fails to compile until it explicitly handles the new variant. There is no silent deferral — the gate is the sole authority and must have a position on every operation type.

---

## 4. Workspace Integration

`Workspace` in `workspace.rs` holds one gate:

```rust
pub struct Workspace {
    // ... existing fields ...
    permission_gate: Box<dyn PermissionGate>,
}
```

### 4.1 Construction

The gate is passed in at construction time. The desktop crate supplies the RBAC gate:

```rust
// krillnotes-desktop / src-tauri / lib.rs
use krillnotes_rbac::RbacGate;

let workspace = Workspace::open(path, password, Box::new(RbacGate::new()))?;
```

`Workspace::open` calls `gate.ensure_schema(&conn)?` immediately after the core schema migrations.

### 4.2 Authorization call sites

Every mutating `Workspace` method that originates from a peer identity calls `authorize` before applying:

```rust
// Pattern used in every mutating workspace method
if let Some(actor) = &actor_identity {
    self.permission_gate
        .authorize(&self.conn, actor, &operation)
        .map_err(KrillnotesError::Permission)?;
}
```

Locally-initiated operations from the workspace owner pass the owner's `IdentityId` as `actor`. The RBAC gate resolves the owner to the `Owner` role and the check passes.

Inbound operations from `.swarm` bundle application pass the `authored_by` identity from the signed operation envelope as `actor`.

### 4.3 Permission operation call site

After `authorize()` passes for a `SetPermission` or `RevokePermission` operation, the workspace calls `apply_permission_op` within the same database transaction before committing:

```rust
match &operation {
    Operation::SetPermission { .. } | Operation::RevokePermission { .. } => {
        self.permission_gate
            .apply_permission_op(&self.conn, &operation)
            .map_err(KrillnotesError::Permission)?;
    }
    _ => {}
}
```

### 4.4 Bundle ingest — protocol check

When a `.swarm` bundle arrives for processing, the workspace checks the protocol field before decrypting the payload:

```rust
if bundle.header.protocol != self.permission_gate.protocol_id() {
    return Err(KrillnotesError::ProtocolMismatch {
        expected: self.permission_gate.protocol_id(),
        found: bundle.header.protocol.clone(),
    });
}
```

This check happens before decryption, before signature verification, and before any operation is touched.

---

## 5. Crate Structure

```
krillnotes/
├── krillnotes-core/
│   └── src/core/
│       ├── permission.rs          # PermissionGate trait + PermissionError
│       └── operation.rs           # SetPermission + RevokePermission variants added here
│
└── krillnotes-rbac/               # new crate, same workspace
    ├── Cargo.toml                 # depends on krillnotes-core only
    └── src/
        ├── lib.rs                 # pub use RbacGate
        ├── gate.rs                # RbacGate: implements PermissionGate
        ├── resolver.rs            # tree-walk permission resolution
        └── schema.sql             # note_permissions table DDL
```

`krillnotes-desktop` adds `krillnotes-rbac` as a direct dependency. `krillnotes-core` has no dependency on `krillnotes-rbac`.

### Cargo.toml (workspace root)

```toml
[workspace]
members = [
    "krillnotes-core",
    "krillnotes-rbac",       # new
    "krillnotes-desktop",
]
```

### krillnotes-rbac/Cargo.toml

```toml
[package]
name = "krillnotes-rbac"
version = "0.1.0"
edition = "2021"

[dependencies]
krillnotes-core = { path = "../krillnotes-core" }
rusqlite = { version = "0.31", features = ["bundled"] }
serde_json = "1"
thiserror = "1"
```

---

## 6. The RBAC Implementation (`krillnotes-rbac`)

Full behavioural specification is in **RBAC Specification v1.0**. This section covers only the implementation contract.

### 6.1 Protocol identity

```rust
fn protocol_id(&self) -> &'static str { "krillnotes/1" }
```

This string is embedded verbatim in every outbound `.swarm` bundle header. It must not change without a major version bump and a migration path.

### 6.2 Database schema

The gate owns one table, created by `ensure_schema`:

```sql
CREATE TABLE IF NOT EXISTS note_permissions (
    note_id     TEXT NOT NULL,   -- note UUID, or workspace UUID for workspace-level grants
    user_id     TEXT NOT NULL,   -- IdentityId (public key fingerprint)
    role        TEXT NOT NULL CHECK(role IN ('owner', 'writer', 'reader', 'none')),
    granted_by  TEXT NOT NULL,   -- IdentityId of the granting identity
    PRIMARY KEY (note_id, user_id)
);
```

No foreign key constraint on `note_id`. The column may contain either a note UUID (from the `notes` table) or the **workspace UUID** (stored in `workspace_meta`). The workspace UUID acts as the virtual root — the parent above all root-level notes. It is never in the `notes` table.

At workspace creation (first open), the gate inserts a single bootstrap row: the workspace owner holds `owner` role on the workspace UUID. When a peer is invited, their assigned role is stored as a row against the workspace UUID. This is their workspace-level role, inherited by all notes in the tree.

### 6.3 Authorization

`authorize()` implements the operation-type permission matrix from RBAC Spec §5.2.

**Scope resolution** — the note ID used as the starting point for the tree walk depends on the operation type:

| Operation | Scope for permission check |
|---|---|
| `CreateNote` | Parent note. Root-level creation (`parent_id` null) → workspace UUID |
| `UpdateNote`, `UpdateField`, `SetTags`, `DeleteNote`, `MoveNote`, `AddAttachment`, `RemoveAttachment` | Target `note_id` |
| `SetPermission`, `RevokePermission` | Target `note_id` (the note whose permissions are changing) |
| `CreateUserScript`, `UpdateUserScript`, `DeleteUserScript` | Workspace UUID — these are workspace-level operations |
| `ExportWorkspace` | Workspace UUID |
| `Extension { .. }` | Workspace UUID — denied by RBAC gate regardless of resolved role |

**Tree walk (note-scoped operations):**

1. Starting from the resolved scope, walk up the tree via `parent_id` links, checking `note_permissions` for an explicit entry for `actor` at each node.
2. Continue until an entry is found or the workspace UUID is reached. Check the workspace UUID row last.
3. If no entry is found anywhere, access is denied.
4. Check the resolved role against the operation type. Both `Owner` and `Writer` permit `CreateNote` at any level, including root level (RBAC Spec §5.2).
5. For `SetPermission`: additionally verify that `actor`'s role ≥ the role in the payload (role-capped sub-delegation, RBAC Spec §4.1).
6. For `RevokePermission`: verify that `actor` holds Owner on the scope OR is the `granted_by` identity for the target grant (RBAC Spec §6.3).

**Workspace-scoped operations** (scope = workspace UUID):

The resolved role is the actor's `note_permissions` entry on the workspace UUID — their workspace-level role assigned at invitation time.

| Operation | Required role |
|---|---|
| `CreateNote` (root level) | `Owner` or `Writer` |
| `CreateUserScript`, `UpdateUserScript`, `DeleteUserScript` | `Owner` only |
| `ExportWorkspace` | `Owner` only |
| `SetPermission` on workspace UUID | `Owner` only |
| `Extension { .. }` | Denied |

These correspond to the root-owner-only privileges in RBAC Spec §7.1. Enterprise gate implementations (ACL) may relax these using workspace-level permission bits (`s`, `x`, `a`, `g`, `e`) as specified in the ACL Specification.

### 6.4 `apply_permission_op`

- `SetPermission`: upsert a row into `note_permissions`.
- `RevokePermission`: delete the row for `(note_id, actor)`. Then cascade: identify all rows whose `granted_by` is `actor`, evaluate whether the sub-delegation chain is still valid, and delete rows that are no longer backed by a valid granter (RBAC Spec §4.3).

### 6.5 `SetPermission` payload

The RBAC gate deserialises the payload as:

```json
{ "role": "writer" }
```

Valid values: `"owner"`, `"writer"`, `"reader"`, `"none"`. Any other value causes `apply_permission_op` to return `PermissionError::Denied`.

---

## 7. Bundle Header Change

The `.swarm` bundle header (Operation Envelope Spec) gains one mandatory field:

```json
{
  "protocol": "krillnotes/1",
  "version": "1.0",
  "sender": "<identity-id>",
  ...
}
```

`protocol` is a required field. Bundles missing this field are rejected. `protocol` is checked before any other processing — before decryption, before signature verification, before any operation is touched.

---

## 8. Error Propagation

`PermissionError` is wrapped by `KrillnotesError`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum KrillnotesError {
    // ... existing variants ...
    #[error("permission denied: {0}")]
    Permission(#[from] PermissionError),
    #[error("protocol mismatch: expected {expected}, found {found}")]
    ProtocolMismatch { expected: &'static str, found: String },
}
```

Tauri commands surface `PermissionError::Denied` to the frontend as a user-visible error string. `ProtocolMismatch` should be surfaced as a distinct UI state ("This bundle is from an incompatible Swarm product").

---

*End of Permission Gate Architecture Specification v1.0*
