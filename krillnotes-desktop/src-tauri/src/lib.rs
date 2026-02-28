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

/// Creates a new workspace database at `path` and opens it in a new window.
#[tauri::command]
async fn create_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    if path_buf.exists() {
        return Err("File already exists. Use Open Workspace instead.".to_string());
    }

    match find_window_for_path(&state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &path_buf);
            let workspace = Workspace::create(&path_buf, &password)
                .map_err(|e| format!("Failed to create: {e}"))?;

            // Cache password if setting is enabled
            let settings = settings::load_settings();
            if settings.cache_workspace_passwords {
                state.workspace_passwords.lock().expect("Mutex poisoned")
                    .insert(path_buf.clone(), password);
            }

            let new_window = create_workspace_window(&app, &label, &window)?;
            store_workspace(&state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {label}"))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}

/// Opens an existing workspace database at `path` in a new window.
#[tauri::command]
async fn open_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    if !path_buf.exists() {
        return Err("File does not exist".to_string());
    }

    match find_window_for_path(&state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &path_buf);
            let workspace = Workspace::open(&path_buf, &password)
                .map_err(|e| match e {
                    KrillnotesError::WrongPassword => "WRONG_PASSWORD".to_string(),
                    KrillnotesError::UnencryptedWorkspace => "UNENCRYPTED_WORKSPACE".to_string(),
                    other => format!("Failed to open: {other}"),
                })?;

            // Cache password if setting is enabled
            let settings = settings::load_settings();
            if settings.cache_workspace_passwords {
                state.workspace_passwords.lock().expect("Mutex poisoned")
                    .insert(path_buf.clone(), password);
            }

            let new_window = create_workspace_window(&app, &label, &window)?;
            store_workspace(&state, label.clone(), workspace, path_buf.clone());

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

/// Imports an export archive into a new workspace and opens it in a new window.
#[tauri::command]
async fn execute_import(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    db_path: String,
    password: Option<String>,
    workspace_password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let db_path_buf = PathBuf::from(&db_path);

    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    import_workspace(reader, &db_path_buf, password.as_deref(), &workspace_password)
        .map_err(|e| e.to_string())?;

    let workspace = Workspace::open(&db_path_buf, &workspace_password)
        .map_err(|e| e.to_string())?;
    let label = generate_unique_label(&state, &db_path_buf);

    let new_window = create_workspace_window(&app, &label, &window)?;
    store_workspace(&state, label.clone(), workspace, db_path_buf);

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

/// Entry returned by [`list_workspace_files`], representing a `.db` file
/// in the configured workspace directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    /// Filename without extension.
    name: String,
    /// Absolute path to the `.db` file.
    path: String,
    /// Whether this workspace is currently open in a window.
    is_open: bool,
}

/// Lists all `.db` files in the configured workspace directory.
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

    // Collect currently open workspace paths for the is_open check
    let open_paths: Vec<PathBuf> = state
        .workspace_paths
        .lock()
        .expect("Mutex poisoned")
        .values()
        .cloned()
        .collect();

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&dir)
        .map_err(|e| format!("Failed to read directory: {e}"))?;

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "db").unwrap_or(false) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let is_open = open_paths.iter().any(|p| *p == path);
                entries.push(WorkspaceEntry {
                    name: stem.to_string(),
                    path: path.display().to_string(),
                    is_open,
                });
            }
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
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
        .manage(AppState {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            workspace_paths: Arc::new(Mutex::new(HashMap::new())),
            focused_window: Arc::new(Mutex::new(None)),
            workspace_passwords: Arc::new(Mutex::new(HashMap::new())),
            paste_menu_items: Arc::new(Mutex::new(HashMap::new())),
            workspace_menu_items: Arc::new(Mutex::new(HashMap::new())),
        })
        .on_window_event(|window, event| {
            let label = window.label().to_string();
            let state = window.state::<AppState>();
            match event {
                // Remove workspace state when a window is destroyed so the same
                // file can be reopened after its window has been closed.
                tauri::WindowEvent::Destroyed => {
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
            get_settings,
            update_settings,
            list_themes,
            read_theme,
            write_theme,
            delete_theme,
            read_file_content,
            list_workspace_files,
            get_cached_password,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
