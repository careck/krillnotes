// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Manual channel marker.
//!
//! The manual channel does not implement `SyncChannel` — it is explicitly
//! excluded from the automated dispatch loop. `ChannelType::Manual` on a peer
//! tells `poll()` to skip that peer.
//!
//! Outbound: user clicks "Generate delta" in the peers dialog.
//! Inbound: user imports .swarm via SwarmOpenDialog.

/// Returns the default outbox directory for manual bundle export.
///
/// Returns `None` here; the host app resolves the download directory
/// platform-specifically (Tauri's path API on desktop).
pub fn default_outbox_dir() -> Option<std::path::PathBuf> {
    None  // Host app resolves download dir platform-specifically
}
