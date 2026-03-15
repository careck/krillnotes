# Sync Engine Design — Multi-Channel Transport for Krillnotes

**Date:** 2026-03-14

**Status:** DESIGN

**Companion to:** KrillNotes Swarm Design v0.7, KrillNotes Relay Design Concept v0.1

---

## 1. Overview

The sync engine adds automated transport to the existing invite → snapshot → delta pipeline. Currently all .swarm bundle exchange is manual (user saves/opens files). This design introduces three transport channels — relay, folder, and manual — that automate bundle delivery while keeping the core protocol unchanged.

### Design Principles

- **krillnotes-core owns the sync logic** — no Tauri dependency. Reusable by CLI, mobile, and web clients.
- **Host-driven execution** — the core library exposes `poll()` but never spawns threads or timers. The host app decides when and how often to call it.
- **Only poll open workspaces** — sync runs against workspaces with an unlocked identity and active UI. This guarantees the decryption key is available and the user can respond to alerts.
- **Transport-agnostic pipeline** — all three channels feed the same ingestion pipeline. The channel is purely about getting bytes from A to B.

---

## 2. Module Structure

```
krillnotes-core/src/core/
├── sync/
│   ├── mod.rs             # SyncEngine, SyncEvent enum, public API
│   ├── channel.rs         # SyncChannel trait + ChannelType enum
│   ├── relay/
│   │   ├── mod.rs         # RelayChannel (implements SyncChannel)
│   │   ├── client.rs      # HTTP client (reqwest::blocking) — REST calls to relay API
│   │   └── auth.rs        # Registration, PoP challenge, login, session management
│   ├── folder.rs          # FolderChannel (implements SyncChannel)
│   └── manual.rs          # ManualChannel (excluded from poll, marker only)
```

### Dependency: `reqwest`

The relay channel requires an HTTP client. `reqwest::blocking` is added directly to `krillnotes-core` behind a `relay` Cargo feature flag. Consumers that don't need relay support (e.g., future mobile targets) can opt out. A future refactor may extract this behind a plugin interface, but for three known channels this is the right trade-off.

### `manual.rs` — Marker Module

`manual.rs` does not implement the `SyncChannel` trait. It contains the `ChannelType::Manual` documentation, any validation helpers for manual-channel peer configuration, and the outbox path utilities used by the host app's "Generate delta" UI action.

---

## 3. SyncChannel Trait (Hybrid Approach)

The trait covers the universal sync operations used by the dispatch loop. Channel-specific setup (relay auth, folder path validation) stays as concrete methods on each channel struct.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Relay,
    Folder,
    Manual,
}

/// Lightweight view of a peer registry entry, passed to channel methods.
/// Projection of the extended SyncPeer struct after the S1 migration.
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
    /// Relay: DELETE /bundles/{id}. Folder: delete file.
    fn acknowledge(&self, bundle_ref: &BundleRef) -> Result<(), KrillnotesError>;

    /// Channel type identifier.
    fn channel_type(&self) -> ChannelType;
}
```

### What the trait does NOT cover

- Relay registration, login, PoP challenge flow
- Relay credential storage and session management
- Relay mailbox management
- Folder path validation
- Password reset flow

These are methods on the concrete channel types, called by the host app's configuration UI.

---

## 4. Peer Registry Extension

### New Columns on `sync_peers`

```sql
ALTER TABLE sync_peers ADD COLUMN channel_type TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE sync_peers ADD COLUMN channel_params TEXT NOT NULL DEFAULT '{}';
ALTER TABLE sync_peers ADD COLUMN sync_status TEXT NOT NULL DEFAULT 'idle';
ALTER TABLE sync_peers ADD COLUMN sync_status_detail TEXT;
ALTER TABLE sync_peers ADD COLUMN last_sync_error TEXT;
```

| Column | Purpose |
|--------|---------|
| `channel_type` | `'relay'`, `'folder'`, or `'manual'` |
| `channel_params` | JSON blob — channel-specific local configuration |
| `sync_status` | `'idle'`, `'syncing'`, `'error'`, `'auth_expired'` |
| `sync_status_detail` | Human-readable context (e.g. "Relay unreachable") |
| `last_sync_error` | Last error message, cleared on successful sync |

### Channel Params by Type

- **Relay:** `{ "relay_url": "https://swarm.krillnotes.org" }`
- **Folder:** `{ "path": "/Users/carsten/Dropbox/shared-with-bob" }`
- **Manual:** `{}`

### Sync Status Enum

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Idle,
    Syncing,
    Error,
    AuthExpired,
}
```

