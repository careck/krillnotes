# Workspace Export/Import Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow users to export a workspace as a `.zip` file (notes JSON + individual `.rhai` scripts) and import a zip into a new workspace, with app version checking.

**Architecture:** All export/import logic lives in `krillnotes-core` in a new `export` module. The core exposes functions taking generic `Read+Seek`/`Write+Seek` traits. The Tauri layer handles file dialogs and calls core functions. A new `ExportError` type wraps zip/JSON/format errors, converting to the existing `KrillnotesError` at boundaries.

**Tech Stack:** Rust `zip` crate for archive handling, existing `serde`/`serde_json` for serialization, `@tauri-apps/plugin-dialog` on the frontend for file pickers.

---

## Task 1: Add `zip` dependency to `krillnotes-core`

**Files:**
- Modify: `krillnotes-core/Cargo.toml:18` (add after `include_dir`)

**Step 1: Add the dependency**

In `krillnotes-core/Cargo.toml`, add `zip` after line 18 (`include_dir = "0.7"`):

```toml
zip = { version = "2", default-features = false, features = ["deflate"] }
```

**Step 2: Verify it compiles**

Run: `cargo check -p krillnotes-core`
Expected: compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-core/Cargo.toml Cargo.lock
git commit -m "deps: add zip crate to krillnotes-core"
```

---

## Task 2: Create the export module with types and `ExportError`

**Files:**
- Create: `krillnotes-core/src/core/export.rs`
- Modify: `krillnotes-core/src/core/mod.rs:1` (add module declaration)
- Modify: `krillnotes-core/src/lib.rs:13` (add re-export)

**Step 1: Write the failing test**

Create `krillnotes-core/src/core/export.rs` with just the test and type stubs:

```rust
//! Workspace export and import as `.zip` archives.

use serde::{Deserialize, Serialize};

use crate::core::note::Note;

/// Top-level JSON structure in `notes.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportNotes {
    /// Format version (currently 1).
    pub version: u32,
    /// Krillnotes app version that created this export.
    pub app_version: String,
    /// All notes in the workspace.
    pub notes: Vec<Note>,
}

/// One entry in `scripts/scripts.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptManifestEntry {
    /// Filename within the `scripts/` directory (e.g. `"contacts.rhai"`).
    pub filename: String,
    /// Execution order.
    pub load_order: i32,
    /// Whether the script was enabled at export time.
    pub enabled: bool,
}

/// The `scripts/scripts.json` manifest.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScriptManifest {
    pub scripts: Vec<ScriptManifestEntry>,
}

/// Result returned after reading an export archive's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// The app version that created the export.
    pub app_version: String,
    /// Number of notes in the archive.
    pub note_count: usize,
    /// Number of user scripts in the archive.
    pub script_count: usize,
}

/// Errors specific to export/import operations.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid export format: {0}")]
    InvalidFormat(String),

    #[error("Database error: {0}")]
    Database(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_notes_serialization() {
        let export = ExportNotes {
            version: 1,
            app_version: "0.1.0".to_string(),
            notes: vec![],
        };
        let json = serde_json::to_string(&export).unwrap();
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"appVersion\":\"0.1.0\""));
        assert!(json.contains("\"notes\":[]"));

        let parsed: ExportNotes = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.app_version, "0.1.0");
    }

    #[test]
    fn test_script_manifest_serialization() {
        let manifest = ScriptManifest {
            scripts: vec![ScriptManifestEntry {
                filename: "contacts.rhai".to_string(),
                load_order: 0,
                enabled: true,
            }],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("\"loadOrder\":0"));
        assert!(json.contains("\"filename\":\"contacts.rhai\""));
    }
}
```

**Step 2: Register the module**

In `krillnotes-core/src/core/mod.rs`, add after line 1 (`pub mod delete;`):

```rust
pub mod export;
```

In `krillnotes-core/src/lib.rs`, add a new re-export line inside the `pub use core::` block (after line 15, the `error` line):

```rust
    export::{ExportError, ExportNotes, ImportResult, ScriptManifest, ScriptManifestEntry},
```

**Step 3: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core export`
Expected: 2 tests pass

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/export.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/lib.rs
git commit -m "feat: add export module with types and ExportError"
```

---

## Task 3: Implement `slugify_script_name`

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`

**Step 1: Write the failing test**

Add to the `tests` module in `export.rs`:

