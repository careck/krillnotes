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

use tauri::Emitter;

// Re-export all public core library types into this crate's namespace.
#[doc(inline)]
pub use krillnotes_core::*;

use uuid::Uuid;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

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

/// Serialisable summary of an open workspace, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    /// File name without extension (used as the window title).
    pub filename: String,
    /// Absolute filesystem path to the `.krillnotes` database file.
    pub path: String,
    /// Total number of notes in the workspace.
    pub note_count: usize,
    /// ID of the note selected when the workspace was last saved, if any.
    pub selected_note_id: Option<String>,
    /// UUID of the identity bound to this workspace, if any.
    pub identity_uuid: Option<String>,
}

/// Information about a workspace bound to an identity, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBindingInfo {
    pub workspace_uuid: String,
    pub folder_path: String,
}

/// Serialisable contact record returned to the frontend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInfo {
    pub contact_id: String,
    pub declared_name: String,
    pub local_name: Option<String>,
    pub public_key: String,
    pub fingerprint: String,
    pub trust_level: String,
    pub first_seen: String,
    pub notes: Option<String>,
}

impl ContactInfo {
    fn from_contact(c: krillnotes_core::core::contact::Contact) -> Self {
        Self {
            contact_id: c.contact_id.to_string(),
            declared_name: c.declared_name,
            local_name: c.local_name,
            public_key: c.public_key,
            fingerprint: c.fingerprint,
            trust_level: trust_level_to_str(&c.trust_level).to_string(),
            first_seen: c.first_seen.to_rfc3339(),
            notes: c.notes,
        }
    }
}

/// Derives a unique window label from the `path` filename stem.
///
/// Appends a numeric suffix (`-2`, `-3`, …) until the label is absent
/// from the currently open workspace labels in `state`.
fn generate_unique_label(state: &AppState, path: &Path) -> String {
    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");

    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let mut label = filename.to_string();
    let mut counter = 2;

    while workspaces.contains_key(&label) {
        label = format!("{filename}-{counter}");
        counter += 1;
    }

    label
}

/// Returns the window label for a workspace already open at `path`, if any.
fn find_window_for_path(state: &AppState, path: &Path) -> Option<String> {
    state.workspace_paths.lock()
        .expect("Mutex poisoned")
        .iter()
        .find(|(_, p)| *p == path)
        .map(|(label, _)| label.clone())
}

/// Brings the window identified by `label` to the foreground.
///
/// # Errors
///
/// Returns an error string if the window does not exist or `set_focus` fails.
fn focus_window(app: &AppHandle, label: &str) -> std::result::Result<(), String> {
    app.get_webview_window(label)
        .ok_or_else(|| "Window not found".to_string())
        .and_then(|window| {
            window.set_focus()
                .map_err(|e| format!("Failed to focus: {e}"))
        })
}

/// Dispatches an OS file-open event to the appropriate handler based on
/// the file extension. Add new file type handlers here as new cases.
fn handle_file_opened(app: &AppHandle, state: &AppState, path: PathBuf) {
    match path.extension().and_then(|e| e.to_str()) {
        Some("krillnotes") => handle_krillnotes_open(app, state, path),
        Some("swarm") => handle_swarm_open(app, state, path),
        _ => {}
    }
}

/// Handles opening a `.krillnotes` file from the OS.
///
/// Stores the path in [`AppState::pending_file_open`] for the cold-start
/// case (frontend not yet ready), then either emits a `"file-opened"` event
/// to the existing `"main"` window or creates a new one that will poll
/// `consume_pending_file_open` on mount.
fn handle_krillnotes_open(app: &AppHandle, state: &AppState, path: PathBuf) {
    {
        let mut pending = state.pending_file_open.lock().expect("Mutex poisoned");
        *pending = Some(path.clone());
    }

    if let Some(win) = app.get_webview_window("main") {
        // App is warm-started and the launcher window is open — JS is listening.
        // Emit the event; the listener calls consume_pending_file_open to dequeue.
        win.emit("file-opened", path.to_string_lossy().to_string()).ok();
    } else {
        // No launcher window — create one. Its mount effect will call
        // consume_pending_file_open and start the import flow.
        create_main_window(app);
    }
}

/// Handles opening a `.swarm` file from the OS.
///
/// Stores the path in [`AppState::pending_file_open`] for the cold-start
/// case (frontend not yet ready), then emits a `"swarm-file-opened"` event
/// to the focused window.
fn handle_swarm_open(app: &AppHandle, state: &AppState, path: PathBuf) {
    // Store path for cold-start retrieval.
    {
        let mut pending = state.pending_file_open.lock().expect("Mutex poisoned");
        *pending = Some(path.clone());
    }
    // Emit to the focused window first; fall back to any open window.
    let target_label = state
        .focused_window
        .lock()
        .expect("Mutex poisoned")
        .clone()
        .unwrap_or_else(|| "main".to_string());

    if let Some(win) = app.get_webview_window(&target_label) {
        win.emit("swarm-file-opened", path.to_string_lossy().to_string()).ok();
    }
}

/// Creates a new launcher ("main") window programmatically.
///
/// Used when the user opens a `.krillnotes` file while all launcher windows
/// have been closed (only workspace windows remain open).
fn create_main_window(app: &AppHandle) {
    let lang = settings::load_settings().language;
    let strings = locales::menu_strings(&lang);
    if let Ok(menu_result) = menu::build_menu(app, &strings) {
        let _ = tauri::WebviewWindowBuilder::new(
            app,
            "main",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Krillnotes")
        .inner_size(800.0, 600.0)
        .disable_drag_drop_handler()
        .menu(menu_result.menu)
        .build();
    }
}

/// Opens a new 1024×768 webview window with the given `label`.
///
/// The menu is built and attached explicitly so that Windows workspace windows
/// get a menu bar. On macOS the app-level menu set in `setup()` is shared
/// globally, but on Windows each window owns its own menu bar and does not
/// inherit the app-level default when created after startup.
///
/// # Errors
///
/// Returns an error string if Tauri fails to build the menu or the window.
fn create_workspace_window(
    app: &AppHandle,
    label: &str,
    caller: &tauri::Window,
) -> std::result::Result<tauri::WebviewWindow, String> {
    let lang = settings::load_settings().language;
    let strings = locales::menu_strings(&lang);
    let menu_result = menu::build_menu(app, &strings)
        .map_err(|e| format!("Failed to build menu: {e}"))?;

    // Enable workspace-specific menu items for this new workspace window.
    // On macOS the menu bar is global, so we update the shared handles stored
    // under "macos". On Windows each window owns its own menu bar, so we
    // enable the items in the freshly-built menu before attaching it.
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<AppState>();
        let items = state.workspace_menu_items.lock().expect("Mutex poisoned");
        if let Some(ws_items) = items.get("macos") {
            for item in ws_items {
                item.set_enabled(true).map_err(|e| format!("Failed to enable menu item: {e}"))?;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Enable workspace items in this window's private menu before attaching it.
        for item in &menu_result.workspace_items {
            item.set_enabled(true).map_err(|e| format!("Failed to enable menu item: {e}"))?;
        }
        // Store the paste handles per window label so set_paste_menu_enabled can find them.
        let state = app.state::<AppState>();
        state.paste_menu_items.lock().expect("Mutex poisoned")
            .insert(label.to_string(), (menu_result.paste_as_child, menu_result.paste_as_sibling));
    }

    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App("index.html".into())
    )
    .title(format!("Krillnotes - {label}"))
    .inner_size(1024.0, 768.0)
    .disable_drag_drop_handler()
    .menu(menu_result.menu);

    // Cascade new windows when opening from an existing workspace window.
    if caller.label() != "main" {
        if let Ok(pos) = caller.outer_position() {
            builder = builder.position((pos.x + 30) as f64, (pos.y + 30) as f64);
        }
    }

    builder.build()
        .map_err(|e| format!("Failed to create window: {e}"))
}

/// Rebuilds and reapplies the native menu for all open windows using `lang`.
///
/// On macOS the menu bar is global: one menu is set on the app and the stored
/// paste/workspace handles in AppState are updated.
/// On Windows each window owns its own menu: every open window gets a freshly
/// built menu, with workspace items pre-enabled for workspace windows.
fn rebuild_menus(app: &AppHandle, state: &AppState, lang: &str) -> std::result::Result<(), String> {
    let strings = locales::menu_strings(lang);

    #[cfg(target_os = "macos")]
    {
        let result = menu::build_menu(app, &strings)
            .map_err(|e| format!("Failed to build menu: {e}"))?;
        app.set_menu(result.menu)
            .map_err(|e| format!("Failed to set menu: {e}"))?;
        state.paste_menu_items.lock().expect("Mutex poisoned")
            .insert("macos".to_string(), (result.paste_as_child, result.paste_as_sibling));
        state.workspace_menu_items.lock().expect("Mutex poisoned")
            .insert("macos".to_string(), result.workspace_items);

        // Re-enable workspace items if any workspace is currently open.
        let any_open = !state.workspace_paths.lock().expect("Mutex poisoned").is_empty();
        if any_open {
            if let Some(items) = state.workspace_menu_items.lock()
                .expect("Mutex poisoned")
                .get("macos")
            {
                for item in items {
                    let _ = item.set_enabled(true);
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Collect workspace labels first to avoid holding the lock while calling Tauri APIs.
        let ws_labels: std::collections::HashSet<String> = state
            .workspace_paths
            .lock()
            .expect("Mutex poisoned")
            .keys()
            .cloned()
            .collect();

        for (label, window) in app.webview_windows() {
            let result = menu::build_menu(app, &strings)
                .map_err(|e| format!("Failed to build menu: {e}"))?;

            if ws_labels.contains(&label) {
                for item in &result.workspace_items {
                    item.set_enabled(true)
                        .map_err(|e| format!("Failed to enable menu item: {e}"))?;
                }
            }

            window
                .set_menu(result.menu)
                .map_err(|e| format!("Failed to set window menu: {e}"))?;

            state.paste_menu_items.lock().expect("Mutex poisoned")
                .insert(label.clone(), (result.paste_as_child, result.paste_as_sibling));
            state.workspace_menu_items.lock().expect("Mutex poisoned")
                .insert(label, result.workspace_items);
        }
    }

    Ok(())
}

/// Inserts `workspace`, its `path`, and its `identity_uuid` into `state` under `label`.
fn store_workspace(
    state: &AppState,
    label: String,
    workspace: Workspace,
    path: PathBuf,
    identity_uuid: Uuid,
) {
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let mut paths = state.workspace_paths.lock()
        .expect("Mutex poisoned");
    let mut identities = state.workspace_identities.lock()
        .expect("Mutex poisoned");

    workspaces.insert(label.clone(), workspace);
    paths.insert(label.clone(), path);
    identities.insert(label, identity_uuid);
}

/// Assembles a [`WorkspaceInfo`] for the workspace registered under `label`.
///
/// # Errors
///
/// Returns an error string if no workspace or path is registered for `label`.
fn get_workspace_info_internal(
    state: &AppState,
    label: &str
) -> std::result::Result<WorkspaceInfo, String> {
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let paths = state.workspace_paths.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get(label)
        .ok_or("No workspace found")?;
    let path = paths.get(label)
        .ok_or("No path found")?;

    let note_count = workspace.list_all_notes()
        .map(|notes| notes.len())
        .unwrap_or(0);

    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    let selected_note_id = workspace.get_selected_note()
        .ok()
        .flatten();

    let identity_uuid = state.identity_manager.lock().expect("Mutex poisoned")
        .get_workspace_binding(path.as_path())
        .ok()
        .flatten()
        .map(|b| b.identity_uuid.to_string());

    Ok(WorkspaceInfo {
        filename,
        path: path.display().to_string(),
        note_count,
        selected_note_id,
        identity_uuid,
    })
}

/// Creates a new workspace folder at `path` and opens it in a new window.
///
/// `path` is the workspace folder path. A `notes.db` file and an
/// `attachments/` subdirectory are created inside it.
#[tauri::command]
async fn create_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    identity_uuid: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let folder = PathBuf::from(&path);

    if folder.exists() {
        return Err("Workspace already exists. Use Open Workspace instead.".to_string());
    }

    match find_window_for_path(&state, &folder) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

            // Generate random DB password (32 bytes, base64-encoded)
            let password: String = {
                use base64::Engine;
                use rand::RngCore;
                let mut bytes = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut bytes);
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            };

            // Get the signing key from the unlocked identity before creating the workspace.
            let signing_key = {
                let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
                let unlocked = identities.get(&uuid)
                    .ok_or_else(|| "Identity is not unlocked".to_string())?;
                Ed25519SigningKey::from_bytes(&unlocked.signing_key.to_bytes())
            };

            let label = generate_unique_label(&state, &folder);
            std::fs::create_dir_all(&folder)
                .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
            let db_path = folder.join("notes.db");
            let workspace = Workspace::create(&db_path, &password, &uuid.to_string(), signing_key)
                .map_err(|e| format!("Failed to create: {e}"))?;

            // Read the workspace_id from the newly created workspace
            let workspace_uuid = workspace.workspace_id().to_string();

            // Bind workspace to identity (encrypt DB password with identity seed)
            {
                let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
                let unlocked = identities.get(&uuid)
                    .ok_or_else(|| "Identity is not unlocked".to_string())?;
                let seed = unlocked.signing_key.to_bytes();
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                mgr.bind_workspace(
                    &uuid,
                    &workspace_uuid,
                    &folder,
                    &password,
                    &seed,
                ).map_err(|e| format!("Failed to bind workspace to identity: {e}"))?;
            }

            let new_window = create_workspace_window(&app, &label, &window)?;
            store_workspace(&state, label.clone(), workspace, folder.clone(), uuid);

            new_window.set_title(&format!("Krillnotes - {label}"))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}

/// Opens an existing workspace folder at `path` in a new window.
///
/// `path` is the workspace folder path. The `notes.db` file inside it is
/// opened by the core library.
#[tauri::command]
async fn open_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let folder = PathBuf::from(&path);

    if !folder.is_dir() {
        return Err("Workspace folder does not exist".to_string());
    }

    match find_window_for_path(&state, &folder) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &folder);
            let db_path = folder.join("notes.db");

            // Read workspace_id from info.json
            let (ws_uuid_opt, _, _, _) = read_info_json_full(&folder);
            // Guard: workspace must have a UUID to be bound; frontend checks for "IDENTITY_REQUIRED"
            ws_uuid_opt.ok_or_else(|| "IDENTITY_REQUIRED".to_string())?;

            // Look up which identity this workspace is bound to and decrypt the DB password.
            // Lock ordering: always acquire identity_manager before unlocked_identities,
            // and drop identity_manager before re-acquiring it — avoids deadlock with
            // create_workspace / execute_import which use the same ordering.
            // contact_managers is always acquired after (and separately from) both of
            // the above; never hold identity_manager or unlocked_identities simultaneously
            // with contact_managers.

            // Step 1: Get identity_uuid from identity_manager (drop lock after)
            let identity_uuid = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let binding = mgr.get_workspace_binding(&folder)
                    .map_err(|e: KrillnotesError| e.to_string())?
                    .ok_or_else(|| "IDENTITY_REQUIRED".to_string())?;
                binding.identity_uuid
                // mgr drops here
            };

            // Step 2: Get signing key from unlocked_identities (drop lock after)
            let seed = {
                let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
                let unlocked = identities.get(&identity_uuid)
                    .ok_or_else(|| format!("IDENTITY_LOCKED:{}", identity_uuid))?;
                unlocked.signing_key.to_bytes()
                // identities drops here
            };

            // Step 3: Decrypt DB password (no other locks held)
            let db_password = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                mgr.decrypt_db_password(&folder, &seed)
                    .map_err(|e| format!("Failed to decrypt DB password: {e}"))?
            };

            let signing_key = Ed25519SigningKey::from_bytes(&seed);
            let mut workspace = Workspace::open(&db_path, &db_password, &identity_uuid.to_string(), signing_key)
                .map_err(|e| match e {
                    KrillnotesError::WrongPassword => "WRONG_PASSWORD".to_string(),
                    KrillnotesError::UnencryptedWorkspace => "UNENCRYPTED_WORKSPACE".to_string(),
                    other => format!("Failed to open: {other}"),
                })?;

            let migration_results = std::mem::take(&mut workspace.pending_migration_results);
            let new_window = create_workspace_window(&app, &label, &window)?;
            store_workspace(&state, label.clone(), workspace, folder.clone(), identity_uuid);

            // Emit one event per migrated schema type so the frontend can show a toast.
            for (schema_name, from_version, to_version, notes_migrated) in &migration_results {
                let _ = new_window.emit("schema-migrated", serde_json::json!({
                    "schemaName": schema_name,
                    "fromVersion": from_version,
                    "toVersion": to_version,
                    "notesMigrated": notes_migrated,
                }));
            }

            new_window.set_title(&format!("Krillnotes - {label}"))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}

