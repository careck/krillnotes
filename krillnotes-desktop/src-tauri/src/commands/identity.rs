// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::path::PathBuf;

/// Information about a workspace bound to an identity, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBindingInfo {
    pub workspace_uuid: String,
    pub folder_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityKeyInfo {
    pub public_key: String,
    pub fingerprint: String,
}

// ── Identity commands ─────────────────────────────────────────────

/// Lists all registered identities.
#[tauri::command]
pub fn list_identities(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<crate::IdentityRef>, String> {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.list_identities().map_err(|e| e.to_string())
}

/// Resolves a public key to a display name.
/// Checks local identities first, then the contacts address book.
/// Returns a truncated fingerprint (first 8 chars) if the key is unknown but non-empty,
/// or None if the key is empty.
#[tauri::command]
pub fn resolve_identity_name(
    state: State<'_, AppState>,
    public_key: String,
) -> Option<String> {
    if public_key.is_empty() {
        return None;
    }
    // 1. Local identity (keys owned by this device)
    let identity_mgr = state.identity_manager.lock().expect("Mutex poisoned");
    if let Some(name) = identity_mgr.lookup_display_name(&public_key) {
        return Some(name);
    }
    drop(identity_mgr);
    // 2. Contacts address book (remote peers) — search all unlocked identity managers
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    for cm in cms.values() {
        if let Ok(Some(contact)) = cm.find_by_public_key(&public_key) {
            return Some(contact.display_name().to_string());
        }
    }
    drop(cms);
    // 3. Unknown key — show a short fingerprint so it's not blank
    Some(public_key.chars().take(8).collect())
}

/// Creates a new identity and auto-unlocks it in memory.
#[tauri::command]
pub fn create_identity(
    state: State<'_, AppState>,
    display_name: String,
    passphrase: String,
) -> std::result::Result<crate::IdentityRef, String> {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let file = mgr.create_identity(&display_name, &passphrase)
        .map_err(|e| e.to_string())?;
    let uuid = file.identity_uuid;

    // Auto-unlock after creation
    let unlocked = mgr.unlock_identity(&uuid, &passphrase)
        .map_err(|e| e.to_string())?;
    drop(mgr); // Release the lock before acquiring unlocked_identities
    // Derive contacts key before consuming `unlocked` via insert
    let contacts_key = unlocked.contacts_key();
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .insert(uuid, unlocked);
    let contacts_dir = crate::settings::config_dir()
        .join("identities")
        .join(uuid.to_string())
        .join("contacts");
    match krillnotes_core::core::contact::ContactManager::for_identity(contacts_dir, contacts_key) {
        Ok(cm) => {
            state.contact_managers.lock().expect("Mutex poisoned").insert(uuid, cm);
        }
        Err(e) => {
            // Non-fatal: log but don't fail creation
            eprintln!("Warning: failed to initialize contact manager for {uuid}: {e}");
        }
    }
    let invites_dir = crate::settings::config_dir()
        .join("identities")
        .join(uuid.to_string())
        .join("invites");
    match krillnotes_core::core::invite::InviteManager::new(invites_dir) {
        Ok(im) => { state.invite_managers.lock().expect("Mutex poisoned").insert(uuid, im); }
        Err(e) => { eprintln!("Warning: failed to initialize invite manager for {uuid}: {e}"); }
    }

    // Return the IdentityRef
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let identities = mgr.list_identities().map_err(|e| e.to_string())?;
    identities.into_iter().find(|i| i.uuid == uuid)
        .ok_or_else(|| "Identity created but not found in registry".to_string())
}

/// Unlocks an identity and stores the unlocked state in memory.
#[tauri::command]
pub fn unlock_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
    passphrase: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let unlocked = mgr.unlock_identity(&uuid, &passphrase)
        .map_err(|e| match e {
            crate::KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        })?;
    drop(mgr);
    // Derive contacts key before consuming `unlocked` via insert
    let contacts_key = unlocked.contacts_key();
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .insert(uuid, unlocked);
    // Create per-identity ContactManager (decrypts contacts into memory)
    let contacts_dir = crate::settings::config_dir()
        .join("identities")
        .join(uuid.to_string())
        .join("contacts");
    match krillnotes_core::core::contact::ContactManager::for_identity(contacts_dir, contacts_key) {
        Ok(cm) => {
            state.contact_managers.lock().expect("Mutex poisoned").insert(uuid, cm);
        }
        Err(e) => {
            // Non-fatal: log but don't fail unlock
            eprintln!("Warning: failed to initialize contact manager for {uuid}: {e}");
        }
    }
    let invites_dir = crate::settings::config_dir()
        .join("identities")
        .join(uuid.to_string())
        .join("invites");
    match krillnotes_core::core::invite::InviteManager::new(invites_dir) {
        Ok(im) => { state.invite_managers.lock().expect("Mutex poisoned").insert(uuid, im); }
        Err(e) => { eprintln!("Warning: failed to initialize invite manager for {uuid}: {e}"); }
    }
    Ok(())
}

