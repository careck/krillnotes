# Relay Configure Dialog — Design Spec

**Date:** 2026-03-15
**Branch:** feat/sync-engine
**Status:** Approved by user

---

## Summary

Implement relay server configuration in the Workspace Peers dialog. When a user clicks "Configure" for a peer whose channel type is "Relay", a modal dialog opens allowing them to register a new relay account or log in to an existing one. On success the session credentials are stored encrypted on disk and subsequent `poll_sync` calls automatically include a `RelayChannel`.

---

## Background

The `configure_relay` and `relay_login` Tauri commands exist as stubs returning `Err("not yet implemented")`. The relay client (`RelayClient`), credential storage (`save_relay_credentials` / `load_relay_credentials`), and PoP challenge logic (`decrypt_pop_challenge`) are all fully implemented in `krillnotes-core`. The `relay` feature flag is already enabled in the desktop crate (`krillnotes-core = { ..., features = ["relay"] }` in `src-tauri/Cargo.toml` line 30), so all `#[cfg(feature = "relay")]`-gated code including `decrypt_pop_challenge` is available.

---

## Architecture

### Data Flow

```
User clicks "Configure" (relay peer)
  → ConfigureRelayDialog opens
  → on mount: invoke("get_relay_info") → { relayUrl, email } | null
  → if Some: pre-fill Login tab, set active tab = "login"
  → if None: active tab = "register"

Register tab submit:
  invoke("configure_relay", { identityUuid, relayUrl, email, password })
  → Rust: register → PoP decrypt → verify → save creds to disk

Login tab submit:
  invoke("relay_login", { identityUuid, relayUrl, email, password })
  → Rust: login → save updated creds to disk

Subsequent poll_sync calls:
  → Rust: load relay creds from disk if present → add RelayChannel to engine

On dialog success:
  invoke("update_peer_channel", { peerDeviceId, channelType: "relay", channelParams: "{}" })
  → close dialog, reload peers
```

### Key constraint: relay credentials are **identity-scoped**, not peer-scoped
One set of relay credentials per identity (stored at `<config_dir>/relay/<identity_uuid>.json`). The "Configure" button on any relay peer opens the same dialog — it configures the identity's relay account. Clicking it when credentials already exist defaults to the Login tab (re-auth).

### relay_dir convention (used in every command below)
```rust
let relay_dir = crate::settings::config_dir().join("relay");
```

---

## Rust Core Changes

### 1. Add `relay_key()` to `UnlockedIdentity` (`krillnotes-core/src/core/identity.rs`)

Follows the existing `contacts_key()` pattern exactly (no salt — Ed25519 seed provides sufficient entropy as IKM):

```rust
pub fn relay_key(&self) -> [u8; 32] {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, self.signing_key.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"krillnotes-relay-v1", &mut okm)
        .expect("HKDF expand failed");
    okm
}
```

---

## Tauri Command Changes (`krillnotes-desktop/src-tauri/src/commands/sync.rs`)

### 2. Implement `configure_relay` (registration flow)

Signature (stub parameters renamed, `_` prefixes removed):
```rust
pub async fn configure_relay(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<(), String>
```

Steps:
1. Parse `identity_uuid` as `Uuid`
2. Obtain `(signing_key, verifying_key, relay_key)` under lock:
   ```rust
   let identities = state.unlocked_identities.lock()…;
   let id = identities.get(&uuid)…; // key is Uuid
   let signing_key = Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes());
   let verifying_key = id.verifying_key;
   let relay_key = id.relay_key();
   ```
3. Compute `device_public_key = hex::encode(verifying_key.to_bytes())`
   — **Note:** this is hex, unlike `poll_sync` which uses Base64 for a different purpose. Do not copy the `BASE64.encode` pattern from `poll_sync`.
