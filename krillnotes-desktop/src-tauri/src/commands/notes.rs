// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::State;
use std::collections::BTreeMap;
use serde_json::Value;

/// Returns all notes in the calling window's workspace.
#[tauri::command]
pub fn list_notes(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<crate::Note>, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_all_notes()
        .map_err(|e| e.to_string())
}

/// Returns a single note by ID from the calling window's workspace.
#[tauri::command]
pub fn get_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<crate::Note, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_note(&note_id)
        .map_err(|e| e.to_string())
}

/// Returns the registered note types for the calling window's workspace.
#[tauri::command]
pub fn get_node_types(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    let types = workspace.list_node_types()
        .map_err(|e| e.to_string())?;

    Ok(types)
}

/// Toggles the expansion state of `note_id` in the calling window's workspace.
#[tauri::command]
pub fn toggle_note_expansion(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.toggle_note_expansion(&note_id)
        .map_err(|e| e.to_string())
}

/// Persists the selected note ID for the calling window's workspace.
#[tauri::command]
pub fn set_selected_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: Option<String>,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.set_selected_note(note_id.as_deref())
        .map_err(|e| e.to_string())
}

/// Creates a new note and returns it; uses root insertion when `parent_id` is `None`.
#[tauri::command]
pub async fn create_note_with_type(
    window: tauri::Window,
    state: State<'_, AppState>,
    parent_id: Option<String>,
    position: String,
    schema: String,
) -> std::result::Result<crate::Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    // Convert position string to AddPosition enum
    let add_position = match position.as_str() {
        "child" => crate::AddPosition::AsChild,
        "sibling" => crate::AddPosition::AsSibling,
        _ => return Err("Invalid position: must be 'child' or 'sibling'".to_string()),
    };

    // If no parent_id, create root note
    let note_id = if let Some(pid) = parent_id {
        workspace.create_note(&pid, add_position, &schema)
            .map_err(|e| e.to_string())?
    } else {
        // Create root note (parent_id = null, position = 0)
        workspace.create_note_root(&schema)
            .map_err(|e| e.to_string())?
    };

    // Fetch and return the created note
    workspace.get_note(&note_id)
        .map_err(|e| e.to_string())
}

/// Updates the title and fields of an existing note, returning the updated note.
#[tauri::command]
pub fn update_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    title: String,
    fields: BTreeMap<String, crate::FieldValue>,
) -> std::result::Result<crate::Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.update_note(&note_id, title, fields)
        .map_err(|e| e.to_string())
}

/// Full save pipeline: runs group visibility, field validation, required checks,
/// on_save hook, and then writes to the database.
#[tauri::command]
pub fn save_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    title: String,
    fields: BTreeMap<String, crate::FieldValue>,
) -> std::result::Result<crate::SaveResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.save_note_with_pipeline(&note_id, title, fields)
        .map_err(|e| e.to_string())
}

/// Searches for notes whose title or text-like field values contain `query`.
#[tauri::command]
pub fn search_notes(
    window: tauri::Window,
    state: State<'_, AppState>,
    query: String,
    target_schema: Option<String>,
) -> std::result::Result<Vec<crate::NoteSearchResult>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.search_notes(&query, target_schema.as_deref())
        .map_err(|e| e.to_string())
}

/// Returns the number of direct children of the note identified by `note_id`.
#[tauri::command]
pub fn count_children(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<usize, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    workspace.count_children(&note_id)
        .map_err(|e| e.to_string())
}

/// Deletes the note identified by `note_id` using the specified [`DeleteStrategy`].
#[tauri::command]
pub fn delete_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    strategy: crate::DeleteStrategy,
) -> std::result::Result<crate::DeleteResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.delete_note(&note_id, strategy)
        .map_err(|e| e.to_string())
}

