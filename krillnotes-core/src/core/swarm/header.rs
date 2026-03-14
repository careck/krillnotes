// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! `.swarm` bundle header — always `header.json`, always unencrypted.

use serde::{Deserialize, Serialize};
use crate::Result;
use crate::KrillnotesError;

/// The four bundle modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwarmMode {
    Invite,
    Accept,
    Snapshot,
    Delta,
}

/// Encrypted payload key for one recipient.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientEntry {
    /// Device ID or display identifier of the recipient.
    pub peer_id: String,
    /// AES-256-GCM key wrapped with recipient's X25519 public key.
    #[serde(with = "base64_bytes")]
    pub encrypted_key: Vec<u8>,
}

/// The common + mode-specific `.swarm` header.
///
/// Serialised as `header.json` in the zip archive. Never encrypted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmHeader {
    pub format_version: u32,
    pub mode: SwarmMode,
    pub workspace_id: String,
    pub workspace_name: String,
    pub source_device_id: String,
    /// Ed25519 public key of the sending identity (base64).
    pub source_identity: String,
    pub source_display_name: String,
    /// ISO 8601 creation time.
    pub created_at: String,

    // Invite + Accept
    pub pairing_token: Option<String>,
    /// Invite only: role being offered ("owner" | "writer" | "reader").
    pub offered_role: Option<String>,
    /// Invite only: subtree root note ID, or None for workspace-level.
    pub offered_scope: Option<String>,
    pub inviter_fingerprint: Option<String>,

    // Accept only
    pub accepted_identity: Option<String>,
    pub accepted_display_name: Option<String>,
    pub accepted_fingerprint: Option<String>,

    // Snapshot only
    pub as_of_operation_id: Option<String>,

    // Delta only
    pub since_operation_id: Option<String>,
    pub target_peer: Option<String>,

    // Snapshot + Delta
    pub recipients: Option<Vec<RecipientEntry>>,
    pub has_attachments: bool,

    /// Ed25519 public key of the workspace owner (base64). Present in all new bundles.
    pub owner_pubkey: Option<String>,
}

impl SwarmHeader {
    /// Validate that all required fields for the bundle's mode are present.
    pub fn validate(&self) -> Result<()> {
        match self.mode {
            SwarmMode::Invite => {
                require_field(self.pairing_token.as_ref(), "pairing_token", "invite")?;
                require_field(self.offered_role.as_ref(), "offered_role", "invite")?;
            }
            SwarmMode::Accept => {
                require_field(self.pairing_token.as_ref(), "pairing_token", "accept")?;
                require_field(self.accepted_identity.as_ref(), "accepted_identity", "accept")?;
            }
            SwarmMode::Snapshot => {
                require_field(self.as_of_operation_id.as_ref(), "as_of_operation_id", "snapshot")?;
                require_field(self.recipients.as_ref(), "recipients", "snapshot")?;
            }
            SwarmMode::Delta => {
                require_field(self.since_operation_id.as_ref(), "since_operation_id", "delta")?;
            }
        }
        Ok(())
    }
}

fn require_field<T>(val: Option<&T>, name: &str, mode: &str) -> Result<()> {
    if val.is_none() {
        Err(KrillnotesError::Swarm(format!(
            "missing required field '{name}' for {mode} bundle"
        )))
    } else {
        Ok(())
    }
}

/// serde helper: serialize Vec<u8> as base64 string.
mod base64_bytes {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&BASE64.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(de)?;
        BASE64.decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header(mode: SwarmMode) -> SwarmHeader {
        SwarmHeader {
            format_version: 1,
            mode,
            workspace_id: "ws-uuid".to_string(),
            workspace_name: "Test WS".to_string(),
            source_device_id: "dev-uuid".to_string(),
            source_identity: "pubkey_b64".to_string(),
            source_display_name: "Alice".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pairing_token: None,
            offered_role: None,
            offered_scope: None,
            inviter_fingerprint: None,
            accepted_identity: None,
            accepted_display_name: None,
            accepted_fingerprint: None,
            as_of_operation_id: None,
            since_operation_id: None,
            target_peer: None,
            recipients: None,
            has_attachments: false,
            owner_pubkey: None,
        }
    }

    #[test]
    fn test_header_roundtrip_delta() {
        let mut h = sample_header(SwarmMode::Delta);
        h.since_operation_id = Some("op-uuid".to_string());
        h.recipients = Some(vec![RecipientEntry {
            peer_id: "dev-1".to_string(),
            encrypted_key: vec![1, 2, 3],
        }]);
        let json = serde_json::to_string(&h).unwrap();
        let back: SwarmHeader = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mode, SwarmMode::Delta);
        assert_eq!(back.since_operation_id.as_deref(), Some("op-uuid"));
    }

    #[test]
    fn test_header_roundtrip_invite() {
        let mut h = sample_header(SwarmMode::Invite);
        h.pairing_token = Some("token_b64".to_string());
        h.offered_role = Some("writer".to_string());
        let json = serde_json::to_string(&h).unwrap();
        let back: SwarmHeader = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pairing_token.as_deref(), Some("token_b64"));
    }

    #[test]
    fn test_validate_delta_requires_since_op() {
        let h = sample_header(SwarmMode::Delta);
        assert!(h.validate().is_err());
    }

    #[test]
    fn test_validate_invite_requires_pairing_token() {
        let h = sample_header(SwarmMode::Invite);
        assert!(h.validate().is_err());
    }

    #[test]
    fn test_header_roundtrip_with_owner_pubkey() {
        let mut h = sample_header(SwarmMode::Delta);
        h.since_operation_id = Some("op-uuid".to_string());
        h.owner_pubkey = Some("owner_b64_key".to_string());
        let json = serde_json::to_string(&h).unwrap();
        let back: SwarmHeader = serde_json::from_str(&json).unwrap();
        assert_eq!(back.owner_pubkey.as_deref(), Some("owner_b64_key"));
    }

    #[test]
    fn test_header_roundtrip_without_owner_pubkey() {
        let mut h = sample_header(SwarmMode::Delta);
        h.since_operation_id = Some("op-uuid".to_string());
        // owner_pubkey is None (backward compat)
        let json = serde_json::to_string(&h).unwrap();
        let back: SwarmHeader = serde_json::from_str(&json).unwrap();
        assert!(back.owner_pubkey.is_none());
    }
}
