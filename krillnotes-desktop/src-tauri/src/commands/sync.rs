// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Tauri commands for relay-based sync operations.

use crate::AppState;
use tauri::{State, Window};

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

/// Run one sync poll cycle for the given workspace.
///
/// TODO: Get workspace, identity, and contact manager.
/// TODO: Call sync_engine.poll(workspace, ctx).
/// TODO: Emit events to the window and return them.
#[tauri::command]
pub async fn poll_sync(
    _window: Window,
    _state: State<'_, AppState>,
    _workspace_label: String,
) -> Result<Vec<serde_json::Value>, String> {
    Err("poll_sync not yet implemented".to_string())
}

// ── configure_relay ────────────────────────────────────────────────────────

/// Register with a relay server and store credentials, then create a
/// `SyncEngine` with a `RelayChannel` for the given identity.
///
/// TODO: Run registration + PoP flow, save credentials, create SyncEngine
///       with RelayChannel.
#[tauri::command]
pub async fn configure_relay(
    _state: State<'_, AppState>,
    _identity_uuid: String,
    _relay_url: String,
    _email: String,
    _password: String,
) -> Result<(), String> {
    Err("configure_relay not yet implemented".to_string())
}

// ── relay_login ────────────────────────────────────────────────────────────

/// Re-authenticate with an existing relay account (e.g. after a token
/// expiry).
///
/// TODO: Re-login with existing credentials.
#[tauri::command]
pub async fn relay_login(
    _state: State<'_, AppState>,
    _identity_uuid: String,
    _email: String,
    _password: String,
) -> Result<(), String> {
    Err("relay_login not yet implemented".to_string())
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
    _window: Window,
    _state: State<'_, AppState>,
) -> Result<bool, String> {
    Ok(false)
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