4. Create `let client = RelayClient::new(&relay_url);`
5. Call `client.register(&email, &password, &identity_uuid, &device_public_key)` → `RegisterResult { challenge, .. }`
6. `let nonce_bytes = decrypt_pop_challenge(&signing_key, &challenge.encrypted_nonce, &challenge.server_public_key).map_err(|e| e.to_string())?;`
7. `let nonce_hex = hex::encode(&nonce_bytes);`
8. `let session = client.register_verify(&device_public_key, &nonce_hex).map_err(|e| e.to_string())?;`
9. Build credentials:
   ```rust
   let creds = RelayCredentials {
       relay_url: relay_url.clone(),
       email,
       session_token: session.session_token,
       session_expires_at: chrono::Utc::now() + chrono::Duration::days(30),
       device_public_key,
   };
   ```
   *(Note: 30 days is a local approximation; the relay server's actual expiry governs re-auth.)*
10. `save_relay_credentials(&relay_dir, &identity_uuid, &creds, &relay_key).map_err(|e| e.to_string())?;`

### 3. Implement `relay_login` (re-authentication flow)

**Signature change** (stub gains `relay_url` parameter — needed because the Login tab UI exposes an editable URL field):
```rust
pub async fn relay_login(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<(), String>
```

Steps:
1. Parse `identity_uuid` as `Uuid`
2. Get `(relay_key, device_public_key)` under lock:
   - Try to load existing creds: `load_relay_credentials(&relay_dir, &identity_uuid, &relay_key)`
   - If `Some(existing)`: use `existing.device_public_key`
   - If `None`: derive from `verifying_key` as in step 3 of `configure_relay`
3. `let client = RelayClient::new(&relay_url);`
4. `let session = client.login(&email, &password).map_err(|e| e.to_string())?;`
5. Build and save `RelayCredentials` (same pattern as `configure_relay`)

### 4. Implement `has_relay_credentials`

```rust
pub async fn has_relay_credentials(
    window: Window,
    state: State<'_, AppState>,
) -> Result<bool, String>
```

Steps:
1. `let workspace_label = window.label().to_string();`
2. Get identity UUID (as `Uuid`) from `state.workspace_identities` (key: `&workspace_label`, value: `Uuid`)
3. Get `relay_key` from `state.unlocked_identities` (key: `&uuid`)
4. `let relay_dir = crate::settings::config_dir().join("relay");`
5. `let creds = load_relay_credentials(&relay_dir, &uuid.to_string(), &relay_key)…;`
6. `Ok(creds.is_some())`

### 5. Add `get_relay_info` command (new)

New struct (in `sync.rs`):
```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayInfo {
    pub relay_url: String,
    pub email: String,
}
```

Command:
```rust
#[tauri::command]
pub async fn get_relay_info(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Option<RelayInfo>, String>
```

Steps: same as `has_relay_credentials` 1–5, then:
- If `Some(creds)` → `Ok(Some(RelayInfo { relay_url: creds.relay_url, email: creds.email }))`
- If `None` → `Ok(None)`

### 6. Update `poll_sync` to include `RelayChannel` when credentials exist

Two changes to `poll_sync`:

**a) Extend the existing identity lock block (currently lines 59–64) to also capture `relay_key` and `sender_device_key_hex`:**

```rust
let (signing_key, sender_display_name, identity_pubkey, relay_key, sender_device_key_hex) = {
    let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
    let id = m.get(&identity_uuid).ok_or("Identity not unlocked")?;
    let pubkey_b64 = BASE64.encode(id.verifying_key.as_bytes()); // FolderChannel (Base64)
    let pubkey_hex = hex::encode(id.verifying_key.to_bytes());   // RelayChannel (hex)
    let rk = id.relay_key();
    (id.signing_key.clone(), id.display_name.clone(), pubkey_b64, rk, pubkey_hex)
};
```

**b) After `workspace` is obtained** (after the `workspaces.get_mut` call, currently around line 88), attempt to load relay credentials and register `RelayChannel`:

```rust
// Try to add relay channel if credentials exist for this identity
let relay_dir = crate::settings::config_dir().join("relay");
if let Ok(Some(creds)) = load_relay_credentials(&relay_dir, &identity_uuid.to_string(), &relay_key) {
    let relay_client = RelayClient::new(&creds.relay_url)
        .with_session_token(&creds.session_token);
    let workspace_id_str = workspace.workspace_id().to_string();
    let relay_channel = RelayChannel::new(relay_client, workspace_id_str, sender_device_key_hex.clone());
    engine.register_channel(Box::new(relay_channel));
}
```

`state.sync_engines` is **not used** — `poll_sync` builds a fresh engine per call. The stored credentials on disk are the source of truth.

### 7. Register `get_relay_info` in `lib.rs`

