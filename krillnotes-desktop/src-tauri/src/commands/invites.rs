// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::State;
use uuid::Uuid;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub use_count: u32,
    pub relay_url: Option<String>,
    pub scope_note_id: Option<String>,
    pub scope_note_title: Option<String>,
}

impl From<krillnotes_core::core::invite::InviteRecord> for InviteInfo {
    fn from(r: krillnotes_core::core::invite::InviteRecord) -> Self {
        Self {
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            created_at: r.created_at.to_rfc3339(),
            expires_at: r.expires_at.map(|dt| dt.to_rfc3339()),
            revoked: r.revoked,
            use_count: r.use_count,
            relay_url: r.relay_url,
            scope_note_id: r.scope_note_id,
            scope_note_title: r.scope_note_title,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedRelayInvite {
    pub invite: InviteFileData,
    pub temp_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteFileData {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_description: Option<String>,
    pub workspace_author_name: Option<String>,
    pub workspace_author_org: Option<String>,
    pub workspace_homepage_url: Option<String>,
    pub workspace_license: Option<String>,
    pub workspace_language: Option<String>,
    pub workspace_tags: Vec<String>,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    pub inviter_fingerprint: String,
    pub expires_at: Option<String>,
}

/// Serialisable pending peer data returned to the frontend after parsing a response file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPeer {
    pub invite_id: String,
    pub invitee_public_key: String,
    pub invitee_declared_name: String,
    pub fingerprint: String,
}

#[tauri::command]
pub fn list_invites(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<InviteInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get(&uuid).ok_or("Identity not unlocked")?;
    let records = im.list_invites().map_err(|e| {
        log::error!("list_invites(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })?;
    Ok(records.into_iter().map(InviteInfo::from).collect())
}

#[tauri::command]
pub fn create_invite(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_name: String,
    expires_in_days: Option<u32>,
    save_path: String,
    scope_note_id: Option<String>,
) -> std::result::Result<InviteInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Get signing key + declared name from unlocked identity.
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Resolve scope note title from workspace if scope_note_id is provided.
    let scope_note_title = if let Some(ref nid) = scope_note_id {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label())
            .ok_or("No workspace for this window")?;
        Some(ws.get_note(nid).map_err(|e| e.to_string())?.title)
    } else {
        None
    };

    // Get workspace id + metadata from the current window's workspace.
    let (ws_id, ws_desc, ws_author, ws_org, ws_url, ws_license, ws_tags) = {
        let wss = state.workspaces.lock().expect("Mutex poisoned");
        let ws = wss.get(window.label()).ok_or("No workspace open")?;
        let meta = ws.get_workspace_metadata().map_err(|e| e.to_string())?;
        (
            ws.workspace_id().to_string(),
            meta.description,
            meta.author_name,
            meta.author_org,
            meta.homepage_url,
            meta.license,
            meta.tags,
        )
    };

    let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
    let (record, file) = im
        .create_invite(
            &ws_id,
            &workspace_name,
            expires_in_days,
            &signing_key,
            &declared_name,
            ws_desc,
            ws_author,
            ws_org,
            ws_url,
            ws_license,
            ws_tags,
            scope_note_id,
            scope_note_title,
            "writer".to_string(),
        )
        .map_err(|e| {
            log::error!("create_invite(identity={identity_uuid}) failed: {e}");
            e.to_string()
        })?;

    krillnotes_core::core::invite::InviteManager::save_invite_file(&file, std::path::Path::new(&save_path))
        .map_err(|e| {
            log::error!("create_invite save_file failed: {e}");
            e.to_string()
        })?;

    Ok(InviteInfo::from(record))
}

/// Re-export an existing invite as a `.swarm` file.
/// Re-builds the InviteFile from the stored record + current identity + current workspace
/// metadata, then writes it to `save_path`. This allows the user to save the file again
/// from the share step without creating a duplicate invite record.
#[tauri::command]
pub fn save_invite_file(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
    save_path: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let invite_uuid = Uuid::parse_str(&invite_id).map_err(|e| e.to_string())?;

    // Get signing key + declared name from unlocked identity.
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Look up the existing invite record.
    let record = {
        let ims = state.invite_managers.lock().expect("Mutex poisoned");
        let im = ims.get(&uuid).ok_or("Identity not unlocked")?;
        im.get_invite(invite_uuid)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Invite {} not found", invite_id))?
    };

    // Get workspace metadata from the current window's workspace.
    let (ws_desc, ws_author, ws_org, ws_url, ws_license, ws_tags) = {
        let wss = state.workspaces.lock().expect("Mutex poisoned");
        let ws = wss.get(window.label()).ok_or("No workspace open")?;
        let meta = ws.get_workspace_metadata().map_err(|e| e.to_string())?;
        (
            meta.description,
            meta.author_name,
            meta.author_org,
            meta.homepage_url,
            meta.license,
            meta.tags,
        )
    };

    // Re-build and re-sign the InviteFile from the stored record.
    let pubkey_b64 = {
        use base64::{engine::general_purpose::STANDARD, Engine};
        STANDARD.encode(signing_key.verifying_key().to_bytes())
    };
    let mut file = krillnotes_core::core::invite::InviteFile {
        file_type: "krillnotes-invite-v1".to_string(),
        invite_id: record.invite_id.to_string(),
        workspace_id: record.workspace_id.clone(),
        workspace_name: record.workspace_name.clone(),
        workspace_description: ws_desc,
        workspace_author_name: ws_author,
        workspace_author_org: ws_org,
        workspace_homepage_url: ws_url,
        workspace_license: ws_license,
        workspace_language: None,
        workspace_tags: ws_tags,
        inviter_public_key: pubkey_b64,
        inviter_declared_name: declared_name,
        expires_at: record.expires_at.map(|dt| dt.to_rfc3339()),
        scope_note_id: record.scope_note_id.clone(),
        scope_note_title: record.scope_note_title.clone(),
        offered_role: record.offered_role.clone(),
        signature: String::new(),
    };
    let payload = serde_json::to_value(&file).map_err(|e| e.to_string())?;
    file.signature = krillnotes_core::core::invite::sign_payload(&payload, &signing_key);

    krillnotes_core::core::invite::InviteManager::save_invite_file(
        &file,
        std::path::Path::new(&save_path),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn revoke_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let invite_uuid = Uuid::parse_str(&invite_id).map_err(|e| e.to_string())?;
    let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
    im.revoke_invite(invite_uuid).map_err(|e| {
        log::error!("revoke_invite(invite={invite_id}) failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn delete_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let invite_uuid = Uuid::parse_str(&invite_id).map_err(|e| e.to_string())?;
    let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
    im.delete_invite(invite_uuid).map_err(|e| {
        log::error!("delete_invite(invite={invite_id}) failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn delete_revoked_invites(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<u32, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
    im.delete_revoked_invites().map_err(|e| {
        log::error!("delete_revoked_invites(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn import_invite_response(
    state: State<'_, AppState>,
    identity_uuid: String,
    path: String,
) -> std::result::Result<PendingPeer, String> {
    use krillnotes_core::core::invite::InviteManager;
    use krillnotes_core::core::contact::generate_fingerprint;

    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let response = InviteManager::parse_and_verify_response(std::path::Path::new(&path))
        .map_err(|e| {
            log::error!("import_invite_response(identity={identity_uuid}) failed: {e}");
            e.to_string()
        })?;

    // Validate invite is still active and increment use count.
    let invite_uuid = Uuid::parse_str(&response.invite_id).map_err(|e| e.to_string())?;
    let (invite_workspace_id, invite_workspace_name, invite_scope_note_id, invite_scope_note_title) = {
        let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
        let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
        let record = im
            .get_invite(invite_uuid)
            .map_err(|e| e.to_string())?
            .ok_or("Invite not found")?;
        if record.revoked {
            return Err("Invite has been revoked".to_string());
        }
        if let Some(exp) = record.expires_at {
            if chrono::Utc::now() > exp {
                return Err("Invite has expired".to_string());
            }
        }
        im.increment_use_count(invite_uuid).map_err(|e| e.to_string())?;
        (record.workspace_id.clone(), record.workspace_name.clone(), record.scope_note_id.clone(), record.scope_note_title.clone())
    };

    let fingerprint = generate_fingerprint(&response.invitee_public_key)
        .map_err(|e| e.to_string())?;

    let pending_peer = PendingPeer {
        invite_id: response.invite_id,
        invitee_public_key: response.invitee_public_key,
        invitee_declared_name: response.invitee_declared_name,
        fingerprint,
    };

    // Create a ReceivedResponse record so the polling UI can track this response.
    {
        let mut rrm = state.received_response_managers.lock().expect("Mutex poisoned");
        if let Some(rr_mgr) = rrm.get_mut(&uuid) {
            let existing = rr_mgr
                .find_by_invite_and_invitee(invite_uuid, &pending_peer.invitee_public_key)
                .map_err(|e| e.to_string())?;
            if existing.is_none() {
                let rr = krillnotes_core::core::received_response::ReceivedResponse::new(
                    invite_uuid,
                    invite_workspace_id,
                    invite_workspace_name,
                    pending_peer.invitee_public_key.clone(),
                    pending_peer.invitee_declared_name.clone(),
                    invite_scope_note_id,
                    invite_scope_note_title,
                );
                let _ = rr_mgr.save(&rr);
            }
        }
    }

    Ok(pending_peer)
}

#[tauri::command]
pub fn import_invite(path: String) -> std::result::Result<InviteFileData, String> {
    use krillnotes_core::core::invite::InviteManager;
    use krillnotes_core::core::contact::generate_fingerprint;

    let invite = InviteManager::parse_and_verify_invite(std::path::Path::new(&path))
        .map_err(|e| {
            log::error!("import_invite failed: {e}");
            e.to_string()
        })?;

    let fingerprint = generate_fingerprint(&invite.inviter_public_key).map_err(|e| e.to_string())?;

    Ok(InviteFileData {
        invite_id: invite.invite_id,
        workspace_id: invite.workspace_id,
        workspace_name: invite.workspace_name,
        workspace_description: invite.workspace_description,
        workspace_author_name: invite.workspace_author_name,
        workspace_author_org: invite.workspace_author_org,
        workspace_homepage_url: invite.workspace_homepage_url,
        workspace_license: invite.workspace_license,
        workspace_language: invite.workspace_language,
        workspace_tags: invite.workspace_tags,
        inviter_public_key: invite.inviter_public_key,
        inviter_declared_name: invite.inviter_declared_name,
        inviter_fingerprint: fingerprint,
        expires_at: invite.expires_at,
    })
}

#[tauri::command]
pub fn respond_to_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_path: String,
    save_path: String,
) -> std::result::Result<(), String> {
    use krillnotes_core::core::invite::InviteManager;

    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    let invite = InviteManager::parse_and_verify_invite(std::path::Path::new(&invite_path))
        .map_err(|e| e.to_string())?;

    InviteManager::build_and_save_response(
        &invite,
        &signing_key,
        &declared_name,
        std::path::Path::new(&save_path),
    )
    .map_err(|e| {
        log::error!("respond_to_invite(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn accept_peer(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    invitee_public_key: String,
    declared_name: String,
    trust_level: String,
    local_name: Option<String>,
) -> std::result::Result<crate::ContactInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let trust = parse_trust_level_local(&trust_level)?;

    // Create or find existing contact by public key (handles duplicate public key per spec C5).
    let contact = {
        let cms = state.contact_managers.lock().expect("Mutex poisoned");
        let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
        let mut c = cm
            .find_or_create_by_public_key(&declared_name, &invitee_public_key, trust)
            .map_err(|e| e.to_string())?;
        if let Some(name) = local_name {
            c.local_name = Some(name);
            cm.save_contact(&c).map_err(|e| e.to_string())?;
        }
        c
    };

    // Add as pre-authorised workspace peer.
    {
        let wss = state.workspaces.lock().expect("Mutex poisoned");
        if let Some(ws) = wss.get(window.label()) {
            ws.add_contact_as_peer(&invitee_public_key)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(crate::ContactInfo::from_contact(contact))
}

fn parse_trust_level_local(s: &str) -> std::result::Result<krillnotes_core::core::contact::TrustLevel, String> {
    use krillnotes_core::core::contact::TrustLevel;
    match s {
        "Tofu" => Ok(TrustLevel::Tofu),
        "CodeVerified" => Ok(TrustLevel::CodeVerified),
        "Vouched" => Ok(TrustLevel::Vouched),
        "VerifiedInPerson" => Ok(TrustLevel::VerifiedInPerson),
        other => Err(format!("Unknown trust level: {other}")),
    }
}
