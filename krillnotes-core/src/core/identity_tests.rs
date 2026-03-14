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
    };
    let json = serde_json::to_string_pretty(&file).unwrap();
    let parsed: IdentityFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.identity_uuid, file.identity_uuid);
    assert_eq!(parsed.display_name, "Test User");
    assert_eq!(parsed.private_key_enc.kdf, "argon2id");
}

#[test]
fn test_identity_settings_default_empty() {
    let settings = IdentitySettings::default();
    assert!(settings.identities.is_empty());
    assert!(settings.workspaces.is_empty());
}

#[test]
fn test_identity_manager_creates_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    assert!(mgr.identities_dir().exists());
}

#[test]
fn test_settings_load_save_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

    // Fresh — no file yet, returns default
    let settings = mgr.load_settings().unwrap();
    assert!(settings.identities.is_empty());

    // Save and reload
    let mut settings = IdentitySettings::default();
    settings.identities.push(IdentityRef {
        uuid: Uuid::new_v4(),
        display_name: "Test".to_string(),
        file: "identities/test.json".to_string(),
        last_used: Utc::now(),
    });
    mgr.save_settings(&settings).unwrap();

    let reloaded = mgr.load_settings().unwrap();
    assert_eq!(reloaded.identities.len(), 1);
    assert_eq!(reloaded.identities[0].display_name, "Test");
}

#[test]
fn test_create_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

    let identity_file = mgr.create_identity("Alice", "password123").unwrap();

    // File was written in the new per-identity subfolder
    let file_path = dir.path().join("identities")
        .join(identity_file.identity_uuid.to_string())
        .join("identity.json");
    assert!(file_path.exists());

    // Settings updated
    let settings = mgr.load_settings().unwrap();
    assert_eq!(settings.identities.len(), 1);
    assert_eq!(settings.identities[0].display_name, "Alice");
    assert_eq!(settings.identities[0].uuid, identity_file.identity_uuid);

    // Public key is valid base64 and 32 bytes
    let pk_bytes = BASE64.decode(&identity_file.public_key).unwrap();
    assert_eq!(pk_bytes.len(), 32);

    // KDF params match expectations
    assert_eq!(identity_file.private_key_enc.kdf, "argon2id");
    assert_eq!(identity_file.private_key_enc.kdf_params.p_cost, 1);
}

#[test]
fn test_unlock_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("ToDelete", "pass").unwrap();

    let identity_dir = dir.path().join("identities").join(identity.identity_uuid.to_string());
    assert!(identity_dir.join("identity.json").exists());

    // Empty workspace base dir — no bound workspaces
    let ws_base = dir.path().join("workspaces");
    std::fs::create_dir_all(&ws_base).unwrap();
    mgr.delete_identity(&identity.identity_uuid, &ws_base).unwrap();

    assert!(!identity_dir.exists());
    let list = mgr.list_identities().unwrap();
    assert!(list.is_empty());
}

#[test]
fn test_change_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
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
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir).unwrap();

    let identity_uuid = Uuid::new_v4();
    let workspace_uuid = Uuid::new_v4().to_string();
    let workspace_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let seed = [42u8; 32];
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

    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir).unwrap();

    assert!(mgr.get_workspace_binding(&ws_dir).unwrap().is_none());
}

#[test]
fn decrypt_db_password_round_trips_multiple_workspaces() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir).unwrap();
    let identity_uuid = Uuid::new_v4();
    let seed = [7u8; 32];

    for i in 0..3 {
        let ws_uuid = Uuid::new_v4().to_string();
        let ws_dir = tmp.path().join(format!("ws{i}"));
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
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let binding_path = ws_dir.join("binding.json");
    std::fs::write(&binding_path, "{}").unwrap();

    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir).unwrap();

    mgr.unbind_workspace(&ws_dir).unwrap();
    assert!(!binding_path.exists());
}

