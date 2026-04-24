use super::*;
use base64::Engine;
use ed25519_dalek::Signer;

#[test]
fn test_identity_file_roundtrip_serde() {
    let file = IdentityFile {
        identity_uuid: Uuid::new_v4(),
        display_name: "Test User".to_string(),
        public_key: "AAAA".to_string(),
        private_key_enc: EncryptedKey {
            ciphertext: "BBBB".to_string(),
            nonce: "CCCC".to_string(),
            kdf: "argon2id".to_string(),
            kdf_params: KdfParams {
                salt: "DDDD".to_string(),
                m_cost: 65536,
                t_cost: 3,
                p_cost: 1,
            },
        },
        last_used: None,
    };
    let json = serde_json::to_string_pretty(&file).unwrap();
    let parsed: IdentityFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.identity_uuid, file.identity_uuid);
    assert_eq!(parsed.display_name, "Test User");
    assert_eq!(parsed.private_key_enc.kdf, "argon2id");
}

#[test]
fn test_identity_manager_creates_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    assert!(mgr.home_dir().exists());
}

#[test]
fn test_create_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Test Identity", "my-passphrase").unwrap();
    assert_eq!(file.display_name, "Test Identity");
    assert!(file.last_used.is_some());
    let base = tmp.path().join("Test Identity");
    assert!(base.join(".identity").join("identity.json").exists());
    assert!(base.join(".identity").join("contacts").is_dir());
    assert!(base.join(".identity").join("relays").is_dir());
    assert!(base.join(".identity").join("invites").is_dir());
}

#[test]
fn test_unlock_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity_file = mgr.create_identity("Bob", "secret").unwrap();

    let unlocked = mgr.unlock_identity(&identity_file.identity_uuid, "secret").unwrap();
    assert_eq!(unlocked.identity_uuid, identity_file.identity_uuid);
    assert_eq!(unlocked.display_name, "Bob");

    // Public key matches
    let pk_bytes = BASE64.decode(&identity_file.public_key).unwrap();
    assert_eq!(unlocked.verifying_key.as_bytes(), pk_bytes.as_slice());
}

#[test]
fn test_wrong_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity_file = mgr.create_identity("Carol", "correct").unwrap();

    let result = mgr.unlock_identity(&identity_file.identity_uuid, "wrong");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::KrillnotesError::IdentityWrongPassphrase
    ));
}

#[test]
fn test_sign_and_verify() {
    use ed25519_dalek::Verifier;

    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity_file = mgr.create_identity("Dave", "pass").unwrap();
    let unlocked = mgr.unlock_identity(&identity_file.identity_uuid, "pass").unwrap();

    let message = b"hello world";
    let signature = unlocked.signing_key.sign(message);
    assert!(unlocked.verifying_key.verify(message, &signature).is_ok());

    // Also verify using the public key loaded from the file (not from unlock)
    let pk_bytes = BASE64.decode(&identity_file.public_key).unwrap();
    let file_vk = ed25519_dalek::VerifyingKey::from_bytes(
        pk_bytes.as_slice().try_into().unwrap()
    ).unwrap();
    assert!(file_vk.verify(message, &signature).is_ok());
}

#[test]
fn test_list_identities() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

    mgr.create_identity("Alice", "pass1").unwrap();
    mgr.create_identity("Bob", "pass2").unwrap();
    mgr.create_identity("Carol", "pass3").unwrap();

    let list = mgr.list_identities().unwrap();
    assert_eq!(list.len(), 3);
    let names: Vec<&str> = list.iter().map(|i| i.display_name.as_str()).collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Bob"));
    assert!(names.contains(&"Carol"));
}

#[test]
fn test_delete_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("ToDelete", "pass").unwrap();

    let identity_dir = mgr.identity_dir(&identity.identity_uuid);
    assert!(identity_dir.join("identity.json").exists());

    mgr.delete_identity(&identity.identity_uuid).unwrap();

    assert!(!identity_dir.exists());
    let list = mgr.list_identities().unwrap();
    assert!(list.is_empty());
}

