# RBAC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the RBAC permission model (v2.0 spec) as a pluggable gate behind the `PermissionGate` trait, enforcing role-based access on all workspace operations.

**Architecture:** A `PermissionGate` trait in `krillnotes-core` defines the plugin seam. A new `krillnotes-rbac` crate implements `RbacGate` with tree-walk resolution, role-capped delegation, and cascade revocation. `Workspace` holds an `Option<Box<dyn PermissionGate>>`  — `None` in single-user/test mode, `Some(RbacGate)` in production. Authorization is checked before every mutating operation.

**Deliberate deviation:** The Permission Gate Spec v1.0 says the gate is always installed (non-optional). This plan uses `Option` to avoid changing all existing test call sites in krillnotes-core (which can't depend on krillnotes-rbac without a circular dependency). This can be tightened in a follow-up PR.

**Deferred to future work:**
- `ProtocolMismatch` error variant (needed when bundle ingest is implemented)
- Contested operation states (three-state model: valid/rejected/contested for eventual consistency — see RBAC Spec §5.1)
- Bundle-level protocol header checks (Permission Gate Spec §4.4)

**Tech Stack:** Rust, rusqlite (SQLCipher), ed25519-dalek (signatures), serde_json, thiserror

**Specs:**
- Behavioral: `docs/swarm/Swarm_RBAC_Spec_v2_0.md`
- Plugin architecture: `docs/swarm/Permission_Gate_Spec_v1_0.md`

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `krillnotes-core/src/core/permission.rs` | `PermissionGate` trait, `PermissionError` enum |
| `krillnotes-rbac/Cargo.toml` | Crate manifest, depends on `krillnotes-core` |
| `krillnotes-rbac/src/lib.rs` | Public re-exports |
| `krillnotes-rbac/src/gate.rs` | `RbacGate` struct, `PermissionGate` impl |
| `krillnotes-rbac/src/resolver.rs` | Tree-walk permission resolution |
| `krillnotes-rbac/src/schema.sql` | `note_permissions` table DDL |
| `krillnotes-rbac/src/tests/mod.rs` | Test module root |
| `krillnotes-rbac/src/tests/resolver_tests.rs` | Tree-walk resolution tests |
| `krillnotes-rbac/src/tests/gate_tests.rs` | Authorization + apply tests |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace root) | Add `krillnotes-rbac` to workspace members |
| `krillnotes-core/src/core/mod.rs` | Add `pub mod permission;` |
| `krillnotes-core/src/core/error.rs` | Add `Permission` variant to `KrillnotesError` |
| `krillnotes-core/src/lib.rs` | Re-export `permission` module |
| `krillnotes-core/src/core/operation.rs` | Add `RemovePeer` and `TransferRootOwnership` variants |
| `krillnotes-core/src/core/workspace/mod.rs` | Add `permission_gate` field, update constructors |
| `krillnotes-core/src/core/workspace/notes.rs` | Add `authorize()` calls before mutations |
| `krillnotes-core/src/core/workspace/sync.rs` | Add `authorize()` calls in bundle application |
| `krillnotes-desktop/src-tauri/Cargo.toml` | Add `krillnotes-rbac` dependency |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Pass `RbacGate` when creating workspaces |

---

## Task 1: PermissionGate Trait + PermissionError

**Files:**
- Create: `krillnotes-core/src/core/permission.rs`
- Modify: `krillnotes-core/src/core/mod.rs`
- Modify: `krillnotes-core/src/core/error.rs`
- Modify: `krillnotes-core/src/lib.rs`

- [ ] **Step 1: Create the `PermissionGate` trait and `PermissionError` enum**

