pub mod menu;

use tauri::Emitter;

// Re-export core library
pub use krillnotes_core::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State, Window};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let menu = menu::build_menu(app.handle())?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            let message = match event.id().as_ref() {
                "file_new" => "File > New Workspace clicked",
                "file_open" => "File > Open Workspace clicked",
                "edit_add_note" => "Edit > Add Note clicked",
                "edit_delete_note" => "Edit > Delete Note clicked",
                "view_refresh" => "View > Refresh clicked",
                "help_about" => "Help > About Krillnotes clicked",
                _ => return, // Ignore unknown events
            };

            // Emit event to frontend
            app.emit("menu-action", message).ok();
        })
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