#[test]
fn test_change_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Eve", "old-pass").unwrap();

    // Unlock with old passphrase — get the public key for comparison
    let unlocked_before = mgr.unlock_identity(&identity.identity_uuid, "old-pass").unwrap();
    let pk_before = *unlocked_before.verifying_key.as_bytes();

    // Change passphrase
    mgr.change_passphrase(&identity.identity_uuid, "old-pass", "new-pass").unwrap();

    // Old passphrase no longer works
    let result = mgr.unlock_identity(&identity.identity_uuid, "old-pass");
    assert!(matches!(result.unwrap_err(), crate::KrillnotesError::IdentityWrongPassphrase));

    // New passphrase works and produces the same keypair
    let unlocked_after = mgr.unlock_identity(&identity.identity_uuid, "new-pass").unwrap();
    assert_eq!(*unlocked_after.verifying_key.as_bytes(), pk_before);
}

#[test]
fn bind_and_get_workspace_binding_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();

    let file = mgr.create_identity("Alice", "pass").unwrap();
    let identity_uuid = file.identity_uuid;
    let unlocked = mgr.unlock_identity(&identity_uuid, "pass").unwrap();
    let seed = unlocked.signing_key.to_bytes();

    let workspace_uuid = Uuid::new_v4().to_string();
    let workspace_dir = mgr.identity_base_dir(&identity_uuid).unwrap().join("MyWorkspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let password = "hunter2";

    mgr.bind_workspace(&identity_uuid, &workspace_uuid, &workspace_dir, password, &seed).unwrap();

    // binding.json must exist
    assert!(workspace_dir.join("binding.json").exists());

    let binding = mgr.get_workspace_binding(&workspace_dir).unwrap().unwrap();
    assert_eq!(binding.workspace_uuid, workspace_uuid);
    assert_eq!(binding.identity_uuid, identity_uuid);

    // Decrypt round-trip
    let decrypted = mgr.decrypt_db_password(&workspace_dir, &seed).unwrap();
    assert_eq!(decrypted, password);
}

#[test]
fn get_workspace_binding_returns_none_when_no_binding_json() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();

    let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();

    assert!(mgr.get_workspace_binding(&ws_dir).unwrap().is_none());
}

#[test]
fn decrypt_db_password_round_trips_multiple_workspaces() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();

    let file = mgr.create_identity("Alice", "pass").unwrap();
    let identity_uuid = file.identity_uuid;
    let unlocked = mgr.unlock_identity(&identity_uuid, "pass").unwrap();
    let seed = unlocked.signing_key.to_bytes();

    let base = mgr.identity_base_dir(&identity_uuid).unwrap();
    for i in 0..3 {
        let ws_uuid = Uuid::new_v4().to_string();
        let ws_dir = base.join(format!("ws{i}"));
        std::fs::create_dir_all(&ws_dir).unwrap();
        let password = format!("pass{i}");
        mgr.bind_workspace(&identity_uuid, &ws_uuid, &ws_dir, &password, &seed).unwrap();
        let decrypted = mgr.decrypt_db_password(&ws_dir, &seed).unwrap();
        assert_eq!(decrypted, password);
    }
}

#[test]
fn unbind_workspace_removes_binding_json() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();

    let file = mgr.create_identity("Alice", "pass").unwrap();
    let base = mgr.identity_base_dir(&file.identity_uuid).unwrap();
    let ws_dir = base.join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let binding_path = ws_dir.join("binding.json");
    std::fs::write(&binding_path, "{}").unwrap();

    mgr.unbind_workspace(&ws_dir).unwrap();
    assert!(!binding_path.exists());
}

#[test]
fn test_multiple_identities_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();

    let id_a = mgr.create_identity("IdentA", "passA").unwrap();
    let id_b = mgr.create_identity("IdentB", "passB").unwrap();
    let unlocked_a = mgr.unlock_identity(&id_a.identity_uuid, "passA").unwrap();
    let unlocked_b = mgr.unlock_identity(&id_b.identity_uuid, "passB").unwrap();

    let ws_a = mgr.identity_base_dir(&id_a.identity_uuid).unwrap().join("ws_a");
    let ws_b = mgr.identity_base_dir(&id_b.identity_uuid).unwrap().join("ws_b");
    std::fs::create_dir_all(&ws_a).unwrap();
    std::fs::create_dir_all(&ws_b).unwrap();

    mgr.bind_workspace(&id_a.identity_uuid, "ws-a-uuid", &ws_a, "pw-a", unlocked_a.signing_key.as_bytes()).unwrap();
    mgr.bind_workspace(&id_b.identity_uuid, "ws-b-uuid", &ws_b, "pw-b", unlocked_b.signing_key.as_bytes()).unwrap();

    // A can decrypt A's workspace
    assert_eq!(mgr.decrypt_db_password(&ws_a, unlocked_a.signing_key.as_bytes()).unwrap(), "pw-a");

    // B can decrypt B's workspace
    assert_eq!(mgr.decrypt_db_password(&ws_b, unlocked_b.signing_key.as_bytes()).unwrap(), "pw-b");

    // A cannot decrypt B's workspace (wrong key, AES-GCM will fail)
    let result = mgr.decrypt_db_password(&ws_b, unlocked_a.signing_key.as_bytes());
    assert!(result.is_err());
}

