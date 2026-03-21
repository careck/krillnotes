# RBAC Backend: Permission Queries + Cascade Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add backend query methods, Tauri commands, and TS types for the RBAC permission UI, refactor cascade_revoke to be opt-in, fix MoveNote destination authorization, and clean up grants on note deletion.

**Architecture:** New query methods live in `krillnotes-rbac/src/resolver.rs` (pure SQL queries) and `krillnotes-core/src/core/workspace/permissions.rs` (workspace-level wrappers). New Tauri commands in `krillnotes-desktop/src-tauri/src/commands/permissions.rs`. The `cascade_revoke` auto-call is removed from `RbacGate::apply_permission_op` and replaced with a read-only `preview_cascade` query.

**Tech Stack:** Rust, rusqlite, serde, Tauri v2, TypeScript

**Spec:** `docs/plans/2026-03-22-rbac-ui-design.md` (Plan A of 3 — this plan unblocks Plans B and C)

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `krillnotes-rbac/src/queries.rs` | Read-only permission queries: `get_note_permissions`, `get_effective_role`, `get_inherited_permissions`, `get_all_effective_roles`, `preview_cascade` |
| `krillnotes-core/src/core/workspace/permissions.rs` | Workspace methods wrapping query + mutation operations for permissions |
| `krillnotes-rbac/src/tests/query_tests.rs` | Tests for all new query functions |
| `krillnotes-desktop/src-tauri/src/commands/permissions.rs` | Tauri commands exposing permission queries + mutations to the frontend |
| `krillnotes-desktop/src/types.ts` (modify) | Add TS types: `PermissionGrant`, `EffectiveRole`, `CascadeImpact` |

### Modified files

| File | Change |
|------|--------|
| `krillnotes-rbac/src/lib.rs` | Export `queries` module and `Role` from resolver |
| `krillnotes-rbac/src/gate.rs` | Remove `cascade_revoke` calls from `apply_permission_op`; add MoveNote destination check in `check_role_for_operation` |
| `krillnotes-rbac/src/resolver.rs` | Make `Role` and `resolve_role` public (already pub, but ensure re-exported) |
| `krillnotes-core/src/core/workspace/mod.rs` | Add `mod permissions;` and expose new methods |
| `krillnotes-core/src/core/workspace/notes.rs` | Add `note_permissions` cleanup in `delete_note_recursive` and `delete_note_promote` |
| `krillnotes-desktop/src-tauri/src/commands/mod.rs` | Add `pub mod permissions;` |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Register new permission commands in `generate_handler![]` |

---

### Task 1: Add `get_note_permissions` query

**Files:**
- Create: `krillnotes-rbac/src/queries.rs`
- Create: `krillnotes-rbac/src/tests/query_tests.rs`
- Modify: `krillnotes-rbac/src/lib.rs`

- [ ] **Step 1: Create query module with `get_note_permissions` and write failing test**

Create `krillnotes-rbac/src/tests/query_tests.rs`:

```rust
use crate::queries::PermissionGrantRow;
use crate::queries;
use crate::tests::helpers::*; // reuse existing test helpers

#[test]
fn test_get_note_permissions_returns_anchored_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    // Insert two grants anchored at note-1
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    ).unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "bob_key", "reader", "root_pubkey"],
    ).unwrap();

    let grants = queries::get_note_permissions(&conn, "note-1").unwrap();
    assert_eq!(grants.len(), 2);
    assert!(grants.iter().any(|g| g.user_id == "alice_key" && g.role == "writer"));
    assert!(grants.iter().any(|g| g.user_id == "bob_key" && g.role == "reader"));
}

#[test]
fn test_get_note_permissions_empty_for_no_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    let grants = queries::get_note_permissions(&conn, "note-1").unwrap();
    assert!(grants.is_empty());
}
```

Create `krillnotes-rbac/src/queries.rs`:

```rust
use rusqlite::Connection;

/// A single permission grant row from note_permissions.
#[derive(Debug, Clone)]
pub struct PermissionGrantRow {
    pub note_id: String,
    pub user_id: String,
    pub role: String,
    pub granted_by: String,
}

/// Returns all explicit grants anchored at `note_id`.
pub fn get_note_permissions(
    conn: &Connection,
    note_id: &str,
) -> Result<Vec<PermissionGrantRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT note_id, user_id, role, granted_by FROM note_permissions WHERE note_id = ?1",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![note_id], |row| {
            Ok(PermissionGrantRow {
                note_id: row.get(0)?,
                user_id: row.get(1)?,
                role: row.get(2)?,
                granted_by: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
```

