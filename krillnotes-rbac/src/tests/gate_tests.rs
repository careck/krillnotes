// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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
