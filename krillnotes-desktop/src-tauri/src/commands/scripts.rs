// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::commands::scripting::ScriptMutationResult;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn list_user_scripts(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<crate::UserScript>, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_user_scripts()
        .map_err(|e| e.to_string())
}

/// Returns a single user script by ID.
#[tauri::command]
pub fn get_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
) -> std::result::Result<crate::UserScript, String> {
    let label = window.label();
    state
        .workspaces
        .lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_user_script(&script_id)
        .map_err(|e| e.to_string())
}

/// Creates a new user script from source code.
#[tauri::command]
pub fn create_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    source_code: String,
    category: Option<String>,
) -> std::result::Result<ScriptMutationResult<crate::UserScript>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    let (data, load_errors) = match category {
        Some(cat) => workspace.create_user_script_with_category(&source_code, &cat),
        None => workspace.create_user_script(&source_code),
    }
    .map_err(|e| {
        log::error!("create_user_script failed: {e}");
        e.to_string()
    })?;
    Ok(ScriptMutationResult { data, load_errors })
}

/// Updates an existing user script's source code.
#[tauri::command]
pub fn update_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    source_code: String,
) -> std::result::Result<ScriptMutationResult<crate::UserScript>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    let (data, load_errors) = workspace
        .update_user_script(&script_id, &source_code)
        .map_err(|e| {
            log::error!("update_user_script failed: {e}");
            e.to_string()
        })?;
    Ok(ScriptMutationResult { data, load_errors })
}

/// Deletes a user script by ID.
#[tauri::command]
pub fn delete_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
) -> std::result::Result<Vec<crate::ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.delete_user_script(&script_id).map_err(|e| {
        log::error!("delete_user_script failed: {e}");
        e.to_string()
    })
}

/// Toggles the enabled state of a user script.
#[tauri::command]
pub fn toggle_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    enabled: bool,
) -> std::result::Result<Vec<crate::ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .toggle_user_script(&script_id, enabled)
        .map_err(|e| e.to_string())
}

/// Changes the load order of a user script.
#[tauri::command]
pub fn reorder_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    new_load_order: i32,
) -> std::result::Result<Vec<crate::ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .reorder_user_script(&script_id, new_load_order)
        .map_err(|e| e.to_string())
}

/// Reassigns sequential load order to all scripts given in order, then reloads.
#[tauri::command]
pub fn reorder_all_user_scripts(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_ids: Vec<String>,
) -> std::result::Result<Vec<crate::ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .reorder_all_user_scripts(&script_ids)
        .map_err(|e| e.to_string())
}
