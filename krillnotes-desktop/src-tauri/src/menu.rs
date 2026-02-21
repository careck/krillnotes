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
