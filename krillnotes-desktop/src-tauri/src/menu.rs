//! Application menu construction for Krillnotes.

use tauri::{menu::*, AppHandle, Runtime};

/// Builds the application menu with File, Edit, View, and Help submenus.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item or submenu fails to build.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Menu<R>, tauri::Error> {
    let menu = MenuBuilder::new(app)
        // File menu
        .items(&[
            &SubmenuBuilder::new(app, "File")
                .items(&[
                    &MenuItemBuilder::with_id("file_new", "New Workspace")
                        .accelerator("CmdOrCtrl+N")
                        .build(app)?,
                    &MenuItemBuilder::with_id("file_open", "Open Workspace...")
                        .accelerator("CmdOrCtrl+O")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("file_export", "Export Workspace...")
                        .build(app)?,
                    &MenuItemBuilder::with_id("file_import", "Import Workspace...")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::close_window(app, None)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ])
                .build()?,

            // Edit menu
            &SubmenuBuilder::new(app, "Edit")
                .items(&[
                    &MenuItemBuilder::with_id("edit_add_note", "Add Note")
                        .accelerator("CmdOrCtrl+Shift+N")
                        .build(app)?,
                    &MenuItemBuilder::with_id("edit_delete_note", "Delete Note")
                        .accelerator("CmdOrCtrl+Backspace")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("edit_manage_scripts", "Manage Scripts...")
                        .build(app)?,
                    &MenuItemBuilder::with_id("edit_settings", "Settings...")
                        .accelerator("CmdOrCtrl+,")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                ])
                .build()?,

            // View menu
            &SubmenuBuilder::new(app, "View")
                .items(&[
                    &PredefinedMenuItem::fullscreen(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("view_operations_log", "Operations Log...")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("view_refresh", "Refresh")
                        .accelerator("CmdOrCtrl+R")
                        .build(app)?,
                ])
                .build()?,

            // Help menu
            &SubmenuBuilder::new(app, "Help")
                .items(&[
                    &MenuItemBuilder::with_id("help_about", "About Krillnotes")
                        .build(app)?,
                ])
                .build()?,
        ])
        .build()?;

    Ok(menu)
}

/// Builds the View submenu (Fullscreen, separator, Refresh).
///
/// Operations Log is excluded from this helper (it belongs in `build_tools_menu`); the inline `build_menu` still references it until Task 6 wires these helpers in.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_view_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "View")
        .items(&[
            &PredefinedMenuItem::fullscreen(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItemBuilder::with_id("view_refresh", "Refresh")
                .accelerator("CmdOrCtrl+R")
                .build(app)?,
        ])
        .build()
}

/// Builds the Tools submenu (Manage Scripts, Operations Log).
///
/// Item IDs are kept identical to their previous locations so that existing
/// frontend event-routing code continues to work without changes.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_tools_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "Tools")
        .items(&[
            &MenuItemBuilder::with_id("edit_manage_scripts", "Manage Scripts...")
                .build(app)?,
            &MenuItemBuilder::with_id("view_operations_log", "Operations Log...")
                .build(app)?,
        ])
        .build()
}

/// Builds the File submenu.
///
/// On macOS, Quit is intentionally absent — it belongs in the Krillnotes app menu (added in Task 5).
/// On all other platforms, Quit is included at the bottom of File.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_file_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    let new_item = MenuItemBuilder::with_id("file_new", "New Workspace")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let open_item = MenuItemBuilder::with_id("file_open", "Open Workspace...")
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let export_item = MenuItemBuilder::with_id("file_export", "Export Workspace...").build(app)?;
    let import_item = MenuItemBuilder::with_id("file_import", "Import Workspace...").build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let close_item = PredefinedMenuItem::close_window(app, None)?;

    let builder = SubmenuBuilder::new(app, "File")
        .items(&[&new_item, &open_item, &sep1, &export_item, &import_item, &sep2, &close_item]);

    #[cfg(not(target_os = "macos"))]
    let builder = {
        let quit_item = PredefinedMenuItem::quit(app, None)?;
        builder.item(&quit_item)
    };

    builder.build()
}

/// Builds the Edit submenu.
///
/// On macOS, Settings is intentionally absent — it belongs in the Krillnotes app menu (added in Task 5).
/// On all other platforms, Settings... (⌘,) is included between the first separator and the undo/redo block.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_edit_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    let add_note = MenuItemBuilder::with_id("edit_add_note", "Add Note")
        .accelerator("CmdOrCtrl+Shift+N")
        .build(app)?;
    let delete_note = MenuItemBuilder::with_id("edit_delete_note", "Delete Note")
        .accelerator("CmdOrCtrl+Backspace")
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let undo = PredefinedMenuItem::undo(app, None)?;
    let redo = PredefinedMenuItem::redo(app, None)?;
    let copy = PredefinedMenuItem::copy(app, None)?;
    let paste = PredefinedMenuItem::paste(app, None)?;

    let builder = SubmenuBuilder::new(app, "Edit")
        .items(&[&add_note, &delete_note, &sep1]);

    #[cfg(not(target_os = "macos"))]
    let builder = {
        let settings = MenuItemBuilder::with_id("edit_settings", "Settings...")
            .accelerator("CmdOrCtrl+,")
            .build(app)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        builder.item(&settings).item(&sep2)
    };

    builder.items(&[&undo, &redo, &copy, &paste]).build()
}
