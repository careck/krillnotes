# Relay Account Identity-Level Management

**Date:** 2026-03-15
**Status:** Design approved

## Problem

Relay accounts are currently configured per-peer in the Workspace Peers dialog via a ConfigureRelayDialog. This is inconvenient when multiple peers all use the same relay service — the user must navigate through each peer to configure relay. Additionally, the user must re-enter their relay password whenever a session expires.

## Solution

Move relay account management to the Identity Manager, alongside contacts. Relay accounts are stored per-identity, encrypted with the identity's relay key, and include the account password for automatic session renewal. When configuring a peer to use relay, the user picks from a dropdown of stored relay accounts.

## Data Model & Storage

### On-disk layout

```
~/.config/krillnotes/identities/<identity_uuid>/
├── identity.json
├── contacts/
│   └── <contact_uuid>.json
└── relays/                              ← NEW
    └── <relay_account_id>.json          ← one per relay account
```

### RelayAccount struct

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccount {
    pub relay_account_id: Uuid,
    pub relay_url: String,
    pub email: String,
    pub password: String,               // stored for auto-login
    pub session_token: String,
    pub session_expires_at: DateTime<Utc>,
    pub device_public_key: String,      // hex-encoded Ed25519 public key, used by RelayChannel for relay server auth
}
```

On-disk format: camelCase JSON (consistent with `Contact`), wrapped in an `EncryptedRelayFile` envelope (`nonce` + `ciphertext`, both base64-encoded). Encrypted with AES-256-GCM using the identity's existing `relay_key()` derivation (HKDF-SHA256 from Ed25519 seed, info string `b"krillnotes-relay-v1"`).

### RelayAccountManager

Analogous to `ContactManager`. In-memory cache of decrypted relay accounts for an unlocked identity.

**API:**
- `for_identity(relays_dir, key)` — load and decrypt all relay accounts
- `create_relay_account(...)` → `RelayAccount` (errors if `find_by_url` finds a duplicate)
- `save_relay_account(account)` — encrypt and persist
- `list_relay_accounts()` → sorted by relay_url
- `get_relay_account(id)` → by UUID
- `find_by_url(url)` → deduplication (one account per relay server)
- `delete_relay_account(id)` — remove file

### AppState addition

```rust
pub relay_account_managers: Arc<Mutex<HashMap<Uuid, RelayAccountManager>>>,
```

Initialized on `unlock_identity`, cleared on `lock_identity` — same lifecycle as `contact_managers`.

## Auto-Login

On `unlock_identity`, after loading the `RelayAccountManager`:
1. Iterate all relay accounts
2. For any with an expired session (`session_expires_at < now`) and a non-empty password, attempt re-login
3. Since `RelayClient` uses `reqwest::blocking` (which panics inside async contexts), auto-login must run via `tokio::task::spawn_blocking` — same pattern used by `configure_relay` and `relay_login` today
4. Fire-and-forget: spawn the blocking task, don't await it in `unlock_identity`. On completion, update the stored session token and expiry in `RelayAccountManager`. If it fails (no internet, wrong password, etc.), skip silently — `poll_sync` retries later
5. `poll_sync` also checks session expiry before syncing and re-attempts auto-login if needed

The user only types the relay password once — at initial registration or first login. After that, auto-login handles session renewal transparently.

## Tauri Commands

### New commands

| Command | Args | Returns | Purpose |
|---------|------|---------|---------|
| `list_relay_accounts` | `identity_uuid` | `RelayAccountInfo[]` | List all relay accounts for identity |
| `register_relay_account` | `identity_uuid, relay_url, email, password` | `RelayAccountInfo` | Register on relay server, store credentials including password |
| `login_relay_account` | `identity_uuid, relay_url, email, password` | `RelayAccountInfo` | Login to existing relay account, store credentials including password |
| `delete_relay_account` | `identity_uuid, relay_account_id` | `()` | Remove stored credentials |
| `set_peer_relay` | `window, peer_device_id, relay_account_id` | `()` | Assign stored relay account to a workspace peer (window identifies workspace) |

### Removed commands

- `configure_relay` — absorbed into `register_relay_account`
- `relay_login` — absorbed into `login_relay_account`
- `get_relay_info` — replaced by `list_relay_accounts`

### TypeScript type

```typescript
export interface RelayAccountInfo {
  relayAccountId: string;
  relayUrl: string;
  email: string;
  sessionValid: boolean;  // derived: session_expires_at > now
}
```

`session_token` and `password` are never exposed to the frontend. Note: `sessionValid` is derived client-side from `session_expires_at > now`. The relay server is the source of truth for session validity — a session could be server-revoked while still showing as valid locally. This is acceptable; sync failures from invalid sessions trigger auto-login retry.

## UI Changes

### Identity Manager Dialog

Add a **"Relays (N)"** button per identity, alongside "Contacts (N)":
- Only shown/enabled when identity is unlocked
- Count reflects number of stored relay accounts

### RelayBookDialog (new)

Modeled on `ContactBookDialog`:
- List view: relay URL, email, session status indicator (valid/expired)
- "Add Relay Account" button → opens `AddRelayAccountDialog`
- Click existing account → opens `EditRelayAccountDialog`
- Delete with confirmation

### AddRelayAccountDialog (new)

Replaces `ConfigureRelayDialog`. Same Register/Login tabs but:
- Opened from `RelayBookDialog`, not from a peer context
- No `peerDeviceId` prop — purely identity-scoped
- On success: creates relay account, closes dialog

### EditRelayAccountDialog (new)

- Shows: relay URL (read-only), email (read-only), session status
- Action: Delete (with confirmation)
- No manual re-login needed — auto-login handles session renewal

### Workspace Peers Dialog — relay picker

When user selects "relay" as channel type for a peer:
- Show a dropdown of relay accounts from the bound identity
- Items formatted as: `email @ relay_url`
- If no relay accounts exist: "No relay accounts configured. Add one in Identity Manager → Relays."
- Selecting an account calls `set_peer_relay`

### Deleted component

`ConfigureRelayDialog.tsx` — removed entirely.

## Sync Engine Integration

### poll_sync changes

- Current: loads single `RelayCredentials` from `<config_dir>/relay/<identity_uuid>.json`, creates one `RelayChannel`
- New: reads `relay_account_id` from each peer's `channel_params`, fetches that `RelayAccount` from `RelayAccountManager`, creates a `RelayChannel` per distinct relay account used by peers
- If session is expired at sync time, auto-login using stored password before syncing

### channel_params format

- Current: `{"relay_url": "https://..."}` (set by `ConfigureRelayDialog` — note: the test in `peer_registry.rs` uses `{"url": "..."}` which should be aligned)
- New: `{"relay_account_id": "<uuid>"}`
- Relay URL looked up from the relay account, no longer stored in channel_params
- Note: currently `poll_sync` does NOT read `channel_params` for relay peers — it loads credentials from the single disk file. The new design changes this to read `relay_account_id` from `channel_params` to support multiple relay accounts

## Migration

On `unlock_identity`:

1. **Credential file migration (synchronous, completes before `unlock_identity` returns):** If old-style `<config_dir>/relay/<identity_uuid>.json` exists:
   - Decrypt using identity's relay key (reuse existing `load_relay_credentials` from `auth.rs`)
   - Create a `RelayAccount` with a new UUID in `identities/<uuid>/relays/`, copying `relay_url`, `email`, `session_token`, `session_expires_at`, `device_public_key` from the old `RelayCredentials`
   - The old format does not store the password — migrated account will have an empty password
   - Session will eventually expire; user must re-login once via Relay Book to store the password
   - Delete old file using existing `delete_relay_credentials` from `auth.rs`

2. **channel_params migration:** Per-workspace, on workspace open:
   - Any `sync_peers` rows with `channel_type = "relay"` and `channel_params` containing `relay_url` (old format) are updated to `{"relay_account_id": "<uuid>"}` format
   - Match by relay URL from the old `channel_params` to find the corresponding relay account UUID in the `RelayAccountManager`
   - If no matching relay account found (e.g., identity doesn't have one for that URL), set channel to `manual` and log a warning

## Error Handling

- **Registration fails (network, duplicate email):** Show error in `AddRelayAccountDialog`, no state change
- **Auto-login fails (network):** Skip silently, retry on next `poll_sync`
- **Auto-login fails (wrong password, account deleted server-side):** Mark session as invalid, surface in RelayBookDialog status indicator
- **Peer references deleted relay account:** Treat as `manual` channel, log warning
- **No relay accounts when picking for peer:** Show guidance message, no dropdown
