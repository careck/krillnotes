// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Workspace export and import as `.zip` archives.

use std::collections::HashSet;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use ed25519_dalek;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::AesMode;
use zip::{ZipArchive, ZipWriter};

use crate::core::attachment::AttachmentMeta;
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
    #[serde(default)]
    pub category: Option<String>,
}

/// The `scripts/scripts.json` manifest.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScriptManifest {
    pub scripts: Vec<ScriptManifestEntry>,
}

/// Top-level JSON structure in `workspace.json`.
///
/// This is written on export and read back on import to carry authorship,
/// licensing, and gallery-discovery metadata for template distribution.
/// All fields except `version` are optional so old archives without them
/// deserialise successfully (all missing fields default to `None` / empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMetadata {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_org: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Workspace-level taxonomy tags for gallery discovery (distinct from per-note tags).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
            category: Some(script.category.clone()),
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

    // Write workspace.json (workspace metadata)
    let mut ws_meta = workspace
        .get_workspace_metadata()
        .map_err(|e| ExportError::Database(e.to_string()))?;
    ws_meta.version = 1;
    zip.start_file("workspace.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &ws_meta)?;

    // Write attachments
    let all_attachments = workspace
        .list_all_attachments()
        .map_err(|e| ExportError::Database(e.to_string()))?;

    if !all_attachments.is_empty() {
        // Write attachments.json manifest
        zip.start_file("attachments.json", options)?;
        serde_json::to_writer(&mut zip, &all_attachments)
            .map_err(|e| ExportError::Database(e.to_string()))?;

        // Write each attachment file (plaintext — zip password protects them at rest)
        for meta in &all_attachments {
            let plaintext = workspace
                .get_attachment_bytes(&meta.id)
                .map_err(|e| ExportError::Database(e.to_string()))?;
            zip.start_file(
                format!("attachments/{}/{}", meta.id, meta.filename),
                options,
            )?;
            zip.write_all(&plaintext)?;
        }
    }

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
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
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
    let mut script_sources: Vec<(String, i32, bool, String)> = Vec::new(); // (source_code, load_order, enabled, category)
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
            let category = entry.category.clone().unwrap_or_else(|| "schema".to_string());
            script_sources.push((source, entry.load_order, entry.enabled, category));
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
            ["identity_uuid", identity_uuid],
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
                "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    note.id,
                    note.title,
                    note.schema,
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
        for (source_code, load_order, enabled, category) in &script_sources {
            let id = uuid::Uuid::new_v4().to_string();
            let fm = user_script::parse_front_matter(source_code);
            tx.execute(
                "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![id, fm.name, fm.description, source_code, load_order, enabled, now, now, category],
            )
            .map_err(|e| ExportError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| ExportError::Database(e.to_string()))?;
    }

    // Read workspace metadata before dropping storage (archive must still be alive).
    let workspace_metadata: Option<WorkspaceMetadata> =
        try_read_entry(&mut archive, "workspace.json", zip_password)
            .and_then(|cursor| serde_json::from_reader(cursor).ok());

    // Drop storage before opening via Workspace::open (avoids double-locking the file).
    drop(storage);

    // Rebuild the note_links index from the imported fields_json data.
    let mut workspace = Workspace::open(db_path, workspace_password, identity_uuid, signing_key)
        .map_err(|e| ExportError::Database(e.to_string()))?;
    workspace
        .rebuild_note_links_index()
        .map_err(|e| ExportError::Database(e.to_string()))?;

    // Restore attachments if the archive contains them.
    // Use try_read_entry so the borrow on archive is fully released before
    // we call by_name again for each individual attachment file.
    if let Some(att_cursor) = try_read_entry(&mut archive, "attachments.json", zip_password) {
        let att_json = String::from_utf8(att_cursor.into_inner()).unwrap_or_default();
        let attachment_metas: Vec<AttachmentMeta> =
            serde_json::from_str(&att_json).unwrap_or_default();

        for meta in attachment_metas {
            let zip_path = format!("attachments/{}/{}", meta.id, meta.filename);
            if let Some(file_cursor) = try_read_entry(&mut archive, &zip_path, zip_password) {
                let plaintext = file_cursor.into_inner();
                let _ = workspace.attach_file_with_id(
                    &meta.id,
                    &meta.note_id,
                    &meta.filename,
                    meta.mime_type.as_deref(),
                    &plaintext,
                );
            }
        }
    }

    // Restore workspace metadata if the archive contained it.
    if let Some(meta) = workspace_metadata {
        workspace
            .set_workspace_metadata(&meta)
            .map_err(|e| ExportError::Database(e.to_string()))?;
    }

    Ok(ImportResult {
        app_version: export_notes.app_version,
        note_count: export_notes.notes.len(),
        script_count,
    })
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
