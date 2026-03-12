// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Invite record and `.swarm` file structures for the peer invite flow.

use std::path::{Path, PathBuf};
use std::collections::BTreeMap;
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{SigningKey, Signer, VerifyingKey, Verifier, Signature};
use crate::core::error::KrillnotesError;

type Result<T> = std::result::Result<T, KrillnotesError>;

// ── On-disk invite record (plaintext, managed by inviter) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteRecord {
    pub invite_id: Uuid,
    pub workspace_id: String,
    pub workspace_name: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
    pub use_count: u32,
}

// ── .swarm file formats ───────────────────────────────────────────────────────

/// The invite `.swarm` file sent to invitees. All workspace_* fields are optional.
/// NOTE: No `rename_all` — field names already match the spec's snake_case wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_author_org: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_homepage_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_language: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_tags: Vec<String>,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub signature: String,
}

/// The response `.swarm` file sent back by the invitee.
/// NOTE: No `rename_all` — field names match the spec's snake_case wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteResponseFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub invite_id: String,
    pub invitee_public_key: String,
    pub invitee_declared_name: String,
    pub signature: String,
}

// ── Signing helpers ───────────────────────────────────────────────────────────

/// Sorts all JSON object keys recursively (for canonical serialization).
fn sort_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> = map
                .into_iter()
                .map(|(k, v)| (k, sort_json(v)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json).collect())
        }
        other => other,
    }
}

/// Sign a JSON value (with signature field removed) using Ed25519.
/// Returns base64-encoded signature.
pub fn sign_payload(payload: &serde_json::Value, signing_key: &SigningKey) -> String {
    let mut v = payload.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("signature");
    }
    let canonical = serde_json::to_string(&sort_json(v)).expect("serialization cannot fail");
    let sig = signing_key.sign(canonical.as_bytes());
    STANDARD.encode(sig.to_bytes())
}

/// Verify a JSON payload against a base64-encoded Ed25519 signature and public key.
pub fn verify_payload(
    payload: &serde_json::Value,
    signature_b64: &str,
    public_key_b64: &str,
) -> Result<()> {
    let pubkey_bytes: [u8; 32] = STANDARD
        .decode(public_key_b64)
        .map_err(|_| KrillnotesError::InvalidSignature)?
        .try_into()
        .map_err(|_| KrillnotesError::InvalidSignature)?;
    let verifying_key =
        VerifyingKey::from_bytes(&pubkey_bytes).map_err(|_| KrillnotesError::InvalidSignature)?;
    let sig_bytes: [u8; 64] = STANDARD
        .decode(signature_b64)
        .map_err(|_| KrillnotesError::InvalidSignature)?
        .try_into()
        .map_err(|_| KrillnotesError::InvalidSignature)?;
    let signature = Signature::from_bytes(&sig_bytes);

    let mut v = payload.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("signature");
    }
    let canonical = serde_json::to_string(&sort_json(v)).expect("serialization cannot fail");
    verifying_key
        .verify(canonical.as_bytes(), &signature)
        .map_err(|_| KrillnotesError::InvalidSignature)
}

// ── Zip I/O helpers ───────────────────────────────────────────────────────────

/// Write a single JSON string into a zip archive at `path`, stored as `entry_name`.
fn write_json_zip(path: &Path, entry_name: &str, json: &str) -> Result<()> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;
    let file = std::fs::File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(entry_name, options)
        .map_err(|e| KrillnotesError::Swarm(format!("zip write error: {e}")))?;
    zip.write_all(json.as_bytes())?;
    zip.finish()
        .map_err(|e| KrillnotesError::Swarm(format!("zip finish error: {e}")))?;
    Ok(())
}

/// Read the contents of `entry_name` from the zip archive at `path`.
fn read_json_from_zip(path: &Path, entry_name: &str) -> Result<String> {
    use std::io::Read;
    use zip::ZipArchive;
    let file = std::fs::File::open(path)?;
    let mut zip = ZipArchive::new(file)
        .map_err(|e| KrillnotesError::Swarm(format!("Cannot open .swarm file: {e}")))?;
    let mut entry = zip.by_name(entry_name)
        .map_err(|_| KrillnotesError::Swarm(format!("Missing '{}' in .swarm file", entry_name)))?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf)?;
    Ok(buf)
}

// ── InviteManager ─────────────────────────────────────────────────────────────

pub struct InviteManager {
    invites_dir: PathBuf,
}

