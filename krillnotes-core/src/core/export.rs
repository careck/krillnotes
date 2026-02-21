//! Workspace export and import as `.zip` archives.

use std::collections::HashSet;
use std::io::{Read, Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::core::note::Note;
use crate::core::user_script;
use crate::core::workspace::Workspace;
use crate::get_device_id;
use crate::Storage;

/// The current Krillnotes app version, read from Cargo.toml at compile time.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Top-level JSON structure in `notes.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportNotes {
    pub version: u32,
    pub app_version: String,
    pub notes: Vec<Note>,
}

/// One entry in `scripts/scripts.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptManifestEntry {
    pub filename: String,
    pub load_order: i32,
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
    pub app_version: String,
    pub note_count: usize,
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

/// Converts a script name into a safe filename stem.
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

/// Exports the workspace contents as a zip archive.
///
/// The archive contains:
/// - `notes.json` -- all notes with format version and app version
/// - `scripts/scripts.json` -- script metadata (filename, load_order, enabled)
/// - `scripts/<name>.rhai` -- each user script's source code
///
/// The `operations` table and `workspace_meta` are excluded.
pub fn export_workspace<W: Write + Seek>(
    workspace: &Workspace,
    writer: W,
) -> Result<(), ExportError> {
    let notes = workspace
        .list_all_notes()
        .map_err(|e| ExportError::Database(e.to_string()))?;
    let scripts = workspace
        .list_user_scripts()
        .map_err(|e| ExportError::Database(e.to_string()))?;

    let mut zip = ZipWriter::new(writer);
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

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
    let mut used_filenames: HashSet<String> = HashSet::new();

    for script in &scripts {
        let base = slugify_script_name(&script.name);
        let mut filename = format!("{base}.rhai");

        // Deduplicate filenames with numeric suffix
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
    let manifest = ScriptManifest {
        scripts: manifest_entries,
    };
    zip.start_file("scripts/scripts.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;

    zip.finish()?;
    Ok(())
}

/// Reads the metadata from an export archive without creating a workspace.
///
/// Opens the zip, parses `notes.json` to extract the note count and app version,
/// and optionally reads `scripts/scripts.json` for the script count.
///
/// # Errors
///
/// Returns [`ExportError::InvalidFormat`] if the format version is not `1` or
/// `notes.json` is missing. Returns other `ExportError` variants for I/O, zip,
/// or JSON failures.
pub fn peek_import<R: Read + Seek>(reader: R) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Read and parse notes.json
    let notes_file = archive.by_name("notes.json").map_err(|_| {
        ExportError::InvalidFormat("Missing notes.json in archive".to_string())
    })?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_file)?;

    // Validate format version
    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    // Count scripts from manifest (0 if missing)
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

/// Imports an export archive into a new workspace database.
///
/// Creates a fresh database at `db_path` using [`Storage::create`], then bulk-inserts
/// all notes (preserving original IDs, parent relationships, and positions) and all
/// scripts (with new UUIDs, preserving source code, load order, and enabled state).
///
/// Does **not** create a root note — the exported notes already contain one.
///
/// # Errors
///
/// Returns [`ExportError::InvalidFormat`] if `notes.json` is missing or the format
/// version is not `1`. Returns [`ExportError::Database`] for any storage or SQL
/// failure. Returns other `ExportError` variants for I/O, zip, or JSON errors.
pub fn import_workspace<R: Read + Seek>(reader: R, db_path: &Path) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Read and parse notes.json
    let notes_file = archive.by_name("notes.json").map_err(|_| {
        ExportError::InvalidFormat("Missing notes.json in archive".to_string())
    })?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_file)?;

    // Validate format version
    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    // Read script manifest and source files
    let manifest = match archive.by_name("scripts/scripts.json") {
        Ok(manifest_file) => {
            let m: ScriptManifest = serde_json::from_reader(manifest_file)?;
            Some(m)
        }
        Err(_) => None,
    };

    // Read each .rhai script source from the archive
    let mut script_sources: Vec<(String, i32, bool)> = Vec::new(); // (source_code, load_order, enabled)
    if let Some(ref manifest) = manifest {
        for entry in &manifest.scripts {
            let path = format!("scripts/{}", entry.filename);
            let mut rhai_file = archive.by_name(&path).map_err(|e| {
                ExportError::InvalidFormat(format!(
                    "Script file '{}' referenced in manifest but missing from archive: {}",
                    path, e
                ))
            })?;
            let mut source = String::new();
            rhai_file.read_to_string(&mut source)?;
            script_sources.push((source, entry.load_order, entry.enabled));
        }
    }

    // Create the database
    let mut storage = Storage::create(db_path).map_err(|e| ExportError::Database(e.to_string()))?;

    // Insert workspace metadata
    let device_id = get_device_id().map_err(|e| ExportError::Database(e.to_string()))?;
    storage
        .connection()
        .execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["device_id", &device_id],
        )
        .map_err(|e| ExportError::Database(e.to_string()))?;
    storage
        .connection()
        .execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["current_user_id", "0"],
        )
        .map_err(|e| ExportError::Database(e.to_string()))?;

    // Bulk-insert notes in a transaction
    {
        let tx = storage
            .connection_mut()
            .transaction()
            .map_err(|e| ExportError::Database(e.to_string()))?;
        for note in &export_notes.notes {
            let fields_json = serde_json::to_string(&note.fields)?;
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
                    fields_json,
                    note.is_expanded,
                ],
            )
            .map_err(|e| ExportError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| ExportError::Database(e.to_string()))?;
    }

    // Bulk-insert scripts in a transaction
    let script_count = script_sources.len();
    if !script_sources.is_empty() {
        let tx = storage
            .connection_mut()
            .transaction()
            .map_err(|e| ExportError::Database(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        for (source_code, load_order, enabled) in &script_sources {
            let id = uuid::Uuid::new_v4().to_string();
            let fm = user_script::parse_front_matter(source_code);
            tx.execute(
                "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![id, fm.name, fm.description, source_code, load_order, enabled, now, now],
            )
            .map_err(|e| ExportError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| ExportError::Database(e.to_string()))?;
    }

    Ok(ImportResult {
        app_version: export_notes.app_version,
        note_count: export_notes.notes.len(),
        script_count,
    })
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

    #[test]
    fn test_slugify_script_name() {
        assert_eq!(slugify_script_name("Contacts"), "contacts");
        assert_eq!(slugify_script_name("My Tasks"), "my-tasks");
        assert_eq!(slugify_script_name("Hello World!"), "hello-world");
        assert_eq!(slugify_script_name("  Spaced  Out  "), "spaced-out");
        assert_eq!(slugify_script_name(""), "script");
        assert_eq!(slugify_script_name("---"), "script");
    }

    use crate::{AddPosition, Workspace};
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    #[test]
    fn test_export_workspace_creates_valid_zip() {
        // Create a workspace with a note and a script
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

        // Add a user script
        let script_source =
            "// @name: Contacts\n// @description: Contact cards\nschema(\"Contact\", #{});";
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

    #[test]
    fn test_peek_import_reads_metadata() {
        // Create a workspace with a script
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();

        let script_source =
            "// @name: Contacts\n// @description: Contact cards\nschema(\"Contact\", #{});";
        ws.create_user_script(script_source).unwrap();

        // Export to a buffer
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf)).unwrap();

        // Peek at the export
        let result = peek_import(Cursor::new(&buf)).unwrap();
        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 1); // root note
        assert_eq!(result.script_count, 1);
    }

    /// NOTE: This test works because export currently reads through the Workspace API
    /// and import writes through bulk SQL inserts. If export ever reads directly from
    /// SQLite (e.g. for streaming large workspaces), this in-memory round-trip approach
    /// will need to be revised to use actual database files for both sides.
    #[test]
    fn test_round_trip_export_import() {
        // Create a workspace with nested notes and a script
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path()).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "Root Note".to_string()).unwrap();

        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&child_id, "Child Note".to_string()).unwrap();

        let grandchild_id = ws
            .create_note(&child_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&grandchild_id, "Grandchild Note".to_string())
            .unwrap();

        let script_source =
            "// @name: Contacts\n// @description: Contact cards\nschema(\"Contact\", #{});";
        ws.create_user_script(script_source).unwrap();

        // Export
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf)).unwrap();

        // Import into a new workspace file
        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path()).unwrap();

        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 3);
        assert_eq!(result.script_count, 1);

        // Open the imported workspace and verify contents
        let imported_ws = Workspace::open(temp_dst.path()).unwrap();

        let notes = imported_ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 3);

        // Verify note titles
        let titles: Vec<&str> = notes.iter().map(|n| n.title.as_str()).collect();
        assert!(titles.contains(&"Root Note"));
        assert!(titles.contains(&"Child Note"));
        assert!(titles.contains(&"Grandchild Note"));

        // Verify parent-child relationships are preserved
        let root_note = notes.iter().find(|n| n.title == "Root Note").unwrap();
        let child_note = notes.iter().find(|n| n.title == "Child Note").unwrap();
        let grandchild_note = notes.iter().find(|n| n.title == "Grandchild Note").unwrap();

        assert_eq!(root_note.parent_id, None);
        assert_eq!(child_note.parent_id, Some(root_note.id.clone()));
        assert_eq!(grandchild_note.parent_id, Some(child_note.id.clone()));

        // Verify scripts
        let scripts = imported_ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "Contacts");
        assert_eq!(scripts[0].description, "Contact cards");
        assert!(scripts[0].source_code.contains("@name: Contacts"));
    }

    #[test]
    fn test_import_invalid_zip() {
        let garbage = b"this is not a zip file at all";
        let result = import_workspace(Cursor::new(garbage), Path::new("/tmp/invalid.db"));
        assert!(result.is_err());
    }

    #[test]
    fn test_import_missing_notes_json() {
        // Create a valid zip that has no notes.json
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("readme.txt", options).unwrap();
            zip.write_all(b"no notes here").unwrap();
            zip.finish().unwrap();
        }

        let result = import_workspace(Cursor::new(&buf), Path::new("/tmp/missing_notes.db"));
        assert!(matches!(result, Err(ExportError::InvalidFormat(_))));
    }
}
