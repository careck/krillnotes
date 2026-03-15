# Sync Engine Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add automated multi-channel sync transport (relay, folder, manual) to the existing invite → snapshot → delta pipeline in krillnotes-core.

**Architecture:** Host-driven polling model. A `SyncChannel` trait abstracts transport; `SyncEngine` dispatches per-peer based on channel config stored in the extended `sync_peers` table. Relay uses `reqwest::blocking` behind a Cargo feature flag. Folder scans a shared directory. Manual is excluded from automation. All channels feed the same `apply_delta()` / `import_snapshot_json()` ingestion pipeline.

**Tech Stack:** Rust (krillnotes-core), reqwest::blocking, crypto_box crate, TypeScript/React (krillnotes-desktop), Tauri v2 IPC.

**Spec:** `docs/plans/2026-03-14-sync-engine-design.md`

---

## File Structure

### New Files (krillnotes-core)

| File | Responsibility |
|------|---------------|
| `src/core/sync/mod.rs` | `SyncEngine`, `SyncContext`, `SyncEvent` enum, `poll()` dispatch loop |
| `src/core/sync/channel.rs` | `SyncChannel` trait, `ChannelType` enum, `PeerSyncInfo`, `BundleRef` |
| `src/core/sync/relay/mod.rs` | `RelayChannel` (implements `SyncChannel`), channel construction |
| `src/core/sync/relay/client.rs` | `RelayClient` — reqwest HTTP wrapper, all relay API endpoints |
| `src/core/sync/relay/auth.rs` | `RelayCredentials`, PoP challenge resolution, credential file I/O |
| `src/core/sync/folder.rs` | `FolderChannel` (implements `SyncChannel`), directory scan, file routing |
| `src/core/sync/manual.rs` | `ChannelType::Manual` marker, outbox path utilities |

### Modified Files (krillnotes-core)

| File | Change |
|------|--------|
| `src/core/error.rs:12-114` | Add `RelayAuthExpired`, `RelayRateLimited`, `RelayNotFound`, `RelayUnavailable` variants |
| `src/core/storage.rs:101-240` | Add migration for new `sync_peers` columns |
| `src/core/peer_registry.rs:20-32` | Extend `SyncPeer` struct with `channel_type`, `channel_params`, `sync_status`, `sync_status_detail`, `last_sync_error` |
| `src/core/peer_registry.rs:36-57` | Extend `PeerInfo` struct with same fields for frontend exposure |
| `src/core/peer_registry.rs:101-118` | Update `list_peers()` SQL to include new columns |
| `src/core/swarm/invite.rs:27-38` | Add `reply_channels` to `InviteParams` |
| `src/core/swarm/invite.rs:175-184` | Add `channel_preference` to `AcceptParams` |
| `src/core/swarm/mod.rs:9-15` | Add `pub mod sync_engine;` re-export (or add to lib.rs) |
| `src/lib.rs:17-42` | Re-export sync module |
| `Cargo.toml:11-41` | Add `reqwest`, `crypto_box` dependencies behind `relay` feature flag |

### Modified Files (krillnotes-desktop)

| File | Change |
|------|--------|
| `src-tauri/src/lib.rs:39-77` | Add `SyncEngine` map to `AppState`, add sync Tauri commands |
| `src/types.ts:237-246` | Extend `PeerInfo` with `channelType`, `syncStatus`, `syncStatusDetail` |
| `src/components/WorkspacePeersDialog.tsx` | Show channel config + sync status per peer |
| `src/components/CreateInviteDialog.tsx` | Add relay upload option ("Copy link" / "Save file" / both) |
| `src/components/ImportInviteDialog.tsx` | Add "Import from relay URL" field |

---

## Chunk 1: Foundation — Types, Errors, and Peer Registry Extension

### Task 1: Add Relay Error Variants to KrillnotesError

**Files:**
- Modify: `krillnotes-core/src/core/error.rs:12-114`

- [ ] **Step 1: Add relay error variants**

In `krillnotes-core/src/core/error.rs`, add before the closing `}` of `KrillnotesError` (after the `NotOwner` variant at line 113):

```rust
    /// Relay session has expired or token is invalid (HTTP 401).
    #[error("Relay auth expired: {0}")]
    RelayAuthExpired(String),

    /// Relay rate limit exceeded (HTTP 429).
    #[error("Relay rate limited: {0}")]
    RelayRateLimited(String),

    /// Relay resource not found or expired (HTTP 404/410).
    #[error("Relay not found: {0}")]
    RelayNotFound(String),

    /// Relay server unreachable or returned a server error.
    #[error("Relay unavailable: {0}")]
    RelayUnavailable(String),
```

**Note on error types:** These all take `String`, matching the existing `Swarm(String)` and `Crypto(String)` patterns. Do NOT use `Io(String)` — the existing `Io` variant uses `#[from] std::io::Error`. For IO-related relay/credential errors, wrap via `Swarm(format!("..."))` or these new relay variants.

- [ ] **Step 2: Add user_message() implementations**

In the `user_message()` match block (after `Self::NotOwner =>` at line 193), add:

```rust
Self::RelayAuthExpired(_) => "Relay session expired. Please log in again.".to_string(),
Self::RelayRateLimited(_) => "Relay is rate limiting requests. Please try again later.".to_string(),
Self::RelayNotFound(_) => "The requested relay resource was not found or has expired.".to_string(),
Self::RelayUnavailable(msg) => format!("Relay server unavailable: {msg}"),
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p krillnotes-core`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/error.rs
git commit -m "feat(sync): add relay error variants to KrillnotesError"
```

---

### Task 2: Add SyncChannel Trait and Type Definitions

**Files:**
- Create: `krillnotes-core/src/core/sync/mod.rs`
- Create: `krillnotes-core/src/core/sync/channel.rs`
- Create: `krillnotes-core/src/core/sync/manual.rs`
- Modify: `krillnotes-core/src/lib.rs:17-42`

- [ ] **Step 1: Create the sync module directory**

```bash
mkdir -p krillnotes-core/src/core/sync
```

- [ ] **Step 2: Write the channel types and trait in `channel.rs`**

Create `krillnotes-core/src/core/sync/channel.rs`:

```rust
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
    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError>;

    /// Channel type identifier.
    fn channel_type(&self) -> ChannelType;

    /// Downcast support for channel-specific operations (e.g., ensure_mailbox on relay).
    fn as_any(&self) -> &dyn std::any::Any;
}
```

- [ ] **Step 3: Write the manual marker module in `manual.rs`**

Create `krillnotes-core/src/core/sync/manual.rs`:

```rust
//! Manual channel marker.
//!
//! The manual channel does not implement `SyncChannel` — it is explicitly
//! excluded from the automated dispatch loop. `ChannelType::Manual` on a peer
//! tells `poll()` to skip that peer.
//!
//! Outbound: user clicks "Generate delta" in the peers dialog.
//! Inbound: user imports .swarm via SwarmOpenDialog.

use std::path::PathBuf;

/// Returns the default outbox directory for manual bundle export.
pub fn default_outbox_dir() -> Option<PathBuf> {
    dirs::download_dir().map(|d| d.join("KrillNotes"))
}
```

- [ ] **Step 4: Write the sync module root in `mod.rs`**

Create `krillnotes-core/src/core/sync/mod.rs`:

```rust
pub mod channel;
pub mod manual;

pub use channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};
```

- [ ] **Step 5: Wire the sync module into the crate**

In `krillnotes-core/src/lib.rs`, add the module declaration alongside the existing `pub mod core` structure. Find where the core submodules are declared and add:

```rust
pub mod sync;
```

inside the `core` module (or wherever submodules are declared — check the existing pattern).

Also add re-exports in the crate root as appropriate.

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p krillnotes-core`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/sync/
git add krillnotes-core/src/lib.rs
git commit -m "feat(sync): add SyncChannel trait, ChannelType enum, PeerSyncInfo"
```

---

### Task 3: Extend Peer Registry with Channel and Status Columns

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs:101-240`
- Modify: `krillnotes-core/src/core/peer_registry.rs:20-57`
- Modify: `krillnotes-core/src/core/peer_registry.rs:79-118`
- Modify: `krillnotes-core/src/core/peer_registry.rs:155-244`
- Test: `krillnotes-core/src/core/tests.rs` (existing test file)

