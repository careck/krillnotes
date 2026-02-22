# Menu Restructure Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restructure the app menu so macOS shows a proper Krillnotes app menu (with Settings) and a visible File menu, while adding a cross-platform Tools menu for Manage Scripts and Operations Log.

**Architecture:** All changes are confined to `menu.rs`. The file is refactored into focused helper functions assembled conditionally via `#[cfg(target_os = "macos")]`. All menu item IDs are preserved so `lib.rs` and the frontend need no changes — the routing system (`MENU_MESSAGES` + frontend string matching) is untouched.

**Tech Stack:** Rust, Tauri 2.x (`tauri::menu::{MenuBuilder, SubmenuBuilder, MenuItemBuilder, PredefinedMenuItem}`)

---

### Task 1: Create worktree and feature branch

**Step 1: Create worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/menu-restructure -b feat/menu-restructure
```

Expected: new directory `.worktrees/feat/menu-restructure/` with a clean checkout on branch `feat/menu-restructure`.

**Step 2: Verify worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree list
```

Expected: three rows — main checkout on `master`, worktree on `feat/menu-restructure`.

All subsequent work happens inside the worktree:
`/Users/careck/Source/Krillnotes/.worktrees/feat/menu-restructure/`

---

### Task 2: Extract `build_view_menu` and `build_tools_menu` helper functions

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

This task is a pure refactor + new function. No behaviour changes yet.

**Step 1: Replace the View submenu inline code with a `build_view_menu` function**

The current View submenu in `build_menu` is:
```rust
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
```

Extract it — and **drop Operations Log from View** (it moves to Tools):

```rust
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
```

**Step 2: Add `build_tools_menu` — new cross-platform function**

```rust
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
```

Note: `edit_manage_scripts` and `view_operations_log` keep their existing IDs deliberately — the frontend event routing matches on these IDs and must not change.

**Step 3: Compile**

```bash
cargo build --manifest-path krillnotes-desktop/src-tauri/Cargo.toml
```

Expected: compiles successfully (the existing `build_menu` function still builds inline for now — these new functions are unused but that's fine).

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "refactor(menu): extract build_view_menu, add build_tools_menu"
```

---

### Task 3: Extract `build_file_menu` with platform-conditional Quit

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

On macOS, Quit belongs in the app menu (added in Task 5). On all other platforms it stays in File.

**Step 1: Add `build_file_menu` function**

```rust
fn build_file_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    #[cfg(target_os = "macos")]
    {
        return SubmenuBuilder::new(app, "File")
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
                // Quit is intentionally absent on macOS — it lives in the Krillnotes app menu
            ])
            .build();
    }

    #[cfg(not(target_os = "macos"))]
    {
        return SubmenuBuilder::new(app, "File")
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
            .build();
    }
}
```

**Step 2: Compile**

```bash
cargo build --manifest-path krillnotes-desktop/src-tauri/Cargo.toml
```

Expected: PASS.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "refactor(menu): extract build_file_menu, omit Quit on macOS"
```

---

### Task 4: Extract `build_edit_menu` with platform-conditional Settings

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

On macOS, Settings moves to the app menu (Task 5). On all other platforms it stays in Edit.

**Step 1: Add `build_edit_menu` function**

```rust
fn build_edit_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    #[cfg(target_os = "macos")]
    {
        return SubmenuBuilder::new(app, "Edit")
            .items(&[
                &MenuItemBuilder::with_id("edit_add_note", "Add Note")
                    .accelerator("CmdOrCtrl+Shift+N")
                    .build(app)?,
                &MenuItemBuilder::with_id("edit_delete_note", "Delete Note")
                    .accelerator("CmdOrCtrl+Backspace")
                    .build(app)?,
                &PredefinedMenuItem::separator(app)?,
                // Settings is intentionally absent on macOS — it lives in the Krillnotes app menu
                &PredefinedMenuItem::undo(app, None)?,
                &PredefinedMenuItem::redo(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
            ])
            .build();
    }

    #[cfg(not(target_os = "macos"))]
    {
        return SubmenuBuilder::new(app, "Edit")
            .items(&[
                &MenuItemBuilder::with_id("edit_add_note", "Add Note")
                    .accelerator("CmdOrCtrl+Shift+N")
                    .build(app)?,
                &MenuItemBuilder::with_id("edit_delete_note", "Delete Note")
                    .accelerator("CmdOrCtrl+Backspace")
                    .build(app)?,
                &PredefinedMenuItem::separator(app)?,
                &MenuItemBuilder::with_id("edit_settings", "Settings...")
                    .accelerator("CmdOrCtrl+,")
                    .build(app)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::undo(app, None)?,
                &PredefinedMenuItem::redo(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
            ])
            .build();
    }
}
```