/// Returns the [`WorkspaceInfo`] for the calling window's workspace.
#[tauri::command]
fn get_workspace_info(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<WorkspaceInfo, String> {
    get_workspace_info_internal(&state, window.label())
}

/// Returns all notes in the calling window's workspace.
#[tauri::command]
fn list_notes(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<Note>, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_all_notes()
        .map_err(|e| e.to_string())
}

/// Returns the registered note types for the calling window's workspace.
#[tauri::command]
fn get_node_types(
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
fn toggle_note_expansion(
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
fn set_selected_note(
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
async fn create_note_with_type(
    window: tauri::Window,
    state: State<'_, AppState>,
    parent_id: Option<String>,
    position: String,
    schema: String,
) -> std::result::Result<Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    // Convert position string to AddPosition enum
    let add_position = match position.as_str() {
        "child" => AddPosition::AsChild,
        "sibling" => AddPosition::AsSibling,
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


/// Response type for script mutation commands that return a result alongside
/// any script load errors that occurred during the full registry reload.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptMutationResult<T: serde::Serialize> {
    data: T,
    load_errors: Vec<ScriptError>,
}

/// Serializable field definition with an extra `has_validate` flag for the frontend.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldDefInfo {
    name: String,
    field_type: String,
    required: bool,
    can_view: bool,
    can_edit: bool,
    options: Vec<String>,
    max: i64,
    target_schema: Option<String>,
    show_on_hover: bool,
    allowed_types: Vec<String>,
    /// `true` if a `validate` closure is registered for this field.
    has_validate: bool,
}

impl From<&FieldDefinition> for FieldDefInfo {
    fn from(f: &FieldDefinition) -> Self {
        Self {
            name: f.name.clone(),
            field_type: f.field_type.clone(),
            required: f.required,
            can_view: f.can_view,
            can_edit: f.can_edit,
            options: f.options.clone(),
            max: f.max,
            target_schema: f.target_schema.clone(),
            show_on_hover: f.show_on_hover,
            allowed_types: f.allowed_types.clone(),
            has_validate: f.validate.is_some(),
        }
    }
}

/// Serializable field group for the SchemaInfo Tauri response.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldGroupInfo {
    name: String,
    fields: Vec<FieldDefInfo>,
    collapsed: bool,
    has_visible_closure: bool,
}

/// Response type for the `get_schema_fields` Tauri command, bundling field
/// definitions with schema-level title visibility flags.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaInfo {
    fields: Vec<FieldDefInfo>,
    title_can_view: bool,
    title_can_edit: bool,
    children_sort: String,
    allowed_parent_schemas: Vec<String>,
    allowed_children_schemas: Vec<String>,
    allow_attachments: bool,
    attachment_types: Vec<String>,
    has_views: bool,
    has_hover: bool,
    field_groups: Vec<FieldGroupInfo>,
    is_leaf: bool,
}

fn schema_to_info(schema: &Schema, has_views: bool, has_hover: bool) -> SchemaInfo {
    SchemaInfo {
        has_views,
        has_hover,
        fields: schema.fields.iter().map(FieldDefInfo::from).collect(),
        title_can_view: schema.title_can_view,
        title_can_edit: schema.title_can_edit,
        children_sort: schema.children_sort.clone(),
        allowed_parent_schemas: schema.allowed_parent_schemas.clone(),
        allowed_children_schemas: schema.allowed_children_schemas.clone(),
        allow_attachments: schema.allow_attachments,
        attachment_types: schema.attachment_types.clone(),
        field_groups: schema.field_groups.iter().map(|g| FieldGroupInfo {
            name: g.name.clone(),
            fields: g.fields.iter().map(FieldDefInfo::from).collect(),
            collapsed: g.collapsed,
            has_visible_closure: g.visible.is_some(),
        }).collect(),
        is_leaf: schema.is_leaf,
    }
}

/// Returns the field definitions for the schema identified by `schema`.
///
/// Looks up the schema registered under `schema` in the calling window's
/// workspace and returns its list of [`FieldDefinition`] values so the
/// frontend can render an appropriate editing form.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// or if `schema` is not registered in the schema registry.
#[tauri::command]
fn get_schema_fields(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema: String,
) -> std::result::Result<SchemaInfo, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schema_def = workspace.script_registry().get_schema(&schema)
        .map_err(|e: KrillnotesError| e.to_string())?;

    Ok(schema_to_info(
        &schema_def,
        workspace.script_registry().has_views(&schema),
        workspace.script_registry().has_hover(&schema),
    ))
}

/// Returns all schema infos keyed by node type name.
#[tauri::command]
fn get_all_schemas(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, SchemaInfo>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schemas = workspace.script_registry().all_schemas();
    let mut result = HashMap::new();
    for (name, schema) in schemas {
        let has_view_hook = workspace.script_registry().has_views(&name);
        let has_hover_hook = workspace.script_registry().has_hover(&name);
        result.insert(name, schema_to_info(&schema, has_view_hook, has_hover_hook));
    }
    Ok(result)
}

/// Returns a map of `note_type → [action_label, …]` for all registered tree actions.
#[tauri::command]
fn get_tree_action_map(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, Vec<String>>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.tree_action_map())
}

/// Runs the tree action `label` on `note_id`.
#[tauri::command]
fn invoke_tree_action(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    label: String,
) -> std::result::Result<(), String> {
    let window_label = window.label().to_string();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(&window_label).ok_or("No workspace open")?;
    workspace.run_tree_action(&note_id, &label)
        .map_err(|e| e.to_string())
}

/// Returns the custom HTML view for a note generated by its `on_view` hook, if any.
/// Returns the HTML view for a note.
///
/// When an `on_view` Rhai hook is registered for the note's schema the hook
/// generates the HTML; otherwise a default view is generated, with `textarea`
/// fields rendered as CommonMark markdown.
///
/// # Errors
///
/// Returns an error string if no workspace is open or if the hook fails.
#[tauri::command]
fn get_note_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_view_hook(&note_id).map_err(|e| e.to_string())
}

/// Returns the on_hover hook HTML for a note, or `null` if no hook is registered.
#[tauri::command]
fn get_note_hover(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Option<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_hover_hook(&note_id).map_err(|e| e.to_string())
}

/// Returns the list of registered views for a note type.
#[tauri::command]
fn get_views_for_type(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
) -> std::result::Result<Vec<ViewInfo>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    let views = workspace.get_views_for_type(&schema_name);
    Ok(views.iter().map(|v| ViewInfo {
        label: v.label.clone(),
        display_first: v.display_first,
    }).collect())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewInfo {
    label: String,
    display_first: bool,
}

