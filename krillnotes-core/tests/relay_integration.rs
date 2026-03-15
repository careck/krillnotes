// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Integration tests for the Krillnotes sync engine.
//!
//! # Relay tests (ignored by default)
//!
//! Tests marked `#[ignore]` require a running relay server. Run them with:
//!
//! ```sh
//! RELAY_URL=http://localhost:8080 cargo test -p krillnotes-core --features relay -- --ignored relay_
//! ```
//!
//! # Folder channel test (always runs)
//!
//! `folder_channel_delta_roundtrip` exercises the full generate → write file →
//! read file → apply pipeline using a temporary directory as the shared folder.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tempfile::NamedTempFile;

use krillnotes_core::{
    core::{
        contact::{ContactManager, TrustLevel},
        sync::{
            channel::{ChannelType, PeerSyncInfo, SyncChannel},
            folder::FolderChannel,
            SyncContext, SyncEngine,
        },
        swarm::sync::{apply_delta, generate_delta},
    },
    Workspace,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

fn b64_pubkey(key: &SigningKey) -> String {
    BASE64.encode(key.verifying_key().as_bytes())
}

/// Create an in-memory (temp-file backed) workspace.
fn make_workspace(key: &SigningKey, identity_id: &str) -> (NamedTempFile, Workspace) {
    let tmp = NamedTempFile::new().expect("tempfile");
    let ws = Workspace::create(tmp.path(), "", identity_id, SigningKey::from_bytes(&key.to_bytes()))
        .expect("Workspace::create");
    (tmp, ws)
}

/// Create a `ContactManager` backed by a temp dir.
fn make_contact_manager(enc_key: [u8; 32]) -> (tempfile::TempDir, ContactManager) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cm = ContactManager::for_identity(dir.path().to_path_buf(), enc_key)
        .expect("ContactManager::for_identity");
    (dir, cm)
}

// ── Relay tests (ignored) ─────────────────────────────────────────────────────

/// End-to-end relay registration flow:
/// 1. Generate an Ed25519 keypair (acts as the device key).
/// 2. Register a new account on the relay.
/// 3. Solve the proof-of-possession challenge (decrypt the encrypted nonce).
/// 4. Verify the session token works by fetching account info.
/// 5. Clean up: log out.
///
/// Run with:
/// ```sh
/// RELAY_URL=http://localhost:8080 cargo test -p krillnotes-core --features relay -- --ignored relay_registration_flow
/// ```
#[test]
#[ignore]
#[cfg(feature = "relay")]
fn relay_registration_flow() {
    use krillnotes_core::core::sync::relay::{auth::decrypt_pop_challenge, client::RelayClient};
    use uuid::Uuid;

    let relay_url = std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    // 1. Generate identity keypair.
    let device_key = make_key();
    let device_pubkey_b64 = b64_pubkey(&device_key);

    let identity_uuid = Uuid::new_v4().to_string();
    let email = format!("test-{}@example.com", &identity_uuid[..8]);
    let password = "test-password-integration";

    let client = RelayClient::new(&relay_url);

    // 2. Register account — receive PoP challenge.
    let reg = client
        .register(&email, password, &identity_uuid, &device_pubkey_b64)
        .expect("register should succeed");

    // 3. Solve PoP challenge: decrypt the nonce and re-encode as hex.
    let plaintext_nonce = decrypt_pop_challenge(
        &device_key,
        &reg.challenge.encrypted_nonce,
        &reg.challenge.server_public_key,
    )
    .expect("decrypt_pop_challenge should succeed");
    let nonce_hex = hex::encode(&plaintext_nonce);

    // 4. Verify registration → receive session token.
    let session = client
        .register_verify(&device_pubkey_b64, &nonce_hex)
        .expect("register_verify should succeed");

    assert!(!session.session_token.is_empty(), "session token must be non-empty");

    // Verify the token works.
    let authed_client = RelayClient::new(&relay_url).with_session_token(&session.session_token);
    let account = authed_client.get_account().expect("get_account should succeed");
    assert_eq!(account.identity_uuid, identity_uuid);

    // 5. Clean up: log out.
    authed_client.logout().expect("logout should succeed");
}