```rust
    #[test]
    fn test_slugify_script_name() {
        assert_eq!(slugify_script_name("Contacts"), "contacts");
        assert_eq!(slugify_script_name("My Tasks"), "my-tasks");
        assert_eq!(slugify_script_name("Hello World!"), "hello-world");
        assert_eq!(slugify_script_name("  Spaced  Out  "), "spaced-out");
        assert_eq!(slugify_script_name(""), "script");
        assert_eq!(slugify_script_name("---"), "script");
    }
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_slugify`
Expected: FAIL — function not found

**Step 3: Implement the function**

Add to `export.rs` (above the `tests` module):

```rust
/// Converts a script name into a safe filename stem.
///
/// Lowercases, replaces spaces and non-alphanumeric characters with hyphens,
/// collapses consecutive hyphens, and trims leading/trailing hyphens.
/// Returns `"script"` if the result would be empty.
pub fn slugify_script_name(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug: String = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() { "script".to_string() } else { slug }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_slugify`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/export.rs
git commit -m "feat: add slugify_script_name for export filenames"
```

---

## Task 4: Implement `export_workspace`

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`

**Step 1: Write the failing test**

Add to the `tests` module. This test creates a workspace with notes and scripts, exports it, then reads back the zip to verify contents:

```rust
    use crate::Workspace;
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    #[test]
    fn test_export_workspace_creates_valid_zip() {
        // Create a workspace with a note and a script
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

        // Add a user script
        let script_source = "// @name: Contacts\n// @description: Contact cards\nschema(\"Contact\", #{});";
        ws.create_user_script(script_source).unwrap();

        // Export to a buffer
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf)).unwrap();

        // Read back the zip and verify structure
        let reader = Cursor::new(&buf);
        let mut archive = zip::ZipArchive::new(reader).unwrap();

        // Must contain notes.json
        let notes_file = archive.by_name("notes.json").unwrap();
        let notes_data: ExportNotes = serde_json::from_reader(notes_file).unwrap();
        assert_eq!(notes_data.version, 1);
        assert!(!notes_data.app_version.is_empty());
        assert!(!notes_data.notes.is_empty()); // at least the root note

        // Must contain scripts/scripts.json
        let manifest_file = archive.by_name("scripts/scripts.json").unwrap();
        let manifest: ScriptManifest = serde_json::from_reader(manifest_file).unwrap();
        assert_eq!(manifest.scripts.len(), 1);
        assert_eq!(manifest.scripts[0].filename, "contacts.rhai");

        // Must contain the .rhai file
        let mut rhai_file = archive.by_name("scripts/contacts.rhai").unwrap();
        let mut source = String::new();
        std::io::Read::read_to_string(&mut rhai_file, &mut source).unwrap();
        assert!(source.contains("@name: Contacts"));
    }
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_export_workspace`
Expected: FAIL — `export_workspace` not found

**Step 3: Implement `export_workspace`**

Add to `export.rs` (above the `tests` module), with the necessary imports at the top of the file:

```rust
use std::io::{Read, Seek, Write};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::core::workspace::Workspace;

/// The current Krillnotes app version, read from Cargo.toml at compile time.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exports the workspace contents as a zip archive.
///
/// The archive contains:
/// - `notes.json` — all notes with format version and app version
/// - `scripts/scripts.json` — script metadata (filename, load_order, enabled)
/// - `scripts/<name>.rhai` — each user script's source code
///
/// The `operations` table and `workspace_meta` are excluded.
pub fn export_workspace<W: Write + Seek>(workspace: &Workspace, writer: W) -> Result<(), ExportError> {
    let notes = workspace.list_all_notes().map_err(|e| ExportError::Database(e.to_string()))?;
    let scripts = workspace.list_user_scripts().map_err(|e| ExportError::Database(e.to_string()))?;

    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Write notes.json
    let export_notes = ExportNotes {
        version: 1,
        app_version: APP_VERSION.to_string(),
        notes,
    };
    zip.start_file("notes.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &export_notes)?;

    // Build script manifest and write .rhai files
    let mut manifest_entries = Vec::new();
    let mut used_filenames = std::collections::HashSet::new();

    for script in &scripts {
        let mut base = slugify_script_name(&script.name);
        let mut filename = format!("{base}.rhai");

        // Deduplicate filenames
        let mut counter = 1u32;
        while used_filenames.contains(&filename) {
            counter += 1;
            filename = format!("{base}-{counter}.rhai");
        }
        used_filenames.insert(filename.clone());

        manifest_entries.push(ScriptManifestEntry {
            filename: filename.clone(),
            load_order: script.load_order,
            enabled: script.enabled,
        });

        zip.start_file(format!("scripts/{filename}"), options)?;
        zip.write_all(script.source_code.as_bytes())?;
    }

    // Write scripts/scripts.json
    let manifest = ScriptManifest { scripts: manifest_entries };
    zip.start_file("scripts/scripts.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;

    zip.finish()?;
    Ok(())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_export_workspace`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/export.rs