```rust
// krillnotes-core/src/core/permission.rs

use rusqlite::Connection;
use crate::core::operation::Operation;

/// A pluggable permission enforcement backend.
///
/// The workspace holds an optional gate. When present, every mutating
/// operation is checked via `authorize()` before being applied.
/// The gate owns its own database tables and manages them via
/// `ensure_schema()` and `apply_permission_op()`.
pub trait PermissionGate: Send + Sync {
    /// Protocol discriminator embedded in every outbound .swarm bundle header.
    /// Krillnotes RBAC: "krillnotes/1"
    fn protocol_id(&self) -> &'static str;

    /// Authorise an operation before it is applied.
    ///
    /// Called for every mutating operation — both locally generated and
    /// inbound from a .swarm bundle — before the operation is written
    /// to the database.
    ///
    /// `actor` is the base64-encoded Ed25519 public key of the identity
    /// performing the operation.
    ///
    /// Returns `Ok(())` if permitted, `Err(PermissionError)` if denied.
    fn authorize(
        &self,
        conn: &Connection,
        actor: &str,
        operation: &Operation,
    ) -> Result<(), PermissionError>;

    /// Apply a permission-modifying operation to the gate's own tables.
    ///
    /// Called after `authorize()` has returned `Ok(())` for a
    /// `SetPermission` or `RevokePermission` operation, within the
    /// same database transaction.
    fn apply_permission_op(
        &self,
        conn: &Connection,
        operation: &Operation,
    ) -> Result<(), PermissionError>;

    /// Create or migrate the gate's database tables.
    /// Called once when the workspace is opened.
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

- [ ] **Step 2: Wire into krillnotes-core modules**

Add to `krillnotes-core/src/core/mod.rs`:
```rust
pub mod permission;
```

Add to `krillnotes-core/src/lib.rs` re-exports (alongside existing re-exports):
```rust
pub use core::permission::{PermissionGate, PermissionError};
```

- [ ] **Step 3: Add Permission variant to KrillnotesError**

In `krillnotes-core/src/core/error.rs`, add variant to the `KrillnotesError` enum:
```rust
    #[error("permission denied: {0}")]
    Permission(#[from] crate::core::permission::PermissionError),
```

Add a case to `user_message()`:
```rust
    Self::Permission(e) => format!("Permission denied: {}", e),
```

- [ ] **Step 4: Verify core crate compiles**

Run: `cargo check -p krillnotes-core`
Expected: compiles with no errors

- [ ] **Step 5: Run existing tests to confirm no regressions**

Run: `cargo test -p krillnotes-core`
Expected: all existing tests pass

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/permission.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/core/error.rs krillnotes-core/src/lib.rs
git commit -m "feat(core): add PermissionGate trait and PermissionError"
```

---

## Task 2: krillnotes-rbac Crate Scaffolding

**Files:**
- Create: `krillnotes-rbac/Cargo.toml`
- Create: `krillnotes-rbac/src/lib.rs`
- Create: `krillnotes-rbac/src/gate.rs`
- Create: `krillnotes-rbac/src/resolver.rs`
- Create: `krillnotes-rbac/src/schema.sql`
- Create: `krillnotes-rbac/src/tests/mod.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create crate directory and Cargo.toml**

```toml
# krillnotes-rbac/Cargo.toml
[package]
name = "krillnotes-rbac"
version = "0.1.0"
edition = "2021"

[dependencies]
krillnotes-core = { path = "../krillnotes-core" }
rusqlite = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
log = { workspace = true }

[dev-dependencies]
uuid = { workspace = true }
```

- [ ] **Step 2: Add to workspace members**

In root `Cargo.toml`, change:
```toml
members = ["krillnotes-core", "krillnotes-desktop/src-tauri"]
```
to:
```toml
members = ["krillnotes-core", "krillnotes-rbac", "krillnotes-desktop/src-tauri"]
```

- [ ] **Step 3: Create schema.sql**

```sql
-- krillnotes-rbac/src/schema.sql
-- RBAC permission entries. One row per (note, user) pair.
-- note_id is always a note UUID (never NULL for RBAC).
-- The Root Owner has no entry here — they are identified by identity check.
CREATE TABLE IF NOT EXISTS note_permissions (
    note_id     TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    role        TEXT NOT NULL CHECK(role IN ('owner', 'writer', 'reader')),
    granted_by  TEXT NOT NULL,
    PRIMARY KEY (note_id, user_id)
);
```

- [ ] **Step 4: Create stub files**

`krillnotes-rbac/src/lib.rs`:
```rust
mod gate;
mod resolver;
#[cfg(test)]
mod tests;

pub use gate::RbacGate;
```

`krillnotes-rbac/src/gate.rs`:
```rust
use krillnotes_core::core::operation::Operation;
use krillnotes_core::core::permission::{PermissionError, PermissionGate};
use rusqlite::Connection;

/// RBAC permission gate for Krillnotes (open source).
///
/// Implements the 4-role model: Root Owner > Owner > Writer > Reader.
/// The Root Owner is identified by public key comparison, not by a
/// database entry. All other roles are stored in `note_permissions`.
pub struct RbacGate {
    /// Base64-encoded Ed25519 public key of the workspace creator.
    owner_pubkey: String,
}

impl RbacGate {
    pub fn new(owner_pubkey: String) -> Self {
        Self { owner_pubkey }
    }

    /// Returns true if the given actor is the Root Owner.
    fn is_root_owner(&self, actor: &str) -> bool {
        actor == self.owner_pubkey
    }
}

impl PermissionGate for RbacGate {
    fn protocol_id(&self) -> &'static str {
        "krillnotes/1"
    }

    fn authorize(
        &self,
        _conn: &Connection,
        _actor: &str,
        _operation: &Operation,
    ) -> Result<(), PermissionError> {
        // TODO: implement in Task 5
        Ok(())
    }

    fn apply_permission_op(
        &self,
        _conn: &Connection,
        _operation: &Operation,
    ) -> Result<(), PermissionError> {
        // TODO: implement in Task 6
        Ok(())
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<(), PermissionError> {
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(())
    }
}
```

`krillnotes-rbac/src/resolver.rs`:
```rust
use rusqlite::Connection;

/// RBAC roles, ordered from most to least privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Owner = 3,
    Writer = 2,
    Reader = 1,
}

impl Role {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "writer" => Some(Self::Writer),
            "reader" => Some(Self::Reader),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Writer => "writer",
            Self::Reader => "reader",
        }
    }
}

/// Resolve the effective role of `user_id` on `note_id` by walking up the tree.
///
/// Returns `None` if no explicit grant exists anywhere in the ancestry chain
/// (default-deny).
pub fn resolve_role(
    conn: &Connection,
    user_id: &str,
    note_id: &str,
) -> Result<Option<Role>, rusqlite::Error> {
    // TODO: implement in Task 4
    let _ = (conn, user_id, note_id);
    Ok(None)
}
```

`krillnotes-rbac/src/tests/mod.rs`:
```rust
mod resolver_tests;
mod gate_tests;
```

`krillnotes-rbac/src/tests/resolver_tests.rs`:
```rust
// Tests added in Task 4
```

`krillnotes-rbac/src/tests/gate_tests.rs`:
```rust
// Tests added in Task 5 and 6
```

- [ ] **Step 5: Verify the crate compiles and existing tests still pass**

Run: `cargo check -p krillnotes-rbac && cargo test -p krillnotes-core`
Expected: both pass

- [ ] **Step 6: Commit**

```bash
git add krillnotes-rbac/ Cargo.toml
git commit -m "feat(rbac): scaffold krillnotes-rbac crate with PermissionGate stubs"
```

---

## Task 3: Tree-Walk Permission Resolver (TDD)

**Files:**
- Modify: `krillnotes-rbac/src/resolver.rs`
- Modify: `krillnotes-rbac/src/tests/resolver_tests.rs`

The resolver is the core algorithm: given a user and a note, walk up the tree to find the first explicit permission entry. This task is heavily tested because it underpins all authorization.

**Test setup helper** — all resolver tests use an in-memory SQLite database with the core schema + RBAC schema + a test tree. Create this helper at the top of `resolver_tests.rs`:

```rust
use crate::resolver::{resolve_role, Role};
use rusqlite::Connection;

