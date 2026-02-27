//! Application menu construction for Krillnotes.

use tauri::{menu::*, AppHandle, Runtime};

/// Return type of [`build_menu`], carrying both the assembled menu and the
/// paste-note `MenuItem` handles needed for dynamic enable/disable.
pub struct MenuResult<R: Runtime> {
    pub menu: Menu<R>,
    pub paste_as_child: MenuItem<R>,
    pub paste_as_sibling: MenuItem<R>,
    /// Workspace-specific items that start disabled and are enabled when a
    /// workspace window opens. Includes Add Note, Delete Note, Copy Note,
    /// Manage Scripts, Operations Log, and Export Workspace.
    pub workspace_items: Vec<MenuItem<R>>,
}

/// Return type of [`build_edit_menu`], exposing the paste handles alongside the submenu.
struct EditMenuResult<R: Runtime> {
    submenu: Submenu<R>,
    paste_as_child: MenuItem<R>,
    paste_as_sibling: MenuItem<R>,
    workspace_items: Vec<MenuItem<R>>,
}

/// Return type of [`build_file_menu`].
struct FileMenuResult<R: Runtime> {
    submenu: Submenu<R>,
    workspace_items: Vec<MenuItem<R>>,
}

/// Return type of [`build_tools_menu`].
struct ToolsMenuResult<R: Runtime> {
    submenu: Submenu<R>,
    workspace_items: Vec<MenuItem<R>>,
}

/// Builds the application menu using platform-conditional assembly.
///
/// On macOS: App menu (Krillnotes), File, Edit, Tools, View.
/// On other platforms: File, Edit, Tools, View, Help.
///
/// Workspace-specific items are built with `enabled(false)` and their handles
/// are returned in [`MenuResult::workspace_items`] so the caller can enable
/// them when a workspace window opens.
///
/// Returns a [`MenuResult`] with the assembled menu and paste item handles.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item or submenu fails to build.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> Result<MenuResult<R>, tauri::Error> {
    let file_result = build_file_menu(app)?;
    let edit_result = build_edit_menu(app)?;
    let tools_result = build_tools_menu(app)?;
    let view_menu = build_view_menu(app)?;

    let mut workspace_items = Vec::new();
    workspace_items.extend(file_result.workspace_items);
    workspace_items.extend(edit_result.workspace_items);
    workspace_items.extend(tools_result.workspace_items);

    #[cfg(target_os = "macos")]
    {
        let app_menu = build_macos_app_menu(app)?;
        let menu = MenuBuilder::new(app)
            .items(&[&app_menu, &file_result.submenu, &edit_result.submenu, &tools_result.submenu, &view_menu])
            .build()?;
        return Ok(MenuResult {
            menu,
            paste_as_child: edit_result.paste_as_child,
            paste_as_sibling: edit_result.paste_as_sibling,
            workspace_items,
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let help_menu = build_help_menu(app)?;
        let menu = MenuBuilder::new(app)
            .items(&[&file_result.submenu, &edit_result.submenu, &tools_result.submenu, &view_menu, &help_menu])
            .build()?;
        return Ok(MenuResult {
            menu,
            paste_as_child: edit_result.paste_as_child,
            paste_as_sibling: edit_result.paste_as_sibling,
            workspace_items,
        });
    }
}

/// Builds the View submenu (Fullscreen, separator, Refresh).
///
/// Operations Log is excluded from this helper — it belongs in `build_tools_menu`.
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
/// Both items require an active workspace and are built with `enabled(false)`.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_tools_menu<R: Runtime>(app: &AppHandle<R>) -> Result<ToolsMenuResult<R>, tauri::Error> {
    let manage_scripts = MenuItemBuilder::with_id("edit_manage_scripts", "Manage Scripts...")
        .enabled(false)
        .build(app)?;
    let operations_log = MenuItemBuilder::with_id("view_operations_log", "Operations Log...")
        .enabled(false)
        .build(app)?;

    let submenu = SubmenuBuilder::new(app, "Tools")
        .items(&[&manage_scripts, &operations_log])
        .build()?;

    Ok(ToolsMenuResult {
        submenu,
        workspace_items: vec![manage_scripts, operations_log],
    })
}

