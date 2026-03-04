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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// Label of the window that most recently gained focus. Used to route
    /// native menu events to the correct window without relying on async
    /// focus checks in the frontend (which are unreliable on Windows).
    pub focused_window: Arc<Mutex<Option<String>>>,
    /// In-memory password cache keyed by workspace file path.
    /// Populated only when settings.cacheWorkspacePasswords is true.
    pub workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>,
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
        // future: Some("swarm") => handle_swarm_open(app, state, path),
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

/// Inserts `workspace` and its `path` into `state` under `label`.
fn store_workspace(
    state: &AppState,
    label: String,
    workspace: Workspace,
    path: PathBuf,
) {
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let mut paths = state.workspace_paths.lock()
        .expect("Mutex poisoned");

    workspaces.insert(label.clone(), workspace);
    paths.insert(label, path);
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

    Ok(WorkspaceInfo {
        filename,
        path: path.display().to_string(),
        note_count,
        selected_note_id,
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
    password: String,
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
            let label = generate_unique_label(&state, &folder);
            std::fs::create_dir_all(&folder)
                .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
            let db_path = folder.join("notes.db");
            let workspace = Workspace::create(&db_path, &password)
                .map_err(|e| format!("Failed to create: {e}"))?;

            // Cache password if setting is enabled
            let settings = settings::load_settings();
            if settings.cache_workspace_passwords {
                state.workspace_passwords.lock().expect("Mutex poisoned")
                    .insert(folder.clone(), password);
            }

            let new_window = create_workspace_window(&app, &label, &window)?;
            store_workspace(&state, label.clone(), workspace, folder.clone());

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
    password: String,
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
            let workspace = Workspace::open(&db_path, &password)
                .map_err(|e| match e {
                    KrillnotesError::WrongPassword => "WRONG_PASSWORD".to_string(),
                    KrillnotesError::UnencryptedWorkspace => "UNENCRYPTED_WORKSPACE".to_string(),
                    other => format!("Failed to open: {other}"),
                })?;

            // Cache password if setting is enabled
            let settings = settings::load_settings();
            if settings.cache_workspace_passwords {
                state.workspace_passwords.lock().expect("Mutex poisoned")
                    .insert(folder.clone(), password);
            }

            let new_window = create_workspace_window(&app, &label, &window)?;
            store_workspace(&state, label.clone(), workspace, folder.clone());

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
    node_type: String,
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
        workspace.create_note(&pid, add_position, &node_type)
            .map_err(|e| e.to_string())?
    } else {
        // Create root note (parent_id = null, position = 0)
        workspace.create_note_root(&node_type)
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

/// Response type for the `get_schema_fields` Tauri command, bundling field
/// definitions with schema-level title visibility flags.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaInfo {
    fields: Vec<FieldDefinition>,
    title_can_view: bool,
    title_can_edit: bool,
    children_sort: String,
    allowed_parent_types: Vec<String>,
    allowed_children_types: Vec<String>,
    allow_attachments: bool,
    attachment_types: Vec<String>,
    has_view_hook: bool,
    has_hover_hook: bool,
}

/// Returns the field definitions for the schema identified by `node_type`.
///
/// Looks up the schema registered under `node_type` in the calling window's
/// workspace and returns its list of [`FieldDefinition`] values so the
/// frontend can render an appropriate editing form.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// or if `node_type` is not registered in the schema registry.
#[tauri::command]
fn get_schema_fields(
    window: tauri::Window,
    state: State<'_, AppState>,
    node_type: String,
) -> std::result::Result<SchemaInfo, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schema = workspace.script_registry().get_schema(&node_type)
        .map_err(|e: KrillnotesError| e.to_string())?;

    Ok(SchemaInfo {
        has_view_hook: workspace.script_registry().has_view_hook(&node_type),
        has_hover_hook: workspace.script_registry().has_hover_hook(&node_type),
        fields: schema.fields,
        title_can_view: schema.title_can_view,
        title_can_edit: schema.title_can_edit,
        children_sort: schema.children_sort,
        allowed_parent_types: schema.allowed_parent_types,
        allowed_children_types: schema.allowed_children_types,
        allow_attachments: schema.allow_attachments,
        attachment_types: schema.attachment_types,
    })
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
        let has_view_hook = workspace.script_registry().has_view_hook(&name);
        let has_hover_hook = workspace.script_registry().has_hover_hook(&name);
        result.insert(name, SchemaInfo {
            has_view_hook,
            has_hover_hook,
            fields: schema.fields,
            title_can_view: schema.title_can_view,
            title_can_edit: schema.title_can_edit,
            children_sort: schema.children_sort,
            allowed_parent_types: schema.allowed_parent_types,
            allowed_children_types: schema.allowed_children_types,
            allow_attachments: schema.allow_attachments,
            attachment_types: schema.attachment_types,
        });
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
    fields: HashMap<String, FieldValue>,
) -> std::result::Result<Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.update_note(&note_id, title, fields)
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
    target_type: Option<String>,
) -> std::result::Result<Vec<NoteSearchResult>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;
    workspace.search_notes(&query, target_type.as_deref())
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
    new_position: i32,
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
) -> std::result::Result<ScriptMutationResult<UserScript>, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    let (data, load_errors) = workspace.create_user_script(&source_code)
        .map_err(|e| e.to_string())?;
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
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_operations(type_filter.as_deref(), since, until)
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
#[tauri::command]
async fn execute_import(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    folder_path: String,
    password: Option<String>,
    workspace_password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let folder = PathBuf::from(&folder_path);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
    let db_path_buf = folder.join("notes.db");

    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    import_workspace(reader, &db_path_buf, password.as_deref(), &workspace_password)
        .map_err(|e| e.to_string())?;

    // Ensure the attachments directory exists after import
    let _ = std::fs::create_dir_all(folder.join("attachments"));

    let workspace = Workspace::open(&db_path_buf, &workspace_password)
        .map_err(|e| e.to_string())?;
    let label = generate_unique_label(&state, &folder);

    let new_window = create_workspace_window(&app, &label, &window)?;
    store_workspace(&state, label.clone(), workspace, folder);

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

/// Returns the cached password for the workspace at `path`, if one is stored.
///
/// Returns `None` when the `cache_workspace_passwords` setting is disabled or
/// when no password has been cached for this path yet.
#[tauri::command]
fn get_cached_password(
    state: State<'_, AppState>,
    path: String,
) -> Option<String> {
    let settings = settings::load_settings();
    if !settings.cache_workspace_passwords {
        return None;
    }
    let path_buf = PathBuf::from(&path);
    state.workspace_passwords
        .lock()
        .expect("Mutex poisoned")
        .get(&path_buf)
        .cloned()
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

/// Reads `info.json` from `workspace_dir` and returns `(created_at, note_count, attachment_count)`.
/// Returns `(None, None, None)` if the file is missing or malformed.
fn read_info_json(workspace_dir: &Path) -> (Option<i64>, Option<usize>, Option<usize>) {
    let path = workspace_dir.join("info.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (None, None, None),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };
    let created_at = v["created_at"].as_i64();
    let note_count = v["note_count"].as_u64().map(|n| n as usize);
    let attachment_count = v["attachment_count"].as_u64().map(|n| n as usize);
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
            let (created_at, note_count, attachment_count) = read_info_json(&folder);

            entries.push(WorkspaceEntry {
                name: name.to_string(),
                path: folder.display().to_string(),
                is_open,
                last_modified,
                size_bytes,
                created_at,
                note_count,
                attachment_count,
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
/// Does NOT open the duplicated workspace in a window — just creates it on disk.
#[tauri::command]
fn duplicate_workspace(
    source_path: String,
    source_password: String,
    new_name: String,
    new_password: String,
) -> std::result::Result<(), String> {
    let app_settings = settings::load_settings();
    let workspace_dir = PathBuf::from(&app_settings.workspace_directory);
    let dest_folder = workspace_dir.join(&new_name);

    if dest_folder.exists() {
        return Err(format!("A workspace named '{new_name}' already exists."));
    }

    // Open the source workspace to validate password and export.
    let source_db = PathBuf::from(&source_path).join("notes.db");
    let workspace = Workspace::open(&source_db, &source_password)
        .map_err(|e| e.to_string())?;

    // Export to a temp file.
    let mut tmp_file = tempfile::tempfile()
        .map_err(|e| format!("Failed to create temp file: {e}"))?;
    export_workspace(&workspace, &mut tmp_file, Some(&source_password))
        .map_err(|e| e.to_string())?;

    // Import from temp file into dest folder.
    std::fs::create_dir_all(&dest_folder)
        .map_err(|e| format!("Failed to create destination: {e}"))?;
    let dest_db = dest_folder.join("notes.db");

    use std::io::Seek;
    tmp_file
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Seek failed: {e}"))?;
    import_workspace(tmp_file, &dest_db, Some(&source_password), &new_password)
        .map_err(|e| e.to_string())?;

    // Write info.json for the new workspace (best-effort).
    if let Ok(new_ws) = Workspace::open(&dest_db, &new_password) {
        let _ = new_ws.write_info_json();
    }

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
            focused_window: Arc::new(Mutex::new(None)),
            workspace_passwords: Arc::new(Mutex::new(HashMap::new())),
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
            update_note,
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
            purge_operations,
            export_workspace_cmd,
            peek_import_cmd,
            execute_import,
            get_app_version,
            consume_pending_file_open,
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
            get_cached_password,
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