### Local-Only Invariant

The `sync_peers` table (including all new columns) is strictly local device state. It is never included in:
- Archive exports (`export.rs`)
- Snapshot bundles (`swarm/snapshot.rs`)
- Delta bundles (`swarm/delta.rs`)

Each device configures its own channels independently. Bob's desktop might use relay while Bob's phone uses manual for the same peer.

### Frontend Exposure

`PeerInfo` (already exposed to TypeScript) gains `channel_type: string`, `sync_status: string`, and `sync_status_detail: string | null` fields. The `WorkspacePeersDialog` can display per-peer sync status and channel info.

---

## 5. Relay Credential Storage

### File Location

```
~/.config/krillnotes/relay/<identity_uuid>.json
```

One file per identity. Encrypted with the identity's HKDF-derived symmetric key using AES-256-GCM, consistent with the contact storage pattern in `contact.rs` (which uses `contacts_key()` from `UnlockedIdentity`).

### Decrypted Payload

```rust
pub struct RelayCredentials {
    /// Relay server URL
    pub relay_url: String,
    /// Account email address
    pub email: String,
    /// Session token from login or register/verify
    pub session_token: String,
    /// Session expiry (30-day lifetime)
    pub session_expires_at: DateTime<Utc>,
    /// The device key registered with this relay account
    pub device_public_key: String,
}
```

### Lifecycle

1. **Registration** — User provides relay URL, email, password. App runs PoP flow (Ed25519 → X25519, decrypt server challenge). On success, stores credentials encrypted to identity key.
2. **Session use** — On each poll, relay client reads decrypted credentials from memory (identity is unlocked). Attaches `session_token` as Bearer header.
3. **Session expiry** — Relay returns 401 → sync engine sets `sync_status = AuthExpired` on all relay-channel peers for this identity, emits `SyncEvent::AuthExpired`.
4. **Re-login** — User provides password, app calls `POST /auth/login`, receives new token, updates credential file.
5. **Password reset** — Fully in-app: `POST /auth/reset-password` (email pre-filled from stored credentials) → user enters token from email + new password → `POST /auth/reset-password/confirm` → immediate re-login.
6. **Identity lock** — Credentials wiped from memory. No relay polling for locked identities.

### No Password Storage

The relay account password is never persisted. Only the session token (30-day lifetime) is stored. Password is used during registration, login, and password reset, then discarded.

---

## 6. Relay Client (HTTP Layer)

### Structure

```rust
pub struct RelayClient {
    http: reqwest::blocking::Client,
    base_url: String,
    session_token: Option<String>,
}
```

Using `reqwest::blocking` since `krillnotes-core` is synchronous (Workspace methods, SQLite access). The host app wraps calls in background threads as needed.

### Methods

**Auth:**
- `register(email, password, identity_uuid, device_public_key)` → `RegisterChallenge`
- `register_verify(device_public_key, nonce)` → `SessionToken`
- `login(email, password)` → `SessionToken`
- `logout()`
- `reset_password(email)`
- `reset_password_confirm(token, new_password)`

**Account & Devices:**
- `get_account()` → `AccountInfo`
- `delete_account()`
- `add_device(device_public_key)` → `DeviceChallenge`
- `verify_device(device_public_key, nonce)` → success
- `remove_device(device_key)` → success

**Mailboxes:**
- `ensure_mailbox(workspace_id)` — idempotent, creates if not exists
- `list_mailboxes()` → `Vec<MailboxInfo>`

Note: the app never deletes mailboxes. Mailbox cleanup is server-side policy (inactivity expiry, admin action, or future user-initiated removal via relay UI).

**Bundles:**
- `upload_bundle(bundle_bytes)` → `BundleId`
- `list_bundles()` → `Vec<BundleMeta>`
- `download_bundle(bundle_id)` → `Vec<u8>`
- `delete_bundle(bundle_id)`

**Invites:**
- `create_invite(payload_base64, expires_at)` → `InviteInfo { token, url }`
- `list_invites()` → `Vec<InviteInfo>`
- `fetch_invite(token)` → `InvitePayload`
- `delete_invite(token)`

### Error Mapping

| HTTP Status | KrillnotesError variant |
|-------------|------------------------|
| 401 | `RelayAuthExpired` |
| 429 | `RelayRateLimited` |
| 404 / 410 | `RelayNotFound` |
| 5xx / network | `RelayUnavailable` |

### Proof-of-Possession Challenge Resolution

