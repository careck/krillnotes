use super::*;

fn dummy_hlc() -> HlcTimestamp {
    HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 }
}

#[test]
fn test_set_permission_roundtrip() {
    let op = Operation::SetPermission {
        operation_id: "op1".to_string(),
        timestamp: dummy_hlc(),
        device_id: "dev1".to_string(),
        note_id: Some("note1".to_string()),
        user_id: "pubkey_b64".to_string(),
        role: "writer".to_string(),
        granted_by: "grantor_b64".to_string(),
        signature: "sig_b64".to_string(),
    };
    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op1");
}

#[test]
fn test_revoke_permission_roundtrip() {
    let op = Operation::RevokePermission {
        operation_id: "op2".to_string(),
        timestamp: dummy_hlc(),
        device_id: "dev1".to_string(),
        note_id: Some("note1".to_string()),
        user_id: "pubkey_b64".to_string(),
        revoked_by: "revoker_b64".to_string(),
        signature: "sig_b64".to_string(),
    };
    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op2");
}

#[test]
fn test_remove_peer_roundtrip() {
    let op = Operation::RemovePeer {
        operation_id: "op-rp1".to_string(),
        timestamp: dummy_hlc(),
        device_id: "dev1".to_string(),
        user_id: "bob_pubkey".to_string(),
        removed_by: "alice_pubkey".to_string(),
        signature: "sig".to_string(),
    };
    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op-rp1");
    assert_eq!(back.author_key(), "alice_pubkey");
}

#[test]
fn test_transfer_root_ownership_roundtrip() {
    let op = Operation::TransferRootOwnership {
        operation_id: "op-tro1".to_string(),
        timestamp: dummy_hlc(),
        device_id: "dev1".to_string(),
        new_owner: "bob_pubkey".to_string(),
        transferred_by: "alice_pubkey".to_string(),
        signature: "sig".to_string(),
    };
    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op-tro1");
    assert_eq!(back.author_key(), "alice_pubkey");
}

#[test]
fn test_join_workspace_roundtrip() {
    let op = Operation::JoinWorkspace {
        operation_id: "op3".to_string(),
        timestamp: dummy_hlc(),
        device_id: "dev1".to_string(),
        identity_public_key: "pubkey_b64".to_string(),
        declared_name: "Alice".to_string(),
        pairing_token: "token_b64".to_string(),
        signature: "sig_b64".to_string(),
    };
    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op3");
}

fn dummy_timestamp() -> HlcTimestamp {
    HlcTimestamp {
        wall_ms: 1_000_000,
        counter: 0,
        node_id: 0,
    }
}

#[test]
fn test_retract_operation_serialization() {
    use crate::RetractInverse;
    let op = Operation::RetractOperation {
        operation_id: "ret-1".into(),
        timestamp: HlcTimestamp {
            wall_ms: 9_999_000,
            counter: 0,
            node_id: 0,
        },
        device_id: "dev-1".into(),
        retracted_ids: vec!["op-1".into(), "op-2".into()],
        inverse: RetractInverse::DeleteNote { note_id: "n-1".into() },
        propagate: true,
    };
    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "ret-1");
    assert_eq!(back.timestamp().wall_ms, 9_999_000);
}

#[test]
fn test_operation_serialization() {
    let op = Operation::CreateNote {
        operation_id: "op-123".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        note_id: "note-1".to_string(),
        parent_id: None,
        position: 0.0,
        schema: "TextNote".to_string(),
        title: "Test".to_string(),
        fields: BTreeMap::new(),
        created_by: String::new(),
        signature: String::new(),
    };

    let json = serde_json::to_string(&op).unwrap();
    let deserialized: Operation = serde_json::from_str(&json).unwrap();

    assert_eq!(op.operation_id(), deserialized.operation_id());
}

#[test]
fn test_sign_and_verify() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut op = Operation::UpdateField {
        operation_id: "op-sign-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        note_id: "note-1".to_string(),
        field: "body".to_string(),
        value: crate::FieldValue::Text("hello".to_string()),
        modified_by: String::new(),
        signature: String::new(),
    };

    op.sign(&signing_key);

    // Signature and author key must be set after signing.
    assert!(!op.get_signature().is_empty());
    assert!(!op.author_key().is_empty());

    // Verification must pass with the correct key.
    assert!(op.verify(&verifying_key), "signature should verify");

    // Tamper with the operation — verification must fail.
    if let Operation::UpdateField { ref mut field, .. } = op {
        *field = "tampered".to_string();
    }
    assert!(!op.verify(&verifying_key), "tampered operation should not verify");
}