/// Renders a specific named view tab for a note.
#[tauri::command]
fn render_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    view_label: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.render_view(&note_id, &view_label).map_err(|e| e.to_string())
}

/// Returns script warnings (unresolved bindings).
#[tauri::command]
fn get_script_warnings(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<krillnotes_core::ScriptWarning>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.get_script_warnings())
}

/// Updates the title and fields of an existing note, returning the updated note.
///
/// Looks up the note identified by `note_id` in the calling window's workspace,
/// applies the new `title` and `fields` values, persists the changes, and returns
/// the updated [`Note`] so the frontend can reflect the current state immediately.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// if `note_id` does not identify an existing note, or if the underlying
/// storage operation fails.
#[tauri::command]
fn update_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    title: String,
    fields: BTreeMap<String, FieldValue>,
) -> std::result::Result<Note, String> {
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
///
/// Returns `SaveResult::Ok(note)` on success or `SaveResult::ValidationErrors`
/// when any validation step fails.
#[tauri::command]
fn save_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    title: String,
    fields: BTreeMap<String, FieldValue>,
) -> std::result::Result<SaveResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.save_note_with_pipeline(&note_id, title, fields)
        .map_err(|e| e.to_string())
}

/// Runs the `validate` closure for a single field.
///
/// Returns `None` when the field is valid or has no validate closure.
/// Returns `Some(error_message)` when the closure returns an error.
#[tauri::command]
fn validate_field(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
    field_name: String,
    value: serde_json::Value,
) -> std::result::Result<Option<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    let fv: FieldValue = serde_json::from_value(value).map_err(|e| e.to_string())?;
    workspace.script_registry()
        .validate_field(&schema_name, &field_name, &fv)
        .map_err(|e| e.to_string())
}

/// Runs `validate` closures for all fields that have them.
///
/// Returns a map of `field_name → error_message` for each invalid field.
#[tauri::command]
fn validate_fields(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
    fields: BTreeMap<String, FieldValue>,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    workspace.script_registry()
        .validate_fields(&schema_name, &fields)
        .map_err(|e| e.to_string())
}

/// Evaluates `visible` closures for each `FieldGroup`.
///
/// Returns a map of `group_name → bool`.
#[tauri::command]
fn evaluate_group_visibility(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
    fields: BTreeMap<String, FieldValue>,
) -> std::result::Result<BTreeMap<String, bool>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    workspace.script_registry()
        .evaluate_group_visibility(&schema_name, &fields)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_note_tags(
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
fn get_all_tags(
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
fn get_workspace_metadata(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<WorkspaceMetadata, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_workspace_metadata()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_workspace_metadata(
    window: tauri::Window,
    state: State<'_, AppState>,
    metadata: WorkspaceMetadata,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.set_workspace_metadata(&metadata)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_notes_for_tag(
    window: tauri::Window,
    state: State<'_, AppState>,
    tags: Vec<String>,
) -> std::result::Result<Vec<Note>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_notes_for_tag(&tags)
        .map_err(|e| e.to_string())
}

/// Returns a single note by ID from the calling window's workspace.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// or if no note with the given ID exists.
#[tauri::command]
fn get_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Note, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_note(&note_id)
        .map_err(|e| e.to_string())
}

/// Searches for notes in the calling window's workspace whose title or
/// text-like field values contain `query` (case-insensitive substring match).
///
/// If `target_type` is `Some`, only notes of that schema type are included.
/// Returns an empty array when `query` is blank.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// or if the underlying SQLite query fails.
#[tauri::command]
fn search_notes(
    window: tauri::Window,
    state: State<'_, AppState>,
    query: String,
    target_schema: Option<String>,
) -> std::result::Result<Vec<NoteSearchResult>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.search_notes(&query, target_schema.as_deref())
        .map_err(|e| e.to_string())
}

/// Returns the number of direct children of the note identified by `note_id`.
///
/// Queries the calling window's workspace for the count of notes whose
/// `parent_id` matches `note_id`. The count is zero when `note_id` has no
/// children; the note itself does not need to be expanded for this query to
/// succeed.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// or if the underlying SQLite query fails.
#[tauri::command]
fn count_children(
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
///
/// Dispatches to either recursive deletion or child-promotion depending on
/// `strategy`:
///
/// - `"DeleteAll"` — removes `note_id` and every descendant in one atomic
///   transaction. The returned [`DeleteResult`] includes all deleted IDs.
/// - `"PromoteChildren"` — removes only `note_id` and re-parents its direct
///   children to the deleted note's former parent. `deleted_count` is always 1.
///
/// The `strategy` value is deserialised from the PascalCase string sent by the
/// frontend (`"DeleteAll"` or `"PromoteChildren"`).
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// if `note_id` does not exist (for `PromoteChildren`), or if any SQLite
/// operation fails.
#[tauri::command]
fn delete_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    strategy: DeleteStrategy,
) -> std::result::Result<DeleteResult, String> {
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
fn move_note(
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
fn deep_copy_note_cmd(
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
        AddPosition::AsChild
    } else {
        AddPosition::AsSibling
    };
    ws.deep_copy_note(&source_note_id, &target_note_id, pos)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_paste_menu_enabled(
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

// ── User-script commands ──────────────────────────────────────────

/// Returns all user scripts for the calling window's workspace.
#[tauri::command]
fn list_user_scripts(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<UserScript>, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_user_scripts()
        .map_err(|e| e.to_string())
}

/// Returns a single user script by ID.
#[tauri::command]
fn get_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
) -> std::result::Result<UserScript, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_user_script(&script_id)
        .map_err(|e| e.to_string())
}

/// Creates a new user script from source code.
#[tauri::command]
fn create_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    source_code: String,
    category: Option<String>,
) -> std::result::Result<ScriptMutationResult<UserScript>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    let (data, load_errors) = match category {
        Some(cat) => workspace.create_user_script_with_category(&source_code, &cat),
        None => workspace.create_user_script(&source_code),
    }.map_err(|e| e.to_string())?;
    Ok(ScriptMutationResult { data, load_errors })
}

/// Updates an existing user script's source code.
#[tauri::command]
fn update_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    source_code: String,
) -> std::result::Result<ScriptMutationResult<UserScript>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    let (data, load_errors) = workspace.update_user_script(&script_id, &source_code)
        .map_err(|e| e.to_string())?;
    Ok(ScriptMutationResult { data, load_errors })
}

/// Deletes a user script by ID.
#[tauri::command]
fn delete_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
) -> std::result::Result<Vec<ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.delete_user_script(&script_id)
        .map_err(|e| e.to_string())
}

/// Toggles the enabled state of a user script.
#[tauri::command]
fn toggle_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    enabled: bool,
) -> std::result::Result<Vec<ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.toggle_user_script(&script_id, enabled)
        .map_err(|e| e.to_string())
}

/// Changes the load order of a user script.
#[tauri::command]
fn reorder_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    new_load_order: i32,
) -> std::result::Result<Vec<ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.reorder_user_script(&script_id, new_load_order)
        .map_err(|e| e.to_string())
}

/// Reassigns sequential load order to all scripts given in order, then reloads.
#[tauri::command]
fn reorder_all_user_scripts(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_ids: Vec<String>,
) -> std::result::Result<Vec<ScriptError>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.reorder_all_user_scripts(&script_ids)
        .map_err(|e| e.to_string())
}

// ── Operations log commands ──────────────────────────────────────

/// Returns operation summaries matching the given filters.
#[tauri::command]
fn list_operations(
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
    // We track which indices are already resolved so pass 2 doesn't clobber them.
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
    } // identity_manager lock released here

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
                // Unknown key: show first 8 chars of base64 as a compact fingerprint.
                summary.author_key = summary.author_key.chars().take(8).collect();
            }
        }
    } // contact_managers lock released here

    Ok(summaries)
}