Registration and device-add flows require decrypting a server challenge:

1. `register()` returns `{ encrypted_nonce, server_public_key }`
2. Client converts the device's Ed25519 secret key to X25519
3. Derives shared secret with the server's ephemeral X25519 public key
4. Decrypts nonce using NaCl `crypto_box_open` (X25519 + XSalsa20-Poly1305)
5. Sends plaintext nonce to `register_verify()`

**Crypto note:** The existing `swarm/crypto.rs` uses Ed25519→X25519 conversion + HKDF + AES-256-GCM, which is a different cipher than NaCl `crypto_box` (XSalsa20-Poly1305). The Ed25519→X25519 key conversion is reusable, but the PoP flow requires a `crypto_box` implementation to match the PHP relay's `ext-sodium`. This needs the `crypto_box` crate (or equivalent) as a new dependency in `krillnotes-core`.

---

## 7. SyncEngine — The Dispatch Loop

### Core API

```rust
pub struct SyncEngine {
    channels: HashMap<ChannelType, Box<dyn SyncChannel>>,
}

/// Context needed for sync operations. Bundles the references that
/// generate_delta() and apply_delta() require beyond the Workspace itself.
pub struct SyncContext<'a> {
    pub signing_key: &'a SigningKey,
    pub contact_manager: &'a mut ContactManager,
    pub workspace_name: &'a str,
    pub sender_display_name: &'a str,
}

impl SyncEngine {
    /// Called by the host app on a timer for each open workspace.
    /// Returns Ok with a list of sync events, or Err if polling cannot
    /// proceed at all (e.g., database error querying sync_peers).
    /// Per-peer errors are reported as SyncEvent variants, not as Err.
    pub fn poll(
        &self,
        workspace: &mut Workspace,
        ctx: &mut SyncContext<'_>,
    ) -> Result<Vec<SyncEvent>, KrillnotesError>;
}
```

The host app owns the timer. `krillnotes-core` never spawns threads or runs its own scheduler.

The `SyncContext` bundles the additional references that `generate_delta()` and `apply_delta()` (in `swarm/sync.rs`) require: the identity's `SigningKey`, a mutable `ContactManager`, the workspace display name, and the sender's display name. The host app constructs this from the unlocked identity.

