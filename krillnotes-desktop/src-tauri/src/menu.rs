//! Application menu construction for Krillnotes.

use serde_json::Value;
use tauri::{menu::*, AppHandle, Runtime};

/// Return type of [`build_menu`].
pub struct MenuResult<R: Runtime> {
    pub menu: Menu<R>,
    pub paste_as_child: MenuItem<R>,
    pub paste_as_sibling: MenuItem<R>,
    pub workspace_items: Vec<MenuItem<R>>,
}

struct EditMenuResult<R: Runtime> {
    submenu: Submenu<R>,
    paste_as_child: MenuItem<R>,
    paste_as_sibling: MenuItem<R>,
    workspace_items: Vec<MenuItem<R>>,
}

struct FileMenuResult<R: Runtime> {
    submenu: Submenu<R>,
    workspace_items: Vec<MenuItem<R>>,
}

struct ToolsMenuResult<R: Runtime> {
    submenu: Submenu<R>,
    workspace_items: Vec<MenuItem<R>>,
}

/// Builds the application menu with labels from `strings` (the `menu` section
/// of a locale JSON, as returned by [`crate::locales::menu_strings`]).
///
/// On macOS: App menu (Krillnotes), File, Edit, Tools, View.
/// On other platforms: File, Edit, Tools, View, Help.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<MenuResult<R>, tauri::Error> {
    let file_result = build_file_menu(app, strings)?;
    let edit_result = build_edit_menu(app, strings)?;
    let tools_result = build_tools_menu(app, strings)?;
    let view_menu = build_view_menu(app, strings)?;

    let mut workspace_items = Vec::new();
    workspace_items.extend(file_result.workspace_items);
    workspace_items.extend(edit_result.workspace_items);
    workspace_items.extend(tools_result.workspace_items);

    #[cfg(target_os = "macos")]
    {
        let app_menu = build_macos_app_menu(app, strings)?;
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
        let help_menu = build_help_menu(app, strings)?;
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

fn s<'a>(strings: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    strings[key].as_str().unwrap_or(fallback)
}

fn build_view_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, s(strings, "view", "View"))
        .items(&[
            &PredefinedMenuItem::fullscreen(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItemBuilder::with_id("view_refresh", s(strings, "refresh", "Refresh"))
                .accelerator("CmdOrCtrl+R")
                .build(app)?,
        ])
        .build()
}

fn build_tools_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<ToolsMenuResult<R>, tauri::Error> {
    let manage_scripts = MenuItemBuilder::with_id("edit_manage_scripts", s(strings, "manageScripts", "Manage Scripts..."))
        .enabled(false)
        .build(app)?;
    let operations_log = MenuItemBuilder::with_id("view_operations_log", s(strings, "operationsLog", "Operations Log..."))
        .enabled(false)
        .build(app)?;

    let submenu = SubmenuBuilder::new(app, s(strings, "tools", "Tools"))
        .items(&[&manage_scripts, &operations_log])
        .build()?;

    Ok(ToolsMenuResult {
        submenu,
        workspace_items: vec![manage_scripts],
    })
}

fn build_file_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<FileMenuResult<R>, tauri::Error> {
    let new_item = MenuItemBuilder::with_id("file_new", s(strings, "newWorkspace", "New Workspace"))
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let open_item = MenuItemBuilder::with_id("file_open", s(strings, "openWorkspace", "Open Workspace..."))
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let export_item = MenuItemBuilder::with_id("file_export", s(strings, "exportWorkspace", "Export Workspace..."))
        .enabled(false)
        .build(app)?;
    let import_item = MenuItemBuilder::with_id("file_import", s(strings, "importWorkspace", "Import Workspace..."))
        .build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let close_item = PredefinedMenuItem::close_window(app, None)?;

    let builder = SubmenuBuilder::new(app, s(strings, "file", "File"))
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

fn build_edit_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<EditMenuResult<R>, tauri::Error> {
    let add_note = MenuItemBuilder::with_id("edit_add_note", s(strings, "addNote", "Add Note"))
        .accelerator("CmdOrCtrl+Shift+N")
        .enabled(false)
        .build(app)?;
    let delete_note = MenuItemBuilder::with_id("edit_delete_note", s(strings, "deleteNote", "Delete Note"))
        .accelerator("CmdOrCtrl+Backspace")
        .enabled(false)
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let copy_note = MenuItemBuilder::with_id("edit_copy_note", s(strings, "copyNote", "Copy Note"))
        .enabled(false)
        .build(app)?;
    let paste_child = MenuItemBuilder::with_id("edit_paste_as_child", s(strings, "pasteAsChild", "Paste as Child"))
        .enabled(false)
        .build(app)?;
    let paste_sibling = MenuItemBuilder::with_id("edit_paste_as_sibling", s(strings, "pasteAsSibling", "Paste as Sibling"))
        .enabled(false)
        .build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let workspace_properties = MenuItemBuilder::with_id("workspace_properties", s(strings, "workspaceProperties", "Workspace Properties\u{2026}"))
        .enabled(false)
        .build(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let undo       = PredefinedMenuItem::undo(app, None)?;
    let redo       = PredefinedMenuItem::redo(app, None)?;
    let cut        = PredefinedMenuItem::cut(app, None)?;
    let copy       = PredefinedMenuItem::copy(app, None)?;
    let paste      = PredefinedMenuItem::paste(app, None)?;
    let select_all = PredefinedMenuItem::select_all(app, None)?;

    let builder = SubmenuBuilder::new(app, s(strings, "edit", "Edit"))
        .items(&[&add_note, &delete_note, &sep1, &copy_note, &paste_child, &paste_sibling,
                 &sep2, &workspace_properties, &sep3]);

    #[cfg(not(target_os = "macos"))]
    let builder = {
        let settings = MenuItemBuilder::with_id("edit_settings", s(strings, "settings", "Settings..."))
            .accelerator("CmdOrCtrl+,")
            .build(app)?;
        let sep4 = PredefinedMenuItem::separator(app)?;
        builder.item(&settings).item(&sep4)
    };

    let submenu = builder.items(&[&undo, &redo, &cut, &copy, &paste, &select_all]).build()?;
    Ok(EditMenuResult {
        submenu,
        paste_as_child: paste_child,
        paste_as_sibling: paste_sibling,
        workspace_items: vec![add_note, delete_note, copy_note, workspace_properties],
    })
}

#[cfg(target_os = "macos")]
fn build_macos_app_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "Krillnotes")
        .items(&[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItemBuilder::with_id("edit_settings", s(strings, "settings", "Settings..."))
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

#[cfg(not(target_os = "macos"))]
fn build_help_menu<R: Runtime>(app: &AppHandle<R>, strings: &Value) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, s(strings, "help", "Help"))
        .items(&[
            &MenuItemBuilder::with_id("help_about", s(strings, "aboutKrillnotes", "About Krillnotes"))
                .build(app)?,
        ])
        .build()
}
