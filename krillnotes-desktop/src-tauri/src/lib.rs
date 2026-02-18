pub mod menu;

use tauri::Emitter;

// Re-export core library
pub use krillnotes_core::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

#[derive(Clone)]
pub struct AppState {
    pub workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub filename: String,
    pub path: String,
    pub note_count: usize,
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn generate_unique_label(state: &AppState, path: &PathBuf) -> String {
    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");

    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let mut label = filename.to_string();
    let mut counter = 2;

    while workspaces.contains_key(&label) {
        label = format!("{}-{}", filename, counter);
        counter += 1;
    }

    label
}

fn find_window_for_path(state: &AppState, path: &PathBuf) -> Option<String> {
    state.workspace_paths.lock()
        .expect("Mutex poisoned")
        .iter()
        .find(|(_, p)| *p == path)
        .map(|(label, _)| label.clone())
}

fn focus_window(app: &AppHandle, label: &str) -> std::result::Result<(), String> {
    app.get_webview_window(label)
        .ok_or_else(|| "Window not found".to_string())
        .and_then(|window| {
            window.set_focus()
                .map_err(|e| format!("Failed to focus: {}", e))
        })
}

fn create_workspace_window(
    app: &AppHandle,
    label: &str
) -> std::result::Result<tauri::WebviewWindow, String> {
    tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App("index.html".into())
    )
    .title(&format!("Krillnotes - {}", label))
    .inner_size(1024.0, 768.0)
    .build()
    .map_err(|e| format!("Failed to create window: {}", e))
}

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

    Ok(WorkspaceInfo {
        filename,
        path: path.display().to_string(),
        note_count,
    })
}

#[tauri::command]
async fn create_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    // Validate path doesn't exist
    if path_buf.exists() {
        return Err("File already exists. Use Open Workspace instead.".to_string());
    }

    // Check if this path is already open
    match find_window_for_path(&*state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&*state, &path_buf);
            let workspace = Workspace::create(&path_buf)
                .map_err(|e| format!("Failed to create: {}", e))?;

            let new_window = create_workspace_window(&app, &label)?;
            store_workspace(&*state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {}", label))
                .map_err(|e| e.to_string())?;

            // Close main window if this is first workspace
            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&*state, &label)
        }
    }
}

#[tauri::command]
async fn open_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    // Validate path exists
    if !path_buf.exists() {
        return Err("File does not exist".to_string());
    }

    // Check for duplicate open
    match find_window_for_path(&*state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&*state, &path_buf);
            let workspace = Workspace::open(&path_buf)
                .map_err(|e| format!("Failed to open: {}", e))?;

            let new_window = create_workspace_window(&app, &label)?;
            store_workspace(&*state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {}", label))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&*state, &label)
        }
    }
}

#[tauri::command]
fn get_workspace_info(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<WorkspaceInfo, String> {
    get_workspace_info_internal(&*state, window.label())
}

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

// Note: Window cleanup is handled implicitly through Drop traits.
// Tauri v2 automatically cleans up resources when windows are destroyed.
// The AppState HashMap entries will be cleaned up when the app exits.

const MENU_MESSAGES: &[(&str, &str)] = &[
    ("file_new", "File > New Workspace clicked"),
    ("file_open", "File > Open Workspace clicked"),
    ("edit_add_note", "Edit > Add Note clicked"),
    ("edit_delete_note", "Edit > Delete Note clicked"),
    ("view_refresh", "View > Refresh clicked"),
    ("help_about", "Help > About Krillnotes clicked"),
];

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    MENU_MESSAGES.iter()
        .find(|(id, _)| id == &event.id().as_ref())
        .map(|(_, message)| app.emit("menu-action", message))
        .transpose()
        .ok();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            workspace_paths: Arc::new(Mutex::new(HashMap::new())),
        })
        .setup(|app| {
            let menu = menu::build_menu(app.handle())?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(handle_menu_event)
        .invoke_handler(tauri::generate_handler![
            greet,
            create_workspace,
            open_workspace,
            get_workspace_info,
            list_notes,
            get_node_types,
            toggle_note_expansion,
            set_selected_note,
            create_note_with_type,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
