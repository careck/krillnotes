// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Invite record and `.swarm` file structures for the peer invite flow.

#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[allow(unused_imports)]
use base64::{engine::general_purpose::STANDARD, Engine};
#[allow(unused_imports)]
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