**Ownership note:** `Workspace` and `ContactManager` are owned separately by the host app (Workspace in AppState's workspace map, ContactManager on the unlocked identity). This avoids borrow-checker conflicts — `poll()` borrows `&mut Workspace` and `&mut ContactManager` from different owners.

### Ownership: Who Creates the SyncEngine?

One `SyncEngine` per identity, owned by the host app alongside the identity's unlocked state. When an identity is unlocked, the host constructs a `SyncEngine` and registers the appropriate channels (relay if credentials exist, folder/manual as configured). When the identity is locked, the engine is dropped.

In the Tauri desktop app, the engine would live in `AppState` keyed by identity UUID, alongside the existing workspace and identity maps. The CLI would hold it in its own runtime context.

### Poll Behavior

**Outbound (send):**

1. Query `sync_peers` for all peers in this workspace.
2. For each peer with `channel_type != manual`:
   - Check if new operations exist since `last_sent_op`.
   - If yes: call `generate_delta()` from `swarm/sync.rs` (requires `&mut Workspace`, `&SigningKey`, `&ContactManager`, workspace name, sender display name).
   - Call `channel.send_bundle(peer, bundle_bytes)`.
   - On success: update `last_sent_op`, set `sync_status = Idle`, update `last_sync`.
   - On failure: set `sync_status = Error` (or `AuthExpired`), record error, emit event.

**Inbound (receive):**

3. For each active channel type used by at least one peer in this workspace:
   - Call `channel.receive_bundles(workspace_id)`.
   - For each received bundle:
     - Read the `SwarmHeader` to determine the bundle mode (delta, snapshot, invite, accept).
     - **Delta:** call `apply_delta()` from `swarm/sync.rs` (requires `&mut Workspace`, `&SigningKey`, `&mut ContactManager`).
     - **Snapshot:** call `import_snapshot_json()` on `Workspace`.
     - **Invite/Accept:** these are not expected during automated polling (they go through the invitation UI), but if received, emit `SyncEvent::UnexpectedBundleMode` and skip.
     - On success: `channel.acknowledge(bundle_ref)`, update `last_received_op` + `last_sync` on the sending peer.
     - On failure: emit `SyncEvent::IngestError`, do NOT acknowledge (bundle stays pending for retry).

**Manual channel is skipped** — `poll()` never touches manual-channel peers. They sync through explicit user action via SwarmOpenDialog.

### Per-Peer Delta Cost

Each outbound poll generates one delta bundle per peer. For workspaces with many relay peers, this means multiple `generate_delta()` calls per poll cycle. This is acceptable for the expected peer counts (small teams, family sharing). If high-peer-count workspaces become common, a future optimization could batch encryption for multiple recipients in a single pass.

### No-Op Fast Path

If a workspace has no non-manual peers, or no peers with pending operations, `poll()` returns an empty event list immediately.

---

## 8. SyncEvent Callback System

### Event Enum

All events carry a `workspace_id` so the host app can route events to the correct UI context (e.g., the right Tauri window).

```rust
pub enum SyncEvent {
    /// Successfully sent delta to a peer
    DeltaSent { workspace_id: String, peer_device_id: String, op_count: usize },
    /// Successfully received and applied a bundle
    BundleApplied { workspace_id: String, peer_device_id: String, op_count: usize },
    /// Relay session expired, user needs to re-authenticate
    AuthExpired { relay_url: String },
    /// Send or receive failed (non-auth)
    SyncError { workspace_id: String, peer_device_id: String, error: String },
    /// Bundle received but failed to apply
    IngestError { workspace_id: String, peer_device_id: String, error: String },
    /// Received a bundle mode not expected during polling (invite/accept)
    UnexpectedBundleMode { workspace_id: String, mode: String },
}
```

### New KrillnotesError Variants

The relay client introduces four new error variants in `error.rs`:

- `RelayAuthExpired` — 401 from relay, session token invalid or expired
- `RelayRateLimited` — 429 from relay, poll interval too short
- `RelayNotFound` — 404/410 from relay, resource missing or expired
- `RelayUnavailable` — 5xx or network error, relay unreachable

Each needs a `user_message()` implementation and a `From<reqwest::Error>` conversion (gated behind the `relay` feature flag).

### Delivery Mechanism

`poll()` returns `Vec<SyncEvent>` synchronously. The host app can also register a callback for events that fire during other operations (e.g., manual .swarm import):

```rust
pub type SyncEventCallback = Box<dyn Fn(SyncEvent) + Send + Sync>;

impl Workspace {
    pub fn set_sync_event_handler(&mut self, callback: SyncEventCallback);
}
```

### Host App Mapping (Tauri Example)

| Event | Tauri Action |
|-------|-------------|
| `BundleApplied` | Emit `sync-bundle-applied` → frontend refreshes tree |
| `AuthExpired` | Emit `sync-auth-expired` → frontend shows re-login prompt |
| `SyncError` | Emit `sync-error` → frontend shows warning toast |
| `DeltaSent` | Emit `sync-delta-sent` → frontend updates peer status |
| `IngestError` | Emit `sync-ingest-error` → frontend shows error detail |

---

## 9. Folder Channel

### Concept

The folder channel writes and reads .swarm files in a shared directory (Dropbox, Syncthing, NAS mount). The folder path is local to each device — two peers agree out-of-band on a shared folder concept, but each configures their own filesystem path.

### File Naming Convention

```
<identity_uuid_short>_<device_key_short>_<timestamp>_<uuid>.swarm
```

Identity + device key in the filename ensures correct filtering when multiple identities operate on the same machine.

### Operations

**send_bundle:** Write bundle bytes to `<path>/<identity_short>_<device_short>_<timestamp>_<uuid>.swarm`.

**receive_bundles:** Scan `<path>` for `.swarm` files. Read each header. Return those where `(source_identity, source_device_key) != (self_identity, self_device_key)`. This correctly handles multiple identities on the same device.

**acknowledge:** Delete the file after successful ingestion.

### Edge Cases

- **Folder not found** → `SyncError`, set `sync_status = Error` with detail "Folder not found"
- **Permission denied** → same treatment
- **Partially written file** (sync tool still transferring) → skip files that fail to parse, pick them up next poll

### Path Configuration

The folder path in `channel_params` is purely local:
```json
{ "path": "/Users/carsten/Dropbox/shared-with-bob" }
```

This is never sent to peers. Bob's path might be `D:\Dropbox\shared-with-carsten`. Both point to the same logical folder via their sync tool. The folder setup (Dropbox invite, shared drive mapping) happens entirely out-of-band.

---

## 10. Manual Channel

The manual channel is the absence of automation. `channel_type = 'manual'` on a peer is a marker that tells `poll()` to skip this peer.

**Outbound:** The user clicks "Generate delta" in the peers dialog. The app generates the bundle and saves it via a file save dialog. The user delivers it however they want (email, USB, AirDrop).

**Inbound:** The user opens a .swarm file via SwarmOpenDialog. The existing `apply_delta()` / `import_snapshot_json()` pipeline handles it.

No code in `manual.rs` implements the `SyncChannel` trait — manual is explicitly excluded from the dispatch loop. See Section 2 for what `manual.rs` contains.

---

## 11. Invitation Flow via Relay

### Current Flow (Manual Only)

1. Inviter generates invite → saves `.swarm` file
2. Shares file out-of-band
3. Recipient imports via SwarmOpenDialog → accept .swarm generated
4. Recipient shares accept .swarm back out-of-band
5. Inviter imports accept → handshake complete, snapshot follows

### Enhanced Flow (Unified Dialog)

The `CreateInviteDialog` gains a distribution step after generating the invite.

**Step 1:** Existing invite generation (workspace, role, expiry) — unchanged.

**Step 2:** "How do you want to share this?"

- **"Copy link"** — visible only if the current identity has relay credentials. The app calls `relay_client.create_invite(payload_base64, expires_at)`, receives `{ token, url }`, copies URL to clipboard.
- **"Save .swarm file"** — existing behavior, always available.
- **Both** — upload to relay AND save file.

### Recipient Side

The recipient pastes the relay invite URL into an "Import from relay" field in `ImportInviteDialog`. The app calls `GET /invites/{token}` with `Accept: application/json`, receives the encrypted blob, decrypts, and displays workspace info. Recipient accepts as normal.

### Accept Reply Path

- **Both have relay accounts:** The accept .swarm is uploaded as a regular bundle via `POST /bundles`, routed to the inviter's device key. The inviter's next `poll()` picks it up.
- **Recipient has no relay:** The accept is saved as a .swarm file for manual delivery.

### Channel Constraint During Accept

The inviter's invite payload includes `reply_channels` — the set of channels the inviter offers. The acceptor picks from this list; they cannot propose a channel the inviter didn't offer.

- Inviter has relay → offers `relay` (with specific URL) + `manual`
- Inviter has no relay → offers `folder` + `manual`, or just `manual`

If the inviter specifies a particular relay URL, the acceptor must use that URL or pick a different offered channel. They cannot counter-propose a different relay URL.

### Channel Preference in Accept Payload

```rust
pub struct ChannelPreference {
    pub channel_type: ChannelType,
    /// If relay: the inviter's relay URL (echoed back to confirm)
    pub relay_url: Option<String>,
    // No folder path — folder paths are local-only, configured independently per device
}
```

### Existing Struct Extensions

The invitation and accept flows require new fields on existing structs:

- **`InviteParams`** (`swarm/invite.rs`): Add `reply_channels: Vec<ReplyChannel>` listing the channels the inviter offers. Use `#[serde(default)]` for backward compatibility with older invite bundles that lack this field.
- **`AcceptParams`** (or the accept payload struct): Add `channel_preference: ChannelPreference` declaring the acceptor's chosen channel. Use `#[serde(default)]` — older accepts without this field default to manual.
- **`SwarmHeader`** (`swarm/header.rs`): No changes needed — the header already carries mode and identity info. Channel data lives in the encrypted payload, not the header.

### Invite Management

`InviteManagerDialog` gains a column showing distribution method (relay URL vs file) and download count (from `GET /invites`). Revoking a relay invite calls `DELETE /invites/{token}`.

---

## 12. Relay Mailbox Lifecycle

### App Responsibility: Registration Only

The app's sole mailbox operation is `POST /mailboxes` — registering a new mailbox when a relay-channel peer is added to a workspace. `ensure_mailbox()` is idempotent, safe to call on every poll.

The app never deletes mailboxes. Mailbox cleanup is entirely server-side policy:

- User requests removal (mechanism TBD by relay service)
- Inactivity expiry (configurable by relay operator)
- Admin removes user from the system

This keeps the app simple and prevents it from making irreversible decisions about relay state.

---

## 13. Error Handling and Offline Resilience

### Per-Peer Status

Each peer's `sync_status` is updated on every poll attempt:

| Status | Meaning | UI Treatment |
|--------|---------|-------------|
| `idle` | Last sync succeeded or no sync attempted | Normal display |
| `syncing` | Currently in a send/receive cycle | Spinner or activity indicator |
| `error` | Last attempt failed (network, folder missing, etc.) | Warning icon + detail on hover |
| `auth_expired` | Relay returned 401 | Prompt to re-authenticate |

`sync_status_detail` provides human-readable context. `last_sync_error` stores the full error message.

### Relay Offline

If the relay is unreachable, all relay-channel peers get `sync_status = Error`. The `last_sync` timestamp goes stale, naturally signaling the issue. Next poll retries automatically. No bundles are lost — they accumulate on the relay server (up to 30-day retention).

### Folder Missing

If a watched folder disappears (unmounted drive, Dropbox logged out), the folder-channel peer gets `sync_status = Error` with detail "Folder not found". Next poll retries.

### Session Expiry

Relay returns 401 → `SyncEvent::AuthExpired` emitted → host app prompts re-login. All relay-channel peers for that identity get `sync_status = AuthExpired`. After re-login, next poll resumes normally.

### Bundle Ingestion Failure

If a received bundle fails to apply (bad signature, decryption failure, unknown operation type), the bundle is NOT acknowledged. It stays pending on the channel for investigation. `SyncEvent::IngestError` is emitted with details. This prevents data loss — the bundle can be retried after a client update or investigated manually.

---

## 14. Bundle Deduplication

The sync engine does not perform bundle-level deduplication. If a user receives the same operations via multiple channels (e.g., both relay and a shared folder), the ingestion pipeline handles this gracefully. Operations are applied idempotently — `apply_delta()` checks each operation's `operation_id` against the existing operation log and skips duplicates. Applying the same signed operation twice has no effect.

When a bundle from the relay contains only operations already applied (e.g., from a prior folder-based exchange), the bundle is still acknowledged and deleted from the relay. No data corruption or duplication occurs.

---

## 15. Test Relay Compatibility

The test relay service (swarm-relay, PHP 8.3) implements the full API surface needed by this design:

| Feature | Relay Endpoint | Status |
|---------|---------------|--------|
| Registration + PoP | `POST /auth/register`, `POST /auth/register/verify` | Implemented |
| Login / Logout | `POST /auth/login`, `POST /auth/logout` | Implemented |
| Password Reset | `POST /auth/reset-password`, `POST /auth/reset-password/confirm` | Implemented |
| Account & Devices | `GET /account`, `POST /account/devices`, etc. | Implemented |
| Mailboxes | `POST /mailboxes`, `GET /mailboxes`, `DELETE /mailboxes/{id}` | Implemented |
| Bundle Transfer | `POST /bundles`, `GET /bundles`, `GET /bundles/{id}`, `DELETE /bundles/{id}` | Implemented |
| Invites | `POST /invites`, `GET /invites/{token}`, `DELETE /invites/{token}` | Implemented |

**Not implemented:** WebSocket (`WS /stream`). The design uses polling only (60-second minimum interval enforced server-side).

**PoP crypto:** The relay uses `ext-sodium` (libsodium) for the X25519 challenge. The client-side Ed25519 → X25519 conversion already exists in `swarm/crypto.rs`.

---

## 16. Build Sequence

| Phase | Scope | Dependencies |
|-------|-------|-------------|
| **S1** | Peer registry extension (new columns + migration) | None |
| **S2** | SyncChannel trait + ChannelType enum + PeerSyncInfo | None (trait definition is independent of DB migration) |
| **S3** | Relay credential storage (encrypted per-identity file) | Identity system |
| **S4** | Relay client (reqwest HTTP layer, all endpoints) | S3 |
| **S5** | Relay auth flow (registration, PoP, login, password reset) | S4 + swarm/crypto.rs + `crypto_box` crate |
| **S6** | RelayChannel (implements SyncChannel trait) | S2 + S4 |
| **S7** | FolderChannel (implements SyncChannel trait) | S2 |
| **S8** | SyncEngine dispatch loop + SyncEvent system | S1 + S2 + S6 + S7 |
| **S9** | Tauri wiring (background poll, event forwarding) | S8 |
| **S10** | Invitation flow enhancement (unified dialog, relay upload) | S4 + existing invite UI |
| **S11** | Peers dialog enhancement (channel config, sync status display) | S1 + S9 |

S1–S8 are pure `krillnotes-core`. S9–S11 touch `krillnotes-desktop`.

### Parallelisation Opportunities

- S1 and S2 have no dependency on each other — can run in parallel.
- S3/S4/S5 (relay credential + client + auth) can run in parallel with S7 (folder channel).
- S8 requires S1 (peer registry columns exist for `poll()` to query `channel_type`).