/// End-to-end two-identity delta sync via relay:
/// 1. Create two identities (Alice, Bob).
/// 2. Register both on the relay.
/// 3. Alice creates a workspace and registers Bob as a peer.
/// 4. Alice generates a delta bundle and uploads it to the relay.
/// 5. Bob polls the relay, downloads the bundle, and applies it.
/// 6. Verify Bob's workspace received Alice's operations.
///
/// Run with:
/// ```sh
/// RELAY_URL=http://localhost:8080 cargo test -p krillnotes-core --features relay -- --ignored relay_delta_roundtrip
/// ```
#[test]
#[ignore]
#[cfg(feature = "relay")]
fn relay_delta_roundtrip() {
    use krillnotes_core::core::sync::relay::{
        auth::decrypt_pop_challenge,
        client::RelayClient,
    };
    use uuid::Uuid;

    let relay_url = std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    // ── 1. Create identities ────────────────────────────────────────────────
    let alice_key = make_key();
    let bob_key = make_key();
    let bob_pubkey_b64 = b64_pubkey(&bob_key);

    let alice_uuid = Uuid::new_v4().to_string();
    let bob_uuid = Uuid::new_v4().to_string();
    let alice_email = format!("alice-{}@example.com", &alice_uuid[..8]);
    let bob_email = format!("bob-{}@example.com", &bob_uuid[..8]);
    let password = "test-password-relay-delta";

    // ── 2. Register both on relay ───────────────────────────────────────────
    let register_and_verify = |key: &SigningKey, uuid: &str, email: &str| -> RelayClient {
        let relay = RelayClient::new(&relay_url);
        let pk = b64_pubkey(key);
        let reg = relay.register(email, password, uuid, &pk).expect("register");
        let nonce = decrypt_pop_challenge(key, &reg.challenge.encrypted_nonce, &reg.challenge.server_public_key)
            .expect("decrypt_pop_challenge");
        let session = relay
            .register_verify(&pk, &hex::encode(&nonce))
            .expect("register_verify");
        RelayClient::new(&relay_url).with_session_token(&session.session_token)
    };

    let alice_client = register_and_verify(&alice_key, &alice_uuid, &alice_email);
    let bob_client = register_and_verify(&bob_key, &bob_uuid, &bob_email);

    // ── 3. Alice creates workspace, registers Bob as peer ──────────────────
    let (alice_tmp, mut alice_ws) = make_workspace(&alice_key, &alice_uuid);
    let (_alice_cm_dir, alice_cm) = make_contact_manager([0xAAu8; 32]);

    // Set snapshot watermark BEFORE adding the note, so the note op is in the delta.
    let snap_op = alice_ws
        .get_latest_operation_id()
        .expect("get_latest_operation_id")
        .unwrap_or_default();

    // Register Bob as a peer with snapshot watermark.
    alice_ws
        .upsert_sync_peer("bob-device", &bob_pubkey_b64, Some(&snap_op), None)
        .expect("upsert_sync_peer");

    // Add a note AFTER the watermark so it's included in the delta.
    alice_ws.create_note_root("TextNote").expect("create_note_root");

    // Register Bob as a contact so the encryption key can be resolved.
    alice_cm
        .find_or_create_by_public_key("Bob", &bob_pubkey_b64, TrustLevel::Tofu)
        .expect("find_or_create_by_public_key");

    // ── 4. Alice generates delta and uploads to relay ───────────────────────
    alice_client
        .ensure_mailbox(alice_ws.workspace_id())
        .expect("ensure_mailbox");

    let bundle = generate_delta(
        &mut alice_ws,
        "bob-device",
        "TestRelayWorkspace",
        &alice_key,
        "Alice",
        &alice_cm,
    )
    .expect("generate_delta");

    use krillnotes_core::core::sync::relay::client::BundleHeader;
    let header = BundleHeader {
        workspace_id: alice_ws.workspace_id().to_string(),
        sender_device_key: b64_pubkey(&alice_key),
        recipient_device_keys: vec![bob_pubkey_b64.clone()],
        mode: Some("delta".to_string()),
    };
    let bundle_ids = alice_client.upload_bundle(&header, &bundle.bundle_bytes).expect("upload_bundle");
    let bundle_id = bundle_ids.into_iter().next().expect("relay returned at least one bundle_id");

    // ── 5. Bob downloads and applies the delta ─────────────────────────────
    // Drop alice_ws before Bob opens the same DB file to avoid concurrent access.
    drop(alice_ws);

    // Bob opens the same DB so workspace_id matches the bundle.
    let mut bob_ws = Workspace::open(
        alice_tmp.path(),
        "",
        &bob_uuid,
        SigningKey::from_bytes(&bob_key.to_bytes()),
    )
    .expect("Workspace::open");
    let (_bob_cm_dir, mut bob_cm) = make_contact_manager([0xBBu8; 32]);

    let bundle_bytes = bob_client.download_bundle(&bundle_id).expect("bob download bundle");
    let result = apply_delta(&bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm)
        .expect("apply_delta");
    let applied_total = result.operations_applied;
    bob_client.delete_bundle(&bundle_id).expect("delete_bundle");

    // ── 6. Verify Bob has Alice's operations ───────────────────────────────
    assert!(
        applied_total > 0,
        "Bob should have applied at least one operation"
    );

    // Clean up relay sessions.
    alice_client.logout().ok();
    bob_client.logout().ok();
}