git commit -m "feat: implement export_workspace"
```

---

## Task 5: Implement `peek_import` and `import_workspace`

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`
- Modify: `krillnotes-core/src/core/workspace.rs` (add `import_notes_bulk` and `import_script_bulk` helpers)

**Step 1: Write the failing tests**

Add to the `tests` module in `export.rs`:

```rust
    #[test]
    fn test_peek_import_reads_metadata() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();
        let script_source = "// @name: Tasks\n// @description: Task list\nschema(\"Task\", #{});";
        ws.create_user_script(script_source).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf)).unwrap();

        let result = peek_import(Cursor::new(&buf)).unwrap();
        assert_eq!(result.app_version, APP_VERSION);
        assert!(result.note_count >= 1); // at least root note
        assert_eq!(result.script_count, 1);
    }

    /// NOTE: This test works because export currently reads through the Workspace API
    /// and import writes through bulk SQL inserts. If export ever reads directly from
    /// SQLite (e.g. for streaming large workspaces), this in-memory round-trip approach
    /// will need to be revised to use actual database files for both sides.
    #[test]
    fn test_round_trip_export_import() {
        // Create source workspace with nested notes and a script
        let src_temp = NamedTempFile::new().unwrap();
        let mut src_ws = Workspace::create(src_temp.path()).unwrap();

        let root_notes = src_ws.list_all_notes().unwrap();
        let root_id = &root_notes[0].id;

        // Add a child note under root
        src_ws
            .create_note_with_type("TextNote", Some(root_id), crate::AddPosition::AsChild)
            .unwrap();

        let script_source = "// @name: Contacts\n// @description: Contact cards\nschema(\"Contact\", #{});";
        src_ws.create_user_script(script_source).unwrap();

        // Export
        let mut buf = Vec::new();
        export_workspace(&src_ws, Cursor::new(&mut buf)).unwrap();

        // Import into a new workspace
        let dst_temp = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), dst_temp.path()).unwrap();

        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 2); // root + child
        assert_eq!(result.script_count, 1);

        // Open the imported workspace and verify data
        let dst_ws = Workspace::open(dst_temp.path()).unwrap();
        let imported_notes = dst_ws.list_all_notes().unwrap();
        assert_eq!(imported_notes.len(), 2);

        // Verify parent-child relationship preserved
        let child = imported_notes.iter().find(|n| n.parent_id.is_some()).unwrap();
        assert_eq!(child.parent_id.as_deref(), Some(root_id.as_str()));

        let imported_scripts = dst_ws.list_user_scripts().unwrap();
        assert_eq!(imported_scripts.len(), 1);
        assert!(imported_scripts[0].source_code.contains("@name: Contacts"));
    }

    #[test]
    fn test_import_invalid_zip() {
        let temp = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(b"not a zip"), temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_import_missing_notes_json() {
        // Create a valid zip but without notes.json
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default();
            zip.start_file("random.txt", options).unwrap();
            zip.write_all(b"hello").unwrap();
            zip.finish().unwrap();
        }

        let temp = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp.path());
        assert!(matches!(result, Err(ExportError::InvalidFormat(_))));
    }
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_peek_import test_round_trip test_import_invalid test_import_missing`
Expected: FAIL — functions not found

**Step 3: Add bulk import helpers to `Workspace`**

In `krillnotes-core/src/core/workspace.rs`, add these methods to the `impl Workspace` block (at the end, before the closing `}`):

