# Streamline Relay Invite Workflow — Design Spec

**Date:** 2026-03-16
**Status:** Draft

## Problem

The current invite workflow requires too many steps, forces a file save even when the user only wants a relay link, and the relay link functionality (`create_relay_invite`, `fetch_relay_invite`) is stubbed out. Users must navigate: Workspace Peers → Manage Invites → Create Invite → Configure expiry → Save file → then finally reach sharing options.

## Goal

One-click "Share Invite Link" from the Workspace Peers dialog: creates the invite, uploads to relay, copies the URL to clipboard. The full invite round-trip (create → share → accept → respond) can happen entirely over relay links with no file exchange.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Share method | One-click button, no `.swarm` file save required | Eliminates the forced file-save step |
| Relay fallback | Inline prompt to set up relay account, then auto-continue | Seamless recovery, no lost state |
| Default expiry | User-configurable per-identity setting, initial default 7 days | Flexibility without per-invite friction; per-identity because InviteManager is keyed by identity |
| Button placement | Both WorkspacePeersDialog and InviteManagerDialog | Maximum discoverability |
| Relay URL persistence | Stored on `InviteRecord`, shown in invite list | Easy lookup after creation |
| Response delivery | Upload response to relay via `POST /invites`, inviter fetches by URL | Symmetric design — no mailbox/bundle requirements, both sides just need a relay account |
| Relay revocation | Deferred — local revoke does not delete from relay in this iteration | Keeps scope tight; relay invites expire naturally (max 90 days) |

---

## 1. Data Model Changes

### 1.1 `InviteRecord` (krillnotes-core, `invite.rs`)

Add one field:

```rust
#[serde(default)]
pub relay_url: Option<String>,
```

`#[serde(default)]` ensures backward compatibility — existing invite JSON files without this field deserialize as `None`.

Note: The `InviteRecord` JSON file (metadata) is always saved to disk by `InviteManager::create_invite` — this is required for `list_invites` to work. What changes is that no `.swarm` *export* file is saved in the one-click flow.

### 1.2 Tauri `InviteInfo` DTO (`commands/invites.rs`)

Add matching field:

```rust
pub relay_url: Option<String>,
```

Update `From<InviteRecord>` impl to map it through.

**Name disambiguation:** This is the *Tauri DTO* `InviteInfo` (in `commands/invites.rs`), distinct from the *relay client* `relay::client::InviteInfo` (which has `url`, `token`, `invite_id`, `expires_at`). When converting between them, map `relay::client::InviteInfo::url` → `InviteRecord::relay_url`.

### 1.3 `InviteInfo` (TypeScript, `types.ts`)

```typescript
relayUrl: string | null;
```

---

## 2. Core Methods

### 2.1 `InviteManager::set_relay_url(invite_id: Uuid, url: String)`

Loads the invite record from disk, sets `relay_url = Some(url)`, saves it back. Called after a successful relay upload.

### 2.2 `InviteManager::parse_and_verify_invite_bytes(bytes: &[u8]) -> Result<InviteFile>`

Same logic as `parse_and_verify_invite(path: &Path)` but reads from a byte slice instead of a file. Refactor: extract the shared ZIP-reading + JSON-parsing + signature-verification logic into an internal helper (e.g., `parse_and_verify_invite_from_reader<R: Read + Seek>`) that both methods call.

### 2.3 `InviteManager::serialize_invite_to_bytes(invite_file: &InviteFile) -> Result<Vec<u8>>`

Serializes an `InviteFile` to the ZIP `.swarm` format in memory (no file I/O). Implementation: use `ZipWriter::new(Cursor::new(Vec::new()))` since the `zip` crate accepts any `Write` implementor. Used by `share_invite_link` to prepare the upload payload without saving a file.

Similarly, add `serialize_response_to_bytes(response: &InviteResponseFile) -> Result<Vec<u8>>` for the relay response flow.

The existing `write_json_zip` helper (which writes to `std::fs::File`) can be refactored into `write_json_zip_to_writer<W: Write + Seek>` so both the file and in-memory paths share the same logic.

