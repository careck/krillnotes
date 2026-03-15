// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

pub mod channel;
pub mod folder;
pub mod manual;

#[cfg(feature = "relay")]
pub mod relay;

pub use channel::{BundleRef, ChannelType, PeerSyncInfo, SendResult, SyncChannel};
pub use folder::FolderChannel;

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use ed25519_dalek::SigningKey;
use zip::ZipArchive;

use crate::core::contact::ContactManager;
use crate::core::error::KrillnotesError;
use crate::core::swarm::header::{SwarmHeader, SwarmMode};
use crate::core::swarm::sync::DeltaBundle;
use crate::core::workspace::Workspace;

// ── SyncEvent ──────────────────────────────────────────────────────────────

/// Events emitted by a single `SyncEngine::poll()` cycle.
///
/// The caller (Tauri command layer) can forward these to the frontend via
/// Tauri events, display toasts, or log them.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// A delta bundle was successfully sent to a peer.
    DeltaSent {
        workspace_id: String,
        peer_device_id: String,
        op_count: usize,
    },
    /// A delta bundle from a peer was successfully applied.
    BundleApplied {
        workspace_id: String,
        peer_device_id: String,
        op_count: usize,
    },
    /// The relay session token has expired — the user must re-authenticate.
    AuthExpired {
        relay_url: String,
    },
    /// A non-fatal error occurred during outbound sync for a peer.
    SyncError {
        workspace_id: String,
        peer_device_id: String,
        error: String,
    },
    /// A non-fatal error occurred while ingesting an inbound bundle.
    IngestError {
        workspace_id: String,
        peer_device_id: String,
        error: String,
    },
    /// Received a bundle with a mode we don't handle in the dispatch loop
    /// (e.g. Snapshot, Invite, Accept).
    UnexpectedBundleMode {
        workspace_id: String,
        mode: String,
    },
    /// Bundle transport succeeded but the relay did not route it to any recipient
    /// (e.g. peer's device key is unknown or unverified). Watermark NOT advanced.
    SendSkipped {
        workspace_id: String,
        peer_device_id: String,
        reason: String,
    },
}

// ── SyncContext ─────────────────────────────────────────────────────────────

/// Ambient context required by the dispatch loop but owned externally.
pub struct SyncContext<'a> {
    pub signing_key: &'a SigningKey,
    pub contact_manager: &'a mut ContactManager,
    pub workspace_name: &'a str,
    pub sender_display_name: &'a str,
}

// ── SyncEventCallback ──────────────────────────────────────────────────────

/// Optional callback for streaming events out of `poll()` as they occur.
pub type SyncEventCallback = Box<dyn Fn(SyncEvent) + Send + Sync>;

// ── SyncEngine ─────────────────────────────────────────────────────────────

/// Orchestrates outbound delta generation + send and inbound bundle
/// receive + apply across all registered transport channels.
pub struct SyncEngine {
    channels: HashMap<ChannelType, Box<dyn SyncChannel>>,
}

impl SyncEngine {
    pub fn new() -> Self {
        log::info!(target: "krillnotes::sync", "sync engine created");
        Self {
            channels: HashMap::new(),
        }
    }

    /// Register a transport channel. Replaces any existing channel of the
    /// same `ChannelType`.
    pub fn register_channel(&mut self, channel: Box<dyn SyncChannel>) {
        let ct = channel.channel_type();
        log::info!(target: "krillnotes::sync", "registered channel: {ct}");
        self.channels.insert(ct, channel);
    }

