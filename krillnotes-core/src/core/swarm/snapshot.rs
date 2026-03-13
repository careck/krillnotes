// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Snapshot bundle: full resolved workspace state, encrypted.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::core::swarm::crypto::{
    decrypt_blob, decrypt_payload_with_key, encrypt_blob, encrypt_for_recipients_with_key,
};
use crate::core::swarm::header::{SwarmHeader, SwarmMode};
use crate::core::swarm::invite::read_zip_file;
use crate::core::swarm::signature::{sign_manifest, verify_manifest};
use crate::{KrillnotesError, Result};

pub struct SnapshotParams<'a> {
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    pub as_of_operation_id: String,
    /// Pre-serialised workspace.json bytes.
    pub workspace_json: Vec<u8>,
    pub sender_key: &'a SigningKey,
    pub recipient_keys: Vec<&'a VerifyingKey>,
    pub recipient_peer_ids: Vec<String>,
    /// (attachment_id, plaintext_bytes). Encrypted into the bundle with the same key as the payload.
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
}

pub struct ParsedSnapshot {
    pub workspace_id: String,
    pub workspace_name: String,
    pub as_of_operation_id: String,
    pub sender_public_key: String,
    /// Decrypted workspace.json bytes — caller parses with serde.
    pub workspace_json: Vec<u8>,
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
}

/// Generate a snapshot.swarm bundle.
pub fn create_snapshot_bundle(params: SnapshotParams<'_>) -> Result<Vec<u8>> {
    let vk = params.sender_key.verifying_key();
    let pubkey_b64 = BASE64.encode(vk.as_bytes());

    let (ciphertext, sym_key, mut entries) =
        encrypt_for_recipients_with_key(&params.workspace_json, &params.recipient_keys)?;

    // Replace placeholder peer_ids with real ones.
    for (entry, peer_id) in entries.iter_mut().zip(params.recipient_peer_ids.iter()) {
        entry.peer_id = peer_id.clone();
    }

    // Validate attachment IDs are safe ZIP path segments (UUID-shaped).
    for (att_id, _) in &params.attachment_blobs {
        debug_assert!(
            att_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "attachment ID must be alphanumeric+hyphens only (UUID-shaped), got: {att_id}"
        );
    }

    // Encrypt each attachment blob with the same symmetric key.
    let mut att_entries: Vec<(String, Vec<u8>)> = Vec::new();
    for (att_id, plaintext) in &params.attachment_blobs {
        let ct = encrypt_blob(&sym_key, plaintext)?;
        att_entries.push((att_id.clone(), ct));
    }

    let has_attachments = !att_entries.is_empty();

    let header = SwarmHeader {
        format_version: 1,
        mode: SwarmMode::Snapshot,
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
        as_of_operation_id: Some(params.as_of_operation_id),
        since_operation_id: None,
        target_peer: None,
        recipients: Some(entries),
        has_attachments,
    };
    header.validate()?;

    let header_bytes = serde_json::to_vec(&header)?;
    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.enc", &ciphertext),
    ];
    // Attachment blobs are authenticated by their individual AES-GCM tags (using the
    // same symmetric key as the payload), so they do not need to be included in the
    // Ed25519 manifest signature.
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
        for (att_id, att_ct) in &att_entries {
            zip.start_file(format!("attachments/{att_id}.enc"), opts)?;
            zip.write_all(att_ct)?;
        }
        zip.start_file("signature.bin", opts)?;
        zip.write_all(&sig)?;
        zip.finish()?;
    }
    Ok(buf)
}

