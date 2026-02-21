# Workspace Export/Import Design

## Goal

Allow users to export a workspace as a shareable `.zip` file and import someone else's workspace from a zip into a new local workspace. This enables workspace sharing without syncing entire databases.

## Approach

All export/import logic lives in `krillnotes-core` (Approach A). The core exposes functions that take generic `Read`/`Write` traits. The Tauri layer handles file dialogs and calls these functions. This keeps the core self-contained and testable without Tauri.

## App Versioning

Krillnotes gets a semver version number starting at `0.1.0`. The source of truth is `krillnotes-core/Cargo.toml`'s `version` field, accessible at runtime via `env!("CARGO_PKG_VERSION")`.

The app version is embedded in every export. On import, if the export's app version is newer than the running app, a confirmation dialog warns the user before proceeding.

## Export Zip Format

```
my-workspace.krillnotes.zip
├── notes.json
└── scripts/
    ├── scripts.json
    ├── contacts.rhai
    ├── tasks.rhai
    └── projects.rhai
```

### notes.json

```json
{
  "version": 1,
  "appVersion": "0.1.0",
  "notes": [
    {
      "id": "uuid-v4",
      "title": "My Note",
      "nodeType": "TextNote",
      "parentId": null,
      "position": 0,
      "createdAt": 1708617600,
      "modifiedAt": 1708617600,
      "createdBy": 0,
      "modifiedBy": 0,
      "fields": { "body": { "Text": "Hello" } },
      "isExpanded": true
    }
  ]
}
```

Uses existing `Note` serde serialization (camelCase). `version` is the format version (integer). `appVersion` is the Krillnotes semver version that created the export.

### scripts/scripts.json

```json
{
  "scripts": [
    { "filename": "contacts.rhai", "loadOrder": 0, "enabled": true },
    { "filename": "tasks.rhai", "loadOrder": 1, "enabled": true }
  ]
}
```

### .rhai files

Raw `source_code` from the database. Already contains front-matter comments (`// @name:`, `// @description:`).

### Excluded from export

- `operations` table (machine-specific sync log, transient, redundant)
- `workspace_meta` keys (`device_id`, `current_user_id`, `selected_note_id` — per-device state)

## Export Flow

1. User clicks File > "Export Workspace..."
2. Menu event fires to frontend
3. Frontend shows save dialog (`@tauri-apps/plugin-dialog`) with filter `*.zip`, default name from workspace filename
4. Frontend calls Tauri command `export_workspace(windowLabel, path)`
5. Tauri looks up `Workspace` from `AppState`, calls `core::export::export_workspace(&workspace, file)`
6. Core fetches all notes and scripts, writes zip archive

## Import Flow

1. User clicks File > "Import Workspace..."
2. Menu event fires to frontend
3. Frontend shows open dialog with filter `*.zip`
4. Frontend shows save dialog asking where to save the new `.db` file
5. Frontend calls Tauri command `import_workspace(zipPath, dbPath)`
6. Core reads zip, validates format and version, returns `ImportResult` with `appVersion`
7. If export `appVersion` is newer than current app, frontend shows confirmation dialog: "This was made with a newer version. Import anyway?"
8. If user confirms (or versions are compatible), core creates new workspace and inserts all notes and scripts
9. Tauri opens the new workspace in a new window

Note: The import is a two-phase operation. The first Tauri command (`peek_import`) reads the zip metadata to check versions. If the user confirms, a second command (`execute_import`) performs the actual import. This avoids doing work that might be cancelled.

## Menu Changes

```
File
  New Workspace          Cmd/Ctrl+N
  Open Workspace...      Cmd/Ctrl+O
  ─────────────────────────────────
  Export Workspace...                    NEW
  Import Workspace...                    NEW
  ─────────────────────────────────
  Close Window
  Quit
```

No keyboard shortcuts for export/import.

## Core Module Structure

New module: `krillnotes-core/src/core/export.rs`

### Structs

- `ExportNotes` — `{ version: u32, app_version: String, notes: Vec<Note> }`
- `ScriptManifestEntry` — `{ filename: String, load_order: i32, enabled: bool }`
- `ScriptManifest` — `{ scripts: Vec<ScriptManifestEntry> }`
- `ImportResult` — `{ app_version: String, note_count: usize, script_count: usize }`

### Functions

- `export_workspace(workspace: &Workspace, writer: impl Write + Seek) -> Result<()>`
- `import_workspace(reader: impl Read + Seek, db_path: &Path) -> Result<ImportResult>`
- `peek_import(reader: impl Read + Seek) -> Result<ImportResult>` — reads metadata without creating a workspace
- `slugify_script_name(name: &str) -> String` — converts script names to filenames

### Error Handling

New `ExportError` enum using `thiserror`:

- `Io(std::io::Error)`
- `Zip(zip::result::ZipError)`
- `Json(serde_json::Error)`
- `InvalidFormat(String)`
- `Database(String)`

### Dependencies

- `zip` crate added to `krillnotes-core/Cargo.toml`

## Frontend Integration

- **Export:** handled in `WorkspaceView.tsx` (requires open workspace context)
- **Import:** handled in `App.tsx` (available without a workspace, creates a new one)

Both use `@tauri-apps/plugin-dialog` (already a dependency) for file dialogs.

## Tauri Commands

- `export_workspace(window_label: String, path: String) -> Result<(), String>`
- `peek_import(zip_path: String) -> Result<ImportResult, String>`
- `execute_import(zip_path: String, db_path: String) -> Result<(), String>`

## Testing

Unit tests in `krillnotes-core/src/core/export.rs`:

- **Round-trip test:** Create workspace, add notes (nested tree) and scripts, export to `Vec<u8>` via `Cursor`, import into new workspace, assert all data matches
  - Note: This test works because export currently reads through the `Workspace` API. If export ever reads directly from SQLite, this in-memory approach will need revision.
- **Slugify test:** Name-to-filename conversion (spaces, special chars, duplicates)
- **Invalid zip test:** Import garbage bytes, assert `ExportError::Zip`
- **Missing files test:** Zip without `notes.json`, assert `ExportError::InvalidFormat`
- **Version check test:** Export with `appVersion` "99.0.0", verify `ImportResult.app_version` returned correctly