```rust
    /// Bulk-inserts notes from an export archive, bypassing operation logging.
    ///
    /// Used only during workspace import. Notes are inserted exactly as-is,
    /// preserving their original IDs, parent relationships, and positions.
    pub(crate) fn import_notes_bulk(&mut self, notes: &[Note]) -> Result<()> {
        let tx = self.storage.connection_mut().transaction()?;
        for note in notes {
            tx.execute(
                "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    note.id,
                    note.title,
                    note.node_type,
                    note.parent_id,
                    note.position,
                    note.created_at,
                    note.modified_at,
                    note.created_by,
                    note.modified_by,
                    serde_json::to_string(&note.fields)?,
                    note.is_expanded,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Bulk-inserts user scripts from an export archive, bypassing operation logging.
    ///
    /// Used only during workspace import. Scripts are inserted with new UUIDs
    /// but preserve load_order, enabled state, and source code.
    pub(crate) fn import_scripts_bulk(
        &mut self,
        scripts: &[(String, i32, bool)], // (source_code, load_order, enabled)
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let tx = self.storage.connection_mut().transaction()?;
        for (source_code, load_order, enabled) in scripts {
            let id = Uuid::new_v4().to_string();
            let fm = user_script::parse_front_matter(source_code);
            tx.execute(
                "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![id, fm.name, fm.description, source_code, load_order, enabled, now, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
```

**Step 4: Implement `peek_import` and `import_workspace`**

Add to `export.rs` (after `export_workspace`), updating imports at the top:

```rust
use std::path::Path;
use zip::ZipArchive;

/// Reads export metadata without creating a workspace.
///
/// Use this to check the app version before committing to a full import.
pub fn peek_import<R: Read + Seek>(reader: R) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Read notes.json
    let notes_file = archive.by_name("notes.json").map_err(|_| {
        ExportError::InvalidFormat("Archive missing notes.json".to_string())
    })?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_file)?;

    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    // Count scripts from manifest (if present)
    let script_count = match archive.by_name("scripts/scripts.json") {
        Ok(manifest_file) => {
            let manifest: ScriptManifest = serde_json::from_reader(manifest_file)?;
            manifest.scripts.len()
        }
        Err(_) => 0,
    };

    Ok(ImportResult {
        app_version: export_notes.app_version,
        note_count: export_notes.notes.len(),
        script_count,
    })
}

/// Imports an export archive into a new workspace database at `db_path`.
///
/// Creates the database, inserts all notes preserving their IDs and tree
/// structure, and inserts all user scripts. Does **not** create a root note
/// (the exported notes already contain one).
///
/// Returns metadata about the imported content so the caller can check the
/// app version and display confirmation UI.
pub fn import_workspace<R: Read + Seek>(reader: R, db_path: &Path) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Read and validate notes.json
    let notes_file = archive.by_name("notes.json").map_err(|_| {
        ExportError::InvalidFormat("Archive missing notes.json".to_string())
    })?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_file)?;

    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    // Read script manifest and source files
    let mut script_data: Vec<(String, i32, bool)> = Vec::new();
    if let Ok(manifest_file) = archive.by_name("scripts/scripts.json") {
        let manifest: ScriptManifest = serde_json::from_reader(manifest_file)?;

        for entry in &manifest.scripts {
            let path = format!("scripts/{}", entry.filename);
            let mut script_file = archive.by_name(&path).map_err(|_| {
                ExportError::InvalidFormat(format!("Script file missing from archive: {path}"))
            })?;
            let mut source = String::new();
            script_file.read_to_string(&mut source)?;
            script_data.push((source, entry.load_order, entry.enabled));
        }
    }

    let note_count = export_notes.notes.len();
    let script_count = script_data.len();
    let app_version = export_notes.app_version.clone();

    // Create fresh workspace database (without the default root note)
    let mut storage = crate::Storage::create(db_path).map_err(|e| ExportError::Database(e.to_string()))?;
    let device_id = crate::get_device_id().map_err(|e| ExportError::Database(e.to_string()))?;

    // Insert workspace metadata
    storage.connection().execute(
        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
        ["device_id", &device_id],
    ).map_err(|e| ExportError::Database(e.to_string()))?;
    storage.connection().execute(
        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
        ["current_user_id", "0"],
    ).map_err(|e| ExportError::Database(e.to_string()))?;

    // Bulk-insert notes
    {
        let tx = storage.connection_mut().transaction().map_err(|e| ExportError::Database(e.to_string()))?;
        for note in &export_notes.notes {
            tx.execute(
                "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    note.id,
                    note.title,
                    note.node_type,
                    note.parent_id,
                    note.position,
                    note.created_at,
                    note.modified_at,
                    note.created_by,
                    note.modified_by,
                    serde_json::to_string(&note.fields).map_err(|e| ExportError::Json(e))?,
                    note.is_expanded,
                ],
            ).map_err(|e| ExportError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| ExportError::Database(e.to_string()))?;
    }

    // Bulk-insert scripts
    if !script_data.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let tx = storage.connection_mut().transaction().map_err(|e| ExportError::Database(e.to_string()))?;
        for (source_code, load_order, enabled) in &script_data {
            let id = uuid::Uuid::new_v4().to_string();
            let fm = crate::core::user_script::parse_front_matter(source_code);
            tx.execute(
                "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![id, fm.name, fm.description, source_code, load_order, enabled, now, now],
            ).map_err(|e| ExportError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| ExportError::Database(e.to_string()))?;
    }

    Ok(ImportResult {
        app_version,
        note_count,
        script_count,
    })
}
```

