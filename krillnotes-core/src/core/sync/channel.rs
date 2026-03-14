// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use serde::{Deserialize, Serialize};
use crate::core::error::KrillnotesError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Relay,
    Folder,
    Manual,
}

impl Default for ChannelType {
    fn default() -> Self {
        ChannelType::Manual
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Relay => write!(f, "relay"),
            ChannelType::Folder => write!(f, "folder"),
            ChannelType::Manual => write!(f, "manual"),
        }
    }
}

/// Lightweight view of a peer registry entry, passed to channel methods.
#[derive(Debug, Clone)]
pub struct PeerSyncInfo {
    pub peer_device_id: String,
    pub peer_identity_id: String,
    pub channel_type: ChannelType,
    pub channel_params: serde_json::Value,
    pub last_sent_op: Option<String>,
    pub last_received_op: Option<String>,
}

/// A reference to a bundle received from a channel.
#[derive(Debug, Clone)]
pub struct BundleRef {
    /// Channel-specific identifier (relay bundle_id, file path, etc.)
    pub id: String,
    /// Raw .swarm bytes
    pub data: Vec<u8>,
}

/// Trait for sync transport channels.
///
/// Channel instances are constructed with their required context pre-configured:
/// - RelayChannel: holds RelayClient (with session token) and relay URL
/// - FolderChannel: holds local identity key + device key for header filtering
///
/// This avoids pushing identity/device context through every trait method.
pub trait SyncChannel: Send + Sync {
    /// Send a .swarm bundle to a specific peer.
    fn send_bundle(&self, peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<(), KrillnotesError>;

    /// Check for and download any pending inbound bundles.
    fn receive_bundles(&self, workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError>;

    /// Acknowledge successful processing of a bundle.
    /// Relay: DELETE /bundles/{id}. Folder: delete file.
    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError>;

    /// Channel type identifier.
    fn channel_type(&self) -> ChannelType;

    /// Downcast support for channel-specific operations (e.g., ensure_mailbox on relay).
    fn as_any(&self) -> &dyn std::any::Any;
}
