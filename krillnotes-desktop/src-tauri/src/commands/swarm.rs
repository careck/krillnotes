// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use krillnotes_core::Ed25519SigningKey;
use krillnotes_core::Ed25519VerifyingKey;

// ── Swarm bundle commands ──────────────────────────────────────────

/// Info returned to the frontend after peeking at a .swarm bundle header.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum SwarmFileInfo {
    Invite {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        #[serde(rename = "offeredRole")]
        offered_role: String,
        #[serde(rename = "offeredScope")]
        offered_scope: Option<String>,
        #[serde(rename = "inviterDisplayName")]
        inviter_display_name: String,
        #[serde(rename = "inviterFingerprint")]
        inviter_fingerprint: String,
        #[serde(rename = "pairingToken")]
        pairing_token: String,
        #[serde(rename = "targetIdentityUuid")]
        target_identity_uuid: Option<String>,
        #[serde(rename = "targetIdentityName")]
        target_identity_name: Option<String>,
    },
    Accept {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        #[serde(rename = "declaredName")]
        declared_name: String,
        #[serde(rename = "acceptorFingerprint")]
        acceptor_fingerprint: String,
        #[serde(rename = "acceptorPublicKey")]
        acceptor_public_key: String,
        #[serde(rename = "pairingToken")]
        pairing_token: String,
    },
    Snapshot {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        #[serde(rename = "senderDisplayName")]
        sender_display_name: String,
        #[serde(rename = "senderFingerprint")]
        sender_fingerprint: String,
        #[serde(rename = "asOfOperationId")]
        as_of_operation_id: String,
        #[serde(rename = "targetIdentityUuid")]
        target_identity_uuid: Option<String>,
        #[serde(rename = "targetIdentityName")]
        target_identity_name: Option<String>,
    },
    Delta {
        #[serde(rename = "workspaceName")]
        workspace_name: String,
        /// Name of the local workspace this delta targets (folder name).
        /// Present when the recipient identity has a workspace open;
        /// falls back to `workspaceName` (sender's name) if None.
        #[serde(rename = "localWorkspaceName")]
        local_workspace_name: Option<String>,
        #[serde(rename = "senderDisplayName")]
        sender_display_name: String,
        #[serde(rename = "senderFingerprint")]
        sender_fingerprint: String,
        #[serde(rename = "sinceOperationId")]
        since_operation_id: Option<String>,
        #[serde(rename = "targetIdentityUuid")]
        target_identity_uuid: Option<String>,
        #[serde(rename = "targetIdentityName")]
        target_identity_name: Option<String>,
    },
}

/// Read and deserialise just the header.json from a .swarm zip bundle.
fn peek_swarm_header(data: &[u8]) -> std::result::Result<krillnotes_core::core::swarm::header::SwarmHeader, String> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;
    let cursor = Cursor::new(data);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| format!("Cannot open bundle: {e}"))?;

    // Detect Phase C invite/response files before trying to read header.json
    if zip.by_name("invite.json").is_ok() {
        return Err("This is a Phase C invite file. Use the 'Import Invite' button to open it.".to_string());
    }
    if zip.by_name("response.json").is_ok() {
        return Err("This is a Phase C response file. Use the 'Import Response' button to open it.".to_string());
    }

    let header_bytes = {
        let mut file = zip.by_name("header.json")
            .map_err(|_| "bundle missing 'header.json'".to_string())?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|e| format!("Cannot read header: {e}"))?;
        buf
    };
    serde_json::from_slice(&header_bytes)
        .map_err(|e| format!("Invalid header: {e}"))
}

