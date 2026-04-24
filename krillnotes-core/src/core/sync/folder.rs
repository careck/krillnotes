// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::core::error::KrillnotesError;
use crate::core::sync::channel::{BundleRef, ChannelType, PeerSyncInfo, SendResult, SyncChannel};
use chrono::Utc;
use std::path::Path;
use uuid::Uuid;

pub struct FolderChannel {
    /// Short prefix of local identity UUID for inbox filtering
    identity_short: String,
    /// Short prefix of local device key; retained for future use, not in filenames
    _device_short: String,
    /// All unique folder paths configured on peers using this channel.
    /// Updated by the SyncEngine before each poll cycle.
    folder_paths: std::sync::Mutex<Vec<String>>,
}

impl FolderChannel {
    /// Compute a filesystem-safe 8-char prefix from a base64 identity string.
    /// Maps `/`→`-` and `+`→`_` (URL-safe base64) to avoid path-separator issues.
    fn identity_short(id: &str) -> String {
        id.chars()
            .take(8)
            .map(|c| match c {
                '/' => '-',
                '+' => '_',
                c => c,
            })
            .collect()
    }

    pub fn new(identity_id: String, device_id: String) -> Self {
        Self {
            identity_short: Self::identity_short(&identity_id),
            _device_short: Self::identity_short(&device_id),
            folder_paths: std::sync::Mutex::new(vec![]),
        }
    }

    /// Update the set of folder paths to scan. Called by SyncEngine
    /// before each poll cycle with paths from all folder-channel peers.
    pub fn set_folder_paths(&self, paths: Vec<String>) {
        log::debug!(target: "krillnotes::sync::folder", "set_folder_paths: {} paths", paths.len());
        *self.folder_paths.lock().unwrap() = paths;
    }

    fn extract_folder_path(peer: &PeerSyncInfo) -> Result<&str, KrillnotesError> {
        peer.channel_params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                KrillnotesError::Swarm(
                    "Folder channel peer missing 'path' in channel_params".to_string(),
                )
            })
    }

    /// Receive bundles from a specific directory (for testing and internal use).
    pub fn receive_bundles_from_dir(&self, dir: &Path) -> Result<Vec<BundleRef>, KrillnotesError> {
        log::debug!(target: "krillnotes::sync::folder", "scanning directory {}", dir.display());
        if !dir.exists() {
            log::error!(target: "krillnotes::sync::folder", "folder not found: {}", dir.display());
            return Err(KrillnotesError::Swarm(format!(
                "Folder not found: {}",
                dir.display()
            )));
        }

        let mut bundles = Vec::new();

        let entries = std::fs::read_dir(dir).map_err(|e| {
            KrillnotesError::Swarm(format!("Cannot read folder {}: {}", dir.display(), e))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "swarm") {
                continue;
            }

            // Inbox filter: only process files addressed to this device.
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let inbox_prefix = format!("{}_", self.identity_short);
            if !filename.starts_with(&inbox_prefix) {
                continue;
            }

            // Format guard: new-format files have a 14-digit timestamp as the second segment.
            // Old-format files (sender_device_ts_uuid) have an 8-char device short there — skip them.
            let rest = &filename[inbox_prefix.len()..];
            let next_segment = rest.split('_').next().unwrap_or("");
            if next_segment.len() != 14 || !next_segment.chars().all(|c| c.is_ascii_digit()) {
                log::debug!(
                    target: "krillnotes::sync::folder",
                    "skipping old-format or misaddressed file: {}",
                    filename
                );
                continue;
            }

            // Try to read the file; skip if it fails (partially written)
            match std::fs::read(&path) {
                Ok(data) => {
                    log::debug!(target: "krillnotes::sync::folder", "read bundle {} ({} bytes)", path.display(), data.len());
                    bundles.push(BundleRef {
                        id: path.to_string_lossy().to_string(),
                        data,
                    });
                }
                Err(e) => {
                    log::debug!(target: "krillnotes::sync::folder", "skipping partially written file {}: {e}", path.display());
                    continue;
                }
            }
        }

        log::info!(target: "krillnotes::sync::folder", "found {} bundles in {}", bundles.len(), dir.display());
        Ok(bundles)
    }
}

impl SyncChannel for FolderChannel {
    fn send_bundle(
        &self,
        peer: &PeerSyncInfo,
        bundle_bytes: &[u8],
    ) -> Result<SendResult, KrillnotesError> {
        let folder_path = Self::extract_folder_path(peer)?;
        let dir = Path::new(folder_path);

        if !dir.exists() {
            log::error!(target: "krillnotes::sync::folder", "folder not found for send: {}", dir.display());
            return Err(KrillnotesError::Swarm(format!(
                "Folder not found: {}",
                dir.display()
            )));
        }

        let recipient_short = Self::identity_short(&peer.peer_identity_id);
        if recipient_short.is_empty() {
            return Err(crate::core::error::KrillnotesError::Swarm(
                "folder channel peer has no identity key".to_string(),
            ));
        }
        let timestamp = Utc::now().format("%Y%m%d%H%M%S");
        let uuid_short: String = Uuid::new_v4().to_string().chars().take(8).collect();
        let filename = format!("{}_{}_{}.swarm", recipient_short, timestamp, uuid_short);

        let path = dir.join(filename);
        std::fs::write(&path, bundle_bytes).map_err(|e| {
            log::error!(target: "krillnotes::sync::folder", "failed to write bundle to {}: {e}", path.display());
            KrillnotesError::Swarm(format!("Failed to write bundle to {}: {}", path.display(), e))
        })?;

        log::info!(target: "krillnotes::sync::folder", "wrote bundle to {} ({} bytes)", path.display(), bundle_bytes.len());
        // Folder write succeeds = delivered (file is there for the peer to pick up)
        Ok(SendResult::Delivered)
    }