/// Locks an identity: closes all its workspace windows and wipes it from memory.
#[tauri::command]
pub fn lock_identity(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Find and close all workspace windows belonging to this identity
    let workspace_base_dir = PathBuf::from(&crate::settings::load_settings().workspace_directory);
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let bound_folders: std::collections::HashSet<PathBuf> =
        mgr.get_workspaces_for_identity(&uuid, &workspace_base_dir)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(folder, _)| folder)
            .collect();
    drop(mgr);

    let labels_to_close: Vec<String> = state.workspace_paths.lock()
        .expect("Mutex poisoned")
        .iter()
        .filter(|(_, path)| bound_folders.contains(*path))
        .map(|(label, _)| label.clone())
        .collect();

    use tauri::Manager;
    for label in &labels_to_close {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.close();
        }
    }

    // Wipe identity from memory.
    // Remove contact_managers and invite_managers first so there is no window where
    // the identity is "locked" but its managers are still live.
    state.contact_managers.lock().expect("Mutex poisoned").remove(&uuid);
    state.invite_managers.lock().expect("Mutex poisoned").remove(&uuid);
    state.unlocked_identities.lock().expect("Mutex poisoned").remove(&uuid);
    Ok(())
}

/// Deletes an identity. The identity must be locked first.
#[tauri::command]
pub fn delete_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Must be locked first
    let is_unlocked = state.unlocked_identities.lock().expect("Mutex poisoned").contains_key(&uuid);
    if is_unlocked {
        return Err("Lock the identity before deleting it".to_string());
    }

    let workspace_base_dir = PathBuf::from(&crate::settings::load_settings().workspace_directory);
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.delete_identity(&uuid, &workspace_base_dir).map_err(|e| e.to_string())
}

/// Renames an identity.
#[tauri::command]
pub fn rename_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
    new_name: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.rename_identity(&uuid, &new_name).map_err(|e| e.to_string())
}

/// Changes an identity's passphrase.
#[tauri::command]
pub fn change_identity_passphrase(
    state: State<'_, AppState>,
    identity_uuid: String,
    old_passphrase: String,
    new_passphrase: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.change_passphrase(&uuid, &old_passphrase, &new_passphrase)
        .map_err(|e| match e {
            crate::KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        })
}

/// Returns the UUIDs of all currently unlocked identities.
#[tauri::command]
pub fn get_unlocked_identities(
    state: State<'_, AppState>,
) -> Vec<String> {
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .keys()
        .map(|uuid| uuid.to_string())
        .collect()
}

/// Returns true if the given identity is currently unlocked.
#[tauri::command]
pub fn is_identity_unlocked(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> bool {
    Uuid::parse_str(&identity_uuid)
        .map(|uuid| state.unlocked_identities.lock().expect("Mutex poisoned").contains_key(&uuid))
        .unwrap_or(false)
}

/// Returns the workspaces bound to the given identity.
#[tauri::command]
pub fn get_workspaces_for_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<WorkspaceBindingInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let workspace_base_dir = PathBuf::from(&crate::settings::load_settings().workspace_directory);
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let bindings = mgr
        .get_workspaces_for_identity(&uuid, &workspace_base_dir)
        .map_err(|e| e.to_string())?;
    let result: Vec<WorkspaceBindingInfo> = bindings
        .into_iter()
        .map(|(folder, binding)| WorkspaceBindingInfo {
            workspace_uuid: binding.workspace_uuid,
            folder_path: folder.display().to_string(),
        })
        .collect();
    Ok(result)
}

/// Export an identity to a `.swarmid` file at the given path.
/// Verifies the passphrase before writing.
/// Returns `"WRONG_PASSPHRASE"` on passphrase mismatch.
#[tauri::command]
pub fn export_swarmid_cmd(
    state: State<'_, AppState>,
    identity_uuid: String,
    passphrase: String,
    path: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let swarmid = mgr.export_swarmid(&uuid, &passphrase).map_err(|e| {
        match e {
            crate::KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        }
    })?;
    let json = serde_json::to_string_pretty(&swarmid).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Return the Base64-encoded Ed25519 public key and 4-word fingerprint for the given identity.
/// No passphrase required — the public key is stored unencrypted on disk.
#[tauri::command]
pub fn get_identity_public_key(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<IdentityKeyInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let identities = mgr.list_identities().map_err(|e| e.to_string())?;
    let identity_ref = identities
        .into_iter()
        .find(|i| i.uuid == uuid)
        .ok_or("Identity not found")?;
    let full_path = mgr.identity_file_path(&identity_ref.uuid);
    let data = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("Cannot read identity file: {e}"))?;
    let file: krillnotes_core::core::identity::IdentityFile =
        serde_json::from_str(&data).map_err(|e| format!("Invalid identity file: {e}"))?;
    let fingerprint = krillnotes_core::core::contact::generate_fingerprint(&file.public_key)
        .map_err(|e| format!("Cannot generate fingerprint: {e}"))?;
    Ok(IdentityKeyInfo { public_key: file.public_key, fingerprint })
}

/// Import a `.swarmid` file from the given path.
/// Returns the `IdentityRef` on success.
/// Returns `"IDENTITY_EXISTS:<uuid>"` if the same UUID is already registered —
/// frontend should confirm overwrite then call `import_swarmid_overwrite_cmd`.
#[tauri::command]
pub fn import_swarmid_cmd(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<crate::IdentityRef, String> {
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file: crate::SwarmIdFile = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid .swarmid file: {e}"))?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.import_swarmid(file).map_err(|e| {
        match e {
            crate::KrillnotesError::IdentityAlreadyExists(uuid) => format!("IDENTITY_EXISTS:{uuid}"),
            other => other.to_string(),
        }
    })
}

/// Import a `.swarmid` file, overwriting any existing identity with the same UUID.
#[tauri::command]
pub fn import_swarmid_overwrite_cmd(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<crate::IdentityRef, String> {
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file: crate::SwarmIdFile = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid .swarmid file: {e}"))?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.import_swarmid_overwrite(file).map_err(|e| e.to_string())
}
