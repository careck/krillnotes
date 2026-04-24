// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Tauri commands for RBAC permission queries and mutations.

use crate::AppState;
use tauri::State;

// ── Query commands ──────────────────────────────────────────────────

/// Returns explicit permission grants anchored at `note_id`.
#[tauri::command]
pub fn get_note_permissions(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Vec<crate::PermissionGrantRow>, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_note_permissions(&note_id)
        .map_err(|e| {
            log::error!("get_note_permissions failed: {e}");
            e.to_string()
        })
}

/// Returns the effective role for the current user on `note_id`.
#[tauri::command]
pub fn get_effective_role(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<crate::EffectiveRoleInfo, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_effective_role(&note_id)
        .map_err(|e| {
            log::error!("get_effective_role failed: {e}");
            e.to_string()
        })
}

/// Returns effective roles for all notes (for tree dot rendering).
#[tauri::command]
pub fn get_all_effective_roles(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<std::collections::HashMap<String, String>, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_all_effective_roles()
        .map_err(|e| {
            log::error!("get_all_effective_roles failed: {e}");
            e.to_string()
        })
}

/// Returns grants inherited from ancestors of `note_id`.
#[tauri::command]
pub fn get_inherited_permissions(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Vec<crate::InheritedGrant>, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_inherited_permissions(&note_id)
        .map_err(|e| {
            log::error!("get_inherited_permissions failed: {e}");
            e.to_string()
        })
}

/// Preview which downstream grants would be invalidated by changing
/// `user_id`'s role to `new_role` on `note_id`.
#[tauri::command]
pub fn preview_cascade(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    user_id: String,
    new_role: String,
) -> std::result::Result<Vec<crate::CascadeImpactRow>, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .preview_cascade(&note_id, &user_id, &new_role)
        .map_err(|e| {
            log::error!("preview_cascade failed: {e}");
            e.to_string()
        })
}

/// Returns note IDs that have at least one explicit permission grant anchored to them.
#[tauri::command]
pub fn get_share_anchor_ids(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<String>, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_share_anchor_ids()
        .map_err(|e| {
            log::error!("get_share_anchor_ids failed: {e}");
            e.to_string()
        })
}

/// Returns true if the current actor is the workspace root owner.
#[tauri::command]
pub fn is_root_owner(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<bool, String> {
    let label = window.label();
    let ws = state.workspaces.lock().expect("Mutex poisoned");
    let ws = ws.get(label).ok_or("No workspace open")?;
    Ok(ws.is_root_owner())
}

// ── Mutation commands ───────────────────────────────────────────────

/// Grants or updates a permission for `user_id` on `note_id` with the
/// given `role`.
#[tauri::command]
pub fn set_permission(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    user_id: String,
    role: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .set_permission(&note_id, &user_id, &role)
        .map_err(|e| {
            log::error!("set_permission failed: {e}");
            e.to_string()
        })
}

/// Revokes the permission for `user_id` on `note_id`.
#[tauri::command]
pub fn revoke_permission(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    user_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .revoke_permission(&note_id, &user_id)
        .map_err(|e| {
            log::error!("revoke_permission failed: {e}");
            e.to_string()
        })
}