- [ ] **Step 2: Update `lib.rs` to export new module**

In `krillnotes-rbac/src/lib.rs`, add:

```rust
pub mod queries;
pub mod resolver;  // change from `mod resolver` to `pub mod resolver`
```

This makes `queries` and `resolver::Role` available to downstream crates.

- [ ] **Step 3: Register test module**

In `krillnotes-rbac/src/tests/mod.rs`, add:

```rust
mod query_tests;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-rbac query_tests`

Expected: 2 tests pass. If test helpers need adjustment (e.g. `setup_workspace_with_gate`, `insert_note`), check `krillnotes-rbac/src/tests/gate_tests.rs` for patterns and extract shared helpers.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/queries.rs krillnotes-rbac/src/tests/query_tests.rs krillnotes-rbac/src/lib.rs krillnotes-rbac/src/tests/mod.rs
git commit -m "feat(rbac): add get_note_permissions query"
```

---

### Task 2: Add `get_effective_role` query

**Files:**
- Modify: `krillnotes-rbac/src/queries.rs`
- Modify: `krillnotes-rbac/src/tests/query_tests.rs`

- [ ] **Step 1: Write failing test**

Add to `query_tests.rs`:

```rust
use crate::queries::EffectiveRoleInfo;

#[test]
fn test_get_effective_role_direct_grant() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    ).unwrap();

    let info = queries::get_effective_role(&conn, "alice_key", "note-1", "root_pubkey").unwrap();
    assert_eq!(info.role, "writer");
    assert!(info.inherited_from.is_none()); // anchored here, not inherited
    assert_eq!(info.granted_by.as_deref(), Some("root_pubkey"));
}

#[test]
fn test_get_effective_role_inherited_from_parent() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "parent", None);
    insert_note(&conn, "child", Some("parent"));

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["parent", "alice_key", "writer", "root_pubkey"],
    ).unwrap();

    let info = queries::get_effective_role(&conn, "alice_key", "child", "root_pubkey").unwrap();
    assert_eq!(info.role, "writer");
    assert_eq!(info.inherited_from.as_deref(), Some("parent"));
}

#[test]
fn test_get_effective_role_root_owner() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    let info = queries::get_effective_role(&conn, "root_pubkey", "note-1", "root_pubkey").unwrap();
    assert_eq!(info.role, "root_owner");
    assert!(info.inherited_from.is_none());
    assert!(info.granted_by.is_none());
}

#[test]
fn test_get_effective_role_no_access() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    let info = queries::get_effective_role(&conn, "stranger_key", "note-1", "root_pubkey").unwrap();
    assert_eq!(info.role, "none");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-rbac test_get_effective_role`

Expected: FAIL — `EffectiveRoleInfo` and `get_effective_role` not defined yet.

- [ ] **Step 3: Implement `get_effective_role`**

Add to `krillnotes-rbac/src/queries.rs`:

```rust
use crate::resolver::Role;
use rusqlite::OptionalExtension;

/// Extended role info including where the grant was anchored.
#[derive(Debug, Clone)]
pub struct EffectiveRoleInfo {
    /// "owner", "writer", "reader", "root_owner", or "none"
    pub role: String,
    /// note_id where the grant is anchored, None if root_owner or no access
    pub inherited_from: Option<String>,
    /// Title of the anchor note (for display)
    pub inherited_from_title: Option<String>,
    /// Public key of who granted access, None if root_owner
    pub granted_by: Option<String>,
}