/// Create an in-memory DB with the notes table and note_permissions table,
/// populated with a test tree:
///
/// root_a  (root node)
///   ├─ child_1
///   │  └─ grandchild_1
///   └─ child_2
/// root_b  (second root node)
///   └─ child_3
fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("
        CREATE TABLE notes (
            id TEXT PRIMARY KEY,
            parent_id TEXT,
            title TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE note_permissions (
            note_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('owner','writer','reader')),
            granted_by TEXT NOT NULL,
            PRIMARY KEY (note_id, user_id)
        );

        -- Two root nodes
        INSERT INTO notes (id, parent_id, title) VALUES ('root_a', NULL, 'Root A');
        INSERT INTO notes (id, parent_id, title) VALUES ('root_b', NULL, 'Root B');
        -- Children under root_a
        INSERT INTO notes (id, parent_id, title) VALUES ('child_1', 'root_a', 'Child 1');
        INSERT INTO notes (id, parent_id, title) VALUES ('child_2', 'root_a', 'Child 2');
        -- Grandchild
        INSERT INTO notes (id, parent_id, title) VALUES ('grandchild_1', 'child_1', 'Grandchild 1');
        -- Child under root_b
        INSERT INTO notes (id, parent_id, title) VALUES ('child_3', 'root_b', 'Child 3');
    ").unwrap();
    conn
}
```

- [ ] **Step 1: Write test — default-deny (no grants at all)**

```rust
#[test]
fn test_no_grant_returns_none() {
    let conn = setup_test_db();
    // Bob has no grants anywhere
    let role = resolve_role(&conn, "bob", "grandchild_1").unwrap();
    assert_eq!(role, None);
}
```

Run: `cargo test -p krillnotes-rbac test_no_grant_returns_none`
Expected: FAIL (resolve_role always returns None — but this test passes trivially, so also add the next test)

- [ ] **Step 2: Write test — direct grant on the note itself**

```rust
#[test]
fn test_direct_grant_on_note() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('child_1', 'bob', 'writer', 'alice')",
        [],
    ).unwrap();
    let role = resolve_role(&conn, "bob", "child_1").unwrap();
    assert_eq!(role, Some(Role::Writer));
}
```

Run: `cargo test -p krillnotes-rbac test_direct_grant_on_note`
Expected: FAIL — resolve_role returns None instead of Some(Writer)

- [ ] **Step 3: Implement resolve_role**

```rust
pub fn resolve_role(
    conn: &Connection,
    user_id: &str,
    note_id: &str,
) -> Result<Option<Role>, rusqlite::Error> {
    let mut current_id = Some(note_id.to_string());

    while let Some(id) = current_id {
        // Check for explicit grant at this node
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                rusqlite::params![id, user_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(role_str) = role {
            return Ok(Role::from_str(&role_str));
        }

        // Walk up to parent
        current_id = conn
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
    }

    Ok(None) // default-deny
}
```

Add `use rusqlite::OptionalExtension;` at the top of resolver.rs.

- [ ] **Step 4: Run tests to verify both pass**

Run: `cargo test -p krillnotes-rbac resolver_tests`
Expected: both pass

- [ ] **Step 5: Write test — inherited grant from parent**

```rust
#[test]
fn test_inherited_grant_from_parent() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'reader', 'alice')",
        [],
    ).unwrap();
    // grandchild_1 → child_1 → root_a (bob: reader)
    let role = resolve_role(&conn, "bob", "grandchild_1").unwrap();
    assert_eq!(role, Some(Role::Reader));
}
```

Run: `cargo test -p krillnotes-rbac test_inherited`
Expected: PASS (already works with the walk implementation)

- [ ] **Step 6: Write test — override: closer grant wins**

```rust
#[test]
fn test_closer_grant_overrides_inherited() {
    let conn = setup_test_db();
    // root_a: bob = reader
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'reader', 'alice')",
        [],
    ).unwrap();
    // child_1: bob = owner (overrides inherited reader)
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('child_1', 'bob', 'owner', 'alice')",
        [],
    ).unwrap();
    let role = resolve_role(&conn, "bob", "grandchild_1").unwrap();
    assert_eq!(role, Some(Role::Owner));
}
```

- [ ] **Step 7: Write test — different users, different roles on same subtree**

```rust
#[test]
fn test_different_users_different_roles() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'writer', 'alice')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'carol', 'reader', 'alice')",
        [],
    ).unwrap();
    assert_eq!(resolve_role(&conn, "bob", "child_1").unwrap(), Some(Role::Writer));
    assert_eq!(resolve_role(&conn, "carol", "child_1").unwrap(), Some(Role::Reader));
}
```

- [ ] **Step 8: Write test — no cross-tree inheritance**

```rust
#[test]
fn test_no_cross_tree_inheritance() {
    let conn = setup_test_db();
    // Grant on root_a only
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'owner', 'alice')",
        [],
    ).unwrap();
    // child_3 is under root_b — bob has no access
    assert_eq!(resolve_role(&conn, "bob", "child_3").unwrap(), None);
}
```

- [ ] **Step 9: Run all resolver tests**

Run: `cargo test -p krillnotes-rbac resolver_tests`
Expected: all pass

- [ ] **Step 10: Commit**

```bash
git add krillnotes-rbac/src/resolver.rs krillnotes-rbac/src/tests/resolver_tests.rs
git commit -m "feat(rbac): implement tree-walk permission resolver with tests"
```

---

## Task 4: authorize() — Permission Matrix (TDD)

**Files:**
- Modify: `krillnotes-rbac/src/gate.rs`
- Modify: `krillnotes-rbac/src/tests/gate_tests.rs`

Implement the operation-type permission matrix from RBAC Spec v2.0 §3.

**Test setup helper** — gate tests need a full test environment (DB with notes + permissions, an RbacGate instance, and helper functions to create operations). Build this in `gate_tests.rs`:

```rust
use crate::gate::RbacGate;
use crate::resolver::Role;
use krillnotes_core::core::operation::Operation;
use krillnotes_core::core::permission::PermissionGate;
use rusqlite::Connection;

const ROOT_OWNER: &str = "root_owner_pubkey_base64";
const BOB: &str = "bob_pubkey_base64";
const CAROL: &str = "carol_pubkey_base64";

fn setup_gate_db() -> (Connection, RbacGate) {
    let conn = Connection::open_in_memory().unwrap();
    let gate = RbacGate::new(ROOT_OWNER.to_string());
    // Create tables
    conn.execute_batch("
        CREATE TABLE notes (
            id TEXT PRIMARY KEY,
            parent_id TEXT,
            title TEXT NOT NULL DEFAULT '',
            created_by TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE note_permissions (
            note_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('owner','writer','reader')),
            granted_by TEXT NOT NULL,
            PRIMARY KEY (note_id, user_id)
        );
        -- Test tree
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('root_a', NULL, 'Root A', 'root_owner_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_1', 'root_a', 'Child 1', 'bob_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_2', 'root_a', 'Child 2', 'carol_pubkey_base64');
    ").unwrap();
    (conn, gate)
}

/// Helper to build a minimal CreateNote operation for testing.
fn make_create_note(parent_id: &str) -> Operation {
    Operation::CreateNote {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp { wall_ms: 1, counter: 0, node_id: 0 },
        device_id: "test_device".into(),
        parent_id: Some(parent_id.into()),
        title: "Test Note".into(),
        schema: "TextNote".into(),
        note_id: uuid::Uuid::new_v4().to_string(),
        position: 0.0,
        fields: std::collections::BTreeMap::new(),
        created_by: String::new(),
        signature: String::new(),
    }
}

/// Helper for root-level note creation (parent_id = None).
fn make_create_note_root() -> Operation {
    Operation::CreateNote {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp { wall_ms: 1, counter: 0, node_id: 0 },
        device_id: "test_device".into(),
        parent_id: None,
        title: "Root Note".into(),
        schema: "TextNote".into(),
        note_id: uuid::Uuid::new_v4().to_string(),
        position: 0.0,
        fields: std::collections::BTreeMap::new(),
        created_by: String::new(),
        signature: String::new(),
    }
}

/// Helper to insert a permission grant into the test DB.
fn grant(conn: &Connection, note_id: &str, user_id: &str, role: &str) {
    grant_by(conn, note_id, user_id, role, ROOT_OWNER);
}

fn grant_by(conn: &Connection, note_id: &str, user_id: &str, role: &str, granted_by: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![note_id, user_id, role, granted_by],
    ).unwrap();
}