- [ ] **Step 1: Write a failing test for the new peer registry fields**

In the existing test file, add a test that creates a workspace, adds a sync peer, sets channel_type to "relay", and reads it back:

```rust
#[test]
fn test_peer_registry_channel_fields() {
    let (mut ws, _dir) = create_test_workspace();
    let peer_id = "test-device-id";
    let identity_id = "test-identity-key";

    // Add peer with default channel (manual)
    ws.peer_registry().add_peer(peer_id, identity_id).unwrap();

    // Read back — should have default channel_type = manual
    let peer = ws.peer_registry().get_peer(peer_id).unwrap().unwrap();
    assert_eq!(peer.channel_type, "manual");
    assert_eq!(peer.channel_params, "{}");
    assert_eq!(peer.sync_status, "idle");
    assert!(peer.sync_status_detail.is_none());
    assert!(peer.last_sync_error.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_peer_registry_channel_fields`
Expected: FAIL — `SyncPeer` has no field `channel_type`

- [ ] **Step 3: Add migration in `storage.rs`**

In `storage.rs`, inside `run_migrations()`, add after the existing migrations (follow the idempotent `ALTER TABLE` pattern already used — check for column existence before adding):

```rust
// Sync engine: channel and status columns on sync_peers
let has_channel_type: bool = conn
    .query_row(
        "SELECT COUNT(*) FROM pragma_table_info('sync_peers') WHERE name = 'channel_type'",
        [],
        |row| row.get(0),
    )
    .unwrap_or(false);

if !has_channel_type {
    conn.execute_batch(
        "ALTER TABLE sync_peers ADD COLUMN channel_type TEXT NOT NULL DEFAULT 'manual';
         ALTER TABLE sync_peers ADD COLUMN channel_params TEXT NOT NULL DEFAULT '{}';
         ALTER TABLE sync_peers ADD COLUMN sync_status TEXT NOT NULL DEFAULT 'idle';
         ALTER TABLE sync_peers ADD COLUMN sync_status_detail TEXT;
         ALTER TABLE sync_peers ADD COLUMN last_sync_error TEXT;"
    )?;
}
```

- [ ] **Step 4: Extend `SyncPeer` struct in `peer_registry.rs`**

In `peer_registry.rs`, extend the `SyncPeer` struct (lines 20-32) to add the new fields:

```rust
pub struct SyncPeer {
    pub peer_device_id: String,
    pub peer_identity_id: String,
    pub last_sent_op: Option<String>,
    pub last_received_op: Option<String>,
    pub last_sync: Option<String>,
    // New sync engine fields
    pub channel_type: String,
    pub channel_params: String,
    pub sync_status: String,
    pub sync_status_detail: Option<String>,
    pub last_sync_error: Option<String>,
}
```

- [ ] **Step 5: Update all SQL queries that read SyncPeer**

Update `get_peer()` (lines 79-98), `list_peers()` (lines 101-118), and any other query that reads from `sync_peers` to include the new columns in their SELECT and row mapping.

Also update `PeerInfo` (lines 36-57) to include `channel_type`, `sync_status`, and `sync_status_detail` for frontend exposure.

- [ ] **Step 6: Add methods to update channel config and sync status**

Add to `PeerRegistry`:

```rust
pub fn update_channel_config(
    &self,
    peer_device_id: &str,
    channel_type: &str,
    channel_params: &str,
) -> Result<(), KrillnotesError> {
    self.conn.execute(
        "UPDATE sync_peers SET channel_type = ?1, channel_params = ?2 WHERE peer_device_id = ?3",
        params![channel_type, channel_params, peer_device_id],
    )?;
    Ok(())
}

pub fn update_sync_status(
    &self,
    peer_device_id: &str,
    sync_status: &str,
    sync_status_detail: Option<&str>,
    last_sync_error: Option<&str>,
) -> Result<(), KrillnotesError> {
    self.conn.execute(
        "UPDATE sync_peers SET sync_status = ?1, sync_status_detail = ?2, last_sync_error = ?3 WHERE peer_device_id = ?4",
        params![sync_status, sync_status_detail, last_sync_error, peer_device_id],
    )?;
    Ok(())
}
```

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_peer_registry_channel_fields`
Expected: PASS

- [ ] **Step 8: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: all existing tests still pass (migration is backward-compatible)

- [ ] **Step 9: Commit**

```bash
git add krillnotes-core/src/core/storage.rs
git add krillnotes-core/src/core/peer_registry.rs
git commit -m "feat(sync): extend sync_peers with channel_type, sync_status columns"
```

---

### Task 4: Add Workspace Sync Helper Methods

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs:408-519`

The `Workspace` already exposes `list_peers_info()` and peer management methods. Add convenience methods for the sync engine.

- [ ] **Step 1: Write a failing test for listing peers by channel type**

```rust
#[test]
fn test_list_peers_by_channel() {
    let (mut ws, _dir) = create_test_workspace();

    // Add two peers with different channels
    ws.add_contact_as_peer("relay-peer", "relay-identity").unwrap();
    ws.update_peer_channel("relay-peer", "relay", r#"{"relay_url":"https://example.com"}"#).unwrap();

    ws.add_contact_as_peer("manual-peer", "manual-identity").unwrap();

    let relay_peers = ws.list_peers_with_channel("relay").unwrap();
    assert_eq!(relay_peers.len(), 1);
    assert_eq!(relay_peers[0].peer_device_id, "relay-peer");

    let manual_peers = ws.list_peers_with_channel("manual").unwrap();
    assert_eq!(manual_peers.len(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_list_peers_by_channel`
Expected: FAIL — `update_peer_channel` and `list_peers_with_channel` don't exist

- [ ] **Step 3: Implement the workspace methods**

In `krillnotes-core/src/core/workspace/sync.rs`, add:

```rust
/// Update a peer's channel configuration.
pub fn update_peer_channel(
    &self,
    peer_device_id: &str,
    channel_type: &str,
    channel_params: &str,
) -> Result<(), KrillnotesError> {
    self.peer_registry().update_channel_config(peer_device_id, channel_type, channel_params)
}

/// Update a peer's sync status.
pub fn update_peer_sync_status(
    &self,
    peer_device_id: &str,
    sync_status: &str,
    detail: Option<&str>,
    error: Option<&str>,
) -> Result<(), KrillnotesError> {
    self.peer_registry().update_sync_status(peer_device_id, sync_status, detail, error)
}

/// List peers filtered by channel type.
pub fn list_peers_with_channel(&self, channel_type: &str) -> Result<Vec<SyncPeer>, KrillnotesError> {
    self.peer_registry().list_peers_by_channel(channel_type)
}

/// Get PeerSyncInfo for all non-manual peers (used by SyncEngine).
pub fn get_active_sync_peers(&self) -> Result<Vec<PeerSyncInfo>, KrillnotesError> {
    let peers = self.peer_registry().list_peers_by_channel_not("manual")?;
    Ok(peers.into_iter().map(|p| PeerSyncInfo {
        peer_device_id: p.peer_device_id,
        peer_identity_id: p.peer_identity_id,
        channel_type: serde_json::from_str(&format!("\"{}\"", p.channel_type))
            .unwrap_or(ChannelType::Manual),
        channel_params: serde_json::from_str(&p.channel_params)
            .unwrap_or(serde_json::Value::Object(Default::default())),
        last_sent_op: p.last_sent_op,
        last_received_op: p.last_received_op,
    }).collect())
}
```

Also add `list_peers_by_channel()` and `list_peers_by_channel_not()` to `PeerRegistry`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_list_peers_by_channel`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs
git add krillnotes-core/src/core/peer_registry.rs
git commit -m "feat(sync): add workspace helpers for channel config and peer queries"
```

---

## Chunk 2: Relay Infrastructure — Credentials, HTTP Client, Auth

### Task 5: Add reqwest and crypto_box Dependencies

**Files:**
- Modify: `krillnotes-core/Cargo.toml:11-41`

- [ ] **Step 1: Add feature flag and dependencies**

In `krillnotes-core/Cargo.toml`, add:

```toml
[features]
default = ["relay"]
relay = ["dep:reqwest", "dep:crypto_box"]

[dependencies]
# ... existing deps ...
reqwest = { version = "0.12", features = ["blocking", "json"], optional = true }
crypto_box = { version = "0.9", optional = true }
```

- [ ] **Step 2: Verify compilation with feature**

Run: `cargo check -p krillnotes-core --features relay`
Expected: compiles (downloads new deps)

- [ ] **Step 3: Verify compilation without feature**

Run: `cargo check -p krillnotes-core --no-default-features`
Expected: compiles without relay code

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/Cargo.toml
git commit -m "feat(sync): add reqwest and crypto_box deps behind relay feature flag"
```

---

### Task 6: Implement Relay Credential Storage

**Files:**
- Create: `krillnotes-core/src/core/sync/relay/mod.rs`
- Create: `krillnotes-core/src/core/sync/relay/auth.rs`
- Modify: `krillnotes-core/src/core/sync/mod.rs`

- [ ] **Step 1: Write a failing test for credential round-trip**

In `relay/auth.rs`, add a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_credentials_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let relay_dir = dir.path().join("relay");

        let identity_uuid = "test-identity-uuid";
        let encryption_key = [0x42u8; 32];

        let creds = RelayCredentials {
            relay_url: "https://relay.example.com".to_string(),
            email: "test@example.com".to_string(),
            session_token: "tok_abc123".to_string(),
            session_expires_at: chrono::Utc::now() + chrono::Duration::days(30),
            device_public_key: "deadbeef".to_string(),
        };

        save_relay_credentials(&relay_dir, identity_uuid, &creds, &encryption_key).unwrap();
        let loaded = load_relay_credentials(&relay_dir, identity_uuid, &encryption_key).unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.relay_url, creds.relay_url);
        assert_eq!(loaded.email, creds.email);
        assert_eq!(loaded.session_token, creds.session_token);
        assert_eq!(loaded.device_public_key, creds.device_public_key);
    }

    #[test]
    fn test_relay_credentials_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let relay_dir = dir.path().join("relay");
        let encryption_key = [0x42u8; 32];

        let loaded = load_relay_credentials(&relay_dir, "nonexistent", &encryption_key).unwrap();
        assert!(loaded.is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_relay_credentials`
Expected: FAIL — `RelayCredentials`, `save_relay_credentials`, `load_relay_credentials` don't exist

- [ ] **Step 3: Implement RelayCredentials and file I/O in `auth.rs`**

Create `krillnotes-core/src/core/sync/relay/auth.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use crate::core::error::KrillnotesError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayCredentials {
    pub relay_url: String,
    pub email: String,
    pub session_token: String,
    pub session_expires_at: DateTime<Utc>,
    pub device_public_key: String,
}

/// Encrypted envelope stored on disk.
#[derive(Serialize, Deserialize)]
struct EncryptedRelayFile {
    nonce: String,   // base64
    ciphertext: String, // base64
}

/// Save relay credentials encrypted with the identity's HKDF-derived key.
/// Mirrors the ContactManager encryption pattern.
pub fn save_relay_credentials(
    relay_dir: &Path,
    identity_uuid: &str,
    creds: &RelayCredentials,
    encryption_key: &[u8; 32],
) -> Result<(), KrillnotesError> {
    std::fs::create_dir_all(relay_dir).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to create relay dir: {}", e))
    })?;

    let plaintext = serde_json::to_vec(creds).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to serialize credentials: {}", e))
    })?;

    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    use aes_gcm::aead::OsRng;
    use aes_gcm::aead::rand_core::RngCore;

    let cipher = Aes256Gcm::new_from_slice(encryption_key)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid key: {}", e)))?;

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, plaintext.as_ref())
        .map_err(|e| KrillnotesError::Crypto(format!("Encryption failed: {}", e)))?;

    let envelope = EncryptedRelayFile {
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    };

    let path = relay_dir.join(format!("{}.json", identity_uuid));
    let json = serde_json::to_string_pretty(&envelope).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to serialize envelope: {}", e))
    })?;
    std::fs::write(&path, json).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to write relay credentials: {}", e))
    })?;

    Ok(())
}

/// Load and decrypt relay credentials for an identity. Returns None if no file exists.
pub fn load_relay_credentials(
    relay_dir: &Path,
    identity_uuid: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<RelayCredentials>, KrillnotesError> {
    let path = relay_dir.join(format!("{}.json", identity_uuid));
    if !path.exists() {
        return Ok(None);
    }

    let json = std::fs::read_to_string(&path).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to read relay credentials: {}", e))
    })?;
    let envelope: EncryptedRelayFile = serde_json::from_str(&json).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to parse relay file: {}", e))
    })?;

    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    use base64::Engine;

    let nonce_bytes = base64::engine::general_purpose::STANDARD.decode(&envelope.nonce)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid nonce base64: {}", e)))?;
    let ciphertext = base64::engine::general_purpose::STANDARD.decode(&envelope.ciphertext)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid ciphertext base64: {}", e)))?;

    let cipher = Aes256Gcm::new_from_slice(encryption_key)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid key: {}", e)))?;
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext.as_ref())
        .map_err(|e| KrillnotesError::Crypto(format!("Decryption failed: {}", e)))?;

    let creds: RelayCredentials = serde_json::from_slice(&plaintext).map_err(|e| {
        KrillnotesError::Swarm(format!("Failed to parse credentials: {}", e))
    })?;

    Ok(Some(creds))
}

/// Delete relay credentials for an identity.
pub fn delete_relay_credentials(
    relay_dir: &Path,
    identity_uuid: &str,
) -> Result<(), KrillnotesError> {
    let path = relay_dir.join(format!("{}.json", identity_uuid));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| {
            KrillnotesError::Swarm(format!("Failed to delete relay credentials: {}", e))
        })?;
    }
    Ok(())
}
```

- [ ] **Step 4: Create the relay module root**

Create `krillnotes-core/src/core/sync/relay/mod.rs`:

```rust
pub mod auth;
#[cfg(feature = "relay")]
pub mod client;

pub use auth::{RelayCredentials, save_relay_credentials, load_relay_credentials, delete_relay_credentials};
```

- [ ] **Step 5: Update sync/mod.rs to include relay module**

```rust
pub mod channel;
pub mod manual;
#[cfg(feature = "relay")]
pub mod relay;

pub use channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p krillnotes-core test_relay_credentials`
Expected: PASS (both tests)

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/sync/relay/
git add krillnotes-core/src/core/sync/mod.rs
git commit -m "feat(sync): add encrypted relay credential storage"
```

---

### Task 7: Implement Relay HTTP Client

**Files:**
- Create: `krillnotes-core/src/core/sync/relay/client.rs`

This task implements the `RelayClient` struct — a thin reqwest wrapper over the relay REST API. All methods are gated behind `#[cfg(feature = "relay")]`.

- [ ] **Step 1: Write a test for RelayClient construction**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_client_construction() {
        let client = RelayClient::new("https://relay.example.com");
        assert_eq!(client.base_url, "https://relay.example.com");
        assert!(client.session_token.is_none());
    }

    #[test]
    fn test_relay_client_with_token() {
        let client = RelayClient::new("https://relay.example.com")
            .with_session_token("tok_abc123");
        assert_eq!(client.session_token.as_deref(), Some("tok_abc123"));
    }
}
```

- [ ] **Step 2: Implement `RelayClient` struct with all API methods**

Create `krillnotes-core/src/core/sync/relay/client.rs`. This is a large file, so implement it method group by method group:

```rust
use crate::core::error::KrillnotesError;
use serde::{Deserialize, Serialize};

pub struct RelayClient {
    http: reqwest::blocking::Client,
    pub base_url: String,
    pub session_token: Option<String>,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
pub struct RegisterChallenge {
    pub encrypted_nonce: String,
    pub server_public_key: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionResponse {
    pub session_token: String,
}

#[derive(Debug, Deserialize)]
pub struct AccountInfo {
    pub account_id: String,
    pub email: String,
    pub identity_uuid: String,
    pub device_keys: Vec<String>,
    pub role: String,
    pub storage_used: u64,
}

#[derive(Debug, Deserialize)]
pub struct MailboxInfo {
    pub workspace_id: String,
    pub pending_bundles: u32,
    pub storage_used: u64,
}

#[derive(Debug, Deserialize)]
pub struct BundleMeta {
    pub bundle_id: String,
    pub workspace_id: String,
    pub sender_device_key: String,
    pub size: u64,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct InviteInfo {
    pub invite_id: String,
    pub token: String,
    pub url: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
pub struct InvitePayload {
    pub payload: String,  // base64
    pub expires_at: String,
}

// Wrapper for relay JSON responses: { "data": T }
#[derive(Debug, Deserialize)]
struct RelayResponse<T> {
    data: T,
}

impl RelayClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::blocking::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            session_token: None,
        }
    }

