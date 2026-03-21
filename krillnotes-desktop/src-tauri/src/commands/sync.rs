// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Tauri commands for relay-based sync operations.

use crate::AppState;
use std::sync::Arc;
use tauri::{Emitter, State, Window};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use uuid::Uuid;
use krillnotes_core::core::{
    device::get_device_id,
    sync::{FolderChannel, SyncContext, SyncEngine, SyncEvent},
};
use krillnotes_core::core::sync::relay::{RelayAccount, RelayChannel, RelayClient};

// ── update_peer_channel ────────────────────────────────────────────────────

/// Update a peer's channel configuration in the workspace database.
///
/// `channel_type` is a plain string identifier (e.g. `"relay"`, `"folder"`).
/// `channel_params` is a JSON string containing channel-specific parameters.
#[tauri::command]
pub async fn update_peer_channel(
    window: Window,
    state: State<'_, AppState>,
    peer_device_id: String,
    channel_type: String,
    channel_params: String,
) -> Result<(), String> {
    log::debug!("update_peer_channel(peer={peer_device_id}, type={channel_type})");
    let workspace_label = window.label().to_string();
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces
        .get(&workspace_label)
        .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?;
    ws.update_peer_channel(&peer_device_id, &channel_type, &channel_params)
        .map_err(|e| {
            log::error!("update_peer_channel(peer={peer_device_id}, type={channel_type}) failed: {e}");
            e.to_string()
        })
}

// ── poll_sync ──────────────────────────────────────────────────────────────