/// Returns the full JSON payload for a single operation by ID.
#[tauri::command]
fn get_operation_detail(
    window: tauri::Window,
    state: State<'_, AppState>,
    operation_id: String,
) -> std::result::Result<serde_json::Value, String> {
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
fn purge_operations(
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

// ── Export / Import commands ──────────────────────────────────────

/// Exports the calling window's workspace as a zip archive at `path`.
#[tauri::command]
fn export_workspace_cmd(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
) -> std::result::Result<(), String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    export_workspace(workspace, file, password.as_deref()).map_err(|e| e.to_string())
}

/// Reads metadata from an export archive without creating a workspace.
#[tauri::command]
fn peek_import_cmd(
    zip_path: String,
    password: Option<String>,
) -> std::result::Result<ImportResult, String> {
    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    peek_import(reader, password.as_deref()).map_err(|e| match e {
        ExportError::EncryptedArchive => "ENCRYPTED_ARCHIVE".to_string(),
        ExportError::InvalidPassword => "INVALID_PASSWORD".to_string(),
        other => other.to_string(),
    })
}

/// Imports an export archive into a new workspace folder and opens it in a new window.
///
/// `folder_path` is the destination workspace folder. A `notes.db` file is
/// written inside it by the import, and an `attachments/` subdirectory is
/// created alongside it.
///
/// `identity_uuid` is the UUID of the unlocked identity that will own the
/// imported workspace. A random DB password is generated, used to re-encrypt
/// the imported database, and then encrypted into the identity store so it can
/// be recovered on future opens.
#[tauri::command]
async fn execute_import(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    folder_path: String,
    password: Option<String>,
    identity_uuid: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let folder = PathBuf::from(&folder_path);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
    let db_path_buf = folder.join("notes.db");

    // Generate a random DB password for the imported workspace.
    let workspace_password: String = {
        use base64::Engine;
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    };

    // Extract the signing key from the unlocked identity before opening the workspace.
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let import_seed = {
        let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
        let unlocked = identities.get(&uuid)
            .ok_or_else(|| "Identity is not unlocked".to_string())?;
        unlocked.signing_key.to_bytes()
    };

    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    import_workspace(reader, &db_path_buf, password.as_deref(), &workspace_password, &uuid.to_string(), Ed25519SigningKey::from_bytes(&import_seed))
        .map_err(|e| e.to_string())?;

    // Ensure the attachments directory exists after import
    let _ = std::fs::create_dir_all(folder.join("attachments"));

    let import_signing_key = Ed25519SigningKey::from_bytes(&import_seed);
    let workspace = Workspace::open(&db_path_buf, &workspace_password, &uuid.to_string(), import_signing_key)
        .map_err(|e| e.to_string())?;

    // Bind the imported workspace to the chosen identity so it can be opened later.
    let workspace_uuid = workspace.workspace_id().to_string();
    {
        let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
        let unlocked = identities.get(&uuid)
            .ok_or_else(|| "Identity is not unlocked".to_string())?;
        let seed = unlocked.signing_key.to_bytes();
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.bind_workspace(
            &uuid,
            &workspace_uuid,
            &folder,
            &workspace_password,
            &seed,
        ).map_err(|e| format!("Failed to bind workspace to identity: {e}"))?;
    }

    let label = generate_unique_label(&state, &folder);

    let new_window = create_workspace_window(&app, &label, &window)?;
    store_workspace(&state, label.clone(), workspace, folder, uuid);

    new_window.set_title(&format!("Krillnotes - {label}"))
        .map_err(|e| e.to_string())?;

    if window.label() == "main" {
        window.close().map_err(|e| e.to_string())?;
    }

    get_workspace_info_internal(&state, &label)
}

/// Returns the application version string from the core crate.
#[tauri::command]
fn get_app_version() -> String {
    APP_VERSION.to_string()
}

/// Dequeues and returns the pending file path that arrived via OS file-open
/// before the frontend was ready. Returns `None` when no file is pending.
///
/// This is the pull-side of the cold-start delivery mechanism. The frontend
/// calls this once on mount (main window only) so that files opened at app
/// launch are handled even if the `"file-opened"` push event fired before
/// the JS listener was registered.
#[tauri::command]
fn consume_pending_file_open(state: State<'_, AppState>) -> Option<String> {
    state
        .pending_file_open
        .lock()
        .expect("Mutex poisoned")
        .take()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Drain the pending .swarm file path stored for cold-start handling.
///
/// Returns `Some(path)` only when the pending path ends with `.swarm`.
/// Returns `None` when no swarm file is pending.
#[tauri::command]
fn consume_pending_swarm_file(state: State<'_, AppState>) -> Option<String> {
    state
        .pending_file_open
        .lock()
        .expect("Mutex poisoned")
        .take()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| p.ends_with(".swarm"))
}

// ── Identity commands ─────────────────────────────────────────────

/// Lists all registered identities.
#[tauri::command]
fn list_identities(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<IdentityRef>, String> {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.list_identities().map_err(|e| e.to_string())
}

/// Resolves a public key to a display name.
/// Checks local identities first, then the contacts address book.
/// Returns a truncated fingerprint (first 8 chars) if the key is unknown but non-empty,
/// or None if the key is empty.
#[tauri::command]
fn resolve_identity_name(
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
fn create_identity(
    state: State<'_, AppState>,
    display_name: String,
    passphrase: String,
) -> std::result::Result<IdentityRef, String> {
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
    let contacts_dir = settings::config_dir()
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
    let invites_dir = settings::config_dir()
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
fn unlock_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
    passphrase: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let unlocked = mgr.unlock_identity(&uuid, &passphrase)
        .map_err(|e| match e {
            KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        })?;
    drop(mgr);
    // Derive contacts key before consuming `unlocked` via insert
    let contacts_key = unlocked.contacts_key();
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .insert(uuid, unlocked);
    // Create per-identity ContactManager (decrypts contacts into memory)
    let contacts_dir = settings::config_dir()
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
    let invites_dir = settings::config_dir()
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
fn lock_identity(
    app: AppHandle,
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Find and close all workspace windows belonging to this identity
    let workspace_base_dir = PathBuf::from(&settings::load_settings().workspace_directory);
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
fn delete_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Must be locked first
    let is_unlocked = state.unlocked_identities.lock().expect("Mutex poisoned").contains_key(&uuid);
    if is_unlocked {
        return Err("Lock the identity before deleting it".to_string());
    }

    let workspace_base_dir = PathBuf::from(&settings::load_settings().workspace_directory);
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.delete_identity(&uuid, &workspace_base_dir).map_err(|e| e.to_string())
}

/// Renames an identity.
#[tauri::command]
fn rename_identity(
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
fn change_identity_passphrase(
    state: State<'_, AppState>,
    identity_uuid: String,
    old_passphrase: String,
    new_passphrase: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.change_passphrase(&uuid, &old_passphrase, &new_passphrase)
        .map_err(|e| match e {
            KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        })
}

/// Returns the UUIDs of all currently unlocked identities.
#[tauri::command]
fn get_unlocked_identities(
    state: State<'_, AppState>,
) -> Vec<String> {
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .keys()
        .map(|uuid| uuid.to_string())
        .collect()
}

/// Returns true if the given identity is currently unlocked.
#[tauri::command]
fn is_identity_unlocked(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> bool {
    Uuid::parse_str(&identity_uuid)
        .map(|uuid| state.unlocked_identities.lock().expect("Mutex poisoned").contains_key(&uuid))
        .unwrap_or(false)
}

/// Returns the workspaces bound to the given identity.
#[tauri::command]
fn get_workspaces_for_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<WorkspaceBindingInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let workspace_base_dir = PathBuf::from(&settings::load_settings().workspace_directory);
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
fn export_swarmid_cmd(
    state: State<'_, AppState>,
    identity_uuid: String,
    passphrase: String,
    path: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let swarmid = mgr.export_swarmid(&uuid, &passphrase).map_err(|e| {
        match e {
            KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        }
    })?;
    let json = serde_json::to_string_pretty(&swarmid).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentityKeyInfo {
    public_key: String,
    fingerprint: String,
}

/// Return the Base64-encoded Ed25519 public key and 4-word fingerprint for the given identity.
/// No passphrase required — the public key is stored unencrypted on disk.
#[tauri::command]
fn get_identity_public_key(
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
fn import_swarmid_cmd(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<IdentityRef, String> {
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file: SwarmIdFile = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid .swarmid file: {e}"))?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.import_swarmid(file).map_err(|e| {
        match e {
            KrillnotesError::IdentityAlreadyExists(uuid) => format!("IDENTITY_EXISTS:{uuid}"),
            other => other.to_string(),
        }
    })
}

/// Import a `.swarmid` file, overwriting any existing identity with the same UUID.
#[tauri::command]
fn import_swarmid_overwrite_cmd(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<IdentityRef, String> {
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file: SwarmIdFile = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid .swarmid file: {e}"))?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.import_swarmid_overwrite(file).map_err(|e| e.to_string())
}

// ── Contact commands ──────────────────────────────────────────────

fn parse_trust_level(s: &str) -> std::result::Result<krillnotes_core::core::contact::TrustLevel, String> {
    use krillnotes_core::core::contact::TrustLevel;
    match s {
        "Tofu" => Ok(TrustLevel::Tofu),
        "CodeVerified" => Ok(TrustLevel::CodeVerified),
        "Vouched" => Ok(TrustLevel::Vouched),
        "VerifiedInPerson" => Ok(TrustLevel::VerifiedInPerson),
        other => Err(format!("Unknown trust level: {other}")),
    }
}

/// Explicit string mapping — do NOT use `format!("{:?}", ...)` which is fragile.
fn trust_level_to_str(tl: &krillnotes_core::core::contact::TrustLevel) -> &'static str {
    use krillnotes_core::core::contact::TrustLevel;
    match tl {
        TrustLevel::Tofu => "Tofu",
        TrustLevel::CodeVerified => "CodeVerified",
        TrustLevel::Vouched => "Vouched",
        TrustLevel::VerifiedInPerson => "VerifiedInPerson",
    }
}

#[tauri::command]
fn list_contacts(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<ContactInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let contacts = cm.list_contacts().map_err(|e| e.to_string())?;
    Ok(contacts.into_iter().map(ContactInfo::from_contact).collect())
}

#[tauri::command]
fn get_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> std::result::Result<Option<ContactInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cid = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let contact = cm.get_contact(cid).map_err(|e| e.to_string())?;
    Ok(contact.map(ContactInfo::from_contact))
}

#[tauri::command]
fn create_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    declared_name: String,
    public_key: String,
    trust_level: String,
) -> std::result::Result<ContactInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let tl = parse_trust_level(&trust_level)?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let contact = cm.create_contact(&declared_name, &public_key, tl)
        .map_err(|e| e.to_string())?;
    Ok(ContactInfo::from_contact(contact))
}

#[tauri::command]
fn update_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
    local_name: Option<String>,
    notes: Option<String>,
    trust_level: String,
) -> std::result::Result<ContactInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cid = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;
    let tl = parse_trust_level(&trust_level)?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    let mut contact = cm.get_contact(cid)
        .map_err(|e| e.to_string())?
        .ok_or("Contact not found")?;
    contact.local_name = local_name;
    contact.notes = notes;
    contact.trust_level = tl;
    cm.save_contact(&contact).map_err(|e| e.to_string())?;
    Ok(ContactInfo::from_contact(contact))
}

#[tauri::command]
fn delete_contact(
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let cid = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;
    let cms = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
    cm.delete_contact(cid).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_fingerprint(public_key: String) -> std::result::Result<String, String> {
    krillnotes_core::core::contact::generate_fingerprint(&public_key)
        .map_err(|e| e.to_string())
}

// ── Peer commands ─────────────────────────────────────────────────

/// Returns all sync peers registered for the calling window's workspace,
/// enriching each entry with the matching contact name where available.
#[tauri::command]
fn list_workspace_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<krillnotes_core::PeerInfo>, String> {
    let window_label = window.label().to_string();

    // Resolve identity UUID from workspace binding.
    let identity_uuid = {
        let paths = state.workspace_paths.lock().expect("Mutex poisoned");
        let folder = paths.get(&window_label).ok_or("Workspace path not found")?.clone();
        drop(paths);
        let (ws_uuid_opt, _, _, _) = read_info_json_full(&folder);
        // Guard: workspace must have a UUID in info.json to be valid
        ws_uuid_opt.ok_or("Workspace UUID missing from info.json")?;
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.get_workspace_binding(&folder)
            .map_err(|e| e.to_string())?
            .ok_or("No identity bound to this workspace")?
            .identity_uuid
    };

    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
    let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = contact_managers
        .get(&identity_uuid)
        .ok_or("Contact manager not found — identity must be unlocked")?;
    workspace.list_peers_info(cm).map_err(|e| e.to_string())
}

/// Returns resolved peer info for the current workspace's sync_peers.
/// Used to populate the CreateDeltaDialog peer checklist.
#[tauri::command]
fn get_workspace_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<krillnotes_core::core::peer_registry::PeerInfo>, String> {
    let identity_uuid_str = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        ws.identity_uuid().to_string()
    };
    let identity_uuid = Uuid::parse_str(&identity_uuid_str).map_err(|e| e.to_string())?;
    let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
    let cm = cm_guard.get(&identity_uuid).ok_or("Contact manager not available")?;
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
    ws.list_peers_info(cm).map_err(|e| e.to_string())
}

/// Removes a sync peer from the calling window's workspace by device ID.
#[tauri::command]
fn remove_workspace_peer(
    window: tauri::Window,
    state: State<'_, AppState>,
    peer_device_id: String,
) -> std::result::Result<(), String> {
    let window_label = window.label().to_string();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
    workspace.remove_peer(&peer_device_id).map_err(|e| e.to_string())
}

/// Pre-authorises a contact as a sync peer for the calling window's workspace.
/// Returns the newly created `PeerInfo` entry.
#[tauri::command]
fn add_contact_as_peer(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> std::result::Result<krillnotes_core::PeerInfo, String> {
    let window_label = window.label().to_string();
    let identity_uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let contact_id = Uuid::parse_str(&contact_id).map_err(|e| e.to_string())?;

    // Step 1: Get contact's public key — drop contact_managers lock before touching workspaces.
    let peer_identity_id = {
        let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
        let cm = contact_managers
            .get(&identity_uuid)
            .ok_or("Contact manager not found — identity must be unlocked")?;
        let contact = cm
            .get_contact(contact_id)
            .map_err(|e| e.to_string())?
            .ok_or("Contact not found")?;
        contact.public_key.clone()
    };

    // Step 2: Add to workspace — acquire workspaces lock separately.
    {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
        workspace.add_contact_as_peer(&peer_identity_id).map_err(|e| e.to_string())?;
    }

    // Step 3: Build PeerInfo for the caller — re-acquire both locks.
    let peers = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
        let contact_managers = state.contact_managers.lock().expect("Mutex poisoned");
        let cm = contact_managers
            .get(&identity_uuid)
            .ok_or("Contact manager not found")?;
        workspace.list_peers_info(cm).map_err(|e| e.to_string())?
    };
    peers
        .into_iter()
        .find(|p| p.peer_identity_id == peer_identity_id)
        .ok_or_else(|| "Peer not found after insert".to_string())
}

// ── Theme commands ────────────────────────────────────────────────

/// Lists all user theme files in the themes directory.
#[tauri::command]
fn list_themes() -> std::result::Result<Vec<themes::ThemeMeta>, String> {
    themes::list_themes()
}

/// Returns the raw JSON content of a theme file.
#[tauri::command]
fn read_theme(filename: String) -> std::result::Result<String, String> {
    themes::read_theme(&filename)
}

/// Writes (creates or overwrites) a theme file.
#[tauri::command]
fn write_theme(filename: String, content: String) -> std::result::Result<(), String> {
    themes::write_theme(&filename, &content)
}

/// Deletes a theme file.
#[tauri::command]
fn delete_theme(filename: String) -> std::result::Result<(), String> {
    themes::delete_theme(&filename)
}

/// Reads and returns the text content of the file at `path`.
/// Only `.rhai` and `.krilltheme` files are allowed.
/// Returns an error string if the extension is not permitted, the file does
/// not exist, or cannot be read.
fn read_file_content_impl(path: &str) -> std::result::Result<String, String> {
    let allowed = path.ends_with(".rhai") || path.ends_with(".krilltheme");
    if !allowed {
        return Err(format!("Only .rhai and .krilltheme files may be imported: {path}"));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Reads and returns the full text of a user-selected import file.
/// Accepts only `.rhai` and `.krilltheme` files.
#[tauri::command]
fn read_file_content(path: String) -> std::result::Result<String, String> {
    read_file_content_impl(&path)
}

// ── Undo / Redo commands ──────────────────────────────────────────

/// Undoes the most recent workspace mutation.
/// Returns the note_id to re-select, or null if not applicable.
#[tauri::command]
fn undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.undo().map_err(|e| e.to_string())
}

/// Re-applies the most recently undone mutation.
#[tauri::command]
fn redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.redo().map_err(|e| e.to_string())
}

/// Returns true if there is an action to undo.
#[tauri::command]
fn can_undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_undo()).unwrap_or(false)
}

