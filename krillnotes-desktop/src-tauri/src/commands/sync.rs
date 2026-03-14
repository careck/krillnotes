// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Tauri commands for relay-based sync operations.

use crate::AppState;
use chrono::Utc;
use std::sync::Arc;
use tauri::{Emitter, State, Window};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use uuid::Uuid;
use krillnotes_core::core::{
    device::get_device_id,
    sync::{FolderChannel, SyncContext, SyncEngine, SyncEvent},
    sync::relay::{
        RelayCredentials,
        load_relay_credentials,
        save_relay_credentials,
    },
};
use krillnotes_core::core::sync::relay::{RelayChannel, RelayClient};
use krillnotes_core::core::sync::relay::auth::decrypt_pop_challenge;

/// Relay account info returned by `get_relay_info`.
/// Serialised with camelCase keys so the TypeScript interface matches.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayInfo {
    pub relay_url: String,
    pub email: String,
}

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
    let workspace_label = window.label().to_string();
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces
        .get(&workspace_label)
        .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?;
    ws.update_peer_channel(&peer_device_id, &channel_type, &channel_params)
        .map_err(|e| e.to_string())
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
    let workspace_label = window.label().to_string();

    // -- Collect context data under brief locks (all guards released before spawn) --
    let identity_uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };

    let (signing_key, sender_display_name, identity_pubkey, relay_key, sender_device_key_hex) = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&identity_uuid).ok_or("Identity not unlocked")?;
        let pubkey_b64 = BASE64.encode(id.verifying_key.as_bytes()); // FolderChannel uses Base64
        let pubkey_hex = hex::encode(id.verifying_key.to_bytes());   // RelayChannel uses hex
        let rk = id.relay_key();
        (id.signing_key.clone(), id.display_name.clone(), pubkey_b64, rk, pubkey_hex)
    };

    let workspace_name = {
        let m = state.workspace_paths.lock().map_err(|e| e.to_string())?;
        m.get(&workspace_label)
            .and_then(|p| p.file_stem().and_then(|n| n.to_str()).map(String::from))
            .unwrap_or_else(|| workspace_label.clone())
    };

    let device_id = get_device_id().map_err(|e| e.to_string())?;

    // Load relay credentials and workspace_id while we can still briefly lock.
    let relay_dir = crate::settings::config_dir().join("relay");
    let relay_creds = load_relay_credentials(&relay_dir, &identity_uuid.to_string(), &relay_key)
        .map_err(|e| e.to_string())?;

    let workspace_id_str = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        workspaces
            .get(&workspace_label)
            .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?
            .workspace_id()
            .to_string()
    };

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

        if let Some(creds) = relay_creds {
            let relay_client = RelayClient::new(&creds.relay_url)
                .with_session_token(&creds.session_token);
            engine.register_channel(Box::new(RelayChannel::new(
                relay_client,
                workspace_id_str,
                sender_device_key_hex,
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

        engine.poll(workspace, &mut ctx).map_err(|e| e.to_string())
        // engine (and RelayClient) dropped here — safe on a spawn_blocking thread
    })
    .await
    .map_err(|e| e.to_string())??;

    // If any bundles were applied, notify WorkspaceView to reload the note tree.
    let bundles_applied = events.iter().any(|e| matches!(e, SyncEvent::BundleApplied { .. }));
    if bundles_applied {
        let _ = window.emit("workspace-updated", ());
    }

    Ok(events)
}

// ── configure_relay ────────────────────────────────────────────────────────

/// Register with a relay server and store credentials, then create a
/// `SyncEngine` with a `RelayChannel` for the given identity.
#[tauri::command]
pub async fn configure_relay(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Capture signing key, verifying key, and relay encryption key in one lock.
    let (signing_key, verifying_key, relay_key) = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&uuid)
            .ok_or("Identity is not unlocked — please unlock your identity first")?;
        // Use .clone() — consistent with how poll_sync clones the signing key.
        let sk = id.signing_key.clone();
        let vk = id.verifying_key;
        let rk = id.relay_key();
        (sk, vk, rk)
    };

    // device_public_key is hex-encoded (not Base64 — relay API uses hex throughout).
    let device_public_key = hex::encode(verifying_key.to_bytes());
    let relay_dir = crate::settings::config_dir().join("relay");

    // RelayClient uses reqwest::blocking, which owns its own Tokio runtime.
    // Dropping it inside an async context panics. Run everything in spawn_blocking
    // so the client's lifetime is entirely within a non-async thread.
    tokio::task::spawn_blocking(move || {
        let client = RelayClient::new(&relay_url);

        // Step 1: Register → receive PoP challenge.
        let result = client
            .register(&email, &password, &identity_uuid, &device_public_key)
            .map_err(|e| e.to_string())?;

        // Step 2: Decrypt the PoP challenge using the identity's Ed25519 signing key.
        let nonce_bytes = decrypt_pop_challenge(
            &signing_key,
            &result.challenge.encrypted_nonce,
            &result.challenge.server_public_key,
        )
        .map_err(|e| e.to_string())?;
        let nonce_hex = hex::encode(&nonce_bytes);

        // Step 3: Verify registration — obtain session token.
        let session = client
            .register_verify(&device_public_key, &nonce_hex)
            .map_err(|e| e.to_string())?;

        let creds = RelayCredentials {
            relay_url,
            email,
            session_token: session.session_token,
            // 30 days is a local approximation; relay server governs actual expiry.
            session_expires_at: Utc::now() + chrono::Duration::days(30),
            device_public_key,
        };
        save_relay_credentials(&relay_dir, &identity_uuid, &creds, &relay_key)
            .map_err(|e| e.to_string())?;

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── relay_login ────────────────────────────────────────────────────────────

/// Re-authenticate with an existing relay account (e.g. after a token
/// expiry).
#[tauri::command]
pub async fn relay_login(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let relay_key = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        m.get(&uuid)
            .ok_or("Identity is not unlocked — please unlock your identity first")?
            .relay_key()
    };

    let relay_dir = crate::settings::config_dir().join("relay");

    // Reuse existing device_public_key if credentials are already stored,
    // otherwise derive it fresh from the verifying key.
    let device_public_key = {
        match load_relay_credentials(&relay_dir, &identity_uuid, &relay_key)
            .map_err(|e| e.to_string())?
        {
            Some(existing) => existing.device_public_key,
            None => {
                let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
                let id = m.get(&uuid)
                    .ok_or("Identity is not unlocked")?;
                hex::encode(id.verifying_key.to_bytes())
            }
        }
    };

    // Same spawn_blocking pattern as configure_relay — reqwest::blocking must not
    // be dropped inside an async context.
    tokio::task::spawn_blocking(move || {
        let client = RelayClient::new(&relay_url);
        let session = client
            .login(&email, &password, &device_public_key)
            .map_err(|e| e.to_string())?;

        let creds = RelayCredentials {
            relay_url,
            email,
            session_token: session.session_token,
            session_expires_at: Utc::now() + chrono::Duration::days(30),
            device_public_key,
        };
        save_relay_credentials(&relay_dir, &identity_uuid, &creds, &relay_key)
            .map_err(|e| e.to_string())?;

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── create_relay_invite ────────────────────────────────────────────────────

/// Upload an invite to the relay and return the shareable URL.
///
/// `token` is the invite token previously created by `create_invite`.
///
/// TODO: Upload invite bytes to relay, return hosted URL.
#[tauri::command]
pub async fn create_relay_invite(
    _window: Window,
    _state: State<'_, AppState>,
    token: String,
) -> Result<String, String> {
    let _ = token;
    Err("Relay not yet configured".to_string())
}

// ── fetch_relay_invite ─────────────────────────────────────────────────────

/// Fetch an invite file from the relay by token and return the raw bytes.
///
/// `token` is the path component extracted from the relay invite URL.
///
/// TODO: Download invite bytes from relay and return them.
#[tauri::command]
pub async fn fetch_relay_invite(
    _window: Window,
    _state: State<'_, AppState>,
    token: String,
) -> Result<Vec<u8>, String> {
    let _ = token;
    Err("Relay not yet configured".to_string())
}

// ── has_relay_credentials ──────────────────────────────────────────────────

/// Return `true` if the current identity has relay credentials configured.
///
/// TODO: Check relay credentials for the active identity.
#[tauri::command]
pub async fn has_relay_credentials(
    window: Window,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let workspace_label = window.label().to_string();
    let identity_uuid: Uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };
    let relay_key = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        m.get(&identity_uuid)
            .ok_or("Identity not unlocked")?
            .relay_key()
    };
    let relay_dir = crate::settings::config_dir().join("relay");
    let creds = load_relay_credentials(&relay_dir, &identity_uuid.to_string(), &relay_key)
        .map_err(|e| e.to_string())?;
    Ok(creds.is_some())
}

// ── get_relay_info ──────────────────────────────────────────────────────────

/// Return relay account info (URL + email) if credentials are stored for the
/// identity bound to this workspace window. Returns `null` if not configured.
#[tauri::command]
pub async fn get_relay_info(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Option<RelayInfo>, String> {
    let workspace_label = window.label().to_string();
    let identity_uuid: Uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };
    let relay_key = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        m.get(&identity_uuid)
            .ok_or("Identity not unlocked")?
            .relay_key()
    };
    let relay_dir = crate::settings::config_dir().join("relay");
    match load_relay_credentials(&relay_dir, &identity_uuid.to_string(), &relay_key)
        .map_err(|e| e.to_string())?
    {
        Some(creds) => Ok(Some(RelayInfo {
            relay_url: creds.relay_url,
            email: creds.email,
        })),
        None => Ok(None),
    }
}

// ── parse_invite_bytes ─────────────────────────────────────────────────────

/// Parse raw invite bytes (e.g. from relay download) and return invite info.
///
/// TODO: Parse and verify invite from raw bytes once relay is implemented.
#[tauri::command]
pub async fn parse_invite_bytes(
    _window: Window,
    _state: State<'_, AppState>,
    _bytes: Vec<u8>,
) -> Result<crate::commands::invites::InviteFileData, String> {
    Err("parse_invite_bytes not yet implemented".to_string())
}

// ── write_temp_swarm_bytes ─────────────────────────────────────────────────

/// Write raw invite bytes to a temporary file and return the path.
///
/// This is used to save relay-fetched invite bytes so they can be passed
/// to `respond_to_invite` which requires a file path.
///
/// TODO: Write bytes to OS temp dir and return path.
#[tauri::command]
pub async fn write_temp_swarm_bytes(
    _bytes: Vec<u8>,
) -> Result<String, String> {
    Err("write_temp_swarm_bytes not yet implemented".to_string())
}
