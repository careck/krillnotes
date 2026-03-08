// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Invite and Accept bundle generation and parsing.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use std::io::{Cursor, Read, Write};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::core::hlc::HlcTimestamp;
use crate::core::operation::Operation;
use crate::core::swarm::header::{SwarmHeader, SwarmMode};
use crate::core::swarm::signature::{sign_manifest, verify_manifest};
use crate::{KrillnotesError, Result};

// ---------------------------------------------------------------------------
// Invite
// ---------------------------------------------------------------------------

pub struct InviteParams<'a> {
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    pub source_display_name: String,
    pub offered_role: String,
    pub offered_scope: Option<String>,
    pub contact_public_key: Option<String>,
    pub inviter_key: &'a SigningKey,
}

pub struct ParsedInvite {
    pub workspace_id: String,
    pub workspace_name: String,
    pub offered_role: Option<String>,
    pub offered_scope: Option<String>,
    pub inviter_fingerprint: Option<String>,
    pub inviter_public_key: String,
    /// Base64-encoded 32-byte pairing token.
    pub pairing_token: String,
    pub set_permission_op: Operation,
}

/// Generate an invite.swarm bundle (signed, unencrypted).
pub fn create_invite_bundle(params: InviteParams<'_>) -> Result<Vec<u8>> {
    let vk = params.inviter_key.verifying_key();
    let pubkey_b64 = BASE64.encode(vk.as_bytes());
    let fingerprint = crate::core::contact::generate_fingerprint(&pubkey_b64)?;

    // Generate 32-byte pairing token.
    let mut token_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut token_bytes);
    let pairing_token = BASE64.encode(token_bytes);

    let header = SwarmHeader {
        format_version: 1,
        mode: SwarmMode::Invite,
        workspace_id: params.workspace_id.clone(),
        workspace_name: params.workspace_name,
        source_device_id: params.source_device_id.clone(),
        source_identity: pubkey_b64.clone(),
        source_display_name: params.source_display_name.clone(),
        created_at: Utc::now().to_rfc3339(),
        pairing_token: Some(pairing_token.clone()),
        offered_role: Some(params.offered_role.clone()),
        offered_scope: params.offered_scope.clone(),
        inviter_fingerprint: Some(fingerprint),
        accepted_identity: None,
        accepted_display_name: None,
        accepted_fingerprint: None,
        as_of_operation_id: None,
        since_operation_id: None,
        target_peer: params.contact_public_key.clone(),
        recipients: None,
        has_attachments: false,
    };
    header.validate()?;

    // SetPermission with placeholder target.
    let set_perm = Operation::SetPermission {
        operation_id: Uuid::new_v4().to_string(),
        timestamp: HlcTimestamp { wall_ms: Utc::now().timestamp_millis() as u64, counter: 0, node_id: 0 },
        device_id: params.source_device_id,
        note_id: params.offered_scope,
        user_id: "PLACEHOLDER".to_string(),
        role: params.offered_role,
        granted_by: pubkey_b64,
        signature: String::new(),
    };

    let header_bytes = serde_json::to_vec(&header)?;
    let payload_bytes = serde_json::to_vec(&set_perm)?;

    // Sign manifest.
    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.json", &payload_bytes),
    ];
    let sig = sign_manifest(&files, params.inviter_key);

    // Build zip.
    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);
        let opts = SimpleFileOptions::default();
        zip.start_file("header.json", opts)?;
        zip.write_all(&header_bytes)?;
        zip.start_file("payload.json", opts)?;
        zip.write_all(&payload_bytes)?;
        zip.start_file("signature.bin", opts)?;
        zip.write_all(&sig)?;
        zip.finish()?;
    }
    Ok(buf)
}