/// Returns true if there is an action to redo.
#[tauri::command]
fn can_redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_redo()).unwrap_or(false)
}

/// Undoes the most recent script mutation (isolated from the note undo stack).
#[tauri::command]
fn script_undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.script_undo().map_err(|e| e.to_string())
}

/// Re-applies the most recently undone script mutation.
#[tauri::command]
fn script_redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<UndoResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace.script_redo().map_err(|e| e.to_string())
}

/// Returns true if there is a script action to undo.
#[tauri::command]
fn can_script_undo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_script_undo()).unwrap_or(false)
}

/// Returns true if there is a script action to redo.
#[tauri::command]
fn can_script_redo(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> bool {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    workspaces.get(label).map(|ws| ws.can_script_redo()).unwrap_or(false)
}

/// Returns the workspace undo history limit.
#[tauri::command]
fn get_undo_limit(
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
fn set_undo_limit(
    window: tauri::Window,
    state: State<'_, AppState>,
    limit: usize,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces.get_mut(label).ok_or("No workspace open")?;
    ws.set_undo_limit(limit).map_err(|e| e.to_string())
}

/// Opens an undo group. Subsequent mutations accumulate in a staging buffer
/// until `end_undo_group` is called, collapsing them into a single undo step.
/// Nested calls are ignored — the outermost begin/end pair wins.
#[tauri::command]
fn begin_undo_group(
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
/// No-op if no group is open or the buffer is empty.
#[tauri::command]
fn end_undo_group(
    window: tauri::Window,
    state: State<'_, AppState>,
) {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    if let Some(ws) = workspaces.get_mut(label) {
        ws.end_undo_group();
    }
}

// ── Settings commands ─────────────────────────────────────────────

/// Returns the current application settings.
#[tauri::command]
fn get_settings() -> std::result::Result<settings::AppSettings, String> {
    Ok(settings::load_settings())
}

/// Updates and persists the application settings.
///
/// Accepts a partial JSON object and merges it onto the current settings on
/// disk, so callers only need to supply the fields they care about — missing
/// fields are preserved rather than reset to defaults.
#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> std::result::Result<(), String> {
    let current = settings::load_settings();
    let old_lang = current.language.clone();

    let mut current_value = serde_json::to_value(&current)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    if let (serde_json::Value::Object(curr), serde_json::Value::Object(p)) =
        (&mut current_value, patch)
    {
        for (k, v) in p {
            curr.insert(k, v);
        }
    }
    let updated: settings::AppSettings = serde_json::from_value(current_value)
        .map_err(|e| format!("Failed to deserialize merged settings: {e}"))?;
    settings::save_settings(&updated)?;

    if updated.language != old_lang {
        rebuild_menus(&app, &state, &updated.language)?;
    }

    Ok(())
}

/// Entry returned by [`list_workspace_files`], representing a workspace folder
/// (containing `notes.db`) in the configured workspace directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    /// Folder name (used as the workspace display name).
    name: String,
    /// Absolute path to the workspace folder.
    path: String,
    /// Whether this workspace is currently open in a window.
    is_open: bool,
    /// Unix timestamp (seconds) of the workspace folder's last modification.
    last_modified: i64,
    /// Total size in bytes: notes.db + attachments/ directory combined.
    size_bytes: u64,
    /// From info.json: root note's created_at. None if info.json is missing.
    created_at: Option<i64>,
    /// From info.json: number of notes excluding the root. None if info.json is missing.
    note_count: Option<usize>,
    /// From info.json: number of attachments. None if info.json is missing.
    attachment_count: Option<usize>,
    /// From info.json: stable UUID assigned to this workspace. None if info.json is missing.
    workspace_uuid: Option<String>,
    /// UUID of the identity this workspace is bound to, if any.
    identity_uuid: Option<String>,
    /// Display name of the bound identity, if any and if its file is readable.
    identity_name: Option<String>,
}

/// Returns the total size in bytes of all files under `dir` (recursive).
fn dir_size_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                total += dir_size_bytes(&p);
            }
        }
    }
    total
}

/// Reads `info.json` from `workspace_dir` and returns all stored fields.
/// Returns `(None, None, None, None)` if the file is missing or malformed.
fn read_info_json_full(workspace_dir: &Path) -> (Option<String>, Option<i64>, Option<usize>, Option<usize>) {
    let path = workspace_dir.join("info.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (None, None, None, None),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None),
    };
    let workspace_id = v["workspace_id"].as_str().map(|s| s.to_string());
    let created_at = v["created_at"].as_i64();
    let note_count = v["note_count"].as_u64().map(|n| n as usize);
    let attachment_count = v["attachment_count"].as_u64().map(|n| n as usize);
    (workspace_id, created_at, note_count, attachment_count)
}

/// Reads `info.json` from `workspace_dir` and returns `(created_at, note_count, attachment_count)`.
/// Returns `(None, None, None)` if the file is missing or malformed.
fn read_info_json(workspace_dir: &Path) -> (Option<i64>, Option<usize>, Option<usize>) {
    let (_, created_at, note_count, attachment_count) = read_info_json_full(workspace_dir);
    (created_at, note_count, attachment_count)
}