/// Moves a note to a new parent and/or position.
#[tauri::command]
pub fn move_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    new_parent_id: Option<String>,
    new_position: f64,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.move_note(
        &note_id,
        new_parent_id.as_deref(),
        new_position,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn deep_copy_note_cmd(
    state: State<'_, AppState>,
    window: tauri::Window,
    source_note_id: String,
    target_note_id: String,
    position: String, // "child" or "sibling"
) -> std::result::Result<String, String> {
    let label = window.label().to_string();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces
        .get_mut(&label)
        .ok_or_else(|| "No workspace open".to_string())?;
    let pos = if position == "child" {
        crate::AddPosition::AsChild
    } else {
        crate::AddPosition::AsSibling
    };
    ws.deep_copy_note(&source_note_id, &target_note_id, pos)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_note_tags(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    tags: Vec<String>,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.update_note_tags(&note_id, tags)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_all_tags(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_all_tags()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_notes_for_tag(
    window: tauri::Window,
    state: State<'_, AppState>,
    tags: Vec<String>,
) -> std::result::Result<Vec<crate::Note>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_notes_for_tag(&tags)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_paste_menu_enabled(
    state: State<'_, AppState>,
    _window: tauri::Window,
    enabled: bool,
) -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let items = state.paste_menu_items.lock().expect("Mutex poisoned");
        if let Some((child_item, sibling_item)) = items.get("macos") {
            child_item.set_enabled(enabled).map_err(|e| e.to_string())?;
            sibling_item.set_enabled(enabled).map_err(|e| e.to_string())?;
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let label = _window.label().to_string();
        let items = state.paste_menu_items.lock().expect("Mutex poisoned");
        if let Some((child_item, sibling_item)) = items.get(&label) {
            child_item.set_enabled(enabled).map_err(|e| e.to_string())?;
            sibling_item.set_enabled(enabled).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

// ── Operations log commands ──────────────────────────────────────

/// Returns operation summaries matching the given filters.
#[tauri::command]
pub fn list_operations(
    window: tauri::Window,
    state: State<'_, AppState>,
    type_filter: Option<String>,
    since: Option<i64>,
    until: Option<i64>,
) -> std::result::Result<Vec<krillnotes_core::OperationSummary>, String> {
    let label = window.label();
    let mut summaries = state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_operations(type_filter.as_deref(), since, until)
        .map_err(|e| e.to_string())?;

    // Resolve raw base64 public keys to display names where possible.
    let mut resolved_indices = vec![false; summaries.len()];

    // Pass 1: resolve via identity_manager, then drop the lock.
    {
        let identity_manager = state.identity_manager.lock().expect("Mutex poisoned");
        for (i, summary) in summaries.iter_mut().enumerate() {
            if !summary.author_key.is_empty() {
                if let Some(name) = identity_manager.lookup_display_name(&summary.author_key) {
                    summary.author_key = name;
                    resolved_indices[i] = true;
                }
            }
        }
    }

    // Pass 2: for keys not yet resolved, try contact_managers, then fall back to fingerprint.
    {
        let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
        for (i, summary) in summaries.iter_mut().enumerate() {
            if resolved_indices[i] || summary.author_key.is_empty() {
                continue;
            }
            let mut resolved = false;
            for cm in contact_managers.values() {
                if let Ok(Some(contact)) = cm.find_by_public_key(&summary.author_key) {
                    summary.author_key = contact.display_name().to_string();
                    resolved = true;
                    break;
                }
            }
            if !resolved {
                summary.author_key = summary.author_key.chars().take(8).collect();
            }
        }
    }

    Ok(summaries)
}

/// Returns the full JSON payload for a single operation by ID.
#[tauri::command]
pub fn get_operation_detail(
    window: tauri::Window,
    state: State<'_, AppState>,
    operation_id: String,
) -> std::result::Result<Value, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_operation_detail(&operation_id)
        .map_err(|e| e.to_string())
}

/// Deletes all operations from the log.
#[tauri::command]
pub fn purge_operations(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<usize, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .purge_all_operations()
        .map_err(|e| e.to_string())
}

// ── Undo / Redo commands ──────────────────────────────────────────

/// Undoes the most recent workspace mutation.
#[tauri::command]
pub fn undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<crate::UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.undo().map_err(|e| e.to_string())
}

/// Re-applies the most recently undone mutation.
#[tauri::command]
pub fn redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<crate::UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.redo().map_err(|e| e.to_string())
}

/// Returns true if there is an action to undo.
#[tauri::command]
pub fn can_undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_undo()).unwrap_or(false)
}

/// Returns true if there is an action to redo.
#[tauri::command]
pub fn can_redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_redo()).unwrap_or(false)
}

/// Returns the workspace undo history limit.
#[tauri::command]
pub fn get_undo_limit(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<usize, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces.get(label).ok_or("No workspace open")?;
    Ok(ws.get_undo_limit())
}

/// Sets the workspace undo history limit (1–500).
#[tauri::command]
pub fn set_undo_limit(
    window: tauri::Window,
    state: State<'_, AppState>,
    limit: usize,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces.get_mut(label).ok_or("No workspace open")?;
    ws.set_undo_limit(limit).map_err(|e| e.to_string())
}

/// Opens an undo group.
#[tauri::command]
pub fn begin_undo_group(
    window: tauri::Window,
    state: State<'_, AppState>,
) {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    if let Some(ws) = workspaces.get_mut(label) {
        ws.begin_undo_group();
    }
}

/// Closes the open undo group and collapses buffered mutations into one entry.
#[tauri::command]
pub fn end_undo_group(
    window: tauri::Window,
    state: State<'_, AppState>,
) {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    if let Some(ws) = workspaces.get_mut(label) {
        ws.end_undo_group();
    }
}

/// Undoes the most recent script mutation (isolated from the note undo stack).
#[tauri::command]
pub fn script_undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<crate::UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.script_undo().map_err(|e| e.to_string())
}

/// Re-applies the most recently undone script mutation.
#[tauri::command]
pub fn script_redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<crate::UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.script_redo().map_err(|e| e.to_string())
}

/// Returns true if there is a script action to undo.
#[tauri::command]
pub fn can_script_undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_script_undo()).unwrap_or(false)
}

/// Returns true if there is a script action to redo.
#[tauri::command]
pub fn can_script_redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_script_redo()).unwrap_or(false)
}
