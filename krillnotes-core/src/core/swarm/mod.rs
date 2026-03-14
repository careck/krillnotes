// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! `.swarm` bundle format — codec, crypto, and state machine.

pub mod crypto;
pub mod delta;
pub mod header;
pub mod invite;
pub mod signature;
pub mod snapshot;
pub mod sync;

#[cfg(test)]
mod integration_tests {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    use crate::core::swarm::delta::{create_delta_bundle, parse_delta_bundle, DeltaParams};
    use crate::core::swarm::invite::*;
    use crate::core::swarm::snapshot::{create_snapshot_bundle, parse_snapshot_bundle, SnapshotParams};
    use crate::core::operation::Operation;
    use crate::core::hlc::HlcTimestamp;

    fn make_key() -> SigningKey { SigningKey::generate(&mut OsRng) }

    fn dummy_op(id: &str, note_id: &str) -> Operation {
        Operation::UpdateNote {
            operation_id: id.to_string(),
            timestamp: HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 0 },
            device_id: "dev-alice".to_string(),
            note_id: note_id.to_string(),
            title: "Alice's edit".to_string(),
            modified_by: "pk_alice".to_string(),
            signature: "sig".to_string(),
        }
    }

    /// Full T2 handshake: unknown peer invitation → snapshot → delta exchange.
    #[test]
    fn test_full_unknown_peer_handshake() {
        let alice_key = make_key();
        let bob_key = make_key();

        // === Step 1: Alice generates invite.swarm ===
        let invite_bundle = create_invite_bundle(InviteParams {
            workspace_id: "ws-alpha".to_string(),
            workspace_name: "Project Alpha".to_string(),
            source_device_id: "dev-alice".to_string(),
            source_display_name: String::new(),
            offered_role: "writer".to_string(),
            offered_scope: None,
            contact_public_key: None,
            inviter_key: &alice_key,
            owner_pubkey: "owner-pk-alice".to_string(),
            reply_channels: vec![],
        }).unwrap();

        // === Step 2: Bob reads invite ===
        let parsed_invite = parse_invite_bundle(&invite_bundle).unwrap();
        assert_eq!(parsed_invite.workspace_id, "ws-alpha");
        assert_eq!(parsed_invite.offered_role.as_deref(), Some("writer"));
        let pairing_token = parsed_invite.pairing_token.clone();

        // === Step 3: Bob generates accept.swarm ===
        let accept_bundle = create_accept_bundle(AcceptParams {
            workspace_id: "ws-alpha".to_string(),
            workspace_name: "Project Alpha".to_string(),
            source_device_id: "dev-bob".to_string(),
            declared_name: "Bob".to_string(),
            pairing_token: pairing_token.clone(),
            acceptor_key: &bob_key,
            owner_pubkey: parsed_invite.owner_pubkey.clone(),
            channel_preference: ChannelPreference::default(),
        }).unwrap();

        // === Step 4: Alice processes accept ===
        let parsed_accept = parse_accept_bundle(&accept_bundle).unwrap();
        assert_eq!(parsed_accept.declared_name, "Bob");
        assert_eq!(parsed_accept.pairing_token, pairing_token);

        // === Step 5: Alice sends snapshot.swarm to Bob ===
        let workspace_state = serde_json::to_vec(&serde_json::json!({
            "notes": [],
            "scripts": []
        })).unwrap();

        let snapshot_bundle = create_snapshot_bundle(SnapshotParams {
            workspace_id: "ws-alpha".to_string(),
            workspace_name: "Project Alpha".to_string(),
            source_device_id: "dev-alice".to_string(),
            source_display_name: "Alice".to_string(),
            as_of_operation_id: "op-baseline".to_string(),
            workspace_json: workspace_state.clone(),
            sender_key: &alice_key,
            recipient_keys: vec![&bob_key.verifying_key()],
            recipient_peer_ids: vec!["dev-bob".to_string()],
            attachment_blobs: vec![],
            owner_pubkey: "owner-pk-alice".to_string(),
        }).unwrap();

        // === Step 6: Bob imports snapshot ===
        let parsed_snapshot = parse_snapshot_bundle(&snapshot_bundle, &bob_key).unwrap();
        assert_eq!(parsed_snapshot.workspace_id, "ws-alpha");
        assert_eq!(parsed_snapshot.as_of_operation_id, "op-baseline");
        assert_eq!(parsed_snapshot.workspace_json, workspace_state);

        // === Step 7: Alice sends delta to Bob ===
        let alice_ops = vec![dummy_op("op-1", "note-abc"), dummy_op("op-2", "note-abc")];
        let delta_bundle = create_delta_bundle(DeltaParams {
            workspace_id: "ws-alpha".to_string(),
            workspace_name: "Project Alpha".to_string(),
            source_device_id: "dev-alice".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: "op-baseline".to_string(),
            operations: alice_ops.clone(),
            sender_key: &alice_key,
            recipient_keys: vec![&bob_key.verifying_key()],
            recipient_peer_ids: vec!["dev-bob".to_string()],
            recipient_identity_id: "pk-bob".to_string(),
            owner_pubkey: "owner-pk-alice".to_string(),
        }).unwrap();

        // === Step 8: Bob applies delta ===
        let parsed_delta = parse_delta_bundle(&delta_bundle, &bob_key).unwrap();
        assert_eq!(parsed_delta.operations.len(), 2);
        assert_eq!(parsed_delta.operations[0].operation_id(), "op-1");
        assert_eq!(parsed_delta.since_operation_id, "op-baseline");
    }
}