/// Lists all workspace folders (subdirectories containing `notes.db`) in the
/// configured workspace directory.
///
/// Each entry includes an `is_open` flag indicating whether the workspace
/// is currently open in a window, so the frontend can grey those out.
#[tauri::command]
fn list_workspace_files(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<WorkspaceEntry>, String> {
    let app_settings = settings::load_settings();
    let dir = PathBuf::from(&app_settings.workspace_directory);

    // Create the directory if it doesn't exist yet
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
    }

    // Build path → label map for open workspaces.
    // Collected as an owned HashMap so the lock is released before we
    // later lock state.workspaces to refresh info.json.
    let open_labels: HashMap<PathBuf, String> = state
        .workspace_paths
        .lock()
        .expect("Mutex poisoned")
        .iter()
        .map(|(label, path)| (path.clone(), label.clone()))
        .collect();

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&dir)
        .map_err(|e| format!("Failed to read directory: {e}"))?;

    for entry in read_dir.flatten() {
        let folder = entry.path();
        if !folder.is_dir() { continue; }
        let db_file = folder.join("notes.db");
        if !db_file.exists() { continue; }
        if let Some(name) = folder.file_name().and_then(|s| s.to_str()) {
            let is_open = open_labels.contains_key(&folder);

            // For open workspaces, refresh info.json from the live workspace
            // object so that notes created since open() are counted correctly.
            if let Some(label) = open_labels.get(&folder) {
                if let Some(ws) = state.workspaces.lock().expect("Mutex poisoned").get(label) {
                    let _ = ws.write_info_json();
                }
            }
            let last_modified = std::fs::metadata(&folder)
                .and_then(|m| m.modified())
                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64)
                .unwrap_or(0);
            let size_bytes = dir_size_bytes(&folder);
            let (workspace_id, created_at, note_count, attachment_count) = read_info_json_full(&folder);

            // Look up identity binding for this workspace
            let (identity_uuid, identity_name) = if workspace_id.is_some() {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                if let Ok(Some(binding)) = mgr.get_workspace_binding(&folder) {
                    let identities = mgr.list_identities().unwrap_or_default();
                    let identity = identities.iter().find(|i| i.uuid == binding.identity_uuid);
                    (
                        Some(binding.identity_uuid.to_string()),
                        identity.map(|i| i.display_name.clone()),
                    )
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            entries.push(WorkspaceEntry {
                name: name.to_string(),
                path: folder.display().to_string(),
                is_open,
                last_modified,
                size_bytes,
                created_at,
                note_count,
                attachment_count,
                workspace_uuid: workspace_id,
                identity_uuid,
                identity_name,
            });
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
}

/// Permanently deletes a workspace folder and all its contents.
/// Returns an error if the workspace is currently open in any window.
#[tauri::command]
fn delete_workspace(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<(), String> {
    let folder = PathBuf::from(&path);

    let is_open = state
        .workspace_paths
        .lock()
        .expect("Mutex poisoned")
        .values()
        .any(|p| *p == folder);

    if is_open {
        return Err("Close the workspace before deleting it.".to_string());
    }

    std::fs::remove_dir_all(&folder)
        .map_err(|e| format!("Failed to delete workspace: {e}"))
}

/// Duplicates a workspace by exporting it to a temp file and importing it
/// under a new name in the same workspace directory.
/// Derives the source DB password from the identity binding and assigns a new
/// random DB password to the duplicate, binding it to the same identity.
/// Does NOT open the duplicated workspace in a window — just creates it on disk.
#[tauri::command]
fn duplicate_workspace(
    state: State<'_, AppState>,
    source_path: String,
    identity_uuid: String,
    new_name: String,
) -> std::result::Result<(), String> {
    let app_settings = settings::load_settings();
    let workspace_dir = PathBuf::from(&app_settings.workspace_directory);
    let dest_folder = workspace_dir.join(&new_name);

    if dest_folder.exists() {
        return Err(format!("A workspace named '{new_name}' already exists."));
    }

    let source_folder = PathBuf::from(&source_path);
    let source_db = source_folder.join("notes.db");

    // Decrypt the source DB password via identity.
    // Lock ordering: identity_manager then unlocked_identities, never both held simultaneously.
    let source_password = {
        let (ws_uuid_opt, _, _, _) = read_info_json_full(&source_folder);
        // Guard: source workspace must have a UUID in info.json
        ws_uuid_opt.ok_or_else(|| "Source workspace has no UUID in info.json".to_string())?;

        // Step 1: Get identity_uuid from identity_manager (drop lock after)
        let identity_uuid = {
            let mgr = state.identity_manager.lock().expect("Mutex poisoned");
            let binding = mgr
                .get_workspace_binding(&source_folder)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Source workspace is not bound to any identity".to_string())?;
            binding.identity_uuid
            // mgr drops here
        };

        // Step 2: Get signing key from unlocked_identities (drop lock after)
        let seed = {
            let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
            let unlocked = identities
                .get(&identity_uuid)
                .ok_or_else(|| format!("IDENTITY_LOCKED:{}", identity_uuid))?;
            unlocked.signing_key.to_bytes()
            // identities drops here
        };

        // Step 3: Decrypt DB password (no other locks held)
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.decrypt_db_password(&source_folder, &seed)
            .map_err(|e| format!("Failed to decrypt source password: {e}"))?
    };

    // Generate a new random DB password for the duplicate
    let new_password: String = {
        use base64::Engine;
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    };

    // Derive the signing key for the copy operation (identity_uuid is the function parameter).
    let copy_uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let copy_seed = {
        let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
        let unlocked = identities
            .get(&copy_uuid)
            .ok_or_else(|| format!("IDENTITY_LOCKED:{}", identity_uuid))?;
        unlocked.signing_key.to_bytes()
    };

    // Open the source workspace and export to a temp file.
    let workspace = Workspace::open(&source_db, &source_password, &identity_uuid, Ed25519SigningKey::from_bytes(&copy_seed))
        .map_err(|e| e.to_string())?;

    let mut tmp_file = tempfile::tempfile()
        .map_err(|e| format!("Failed to create temp file: {e}"))?;
    export_workspace(&workspace, &mut tmp_file, Some(&source_password))
        .map_err(|e| e.to_string())?;

    // Import from temp file into dest folder
    std::fs::create_dir_all(&dest_folder)
        .map_err(|e| format!("Failed to create destination: {e}"))?;
    let dest_db = dest_folder.join("notes.db");

    use std::io::Seek;
    tmp_file
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Seek failed: {e}"))?;
    import_workspace(tmp_file, &dest_db, Some(&source_password), &new_password, &identity_uuid, Ed25519SigningKey::from_bytes(&copy_seed))
        .map_err(|e| e.to_string())?;

    // Write info.json for the new workspace so we can read its UUID.
    let new_ws = Workspace::open(&dest_db, &new_password, &identity_uuid, Ed25519SigningKey::from_bytes(&copy_seed))
        .map_err(|e| format!("Failed to open new workspace: {e}"))?;
    let _ = new_ws.write_info_json();
    let new_ws_uuid = new_ws.workspace_id().to_string();
    drop(new_ws);

    // Bind the duplicate workspace to the same identity
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
    let unlocked = identities
        .get(&uuid)
        .ok_or_else(|| "Identity is not unlocked".to_string())?;
    let seed = unlocked.signing_key.to_bytes();
    drop(identities);

    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.bind_workspace(
        &uuid,
        &new_ws_uuid,
        &dest_folder,
        &new_password,
        &seed,
    )
    .map_err(|e| format!("Failed to bind new workspace to identity: {e}"))?;

    Ok(())
}

/// Attaches a file to a note. Reads the file from disk, encrypts it, and stores it.
#[tauri::command]
fn attach_file(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    file_path: String,
) -> std::result::Result<AttachmentMeta, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    let path = std::path::Path::new(&file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid file path")?
        .to_string();

    let mime_type = mime_guess::from_path(path)
        .first()
        .map(|m| m.to_string());

    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    workspace
        .attach_file(&note_id, &filename, mime_type.as_deref(), &data)
        .map_err(|e| e.to_string())
}

/// Attaches a file to a note from raw bytes (used for drag-and-drop, where only
/// file data — not a filesystem path — is available in the frontend).
///
/// Uses binary IPC: the caller passes a `Uint8Array` as the invoke body with
/// `Content-Type: application/octet-stream`, avoiding the ~3× overhead of
/// JSON number-array serialisation.  Metadata travels as HTTP headers:
///   `x-note-id`  — note UUID (ASCII)
///   `x-filename` — base64(UTF-8 bytes of filename) to survive ASCII-only headers
#[tauri::command]
fn attach_file_bytes(
    request: tauri::ipc::Request<'_>,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<AttachmentMeta, String> {
    // Extract raw binary body.
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("attach_file_bytes: expected raw binary body".to_string());
    };
    // note_id is a plain UUID — safe as ASCII header value.
    let note_id = request
        .headers()
        .get("x-note-id")
        .and_then(|v| v.to_str().ok())
        .ok_or("attach_file_bytes: missing x-note-id header")?
        .to_owned();
    // filename is base64(UTF-8 bytes) so non-ASCII names survive the ASCII header constraint.
    let filename_b64 = request
        .headers()
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .ok_or("attach_file_bytes: missing x-filename header")?;
    let filename_bytes = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(filename_b64)
            .map_err(|e| format!("attach_file_bytes: invalid filename encoding: {e}"))?
    };
    let filename = String::from_utf8(filename_bytes)
        .map_err(|e| format!("attach_file_bytes: invalid UTF-8 in filename: {e}"))?;

    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    let mime_type = mime_guess::from_path(&filename)
        .first()
        .map(|m| m.to_string());
    workspace
        .attach_file(&note_id, &filename, mime_type.as_deref(), data)
        .map_err(|e| e.to_string())
}

/// Returns attachment metadata for all attachments on a note.
#[tauri::command]
fn get_attachments(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Vec<AttachmentMeta>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.get_attachments(&note_id).map_err(|e| e.to_string())
}

/// Returns the decrypted base64-encoded bytes of an attachment together with its MIME type.
#[derive(serde::Serialize)]
struct AttachmentDataResponse {
    data: String,
    mime_type: Option<String>,
}

#[tauri::command]
fn get_attachment_data(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<AttachmentDataResponse, String> {
    use base64::Engine;
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    let (bytes, mime_type) = workspace
        .get_attachment_bytes_and_mime(&attachment_id)
        .map_err(|e| e.to_string())?;
    Ok(AttachmentDataResponse {
        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        mime_type,
    })
}

/// Deletes an attachment from a note.
#[tauri::command]
fn delete_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .delete_attachment(&attachment_id)
        .map_err(|e| e.to_string())
}

/// Restores a previously soft-deleted attachment (moves `.enc.trash` → `.enc`, re-inserts DB row).
/// Called from the in-section "Restore" button in AttachmentsSection.
#[tauri::command]
fn restore_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    meta: AttachmentMeta,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .restore_attachment(&meta)
        .map_err(|e| e.to_string())
}

/// Decrypts an attachment to a temp file and opens it with the default system application.
#[tauri::command]
async fn open_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
    filename: String,
) -> std::result::Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let bytes = {
        let label = window.label();
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(label).ok_or("No workspace open")?;
        workspace
            .get_attachment_bytes(&attachment_id)
            .map_err(|e| e.to_string())?
    };

    let tmp_dir = std::env::temp_dir().join("krillnotes-attachments");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let tmp_path = tmp_dir.join(&filename);
    std::fs::write(&tmp_path, &bytes).map_err(|e| e.to_string())?;

    window
        .app_handle()
        .opener()
        .open_path(tmp_path.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|e| e.to_string())
}

// ── Swarm bundle commands ──────────────────────────────────────────

/// Info returned to the frontend after peeking at a .swarm bundle header.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum SwarmFileInfo {
    Invite {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        #[serde(rename = "offeredRole")]
        offered_role: String,
        #[serde(rename = "offeredScope")]
        offered_scope: Option<String>,
        #[serde(rename = "inviterDisplayName")]
        inviter_display_name: String,
        #[serde(rename = "inviterFingerprint")]
        inviter_fingerprint: String,
        #[serde(rename = "pairingToken")]
        pairing_token: String,
        #[serde(rename = "targetIdentityUuid")]
        target_identity_uuid: Option<String>,
        #[serde(rename = "targetIdentityName")]
        target_identity_name: Option<String>,
    },
    Accept {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        #[serde(rename = "declaredName")]
        declared_name: String,
        #[serde(rename = "acceptorFingerprint")]
        acceptor_fingerprint: String,
        #[serde(rename = "acceptorPublicKey")]
        acceptor_public_key: String,
        #[serde(rename = "pairingToken")]
        pairing_token: String,
    },
    Snapshot {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        #[serde(rename = "senderDisplayName")]
        sender_display_name: String,
        #[serde(rename = "senderFingerprint")]
        sender_fingerprint: String,
        #[serde(rename = "asOfOperationId")]
        as_of_operation_id: String,
        #[serde(rename = "targetIdentityUuid")]
        target_identity_uuid: Option<String>,
        #[serde(rename = "targetIdentityName")]
        target_identity_name: Option<String>,
    },
    Delta {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        /// Name of the local workspace this delta targets (folder name).
        /// Present when the recipient identity has a workspace open;
        /// falls back to `workspaceName` (sender's name) if None.
        #[serde(rename = "localWorkspaceName")]
        local_workspace_name: Option<String>,
        #[serde(rename = "senderDisplayName")]
        sender_display_name: String,
        #[serde(rename = "senderFingerprint")]
        sender_fingerprint: String,
        #[serde(rename = "sinceOperationId")]
        since_operation_id: Option<String>,
        #[serde(rename = "targetIdentityUuid")]
        target_identity_uuid: Option<String>,
        #[serde(rename = "targetIdentityName")]
        target_identity_name: Option<String>,
    },
}

/// Read and deserialise just the header.json from a .swarm zip bundle.
fn peek_swarm_header(data: &[u8]) -> std::result::Result<krillnotes_core::core::swarm::header::SwarmHeader, String> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;
    let cursor = Cursor::new(data);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| format!("Cannot open bundle: {e}"))?;

    // Detect Phase C invite/response files before trying to read header.json
    if zip.by_name("invite.json").is_ok() {
        return Err("This is a Phase C invite file. Use the 'Import Invite' button to open it.".to_string());
    }
    if zip.by_name("response.json").is_ok() {
        return Err("This is a Phase C response file. Use the 'Import Response' button to open it.".to_string());
    }

    let header_bytes = {
        let mut file = zip.by_name("header.json")
            .map_err(|_| "bundle missing 'header.json'".to_string())?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|e| format!("Cannot read header: {e}"))?;
        buf
    };
    serde_json::from_slice(&header_bytes)
        .map_err(|e| format!("Invalid header: {e}"))
}