/// Run one sync poll cycle for the current workspace window.
///
/// Builds a fresh `SyncEngine` with the `FolderChannel` registered, runs one
/// `poll()` cycle, and returns the resulting `SyncEvent` list as JSON.
#[tauri::command]
pub async fn poll_sync(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Vec<SyncEvent>, String> {
    log::debug!("poll_sync(window={})", window.label());
    let workspace_label = window.label().to_string();

    // -- Collect context data under brief locks (all guards released before spawn) --
    let identity_uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };

    let (signing_key, sender_display_name, identity_pubkey) = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&identity_uuid).ok_or("Identity not unlocked")?;
        let pubkey_b64 = BASE64.encode(id.verifying_key.as_bytes()); // FolderChannel uses Base64
        (id.signing_key.clone(), id.display_name.clone(), pubkey_b64)
    };

    let workspace_name = {
        let m = state.workspace_paths.lock().map_err(|e| e.to_string())?;
        m.get(&workspace_label)
            .and_then(|p| p.file_stem().and_then(|n| n.to_str()).map(String::from))
            .unwrap_or_else(|| workspace_label.clone())
    };

    let device_id = get_device_id().map_err(|e| e.to_string())?;

    // Load all relay accounts from RelayAccountManager (clone before spawn_blocking)
    let relay_accounts: Vec<RelayAccount> = {
        let ram = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
        if let Some(mgr) = ram.get(&identity_uuid) {
            mgr.list_relay_accounts().unwrap_or_default()
        } else {
            vec![]
        }
    };

    let workspace_id_str = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        workspaces
            .get(&workspace_label)
            .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?
            .workspace_id()
            .to_string()
    };

    // Migrate old-format channel_params for relay peers.
    // Old format: {"relay_url": "..."} → New format: {"relay_account_id": "<uuid>"}
    {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        if let Some(ws) = workspaces.get(&workspace_label) {
            if let Ok(relay_peers) = ws.list_peers_with_channel("relay") {
                for peer in relay_peers {
                    // Skip already-migrated peers
                    if peer.channel_params.contains("relay_account_id") {
                        continue;
                    }
                    // Try to parse old format with relay_url
                    if let Ok(params) = serde_json::from_str::<serde_json::Value>(&peer.channel_params) {
                        if let Some(url) = params.get("relay_url").and_then(|v| v.as_str()) {
                            // Look up matching relay account by URL
                            let matched = relay_accounts.iter().find(|a| a.relay_url == url);
                            match matched {
                                Some(acct) => {
                                    let new_params = serde_json::json!({
                                        "relay_account_id": acct.relay_account_id.to_string()
                                    });
                                    if let Err(e) = ws.update_peer_channel(
                                        &peer.peer_device_id,
                                        "relay",
                                        &new_params.to_string(),
                                    ) {
                                        log::warn!("Failed to migrate channel_params for peer {}: {e}", peer.peer_device_id);
                                    } else {
                                        log::info!("Migrated channel_params for peer {} to relay_account_id {}", peer.peer_device_id, acct.relay_account_id);
                                    }
                                }
                                None => {
                                    // No matching relay account — set to manual
                                    log::warn!("No relay account found for URL {url}, setting peer {} to manual", peer.peer_device_id);
                                    let _ = ws.update_peer_channel(
                                        &peer.peer_device_id,
                                        "manual",
                                        "{}",
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Clone Arcs so the spawn_blocking closure can own them (guards are NOT held).
    // RelayChannel holds a reqwest::blocking::Client which owns an internal Tokio
    // runtime. Creating, using, and dropping it must happen on a spawn_blocking
    // thread (no outer tokio context) — block_in_place is insufficient because the
    // outer runtime context is still present on the thread, causing a panic on drop.
    let workspaces_arc = Arc::clone(&state.workspaces);
    let contact_managers_arc = Arc::clone(&state.contact_managers);

    let events = tokio::task::spawn_blocking(move || -> Result<Vec<SyncEvent>, String> {
        let mut engine = SyncEngine::new();
        engine.register_channel(Box::new(FolderChannel::new(identity_pubkey, device_id)));

        // NOTE: SyncEngine supports one channel per ChannelType (HashMap keyed by type).
        // Multiple relay accounts would overwrite each other. For now, register only the
        // first relay account. Multi-relay support requires SyncEngine architecture changes.
        if let Some(acct) = relay_accounts.first() {
            let mut token = acct.session_token.clone();
            // Auto-login if session expired and password stored
            if acct.session_expires_at < chrono::Utc::now() && !acct.password.is_empty() {
                let client = RelayClient::new(&acct.relay_url);
                match client.login(&acct.email, &acct.password, &acct.device_public_key) {
                    Ok(session) => token = session.session_token,
                    Err(e) => log::warn!("poll_sync: inline auto-login failed for {}: {e}", acct.relay_url),
                }
            }
            let relay_client = RelayClient::new(&acct.relay_url)
                .with_session_token(&token);
            engine.register_channel(Box::new(RelayChannel::new(
                relay_client,
                workspace_id_str.clone(),
                acct.device_public_key.clone(),
            )));
        }

        let mut contact_managers = contact_managers_arc.lock().map_err(|e| e.to_string())?;
        let contact_manager = contact_managers
            .get_mut(&identity_uuid)
            .ok_or("Contact manager not found — is the identity unlocked?")?;

        let mut workspaces = workspaces_arc.lock().map_err(|e| e.to_string())?;
        let workspace = workspaces
            .get_mut(&workspace_label)
            .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?;

        let mut ctx = SyncContext {
            signing_key: &signing_key,
            contact_manager,
            workspace_name: &workspace_name,
            sender_display_name: &sender_display_name,
        };

        engine.poll(workspace, &mut ctx).map_err(|e| {
            log::error!("poll_sync(window={workspace_label}) failed: {e}");
            e.to_string()
        })
        // engine (and RelayClient) dropped here — safe on a spawn_blocking thread
    })
    .await
    .map_err(|e| {
        log::error!("poll_sync spawn_blocking join failed: {e}");
        e.to_string()
    })??;

    // If any bundles were applied, notify WorkspaceView to reload the note tree.
    let bundles_applied = events.iter().any(|e| matches!(e, SyncEvent::BundleApplied { .. }));
    if bundles_applied {
        let _ = window.emit("workspace-updated", ());
    }

    Ok(events)
}

// ── share_invite_link ──────────────────────────────────────────────────────

/// One-click command: create an invite + upload it to the relay + return the
/// shareable URL. The invite record is persisted with `relay_url` set.
#[tauri::command]
pub async fn share_invite_link(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_name: String,
    expires_in_days: Option<u32>,
) -> Result<crate::commands::invites::InviteInfo, String> {
    log::debug!("share_invite_link(identity={identity_uuid})");
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Get signing key + declared name.
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Get workspace metadata from the current window's workspace.
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

    // Create the invite record + InviteFile.
    let (record, file) = {
        let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
        let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
        im.create_invite(
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
        )
        .map_err(|e| {
            log::error!("share_invite_link create_invite failed: {e}");
            e.to_string()
        })?
    };

    // Serialize + base64-encode the invite file.
    let bytes = krillnotes_core::core::invite::InviteManager::serialize_invite_to_bytes(&file)
        .map_err(|e| e.to_string())?;
    let payload_b64 = BASE64.encode(&bytes);

    // Compute expiry timestamp.
    let expires_at = {
        let days = expires_in_days.unwrap_or(7) as i64;
        (chrono::Utc::now() + chrono::Duration::days(days)).to_rfc3339()
    };

    // Build relay client with auto-login.
    let invite_id = record.invite_id;
    let relay_account = {
        let ram = state.relay_account_managers.lock().expect("Mutex poisoned");
        let mgr = ram.get(&uuid).ok_or("No relay account manager for identity")?;
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        accounts.into_iter().next().ok_or("No relay account configured")?
    };

    let relay_url_result = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut token = relay_account.session_token.clone();
        if relay_account.session_expires_at < chrono::Utc::now() && !relay_account.password.is_empty() {
            let client = RelayClient::new(&relay_account.relay_url);
            match client.login(&relay_account.email, &relay_account.password, &relay_account.device_public_key) {
                Ok(session) => token = session.session_token,
                Err(e) => log::warn!("share_invite_link: auto-login failed: {e}"),
            }
        }
        let client = RelayClient::new(&relay_account.relay_url).with_session_token(&token);
        let info = client.create_invite(&payload_b64, &expires_at).map_err(|e| {
            log::error!("share_invite_link: relay create_invite failed: {e}");
            e.to_string()
        })?;
        Ok(info.url)
    })
    .await
    .map_err(|e| e.to_string())??;

    // Persist the relay URL on the invite record.
    {
        let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
        let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
        im.set_relay_url(invite_id, relay_url_result.clone())
            .map_err(|e| e.to_string())?;
    }

    // Return updated InviteInfo with relay_url populated.
    let updated_record = {
        let ims = state.invite_managers.lock().expect("Mutex poisoned");
        let im = ims.get(&uuid).ok_or("Identity not unlocked")?;
        im.get_invite(invite_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Invite not found after creation".to_string())?
    };

    Ok(crate::commands::invites::InviteInfo::from(updated_record))
}

// ── create_relay_invite ────────────────────────────────────────────────────

/// Upload an already-created invite to the relay and return the shareable URL.
/// Persists the relay URL in the invite record.
#[tauri::command]
pub async fn create_relay_invite(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> Result<String, String> {
    log::debug!("create_relay_invite(identity={identity_uuid}, invite={invite_id})");
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let invite_uuid = Uuid::parse_str(&invite_id).map_err(|e| e.to_string())?;

    // Get signing key + declared name.
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
            .ok_or_else(|| format!("Invite {invite_id} not found"))?
    };

    if record.revoked {
        return Err("Invite has been revoked".to_string());
    }

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
        BASE64.encode(signing_key.verifying_key().to_bytes())
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
        signature: String::new(),
    };
    let payload = serde_json::to_value(&file).map_err(|e| e.to_string())?;
    file.signature = krillnotes_core::core::invite::sign_payload(&payload, &signing_key);

    // Serialize + base64-encode.
    let bytes = krillnotes_core::core::invite::InviteManager::serialize_invite_to_bytes(&file)
        .map_err(|e| e.to_string())?;
    let payload_b64 = BASE64.encode(&bytes);

    // Compute expiry timestamp (use record's expiry or default 7 days from now).
    let expires_at = record
        .expires_at
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339());

    // Build relay client with auto-login.
    let relay_account = {
        let ram = state.relay_account_managers.lock().expect("Mutex poisoned");
        let mgr = ram.get(&uuid).ok_or("No relay account manager for identity")?;
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        accounts.into_iter().next().ok_or("No relay account configured")?
    };

    let relay_url_result = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut token = relay_account.session_token.clone();
        if relay_account.session_expires_at < chrono::Utc::now() && !relay_account.password.is_empty() {
            let client = RelayClient::new(&relay_account.relay_url);
            match client.login(&relay_account.email, &relay_account.password, &relay_account.device_public_key) {
                Ok(session) => token = session.session_token,
                Err(e) => log::warn!("create_relay_invite: auto-login failed: {e}"),
            }
        }
        let client = RelayClient::new(&relay_account.relay_url).with_session_token(&token);
        let info = client.create_invite(&payload_b64, &expires_at).map_err(|e| {
            log::error!("create_relay_invite: relay create_invite failed: {e}");
            e.to_string()
        })?;
        Ok(info.url)
    })
    .await
    .map_err(|e| e.to_string())??;

    // Persist the relay URL on the invite record.
    {
        let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
        let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
        im.set_relay_url(invite_uuid, relay_url_result.clone())
            .map_err(|e| e.to_string())?;
    }

    Ok(relay_url_result)
}

// ── fetch_relay_invite ─────────────────────────────────────────────────────

/// Fetch an invite from the relay by token. Downloads, verifies, writes to a
/// temp file, and returns the invite data along with the temp file path.
#[tauri::command]
pub async fn fetch_relay_invite(
    _window: Window,
    _state: State<'_, AppState>,
    token: String,
    relay_base_url: Option<String>,
) -> Result<crate::commands::invites::FetchedRelayInvite, String> {
    log::debug!("fetch_relay_invite(token={token})");
    use krillnotes_core::core::invite::InviteManager;
    use krillnotes_core::core::contact::generate_fingerprint;

    let base_url = relay_base_url.unwrap_or_else(|| "https://swarm.krillnotes.org".to_string());

    let (invite, bytes) = tokio::task::spawn_blocking(move || -> Result<(krillnotes_core::core::invite::InviteFile, Vec<u8>), String> {
        let client = RelayClient::new(&base_url);
        let payload = client.fetch_invite(&token).map_err(|e| {
            log::error!("fetch_relay_invite: relay fetch failed: {e}");
            e.to_string()
        })?;
        let bytes = BASE64.decode(&payload.payload).map_err(|e| {
            log::error!("fetch_relay_invite: base64 decode failed: {e}");
            e.to_string()
        })?;
        let invite = InviteManager::parse_and_verify_invite_bytes(&bytes).map_err(|e| {
            log::error!("fetch_relay_invite: parse/verify failed: {e}");
            e.to_string()
        })?;
        Ok((invite, bytes))
    })
    .await
    .map_err(|e| e.to_string())??;

    // Write bytes to a temp file for later use by respond_to_invite.
    let temp_path = {
        let dir = std::env::temp_dir();
        let filename = format!("krillnotes-invite-{}.swarm", uuid::Uuid::new_v4());
        let path = dir.join(&filename);
        std::fs::write(&path, &bytes).map_err(|e| {
            log::error!("fetch_relay_invite: failed to write temp file: {e}");
            e.to_string()
        })?;
        path.to_string_lossy().to_string()
    };

    let fingerprint = generate_fingerprint(&invite.inviter_public_key)
        .map_err(|e| e.to_string())?;

    let invite_data = crate::commands::invites::InviteFileData {
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
    };

    Ok(crate::commands::invites::FetchedRelayInvite {
        invite: invite_data,
        temp_path,
    })
}

// ── send_invite_response_via_relay ─────────────────────────────────────────

/// Build a response to a relay-fetched invite and upload it to the relay.
/// Returns the shareable URL of the uploaded response.
#[tauri::command]
pub async fn send_invite_response_via_relay(
    _window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    temp_path: String,
    expires_in_days: Option<u32>,
) -> Result<String, String> {
    log::debug!("send_invite_response_via_relay(identity={identity_uuid}, temp={temp_path})");
    use krillnotes_core::core::invite::InviteManager;

    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Get signing key + declared name.
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Build relay client with auto-login.
    let relay_account = {
        let ram = state.relay_account_managers.lock().expect("Mutex poisoned");
        let mgr = ram.get(&uuid).ok_or("No relay account manager for identity")?;
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        accounts.into_iter().next().ok_or("No relay account configured")?
    };

    let expires_in_days = expires_in_days.unwrap_or(7);
    let relay_account_url = relay_account.relay_url.clone();

    let url = tokio::task::spawn_blocking(move || -> Result<String, String> {
        // Parse the invite from the temp file.
        let invite = InviteManager::parse_and_verify_invite(std::path::Path::new(&temp_path))
            .map_err(|e| {
                log::error!("send_invite_response_via_relay: parse invite failed: {e}");
                e.to_string()
            })?;

        // Build the response.
        let response = InviteManager::build_response(&invite, &signing_key, &declared_name)
            .map_err(|e| {
                log::error!("send_invite_response_via_relay: build_response failed: {e}");
                e.to_string()
            })?;

        // Serialize + base64-encode.
        let bytes = InviteManager::serialize_response_to_bytes(&response).map_err(|e| {
            log::error!("send_invite_response_via_relay: serialize failed: {e}");
            e.to_string()
        })?;
        let payload_b64 = BASE64.encode(&bytes);
        let expires_at = (chrono::Utc::now() + chrono::Duration::days(expires_in_days as i64)).to_rfc3339();

        // Auto-login if needed.
        let mut token = relay_account.session_token.clone();
        if relay_account.session_expires_at < chrono::Utc::now() && !relay_account.password.is_empty() {
            let client = RelayClient::new(&relay_account.relay_url);
            match client.login(&relay_account.email, &relay_account.password, &relay_account.device_public_key) {
                Ok(session) => token = session.session_token,
                Err(e) => log::warn!("send_invite_response_via_relay: auto-login failed: {e}"),
            }
        }
        let client = RelayClient::new(&relay_account.relay_url).with_session_token(&token);
        let info = client.create_invite(&payload_b64, &expires_at).map_err(|e| {
            log::error!("send_invite_response_via_relay: relay upload failed: {e}");
            e.to_string()
        })?;

        // Also upload an Accept bundle so the inviter can discover the response
        // via list_bundles() during polling. This is best-effort — the invite URL
        // was already created successfully above.
        let device_id = krillnotes_core::core::device::get_device_id().unwrap_or_default();
        let accept_bundle_bytes = krillnotes_core::core::swarm::invite::create_accept_bundle(
            krillnotes_core::core::swarm::invite::AcceptParams {
                protocol: "krillnotes/1".to_string(),
                workspace_id: invite.workspace_id.clone(),
                workspace_name: invite.workspace_name.clone(),
                source_device_id: device_id,
                declared_name: declared_name.clone(),
                pairing_token: String::new(), // Not available from InviteFile; left empty
                acceptor_key: &signing_key,
                owner_pubkey: Some(invite.inviter_public_key.clone()),
                channel_preference: krillnotes_core::core::swarm::invite::ChannelPreference {
                    channel_type: "relay".to_string(),
                    relay_url: Some(relay_account_url.clone()),
                },
            },
        );
        match accept_bundle_bytes {
            Ok(bundle_bytes) => {
                // Relay device keys are hex-encoded, not base64
                let invitee_pubkey_hex = hex::encode(signing_key.verifying_key().to_bytes());
                let inviter_pubkey_hex = BASE64.decode(&invite.inviter_public_key)
                    .map(|bytes| hex::encode(&bytes))
                    .unwrap_or_default();
                let bundle_header = krillnotes_core::core::sync::relay::client::BundleHeader {
                    workspace_id: invite.workspace_id.clone(),
                    sender_device_key: invitee_pubkey_hex,
                    recipient_device_keys: vec![inviter_pubkey_hex],
                    mode: Some("accept".to_string()),
                };
                match client.upload_bundle(&bundle_header, &bundle_bytes) {
                    Ok(ids) => log::info!("send_invite_response_via_relay: accept bundle uploaded ({} copies)", ids.len()),
                    Err(e) => log::warn!("send_invite_response_via_relay: accept bundle upload failed (non-fatal): {e}"),
                }
            }
            Err(e) => {
                log::warn!("send_invite_response_via_relay: failed to create accept bundle (non-fatal): {e}");
            }
        }

        Ok(info.url)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(url)
}

// ── fetch_relay_invite_response ────────────────────────────────────────────

/// Fetch an invite response that was uploaded to the relay by the invitee.
/// Verifies the response, validates the invite, increments use count, and
/// returns the pending peer data ready for `accept_peer`.
#[tauri::command]
pub async fn fetch_relay_invite_response(
    _window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    token: String,
    relay_base_url: Option<String>,
) -> Result<crate::commands::invites::PendingPeer, String> {
    log::debug!("fetch_relay_invite_response(identity={identity_uuid}, token={token})");
    use krillnotes_core::core::invite::InviteManager;
    use krillnotes_core::core::contact::generate_fingerprint;

    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let base_url = relay_base_url.unwrap_or_else(|| "https://swarm.krillnotes.org".to_string());

    // Fetch + parse response from relay (unauthenticated GET).
    let response = tokio::task::spawn_blocking(move || -> Result<krillnotes_core::core::invite::InviteResponseFile, String> {
        let client = RelayClient::new(&base_url);
        let payload = client.fetch_invite(&token).map_err(|e| {
            log::error!("fetch_relay_invite_response: relay fetch failed: {e}");
            e.to_string()
        })?;
        let bytes = BASE64.decode(&payload.payload).map_err(|e| {
            log::error!("fetch_relay_invite_response: base64 decode failed: {e}");
            e.to_string()
        })?;
        let response = InviteManager::parse_and_verify_response_bytes(&bytes).map_err(|e| {
            log::error!("fetch_relay_invite_response: parse/verify failed: {e}");
            e.to_string()
        })?;
        Ok(response)
    })
    .await
    .map_err(|e| e.to_string())??;

    // Validate invite is still active and increment use count.
    let invite_uuid = Uuid::parse_str(&response.invite_id).map_err(|e| e.to_string())?;
    {
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
    }

    let fingerprint = generate_fingerprint(&response.invitee_public_key)
        .map_err(|e| e.to_string())?;

    Ok(crate::commands::invites::PendingPeer {
        invite_id: response.invite_id,
        invitee_public_key: response.invitee_public_key,
        invitee_declared_name: response.invitee_declared_name,
        fingerprint,
    })
}

// ── has_relay_credentials ──────────────────────────────────────────────────

/// Return `true` if the given identity has any relay accounts configured.
/// Accepts an optional `identity_uuid` param; falls back to workspace lookup.
#[tauri::command]
pub async fn has_relay_credentials(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: Option<String>,
) -> Result<bool, String> {
    let identity_uuid: Uuid = if let Some(ref uuid_str) = identity_uuid {
        Uuid::parse_str(uuid_str).map_err(|e| e.to_string())?
    } else {
        log::debug!("has_relay_credentials(window={})", window.label());
        let workspace_label = window.label().to_string();
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };
    let managers = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
    if let Some(mgr) = managers.get(&identity_uuid) {
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        Ok(!accounts.is_empty())
    } else {
        Ok(false)
    }
}

// ── reset_peer_watermark ───────────────────────────────────────────────────

/// Reset the `last_sent_op` watermark for a peer to `None`.
///
/// This forces the next sync cycle to generate a full delta from the beginning,
/// which is useful when the peer has reported they are missing operations.
#[tauri::command]
pub fn reset_peer_watermark(
    window: Window,
    state: State<'_, AppState>,
    peer_device_id: String,
) -> Result<(), String> {
    log::info!("reset_peer_watermark(window={}, peer={})", window.label(), peer_device_id);
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces
        .get(window.label())
        .ok_or("No workspace open for this window")?;
    ws.reset_peer_watermark(&peer_device_id, None)
        .map_err(|e| e.to_string())
}

// ── has_pending_sync_ops ───────────────────────────────────────────────────

/// Returns true if there are operations queued to send to at least one non-manual peer.
///
/// Used by the frontend to grey out the "Sync Now" button when nothing is pending.
#[tauri::command]
pub fn has_pending_sync_ops(
    window: Window,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces
        .get(window.label())
        .ok_or("No workspace open for this window")?;
    ws.has_pending_ops_for_any_peer().map_err(|e| e.to_string())
}
