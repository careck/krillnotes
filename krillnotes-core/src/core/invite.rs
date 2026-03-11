// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Invite record and `.swarm` file structures for the peer invite flow.

#[allow(unused_imports)]
use std::path::{Path, PathBuf};
use std::collections::BTreeMap;
#[allow(unused_imports)]
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{SigningKey, Signer, VerifyingKey, Verifier, Signature};
use crate::core::error::KrillnotesError;

#[allow(dead_code)]
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