Add `get_relay_info` to `tauri::generate_handler!`. `configure_relay` and `relay_login` are already registered.

---

## TypeScript / React Changes

### 8. Add `RelayInfo` to `types.ts`

```typescript
export interface RelayInfo {
  relayUrl: string;
  email: string;
}
```

### 9. New `ConfigureRelayDialog` component

**File:** `krillnotes-desktop/src/components/ConfigureRelayDialog.tsx`

**Props:**
```typescript
interface Props {
  identityUuid: string;
  peerDeviceId: string;
  onClose: () => void;
  onConfigured: () => void;
}
```

**State:**
- `activeTab: 'register' | 'login'`
- `relayUrl`, `email`, `password`, `confirmPassword` (strings)
- `loading: boolean`
- `error: string | null`

**Behaviour:**
- On mount: `invoke<RelayInfo | null>("get_relay_info")`
  - If result is non-null: pre-fill `relayUrl` + `email`, set `activeTab = 'login'`
  - If null: `activeTab = 'register'`
- Escape key closes dialog
- **Register tab fields:** Relay URL, Email, Password, Confirm Password
  - Confirm Password is validated client-side only (must match Password)
  - On submit: `invoke("configure_relay", { identityUuid, relayUrl, email, password })`
- **Login tab fields:** Relay URL (pre-filled, editable), Email (pre-filled, editable), Password
  - On submit: `invoke("relay_login", { identityUuid, relayUrl, email, password })`
- Submit button shows spinner while `loading === true`
- Error displayed in red banner below tab content
- On success:
  1. `invoke("update_peer_channel", { peerDeviceId, channelType: "relay", channelParams: "{}" })`
  2. Call `onConfigured()`

**Error message mapping:**
| Error substring | User message |
|-----------------|--------------|
| `Identity not unlocked` / `Identity is not unlocked` | "Please unlock your identity before configuring relay" |
| `Cannot reach` / `relay server` / `unavailable` | "Cannot reach relay server. Check the URL and try again." |
| `HTTP 401` / `auth` | "Invalid credentials. Please check your email and password." |
| `HTTP 409` / `already` | "Email already registered — try the Login tab." |
| `HTTP 404` | "Relay server not found at this URL." |
| Fallback | Show raw error string |

### 10. Update `WorkspacePeersDialog`

- Add state: `showConfigureRelay: PeerInfo | null` (initially `null`)
- Change "Configure" button `onClick`:
  ```typescript
  onClick={() => {
    if (selectedChannelType === 'relay') {
      setShowConfigureRelay(peer);
    } else {
      handleUpdateChannel(peer, selectedChannelType);
    }
  }}
  ```
- Update disable condition: button is **always enabled** when `selectedChannelType === 'relay'`
  (current condition disables when type matches existing type and nothing is pending — this should not apply to relay since there's always something to configure/reconfigure)
- Render `<ConfigureRelayDialog>` when `showConfigureRelay !== null`:
  ```tsx
  {showConfigureRelay && (
    <ConfigureRelayDialog
      identityUuid={identityUuid}
      peerDeviceId={showConfigureRelay.peerDeviceId}
      onClose={() => setShowConfigureRelay(null)}
      onConfigured={async () => {
        setShowConfigureRelay(null);
        await loadPeers();
      }}
    />
  )}
  ```

---

## Files to Create / Modify

| File | Change |
|------|--------|
| `krillnotes-core/src/core/identity.rs` | Add `relay_key()` method to `UnlockedIdentity` |
| `krillnotes-desktop/src-tauri/src/commands/sync.rs` | Implement `configure_relay`, `relay_login`, `has_relay_credentials`; add `get_relay_info` + `RelayInfo` struct; update `poll_sync` |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Add `get_relay_info` to `generate_handler!` |
| `krillnotes-desktop/src/types.ts` | Add `RelayInfo` interface |
| `krillnotes-desktop/src/components/ConfigureRelayDialog.tsx` | New file |
| `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` | Wire Configure button for relay, render `ConfigureRelayDialog` |

---

## Out of Scope

- Automatic relay re-auth on `auth_expired` sync events (separate task)
- Multiple relay servers per identity
- Relay account management (password change, delete account)
- `state.sync_engines` usage — `poll_sync` builds a fresh engine per call; the map is reserved for future background sync
