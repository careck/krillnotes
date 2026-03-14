// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::{AppHandle, Emitter, Manager, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use krillnotes_core::{Ed25519SigningKey, KrillnotesError, Workspace};

// ── WorkspaceInfo ─────────────────────────────────────────────────

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

// ── Private helpers ───────────────────────────────────────────────

/// Derives a unique window label from the `path` filename stem.
///
/// Appends a numeric suffix (`-2`, `-3`, …) until the label is absent
/// from the currently open workspace labels in `state`.
pub fn generate_unique_label(state: &AppState, path: &Path) -> String {
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
pub fn handle_file_opened(app: &AppHandle, state: &AppState, path: PathBuf) {
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
pub fn create_main_window(app: &AppHandle) {
    let lang = crate::settings::load_settings().language;
    let strings = crate::locales::menu_strings(&lang);
    if let Ok(menu_result) = crate::menu::build_menu(app, &strings) {
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
pub fn create_workspace_window(
    app: &AppHandle,
    label: &str,
    caller: &tauri::Window,
) -> std::result::Result<tauri::WebviewWindow, String> {
    let lang = crate::settings::load_settings().language;
    let strings = crate::locales::menu_strings(&lang);
    let menu_result = crate::menu::build_menu(app, &strings)
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
/// On Windows each window owns its own menu bar: every open window gets a freshly
/// built menu, with workspace items pre-enabled for workspace windows.
pub fn rebuild_menus(app: &AppHandle, state: &AppState, lang: &str) -> std::result::Result<(), String> {
    let strings = crate::locales::menu_strings(lang);

    #[cfg(target_os = "macos")]
    {
        let result = crate::menu::build_menu(app, &strings)
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
            let result = crate::menu::build_menu(app, &strings)
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
pub fn store_workspace(
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
pub fn get_workspace_info_internal(
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

// ── Workspace Tauri commands ──────────────────────────────────────

/// Creates a new workspace folder at `path` and opens it in a new window.
///
/// `path` is the workspace folder path. A `notes.db` file and an
/// `attachments/` subdirectory are created inside it.
#[tauri::command]
pub async fn create_workspace(
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
pub async fn open_workspace(
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
pub fn get_workspace_info(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<WorkspaceInfo, String> {
    get_workspace_info_internal(&state, window.label())
}

#[tauri::command]
pub fn get_workspace_metadata(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<krillnotes_core::WorkspaceMetadata, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.get_workspace_metadata()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_workspace_metadata(
    window: tauri::Window,
    state: State<'_, AppState>,
    metadata: krillnotes_core::WorkspaceMetadata,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.set_workspace_metadata(&metadata)
        .map_err(|e| e.to_string())
}


// ── Export / Import commands ──────────────────────────────────────

/// Exports the calling window's workspace as a zip archive at `path`.
#[tauri::command]
pub fn export_workspace_cmd(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
) -> std::result::Result<(), String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    krillnotes_core::export_workspace(workspace, file, password.as_deref()).map_err(|e| e.to_string())
}

/// Reads metadata from an export archive without creating a workspace.
#[tauri::command]
pub fn peek_import_cmd(
    zip_path: String,
    password: Option<String>,
) -> std::result::Result<krillnotes_core::ImportResult, String> {
    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    krillnotes_core::peek_import(reader, password.as_deref()).map_err(|e| match e {
        krillnotes_core::ExportError::EncryptedArchive => "ENCRYPTED_ARCHIVE".to_string(),
        krillnotes_core::ExportError::InvalidPassword => "INVALID_PASSWORD".to_string(),
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
pub async fn execute_import(
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
    krillnotes_core::import_workspace(reader, &db_path_buf, password.as_deref(), &workspace_password, &uuid.to_string(), Ed25519SigningKey::from_bytes(&import_seed))
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
pub fn get_app_version() -> String {
    krillnotes_core::APP_VERSION.to_string()
}

/// Dequeues and returns the pending file path that arrived via OS file-open
/// before the frontend was ready. Returns `None` when no file is pending.
///
/// This is the pull-side of the cold-start delivery mechanism. The frontend
/// calls this once on mount (main window only) so that files opened at app
/// launch are handled even if the `"file-opened"` push event fired before
/// the JS listener was registered.
#[tauri::command]
pub fn consume_pending_file_open(state: State<'_, AppState>) -> Option<String> {
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
pub fn consume_pending_swarm_file(state: State<'_, AppState>) -> Option<String> {
    state
        .pending_file_open
        .lock()
        .expect("Mutex poisoned")
        .take()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| p.ends_with(".swarm"))
}

// ── Theme commands ────────────────────────────────────────────────

/// Lists all user theme files in the themes directory.
#[tauri::command]
pub fn list_themes() -> std::result::Result<Vec<crate::themes::ThemeMeta>, String> {
    crate::themes::list_themes()
}

/// Returns the raw JSON content of a theme file.
#[tauri::command]
pub fn read_theme(filename: String) -> std::result::Result<String, String> {
    crate::themes::read_theme(&filename)
}

/// Writes (creates or overwrites) a theme file.
#[tauri::command]
pub fn write_theme(filename: String, content: String) -> std::result::Result<(), String> {
    crate::themes::write_theme(&filename, &content)
}

/// Deletes a theme file.
#[tauri::command]
pub fn delete_theme(filename: String) -> std::result::Result<(), String> {
    crate::themes::delete_theme(&filename)
}

/// Reads and returns the text content of the file at `path`.
/// Only `.rhai` and `.krilltheme` files are allowed.
/// Returns an error string if the extension is not permitted, the file does
/// not exist, or cannot be read.
pub(crate) fn read_file_content_impl(path: &str) -> std::result::Result<String, String> {
    let allowed = path.ends_with(".rhai") || path.ends_with(".krilltheme");
    if !allowed {
        return Err(format!("Only .rhai and .krilltheme files may be imported: {path}"));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Reads and returns the full text of a user-selected import file.
/// Accepts only `.rhai` and `.krilltheme` files.
#[tauri::command]
pub fn read_file_content(path: String) -> std::result::Result<String, String> {
    read_file_content_impl(&path)
}

// ── Settings commands ─────────────────────────────────────────────

/// Returns the current application settings.
#[tauri::command]
pub fn get_settings() -> std::result::Result<crate::settings::AppSettings, String> {
    Ok(crate::settings::load_settings())
}

/// Updates and persists the application settings.
///
/// Accepts a partial JSON object and merges it onto the current settings on
/// disk, so callers only need to supply the fields they care about — missing
/// fields are preserved rather than reset to defaults.
#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> std::result::Result<(), String> {
    let current = crate::settings::load_settings();
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
    let updated: crate::settings::AppSettings = serde_json::from_value(current_value)
        .map_err(|e| format!("Failed to deserialize merged settings: {e}"))?;
    crate::settings::save_settings(&updated)?;

    if updated.language != old_lang {
        rebuild_menus(&app, &state, &updated.language)?;
    }

    Ok(())
}

/// Entry returned by [`list_workspace_files`], representing a workspace folder
/// (containing `notes.db`) in the configured workspace directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
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
pub fn read_info_json_full(workspace_dir: &Path) -> (Option<String>, Option<i64>, Option<usize>, Option<usize>) {
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

/// Lists all workspace folders (subdirectories containing `notes.db`) in the
/// configured workspace directory.
///
/// Each entry includes an `is_open` flag indicating whether the workspace
/// is currently open in a window, so the frontend can grey those out.
#[tauri::command]
pub fn list_workspace_files(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<WorkspaceEntry>, String> {
    let app_settings = crate::settings::load_settings();
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
pub fn delete_workspace(
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
pub fn duplicate_workspace(
    state: State<'_, AppState>,
    source_path: String,
    identity_uuid: String,
    new_name: String,
) -> std::result::Result<(), String> {
    let app_settings = crate::settings::load_settings();
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
    krillnotes_core::export_workspace(&workspace, &mut tmp_file, Some(&source_password))
        .map_err(|e| e.to_string())?;

    // Import from temp file into dest folder
    std::fs::create_dir_all(&dest_folder)
        .map_err(|e| format!("Failed to create destination: {e}"))?;
    let dest_db = dest_folder.join("notes.db");

    use std::io::Seek;
    tmp_file
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Seek failed: {e}"))?;
    krillnotes_core::import_workspace(tmp_file, &dest_db, Some(&source_password), &new_password, &identity_uuid, Ed25519SigningKey::from_bytes(&copy_seed))
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

#[tauri::command]
pub fn is_workspace_owner(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<bool, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.is_owner())
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