#[test]
fn test_multiple_identities_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir).unwrap();

    let id_a = mgr.create_identity("IdentA", "passA").unwrap();
    let id_b = mgr.create_identity("IdentB", "passB").unwrap();
    let unlocked_a = mgr.unlock_identity(&id_a.identity_uuid, "passA").unwrap();
    let unlocked_b = mgr.unlock_identity(&id_b.identity_uuid, "passB").unwrap();

    let ws_a = tmp.path().join("ws_a");
    let ws_b = tmp.path().join("ws_b");
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
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir.clone()).unwrap();

    let seed = [1u8; 32];
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();

    // Create the identity first
    let display_name = "Test";
    let passphrase = "testpass";
    mgr.create_identity(display_name, passphrase).unwrap();

    // Get the UUID we just created
    let settings = mgr.load_settings().unwrap();
    let id_ref = settings.identities.first().unwrap();
    let real_uuid = id_ref.uuid;

    // Bind a workspace to it
    let ws_uuid = Uuid::new_v4().to_string();
    mgr.bind_workspace(&real_uuid, &ws_uuid, &ws_dir, "pass", &seed).unwrap();

    // delete_identity must fail because a workspace is still bound
    let ws_base = tmp.path().to_path_buf();
    let result = mgr.delete_identity(&real_uuid, &ws_base);
    assert!(result.is_err(), "should fail when workspaces are bound");
}

#[test]
fn test_rename_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Old Name", "pass123").unwrap();
    let uuid = file.identity_uuid;

    mgr.rename_identity(&uuid, "New Name").unwrap();

    // Check settings
    let identities = mgr.list_identities().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].display_name, "New Name");

    // Check identity file
    let unlocked = mgr.unlock_identity(&uuid, "pass123").unwrap();
    assert_eq!(unlocked.display_name, "New Name");
}

#[test]
fn get_workspaces_for_identity_scans_workspace_base_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let mgr = IdentityManager::new(config_dir).unwrap();

    let identity_a = Uuid::new_v4();
    let identity_b = Uuid::new_v4();
    let ws_base = tmp.path().join("workspaces");

    // Two workspaces for identity_a, one for identity_b
    for (name, owner) in &[("ws1", identity_a), ("ws2", identity_a), ("ws3", identity_b)] {
        let ws_dir = ws_base.join(name);
        std::fs::create_dir_all(&ws_dir).unwrap();
        let binding = WorkspaceBinding {
            workspace_uuid: Uuid::new_v4().to_string(),
            identity_uuid: *owner,
            db_password_enc: "enc".to_string(),
        };
        std::fs::write(
            ws_dir.join("binding.json"),
            serde_json::to_string(&binding).unwrap()
        ).unwrap();
    }
    // ws4 has no binding.json — must be ignored
    std::fs::create_dir_all(ws_base.join("ws4")).unwrap();

    let results = mgr.get_workspaces_for_identity(&identity_a, &ws_base).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(_, b)| b.identity_uuid == identity_a));
}

#[test]
fn test_identity_file_format_matches_spec() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Spec Check", "pass").unwrap();

    // Read the raw JSON file from the new per-identity subfolder
    let file_path = dir.path().join("identities")
        .join(identity.identity_uuid.to_string())
        .join("identity.json");
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
    };
    let swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: inner.clone(),
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Alice", "correct-passphrase").unwrap();
    let result = mgr.export_swarmid(&identity.identity_uuid, "wrong-passphrase");
    assert!(matches!(result, Err(crate::KrillnotesError::IdentityWrongPassphrase)));
}

#[test]
fn export_swarmid_correct_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

    // Create identity, export it, then delete to simulate a fresh device
    let original = mgr.create_identity("Charlie", "passphrase").unwrap();
    let swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: original.clone(),
    };
    let ws_base = dir.path().join("workspaces");
    std::fs::create_dir_all(&ws_base).unwrap();
    mgr.delete_identity(&original.identity_uuid, &ws_base).unwrap();
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let original = mgr.create_identity("Dave", "passphrase").unwrap();
    let swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: original.clone(),
    };
    // Import again — same UUID should fail with IdentityAlreadyExists
    let result = mgr.import_swarmid(swarmid);
    assert!(matches!(result, Err(crate::KrillnotesError::IdentityAlreadyExists(_))));
}

