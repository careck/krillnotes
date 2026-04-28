// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;
use uuid::Uuid;

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
    mgr.list_identities().map_err(|e| {
        log::error!("list_identities failed: {e}");
        e.to_string()
    })
}

/// Resolves a public key to a display name.
/// Checks local identities first, then the contacts address book.
/// Returns a truncated fingerprint (first 8 chars) if the key is unknown but non-empty,
/// or None if the key is empty.
#[tauri::command]
pub fn resolve_identity_name(state: State<'_, AppState>, public_key: String) -> Option<String> {
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
    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let file = mgr
        .create_identity(&display_name, &passphrase)
        .map_err(|e| {
            log::error!("create_identity failed: {e}");
            e.to_string()
        })?;
    let uuid = file.identity_uuid;

    // Auto-unlock after creation
    let unlocked = mgr
        .unlock_identity(&uuid, &passphrase)
        .map_err(|e| e.to_string())?;
    let identity_dir = mgr.identity_dir(&uuid);
    drop(mgr); // Release the lock before acquiring unlocked_identities
               // Derive contacts key before consuming `unlocked` via insert
    let contacts_key = unlocked.contacts_key();
    state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .insert(uuid, unlocked);
    let contacts_dir = identity_dir.join("contacts");
    match krillnotes_core::core::contact::ContactManager::for_identity(contacts_dir, contacts_key) {
        Ok(cm) => {
            state
                .contact_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, cm);
        }
        Err(e) => {
            // Non-fatal: log but don't fail creation
            log::warn!("Failed to initialize contact manager for {uuid}: {e}");
        }
    }
    let invites_dir = identity_dir.join("invites");
    match krillnotes_core::core::invite::InviteManager::new(invites_dir) {
        Ok(im) => {
            state
                .invite_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, im);
        }
        Err(e) => {
            log::warn!("Failed to initialize invite manager for {uuid}: {e}");
        }
    }

    // Initialize per-identity RelayAccountManager (no migration needed for fresh identity)
    let relay_key = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        ids.get(&uuid).map(|u| u.relay_key())
    };
    if let Some(relay_key) = relay_key {
        let relays_dir = identity_dir.join("relays");
        match krillnotes_core::core::sync::relay::RelayAccountManager::for_identity(
            relays_dir, relay_key,
        ) {
            Ok(relay_mgr) => {
                state
                    .relay_account_managers
                    .lock()
                    .expect("Mutex poisoned")
                    .insert(uuid, relay_mgr);
            }
            Err(e) => {
                log::warn!("Failed to initialize relay account manager for {uuid}: {e}");
            }
        }
    }

    let accepted_dir = identity_dir.join("accepted_invites");
    match krillnotes_core::core::accepted_invite::AcceptedInviteManager::new(accepted_dir) {
        Ok(mgr) => {
            state
                .accepted_invite_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, mgr);
        }
        Err(e) => {
            log::warn!("Failed to initialize accepted invite manager for {uuid}: {e}");
        }
    }

    let responses_dir = identity_dir.join("invite_responses");
    match krillnotes_core::core::received_response::ReceivedResponseManager::new(responses_dir) {
        Ok(mgr) => {
            state
                .received_response_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, mgr);
        }
        Err(e) => {
            log::warn!("Failed to initialize received response manager for {uuid}: {e}");
        }
    }

    // Return the IdentityRef
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let identities = mgr.list_identities().map_err(|e| e.to_string())?;
    identities
        .into_iter()
        .find(|i| i.uuid == uuid)
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
    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let unlocked = mgr
        .unlock_identity(&uuid, &passphrase)
        .map_err(|e| match e {
            crate::KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => {
                log::error!("unlock_identity(identity={identity_uuid}) failed: {other}");
                other.to_string()
            }
        })?;
    let identity_dir = mgr.identity_dir(&uuid);
    drop(mgr);
    // Derive contacts key before consuming `unlocked` via insert
    let contacts_key = unlocked.contacts_key();
    state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .insert(uuid, unlocked);
    // Create per-identity ContactManager (decrypts contacts into memory)
    let contacts_dir = identity_dir.join("contacts");
    match krillnotes_core::core::contact::ContactManager::for_identity(contacts_dir, contacts_key) {
        Ok(cm) => {
            state
                .contact_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, cm);
        }
        Err(e) => {
            // Non-fatal: log but don't fail unlock
            log::warn!("Failed to initialize contact manager for {uuid}: {e}");
        }
    }
    let invites_dir = identity_dir.join("invites");
    match krillnotes_core::core::invite::InviteManager::new(invites_dir) {
        Ok(im) => {
            state
                .invite_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, im);
        }
        Err(e) => {
            log::warn!("Failed to initialize invite manager for {uuid}: {e}");
        }
    }

    // Initialize per-identity RelayAccountManager (encrypted relay accounts)
    let relay_key = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        ids.get(&uuid).map(|u| u.relay_key())
    };
    if let Some(relay_key) = relay_key {
        let relays_dir = identity_dir.join("relays");
        match krillnotes_core::core::sync::relay::RelayAccountManager::for_identity(
            relays_dir, relay_key,
        ) {
            Ok(relay_mgr) => {
                state
                    .relay_account_managers
                    .lock()
                    .expect("Mutex poisoned")
                    .insert(uuid, relay_mgr);
            }
            Err(e) => {
                log::warn!("Failed to initialize relay account manager for {uuid}: {e}");
            }
        }
    }

    // ── Auto-refresh stale relay device keys ────────────────────────────
    // When a .swarmid is imported to a new device, the relay accounts carry
    // the old device's per-device key. Detect this and re-login to register
    // the current device's key with the relay.
    let current_dpk = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        if let (Some(id), Ok(device_id)) = (
            ids.get(&uuid),
            krillnotes_core::core::device::get_device_id(),
        ) {
            let device_sk = id.device_signing_key(&device_id);
            let dpk_hex = hex::encode(device_sk.verifying_key().to_bytes());
            let composite = format!("{}:identity:{}", device_id, uuid);
            let identity_sk = crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes());
            let identity_pk = hex::encode(identity_sk.verifying_key().to_bytes());
            Some((device_sk, dpk_hex, composite, identity_sk, identity_pk))
        } else {
            None
        }
    };

    if let Some((
        device_sk,
        current_dpk_hex,
        composite_device_id,
        identity_signing_key,
        identity_pubkey_hex,
    )) = current_dpk
    {
        let stale_accounts = {
            let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
            managers
                .get(&uuid)
                .and_then(|mgr| mgr.list_relay_accounts().ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|a| a.device_public_key != current_dpk_hex)
                .collect::<Vec<_>>()
        };
        // All locks released here — safe to do blocking network I/O.

        for mut account in stale_accounts {
            log::info!(
                "Relay account {} has stale device key — re-authenticating",
                account.relay_url
            );
            let client = krillnotes_core::core::sync::relay::RelayClient::new(&account.relay_url);
            match client.login(&account.email, &account.password, &current_dpk_hex) {
                Ok(session) => {
                    if let Some(challenge) = &session.challenge {
                        match krillnotes_core::core::sync::relay::auth::decrypt_pop_challenge(
                            &device_sk,
                            &challenge.encrypted_nonce,
                            &challenge.server_public_key,
                        ) {
                            Ok(nonce_bytes) => {
                                let nonce_hex = hex::encode(&nonce_bytes);
                                let mut authed =
                                    krillnotes_core::core::sync::relay::RelayClient::new(
                                        &account.relay_url,
                                    );
                                authed.set_session_token(&session.session_token);
                                if let Err(e) = authed.verify_device(
                                    &current_dpk_hex,
                                    &nonce_hex,
                                    Some(&composite_device_id),
                                ) {
                                    log::warn!(
                                        "Device verify failed for {}: {e}",
                                        account.relay_url
                                    );
                                } else {
                                    log::info!("Device verified on {}", account.relay_url);
                                }
                            }
                            Err(e) => {
                                log::warn!("PoP decryption failed for {}: {e}", account.relay_url)
                            }
                        }
                    }
                    // Also register the identity's main public key for peer routing (best-effort).
                    if identity_pubkey_hex != current_dpk_hex {
                        let mut id_client = krillnotes_core::core::sync::relay::RelayClient::new(
                            &account.relay_url,
                        );
                        id_client.set_session_token(&session.session_token);
                        if let Ok(result) = id_client.add_device(&identity_pubkey_hex) {
                            if let Ok(nonce) =
                                krillnotes_core::core::sync::relay::auth::decrypt_pop_challenge(
                                    &identity_signing_key,
                                    &result.challenge.encrypted_nonce,
                                    &result.challenge.server_public_key,
                                )
                            {
                                let _ = id_client.verify_device(
                                    &identity_pubkey_hex,
                                    &hex::encode(&nonce),
                                    None,
                                );
                                log::info!(
                                    "Registered identity public key on {}",
                                    account.relay_url
                                );
                            }
                        } // Err: 409 KEY_EXISTS expected
                    }

                    // Update account with new session + device key
                    account.session_token = session.session_token;
                    account.session_expires_at = chrono::Utc::now() + chrono::Duration::days(30);
                    account.device_public_key = current_dpk_hex.clone();
                    let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
                    if let Some(mgr) = managers.get(&uuid) {
                        if let Err(e) = mgr.save_relay_account(&account) {
                            log::warn!("Failed to save refreshed relay account: {e}");
                        }
                    }
                }
                Err(e) => log::warn!("Relay re-login failed for {}: {e}", account.relay_url),
            }
        }
    }

    let accepted_dir = identity_dir.join("accepted_invites");
    match krillnotes_core::core::accepted_invite::AcceptedInviteManager::new(accepted_dir) {
        Ok(mgr) => {
            state
                .accepted_invite_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, mgr);
        }
        Err(e) => {
            log::warn!("Failed to initialize accepted invite manager for {uuid}: {e}");
        }
    }

    let responses_dir = identity_dir.join("invite_responses");
    match krillnotes_core::core::received_response::ReceivedResponseManager::new(responses_dir) {
        Ok(mgr) => {
            state
                .received_response_managers
                .lock()
                .expect("Mutex poisoned")
                .insert(uuid, mgr);
        }
        Err(e) => {
            log::warn!("Failed to initialize received response manager for {uuid}: {e}");
        }
    }

    // Fire-and-forget: auto-login expired relay sessions on a background thread.
    // Uses std::thread::spawn (not tokio::task::spawn) because sync Tauri commands
    // may not have a Tokio runtime context. RelayClient uses reqwest::blocking which
    // manages its own internal runtime, so no Tokio runtime is needed here.
    let ram_clone = state.relay_account_managers.clone();
    let uuid_clone = uuid;
    std::thread::spawn(move || {
        let accounts = {
            let mgrs = match ram_clone.lock() {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("relay_account_managers mutex poisoned: {e}");
                    return;
                }
            };
            match mgrs.get(&uuid_clone) {
                Some(mgr) => mgr.list_relay_accounts().unwrap_or_default(),
                None => return,
            }
        };

        for acct in accounts {
            if acct.session_expires_at > chrono::Utc::now() || acct.password.is_empty() {
                continue; // session still valid or no password stored
            }

            let url = acct.relay_url.clone();
            let client = krillnotes_core::core::sync::relay::RelayClient::new(&url);
            let result = client.login(&acct.email, &acct.password, &acct.device_public_key);

            let mgrs = match ram_clone.lock() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if let Some(mgr) = mgrs.get(&uuid_clone) {
                if let Ok(Some(mut updated)) = mgr.get_relay_account(acct.relay_account_id) {
                    match result {
                        Ok(session) => {
                            updated.session_token = session.session_token;
                            updated.session_expires_at =
                                chrono::Utc::now() + chrono::Duration::days(30);
                            let _ = mgr.save_relay_account(&updated);
                            log::info!("Auto-login succeeded for relay {url}");
                        }
                        Err(e) => {
                            // Auth failure: mark session as invalid
                            updated.session_expires_at = chrono::DateTime::<chrono::Utc>::MIN_UTC;
                            let _ = mgr.save_relay_account(&updated);
                            log::warn!("Auto-login failed for relay {url}: {e}");
                        }
                    }
                }
            }
        }
    });

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
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let bound_folders: std::collections::HashSet<PathBuf> = mgr
        .get_workspaces_for_identity(&uuid)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(folder, _)| folder)
        .collect();
    drop(mgr);

    let labels_to_close: Vec<String> = state
        .workspace_paths
        .lock()
        .expect("Mutex poisoned")
        .iter()
        .filter(|(_, path)| bound_folders.contains(*path))
        .map(|(label, _)| label.clone())
        .collect();

    use tauri::Manager;
    for label in &labels_to_close {
        state
            .closing_windows
            .lock()
            .expect("Mutex poisoned")
            .insert(label.clone());
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.destroy();
        }
        state
            .workspaces
            .lock()
            .expect("Mutex poisoned")
            .remove(label);
        state
            .workspace_paths
            .lock()
            .expect("Mutex poisoned")
            .remove(label);
        state
            .workspace_identities
            .lock()
            .expect("Mutex poisoned")
            .remove(label);
    }

    // Wipe identity from memory.
    // Remove per-identity managers first so there is no window where
    // the identity is "locked" but its managers are still live.
    state
        .contact_managers
        .lock()
        .expect("Mutex poisoned")
        .remove(&uuid);
    state
        .invite_managers
        .lock()
        .expect("Mutex poisoned")
        .remove(&uuid);
    state
        .relay_account_managers
        .lock()
        .expect("Mutex poisoned")
        .remove(&uuid);
    state
        .accepted_invite_managers
        .lock()
        .expect("Mutex poisoned")
        .remove(&uuid);
    state
        .received_response_managers
        .lock()
        .expect("Mutex poisoned")
        .remove(&uuid);
    state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .remove(&uuid);
    Ok(())
}

