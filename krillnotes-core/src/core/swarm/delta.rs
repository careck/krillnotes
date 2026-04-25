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
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::core::operation::Operation;
use crate::core::swarm::crypto::{
    decrypt_blob, decrypt_payload_with_key, encrypt_blob, encrypt_for_recipients_with_key,
};
use crate::core::swarm::header::{SwarmHeader, SwarmMode};
use crate::core::swarm::invite::read_zip_file;
use crate::core::swarm::signature::{sign_manifest, verify_manifest};
use crate::{KrillnotesError, Result};

/// Wraps an [`Operation`] with optional per-operation verification metadata.
///
/// When `verified_by` is `Some`, it contains the base64-encoded public key of
/// the peer who vouched for this operation (e.g. the workspace owner countersigning
/// a contributor's op). When `None`, the field is omitted from serialized JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeltaOperation {
    pub op: Operation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
}

pub struct DeltaParams<'a> {
    pub protocol: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    pub source_display_name: String,
    /// operation_id of the last operation the recipient has seen from us.
    pub since_operation_id: String,
    pub delta_operations: Vec<DeltaOperation>,
    pub sender_key: &'a SigningKey,
    pub recipient_keys: Vec<&'a VerifyingKey>,
    pub recipient_peer_ids: Vec<String>,
    /// Base64 public key of the intended recipient (for target_peer in header).
    pub recipient_identity_id: String,
    /// Base64 public key of the workspace owner.
    pub owner_pubkey: String,
    /// ACK: the last operation we received FROM the recipient.
    /// Lets the recipient self-correct its watermark if they're ahead of us.
    pub ack_operation_id: Option<String>,
    /// Plaintext attachment bytes keyed by attachment_id.
    /// Each blob corresponds to an AddAttachment operation in the batch.
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
}

pub struct ParsedDelta {
    /// Protocol extracted from the encrypted payload (tamper-proof).
    pub protocol: String,
    pub workspace_id: String,
    pub since_operation_id: String,
    pub sender_public_key: String,
    pub sender_device_id: String,
    pub delta_operations: Vec<DeltaOperation>,
    pub owner_pubkey: Option<String>,
    /// ACK from the sender: the last operation they received FROM us.
    pub ack_operation_id: Option<String>,
    /// Decrypted attachment blobs from the delta bundle sidecar files.
    pub attachment_blobs: Vec<(String, Vec<u8>)>,
}