/// Parse and decrypt a snapshot.swarm bundle.
pub fn parse_snapshot_bundle(data: &[u8], recipient_key: &SigningKey) -> Result<ParsedSnapshot> {
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
        .map_err(|_| KrillnotesError::Swarm("source_identity key wrong length".to_string()))?;
    let vk = VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid sender key: {e}")))?;
    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.enc", &ciphertext),
    ];
    verify_manifest(&files, &sig_bytes, &vk)?;

    // Find our entry in recipients.
    let recipients = header.recipients
        .ok_or_else(|| KrillnotesError::Swarm("no recipients in snapshot".to_string()))?;

    // Try each entry (we don't know our peer_id from the outside).
    let mut plaintext = None;
    let mut sym_key_found = None;
    for entry in &recipients {
        if let Ok((pt, key)) = decrypt_payload_with_key(&ciphertext, entry, recipient_key) {
            plaintext = Some(pt);
            sym_key_found = Some(key);
            break;
        }
    }
    let workspace_json = plaintext
        .ok_or_else(|| KrillnotesError::Swarm("no recipient entry matched our key".to_string()))?;
    let sym_key = sym_key_found.expect("sym_key is set iff plaintext decryption succeeded");

    // Decrypt attachment blobs — entries named "attachments/<id>.enc"
    let mut attachment_blobs = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)
            .map_err(|e| KrillnotesError::Swarm(format!("zip index {i}: {e}")))?;
        let name = file.name().to_string();
        if let Some(att_id) = name
            .strip_prefix("attachments/")
            .and_then(|n| n.strip_suffix(".enc"))
        {
            let mut ct = Vec::new();
            file.read_to_end(&mut ct)
                .map_err(|e| KrillnotesError::Swarm(format!("read att {att_id}: {e}")))?;
            let pt = decrypt_blob(&sym_key, &ct)?;
            attachment_blobs.push((att_id.to_string(), pt));
        }
    }

    Ok(ParsedSnapshot {
        workspace_id: header.workspace_id,
        workspace_name: header.workspace_name,
        as_of_operation_id: header.as_of_operation_id.unwrap_or_default(),
        sender_public_key: header.source_identity,
        workspace_json,
        attachment_blobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_key() -> SigningKey { SigningKey::generate(&mut OsRng) }

    #[test]
    fn test_snapshot_encrypt_decrypt_roundtrip() {
        let sender_key = make_key();
        let recipient_key = make_key();

        let payload = b"workspace json here";
        let workspace_id = "ws-1".to_string();
        let bundle = create_snapshot_bundle(SnapshotParams {
            workspace_id: workspace_id.clone(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            as_of_operation_id: "op-uuid-1".to_string(),
            workspace_json: payload.to_vec(),
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["dev-2".to_string()],
            attachment_blobs: vec![],
        }).unwrap();

        let result = parse_snapshot_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(result.workspace_json, payload);
        assert_eq!(result.workspace_id, workspace_id);
        assert_eq!(result.as_of_operation_id, "op-uuid-1");
        assert_eq!(result.workspace_name, "Test");
    }

    #[test]
    fn test_snapshot_wrong_key_fails() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let wrong_key = make_key();

        let bundle = create_snapshot_bundle(SnapshotParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            as_of_operation_id: "op-uuid-1".to_string(),
            workspace_json: b"payload".to_vec(),
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["dev-2".to_string()],
            attachment_blobs: vec![],
        }).unwrap();

        assert!(parse_snapshot_bundle(&bundle, &wrong_key).is_err());
    }

    #[test]
    fn test_snapshot_with_attachments_roundtrip() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let payload = b"{}";
        let att_id = "att-uuid-abc";
        let att_blob = b"raw attachment bytes here";

        let bundle = create_snapshot_bundle(SnapshotParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test WS".to_string(),
            source_device_id: "dev-1".to_string(),
            as_of_operation_id: "op-1".to_string(),
            workspace_json: payload.to_vec(),
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["peer-pub-key".to_string()],
            attachment_blobs: vec![(att_id.to_string(), att_blob.to_vec())],
        }).unwrap();

        let parsed = parse_snapshot_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.workspace_json, payload.to_vec());
        assert_eq!(parsed.workspace_name, "Test WS");
        assert_eq!(parsed.attachment_blobs.len(), 1);
        assert_eq!(parsed.attachment_blobs[0].0, att_id);
        assert_eq!(parsed.attachment_blobs[0].1, att_blob.to_vec());
    }

    #[test]
    fn test_snapshot_empty_attachments_roundtrip() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let bundle = create_snapshot_bundle(SnapshotParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            as_of_operation_id: "op-1".to_string(),
            workspace_json: b"payload".to_vec(),
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["p1".to_string()],
            attachment_blobs: vec![],
        }).unwrap();
        let parsed = parse_snapshot_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.attachment_blobs.len(), 0);
        assert_eq!(parsed.workspace_name, "Test");
    }

    #[test]
    fn test_snapshot_multi_attachment_roundtrip() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let blobs = vec![
            ("att-1".to_string(), b"blob one".to_vec()),
            ("att-2".to_string(), b"blob two".to_vec()),
            ("att-3".to_string(), b"blob three".to_vec()),
        ];
        let bundle = create_snapshot_bundle(SnapshotParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "Multi".to_string(),
            source_device_id: "dev-1".to_string(),
            as_of_operation_id: "op-1".to_string(),
            workspace_json: b"{}".to_vec(),
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["p1".to_string()],
            attachment_blobs: blobs.clone(),
        }).unwrap();
        let parsed = parse_snapshot_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.attachment_blobs.len(), 3);
        // All IDs and blobs must be present (order may differ)
        let mut result = parsed.attachment_blobs.clone();
        result.sort_by_key(|(id, _)| id.clone());
        assert_eq!(result[0], ("att-1".to_string(), b"blob one".to_vec()));
        assert_eq!(result[1], ("att-2".to_string(), b"blob two".to_vec()));
        assert_eq!(result[2], ("att-3".to_string(), b"blob three".to_vec()));
    }
}