/// Deletes an identity. The identity must be locked first.
#[tauri::command]
pub fn delete_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Must be unlocked (proves ownership via passphrase)
    let is_unlocked = state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .contains_key(&uuid);
    if !is_unlocked {
        return Err(format!("IDENTITY_LOCKED:{}", identity_uuid));
    }

    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.delete_identity(&uuid).map_err(|e| {
        log::error!("delete_identity(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })
}

/// Renames an identity.
#[tauri::command]
pub fn rename_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
    new_name: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Must be unlocked
    let is_unlocked = state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .contains_key(&uuid);
    if !is_unlocked {
        return Err(format!("IDENTITY_LOCKED:{}", identity_uuid));
    }

    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.rename_identity(&uuid, &new_name).map_err(|e| {
        log::error!("rename_identity(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })
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

    // Must be unlocked
    let is_unlocked = state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .contains_key(&uuid);
    if !is_unlocked {
        return Err(format!("IDENTITY_LOCKED:{}", identity_uuid));
    }

    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.change_passphrase(&uuid, &old_passphrase, &new_passphrase)
        .map_err(|e| match e {
            crate::KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => {
                log::error!("change_identity_passphrase(identity={identity_uuid}) failed: {other}");
                other.to_string()
            }
        })
}

/// Returns the UUIDs of all currently unlocked identities.
#[tauri::command]
pub fn get_unlocked_identities(state: State<'_, AppState>) -> Vec<String> {
    state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .keys()
        .map(|uuid| uuid.to_string())
        .collect()
}

/// Returns true if the given identity is currently unlocked.
#[tauri::command]
pub fn is_identity_unlocked(state: State<'_, AppState>, identity_uuid: String) -> bool {
    Uuid::parse_str(&identity_uuid)
        .map(|uuid| {
            state
                .unlocked_identities
                .lock()
                .expect("Mutex poisoned")
                .contains_key(&uuid)
        })
        .unwrap_or(false)
}

/// Returns the workspaces bound to the given identity.
#[tauri::command]
pub fn get_workspaces_for_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<WorkspaceBindingInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let bindings = mgr
        .get_workspaces_for_identity(&uuid)
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
/// Identity must already be unlocked (ownership proven via passphrase at unlock time).
#[tauri::command]
pub fn export_swarmid_cmd(
    state: State<'_, AppState>,
    identity_uuid: String,
    path: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Must be unlocked (proves ownership)
    let is_unlocked = state
        .unlocked_identities
        .lock()
        .expect("Mutex poisoned")
        .contains_key(&uuid);
    if !is_unlocked {
        return Err(format!("IDENTITY_LOCKED:{}", identity_uuid));
    }

    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let swarmid = mgr
        .export_swarmid_no_verify(&uuid)
        .map_err(|e| e.to_string())?;
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
    Ok(IdentityKeyInfo {
        public_key: file.public_key,
        fingerprint,
    })
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
    let file: crate::SwarmIdFile =
        serde_json::from_str(&data).map_err(|e| format!("Invalid .swarmid file: {e}"))?;
    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.import_swarmid(file).map_err(|e| match e {
        crate::KrillnotesError::IdentityAlreadyExists(uuid) => format!("IDENTITY_EXISTS:{uuid}"),
        other => {
            log::error!("import_swarmid failed: {other}");
            other.to_string()
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
    let file: crate::SwarmIdFile =
        serde_json::from_str(&data).map_err(|e| format!("Invalid .swarmid file: {e}"))?;
    let mut mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.import_swarmid_overwrite(file).map_err(|e| {
        log::error!("import_swarmid_overwrite failed: {e}");
        e.to_string()
    })
}
