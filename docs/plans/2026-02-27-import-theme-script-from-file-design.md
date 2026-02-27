# Design: Import Theme/Script from File (Issue #31)

## Summary

Add "Import from file" to both the Manage Themes dialog and the Script Manager dialog. Users can pick a `.krilltheme` or `.rhai` file from disk; the content is loaded into the existing editor view for review. If a theme/script with the same name already exists, the Save button changes to "Replace" and a confirm dialog guards the overwrite.

## Approach

Option A: one minimal new Rust command (`read_file_content`) reads the picked file path returned by the OS file picker. No new plugin, no schema changes.

## Backend

### New Tauri command — `read_file_content`

Added to `krillnotes-desktop/src-tauri/src/lib.rs` and registered in `generate_handler![]`.

```rust
#[tauri::command]
fn read_file_content(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}
```

Path comes from the OS native file picker, not from user input, so no path-traversal validation is required.

## Frontend — ManageThemesDialog

### List view
- Add "Import from file" button in the footer, alongside "+ New Theme".

### Import flow
1. `open({ filters: [{ name: 'Theme', extensions: ['krilltheme'] }] })`
2. Cancel → no-op.
3. `read_file_content(path)` → raw JSON string.
4. Strip comment lines and parse JSON to extract `name` field.
5. Check `themes` list for a name match → `importConflict: ThemeMeta | null`.
6. Set `editorContent` = file content, `editingMeta` = conflicting ThemeMeta (if any), navigate to editor view.

### Editor view (import path)
- `importConflict` set → yellow warning banner: `"A theme named '${name}' already exists. Saving will replace it."`
- Save button label: `importConflict ? "Replace" : "Save"`
- "Replace" click → `confirm("Replace theme '${name}'? This cannot be undone.")` → existing `handleSave`.

Setting `editingMeta` to the conflicting record means `handleSave` naturally calls `write_theme` with the existing filename — no save logic changes.

## Frontend — ScriptManagerDialog

### List view
- Add "Import from file" button in the header row, alongside "+ Add".

### Import flow
1. `open({ filters: [{ name: 'Rhai Script', extensions: ['rhai'] }] })`
2. Cancel → no-op.
3. `read_file_content(path)` → source text.
4. Parse `@name` front-matter in JS (same logic as Rust `parse_front_matter`).
5. Check `scripts` list for a name match → `importConflict: UserScript | null`.
6. Set `editorContent` = file content, `editingScript` = conflicting UserScript (if any), navigate to editor view.

### Editor view (import path)
- `importConflict` set → yellow warning banner: `"A script named '${name}' already exists. Saving will replace it."`
- Save button label: `importConflict ? "Replace" : "Save"`
- "Replace" click → `confirm("Replace script '${name}'? This cannot be undone.")` → existing `handleSave`.

Setting `editingScript` to the conflicting record means `handleSave` calls `update_user_script` with the correct UUID — no save logic changes.

## What does not change

- Save/update logic in both dialogs is unchanged.
- No new DB columns or migrations.
- No new npm packages or Rust crates.
- Front-matter parsing for conflict detection is done inline in JS (a few lines), not via a new Tauri call.
