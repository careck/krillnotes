//! Workspace export and import as `.zip` archives.

use std::collections::HashSet;
use std::io::{Seek, Write};

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::core::note::Note;
use crate::core::workspace::Workspace;

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

    use crate::Workspace;
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
}