/// Peek at a .swarm file and return its type + display metadata.
#[tauri::command]
pub fn open_swarm_file_cmd(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<SwarmFileInfo, String> {
    use krillnotes_core::core::swarm::header::SwarmMode;
    let data = std::fs::read(&path).map_err(|e| { log::error!("open_swarm_file failed: {e}"); format!("Cannot read file: {e}") })?;
    let header = peek_swarm_header(&data)?;

    let fingerprint = krillnotes_core::core::contact::generate_fingerprint(&header.source_identity)
        .map_err(|e| e.to_string())?;

    match header.mode {
        SwarmMode::Invite => {
            let (target_identity_uuid, target_identity_name) = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let identities = mgr.list_identities().unwrap_or_default();
                let mut found_uuid = None;
                let mut found_name = None;
                if let Some(ref target_pubkey) = header.target_peer {
                    for identity_ref in &identities {
                        let full_path = mgr.identity_file_path(&identity_ref.uuid);
                        if let Ok(data) = std::fs::read_to_string(&full_path) {
                            if let Ok(file) = serde_json::from_str::<krillnotes_core::core::identity::IdentityFile>(&data) {
                                if &file.public_key == target_pubkey {
                                    found_uuid = Some(identity_ref.uuid.to_string());
                                    found_name = Some(identity_ref.display_name.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
                (found_uuid, found_name)
            };
            Ok(SwarmFileInfo::Invite {
                workspace_name: header.workspace_name,
                offered_role: header.offered_role.unwrap_or_default(),
                offered_scope: header.offered_scope,
                inviter_display_name: header.source_display_name,
                inviter_fingerprint: fingerprint,
                pairing_token: header.pairing_token.unwrap_or_default(),
                target_identity_uuid,
                target_identity_name,
            })
        }
        SwarmMode::Accept => Ok(SwarmFileInfo::Accept {
            workspace_name: header.workspace_name,
            declared_name: header.source_display_name,
            acceptor_fingerprint: fingerprint,
            acceptor_public_key: header.source_identity,
            pairing_token: header.pairing_token.unwrap_or_default(),
        }),
        SwarmMode::Snapshot => {
            // Try to identify which local identity this snapshot is for.
            let (target_identity_uuid, target_identity_name) = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let identities = mgr.list_identities().unwrap_or_default();
                // Read each identity's public key and match against recipient peer_ids.
                let peer_ids: Vec<String> = header.recipients.as_ref()
                    .map(|r| r.iter().map(|e| e.peer_id.clone()).collect())
                    .unwrap_or_default();
                let mut found_uuid = None;
                let mut found_name = None;
                for identity_ref in &identities {
                    let full_path = mgr.identity_file_path(&identity_ref.uuid);
                    if let Ok(data) = std::fs::read_to_string(&full_path) {
                        if let Ok(file) = serde_json::from_str::<krillnotes_core::core::identity::IdentityFile>(&data) {
                            if peer_ids.contains(&file.public_key) {
                                found_uuid = Some(identity_ref.uuid.to_string());
                                found_name = Some(identity_ref.display_name.clone());
                                break;
                            }
                        }
                    }
                }
                (found_uuid, found_name)
            };
            Ok(SwarmFileInfo::Snapshot {
                workspace_name: header.workspace_name,
                sender_display_name: header.source_display_name,
                sender_fingerprint: fingerprint,
                as_of_operation_id: header.as_of_operation_id.unwrap_or_default(),
                target_identity_uuid,
                target_identity_name,
            })
        }
        SwarmMode::Delta => {
            // Identify which local identity this delta is addressed to.
            let (target_identity_uuid, target_identity_name) = {
                let mgr = state.identity_manager.lock().expect("Mutex poisoned");
                let identities = mgr.list_identities().unwrap_or_default();
                let target_pubkey = header.target_peer.as_deref().unwrap_or("");
                let mut found_uuid = None;
                let mut found_name = None;
                for identity_ref in &identities {
                    let full_path = mgr.identity_file_path(&identity_ref.uuid);
                    if let Ok(file_data) = std::fs::read_to_string(&full_path) {
                        if let Ok(file) = serde_json::from_str::<krillnotes_core::core::identity::IdentityFile>(&file_data) {
                            if file.public_key == target_pubkey {
                                found_uuid = Some(identity_ref.uuid.to_string());
                                found_name = Some(identity_ref.display_name.clone());
                                break;
                            }
                        }
                    }
                }
                (found_uuid, found_name)
            };
            // Find the local workspace name for the recipient identity's open workspace.
            let local_workspace_name = target_identity_uuid.as_deref()
                .and_then(|uuid_str| Uuid::parse_str(uuid_str).ok())
                .and_then(|uuid| {
                    let identity_map = state.workspace_identities.lock().expect("Mutex poisoned");
                    let paths = state.workspace_paths.lock().expect("Mutex poisoned");
                    identity_map.iter()
                        .find(|(_, id)| **id == uuid)
                        .and_then(|(lbl, _)| paths.get(lbl))
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                });
            Ok(SwarmFileInfo::Delta {
                workspace_name: header.workspace_name,
                local_workspace_name,
                sender_display_name: header.source_display_name,
                sender_fingerprint: fingerprint,
                since_operation_id: header.since_operation_id,
                target_identity_uuid,
                target_identity_name,
            })
        }
    }
}

/// Serialisable result returned after a snapshot bundle is written to disk.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCreatedResult {
    pub saved_path: String,
    pub peer_count: usize,
    pub as_of_operation_id: String,
}

#[tauri::command]
pub async fn create_snapshot_for_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    peer_public_keys: Vec<String>,   // base64-encoded Ed25519 verifying keys
    save_path: String,
) -> std::result::Result<SnapshotCreatedResult, String> {
    use base64::Engine;
    use krillnotes_core::core::swarm::snapshot::create_snapshot_bundle;
    use krillnotes_core::core::swarm::snapshot::SnapshotParams;

    let identity_uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // 1. Sender signing key + display name.
    let (signing_key, source_display_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid).ok_or("Identity not unlocked")?;
        (
            Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };
    let source_device_id = krillnotes_core::get_device_id().map_err(|e| e.to_string())?;

    // 2. Decode recipient verifying keys from base64.
    let recipient_vks: Vec<Ed25519VerifyingKey> = peer_public_keys
        .iter()
        .map(|pk_b64| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(pk_b64)
                .map_err(|e| e.to_string())?;
            let arr: [u8; 32] = bytes.try_into().map_err(|_| "key wrong length".to_string())?;
            Ed25519VerifyingKey::from_bytes(&arr).map_err(|e| e.to_string())
        })
        .collect::<std::result::Result<_, _>>()?;

    // 3. Collect workspace data (hold lock only briefly).
    let (workspace_id, workspace_name, workspace_json, attachment_blobs, as_of_op_id, owner_pubkey) = {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let paths = state.workspace_paths.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        let workspace_name = paths
            .get(window.label())
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let workspace_id = ws.workspace_id().to_string();
        let owner_pubkey = ws.owner_pubkey().to_string();

        let workspace_json = ws.to_snapshot_json().map_err(|e| e.to_string())?;

        // Get attachment metadata from the snapshot JSON to load blobs.
        let snapshot: krillnotes_core::core::workspace::WorkspaceSnapshot = serde_json::from_slice(&workspace_json)
            .map_err(|e| e.to_string())?;
        let mut attachment_blobs: Vec<(String, Vec<u8>)> = Vec::new();
        for meta in &snapshot.attachments {
            let plaintext = ws.get_attachment_bytes(&meta.id).map_err(|e| e.to_string())?;
            attachment_blobs.push((meta.id.clone(), plaintext));
        }

        let as_of_op_id = ws.get_latest_operation_id()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();

        (workspace_id, workspace_name, workspace_json, attachment_blobs, as_of_op_id, owner_pubkey)
    };

    // 4. Build the bundle.
    let recipient_refs: Vec<&Ed25519VerifyingKey> = recipient_vks.iter().collect();
    let bundle_bytes = create_snapshot_bundle(SnapshotParams {
        workspace_id: workspace_id.clone(),
        workspace_name,
        source_device_id,
        source_display_name,
        as_of_operation_id: as_of_op_id.clone(),
        workspace_json,
        sender_key: &signing_key,
        recipient_keys: recipient_refs,
        recipient_peer_ids: peer_public_keys.clone(),
        attachment_blobs,
        owner_pubkey,
    }).map_err(|e| { log::error!("create_snapshot_for_peers failed: {e}"); e.to_string() })?;

    // 5. Write to file.
    std::fs::write(&save_path, &bundle_bytes).map_err(|e| e.to_string())?;

    // 6. Update last_sent_op for each recipient — always, even for empty workspaces.
    // An empty as_of_op_id ("") is a valid sentinel meaning "start of log": operations_since
    // falls back to sending all ops, and the recipient's INSERT OR IGNORE handles duplicates.
    {
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        if let Some(ws) = workspaces.get(window.label()) {
            for pk in &peer_public_keys {
                let _ = ws.update_peer_last_sent_by_identity(pk, &as_of_op_id);
            }
        }
    }

    Ok(SnapshotCreatedResult {
        saved_path: save_path,
        peer_count: peer_public_keys.len(),
        as_of_operation_id: as_of_op_id,
    })
}

/// Apply a `.swarm` snapshot bundle to create a new local workspace.
///
/// Mirrors `execute_import`: decrypts the bundle, creates the workspace DB with
/// the snapshot's UUID preserved (required for CRDT convergence), restores all
/// notes, user scripts, and attachments, then opens a new window.
#[tauri::command]
pub async fn apply_swarm_snapshot(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    identity_uuid: String,
    workspace_name_override: Option<String>,
) -> std::result::Result<crate::WorkspaceInfo, String> {
    use base64::Engine;
    use krillnotes_core::core::swarm::snapshot::parse_snapshot_bundle;
    use krillnotes_core::core::workspace::WorkspaceSnapshot;
    use rand::RngCore;
    use krillnotes_core::Workspace;

    let identity_uuid_parsed = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // 1. Read bundle bytes and get the recipient signing key from the unlocked identity.
    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
    let import_seed = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid_parsed).ok_or("Identity not unlocked")?;
        id.signing_key.to_bytes()
    };
    let recipient_key = Ed25519SigningKey::from_bytes(&import_seed);
    let parsed = parse_snapshot_bundle(&data, &recipient_key).map_err(|e| { log::error!("apply_swarm_snapshot failed: {e}"); e.to_string() })?;

    // Deserialise snapshot JSON now so we can look up attachment metadata later.
    let snapshot: WorkspaceSnapshot = serde_json::from_slice(&parsed.workspace_json)
        .map_err(|e| e.to_string())?;

    // 2. Determine workspace name → folder name (mirrors file-stem convention).
    let ws_name = workspace_name_override
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| parsed.workspace_name.clone());

    // Derive folder inside the user's configured workspace directory,
    // the same location used by create_workspace and list_workspace_files.
    let folder = PathBuf::from(&crate::settings::load_settings().workspace_directory)
        .join(&ws_name);

    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("create workspace dir: {e}"))?;
    let db_path = folder.join("notes.db");
    if db_path.exists() {
        return Err(format!("Workspace '{}' already exists locally.", ws_name));
    }

    // 3. Generate a fresh DB encryption password (never leaves this device).
    let workspace_password: String = {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        base64::engine::general_purpose::STANDARD.encode(bytes)
    };

    // 4. Create workspace DB preserving the snapshot's UUID.
    let mut ws = Workspace::create_empty_with_id(
        &db_path,
        &workspace_password,
        &identity_uuid,
        Ed25519SigningKey::from_bytes(&import_seed),
        &parsed.workspace_id,
    )
    .map_err(|e| e.to_string())?;

    // 5. Restore notes + user scripts from the snapshot.
    ws.import_snapshot_json(&parsed.workspace_json)
        .map_err(|e| e.to_string())?;
    // Run the imported scripts in the Rhai engine so all schemas are registered.
    ws.reload_all_scripts().map_err(|e| e.to_string())?;

    // 6. Restore attachment blobs — look up metadata from snapshot to pass correct fields.
    let _ = std::fs::create_dir_all(folder.join("attachments"));
    for (att_id, plaintext) in &parsed.attachment_blobs {
        if let Some(meta) = snapshot.attachments.iter().find(|a| a.id == *att_id) {
            ws.attach_file_with_id(
                att_id,
                &meta.note_id,
                &meta.filename,
                meta.mime_type.as_deref(),
                plaintext,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // Set the true workspace owner from the snapshot header
    if let Some(ref snapshot_owner) = parsed.owner_pubkey {
        ws.set_owner_pubkey(snapshot_owner)
            .map_err(|e| e.to_string())?;
    }

    // 7. Register the snapshot sender as a sync peer with last_received_op = snapshot watermark.
    let placeholder_device_id = format!("identity:{}", parsed.sender_public_key);
    let _ = ws.upsert_sync_peer(
        &placeholder_device_id,
        &parsed.sender_public_key,
        Some(&parsed.as_of_operation_id),  // last_sent_op — snapshot is the baseline
        Some(&parsed.as_of_operation_id),  // last_received_op
    );

    // 7b. Register sender in the contact manager so generate_delta can resolve their
    //     encryption key. Snapshot bundles carry no display name, so use a synthetic
    //     Falls back to a key-prefix placeholder if the bundle has no display name.
    {
        use krillnotes_core::core::contact::TrustLevel;
        let sender_key = &parsed.sender_public_key;
        let name = if parsed.sender_display_name.is_empty() {
            format!("{}…", &sender_key[..8.min(sender_key.len())])
        } else {
            parsed.sender_display_name.clone()
        };
        let cms = state.contact_managers.lock().expect("Mutex poisoned");
        if let Some(cm) = cms.get(&identity_uuid_parsed) {
            let _ = cm.find_or_create_by_public_key(&name, sender_key, TrustLevel::Tofu);
        }
    }

    // 8. Bind workspace to identity so it can be reopened on next launch.
    let workspace_uuid = ws.workspace_id().to_string();
    {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let unlocked = ids.get(&identity_uuid_parsed).ok_or("Identity not unlocked")?;
        let seed = unlocked.signing_key.to_bytes();
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.bind_workspace(
            &identity_uuid_parsed,
            &workspace_uuid,
            &folder,
            &workspace_password,
            &seed,
        )
        .map_err(|e| format!("bind_workspace: {e}"))?;
    }

    // 9. Open the workspace in a new window (mirrors execute_import exactly).
    let label = crate::generate_unique_label(&state, &folder);
    let new_window = crate::create_workspace_window(&app, &label, &window)?;
    crate::store_workspace(&state, label.clone(), ws, folder, identity_uuid_parsed);
    new_window
        .set_title(&format!("Krillnotes - {ws_name}"))
        .map_err(|e| e.to_string())?;
    if window.label() == "main" {
        window.close().map_err(|e| e.to_string())?;
    }

    crate::get_workspace_info_internal(&state, &label)
}

/// Apply a `.swarm` delta bundle to the currently open workspace.
///
/// Decrypts, verifies, and applies operations from the delta to the workspace.
/// Emits `workspace-updated` so the frontend refreshes the tree view.
#[tauri::command]
pub async fn apply_swarm_delta(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
    identity_uuid: String,
) -> std::result::Result<String, String> {
    use krillnotes_core::core::swarm::sync::apply_delta;

    let identity_uuid_parsed = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let bundle_bytes = std::fs::read(&path).map_err(|e| e.to_string())?;

    let recipient_key = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&identity_uuid_parsed).ok_or("Identity not unlocked")?;
        Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes())
    };

    // Find the workspace window that belongs to the recipient identity.
    // Using window.label() would route to whichever window opened the file,
    // which may be a different user's workspace in a multi-workspace session.
    let target_label = {
        let identity_map = state.workspace_identities.lock().expect("Mutex poisoned");
        identity_map.iter()
            .find(|(_, id)| **id == identity_uuid_parsed)
            .map(|(lbl, _)| lbl.clone())
            .ok_or("No open workspace for this identity")?
    };

    let apply_result = {
        let mut cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
        let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get_mut(&target_label).ok_or("Workspace not open")?;
        let cm = cm_guard.get_mut(&identity_uuid_parsed).ok_or("Contact manager not available")?;
        apply_delta(&bundle_bytes, ws, &recipient_key, cm).map_err(|e| { log::error!("apply_swarm_delta failed: {e}"); e.to_string() })?
    };

    // Emit workspace-updated on the target workspace's window so it refreshes.
    if let Some(target_win) = window.app_handle().get_webview_window(&target_label) {
        let _ = target_win.emit("workspace-updated", ());
    } else {
        let _ = window.emit("workspace-updated", ());
    }

    Ok(serde_json::json!({
        "mode": "delta",
        "operationsApplied": apply_result.operations_applied,
        "operationsSkipped": apply_result.operations_skipped,
        "newTofu": apply_result.new_tofu_contacts,
    }).to_string())
}