#[test]
fn delete_identity_fails_if_workspaces_still_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let uuid = file.identity_uuid;
    let unlocked = mgr.unlock_identity(&uuid, "pass").unwrap();
    let seed = unlocked.signing_key.to_bytes();
    let ws_dir = mgr.identity_base_dir(&uuid).unwrap().join("My Workspace");
    std::fs::create_dir_all(&ws_dir).unwrap();
    mgr.bind_workspace(&uuid, "ws-uuid-1", &ws_dir, "db-pass", &seed).unwrap();
    let err = mgr.delete_identity(&uuid).unwrap_err();
    assert!(matches!(err, crate::KrillnotesError::IdentityHasBoundWorkspaces(_)));
}

#[test]
fn test_rename_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Old Name", "pass123").unwrap();
    let uuid = file.identity_uuid;

    mgr.rename_identity(&uuid, "New Name").unwrap();

    // Check list
    let identities = mgr.list_identities().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].display_name, "New Name");

    // Check identity file
    let unlocked = mgr.unlock_identity(&uuid, "pass123").unwrap();
    assert_eq!(unlocked.display_name, "New Name");
}

#[test]
fn get_workspaces_for_identity_scans_identity_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let uuid = file.identity_uuid;
    let unlocked = mgr.unlock_identity(&uuid, "pass").unwrap();
    let seed = unlocked.signing_key.to_bytes();
    let base = mgr.identity_base_dir(&uuid).unwrap();
    for name in &["Work", "Personal"] {
        let ws_dir = base.join(name);
        std::fs::create_dir_all(&ws_dir).unwrap();
        mgr.bind_workspace(&uuid, &format!("uuid-{name}"), &ws_dir, "pass", &seed).unwrap();
    }
    let workspaces = mgr.get_workspaces_for_identity(&uuid).unwrap();
    assert_eq!(workspaces.len(), 2);
}

#[test]
fn test_identity_file_format_matches_spec() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Spec Check", "pass").unwrap();

    // Read the raw JSON file from the identity folder
    let file_path = mgr.identity_file_path(&identity.identity_uuid);
    let raw = std::fs::read_to_string(&file_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

    // Verify top-level keys match spec
    assert!(json.get("identity_uuid").unwrap().is_string());
    assert!(json.get("display_name").unwrap().is_string());
    assert!(json.get("public_key").unwrap().is_string());

    let enc = json.get("private_key_enc").unwrap();
    assert!(enc.get("ciphertext").unwrap().is_string());
    assert!(enc.get("nonce").unwrap().is_string());
    assert_eq!(enc.get("kdf").unwrap().as_str().unwrap(), "argon2id");

    let params = enc.get("kdf_params").unwrap();
    assert!(params.get("salt").unwrap().is_string());
    assert!(params.get("m_cost").unwrap().is_u64());
    assert!(params.get("t_cost").unwrap().is_u64());
    assert!(params.get("p_cost").unwrap().is_u64());
}

#[test]
fn swarmid_file_roundtrip() {
    let inner = IdentityFile {
        identity_uuid: Uuid::new_v4(),
        display_name: "Test".to_string(),
        public_key: "abc".to_string(),
        private_key_enc: EncryptedKey {
            ciphertext: "ct".to_string(),
            nonce: "nn".to_string(),
            kdf: "argon2id".to_string(),
            kdf_params: KdfParams {
                salt: "sl".to_string(),
                m_cost: 1,
                t_cost: 1,
                p_cost: 1,
            },
        },
        last_used: None,
    };
    let swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: inner.clone(),
        relays: vec![],
    };
    let json = serde_json::to_string(&swarmid).unwrap();
    assert!(json.contains("\"format\":\"swarmid\""));
    assert!(json.contains("\"version\":1"));
    let parsed: SwarmIdFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.identity.display_name, "Test");
}

