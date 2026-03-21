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

// ── Full Workspace integration tests ─────────────────────────────────

/// Root owner should be able to perform every category of workspace operation
/// when RbacGate is installed.
#[test]
fn test_root_owner_can_do_everything() {
    use ed25519_dalek::SigningKey;
    use krillnotes_core::core::workspace::{AddPosition, Workspace};

    let signing_key = SigningKey::from_bytes(&[1u8; 32]);
    let owner_pubkey = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().as_bytes())
    };
    let gate: Box<dyn PermissionGate> = Box::new(RbacGate::new(owner_pubkey));

    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(), "", "test-identity", SigningKey::from_bytes(&[1u8; 32]), gate,
    ).unwrap();

    // Create note
    let root = ws.list_all_notes().unwrap()[0].clone();
    let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

    // Update note
    ws.update_note_title(&child_id, "Test Note".to_string()).unwrap();

    // Move note (create another child, move under it)
    let child2_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
    ws.move_note(&child_id, Some(&child2_id), 0.0).unwrap();

    // Delete note
    ws.delete_note_recursive(&child_id).unwrap();

    // Undo + redo
    ws.undo().unwrap();
    ws.redo().unwrap();

    // Script operations are guarded by is_owner(), not authorize().
    // Testing them here would exercise the NotOwner check, not the RBAC gate.
    // The gate authorize() path is fully covered by the note operations above.

    // All operations succeeded — root owner has full access
}

/// A non-owner identity without any grants should be denied on mutating operations.
#[test]
fn test_non_owner_without_grants_is_denied() {
    use ed25519_dalek::SigningKey;
    use krillnotes_core::core::workspace::{AddPosition, Workspace};

    let owner_key = SigningKey::from_bytes(&[1u8; 32]);
    let owner_pubkey = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .encode(owner_key.verifying_key().as_bytes())
    };
    let gate: Box<dyn PermissionGate> = Box::new(RbacGate::new(owner_pubkey));

    // Create workspace as owner.
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(), "", "owner-identity", SigningKey::from_bytes(&[1u8; 32]), gate,
    ).unwrap();

    // Add a child note so there's something to operate on.
    let root = ws.list_all_notes().unwrap()[0].clone();
    ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
    drop(ws);

    // Re-open the workspace as a different identity (non-owner).
    let non_owner_key = SigningKey::from_bytes(&[2u8; 32]);
    let non_owner_gate: Box<dyn PermissionGate> = Box::new(RbacGate::new(
        // Still the ORIGINAL owner's pubkey — this is the gate's "who is root" setting
        {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .encode(owner_key.verifying_key().as_bytes())
        },
    ));
    let mut ws2 = Workspace::open(
        temp.path(), "", "non-owner-identity", non_owner_key, non_owner_gate,
    ).unwrap();

    // Attempting to create a note should be denied.
    let root2 = ws2.list_all_notes().unwrap()[0].clone();
    let result = ws2.create_note(&root2.id, AddPosition::AsChild, "TextNote");
    assert!(result.is_err(), "non-owner without grants should be denied");
    let err = result.unwrap_err();
    assert!(
        matches!(err, krillnotes_core::KrillnotesError::Permission(_)),
        "error should be Permission, got: {err}"
    );
}
