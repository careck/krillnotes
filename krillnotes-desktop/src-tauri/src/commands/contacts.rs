// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::State;
use serde::Serialize;
use uuid::Uuid;

/// Serialisable contact record returned to the frontend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInfo {
    pub contact_id: String,
    pub declared_name: String,
    pub local_name: Option<String>,
    pub public_key: String,
    pub fingerprint: String,
    pub trust_level: String,
    pub first_seen: String,
    pub notes: Option<String>,
}

impl ContactInfo {
    pub fn from_contact(c: krillnotes_core::core::contact::Contact) -> Self {
        Self {
            contact_id: c.contact_id.to_string(),
            declared_name: c.declared_name,
            local_name: c.local_name,
            public_key: c.public_key,
            fingerprint: c.fingerprint,
            trust_level: trust_level_to_str(&c.trust_level).to_string(),
            first_seen: c.first_seen.to_rfc3339(),
            notes: c.notes,
        }
    }
}

pub(crate) fn parse_trust_level(s: &str) -> std::result::Result<krillnotes_core::core::contact::TrustLevel, String> {
    use krillnotes_core::core::contact::TrustLevel;
    match s {
        "Tofu" => Ok(TrustLevel::Tofu),
        "CodeVerified" => Ok(TrustLevel::CodeVerified),
        "Vouched" => Ok(TrustLevel::Vouched),
        "VerifiedInPerson" => Ok(TrustLevel::VerifiedInPerson),
        other => Err(format!("Unknown trust level: {other}")),
    }
}

/// Explicit string mapping — do NOT use `format!("{:?}", ...)` which is fragile.
pub(crate) fn trust_level_to_str(tl: &krillnotes_core::core::contact::TrustLevel) -> &'static str {
    use krillnotes_core::core::contact::TrustLevel;
    match tl {
        TrustLevel::Tofu => "Tofu",
        TrustLevel::CodeVerified => "CodeVerified",
        TrustLevel::Vouched => "Vouched",
        TrustLevel::VerifiedInPerson => "VerifiedInPerson",
    }
}

#[tauri::command]
pub fn list_contacts(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<ContactInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let contacts = cm.list_contacts().map_err(|e| e.to_string())?;
    Ok(contacts.into_iter().map(ContactInfo::from_contact).collect())
}

#[tauri::command]
pub fn get_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> std::result::Result<Option<ContactInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cid = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let contact = cm.get_contact(cid).map_err(|e| e.to_string())?;
    Ok(contact.map(ContactInfo::from_contact))
}

#[tauri::command]
pub fn create_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    declared_name: String,
    public_key: String,
    trust_level: String,
) -> std::result::Result<ContactInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let tl = parse_trust_level(&trust_level)?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let contact = cm.create_contact(&declared_name, &public_key, tl)
        .map_err(|e| e.to_string())?;
    Ok(ContactInfo::from_contact(contact))
}

#[tauri::command]
pub fn update_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
    local_name: Option<String>,
    notes: Option<String>,
    trust_level: String,
) -> std::result::Result<ContactInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cid = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;
    let tl = parse_trust_level(&trust_level)?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let mut contact = cm.get_contact(cid)
        .map_err(|e| e.to_string())?
        .ok_or("Contact not found")?;
    contact.local_name = local_name;
    contact.notes = notes;
    contact.trust_level = tl;
    cm.save_contact(&contact).map_err(|e| e.to_string())?;
    Ok(ContactInfo::from_contact(contact))
}

#[tauri::command]
pub fn delete_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cid = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    cm.delete_contact(cid).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_fingerprint(public_key: String) -> std::result::Result<String, String> {
    krillnotes_core::core::contact::generate_fingerprint(&public_key)
        .map_err(|e| e.to_string())
}

// ── Peer commands ─────────────────────────────────────────────────

/// Returns all sync peers registered for the calling window's workspace,
/// enriching each entry with the matching contact name where available.
#[tauri::command]
pub fn list_workspace_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<krillnotes_core::PeerInfo>, String> {
    let window_label = window.label().to_string();

    // Resolve identity UUID from workspace binding.
    let identity_uuid = {
        let paths = state.workspace_paths.lock().expect("Mutex poisoned");
        let folder = paths.get(&window_label).ok_or("Workspace path not found")?.clone();
        drop(paths);
        let (ws_uuid_opt, _, _, _) = crate::read_info_json_full(&folder);
        // Guard: workspace must have a UUID in info.json to be valid
        ws_uuid_opt.ok_or("Workspace UUID missing from info.json")?;
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.get_workspace_binding(&folder)
            .map_err(|e| e.to_string())?
            .ok_or("No identity bound to this workspace")?
            .identity_uuid
    };

    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
    let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = contact_managers
        .get(&identity_uuid)
        .ok_or("Contact manager not found — identity must be unlocked")?;
    workspace.list_peers_info(cm).map_err(|e| e.to_string())
}

/// Returns resolved peer info for the current workspace's sync_peers.
/// Used to populate the CreateDeltaDialog peer checklist.
#[tauri::command]
pub fn get_workspace_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<krillnotes_core::core::peer_registry::PeerInfo>, String> {
    let identity_uuid_str = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        ws.identity_uuid().to_string()
    };
    let identity_uuid = Uuid::parse_str(&identity_uuid_str).map_err(|e| e.to_string())?;
    let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cm_guard.get(&identity_uuid).ok_or("Contact manager not available")?;
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
    ws.list_peers_info(cm).map_err(|e| e.to_string())
}

/// Removes a sync peer from the calling window's workspace by device ID.
#[tauri::command]
pub fn remove_workspace_peer(
    window: tauri::Window,
    state: State<'_, AppState>,
    peer_device_id: String,
) -> std::result::Result<(), String> {
    let window_label = window.label().to_string();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
    workspace.remove_peer(&peer_device_id).map_err(|e| e.to_string())
}

/// Pre-authorises a contact as a sync peer for the calling window's workspace.
/// Returns the newly created `PeerInfo` entry.
#[tauri::command]
pub fn add_contact_as_peer(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> std::result::Result<krillnotes_core::PeerInfo, String> {
    let window_label = window.label().to_string();
    let identity_uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let contact_id = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;

    // Step 1: Get contact's public key — drop contact_managers lock before touching workspaces.
    let peer_identity_id = {
        let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
        let cm = contact_managers
            .get(&identity_uuid)
            .ok_or("Contact manager not found — identity must be unlocked")?;
        let contact = cm
            .get_contact(contact_id)
            .map_err(|e| e.to_string())?
            .ok_or("Contact not found")?;
        contact.public_key.clone()
    };

    // Step 2: Add to workspace — acquire workspaces lock separately.
    {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
        workspace.add_contact_as_peer(&peer_identity_id).map_err(|e| e.to_string())?;
    }

    // Step 3: Build PeerInfo for the caller — re-acquire both locks.
    let peers = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
        let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
        let cm = contact_managers
            .get(&identity_uuid)
            .ok_or("Contact manager not found")?;
        workspace.list_peers_info(cm).map_err(|e| e.to_string())?
    };
    peers
        .into_iter()
        .find(|p| p.peer_identity_id == peer_identity_id)
        .ok_or_else(|| "Peer not found after insert".to_string())
}
