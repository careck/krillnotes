# Relay Polling & Invite Status Tracking — Design

**Date:** 2026-03-17
**Status:** Draft

## Problem

After the relay invite workflow was streamlined for the invitee (PR #104), the inviter still has no easy way to see and act on responses to their invites, and the invitee has no visibility into whether they've received a snapshot yet. Both sides require manual file exchanges or URL sharing to progress. Additionally, there is no automatic polling — users must manually trigger sync operations.

## Goals

1. **Automatic receive-only polling** on relay channels at 1-minute intervals
2. **Invitee view** — see accepted invites and their status (waiting for snapshot / workspace created) in the Identity Manager dialog
3. **Inviter view** — see received invite responses and their status (pending / peer added / snapshot sent) in the Workspace Peers dialog
4. **Toast notifications** — notify the inviter when a new response arrives via polling

## Non-Goals

- Automatic sending (deltas, snapshots) — sending remains manual to avoid network chatter
- Folder-channel polling (future work)
- Cross-relay federation

## Architecture

### Core Principle: Maximum Logic in `krillnotes-core`

The Rust core library must remain Tauri-independent and reusable for future targets (mobile via Kotlin Multiplatform, web, headless). All polling logic, bundle processing, and state management lives in core. The Tauri layer is a thin adapter that calls core functions and emits platform-specific events.

### Two Polling Contexts

```
┌─────────────────────────────────────────────┐
│  Identity-level polling (invitee)           │
│  Trigger: identity unlocked                 │
│           + relay account registered        │
│           + accepted invites in             │
│             WaitingSnapshot status           │
│  Receives: snapshots only                   │
│  Polls: all relay accounts for identity     │
│  Scope: workspace IDs from accepted invites │
└─────────────────────────────────────────────┘

┌─────────────────────────────────────────────┐
│  Workspace-level polling (inviter + sync)   │
│  Trigger: workspace opens with              │
│           relay-configured peers            │
│  Receives: deltas + invite responses        │
│  No snapshots (workspace already exists)    │
│  Scope: single workspace ID                 │
└─────────────────────────────────────────────┘
```

Both use 60-second intervals. The frontend controls the timer (`setInterval`); core provides pure request/response functions.

### Relationship to Existing `SyncEngine::poll()` and `poll_sync` Command

The existing `SyncEngine::poll()` performs a full inbound+outbound cycle (receive bundles, apply them, then send deltas to all peers). The existing `poll_sync` Tauri command invokes this and remains the **manual full sync** path.

The new polling functions are **receive-only** and run automatically on a timer. They supplement, not replace, `poll_sync`:

| | `poll_sync` (existing) | `poll_receive_workspace` (new) | `poll_receive_identity` (new) |
|---|---|---|---|
| **Trigger** | Manual (user-initiated) | Automatic (60s interval) | Automatic (60s interval) |
| **Direction** | Send + receive | Receive only | Receive only |
| **Bundle modes** | All (delta, snapshot) | delta + accept | snapshot only |
| **Scope** | Single workspace | Single workspace | All accepted invites for identity |

To avoid duplicate `list_bundles` calls when the user manually triggers `poll_sync` close to an automatic poll, the Tauri layer should track the last poll timestamp and skip the automatic poll if a manual poll ran within the last 30 seconds.

**Note on `SyncEngine::poll()` and `Accept` bundles:** The existing `SyncEngine::poll()` encounters `Accept` mode bundles but currently drops them (falls through to a catch-all arm). The new workspace-level polling adds handling for `Accept` bundles. The implementation should extend `SyncEngine` to support Accept bundle processing via a `ReceivedResponseManager` in `SyncContext`, so that both manual and automatic polling handle invite responses consistently.

## Data Models

### AcceptedInvite (invitee side — new)

Tracks invites the user has accepted but may not yet have a workspace for.

```rust
pub struct AcceptedInvite {
    pub invite_id: Uuid,
    pub workspace_id: String,
    pub workspace_name: String,
    pub inviter_public_key: String,       // base64 Ed25519
    pub inviter_declared_name: String,
    pub accepted_at: DateTime<Utc>,
    pub response_relay_url: Option<String>, // set if response was sent via relay
    pub status: AcceptedInviteStatus,
    pub workspace_path: Option<String>,     // set once workspace is created
}

pub enum AcceptedInviteStatus {
    WaitingSnapshot,
    WorkspaceCreated,
}
```

**Storage:** `identities/<uuid>/accepted_invites/<invite_id>.json` (plaintext JSON, per identity)

**Manager:** `AcceptedInviteManager` — follows the `InviteManager` pattern (directory + direct file I/O, no in-memory cache). These records are plaintext and infrequently accessed, so a cache is unnecessary.

### ReceivedResponse (inviter side — new)

Tracks responses received to invites the user has sent.

```rust
pub struct ReceivedResponse {
    pub response_id: Uuid,
    pub invite_id: Uuid,
    pub workspace_id: String,
    pub workspace_name: String,
    pub invitee_public_key: String,        // base64 Ed25519
    pub invitee_declared_name: String,
    pub received_at: DateTime<Utc>,
    pub status: ReceivedResponseStatus,
}

pub enum ReceivedResponseStatus {
    Pending,       // response received, no action taken yet
    PeerAdded,     // peer record created, snapshot not yet sent
    SnapshotSent,  // initial snapshot delivered
}
```

**Storage:** `identities/<uuid>/invite_responses/<response_id>.json` (plaintext JSON, per identity)

**Manager:** `ReceivedResponseManager` — same `InviteManager` pattern (directory + direct file I/O, no cache).

## Core Polling Functions

### Workspace-level: `receive_poll_workspace()`

Extends `SyncEngine` by adding Accept bundle handling to the existing receive path. The new function is a standalone receive-only poll that reuses `SyncEngine`'s bundle fetching and delta application, but adds invite response processing.

```rust
pub fn receive_poll_workspace(
    workspace: &mut Workspace,
    relay_client: &RelayClient,
    identity: &UnlockedIdentity,
    received_response_manager: &mut ReceivedResponseManager,
    invite_manager: &InviteManager,
) -> Result<WorkspacePollResult, KrillnotesError>;
```

**Note:** This is a standalone function, not a method on `Workspace`, because it orchestrates multiple concerns (relay transport, response management, invite lookup) that don't belong on the Workspace struct.

Steps:
1. Call `relay_client.list_bundles()` — returns all pending bundles for the authenticated account
2. Filter results client-side by `workspace_id` matching this workspace (using `BundleMeta.workspace_id`)
3. For each matching bundle, fetch the full bundle and inspect the `SwarmMode` from the parsed header:
   - `SwarmMode::Delta` → apply via existing SyncEngine delta application logic → record in `applied_bundles`
   - `SwarmMode::Accept` → parse invite response payload, validate against known invites via `invite_manager`:
     - If matching invite found and not revoked/expired: create `ReceivedResponse` via manager → record in `new_responses`
     - If no matching invite found, invite is revoked, or invite is expired: log a warning, acknowledge the bundle to prevent re-processing, record in `errors`
   - `SwarmMode::Snapshot` / `SwarmMode::Invite` / other → skip (not handled by workspace-level polling), do not acknowledge
4. Acknowledge all processed bundles
5. Return `WorkspacePollResult`

```rust
pub struct WorkspacePollResult {
    pub applied_bundles: Vec<AppliedBundle>,
    pub new_responses: Vec<ReceivedResponse>,
    pub errors: Vec<PollError>,
}

pub struct AppliedBundle {
    pub peer_device_id: String,
    pub mode: String,       // "delta"
    pub op_count: usize,
}

pub struct PollError {
    pub bundle_id: Option<String>,
    pub error: String,
}
```

### Identity-level: `receive_poll_identity()`

A standalone function (not on Workspace, since no workspace exists yet):

```rust
pub struct RelayConnection {
    pub account: RelayAccount,
    pub client: RelayClient,
}

pub fn receive_poll_identity(
    relay_connections: &[RelayConnection],
    accepted_invites: &[AcceptedInvite],   // only WaitingSnapshot ones
) -> Result<IdentityPollResult, KrillnotesError>;
```

Steps:
1. Collect workspace IDs from accepted invites in `WaitingSnapshot` status
2. For each relay connection, for each workspace ID:
   - Register mailbox for the workspace ID if not already registered. **Note:** The invitee is registering a mailbox on THEIR relay account for the INVITER's workspace ID. This is how the relay routes the snapshot bundle to the correct recipient — the inviter uploads the snapshot targeting the invitee's device key, and the invitee's mailbox registration for that workspace ID makes the bundle visible to them.
   - Call `relay_client.list_bundles()` and filter results client-side by `workspace_id` and `BundleMeta.mode == "snapshot"`
   - Fetch matching bundles one at a time (to limit memory usage — snapshots can be large)
   - Write each snapshot to a temporary file and return the path, rather than holding all bytes in memory
   - Acknowledge processed bundles
3. Return `IdentityPollResult`

```rust
pub struct IdentityPollResult {
    pub received_snapshots: Vec<ReceivedSnapshot>,
    pub errors: Vec<PollError>,
}

pub struct ReceivedSnapshot {
    pub workspace_id: String,
    pub invite_id: Uuid,
    pub snapshot_path: PathBuf,        // temp file with snapshot data
    pub sender_device_key: String,
}
```

**Memory note:** Snapshots can be up to 10 MB each (relay limit). To avoid holding multiple large bundles in memory simultaneously, the function processes and persists snapshots one at a time, returning file paths rather than raw bytes.

## Tauri Layer

### New Commands

**Polling:**
- `poll_receive_workspace(window, state)` → calls `receive_poll_workspace()`, emits events, returns `Ok(())`
- `poll_receive_identity(identity_uuid)` → calls `receive_poll_identity()`, emits events, returns `Ok(())`

**AcceptedInvite management:**
- `list_accepted_invites(identity_uuid)` → `Vec<AcceptedInviteInfo>`
- `save_accepted_invite(identity_uuid, invite_id, workspace_id, workspace_name, inviter_key, inviter_name, response_relay_url?)` — called when invitee sends a response
- `update_accepted_invite_status(identity_uuid, invite_id, status, workspace_path?)` — called when workspace is created from snapshot

**ReceivedResponse management:**
- `list_received_responses(identity_uuid, workspace_id?)` → `Vec<ReceivedResponseInfo>` — optionally filtered by workspace
- `update_response_status(identity_uuid, response_id, status)` — called when peer is added or snapshot sent
- `dismiss_response(identity_uuid, response_id)` — delete/ignore a response

### Tauri Events

Emitted by polling commands after processing `PollResult`:

| Event | Payload | Emitted by |
|-------|---------|------------|
| `invite-response-received` | `ReceivedResponseInfo` | `poll_receive_workspace` |
| `sync-bundle-applied` | `{ peerDeviceId, mode, opCount }` | `poll_receive_workspace` |
| `snapshot-received` | `{ workspaceId, inviteId, snapshotPath }` | `poll_receive_identity` |
| `poll-error` | `{ error: String }` | both |

### AppState Additions

```rust
pub struct AppState {
    // ... existing fields ...
    pub accepted_invite_managers: Arc<Mutex<HashMap<Uuid, AcceptedInviteManager>>>,
    pub received_response_managers: Arc<Mutex<HashMap<Uuid, ReceivedResponseManager>>>,
}
```

## Frontend

### Polling Lifecycle

**Workspace-level (in `WorkspaceView`):**
```
onMount:
  if workspace has relay-configured peers:
    invoke("poll_receive_workspace")          // immediate first poll
    interval = setInterval(60s):
      invoke("poll_receive_workspace")
onUnmount:
  clearInterval(interval)
```

**Identity-level (in `App.tsx` or top-level context):**
```
on identity unlock OR relay account registration:
  if identity unlocked
     AND relay account exists
     AND accepted invites in WaitingSnapshot exist:
    invoke("poll_receive_identity")           // immediate first poll
    interval = setInterval(60s):
      invoke("poll_receive_identity")

on accepted invite status change (all resolved):
  clearInterval(interval)
```

### Rate Limit Protection for Multi-Workspace Scenarios

The relay rate-limits `list_bundles` to 1 call per 60 seconds per account. If multiple workspaces are open using the same relay account, each workspace's polling interval would compete for the same rate limit.

**Solution:** The Tauri layer maintains a per-relay-account timestamp of the last `list_bundles` call. Before calling core's `receive_poll_workspace()`, the Tauri command checks if the account was polled in the last 55 seconds. If so, it skips this cycle and returns an empty result. This ensures at most one `list_bundles` call per 60 seconds per relay account, regardless of how many workspaces are open.

For identity-level polling using multiple relay accounts, each account is polled independently (each has its own rate limit).

### UI: Invitee View — Identity Manager Dialog

**Location:** New "Accepted Invites" section in the Identity Manager dialog.

**Content:** List of accepted invites for the active identity, each showing:
- Workspace name
- Inviter name
- Accepted date
- Status badge: "Waiting for snapshot" (amber) or "Workspace created" (green)
- "Open" button when workspace is created

**Data source:** `list_accepted_invites(identity_uuid)`

**Live updates:** Listens for `snapshot-received` event to refresh the list.

### UI: Inviter View — Workspace Peers Dialog

**Location:** New "Pending Invite Responses" section in the Workspace Peers dialog, above or below the existing active peers list.

**Content:** List of received responses for this workspace, each showing:
- Invitee name + fingerprint
- Which invite they responded to
- Received date
- Status: "Action needed" (amber, with left border highlight) / "Peer added" (blue) / "Snapshot sent" (green)
- Action button: "Accept & Send Snapshot" (pending) or "Send Snapshot" (peer added)

**Data source:** `list_received_responses(identity_uuid, workspace_id)`

**Live updates:** Listens for `invite-response-received` event to refresh the list.

**Actions:**
- "Accept & Send Snapshot" → launches existing AcceptPeer → PostAccept → SendSnapshot dialog flow, then updates status to `SnapshotSent`
- "Send Snapshot" → launches SendSnapshot dialog directly (peer already added)

### UI: Toast Notifications

**Location:** `WorkspaceView`, bottom-right overlay (extends existing migration toast pattern).

**Trigger:** `invite-response-received` event

**Content:**
- Title: "New invite response"
- Body: "{invitee name} responded to your invite"
- Actions: "View in Peers" (opens Workspace Peers dialog) / "Dismiss"
- Auto-dismiss after 10 seconds

## Integration with Existing Flows

### ImportInviteDialog (invitee responds)

After successfully building and sending a response, also call `save_accepted_invite()` to create the `AcceptedInvite` record. This is the moment the invitee "accepts" and starts waiting for a snapshot.

### import_invite_response (inviter receives manually)

The existing manual import flow should also create a `ReceivedResponse` record, keeping both manual and polled responses in the same list.

### AcceptPeerDialog / SendSnapshotDialog

After the inviter accepts a peer and sends a snapshot, update the corresponding `ReceivedResponse` status to `PeerAdded` then `SnapshotSent`.

## Key Constraints

- **One identity → many relay accounts.** Identity-level polling must iterate over ALL relay accounts for the identity.
- **Relay rate limit:** `list_bundles` is limited to 1 call per 60 seconds per account. The 60-second polling interval respects this. Multi-workspace scenarios are protected by per-account timestamp tracking in the Tauri layer.
- **Receive-only:** Polling never sends bundles. Sending remains a manual action.
- **No Tauri deps in core:** All polling logic, state management, and bundle processing is in `krillnotes-core`.
- **`RelayClient.list_bundles()` returns all bundles for the account.** Client-side filtering by `workspace_id` and `SwarmMode`/`BundleMeta.mode` is required. The relay API does not support server-side filtering.
