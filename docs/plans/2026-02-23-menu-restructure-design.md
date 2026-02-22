# Menu Restructure Design

**Date:** 2026-02-23
**Feature:** Platform-conditional menu restructure

## Summary

The current menu is built as a flat list starting with "File". On macOS, Tauri renders the first submenu as the application menu, so workspace commands appear under the "Krillnotes" app menu instead of a dedicated File menu — and there is no visible File menu at all. This redesign fixes that by introducing a proper macOS application menu and a cross-platform "Tools" menu.

## Goals

- Give macOS users a proper app menu (Krillnotes) with Settings and system items
- Expose a visible File menu with workspace operations on all platforms
- Move configuration-adjacent items (Settings, Manage Scripts) to semantically correct locations
- Add a "Tools" menu as an extensible home for developer/power-user commands (scripts, logs, and future script-registered actions)
- Keep Windows/Linux behaviour unchanged except for the new Tools menu

## Menu Structure

### macOS

| Menu | Items |
|------|-------|
| **Krillnotes** (app menu) | About Krillnotes *(predefined)*, — , Settings... ⌘, , — , Services *(predefined)*, — , Hide / Hide Others / Show All *(predefined)*, — , Quit *(predefined)* |
| **File** | New Workspace ⌘N, Open Workspace... ⌘O, — , Export Workspace..., Import Workspace..., — , Close Window *(predefined)* |
| **Edit** | Add Note ⌘⇧N, Delete Note ⌘⌫, — , Undo / Redo / Copy / Paste *(predefined)* |
| **Tools** | Manage Scripts..., Operations Log... |
| **View** | Fullscreen *(predefined)*, — , Refresh ⌘R |

Help menu is omitted on macOS (its only item, About, moves to the app menu).

### Windows / Linux

| Menu | Items |
|------|-------|
| **File** | New Workspace ⌘N, Open Workspace... ⌘O, — , Export Workspace..., Import Workspace..., — , Close Window *(predefined)*, Quit *(predefined)* |
| **Edit** | Add Note ⌘⇧N, Delete Note ⌘⌫, — , Settings... ⌘, , — , Undo / Redo / Copy / Paste *(predefined)* |
| **Tools** | Manage Scripts..., Operations Log... |
| **View** | Fullscreen *(predefined)*, — , Refresh ⌘R |
| **Help** | About Krillnotes |

## Architecture

### `menu.rs` (only file changed on the backend)

Refactored into focused helper functions:

- `build_file_menu(app, include_quit: bool)` — macOS passes `false`; others `true`
- `build_edit_menu(app, include_settings: bool)` — macOS passes `false`; others `true`
- `build_tools_menu(app)` — shared across all platforms
- `build_view_menu(app)` — shared, Operations Log removed
- `build_help_menu(app)` — non-macOS only
- `build_macos_app_menu(app)` — macOS only
- `build_menu(app)` — top-level assembler using `#[cfg(target_os = "macos")]` blocks

### `lib.rs` (minor update)

- Rename `"Edit > Settings clicked"` message to `"Settings clicked"` in `MENU_MESSAGES`
- Add `("view_operations_log", "Tools > Operations Log clicked")` entry (or keep existing message string — ID is unchanged so frontend routing is unaffected)
- No new Tauri commands needed

### Frontend (`App.tsx`, `WorkspaceView.tsx`)

No handler changes needed. Menu item IDs are preserved:
- `edit_settings` stays the same ID — handler in `App.tsx` fires regardless of which menu the item lives in
- `edit_manage_scripts` stays the same ID — handler in `WorkspaceView.tsx` unchanged
- `view_operations_log` stays the same ID — handler in `WorkspaceView.tsx` unchanged

## Future Extension Point

The Tools menu is designed to be extensible. Scripts will eventually be able to register additional entries via a `register_tool_command` API, which will append items to this menu at runtime.
