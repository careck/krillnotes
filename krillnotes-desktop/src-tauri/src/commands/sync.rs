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
    _window: Window,
    state: State<'_, AppState>,
    workspace_label: String,
    peer_device_id: String,
    channel_type: String,
    channel_params: String,
) -> Result<(), String> {
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