/// Parse and verify an invite.swarm bundle.
pub fn parse_invite_bundle(data: &[u8]) -> Result<ParsedInvite> {
    let cursor = Cursor::new(data);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| KrillnotesError::Swarm(format!("zip open: {e}")))?;

    let header_bytes = read_zip_file(&mut zip, "header.json")?;
    let payload_bytes = read_zip_file(&mut zip, "payload.json")?;
    let sig_bytes = read_zip_file(&mut zip, "signature.bin")?;

    let header: SwarmHeader = serde_json::from_slice(&header_bytes)?;
    header.validate()?;

    // Verify signature against sender's public key.
    let vk_bytes = BASE64.decode(&header.source_identity)
        .map_err(|e| KrillnotesError::Swarm(format!("bad source_identity: {e}")))?;
    let vk_arr: [u8; 32] = vk_bytes.try_into()
        .map_err(|_| KrillnotesError::Swarm("source_identity must be 32 bytes".to_string()))?;
    let vk = VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid public key: {e}")))?;

    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.json", &payload_bytes),
    ];
    verify_manifest(&files, &sig_bytes, &vk)?;

    let op: Operation = serde_json::from_slice(&payload_bytes)?;

    Ok(ParsedInvite {
        workspace_id: header.workspace_id,
        workspace_name: header.workspace_name,
        offered_role: header.offered_role,
        offered_scope: header.offered_scope,
        inviter_fingerprint: header.inviter_fingerprint,
        inviter_public_key: header.source_identity,
        pairing_token: header.pairing_token.unwrap_or_default(),
        set_permission_op: op,
    })
}

// ---------------------------------------------------------------------------
// Accept
// ---------------------------------------------------------------------------

pub struct AcceptParams<'a> {
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    pub declared_name: String,
    pub pairing_token: String,
    pub acceptor_key: &'a SigningKey,
}

pub struct ParsedAccept {
    pub workspace_id: String,
    pub declared_name: String,
    pub acceptor_public_key: String,
    pub pairing_token: String,
    pub join_op: Operation,
}

/// Generate an accept.swarm bundle (signed, unencrypted).
pub fn create_accept_bundle(params: AcceptParams<'_>) -> Result<Vec<u8>> {
    let vk = params.acceptor_key.verifying_key();
    let pubkey_b64 = BASE64.encode(vk.as_bytes());
    let fingerprint = crate::core::contact::generate_fingerprint(&pubkey_b64)?;

    let header = SwarmHeader {
        format_version: 1,
        mode: SwarmMode::Accept,
        workspace_id: params.workspace_id,
        workspace_name: params.workspace_name,
        source_device_id: params.source_device_id.clone(),
        source_identity: pubkey_b64.clone(),
        source_display_name: params.declared_name.clone(),
        created_at: Utc::now().to_rfc3339(),
        pairing_token: Some(params.pairing_token.clone()),
        offered_role: None,
        offered_scope: None,
        inviter_fingerprint: None,
        accepted_identity: Some(pubkey_b64.clone()),
        accepted_display_name: Some(params.declared_name.clone()),
        accepted_fingerprint: Some(fingerprint),
        as_of_operation_id: None,
        since_operation_id: None,
        target_peer: None,
        recipients: None,
        has_attachments: false,
    };
    header.validate()?;

    let join_op = Operation::JoinWorkspace {
        operation_id: Uuid::new_v4().to_string(),
        timestamp: HlcTimestamp { wall_ms: Utc::now().timestamp_millis() as u64, counter: 0, node_id: 0 },
        device_id: params.source_device_id,
        identity_public_key: pubkey_b64,
        declared_name: params.declared_name,
        pairing_token: params.pairing_token,
        signature: String::new(),
    };

    let header_bytes = serde_json::to_vec(&header)?;
    let payload_bytes = serde_json::to_vec(&join_op)?;
    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.json", &payload_bytes),
    ];
    let sig = sign_manifest(&files, params.acceptor_key);

    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);
        let opts = SimpleFileOptions::default();
        zip.start_file("header.json", opts)?;
        zip.write_all(&header_bytes)?;
        zip.start_file("payload.json", opts)?;
        zip.write_all(&payload_bytes)?;
        zip.start_file("signature.bin", opts)?;
        zip.write_all(&sig)?;
        zip.finish()?;
    }
    Ok(buf)
}

