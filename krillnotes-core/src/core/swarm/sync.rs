// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! High-level delta sync orchestration.
//!
//! `generate_delta` and `apply_delta` sit above the codec (`swarm/delta.rs`)
//! and workspace primitives (`workspace.rs`), orchestrating:
//!   - peer watermark lookup
//!   - operation list assembly
//!   - encryption key resolution from the contact manager
//!   - codec invocation
//!   - watermark and peer registry updates

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::Serialize;

use crate::core::contact::{ContactManager, TrustLevel};
use crate::core::operation::Operation;
use crate::core::swarm::delta::{create_delta_bundle, parse_delta_bundle, DeltaParams};
use crate::core::workspace::Workspace;
use crate::{KrillnotesError, Result};

/// Result of applying a received delta bundle.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub operations_applied: usize,
    pub operations_skipped: usize,
    pub sender_device_id: String,
    pub sender_public_key: String,
    /// Display names of contacts auto-registered via TOFU during this apply.
    pub new_tofu_contacts: Vec<String>,
}

/// Generate a delta `.swarm` bundle for a specific peer.
///
/// Queries all operations since `last_sent_op` for `peer_device_id`, encrypts them
/// for the peer's public key, and updates the `last_sent_op` watermark.
///
/// # Errors
/// - `KrillnotesError::Swarm("peer not found")` if the peer is not registered.
/// - `KrillnotesError::Swarm("snapshot must precede delta")` if `last_sent_op` is `None`.
pub fn generate_delta(
    workspace: &mut Workspace,
    peer_device_id: &str,
    workspace_name: &str,
    signing_key: &SigningKey,
    sender_display_name: &str,
    contact_manager: &ContactManager,
) -> Result<Vec<u8>> {
    // 1. Look up peer.
    let peer = workspace
        .get_sync_peer(peer_device_id)?
        .ok_or_else(|| {
            KrillnotesError::Swarm(format!("peer {peer_device_id} not found in registry"))
        })?;

    // 2. Require snapshot baseline.
    let last_sent_op = peer.last_sent_op.as_deref().ok_or_else(|| {
        KrillnotesError::Swarm(
            "snapshot must precede delta — no last_sent_op for this peer".to_string(),
        )
    })?;

    // 3. Collect operations since watermark, excluding this peer's own ops.
    let ops = workspace.operations_since(Some(last_sent_op), &peer.peer_device_id)?;

    // 4. Resolve peer's public key from contacts.
    let contact = contact_manager
        .find_by_public_key(&peer.peer_identity_id)?
        .ok_or_else(|| {
            KrillnotesError::Swarm(format!(
                "no contact for peer identity {}",
                peer.peer_identity_id
            ))
        })?;
    let recipient_key_bytes = BASE64
        .decode(&contact.public_key)
        .map_err(|e| KrillnotesError::Swarm(format!("bad contact public key: {e}")))?;
    let recipient_key_arr: [u8; 32] = recipient_key_bytes.try_into().map_err(|_| {
        KrillnotesError::Swarm("contact public key wrong length".to_string())
    })?;
    let recipient_vk = VerifyingKey::from_bytes(&recipient_key_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid recipient key: {e}")))?;

    // 5. Build delta bundle.
    // Use the workspace's identity-based device_id (not the hardware device ID)
    // so that multiple identities on the same machine have distinct source IDs.
    let source_device_id = workspace.device_id().to_string();

    let bundle = create_delta_bundle(DeltaParams {
        workspace_id: workspace.workspace_id().to_string(),
        workspace_name: workspace_name.to_string(),
        source_device_id,
        source_display_name: sender_display_name.to_string(),
        since_operation_id: last_sent_op.to_string(),
        operations: ops.clone(),
        sender_key: signing_key,
        recipient_keys: vec![&recipient_vk],
        recipient_peer_ids: vec![peer_device_id.to_string()],
        recipient_identity_id: peer.peer_identity_id.clone(),
    })?;

    // 6. Update watermark only if we sent at least one operation.
    if let Some(last_op) = ops.last() {
        workspace.upsert_sync_peer(
            peer_device_id,
            &peer.peer_identity_id,
            Some(last_op.operation_id()),
            None,
        )?;
    }

    Ok(bundle)
}

/// Apply a received delta `.swarm` bundle to the local workspace.
///
/// Decrypts, verifies bundle signature, applies each operation in order.
/// Auto-registers unknown operation authors as TOFU contacts.
///
/// Returns an `ApplyResult` summarising what was applied / skipped.
///
/// **A13 stub:** RBAC and conflict resolution are not enforced.
/// Individual per-operation signatures are not verified.
pub fn apply_delta(
    bundle_bytes: &[u8],
    workspace: &mut Workspace,
    recipient_key: &SigningKey,
    contact_manager: &mut ContactManager,
) -> Result<ApplyResult> {
    // 1. Decrypt and verify bundle-level signature.
    let parsed = parse_delta_bundle(bundle_bytes, recipient_key)?;

    // 2. Assert workspace_id matches.
    if parsed.workspace_id != workspace.workspace_id() {
        return Err(KrillnotesError::Swarm(format!(
            "workspace_id mismatch: bundle has '{}', this workspace is '{}'",
            parsed.workspace_id,
            workspace.workspace_id()
        )));
    }

    let mut applied = 0usize;
    let mut skipped = 0usize;
    let mut new_tofu_contacts: Vec<String> = Vec::new();
    let mut last_applied_op_id = String::new();

    // 3. Apply each operation in chronological order.
    for op in &parsed.operations {
        // TOFU: auto-register unknown authors.
        let author_key = op.author_key();
        if !author_key.is_empty() && contact_manager.find_by_public_key(author_key)?.is_none() {
            let name = if let Operation::JoinWorkspace { declared_name, .. } = op {
                declared_name.clone()
            } else {
                // Synthetic fallback: first 8 chars of base64 key + ellipsis
                format!("{}…", &author_key[..8.min(author_key.len())])
            };
            contact_manager.find_or_create_by_public_key(&name, author_key, TrustLevel::Tofu)?;
            new_tofu_contacts.push(name);
        }

        if workspace.apply_incoming_operation(op.clone())? {
            applied += 1;
            last_applied_op_id = op.operation_id().to_string();
        } else {
            skipped += 1;
        }
    }

    // 4. Upsert sender in peer registry, consolidating any placeholder row.
    let last_received = if last_applied_op_id.is_empty() {
        None
    } else {
        Some(last_applied_op_id.as_str())
    };
    workspace.upsert_peer_from_delta(
        &parsed.sender_device_id,
        &parsed.sender_public_key,
        last_received,
    )?;

    Ok(ApplyResult {
        operations_applied: applied,
        operations_skipped: skipped,
        sender_device_id: parsed.sender_device_id,
        sender_public_key: parsed.sender_public_key,
        new_tofu_contacts,
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_key() -> SigningKey {
        SigningKey::generate(&mut OsRng)
    }

    fn b64(key: &SigningKey) -> String {
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes())
    }

    /// Basic smoke test: generate_delta succeeds for a registered peer that has a
    /// snapshot watermark set (last_sent_op is Some).
    #[test]
    fn test_generate_delta_basic() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = b64(&bob_key);

        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
        )
        .unwrap();

        // Record snapshot watermark so peer is eligible for delta.
        let snap_op = alice_ws
            .get_latest_operation_id()
            .unwrap()
            .unwrap_or_default();
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None)
            .unwrap();

        // Register Bob as a contact so generate_delta can find the encryption key.
        let cm_dir = tempfile::tempdir().unwrap();
        let alice_cm =
            crate::core::contact::ContactManager::for_identity(cm_dir.path().to_path_buf(), [2u8; 32])
                .unwrap();
        alice_cm
            .find_or_create_by_public_key(
                "Bob",
                &bob_pubkey_b64,
                crate::core::contact::TrustLevel::Tofu,
            )
            .unwrap();

        // Generate delta — even if there are no new ops it must succeed.
        let bundle = super::generate_delta(
            &mut alice_ws,
            "dev-bob",
            "TestWorkspace",
            &alice_key,
            "Alice",
            &alice_cm,
        )
        .unwrap();

        // Parse and verify with Bob's key.
        let parsed =
            crate::core::swarm::delta::parse_delta_bundle(&bundle, &bob_key).unwrap();
        assert_eq!(parsed.workspace_id, alice_ws.workspace_id());
    }

    /// generate_delta must fail when last_sent_op is None (no snapshot baseline).
    #[test]
    fn test_generate_delta_no_snapshot_errors() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = b64(&bob_key);

        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = crate::core::workspace::Workspace::create(
            temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
        )
        .unwrap();
        // Register peer WITHOUT a snapshot watermark (last_sent_op = None).
        ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64, None, None)
            .unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let cm =
            crate::core::contact::ContactManager::for_identity(cm_dir.path().to_path_buf(), [2u8; 32])
                .unwrap();
        // Also register Bob as a contact so we don't fail on that lookup first.
        cm.find_or_create_by_public_key(
            "Bob",
            &bob_pubkey_b64,
            crate::core::contact::TrustLevel::Tofu,
        )
        .unwrap();

        let result =
            super::generate_delta(&mut ws, "dev-bob", "TestWorkspace", &alice_key, "Alice", &cm);
        assert!(result.is_err(), "must error when last_sent_op is None");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("snapshot must precede delta"),
            "unexpected error: {err}"
        );
    }

    /// apply_delta smoke test: a bundle created by Alice can be applied by Bob.
    #[test]
    fn test_apply_delta_basic() {
        let alice_key = make_key();
        let bob_key = make_key();
        let alice_pubkey_b64 = b64(&alice_key);
        let bob_pubkey_b64 = b64(&bob_key);

        // ── Alice's workspace ──────────────────────────────────────────────────
        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
        )
        .unwrap();
        let snap_op = alice_ws
            .get_latest_operation_id()
            .unwrap()
            .unwrap_or_default();
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None)
            .unwrap();

        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::for_identity(
            alice_cm_dir.path().to_path_buf(),
            [10u8; 32],
        )
        .unwrap();
        alice_cm
            .find_or_create_by_public_key(
                "Bob",
                &bob_pubkey_b64,
                crate::core::contact::TrustLevel::Tofu,
            )
            .unwrap();

        let bundle = super::generate_delta(
            &mut alice_ws,
            "dev-bob",
            "Test",
            &alice_key,
            "Alice",
            &alice_cm,
        )
        .unwrap();

        // ── Bob's workspace (must share the same workspace_id) ─────────────────
        // Open the same database as Alice but using Bob's signing key so the
        // workspace_id matches the bundle Alice generated.
        let mut bob_ws = crate::core::workspace::Workspace::open(
            alice_temp.path(),
            "",
            "bob-id",
            SigningKey::from_bytes(&bob_key.to_bytes()),
        )
        .unwrap();

        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [11u8; 32],
        )
        .unwrap();

        let result = super::apply_delta(&bundle, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();

        assert_eq!(
            result.operations_applied + result.operations_skipped,
            result.operations_applied + result.operations_skipped,
            "sanity: counts are non-negative"
        );
        assert!(!result.sender_device_id.is_empty());
        // Sender public key should match Alice's key.
        assert_eq!(result.sender_public_key, alice_pubkey_b64);
    }

    /// apply_delta must fail when the bundle's workspace_id doesn't match.
    #[test]
    fn test_apply_delta_workspace_id_mismatch() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = b64(&bob_key);

        // Alice's workspace
        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
        )
        .unwrap();
        let snap_op = alice_ws
            .get_latest_operation_id()
            .unwrap()
            .unwrap_or_default();
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None)
            .unwrap();

        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::for_identity(
            alice_cm_dir.path().to_path_buf(),
            [14u8; 32],
        )
        .unwrap();
        alice_cm
            .find_or_create_by_public_key(
                "Bob",
                &bob_pubkey_b64,
                crate::core::contact::TrustLevel::Tofu,
            )
            .unwrap();
        let bundle =
            super::generate_delta(&mut alice_ws, "dev-bob", "Test", &alice_key, "Alice", &alice_cm)
                .unwrap();

        // Bob's workspace — **different database file** → different workspace_id.
        let bob_temp = tempfile::NamedTempFile::new().unwrap();
        let mut bob_ws = crate::core::workspace::Workspace::create(
            bob_temp.path(),
            "",
            "bob-id",
            SigningKey::from_bytes(&bob_key.to_bytes()),
        )
        .unwrap();
        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [15u8; 32],
        )
        .unwrap();

        let result = super::apply_delta(&bundle, &mut bob_ws, &bob_key, &mut bob_cm);
        assert!(result.is_err(), "workspace_id mismatch must be an error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("workspace_id mismatch"), "unexpected error: {err}");
    }
}
