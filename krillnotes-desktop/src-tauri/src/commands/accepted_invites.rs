// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedInviteInfo {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    pub accepted_at: String,
    pub response_relay_url: Option<String>,
    pub status: String,
    pub workspace_path: Option<String>,
    pub snapshot_path: Option<String>,
    pub offered_role: String,
}

impl From<krillnotes_core::core::accepted_invite::AcceptedInvite> for AcceptedInviteInfo {
    fn from(r: krillnotes_core::core::accepted_invite::AcceptedInvite) -> Self {
        Self {
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            inviter_public_key: r.inviter_public_key,
            inviter_declared_name: r.inviter_declared_name,
            accepted_at: r.accepted_at.to_rfc3339(),
            response_relay_url: r.response_relay_url,
            status: match r.status {
                krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WaitingSnapshot => "waitingSnapshot".to_string(),
                krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WorkspaceCreated => "workspaceCreated".to_string(),
            },
            workspace_path: r.workspace_path,
            snapshot_path: r.snapshot_path,
            offered_role: r.offered_role,
        }
    }
}

#[tauri::command]
pub fn list_accepted_invites(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<AcceptedInviteInfo>, String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let records = mgr.list().map_err(|e| {
        log::error!("list_accepted_invites(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })?;
    Ok(records.into_iter().map(AcceptedInviteInfo::from).collect())
}

#[tauri::command]
pub fn save_accepted_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
    workspace_id: String,
    workspace_name: String,
    inviter_public_key: String,
    inviter_declared_name: String,
    response_relay_url: Option<String>,
    offered_role: String,
) -> std::result::Result<AcceptedInviteInfo, String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let invite_uuid: Uuid = invite_id.parse().map_err(|e| format!("Invalid invite UUID: {e}"))?;
    let mut managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;

    let invite = krillnotes_core::core::accepted_invite::AcceptedInvite::new(
        invite_uuid, workspace_id, workspace_name,
        inviter_public_key, inviter_declared_name, response_relay_url,
        offered_role,
    );
    mgr.save(&invite).map_err(|e| e.to_string())?;
    Ok(AcceptedInviteInfo::from(invite))
}

#[tauri::command]
pub fn update_accepted_invite_status(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
    status: String,
    workspace_path: Option<String>,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let invite_uuid: Uuid = invite_id.parse().map_err(|e| format!("Invalid invite UUID: {e}"))?;
    let mut managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    let new_status = match status.as_str() {
        "waitingSnapshot" => krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WaitingSnapshot,
        "workspaceCreated" => krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WorkspaceCreated,
        _ => return Err(format!("Invalid status: {status}")),
    };
    mgr.update_status(invite_uuid, new_status, workspace_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_accepted_invite_snapshot_path(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
    snapshot_path: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let invite_uuid: Uuid = invite_id.parse().map_err(|e| format!("Invalid invite UUID: {e}"))?;
    let mut managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    mgr.update_snapshot_path(invite_uuid, snapshot_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_accepted_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let invite_uuid: Uuid = invite_id.parse().map_err(|e| format!("Invalid invite UUID: {e}"))?;
    let mut managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    mgr.delete(invite_uuid).map_err(|e| e.to_string())
}
