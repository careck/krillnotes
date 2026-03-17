// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State, Window};
use uuid::Uuid;
use krillnotes_core::core::sync::relay::RelayClient;
use krillnotes_core::core::sync::SyncChannel;

// --- ReceivedResponse CRUD ---

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedResponseInfo {
    pub response_id: String,
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub invitee_public_key: String,
    pub invitee_declared_name: String,
    pub received_at: String,
    pub status: String,
}

impl From<krillnotes_core::core::received_response::ReceivedResponse> for ReceivedResponseInfo {
    fn from(r: krillnotes_core::core::received_response::ReceivedResponse) -> Self {
        Self {
            response_id: r.response_id.to_string(),
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            invitee_public_key: r.invitee_public_key,
            invitee_declared_name: r.invitee_declared_name,
            received_at: r.received_at.to_rfc3339(),
            status: match r.status {
                krillnotes_core::core::received_response::ReceivedResponseStatus::Pending => "pending".to_string(),
                krillnotes_core::core::received_response::ReceivedResponseStatus::PeerAdded => "peerAdded".to_string(),
                krillnotes_core::core::received_response::ReceivedResponseStatus::SnapshotSent => "snapshotSent".to_string(),
            },
        }
    }
}

#[tauri::command]
pub fn list_received_responses(
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_id: Option<String>,
) -> std::result::Result<Vec<ReceivedResponseInfo>, String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let managers = state.received_response_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let records = if let Some(ws_id) = workspace_id {
        mgr.list_by_workspace(&ws_id).map_err(|e| e.to_string())?
    } else {
        mgr.list().map_err(|e| e.to_string())?
    };
    Ok(records.into_iter().map(ReceivedResponseInfo::from).collect())
}

