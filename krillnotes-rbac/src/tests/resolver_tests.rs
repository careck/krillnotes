// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::resolver::{resolve_role, Role};
use rusqlite::Connection;

/// Create an in-memory DB with the notes table and note_permissions table,
/// populated with a test tree:
///
/// root_a  (root node)
///   +- child_1
///   |  +- grandchild_1
///   +- child_2
/// root_b  (second root node)
///   +- child_3
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

#[test]
fn test_no_grant_returns_none() {
    let conn = setup_test_db();
    let role = resolve_role(&conn, "bob", "grandchild_1").unwrap();
    assert_eq!(role, None);
}

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

#[test]
fn test_inherited_grant_from_parent() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'reader', 'alice')",
        [],
    ).unwrap();
    let role = resolve_role(&conn, "bob", "grandchild_1").unwrap();
    assert_eq!(role, Some(Role::Reader));
}

#[test]
fn test_closer_grant_overrides_inherited() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'reader', 'alice')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('child_1', 'bob', 'owner', 'alice')",
        [],
    ).unwrap();
    let role = resolve_role(&conn, "bob", "grandchild_1").unwrap();
    assert_eq!(role, Some(Role::Owner));
}

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

#[test]
fn test_no_cross_tree_inheritance() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO note_permissions (note_id, user_id, role, granted_by) VALUES ('root_a', 'bob', 'owner', 'alice')",
        [],
    ).unwrap();
    assert_eq!(resolve_role(&conn, "bob", "child_3").unwrap(), None);
}
