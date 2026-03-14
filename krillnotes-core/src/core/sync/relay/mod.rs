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
use crate::core::sync::channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};

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
    fn send_bundle(&self, peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<(), KrillnotesError> {
        let header = client::BundleHeader {
            workspace_id: self.workspace_id.clone(),
            sender_device_key: self.sender_device_key.clone(),
            recipient_device_keys: vec![peer.peer_device_id.clone()],
            mode: None,
        };
        self.client.upload_bundle(&header, bundle_bytes)?;
        Ok(())
    }

    fn receive_bundles(&self, workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError> {
        // Ensure a mailbox exists for this workspace so the relay routes bundles
        // to this account. The call is idempotent (201 on first call, 200 after).
        self.client.ensure_mailbox(workspace_id)?;
        let metas = self.client.list_bundles()?;
        let metas: Vec<_> = metas.into_iter().filter(|m| m.workspace_id == workspace_id).collect();
        let mut bundles = Vec::new();
        for meta in metas {
            let data = self.client.download_bundle(&meta.bundle_id)?;
            bundles.push(BundleRef {
                id: meta.bundle_id,
                data,
            });
        }
        Ok(bundles)
    }

    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError> {
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