#[tauri::command]
pub fn update_response_status(
    state: State<'_, AppState>,
    identity_uuid: String,
    response_id: String,
    status: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let resp_uuid: Uuid = response_id.parse().map_err(|e| format!("Invalid response UUID: {e}"))?;
    let mut managers = state.received_response_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    let new_status = match status.as_str() {
        "pending" => krillnotes_core::core::received_response::ReceivedResponseStatus::Pending,
        "peerAdded" => krillnotes_core::core::received_response::ReceivedResponseStatus::PeerAdded,
        "snapshotSent" => krillnotes_core::core::received_response::ReceivedResponseStatus::SnapshotSent,
        _ => return Err(format!("Invalid status: {status}")),
    };
    mgr.update_status(resp_uuid, new_status).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn dismiss_response(
    state: State<'_, AppState>,
    identity_uuid: String,
    response_id: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let resp_uuid: Uuid = response_id.parse().map_err(|e| format!("Invalid response UUID: {e}"))?;
    let mut managers = state.received_response_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    mgr.delete(resp_uuid).map_err(|e| e.to_string())
}

// --- Async polling commands ---

/// Result of a single poll_receive_workspace cycle.
struct PollReceiveResult {
    new_responses: Vec<ReceivedResponseInfo>,
    bundles_applied: usize,
}

/// Poll relay and folder channels for delta bundles and accept-mode bundles
/// for the current workspace. This is the **receive-only** background poller
/// — it never sends.
///
/// - Delta bundles (relay + folder): downloaded/read, decrypted, operations
///   applied to the workspace.
/// - Accept bundles (relay only): parsed into `ReceivedResponse` records for
///   the inviter to review in the Workspace Peers dialog.
#[tauri::command]
pub async fn poll_receive_workspace(
    window: Window,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

    let workspace_label = window.label().to_string();

    // -- Collect context data under brief locks (all guards released before spawn) --
    let identity_uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };

    let workspace_id = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        workspaces
            .get(&workspace_label)
            .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?
            .workspace_id()
            .to_string()
    };

    // Signing key + identity public key (base64) for decrypting and folder inbox.
    let (signing_key, identity_pubkey) = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&identity_uuid).ok_or("Identity not unlocked")?;
        let pubkey_b64 = BASE64.encode(id.verifying_key.as_bytes());
        (id.signing_key.clone(), pubkey_b64)
    };

    let device_id = krillnotes_core::core::device::get_device_id()
        .map_err(|e| e.to_string())?;

    // Get relay accounts for this identity (brief lock).
    let relay_accounts = {
        let ram = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
        if let Some(mgr) = ram.get(&identity_uuid) {
            mgr.list_relay_accounts().unwrap_or_default()
        } else {
            vec![]
        }
    };

    // Get folder paths from folder-channel peers (brief lock on workspaces).
    let folder_paths: Vec<String> = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        if let Some(ws) = workspaces.get(&workspace_label) {
            ws.list_peers_with_channel("folder")
                .unwrap_or_default()
                .iter()
                .filter_map(|p| {
                    serde_json::from_str::<serde_json::Value>(&p.channel_params)
                        .ok()
                        .and_then(|v| v.get("path")?.as_str().map(String::from))
                })
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect()
        } else {
            vec![]
        }
    };

    let has_relay = !relay_accounts.is_empty();
    let has_folders = !folder_paths.is_empty();

    log::debug!(
        "poll_receive_workspace: window={}, identity={}, workspace={}, relay_accounts={}, folder_paths={}",
        workspace_label, identity_uuid, workspace_id, relay_accounts.len(), folder_paths.len(),
    );

    if !has_relay && !has_folders {
        log::debug!("poll_receive_workspace: no relay accounts or folder paths, skipping");
        return Ok(());
    }

    // Get active (non-revoked) invites for this workspace to correlate accept bundles.
    let active_invites: Vec<krillnotes_core::core::invite::InviteRecord> = {
        let ims = state.invite_managers.lock().map_err(|e| e.to_string())?;
        if let Some(im) = ims.get(&identity_uuid) {
            im.list_invites()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| !r.revoked && r.workspace_id == workspace_id)
                .collect()
        } else {
            vec![]
        }
    };

    log::debug!(
        "poll_receive_workspace: found {} active invite(s) for workspace {}",
        active_invites.len(), workspace_id
    );

    // Clone Arcs for use inside spawn_blocking.
    let received_response_managers_arc = Arc::clone(&state.received_response_managers);
    let invite_managers_arc = Arc::clone(&state.invite_managers);
    let workspaces_arc = Arc::clone(&state.workspaces);
    let contact_managers_arc = Arc::clone(&state.contact_managers);
    let workspace_id_clone = workspace_id.clone();
    let workspace_label_clone = workspace_label.clone();

    // Perform relay + folder I/O on a blocking thread.
    let result = tokio::task::spawn_blocking(move || -> Result<PollReceiveResult, String> {
        use krillnotes_core::core::swarm::delta::parse_delta_bundle;
        use krillnotes_core::core::swarm::header::{read_header, SwarmMode};
        use krillnotes_core::core::swarm::invite::parse_accept_bundle;
        use krillnotes_core::core::sync::FolderChannel;
        use krillnotes_core::core::contact::TrustLevel;
        use krillnotes_core::core::operation::Operation;
        use std::collections::HashMap;

        let mut poll_result = PollReceiveResult {
            new_responses: Vec::new(),
            bundles_applied: 0,
        };

        // A parsed delta bundle with its acknowledgement info.
        struct DownloadedDelta {
            ack: DeltaAck,
            parsed: krillnotes_core::core::swarm::delta::ParsedDelta,
        }
        // How to acknowledge a processed bundle (relay vs folder).
        enum DeltaAck {
            /// Relay bundle: DELETE from server.
            Relay(String),
            /// Folder bundle: delete local .swarm file.
            FolderFile(String),
        }

        let mut downloaded_deltas: Vec<DownloadedDelta> = Vec::new();

        // ── Relay channel ────────────────────────────────────────────────────
        // Build an optional relay client (None if no relay accounts).
        let relay_client: Option<RelayClient> = if has_relay {
            let acct = &relay_accounts[0];
            log::debug!("poll_receive_workspace: using relay account {} on {}", acct.email, acct.relay_url);
            let mut token = acct.session_token.clone();
            let mut login_failed = false;
            if acct.session_expires_at < chrono::Utc::now() && !acct.password.is_empty() {
                let client = RelayClient::new(&acct.relay_url);
                match client.login(&acct.email, &acct.password, &acct.device_public_key) {
                    Ok(session) => token = session.session_token,
                    Err(e) => {
                        log::warn!("poll_receive_workspace: auto-login failed for {}: {e}", acct.relay_url);
                        login_failed = true;
                    }
                }
            }
            if login_failed { None } else {
                Some(RelayClient::new(&acct.relay_url).with_session_token(&token))
            }
        } else {
            None
        };

        if let Some(ref client) = relay_client {
            // Ensure mailbox so we can receive bundles for this workspace.
            if let Err(e) = client.ensure_mailbox(&workspace_id_clone) {
                log::warn!("poll_receive_workspace: ensure_mailbox failed: {e}");
            }

            // List all pending relay bundles (single API call).
            let all_bundles = match client.list_bundles() {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("poll_receive_workspace: list_bundles failed: {e}");
                    vec![]
                }
            };

            // ── Accept bundles (invite responses, relay only) ────────────
            if !active_invites.is_empty() {
                let accept_bundles: Vec<_> = all_bundles
                    .iter()
                    .filter(|b| b.mode == "accept" && b.workspace_id == workspace_id_clone)
                    .collect();

                if !accept_bundles.is_empty() {
                    log::info!(
                        "poll_receive_workspace: found {} accept bundle(s) for workspace {}",
                        accept_bundles.len(), workspace_id_clone,
                    );
                }

                for bundle_meta in accept_bundles {
                    let bundle_bytes = match client.download_bundle(&bundle_meta.bundle_id) {
                        Ok(b) => b,
                        Err(e) => {
                            log::warn!("poll_receive_workspace: download accept bundle {} failed: {e}", bundle_meta.bundle_id);
                            continue;
                        }
                    };

                    let parsed = match parse_accept_bundle(&bundle_bytes) {
                        Ok(p) => p,
                        Err(e) => {
                            log::warn!("poll_receive_workspace: parse accept bundle {} failed: {e}", bundle_meta.bundle_id);
                            let _ = client.delete_bundle(&bundle_meta.bundle_id);
                            continue;
                        }
                    };

                    let matched_invite = active_invites
                        .iter()
                        .find(|inv| inv.workspace_id == parsed.workspace_id);

                    let invite_id = match matched_invite {
                        Some(inv) => inv.invite_id,
                        None => {
                            log::warn!(
                                "poll_receive_workspace: no matching invite for workspace {} in accept bundle {}",
                                parsed.workspace_id, bundle_meta.bundle_id,
                            );
                            let _ = client.delete_bundle(&bundle_meta.bundle_id);
                            continue;
                        }
                    };

                    // Check for duplicate responses.
                    {
                        let managers = received_response_managers_arc.lock().map_err(|e| e.to_string())?;
                        if let Some(mgr) = managers.get(&identity_uuid) {
                            if let Ok(Some(_existing)) =
                                mgr.find_by_invite_and_invitee(invite_id, &parsed.acceptor_public_key)
                            {
                                log::info!(
                                    "poll_receive_workspace: duplicate accept from {} for invite {}, skipping",
                                    parsed.declared_name, invite_id,
                                );
                                let _ = client.delete_bundle(&bundle_meta.bundle_id);
                                continue;
                            }
                        }
                    }

                    let workspace_name = matched_invite
                        .map(|inv| inv.workspace_name.clone())
                        .unwrap_or_default();
                    let received = krillnotes_core::core::received_response::ReceivedResponse::new(
                        invite_id,
                        parsed.workspace_id.clone(),
                        workspace_name,
                        parsed.acceptor_public_key.clone(),
                        parsed.declared_name.clone(),
                    );

                    {
                        let mut managers = received_response_managers_arc.lock().map_err(|e| e.to_string())?;
                        if let Some(mgr) = managers.get_mut(&identity_uuid) {
                            if let Err(e) = mgr.save(&received) {
                                log::error!("poll_receive_workspace: failed to save received response: {e}");
                                continue;
                            }
                        }
                    }

                    {
                        let mut ims = invite_managers_arc.lock().map_err(|e| e.to_string())?;
                        if let Some(im) = ims.get_mut(&identity_uuid) {
                            let _ = im.increment_use_count(invite_id);
                        }
                    }

                    log::info!(
                        "poll_receive_workspace: recorded response from '{}' for invite {}",
                        parsed.declared_name, invite_id,
                    );

                    poll_result.new_responses.push(ReceivedResponseInfo::from(received));
                    let _ = client.delete_bundle(&bundle_meta.bundle_id);
                }
            }

            // ── Relay delta bundles ──────────────────────────────────────
            let relay_deltas: Vec<_> = all_bundles
                .iter()
                .filter(|b| b.mode == "delta" && b.workspace_id == workspace_id_clone)
                .collect();

            if !relay_deltas.is_empty() {
                log::info!(
                    "poll_receive_workspace: found {} relay delta bundle(s) for workspace {}",
                    relay_deltas.len(), workspace_id_clone,
                );
            }

            for bundle_meta in &relay_deltas {
                let bundle_bytes = match client.download_bundle(&bundle_meta.bundle_id) {
                    Ok(b) => b,
                    Err(e) => {
                        log::warn!("poll_receive_workspace: download relay delta {} failed: {e}", bundle_meta.bundle_id);
                        continue;
                    }
                };

                match parse_delta_bundle(&bundle_bytes, &signing_key) {
                    Ok(parsed) => {
                        if parsed.workspace_id != workspace_id_clone {
                            log::warn!(
                                "poll_receive_workspace: workspace_id mismatch in relay delta {}: expected {}, got {}",
                                bundle_meta.bundle_id, workspace_id_clone, parsed.workspace_id,
                            );
                            let _ = client.delete_bundle(&bundle_meta.bundle_id);
                            continue;
                        }
                        downloaded_deltas.push(DownloadedDelta {
                            ack: DeltaAck::Relay(bundle_meta.bundle_id.clone()),
                            parsed,
                        });
                    }
                    Err(e) => {
                        log::warn!("poll_receive_workspace: parse relay delta {} failed: {e}", bundle_meta.bundle_id);
                        // Don't acknowledge — retry on next poll.
                    }
                }
            }
        }

        // ── Folder channel ───────────────────────────────────────────────────
        if has_folders {
            let folder_channel = FolderChannel::new(identity_pubkey, device_id);
            folder_channel.set_folder_paths(folder_paths);

            let folder_bundles = match folder_channel.receive_bundles(&workspace_id_clone) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("poll_receive_workspace: folder receive_bundles failed: {e}");
                    vec![]
                }
            };

            if !folder_bundles.is_empty() {
                log::info!(
                    "poll_receive_workspace: found {} folder bundle(s)",
                    folder_bundles.len(),
                );
            }

            for bundle_ref in &folder_bundles {
                // Read header to check mode — only process deltas.
                let header = match read_header(&bundle_ref.data) {
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!("poll_receive_workspace: failed to read folder bundle header {}: {e}", bundle_ref.id);
                        continue;
                    }
                };

                if header.mode != SwarmMode::Delta {
                    log::debug!(
                        "poll_receive_workspace: skipping non-delta folder bundle {} (mode={:?})",
                        bundle_ref.id, header.mode,
                    );
                    continue;
                }

                match parse_delta_bundle(&bundle_ref.data, &signing_key) {
                    Ok(parsed) => {
                        if parsed.workspace_id != workspace_id_clone {
                            log::warn!(
                                "poll_receive_workspace: workspace_id mismatch in folder bundle {}: expected {}, got {}",
                                bundle_ref.id, workspace_id_clone, parsed.workspace_id,
                            );
                            // Acknowledge (delete) misaddressed file.
                            let _ = folder_channel.acknowledge(bundle_ref);
                            continue;
                        }
                        downloaded_deltas.push(DownloadedDelta {
                            ack: DeltaAck::FolderFile(bundle_ref.id.clone()),
                            parsed,
                        });
                    }
                    Err(e) => {
                        log::warn!("poll_receive_workspace: parse folder delta {} failed: {e}", bundle_ref.id);
                        // Don't delete — retry on next poll.
                    }
                }
            }
        }

        // ── Apply all collected deltas (relay + folder) ──────────────────────
        if downloaded_deltas.is_empty() {
            return Ok(poll_result);
        }

        // Collect all operations sorted by HLC for causal ordering.
        struct OpEntry {
            op: krillnotes_core::core::operation::Operation,
            sender_device_id: String,
        }

        let mut op_entries: Vec<OpEntry> = downloaded_deltas
            .iter()
            .flat_map(|dd| {
                let sender = dd.parsed.sender_device_id.clone();
                dd.parsed.operations.iter().map(move |op| OpEntry {
                    op: op.clone(),
                    sender_device_id: sender.clone(),
                })
            })
            .collect();

        op_entries.sort_by_key(|e| e.op.timestamp());

        // Deduplicate by operation_id — the same op can arrive via both relay
        // and folder, or across overlapping bundles from the same peer.
        // apply_incoming_operation handles dups safely (returns Ok(false)),
        // but pre-deduplicating avoids unnecessary DB lookups.
        {
            let mut seen = std::collections::HashSet::new();
            op_entries.retain(|e| seen.insert(e.op.operation_id().to_string()));
        }

        // Lock contact_managers then workspaces (same order as poll_sync).
        let mut contact_managers = contact_managers_arc.lock().map_err(|e| e.to_string())?;
        let contact_manager = match contact_managers.get_mut(&identity_uuid) {
            Some(cm) => cm,
            None => {
                log::warn!("poll_receive_workspace: contact manager not found for identity {identity_uuid}");
                return Ok(poll_result);
            }
        };

        let mut workspaces = workspaces_arc.lock().map_err(|e| e.to_string())?;
        let workspace = match workspaces.get_mut(&workspace_label_clone) {
            Some(ws) => ws,
            None => {
                log::warn!("poll_receive_workspace: workspace not found: {workspace_label_clone}");
                return Ok(poll_result);
            }
        };

        // Apply operations with TOFU.
        let mut sender_applied: HashMap<String, usize> = HashMap::new();

        for entry in &op_entries {
            // TOFU: auto-register unknown operation authors.
            let author_key = entry.op.author_key();
            if !author_key.is_empty()
                && contact_manager.find_by_public_key(author_key).map_err(|e| e.to_string())?.is_none()
            {
                let name = if let Operation::JoinWorkspace { declared_name, .. } = &entry.op {
                    declared_name.clone()
                } else {
                    format!("{}…", &author_key[..8.min(author_key.len())])
                };
                contact_manager
                    .find_or_create_by_public_key(&name, author_key, TrustLevel::Tofu)
                    .map_err(|e| e.to_string())?;
            }

            match workspace.apply_incoming_operation(entry.op.clone(), &entry.sender_device_id) {
                Ok(true) => {
                    log::debug!(
                        "poll_receive_workspace: applied op {} from {}",
                        entry.op.operation_id(), entry.sender_device_id,
                    );
                    *sender_applied.entry(entry.sender_device_id.clone()).or_insert(0) += 1;
                }
                Ok(false) => {
                    log::debug!(
                        "poll_receive_workspace: duplicate op {}, skipping",
                        entry.op.operation_id(),
                    );
                }
                Err(e) => {
                    log::error!(
                        "poll_receive_workspace: apply_incoming_operation failed for op {} from {}: {e}",
                        entry.op.operation_id(), entry.sender_device_id,
                    );
                }
            }
        }

        // Track the HLC-max op per sender for last_received_op.
        let mut sender_last_op: HashMap<String, String> = HashMap::new();
        for entry in &op_entries {
            sender_last_op.insert(
                entry.sender_device_id.clone(),
                entry.op.operation_id().to_string(),
            );
        }

        // Upsert peer registry.
        let mut upserted = std::collections::HashSet::new();
        for dd in &downloaded_deltas {
            let sender = &dd.parsed.sender_device_id;

            if !upserted.contains(sender) {
                upserted.insert(sender.clone());
                let last_received = sender_last_op.get(sender).map(|s| s.as_str());
                if let Err(e) = workspace.upsert_peer_from_delta(
                    sender,
                    &dd.parsed.sender_public_key,
                    last_received,
                ) {
                    log::error!("poll_receive_workspace: upsert_peer_from_delta failed: {e}");
                }
            }

            let applied = sender_applied.get(sender).copied().unwrap_or(0);
            log::info!(
                "poll_receive_workspace: applied delta from peer {}: {} ops",
                sender, applied,
            );
        }

        // Release locks before I/O (acknowledge bundles).
        drop(workspaces);
        drop(contact_managers);

        // Acknowledge all processed bundles.
        for dd in &downloaded_deltas {
            match &dd.ack {
                DeltaAck::Relay(bundle_id) => {
                    if let Some(ref client) = relay_client {
                        let _ = client.delete_bundle(bundle_id);
                    }
                }
                DeltaAck::FolderFile(path) => {
                    if let Err(e) = std::fs::remove_file(path) {
                        log::warn!("poll_receive_workspace: failed to delete folder bundle {path}: {e}");
                    }
                }
            }
        }

        poll_result.bundles_applied = sender_applied.values().sum();
        Ok(poll_result)
    })
    .await
    .map_err(|e| {
        log::error!("poll_receive_workspace spawn_blocking join failed: {e}");
        e.to_string()
    })??;

    // Emit events for each new invite response.
    for resp in &result.new_responses {
        let _ = window.emit("invite-response-received", serde_json::json!({
            "responseId": resp.response_id,
            "inviteId": resp.invite_id,
            "workspaceId": resp.workspace_id,
            "inviteeDeclaredName": resp.invitee_declared_name,
        }));
    }

    // Notify WorkspaceView to reload the note tree if deltas were applied.
    if result.bundles_applied > 0 {
        let _ = window.emit("workspace-updated", ());
    }

    Ok(())
}

