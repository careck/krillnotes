// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State, Window};
use uuid::Uuid;
use krillnotes_core::core::sync::relay::RelayClient;

// --- ReceivedResponse CRUD ---

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedResponseInfo {
    pub response_id: String,
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub invitee_public_key: String,
    pub invitee_declared_name: String,
    pub received_at: String,
    pub status: String,
}

impl From<krillnotes_core::core::received_response::ReceivedResponse> for ReceivedResponseInfo {
    fn from(r: krillnotes_core::core::received_response::ReceivedResponse) -> Self {
        Self {
            response_id: r.response_id.to_string(),
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            invitee_public_key: r.invitee_public_key,
            invitee_declared_name: r.invitee_declared_name,
            received_at: r.received_at.to_rfc3339(),
            status: match r.status {
                krillnotes_core::core::received_response::ReceivedResponseStatus::Pending => "pending".to_string(),
                krillnotes_core::core::received_response::ReceivedResponseStatus::PeerAdded => "peerAdded".to_string(),
                krillnotes_core::core::received_response::ReceivedResponseStatus::SnapshotSent => "snapshotSent".to_string(),
            },
        }
    }
}

#[tauri::command]
pub fn list_received_responses(
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_id: Option<String>,
) -> std::result::Result<Vec<ReceivedResponseInfo>, String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let managers = state.received_response_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let records = if let Some(ws_id) = workspace_id {
        mgr.list_by_workspace(&ws_id).map_err(|e| e.to_string())?
    } else {
        mgr.list().map_err(|e| e.to_string())?
    };
    Ok(records.into_iter().map(ReceivedResponseInfo::from).collect())
}

