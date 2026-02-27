# Fix: Workspace Menu Items Disabled Until Workspace Open (Issue #32)

## Summary

Workspace-specific menu items (Add Note, Delete Note, Copy Note, Manage Scripts, Operations Log, Export Workspace) are enabled on the startup window before any workspace is loaded. This is confusing — clicking them does nothing.

## Root Cause

`build_menu()` in `menu.rs` creates all items with `enabled(true)` (Tauri default). On macOS, the menu bar is global and shared across all windows, so items are visible and enabled on the startup `main` window. On Windows, the `main` window inherits the app-level default menu which has the same issue.

## Approach

1. Build workspace-specific items with `.enabled(false)` in `build_menu()`.
2. Return their handles in `MenuResult.workspace_items`.
3. Store handles in `AppState.workspace_menu_items` (keyed by `"macos"` on macOS, not needed on Windows).
4. In `create_workspace_window()`: enable the items immediately (macOS: enable global handles; Windows: enable per-window handles before attaching to window).
5. On macOS only: in `WindowEvent::Destroyed`, disable items again if no workspaces remain.

## Items Affected

| Item ID | Menu | Requires workspace? |
|---|---|---|
| `file_export` | File | Yes |
| `edit_add_note` | Edit | Yes |
| `edit_delete_note` | Edit | Yes |
| `edit_copy_note` | Edit | Yes |
| `edit_manage_scripts` | Tools | Yes |
| `view_operations_log` | Tools | Yes |

Paste items (`edit_paste_as_child`, `edit_paste_as_sibling`) already start disabled and are managed by `set_paste_menu_enabled`. Not changed.

## Files

- `krillnotes-desktop/src-tauri/src/menu.rs` — disable items, return handles
- `krillnotes-desktop/src-tauri/src/lib.rs` — store handles, enable on open, disable on close (macOS)

## Step-by-Step Tasks

1. `menu.rs`: Add `workspace_items: Vec<MenuItem<R>>` to `MenuResult`.
2. `menu.rs`: Add `workspace_items: Vec<MenuItem<R>>` to `EditMenuResult`.
3. `menu.rs`: Add `FileMenuResult<R>` struct `{ submenu, workspace_items }`.
4. `menu.rs`: Add `ToolsMenuResult<R>` struct `{ submenu, workspace_items }`.
5. `menu.rs`: Update `build_file_menu` — disable `export_item`, return new struct.
6. `menu.rs`: Update `build_tools_menu` — disable both items, return new struct.
7. `menu.rs`: Update `build_edit_menu` — disable `add_note`, `delete_note`, `copy_note`; add to result.
8. `menu.rs`: Update `build_menu` — collect workspace_items from all sub-results; fix callers.
9. `lib.rs`: Add `workspace_menu_items` field to `AppState`.
10. `lib.rs`: Populate `workspace_menu_items` in `setup()` (macOS).
11. `lib.rs`: In `create_workspace_window()`, enable workspace items (macOS: global; Windows: per-window).
12. `lib.rs`: In `WindowEvent::Destroyed`, disable global workspace items if no workspaces remain (macOS only).
