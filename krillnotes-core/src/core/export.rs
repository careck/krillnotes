//! Workspace export and import as `.zip` archives.

use std::collections::HashSet;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::AesMode;
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

/// Top-level JSON structure in `workspace.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceJson {
    pub version: u32,
    /// Complete sorted list of distinct tags across the workspace.
    pub tags: Vec<String>,
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

    #[error("Archive is password-protected; provide a password to decrypt")]
    EncryptedArchive,

    #[error("Incorrect password")]
    InvalidPassword,
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


/// Opens a named entry and reads all its bytes, decrypting with `password` if provided.
/// Returns `ExportError::InvalidPassword` if the password is wrong (detected via MAC verification).
/// Returns `ExportError::InvalidFormat` if the entry doesn't exist.
fn read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    password: Option<&str>,
) -> Result<Cursor<Vec<u8>>, ExportError> {
    let mut content = Vec::new();
    if let Some(pwd) = password {
        let mut file = archive
            .by_name_decrypt(name, pwd.as_bytes())
            .map_err(|e| match e {
                zip::result::ZipError::FileNotFound => {
                    ExportError::InvalidFormat(format!("Missing '{name}' in archive"))
                }
                zip::result::ZipError::InvalidPassword => ExportError::InvalidPassword,
                other => ExportError::Zip(other),
            })?;
        file.read_to_end(&mut content)
            .map_err(|_| ExportError::InvalidPassword)?;
    } else {
        let mut file = archive
            .by_name(name)
            .map_err(|_| ExportError::InvalidFormat(format!("Missing '{name}' in archive")))?;
        file.read_to_end(&mut content)?;
    }
    Ok(Cursor::new(content))
}