/// Peek at a .swarm file and return its type + display metadata.
#[tauri::command]
fn open_swarm_file_cmd(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<SwarmFileInfo, String> {
    use krillnotes_core::core::swarm::header::SwarmMode;
    let data = std::fs::read(&path).map_err(|e| format!("Cannot read file: {e}"))?;
    let header = peek_swarm_header(&data)?;

    let fingerprint = krillnotes_core::core::contact::generate_fingerprint(&header.source_identity)
        .map_err(|e| e.to_string())?;

    match header.mode {
        SwarmMode::Invite => {
            let (target_identity_uuid, target_identity_name) = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let identities = mgr.list_identities().unwrap_or_default();
                let mut found_uuid = None;
                let mut found_name = None;
                if let Some(ref target_pubkey) = header.target_peer {
                    for identity_ref in &identities {
                        let full_path = mgr.identity_file_path(&identity_ref.uuid);
                        if let Ok(data) = std::fs::read_to_string(&full_path) {
                            if let Ok(file) = serde_json::from_str::<krillnotes_core::core::identity::IdentityFile>(&data) {
                                if &file.public_key == target_pubkey {
                                    found_uuid = Some(identity_ref.uuid.to_string());
                                    found_name = Some(identity_ref.display_name.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
                (found_uuid, found_name)
            };
            Ok(SwarmFileInfo::Invite {
                workspace_name: header.workspace_name,
                offered_role: header.offered_role.unwrap_or_default(),
                offered_scope: header.offered_scope,
                inviter_display_name: header.source_display_name,
                inviter_fingerprint: fingerprint,
                pairing_token: header.pairing_token.unwrap_or_default(),
                target_identity_uuid,
                target_identity_name,
            })
        }
        SwarmMode::Accept => Ok(SwarmFileInfo::Accept {
            workspace_name: header.workspace_name,
            declared_name: header.source_display_name,
            acceptor_fingerprint: fingerprint,
            acceptor_public_key: header.source_identity,
            pairing_token: header.pairing_token.unwrap_or_default(),
        }),
        SwarmMode::Snapshot => {
            // Try to identify which local identity this snapshot is for.
            let (target_identity_uuid, target_identity_name) = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let identities = mgr.list_identities().unwrap_or_default();
                // Read each identity's public key and match against recipient peer_ids.
                let peer_ids: Vec<String> = header.recipients.as_ref()
                    .map(|r| r.iter().map(|e| e.peer_id.clone()).collect())
                    .unwrap_or_default();
                let mut found_uuid = None;
                let mut found_name = None;
                for identity_ref in &identities {
                    let full_path = mgr.identity_file_path(&identity_ref.uuid);
                    if let Ok(data) = std::fs::read_to_string(&full_path) {
                        if let Ok(file) = serde_json::from_str::<krillnotes_core::core::identity::IdentityFile>(&data) {
                            if peer_ids.contains(&file.public_key) {
                                found_uuid = Some(identity_ref.uuid.to_string());
                                found_name = Some(identity_ref.display_name.clone());
                                break;
                            }
                        }
                    }
                }
                (found_uuid, found_name)
            };
            Ok(SwarmFileInfo::Snapshot {
                workspace_name: header.workspace_name,
                sender_display_name: header.source_display_name,
                sender_fingerprint: fingerprint,
                as_of_operation_id: header.as_of_operation_id.unwrap_or_default(),
                target_identity_uuid,
                target_identity_name,
            })
        }
        SwarmMode::Delta => {
            // Identify which local identity this delta is addressed to.
            let (target_identity_uuid, target_identity_name) = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let identities = mgr.list_identities().unwrap_or_default();
                let target_pubkey = header.target_peer.as_deref().unwrap_or("");
                let mut found_uuid = None;
                let mut found_name = None;
                for identity_ref in &identities {
                    let full_path = mgr.identity_file_path(&identity_ref.uuid);
                    if let Ok(file_data) = std::fs::read_to_string(&full_path) {
                        if let Ok(file) = serde_json::from_str::<krillnotes_core::core::identity::IdentityFile>(&file_data) {
                            if file.public_key == target_pubkey {
                                found_uuid = Some(identity_ref.uuid.to_string());
                                found_name = Some(identity_ref.display_name.clone());
                                break;
                            }
                        }
                    }
                }
                (found_uuid, found_name)
            };
            // Find the local workspace name for the recipient identity's open workspace.
            let local_workspace_name = target_identity_uuid.as_deref()
                .and_then(|uuid_str| Uuid::parse_str(uuid_str).ok())
                .and_then(|uuid| {
                    let identity_map = state.workspace_identities.lock().expect("Mutex poisoned");
                    let paths = state.workspace_paths.lock().expect("Mutex poisoned");
                    identity_map.iter()
                        .find(|(_, id)| **id == uuid)
                        .and_then(|(lbl, _)| paths.get(lbl))
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                });
            Ok(SwarmFileInfo::Delta {
                workspace_name: header.workspace_name,
                local_workspace_name,
                sender_display_name: header.source_display_name,
                sender_fingerprint: fingerprint,
                since_operation_id: header.since_operation_id,
                target_identity_uuid,
                target_identity_name,
            })
        }
    }
}

// ── Invite commands (Phase C) ─────────────────────────────────────────────

/// Serialisable invite record returned to the frontend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub use_count: u32,
}

impl From<krillnotes_core::core::invite::InviteRecord> for InviteInfo {
    fn from(r: krillnotes_core::core::invite::InviteRecord) -> Self {
        Self {
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            created_at: r.created_at.to_rfc3339(),
            expires_at: r.expires_at.map(|dt| dt.to_rfc3339()),
            revoked: r.revoked,
            use_count: r.use_count,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteFileData {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_description: Option<String>,
    pub workspace_author_name: Option<String>,
    pub workspace_author_org: Option<String>,
    pub workspace_homepage_url: Option<String>,
    pub workspace_license: Option<String>,
    pub workspace_language: Option<String>,
    pub workspace_tags: Vec<String>,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    pub inviter_fingerprint: String,
    pub expires_at: Option<String>,
}

#[tauri::command]
fn list_invites(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<InviteInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get(&uuid).ok_or("Identity not unlocked")?;
    let records = im.list_invites().map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(InviteInfo::from).collect())
}

#[tauri::command]
fn create_invite(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_name: String,
    expires_in_days: Option<u32>,
    save_path: String,
) -> std::result::Result<InviteInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Get signing key + declared name from unlocked identity.
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Get workspace id + metadata from the current window's workspace.
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

    let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
    let (record, file) = im
        .create_invite(
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
        .map_err(|e| e.to_string())?;

    krillnotes_core::core::invite::InviteManager::save_invite_file(&file, std::path::Path::new(&save_path))
        .map_err(|e| e.to_string())?;

    Ok(InviteInfo::from(record))
}

#[tauri::command]
fn revoke_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let invite_uuid = Uuid::parse_str(&invite_id).map_err(|e| e.to_string())?;
    let mut ims = state.invite_managers.lock().expect("Mutex poisoned");
    let im = ims.get_mut(&uuid).ok_or("Identity not unlocked")?;
    im.revoke_invite(invite_uuid).map_err(|e| e.to_string())
}

/// Serialisable pending peer data returned to the frontend after parsing a response file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPeer {
    pub invite_id: String,
    pub invitee_public_key: String,
    pub invitee_declared_name: String,
    pub fingerprint: String,
}

#[tauri::command]
fn import_invite_response(
    state: State<'_, AppState>,
    identity_uuid: String,
    path: String,
) -> std::result::Result<PendingPeer, String> {
    use krillnotes_core::core::invite::InviteManager;
    use krillnotes_core::core::contact::generate_fingerprint;

    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let response = InviteManager::parse_and_verify_response(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;

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
    Ok(PendingPeer {
        invite_id: response.invite_id,
        invitee_public_key: response.invitee_public_key,
        invitee_declared_name: response.invitee_declared_name,
        fingerprint,
    })
}

#[tauri::command]
fn import_invite(path: String) -> std::result::Result<InviteFileData, String> {
    use krillnotes_core::core::invite::InviteManager;
    use krillnotes_core::core::contact::generate_fingerprint;

    let invite = InviteManager::parse_and_verify_invite(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;

    let fingerprint = generate_fingerprint(&invite.inviter_public_key).map_err(|e| e.to_string())?;

    Ok(InviteFileData {
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
    })
}

#[tauri::command]
fn respond_to_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_path: String,
    save_path: String,
) -> std::result::Result<(), String> {
    use krillnotes_core::core::invite::InviteManager;

    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    let invite = InviteManager::parse_and_verify_invite(std::path::Path::new(&invite_path))
        .map_err(|e| e.to_string())?;

    InviteManager::build_and_save_response(
        &invite,
        &signing_key,
        &declared_name,
        std::path::Path::new(&save_path),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn accept_peer(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    invitee_public_key: String,
    declared_name: String,
    trust_level: String,
    local_name: Option<String>,
) -> std::result::Result<ContactInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let trust = parse_trust_level(&trust_level)?;

    // Create or find existing contact by public key (handles duplicate public key per spec C5).
    let contact = {
        let cms = state.contact_managers.lock().expect("Mutex poisoned");
        let cm = cms.get(&uuid).ok_or("Identity not unlocked")?;
        let mut c = cm
            .find_or_create_by_public_key(&declared_name, &invitee_public_key, trust)
            .map_err(|e| e.to_string())?;
        if let Some(name) = local_name {
            c.local_name = Some(name);
            cm.save_contact(&c).map_err(|e| e.to_string())?;
        }
        c
    };

    // Add as pre-authorised workspace peer.
    {
        let wss = state.workspaces.lock().expect("Mutex poisoned");
        if let Some(ws) = wss.get(window.label()) {
            ws.add_contact_as_peer(&invitee_public_key)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(ContactInfo::from_contact(contact))
}

/// Serialisable result returned after a snapshot bundle is written to disk.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCreatedResult {
    pub saved_path: String,
    pub peer_count: usize,
    pub as_of_operation_id: String,
}

#[tauri::command]
async fn create_snapshot_for_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    peer_public_keys: Vec<String>,   // base64-encoded Ed25519 verifying keys
    save_path: String,
) -> std::result::Result<SnapshotCreatedResult, String> {
    use base64::Engine;
    use krillnotes_core::core::swarm::snapshot::create_snapshot_bundle;
    use krillnotes_core::core::swarm::snapshot::SnapshotParams;

    let identity_uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // 1. Sender signing key + display name.
    let (signing_key, source_display_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid).ok_or("Identity not unlocked")?;
        (
            Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };
    let source_device_id = krillnotes_core::get_device_id().map_err(|e| e.to_string())?;

    // 2. Decode recipient verifying keys from base64.
    let recipient_vks: Vec<Ed25519VerifyingKey> = peer_public_keys
        .iter()
        .map(|pk_b64| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(pk_b64)
                .map_err(|e| e.to_string())?;
            let arr: [u8; 32] = bytes.try_into().map_err(|_| "key wrong length".to_string())?;
            Ed25519VerifyingKey::from_bytes(&arr).map_err(|e| e.to_string())
        })
        .collect::<std::result::Result<_, _>>()?;

    // 3. Collect workspace data (hold lock only briefly).
    let (workspace_id, workspace_name, workspace_json, attachment_blobs, as_of_op_id) = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let paths = state.workspace_paths.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        let workspace_name = paths
            .get(window.label())
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let workspace_id = ws.workspace_id().to_string();

        let workspace_json = ws.to_snapshot_json().map_err(|e| e.to_string())?;

        // Get attachment metadata from the snapshot JSON to load blobs.
        let snapshot: krillnotes_core::core::workspace::WorkspaceSnapshot = serde_json::from_slice(&workspace_json)
            .map_err(|e| e.to_string())?;
        let mut attachment_blobs: Vec<(String, Vec<u8>)> = Vec::new();
        for meta in &snapshot.attachments {
            let plaintext = ws.get_attachment_bytes(&meta.id).map_err(|e| e.to_string())?;
            attachment_blobs.push((meta.id.clone(), plaintext));
        }

        let as_of_op_id = ws.get_latest_operation_id()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();

        (workspace_id, workspace_name, workspace_json, attachment_blobs, as_of_op_id)
    };

    // 4. Build the bundle.
    let recipient_refs: Vec<&Ed25519VerifyingKey> = recipient_vks.iter().collect();
    let bundle_bytes = create_snapshot_bundle(SnapshotParams {
        workspace_id: workspace_id.clone(),
        workspace_name,
        source_device_id,
        source_display_name,
        as_of_operation_id: as_of_op_id.clone(),
        workspace_json,
        sender_key: &signing_key,
        recipient_keys: recipient_refs,
        recipient_peer_ids: peer_public_keys.clone(),
        attachment_blobs,
    }).map_err(|e| e.to_string())?;

    // 5. Write to file.
    std::fs::write(&save_path, &bundle_bytes).map_err(|e| e.to_string())?;

    // 6. Update last_sent_op for each recipient — always, even for empty workspaces.
    // An empty as_of_op_id ("") is a valid sentinel meaning "start of log": operations_since
    // falls back to sending all ops, and the recipient's INSERT OR IGNORE handles duplicates.
    {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        if let Some(ws) = workspaces.get(window.label()) {
            for pk in &peer_public_keys {
                let _ = ws.update_peer_last_sent_by_identity(pk, &as_of_op_id);
            }
        }
    }

    Ok(SnapshotCreatedResult {
        saved_path: save_path,
        peer_count: peer_public_keys.len(),
        as_of_operation_id: as_of_op_id,
    })
}

/// Apply a `.swarm` snapshot bundle to create a new local workspace.
///
/// Mirrors `execute_import`: decrypts the bundle, creates the workspace DB with
/// the snapshot's UUID preserved (required for CRDT convergence), restores all
/// notes, user scripts, and attachments, then opens a new window.
#[tauri::command]
async fn apply_swarm_snapshot(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    identity_uuid: String,
    workspace_name_override: Option<String>,
) -> std::result::Result<WorkspaceInfo, String> {
    use base64::Engine;
    use krillnotes_core::core::swarm::snapshot::parse_snapshot_bundle;
    use krillnotes_core::core::workspace::WorkspaceSnapshot;
    use rand::RngCore;

    let identity_uuid_parsed = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // 1. Read bundle bytes and get the recipient signing key from the unlocked identity.
    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
    let import_seed = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid_parsed).ok_or("Identity not unlocked")?;
        id.signing_key.to_bytes()
    };
    let recipient_key = Ed25519SigningKey::from_bytes(&import_seed);
    let parsed = parse_snapshot_bundle(&data, &recipient_key).map_err(|e| e.to_string())?;

    // Deserialise snapshot JSON now so we can look up attachment metadata later.
    let snapshot: WorkspaceSnapshot = serde_json::from_slice(&parsed.workspace_json)
        .map_err(|e| e.to_string())?;

    // 2. Determine workspace name → folder name (mirrors file-stem convention).
    let ws_name = workspace_name_override
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| parsed.workspace_name.clone());

    // Derive folder inside the user's configured workspace directory,
    // the same location used by create_workspace and list_workspace_files.
    let folder = PathBuf::from(&settings::load_settings().workspace_directory)
        .join(&ws_name);

    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("create workspace dir: {e}"))?;
    let db_path = folder.join("notes.db");
    if db_path.exists() {
        return Err(format!("Workspace '{}' already exists locally.", ws_name));
    }

    // 3. Generate a fresh DB encryption password (never leaves this device).
    let workspace_password: String = {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        base64::engine::general_purpose::STANDARD.encode(bytes)
    };

    // 4. Create workspace DB preserving the snapshot's UUID.
    let mut ws = Workspace::create_empty_with_id(
        &db_path,
        &workspace_password,
        &identity_uuid,
        Ed25519SigningKey::from_bytes(&import_seed),
        &parsed.workspace_id,
    )
    .map_err(|e| e.to_string())?;

    // 5. Restore notes + user scripts from the snapshot.
    ws.import_snapshot_json(&parsed.workspace_json)
        .map_err(|e| e.to_string())?;
    // Run the imported scripts in the Rhai engine so all schemas are registered.
    ws.reload_all_scripts().map_err(|e| e.to_string())?;

    // 6. Restore attachment blobs — look up metadata from snapshot to pass correct fields.
    let _ = std::fs::create_dir_all(folder.join("attachments"));
    for (att_id, plaintext) in &parsed.attachment_blobs {
        if let Some(meta) = snapshot.attachments.iter().find(|a| a.id == *att_id) {
            ws.attach_file_with_id(
                att_id,
                &meta.note_id,
                &meta.filename,
                meta.mime_type.as_deref(),
                plaintext,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // 7. Register the snapshot sender as a sync peer with last_received_op = snapshot watermark.
    let placeholder_device_id = format!("identity:{}", parsed.sender_public_key);
    let _ = ws.upsert_sync_peer(
        &placeholder_device_id,
        &parsed.sender_public_key,
        Some(&parsed.as_of_operation_id),  // last_sent_op — snapshot is the baseline
        Some(&parsed.as_of_operation_id),  // last_received_op
    );

    // 7b. Register sender in the contact manager so generate_delta can resolve their
    //     encryption key. Snapshot bundles carry no display name, so use a synthetic
    //     Falls back to a key-prefix placeholder if the bundle has no display name.
    {
        use krillnotes_core::core::contact::TrustLevel;
        let sender_key = &parsed.sender_public_key;
        let name = if parsed.sender_display_name.is_empty() {
            format!("{}…", &sender_key[..8.min(sender_key.len())])
        } else {
            parsed.sender_display_name.clone()
        };
        let cms = state.contact_managers.lock().expect("Mutex poisoned");
        if let Some(cm) = cms.get(&identity_uuid_parsed) {
            let _ = cm.find_or_create_by_public_key(&name, sender_key, TrustLevel::Tofu);
        }
    }

    // 8. Bind workspace to identity so it can be reopened on next launch.
    let workspace_uuid = ws.workspace_id().to_string();
    {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let unlocked = ids.get(&identity_uuid_parsed).ok_or("Identity not unlocked")?;
        let seed = unlocked.signing_key.to_bytes();
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.bind_workspace(
            &identity_uuid_parsed,
            &workspace_uuid,
            &folder,
            &workspace_password,
            &seed,
        )
        .map_err(|e| format!("bind_workspace: {e}"))?;
    }

    // 9. Open the workspace in a new window (mirrors execute_import exactly).
    let label = generate_unique_label(&state, &folder);
    let new_window = create_workspace_window(&app, &label, &window)?;
    store_workspace(&state, label.clone(), ws, folder, identity_uuid_parsed);
    new_window
        .set_title(&format!("Krillnotes - {ws_name}"))
        .map_err(|e| e.to_string())?;
    if window.label() == "main" {
        window.close().map_err(|e| e.to_string())?;
    }

    get_workspace_info_internal(&state, &label)
}

/// Apply a `.swarm` delta bundle to the currently open workspace.
///
/// Decrypts, verifies, and applies operations from the delta to the workspace.
/// Emits `workspace-updated` so the frontend refreshes the tree view.
#[tauri::command]
async fn apply_swarm_delta(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
    identity_uuid: String,
) -> std::result::Result<String, String> {
    use krillnotes_core::core::swarm::sync::apply_delta;

    let identity_uuid_parsed = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let bundle_bytes = std::fs::read(&path).map_err(|e| e.to_string())?;

    let recipient_key = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid_parsed).ok_or("Identity not unlocked")?;
        Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes())
    };

    // Find the workspace window that belongs to the recipient identity.
    // Using window.label() would route to whichever window opened the file,
    // which may be a different user's workspace in a multi-workspace session.
    let target_label = {
        let identity_map = state.workspace_identities.lock().expect("Mutex poisoned");
        identity_map.iter()
            .find(|(_, id)| **id == identity_uuid_parsed)
            .map(|(lbl, _)| lbl.clone())
            .ok_or("No open workspace for this identity")?
    };

    let apply_result = {
        let mut cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
        let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get_mut(&target_label).ok_or("Workspace not open")?;
        let cm = cm_guard.get_mut(&identity_uuid_parsed).ok_or("Contact manager not available")?;
        apply_delta(&bundle_bytes, ws, &recipient_key, cm).map_err(|e| e.to_string())?
    };

    // Emit workspace-updated on the target workspace's window so it refreshes.
    if let Some(target_win) = window.app_handle().get_webview_window(&target_label) {
        let _ = target_win.emit("workspace-updated", ());
    } else {
        let _ = window.emit("workspace-updated", ());
    }

    Ok(serde_json::json!({
        "mode": "delta",
        "operationsApplied": apply_result.operations_applied,
        "operationsSkipped": apply_result.operations_skipped,
        "newTofu": apply_result.new_tofu_contacts,
    }).to_string())
}