// Add similar helpers for other operation types as needed: make_update_field,
// make_delete_note, make_move_note, make_set_permission, etc.
// Each helper takes the minimum parameters needed and fills boilerplate.
// All HlcTimestamp fields use: { wall_ms: 1, counter: 0, node_id: 0 }
// Import HlcTimestamp from krillnotes_core::HlcTimestamp (crate root re-export).
```

- [ ] **Step 1: Write test — Root Owner bypasses all checks**

```rust
#[test]
fn test_root_owner_allowed_everything() {
    let (conn, gate) = setup_gate_db();
    let op = make_create_note("root_a");
    assert!(gate.authorize(&conn, ROOT_OWNER, &op).is_ok());
}
```

Run: `cargo test -p krillnotes-rbac test_root_owner_allowed`
Expected: PASS (stub returns Ok)

- [ ] **Step 2: Write test — user with no grant is denied**

```rust
#[test]
fn test_no_grant_denied() {
    let (conn, gate) = setup_gate_db();
    let op = make_create_note("root_a");
    assert!(gate.authorize(&conn, BOB, &op).is_err());
}
```

Run: `cargo test -p krillnotes-rbac test_no_grant_denied`
Expected: FAIL (stub returns Ok for everyone)

- [ ] **Step 3: Implement authorize() core logic**

In `gate.rs`, implement `authorize()`:

```rust
fn authorize(
    &self,
    conn: &Connection,
    actor: &str,
    operation: &Operation,
) -> Result<(), PermissionError> {
    // Root Owner bypasses all checks
    if self.is_root_owner(actor) {
        return Ok(());
    }

    // Determine the scope note for this operation
    let scope_note_id = self.resolve_scope(operation)?;

    // Workspace-level operations are Root Owner only
    if scope_note_id.is_none() {
        return Err(PermissionError::Denied(
            "workspace-level operations require Root Owner".into(),
        ));
    }

    let note_id = scope_note_id.unwrap();
    let role = crate::resolver::resolve_role(conn, actor, &note_id)?
        .ok_or_else(|| PermissionError::Denied("no access to this subtree".into()))?;

    self.check_role_for_operation(conn, actor, role, operation)
}
```

Add helper methods to `RbacGate`:

```rust
/// Determine which note_id to use as the scope for permission checking.
/// Returns None for workspace-level operations (Root Owner only).
fn resolve_scope(&self, operation: &Operation) -> Result<Option<String>, PermissionError> {
    match operation {
        // Note operations: scope is the target note or parent
        Operation::CreateNote { parent_id, .. } => {
            // Root-level creation (parent_id = None) is workspace-level
            Ok(parent_id.clone())
        }
        Operation::UpdateNote { note_id, .. }
        | Operation::UpdateField { note_id, .. }
        | Operation::DeleteNote { note_id, .. }
        | Operation::SetTags { note_id, .. } => Ok(Some(note_id.clone())),
        Operation::MoveNote { note_id, .. } => Ok(Some(note_id.clone())),
        // Permission operations
        Operation::SetPermission { note_id, .. } => Ok(note_id.clone()),
        Operation::RevokePermission { note_id, .. } => Ok(note_id.clone()),
        // Workspace-level operations
        Operation::CreateUserScript { .. }
        | Operation::UpdateUserScript { .. }
        | Operation::DeleteUserScript { .. }
        | Operation::RemovePeer { .. }
        | Operation::TransferRootOwnership { .. } => Ok(None),
        // Existing operations that don't need RBAC
        Operation::UpdateSchema { .. }
        | Operation::RetractOperation { .. }
        | Operation::JoinWorkspace { .. } => Ok(None),
    }
}

/// Check whether the resolved role permits the given operation.
fn check_role_for_operation(
    &self,
    conn: &Connection,
    actor: &str,
    role: Role,
    operation: &Operation,
) -> Result<(), PermissionError> {
    match operation {
        // Read: all roles
        // CreateNote: Owner, Writer
        Operation::CreateNote { .. } => {
            require_at_least(role, Role::Writer)?;
        }
        // Update: Owner, Writer
        Operation::UpdateNote { .. }
        | Operation::UpdateField { .. }
        | Operation::SetTags { .. } => {
            require_at_least(role, Role::Writer)?;
        }
        // Delete: Owner always, Writer only own
        // For DeleteAll strategy: Writer must have authored ALL descendants.
        // The delete strategy is determined at the workspace level. The gate
        // checks the target note's authorship. For DeleteAll, the workspace
        // must call authorize() for each descendant that will be deleted.
        Operation::DeleteNote { note_id, .. } => {
            if role < Role::Owner {
                self.require_authorship(conn, actor, note_id, role)?;
            }
        }
        // Attachments: Owner, Writer
        Operation::AddAttachment { note_id, .. }
        | Operation::RemoveAttachment { note_id, .. } => {
            require_at_least(role, Role::Writer)?;
        }
        // Move: Owner always, Writer only own — DUAL SCOPE CHECK
        // The initial authorize() call checks the source. A second check
        // for the destination parent (new_parent_id) is done separately
        // by the workspace method, which calls authorize() twice.
        Operation::MoveNote { note_id, .. } => {
            if role < Role::Owner {
                self.require_authorship(conn, actor, note_id, role)?;
            }
        }
        // RetractOperation: own ops only, Owner can retract within subtree
        Operation::RetractOperation { retracted_op_ids, .. } => {
            // The workspace must resolve the target note for each retracted op
            // and call authorize() per-op. The gate checks:
            // - Writer/Reader: can only retract own operations
            // - Owner: can retract any operation in their subtree
            // This is handled at the workspace level since the gate only
            // sees one operation at a time. See Task 7 for the workspace-side logic.
        }
        // SetPermission: Owner only
        Operation::SetPermission { role: granted_role, .. } => {
            require_at_least(role, Role::Owner)?;
            // Role-capped: can grant up to and including Owner
            if let Some(target_role) = Role::from_str(granted_role) {
                if target_role > role {
                    return Err(PermissionError::Denied(
                        format!("cannot grant {} (you hold {})", granted_role, role.as_str()),
                    ));
                }
            } else {
                return Err(PermissionError::Denied(
                    format!("invalid role: {}", granted_role),
                ));
            }
        }
        // Revoke: Owner only
        Operation::RevokePermission { .. } => {
            require_at_least(role, Role::Owner)?;
        }
        _ => {}
    }
    Ok(())
}