    fn receive_bundles(&self, _workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError> {
        let paths = self.folder_paths.lock().unwrap().clone();
        log::debug!(target: "krillnotes::sync::folder", "receiving bundles from {} folder paths", paths.len());
        let mut all_bundles = Vec::new();
        for path in &paths {
            match self.receive_bundles_from_dir(Path::new(path)) {
                Ok(bundles) => all_bundles.extend(bundles),
                Err(e) => {
                    log::warn!(target: "krillnotes::sync::folder", "skipping inaccessible folder {path}: {e}");
                    continue;
                }
            }
        }
        Ok(all_bundles)
    }

    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError> {
        let path = Path::new(&bundle_ref.id);
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| {
                log::error!(target: "krillnotes::sync::folder", "failed to delete {}: {e}", path.display());
                KrillnotesError::Swarm(format!("Failed to delete {}: {}", path.display(), e))
            })?;
            log::debug!(target: "krillnotes::sync::folder", "acknowledged and deleted {}", path.display());
        }
        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Folder
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sync::channel::{ChannelType, PeerSyncInfo, SyncChannel};

    fn make_test_peer(device_id: &str, identity_id: &str, path: &str) -> PeerSyncInfo {
        PeerSyncInfo {
            peer_device_id: device_id.to_string(),
            peer_identity_id: identity_id.to_string(),
            channel_type: ChannelType::Folder,
            channel_params: serde_json::json!({ "path": path }),
            last_sent_op: None,
            last_received_op: None,
        }
    }

    #[test]
    fn test_folder_channel_send_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let channel = FolderChannel::new("my-identity".to_string(), "my-device".to_string());
        let peer = make_test_peer("peer-dev", "peer-id", dir.path().to_str().unwrap());

        channel.send_bundle(&peer, b"test bundle data").unwrap();

        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "swarm"))
            .collect();
        assert_eq!(files.len(), 1);

        // New format: {RECIPIENT_identity_short}_{14-digit-ts}_{8-char-uuid}.swarm
        let filename = files[0]
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let stem = filename.strip_suffix(".swarm").unwrap();
        // Use splitn(3) so the uuid part is kept whole even if it contains '_'
        let parts: Vec<&str> = stem.splitn(3, '_').collect();
        assert_eq!(
            parts.len(),
            3,
            "filename must have exactly 3 '_'-separated segments"
        );
        // First segment = recipient_short = "peer-id".chars().take(8).collect() = "peer-id" (7 chars)
        assert_eq!(
            parts[0], "peer-id",
            "first segment must be recipient identity short"
        );
        // Second segment = 14-digit timestamp
        assert_eq!(
            parts[1].len(),
            14,
            "second segment must be 14-digit timestamp"
        );
        assert!(
            parts[1].chars().all(|c| c.is_ascii_digit()),
            "second segment must be all digits"
        );
        // Third segment = 8-char uuid short
        assert_eq!(parts[2].len(), 8, "third segment must be 8-char uuid");
    }

    #[test]
    fn test_folder_channel_inbox_prefix_filtering() {
        let dir = tempfile::tempdir().unwrap();
        // identity_short = first 8 chars of "my-identity" = "my-ident"
        let channel = FolderChannel::new("my-identity".to_string(), "my-device".to_string());

        // File addressed TO this device (new format) — must be returned
        let inbox_file = dir.path().join("my-ident_20260315120000_abcdef01.swarm");
        std::fs::write(&inbox_file, b"for me").unwrap();

        // File addressed to a different device — must be skipped
        let other_file = dir.path().join("other-ide_20260315120000_abcdef02.swarm");
        std::fs::write(&other_file, b"not for me").unwrap();

        let bundles = channel.receive_bundles_from_dir(dir.path()).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].data, b"for me");
    }

    #[test]
    fn test_folder_channel_ignores_old_format_files() {
        let dir = tempfile::tempdir().unwrap();
        // identity_short = "my-ident"
        let channel = FolderChannel::new("my-identity".to_string(), "my-device".to_string());

        // Old-format file: starts with MY identity short but second segment is NOT 14 digits
        // (it's "my-devic", an 8-char device short)
        let old_file = dir.path().join("my-ident_my-devic_20260315_abcdef.swarm");
        std::fs::write(&old_file, b"old format").unwrap();

        // New-format file addressed to this device — must be returned
        let new_file = dir.path().join("my-ident_20260315120000_abcdef01.swarm");
        std::fs::write(&new_file, b"new format").unwrap();

        let bundles = channel.receive_bundles_from_dir(dir.path()).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].data, b"new format");
    }

    #[test]
    fn test_folder_channel_acknowledge_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.swarm");
        std::fs::write(&path, b"data").unwrap();

        let channel = FolderChannel::new("id".to_string(), "dev".to_string());
        let bundle_ref = BundleRef {
            id: path.to_str().unwrap().to_string(),
            data: vec![],
        };

        channel.acknowledge(&bundle_ref).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_folder_channel_missing_dir_returns_error() {
        let channel = FolderChannel::new("id".to_string(), "dev".to_string());
        let result = channel.receive_bundles_from_dir(std::path::Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }
}
