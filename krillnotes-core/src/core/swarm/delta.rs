// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Delta bundle: signed, encrypted array of `Operation` values.
//!
//! WP-B RBAC stubs: ingest applies all operations unconditionally.
//! WP-C will replace the ingest loop with signature + RBAC + conflict resolution.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::core::operation::Operation;
use crate::core::swarm::crypto::{decrypt_payload, encrypt_for_recipients};
use crate::core::swarm::header::{SwarmHeader, SwarmMode};
use crate::core::swarm::invite::read_zip_file;
use crate::core::swarm::signature::{sign_manifest, verify_manifest};
use crate::{KrillnotesError, Result};

pub struct DeltaParams<'a> {
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    /// operation_id of the last operation the recipient has seen from us.
    pub since_operation_id: String,
    pub operations: Vec<Operation>,
    pub sender_key: &'a SigningKey,
    pub recipient_keys: Vec<&'a VerifyingKey>,
    pub recipient_peer_ids: Vec<String>,
}

pub struct ParsedDelta {
    pub workspace_id: String,
    pub since_operation_id: String,
    pub sender_public_key: String,
    pub operations: Vec<Operation>,
}

/// Generate a delta.swarm bundle.
pub fn create_delta_bundle(params: DeltaParams<'_>) -> Result<Vec<u8>> {
    let vk = params.sender_key.verifying_key();
    let pubkey_b64 = BASE64.encode(vk.as_bytes());

    let ops_json = serde_json::to_vec(&params.operations)?;
    let (ciphertext, mut entries) =
        encrypt_for_recipients(&ops_json, &params.recipient_keys)?;
    for (entry, peer_id) in entries.iter_mut().zip(params.recipient_peer_ids.iter()) {
        entry.peer_id = peer_id.clone();
    }

    let header = SwarmHeader {
        format_version: 1,
        mode: SwarmMode::Delta,
        workspace_id: params.workspace_id,
        workspace_name: params.workspace_name,
        source_device_id: params.source_device_id,
        source_identity: pubkey_b64,
        source_display_name: String::new(),
        created_at: Utc::now().to_rfc3339(),
        pairing_token: None,
        offered_role: None,
        offered_scope: None,
        inviter_fingerprint: None,
        accepted_identity: None,
        accepted_display_name: None,
        accepted_fingerprint: None,
        as_of_operation_id: None,
        since_operation_id: Some(params.since_operation_id),
        target_peer: None,
        recipients: Some(entries),
        has_attachments: false,
    };
    header.validate()?;

    let header_bytes = serde_json::to_vec(&header)?;
    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.enc", &ciphertext),
    ];
    let sig = sign_manifest(&files, params.sender_key);

    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);
        let opts = SimpleFileOptions::default();
        zip.start_file("header.json", opts)?;
        zip.write_all(&header_bytes)?;
        zip.start_file("payload.enc", opts)?;
        zip.write_all(&ciphertext)?;
        zip.start_file("signature.bin", opts)?;
        zip.write_all(&sig)?;
        zip.finish()?;
    }
    Ok(buf)
}

/// Parse and decrypt a delta.swarm bundle.
///
/// **STUB:** individual operation signatures are NOT verified yet — WP-C adds that.
/// **STUB:** RBAC is NOT enforced — WP-B adds that.
/// Operations are returned as-is for the caller to apply.
pub fn parse_delta_bundle(data: &[u8], recipient_key: &SigningKey) -> Result<ParsedDelta> {
    let cursor = Cursor::new(data);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| KrillnotesError::Swarm(format!("zip open: {e}")))?;

    let header_bytes = read_zip_file(&mut zip, "header.json")?;
    let ciphertext = read_zip_file(&mut zip, "payload.enc")?;
    let sig_bytes = read_zip_file(&mut zip, "signature.bin")?;

    let header: SwarmHeader = serde_json::from_slice(&header_bytes)?;
    header.validate()?;

    // Verify bundle signature.
    let vk_bytes = BASE64.decode(&header.source_identity)
        .map_err(|e| KrillnotesError::Swarm(format!("bad source_identity: {e}")))?;
    let vk_arr: [u8; 32] = vk_bytes.try_into()
        .map_err(|_| KrillnotesError::Swarm("source_identity wrong length".to_string()))?;
    let vk = VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid sender key: {e}")))?;
    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.enc", &ciphertext),
    ];
    verify_manifest(&files, &sig_bytes, &vk)?;

    // Decrypt.
    let recipients = header.recipients
        .ok_or_else(|| KrillnotesError::Swarm("no recipients in delta".to_string()))?;
    let mut plaintext = None;
    for entry in &recipients {
        if let Ok(pt) = decrypt_payload(&ciphertext, entry, recipient_key) {
            plaintext = Some(pt);
            break;
        }
    }
    let ops_json = plaintext
        .ok_or_else(|| KrillnotesError::Swarm("no recipient entry matched our key".to_string()))?;

    let operations: Vec<Operation> = serde_json::from_slice(&ops_json)?;

    Ok(ParsedDelta {
        workspace_id: header.workspace_id,
        since_operation_id: header.since_operation_id.unwrap_or_default(),
        sender_public_key: header.source_identity,
        operations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use crate::core::hlc::HlcTimestamp;
    use crate::core::operation::Operation;

    fn make_key() -> SigningKey { SigningKey::generate(&mut OsRng) }

    fn dummy_op(id: &str) -> Operation {
        Operation::UpdateNote {
            operation_id: id.to_string(),
            timestamp: HlcTimestamp { wall_ms: 1, counter: 0, node_id: 0 },
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            title: "Updated".to_string(),
            modified_by: "pk".to_string(),
            signature: "sig".to_string(),
        }
    }

    #[test]
    fn test_delta_roundtrip() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let ops = vec![dummy_op("op-1"), dummy_op("op-2")];

        let bundle = create_delta_bundle(DeltaParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            since_operation_id: "op-0".to_string(),
            operations: ops.clone(),
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["dev-2".to_string()],
        }).unwrap();

        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.operations.len(), 2);
        assert_eq!(parsed.operations[0].operation_id(), "op-1");
        assert_eq!(parsed.since_operation_id, "op-0");
    }

    #[test]
    fn test_empty_delta_allowed() {
        let sender_key = make_key();
        let recipient_key = make_key();

        let bundle = create_delta_bundle(DeltaParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            since_operation_id: "op-0".to_string(),
            operations: vec![],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["dev-2".to_string()],
        }).unwrap();

        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.operations.len(), 0);
    }
}