/// Serialisable result returned after one or more delta bundles are written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateDeltasResult {
    succeeded: Vec<String>,          // peer_device_ids that worked
    failed: Vec<(String, String)>,   // (peer_device_id, error_message)
    files_written: Vec<String>,      // absolute paths of written .swarm files
}

/// Batch-generates one delta .swarm per selected peer into `dir_path`.
///
/// Continues on per-peer errors so a single failure doesn't block the others.
#[tauri::command]
pub async fn generate_deltas_for_peers(
    window: tauri::Window,
    state: State<'_, AppState>,
    dir_path: String,
    peer_device_ids: Vec<String>,
) -> std::result::Result<GenerateDeltasResult, String> {
    use krillnotes_core::core::swarm::sync::generate_delta;

    // Get signing key, display name, and workspace name upfront (before per-peer loop).
    let (signing_key, sender_display_name, workspace_name, identity_uuid) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
        let identity_uuid_str = ws.identity_uuid().to_string();
        let identity_uuid = Uuid::parse_str(&identity_uuid_str).map_err(|e| e.to_string())?;

        let id = ids.get(&identity_uuid).ok_or("Identity not unlocked")?;
        let key = Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes());
        let display_name = id.display_name.clone();

        let paths = state.workspace_paths.lock().expect("Mutex poisoned");
        let ws_name = paths
            .get(window.label())
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        (key, display_name, ws_name, identity_uuid)
    };

    let dir = std::path::Path::new(&dir_path);
    if !dir.exists() {
        return Err(format!("Directory does not exist: {dir_path}"));
    }

    let mut result = GenerateDeltasResult {
        succeeded: Vec::new(),
        failed: Vec::new(),
        files_written: Vec::new(),
    };

    for peer_id in &peer_device_ids {
        // Resolve display name for file naming.
        let display_name = {
            let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
            let workspaces = state.workspaces.lock().expect("Mutex poisoned");
            let ws = workspaces.get(window.label()).ok_or("Workspace not open")?;
            if let Some(cm) = cm_guard.get(&identity_uuid) {
                ws.list_peers_info(cm)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|p| &p.peer_device_id == peer_id)
                    .map(|p| p.display_name)
                    .unwrap_or_else(|| peer_id[..8.min(peer_id.len())].to_string())
            } else {
                peer_id[..8.min(peer_id.len())].to_string()
            }
        };

        // Sanitise display name for use in file path.
        let safe_name: String = display_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let base_name = format!("delta-{safe_name}-{date}.swarm");

        // Avoid overwriting existing files.
        let file_path = {
            let mut p = dir.join(&base_name);
            let mut n = 2u32;
            while p.exists() {
                let stem = format!("delta-{safe_name}-{date}-{n}.swarm");
                p = dir.join(stem);
                n += 1;
            }
            p
        };

        // Generate the delta.
        let bundle_result = {
            let cm_guard = state.contact_managers.lock().expect("Mutex poisoned");
            let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
            let ws = workspaces.get_mut(window.label()).ok_or("Workspace not open")?;
            if let Some(cm) = cm_guard.get(&identity_uuid) {
                generate_delta(ws, peer_id, &workspace_name, &signing_key, &sender_display_name, cm)
                    .map_err(|e| e.to_string())
            } else {
                Err("Contact manager not available".to_string())
            }
        };

        match bundle_result {
            Ok(delta) => match std::fs::write(&file_path, &delta.bundle_bytes) {
                Ok(()) => {
                    result.succeeded.push(peer_id.clone());
                    result.files_written.push(file_path.to_string_lossy().to_string());
                }
                Err(e) => result.failed.push((peer_id.clone(), e.to_string())),
            },
            Err(e) => result.failed.push((peer_id.clone(), e)),
        }
    }

    Ok(result)
}