    /// Run one full sync cycle: outbound deltas, then inbound bundles.
    ///
    /// Returns all events that occurred during the cycle. Errors from
    /// individual peers or bundles are captured as `SyncEvent` variants
    /// rather than aborting the entire poll.
    pub fn poll(
        &self,
        workspace: &mut Workspace,
        ctx: &mut SyncContext<'_>,
    ) -> Result<Vec<SyncEvent>, KrillnotesError> {
        let mut events = Vec::new();
        let workspace_id = workspace.workspace_id().to_string();
        log::info!(target: "krillnotes::sync", "poll started for workspace {workspace_id}");

        // ── 0. Ensure relay mailbox (if relay channel is registered) ────────
        #[cfg(feature = "relay")]
        {
            if let Some(channel) = self.channels.get(&ChannelType::Relay) {
                if let Some(relay) = channel.as_any().downcast_ref::<relay::RelayChannel>() {
                    log::debug!(target: "krillnotes::sync", "ensuring relay mailbox for workspace {workspace_id}");
                    match relay.client().ensure_mailbox(&workspace_id) {
                        Ok(()) => {
                            log::debug!(target: "krillnotes::sync", "relay mailbox ensured");
                        }
                        Err(KrillnotesError::RelayAuthExpired(_)) => {
                            log::warn!(target: "krillnotes::sync", "relay auth expired for {}", relay.client().base_url);
                            events.push(SyncEvent::AuthExpired {
                                relay_url: relay.client().base_url.clone(),
                            });
                            // Don't abort — folder channel peers can still sync.
                        }
                        Err(e) => {
                            log::error!(target: "krillnotes::sync", "ensure_mailbox failed: {e}");
                            // Non-fatal: log as a generic sync error with empty peer
                            events.push(SyncEvent::SyncError {
                                workspace_id: workspace_id.clone(),
                                peer_device_id: String::new(),
                                error: format!("ensure_mailbox failed: {e}"),
                            });
                        }
                    }
                }
            }
        }

        // ── 1. Outbound: generate + send deltas ────────────────────────────
        let active_peers = workspace.get_active_sync_peers()?;
        log::debug!(target: "krillnotes::sync", "outbound: {} active peers", active_peers.len());

        for peer in &active_peers {
            // Mark peer as syncing
            let _ = workspace.update_peer_sync_status(
                &peer.peer_device_id,
                "syncing",
                None,
                None,
            );

            log::debug!(target: "krillnotes::sync", "generating delta for peer {} via {}", peer.peer_device_id, peer.channel_type);

            // Find the channel for this peer
            let channel = match self.channels.get(&peer.channel_type) {
                Some(ch) => ch,
                None => {
                    log::error!(target: "krillnotes::sync", "no channel registered for {}", peer.channel_type);
                    let _ = workspace.update_peer_sync_status(
                        &peer.peer_device_id,
                        "error",
                        None,
                        Some(&format!("no channel registered for {}", peer.channel_type)),
                    );
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        error: format!("no channel registered for {}", peer.channel_type),
                    });
                    continue;
                }
            };