#[test]
fn import_swarmid_overwrite_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let original = mgr.create_identity("Eve", "passphrase").unwrap();
    let mut swarmid = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: SwarmIdFile::VERSION,
        identity: original.clone(),
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
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let identity = mgr.create_identity("Test", "pass").unwrap();

    let bad_format = SwarmIdFile {
        format: "notswarmid".to_string(),
        version: 1,
        identity: identity.clone(),
    };
    assert!(matches!(
        mgr.import_swarmid(bad_format),
        Err(crate::KrillnotesError::SwarmIdInvalidFormat(_))
    ));

    let bad_version = SwarmIdFile {
        format: SwarmIdFile::FORMAT.to_string(),
        version: 99,
        identity,
    };
    assert!(matches!(
        mgr.import_swarmid(bad_version),
        Err(crate::KrillnotesError::SwarmIdVersionUnsupported(99))
    ));
}

#[test]
fn contacts_key_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
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
fn old_identity_settings_with_workspaces_key_deserialises() {
    // Old format still deserialises (workspaces key is readable)
    let json = r#"{
        "identities": [],
        "workspaces": {
            "ws-uuid-1": {
                "db_path": "/tmp/foo/notes.db",
                "identity_uuid": "00000000-0000-0000-0000-000000000001",
                "db_password_enc": "aGVsbG8="
            }
        }
    }"#;
    let settings: IdentitySettings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.workspaces.len(), 1);
    let binding = settings.workspaces.get("ws-uuid-1").unwrap();
    assert_eq!(binding.db_path, "/tmp/foo/notes.db");
}

#[test]
fn new_identity_settings_serialises_without_workspaces_key() {
    let settings = IdentitySettings::default();
    let json = serde_json::to_string(&settings).unwrap();
    assert!(!json.contains("workspaces"),
        "workspaces key must not appear in serialised output");
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
fn identity_dir_returns_uuid_subfolder() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let uuid = Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
    assert_eq!(
        mgr.identity_dir(&uuid),
        tmp.path().join("identities").join("aaaaaaaa-0000-0000-0000-000000000001")
    );
}

#[test]
fn identity_file_path_returns_identity_json_inside_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let uuid = Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
    assert_eq!(
        mgr.identity_file_path(&uuid),
        tmp.path().join("identities").join("aaaaaaaa-0000-0000-0000-000000000001").join("identity.json")
    );
}

#[test]
fn migration_pass1_moves_flat_json_into_identity_subfolder() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let identities_dir = config_dir.join("identities");
    std::fs::create_dir_all(&identities_dir).unwrap();

    // Create legacy flat identity file
    let uuid = Uuid::new_v4();
    let legacy_path = identities_dir.join(format!("{uuid}.json"));
    let identity_file = serde_json::json!({
        "identity_uuid": uuid.to_string(),
        "display_name": "Test",
        "public_key": "dGVzdA==",
        "private_key_enc": {
            "ciphertext": "dGVzdA==",
            "nonce": "dGVzdA==",
            "kdf": "argon2id",
            "kdf_params": { "salt": "dGVzdA==", "m_cost": 1024, "t_cost": 1, "p_cost": 1 }
        }
    });
    std::fs::write(&legacy_path, serde_json::to_string(&identity_file).unwrap()).unwrap();

    // Create identity_settings.json referencing the flat file
    let settings = serde_json::json!({
        "identities": [{
            "uuid": uuid.to_string(),
            "displayName": "Test",
            "file": format!("identities/{uuid}.json"),
            "lastUsed": "2026-01-01T00:00:00Z"
        }]
    });
    std::fs::write(config_dir.join("identity_settings.json"),
        serde_json::to_string(&settings).unwrap()).unwrap();

    // Trigger migration
    let _mgr = IdentityManager::new(config_dir.clone()).unwrap();

    // Flat file must be gone
    assert!(!legacy_path.exists(), "flat file should be removed");

    // New path must exist
    let new_path = identities_dir.join(uuid.to_string()).join("identity.json");
    assert!(new_path.exists(), "identity.json inside folder must exist");

    // settings must be updated
    let raw = std::fs::read_to_string(config_dir.join("identity_settings.json")).unwrap();
    let updated: IdentitySettings = serde_json::from_str(&raw).unwrap();
    assert_eq!(updated.identities[0].file,
        format!("identities/{uuid}/identity.json"));
}

