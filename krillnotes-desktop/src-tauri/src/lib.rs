pub mod menu;

use tauri::Emitter;

// Re-export core library
pub use krillnotes_core::*;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
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
