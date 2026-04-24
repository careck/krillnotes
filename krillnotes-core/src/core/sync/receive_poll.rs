// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Receive-only polling types and functions.
//!
//! This module provides types for reporting poll results and a standalone
//! `receive_poll_identity()` function that checks relay mailboxes for
//! snapshots destined to accepted invites that are still waiting.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::core::error::KrillnotesError;
use crate::core::received_response::ReceivedResponse;

type Result<T> = std::result::Result<T, KrillnotesError>;

// ── Result types ─────────────────────────────────────────────────────────────

/// A single successfully-applied delta bundle from a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedBundle {
    pub peer_device_id: String,
    pub mode: String,
    pub op_count: usize,
}

/// A non-fatal error encountered during a poll cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollError {
    pub bundle_id: Option<String>,
    pub error: String,
}

/// Summary of polling one workspace's relay mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePollResult {
    pub applied_bundles: Vec<AppliedBundle>,
    pub new_responses: Vec<ReceivedResponse>,
    pub errors: Vec<PollError>,
}

/// A snapshot downloaded from a relay mailbox for a pending accepted invite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedSnapshot {
    pub workspace_id: String,
    pub invite_id: Uuid,
    pub snapshot_path: PathBuf,
    pub sender_device_key: String,
}

/// Summary of polling all relay connections at the identity level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPollResult {
    pub received_snapshots: Vec<ReceivedSnapshot>,
    pub errors: Vec<PollError>,
}

// ── Relay-specific types and functions ───────────────────────────────────────

#[cfg(feature = "relay")]
use crate::core::accepted_invite::{AcceptedInvite, AcceptedInviteStatus};
#[cfg(feature = "relay")]
use crate::core::sync::relay::client::RelayClient;
#[cfg(feature = "relay")]
use crate::core::sync::relay::relay_account::RelayAccount;

/// A relay account paired with its authenticated client, ready to poll.
#[cfg(feature = "relay")]
pub struct RelayConnection {
    pub account: RelayAccount,
    pub client: RelayClient,
}

/// Poll all relay connections for snapshot bundles destined to accepted
/// invites that are still in `WaitingSnapshot` status.
///
/// For each matching bundle the snapshot bytes are written to a temp file
/// under `temp_dir` and the bundle is deleted from the relay.  The caller
/// is responsible for importing the snapshot into a workspace.
#[cfg(feature = "relay")]
pub fn receive_poll_identity(
    relay_connections: &[RelayConnection],
    accepted_invites: &[AcceptedInvite],
    temp_dir: &std::path::Path,
    device_id: &str,
) -> Result<IdentityPollResult> {
    let mut result = IdentityPollResult {
        received_snapshots: Vec::new(),
        errors: Vec::new(),
    };

    let waiting_ws_ids: std::collections::HashSet<String> = accepted_invites
        .iter()
        .filter(|i| i.status == AcceptedInviteStatus::WaitingSnapshot)
        .map(|i| i.workspace_id.clone())
        .collect();

    if waiting_ws_ids.is_empty() {
        return Ok(result);
    }

    for conn in relay_connections {
        // Register mailboxes so the relay routes bundles to this account.
        for ws_id in &waiting_ws_ids {
            if let Err(e) = conn.client.ensure_mailbox(ws_id) {
                log::warn!("Failed to ensure mailbox for {ws_id}: {e}");
            }
        }

        let bundle_metas = match conn.client.list_bundles(device_id) {
            Ok(metas) => metas,
            Err(e) => {
                result.errors.push(PollError {
                    bundle_id: None,
                    error: format!("list_bundles on {} failed: {e}", conn.account.relay_url),
                });
                continue;
            }
        };

        let snapshots: Vec<_> = bundle_metas
            .into_iter()
            .filter(|m| waiting_ws_ids.contains(&m.workspace_id))
            .filter(|m| m.mode == "snapshot")
            .collect();

        for meta in snapshots {
            let bid = meta.bundle_id.clone();
            match conn.client.download_bundle(&meta.bundle_id) {
                Ok(bundle_bytes) => {
                    let snapshot_path = temp_dir.join(format!("snapshot-{}.bin", Uuid::new_v4()));
                    if let Err(e) = std::fs::write(&snapshot_path, &bundle_bytes) {
                        result.errors.push(PollError {
                            bundle_id: Some(bid),
                            error: format!("Failed to write snapshot: {e}"),
                        });
                        continue;
                    }

                    let invite_id = accepted_invites
                        .iter()
                        .find(|i| i.workspace_id == meta.workspace_id)
                        .map(|i| i.invite_id)
                        .unwrap_or_default();

                    result.received_snapshots.push(ReceivedSnapshot {
                        workspace_id: meta.workspace_id.clone(),
                        invite_id,
                        snapshot_path,
                        sender_device_key: meta.sender_device_key.clone(),
                    });

                    let _ = conn.client.delete_bundle(&bid);
                }
                Err(e) => {
                    result.errors.push(PollError {
                        bundle_id: Some(bid),
                        error: format!("download_bundle failed: {e}"),
                    });
                }
            }
        }
    }

    Ok(result)
}

// Workspace-level polling (receive_poll_workspace) is handled by the
// existing SyncEngine::poll() and is not duplicated here.

#[cfg(test)]
#[path = "receive_poll_tests.rs"]
mod tests;