impl InviteManager {
    pub fn new(invites_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&invites_dir)?;
        Ok(Self { invites_dir })
    }

    fn path_for(&self, id: Uuid) -> PathBuf {
        self.invites_dir.join(format!("{}.json", id))
    }

    fn save_record(&self, record: &InviteRecord) -> Result<()> {
        let json = serde_json::to_string_pretty(record)?;
        std::fs::write(self.path_for(record.invite_id), json)?;
        Ok(())
    }

    /// Create a new invite and return the record + signed InviteFile.
    #[allow(clippy::too_many_arguments)]
    pub fn create_invite(
        &mut self,
        workspace_id: &str,
        workspace_name: &str,
        expires_in_days: Option<u32>,
        signing_key: &SigningKey,
        inviter_declared_name: &str,
        workspace_description: Option<String>,
        workspace_author_name: Option<String>,
        workspace_author_org: Option<String>,
        workspace_homepage_url: Option<String>,
        workspace_license: Option<String>,
        workspace_tags: Vec<String>,
    ) -> Result<(InviteRecord, InviteFile)> {
        let invite_id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = expires_in_days.map(|d| now + Duration::days(d as i64));

        let record = InviteRecord {
            invite_id,
            workspace_id: workspace_id.to_string(),
            workspace_name: workspace_name.to_string(),
            created_at: now,
            expires_at,
            revoked: false,
            use_count: 0,
        };
        self.save_record(&record)?;

        let pubkey_b64 = STANDARD.encode(signing_key.verifying_key().to_bytes());
        let mut file = InviteFile {
            file_type: "krillnotes-invite-v1".to_string(),
            invite_id: invite_id.to_string(),
            workspace_id: workspace_id.to_string(),
            workspace_name: workspace_name.to_string(),
            workspace_description,
            workspace_author_name,
            workspace_author_org,
            workspace_homepage_url,
            workspace_license,
            workspace_language: None, // TODO: expose workspace_language parameter when needed
            workspace_tags,
            inviter_public_key: pubkey_b64,
            inviter_declared_name: inviter_declared_name.to_string(),
            expires_at: expires_at.map(|dt| dt.to_rfc3339()),
            signature: String::new(),
        };
        let payload = serde_json::to_value(&file)?;
        file.signature = sign_payload(&payload, signing_key);
        Ok((record, file))
    }

    pub fn list_invites(&self) -> Result<Vec<InviteRecord>> {
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&self.invites_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let json = std::fs::read_to_string(entry.path())?;
            let record: InviteRecord = serde_json::from_str(&json)?;
            records.push(record);
        }
        records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(records)
    }

    pub fn get_invite(&self, invite_id: Uuid) -> Result<Option<InviteRecord>> {
        let path = self.path_for(invite_id);
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&json)?))
    }

    pub fn revoke_invite(&mut self, invite_id: Uuid) -> Result<()> {
        let mut record = self
            .get_invite(invite_id)?
            .ok_or_else(|| KrillnotesError::Swarm(format!("Invite {} not found", invite_id)))?;
        record.revoked = true;
        self.save_record(&record)
    }

    pub fn increment_use_count(&mut self, invite_id: Uuid) -> Result<()> {
        let mut record = self
            .get_invite(invite_id)?
            .ok_or_else(|| KrillnotesError::Swarm(format!("Invite {} not found", invite_id)))?;
        record.use_count += 1;
        self.save_record(&record)
    }

    /// Parse and verify a response `.swarm` file (inviter side).
    /// Returns the PendingPeer data. Does NOT check invite validity here —
    /// the Tauri command does that after looking up the record.
    pub fn parse_and_verify_response(path: &Path) -> Result<InviteResponseFile> {
        let json = read_json_from_zip(path, "response.json")?;
        let response: InviteResponseFile = serde_json::from_str(&json)?;
        if response.file_type != "krillnotes-invite-response-v1" {
            return Err(KrillnotesError::Swarm("Not a response file".to_string()));
        }
        let payload = serde_json::to_value(&response)?;
        verify_payload(&payload, &response.signature, &response.invitee_public_key)?;
        Ok(response)
    }

    /// Parse and verify an invite `.swarm` file (invitee side).
    pub fn parse_and_verify_invite(path: &Path) -> Result<InviteFile> {
        let json = read_json_from_zip(path, "invite.json")?;
        let invite: InviteFile = serde_json::from_str(&json)?;
        if invite.file_type != "krillnotes-invite-v1" {
            return Err(KrillnotesError::Swarm("Not an invite file".to_string()));
        }
        let payload = serde_json::to_value(&invite)?;
        verify_payload(&payload, &invite.signature, &invite.inviter_public_key)?;
        Ok(invite)
    }

    /// Build and sign a response file (invitee side). Writes to `save_path`.
    pub fn build_and_save_response(
        invite: &InviteFile,
        signing_key: &SigningKey,
        declared_name: &str,
        save_path: &Path,
    ) -> Result<()> {
        let pubkey_b64 = STANDARD.encode(signing_key.verifying_key().to_bytes());
        let mut response = InviteResponseFile {
            file_type: "krillnotes-invite-response-v1".to_string(),
            invite_id: invite.invite_id.clone(),
            invitee_public_key: pubkey_b64,
            invitee_declared_name: declared_name.to_string(),
            signature: String::new(),
        };
        let payload = serde_json::to_value(&response)?;
        response.signature = sign_payload(&payload, signing_key);
        let json = serde_json::to_string_pretty(&response)?;
        write_json_zip(save_path, "response.json", &json)?;
        Ok(())
    }

    /// Write a signed InviteFile as a zip archive. Called by the Tauri command after create_invite.
    pub fn save_invite_file(file: &InviteFile, save_path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(file)?;
        write_json_zip(save_path, "invite.json", &json)
    }
}

