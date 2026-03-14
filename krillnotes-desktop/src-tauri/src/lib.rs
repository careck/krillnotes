// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Tauri application backend for Krillnotes.
//!
//! Exposes Tauri commands that the React frontend calls via `invoke()`.
//! Each command is scoped to the calling window's workspace via
//! [`AppState`] and the window label.

pub mod locales;
pub mod menu;
pub mod settings;
pub mod themes;

mod commands;
pub use commands::*;

use tauri::Emitter;

// Re-export all public core library types into this crate's namespace.
#[doc(inline)]
pub use krillnotes_core::*;

use uuid::Uuid;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

/// Per-process state shared across all workspace windows.
///
/// Each window label maps to its open [`Workspace`] and the filesystem path
/// of its database file. Both maps are protected by a [`Mutex`] since Tauri
/// may call commands from multiple threads.
pub struct AppState {
    /// Map from window label to the open [`Workspace`] for that window.
    pub workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    /// Map from window label to the filesystem path of the open database.
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
    /// Map from window label to the identity UUID that opened that workspace.
    /// Used to route `.swarm` delta bundles to the correct workspace when
    /// multiple workspaces are open simultaneously.
    pub workspace_identities: Arc<Mutex<HashMap<String, Uuid>>>,
    /// Label of the window that most recently gained focus. Used to route
    /// native menu events to the correct window without relying on async
    /// focus checks in the frontend (which are unreliable on Windows).
    pub focused_window: Arc<Mutex<Option<String>>>,
    /// Identity manager — handles identity CRUD, unlock, and workspace bindings.
    pub identity_manager: Arc<Mutex<IdentityManager>>,
    /// Per-identity contact managers — keyed by identity UUID, created on unlock.
    pub contact_managers: Arc<Mutex<HashMap<Uuid, krillnotes_core::core::contact::ContactManager>>>,
    /// Per-identity invite managers — keyed by identity UUID, created on unlock.
    pub invite_managers: Arc<Mutex<HashMap<Uuid, krillnotes_core::core::invite::InviteManager>>>,
    /// In-memory map of currently unlocked identities (UUID → unlocked state).
    /// Entries are removed when an identity is locked or the app exits.
    pub unlocked_identities: Arc<Mutex<HashMap<Uuid, UnlockedIdentity>>>,
    /// Paste menu item handles for dynamic enable/disable.
    /// On macOS: one global pair keyed by "macos" (the menu bar is shared).
    /// On Windows: keyed by window label (each window owns its own menu bar).
    pub paste_menu_items: Arc<Mutex<HashMap<String, (tauri::menu::MenuItem<tauri::Wry>, tauri::menu::MenuItem<tauri::Wry>)>>>,
    /// Workspace-specific menu item handles (Add Note, Delete Note, Copy Note,
    /// Manage Scripts, Operations Log, Export Workspace).
    /// On macOS: one global list keyed by "macos" — enabled when a workspace
    /// opens, disabled when the last workspace window closes.
    /// On Windows: keyed by window label — `rebuild_menus` stores into this map during
    /// language changes, but items are enabled at build time so the stored handles are
    /// never read back to toggle enabled state.
    pub workspace_menu_items: Arc<Mutex<HashMap<String, Vec<tauri::menu::MenuItem<tauri::Wry>>>>>,
    /// File path that arrived via OS file-open before the frontend JS was
    /// ready to receive a pushed event. Cleared on first read by
    /// `consume_pending_file_open`. `None` when no file is pending.
    pub pending_file_open: Arc<Mutex<Option<PathBuf>>>,
}

/// Maps raw menu event IDs to the user-facing message strings emitted to the frontend.
const MENU_MESSAGES: &[(&str, &str)] = &[
    ("file_new", "File > New Workspace clicked"),
    ("file_open", "File > Open Workspace clicked"),
    ("file_export", "File > Export Workspace clicked"),
    ("file_import", "File > Import Workspace clicked"),
    ("edit_add_note", "Edit > Add Note clicked"),
    ("edit_delete_note", "Edit > Delete Note clicked"),
    ("view_refresh", "View > Refresh clicked"),
    ("help_about", "Help > About Krillnotes clicked"),
    ("edit_manage_scripts", "Edit > Manage Scripts clicked"),
    ("edit_settings", "Edit > Settings clicked"),
    // Retained for when sync is enabled per-workspace and the Operations Log item is unlocked.
    ("view_operations_log", "View > Operations Log clicked"),
    ("edit_copy_note",        "Edit > Copy Note clicked"),
    ("edit_paste_as_child",   "Edit > Paste as Child clicked"),
    ("edit_paste_as_sibling", "Edit > Paste as Sibling clicked"),
    ("workspace_properties",  "Edit > Workspace Properties clicked"),
    ("workspace_peers",       "Edit > Workspace Peers clicked"),
    ("file_identities",       "File > Manage Identities clicked"),
    ("file_open_swarm",       "File > Open Swarm File clicked"),
    ("create_delta_swarm",    "Edit > Create delta Swarm clicked"),
];