/// Returns the effective role for `user_id` on `note_id`, including
/// which ancestor the grant is inherited from.
///
/// `owner_pubkey` is used to detect root owner (bypasses resolver).
pub fn get_effective_role(
    conn: &Connection,
    user_id: &str,
    note_id: &str,
    owner_pubkey: &str,
) -> Result<EffectiveRoleInfo, rusqlite::Error> {
    // Root owner short-circuit
    if user_id == owner_pubkey {
        return Ok(EffectiveRoleInfo {
            role: "root_owner".to_string(),
            inherited_from: None,
            inherited_from_title: None,
            granted_by: None,
        });
    }

    let mut current_id = Some(note_id.to_string());
    while let Some(id) = current_id {
        // Check for explicit grant at this node
        let grant: Option<(String, String)> = conn
            .query_row(
                "SELECT role, granted_by FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                rusqlite::params![id, user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((role, granted_by)) = grant {
            let inherited_from = if id != note_id { Some(id.clone()) } else { None };
            let inherited_from_title = if let Some(ref anchor_id) = inherited_from {
                conn.query_row(
                    "SELECT title FROM notes WHERE id = ?1",
                    [anchor_id],
                    |row| row.get(0),
                ).optional()?
            } else {
                None
            };
            return Ok(EffectiveRoleInfo {
                role,
                inherited_from,
                inherited_from_title,
                granted_by: Some(granted_by),
            });
        }

        // Walk up
        current_id = conn
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
    }

    Ok(EffectiveRoleInfo {
        role: "none".to_string(),
        inherited_from: None,
        inherited_from_title: None,
        granted_by: None,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-rbac test_get_effective_role`

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/queries.rs krillnotes-rbac/src/tests/query_tests.rs
git commit -m "feat(rbac): add get_effective_role query with inheritance tracking"
```

---

### Task 3: Add `get_inherited_permissions` query

**Files:**
- Modify: `krillnotes-rbac/src/queries.rs`
- Modify: `krillnotes-rbac/src/tests/query_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
use crate::queries::InheritedGrant;

#[test]
fn test_get_inherited_permissions_from_ancestors() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "grandparent", None);
    insert_note(&conn, "parent", Some("grandparent"));
    insert_note(&conn, "child", Some("parent"));

    // Grant on grandparent
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["grandparent", "alice_key", "owner", "root_pubkey"],
    ).unwrap();
    // Grant on parent
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["parent", "bob_key", "writer", "alice_key"],
    ).unwrap();

    let inherited = queries::get_inherited_permissions(&conn, "child").unwrap();
    assert_eq!(inherited.len(), 2);
    assert!(inherited.iter().any(|g| g.grant.user_id == "alice_key" && g.anchor_note_id == "grandparent"));
    assert!(inherited.iter().any(|g| g.grant.user_id == "bob_key" && g.anchor_note_id == "parent"));
}

#[test]
fn test_get_inherited_permissions_excludes_grants_on_self() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    ).unwrap();

    // Grants anchored on the note itself are NOT inherited
    let inherited = queries::get_inherited_permissions(&conn, "note-1").unwrap();
    assert!(inherited.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-rbac test_get_inherited_permissions`

- [ ] **Step 3: Implement `get_inherited_permissions`**

Add to `queries.rs`:

```rust
/// A grant inherited from an ancestor, with the anchor location.
#[derive(Debug, Clone)]
pub struct InheritedGrant {
    pub grant: PermissionGrantRow,
    pub anchor_note_id: String,
    pub anchor_note_title: Option<String>,
}

/// Walk up from `note_id` to root, collecting all grants from
/// ancestor nodes (excluding grants anchored on `note_id` itself).
pub fn get_inherited_permissions(
    conn: &Connection,
    note_id: &str,
) -> Result<Vec<InheritedGrant>, rusqlite::Error> {
    let mut results = Vec::new();

    // Start from parent, not self
    let mut current_id: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM notes WHERE id = ?1",
            [note_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    while let Some(id) = current_id {
        let title: Option<String> = conn
            .query_row("SELECT title FROM notes WHERE id = ?1", [&id], |row| row.get(0))
            .optional()?;

        let mut stmt = conn.prepare(
            "SELECT note_id, user_id, role, granted_by FROM note_permissions WHERE note_id = ?1",
        )?;
        let grants = stmt
            .query_map([&id], |row| {
                Ok(PermissionGrantRow {
                    note_id: row.get(0)?,
                    user_id: row.get(1)?,
                    role: row.get(2)?,
                    granted_by: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for grant in grants {
            results.push(InheritedGrant {
                grant,
                anchor_note_id: id.clone(),
                anchor_note_title: title.clone(),
            });
        }

        // Walk up
        current_id = conn
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                [&id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
    }

    Ok(results)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-rbac test_get_inherited_permissions`

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/queries.rs krillnotes-rbac/src/tests/query_tests.rs
git commit -m "feat(rbac): add get_inherited_permissions query"
```

---

### Task 4: Add `get_all_effective_roles` batch query

**Files:**
- Modify: `krillnotes-rbac/src/queries.rs`
- Modify: `krillnotes-rbac/src/tests/query_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
use std::collections::HashMap;

#[test]
fn test_get_all_effective_roles_propagates_down_tree() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "root", None);
    insert_note(&conn, "child1", Some("root"));
    insert_note(&conn, "child2", Some("root"));
    insert_note(&conn, "grandchild", Some("child1"));

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["root", "alice_key", "writer", "root_pubkey"],
    ).unwrap();

    let roles = queries::get_all_effective_roles(&conn, "alice_key", "root_pubkey").unwrap();
    assert_eq!(roles.get("root").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("child1").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("child2").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("grandchild").map(|r| r.as_str()), Some("writer"));
}

