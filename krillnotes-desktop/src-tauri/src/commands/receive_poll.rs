// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

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

#[tauri::command]
pub async fn poll_receive_workspace(
    _state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    // For now, this delegates to the existing poll_sync mechanism.
    // The workspace-level polling reuses SyncEngine::poll() which already
    // handles delta bundles. This command is a placeholder that can be
    // enhanced later to also check for accept-mode bundles.
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