#[tauri::command]
pub fn update_response_status(
    state: State<'_, AppState>,
    identity_uuid: String,
    response_id: String,
    status: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let resp_uuid: Uuid = response_id.parse().map_err(|e| format!("Invalid response UUID: {e}"))?;
    let mut managers = state.received_response_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    let new_status = match status.as_str() {
        "pending" => krillnotes_core::core::received_response::ReceivedResponseStatus::Pending,
        "peerAdded" => krillnotes_core::core::received_response::ReceivedResponseStatus::PeerAdded,
        "snapshotSent" => krillnotes_core::core::received_response::ReceivedResponseStatus::SnapshotSent,
        _ => return Err(format!("Invalid status: {status}")),
    };
    mgr.update_status(resp_uuid, new_status).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn dismiss_response(
    state: State<'_, AppState>,
    identity_uuid: String,
    response_id: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let resp_uuid: Uuid = response_id.parse().map_err(|e| format!("Invalid response UUID: {e}"))?;
    let mut managers = state.received_response_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    mgr.delete(resp_uuid).map_err(|e| e.to_string())
}

// --- Async polling commands ---

/// Poll the relay for Accept-mode bundles addressed to the current workspace.
///
/// When the invitee sends an invite response via relay, they also upload an Accept
/// bundle. This command discovers those bundles, parses them, creates
/// `ReceivedResponse` records, and emits `invite-response-received` events.
#[tauri::command]
pub async fn poll_receive_workspace(
    window: Window,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let workspace_label = window.label().to_string();

    // -- Collect context data under brief locks (all guards released before spawn) --
    let identity_uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };

    let workspace_id = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        workspaces
            .get(&workspace_label)
            .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?
            .workspace_id()
            .to_string()
    };

    // Get relay accounts for this identity (brief lock).
    let relay_accounts = {
        let ram = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
        if let Some(mgr) = ram.get(&identity_uuid) {
            mgr.list_relay_accounts().unwrap_or_default()
        } else {
            vec![]
        }
    };

    log::debug!(
        "poll_receive_workspace: window={}, identity={}, workspace={}, relay_accounts={}",
        workspace_label, identity_uuid, workspace_id, relay_accounts.len()
    );

    if relay_accounts.is_empty() {
        log::debug!("poll_receive_workspace: no relay accounts for identity {identity_uuid}, skipping");
        return Ok(());
    }

    // Get active (non-revoked) invites for this workspace to correlate accept bundles.
    let active_invites: Vec<krillnotes_core::core::invite::InviteRecord> = {
        let ims = state.invite_managers.lock().map_err(|e| e.to_string())?;
        if let Some(im) = ims.get(&identity_uuid) {
            im.list_invites()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| !r.revoked && r.workspace_id == workspace_id)
                .collect()
        } else {
            vec![]
        }
    };

    log::debug!(
        "poll_receive_workspace: found {} active invite(s) for workspace {}",
        active_invites.len(), workspace_id
    );

    if active_invites.is_empty() {
        log::debug!("poll_receive_workspace: no active invites, skipping");
        return Ok(());
    }

    // Clone Arcs for use inside spawn_blocking.
    let received_response_managers_arc = Arc::clone(&state.received_response_managers);
    let invite_managers_arc = Arc::clone(&state.invite_managers);
    let workspace_id_clone = workspace_id.clone();

    // Perform relay I/O on a blocking thread (RelayClient uses reqwest::blocking).
    let new_responses = tokio::task::spawn_blocking(move || -> Result<Vec<ReceivedResponseInfo>, String> {
        use krillnotes_core::core::swarm::invite::parse_accept_bundle;

        let mut all_new_responses = Vec::new();

        // For now, use the first relay account (same pattern as poll_sync).
        let acct = &relay_accounts[0];
        log::debug!("poll_receive_workspace: using relay account {} on {}", acct.email, acct.relay_url);
        let mut token = acct.session_token.clone();
        // Auto-login if session expired.
        if acct.session_expires_at < chrono::Utc::now() && !acct.password.is_empty() {
            let client = RelayClient::new(&acct.relay_url);
            match client.login(&acct.email, &acct.password, &acct.device_public_key) {
                Ok(session) => token = session.session_token,
                Err(e) => {
                    log::warn!("poll_receive_workspace: auto-login failed for {}: {e}", acct.relay_url);
                    return Ok(all_new_responses);
                }
            }
        }
        let client = RelayClient::new(&acct.relay_url).with_session_token(&token);

        // List all pending bundles.
        let bundles = match client.list_bundles() {
            Ok(b) => b,
            Err(e) => {
                log::warn!("poll_receive_workspace: list_bundles failed: {e}");
                return Ok(all_new_responses);
            }
        };

        // Filter for accept-mode bundles targeting this workspace.
        let accept_bundles: Vec<_> = bundles
            .iter()
            .filter(|b| b.mode == "accept" && b.workspace_id == workspace_id_clone)
            .collect();

        if accept_bundles.is_empty() {
            return Ok(all_new_responses);
        }

        log::info!(
            "poll_receive_workspace: found {} accept bundle(s) for workspace {}",
            accept_bundles.len(),
            workspace_id_clone,
        );

        for bundle_meta in accept_bundles {
            // Download the bundle bytes.
            let bundle_bytes = match client.download_bundle(&bundle_meta.bundle_id) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!(
                        "poll_receive_workspace: download bundle {} failed: {e}",
                        bundle_meta.bundle_id,
                    );
                    continue;
                }
            };

            // Parse the accept bundle.
            let parsed = match parse_accept_bundle(&bundle_bytes) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!(
                        "poll_receive_workspace: parse accept bundle {} failed: {e}",
                        bundle_meta.bundle_id,
                    );
                    // Delete invalid bundle so we don't keep re-processing it.
                    let _ = client.delete_bundle(&bundle_meta.bundle_id);
                    continue;
                }
            };

            // Find a matching invite by workspace_id.
            let matched_invite = active_invites
                .iter()
                .find(|inv| inv.workspace_id == parsed.workspace_id);

            let invite_id = match matched_invite {
                Some(inv) => inv.invite_id,
                None => {
                    log::warn!(
                        "poll_receive_workspace: no matching invite for workspace {} in accept bundle {}",
                        parsed.workspace_id,
                        bundle_meta.bundle_id,
                    );
                    // Delete the bundle — we can't correlate it.
                    let _ = client.delete_bundle(&bundle_meta.bundle_id);
                    continue;
                }
            };

            // Check for duplicate responses (same invite + same invitee public key).
            {
                let managers = received_response_managers_arc.lock().map_err(|e| e.to_string())?;
                if let Some(mgr) = managers.get(&identity_uuid) {
                    if let Ok(Some(_existing)) =
                        mgr.find_by_invite_and_invitee(invite_id, &parsed.acceptor_public_key)
                    {
                        log::info!(
                            "poll_receive_workspace: duplicate accept from {} for invite {}, skipping",
                            parsed.declared_name,
                            invite_id,
                        );
                        let _ = client.delete_bundle(&bundle_meta.bundle_id);
                        continue;
                    }
                }
            }

            // Create a ReceivedResponse record.
            let workspace_name = matched_invite
                .map(|inv| inv.workspace_name.clone())
                .unwrap_or_default();
            let received = krillnotes_core::core::received_response::ReceivedResponse::new(
                invite_id,
                parsed.workspace_id.clone(),
                workspace_name,
                parsed.acceptor_public_key.clone(),
                parsed.declared_name.clone(),
            );

            // Save the record.
            {
                let mut managers = received_response_managers_arc.lock().map_err(|e| e.to_string())?;
                if let Some(mgr) = managers.get_mut(&identity_uuid) {
                    if let Err(e) = mgr.save(&received) {
                        log::error!("poll_receive_workspace: failed to save received response: {e}");
                        continue;
                    }
                }
            }

            // Increment the invite use count.
            {
                let mut ims = invite_managers_arc.lock().map_err(|e| e.to_string())?;
                if let Some(im) = ims.get_mut(&identity_uuid) {
                    let _ = im.increment_use_count(invite_id);
                }
            }

            log::info!(
                "poll_receive_workspace: recorded response from '{}' for invite {}",
                parsed.declared_name,
                invite_id,
            );

            all_new_responses.push(ReceivedResponseInfo::from(received));

            // Delete the processed bundle from the relay.
            let _ = client.delete_bundle(&bundle_meta.bundle_id);
        }

        Ok(all_new_responses)
    })
    .await
    .map_err(|e| {
        log::error!("poll_receive_workspace spawn_blocking join failed: {e}");
        e.to_string()
    })??;

    // Emit events for each new response.
    for resp in &new_responses {
        let _ = window.emit("invite-response-received", serde_json::json!({
            "responseId": resp.response_id,
            "inviteId": resp.invite_id,
            "workspaceId": resp.workspace_id,
            "inviteeDeclaredName": resp.invitee_declared_name,
        }));
    }

    Ok(())
}

