# Native Menu i18n Design

**Goal:** Translate the Tauri native application menu using the same locale JSON files as the React frontend, with immediate rebuild when the user saves a language change.

**Architecture:** A `build.rs` build script auto-discovers all locale JSON files and embeds them in the binary at compile time. At startup the saved language is read and used to build the menu. When the user saves a language change, `update_settings` rebuilds and reapplies menus for all open windows.

**Tech Stack:** Rust `build.rs`, `serde_json`, Tauri menu API (`app.set_menu`, `window.set_menu`).

---

## Section 1 — Translation storage

A `menu` block is added to every locale JSON (the same files React already uses). These are the only keys Rust needs to read; all other sections are ignored by the Rust side.

Example (`en.json`):

```json
"menu": {
  "file": "File",
  "edit": "Edit",
  "tools": "Tools",
  "view": "View",
  "help": "Help",
  "newWorkspace": "New Workspace",
  "openWorkspace": "Open Workspace…",
  "exportWorkspace": "Export Workspace…",
  "importWorkspace": "Import Workspace…",
  "addNote": "Add Note",
  "deleteNote": "Delete Note",
  "copyNote": "Copy Note",
  "pasteAsChild": "Paste as Child",
  "pasteAsSibling": "Paste as Sibling",
  "workspaceProperties": "Workspace Properties…",
  "settings": "Settings…",
  "manageScripts": "Manage Scripts…",
  "operationsLog": "Operations Log…",
  "refresh": "Refresh",
  "aboutKrillnotes": "About Krillnotes"
}
```

A `build.rs` in `krillnotes-desktop/src-tauri/` scans `../src/i18n/locales/*.json` at compile time and generates `src/generated/locales.rs` containing a static slice of `(&str, &str)` tuples — language code derived from the filename, JSON content embedded via `include_str!`. Adding a new language requires only dropping a new JSON file in the locales directory; no Rust source changes are needed.

## Section 2 — Runtime lookup

`menu_strings(lang: &str) -> serde_json::Value` iterates the generated `LOCALES` slice, finds the matching entry, parses the JSON, and returns the `menu` object. If the language is not found (e.g. a partially-added translation) it falls back to English. If a specific key is missing within the found locale, `as_str().unwrap_or(english_fallback)` ensures the menu never shows a raw key.

`build_menu` gains a `strings: &serde_json::Value` parameter and uses `strings["key"].as_str().unwrap_or("…")` throughout instead of hardcoded string literals. The call sites (startup `setup()` and `create_workspace_window`) load the current language from settings and pass the resolved strings in.

## Section 3 — Menu rebuild on language change

`update_settings` gains `app: AppHandle` and `state: State<AppState>` parameters (Tauri injects these automatically — no frontend change needed).

After persisting the new settings, if the language field changed, a `rebuild_menus(&app, &state, lang)` helper is called:

- **macOS:** One global menu. Call `app.set_menu(new_menu)`. Update `paste_menu_items["macos"]` and `workspace_menu_items["macos"]` in `AppState` with the new handles so dynamic enable/disable keeps working.

- **Windows:** Each window owns its own menu. Iterate `app.webview_windows()`. For each window: build a fresh menu, check if the label exists in `workspace_paths` (if so, enable workspace items), call `window.set_menu(new_menu)`, update `AppState` handles for that label.

### Startup path

In `setup()`, load settings first, extract the language, call `menu_strings(lang)`, pass the result to `build_menu`. Same change applies to `create_workspace_window` for the per-window menus built on Windows.
