# Relay Polling & Invite Status Tracking — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add automatic receive-only relay polling and invite status tracking so the inviter sees responses and the invitee sees when snapshots arrive.

**Architecture:** Two polling contexts (workspace-level for deltas+responses, identity-level for snapshots) with core logic in `krillnotes-core` and thin Tauri adapter emitting events. New `AcceptedInviteManager` and `ReceivedResponseManager` follow the `InviteManager` file-I/O pattern.

**Tech Stack:** Rust (krillnotes-core), Tauri v2 commands + events, React 19, TypeScript, Tailwind v4

**Design Doc:** `docs/plans/2026-03-17-relay-polling-invite-status-design.md`

---

## File Structure

### New Files — Core (`krillnotes-core/src/core/`)

| File | Responsibility |
|------|---------------|
| `accepted_invite.rs` | `AcceptedInvite` struct, `AcceptedInviteStatus` enum, `AcceptedInviteManager` (CRUD, file I/O) |
| `received_response.rs` | `ReceivedResponse` struct, `ReceivedResponseStatus` enum, `ReceivedResponseManager` (CRUD, file I/O) |
| `sync/receive_poll.rs` | `receive_poll_workspace()` and `receive_poll_identity()` standalone functions, result types |
| `accepted_invite_tests.rs` | Tests for AcceptedInviteManager |
| `received_response_tests.rs` | Tests for ReceivedResponseManager |
| `sync/receive_poll_tests.rs` | Tests for polling types |

### New Files — Tauri (`krillnotes-desktop/src-tauri/src/commands/`)

| File | Responsibility |
|------|---------------|
| `accepted_invites.rs` | Tauri commands for AcceptedInvite CRUD |
| `receive_poll.rs` | Tauri commands for `poll_receive_workspace` and `poll_receive_identity`, ReceivedResponse CRUD, event emission |

### New Files — Frontend (`krillnotes-desktop/src/`)

| File | Responsibility |
|------|---------------|
| `components/AcceptedInvitesSection.tsx` | Invitee's accepted invites list (embedded in IdentityManagerDialog) |
| `components/PendingResponsesSection.tsx` | Inviter's received responses list (embedded in WorkspacePeersDialog) |
| `hooks/useRelayPolling.ts` | Workspace-level polling hook (setInterval + event listeners) |
| `hooks/useIdentityPolling.ts` | Identity-level polling hook (setInterval + event listeners) |

### Modified Files

| File | Changes |
|------|---------|
| `krillnotes-core/src/core/mod.rs` | Add `pub mod accepted_invite; pub mod received_response;` |
| `krillnotes-core/src/lib.rs` | Add re-exports for new types |
| `krillnotes-core/src/core/sync/mod.rs` | Add `pub mod receive_poll;` |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Add managers to AppState, register new commands |
| `krillnotes-desktop/src-tauri/src/commands/mod.rs` | Add `pub mod accepted_invites; pub mod receive_poll;` + glob re-exports |
| `krillnotes-desktop/src-tauri/src/commands/identity.rs` | Initialize new managers in `create_identity` and `unlock_identity` |
| `krillnotes-desktop/src/types.ts` | Add `AcceptedInviteInfo`, `ReceivedResponseInfo`, `SnapshotReceivedEvent` |
| `krillnotes-desktop/src/components/IdentityManagerDialog.tsx` | Embed `AcceptedInvitesSection` |
| `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` | Embed `PendingResponsesSection` |
| `krillnotes-desktop/src/components/WorkspaceView.tsx` | Add `useRelayPolling`, response toast, `workspace-updated` listener |
| `krillnotes-desktop/src/App.tsx` | Add `useIdentityPolling` |
| `krillnotes-desktop/src/components/ImportInviteDialog.tsx` | Call `save_accepted_invite` after sending response |
| `krillnotes-desktop/src-tauri/src/commands/invites.rs` | Create `ReceivedResponse` in `import_invite_response` |
| `krillnotes-desktop/src/i18n/locales/en.json` | Add namespaced i18n keys under `"polling"` |

---

## Chunk 1: Core Data Models & Managers

### Task 1: AcceptedInvite model and manager

**Files:**
- Create: `krillnotes-core/src/core/accepted_invite.rs`
- Create: `krillnotes-core/src/core/accepted_invite_tests.rs`
- Modify: `krillnotes-core/src/core/mod.rs`
- Modify: `krillnotes-core/src/lib.rs`

- [ ] **Step 1: Write the test file**

The test file uses `use super::*;` (no wrapper module — the `#[cfg(test)]` and `mod tests` are handled by the `#[path]` attribute in the source file).

```rust
// krillnotes-core/src/core/accepted_invite_tests.rs
use super::*;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, AcceptedInviteManager) {
    let dir = TempDir::new().unwrap();
    let mgr = AcceptedInviteManager::new(dir.path().to_path_buf()).unwrap();
    (dir, mgr)
}

#[test]
fn test_save_and_get() {
    let (_dir, mut mgr) = setup();
    let invite = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-123".to_string(),
        "Research Notes".to_string(),
        "base64key".to_string(),
        "Alice".to_string(),
        Some("https://relay.example.com/invites/abc".to_string()),
    );
    let id = invite.invite_id;
    mgr.save(&invite).unwrap();

    let fetched = mgr.get(id).unwrap().unwrap();
    assert_eq!(fetched.workspace_name, "Research Notes");
    assert_eq!(fetched.status, AcceptedInviteStatus::WaitingSnapshot);
    assert!(fetched.workspace_path.is_none());
}

#[test]
fn test_list_returns_sorted_by_accepted_at_desc() {
    let (_dir, mut mgr) = setup();
    let invite1 = AcceptedInvite::new(
        Uuid::new_v4(), "ws-1".into(), "First".into(),
        "key1".into(), "Alice".into(), None,
    );
    let invite2 = AcceptedInvite::new(
        Uuid::new_v4(), "ws-2".into(), "Second".into(),
        "key2".into(), "Bob".into(), None,
    );
    mgr.save(&invite1).unwrap();
    mgr.save(&invite2).unwrap();

    let list = mgr.list().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].accepted_at >= list[1].accepted_at);
}

#[test]
fn test_update_status_to_workspace_created() {
    let (_dir, mut mgr) = setup();
    let invite = AcceptedInvite::new(
        Uuid::new_v4(), "ws-1".into(), "Notes".into(),
        "key".into(), "Alice".into(), None,
    );
    let id = invite.invite_id;
    mgr.save(&invite).unwrap();

    mgr.update_status(id, AcceptedInviteStatus::WorkspaceCreated, Some("/path/to/ws".to_string())).unwrap();

    let fetched = mgr.get(id).unwrap().unwrap();
    assert_eq!(fetched.status, AcceptedInviteStatus::WorkspaceCreated);
    assert_eq!(fetched.workspace_path.as_deref(), Some("/path/to/ws"));
}

#[test]
fn test_list_waiting_snapshot() {
    let (_dir, mut mgr) = setup();
    let invite1 = AcceptedInvite::new(
        Uuid::new_v4(), "ws-1".into(), "First".into(),
        "key1".into(), "Alice".into(), None,
    );
    let id1 = invite1.invite_id;
    mgr.save(&invite1).unwrap();

    let invite2 = AcceptedInvite::new(
        Uuid::new_v4(), "ws-2".into(), "Second".into(),
        "key2".into(), "Bob".into(), None,
    );
    mgr.save(&invite2).unwrap();

    mgr.update_status(id1, AcceptedInviteStatus::WorkspaceCreated, Some("/path".to_string())).unwrap();

    let waiting = mgr.list_waiting_snapshot().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].workspace_name, "Second");
}

#[test]
fn test_delete() {
    let (_dir, mut mgr) = setup();
    let invite = AcceptedInvite::new(
        Uuid::new_v4(), "ws-1".into(), "Notes".into(),
        "key".into(), "Alice".into(), None,
    );
    let id = invite.invite_id;
    mgr.save(&invite).unwrap();
    assert!(mgr.get(id).unwrap().is_some());

    mgr.delete(id).unwrap();
    assert!(mgr.get(id).unwrap().is_none());
}

#[test]
fn test_get_nonexistent_returns_none() {
    let (_dir, mgr) = setup();
    assert!(mgr.get(Uuid::new_v4()).unwrap().is_none());
}
```

