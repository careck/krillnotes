# Multi-Device Sync Fixes — Design Spec

**Date:** 2026-03-30
**Scope:** Two bugs that block multi-device sync (same identity on multiple machines)

## Problem

When an identity is imported to a second device and a workspace is bootstrapped via self-snapshot:

1. **`generate_delta` fails** with "no contact for peer identity" — the self-snapshot import skips contact creation for the sender (same identity), but `generate_delta` always resolves the encryption key via the contact manager with no fallback for self-identity peers.

2. **Relay delivery fails** with "relay skipped all recipients" — the second device logs into the relay with the same credentials, but the relay server doesn't learn about the new device key. Bundles addressed to that key are skipped as "unknown."

## Fix 1: `generate_delta` self-identity bypass

**File:** `krillnotes-core/src/core/swarm/sync.rs`, `generate_delta()` (lines 96–112)

**Current behavior:** Always resolves `peer.peer_identity_id` via `contact_manager.find_by_public_key()`. Fails for self-identity peers because `apply_swarm_snapshot` deliberately skips contact creation when `is_self_snapshot = true`.

**New behavior:** Before the contact lookup, derive the sender's own public key from the `signing_key` parameter (already available). Compare it with `peer.peer_identity_id`:

- **Match (self-identity):** Use `signing_key.verifying_key()` directly as the recipient verifying key. No contact manager lookup.
- **No match (different identity):** Existing contact manager lookup path, unchanged.

**Why this is correct:** Delta encryption uses the recipient's Ed25519 public key. For multi-device peers the recipient holds the same private key, so encrypting to `signing_key.verifying_key()` produces a bundle the other device can decrypt.

## Fix 2: Relay auto-registers new device on login

### Overview

When a second device calls `POST /auth/login` with the same email/password, the relay server should detect an unknown `device_public_key` and return a PoP (Proof of Possession) challenge inline with the session token. The client automatically completes the challenge, leaving the device verified — zero extra round-trips for already-known devices, one extra call for new devices.

### Server change: `LoginHandler.php`

**Current:** Accepts `email` + `password`, returns `{ session_token }`. Ignores `device_public_key`.

**New:** Reads optional `device_public_key` from the request body. After credential validation:

| Device key state | Action |
|---|---|
| Not provided | Return session token only (backward compatible) |
| Found + verified | Return session token only (1 DB lookup, no overhead) |
| Not found | Insert as unverified, create PoP challenge, return session token + challenge |
| Found + unverified | Create fresh PoP challenge, return session token + challenge |

**Response schema:**

```json
{
  "data": {
    "session_token": "...",
    "challenge": {
      "encrypted_nonce": "...",
      "server_public_key": "..."
    }
  }
}
```

The `challenge` field is **only present** when the device needs verification. Omitted entirely for known+verified devices.

**Dependencies injected into LoginHandler:** `DeviceKeyRepository`, `ChallengeRepository`, `CryptoService` — same services already used by `AddDeviceHandler` and `RegisterHandler`.

### Client change: `client.rs`

**`SessionResponse` struct** gains an optional challenge field:

```rust
pub struct SessionResponse {
    pub session_token: String,
    pub challenge: Option<RegisterChallenge>,
}
```

`RegisterChallenge` already exists (used by register and add_device flows) with `encrypted_nonce` and `server_public_key` fields.

No new methods needed — the existing `verify_device(device_public_key, nonce)` endpoint handles verification.

### Tauri command change: `login_relay_account`

**File:** `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs`, `login_relay_account()` (lines 156–226)

**Current `spawn_blocking` block:**

1. `client.login(email, password, device_public_key)` → extract `session_token`

**New `spawn_blocking` block:**

1. `client.login(email, password, device_public_key)` → get full response
2. If `response.challenge` is `Some`:
   - `decrypt_pop_challenge(signing_key, encrypted_nonce, server_public_key)` → nonce bytes
   - `hex::encode(nonce_bytes)` → nonce hex
   - `client.with_session_token(session_token).verify_device(device_public_key, nonce_hex)`
3. Return `session_token`

This requires capturing `signing_key` before entering the `spawn_blocking` block — same pattern already used by `register_relay_account` on the same file (line 86).

The `decrypt_pop_challenge` function is already imported at line 20 of the file.

## Files changed

| File | Change |
|---|---|
| `krillnotes-core/src/core/swarm/sync.rs` | Self-identity bypass in `generate_delta` |
| `krillnotes-core/src/core/sync/relay/client.rs` | Add optional `challenge` to `SessionResponse` |
| `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs` | Auto-verify in `login_relay_account` |
| `krillnotes-relay/src/Handler/Auth/LoginHandler.php` | Conditional device registration + PoP challenge |

## Out of scope

- Exporting relay account credentials as part of `.swarmid` (user must re-enter email/password on new device — this is intentional)
- Changes to the `register` flow (already handles device verification correctly)
- Changes to `apply_swarm_snapshot` contact creation logic (Fix 1 makes it unnecessary for self-identity)