    pub fn with_session_token(mut self, token: &str) -> Self {
        self.session_token = Some(token.to_string());
        self
    }

    pub fn set_session_token(&mut self, token: &str) {
        self.session_token = Some(token.to_string());
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth_header(&self) -> Result<String, KrillnotesError> {
        self.session_token.as_ref()
            .map(|t| format!("Bearer {}", t))
            .ok_or_else(|| KrillnotesError::RelayAuthExpired("No session token".to_string()))
    }

    fn map_error(resp: reqwest::blocking::Response) -> KrillnotesError {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        match status.as_u16() {
            401 => KrillnotesError::RelayAuthExpired(body),
            429 => KrillnotesError::RelayRateLimited(body),
            404 | 410 => KrillnotesError::RelayNotFound(body),
            _ => KrillnotesError::RelayUnavailable(format!("HTTP {}: {}", status, body)),
        }
    }

    fn handle_response<T: serde::de::DeserializeOwned>(
        resp: reqwest::blocking::Response,
    ) -> Result<T, KrillnotesError> {
        if resp.status().is_success() {
            let wrapper: RelayResponse<T> = resp.json().map_err(|e| {
                KrillnotesError::RelayUnavailable(format!("Invalid JSON: {}", e))
            })?;
            Ok(wrapper.data)
        } else {
            Err(Self::map_error(resp))
        }
    }

    // --- Auth ---

    pub fn register(
        &self,
        email: &str,
        password: &str,
        identity_uuid: &str,
        device_public_key: &str,
    ) -> Result<RegisterChallenge, KrillnotesError> {
        let resp = self.http.post(self.url("/auth/register"))
            .json(&serde_json::json!({
                "email": email,
                "password": password,
                "identity_uuid": identity_uuid,
                "device_public_key": device_public_key,
            }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn register_verify(
        &self,
        device_public_key: &str,
        nonce: &str,
    ) -> Result<SessionResponse, KrillnotesError> {
        let resp = self.http.post(self.url("/auth/register/verify"))
            .json(&serde_json::json!({
                "device_public_key": device_public_key,
                "nonce": nonce,
            }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<SessionResponse, KrillnotesError> {
        let resp = self.http.post(self.url("/auth/login"))
            .json(&serde_json::json!({
                "email": email,
                "password": password,
            }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn logout(&self) -> Result<(), KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.post(self.url("/auth/logout"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }

    pub fn reset_password(&self, email: &str) -> Result<(), KrillnotesError> {
        let resp = self.http.post(self.url("/auth/reset-password"))
            .json(&serde_json::json!({ "email": email }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }

    pub fn reset_password_confirm(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<(), KrillnotesError> {
        let resp = self.http.post(self.url("/auth/reset-password/confirm"))
            .json(&serde_json::json!({
                "token": token,
                "new_password": new_password,
            }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }

    // --- Account & Devices ---

    pub fn get_account(&self) -> Result<AccountInfo, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.get(self.url("/account"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn add_device(
        &self,
        device_public_key: &str,
    ) -> Result<RegisterChallenge, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.post(self.url("/account/devices"))
            .header("Authorization", auth)
            .json(&serde_json::json!({ "device_public_key": device_public_key }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn verify_device(
        &self,
        device_public_key: &str,
        nonce: &str,
    ) -> Result<(), KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.post(self.url("/account/devices/verify"))
            .header("Authorization", auth)
            .json(&serde_json::json!({
                "device_public_key": device_public_key,
                "nonce": nonce,
            }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }

    // --- Mailboxes ---

    pub fn ensure_mailbox(&self, workspace_id: &str) -> Result<(), KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.post(self.url("/mailboxes"))
            .header("Authorization", auth)
            .json(&serde_json::json!({ "workspace_id": workspace_id }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        // 201 Created or 200 OK (already exists) are both success
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }

    pub fn list_mailboxes(&self) -> Result<Vec<MailboxInfo>, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.get(self.url("/mailboxes"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    // --- Bundles ---

    pub fn upload_bundle(&self, bundle_bytes: &[u8]) -> Result<String, KrillnotesError> {
        let auth = self.auth_header()?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bundle_bytes);
        let resp = self.http.post(self.url("/bundles"))
            .header("Authorization", auth)
            .json(&serde_json::json!({ "payload": encoded }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;

        #[derive(Deserialize)]
        struct BundleCreated { bundle_id: String }
        let created: BundleCreated = Self::handle_response(resp)?;
        Ok(created.bundle_id)
    }

    pub fn list_bundles(&self) -> Result<Vec<BundleMeta>, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.get(self.url("/bundles"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn download_bundle(&self, bundle_id: &str) -> Result<Vec<u8>, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.get(self.url(&format!("/bundles/{}", bundle_id)))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;

        if resp.status().is_success() {
            #[derive(Deserialize)]
            struct BundlePayload { payload: String }
            let wrapper: RelayResponse<BundlePayload> = resp.json().map_err(|e| {
                KrillnotesError::RelayUnavailable(format!("Invalid JSON: {}", e))
            })?;
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(&wrapper.data.payload)
                .map_err(|e| KrillnotesError::RelayUnavailable(format!("Invalid base64: {}", e)))
        } else {
            Err(Self::map_error(resp))
        }
    }

    pub fn delete_bundle(&self, bundle_id: &str) -> Result<(), KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.delete(self.url(&format!("/bundles/{}", bundle_id)))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }

    // --- Invites ---

    pub fn create_invite(
        &self,
        payload_base64: &str,
        expires_at: &str,
    ) -> Result<InviteInfo, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.post(self.url("/invites"))
            .header("Authorization", auth)
            .json(&serde_json::json!({
                "payload": payload_base64,
                "expires_at": expires_at,
            }))
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn list_invites(&self) -> Result<Vec<InviteInfo>, KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.get(self.url("/invites"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn fetch_invite(&self, token: &str) -> Result<InvitePayload, KrillnotesError> {
        let resp = self.http.get(self.url(&format!("/invites/{}", token)))
            .header("Accept", "application/json")
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        Self::handle_response(resp)
    }

    pub fn delete_invite(&self, token: &str) -> Result<(), KrillnotesError> {
        let auth = self.auth_header()?;
        let resp = self.http.delete(self.url(&format!("/invites/{}", token)))
            .header("Authorization", auth)
            .send()
            .map_err(|e| KrillnotesError::RelayUnavailable(e.to_string()))?;
        if resp.status().is_success() { Ok(()) } else { Err(Self::map_error(resp)) }
    }
}
```

- [ ] **Step 3: Run unit tests**

Run: `cargo test -p krillnotes-core test_relay_client`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/sync/relay/client.rs
git commit -m "feat(sync): implement RelayClient HTTP wrapper for all relay API endpoints"
```

---

### Task 8: Implement Proof-of-Possession Challenge Resolution

**Files:**
- Modify: `krillnotes-core/src/core/sync/relay/auth.rs`

The relay's PoP uses NaCl `crypto_box` (X25519 + XSalsa20-Poly1305). We reuse the existing Ed25519→X25519 conversion from `swarm/crypto.rs` but need the `crypto_box` crate for the actual decryption.

**Prerequisite:** `ed25519_sk_to_x25519()` in `swarm/crypto.rs:38` is currently private (`fn`). Change it to `pub(crate) fn` before implementing this task. The function returns `x25519_dalek::StaticSecret` — use `.to_bytes()` to get raw bytes for `crypto_box::SecretKey::from()`.

- [ ] **Step 1: Write a test for PoP nonce decryption**

```rust
#[test]
fn test_pop_challenge_decrypt() {
    // Simulate what the relay server does:
    // 1. Generate ephemeral X25519 keypair
    // 2. Encrypt a nonce to the client's X25519 public key
    // 3. Client decrypts with their X25519 secret key

    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    // Client generates Ed25519 keypair
    let client_signing_key = SigningKey::generate(&mut OsRng);
    let client_verifying_key = client_signing_key.verifying_key();

    // Server side: generate ephemeral keypair, encrypt nonce
    let nonce_plaintext = b"test-challenge-nonce-1234567890ab";
    let (encrypted_nonce, server_public_key) =
        simulate_server_challenge(&client_verifying_key, nonce_plaintext);

    // Client side: decrypt
    let decrypted = decrypt_pop_challenge(
        &client_signing_key,
        &encrypted_nonce,
        &server_public_key,
    ).unwrap();

    assert_eq!(decrypted, nonce_plaintext);
}
```

- [ ] **Step 2: Implement `decrypt_pop_challenge` and test helper**

Add to `auth.rs`:

```rust
#[cfg(feature = "relay")]
pub fn decrypt_pop_challenge(
    client_signing_key: &ed25519_dalek::SigningKey,
    encrypted_nonce_hex: &str,
    server_public_key_hex: &str,
) -> Result<Vec<u8>, KrillnotesError> {
    use crate::core::swarm::crypto::ed25519_sk_to_x25519;

    // Convert Ed25519 signing key to X25519 secret key
    let client_x25519_sk = ed25519_sk_to_x25519(client_signing_key);

    // Decode server's ephemeral public key
    let server_pk_bytes = hex::decode(server_public_key_hex)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid server pubkey hex: {}", e)))?;
    let server_pk: crypto_box::PublicKey = crypto_box::PublicKey::from_slice(&server_pk_bytes)
        .map_err(|_| KrillnotesError::Crypto("Invalid server public key".to_string()))?;

    // Build crypto_box with client's X25519 secret key and server's public key
    let client_sk = crypto_box::SecretKey::from_slice(&client_x25519_sk)
        .map_err(|_| KrillnotesError::Crypto("Invalid client secret key".to_string()))?;
    let the_box = crypto_box::SalsaBox::new(&server_pk, &client_sk);

    // Decode encrypted nonce (nonce prefix + ciphertext)
    let encrypted_bytes = hex::decode(encrypted_nonce_hex)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid encrypted nonce hex: {}", e)))?;

    // crypto_box nonce is 24 bytes, prepended to ciphertext
    if encrypted_bytes.len() < 24 {
        return Err(KrillnotesError::Crypto("Encrypted nonce too short".to_string()));
    }
    let (nonce_bytes, ciphertext) = encrypted_bytes.split_at(24);
    let nonce = crypto_box::Nonce::from_slice(nonce_bytes);

    use crypto_box::aead::Aead;
    let plaintext = the_box.decrypt(nonce, ciphertext)
        .map_err(|_| KrillnotesError::Crypto("PoP challenge decryption failed".to_string()))?;

    Ok(plaintext)
}
```

Note: The exact format (hex-encoded nonce+ciphertext, 24-byte nonce prefix) must match what the PHP relay's `ext-sodium crypto_box` produces. Check the relay source for the exact encoding. Adjust if the relay uses base64 instead of hex.

- [ ] **Step 3: Run test**

Run: `cargo test -p krillnotes-core test_pop_challenge_decrypt`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/sync/relay/auth.rs
git commit -m "feat(sync): implement PoP challenge decryption using crypto_box"
```

---

## Chunk 3: Channel Implementations

### Task 9: Implement FolderChannel

**Files:**
- Create: `krillnotes-core/src/core/sync/folder.rs`
- Modify: `krillnotes-core/src/core/sync/mod.rs`

- [ ] **Step 1: Write failing tests for FolderChannel**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sync::channel::{SyncChannel, PeerSyncInfo, ChannelType};

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
        let channel = FolderChannel::new(
            "my-identity".to_string(),
            "my-device".to_string(),
        );
        let peer = make_test_peer("peer-dev", "peer-id", dir.path().to_str().unwrap());

        channel.send_bundle(&peer, b"test bundle data").unwrap();

        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "swarm"))
            .collect();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_folder_channel_receive_filters_own_bundles() {
        let dir = tempfile::tempdir().unwrap();
        let channel = FolderChannel::new(
            "my-identity".to_string(),
            "my-device".to_string(),
        );

        // Write a bundle from "our" identity+device — should be filtered out
        let own_file = dir.path().join("my-iden_my-devi_20260314_test.swarm");
        std::fs::write(&own_file, b"own bundle").unwrap();

        // Write a bundle from a different identity — should be picked up
        // (This is a simplified test — real bundles have headers. The actual
        //  implementation reads SwarmHeader from the zip. For this test, we
        //  rely on filename-based filtering as a fast path.)
        let peer_file = dir.path().join("other-i_other-d_20260314_test.swarm");
        std::fs::write(&peer_file, b"peer bundle").unwrap();

        let bundles = channel.receive_bundles_from_dir(dir.path()).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].data, b"peer bundle");
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
        let result = channel.receive_bundles_from_dir(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_folder_channel`
Expected: FAIL — `FolderChannel` doesn't exist

- [ ] **Step 3: Implement `FolderChannel`**

Create `krillnotes-core/src/core/sync/folder.rs`:

```rust
use std::path::Path;
use chrono::Utc;
use uuid::Uuid;
use crate::core::error::KrillnotesError;
use crate::core::sync::channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};

pub struct FolderChannel {
    /// Short prefix of local identity UUID for filename generation
    identity_short: String,
    /// Short prefix of local device key for filename generation
    device_short: String,
    /// All unique folder paths configured on peers using this channel.
    /// Updated by the SyncEngine before each poll cycle.
    folder_paths: std::sync::Mutex<Vec<String>>,
}

impl FolderChannel {
    pub fn new(identity_id: String, device_id: String) -> Self {
        Self {
            identity_short: identity_id.chars().take(8).collect(),
            device_short: device_id.chars().take(8).collect(),
            folder_paths: std::sync::Mutex::new(vec![]),
        }
    }

    /// Update the set of folder paths to scan. Called by SyncEngine
    /// before each poll cycle with paths from all folder-channel peers.
    pub fn set_folder_paths(&self, paths: Vec<String>) {
        *self.folder_paths.lock().unwrap() = paths;
    }

    fn extract_folder_path(peer: &PeerSyncInfo) -> Result<&str, KrillnotesError> {
        peer.channel_params.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| KrillnotesError::Swarm(
                "Folder channel peer missing 'path' in channel_params".to_string()
            ))
    }

    /// Internal method for receiving bundles from a specific directory.
    pub fn receive_bundles_from_dir(&self, dir: &Path) -> Result<Vec<BundleRef>, KrillnotesError> {
        if !dir.exists() {
            return Err(KrillnotesError::Swarm(format!("Folder not found: {}", dir.display())));
        }

        let own_prefix = format!("{}_{}",  self.identity_short, self.device_short);
        let mut bundles = Vec::new();

        let entries = std::fs::read_dir(dir).map_err(|e| {
            KrillnotesError::Swarm(format!("Cannot read folder {}: {}", dir.display(), e))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "swarm") {
                continue;
            }

            // Filename-based fast filter: skip files we wrote ourselves
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if filename.starts_with(&own_prefix) {
                continue;
            }

            // Try to read the file; skip if it fails (partially written)
            match std::fs::read(&path) {
                Ok(data) => {
                    bundles.push(BundleRef {
                        id: path.to_string_lossy().to_string(),
                        data,
                    });
                }
                Err(_) => continue, // Skip partially written files
            }
        }

        Ok(bundles)
    }
}

impl SyncChannel for FolderChannel {
    fn send_bundle(&self, peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<(), KrillnotesError> {
        let folder_path = Self::extract_folder_path(peer)?;
        let dir = Path::new(folder_path);

        if !dir.exists() {
            return Err(KrillnotesError::Swarm(format!("Folder not found: {}", dir.display())));
        }

        let timestamp = Utc::now().format("%Y%m%d%H%M%S");
        let uuid_short = &Uuid::new_v4().to_string()[..8];
        let filename = format!("{}_{}_{}_{}.swarm",
            self.identity_short, self.device_short, timestamp, uuid_short
        );

        let path = dir.join(filename);
        std::fs::write(&path, bundle_bytes).map_err(|e| {
            KrillnotesError::Swarm(format!("Failed to write bundle to {}: {}", path.display(), e))
        })?;

        Ok(())
    }

    fn receive_bundles(&self, _workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError> {
        let paths = self.folder_paths.lock().unwrap().clone();
        let mut all_bundles = Vec::new();
        for path in &paths {
            match self.receive_bundles_from_dir(Path::new(path)) {
                Ok(bundles) => all_bundles.extend(bundles),
                Err(_) => continue, // Skip inaccessible folders, try others
            }
        }
        Ok(all_bundles)
    }

    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError> {
        let path = Path::new(&bundle_ref.id);
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| {
                KrillnotesError::Swarm(format!("Failed to delete {}: {}", path.display(), e))
            })?;
        }
        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Folder
    }
}
```

- [ ] **Step 4: Update `sync/mod.rs`**

```rust
pub mod channel;
pub mod folder;
pub mod manual;
#[cfg(feature = "relay")]
pub mod relay;

pub use channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};
pub use folder::FolderChannel;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p krillnotes-core test_folder_channel`
Expected: PASS (all 4 tests)

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/sync/folder.rs
git add krillnotes-core/src/core/sync/mod.rs
git commit -m "feat(sync): implement FolderChannel with filename-based routing"
```

---

### Task 10: Implement RelayChannel

**Files:**
- Modify: `krillnotes-core/src/core/sync/relay/mod.rs`

- [ ] **Step 1: Write tests for RelayChannel trait implementation**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests against the live relay are in a separate
    // test file. These unit tests verify the channel's contract with mock data.

    #[test]
    fn test_relay_channel_construction() {
        let client = RelayClient::new("https://relay.example.com")
            .with_session_token("tok_test");
        let channel = RelayChannel::new(client);
        assert_eq!(channel.channel_type(), ChannelType::Relay);
    }
}
```

- [ ] **Step 2: Implement `RelayChannel`**

In `krillnotes-core/src/core/sync/relay/mod.rs`:

```rust
pub mod auth;
#[cfg(feature = "relay")]
pub mod client;

pub use auth::{RelayCredentials, save_relay_credentials, load_relay_credentials, delete_relay_credentials};

#[cfg(feature = "relay")]
pub use client::RelayClient;

#[cfg(feature = "relay")]
use crate::core::error::KrillnotesError;
#[cfg(feature = "relay")]
use crate::core::sync::channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};

#[cfg(feature = "relay")]
pub struct RelayChannel {
    client: RelayClient,
}

#[cfg(feature = "relay")]
impl RelayChannel {
    pub fn new(client: RelayClient) -> Self {
        Self { client }
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
    fn send_bundle(&self, _peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<(), KrillnotesError> {
        self.client.upload_bundle(bundle_bytes)?;
        Ok(())
    }

    fn receive_bundles(&self, _workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError> {
        let metas = self.client.list_bundles()?;
        let mut bundles = Vec::new();
        for meta in metas {
            match self.client.download_bundle(&meta.bundle_id) {
                Ok(data) => bundles.push(BundleRef {
                    id: meta.bundle_id,
                    data,
                }),
                Err(e) => {
                    // Log but continue — don't fail the whole receive for one bad bundle
                    eprintln!("Failed to download bundle {}: {}", meta.bundle_id, e);
                }
            }
        }
        Ok(bundles)
    }

    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError> {
        self.client.delete_bundle(&bundle_ref.id)
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Relay
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p krillnotes-core --features relay`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/sync/relay/mod.rs
git commit -m "feat(sync): implement RelayChannel wrapping RelayClient"
```

---

## Chunk 4: Sync Engine — Dispatch Loop and Events

### Task 11: Implement SyncEngine and SyncEvent

**Files:**
- Modify: `krillnotes-core/src/core/sync/mod.rs`

- [ ] **Step 1: Write a failing test for the sync engine poll loop**

This test uses a mock channel to verify the dispatch logic without a real relay or folder.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct MockChannel {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        inbound: Arc<Mutex<Vec<BundleRef>>>,
        acknowledged: Arc<Mutex<Vec<String>>>,
    }

    impl MockChannel {
        fn new() -> Self {
            Self {
                sent: Arc::new(Mutex::new(vec![])),
                inbound: Arc::new(Mutex::new(vec![])),
                acknowledged: Arc::new(Mutex::new(vec![])),
            }
        }
    }

    impl SyncChannel for MockChannel {
        fn send_bundle(&self, _peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<(), KrillnotesError> {
            self.sent.lock().unwrap().push(bundle_bytes.to_vec());
            Ok(())
        }

        fn receive_bundles(&self, _workspace_id: &str) -> Result<Vec<BundleRef>, KrillnotesError> {
            Ok(self.inbound.lock().unwrap().drain(..).collect())
        }

        fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError> {
            self.acknowledged.lock().unwrap().push(bundle_ref.id.clone());
            Ok(())
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Relay
        }
    }

    #[test]
    fn test_sync_engine_skips_manual_peers() {
        // Verify that poll() does not generate deltas for manual-channel peers.
        // This is a structural test — the real logic test requires a workspace
        // with operations, which is tested in integration tests.
        let engine = SyncEngine::new();
        // Engine with no channels registered should return Ok with empty events
        // when there are no non-manual peers.
        // Full test requires workspace setup — see integration tests.
    }
}
```

- [ ] **Step 2: Implement SyncEvent enum and SyncEngine struct**

In `krillnotes-core/src/core/sync/mod.rs`, expand:

```rust
pub mod channel;
pub mod folder;
pub mod manual;
#[cfg(feature = "relay")]
pub mod relay;

pub use channel::{BundleRef, ChannelType, PeerSyncInfo, SyncChannel};
pub use folder::FolderChannel;

use std::collections::HashMap;
use crate::core::error::KrillnotesError;
use crate::core::workspace::Workspace;
use crate::core::contact::ContactManager;
use ed25519_dalek::SigningKey;

/// Events emitted by the sync engine during poll().
/// All events carry workspace_id for host-app routing.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    DeltaSent { workspace_id: String, peer_device_id: String, op_count: usize },
    BundleApplied { workspace_id: String, peer_device_id: String, op_count: usize },
    AuthExpired { relay_url: String },
    SyncError { workspace_id: String, peer_device_id: String, error: String },
    IngestError { workspace_id: String, peer_device_id: String, error: String },
    UnexpectedBundleMode { workspace_id: String, mode: String },
}

/// Context needed for sync operations.
pub struct SyncContext<'a> {
    pub signing_key: &'a SigningKey,
    pub contact_manager: &'a mut ContactManager,
    pub workspace_name: &'a str,
    pub sender_display_name: &'a str,
}

pub type SyncEventCallback = Box<dyn Fn(SyncEvent) + Send + Sync>;

pub struct SyncEngine {
    channels: HashMap<ChannelType, Box<dyn SyncChannel>>,
}

impl SyncEngine {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    pub fn register_channel(&mut self, channel: Box<dyn SyncChannel>) {
        let ct = channel.channel_type();
        self.channels.insert(ct, channel);
    }

    pub fn poll(
        &self,
        workspace: &mut Workspace,
        ctx: &mut SyncContext<'_>,
    ) -> Result<Vec<SyncEvent>, KrillnotesError> {
        let workspace_id = workspace.workspace_id().to_string();
        let mut events = Vec::new();

        // --- Ensure relay mailbox exists (idempotent) ---
        if let Some(relay_ch) = self.channels.get(&ChannelType::Relay) {
            // Downcast to RelayChannel to call ensure_mailbox.
            // This is safe because we registered it as ChannelType::Relay.
            if let Some(relay) = relay_ch.as_any().downcast_ref::<RelayChannel>() {
                let _ = relay.client().ensure_mailbox(&workspace_id);
            }
        }

        // --- Outbound: generate and send deltas ---
        let peers = workspace.get_active_sync_peers()?;
        for peer in &peers {
            if peer.channel_type == ChannelType::Manual {
                continue;
            }

            let channel = match self.channels.get(&peer.channel_type) {
                Some(ch) => ch,
                None => {
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        error: format!("No channel registered for type {:?}", peer.channel_type),
                    });
                    continue;
                }
            };

            workspace.update_peer_sync_status(
                &peer.peer_device_id, "syncing", None, None
            )?;

            // generate_delta checks last_sent_op internally and returns
            // Err if snapshot must precede delta (last_sent_op is None).
            // Actual signature (swarm/sync.rs:48-55):
            //   generate_delta(workspace, peer_device_id, workspace_name,
            //                  signing_key, sender_display_name, contact_manager)
            match crate::core::swarm::sync::generate_delta(
                workspace,
                &peer.peer_device_id,
                ctx.workspace_name,
                ctx.signing_key,
                ctx.sender_display_name,
                ctx.contact_manager,
            ) {
                Ok(bundle_bytes) => {
                    match channel.send_bundle(peer, &bundle_bytes) {
                        Ok(()) => {
                            // generate_delta already updates last_sent_op internally
                            workspace.update_peer_sync_status(
                                &peer.peer_device_id, "idle", None, None
                            )?;
                            events.push(SyncEvent::DeltaSent {
                                workspace_id: workspace_id.clone(),
                                peer_device_id: peer.peer_device_id.clone(),
                                op_count: 0, // exact count not returned by generate_delta
                            });
                        }
                        Err(KrillnotesError::RelayAuthExpired(msg)) => {
                            workspace.update_peer_sync_status(
                                &peer.peer_device_id, "auth_expired", Some(&msg), Some(&msg)
                            )?;
                            events.push(SyncEvent::AuthExpired {
                                relay_url: peer.channel_params
                                    .get("relay_url")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string(),
                            });
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            workspace.update_peer_sync_status(
                                &peer.peer_device_id, "error", Some(&msg), Some(&msg)
                            )?;
                            events.push(SyncEvent::SyncError {
                                workspace_id: workspace_id.clone(),
                                peer_device_id: peer.peer_device_id.clone(),
                                error: msg,
                            });
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    // "snapshot must precede delta" means last_sent_op is None —
                    // this peer needs a snapshot first, not a delta. Not an error per se.
                    workspace.update_peer_sync_status(
                        &peer.peer_device_id, "error", Some(&msg), Some(&msg)
                    )?;
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: peer.peer_device_id.clone(),
                        error: msg,
                    });
                }
            }
        }

        // --- Inbound: receive and apply bundles ---
        // Collect unique channel types in use
        let active_channel_types: Vec<ChannelType> = peers.iter()
            .map(|p| p.channel_type)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .filter(|ct| *ct != ChannelType::Manual)
            .collect();

        for ct in active_channel_types {
            let channel = match self.channels.get(&ct) {
                Some(ch) => ch,
                None => continue,
            };

            // For FolderChannel: update folder paths from peer configs before receiving.
            if ct == ChannelType::Folder {
                if let Some(folder) = channel.as_any().downcast_ref::<FolderChannel>() {
                    let paths: Vec<String> = peers.iter()
                        .filter(|p| p.channel_type == ChannelType::Folder)
                        .filter_map(|p| p.channel_params.get("path").and_then(|v| v.as_str()).map(String::from))
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    folder.set_folder_paths(paths);
                }
            }

            let bundles = match channel.receive_bundles(&workspace_id) {
                Ok(b) => b,
                Err(KrillnotesError::RelayAuthExpired(msg)) => {
                    events.push(SyncEvent::AuthExpired {
                        relay_url: msg.clone(),
                    });
                    continue;
                }
                Err(e) => {
                    events.push(SyncEvent::SyncError {
                        workspace_id: workspace_id.clone(),
                        peer_device_id: "unknown".to_string(),
                        error: e.to_string(),
                    });
                    continue;
                }
            };

            for bundle_ref in bundles {
                // Read header from the zip archive to determine mode.
                // The .swarm format is a zip with header.json at the root.
                use std::io::Cursor;
                use zip::ZipArchive;

                let header = match ZipArchive::new(Cursor::new(&bundle_ref.data))
                    .and_then(|mut zip| {
                        let mut header_file = zip.by_name("header.json")?;
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut header_file, &mut buf)?;
                        Ok(buf)
                    })
                    .map_err(|e| KrillnotesError::Swarm(format!("Cannot read bundle header: {e}")))
                    .and_then(|bytes| {
                        serde_json::from_slice::<crate::core::swarm::header::SwarmHeader>(&bytes)
                            .map_err(KrillnotesError::from)
                    }) {
                    Ok(h) => h,
                    Err(e) => {
                        events.push(SyncEvent::IngestError {
                            workspace_id: workspace_id.clone(),
                            peer_device_id: "unknown".to_string(),
                            error: format!("Failed to read bundle header: {}", e),
                        });
                        continue;
                    }
                };

                use crate::core::swarm::header::SwarmMode;
                // Note: SwarmHeader fields are source_device_id and source_identity
                let sender_device = header.source_device_id.clone();

                match header.mode {
                    SwarmMode::Delta => {
                        // apply_delta signature (swarm/sync.rs:132-209):
                        //   apply_delta(bundle_bytes, workspace, signing_key, contact_manager)
                        match crate::core::swarm::sync::apply_delta(
                            &bundle_ref.data,
                            workspace,
                            ctx.signing_key,
                            ctx.contact_manager,
                        ) {
                            Ok(result) => {
                                let _ = channel.acknowledge(&bundle_ref);
                                events.push(SyncEvent::BundleApplied {
                                    workspace_id: workspace_id.clone(),
                                    peer_device_id: sender_device,
                                    op_count: result.operations_applied,
                                });
                            }
                            Err(e) => {
                                events.push(SyncEvent::IngestError {
                                    workspace_id: workspace_id.clone(),
                                    peer_device_id: sender_device,
                                    error: e.to_string(),
                                });
                                // Do NOT acknowledge — bundle stays pending
                            }
                        }
                    }
                    SwarmMode::Snapshot => {
                        // Snapshot bundles are encrypted zips. They need to be
                        // decrypted via parse_snapshot_bundle() first, then the
                        // JSON payload goes to import_snapshot_json().
                        // For now, snapshot import during polling is uncommon.
                        // TODO: Wire through parse_snapshot_bundle → import_snapshot_json
                        events.push(SyncEvent::UnexpectedBundleMode {
                            workspace_id: workspace_id.clone(),
                            mode: "snapshot (auto-import not yet implemented)".to_string(),
                        });
                        let _ = channel.acknowledge(&bundle_ref);
                    }
                    other => {
                        events.push(SyncEvent::UnexpectedBundleMode {
                            workspace_id: workspace_id.clone(),
                            mode: format!("{:?}", other),
                        });
                        let _ = channel.acknowledge(&bundle_ref);
                    }
                }
            }
        }

        Ok(events)
    }
}
```

**Important implementation note:** The exact method names (`operations_since`, `workspace_id`, `import_snapshot_from_bundle`, `SwarmHeader::from_bundle`, `ApplyResult.applied_count`) must be verified against the actual codebase. The explorer found:
- `workspace.operations_since()` at `workspace/sync.rs:49-118`
- `apply_delta()` at `swarm/sync.rs:132-209`
- `SwarmHeader` at `swarm/header.rs:37-77`
- `ApplyResult` at `swarm/sync.rs:28-38`

Adjust the method calls to match the actual signatures — the code above shows the intended control flow; the exact API may need minor adaptation.

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p krillnotes-core --features relay`
Expected: compiles (may need method signature adjustments)

- [ ] **Step 3: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/sync/mod.rs
git commit -m "feat(sync): implement SyncEngine dispatch loop with SyncEvent system"
```

---

### Task 12: Add Invite Channel Preference Fields

**Files:**
- Modify: `krillnotes-core/src/core/swarm/invite.rs:27-38` (InviteParams)
- Modify: `krillnotes-core/src/core/swarm/invite.rs:175-184` (AcceptParams)

- [ ] **Step 1: Define ReplyChannel and ChannelPreference types**

Add to `swarm/invite.rs` (or a shared location):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyChannel {
    #[serde(rename = "type")]
    pub channel_type: String,  // "relay", "folder", "manual"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,   // relay URL if type == "relay"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>, // human description for manual
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelPreference {
    pub channel_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
}
```

- [ ] **Step 2: Add fields to InviteParams with backward compat**

In `InviteParams` (line 27-38), add:

```rust
#[serde(default)]
pub reply_channels: Vec<ReplyChannel>,
```

- [ ] **Step 3: Add field to AcceptParams with backward compat**

In `AcceptParams` (line 175-184), add:

```rust
#[serde(default)]
pub channel_preference: ChannelPreference,
```

- [ ] **Step 4: Run existing swarm tests to verify backward compat**

Run: `cargo test -p krillnotes-core` (look for invite-related tests)
Expected: all existing tests pass (new fields default to empty/default)

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/swarm/invite.rs
git commit -m "feat(sync): add reply_channels to InviteParams and channel_preference to AcceptParams"
```

---

## Chunk 5: Desktop Integration — Tauri Wiring and UI

### Task 13: Add Sync Tauri Commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:39-77`

- [ ] **Step 1: Add SyncEngine storage to AppState**

In `AppState` (line 39-77), add:

```rust
pub sync_engines: Arc<Mutex<HashMap<String, SyncEngine>>>,  // keyed by identity UUID
```

- [ ] **Step 2: Add Tauri commands for sync operations**

```rust
#[tauri::command]
pub async fn poll_sync(
    window: Window,
    state: State<'_, AppState>,
    workspace_label: String,
) -> Result<Vec<SyncEvent>, String> {
    // Get workspace, identity, and contact manager
    // Call sync_engine.poll(workspace, ctx)
    // Emit events to the window
    // Return events
    todo!("Wire up poll to SyncEngine — requires workspace + identity lookup")
}

#[tauri::command]
pub async fn configure_relay(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<(), String> {
    // Run registration + PoP flow
    // Save credentials
    // Create SyncEngine with RelayChannel
    todo!("Full relay registration flow")
}

#[tauri::command]
pub async fn relay_login(
    state: State<'_, AppState>,
    identity_uuid: String,
    email: String,
    password: String,
) -> Result<(), String> {
    // Re-login with existing credentials
    todo!("Relay re-login flow")
}

#[tauri::command]
pub async fn update_peer_channel(
    window: Window,
    state: State<'_, AppState>,
    workspace_label: String,
    peer_device_id: String,
    channel_type: String,
    channel_params: String,
) -> Result<(), String> {
    // Update peer's channel config in the database
    todo!("Update peer channel configuration")
}
```

- [ ] **Step 3: Register commands in the handler macro**

Add the new commands to `tauri::generate_handler![...]`.

- [ ] **Step 4: Verify compilation**

Run: `cd krillnotes-desktop && npm run tauri build -- --no-bundle` (or `cargo check` on the tauri crate)
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(sync): add Tauri commands for sync polling and relay configuration"
```

---

### Task 14: Extend TypeScript Types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:237-246`

- [ ] **Step 1: Extend PeerInfo interface**

In `types.ts`, update the `PeerInfo` interface to include:

```typescript
export interface PeerInfo {
    // ... existing fields ...
    channelType: string;      // "relay" | "folder" | "manual"
    syncStatus: string;       // "idle" | "syncing" | "error" | "auth_expired"
    syncStatusDetail: string | null;
    lastSyncError: string | null;
}

export interface SyncEvent {
    type: "delta_sent" | "bundle_applied" | "auth_expired" | "sync_error" | "ingest_error" | "unexpected_bundle_mode";
    workspaceId?: string;
    peerDeviceId?: string;
    opCount?: number;
    relayUrl?: string;
    error?: string;
    mode?: string;
}
```

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: type errors in components that use PeerInfo (they need to handle new fields)

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(sync): extend PeerInfo and add SyncEvent TypeScript types"
```

---

### Task 15: Update WorkspacePeersDialog with Channel Status

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Add channel and status display to peer list items**

For each peer in the peers dialog, show:
- Channel type badge ("Relay", "Folder", "Manual")
- Sync status indicator (green dot for idle, spinner for syncing, warning for error, lock for auth_expired)
- `sync_status_detail` as a tooltip on the status indicator
- `last_sync` relative timestamp

- [ ] **Step 2: Add channel configuration controls**

For each peer, add a dropdown or button to change the channel type. When changed, call `invoke("update_peer_channel", ...)`. For relay channel, show the relay URL. For folder, show a path picker.

- [ ] **Step 3: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspacePeersDialog.tsx
git commit -m "feat(sync): show channel type and sync status in peers dialog"
```

---

### Task 16: Enhance Invitation Dialog with Relay Upload

**Files:**
- Modify: `krillnotes-desktop/src/components/CreateInviteDialog.tsx`
- Modify: `krillnotes-desktop/src/components/ImportInviteDialog.tsx`

- [ ] **Step 1: Add distribution step to CreateInviteDialog**

After the existing invite generation step, add a "How do you want to share this?" step with:
- "Copy link" button — calls `invoke("create_relay_invite", ...)`, copies URL to clipboard
- "Save .swarm file" button — existing behavior
- "Both" button

The "Copy link" option is only visible if the current identity has relay credentials.

- [ ] **Step 2: Add relay URL import to ImportInviteDialog**

Add an input field: "Or paste a relay invite URL". When a URL is entered:
- Extract the token from the URL path
- Call `invoke("fetch_relay_invite", { token })` (new Tauri command)
- Display the invite info as if it were imported from a file

- [ ] **Step 3: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 4: Manual test**

Run: `cd krillnotes-desktop && npm run tauri dev`
Test: Create an invite → verify both "Copy link" and "Save file" options appear when relay is configured.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/CreateInviteDialog.tsx
git add krillnotes-desktop/src/components/ImportInviteDialog.tsx
git commit -m "feat(sync): add relay upload/fetch to invitation dialogs"
```

---

## Chunk 6: Integration Tests

### Task 17: Write Integration Tests Against Test Relay

**Files:**
- Create: `krillnotes-core/tests/relay_integration.rs`

These tests run against the test relay at a configurable URL (set via environment variable). They are ignored by default and only run when the relay is available.

- [ ] **Step 1: Write end-to-end relay registration test**

```rust
#[test]
#[ignore] // Run with: cargo test --features relay -- --ignored relay_
fn relay_registration_flow() {
    let relay_url = std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    // 1. Generate identity keypair
    // 2. Register account on relay
    // 3. Solve PoP challenge
    // 4. Verify session token works
    // 5. Clean up: delete account
}
```

- [ ] **Step 2: Write end-to-end sync test (two identities)**

```rust
#[test]
#[ignore]
fn relay_delta_roundtrip() {
    // 1. Create two identities (Alice, Bob)
    // 2. Register both on relay
    // 3. Alice creates workspace, invites Bob
    // 4. Alice generates delta, uploads to relay
    // 5. Bob polls, downloads delta, applies
    // 6. Verify Bob has Alice's operations
}
```

- [ ] **Step 3: Write folder channel integration test**

```rust
#[test]
fn folder_channel_delta_roundtrip() {
    // 1. Create two workspaces (Alice, Bob) with shared temp dir
    // 2. Alice generates delta, folder channel writes it
    // 3. Bob's folder channel picks it up
    // 4. Apply to Bob's workspace
    // 5. Verify operations match
}
```

- [ ] **Step 4: Run folder test (always available)**

Run: `cargo test -p krillnotes-core folder_channel_delta_roundtrip`
Expected: PASS

- [ ] **Step 5: Run relay tests (when relay is running)**

Run: `RELAY_URL=http://localhost:8080 cargo test --features relay -- --ignored relay_`
Expected: PASS (requires `php -S localhost:8080 -t public/ public/index.php` in swarm-relay)

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/tests/relay_integration.rs
git commit -m "test(sync): add integration tests for relay and folder channels"
```

---

### Task 18: Final Verification

- [ ] **Step 1: Run full core test suite**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 3: Run dev build**

Run: `cd krillnotes-desktop && npm update && npm run tauri dev`
Expected: app launches, peers dialog shows channel info

- [ ] **Step 4: Final commit (if any remaining changes)**

```bash
git add -A
git commit -m "feat(sync): sync engine implementation complete"
```