/// Builds the File submenu.
///
/// Export Workspace requires an active workspace and is built with `enabled(false)`.
/// On macOS, Quit is intentionally absent — it belongs in the Krillnotes app menu.
/// On all other platforms, Quit is included at the bottom of File.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_file_menu<R: Runtime>(app: &AppHandle<R>) -> Result<FileMenuResult<R>, tauri::Error> {
    let new_item = MenuItemBuilder::with_id("file_new", "New Workspace")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let open_item = MenuItemBuilder::with_id("file_open", "Open Workspace...")
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let export_item = MenuItemBuilder::with_id("file_export", "Export Workspace...")
        .enabled(false)
        .build(app)?;
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

    let submenu = builder.build()?;
    Ok(FileMenuResult {
        submenu,
        workspace_items: vec![export_item],
    })
}

/// Builds the Edit submenu.
///
/// Add Note, Delete Note, and Copy Note require an active workspace and are
/// built with `enabled(false)`. On macOS, Settings is intentionally absent —
/// it belongs in the Krillnotes app menu. On all other platforms, Settings...
/// (⌘,) is included between the first separator and the undo/redo block.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
fn build_edit_menu<R: Runtime>(app: &AppHandle<R>) -> Result<EditMenuResult<R>, tauri::Error> {
    let add_note = MenuItemBuilder::with_id("edit_add_note", "Add Note")
        .accelerator("CmdOrCtrl+Shift+N")
        .enabled(false)
        .build(app)?;
    let delete_note = MenuItemBuilder::with_id("edit_delete_note", "Delete Note")
        .accelerator("CmdOrCtrl+Backspace")
        .enabled(false)
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let copy_note = MenuItemBuilder::with_id("edit_copy_note", "Copy Note")
        .enabled(false)
        .build(app)?;
    let paste_child = MenuItemBuilder::with_id("edit_paste_as_child", "Paste as Child")
        .enabled(false)
        .build(app)?;
    let paste_sibling = MenuItemBuilder::with_id("edit_paste_as_sibling", "Paste as Sibling")
        .enabled(false)
        .build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let undo = PredefinedMenuItem::undo(app, None)?;
    let redo = PredefinedMenuItem::redo(app, None)?;
    let copy = PredefinedMenuItem::copy(app, None)?;
    let paste = PredefinedMenuItem::paste(app, None)?;

    let builder = SubmenuBuilder::new(app, "Edit")
        .items(&[&add_note, &delete_note, &sep1, &copy_note, &paste_child, &paste_sibling, &sep2]);

    #[cfg(not(target_os = "macos"))]
    let builder = {
        let settings = MenuItemBuilder::with_id("edit_settings", "Settings...")
            .accelerator("CmdOrCtrl+,")
            .build(app)?;
        let sep3 = PredefinedMenuItem::separator(app)?;
        builder.item(&settings).item(&sep3)
    };

    let submenu = builder.items(&[&undo, &redo, &copy, &paste]).build()?;
    Ok(EditMenuResult {
        submenu,
        paste_as_child: paste_child,
        paste_as_sibling: paste_sibling,
        workspace_items: vec![add_note, delete_note, copy_note],
    })
}

/// Builds the macOS app menu (the first menu in the menu bar, labeled with the app name).
///
/// macOS replaces whatever label string you pass to `SubmenuBuilder` with the bundle display name,
/// so the "Krillnotes" string here is a no-op at runtime but makes the source intent clear.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
#[cfg(target_os = "macos")]
fn build_macos_app_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "Krillnotes")
        .items(&[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItemBuilder::with_id("edit_settings", "Settings...")
                .accelerator("CmdOrCtrl+,")
                .build(app)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ])
        .build()
}

/// Builds the Help submenu for non-macOS platforms.
///
/// On macOS, About is in the app menu instead; this submenu is excluded there.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item fails to build.
#[cfg(not(target_os = "macos"))]
fn build_help_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "Help")
        .items(&[
            &MenuItemBuilder::with_id("help_about", "About Krillnotes")
                .build(app)?,
        ])
        .build()
}