#[test]
fn test_create_note_sign_and_verify_multi_field() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    // Build a CreateNote with multiple fields — order must be deterministic.
    let mut fields = BTreeMap::new();
    fields.insert("body".to_string(), crate::FieldValue::Text("hello world".to_string()));
    fields.insert("rating".to_string(), crate::FieldValue::Text("5".to_string()));
    fields.insert("author".to_string(), crate::FieldValue::Text("Alice".to_string()));

    let mut op = Operation::CreateNote {
        operation_id: "op-cn-sign-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        note_id: "note-multi-1".to_string(),
        parent_id: None,
        position: 0.0,
        schema: "TextNote".to_string(),
        title: "Multi-field note".to_string(),
        fields,
        created_by: String::new(),
        signature: String::new(),
    };

    op.sign(&signing_key);

    assert!(!op.get_signature().is_empty());
    assert!(!op.author_key().is_empty());

    // Verification must succeed with the correct key.
    assert!(op.verify(&verifying_key), "CreateNote multi-field signature should verify");

    // Tamper with a field value — verification must fail.
    if let Operation::CreateNote { ref mut title, .. } = op {
        *title = "tampered".to_string();
    }
    assert!(!op.verify(&verifying_key), "tampered CreateNote should not verify");
}

#[test]
fn test_update_note_variant() {
    let op = Operation::UpdateNote {
        operation_id: "op-upd-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-2".to_string(),
        note_id: "note-42".to_string(),
        title: "New Title".to_string(),
        modified_by: String::new(),
        signature: String::new(),
    };

    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op-upd-1");
    assert_eq!(back.author_key(), "");
}

#[test]
fn test_set_tags_variant() {
    let op = Operation::SetTags {
        operation_id: "op-tags-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-3".to_string(),
        note_id: "note-7".to_string(),
        tags: vec!["rust".to_string(), "crdt".to_string()],
        modified_by: String::new(),
        signature: String::new(),
    };

    let json = serde_json::to_string(&op).unwrap();
    let back: Operation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.operation_id(), "op-tags-1");
    if let Operation::SetTags { tags, .. } = back {
        assert_eq!(tags, vec!["rust", "crdt"]);
    } else {
        panic!("wrong variant after round-trip");
    }
}

#[test]
fn test_add_attachment_sign_and_verify() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut op = Operation::AddAttachment {
        operation_id: "op-att-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        attachment_id: "att-uuid-1".to_string(),
        note_id: "note-1".to_string(),
        filename: "photo.jpg".to_string(),
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: 1024,
        hash_sha256: "abc123".to_string(),
        added_by: String::new(),
        signature: String::new(),
    };

    op.sign(&signing_key);
    assert!(!op.get_signature().is_empty());
    assert!(!op.author_key().is_empty());
    assert!(op.verify(&verifying_key));

    // Tamper test
    if let Operation::AddAttachment { ref mut filename, .. } = op {
        *filename = "tampered.jpg".to_string();
    }
    assert!(!op.verify(&verifying_key));
}

#[test]
fn test_remove_attachment_sign_and_verify() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut op = Operation::RemoveAttachment {
        operation_id: "op-ratt-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-1".to_string(),
        attachment_id: "att-uuid-1".to_string(),
        note_id: "note-1".to_string(),
        removed_by: String::new(),
        signature: String::new(),
    };

    op.sign(&signing_key);
    assert!(op.verify(&verifying_key));

    // Tamper test
    if let Operation::RemoveAttachment { ref mut attachment_id, .. } = op {
        *attachment_id = "tampered-id".to_string();
    }
    assert!(!op.verify(&verifying_key));
}

#[test]
fn test_attachment_op_accessors() {
    let ts = dummy_timestamp();
    let op = Operation::AddAttachment {
        operation_id: "op-acc-1".to_string(),
        timestamp: ts,
        device_id: "dev-acc".to_string(),
        attachment_id: "att-1".to_string(),
        note_id: "note-1".to_string(),
        filename: "f.txt".to_string(),
        mime_type: None,
        size_bytes: 100,
        hash_sha256: "hash".to_string(),
        added_by: "key123".to_string(),
        signature: String::new(),
    };

    assert_eq!(op.operation_id(), "op-acc-1");
    assert_eq!(op.timestamp(), ts);
    assert_eq!(op.device_id(), "dev-acc");
    assert_eq!(op.author_key(), "key123");

    let op2 = Operation::RemoveAttachment {
        operation_id: "op-rem-1".to_string(),
        timestamp: dummy_timestamp(),
        device_id: "dev-rem".to_string(),
        attachment_id: "att-2".to_string(),
        note_id: "note-2".to_string(),
        removed_by: "remkey456".to_string(),
        signature: String::new(),
    };

    assert_eq!(op2.operation_id(), "op-rem-1");
    assert_eq!(op2.device_id(), "dev-rem");
    assert_eq!(op2.author_key(), "remkey456");
}