/// For Writer delete/move: verify the actor authored the target note.
fn require_authorship(
    &self,
    conn: &Connection,
    actor: &str,
    note_id: &str,
    role: Role,
) -> Result<(), PermissionError> {
    require_at_least(role, Role::Writer)?;
    let created_by: String = conn
        .query_row(
            "SELECT created_by FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
            |row| row.get(0),
        )
        .map_err(|_| PermissionError::Denied("note not found".into()))?;
    if created_by != actor {
        return Err(PermissionError::Denied(
            "writers can only delete/move notes they authored".into(),
        ));
    }
    Ok(())
}
```

Add standalone helper:
```rust
use crate::resolver::Role;

fn require_at_least(actual: Role, minimum: Role) -> Result<(), PermissionError> {
    if actual >= minimum {
        Ok(())
    } else {
        Err(PermissionError::Denied(
            format!("requires at least {} (you hold {})", minimum.as_str(), actual.as_str()),
        ))
    }
}
```

- [ ] **Step 4: Run tests to verify Root Owner and no-grant tests pass**

Run: `cargo test -p krillnotes-rbac gate_tests`
Expected: both pass

- [ ] **Step 5: Write tests — Owner can do everything in subtree**

```rust
#[test]
fn test_owner_can_create_update_delete() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "owner");

    assert!(gate.authorize(&conn, BOB, &make_create_note("root_a")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_update_field("child_1")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_delete_note("child_2")).is_ok());
}
```

- [ ] **Step 6: Write tests — Writer can create and edit, but only delete own notes**

```rust
#[test]
fn test_writer_can_create_and_edit() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "writer");

    assert!(gate.authorize(&conn, BOB, &make_create_note("root_a")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_update_field("child_1")).is_ok());
}

#[test]
fn test_writer_can_delete_own_note() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "writer");
    // child_1 was created_by BOB
    assert!(gate.authorize(&conn, BOB, &make_delete_note("child_1")).is_ok());
}

#[test]
fn test_writer_cannot_delete_others_note() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "writer");
    // child_2 was created_by CAROL
    assert!(gate.authorize(&conn, BOB, &make_delete_note("child_2")).is_err());
}
```

- [ ] **Step 7: Write tests — Reader is read-only**

```rust
#[test]
fn test_reader_cannot_create() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "reader");
    assert!(gate.authorize(&conn, BOB, &make_create_note("root_a")).is_err());
}

#[test]
fn test_reader_cannot_update() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "reader");
    assert!(gate.authorize(&conn, BOB, &make_update_field("child_1")).is_err());
}
```

- [ ] **Step 8: Write tests — SetPermission requires Owner, role-capped**

```rust
#[test]
fn test_writer_cannot_set_permission() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "writer");
    assert!(gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "reader")).is_err());
}

#[test]
fn test_owner_can_set_permission_up_to_owner() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "owner");
    assert!(gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "owner")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "writer")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "reader")).is_ok());
}
```

- [ ] **Step 9: Write test — workspace-level operations require Root Owner**

```rust
#[test]
fn test_owner_cannot_create_root_note() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "owner");
    // parent_id = None means root-level creation
    let op = make_create_note_root();
    assert!(gate.authorize(&conn, BOB, &op).is_err());
}