Note: This approach does the SQL directly in the export module rather than going through `Workspace` methods, because `import_workspace` needs to create a fresh DB without the auto-generated root note that `Workspace::create` adds. The `import_notes_bulk` and `import_scripts_bulk` helpers added in Step 3 are no longer needed — remove them before committing, or keep them if you prefer the indirection. The direct approach is simpler.

**Step 5: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core export`
Expected: all export tests pass

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/export.rs
git commit -m "feat: implement peek_import and import_workspace"
```

---

## Task 6: Add Export/Import menu items

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs:21-22` (add items between Open and separator)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:678-687` (add menu messages)

**Step 1: Add menu items**

In `krillnotes-desktop/src-tauri/src/menu.rs`, between the `file_open` item (line 21) and the separator (line 22), add:

```rust
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("file_export", "Export Workspace...")
                        .build(app)?,
                    &MenuItemBuilder::with_id("file_import", "Import Workspace...")
                        .build(app)?,
```

**Step 2: Add menu message mappings**

In `krillnotes-desktop/src-tauri/src/lib.rs`, add two entries to `MENU_MESSAGES` (after the `file_open` entry on line 680):

```rust
    ("file_export", "File > Export Workspace clicked"),
    ("file_import", "File > Import Workspace clicked"),
```

**Step 3: Verify it compiles**

Run: `cargo build -p krillnotes-desktop`
Expected: compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add Export/Import menu items to File menu"
```

---

## Task 7: Add Tauri commands for export, peek, and execute import

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (add three new commands and register them)

**Step 1: Add the `export_workspace` command**

Add after the existing `purge_operations` command (around line 675), before the `MENU_MESSAGES` const:

```rust
#[tauri::command]
fn export_workspace(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::new(file);
    krillnotes_core::export::export_workspace(workspace, writer).map_err(|e| e.to_string())
}
```

**Step 2: Add the `peek_import` command**

```rust
#[tauri::command]
fn peek_import(
    zip_path: String,
) -> std::result::Result<krillnotes_core::ImportResult, String> {
    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    krillnotes_core::export::peek_import(reader).map_err(|e| e.to_string())
}
```

**Step 3: Add the `execute_import` command**

This command creates the workspace and opens it in a new window, following the same pattern as `open_workspace`:

```rust
#[tauri::command]
async fn execute_import(
    app: AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    db_path: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let db_path_buf = std::path::PathBuf::from(&db_path);

    // Import from zip into new database
    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    krillnotes_core::export::import_workspace(reader, &db_path_buf)
        .map_err(|e| e.to_string())?;

    // Open the imported workspace in a new window (reuse open_workspace_at_path logic)
    let workspace = Workspace::open(&db_path_buf).map_err(|e| e.to_string())?;
    let label = format!("workspace_{}", uuid::Uuid::new_v4().to_string().replace('-', "_"));
    let title = db_path_buf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Workspace")
        .to_string();

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(&title)
        .inner_size(1200.0, 800.0)
        .menu(menu::build_menu(app.app_handle()).map_err(|e| e.to_string())?)
        .build()
        .map_err(|e| e.to_string())?;

    let info = WorkspaceInfo {
        name: title,
        note_count: workspace.list_all_notes().map(|n| n.len()).unwrap_or(0),
        device_id: String::new(),
    };

    state.workspaces.lock().expect("Mutex poisoned").insert(label.clone(), workspace);
    state.workspace_paths.lock().expect("Mutex poisoned").insert(label, db_path_buf);

    Ok(info)
}
```

**Step 4: Register the commands**

In the `invoke_handler` block (line 732), add the three new commands:

```rust
            export_workspace,
            peek_import,
            execute_import,
