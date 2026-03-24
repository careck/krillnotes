// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::{Manager, State};
use krillnotes_core::Ed25519SigningKey;

/// Get the signing key for the workspace associated with this window label.
/// Returns None if no identity is loaded (e.g., pre-identity workspaces).
fn get_signing_key_for_window(state: &AppState, label: &str) -> Option<Ed25519SigningKey> {
    let identity_uuid = {
        let m = state.workspace_identities.lock().expect("Mutex poisoned");
        m.get(label).cloned()?
    };
    let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
    let id = ids.get(&identity_uuid)?;
    Some(Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()))
}

#[tauri::command]
pub fn attach_file(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    file_path: String,
) -> std::result::Result<crate::AttachmentMeta, String> {
    let label = window.label();

    // Get signing key BEFORE locking workspaces (lock ordering)
    let signing_key = get_signing_key_for_window(&state, label);

    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    let path = std::path::Path::new(&file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid file path")?
        .to_string();

    let mime_type = mime_guess::from_path(path)
        .first()
        .map(|m| m.to_string());

    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    workspace
        .attach_file(&note_id, &filename, mime_type.as_deref(), &data, signing_key.as_ref())
        .map_err(|e| { log::error!("attach_file failed: {e}"); e.to_string() })
}

/// Attaches a file to a note from raw bytes (used for drag-and-drop, where only
/// file data — not a filesystem path — is available in the frontend).
///
/// Uses binary IPC: the caller passes a `Uint8Array` as the invoke body with
/// `Content-Type: application/octet-stream`, avoiding the ~3× overhead of
/// JSON number-array serialisation.  Metadata travels as HTTP headers:
///   `x-note-id`  — note UUID (ASCII)
///   `x-filename` — base64(UTF-8 bytes of filename) to survive ASCII-only headers
#[tauri::command]
pub fn attach_file_bytes(
    request: tauri::ipc::Request<'_>,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<crate::AttachmentMeta, String> {
    // Extract raw binary body.
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("attach_file_bytes: expected raw binary body".to_string());
    };
    // note_id is a plain UUID — safe as ASCII header value.
    let note_id = request
        .headers()
        .get("x-note-id")
        .and_then(|v| v.to_str().ok())
        .ok_or("attach_file_bytes: missing x-note-id header")?
        .to_owned();
    // filename is base64(UTF-8 bytes) so non-ASCII names survive the ASCII header constraint.
    let filename_b64 = request
        .headers()
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .ok_or("attach_file_bytes: missing x-filename header")?;
    let filename_bytes = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(filename_b64)
            .map_err(|e| format!("attach_file_bytes: invalid filename encoding: {e}"))?
    };
    let filename = String::from_utf8(filename_bytes)
        .map_err(|e| format!("attach_file_bytes: invalid UTF-8 in filename: {e}"))?;

    let label = window.label();

    // Get signing key BEFORE locking workspaces (lock ordering)
    let signing_key = get_signing_key_for_window(&state, label);

    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    let mime_type = mime_guess::from_path(&filename)
        .first()
        .map(|m| m.to_string());
    workspace
        .attach_file(&note_id, &filename, mime_type.as_deref(), data, signing_key.as_ref())
        .map_err(|e| { log::error!("attach_file_bytes failed: {e}"); e.to_string() })
}

/// Returns attachment metadata for all attachments on a note.
#[tauri::command]
pub fn get_attachments(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Vec<crate::AttachmentMeta>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.get_attachments(&note_id).map_err(|e| { log::error!("get_attachments failed: {e}"); e.to_string() })
}

/// Returns the decrypted base64-encoded bytes of an attachment together with its MIME type.
#[derive(serde::Serialize)]
pub struct AttachmentDataResponse {
    pub data: String,
    pub mime_type: Option<String>,
}

#[tauri::command]
pub fn get_attachment_data(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<AttachmentDataResponse, String> {
    use base64::Engine;
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    let (bytes, mime_type) = workspace
        .get_attachment_bytes_and_mime(&attachment_id)
        .map_err(|e| { log::error!("get_attachment_data failed: {e}"); e.to_string() })?;
    Ok(AttachmentDataResponse {
        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        mime_type,
    })
}

/// Deletes an attachment from a note.
#[tauri::command]
pub fn delete_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();

    // Get signing key BEFORE locking workspaces (lock ordering)
    let signing_key = get_signing_key_for_window(&state, label);

    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .delete_attachment(&attachment_id, signing_key.as_ref())
        .map_err(|e| { log::error!("delete_attachment failed: {e}"); e.to_string() })
}

/// Restores a previously soft-deleted attachment (moves `.enc.trash` → `.enc`, re-inserts DB row).
/// Called from the in-section "Restore" button in AttachmentsSection.
#[tauri::command]
pub fn restore_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    meta: crate::AttachmentMeta,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .restore_attachment(&meta)
        .map_err(|e| { log::error!("restore_attachment failed: {e}"); e.to_string() })
}

/// Decrypts an attachment to a temp file and opens it with the default system application.
#[tauri::command]
pub async fn open_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
    filename: String,
) -> std::result::Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let bytes = {
        let label = window.label();
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(label).ok_or("No workspace open")?;
        workspace
            .get_attachment_bytes(&attachment_id)
            .map_err(|e| e.to_string())?
    };

    let tmp_dir = std::env::temp_dir().join("krillnotes-attachments");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let tmp_path = tmp_dir.join(&filename);
    std::fs::write(&tmp_path, &bytes).map_err(|e| e.to_string())?;

    window
        .app_handle()
        .opener()
        .open_path(tmp_path.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|e: tauri_plugin_opener::Error| e.to_string())
}