#[test]
fn export_swarmid_wrong_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Alice", "correct-passphrase").unwrap();
    let result = mgr.export_swarmid(&identity.identity_uuid, "wrong-passphrase");
    assert!(matches!(result, Err(crate::KrillnotesError::IdentityWrongPassphrase)));
}

#[test]
fn export_swarmid_correct_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Bob", "my-passphrase").unwrap();
    let swarmid = mgr.export_swarmid(&identity.identity_uuid, "my-passphrase").unwrap();
    assert_eq!(swarmid.format, "swarmid");
    assert_eq!(swarmid.version, 1);
    assert_eq!(swarmid.identity.display_name, "Bob");
    assert_eq!(swarmid.identity.identity_uuid, identity.identity_uuid);
    assert_eq!(swarmid.identity.public_key, identity.public_key);
}

#[test]
fn import_swarmid_adds_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

    // Create identity, export it, then delete to simulate a fresh device
    let original = mgr.create_identity("Charlie", "passphrase").unwrap();
    let swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: original.clone(),
        relays: vec![],
    };
    mgr.delete_identity(&original.identity_uuid).unwrap();
    assert!(mgr.list_identities().unwrap().is_empty());

    let identity_ref = mgr.import_swarmid(swarmid).unwrap();
    assert_eq!(identity_ref.display_name, "Charlie");
    assert_eq!(identity_ref.uuid, original.identity_uuid);

    let identities = mgr.list_identities().unwrap();
    assert_eq!(identities.len(), 1);
}

#[test]
fn import_swarmid_collision_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let original = mgr.create_identity("Dave", "passphrase").unwrap();
    let swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: original.clone(),
        relays: vec![],
    };
    // Import again — same UUID should fail with IdentityAlreadyExists
    let result = mgr.import_swarmid(swarmid);
    assert!(matches!(result, Err(crate::KrillnotesError::IdentityAlreadyExists(_))));
}

#[test]
fn import_swarmid_overwrite_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let original = mgr.create_identity("Eve", "passphrase").unwrap();
    let mut swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: original.clone(),
        relays: vec![],
    };
    swarmid.identity.display_name = "Eve Updated".to_string();
    let identity_ref = mgr.import_swarmid_overwrite(swarmid).unwrap();
    assert_eq!(identity_ref.display_name, "Eve Updated");
    // Only one identity in list
    assert_eq!(mgr.list_identities().unwrap().len(), 1);
}

#[test]
fn import_swarmid_invalid_format_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Test", "pass").unwrap();

    let bad_format = SwarmIdFile {
        format: "notswarmid".to_string(),
        version: 1,
        identity: identity.clone(),
        relays: vec![],
    };
    assert!(matches!(
        mgr.import_swarmid(bad_format),
        Err(crate::KrillnotesError::SwarmIdInvalidFormat(_))
    ));

    let bad_version = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: 99,
        identity,
        relays: vec![],
    };
    assert!(matches!(
        mgr.import_swarmid(bad_version),
        Err(crate::KrillnotesError::SwarmIdVersionUnsupported(99))
    ));
}

#[test]
fn contacts_key_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr
        .create_identity("Test User", "passphrase123")
        .unwrap();
    let unlocked = mgr
        .unlock_identity(&identity.identity_uuid, "passphrase123")
        .unwrap();
    let key1 = unlocked.contacts_key();
    let key2 = unlocked.contacts_key();
    assert_eq!(key1, key2, "contacts_key must be deterministic");
    assert_eq!(key1.len(), 32);
    // Must differ from a different identity
    let identity2 = mgr
        .create_identity("Other User", "passphrase123")
        .unwrap();
    let unlocked2 = mgr
        .unlock_identity(&identity2.identity_uuid, "passphrase123")
        .unwrap();
    assert_ne!(unlocked.contacts_key(), unlocked2.contacts_key());
}

#[test]
fn workspace_binding_serialises_with_workspace_uuid() {
    let b = WorkspaceBinding {
        workspace_uuid: "ws-1".to_string(),
        identity_uuid: Uuid::nil(),
        db_password_enc: "enc".to_string(),
    };
    let json = serde_json::to_string(&b).unwrap();
    assert!(json.contains("workspace_uuid"));
    assert!(json.contains("identity_uuid"));
    assert!(!json.contains("db_path"));
}