/// Generate a delta.swarm bundle.
pub fn create_delta_bundle(params: DeltaParams<'_>) -> Result<Vec<u8>> {
    let vk = params.sender_key.verifying_key();
    let pubkey_b64 = BASE64.encode(vk.as_bytes());

    let ops_json = serde_json::to_vec(&params.delta_operations)?;
    let prefixed = super::header::prefix_protocol(&params.protocol, &ops_json);
    let (ciphertext, sym_key, mut entries) =
        encrypt_for_recipients_with_key(&prefixed, &params.recipient_keys)?;
    for (entry, peer_id) in entries.iter_mut().zip(params.recipient_peer_ids.iter()) {
        entry.peer_id = peer_id.clone();
    }

    let header = SwarmHeader {
        protocol: params.protocol,
        format_version: 1,
        mode: SwarmMode::Delta,
        workspace_id: params.workspace_id,
        workspace_name: params.workspace_name,
        source_device_id: params.source_device_id,
        source_identity: pubkey_b64,
        source_display_name: params.source_display_name,
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
        target_peer: Some(params.recipient_identity_id),
        ack_operation_id: params.ack_operation_id.clone(),
        recipients: Some(entries),
        has_attachments: !params.attachment_blobs.is_empty(),
        owner_pubkey: Some(params.owner_pubkey.clone()),
    };
    header.validate()?;

    let header_bytes = serde_json::to_vec(&header)?;

    // Encrypt attachment sidecars before signing so their ciphertext is in the manifest.
    let encrypted_sidecars: Vec<(String, Vec<u8>)> = params
        .attachment_blobs
        .iter()
        .map(|(att_id, plaintext)| {
            let ct = encrypt_blob(&sym_key, plaintext)?;
            Ok((att_id.clone(), ct))
        })
        .collect::<Result<Vec<_>>>()?;

    // Build manifest over header + payload + all sidecar ciphertexts.
    let mut files: Vec<(&str, &[u8])> =
        vec![("header.json", &header_bytes), ("payload.enc", &ciphertext)];
    let sidecar_names: Vec<String> = encrypted_sidecars
        .iter()
        .map(|(id, _)| format!("attachments/{id}.enc"))
        .collect();
    for (i, (_, ct)) in encrypted_sidecars.iter().enumerate() {
        files.push((&sidecar_names[i], ct));
    }
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
        for (att_id, ct) in &encrypted_sidecars {
            zip.start_file(format!("attachments/{att_id}.enc"), opts)?;
            zip.write_all(ct)?;
        }
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
    let mut zip =
        ZipArchive::new(cursor).map_err(|e| KrillnotesError::Swarm(format!("zip open: {e}")))?;

    let header_bytes = read_zip_file(&mut zip, "header.json")?;
    let ciphertext = read_zip_file(&mut zip, "payload.enc")?;
    let sig_bytes = read_zip_file(&mut zip, "signature.bin")?;

    // Read sidecar ciphertext BEFORE verification so we can include it in the manifest.
    let mut sidecar_entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| KrillnotesError::Swarm(format!("zip index {i}: {e}")))?;
        let name = file.name().to_string();
        if let Some(att_id) = name
            .strip_prefix("attachments/")
            .and_then(|n| n.strip_suffix(".enc"))
        {
            let mut ct = Vec::new();
            file.read_to_end(&mut ct)
                .map_err(|e| KrillnotesError::Swarm(format!("read att {att_id}: {e}")))?;
            sidecar_entries.push((att_id.to_string(), ct));
        }
    }

    let header: SwarmHeader = serde_json::from_slice(&header_bytes)?;
    header.validate()?;

    // Verify bundle signature (includes sidecar ciphertext in manifest).
    let vk_bytes = BASE64
        .decode(&header.source_identity)
        .map_err(|e| KrillnotesError::Swarm(format!("bad source_identity: {e}")))?;
    let vk_arr: [u8; 32] = vk_bytes
        .try_into()
        .map_err(|_| KrillnotesError::Swarm("source_identity wrong length".to_string()))?;
    let vk = VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid sender key: {e}")))?;
    let mut files: Vec<(&str, &[u8])> =
        vec![("header.json", &header_bytes), ("payload.enc", &ciphertext)];
    let sidecar_names: Vec<String> = sidecar_entries
        .iter()
        .map(|(id, _)| format!("attachments/{id}.enc"))
        .collect();
    for (i, (_, ct)) in sidecar_entries.iter().enumerate() {
        files.push((&sidecar_names[i], ct));
    }
    verify_manifest(&files, &sig_bytes, &vk)?;

    // Decrypt.
    let recipients = header
        .recipients
        .ok_or_else(|| KrillnotesError::Swarm("no recipients in delta".to_string()))?;
    let mut plaintext = None;
    let mut sym_key_found: Option<[u8; 32]> = None;
    for entry in &recipients {
        if let Ok((pt, key)) = decrypt_payload_with_key(&ciphertext, entry, recipient_key) {
            plaintext = Some(pt);
            sym_key_found = Some(key);
            break;
        }
    }
    let decrypted = plaintext
        .ok_or_else(|| KrillnotesError::Swarm("no recipient entry matched our key".to_string()))?;
    let sym_key = sym_key_found.expect("sym_key set iff decryption succeeded");

    // Strip the protocol tag embedded before encryption.
    let (protocol, ops_json) = super::header::strip_protocol(&decrypted)?;

    let delta_operations: Vec<DeltaOperation> = serde_json::from_slice(&ops_json)?;

    // Decrypt sidecar blobs from the already-read ciphertext.
    let mut attachment_blobs = Vec::new();
    for (att_id, ct) in &sidecar_entries {
        let pt = decrypt_blob(&sym_key, ct)?;
        attachment_blobs.push((att_id.clone(), pt));
    }

    Ok(ParsedDelta {
        protocol,
        workspace_id: header.workspace_id,
        since_operation_id: header.since_operation_id.unwrap_or_default(),
        sender_public_key: header.source_identity,
        sender_device_id: header.source_device_id,
        delta_operations,
        owner_pubkey: header.owner_pubkey,
        ack_operation_id: header.ack_operation_id,
        attachment_blobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hlc::HlcTimestamp;
    use crate::core::operation::Operation;
    use ed25519_dalek::SigningKey;

    fn make_key() -> SigningKey {
        SigningKey::generate(&mut rand_core::OsRng)
    }

    fn dummy_op(id: &str) -> Operation {
        Operation::UpdateNote {
            operation_id: id.to_string(),
            timestamp: HlcTimestamp {
                wall_ms: 1,
                counter: 0,
                node_id: 0,
            },
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

        let bundle = create_delta_bundle(DeltaParams {
            protocol: "test".to_string(),
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: "op-0".to_string(),
            delta_operations: vec![
                DeltaOperation { op: dummy_op("op-1"), verified_by: None },
                DeltaOperation { op: dummy_op("op-2"), verified_by: Some("voucher-pk".to_string()) },
            ],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["dev-2".to_string()],
            recipient_identity_id: "pk-dev-2".to_string(),
            owner_pubkey: "owner-pk".to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![],
        })
        .unwrap();

        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.sender_device_id, "dev-1");
        assert_eq!(parsed.delta_operations.len(), 2);
        assert_eq!(parsed.delta_operations[0].op.operation_id(), "op-1");
        assert_eq!(parsed.delta_operations[0].verified_by, None);
        assert_eq!(parsed.delta_operations[1].op.operation_id(), "op-2");
        assert_eq!(parsed.delta_operations[1].verified_by, Some("voucher-pk".to_string()));
        assert_eq!(parsed.since_operation_id, "op-0");
    }

    #[test]
    fn test_empty_delta_allowed() {
        let sender_key = make_key();
        let recipient_key = make_key();

        let bundle = create_delta_bundle(DeltaParams {
            protocol: "test".to_string(),
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: "op-0".to_string(),
            delta_operations: vec![],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_key.verifying_key()],
            recipient_peer_ids: vec!["dev-2".to_string()],
            recipient_identity_id: "pk-dev-2".to_string(),
            owner_pubkey: "owner-pk".to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![],
        })
        .unwrap();

        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();
        assert_eq!(parsed.delta_operations.len(), 0);
    }

    #[test]
    fn test_delta_with_attachments_roundtrip() {
        use ed25519_dalek::SigningKey;

        let sender_key = SigningKey::generate(&mut rand_core::OsRng);
        let recipient_key = SigningKey::generate(&mut rand_core::OsRng);
        let recipient_vk = recipient_key.verifying_key();

        let mut op = Operation::AddAttachment {
            operation_id: "op-att-delta-1".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp {
                wall_ms: 1000,
                counter: 0,
                node_id: 1,
            },
            device_id: "dev-1".to_string(),
            attachment_id: "att-uuid-1".to_string(),
            note_id: "note-1".to_string(),
            filename: "test.png".to_string(),
            mime_type: Some("image/png".to_string()),
            size_bytes: 4,
            hash_sha256: "fakehash".to_string(),
            added_by: String::new(),
            signature: String::new(),
        };
        op.sign(&sender_key);

        let blob_data = b"BLOB".to_vec();
        let params = DeltaParams {
            protocol: "krillnotes/1".to_string(),
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: String::new(),
            delta_operations: vec![DeltaOperation { op, verified_by: None }],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_vk],
            recipient_peer_ids: vec!["peer-1".to_string()],
            recipient_identity_id: "recip-id".to_string(),
            owner_pubkey: "owner-key".to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![("att-uuid-1".to_string(), blob_data.clone())],
        };

        let bundle = create_delta_bundle(params).unwrap();
        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();

        assert_eq!(parsed.delta_operations.len(), 1);
        assert_eq!(parsed.attachment_blobs.len(), 1);
        assert_eq!(parsed.attachment_blobs[0].0, "att-uuid-1");
        assert_eq!(parsed.attachment_blobs[0].1, blob_data);
    }

    /// Rebuild a ZIP without any attachments/*.enc entries.
    fn strip_sidecar_from_bundle(bundle: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(bundle);
        let mut zip_in = ZipArchive::new(cursor).unwrap();
        let mut buf = Vec::new();
        {
            let cursor_out = Cursor::new(&mut buf);
            let mut zip_out = ZipWriter::new(cursor_out);
            let opts = SimpleFileOptions::default();
            for i in 0..zip_in.len() {
                let mut file = zip_in.by_index(i).unwrap();
                let name = file.name().to_string();
                if name.starts_with("attachments/") {
                    continue; // strip sidecar
                }
                let mut data = Vec::new();
                file.read_to_end(&mut data).unwrap();
                zip_out.start_file(&name, opts).unwrap();
                zip_out.write_all(&data).unwrap();
            }
            zip_out.finish().unwrap();
        }
        buf
    }

    #[test]
    fn test_stripped_sidecar_fails_verification() {
        let sender_key = make_key();
        let recipient_key = make_key();
        let recipient_vk = recipient_key.verifying_key();

        let mut op = Operation::AddAttachment {
            operation_id: "op-att-strip-1".to_string(),
            timestamp: HlcTimestamp {
                wall_ms: 1000,
                counter: 0,
                node_id: 1,
            },
            device_id: "dev-1".to_string(),
            attachment_id: "att-strip-1".to_string(),
            note_id: "note-1".to_string(),
            filename: "photo.jpg".to_string(),
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: 5,
            hash_sha256: "fakehash".to_string(),
            added_by: String::new(),
            signature: String::new(),
        };
        op.sign(&sender_key);

        let blob_data = b"HELLO".to_vec();
        let bundle = create_delta_bundle(DeltaParams {
            protocol: "krillnotes/1".to_string(),
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: String::new(),
            delta_operations: vec![DeltaOperation { op, verified_by: None }],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_vk],
            recipient_peer_ids: vec!["peer-1".to_string()],
            recipient_identity_id: "recip-id".to_string(),
            owner_pubkey: "owner-key".to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![("att-strip-1".to_string(), blob_data)],
        })
        .unwrap();

        // Tamper: strip the sidecar from the bundle
        let tampered = strip_sidecar_from_bundle(&bundle);

        // Parsing should fail because the manifest hash no longer matches
        let result = parse_delta_bundle(&tampered, &recipient_key);
        match result {
            Ok(_) => panic!("expected error when sidecar stripped, but parse succeeded"),
            Err(e) => {
                let err_msg = e.to_string();
                assert!(
                    err_msg.contains("signature verification failed"),
                    "expected 'signature verification failed', got: {err_msg}"
                );
            }
        }
    }

    #[test]
    fn test_delta_without_attachments_roundtrip() {
        use ed25519_dalek::SigningKey;

        let sender_key = SigningKey::generate(&mut rand_core::OsRng);
        let recipient_key = SigningKey::generate(&mut rand_core::OsRng);
        let recipient_vk = recipient_key.verifying_key();

        let mut op = Operation::UpdateNote {
            operation_id: "op-un-1".to_string(),
            timestamp: crate::core::hlc::HlcTimestamp {
                wall_ms: 1000,
                counter: 0,
                node_id: 1,
            },
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            title: "Updated".to_string(),
            modified_by: String::new(),
            signature: String::new(),
        };
        op.sign(&sender_key);

        let params = DeltaParams {
            protocol: "krillnotes/1".to_string(),
            workspace_id: "ws-1".to_string(),
            workspace_name: "Test".to_string(),
            source_device_id: "dev-1".to_string(),
            source_display_name: "Alice".to_string(),
            since_operation_id: String::new(),
            delta_operations: vec![DeltaOperation { op, verified_by: None }],
            sender_key: &sender_key,
            recipient_keys: vec![&recipient_vk],
            recipient_peer_ids: vec!["peer-1".to_string()],
            recipient_identity_id: "recip-id".to_string(),
            owner_pubkey: "owner-key".to_string(),
            ack_operation_id: None,
            attachment_blobs: vec![],
        };

        let bundle = create_delta_bundle(params).unwrap();
        let parsed = parse_delta_bundle(&bundle, &recipient_key).unwrap();

        assert_eq!(parsed.delta_operations.len(), 1);
        assert!(parsed.attachment_blobs.is_empty());
    }

    #[test]
    fn test_delta_operation_serde_roundtrip() {
        // With verified_by = Some → field present in JSON
        let with_voucher = DeltaOperation {
            op: dummy_op("op-v1"),
            verified_by: Some("voucher-pk-base64".to_string()),
        };
        let json_with = serde_json::to_string(&with_voucher).unwrap();
        assert!(
            json_with.contains("\"verified_by\""),
            "verified_by should be present when Some: {json_with}"
        );
        let deser_with: DeltaOperation = serde_json::from_str(&json_with).unwrap();
        assert_eq!(deser_with.op.operation_id(), "op-v1");
        assert_eq!(
            deser_with.verified_by,
            Some("voucher-pk-base64".to_string())
        );

        // With verified_by = None → field omitted from JSON
        let without_voucher = DeltaOperation {
            op: dummy_op("op-v2"),
            verified_by: None,
        };
        let json_without = serde_json::to_string(&without_voucher).unwrap();
        assert!(
            !json_without.contains("verified_by"),
            "verified_by should be omitted when None: {json_without}"
        );
        let deser_without: DeltaOperation = serde_json::from_str(&json_without).unwrap();
        assert_eq!(deser_without.op.operation_id(), "op-v2");
        assert_eq!(deser_without.verified_by, None);

        // Deserialization from JSON without verified_by field → defaults to None.
        // Build the JSON dynamically from a real Operation to match the actual serde shape.
        let op_json = serde_json::to_string(&dummy_op("op-v3")).unwrap();
        let bare_json = format!(r#"{{"op":{op_json}}}"#);
        let deser_bare: DeltaOperation = serde_json::from_str(&bare_json).unwrap();
        assert_eq!(deser_bare.op.operation_id(), "op-v3");
        assert_eq!(deser_bare.verified_by, None);
    }
}