/// Serialisable result returned after one or more delta bundles are written.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateDeltasResult {
    succeeded: Vec<String>,          // peer_device_ids that worked
    failed: Vec<(String, String)>,   // (peer_device_id, error_message)
    files_written: Vec<String>,      // absolute paths of written .swarm files
}

/// Batch-generates one delta .swarm per selected peer into `dir_path`.
///
/// Continues on per-peer errors so a single failure doesn't block the others.
#[tauri::command]
async fn generate_deltas_for_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
    dir_path: String,
    peer_device_ids: Vec<String>,
) -> std::result::Result<GenerateDeltasResult, String> {
    use krillnotes_core::core::swarm::sync::generate_delta;

    // Get signing key, display name, and workspace name upfront (before per-peer loop).
    let (signing_key, sender_display_name, workspace_name, identity_uuid) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        let identity_uuid_str = ws.identity_uuid().to_string();
        let identity_uuid = Uuid::parse_str(&identity_uuid_str).map_err(|e| e.to_string())?;

        let id = ids.get(&identity_uuid).ok_or("Identity not unlocked")?;
        let key = Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes());
        let display_name = id.display_name.clone();

        let paths = state.workspace_paths.lock().expect("Mutex poisoned");
        let ws_name = paths
            .get(window.label())
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        (key, display_name, ws_name, identity_uuid)
    };

    let dir = std::path::Path::new(&dir_path);
    if !dir.exists() {
        return Err(format!("Directory does not exist: {dir_path}"));
    }

    let mut result = GenerateDeltasResult {
        succeeded: Vec::new(),
        failed: Vec::new(),
        files_written: Vec::new(),
    };

    for peer_id in &peer_device_ids {
        // Resolve display name for file naming.
        let display_name = {
            let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
            let workspaces = state.workspaces.lock().expect("Mutex poisoned");
            let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
            if let Some(cm) = cm_guard.get(&identity_uuid) {
                ws.list_peers_info(cm)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|p| &p.peer_device_id == peer_id)
                    .map(|p| p.display_name)
                    .unwrap_or_else(|| peer_id[..8.min(peer_id.len())].to_string())
            } else {
                peer_id[..8.min(peer_id.len())].to_string()
            }
        };

        // Sanitise display name for use in file path.
        let safe_name: String = display_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let base_name = format!("delta-{safe_name}-{date}.swarm");

        // Avoid overwriting existing files.
        let file_path = {
            let mut p = dir.join(&base_name);
            let mut n = 2u32;
            while p.exists() {
                let stem = format!("delta-{safe_name}-{date}-{n}.swarm");
                p = dir.join(stem);
                n += 1;
            }
            p
        };

        // Generate the delta.
        let bundle_result = {
            let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
            let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
            let ws = workspaces.get_mut(window.label()).ok_or("Workspace not open")?;
            if let Some(cm) = cm_guard.get(&identity_uuid) {
                generate_delta(ws, peer_id, &workspace_name, &signing_key, &sender_display_name, cm)
                    .map_err(|e| e.to_string())
            } else {
                Err("Contact manager not available".to_string())
            }
        };

        match bundle_result {
            Ok(bytes) => match std::fs::write(&file_path, &bytes) {
                Ok(()) => {
                    result.succeeded.push(peer_id.clone());
                    result.files_written.push(file_path.to_string_lossy().to_string());
                }
                Err(e) => result.failed.push((peer_id.clone(), e.to_string())),
            },
            Err(e) => result.failed.push((peer_id.clone(), e)),
        }
    }

    Ok(result)
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
    ("file_invite_peer",      "File > Invite Peer clicked"),
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
                handle_file_opened(app.handle(), &state_ref, path);
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
                            handle_file_opened(app_handle, &state, path);
                        }
                    }
                }
            }
            let _ = event;
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_file_content_impl_rejects_disallowed_extension() {
        let result = super::read_file_content_impl("/some/path/credentials.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Only .rhai and .krilltheme"));
    }

    #[test]
    fn read_file_content_impl_errors_on_missing_rhai_file() {
        // Use a path with allowed extension but nonexistent file
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("__missing__.rhai");
        // Do NOT create the file — it should not exist
        let result = super::read_file_content_impl(path.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn read_file_content_impl_allows_rhai_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.rhai");
        std::fs::write(&path, "// @name: Test").unwrap();
        let result = super::read_file_content_impl(path.to_str().unwrap());
        assert_eq!(result.unwrap(), "// @name: Test");
    }

    #[test]
    fn read_file_content_impl_allows_krilltheme_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.krilltheme");
        std::fs::write(&path, r#"{"name":"Test"}"#).unwrap();
        let result = super::read_file_content_impl(path.to_str().unwrap());
        assert_eq!(result.unwrap(), r#"{"name":"Test"}"#);
    }
}