---

## 3. Tauri Commands

### 3.1 `share_invite_link` (NEW)

**Signature:**
```rust
pub async fn share_invite_link(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_name: String,
    expires_in_days: Option<u32>,
) -> Result<InviteInfo, String>
```

**Flow:**
1. Get signing key, declared name, workspace metadata (same as `create_invite`)
2. `InviteManager::create_invite(...)` → `(InviteRecord, InviteFile)`
3. `InviteManager::serialize_invite_to_bytes(&invite_file)` → ZIP blob in memory
4. Base64-encode the blob
5. Compute `expires_at` as ISO 8601 UTC string
6. Get first relay account for this identity → build `RelayClient` with `session_token`. If session expired, auto-login using stored `email` + `password` (the `device_public_key` field on `LoginRequest` is a client-side artifact, not required by the relay API's `/auth/login` endpoint which only needs email + password)
7. `spawn_blocking` → `client.create_invite(payload_base64, expires_at)` → `relay::client::InviteInfo` (contains `url`)
8. `InviteManager::set_relay_url(invite_id, relay_url)` → persist URL to record on disk
9. Convert `InviteRecord` → Tauri `InviteInfo` via `From`, then **manually set `relay_url`** on the result (since the record was written to disk in step 8 but the in-memory `InviteRecord` from step 2 doesn't have it yet)
10. Return `InviteInfo` (with `relayUrl` populated)

If the relay session is expired, attempt auto-login (same pattern as `poll_sync`). If auto-login fails, return an error indicating relay auth needed.

**Note:** Step 6 uses `.first()` from `list_relay_accounts()`. When multi-relay support lands, this will need a relay account selector. For now, single account is sufficient.

### 3.2 `create_relay_invite` (IMPLEMENT STUB)

For uploading an already-created invite to relay (e.g., from the invite list "Upload to Relay" action).

**Flow:**
1. Look up invite record by `invite_id` from `InviteManager`
2. Reconstruct the `InviteFile` from the record + signing key + workspace metadata (same pattern as existing `save_invite_file` in `commands/invites.rs` which already rebuilds the full `InviteFile` from these inputs)
3. Serialize to bytes via `serialize_invite_to_bytes`, base64-encode
4. Upload via `RelayClient::create_invite`
5. `InviteManager::set_relay_url(invite_id, url)` → persist
6. Return the URL

**Revised signature (breaking change from stub):**
```rust
pub async fn create_relay_invite(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> Result<String, String>  // returns the relay URL
```

**Frontend migration:** `CreateInviteDialog.tsx:83` currently calls `invoke('create_relay_invite', { token: createdInvite.inviteId })`. Update to pass `{ identityUuid, inviteId }`.

### 3.3 `fetch_relay_invite` (IMPLEMENT STUB)

For downloading an invite by its relay token.

**Flow:**
1. Create `RelayClient` pointed at `relay_base_url` (default: `https://swarm.krillnotes.org`), no session token — `GET /invites/{token}` is public
2. `spawn_blocking` → `client.fetch_invite(token)` → `InvitePayload` with base64 blob
3. Decode base64 → raw bytes
4. `InviteManager::parse_and_verify_invite_bytes(bytes)` → `InviteFile`
5. Write the raw bytes to a temp file (e.g., `std::env::temp_dir().join(format!("{token}.swarm"))`) — needed so `respond_to_invite` can use the file path later
6. Convert `InviteFile` to `InviteFileData` DTO
7. Return `(InviteFileData, temp_path)` to frontend

**Revised signature (breaking change from stub):**
```rust
pub async fn fetch_relay_invite(
    window: Window,
    state: State<'_, AppState>,
    token: String,
    relay_base_url: Option<String>,  // defaults to https://swarm.krillnotes.org
) -> Result<FetchedRelayInvite, String>
```

Where `FetchedRelayInvite` is:
```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedRelayInvite {
    pub invite: InviteFileData,
    pub temp_path: String,  // path to temp .swarm file for respond_to_invite
}
```

**Frontend migration:** `ImportInviteDialog.tsx:71-74` currently chains `fetch_relay_invite` → `parse_invite_bytes` → `write_temp_swarm_bytes`. This collapses to a single `fetch_relay_invite` call. The `parse_invite_bytes` and `write_temp_swarm_bytes` stubs can be **removed**.

### 3.4 `send_invite_response_via_relay` (NEW)

Uploads the invite response to the relay as a shareable link (using `POST /invites`, NOT `POST /bundles`). The invitee copies this URL and shares it with the inviter, who fetches it to complete the handshake.

**Why not bundles?** `POST /bundles` requires: (a) inviter's device key registered on relay, (b) inviter's account has a mailbox for the workspace, (c) invitee has a relay account. Conditions (a) and (b) aren't guaranteed — the inviter may use relay only for invite sharing, not sync. Using `POST /invites` for the response is symmetric: both sides upload blobs and share URLs.

**Flow:**
1. Parse `identity_uuid`, get signing key + declared name from unlocked identity
2. Read the invite from `temp_path` (the temp file written by `fetch_relay_invite`) using existing `parse_and_verify_invite(path)`
3. Build `InviteResponseFile` using `InviteManager::build_response(invite_file, signing_key, declared_name)` (new method — like `build_and_save_response` but returns the struct without saving). Refactor `build_and_save_response` to call `build_response` + `write_json_zip` internally to avoid code duplication.
4. `serialize_response_to_bytes(&response)` → ZIP blob in memory
5. Base64-encode the blob
6. Get relay account for this identity → build authenticated `RelayClient`
7. `spawn_blocking` → `client.create_invite(payload_base64, expires_at)` → returns URL
8. Return the URL to frontend (which copies it to clipboard)

**Signature:**
```rust
pub async fn send_invite_response_via_relay(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    temp_path: String,       // path to the fetched invite temp file
    expires_in_days: Option<u32>,  // defaults to invite expiry or 7 days
) -> Result<String, String>  // returns the relay URL for the response
```

### 3.5 `fetch_relay_invite_response` (NEW)

For the inviter to fetch a response that was uploaded via relay.

**Flow:**
1. Create unauthenticated `RelayClient` → `GET /invites/{token}`
2. Decode base64 → raw bytes
3. Parse as `InviteResponseFile` (new method: `InviteManager::parse_and_verify_response_bytes(bytes)`)
4. Write to temp file
5. Import by extracting the validation logic from the `import_invite_response` Tauri command into a shared core helper (validate signature, check invite not revoked/expired, increment use count) that both `import_invite_response` and `fetch_relay_invite_response` call
6. Return `PendingPeer` to frontend

**Signature:**
```rust
pub async fn fetch_relay_invite_response(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    token: String,
    relay_base_url: Option<String>,
) -> Result<PendingPeer, String>
```

---

## 4. UI Changes

### 4.1 WorkspacePeersDialog

Add a prominent **"Share Invite Link"** button near the top of the dialog, alongside the existing "Manage Invites" button.

**Behavior:**
1. Click → check `has_relay_credentials`
2. If no relay account → open `AddRelayAccountDialog` modal, on success auto-continue
3. Call `share_invite_link` with default expiry
4. On success → copy URL to clipboard, show success toast with the URL
5. On error → show error message inline

### 4.2 InviteManagerDialog

**Top-level:** Add the same **"Share Invite Link"** button.

**Invite list rows:** For each invite:
- If `relayUrl` is set: show the URL as a copyable chip/badge + "Copy Link" icon button
- If `relayUrl` is null and not revoked: show an "Upload to Relay" button (calls `create_relay_invite`, persists URL, updates list)
- Existing revoke/delete actions remain unchanged

**"Import Response from Link"** button: opens a small input for pasting a relay response URL. Calls `fetch_relay_invite_response` → flows into the existing `AcceptPeerDialog`.

### 4.3 ImportInviteDialog — Relay Fetch

The relay URL input + "Fetch" button already exist. Wire them to the now-working `fetch_relay_invite`. Store the returned `tempPath` in component state for use in the response step.

After fetch, the existing flow continues: show invite details, select identity, verify fingerprint.

### 4.4 ImportInviteDialog — Relay Response

After the user confirms the invite and clicks "Respond":
1. **Primary action:** "Send via Relay" → calls `send_invite_response_via_relay(identityUuid, tempPath)` → copies returned URL to clipboard → shows toast: "Response link copied! Share it with the inviter."
2. **If no relay account:** open `AddRelayAccountDialog`, on success auto-continue
3. **Secondary action:** "Save response file" — existing file-save behavior (calls `respond_to_invite` with file picker)

### 4.5 Relay Account Fallback Pattern

Both inviter and invitee sides use the same pattern:
1. Set a "pending action" callback in component state
2. Open `AddRelayAccountDialog`
3. On dialog success → execute the pending action callback
4. On dialog cancel → clear pending state, no action taken

### 4.6 Default Expiry Setting

Store as a per-identity setting (since `InviteManager` is keyed by identity UUID, not workspace). Implementation: a `_settings.json` file in the invites directory (underscore prefix to distinguish from invite record files, which are `{uuid}.json`). Initial default: 7 days.

Surface a small control in `InviteManagerDialog` (e.g., a dropdown near the "Share Invite Link" button) to let users change it. Values: 7 days, 30 days, 90 days (relay max), custom.

Client-side validation: cap at 90 days when using relay (the relay API rejects `INVALID_EXPIRY` for >90 days). Show a warning if the user tries to set a higher value.

---

## 5. What's NOT Changing

- Existing file-based invite flow (create → save → import → respond via files)
- `InviteFile` / `InviteResponseFile` wire format (the `.swarm` ZIP structure)
- `RelayClient` HTTP methods (already implemented: `create_invite`, `fetch_invite`, `list_invites`, `delete_invite`)
- Ed25519 signing and verification logic
- `AddRelayAccountDialog` component (reused as-is, just opened from new contexts)
- Relay invite revocation (deferred — local revoke does not call `DELETE /invites/{token}` in this iteration)

---

## 6. Error Handling

| Scenario | Behavior |
|----------|----------|
| No relay account | Open AddRelayAccountDialog, auto-continue on success |
| Relay session expired | Auto-login in background (using stored password + device_public_key), retry once |
| Auto-login fails | Show error: "Relay session expired. Please re-authenticate." |
| Relay upload fails (network) | Show error with retry button |
| Relay returns `PAYLOAD_TOO_LARGE` | Show error: "Invite too large for relay (max 10 MB)" |
| Relay returns `INVALID_EXPIRY` | Show error about expiry range (max 90 days). Client-side validation should prevent this. |
| Invite token not found (`404`) | Show error: "This invite link is no longer valid" |
| Invite expired (`410`) | Show error: "This invite has expired" |
| Clipboard write fails | Show the URL in a copyable text field as fallback |
| Response relay upload fails | Fall back to "Save response file" option |
| `skipped.unknown` on bundle delivery | N/A — not using bundles in this design |

---

## 7. Stubs to Remove

The following stub commands in `commands/sync.rs` are replaced by the new implementations and can be removed:
- `parse_invite_bytes` — parsing now happens inside `fetch_relay_invite`
- `write_temp_swarm_bytes` — temp file creation now happens inside `fetch_relay_invite`

---

## 8. Complete Invite Round-Trip via Relay

For reference, the full no-file flow:

**Inviter:**
1. Click "Share Invite Link" → invite created + uploaded → URL copied to clipboard
2. Send URL to invitee (via chat, email, etc.)

**Invitee:**
3. Paste URL in ImportInviteDialog → "Fetch" → invite details shown
4. Select identity, verify fingerprint, click "Send via Relay"
5. Response uploaded → response URL copied to clipboard
6. Send response URL back to inviter

**Inviter:**
7. Paste response URL in InviteManagerDialog → "Import Response from Link"
8. Response fetched + validated → AcceptPeerDialog shown
9. Set trust level, accept peer → done

Both sides exchange URLs instead of `.swarm` files. Each URL exchange requires an out-of-band channel (chat, email), same as the file exchange did.