/// Like `read_entry` but returns `None` instead of an error when the entry is absent or unreadable.
fn try_read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    password: Option<&str>,
) -> Option<Cursor<Vec<u8>>> {
    let mut content = Vec::new();
    if let Some(pwd) = password {
        let mut file = archive.by_name_decrypt(name, pwd.as_bytes()).ok()?;
        file.read_to_end(&mut content).ok()?;
    } else {
        let mut file = archive.by_name(name).ok()?;
        file.read_to_end(&mut content).ok()?;
    }
    Some(Cursor::new(content))
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
    password: Option<&str>,
) -> Result<(), ExportError> {
    let notes = workspace
        .list_all_notes()
        .map_err(|e| ExportError::Database(e.to_string()))?;
    let scripts = workspace
        .list_user_scripts()
        .map_err(|e| ExportError::Database(e.to_string()))?;

    let mut zip = ZipWriter::new(writer);
    let options = match password {
        Some(pwd) => SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .with_aes_encryption(AesMode::Aes256, pwd),
        None => SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    };

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

    // Write workspace.json (global tag list)
    let all_tags = workspace
        .get_all_tags()
        .map_err(|e| ExportError::Database(e.to_string()))?;
    let workspace_json = WorkspaceJson {
        version: 1,
        tags: all_tags,
    };
    zip.start_file("workspace.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &workspace_json)?;

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
/// Returns [`ExportError::EncryptedArchive`] if the archive is encrypted and no
/// password is provided. Returns [`ExportError::InvalidPassword`] if the password
/// is wrong. Returns [`ExportError::InvalidFormat`] if the format version is not
/// `1` or `notes.json` is missing. Returns other `ExportError` variants for I/O,
/// zip, or JSON failures.
pub fn peek_import<R: Read + Seek>(reader: R, password: Option<&str>) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Detect encryption before trying to read data.
    // by_index_raw reads metadata without decrypting, so .encrypted() is safe to call
    // without a password.
    {
        let index = archive.index_for_name("notes.json").ok_or_else(|| {
            ExportError::InvalidFormat("Missing notes.json in archive".to_string())
        })?;
        let check = archive.by_index_raw(index).map_err(ExportError::Zip)?;
        if check.encrypted() && password.is_none() {
            return Err(ExportError::EncryptedArchive);
        }
    }

    let notes_cursor = read_entry(&mut archive, "notes.json", password)?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_cursor)?;

    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    let script_count = match try_read_entry(&mut archive, "scripts/scripts.json", password) {
        Some(manifest_cursor) => {
            let manifest: ScriptManifest = serde_json::from_reader(manifest_cursor)?;
            manifest.scripts.len()
        }
        None => 0,
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
pub fn import_workspace<R: Read + Seek>(
    reader: R,
    db_path: &Path,
    zip_password: Option<&str>,
    workspace_password: &str,
) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Detect encryption (same pattern as peek_import)
    {
        let index = archive.index_for_name("notes.json").ok_or_else(|| {
            ExportError::InvalidFormat("Missing notes.json in archive".to_string())
        })?;
        let check = archive.by_index_raw(index).map_err(ExportError::Zip)?;
        if check.encrypted() && zip_password.is_none() {
            return Err(ExportError::EncryptedArchive);
        }
    }

    let notes_cursor = read_entry(&mut archive, "notes.json", zip_password)?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_cursor)?;

    // Validate format version
    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    // Read script manifest and source files
    let manifest = match try_read_entry(&mut archive, "scripts/scripts.json", zip_password) {
        Some(manifest_cursor) => {
            let m: ScriptManifest = serde_json::from_reader(manifest_cursor)?;
            Some(m)
        }
        None => None,
    };

    // Read each .rhai script source from the archive
    let mut script_sources: Vec<(String, i32, bool)> = Vec::new(); // (source_code, load_order, enabled)
    if let Some(ref manifest) = manifest {
        for entry in &manifest.scripts {
            let path = format!("scripts/{}", entry.filename);
            let mut rhai_cursor = read_entry(&mut archive, &path, zip_password).map_err(|e| {
                ExportError::InvalidFormat(format!(
                    "Script file '{}' referenced in manifest but missing from archive: {}",
                    path, e
                ))
            })?;
            let mut source = String::new();
            rhai_cursor.read_to_string(&mut source)?;
            script_sources.push((source, entry.load_order, entry.enabled));
        }
    }

    // Create the database
    let mut storage = Storage::create(db_path, workspace_password)
        .map_err(|e| ExportError::Database(e.to_string()))?;

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

    // Bulk-insert notes in a transaction.
    // Defer foreign-key checks so child notes can be inserted before their parents.
    {
        storage
            .connection()
            .execute_batch("PRAGMA defer_foreign_keys = ON;")
            .map_err(|e| ExportError::Database(e.to_string()))?;
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

            // Restore tags for this note.
            for tag in &note.tags {
                tx.execute(
                    "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?, ?)",
                    rusqlite::params![note.id, tag],
                )
                .map_err(|e| ExportError::Database(e.to_string()))?;
            }
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
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        // Add a user script (unique name to avoid collision with starters)
        let script_source =
            "// @name: Custom Widget\n// @description: Widget cards\nschema(\"Widget\", #{ fields: [] });";
        ws.create_user_script(script_source).unwrap();

        // Export to a buffer
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

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
        // Starter scripts + the user-created Widget script
        assert!(manifest.scripts.len() >= 2, "Should have starter scripts plus user script");
        let widget_entry = manifest.scripts.iter().find(|s| s.filename == "custom-widget.rhai");
        assert!(widget_entry.is_some(), "Should contain custom-widget.rhai in manifest");

        // Must contain the .rhai file
        let mut rhai_file = archive.by_name("scripts/custom-widget.rhai").unwrap();
        let mut source = String::new();
        std::io::Read::read_to_string(&mut rhai_file, &mut source).unwrap();
        assert!(source.contains("@name: Custom Widget"));
    }

    #[test]
    fn test_peek_import_reads_metadata() {
        // Create a workspace with a script
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();

        let script_source =
            "// @name: Custom Widget\n// @description: Widget cards\nschema(\"Widget\", #{ fields: [] });";
        ws.create_user_script(script_source).unwrap();

        // Export to a buffer
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        // Peek at the export
        let result = peek_import(Cursor::new(&buf), None).unwrap();
        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 1); // root note
        // Starter scripts + the user-created Widget script
        assert!(result.script_count >= 2, "Should have starters + user script, got {}", result.script_count);
    }

    /// NOTE: This test works because export currently reads through the Workspace API
    /// and import writes through bulk SQL inserts. If export ever reads directly from
    /// SQLite (e.g. for streaming large workspaces), this in-memory round-trip approach
    /// will need to be revised to use actual database files for both sides.
    #[test]
    fn test_round_trip_export_import() {
        // Create a workspace with nested notes and a script
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "").unwrap();

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
            "// @name: Custom Widget\n// @description: Widget cards\nschema(\"Widget\", #{ fields: [] });";
        ws.create_user_script(script_source).unwrap();

        // Export
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        // Import into a new workspace file
        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path(), None, "").unwrap();

        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 3);
        // Starter scripts + the user-created Widget script
        assert!(result.script_count >= 2, "Should have starters + user script, got {}", result.script_count);

        // Open the imported workspace and verify contents
        let imported_ws = Workspace::open(temp_dst.path(), "").unwrap();

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
        assert!(scripts.len() >= 2, "Should have starters + user script");
        let widget = scripts.iter().find(|s| s.name == "Custom Widget").unwrap();
        assert_eq!(widget.description, "Widget cards");
        assert!(widget.source_code.contains("@name: Custom Widget"));
    }

    #[test]
    fn test_export_includes_workspace_json() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let mut archive = zip::ZipArchive::new(Cursor::new(&buf)).unwrap();
        let ws_file = archive.by_name("workspace.json").unwrap();
        let ws_json: WorkspaceJson = serde_json::from_reader(ws_file).unwrap();
        assert_eq!(ws_json.version, 1);
        assert_eq!(ws_json.tags, vec!["design", "rust"]);
    }

    #[test]
    fn test_round_trip_preserves_tags() {
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["rust".into()]).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let temp_dst = NamedTempFile::new().unwrap();
        import_workspace(Cursor::new(&buf), temp_dst.path(), None, "").unwrap();

        let imported = Workspace::open(temp_dst.path(), "").unwrap();
        let tags = imported.get_all_tags().unwrap();
        assert_eq!(tags, vec!["rust"]);

        // Tags are also on the note itself
        let notes = imported.list_all_notes().unwrap();
        let root_imported = notes.iter().find(|n| n.parent_id.is_none()).unwrap();
        assert_eq!(root_imported.tags, vec!["rust"]);
    }

    #[test]
    fn test_import_invalid_zip() {
        let garbage = b"this is not a zip file at all";
        let result = import_workspace(Cursor::new(garbage), Path::new("/tmp/invalid.db"), None, "");
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

        let result = import_workspace(Cursor::new(&buf), Path::new("/tmp/missing_notes.db"), None, "");
        assert!(matches!(result, Err(ExportError::InvalidFormat(_))));
    }

    #[test]
    fn test_export_with_password_creates_encrypted_zip() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("hunter2")).unwrap();

        // notes.json should be marked as encrypted.
        // Use by_index_raw to read metadata without decrypting.
        let reader = Cursor::new(&buf);
        let mut archive = ZipArchive::new(reader).unwrap();
        let index = archive.index_for_name("notes.json").unwrap();
        let notes_file = archive.by_index_raw(index).unwrap();
        assert!(notes_file.encrypted(), "notes.json must be encrypted when password is provided");
    }

    #[test]
    fn test_export_without_password_creates_plain_zip() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let reader = Cursor::new(&buf);
        let mut archive = ZipArchive::new(reader).unwrap();
        let notes_file = archive.by_name("notes.json").unwrap();
        assert!(!notes_file.encrypted(), "notes.json must be plain when no password given");
    }

    #[test]
    fn test_read_entry_wrong_password_returns_invalid_password() {
        // Export with a password
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("correct")).unwrap();

        // Try to read an entry with the wrong password
        let mut archive = ZipArchive::new(Cursor::new(&buf)).unwrap();
        let err = read_entry(&mut archive, "notes.json", Some("wrong")).unwrap_err();
        assert!(matches!(err, ExportError::InvalidPassword), "got: {err:?}");
    }
    #[test]
    fn test_peek_import_returns_encrypted_archive_error_when_no_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

        let err = peek_import(Cursor::new(&buf), None).unwrap_err();
        assert!(matches!(err, ExportError::EncryptedArchive), "got: {err:?}");
    }

    #[test]
    fn test_peek_import_with_correct_password_succeeds() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

        let result = peek_import(Cursor::new(&buf), Some("s3cr3t")).unwrap();
        assert_eq!(result.app_version, APP_VERSION);
        assert!(result.note_count >= 1);
    }

    #[test]
    fn test_peek_import_with_wrong_password_returns_invalid_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

        let err = peek_import(Cursor::new(&buf), Some("wrong-password")).unwrap_err();
        assert!(matches!(err, ExportError::InvalidPassword), "got: {err:?}");
    }

    #[test]
    fn test_encrypted_round_trip_import() {
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "").unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "Encrypted Root".to_string()).unwrap();

        // Export with password
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("mypass")).unwrap();

        // Import with correct password → should succeed
        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path(), Some("mypass"), "").unwrap();
        assert_eq!(result.note_count, 1);

        // Verify imported note title
        let imported_ws = Workspace::open(temp_dst.path(), "").unwrap();
        let notes = imported_ws.list_all_notes().unwrap();
        assert!(notes.iter().any(|n| n.title == "Encrypted Root"));
    }
}