#[test]
fn test_root_owner_can_create_root_note() {
    let (conn, gate) = setup_gate_db();
    let op = make_create_note_root();
    assert!(gate.authorize(&conn, ROOT_OWNER, &op).is_ok());
}
```

- [ ] **Step 10: Run all gate tests**

Run: `cargo test -p krillnotes-rbac gate_tests`
Expected: all pass

- [ ] **Step 11: Commit**

```bash
git add krillnotes-rbac/src/gate.rs krillnotes-rbac/src/tests/gate_tests.rs
git commit -m "feat(rbac): implement authorize() with full permission matrix and tests"
```

---

## Task 5: apply_permission_op — Grant, Revoke, and Cascade (TDD)

**Files:**
- Modify: `krillnotes-rbac/src/gate.rs`
- Modify: `krillnotes-rbac/src/tests/gate_tests.rs`

- [ ] **Step 1: Write test — SetPermission upserts a grant**

```rust
#[test]
fn test_apply_set_permission_creates_grant() {
    let (conn, gate) = setup_gate_db();
    let op = make_set_permission("root_a", BOB, "writer");
    gate.apply_permission_op(&conn, &op).unwrap();

    let role = crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap();
    assert_eq!(role, Some(Role::Writer));
}
```

Run: `cargo test -p krillnotes-rbac test_apply_set_permission`
Expected: FAIL (apply_permission_op is a stub)

- [ ] **Step 2: Implement apply_permission_op for SetPermission**

```rust
fn apply_permission_op(
    &self,
    conn: &Connection,
    operation: &Operation,
) -> Result<(), PermissionError> {
    match operation {
        Operation::SetPermission {
            note_id, user_id, role, granted_by, ..
        } => {
            let note_id = note_id.as_ref()
                .ok_or_else(|| PermissionError::Denied("RBAC requires a note_id".into()))?;
            // Validate role
            Role::from_str(role)
                .ok_or_else(|| PermissionError::Denied(format!("invalid role: {}", role)))?;
            conn.execute(
                "INSERT INTO note_permissions (note_id, user_id, role, granted_by)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(note_id, user_id) DO UPDATE SET role = ?3, granted_by = ?4",
                rusqlite::params![note_id, user_id, role, granted_by],
            )?;
            // Cascade: if this is a demotion, downstream grants issued by
            // this user may now exceed their new role and need invalidation.
            // (e.g., Bob demoted from Owner to Writer → Bob's Owner grants cascade)
            self.cascade_revoke(conn, user_id)?;
            Ok(())
        }
        Operation::RevokePermission {
            note_id, user_id, ..
        } => {
            let note_id = note_id.as_ref()
                .ok_or_else(|| PermissionError::Denied("RBAC requires a note_id".into()))?;
            // Delete the direct grant
            conn.execute(
                "DELETE FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                rusqlite::params![note_id, user_id],
            )?;
            // Cascade: invalidate downstream grants issued by this user
            self.cascade_revoke(conn, user_id)?;
            Ok(())
        }
        Operation::RemovePeer { user_id, .. } => {
            // Delete ALL permission entries for this peer
            conn.execute(
                "DELETE FROM note_permissions WHERE user_id = ?1",
                rusqlite::params![user_id],
            )?;
            // Cascade: invalidate all downstream grants issued by this peer
            self.cascade_revoke(conn, user_id)?;
            Ok(())
        }
        Operation::TransferRootOwnership { new_owner, transferred_by, .. } => {
            // The Workspace updates its own owner_pubkey in workspace_meta.
            // The gate's job: grant the outgoing Root Owner "owner" on each
            // existing root note.
            let root_note_ids: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM notes WHERE parent_id IS NULL"
                )?;
                stmt.query_map([], |row| row.get(0))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            for root_id in root_note_ids {
                conn.execute(
                    "INSERT OR REPLACE INTO note_permissions (note_id, user_id, role, granted_by)
                     VALUES (?1, ?2, 'owner', ?3)",
                    rusqlite::params![root_id, transferred_by, new_owner],
                )?;
            }
            Ok(())
        }
        _ => Err(PermissionError::NotAPermissionOp),
    }
}
```

**Note:** After `TransferRootOwnership`, the `RbacGate.owner_pubkey` field becomes stale. The Workspace must update the gate's owner (via a `set_owner_pubkey` method or by reconstructing the gate). Alternatively, the gate can read `owner_pubkey` from `workspace_meta` on each `authorize()` call — one extra indexed query, always correct.

- [ ] **Step 3: Run test to verify SetPermission works**

Run: `cargo test -p krillnotes-rbac test_apply_set_permission`
Expected: PASS

- [ ] **Step 4: Write test — SetPermission upserts (changes existing role)**

```rust
#[test]
fn test_apply_set_permission_upserts() {
    let (conn, gate) = setup_gate_db();
    gate.apply_permission_op(&conn, &make_set_permission("root_a", BOB, "reader")).unwrap();
    gate.apply_permission_op(&conn, &make_set_permission("root_a", BOB, "owner")).unwrap();

    let role = crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap();
    assert_eq!(role, Some(Role::Owner));
}
```

- [ ] **Step 5: Write test — RevokePermission removes grant**

```rust
#[test]
fn test_apply_revoke_removes_grant() {
    let (conn, gate) = setup_gate_db();
    gate.apply_permission_op(&conn, &make_set_permission("root_a", BOB, "writer")).unwrap();
    gate.apply_permission_op(&conn, &make_revoke_permission("root_a", BOB)).unwrap();

    let role = crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap();
    assert_eq!(role, None);
}
```

- [ ] **Step 6: Write test — cascade revocation**

```rust
#[test]
fn test_cascade_revocation() {
    let (conn, gate) = setup_gate_db();
    // Alice (root owner) grants Bob owner
    grant_by(&conn, "root_a", BOB, "owner", ROOT_OWNER);
    // Bob grants Carol writer
    grant_by(&conn, "root_a", CAROL, "writer", BOB);

    // Revoke Bob's owner role
    gate.apply_permission_op(&conn, &make_revoke_permission("root_a", BOB)).unwrap();

    // Bob is gone
    assert_eq!(resolve_role(&conn, BOB, "root_a").unwrap(), None);
    // Carol's grant (issued by Bob who no longer holds owner) is cascaded
    assert_eq!(resolve_role(&conn, CAROL, "root_a").unwrap(), None);
}
```

- [ ] **Step 7: Implement cascade_revoke**

```rust
/// After revoking a user's grant, check all grants they issued.
/// If the granter no longer holds a sufficient role for the grant,
/// invalidate it and recurse.
fn cascade_revoke(
    &self,
    conn: &Connection,
    revoked_user: &str,
) -> Result<(), PermissionError> {
    // Find all grants issued by the revoked user
    let mut stmt = conn.prepare(
        "SELECT note_id, user_id, role FROM note_permissions WHERE granted_by = ?1"
    )?;
    let downstream: Vec<(String, String, String)> = stmt
        .query_map(rusqlite::params![revoked_user], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (note_id, user_id, granted_role) in downstream {
        // Check if the granter still holds a sufficient role
        let granter_role = crate::resolver::resolve_role(conn, revoked_user, &note_id)?;
        let granted = Role::from_str(&granted_role);

        let still_valid = match (granter_role, granted) {
            (Some(granter), Some(granted)) => granter >= granted,
            _ => false,
        };

        if !still_valid {
            // Invalidate this grant
            conn.execute(
                "DELETE FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                rusqlite::params![note_id, user_id],
            )?;
            // Recurse: check grants issued by the now-invalidated user
            self.cascade_revoke(conn, &user_id)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 8: Write test — demotion cascade (partial)**

```rust
#[test]
fn test_demotion_cascade_partial() {
    let (conn, gate) = setup_gate_db();
    // Bob is owner, grants Carol owner and Dave reader
    grant_by(&conn, "root_a", BOB, "owner", ROOT_OWNER);
    grant_by(&conn, "root_a", CAROL, "owner", BOB);
    grant_by(&conn, "root_a", "dave", "reader", BOB);

    // Demote Bob to writer (delete + re-grant)
    conn.execute("DELETE FROM note_permissions WHERE note_id = 'root_a' AND user_id = ?1", [BOB]).unwrap();
    gate.apply_permission_op(&conn, &make_set_permission_by("root_a", BOB, "writer", ROOT_OWNER)).unwrap();
    // Manually trigger cascade for Bob
    gate.cascade_revoke_public(&conn, BOB).unwrap();

    // Carol's owner grant exceeds Bob's new writer → invalidated
    assert_eq!(resolve_role(&conn, CAROL, "root_a").unwrap(), None);
    // Dave's reader grant is within Bob's writer → still valid
    assert_eq!(resolve_role(&conn, "dave", "root_a").unwrap(), Some(Role::Reader));
}
```

Note: `cascade_revoke_public` is a `#[cfg(test)] pub` wrapper around `cascade_revoke` for testing.

- [ ] **Step 9: Run all gate tests**

Run: `cargo test -p krillnotes-rbac gate_tests`
Expected: all pass

- [ ] **Step 10: Commit**

```bash
git add krillnotes-rbac/src/gate.rs krillnotes-rbac/src/tests/gate_tests.rs
git commit -m "feat(rbac): implement apply_permission_op with grant, revoke, and cascade"
```

---

## Task 6: New Operation Variants — RemovePeer + TransferRootOwnership

**Files:**
- Modify: `krillnotes-core/src/core/operation.rs`
- Modify: `krillnotes-core/src/core/operation_tests.rs`

- [ ] **Step 1: Add RemovePeer variant**

In `operation.rs`, add after the `JoinWorkspace` variant:

```rust
    /// Remove a peer from the workspace entirely.
    /// Root Owner only. Revokes all grants and cuts off sync.
    RemovePeer {
        operation_id: String,
        timestamp: HlcTimestamp,
        device_id: String,
        /// Public key of the peer being removed.
        user_id: String,
        /// Public key of the Root Owner performing the removal.
        removed_by: String,
        signature: String,
    },

    /// Transfer root ownership to another peer.
    /// Root Owner only. Recipient must be an existing peer.
    TransferRootOwnership {
        operation_id: String,
        timestamp: HlcTimestamp,
        device_id: String,
        /// Public key of the new Root Owner.
        new_owner: String,
        /// Public key of the current (outgoing) Root Owner.
        transferred_by: String,
        signature: String,
    },
```

- [ ] **Step 2: Update all Operation impl match arms**

Update each method in the `impl Operation` block to include the new variants:
- `operation_id()` — extract `operation_id`
- `timestamp()` — extract `timestamp`
- `device_id()` — extract `device_id`
- `author_key()` — return `removed_by` / `transferred_by`
- `set_author_key()` — set `removed_by` / `transferred_by`
- `set_signature()` — set `signature`
- `get_signature()` — return `signature`

Follow the exact pattern used by `SetPermission` and `RevokePermission` as templates.

- [ ] **Step 3: Verify core compiles**

Run: `cargo check -p krillnotes-core`
Expected: compiles. The RBAC gate's `resolve_scope` match in `gate.rs` will also need updating — add the new variants to the workspace-level (returns `None`) arm.

- [ ] **Step 4: Add serialization roundtrip tests**

In `operation_tests.rs`, add tests following the existing pattern (see the SetPermission/RevokePermission roundtrip tests as template):

```rust
#[test]
fn test_remove_peer_roundtrip() {
    let op = Operation::RemovePeer {
        operation_id: "test-id".into(),
        timestamp: test_timestamp(),
        device_id: "dev1".into(),
        user_id: "bob_pubkey".into(),
        removed_by: "alice_pubkey".into(),
        signature: "sig".into(),
    };
    let json = serde_json::to_string(&op).unwrap();
    let restored: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(op.operation_id(), restored.operation_id());
}
```

Similar test for `TransferRootOwnership`.

- [ ] **Step 5: Run all core tests**

Run: `cargo test -p krillnotes-core`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/operation.rs krillnotes-core/src/core/operation_tests.rs
git commit -m "feat(core): add RemovePeer and TransferRootOwnership operation variants"
```

---

## Task 7: Workspace Integration

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`
- Modify: `krillnotes-core/src/core/workspace/notes.rs` (if note mutations live here)

This is the wiring task — adding the permission gate to `Workspace` and calling `authorize()` before every mutation.

- [ ] **Step 1: Add permission_gate field to Workspace struct**

In `workspace/mod.rs`, add to the `Workspace` struct (around line 65):

```rust
    permission_gate: Option<Box<dyn krillnotes_core::core::permission::PermissionGate>>,
```

- [ ] **Step 2: Update constructors to accept the gate**

Update `Workspace::create()` and `Workspace::open()` signatures to accept an optional gate:

```rust
pub fn create<P: AsRef<Path>>(
    path: P,
    password: &str,
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
    permission_gate: Option<Box<dyn crate::core::permission::PermissionGate>>,
) -> Result<Self> {
```

In the constructor body, after core schema migrations, call `ensure_schema`:
```rust
if let Some(gate) = &permission_gate {
    gate.ensure_schema(conn)?;
}
```

Store in the struct:
```rust
Self {
    // ... existing fields ...
    permission_gate,
}
```

Do the same for `Workspace::open()`.

- [ ] **Step 3: Add a private authorize helper method**

```rust
impl Workspace {
    /// Check permission before applying an operation.
    /// No-op if no permission gate is installed (single-user/test mode).
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

    /// Apply a permission-modifying operation through the gate.
    fn apply_permission_op(&self, tx: &rusqlite::Transaction, operation: &Operation) -> Result<()> {
        if let Some(gate) = &self.permission_gate {
            gate.apply_permission_op(tx, operation)?;
        }
        Ok(())
    }
}
```

Note: `apply_permission_op` receives `&Transaction` but the trait takes `&Connection`. Since `Transaction` derefs to `Connection`, this works directly.

- [ ] **Step 4: Add authorize() calls to existing mutation methods**

For each mutating workspace method, add the authorize check **after** building the operation struct but **before** creating the transaction. This is critical — `self.authorize()` borrows `self.storage.connection()` (immutable), so it must complete before `self.storage.connection_mut().transaction()` takes a mutable borrow.

The pattern:

```rust
// In create_note, update_field, delete_note, move_note, set_tags, etc.:
let op = Operation::CreateNote { /* ... */ };
self.authorize(&op)?;  // Must happen BEFORE the transaction
let tx = self.storage.connection_mut().transaction()?;
// ... then proceed with transaction
```

For SetPermission/RevokePermission operations, also call `apply_permission_op`:
```rust
match &op {
    Operation::SetPermission { .. } | Operation::RevokePermission { .. } => {
        self.apply_permission_op(&tx, &op)?;
    }
    _ => {}
}
```

**Important:** Do NOT add authorize calls to methods that are only called for the workspace owner (like script CRUD) — those already check `self.owner_pubkey`. But DO add the gate check for consistency so the gate can enforce its own rules.

- [ ] **Step 5: Update all Workspace constructors and their call sites**

There are **four** constructors that need the `permission_gate` parameter:
- `Workspace::create()` (line ~126)
- `Workspace::create_with_id()` (line ~330)
- `Workspace::create_empty_with_id()` (line ~682)
- `Workspace::open()` (line ~788)

All call sites that need updating (pass `None` for core tests/internals, `Some(RbacGate)` for desktop):
- `krillnotes-core/src/core/export.rs` (~line 509) — `Workspace::open` in import
- `krillnotes-core/src/core/workspace/tests.rs` — all test workspace construction
- `krillnotes-core/src/core/export_tests.rs` — multiple call sites
- `krillnotes-core/tests/watermark_recovery.rs` (~lines 238, 312, 381)
- `krillnotes-core/tests/relay_integration.rs` (~lines 290, 297)
- `krillnotes-desktop/src-tauri/src/commands/swarm.rs` (~line 427) — `create_empty_with_id`
- `krillnotes-desktop/src-tauri/src/lib.rs` — workspace open/create commands

For core tests and internal callers, pass `None`:
```rust
Workspace::create(path, password, identity_uuid, signing_key, None)
```

- [ ] **Step 6: Verify all core tests still pass**

Run: `cargo test -p krillnotes-core`
Expected: all pass (None gate = no enforcement = existing behavior preserved)

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/workspace/
git commit -m "feat(core): wire PermissionGate into Workspace with authorize() calls"
```

---

## Task 8: Desktop Integration

**Files:**
- Modify: `krillnotes-desktop/src-tauri/Cargo.toml`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Step 1: Add krillnotes-rbac dependency**

In `krillnotes-desktop/src-tauri/Cargo.toml`, add:
```toml
krillnotes-rbac = { path = "../../krillnotes-rbac" }
```

- [ ] **Step 2: Pass RbacGate when creating workspaces**

In `lib.rs`, find every call to `Workspace::create()` and `Workspace::open()`. Update to pass the RBAC gate:

```rust
use krillnotes_rbac::RbacGate;

// When creating a new workspace:
let gate = Box::new(RbacGate::new(owner_pubkey.clone()));
let workspace = Workspace::create(path, password, identity_uuid, signing_key, Some(gate))?;

// When opening an existing workspace:
let gate = Box::new(RbacGate::new(owner_pubkey.clone()));
let workspace = Workspace::open(path, password, identity_uuid, signing_key, Some(gate))?;
```

The `owner_pubkey` is the base64-encoded public key of the identity that created (or owns) the workspace. This is available from the `signing_key` for workspace creation, or from `workspace_meta` for existing workspaces.

- [ ] **Step 3: Verify desktop builds**

Run: `cd krillnotes-desktop && npm run tauri build`
Expected: builds successfully

- [ ] **Step 4: Verify existing behavior unchanged**

Run: `cd krillnotes-desktop && npm run tauri dev`
Expected: app works normally. Since the local user is always the workspace creator (Root Owner), all operations pass the RBAC gate. No UI changes needed for this task.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/Cargo.toml krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): wire RbacGate into workspace creation"
```

---

## Task 9: Integration Tests — Full RBAC Scenarios

**Files:**
- Create: `krillnotes-rbac/src/tests/integration_tests.rs`
- Modify: `krillnotes-rbac/src/tests/mod.rs`

End-to-end tests that exercise the full gate lifecycle: schema creation, granting, authorization, revocation, cascade. These tests simulate multi-peer scenarios.

- [ ] **Step 1: Write test — full invitation + authorization lifecycle**

```rust
#[test]
fn test_full_lifecycle_invite_and_authorize() {
    let (conn, gate) = setup_gate_db();
    gate.ensure_schema(&conn).unwrap();

    // Root Owner grants Bob owner on root_a
    let grant_op = make_set_permission("root_a", BOB, "owner");
    gate.authorize(&conn, ROOT_OWNER, &grant_op).unwrap();
    gate.apply_permission_op(&conn, &grant_op).unwrap();

    // Bob (now owner) grants Carol writer on root_a
    let grant_op2 = make_set_permission_by("root_a", CAROL, "writer", BOB);
    gate.authorize(&conn, BOB, &grant_op2).unwrap();
    gate.apply_permission_op(&conn, &grant_op2).unwrap();

    // Carol can create and edit
    assert!(gate.authorize(&conn, CAROL, &make_create_note("root_a")).is_ok());
    assert!(gate.authorize(&conn, CAROL, &make_update_field("child_1")).is_ok());

    // Carol cannot delete bob's note
    assert!(gate.authorize(&conn, CAROL, &make_delete_note("child_1")).is_err());

    // Carol cannot set permissions
    assert!(gate.authorize(&conn, CAROL, &make_set_permission("root_a", "dave", "reader")).is_err());
}
```

- [ ] **Step 2: Write test — cascade revocation end-to-end**

```rust
#[test]
fn test_cascade_revocation_end_to_end() {
    let (conn, gate) = setup_gate_db();
    gate.ensure_schema(&conn).unwrap();

    // Chain: root_owner → bob (owner) → carol (writer)
    let g1 = make_set_permission("root_a", BOB, "owner");
    gate.authorize(&conn, ROOT_OWNER, &g1).unwrap();
    gate.apply_permission_op(&conn, &g1).unwrap();

    let g2 = make_set_permission_by("root_a", CAROL, "writer", BOB);
    gate.authorize(&conn, BOB, &g2).unwrap();
    gate.apply_permission_op(&conn, &g2).unwrap();

    // Carol can create
    assert!(gate.authorize(&conn, CAROL, &make_create_note("root_a")).is_ok());

    // Revoke Bob → cascades to Carol
    let revoke = make_revoke_permission("root_a", BOB);
    gate.authorize(&conn, ROOT_OWNER, &revoke).unwrap();
    gate.apply_permission_op(&conn, &revoke).unwrap();

    // Carol is now denied
    assert!(gate.authorize(&conn, CAROL, &make_create_note("root_a")).is_err());
}
```

- [ ] **Step 3: Write test — multi-subtree isolation**

```rust
#[test]
fn test_multi_subtree_isolation() {
    let (conn, gate) = setup_gate_db();
    gate.ensure_schema(&conn).unwrap();

    // Bob is owner on root_a, no access to root_b
    let g1 = make_set_permission("root_a", BOB, "owner");
    gate.authorize(&conn, ROOT_OWNER, &g1).unwrap();
    gate.apply_permission_op(&conn, &g1).unwrap();

    // Bob can operate on root_a subtree
    assert!(gate.authorize(&conn, BOB, &make_create_note("root_a")).is_ok());

    // Bob cannot operate on root_b subtree
    assert!(gate.authorize(&conn, BOB, &make_create_note("root_b")).is_err());
}
```

- [ ] **Step 4: Run all tests across both crates**

Run: `cargo test -p krillnotes-rbac && cargo test -p krillnotes-core`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/tests/
git commit -m "test(rbac): add integration tests for full RBAC lifecycle scenarios"
```

---

## Summary

| Task | Description | Key outcome |
|------|-------------|-------------|
| 1 | PermissionGate trait + PermissionError | Plugin seam established in krillnotes-core |
| 2 | krillnotes-rbac crate scaffolding | New crate with stubs, compiles |
| 3 | Tree-walk resolver (TDD) | Core algorithm with 6+ test cases |
| 4 | authorize() — permission matrix (TDD) | Full operation matrix with 10+ test cases |
| 5 | apply_permission_op — grant/revoke/cascade (TDD) | Upsert, delete, cascade with 5+ tests |
| 6 | New operation variants | RemovePeer + TransferRootOwnership |
| 7 | Workspace integration | Gate wired in, authorize calls on all mutations |
| 8 | Desktop integration | RbacGate passed at workspace creation |
| 9 | Integration tests | End-to-end multi-peer scenarios |

**Tasks 1-2 are sequential.** Tasks 3, 4, 5 are sequential (each builds on the previous). Task 6 is independent of 3-5. Tasks 7-8 depend on 1-6. Task 9 depends on all previous tasks.

**Parallelization opportunity:** Task 6 (new operation variants) can run in parallel with Tasks 3-5 (RBAC crate implementation), since they touch different crates.