#[test]
fn test_get_all_effective_roles_closer_grant_overrides() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "root", None);
    insert_note(&conn, "child", Some("root"));

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["root", "alice_key", "writer", "root_pubkey"],
    ).unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["child", "alice_key", "reader", "root_pubkey"],
    ).unwrap();

    let roles = queries::get_all_effective_roles(&conn, "alice_key", "root_pubkey").unwrap();
    assert_eq!(roles.get("root").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("child").map(|r| r.as_str()), Some("reader")); // overridden
}

#[test]
fn test_get_all_effective_roles_root_owner_all_owner() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "root", None);
    insert_note(&conn, "child", Some("root"));

    let roles = queries::get_all_effective_roles(&conn, "root_pubkey", "root_pubkey").unwrap();
    assert_eq!(roles.get("root").map(|r| r.as_str()), Some("root_owner"));
    assert_eq!(roles.get("child").map(|r| r.as_str()), Some("root_owner"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-rbac test_get_all_effective_roles`

- [ ] **Step 3: Implement `get_all_effective_roles`**

Add to `queries.rs`:

```rust
use std::collections::HashMap;

/// Batch-compute effective role for every note in the workspace.
/// Uses top-down grant propagation to avoid O(N×D) per-note walks.
///
/// Algorithm:
/// 1. Root owner short-circuit: return "root_owner" for all notes.
/// 2. Fetch all grants for `user_id` from `note_permissions`.
/// 3. Build parent→children adjacency from the `notes` table.
/// 4. For each grant anchor, BFS/DFS downward marking descendants,
///    but stop descending into subtrees that have their own grant
///    (closer grant wins).
pub fn get_all_effective_roles(
    conn: &Connection,
    user_id: &str,
    owner_pubkey: &str,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    // 1. Root owner: every note gets "root_owner"
    if user_id == owner_pubkey {
        let mut result = HashMap::new();
        let mut stmt = conn.prepare("SELECT id FROM notes")?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in ids {
            result.insert(id, "root_owner".to_string());
        }
        return Ok(result);
    }

    // 2. Fetch all grants for this user
    let mut stmt = conn.prepare(
        "SELECT note_id, role FROM note_permissions WHERE user_id = ?1",
    )?;
    let grants: Vec<(String, String)> = stmt
        .query_map([user_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    if grants.is_empty() {
        return Ok(HashMap::new());
    }

    // Collect grant anchor note_ids for quick lookup
    let grant_anchors: HashMap<String, String> = grants.into_iter().collect();

    // 3. Build parent→children adjacency
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, parent_id FROM notes")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (id, parent_id) in &rows {
        if let Some(pid) = parent_id {
            children_map.entry(pid.clone()).or_default().push(id.clone());
        }
    }

    // 4. BFS from each grant anchor downward
    let mut result = HashMap::new();
    for (anchor_id, role) in &grant_anchors {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(anchor_id.clone());

        while let Some(current) = queue.pop_front() {
            // If this node has its own grant and it's not the starting anchor, skip
            if current != *anchor_id && grant_anchors.contains_key(&current) {
                continue;
            }
            result.insert(current.clone(), role.clone());

            if let Some(children) = children_map.get(&current) {
                for child in children {
                    queue.push_back(child.clone());
                }
            }
        }
    }

    Ok(result)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-rbac test_get_all_effective_roles`

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/queries.rs krillnotes-rbac/src/tests/query_tests.rs
git commit -m "feat(rbac): add get_all_effective_roles batch query for tree dots"
```

---

### Task 5: Add `preview_cascade` query

**Files:**
- Modify: `krillnotes-rbac/src/queries.rs`
- Modify: `krillnotes-rbac/src/tests/query_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_preview_cascade_shows_invalidated_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    // Alice is owner, granted Bob writer and Carol reader
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "owner", "root_pubkey"],
    ).unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "bob_key", "writer", "alice_key"],
    ).unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "carol_key", "reader", "alice_key"],
    ).unwrap();

    // Preview: demote alice to reader — she can no longer grant anything
    let impact = queries::preview_cascade(&conn, "alice_key").unwrap();
    assert_eq!(impact.len(), 2); // bob and carol both affected
    assert!(impact.iter().any(|g| g.user_id == "bob_key"));
    assert!(impact.iter().any(|g| g.user_id == "carol_key"));
}

#[test]
fn test_preview_cascade_no_downstream_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    ).unwrap();

    // Alice has no downstream grants
    let impact = queries::preview_cascade(&conn, "alice_key").unwrap();
    assert!(impact.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-rbac test_preview_cascade`

- [ ] **Step 3: Implement `preview_cascade`**

Add to `queries.rs`:

```rust
/// Returns all grants issued by `user_id` that would become invalid
/// if their role were changed. Since only Owners can grant, ANY demotion
/// from Owner invalidates all grants they issued.
///
/// This is a read-only preview — no data is modified.
pub fn preview_cascade(
    conn: &Connection,
    user_id: &str,
) -> Result<Vec<PermissionGrantRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT note_id, user_id, role, granted_by FROM note_permissions WHERE granted_by = ?1",
    )?;
    let rows = stmt
        .query_map([user_id], |row| {
            Ok(PermissionGrantRow {
                note_id: row.get(0)?,
                user_id: row.get(1)?,
                role: row.get(2)?,
                granted_by: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
```

Note: Since only Owners can grant (`require_at_least(role, Role::Owner)` in gate.rs:87), any demotion from Owner means ALL downstream grants become invalid. We return all grants issued by this user — the UI decides which to actually revoke.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-rbac test_preview_cascade`

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/queries.rs krillnotes-rbac/src/tests/query_tests.rs
git commit -m "feat(rbac): add preview_cascade read-only query"
```

---

### Task 6: Remove auto-cascade from `apply_permission_op`

**Files:**
- Modify: `krillnotes-rbac/src/gate.rs`
- Modify: `krillnotes-rbac/src/tests/gate_tests.rs`
- Modify: `krillnotes-rbac/src/tests/integration_tests.rs`

- [ ] **Step 1: Remove `cascade_revoke` calls from `apply_permission_op`**

In `krillnotes-rbac/src/gate.rs`, remove the `self.cascade_revoke()` calls at:
- Line 244 (after SetPermission insert)
- Line 257 (after RevokePermission delete)
- Line 265 (after RemovePeer delete)

The `cascade_revoke` method itself stays (it's used by the `cascade_revoke_public` test helper and may be useful later), but it is no longer called automatically.

- [ ] **Step 2: Update tests that expected auto-cascade behavior**

In `gate_tests.rs` and `integration_tests.rs`, find tests that assert on cascade behavior (e.g., `test_cascade_revocation_end_to_end`). Update them to verify that downstream grants are **NOT** automatically revoked. The grants should still exist after a demotion.

For example, `test_cascade_revocation_end_to_end` should now verify:

```rust
// After revoking alice, bob's grant (issued by alice) should STILL exist
// (opt-in cascade — UI decides, not the backend)
let bob_grants = queries::get_note_permissions(&conn, "note-1").unwrap();
assert!(bob_grants.iter().any(|g| g.user_id == "bob_key"));
```

- [ ] **Step 3: Run all rbac tests**

Run: `cargo test -p krillnotes-rbac`

Expected: All tests pass. Some tests may need adjustment to reflect the new opt-in cascade behavior.

- [ ] **Step 4: Run full core tests to check for regressions**

Run: `cargo test -p krillnotes-core`

Expected: All pass. The core crate calls `apply_permission_op` via the gate — removing auto-cascade should not break any core tests since they don't depend on cascade behavior.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/gate.rs krillnotes-rbac/src/tests/
git commit -m "refactor(rbac): remove auto-cascade from apply_permission_op

Cascade is now opt-in: the UI previews impact and the user
decides which downstream grants to revoke. The backend only
applies explicit RevokePermission operations."
```

---

### Task 7: Fix MoveNote destination scope check

**Files:**
- Modify: `krillnotes-rbac/src/gate.rs`
- Modify: `krillnotes-rbac/src/tests/gate_tests.rs`

- [ ] **Step 1: Write failing test**

Add to `gate_tests.rs`:

```rust
#[test]
fn test_move_note_denied_to_inaccessible_destination() {
    let (conn, gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "subtree-a", None);
    insert_note(&conn, "note-1", Some("subtree-a"));
    insert_note(&conn, "subtree-b", None);

    // Alice is writer on subtree-a only
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["subtree-a", "alice_key", "writer", "root_pubkey"],
    ).unwrap();
    // Alice authored note-1 so she can move it
    conn.execute(
        "UPDATE notes SET created_by = 'alice_key' WHERE id = 'note-1'",
        [],
    ).unwrap();

    let op = make_move_note_op("note-1", "subtree-b", "alice_key");
    let result = gate.authorize(&conn, "alice_key", &op);
    assert!(result.is_err()); // denied — no access to subtree-b
}
```

- [ ] **Step 2: Run test to verify it fails (currently passes because destination isn't checked)**

Run: `cargo test -p krillnotes-rbac test_move_note_denied_to_inaccessible_destination`

Expected: FAIL — the test should fail because the current code allows the move (only checks source).

- [ ] **Step 3: Add destination check in `check_role_for_operation`**

In `gate.rs`, modify the `MoveNote` arm of `check_role_for_operation` (around line 76):

```rust
Operation::MoveNote { note_id, new_parent_id, .. } => {
    if role < Role::Owner {
        self.require_authorship(conn, actor, note_id, role)?;
    }
    // Check destination scope
    if let Some(dest_id) = new_parent_id {
        let dest_role = crate::resolver::resolve_role(conn, actor, dest_id)?
            .ok_or_else(|| PermissionError::Denied(
                "no access to move destination".into(),
            ))?;
        require_at_least(dest_role, Role::Writer)?;
    }
}
```

Note: Check the exact field name for destination in the `MoveNote` variant. Read `operation.rs` to confirm — it may be `new_parent_id` or `parent_id`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-rbac`

Expected: All pass including the new test.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-rbac/src/gate.rs krillnotes-rbac/src/tests/gate_tests.rs
git commit -m "fix(rbac): check destination scope on MoveNote authorization"
```

---

### Task 8: Clean up grants on note deletion

**Files:**
- Modify: `krillnotes-core/src/core/workspace/notes.rs`
- Modify: `krillnotes-core/src/core/workspace/tests.rs` (or create new test)

- [ ] **Step 1: Write failing test**

Add a test that verifies grants are cleaned up when a note is deleted:

```rust
#[test]
fn test_delete_note_cleans_up_permissions() {
    let mut ws = create_test_workspace_with_rbac("root_key");
    let note_id = ws.create_note_root("Test Note", "TextNote", &BTreeMap::new()).unwrap();

    // Manually insert a permission grant on this note
    ws.storage.connection().execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![note_id, "alice_key", "writer", "root_key"],
    ).unwrap();

    // Verify grant exists
    let count: i64 = ws.storage.connection().query_row(
        "SELECT COUNT(*) FROM note_permissions WHERE note_id = ?1",
        [&note_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);

    // Delete the note
    ws.delete_note_recursive(&note_id).unwrap();

    // Grant should be cleaned up
    let count: i64 = ws.storage.connection().query_row(
        "SELECT COUNT(*) FROM note_permissions WHERE note_id = ?1",
        [&note_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_delete_note_cleans_up_permissions`

Expected: FAIL — count is still 1 after deletion.

- [ ] **Step 3: Add cleanup to `delete_note_recursive`**

In `krillnotes-core/src/core/workspace/notes.rs`, inside `delete_note_recursive`, add a cleanup step in the deletion transaction (before or after the recursive note deletion). Find the transaction block and add:

```rust
// Clean up any permission grants anchored on deleted notes
tx.execute(
    "DELETE FROM note_permissions WHERE note_id IN (
        WITH RECURSIVE subtree(id) AS (
            SELECT ?1
            UNION ALL
            SELECT n.id FROM notes n JOIN subtree s ON n.parent_id = s.id
        )
        SELECT id FROM subtree
    )",
    [&note_id],
)?;
```

This recursively finds all note IDs in the subtree being deleted and removes their grants in a single query. Wrap in a feature gate if `note_permissions` table only exists with rbac feature:

```rust
// Only clean up if the table exists (rbac feature)
let _ = tx.execute(
    "DELETE FROM note_permissions WHERE note_id IN (...)",
    [&note_id],
);
```

- [ ] **Step 4: Add same cleanup to `delete_note_promote`**

In `delete_note_promote`, add cleanup for just the single deleted note (children are preserved):

```rust
// Clean up permission grants on the deleted note only (children survive)
let _ = tx.execute(
    "DELETE FROM note_permissions WHERE note_id = ?1",
    [&note_id],
);
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p krillnotes-core`

Expected: All pass including the new test.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace/notes.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "fix(rbac): clean up permission grants when notes are deleted"
```

---

### Task 9: Add workspace permission methods

**Files:**
- Create: `krillnotes-core/src/core/workspace/permissions.rs`
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Create `permissions.rs` with workspace-level wrappers**

```rust
use crate::core::error::Result;
use crate::core::workspace::Workspace;

impl Workspace {
    /// Get explicit permission grants anchored at `note_id`.
    pub fn get_note_permissions(
        &self,
        note_id: &str,
    ) -> Result<Vec<krillnotes_rbac::queries::PermissionGrantRow>> {
        let grants = krillnotes_rbac::queries::get_note_permissions(
            self.storage.connection(),
            note_id,
        )?;
        Ok(grants)
    }

    /// Get the effective role for the current user on `note_id`.
    pub fn get_effective_role(
        &self,
        note_id: &str,
    ) -> Result<krillnotes_rbac::queries::EffectiveRoleInfo> {
        let info = krillnotes_rbac::queries::get_effective_role(
            self.storage.connection(),
            &self.current_identity_pubkey,
            note_id,
            &self.owner_pubkey,
        )?;
        Ok(info)
    }

    /// Get grants inherited from ancestors of `note_id`.
    pub fn get_inherited_permissions(
        &self,
        note_id: &str,
    ) -> Result<Vec<krillnotes_rbac::queries::InheritedGrant>> {
        let grants = krillnotes_rbac::queries::get_inherited_permissions(
            self.storage.connection(),
            note_id,
        )?;
        Ok(grants)
    }

    /// Batch-compute effective roles for all notes (for tree dot rendering).
    pub fn get_all_effective_roles(
        &self,
    ) -> Result<std::collections::HashMap<String, String>> {
        let roles = krillnotes_rbac::queries::get_all_effective_roles(
            self.storage.connection(),
            &self.current_identity_pubkey,
            &self.owner_pubkey,
        )?;
        Ok(roles)
    }

    /// Preview which downstream grants would be invalidated if `user_id` is demoted.
    pub fn preview_cascade(
        &self,
        user_id: &str,
    ) -> Result<Vec<krillnotes_rbac::queries::PermissionGrantRow>> {
        let impact = krillnotes_rbac::queries::preview_cascade(
            self.storage.connection(),
            user_id,
        )?;
        Ok(impact)
    }
}
```

Note: These methods need `#[cfg(feature = "rbac")]` gating if the crate conditionally depends on `krillnotes-rbac`. Check how `create_permission_gate` is gated in `workspace.rs` and follow the same pattern. If the rbac crate is always available, no gating needed.

- [ ] **Step 2: Add module declaration**

In `krillnotes-core/src/core/workspace/mod.rs`, add:

```rust
mod permissions;
```

Check whether `current_identity_pubkey` and `owner_pubkey` are accessible fields on `Workspace`. If they're private, add getter methods or use the existing access pattern.

- [ ] **Step 3: Run tests**

Run: `cargo test -p krillnotes-core`

Expected: Compiles and all tests pass.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/workspace/permissions.rs krillnotes-core/src/core/workspace/mod.rs
git commit -m "feat(core): add workspace permission query methods"
```

---

### Task 10: Add Tauri commands and TS types

**Files:**
- Create: `krillnotes-desktop/src-tauri/src/commands/permissions.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/mod.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src/types.ts`

- [ ] **Step 1: Create Tauri commands**

Create `krillnotes-desktop/src-tauri/src/commands/permissions.rs`:

```rust
use serde::{Deserialize, Serialize};
use tauri::{State, Window};
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionGrant {
    pub note_id: Option<String>,
    pub user_id: String,
    pub role: String,
    pub granted_by: String,
    pub display_name: String,
    pub granted_by_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveRole {
    pub role: String,
    pub inherited_from: Option<String>,
    pub inherited_from_title: Option<String>,
    pub granted_by: Option<String>,
    pub granted_by_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CascadeImpact {
    pub affected_grants: Vec<PermissionGrant>,
}

#[tauri::command]
pub async fn get_note_permissions(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<PermissionGrant>, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label()).ok_or("No workspace")?;
    let grants = ws.get_note_permissions(&note_id).map_err(|e| e.to_string())?;
    // TODO: resolve display names from peer registry / contact book
    Ok(grants.into_iter().map(|g| PermissionGrant {
        note_id: Some(g.note_id),
        user_id: g.user_id.clone(),
        role: g.role,
        granted_by: g.granted_by.clone(),
        display_name: resolve_display_name(&g.user_id),
        granted_by_name: resolve_display_name(&g.granted_by),
    }).collect())
}

#[tauri::command]
pub async fn get_effective_role(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
) -> Result<EffectiveRole, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label()).ok_or("No workspace")?;
    let info = ws.get_effective_role(&note_id).map_err(|e| e.to_string())?;
    Ok(EffectiveRole {
        role: info.role,
        inherited_from: info.inherited_from,
        inherited_from_title: info.inherited_from_title,
        granted_by: info.granted_by.clone(),
        granted_by_name: info.granted_by.map(|k| resolve_display_name(&k)),
    })
}

#[tauri::command]
pub async fn get_all_effective_roles(
    window: Window,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label()).ok_or("No workspace")?;
    ws.get_all_effective_roles().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_inherited_permissions(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<PermissionGrant>, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label()).ok_or("No workspace")?;
    let grants = ws.get_inherited_permissions(&note_id).map_err(|e| e.to_string())?;
    Ok(grants.into_iter().map(|g| PermissionGrant {
        note_id: Some(g.anchor_note_id),
        user_id: g.grant.user_id.clone(),
        role: g.grant.role,
        granted_by: g.grant.granted_by.clone(),
        display_name: resolve_display_name(&g.grant.user_id),
        granted_by_name: resolve_display_name(&g.grant.granted_by),
    }).collect())
}

#[tauri::command]
pub async fn preview_cascade(
    window: Window,
    state: State<'_, AppState>,
    user_id: String,
) -> Result<CascadeImpact, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label()).ok_or("No workspace")?;
    let grants = ws.preview_cascade(&user_id).map_err(|e| e.to_string())?;
    Ok(CascadeImpact {
        affected_grants: grants.into_iter().map(|g| PermissionGrant {
            note_id: Some(g.note_id),
            user_id: g.user_id.clone(),
            role: g.role,
            granted_by: g.granted_by.clone(),
            display_name: resolve_display_name(&g.user_id),
            granted_by_name: resolve_display_name(&g.granted_by),
        }).collect(),
    })
}

/// Fallback display name: first 8 chars of base64 key.
/// TODO: In Plan B/C, enhance to resolve from peer registry + contact book.
fn resolve_display_name(pubkey: &str) -> String {
    if pubkey.len() > 8 {
        format!("{}...", &pubkey[..8])
    } else {
        pubkey.to_string()
    }
}
```

- [ ] **Step 2: Register module and commands**

In `commands/mod.rs`, add:

```rust
pub mod permissions;
```

In `lib.rs`, add to the `generate_handler![]` macro:

```rust
commands::permissions::get_note_permissions,
commands::permissions::get_effective_role,
commands::permissions::get_all_effective_roles,
commands::permissions::get_inherited_permissions,
commands::permissions::preview_cascade,
```

- [ ] **Step 3: Add TypeScript types**

Add to `krillnotes-desktop/src/types.ts`:

```typescript
export interface PermissionGrant {
  noteId: string | null;
  userId: string;
  role: "owner" | "writer" | "reader";
  grantedBy: string;
  displayName: string;
  grantedByName: string;
}

export interface EffectiveRole {
  role: "owner" | "writer" | "reader" | "root_owner" | "none";
  inheritedFrom: string | null;
  inheritedFromTitle: string | null;
  grantedBy: string | null;
  grantedByName: string | null;
}

export interface CascadeImpact {
  affectedGrants: PermissionGrant[];
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd krillnotes-desktop && npm run tauri build -- --debug 2>&1 | head -50`

Or just check Rust compilation: `cargo check -p krillnotes-desktop`

Expected: Compiles without errors.

- [ ] **Step 5: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: No type errors.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/permissions.rs \
       krillnotes-desktop/src-tauri/src/commands/mod.rs \
       krillnotes-desktop/src-tauri/src/lib.rs \
       krillnotes-desktop/src/types.ts
git commit -m "feat(desktop): add Tauri permission commands and TS types"
```

---

### Task 11: Run full test suite and verify

- [ ] **Step 1: Run all Rust tests**

Run: `cargo test --workspace`

Expected: All pass.

- [ ] **Step 2: Run TypeScript check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: No errors.

- [ ] **Step 3: Verify dev build starts**

Run: `cd krillnotes-desktop && npm run tauri dev`

Expected: App starts without errors. Permission commands are registered (visible in Tauri logs).

- [ ] **Step 4: Final commit if any fixes were needed**

```bash
git add -A && git commit -m "fix: address test/build issues from permission backend"
```
