// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::gate::RbacGate;
use crate::queries;
use rusqlite::Connection;

const ROOT_OWNER: &str = "root_pubkey";

/// Setup in-memory DB with notes + note_permissions tables and an RbacGate.
fn setup_workspace_with_gate(owner_pubkey: &str) -> (Connection, RbacGate) {
    let conn = Connection::open_in_memory().unwrap();
    let gate = RbacGate::new(owner_pubkey.to_string());
    conn.execute_batch(
        "
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
        ",
    )
    .unwrap();
    (conn, gate)
}

/// Insert a note into the test DB.
fn insert_note(conn: &Connection, id: &str, parent_id: Option<&str>) {
    conn.execute(
        "INSERT INTO notes (id, parent_id, title, created_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, parent_id, id, ROOT_OWNER],
    )
    .unwrap();
}

// ── get_note_permissions tests ──

#[test]
fn test_get_note_permissions_returns_anchored_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "bob_key", "reader", "root_pubkey"],
    )
    .unwrap();

    let grants = queries::get_note_permissions(&conn, "note-1").unwrap();
    assert_eq!(grants.len(), 2);
    assert!(grants
        .iter()
        .any(|g| g.user_id == "alice_key" && g.role == "writer"));
    assert!(grants
        .iter()
        .any(|g| g.user_id == "bob_key" && g.role == "reader"));
}

#[test]
fn test_get_note_permissions_empty_for_no_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    let grants = queries::get_note_permissions(&conn, "note-1").unwrap();
    assert!(grants.is_empty());
}

// ── get_effective_role tests ──

#[test]
fn test_get_effective_role_direct_grant() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    )
    .unwrap();

    let info = queries::get_effective_role(&conn, "alice_key", "note-1", "root_pubkey").unwrap();
    assert_eq!(info.role, "writer");
    assert!(info.inherited_from.is_none());
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
    )
    .unwrap();

    let info = queries::get_effective_role(&conn, "alice_key", "child", "root_pubkey").unwrap();
    assert_eq!(info.role, "writer");
    assert_eq!(info.inherited_from.as_deref(), Some("parent"));
}

#[test]
fn test_get_effective_role_root_owner() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    let info =
        queries::get_effective_role(&conn, "root_pubkey", "note-1", "root_pubkey").unwrap();
    assert_eq!(info.role, "root_owner");
    assert!(info.inherited_from.is_none());
    assert!(info.granted_by.is_none());
}

#[test]
fn test_get_effective_role_no_access() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    let info =
        queries::get_effective_role(&conn, "stranger_key", "note-1", "root_pubkey").unwrap();
    assert_eq!(info.role, "none");
}

// ── get_inherited_permissions tests ──

#[test]
fn test_get_inherited_permissions_from_ancestors() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "grandparent", None);
    insert_note(&conn, "parent", Some("grandparent"));
    insert_note(&conn, "child", Some("parent"));

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["grandparent", "alice_key", "owner", "root_pubkey"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["parent", "bob_key", "writer", "alice_key"],
    )
    .unwrap();

    let inherited = queries::get_inherited_permissions(&conn, "child").unwrap();
    assert_eq!(inherited.len(), 2);
    assert!(inherited
        .iter()
        .any(|g| g.grant.user_id == "alice_key" && g.anchor_note_id == "grandparent"));
    assert!(inherited
        .iter()
        .any(|g| g.grant.user_id == "bob_key" && g.anchor_note_id == "parent"));
}

#[test]
fn test_get_inherited_permissions_excludes_grants_on_self() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    )
    .unwrap();

    let inherited = queries::get_inherited_permissions(&conn, "note-1").unwrap();
    assert!(inherited.is_empty());
}

// ── get_all_effective_roles tests ──

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
    )
    .unwrap();

    let roles = queries::get_all_effective_roles(&conn, "alice_key", "root_pubkey").unwrap();
    assert_eq!(roles.get("root").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("child1").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("child2").map(|r| r.as_str()), Some("writer"));
    assert_eq!(
        roles.get("grandchild").map(|r| r.as_str()),
        Some("writer")
    );
}

#[test]
fn test_get_all_effective_roles_closer_grant_overrides() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "root", None);
    insert_note(&conn, "child", Some("root"));

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["root", "alice_key", "writer", "root_pubkey"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["child", "alice_key", "reader", "root_pubkey"],
    )
    .unwrap();

    let roles = queries::get_all_effective_roles(&conn, "alice_key", "root_pubkey").unwrap();
    assert_eq!(roles.get("root").map(|r| r.as_str()), Some("writer"));
    assert_eq!(roles.get("child").map(|r| r.as_str()), Some("reader"));
}

#[test]
fn test_get_all_effective_roles_root_owner_all_owner() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "root", None);
    insert_note(&conn, "child", Some("root"));

    let roles = queries::get_all_effective_roles(&conn, "root_pubkey", "root_pubkey").unwrap();
    assert_eq!(
        roles.get("root").map(|r| r.as_str()),
        Some("root_owner")
    );
    assert_eq!(
        roles.get("child").map(|r| r.as_str()),
        Some("root_owner")
    );
}

// ── preview_cascade tests ──

#[test]
fn test_preview_cascade_shows_invalidated_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "owner", "root_pubkey"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "bob_key", "writer", "alice_key"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "carol_key", "reader", "alice_key"],
    )
    .unwrap();

    let impact = queries::preview_cascade(&conn, "note-1", "alice_key", "reader").unwrap();
    assert_eq!(impact.len(), 2);
    assert!(impact.iter().any(|g| g.grant.user_id == "bob_key"));
    assert!(impact.iter().any(|g| g.grant.user_id == "carol_key"));
    assert!(impact[0].reason.contains("cannot grant"));
}

#[test]
fn test_preview_cascade_no_downstream_grants() {
    let (conn, _gate) = setup_workspace_with_gate("root_pubkey");
    insert_note(&conn, "note-1", None);

    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["note-1", "alice_key", "writer", "root_pubkey"],
    )
    .unwrap();

    let impact = queries::preview_cascade(&conn, "note-1", "alice_key", "reader").unwrap();
    assert!(impact.is_empty());
}