```

**Step 5: Verify it compiles**

Run: `cargo build -p krillnotes-desktop`
Expected: compiles successfully

**Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add Tauri commands for export/import"
```

---

## Task 8: Wire up frontend export handler

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx` (add export handler to `createMenuHandlers`)

**Step 1: Add the export handler**

In `App.tsx`, add a new entry to the `createMenuHandlers` return object (after the `'File > Open Workspace clicked'` handler, around line 56):

```typescript
  'File > Export Workspace clicked': async () => {
    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        defaultPath: 'workspace.krillnotes.zip',
        title: 'Export Workspace'
      });

      if (!path) return;

      await invoke('export_workspace', { path });
      setStatus('Workspace exported successfully');
    } catch (error) {
      setStatus(`Export failed: ${error}`, true);
    }
  },
```

**Step 2: Verify it compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat: wire up export workspace menu handler"
```

---

## Task 9: Wire up frontend import handler with version check

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx` (add import handler to `createMenuHandlers`)

**Step 1: Add the import handler**

Add another entry to `createMenuHandlers`:

```typescript
  'File > Import Workspace clicked': async () => {
    try {
      // Step 1: Pick the zip file
      const zipPath = await open({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        multiple: false,
        title: 'Import Workspace'
      });

      if (!zipPath || Array.isArray(zipPath)) return;

      // Step 2: Peek at the metadata to check version
      const result = await invoke<{ appVersion: string; noteCount: number; scriptCount: number }>(
        'peek_import', { zipPath }
      );

      // Step 3: Check app version — warn if export is from a newer version
      const currentVersion = await invoke<string>('get_app_version');
      if (result.appVersion > currentVersion) {
        const { confirm } = await import('@tauri-apps/plugin-dialog');
        const proceed = await confirm(
          `This export was created with Krillnotes v${result.appVersion}, but you are running v${currentVersion}. Some data may not import correctly.\n\nImport anyway?`,
          { title: 'Version Mismatch', kind: 'warning' }
        );
        if (!proceed) return;
      }

      // Step 4: Pick where to save the new .db file
      const dbPath = await save({
        filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
        defaultPath: 'imported-workspace.db',
        title: 'Save Imported Workspace As'
      });

      if (!dbPath) return;

      // Step 5: Execute the import
      await invoke('execute_import', { zipPath, dbPath });
      setStatus(`Imported ${result.noteCount} notes and ${result.scriptCount} scripts`);
    } catch (error) {
      setStatus(`Import failed: ${error}`, true);
    }
  },
```

**Step 2: Add the `get_app_version` Tauri command**

In `krillnotes-desktop/src-tauri/src/lib.rs`, add a simple command:

```rust
#[tauri::command]
fn get_app_version() -> String {
    krillnotes_core::export::APP_VERSION.to_string()
}
```

Register it in `invoke_handler`:

```rust
            get_app_version,
```

**Step 3: Verify it compiles**

Run: `cargo build -p krillnotes-desktop && cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/App.tsx krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: wire up import workspace with version check"
```

---

## Task 10: Manual integration test

**Step 1: Run the full app**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run tauri dev`

**Step 2: Test export**

1. Open or create a workspace
2. Add a few notes (different types if you have scripts loaded)
3. File > Export Workspace...
4. Save as `test-export.zip`
5. Verify the status message shows success
6. Open the zip and inspect: `notes.json` should have notes, `scripts/` should have `.rhai` files if scripts exist

**Step 3: Test import**

1. File > Import Workspace...
2. Select the `test-export.zip` you just created
3. Choose where to save the new `.db` file
4. Verify a new window opens with the imported workspace
5. Verify all notes and scripts are present

**Step 4: Test version mismatch (optional)**

1. Manually edit a zip's `notes.json` to set `"appVersion": "99.0.0"`
2. Try to import it
3. Verify the warning dialog appears

**Step 5: Commit any fixes**

If any bugs are found, fix them and commit.

---

## Task 11: Final cleanup and commit

**Step 1: Run all core tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

**Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings

**Step 3: Verify frontend build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: builds cleanly

**Step 4: Final commit if needed**

```bash
git add -A
git commit -m "feat: workspace export/import as zip archives"
```
