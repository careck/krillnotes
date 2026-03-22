// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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
        -- Test tree
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('root_a', NULL, 'Root A', 'root_owner_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_1', 'root_a', 'Child 1', 'bob_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_2', 'root_a', 'Child 2', 'carol_pubkey_base64');
    ",
    )
    .unwrap();
    (conn, gate)
}

fn make_create_note(parent_id: &str) -> Operation {
    Operation::CreateNote {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
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

fn make_create_note_root() -> Operation {
    Operation::CreateNote {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
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

fn make_update_field(note_id: &str) -> Operation {
    Operation::UpdateField {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
        device_id: "test_device".into(),
        note_id: note_id.into(),
        field: "content".into(),
        value: krillnotes_core::FieldValue::Text("test".into()),
        modified_by: String::new(),
        signature: String::new(),
    }
}

fn make_delete_note(note_id: &str) -> Operation {
    Operation::DeleteNote {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
        device_id: "test_device".into(),
        note_id: note_id.into(),
        deleted_by: String::new(),
        signature: String::new(),
    }
}

fn make_set_permission(note_id: &str, user_id: &str, role: &str) -> Operation {
    make_set_permission_by(note_id, user_id, role, ROOT_OWNER)
}

fn make_set_permission_by(
    note_id: &str,
    user_id: &str,
    role: &str,
    granted_by: &str,
) -> Operation {
    Operation::SetPermission {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
        device_id: "test_device".into(),
        note_id: Some(note_id.into()),
        user_id: user_id.into(),
        role: role.into(),
        granted_by: granted_by.into(),
        signature: String::new(),
    }
}

fn grant(conn: &Connection, note_id: &str, user_id: &str, role: &str) {
    grant_by(conn, note_id, user_id, role, ROOT_OWNER);
}

fn grant_by(conn: &Connection, note_id: &str, user_id: &str, role: &str, granted_by: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO note_permissions (note_id, user_id, role, granted_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![note_id, user_id, role, granted_by],
    )
    .unwrap();
}

fn make_revoke_permission(note_id: &str, user_id: &str) -> Operation {
    Operation::RevokePermission {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
        device_id: "test_device".into(),
        note_id: Some(note_id.into()),
        user_id: user_id.into(),
        revoked_by: ROOT_OWNER.into(),
        signature: String::new(),
    }
}

fn make_move_note_op(note_id: &str, new_parent_id: &str) -> Operation {
    Operation::MoveNote {
        operation_id: uuid::Uuid::new_v4().to_string(),
        timestamp: krillnotes_core::HlcTimestamp {
            wall_ms: 1,
            counter: 0,
            node_id: 0,
        },
        device_id: "test_device".into(),
        note_id: note_id.into(),
        new_parent_id: Some(new_parent_id.into()),
        new_position: 0.0,
        moved_by: String::new(),
        signature: String::new(),
    }
}

// --- Tests ---

#[test]
fn test_root_owner_allowed_everything() {
    let (conn, gate) = setup_gate_db();
    let op = make_create_note("root_a");
    assert!(gate.authorize(&conn, ROOT_OWNER, &op).is_ok());
}

#[test]
fn test_no_grant_denied() {
    let (conn, gate) = setup_gate_db();
    let op = make_create_note("root_a");
    assert!(gate.authorize(&conn, BOB, &op).is_err());
}

#[test]
fn test_owner_can_create_update_delete() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "owner");
    assert!(gate.authorize(&conn, BOB, &make_create_note("root_a")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_update_field("child_1")).is_ok());
    assert!(gate.authorize(&conn, BOB, &make_delete_note("child_2")).is_ok());
}

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

#[test]
fn test_writer_cannot_set_permission() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "writer");
    assert!(
        gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "reader"))
            .is_err()
    );
}

#[test]
fn test_owner_can_set_permission_up_to_owner() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "owner");
    assert!(
        gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "owner"))
            .is_ok()
    );
    assert!(
        gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "writer"))
            .is_ok()
    );
    assert!(
        gate.authorize(&conn, BOB, &make_set_permission("root_a", CAROL, "reader"))
            .is_ok()
    );
}

#[test]
fn test_owner_cannot_create_root_note() {
    let (conn, gate) = setup_gate_db();
    grant(&conn, "root_a", BOB, "owner");
    let op = make_create_note_root();
    assert!(gate.authorize(&conn, BOB, &op).is_err());
}

#[test]
fn test_root_owner_can_create_root_note() {
    let (conn, gate) = setup_gate_db();
    let op = make_create_note_root();
    assert!(gate.authorize(&conn, ROOT_OWNER, &op).is_ok());
}

// --- apply_permission_op tests ---

#[test]
fn test_apply_set_permission_creates_grant() {
    let (conn, gate) = setup_gate_db();
    let op = make_set_permission("root_a", BOB, "writer");
    gate.apply_permission_op(&conn, &op).unwrap();
    let role = crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap();
    assert_eq!(role, Some(Role::Writer));
}

