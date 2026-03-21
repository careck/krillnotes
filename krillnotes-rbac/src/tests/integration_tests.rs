// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Integration tests exercising the full authorize → apply_permission_op
//! lifecycle that real code will use.

use crate::gate::RbacGate;
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
        -- Test tree: two independent subtrees
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('root_a', NULL, 'Root A', 'root_owner_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_1', 'root_a', 'Child 1', 'bob_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_2', 'root_a', 'Child 2', 'carol_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('root_b', NULL, 'Root B', 'root_owner_pubkey_base64');
        INSERT INTO notes (id, parent_id, title, created_by) VALUES ('child_3', 'root_b', 'Child 3', 'root_owner_pubkey_base64');
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

// --- Integration Tests ---

/// Full invitation + authorization lifecycle:
/// root_owner grants Bob owner, Bob grants Carol writer,
/// then verify Carol's capabilities and limitations.
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

    // Carol cannot delete Bob's note (child_1 created_by BOB, Carol is writer)
    assert!(gate.authorize(&conn, CAROL, &make_delete_note("child_1")).is_err());

    // Carol cannot set permissions (writers lack that ability)
    assert!(
        gate.authorize(&conn, CAROL, &make_set_permission("root_a", "dave", "reader"))
            .is_err()
    );
}

/// Cascade revocation end-to-end:
/// root_owner → bob (owner) → carol (writer),
/// revoking Bob cascades to Carol.
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

/// Multi-subtree isolation:
/// Bob has owner on root_a but no access to root_b.
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