#[tauri::command]
pub async fn poll_receive_identity(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    log::debug!("poll_receive_identity: identity={uuid}");

    // Get accepted invites in WaitingSnapshot status (brief lock)
    let waiting = {
        let aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
        let ai_mgr = aim.get(&uuid).ok_or("Accepted invite manager not found")?;
        ai_mgr.list_waiting_snapshot().map_err(|e| e.to_string())?
    };

    log::debug!("poll_receive_identity: {} invite(s) waiting for snapshot", waiting.len());

    if waiting.is_empty() {
        return Ok(());
    }

    // Get relay accounts (brief lock)
    let relay_accounts = {
        let rams = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
        let ram = rams.get(&uuid).ok_or("Relay account manager not found")?;
        ram.list_relay_accounts().unwrap_or_default()
    };

    if relay_accounts.is_empty() {
        return Ok(());
    }

    // Build RelayConnections and call core function inside spawn_blocking
    let temp_dir = std::env::temp_dir();

    let result = tokio::task::spawn_blocking(move || {
        use krillnotes_core::core::sync::receive_poll::{RelayConnection, receive_poll_identity};
        use krillnotes_core::core::sync::relay::client::RelayClient;

        let connections: Vec<RelayConnection> = relay_accounts.into_iter()
            .map(|account| {
                let client = RelayClient::new(&account.relay_url)
                    .with_session_token(&account.session_token);
                RelayConnection { account, client }
            })
            .collect();

        receive_poll_identity(&connections, &waiting, &temp_dir)
            .map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())??;

    // Update accepted invites with snapshot paths and emit events
    for snapshot in &result.received_snapshots {
        // Persist the snapshot path on the AcceptedInvite record
        {
            let mut aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
            if let Some(ai_mgr) = aim.get_mut(&uuid) {
                let _ = ai_mgr.update_snapshot_path(
                    snapshot.invite_id,
                    snapshot.snapshot_path.to_string_lossy().to_string(),
                );
            }
        }
        let _ = app_handle.emit("snapshot-received", serde_json::json!({
            "workspaceId": snapshot.workspace_id,
            "inviteId": snapshot.invite_id.to_string(),
            "snapshotPath": snapshot.snapshot_path.to_string_lossy(),
        }));
    }
    for error in &result.errors {
        let _ = app_handle.emit("poll-error", serde_json::json!({ "error": error.error }));
    }

    Ok(())
}

/// Poll for snapshot bundles across ALL unlocked identities that have
/// relay accounts and accepted invites in WaitingSnapshot status.
/// Called on a global 60-second timer — no workspace needed.
#[tauri::command]
pub async fn poll_all_identity_snapshots(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> std::result::Result<(), String> {
    // Collect all identity UUIDs that have accepted invite managers (= unlocked).
    let identity_uuids: Vec<Uuid> = {
        let aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
        aim.keys().copied().collect()
    };

    for uuid in identity_uuids {
        // Check if this identity has waiting invites.
        let waiting = {
            let aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
            match aim.get(&uuid) {
                Some(mgr) => mgr.list_waiting_snapshot().unwrap_or_default(),
                None => continue,
            }
        };
        if waiting.is_empty() { continue; }

        // Check if this identity has relay accounts.
        let relay_accounts = {
            let rams = state.relay_account_managers.lock().map_err(|e| e.to_string())?;
            match rams.get(&uuid) {
                Some(mgr) => mgr.list_relay_accounts().unwrap_or_default(),
                None => continue,
            }
        };
        if relay_accounts.is_empty() { continue; }

        log::debug!(
            "poll_all_identity_snapshots: identity={uuid}, {} waiting invite(s), {} relay account(s)",
            waiting.len(), relay_accounts.len()
        );

        let temp_dir = std::env::temp_dir();
        let result = tokio::task::spawn_blocking(move || {
            use krillnotes_core::core::sync::receive_poll::{RelayConnection, receive_poll_identity};
            use krillnotes_core::core::sync::relay::client::RelayClient;

            let connections: Vec<RelayConnection> = relay_accounts.into_iter()
                .map(|account| {
                    let client = RelayClient::new(&account.relay_url)
                        .with_session_token(&account.session_token);
                    RelayConnection { account, client }
                })
                .collect();

            receive_poll_identity(&connections, &waiting, &temp_dir)
                .map_err(|e| e.to_string())
        }).await.map_err(|e| e.to_string())??;

        // Update accepted invites with snapshot paths and emit events.
        for snapshot in &result.received_snapshots {
            {
                let mut aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
                if let Some(ai_mgr) = aim.get_mut(&uuid) {
                    let _ = ai_mgr.update_snapshot_path(
                        snapshot.invite_id,
                        snapshot.snapshot_path.to_string_lossy().to_string(),
                    );
                }
            }
            let _ = app_handle.emit("snapshot-received", serde_json::json!({
                "workspaceId": snapshot.workspace_id,
                "inviteId": snapshot.invite_id.to_string(),
                "snapshotPath": snapshot.snapshot_path.to_string_lossy(),
            }));
        }
        for error in &result.errors {
            log::warn!("poll_all_identity_snapshots: identity={uuid}: {}", error.error);
        }
    }

    Ok(())
}