/// Translates a native [`tauri::menu::MenuEvent`] into a `"menu-action"` event
/// emitted only to the window that was most recently focused.
///
/// [`tauri::Emitter::emit_to`] with [`tauri::EventTarget::WebviewWindow`]
/// delivers the event exclusively to that window's
/// `getCurrentWebviewWindow().listen()` handler on the frontend, so multiple
/// open workspace windows do not all react to the same menu click.
///
/// This also fixes Windows, where clicking a native menu item briefly
/// unfocuses the application window before the event fires, making async
/// focus checks in the frontend unreliable.
fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let Some((_, message)) = MENU_MESSAGES.iter()
        .find(|(id, _)| id == &event.id().as_ref())
    else {
        return;
    };

    let state = app.state::<AppState>();
    let label = state.focused_window.lock().expect("Mutex poisoned").clone();
    if let Some(label) = label {
        let _ = app.emit_to(
            tauri::EventTarget::WebviewWindow { label },
            "menu-action",
            message,
        );
    } else {
        // Fallback: a menu click always has a focused window in practice,
        // so this path is only reachable during an unusual startup race.
        let _ = app.emit("menu-action", message);
    }
}

/// Configures and starts the Tauri application event loop.
///
/// Registers all plugins, commands, the global [`AppState`], window event
/// handlers, and the application menu before entering the run loop.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(AppState {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            workspace_paths: Arc::new(Mutex::new(HashMap::new())),
            workspace_identities: Arc::new(Mutex::new(HashMap::new())),
            focused_window: Arc::new(Mutex::new(None)),
            identity_manager: Arc::new(Mutex::new(
                IdentityManager::new(settings::config_dir()).expect("Failed to init IdentityManager")
            )),
            contact_managers: Arc::new(Mutex::new(HashMap::new())),
            invite_managers: Arc::new(Mutex::new(HashMap::new())),
            unlocked_identities: Arc::new(Mutex::new(HashMap::new())),
            paste_menu_items: Arc::new(Mutex::new(HashMap::new())),
            workspace_menu_items: Arc::new(Mutex::new(HashMap::new())),
            pending_file_open: Arc::new(Mutex::new(None)),
        })
        .on_window_event(|window, event| {
            let label = window.label().to_string();
            let state = window.state::<AppState>();
            match event {
                // Remove workspace state when a window is destroyed so the same
                // file can be reopened after its window has been closed.
                tauri::WindowEvent::Destroyed => {
                    // Persist cached metadata before dropping the workspace.
                    if let Some(ws) = state.workspaces.lock().expect("Mutex poisoned").get(&label) {
                        let _ = ws.write_info_json();
                    }
                    state.workspaces.lock().expect("Mutex poisoned").remove(&label);
                    state.workspace_paths.lock().expect("Mutex poisoned").remove(&label);
                    state.workspace_identities.lock().expect("Mutex poisoned").remove(&label);

                    // On macOS the menu bar is global. If this was the last
                    // workspace window, disable workspace-specific items so
                    // they appear greyed out if the launch window ever returns.
                    #[cfg(target_os = "macos")]
                    {
                        let no_workspaces_remain = state.workspaces
                            .lock().expect("Mutex poisoned").is_empty();
                        if no_workspaces_remain {
                            let items = state.workspace_menu_items
                                .lock().expect("Mutex poisoned");
                            if let Some(ws_items) = items.get("macos") {
                                for item in ws_items {
                                    let _ = item.set_enabled(false);
                                }
                            }
                        }
                    }
                }
                // Track which window is currently active so that menu events
                // can be routed to the correct window (see handle_menu_event).
                tauri::WindowEvent::Focused(true) => {
                    *state.focused_window.lock().expect("Mutex poisoned") = Some(label);
                }
                _ => {}
            }
        })
        .setup(|app| {
            let lang = settings::load_settings().language;
            let strings = locales::menu_strings(&lang);
            let menu_result = menu::build_menu(app.handle(), &strings)?;
            app.set_menu(menu_result.menu)?;

            // On macOS the menu bar is global (shared by all windows).
            // Store handles under the "macos" key so they can be found from
            // any window later (set_paste_menu_enabled, workspace enable/disable).
            #[cfg(target_os = "macos")]
            {
                let state = app.state::<AppState>();
                state.paste_menu_items.lock().expect("Mutex poisoned")
                    .insert("macos".to_string(), (menu_result.paste_as_child, menu_result.paste_as_sibling));
                state.workspace_menu_items.lock().expect("Mutex poisoned")
                    .insert("macos".to_string(), menu_result.workspace_items);
            }

            // Ensure default workspace directory exists on startup
            let app_settings = settings::load_settings();
            let dir = std::path::Path::new(&app_settings.workspace_directory);
            if !dir.exists() {
                std::fs::create_dir_all(dir).ok();
            }

            // Auto-migrate flat *.db files to per-workspace folders
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "db").unwrap_or(false) {
                    let stem = path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default();
                    if stem.is_empty() { continue; }
                    let new_folder = dir.join(stem);
                    if new_folder.exists() { continue; } // already migrated
                    if std::fs::create_dir_all(&new_folder).is_ok() {
                        if let Err(e) = std::fs::rename(&path, new_folder.join("notes.db")) {
                            eprintln!("[migration] Failed to move {:?}: {e}", path);
                            let _ = std::fs::remove_dir(&new_folder); // rollback folder
                        } else {
                            let _ = std::fs::create_dir_all(new_folder.join("attachments"));
                            eprintln!("[migration] Migrated {:?} → {:?}", path, new_folder);
                        }
                    }
                }
            }

            // Windows / Linux cold-start: the OS passes the file path as a
            // CLI argument when the user opens a .krillnotes file.
            // On macOS this path is empty (files arrive via RunEvent::Opened instead).
            let state_ref = app.state::<AppState>();
            let file_args: Vec<PathBuf> = std::env::args()
                .skip(1)
                .filter_map(|a| {
                    let p = PathBuf::from(&a);
                    if p.exists() { Some(p) } else { None }
                })
                .collect();
            for path in file_args {
                commands::workspace::handle_file_opened(app.handle(), &state_ref, path);
            }

            Ok(())
        })
        .on_menu_event(handle_menu_event)
        .invoke_handler(tauri::generate_handler![
            create_workspace,
            open_workspace,
            get_workspace_info,
            list_notes,
            get_node_types,
            toggle_note_expansion,
            set_selected_note,
            create_note_with_type,
            get_schema_fields,
            get_all_schemas,
            get_tree_action_map,
            invoke_tree_action,
            get_note_view,
            get_note_hover,
            get_views_for_type,
            render_view,
            render_markdown_field,
            get_script_warnings,
            update_note,
            save_note,
            validate_field,
            validate_fields,
            evaluate_group_visibility,
            update_note_tags,
            get_all_tags,
            get_notes_for_tag,
            get_workspace_metadata,
            set_workspace_metadata,
            get_note,
            search_notes,
            count_children,
            delete_note,
            move_note,
            deep_copy_note_cmd,
            set_paste_menu_enabled,
            list_user_scripts,
            get_user_script,
            create_user_script,
            update_user_script,
            delete_user_script,
            toggle_user_script,
            reorder_user_script,
            reorder_all_user_scripts,
            list_operations,
            get_operation_detail,
            purge_operations,
            export_workspace_cmd,
            peek_import_cmd,
            execute_import,
            get_app_version,
            consume_pending_file_open,
            consume_pending_swarm_file,
            get_settings,
            update_settings,
            list_themes,
            read_theme,
            write_theme,
            delete_theme,
            read_file_content,
            list_workspace_files,
            delete_workspace,
            duplicate_workspace,
            is_workspace_owner,
            list_identities,
            resolve_identity_name,
            create_identity,
            unlock_identity,
            lock_identity,
            delete_identity,
            rename_identity,
            change_identity_passphrase,
            get_unlocked_identities,
            is_identity_unlocked,
            get_workspaces_for_identity,
            export_swarmid_cmd,
            get_identity_public_key,
            import_swarmid_cmd,
            import_swarmid_overwrite_cmd,
            open_swarm_file_cmd,
            list_invites,
            create_invite,
            revoke_invite,
            delete_invite,
            delete_revoked_invites,
            import_invite_response,
            import_invite,
            respond_to_invite,
            accept_peer,
            attach_file,
            attach_file_bytes,
            get_attachments,
            get_attachment_data,
            delete_attachment,
            restore_attachment,
            open_attachment,
            undo,
            redo,
            can_undo,
            can_redo,
            get_undo_limit,
            set_undo_limit,
            begin_undo_group,
            end_undo_group,
            script_undo,
            script_redo,
            can_script_undo,
            can_script_redo,
            list_contacts,
            get_contact,
            create_contact,
            update_contact,
            delete_contact,
            get_fingerprint,
            list_workspace_peers,
            get_workspace_peers,
            remove_workspace_peer,
            add_contact_as_peer,
            create_snapshot_for_peers,
            apply_swarm_snapshot,
            apply_swarm_delta,
            generate_deltas_for_peers,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // macOS warm-start and cold-start: the OS delivers file-open events via the
            // NSApplicationDelegate applicationOpenURLs: callback, which Tauri surfaces
            // here as RunEvent::Opened. On Windows and Linux the OS spawns a fresh
            // process instead, so files arrive via std::env::args() in setup().
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                let state = app_handle.state::<AppState>();
                for url in urls {
                    if url.scheme() == "file" {
                        let path = PathBuf::from(url.path());
                        if path.exists() {
                            commands::workspace::handle_file_opened(app_handle, &state, path);
                        }
                    }
                }
            }
            let _ = event;
        });
}