- [ ] **Step 2: Implement AcceptedInvite and AcceptedInviteManager**

Note: Error variant is `KrillnotesError::Swarm(...)` (not `Other` — that doesn't exist). Test inclusion uses `#[path]` at the bottom.

```rust
// krillnotes-core/src/core/accepted_invite.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::error::KrillnotesError;

type Result<T> = std::result::Result<T, KrillnotesError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcceptedInviteStatus {
    WaitingSnapshot,
    WorkspaceCreated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedInvite {
    pub invite_id: Uuid,
    pub workspace_id: String,
    pub workspace_name: String,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    pub accepted_at: DateTime<Utc>,
    pub response_relay_url: Option<String>,
    pub status: AcceptedInviteStatus,
    pub workspace_path: Option<String>,
}

impl AcceptedInvite {
    pub fn new(
        invite_id: Uuid,
        workspace_id: String,
        workspace_name: String,
        inviter_public_key: String,
        inviter_declared_name: String,
        response_relay_url: Option<String>,
    ) -> Self {
        Self {
            invite_id,
            workspace_id,
            workspace_name,
            inviter_public_key,
            inviter_declared_name,
            accepted_at: Utc::now(),
            response_relay_url,
            status: AcceptedInviteStatus::WaitingSnapshot,
            workspace_path: None,
        }
    }
}

pub struct AcceptedInviteManager {
    dir: PathBuf,
}

impl AcceptedInviteManager {
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn save(&mut self, invite: &AcceptedInvite) -> Result<()> {
        let path = self.path_for(invite.invite_id);
        let json = serde_json::to_string_pretty(invite)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn get(&self, invite_id: Uuid) -> Result<Option<AcceptedInvite>> {
        let path = self.path_for(invite_id);
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(path)?;
        let record: AcceptedInvite = serde_json::from_str(&data)?;
        Ok(Some(record))
    }

    pub fn list(&self) -> Result<Vec<AcceptedInvite>> {
        let mut records = Vec::new();
        if !self.dir.exists() {
            return Ok(records);
        }
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let data = std::fs::read_to_string(&path)?;
                if let Ok(record) = serde_json::from_str::<AcceptedInvite>(&data) {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| b.accepted_at.cmp(&a.accepted_at));
        Ok(records)
    }

    pub fn list_waiting_snapshot(&self) -> Result<Vec<AcceptedInvite>> {
        Ok(self.list()?.into_iter()
            .filter(|i| i.status == AcceptedInviteStatus::WaitingSnapshot)
            .collect())
    }

    pub fn update_status(
        &mut self,
        invite_id: Uuid,
        status: AcceptedInviteStatus,
        workspace_path: Option<String>,
    ) -> Result<()> {
        let mut record = self.get(invite_id)?
            .ok_or_else(|| KrillnotesError::Swarm(format!("Accepted invite {invite_id} not found")))?;
        record.status = status;
        if workspace_path.is_some() {
            record.workspace_path = workspace_path;
        }
        self.save(&record)
    }

    pub fn delete(&mut self, invite_id: Uuid) -> Result<()> {
        let path = self.path_for(invite_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "accepted_invite_tests.rs"]
mod tests;
```

- [ ] **Step 3: Register module and re-exports**

In `krillnotes-core/src/core/mod.rs`, add:
```rust
pub mod accepted_invite;
```

In `krillnotes-core/src/lib.rs`, add to the `pub use core::{...}` block:
```rust
accepted_invite::{AcceptedInvite, AcceptedInviteManager, AcceptedInviteStatus},
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p krillnotes-core accepted_invite`
Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/accepted_invite.rs krillnotes-core/src/core/accepted_invite_tests.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/lib.rs
git commit -m "feat(core): add AcceptedInvite model and manager"
```

### Task 2: ReceivedResponse model and manager

**Files:**
- Create: `krillnotes-core/src/core/received_response.rs`
- Create: `krillnotes-core/src/core/received_response_tests.rs`
- Modify: `krillnotes-core/src/core/mod.rs`
- Modify: `krillnotes-core/src/lib.rs`

- [ ] **Step 1: Write the test file**

```rust
// krillnotes-core/src/core/received_response_tests.rs
use super::*;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, ReceivedResponseManager) {
    let dir = TempDir::new().unwrap();
    let mgr = ReceivedResponseManager::new(dir.path().to_path_buf()).unwrap();
    (dir, mgr)
}

#[test]
fn test_save_and_get() {
    let (_dir, mut mgr) = setup();
    let response = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-123".to_string(),
        "Research Notes".to_string(),
        "invitee_key_base64".to_string(),
        "Carol Davis".to_string(),
    );
    let id = response.response_id;
    mgr.save(&response).unwrap();

    let fetched = mgr.get(id).unwrap().unwrap();
    assert_eq!(fetched.invitee_declared_name, "Carol Davis");
    assert_eq!(fetched.status, ReceivedResponseStatus::Pending);
}

#[test]
fn test_list_by_workspace() {
    let (_dir, mut mgr) = setup();
    let r1 = ReceivedResponse::new(Uuid::new_v4(), "ws-1".into(), "Notes".into(), "key1".into(), "Carol".into());
    let r2 = ReceivedResponse::new(Uuid::new_v4(), "ws-2".into(), "Other".into(), "key2".into(), "Dave".into());
    let r3 = ReceivedResponse::new(Uuid::new_v4(), "ws-1".into(), "Notes".into(), "key3".into(), "Eve".into());
    mgr.save(&r1).unwrap();
    mgr.save(&r2).unwrap();
    mgr.save(&r3).unwrap();

    assert_eq!(mgr.list_by_workspace("ws-1").unwrap().len(), 2);
    assert_eq!(mgr.list_by_workspace("ws-2").unwrap().len(), 1);
}