#[cfg(test)]
mod manager_tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, SigningKey) {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[1u8; 32]);
        (dir, key)
    }

    #[test]
    fn create_and_list_invite() {
        let (dir, key) = setup();
        let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
        let (record, file) = mgr
            .create_invite("ws-id", "My Workspace", None, &key, "Alice", None, None, None, None, None, vec![])
            .unwrap();
        assert_eq!(record.workspace_id, "ws-id");
        assert!(!record.revoked);
        assert_eq!(record.use_count, 0);
        assert_eq!(file.file_type, "krillnotes-invite-v1");
        let list = mgr.list_invites().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn revoke_invite() {
        let (dir, key) = setup();
        let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
        let (record, _) = mgr
            .create_invite("ws-id", "My Workspace", None, &key, "Alice", None, None, None, None, None, vec![])
            .unwrap();
        mgr.revoke_invite(record.invite_id).unwrap();
        let list = mgr.list_invites().unwrap();
        assert!(list[0].revoked);
    }

    #[test]
    fn expires_in_days() {
        let (dir, key) = setup();
        let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
        let (record, file) = mgr
            .create_invite("ws-id", "My Workspace", Some(7), &key, "Alice", None, None, None, None, None, vec![])
            .unwrap();
        assert!(record.expires_at.is_some());
        assert!(file.expires_at.is_some());
    }

    #[test]
    fn invite_response_roundtrip() {
        let (dir, inviter_key) = setup();
        let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
        let (_, invite_file) = mgr
            .create_invite("ws-id", "My Workspace", None, &inviter_key, "Alice", None, None, None, None, None, vec![])
            .unwrap();

        // Invitee builds response
        let invitee_key = SigningKey::from_bytes(&[2u8; 32]);
        let response_path = dir.path().join("response.swarm");
        InviteManager::build_and_save_response(&invite_file, &invitee_key, "Bob", &response_path).unwrap();

        // Inviter parses and verifies response
        let response = InviteManager::parse_and_verify_response(&response_path).unwrap();
        assert_eq!(response.invitee_declared_name, "Bob");
        assert_eq!(response.invite_id, invite_file.invite_id);
    }
}

#[cfg(test)]
mod signing_tests {
    use super::*;

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = test_key();
        let pubkey_b64 = STANDARD.encode(key.verifying_key().to_bytes());
        let payload = serde_json::json!({ "hello": "world", "number": 42 });
        let sig = sign_payload(&payload, &key);
        let mut signed = payload.clone();
        signed["signature"] = serde_json::Value::String(sig);
        assert!(verify_payload(&signed, signed["signature"].as_str().unwrap(), &pubkey_b64).is_ok());
    }

    #[test]
    fn verify_fails_on_tampered_payload() {
        let key = test_key();
        let pubkey_b64 = STANDARD.encode(key.verifying_key().to_bytes());
        let payload = serde_json::json!({ "hello": "world" });
        let sig = sign_payload(&payload, &key);
        let mut tampered = payload.clone();
        tampered["hello"] = serde_json::Value::String("evil".to_string());
        tampered["signature"] = serde_json::Value::String(sig);
        assert!(verify_payload(&tampered, tampered["signature"].as_str().unwrap(), &pubkey_b64).is_err());
    }
}
