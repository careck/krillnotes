// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

pub mod auth;

#[cfg(feature = "relay")]
pub mod client;

pub use auth::{
    delete_relay_credentials, load_relay_credentials, save_relay_credentials, RelayCredentials,
};

#[cfg(feature = "relay")]
pub use client::RelayClient;

#[cfg(feature = "relay")]
use crate::core::error::KrillnotesError;
#[cfg(feature = "relay")]
use crate::core::sync::channel::{BundleRef, ChannelType, PeerSyncInfo, SendResult, SyncChannel};

#[cfg(feature = "relay")]
pub struct RelayChannel {
    client: RelayClient,
    workspace_id: String,
    sender_device_key: String,
}

#[cfg(feature = "relay")]
impl RelayChannel {
    pub fn new(client: RelayClient, workspace_id: String, sender_device_key: String) -> Self {
        Self { client, workspace_id, sender_device_key }
    }

    pub fn client(&self) -> &RelayClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut RelayClient {
        &mut self.client
    }
}

#[cfg(feature = "relay")]
impl SyncChannel for RelayChannel {
    fn send_bundle(&self, peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<SendResult, KrillnotesError> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        log::debug!(target: "krillnotes::relay", "sending bundle to peer {} via relay ({} bytes)", peer.peer_device_id, bundle_bytes.len());
        // The relay stores device keys as hex-encoded Ed25519 public keys.
        // Convert the peer's base64 identity key to hex to match.
        let recipient_key_hex = {
            let raw = BASE64.decode(&peer.peer_identity_id).map_err(|e| {
                KrillnotesError::RelayUnavailable(format!(
                    "failed to decode peer identity key: {e}"
                ))
            })?;
            hex::encode(raw)
        };
        let header = client::BundleHeader {
            workspace_id: self.workspace_id.clone(),
            sender_device_key: self.sender_device_key.clone(),
            recipient_device_keys: vec![recipient_key_hex],
            mode: None,
        };
        let bundle_ids = self.client.upload_bundle(&header, bundle_bytes)?;
        if !bundle_ids.is_empty() {
            log::info!(target: "krillnotes::relay", "bundle sent to peer {} via relay", peer.peer_device_id);
            Ok(SendResult::Delivered)
        } else {
            log::warn!(target: "krillnotes::relay", "bundle not delivered to peer {} — relay skipped all recipients", peer.peer_device_id);
            Ok(SendResult::NotDelivered {
                reason: "relay skipped all recipients (unknown or unverified device key)".to_string(),
            })
        }
    }

    fn receive_bundles(&self, workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "receiving bundles for workspace {workspace_id}");
        // Ensure a mailbox exists for this workspace so the relay routes bundles
        // to this account. The call is idempotent (201 on first call, 200 after).
        self.client.ensure_mailbox(workspace_id)?;
        let metas = self.client.list_bundles()?;
        let metas: Vec<_> = metas.into_iter().filter(|m| m.workspace_id == workspace_id).collect();
        log::debug!(target: "krillnotes::relay", "{} bundles pending for workspace {workspace_id}", metas.len());
        let mut bundles = Vec::new();
        for meta in &metas {
            match self.client.download_bundle(&meta.bundle_id) {
                Ok(data) => {
                    bundles.push(BundleRef {
                        id: meta.bundle_id.clone(),
                        data,
                    });
                }
                Err(e) => {
                    log::error!(target: "krillnotes::relay", "failed to download bundle {}: {e}", meta.bundle_id);
                    return Err(e);
                }
            }
        }
        log::info!(target: "krillnotes::relay", "received {} bundles via relay for workspace {workspace_id}", bundles.len());
        Ok(bundles)
    }

    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "acknowledging bundle {}", bundle_ref.id);
        self.client.delete_bundle(&bundle_ref.id)
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Relay
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(all(test, feature = "relay"))]
mod tests {
    use super::*;
    use crate::core::sync::channel::ChannelType;

    #[test]
    fn test_relay_channel_construction() {
        let client = RelayClient::new("https://relay.example.com")
            .with_session_token("tok_test");
        let channel = RelayChannel::new(client, "ws-test".to_string(), "sender-key".to_string());
        assert_eq!(channel.channel_type(), ChannelType::Relay);
    }
}