// ── Folder channel test (always runs) ────────────────────────────────────────

/// End-to-end folder channel delta roundtrip:
/// 1. Alice creates a workspace.
/// 2. Bob creates a fresh, empty workspace with the same workspace_id as Alice's
///    (using `create_with_id`) so `apply_delta`'s workspace_id check passes.
///    Bob's `owner_pubkey` is set to Alice's pubkey so the bundle header check passes.
/// 3. Alice registers Bob as a folder-channel peer with a snapshot watermark set
///    BEFORE the note is created, so the note op is included in the delta.
/// 4. Alice adds a note then generates a delta and writes it via `FolderChannel`.
/// 5. Bob's `FolderChannel` picks up the bundle and applies it.
/// 6. Verify the note arrived in Bob's workspace.
/// 7. Acknowledge (delete) the bundle file.
#[test]
fn folder_channel_delta_roundtrip() {
    // ── Setup ────────────────────────────────────────────────────────────────
    let alice_key = make_key();
    let bob_key = make_key();
    let alice_pubkey_b64 = b64_pubkey(&alice_key);
    let bob_pubkey_b64 = b64_pubkey(&bob_key);

    // Alice creates her workspace.
    let (_alice_tmp, mut alice_ws) = make_workspace(&alice_key, "alice-id");
    let (_alice_cm_dir, alice_cm) = make_contact_manager([0x11u8; 32]);

    // Bob creates a FRESH, SEPARATE database with the same workspace_id so
    // `apply_delta` does not reject the bundle with a workspace_id mismatch.
    // Then the owner_pubkey is overwritten with Alice's pubkey so the bundle's
    // owner_pubkey field also matches.
    let bob_tmp = NamedTempFile::new().expect("bob_tmp");
    let mut bob_ws = Workspace::create_with_id(
        bob_tmp.path(),
        "",
        "bob-id",
        SigningKey::from_bytes(&bob_key.to_bytes()),
        alice_ws.workspace_id(),
    )
    .expect("Workspace::create_with_id for Bob");
    bob_ws
        .set_owner_pubkey(&alice_pubkey_b64)
        .expect("set_owner_pubkey to Alice");
    let (_bob_cm_dir, mut bob_cm) = make_contact_manager([0x22u8; 32]);

    // ── Step 1: Snapshot watermark BEFORE note creation ──────────────────────
    // Setting the watermark first means the subsequent CreateNote op is strictly
    // after it and will be included in the delta.
    let snap_op = alice_ws
        .get_latest_operation_id()
        .expect("get_latest_operation_id")
        .unwrap_or_default();

    alice_ws
        .upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None)
        .expect("upsert_sync_peer (Alice→Bob)");

    // ── Step 2: Alice adds a note AFTER the watermark ────────────────────────
    alice_ws.create_note_root("TextNote").expect("create_note_root");

    // Register Bob as a contact so the encryption key can be resolved.
    alice_cm
        .find_or_create_by_public_key("Bob", &bob_pubkey_b64, TrustLevel::Tofu)
        .expect("Alice registers Bob as contact");

    // ── Step 3: Shared folder ──────────────────────────────────────────────
    let shared_dir = tempfile::tempdir().expect("shared_dir");
    let shared_path = shared_dir.path().to_str().expect("valid path").to_string();

    // ── Step 4: Alice generates delta and writes via FolderChannel ──────────
    let alice_peer_for_bob = PeerSyncInfo {
        peer_device_id: "dev-bob".to_string(),
        peer_identity_id: bob_pubkey_b64.clone(),
        channel_type: ChannelType::Folder,
        channel_params: serde_json::json!({ "path": &shared_path }),
        last_sent_op: Some(snap_op.clone()),
        last_received_op: None,
    };

    let bundle = generate_delta(
        &mut alice_ws,
        "dev-bob",
        "TestFolderWorkspace",
        &alice_key,
        "Alice",
        &alice_cm,
    )
    .expect("generate_delta");

    let alice_folder_ch = FolderChannel::new(
        "alice-identity-uuid".to_string(),
        "alice-device-uuid".to_string(),
    );
    alice_folder_ch
        .send_bundle(&alice_peer_for_bob, &bundle.bundle_bytes)
        .expect("send_bundle to shared folder");

    // Verify a .swarm file was written.
    let files_in_dir: Vec<_> = std::fs::read_dir(shared_dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "swarm"))
        .collect();
    assert_eq!(files_in_dir.len(), 1, "exactly one .swarm file should exist");

    // ── Step 5: Bob's FolderChannel picks up the bundle ────────────────────
    let bob_folder_ch = FolderChannel::new(
        "bob-identity-uuid".to_string(),
        "bob-device-uuid".to_string(),
    );
    let received = bob_folder_ch
        .receive_bundles_from_dir(shared_dir.path())
        .expect("receive_bundles_from_dir");

    assert_eq!(
        received.len(),
        1,
        "Bob should receive exactly one bundle from the shared folder"
    );

    // ── Step 6: Apply delta to Bob's workspace ─────────────────────────────
    let result = apply_delta(&received[0].data, &mut bob_ws, &bob_key, &mut bob_cm)
        .expect("apply_delta");

    // Bob should have applied at least one operation (the CreateNote).
    assert!(
        result.operations_applied > 0,
        "Bob should have applied at least one operation; applied={}, skipped={}",
        result.operations_applied,
        result.operations_skipped,
    );
    assert_eq!(
        result.sender_public_key, alice_pubkey_b64,
        "sender public key in result should match Alice's key"
    );

    // Verify the note arrived in Bob's workspace.
    let notes = bob_ws.list_all_notes().expect("list_all_notes");
    assert!(
        !notes.is_empty(),
        "Bob's workspace should contain at least one note after applying Alice's delta"
    );

    // ── Step 7: Acknowledge (delete) the bundle file ───────────────────────
    bob_folder_ch
        .acknowledge(&received[0])
        .expect("acknowledge should delete the .swarm file");

    let remaining: Vec<_> = std::fs::read_dir(shared_dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "swarm"))
        .collect();
    assert!(
        remaining.is_empty(),
        "the .swarm file should have been deleted after acknowledge"
    );
}