#[test]
fn identity_dir_returns_display_name_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let dir = mgr.identity_dir(&file.identity_uuid);
    assert_eq!(dir, tmp.path().join("Alice").join(".identity"));
}

#[test]
fn identity_file_path_returns_identity_json_inside_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let path = mgr.identity_file_path(&file.identity_uuid);
    assert_eq!(path, tmp.path().join("Alice").join(".identity").join("identity.json"));
}

#[test]
fn test_relay_key_differs_from_contacts_key() {
    // All imports (SigningKey, Uuid, UnlockedIdentity) are in scope via super::*
    let signing_key = SigningKey::generate(&mut rand_core::OsRng);
    let verifying_key = signing_key.verifying_key();
    let unlocked = UnlockedIdentity {
        identity_uuid: Uuid::new_v4(),
        display_name: "Test".to_string(),
        signing_key,
        verifying_key,
    };
    assert_ne!(unlocked.relay_key(), unlocked.contacts_key(),
        "relay_key and contacts_key must differ");
}

#[test]
fn test_relay_key_deterministic() {
    let seed = [0x11u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    let unlocked = UnlockedIdentity {
        identity_uuid: Uuid::new_v4(),
        display_name: "Test".to_string(),
        signing_key,
        verifying_key,
    };
    assert_eq!(unlocked.relay_key(), unlocked.relay_key(),
        "relay_key must be deterministic");
}

// ---------------------------------------------------------------------------
// Tests for ensure_device_uuid and identity_from_device_id
// ---------------------------------------------------------------------------

#[test]
fn test_ensure_device_uuid_creates_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let identity_dir = dir.path();

    // First call: file does not exist — should create and return a UUID.
    let uuid1 = super::ensure_device_uuid(identity_dir).expect("first call should succeed");
    assert!(!uuid1.is_empty(), "UUID should not be empty");
    // Should be a valid UUID format.
    assert!(uuid::Uuid::parse_str(&uuid1).is_ok(), "Must be a valid UUID string: {uuid1}");

    // Second call: file now exists — should return the same UUID.
    let uuid2 = super::ensure_device_uuid(identity_dir).expect("second call should succeed");
    assert_eq!(uuid1, uuid2, "Second call must return the same UUID");

    // The file should exist on disk.
    let device_id_path = identity_dir.join("device_id");
    assert!(device_id_path.exists(), "device_id file must exist after ensure_device_uuid");
}

#[test]
fn test_ensure_device_uuid_reads_existing() {
    let dir = tempfile::tempdir().unwrap();
    let identity_dir = dir.path();
    let device_id_path = identity_dir.join("device_id");

    // Pre-write a known UUID.
    let known_uuid = "550e8400-e29b-41d4-a716-446655440000";
    std::fs::write(&device_id_path, known_uuid).unwrap();

    let result = super::ensure_device_uuid(identity_dir).expect("should read existing UUID");
    assert_eq!(result, known_uuid, "Should return the pre-existing UUID");
}

#[test]
fn test_identity_from_device_id_composite() {
    let device_id = "alice-uuid:device-uuid";
    assert_eq!(super::identity_from_device_id(device_id), "alice-uuid");
}

#[test]
fn test_identity_from_device_id_legacy() {
    let device_id = "legacy-uuid-only";
    assert_eq!(super::identity_from_device_id(device_id), "legacy-uuid-only");
}

// ---------------------------------------------------------------------------
// New tests for unified storage layout
// ---------------------------------------------------------------------------

#[test]
fn create_identity_handles_name_collision() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let _a = mgr.create_identity("Alice", "pass1").unwrap();
    let b = mgr.create_identity("Alice", "pass2").unwrap();
    let base_b = mgr.identity_base_dir(&b.identity_uuid).unwrap();
    assert_eq!(base_b.file_name().unwrap().to_str().unwrap(), "Alice (2)");
}

#[test]
fn new_discovers_existing_identities() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
        mgr.create_identity("Alice", "pass").unwrap();
        mgr.create_identity("Bob", "pass").unwrap();
    }
    let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let identities = mgr.list_identities().unwrap();
    assert_eq!(identities.len(), 2);
    let names: Vec<_> = identities.iter().map(|i| i.display_name.as_str()).collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Bob"));
}