#[tauri::command]
pub async fn poll_receive_identity(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;

    // Get accepted invites in WaitingSnapshot status (brief lock)
    let waiting = {
        let aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
        let ai_mgr = aim.get(&uuid).ok_or("Accepted invite manager not found")?;
        ai_mgr.list_waiting_snapshot().map_err(|e| e.to_string())?
    };

    if waiting.is_empty() {
        return Ok(());
    }

    // Get relay accounts (brief lock)
    let relay_accounts = {
        let rams = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
        let ram = rams.get(&uuid).ok_or("Relay account manager not found")?;
        ram.list_relay_accounts().unwrap_or_default()
    };

    if relay_accounts.is_empty() {
        return Ok(());
    }

    // Build RelayConnections and call core function inside spawn_blocking
    let temp_dir = std::env::temp_dir();

    let result = tokio::task::spawn_blocking(move || {
        use krillnotes_core::core::sync::receive_poll::{RelayConnection, receive_poll_identity};
        use krillnotes_core::core::sync::relay::client::RelayClient;

        let connections: Vec<RelayConnection> = relay_accounts.into_iter()
            .map(|account| {
                let client = RelayClient::new(&account.relay_url)
                    .with_session_token(&account.session_token);
                RelayConnection { account, client }
            })
            .collect();

        receive_poll_identity(&connections, &waiting, &temp_dir)
            .map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())??;

    // Emit events
    for snapshot in &result.received_snapshots {
        let _ = app_handle.emit("snapshot-received", serde_json::json!({
            "workspaceId": snapshot.workspace_id,
            "inviteId": snapshot.invite_id.to_string(),
            "snapshotPath": snapshot.snapshot_path.to_string_lossy(),
        }));
    }
    for error in &result.errors {
        let _ = app_handle.emit("poll-error", serde_json::json!({ "error": error.error }));
    }

    Ok(())
}