// ── SyncEngine integration (folder channel) ──────────────────────────────────

/// Verify `SyncEngine::poll` handles the folder channel outbound path:
/// Alice registers Bob as a folder-channel peer, then polls — the engine
/// should generate a delta, write it to the folder, and emit a `DeltaSent` event.
///
/// This test verifies the outbound half of `poll()` and checks a file lands
/// in the shared folder, but does NOT apply the delta on Bob's side.
#[test]
fn sync_engine_poll_outbound_folder() {
    use krillnotes_core::core::sync::SyncEvent;

    let alice_key = make_key();
    let bob_key = make_key();
    let bob_pubkey_b64 = b64_pubkey(&bob_key);

    let (_alice_tmp, mut alice_ws) = make_workspace(&alice_key, "alice-id");
    let (_alice_cm_dir, mut alice_cm) = make_contact_manager([0x33u8; 32]);

    // Shared folder.
    let shared_dir = tempfile::tempdir().expect("shared_dir");
    let shared_path = shared_dir.path().to_str().expect("valid path").to_string();

    // Record snapshot watermark, then register Bob as a folder-channel peer.
    let snap_op = alice_ws
        .get_latest_operation_id()
        .expect("get_latest_operation_id")
        .unwrap_or_default();
    alice_ws
        .upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None)
        .expect("upsert_sync_peer");

    // Store Bob's folder channel_params on the peer record.
    let params_json = serde_json::json!({ "path": &shared_path }).to_string();
    alice_ws
        .update_peer_channel("dev-bob", "folder", &params_json)
        .expect("update_peer_channel");

    // Register Bob as a contact.
    alice_cm
        .find_or_create_by_public_key("Bob", &bob_pubkey_b64, TrustLevel::Tofu)
        .expect("register Bob as contact");

    // Build the engine with a FolderChannel.
    let mut engine = SyncEngine::new();
    let folder_ch = FolderChannel::new(
        "alice-identity-uuid".to_string(),
        "alice-device-uuid".to_string(),
    );
    engine.register_channel(Box::new(folder_ch));

    // Build sync context.
    let mut ctx = SyncContext {
        signing_key: &alice_key,
        contact_manager: &mut alice_cm,
        workspace_name: "TestEngineWorkspace",
        sender_display_name: "Alice",
    };

    // Run one poll cycle.
    let events = engine.poll(&mut alice_ws, &mut ctx).expect("poll");

    // Expect a DeltaSent event for Bob.
    let delta_sent_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, SyncEvent::DeltaSent { .. }))
        .collect();

    assert!(
        !delta_sent_events.is_empty(),
        "poll should emit at least one DeltaSent event; got events: {:?}",
        events
    );

    // Verify the file was written to the shared folder.
    let swarm_files: Vec<_> = std::fs::read_dir(shared_dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "swarm"))
        .collect();
    assert!(
        !swarm_files.is_empty(),
        "at least one .swarm file should have been written to the shared folder"
    );
}