#[test]
fn migration_pass1_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    // First call (no legacy files) — should succeed silently
    let _m1 = IdentityManager::new(config_dir.clone()).unwrap();
    // Second call — must also succeed
    let _m2 = IdentityManager::new(config_dir.clone()).unwrap();
}

#[test]
fn migration_pass2_writes_binding_json_for_existing_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();

    // Create a fake workspace folder with notes.db
    let ws_dir = tmp.path().join("workspaces").join("my-workspace");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("notes.db"), b"").unwrap();

    let ws_uuid = "aaaaaaaa-1111-0000-0000-000000000001";
    let identity_uuid = "bbbbbbbb-2222-0000-0000-000000000001";

    // Write legacy identity_settings.json with workspaces section
    let settings_json = serde_json::json!({
        "identities": [],
        "workspaces": {
            ws_uuid: {
                "db_path": ws_dir.join("notes.db").display().to_string(),
                "identity_uuid": identity_uuid,
                "db_password_enc": "dGVzdA=="
            }
        }
    });
    std::fs::write(
        config_dir.join("identity_settings.json"),
        serde_json::to_string(&settings_json).unwrap()
    ).unwrap();

    // Trigger migration
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();
    let _mgr = IdentityManager::new(config_dir.clone()).unwrap();

    // binding.json must exist in workspace folder
    let binding_path = ws_dir.join("binding.json");
    assert!(binding_path.exists(), "binding.json must be written");

    let raw = std::fs::read_to_string(&binding_path).unwrap();
    let binding: WorkspaceBinding = serde_json::from_str(&raw).unwrap();
    assert_eq!(binding.workspace_uuid, ws_uuid);
    assert_eq!(binding.identity_uuid.to_string(), identity_uuid);
    assert_eq!(binding.db_password_enc, "dGVzdA==");

    // identity_settings.json must no longer have workspaces key
    let raw_settings = std::fs::read_to_string(config_dir.join("identity_settings.json")).unwrap();
    assert!(!raw_settings.contains("workspaces"),
        "workspaces key must be absent after migration");
}

#[test]
fn migration_pass2_drops_stale_entry_for_missing_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(config_dir.join("identities")).unwrap();

    // Stale binding — workspace folder does not exist
    let settings_json = serde_json::json!({
        "identities": [],
        "workspaces": {
            "dead-ws-uuid": {
                "db_path": "/nonexistent/workspace/notes.db",
                "identity_uuid": "00000000-0000-0000-0000-000000000001",
                "db_password_enc": "dGVzdA=="
            }
        }
    });
    std::fs::write(
        config_dir.join("identity_settings.json"),
        serde_json::to_string(&settings_json).unwrap()
    ).unwrap();

    // Must not panic
    let _mgr = IdentityManager::new(config_dir.clone()).unwrap();

    // identity_settings.json cleaned up
    let raw = std::fs::read_to_string(config_dir.join("identity_settings.json")).unwrap();
    assert!(!raw.contains("workspaces"));
}

#[test]
fn test_relay_key_differs_from_contacts_key() {
    // All imports (SigningKey, Uuid, UnlockedIdentity) are in scope via super::*
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
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