/// Parse and verify an accept.swarm bundle.
pub fn parse_accept_bundle(data: &[u8]) -> Result<ParsedAccept> {
    let cursor = Cursor::new(data);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| KrillnotesError::Swarm(format!("zip open: {e}")))?;

    let header_bytes = read_zip_file(&mut zip, "header.json")?;
    let payload_bytes = read_zip_file(&mut zip, "payload.json")?;
    let sig_bytes = read_zip_file(&mut zip, "signature.bin")?;

    let header: SwarmHeader = serde_json::from_slice(&header_bytes)?;
    header.validate()?;

    let vk_bytes = BASE64.decode(&header.source_identity)
        .map_err(|e| KrillnotesError::Swarm(format!("bad source_identity: {e}")))?;
    let vk_arr: [u8; 32] = vk_bytes.try_into()
        .map_err(|_| KrillnotesError::Swarm("source_identity must be 32 bytes".to_string()))?;
    let vk = VerifyingKey::from_bytes(&vk_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid sender key: {e}")))?;

    let files: Vec<(&str, &[u8])> = vec![
        ("header.json", &header_bytes),
        ("payload.json", &payload_bytes),
    ];
    verify_manifest(&files, &sig_bytes, &vk)?;

    let op: Operation = serde_json::from_slice(&payload_bytes)?;

    let declared_name = header.accepted_display_name.unwrap_or_default();
    Ok(ParsedAccept {
        workspace_id: header.workspace_id,
        declared_name,
        acceptor_public_key: header.source_identity,
        pairing_token: header.pairing_token.unwrap_or_default(),
        join_op: op,
    })
}

// ---------------------------------------------------------------------------
// Shared zip helper
// ---------------------------------------------------------------------------

pub(crate) fn read_zip_file<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut file = zip.by_name(name)
        .map_err(|_| KrillnotesError::Swarm(format!("bundle missing '{name}'")))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_key() -> SigningKey { SigningKey::generate(&mut OsRng) }

    #[test]
    fn test_invite_roundtrip() {
        let inviter_key = make_key();
        let bundle = create_invite_bundle(InviteParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "My WS".to_string(),
            source_device_id: "dev-1".to_string(),
            offered_role: "writer".to_string(),
            offered_scope: None,
            contact_public_key: None,
            inviter_key: &inviter_key,
        }).unwrap();

        let parsed = parse_invite_bundle(&bundle).unwrap();
        assert_eq!(parsed.workspace_id, "ws-1");
        assert_eq!(parsed.offered_role.as_deref(), Some("writer"));
        assert!(!parsed.pairing_token.is_empty());
    }

    #[test]
    fn test_accept_roundtrip() {
        let inviter_key = make_key();
        let invite = create_invite_bundle(InviteParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "My WS".to_string(),
            source_device_id: "dev-1".to_string(),
            offered_role: "writer".to_string(),
            offered_scope: None,
            contact_public_key: None,
            inviter_key: &inviter_key,
        }).unwrap();

        let parsed_invite = parse_invite_bundle(&invite).unwrap();
        let acceptor_key = make_key();
        let accept_bundle = create_accept_bundle(AcceptParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "My WS".to_string(),
            source_device_id: "dev-2".to_string(),
            declared_name: "Bob".to_string(),
            pairing_token: parsed_invite.pairing_token.clone(),
            acceptor_key: &acceptor_key,
        }).unwrap();

        let parsed_accept = parse_accept_bundle(&accept_bundle).unwrap();
        assert_eq!(parsed_accept.declared_name, "Bob");
        assert_eq!(parsed_accept.pairing_token, parsed_invite.pairing_token);
    }

    #[test]
    fn test_invite_signature_tamper_detected() {
        let inviter_key = make_key();
        let mut bundle = create_invite_bundle(InviteParams {
            workspace_id: "ws-1".to_string(),
            workspace_name: "My WS".to_string(),
            source_device_id: "dev-1".to_string(),
            offered_role: "writer".to_string(),
            offered_scope: None,
            contact_public_key: None,
            inviter_key: &inviter_key,
        }).unwrap();
        // Flip a byte in the middle of the bundle
        let len = bundle.len();
        bundle[len / 2] ^= 0xFF;
        assert!(parse_invite_bundle(&bundle).is_err());
    }
}