#[test]
fn test_update_status_progression() {
    let (_dir, mut mgr) = setup();
    let response = ReceivedResponse::new(Uuid::new_v4(), "ws-1".into(), "Notes".into(), "key".into(), "Carol".into());
    let id = response.response_id;
    mgr.save(&response).unwrap();

    mgr.update_status(id, ReceivedResponseStatus::PeerAdded).unwrap();
    assert_eq!(mgr.get(id).unwrap().unwrap().status, ReceivedResponseStatus::PeerAdded);

    mgr.update_status(id, ReceivedResponseStatus::SnapshotSent).unwrap();
    assert_eq!(mgr.get(id).unwrap().unwrap().status, ReceivedResponseStatus::SnapshotSent);
}

#[test]
fn test_find_by_invite_and_invitee() {
    let (_dir, mut mgr) = setup();
    let invite_id = Uuid::new_v4();
    let r1 = ReceivedResponse::new(invite_id, "ws-1".into(), "Notes".into(), "key_carol".into(), "Carol".into());
    let r2 = ReceivedResponse::new(invite_id, "ws-1".into(), "Notes".into(), "key_dave".into(), "Dave".into());
    mgr.save(&r1).unwrap();
    mgr.save(&r2).unwrap();

    let found = mgr.find_by_invite_and_invitee(invite_id, "key_carol").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().invitee_declared_name, "Carol");

    assert!(mgr.find_by_invite_and_invitee(invite_id, "key_unknown").unwrap().is_none());
}

#[test]
fn test_delete() {
    let (_dir, mut mgr) = setup();
    let response = ReceivedResponse::new(Uuid::new_v4(), "ws-1".into(), "Notes".into(), "key".into(), "Carol".into());
    let id = response.response_id;
    mgr.save(&response).unwrap();
    mgr.delete(id).unwrap();
    assert!(mgr.get(id).unwrap().is_none());
}
```

- [ ] **Step 2: Implement ReceivedResponse and ReceivedResponseManager**

Same patterns as Task 1. Use `KrillnotesError::Swarm(...)` for "not found" errors. Include `#[cfg(test)] #[path = "received_response_tests.rs"] mod tests;` at the bottom. See Task 1 implementation for the full template — adapt field names and types.

- [ ] **Step 3: Register module and re-exports**

In `krillnotes-core/src/core/mod.rs`:
```rust
pub mod received_response;
```

In `krillnotes-core/src/lib.rs`, add to re-exports:
```rust
received_response::{ReceivedResponse, ReceivedResponseManager, ReceivedResponseStatus},
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p krillnotes-core received_response`
Expected: all 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/received_response.rs krillnotes-core/src/core/received_response_tests.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/lib.rs
git commit -m "feat(core): add ReceivedResponse model and manager"
```

---

## Chunk 2: Core Polling Functions

### Task 3: Receive polling types and functions

**Files:**
- Create: `krillnotes-core/src/core/sync/receive_poll.rs`
- Create: `krillnotes-core/src/core/sync/receive_poll_tests.rs`
- Modify: `krillnotes-core/src/core/sync/mod.rs`

**Context files to read before implementing:**
- `krillnotes-core/src/core/sync/mod.rs` — `SyncEngine::poll()` inbound phase (lines 172-339) for delta application
- `krillnotes-core/src/core/sync/relay/mod.rs` — `RelayChannel::receive_bundles()` for bundle fetch pattern
- `krillnotes-core/src/core/sync/relay/client.rs` — `RelayClient` API: `list_bundles()` (no params, returns `Vec<BundleMeta>`), `download_bundle(&bundle_id)`, `delete_bundle(&bundle_id)`, `ensure_mailbox(&workspace_id)`
- `krillnotes-core/src/core/sync/relay/client.rs:68-75` — `BundleMeta` struct: `bundle_id`, `workspace_id`, `sender_device_key`, `mode` (all `String`), `size_bytes` (`u64`), `created_at` (`String`)

- [ ] **Step 1: Define types**

```rust
// krillnotes-core/src/core/sync/receive_poll.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::core::error::KrillnotesError;
use crate::core::accepted_invite::{AcceptedInvite, AcceptedInviteStatus};
use crate::core::invite::InviteManager;
use crate::core::received_response::{ReceivedResponse, ReceivedResponseManager};
use crate::core::sync::relay::client::RelayClient;
use crate::core::sync::relay::relay_account::RelayAccount;