**Step 2: Compile**

```bash
cargo build --manifest-path krillnotes-desktop/src-tauri/Cargo.toml
```

Expected: PASS.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "refactor(menu): extract build_edit_menu, omit Settings on macOS"
```

---

### Task 5: Add `build_macos_app_menu` and `build_help_menu` helper functions

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

**Step 1: Add `build_macos_app_menu` (macOS only)**

The macOS app menu is always the first submenu in the bar; macOS automatically relabels it with the app name from the bundle.

```rust
#[cfg(target_os = "macos")]
fn build_macos_app_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "Krillnotes")
        .items(&[
            &PredefinedMenuItem::about(app, None)?,
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
```

> **Compilation note:** If `PredefinedMenuItem::services`, `::hide`, `::hide_others`, or `::show_all` don't compile, check the signature in the [Tauri 2.x `PredefinedMenuItem` docs](https://docs.rs/tauri/latest/tauri/menu/struct.PredefinedMenuItem.html). Some variants take `text: Option<&str>` as a second arg. Try `::hide(app, None)?` etc. If `services` is missing entirely in the version being used, just remove that line.

**Step 2: Add `build_help_menu` (non-macOS only)**

```rust
#[cfg(not(target_os = "macos"))]
fn build_help_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Submenu<R>, tauri::Error> {
    SubmenuBuilder::new(app, "Help")
        .items(&[
            &MenuItemBuilder::with_id("help_about", "About Krillnotes")
                .build(app)?,
        ])
        .build()
}
```

**Step 3: Compile**

```bash
cargo build --manifest-path krillnotes-desktop/src-tauri/Cargo.toml
```

Expected: PASS. These functions are unused until Task 6 wires them in.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "refactor(menu): add build_macos_app_menu and build_help_menu helpers"
```

---

### Task 6: Rewrite `build_menu` to assemble with platform-conditional helpers

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

This is the final wiring step. Replace the entire body of `build_menu` with the assembly logic.

**Step 1: Rewrite `build_menu`**

```rust
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Menu<R>, tauri::Error> {
    let file_menu = build_file_menu(app)?;
    let edit_menu = build_edit_menu(app)?;
    let tools_menu = build_tools_menu(app)?;
    let view_menu = build_view_menu(app)?;

    #[cfg(target_os = "macos")]
    {
        let app_menu = build_macos_app_menu(app)?;
        return MenuBuilder::new(app)
            .items(&[&app_menu, &file_menu, &edit_menu, &tools_menu, &view_menu])
            .build();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let help_menu = build_help_menu(app)?;
        return MenuBuilder::new(app)
            .items(&[&file_menu, &edit_menu, &tools_menu, &view_menu, &help_menu])
            .build();
    }
}
```

**Step 2: Compile**

```bash
cargo build --manifest-path krillnotes-desktop/src-tauri/Cargo.toml
```

Expected: PASS. If there are "unreachable pattern" or "function never used" warnings, they are harmless on the current build target and expected (e.g. `build_help_menu` is dead code on macOS builds).

**Step 3: Run the app and verify manually on macOS**

```bash
cd krillnotes-desktop && cargo tauri dev
```

Verify:
- The menu bar shows: **Krillnotes | File | Edit | Tools | View**
- **Krillnotes** app menu contains: About, Settings... (⌘,), Services, Hide/HideOthers/ShowAll, Quit
- **File** menu contains: New Workspace, Open Workspace, Export, Import, Close Window — NO Quit
- **Edit** menu contains: Add Note, Delete Note, Undo/Redo/Copy/Paste — NO Settings, NO Manage Scripts
- **Tools** menu contains: Manage Scripts..., Operations Log...
- **View** menu contains: Fullscreen, Refresh — NO Operations Log
- No Help menu visible
- Clicking Settings... opens the Settings dialog ✓
- Clicking Manage Scripts... opens the Script Manager dialog ✓
- Clicking Operations Log... opens the Operations Log dialog ✓

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "feat: restructure menu — macOS app menu, Tools menu, platform-conditional layout"
```

---

## Summary of Changes

| File | Change |
|------|--------|
| `menu.rs` | Full refactor into helper functions + `#[cfg]` assembly |
| `lib.rs` | **No changes** — IDs and message strings are preserved |
| `App.tsx` | **No changes** — event routing is ID-based, unaffected |
| `WorkspaceView.tsx` | **No changes** — same |