            // Generate delta
            let delta: DeltaBundle = match crate::core::swarm::sync::generate_delta(
                workspace,
                &peer.peer_device_id,
                ctx.workspace_name,
                ctx.signing_key,
                ctx.sender_display_name,
                ctx.contact_manager,
            ) {
                Ok(d) => d,
                Err(e) => {
                    log::error!(target: "krillnotes::sync", "generate_delta failed for peer {}: {e}", peer.peer_device_id);
                    let _ = workspace.update_peer_sync_status(
                        &peer.peer_device_id,
                        "error",
                        None,
                        Some(&e.to_string()),
                    );
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        error: format!("generate_delta: {e}"),
                    });
                    continue;
                }
            };

            // Send via channel
            log::debug!(target: "krillnotes::sync", "sending bundle ({} bytes, {} ops) to peer {}",
                delta.bundle_bytes.len(), delta.op_count, peer.peer_device_id);
            match channel.send_bundle(peer, &delta.bundle_bytes) {
                Ok(SendResult::Delivered) => {
                    log::info!(target: "krillnotes::sync", "delta sent to peer {} ({} ops)", peer.peer_device_id, delta.op_count);
                    // Advance watermark only after confirmed delivery.
                    if let Some(ref last_op_id) = delta.last_included_op {
                        let _ = workspace.upsert_sync_peer(
                            &peer.peer_device_id,
                            &peer.peer_identity_id,
                            Some(last_op_id.as_str()),
                            None,
                        );
                    }
                    let _ = workspace.update_peer_sync_status(
                        &peer.peer_device_id,
                        "idle",
                        None,
                        None,
                    );
                    events.push(SyncEvent::DeltaSent {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        op_count: delta.op_count,
                    });
                }
                Ok(SendResult::NotDelivered { reason }) => {
                    log::warn!(target: "krillnotes::sync",
                        "bundle not delivered to peer {}: {reason}", peer.peer_device_id);
                    // Do NOT advance watermark — peer never received the bundle.
                    let _ = workspace.update_peer_sync_status(
                        &peer.peer_device_id,
                        "not_delivered",
                        Some(&reason),
                        None,
                    );
                    events.push(SyncEvent::SendSkipped {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        reason,
                    });
                }
                Err(KrillnotesError::RelayAuthExpired(_)) => {
                    log::warn!(target: "krillnotes::sync", "relay auth expired while sending to peer {}", peer.peer_device_id);
                    let _ = workspace.update_peer_sync_status(
                        &peer.peer_device_id,
                        "auth_expired",
                        None,
                        Some("relay session expired"),
                    );
                    #[cfg(feature = "relay")]
                    if let Some(ch) = self.channels.get(&ChannelType::Relay) {
                        if let Some(relay) = ch.as_any().downcast_ref::<relay::RelayChannel>() {
                            events.push(SyncEvent::AuthExpired {
                                relay_url: relay.client().base_url.clone(),
                            });
                        }
                    }
                }
                Err(e) => {
                    log::error!(target: "krillnotes::sync", "send_bundle failed for peer {}: {e}", peer.peer_device_id);
                    let _ = workspace.update_peer_sync_status(
                        &peer.peer_device_id,
                        "error",
                        None,
                        Some(&e.to_string()),
                    );
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        error: format!("send_bundle: {e}"),
                    });
                }
            }
        }

        // ── 2. Inbound: receive + apply bundles ────────────────────────────
        log::debug!(target: "krillnotes::sync", "inbound: checking for bundles");

        // Collect unique channel types from active peers (skip Manual)
        let inbound_channel_types: HashSet<ChannelType> = active_peers
            .iter()
            .map(|p| p.channel_type)
            .filter(|ct| *ct != ChannelType::Manual)
            .collect();

        for ct in &inbound_channel_types {
            let channel = match self.channels.get(ct) {
                Some(ch) => ch,
                None => continue,
            };

            // For FolderChannel: update folder_paths before receiving
            if *ct == ChannelType::Folder {
                if let Some(folder) = channel.as_any().downcast_ref::<FolderChannel>() {
                    let folder_paths: Vec<String> = active_peers
                        .iter()
                        .filter(|p| p.channel_type == ChannelType::Folder)
                        .filter_map(|p| {
                            p.channel_params
                                .get("path")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    folder.set_folder_paths(folder_paths);
                }
            }

            // Receive bundles from channel
            log::debug!(target: "krillnotes::sync", "receiving bundles from channel {ct}");
            let bundles = match channel.receive_bundles(&workspace_id) {
                Ok(b) => {
                    log::debug!(target: "krillnotes::sync", "received {} bundles from channel {ct}", b.len());
                    b
                }
                Err(e) => {
                    log::error!(target: "krillnotes::sync", "receive_bundles({ct}) failed: {e}");
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: String::new(),
                        error: format!("receive_bundles({}): {e}", ct),
                    });
                    continue;
                }
            };

            for bundle_ref in &bundles {
                // Read header from zip to determine mode and sender
                let header = match Self::read_header_from_bundle(&bundle_ref.data) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!(target: "krillnotes::sync", "failed to read bundle header: {e}");
                        events.push(SyncEvent::IngestError {
                            workspace_id: workspace_id.clone(),
                            peer_device_id: String::new(),
                            error: format!("failed to read bundle header: {e}"),
                        });
                        // Acknowledge to avoid reprocessing corrupt bundles
                        let _ = channel.acknowledge(bundle_ref);
                        continue;
                    }
                };

                log::debug!(target: "krillnotes::sync", "processing {:?} bundle from peer {}", header.mode, header.source_device_id);

                match header.mode {
                    SwarmMode::Delta => {
                        match crate::core::swarm::sync::apply_delta(
                            &bundle_ref.data,
                            workspace,
                            ctx.signing_key,
                            ctx.contact_manager,
                        ) {
                            Ok(result) => {
                                log::info!(target: "krillnotes::sync", "applied delta from peer {}: {} ops", result.sender_device_id, result.operations_applied);
                                let _ = channel.acknowledge(bundle_ref);
                                events.push(SyncEvent::BundleApplied {
                                    workspace_id: workspace_id.clone(),
                                    peer_device_id: result.sender_device_id,
                                    op_count: result.operations_applied,
                                });
                            }
                            Err(e) => {
                                log::error!(target: "krillnotes::sync", "apply_delta failed for peer {}: {e}", header.source_device_id);
                                // Do NOT acknowledge — retry on next poll
                                events.push(SyncEvent::IngestError {
                                    workspace_id: workspace_id.clone(),
                                    peer_device_id: header.source_device_id.clone(),
                                    error: format!("apply_delta: {e}"),
                                });
                            }
                        }
                    }
                    SwarmMode::Snapshot => {
                        // Decrypt and parse the snapshot bundle, then import it.
                        match crate::core::swarm::snapshot::parse_snapshot_bundle(
                            &bundle_ref.data,
                            ctx.signing_key,
                        ) {
                            Ok(parsed) => {
                                log::info!(target: "krillnotes::sync", "importing snapshot from peer {}", header.source_device_id);
                                match workspace.import_snapshot_json(&parsed.workspace_json) {
                                    Ok(_) => {
                                        log::info!(target: "krillnotes::sync", "snapshot imported from peer {}", header.source_device_id);
                                        let _ = channel.acknowledge(bundle_ref);
                                        events.push(SyncEvent::BundleApplied {
                                            workspace_id: workspace_id.clone(),
                                            peer_device_id: header.source_device_id.clone(),
                                            op_count: 0,
                                        });
                                    }
                                    Err(e) => {
                                        log::error!(target: "krillnotes::sync", "import_snapshot_json failed for peer {}: {e}", header.source_device_id);
                                        events.push(SyncEvent::IngestError {
                                            workspace_id: workspace_id.clone(),
                                            peer_device_id: header.source_device_id.clone(),
                                            error: format!("import_snapshot_json: {e}"),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!(target: "krillnotes::sync", "parse_snapshot_bundle failed for peer {}: {e}", header.source_device_id);
                                events.push(SyncEvent::IngestError {
                                    workspace_id: workspace_id.clone(),
                                    peer_device_id: header.source_device_id.clone(),
                                    error: format!("parse_snapshot_bundle: {e}"),
                                });
                                // Acknowledge to avoid reprocessing unreadable bundles
                                let _ = channel.acknowledge(bundle_ref);
                            }
                        }
                    }
                    other => {
                        log::warn!(target: "krillnotes::sync", "unexpected bundle mode: {:?}", other);
                        events.push(SyncEvent::UnexpectedBundleMode {
                            workspace_id: workspace_id.clone(),
                            mode: format!("{:?}", other).to_lowercase(),
                        });
                        let _ = channel.acknowledge(bundle_ref);
                    }
                }
            }
        }

        log::info!(target: "krillnotes::sync", "poll complete for workspace {workspace_id}: {} events", events.len());
        Ok(events)
    }

    // ── Private helpers ────────────────────────────────────────────────────

    /// Read `header.json` from a `.swarm` zip archive.
    fn read_header_from_bundle(data: &[u8]) -> Result<SwarmHeader, KrillnotesError> {
        let cursor = Cursor::new(data);
        let mut zip = ZipArchive::new(cursor).map_err(|e| {
            KrillnotesError::Swarm(format!("invalid .swarm zip archive: {e}"))
        })?;
        let mut header_file = zip.by_name("header.json").map_err(|e| {
            KrillnotesError::Swarm(format!("missing header.json in .swarm bundle: {e}"))
        })?;
        let header: SwarmHeader =
            serde_json::from_reader(&mut header_file).map_err(|e| {
                KrillnotesError::Swarm(format!("invalid header.json: {e}"))
            })?;
        Ok(header)
    }

}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_engine_new() {
        let engine = SyncEngine::new();
        // Engine with no channels should compile and construct
        assert!(engine.channels.is_empty());
    }

    #[test]
    fn test_sync_engine_default() {
        let engine = SyncEngine::default();
        assert!(engine.channels.is_empty());
    }

    #[test]
    fn test_sync_engine_register_channel() {
        let mut engine = SyncEngine::new();
        let folder = FolderChannel::new("identity".to_string(), "device".to_string());
        engine.register_channel(Box::new(folder));
        assert!(engine.channels.contains_key(&ChannelType::Folder));
    }

    #[test]
    fn test_sync_engine_register_replaces_existing() {
        let mut engine = SyncEngine::new();
        let folder1 = FolderChannel::new("id1".to_string(), "dev1".to_string());
        let folder2 = FolderChannel::new("id2".to_string(), "dev2".to_string());
        engine.register_channel(Box::new(folder1));
        engine.register_channel(Box::new(folder2));
        // Should still have exactly one Folder channel entry
        assert_eq!(engine.channels.len(), 1);
        assert!(engine.channels.contains_key(&ChannelType::Folder));
    }

    #[test]
    fn test_sync_event_debug() {
        let event = SyncEvent::DeltaSent {
            workspace_id: "ws-1".to_string(),
            peer_device_id: "dev-1".to_string(),
            op_count: 5,
        };
        // Verify Debug is implemented
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("DeltaSent"));
    }

    #[test]
    fn test_sync_event_clone() {
        let event = SyncEvent::AuthExpired {
            relay_url: "https://relay.example.com".to_string(),
        };
        let cloned = event.clone();
        match cloned {
            SyncEvent::AuthExpired { relay_url } => {
                assert_eq!(relay_url, "https://relay.example.com");
            }
            _ => panic!("clone produced wrong variant"),
        }
    }
}