type Result<T> = std::result::Result<T, KrillnotesError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedBundle {
    pub peer_device_id: String,
    pub mode: String,
    pub op_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollError {
    pub bundle_id: Option<String>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePollResult {
    pub applied_bundles: Vec<AppliedBundle>,
    pub new_responses: Vec<ReceivedResponse>,
    pub errors: Vec<PollError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedSnapshot {
    pub workspace_id: String,
    pub invite_id: Uuid,
    pub snapshot_path: PathBuf,
    pub sender_device_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPollResult {
    pub received_snapshots: Vec<ReceivedSnapshot>,
    pub errors: Vec<PollError>,
}

pub struct RelayConnection {
    pub account: RelayAccount,
    pub client: RelayClient,
}
```

- [ ] **Step 2: Implement `receive_poll_workspace()`**

Key API facts:
- `relay_client.list_bundles()` — no params, returns all bundles for the account
- `BundleMeta` fields: `.bundle_id`, `.workspace_id`, `.sender_device_key`, `.mode` (all `String`)
- `relay_client.download_bundle(&meta.bundle_id)` — fetches bundle bytes
- `relay_client.delete_bundle(&meta.bundle_id)` — acknowledges/removes bundle
- `workspace.workspace_id()` returns `&str` (not `Option`)

The function should:
1. Call `list_bundles()`, filter by `workspace_id`
2. For `mode == "delta"` bundles: apply via existing SyncEngine delta logic
3. For `mode == "accept"` bundles: parse invite response, validate against InviteManager, create ReceivedResponse
4. Skip `"snapshot"` / `"invite"` mode (not handled at workspace level)
5. `delete_bundle()` for processed bundles
6. Return `WorkspacePollResult`

**Important:** Read `SyncEngine::poll()` lines 172-339 in `sync/mod.rs` to understand how delta bundles are parsed and applied. Reuse existing parsing logic. For Accept bundles, read `invite.rs` `parse_and_verify_invite` and the response parsing code.

- [ ] **Step 3: Implement `receive_poll_identity()`**

Key API facts:
- `conn.client.ensure_mailbox(&ws_id)` — registers interest in a workspace
- `BundleMeta.mode` is `String`, not `Option<String>` — filter with `m.mode == "snapshot"`
- Process snapshots one at a time, write to temp files (snapshots can be up to 10 MB)

```rust
pub fn receive_poll_identity(
    relay_connections: &[RelayConnection],
    accepted_invites: &[AcceptedInvite],
    temp_dir: &std::path::Path,
) -> Result<IdentityPollResult> {
    let mut result = IdentityPollResult {
        received_snapshots: Vec::new(),
        errors: Vec::new(),
    };

    let waiting_ws_ids: std::collections::HashSet<String> = accepted_invites.iter()
        .filter(|i| i.status == AcceptedInviteStatus::WaitingSnapshot)
        .map(|i| i.workspace_id.clone())
        .collect();

    if waiting_ws_ids.is_empty() {
        return Ok(result);
    }

    for conn in relay_connections {
        // Register mailboxes — invitee registers on THEIR relay for INVITER's workspace IDs
        for ws_id in &waiting_ws_ids {
            if let Err(e) = conn.client.ensure_mailbox(ws_id) {
                log::warn!("Failed to ensure mailbox for {ws_id}: {e}");
            }
        }

        let bundle_metas = match conn.client.list_bundles() {
            Ok(metas) => metas,
            Err(e) => {
                result.errors.push(PollError {
                    bundle_id: None,
                    error: format!("list_bundles on {} failed: {e}", conn.account.relay_url),
                });
                continue;
            }
        };

        // Filter for snapshot bundles for our workspace IDs
        let snapshots: Vec<_> = bundle_metas.into_iter()
            .filter(|m| waiting_ws_ids.contains(&m.workspace_id))
            .filter(|m| m.mode == "snapshot")
            .collect();

        // Process one at a time to limit memory
        for meta in snapshots {
            let bid = meta.bundle_id.clone();
            match conn.client.download_bundle(&meta.bundle_id) {
                Ok(bundle_bytes) => {
                    let snapshot_path = temp_dir.join(format!("snapshot-{}.bin", Uuid::new_v4()));
                    if let Err(e) = std::fs::write(&snapshot_path, &bundle_bytes) {
                        result.errors.push(PollError {
                            bundle_id: Some(bid),
                            error: format!("Failed to write snapshot: {e}"),
                        });
                        continue;
                    }

                    let invite_id = accepted_invites.iter()
                        .find(|i| i.workspace_id == meta.workspace_id)
                        .map(|i| i.invite_id)
                        .unwrap_or_default();

                    result.received_snapshots.push(ReceivedSnapshot {
                        workspace_id: meta.workspace_id.clone(),
                        invite_id,
                        snapshot_path,
                        sender_device_key: meta.sender_device_key.clone(),
                    });

                    let _ = conn.client.delete_bundle(&bid);
                }
                Err(e) => {
                    result.errors.push(PollError {
                        bundle_id: Some(bid),
                        error: format!("download_bundle failed: {e}"),
                    });
                }
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
#[path = "receive_poll_tests.rs"]
mod tests;
```

- [ ] **Step 4: Write type serialization tests**

```rust
// krillnotes-core/src/core/sync/receive_poll_tests.rs
use super::*;

#[test]
fn test_workspace_poll_result_serializes_camel_case() {
    let result = WorkspacePollResult {
        applied_bundles: vec![AppliedBundle {
            peer_device_id: "device-1".to_string(),
            mode: "delta".to_string(),
            op_count: 5,
        }],
        new_responses: vec![],
        errors: vec![],
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("peerDeviceId"));
    assert!(json.contains("opCount"));
}

#[test]
fn test_identity_poll_result_serializes_camel_case() {
    let result = IdentityPollResult {
        received_snapshots: vec![],
        errors: vec![PollError {
            bundle_id: Some("bundle-123".to_string()),
            error: "timeout".to_string(),
        }],
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("bundleId"));
}
```

- [ ] **Step 5: Register module**

In `krillnotes-core/src/core/sync/mod.rs`:
```rust
pub mod receive_poll;
```

- [ ] **Step 6: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests PASS

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/sync/receive_poll.rs krillnotes-core/src/core/sync/receive_poll_tests.rs krillnotes-core/src/core/sync/mod.rs
git commit -m "feat(core): add receive-only polling functions"
```

---

## Chunk 3: Tauri Commands

### Task 4: AppState, manager initialization, and AcceptedInvite commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/identity.rs`
- Create: `krillnotes-desktop/src-tauri/src/commands/accepted_invites.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/mod.rs`

- [ ] **Step 1: Add managers to AppState**

In `lib.rs` AppState struct (after existing `relay_account_managers` field):
```rust
pub accepted_invite_managers: Arc<Mutex<HashMap<Uuid, krillnotes_core::core::accepted_invite::AcceptedInviteManager>>>,
pub received_response_managers: Arc<Mutex<HashMap<Uuid, krillnotes_core::core::received_response::ReceivedResponseManager>>>,
```

In `.manage(AppState { ... })` block:
```rust
accepted_invite_managers: Arc::new(Mutex::new(HashMap::new())),
received_response_managers: Arc::new(Mutex::new(HashMap::new())),
```

- [ ] **Step 2: Initialize managers on identity unlock/create**

In `commands/identity.rs`, find BOTH `create_identity` (~line 82-129) and `unlock_identity` (~line 145-226). After the `invite_managers` initialization block (which looks like the pattern below), add the new managers:

```rust
// Existing pattern (already in the code):
let invites_dir = crate::settings::config_dir()
    .join("identities")
    .join(uuid.to_string())
    .join("invites");
match krillnotes_core::core::invite::InviteManager::new(invites_dir) {
    Ok(im) => { state.invite_managers.lock().expect("Mutex poisoned").insert(uuid, im); }
    Err(e) => { log::warn!("Failed to initialize invite manager for {uuid}: {e}"); }
}

// ADD AFTER the invite_managers block:
let accepted_dir = crate::settings::config_dir()
    .join("identities")
    .join(uuid.to_string())
    .join("accepted_invites");
match krillnotes_core::core::accepted_invite::AcceptedInviteManager::new(accepted_dir) {
    Ok(mgr) => { state.accepted_invite_managers.lock().expect("Mutex poisoned").insert(uuid, mgr); }
    Err(e) => { log::warn!("Failed to initialize accepted invite manager for {uuid}: {e}"); }
}

let responses_dir = crate::settings::config_dir()
    .join("identities")
    .join(uuid.to_string())
    .join("invite_responses");
match krillnotes_core::core::received_response::ReceivedResponseManager::new(responses_dir) {
    Ok(mgr) => { state.received_response_managers.lock().expect("Mutex poisoned").insert(uuid, mgr); }
    Err(e) => { log::warn!("Failed to initialize received response manager for {uuid}: {e}"); }
}
```

Add this in BOTH `create_identity` and `unlock_identity` functions.

- [ ] **Step 3: Create accepted_invites commands**

Follow the exact pattern from `commands/invites.rs`: `InviteInfo` return type with `From` impl, AppState access via brief lock.

```rust
// krillnotes-desktop/src-tauri/src/commands/accepted_invites.rs
use crate::AppState;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedInviteInfo {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    pub accepted_at: String,
    pub response_relay_url: Option<String>,
    pub status: String,
    pub workspace_path: Option<String>,
}

impl From<krillnotes_core::core::accepted_invite::AcceptedInvite> for AcceptedInviteInfo {
    fn from(r: krillnotes_core::core::accepted_invite::AcceptedInvite) -> Self {
        Self {
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            inviter_public_key: r.inviter_public_key,
            inviter_declared_name: r.inviter_declared_name,
            accepted_at: r.accepted_at.to_rfc3339(),
            response_relay_url: r.response_relay_url,
            status: match r.status {
                krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WaitingSnapshot => "waitingSnapshot".to_string(),
                krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WorkspaceCreated => "workspaceCreated".to_string(),
            },
            workspace_path: r.workspace_path,
        }
    }
}

#[tauri::command]
pub fn list_accepted_invites(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<AcceptedInviteInfo>, String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let records = mgr.list().map_err(|e| {
        log::error!("list_accepted_invites(identity={identity_uuid}) failed: {e}");
        e.to_string()
    })?;
    Ok(records.into_iter().map(AcceptedInviteInfo::from).collect())
}

#[tauri::command]
pub fn save_accepted_invite(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
    workspace_id: String,
    workspace_name: String,
    inviter_public_key: String,
    inviter_declared_name: String,
    response_relay_url: Option<String>,
) -> std::result::Result<AcceptedInviteInfo, String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let invite_uuid: Uuid = invite_id.parse().map_err(|e| format!("Invalid invite UUID: {e}"))?;
    let mut managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;

    let invite = krillnotes_core::core::accepted_invite::AcceptedInvite::new(
        invite_uuid, workspace_id, workspace_name,
        inviter_public_key, inviter_declared_name, response_relay_url,
    );
    mgr.save(&invite).map_err(|e| e.to_string())?;
    Ok(AcceptedInviteInfo::from(invite))
}

#[tauri::command]
pub fn update_accepted_invite_status(
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
    status: String,
    workspace_path: Option<String>,
) -> std::result::Result<(), String> {
    let uuid: Uuid = identity_uuid.parse().map_err(|e| format!("Invalid UUID: {e}"))?;
    let invite_uuid: Uuid = invite_id.parse().map_err(|e| format!("Invalid invite UUID: {e}"))?;
    let mut managers = state.accepted_invite_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get_mut(&uuid).ok_or("Identity not unlocked")?;
    let new_status = match status.as_str() {
        "waitingSnapshot" => krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WaitingSnapshot,
        "workspaceCreated" => krillnotes_core::core::accepted_invite::AcceptedInviteStatus::WorkspaceCreated,
        _ => return Err(format!("Invalid status: {status}")),
    };
    mgr.update_status(invite_uuid, new_status, workspace_path).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Register in commands/mod.rs and lib.rs**

In `commands/mod.rs`, add:
```rust
pub mod accepted_invites;
pub use accepted_invites::*;
```

In `lib.rs` `tauri::generate_handler![...]`, add:
```rust
list_accepted_invites,
save_accepted_invite,
update_accepted_invite_status,
```

- [ ] **Step 5: Build**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/accepted_invites.rs krillnotes-desktop/src-tauri/src/commands/mod.rs krillnotes-desktop/src-tauri/src/commands/identity.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(tauri): add AcceptedInvite commands and AppState managers"
```

### Task 5: ReceivedResponse commands + async polling commands

**Files:**
- Create: `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/mod.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**CRITICAL PATTERN:** Both polling commands MUST be `async` and use `tokio::task::spawn_blocking` because `RelayClient` uses `reqwest::blocking::Client` which panics in async context. Follow the exact pattern from `commands/sync.rs` `poll_sync()`:
1. Acquire each mutex briefly, extract/clone needed values, drop the guard immediately
2. Move all relay work into `tokio::task::spawn_blocking`
3. Emit events after spawn_blocking completes

- [ ] **Step 1: Create the file with ReceivedResponse CRUD + polling commands**

```rust
// krillnotes-desktop/src-tauri/src/commands/receive_poll.rs
use crate::AppState;
use serde::Serialize;
use tauri::{Emitter, State, Window};
use uuid::Uuid;

// --- ReceivedResponse CRUD (these are sync, no relay calls) ---

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
// CRITICAL: Must use async + spawn_blocking because RelayClient uses reqwest::blocking

#[tauri::command]
pub async fn poll_receive_workspace(
    window: Window,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let workspace_label = window.label().to_string();

    // -- Collect context under brief locks (follow poll_sync pattern from commands/sync.rs) --
    let identity_uuid = {
        let m = state.workspace_identities.lock().map_err(|e| e.to_string())?;
        *m.get(&workspace_label).ok_or("No identity bound to this workspace")?
    };

    // Read commands/sync.rs poll_sync() (lines 50-120) for the full pattern of:
    // 1. Extracting signing_key, display_name, identity_pubkey under brief locks
    // 2. Getting relay_accounts list
    // 3. Building workspace_id_str
    // 4. Cloning Arc references for spawn_blocking
    // Replicate that pattern here, then call receive_poll_workspace from core.

    // Clone Arcs for move into spawn_blocking
    let workspaces_arc = state.workspaces.clone();
    let rrm_arc = state.received_response_managers.clone();
    let im_arc = state.invite_managers.clone();
    // ... extract other needed values ...

    let window_clone = window.clone();

    let result = tokio::task::spawn_blocking(move || {
        // Inside spawn_blocking: acquire locks, call core function
        // let mut workspaces = workspaces_arc.lock().expect("Mutex poisoned");
        // let workspace = workspaces.get_mut(&workspace_label).ok_or("...")?;
        // ... build relay_client, call receive_poll_workspace() ...
        // Return WorkspacePollResult
        Ok::<_, String>(krillnotes_core::core::sync::receive_poll::WorkspacePollResult {
            applied_bundles: vec![],
            new_responses: vec![],
            errors: vec![],
        })
    }).await.map_err(|e| e.to_string())??;

    // Emit events from the async context (not inside spawn_blocking)
    for response in &result.new_responses {
        let _ = window.emit("invite-response-received", ReceivedResponseInfo::from(response.clone()));
    }
    for bundle in &result.applied_bundles {
        let _ = window.emit("sync-bundle-applied", bundle);
    }
    // CRITICAL: Emit workspace-updated when deltas were applied so the tree refreshes
    if !result.applied_bundles.is_empty() {
        let _ = window.emit("workspace-updated", ());
    }
    for error in &result.errors {
        let _ = window.emit("poll-error", serde_json::json!({ "error": error.error }));
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

    // Get accepted invites in WaitingSnapshot status (brief lock)
    let waiting = {
        let aim = state.accepted_invite_managers.lock().expect("Mutex poisoned");
        let ai_mgr = aim.get(&uuid).ok_or("Accepted invite manager not found")?;
        ai_mgr.list_waiting_snapshot().map_err(|e| e.to_string())?
    };

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
    // Read commands/sync.rs for how RelayClient is constructed from RelayAccount
    let temp_dir = std::env::temp_dir();

    let result = tokio::task::spawn_blocking(move || {
        // Build RelayConnection for each account...
        // Call: receive_poll_identity(&connections, &waiting, &temp_dir)
        Ok::<_, String>(krillnotes_core::core::sync::receive_poll::IdentityPollResult {
            received_snapshots: vec![],
            errors: vec![],
        })
    }).await.map_err(|e| e.to_string())??;

    // Emit events
    for snapshot in &result.received_snapshots {
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
```

**Note to implementer:** The `spawn_blocking` blocks contain placeholder code. Read `commands/sync.rs` `poll_sync()` function (lines 50-200) to understand:
1. How `RelayClient` is constructed from `RelayAccount` (session token, relay URL)
2. How workspace data is extracted under brief locks
3. How Arcs are cloned for the move into `spawn_blocking`
Replicate that exact pattern for building the relay client and calling the core polling functions.

- [ ] **Step 2: Register in commands/mod.rs and lib.rs**

In `commands/mod.rs`:
```rust
pub mod receive_poll;
pub use receive_poll::*;
```

In `lib.rs` `tauri::generate_handler![...]`:
```rust
list_received_responses,
update_response_status,
dismiss_response,
poll_receive_workspace,
poll_receive_identity,
```

- [ ] **Step 3: Build**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: compiles (polling commands have placeholder spawn_blocking — will need fleshing out)

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/receive_poll.rs krillnotes-desktop/src-tauri/src/commands/mod.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(tauri): add ReceivedResponse commands and async polling commands"
```

---

## Chunk 4: Frontend Types & Polling Hooks

### Task 6: TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

- [ ] **Step 1: Add new types**

```typescript
export interface AcceptedInviteInfo {
  inviteId: string;
  workspaceId: string;
  workspaceName: string;
  inviterPublicKey: string;
  inviterDeclaredName: string;
  acceptedAt: string;
  responseRelayUrl: string | null;
  status: "waitingSnapshot" | "workspaceCreated";
  workspacePath: string | null;
}

export interface ReceivedResponseInfo {
  responseId: string;
  inviteId: string;
  workspaceId: string;
  workspaceName: string;
  inviteePublicKey: string;
  inviteeDeclaredName: string;
  receivedAt: string;
  status: "pending" | "peerAdded" | "snapshotSent";
}

export interface SnapshotReceivedEvent {
  workspaceId: string;
  inviteId: string;
  snapshotPath: string;
}
```

- [ ] **Step 2: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(types): add AcceptedInviteInfo, ReceivedResponseInfo, SnapshotReceivedEvent"
```

### Task 7: Polling hooks

**Files:**
- Create: `krillnotes-desktop/src/hooks/useRelayPolling.ts`
- Create: `krillnotes-desktop/src/hooks/useIdentityPolling.ts`

- [ ] **Step 1: Create useRelayPolling**

```typescript
// krillnotes-desktop/src/hooks/useRelayPolling.ts
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

const POLL_INTERVAL_MS = 60_000;

export function useRelayPolling(hasRelayPeers: boolean) {
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!hasRelayPeers) return;

    const poll = async () => {
      try {
        await invoke("poll_receive_workspace");
      } catch (e) {
        console.warn("poll_receive_workspace failed:", e);
      }
    };

    poll(); // immediate first poll
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [hasRelayPeers]);
}
```

- [ ] **Step 2: Create useIdentityPolling**

```typescript
// krillnotes-desktop/src/hooks/useIdentityPolling.ts
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

const POLL_INTERVAL_MS = 60_000;

export function useIdentityPolling(
  identityUuid: string | null,
  hasRelayAccount: boolean,
  hasWaitingInvites: boolean,
) {
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!identityUuid || !hasRelayAccount || !hasWaitingInvites) return;

    const poll = async () => {
      try {
        await invoke("poll_receive_identity", { identityUuid });
      } catch (e) {
        console.warn("poll_receive_identity failed:", e);
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [identityUuid, hasRelayAccount, hasWaitingInvites]);
}
```

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/hooks/useRelayPolling.ts krillnotes-desktop/src/hooks/useIdentityPolling.ts
git commit -m "feat(hooks): add useRelayPolling and useIdentityPolling hooks"
```

---

## Chunk 5: Frontend UI Components

### Task 8: i18n keys

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`

Existing convention: keys are namespaced (e.g., `"common.save"`, `"peers.title"`, `"invite.createTitle"`).

- [ ] **Step 1: Add namespaced keys**

Add a `"polling"` namespace:
```json
"polling": {
  "acceptedInvites": "Accepted Invites",
  "waitingForSnapshot": "Waiting for snapshot",
  "workspaceCreated": "Workspace created",
  "pendingInviteResponses": "Pending Invite Responses",
  "actionNeeded": "Action needed",
  "acceptAndSendSnapshot": "Accept & Send Snapshot",
  "peerAdded": "Peer added",
  "sendSnapshot": "Send Snapshot",
  "snapshotSent": "Snapshot sent",
  "newInviteResponse": "New invite response",
  "respondedToYourInvite": "responded to your invite",
  "viewInPeers": "View in Peers",
  "snapshotHint": "Snapshots arrive automatically when a workspace with relay polling is open."
}
```

- [ ] **Step 2: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/en.json
git commit -m "feat(i18n): add polling namespace i18n keys"
```

### Task 9: AcceptedInvitesSection (invitee view)

**Files:**
- Create: `krillnotes-desktop/src/components/AcceptedInvitesSection.tsx`
- Modify: `krillnotes-desktop/src/components/IdentityManagerDialog.tsx`

**CSS pattern:** Use Tailwind `bg-{color}-{shade}/{opacity}` for badges (matching `WorkspacePeersDialog.tsx` patterns like `bg-green-500/20 text-green-400`). NOT hardcoded dark-mode colors.

- [ ] **Step 1: Create the component**

```typescript
// krillnotes-desktop/src/components/AcceptedInvitesSection.tsx
import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useTranslation } from "react-i18next";
import type { AcceptedInviteInfo, SnapshotReceivedEvent } from "../types";

interface Props {
  identityUuid: string;
}

export default function AcceptedInvitesSection({ identityUuid }: Props) {
  const { t } = useTranslation();
  const [invites, setInvites] = useState<AcceptedInviteInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const loadInvites = useCallback(async () => {
    try {
      const result = await invoke<AcceptedInviteInfo[]>("list_accepted_invites", { identityUuid });
      setInvites(result);
    } catch (e) {
      console.error("Failed to load accepted invites:", e);
    } finally {
      setLoading(false);
    }
  }, [identityUuid]);

  useEffect(() => { loadInvites(); }, [loadInvites]);

  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<SnapshotReceivedEvent>("snapshot-received", () => {
      loadInvites();
    });
    return () => { unlisten.then(f => f()); };
  }, [loadInvites]);

  if (loading || invites.length === 0) return null;

  return (
    <div className="mt-4">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-purple-400 mb-2">
        {t("polling.acceptedInvites")}
      </h4>
      <div className="flex flex-col gap-2">
        {invites.map((invite) => (
          <div key={invite.inviteId}
            className="bg-white/5 rounded-lg px-4 py-3 flex items-center justify-between"
          >
            <div>
              <div className="font-semibold text-sm">{invite.workspaceName}</div>
              <div className="text-xs text-gray-400 mt-0.5">
                {t("common.from", "From")}: {invite.inviterDeclaredName} · {new Date(invite.acceptedAt).toLocaleDateString()}
              </div>
            </div>
            <div className="flex items-center gap-2">
              {invite.status === "waitingSnapshot" ? (
                <span className="bg-amber-500/20 text-amber-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                  {t("polling.waitingForSnapshot")}
                </span>
              ) : (
                <>
                  <span className="bg-green-500/20 text-green-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    ✓ {t("polling.workspaceCreated")}
                  </span>
                  {invite.workspacePath && (
                    <button className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1 rounded-md">
                      {t("common.open")}
                    </button>
                  )}
                </>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Embed in IdentityManagerDialog**

Read `IdentityManagerDialog.tsx`. The dialog renders a list of identities with a `selectedUuid` state. Add `AcceptedInvitesSection` below the selected identity's detail area — specifically after the inline form block (`{activeForm && ...}`), conditionally rendered when the selected identity is unlocked:

```typescript
import AcceptedInvitesSection from "./AcceptedInvitesSection";

// After the inline form rendering for the selected identity:
{selectedUuid && unlockedIds.has(selectedUuid) && (
  <AcceptedInvitesSection identityUuid={selectedUuid} />
)}
```

- [ ] **Step 3: Type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/AcceptedInvitesSection.tsx krillnotes-desktop/src/components/IdentityManagerDialog.tsx
git commit -m "feat(ui): add Accepted Invites section to Identity Manager"
```

### Task 10: PendingResponsesSection (inviter view)

**Files:**
- Create: `krillnotes-desktop/src/components/PendingResponsesSection.tsx`
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Create the component**

Same CSS pattern as Task 9 — use `bg-{color}-500/20 text-{color}-400` for badges.

```typescript
// krillnotes-desktop/src/components/PendingResponsesSection.tsx
import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useTranslation } from "react-i18next";
import type { ReceivedResponseInfo } from "../types";

interface Props {
  identityUuid: string;
  workspaceId: string;
  onAcceptResponse: (response: ReceivedResponseInfo) => void;
  onSendSnapshot: (response: ReceivedResponseInfo) => void;
}

export default function PendingResponsesSection({
  identityUuid, workspaceId, onAcceptResponse, onSendSnapshot,
}: Props) {
  const { t } = useTranslation();
  const [responses, setResponses] = useState<ReceivedResponseInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const loadResponses = useCallback(async () => {
    try {
      const result = await invoke<ReceivedResponseInfo[]>("list_received_responses", {
        identityUuid, workspaceId,
      });
      setResponses(result);
    } catch (e) {
      console.error("Failed to load received responses:", e);
    } finally {
      setLoading(false);
    }
  }, [identityUuid, workspaceId]);

  useEffect(() => { loadResponses(); }, [loadResponses]);

  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<ReceivedResponseInfo>(
      "invite-response-received", () => { loadResponses(); }
    );
    return () => { unlisten.then(f => f()); };
  }, [loadResponses]);

  if (loading || responses.length === 0) return null;

  const pendingCount = responses.filter(r => r.status === "pending").length;

  return (
    <div className="mb-4">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-amber-400 mb-2 flex items-center gap-2">
        {t("polling.pendingInviteResponses")}
        {pendingCount > 0 && (
          <span className="bg-amber-400 text-gray-900 px-1.5 py-0.5 rounded-full text-[10px] font-bold">
            {pendingCount}
          </span>
        )}
      </h4>
      <div className="flex flex-col gap-2">
        {responses.map((resp) => (
          <div key={resp.responseId}
            className={`bg-white/5 rounded-lg px-4 py-3 flex items-center justify-between ${
              resp.status === "pending" ? "border-l-3 border-amber-400" : ""
            } ${resp.status === "snapshotSent" ? "opacity-60" : ""}`}
          >
            <div>
              <div className="font-semibold text-sm">{resp.inviteeDeclaredName}</div>
              <div className="text-xs text-gray-400 mt-0.5">
                {t("polling.responded", "Responded")} {new Date(resp.receivedAt).toLocaleDateString()}
              </div>
            </div>
            <div className="flex items-center gap-2">
              {resp.status === "pending" && (
                <>
                  <span className="bg-amber-500/20 text-amber-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    {t("polling.actionNeeded")}
                  </span>
                  <button
                    className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1.5 rounded-md"
                    onClick={() => onAcceptResponse(resp)}
                  >
                    {t("polling.acceptAndSendSnapshot")}
                  </button>
                </>
              )}
              {resp.status === "peerAdded" && (
                <>
                  <span className="bg-blue-500/20 text-blue-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    {t("polling.peerAdded")}
                  </span>
                  <button
                    className="bg-gray-600 hover:bg-gray-500 text-white text-xs px-3 py-1 rounded-md"
                    onClick={() => onSendSnapshot(resp)}
                  >
                    {t("polling.sendSnapshot")}
                  </button>
                </>
              )}
              {resp.status === "snapshotSent" && (
                <span className="bg-green-500/20 text-green-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                  ✓ {t("polling.snapshotSent")}
                </span>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Embed in WorkspacePeersDialog**

Read `WorkspacePeersDialog.tsx`. Add `PendingResponsesSection` above the existing peers list, passing callbacks that trigger the existing AcceptPeer → SendSnapshot flow. The `workspaceId` can be obtained from `workspaceInfo`.

- [ ] **Step 3: Type check and commit**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

```bash
git add krillnotes-desktop/src/components/PendingResponsesSection.tsx krillnotes-desktop/src/components/WorkspacePeersDialog.tsx
git commit -m "feat(ui): add Pending Responses section to Workspace Peers dialog"
```

### Task 11: Toast notification + workspace polling in WorkspaceView

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**CRITICAL:** The `t` variable from `useTranslation()` must NOT be shadowed by the map variable. Use `toast` as the loop variable name.

- [ ] **Step 1: Add response toast state + event listener**

Read `WorkspaceView.tsx`. Add alongside the existing `migrationToasts`:

```typescript
import type { ReceivedResponseInfo } from "../types";

const [responseToasts, setResponseToasts] = useState<ReceivedResponseInfo[]>([]);

useEffect(() => {
  const unlisten = getCurrentWebviewWindow().listen<ReceivedResponseInfo>(
    "invite-response-received",
    (event) => {
      const toast = event.payload;
      setResponseToasts(prev => [...prev, toast]);
      setTimeout(() => {
        setResponseToasts(prev => prev.filter(t2 => t2 !== toast));
      }, 10000);
    }
  );
  return () => { unlisten.then(f => f()); };
}, []);
```

- [ ] **Step 2: Add toast rendering (use `toast` not `t` as loop variable)**

```tsx
{responseToasts.length > 0 && (
  <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
    {responseToasts.map((toast, i) => (
      <div key={i} className="bg-gray-800 border border-purple-600 rounded-xl px-4 py-3 shadow-lg max-w-xs">
        <div className="font-semibold text-sm">{t("polling.newInviteResponse")}</div>
        <div className="text-xs text-gray-400 mt-1">
          {toast.inviteeDeclaredName} {t("polling.respondedToYourInvite")}
        </div>
        <div className="flex gap-2 mt-2">
          <button
            className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1 rounded-md"
            onClick={() => {
              onOpenWorkspacePeers?.();
              setResponseToasts(prev => prev.filter(t2 => t2 !== toast));
            }}
          >
            {t("polling.viewInPeers")}
          </button>
          <button
            className="bg-transparent text-gray-400 border border-gray-600 text-xs px-3 py-1 rounded-md"
            onClick={() => setResponseToasts(prev => prev.filter(t2 => t2 !== toast))}
          >
            {t("common.dismiss", "Dismiss")}
          </button>
        </div>
      </div>
    ))}
  </div>
)}
```

**Wiring `onOpenWorkspacePeers`:** The "View in Peers" button needs to open the WorkspacePeersDialog, which is managed by `App.tsx` via `setShowWorkspacePeers`. Pass an `onOpenWorkspacePeers` callback prop from `App.tsx` to `WorkspaceView`. Read `App.tsx` to see how `WorkspaceView` is rendered and what props it receives — add `onOpenWorkspacePeers={() => setShowWorkspacePeers(true)}`.

- [ ] **Step 3: Add workspace-level polling**

`WorkspaceView` doesn't currently track whether there are relay peers. Add a new Tauri command `has_relay_peers` that checks the `sync_peers` table, OR check peers on mount:

```typescript
import { useRelayPolling } from "../hooks/useRelayPolling";

const [hasRelayPeers, setHasRelayPeers] = useState(false);

useEffect(() => {
  invoke<PeerInfo[]>("list_workspace_peers")
    .then(peers => setHasRelayPeers(peers.some(p => p.channelType === "relay")))
    .catch(() => {});
}, []);

useRelayPolling(hasRelayPeers);
```

- [ ] **Step 4: Type check and commit**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat(ui): add invite response toasts and workspace-level polling"
```

---

## Chunk 6: Integration & Wiring

### Task 12: Identity-level polling in App.tsx

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Add identity-level polling**

Read `App.tsx` to find where `unlockedIdentityUuid` is available (from `useWorkspaceLifecycle`).

```typescript
import { useIdentityPolling } from "./hooks/useIdentityPolling";
import type { AcceptedInviteInfo } from "./types";

const [hasRelayAccount, setHasRelayAccount] = useState(false);
const [hasWaitingInvites, setHasWaitingInvites] = useState(false);

useEffect(() => {
  if (!unlockedIdentityUuid) {
    setHasRelayAccount(false);
    setHasWaitingInvites(false);
    return;
  }
  invoke<boolean>("has_relay_credentials", { identityUuid: unlockedIdentityUuid })
    .then(setHasRelayAccount)
    .catch(() => setHasRelayAccount(false));
  invoke<AcceptedInviteInfo[]>("list_accepted_invites", { identityUuid: unlockedIdentityUuid })
    .then(invites => setHasWaitingInvites(invites.some(i => i.status === "waitingSnapshot")))
    .catch(() => setHasWaitingInvites(false));
}, [unlockedIdentityUuid]);

useIdentityPolling(unlockedIdentityUuid, hasRelayAccount, hasWaitingInvites);
```

Also pass `onOpenWorkspacePeers` prop to `WorkspaceView`:
```tsx
<WorkspaceView
  // ... existing props ...
  onOpenWorkspacePeers={() => setShowWorkspacePeers(true)}
/>
```

- [ ] **Step 2: Type check and commit**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat(app): wire identity-level polling in App.tsx"
```

### Task 13: Save AcceptedInvite when invitee responds

**Files:**
- Modify: `krillnotes-desktop/src/components/ImportInviteDialog.tsx`

- [ ] **Step 1: Add save_accepted_invite in TWO places**

Read `ImportInviteDialog.tsx`. The invite data is in `effectiveInviteData` and the identity UUID is `selectedUuid`. Add the `save_accepted_invite` call in:

1. **`doSendViaRelay()`** — after `send_invite_response_via_relay` succeeds and `setResponseRelayUrl(url)` is called:
```typescript
await invoke("save_accepted_invite", {
  identityUuid: selectedUuid,
  inviteId: effectiveInviteData.inviteId,
  workspaceId: effectiveInviteData.workspaceId,
  workspaceName: effectiveInviteData.workspaceName,
  inviterPublicKey: effectiveInviteData.inviterPublicKey,
  inviterDeclaredName: effectiveInviteData.inviterDeclaredName,
  responseRelayUrl: url,
});
```

2. **`handleRespond()`** — after `respond_to_invite` succeeds (file-based response path):
```typescript
await invoke("save_accepted_invite", {
  identityUuid: selectedUuid,
  inviteId: effectiveInviteData.inviteId,
  workspaceId: effectiveInviteData.workspaceId,
  workspaceName: effectiveInviteData.workspaceName,
  inviterPublicKey: effectiveInviteData.inviterPublicKey,
  inviterDeclaredName: effectiveInviteData.inviterDeclaredName,
  responseRelayUrl: null,
});
```

- [ ] **Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/ImportInviteDialog.tsx
git commit -m "feat(invite): save AcceptedInvite when invitee responds"
```

### Task 14: Create ReceivedResponse on manual import

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/invites.rs`

- [ ] **Step 1: Add ReceivedResponse creation**

Read `invites.rs` `import_invite_response` command. After the response is parsed and the `PendingPeer` is built, add:

```rust
// After building pending_peer, before returning:
let mut rrm = state.received_response_managers.lock().expect("Mutex poisoned");
if let Some(rr_mgr) = rrm.get_mut(&uuid) {
    let existing = rr_mgr.find_by_invite_and_invitee(
        invite_id_uuid,
        &pending_peer.invitee_public_key,
    ).map_err(|e| e.to_string())?;

    if existing.is_none() {
        // Get workspace info from the invite record
        let invite_record = im.get(invite_id_uuid).map_err(|e| e.to_string())?;
        if let Some(inv) = invite_record {
            let response = krillnotes_core::core::received_response::ReceivedResponse::new(
                invite_id_uuid,
                inv.workspace_id.clone(),
                inv.workspace_name.clone(),
                pending_peer.invitee_public_key.clone(),
                pending_peer.invitee_declared_name.clone(),
            );
            let _ = rr_mgr.save(&response);
        }
    }
}
```

- [ ] **Step 2: Build and commit**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`

```bash
git add krillnotes-desktop/src-tauri/src/commands/invites.rs
git commit -m "feat(invite): create ReceivedResponse on manual import_invite_response"
```

### Task 15: Update ReceivedResponse status after accept/send-snapshot

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Wire status updates in callbacks**

In the `onAcceptResponse` callback (triggered by PendingResponsesSection):
```typescript
// After AcceptPeerDialog completes:
await invoke("update_response_status", {
  identityUuid,
  responseId: response.responseId,
  status: "peerAdded",
});
```

In the `onSendSnapshot` callback (or after SendSnapshotDialog completes):
```typescript
await invoke("update_response_status", {
  identityUuid,
  responseId: response.responseId,
  status: "snapshotSent",
});
```

- [ ] **Step 2: Type check and commit**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

```bash
git add krillnotes-desktop/src/components/WorkspacePeersDialog.tsx
git commit -m "feat(ui): update ReceivedResponse status after accept/send-snapshot"
```

### Task 16: Final verification

- [ ] **Step 1: Run full Rust tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 3: Run dev build**

Run: `cd krillnotes-desktop && npm update && npm run tauri dev`
Expected: app launches, no console errors

- [ ] **Step 4: Final commit if fixups needed**

```bash
git add -A && git commit -m "fix: integration fixups for relay polling"
```