#[test]
fn test_apply_set_permission_upserts() {
    let (conn, gate) = setup_gate_db();
    gate.apply_permission_op(&conn, &make_set_permission("root_a", BOB, "reader"))
        .unwrap();
    gate.apply_permission_op(&conn, &make_set_permission("root_a", BOB, "owner"))
        .unwrap();
    let role = crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap();
    assert_eq!(role, Some(Role::Owner));
}

#[test]
fn test_apply_revoke_removes_grant() {
    let (conn, gate) = setup_gate_db();
    gate.apply_permission_op(&conn, &make_set_permission("root_a", BOB, "writer"))
        .unwrap();
    gate.apply_permission_op(&conn, &make_revoke_permission("root_a", BOB))
        .unwrap();
    let role = crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap();
    assert_eq!(role, None);
}

#[test]
fn test_cascade_revocation() {
    let (conn, gate) = setup_gate_db();
    grant_by(&conn, "root_a", BOB, "owner", ROOT_OWNER);
    grant_by(&conn, "root_a", CAROL, "writer", BOB);
    gate.apply_permission_op(&conn, &make_revoke_permission("root_a", BOB))
        .unwrap();
    // Bob's grant is removed
    assert_eq!(
        crate::resolver::resolve_role(&conn, BOB, "root_a").unwrap(),
        None
    );
    // Carol's grant is PRESERVED (opt-in cascade — UI decides)
    assert_eq!(
        crate::resolver::resolve_role(&conn, CAROL, "root_a").unwrap(),
        Some(crate::resolver::Role::Writer)
    );
}

#[test]
fn test_demotion_cascade_partial() {
    let (conn, gate) = setup_gate_db();
    grant_by(&conn, "root_a", BOB, "owner", ROOT_OWNER);
    grant_by(&conn, "root_a", CAROL, "owner", BOB);
    grant_by(&conn, "root_a", "dave", "reader", BOB);

    // Demote Bob to writer
    conn.execute(
        "DELETE FROM note_permissions WHERE note_id = 'root_a' AND user_id = ?1",
        rusqlite::params![BOB],
    )
    .unwrap();
    gate.apply_permission_op(&conn, &make_set_permission_by("root_a", BOB, "writer", ROOT_OWNER))
        .unwrap();
    gate.cascade_revoke_public(&conn, BOB).unwrap();

    // Carol's owner grant exceeds Bob's writer -> invalidated
    assert_eq!(
        crate::resolver::resolve_role(&conn, CAROL, "root_a").unwrap(),
        None
    );
    // Dave's reader is within Bob's writer -> still valid
    assert_eq!(
        crate::resolver::resolve_role(&conn, "dave", "root_a").unwrap(),
        Some(Role::Reader)
    );
}

// --- MoveNote destination scope tests ---

#[test]
fn test_move_note_denied_to_inaccessible_destination() {
    let (conn, gate) = setup_gate_db();
    // Add subtree_b as a separate root
    conn.execute(
        "INSERT INTO notes (id, parent_id, title, created_by) VALUES ('subtree_b', NULL, 'Subtree B', 'root_owner_pubkey_base64')",
        [],
    )
    .unwrap();

    // Bob is writer on root_a only
    grant(&conn, "root_a", BOB, "writer");
    // Bob authored child_1 (created_by = BOB from setup_gate_db)

    let op = make_move_note_op("child_1", "subtree_b");
    let result = gate.authorize(&conn, BOB, &op);
    assert!(result.is_err(), "should be denied — no access to subtree_b");
}

#[test]
fn test_move_note_allowed_within_same_subtree() {
    let (conn, gate) = setup_gate_db();
    // Bob is writer on root_a, authored child_1
    grant(&conn, "root_a", BOB, "writer");

    // Move child_1 under child_2 (both in root_a)
    let op = make_move_note_op("child_1", "child_2");
    let result = gate.authorize(&conn, BOB, &op);
    assert!(result.is_ok(), "should be allowed — both in root_a subtree");
}

#[test]
fn test_move_note_denied_reader_at_destination() {
    let (conn, gate) = setup_gate_db();
    // Add subtree_b as a separate root
    conn.execute(
        "INSERT INTO notes (id, parent_id, title, created_by) VALUES ('subtree_b', NULL, 'Subtree B', 'root_owner_pubkey_base64')",
        [],
    )
    .unwrap();

    // Bob is owner on root_a, reader on subtree_b
    grant(&conn, "root_a", BOB, "owner");
    grant(&conn, "subtree_b", BOB, "reader");

    let op = make_move_note_op("child_1", "subtree_b");
    let result = gate.authorize(&conn, BOB, &op);
    assert!(
        result.is_err(),
        "should be denied — reader at destination can't write"
    );
}
