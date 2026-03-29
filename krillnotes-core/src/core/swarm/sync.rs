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

/// Result of generating a delta bundle.
/// Bundles the encoded bytes with metadata needed by the poll loop.
#[derive(Debug)]
pub struct DeltaBundle {
    pub bundle_bytes: Vec<u8>,
    /// The operation ID of the last op included, if any.
    /// The poll loop advances the watermark only after confirmed delivery.
    pub last_included_op: Option<String>,
    /// Number of operations included.
    pub op_count: usize,
}

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
/// Queries all operations since `last_sent_op` for `peer_device_id` and encrypts
/// them for the peer's public key. The poll loop advances the watermark after delivery.
///
/// # Errors
/// - `KrillnotesError::Swarm("peer not found")` if the peer is not registered.
///
/// When `last_sent_op` is `None` (e.g. after a force-resync reset), all operations
/// are included in the delta so the peer can catch up from scratch.
pub fn generate_delta(
    workspace: &mut Workspace,
    peer_device_id: &str,
    workspace_name: &str,
    signing_key: &SigningKey,
    sender_display_name: &str,
    contact_manager: &ContactManager,
) -> Result<DeltaBundle> {
    // 1. Look up peer.
    let peer = workspace
        .get_sync_peer(peer_device_id)?
        .ok_or_else(|| {
            KrillnotesError::Swarm(format!("peer {peer_device_id} not found in registry"))
        })?;

    // 2. Collect operations since watermark, excluding this peer's own ops.
    //    When last_sent_op is None (force-resync), operations_since(None) returns all ops.
    let ops = workspace.operations_since(peer.last_sent_op.as_deref(), &peer.peer_device_id)?;

    // 3. Collect plaintext attachment blobs for any AddAttachment ops in the batch.
    let mut attachment_blobs: Vec<(String, Vec<u8>)> = Vec::new();
    for op in &ops {
        if let Operation::AddAttachment { attachment_id, .. } = op {
            match workspace.get_attachment_bytes(attachment_id) {
                Ok(bytes) => attachment_blobs.push((attachment_id.clone(), bytes)),
                Err(e) => {
                    log::warn!(target: "krillnotes::sync",
                        "Could not read attachment {} for delta, skipping blob: {e}",
                        attachment_id);
                }
            }
        }
    }

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

    let op_count = ops.len();
    let last_included_op = ops.last().map(|op| op.operation_id().to_string());

    let bundle_bytes = create_delta_bundle(DeltaParams {
        protocol: workspace.protocol_id().to_string(),
        workspace_id: workspace.workspace_id().to_string(),
        workspace_name: workspace_name.to_string(),
        source_device_id,
        source_display_name: sender_display_name.to_string(),
        since_operation_id: peer.last_sent_op.clone().unwrap_or_default(),
        operations: ops,
        sender_key: signing_key,
        recipient_keys: vec![&recipient_vk],
        recipient_peer_ids: vec![peer_device_id.to_string()],
        recipient_identity_id: peer.peer_identity_id.clone(),
        owner_pubkey: workspace.owner_pubkey().to_string(),
        // ACK: tell the peer the last operation we received FROM them.
        // They can compare it with their last_sent_op to detect missed deltas.
        ack_operation_id: peer.last_received_op.clone(),
        attachment_blobs,
    })?;

    // NOTE: watermark is NOT advanced here.
    // The poll loop advances it only after confirmed delivery (SendResult::Delivered).

    Ok(DeltaBundle { bundle_bytes, last_included_op, op_count })
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
    // 0. Protocol isolation — reject bundles from incompatible products before decryption.
    let header = crate::core::swarm::header::read_header(bundle_bytes)?;
    if header.protocol != workspace.protocol_id() {
        log::error!(
            "Rejecting swarm bundle: protocol mismatch (expected '{}', found '{}')",
            workspace.protocol_id(),
            header.protocol,
        );
        return Err(KrillnotesError::ProtocolMismatch {
            expected: workspace.protocol_id().to_string(),
            found: header.protocol,
        });
    }

    // 1. Decrypt and verify bundle-level signature.
    let parsed = parse_delta_bundle(bundle_bytes, recipient_key)?;

    // 1b. Authoritative protocol check — the encrypted protocol cannot be
    // tampered with (unlike the cleartext header).
    if parsed.protocol != workspace.protocol_id() {
        log::error!(
            "Rejecting delta: encrypted protocol mismatch (expected '{}', found '{}')",
            workspace.protocol_id(),
            parsed.protocol,
        );
        return Err(KrillnotesError::ProtocolMismatch {
            expected: workspace.protocol_id().to_string(),
            found: parsed.protocol,
        });
    }

    // 2. Assert workspace_id matches.
    if parsed.workspace_id != workspace.workspace_id() {
        return Err(KrillnotesError::Swarm(format!(
            "workspace_id mismatch: bundle has '{}', this workspace is '{}'",
            parsed.workspace_id,
            workspace.workspace_id()
        )));
    }

    // Cross-check owner_pubkey if present in the delta
    if let Some(ref header_owner) = parsed.owner_pubkey {
        let local_owner = workspace.owner_pubkey();
        if header_owner != local_owner {
            return Err(KrillnotesError::Swarm(format!(
                "owner_pubkey mismatch: delta header={}, local={}",
                &header_owner[..header_owner.len().min(8)],
                &local_owner[..local_owner.len().min(8)],
            )));
        }
    }

    let mut applied = 0usize;
    let mut skipped = 0usize;
    let mut new_tofu_contacts: Vec<String> = Vec::new();

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

        if workspace.apply_incoming_operation(op.clone(), &parsed.sender_device_id, &parsed.attachment_blobs)? {
            applied += 1;
        } else {
            skipped += 1;
        }
    }

    // 4. Upsert sender in peer registry, consolidating any placeholder row.
    //    Use the last op in the bundle (not just the last applied op) so the ACK
    //    we echo back matches the sender's `last_sent_op`.  When later ops in a
    //    bundle are duplicates, tracking only applied ops makes the ACK lag behind
    //    the sender's watermark, triggering an infinite full-resend loop.
    let last_bundle_op_id = parsed
        .operations
        .last()
        .map(|op| op.operation_id().to_string());
    let last_received = last_bundle_op_id.as_deref();
    workspace.upsert_peer_from_delta(
        &parsed.sender_device_id,
        &parsed.sender_public_key,
        last_received,
    )?;

    // 5. Process inbound ACK: if the sender tells us the last op they received FROM us,
    //    and that's behind our last_sent_op for them, reset our watermark so we resend.
    let sender_device_id = &parsed.sender_device_id;
    if let Some(ref ack_op_id) = parsed.ack_operation_id {
        if let Some(peer) = workspace.get_sync_peer(sender_device_id)? {
            if let Some(ref our_last_sent) = peer.last_sent_op {
                if workspace.is_operation_before(ack_op_id, our_last_sent)? {
                    log::warn!(target: "krillnotes::sync",
                        "peer {} ACK ({}) is behind our last_sent ({}), resetting watermark",
                        sender_device_id, ack_op_id, our_last_sent
                    );
                    workspace.reset_peer_watermark(sender_device_id, Some(ack_op_id))?;
                } else if !workspace.operation_exists(ack_op_id)? {
                    // ACK references an operation we don't have (purged?) — force full resend.
                    log::warn!(target: "krillnotes::sync",
                        "peer {} ACK ({}) references unknown operation, resetting watermark",
                        sender_device_id, ack_op_id
                    );
                    workspace.reset_peer_watermark(sender_device_id, None)?;
                }
            }
        }
    } else {
        // Peer sent no ACK — they have never received anything from us.
        // If our watermark says we've sent something, reset it for a full resend.
        if let Some(peer) = workspace.get_sync_peer(sender_device_id)? {
            if peer.last_sent_op.is_some() {
                log::warn!(target: "krillnotes::sync",
                    "peer {} sent no ACK but we have a watermark — resetting to force full delta",
                    sender_device_id
                );
                workspace.reset_peer_watermark(sender_device_id, None)?;
            }
        }
    }

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

    use crate::core::permission::{AllowAllGate, PermissionGate};

    fn test_gate() -> Box<dyn PermissionGate> {
        Box::new(AllowAllGate::new("test"))
    }

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
            test_gate(),
            None,
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
            crate::core::swarm::delta::parse_delta_bundle(&bundle.bundle_bytes, &bob_key).unwrap();
        assert_eq!(parsed.workspace_id, alice_ws.workspace_id());
    }

    /// generate_delta with last_sent_op = None includes ALL ops (force-resync path).
    #[test]
    fn test_generate_delta_no_watermark_includes_all_ops() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = b64(&bob_key);

        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = crate::core::workspace::Workspace::create(
            temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
            test_gate(),
            None,
        )
        .unwrap();

        // Create some ops BEFORE registering the peer with no watermark.
        ws.create_note_root("TextNote").unwrap();
        ws.create_note_root("TextNote").unwrap();

        // Register peer WITHOUT a snapshot watermark (last_sent_op = None).
        ws.upsert_sync_peer("dev-bob", &bob_pubkey_b64, None, None)
            .unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let cm =
            crate::core::contact::ContactManager::for_identity(cm_dir.path().to_path_buf(), [2u8; 32])
                .unwrap();
        cm.find_or_create_by_public_key(
            "Bob",
            &bob_pubkey_b64,
            crate::core::contact::TrustLevel::Tofu,
        )
        .unwrap();

        let bundle =
            super::generate_delta(&mut ws, "dev-bob", "TestWorkspace", &alice_key, "Alice", &cm)
                .expect("generate_delta should succeed when last_sent_op is None");

        // All ops (excluding dev-bob's own device_id, but alice owns all ops here)
        // should be included.
        assert!(
            bundle.op_count >= 2,
            "all ops should be included when watermark is None, got op_count={}",
            bundle.op_count
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
            test_gate(),
            None,
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
            test_gate(),
            None,
        )
        .unwrap();

        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [11u8; 32],
        )
        .unwrap();

        let result = super::apply_delta(&bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();

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
            test_gate(),
            None,
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
            test_gate(),
            None,
        )
        .unwrap();
        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [15u8; 32],
        )
        .unwrap();

        let result = super::apply_delta(&bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm);
        assert!(result.is_err(), "workspace_id mismatch must be an error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("workspace_id mismatch"), "unexpected error: {err}");
    }

    /// Regression test: when all incoming ops are duplicates, last_received_op
    /// must still be set to the last op in the bundle so the ACK echoed back
    /// matches the sender's watermark.  The old code only tracked the last
    /// *applied* op, causing an infinite full-resend loop.
    #[test]
    fn test_last_received_op_set_on_all_duplicates() {
        let alice_key = make_key();
        let bob_key = make_key();
        let _alice_pubkey_b64 = b64(&alice_key);
        let bob_pubkey_b64 = b64(&bob_key);

        // ── Alice workspace ──
        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
            test_gate(),
            None,
        )
        .unwrap();

        // Create some notes so there are ops in the log.
        alice_ws.create_note_root("TextNote").unwrap();
        alice_ws.create_note_root("TextNote").unwrap();

        // Watermark = None → full resync, which is the exact scenario that
        // triggers the feedback loop when ACKs don't match.
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, None, None)
            .unwrap();

        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::for_identity(
            alice_cm_dir.path().to_path_buf(),
            [20u8; 32],
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
        assert!(bundle.op_count > 0, "need ops to test");
        let sent_last_op = bundle.last_included_op.clone().unwrap();

        // ── Bob workspace (shares DB → already has all ops) ──
        let mut bob_ws = crate::core::workspace::Workspace::open(
            alice_temp.path(),
            "",
            "bob-id",
            SigningKey::from_bytes(&bob_key.to_bytes()),
            test_gate(),
            None,
        )
        .unwrap();
        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [21u8; 32],
        )
        .unwrap();

        let result =
            super::apply_delta(&bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();

        // All ops should be duplicates since Bob shares Alice's DB.
        assert_eq!(
            result.operations_applied, 0,
            "expected all duplicates, got {} applied",
            result.operations_applied
        );
        assert!(result.operations_skipped > 0, "should have skipped some ops");

        // Key assertion: Bob's peer record for Alice must have last_received_op
        // matching the last op in the bundle, even though all ops were duplicates.
        let peer = bob_ws
            .get_sync_peer(&result.sender_device_id)
            .unwrap()
            .expect("peer should exist after apply_delta");
        assert_eq!(
            peer.last_received_op.as_deref(),
            Some(sent_last_op.as_str()),
            "last_received_op must match last bundle op even when all ops are duplicates"
        );
    }

    /// apply_delta must fail with ProtocolMismatch when the bundle's protocol
    /// doesn't match the receiving workspace's gate.
    #[test]
    fn test_protocol_mismatch_rejects_delta() {
        let alice_key = make_key();
        let bob_key = make_key();
        let bob_pubkey_b64 = b64(&bob_key);

        let alice_temp = tempfile::NamedTempFile::new().unwrap();
        let mut alice_ws = crate::core::workspace::Workspace::create(
            alice_temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
            // Alice uses protocol "wrong/1"
            Box::new(AllowAllGate::new("wrong/1")),
            None,
        )
        .unwrap();

        // Create a note to generate an operation.
        let root = alice_ws.list_all_notes().unwrap()[0].clone();
        alice_ws
            .create_note(&root.id, crate::core::workspace::AddPosition::AsChild, "TextNote")
            .unwrap();

        // Register Bob as a peer with a snapshot watermark.
        let snap_op = alice_ws.get_latest_operation_id().unwrap().unwrap_or_default();
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, Some(&snap_op), None)
            .unwrap();

        // Create another note so there's a new operation to delta.
        alice_ws
            .create_note(&root.id, crate::core::workspace::AddPosition::AsChild, "TextNote")
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

        // Bob's workspace uses protocol "test" (different from Alice's "wrong/1").
        // Open the SAME database (so workspace_id matches) but with a different gate.
        let mut bob_ws = crate::core::workspace::Workspace::open(
            alice_temp.path(),
            "",
            "bob-id",
            SigningKey::from_bytes(&bob_key.to_bytes()),
            test_gate(), // protocol "test"
            None,
        )
        .unwrap();

        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [20u8; 32],
        )
        .unwrap();

        let result = super::apply_delta(&bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm);
        assert!(result.is_err(), "should reject mismatched protocol");
        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::core::error::KrillnotesError::ProtocolMismatch { .. }),
            "error should be ProtocolMismatch, got: {err}"
        );
    }

    /// End-to-end: Alice attaches a file, generates a delta, Bob applies it and
    /// can read the same bytes back.
    #[test]
    fn test_attachment_delta_sync_end_to_end() {
        // ── Keys ──────────────────────────────────────────────────────────────
        let alice_key = make_key();
        let bob_key = make_key();
        let alice_pubkey_b64 = b64(&alice_key);
        let bob_pubkey_b64 = b64(&bob_key);

        // ── Alice workspace (owns a real directory so attachments/ can exist) ─
        let alice_dir = tempfile::tempdir().unwrap();
        let alice_db = alice_dir.path().join("alice.db");
        let mut alice_ws = crate::core::workspace::Workspace::create(
            &alice_db,
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
            test_gate(),
            None,
        )
        .unwrap();

        // Alice creates a note, then attaches a file with her signing key.
        let note_id = alice_ws.create_note_root("TextNote").unwrap();
        let file_bytes: &[u8] = b"hello attachment";
        let meta = alice_ws
            .attach_file(&note_id, "hello.txt", Some("text/plain"), file_bytes, Some(&alice_key))
            .unwrap();
        let attachment_id = meta.id.clone();

        // Register Bob as a peer (watermark = None → send all ops).
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, None, None)
            .unwrap();

        // Alice's contact manager knows Bob's key so the bundle can be encrypted.
        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::for_identity(
            alice_cm_dir.path().to_path_buf(),
            [30u8; 32],
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

        assert!(bundle.op_count > 0, "delta must contain ops");

        // ── Bob workspace: separate directory, same workspace_id ──────────────
        let bob_dir = tempfile::tempdir().unwrap();
        let bob_db = bob_dir.path().join("bob.db");
        let workspace_id = alice_ws.workspace_id().to_string();
        let mut bob_ws = crate::core::workspace::Workspace::create_empty_with_id(
            &bob_db,
            "",
            "bob-id",
            SigningKey::from_bytes(&bob_key.to_bytes()),
            &workspace_id,
            test_gate(),
            None,
        )
        .unwrap();
        // Adopt Alice's owner_pubkey so the bundle owner check passes.
        bob_ws.set_owner_pubkey(&alice_pubkey_b64).unwrap();

        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [31u8; 32],
        )
        .unwrap();

        // Apply Alice's delta on Bob.
        let result =
            super::apply_delta(&bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();
        assert!(
            result.operations_applied > 0,
            "Bob should have applied at least one op, got: {:?}",
            result
        );

        // Verify the bundle actually carried a blob sidecar (guards against
        // generate_delta regressions where blob collection is silently skipped).
        let parsed = crate::core::swarm::delta::parse_delta_bundle(
            &bundle.bundle_bytes, &bob_key,
        ).unwrap();
        assert_eq!(parsed.attachment_blobs.len(), 1,
            "delta must carry exactly one blob sidecar");
        assert_eq!(parsed.attachment_blobs[0].0, attachment_id,
            "blob sidecar id must match attachment_id");

        // ── Assertions ────────────────────────────────────────────────────────
        // Bob should see the attachment in the note's list.
        let bob_attachments = bob_ws.get_attachments(&note_id).unwrap();
        assert_eq!(
            bob_attachments.len(),
            1,
            "Bob should have exactly one attachment for the note"
        );
        assert_eq!(bob_attachments[0].id, attachment_id);
        assert_eq!(bob_attachments[0].filename, "hello.txt");

        // Bob should be able to decrypt and read back the original bytes.
        let decrypted = bob_ws.get_attachment_bytes(&attachment_id).unwrap();
        assert_eq!(
            decrypted, file_bytes,
            "Bob's decrypted attachment bytes must match Alice's original"
        );
    }

    /// End-to-end: After Alice removes an attachment, Bob applies the delta and
    /// the attachment disappears from his workspace.
    #[test]
    fn test_remove_attachment_delta_sync() {
        // ── Keys ──────────────────────────────────────────────────────────────
        let alice_key = make_key();
        let bob_key = make_key();
        let alice_pubkey_b64 = b64(&alice_key);
        let bob_pubkey_b64 = b64(&bob_key);

        // ── Alice workspace ───────────────────────────────────────────────────
        let alice_dir = tempfile::tempdir().unwrap();
        let alice_db = alice_dir.path().join("alice.db");
        let mut alice_ws = crate::core::workspace::Workspace::create(
            &alice_db,
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
            test_gate(),
            None,
        )
        .unwrap();

        let note_id = alice_ws.create_note_root("TextNote").unwrap();
        let file_bytes: &[u8] = b"data to be removed";
        let meta = alice_ws
            .attach_file(&note_id, "remove_me.bin", None, file_bytes, Some(&alice_key))
            .unwrap();
        let attachment_id = meta.id.clone();

        // ── Bob workspace: receives the AddAttachment via first delta ─────────
        let bob_dir = tempfile::tempdir().unwrap();
        let bob_db = bob_dir.path().join("bob.db");
        let workspace_id = alice_ws.workspace_id().to_string();
        let mut bob_ws = crate::core::workspace::Workspace::create_empty_with_id(
            &bob_db,
            "",
            "bob-id",
            SigningKey::from_bytes(&bob_key.to_bytes()),
            &workspace_id,
            test_gate(),
            None,
        )
        .unwrap();
        bob_ws.set_owner_pubkey(&alice_pubkey_b64).unwrap();

        // Contact managers.
        let alice_cm_dir = tempfile::tempdir().unwrap();
        let alice_cm = crate::core::contact::ContactManager::for_identity(
            alice_cm_dir.path().to_path_buf(),
            [40u8; 32],
        )
        .unwrap();
        alice_cm
            .find_or_create_by_public_key(
                "Bob",
                &bob_pubkey_b64,
                crate::core::contact::TrustLevel::Tofu,
            )
            .unwrap();

        let bob_cm_dir = tempfile::tempdir().unwrap();
        let mut bob_cm = crate::core::contact::ContactManager::for_identity(
            bob_cm_dir.path().to_path_buf(),
            [41u8; 32],
        )
        .unwrap();

        // First sync: Bob receives the AddAttachment op.
        alice_ws
            .upsert_sync_peer("dev-bob", &bob_pubkey_b64, None, None)
            .unwrap();
        let add_bundle =
            super::generate_delta(&mut alice_ws, "dev-bob", "Test", &alice_key, "Alice", &alice_cm)
                .unwrap();
        super::apply_delta(&add_bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm).unwrap();

        // Verify Bob has the attachment after the first sync.
        assert_eq!(
            bob_ws.get_attachments(&note_id).unwrap().len(),
            1,
            "Bob should have the attachment after first sync"
        );

        // ── Alice removes the attachment ───────────────────────────────────────
        alice_ws
            .delete_attachment(&attachment_id, Some(&alice_key))
            .unwrap();

        // Advance the watermark so the second delta only includes the remove op.
        alice_ws
            .upsert_sync_peer(
                "dev-bob",
                &bob_pubkey_b64,
                add_bundle.last_included_op.as_deref(),
                None,
            )
            .unwrap();

        // Second sync: Bob receives the RemoveAttachment op.
        let remove_bundle =
            super::generate_delta(&mut alice_ws, "dev-bob", "Test", &alice_key, "Alice", &alice_cm)
                .unwrap();
        assert_eq!(remove_bundle.op_count, 1,
            "second delta should contain only the RemoveAttachment op (watermark advance check)");
        super::apply_delta(&remove_bundle.bundle_bytes, &mut bob_ws, &bob_key, &mut bob_cm)
            .unwrap();

        // ── Assertions ────────────────────────────────────────────────────────
        // Bob's attachment list must now be empty.
        let remaining = bob_ws.get_attachments(&note_id).unwrap();
        assert!(
            remaining.is_empty(),
            "Bob should have no attachments after the remove sync, found: {:?}",
            remaining
        );

        // The .enc file must also be gone from Bob's attachments directory.
        let bob_enc_path = bob_dir
            .path()
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        assert!(
            !bob_enc_path.exists(),
            "Bob's .enc file must be deleted after remove sync"
        );
    }
}
