# krillnotes-relay API Reference

**Base URL:** `https://swarm.krillnotes.org`

All request and response bodies use `Content-Type: application/json` unless stated otherwise. All responses follow one of two envelope shapes:

> **Required request headers** — every request must include both of the following headers or the server will return `418` (blocked at the web server layer before the application runs):
>
> | Header | Required value |
> |--------|----------------|
> | `Content-Type` | `application/json` |
> | `User-Agent` | Any non-empty string, e.g. `KrillNotes/1.0` |
>
> Requests sent over HTTP/1.1 without a `User-Agent` header are silently rejected with `418 0 bytes` — no JSON body is returned because the block occurs in the web server (nginx), not in the application.

```json
{ "data": { ... } }        // success
{ "error": { "code": "SNAKE_CASE_CODE", "message": "Human-readable description" } }
```

---

## Authentication

Authenticated endpoints require a session token obtained via `/auth/login` or `/auth/register/verify`:

```
Authorization: Bearer <session_token>
```

Tokens expire after 30 days. A missing or invalid token returns:

```json
HTTP 401
{ "error": { "code": "UNAUTHORIZED", "message": "Missing authorization header" } }
```

---

## Proof-of-Possession (PoP) Handshake

Registration and adding a new device both require a cryptographic PoP challenge. The relay encrypts a random nonce to the client's Ed25519 public key (converted to X25519 via `crypto_sign_ed25519_pk_to_curve25519`) using an ephemeral `crypto_box` keypair. The client must decrypt it and return the plaintext.

**Decryption steps (libsodium):**
1. Convert your Ed25519 secret key → X25519: `crypto_sign_ed25519_sk_to_curve25519(edSk)`
2. Build a box keypair: `crypto_box_keypair_from_secretkey_and_publickey(x25519Sk, hex2bin(server_public_key))`
3. Split `encrypted_nonce` (hex): first `CRYPTO_BOX_NONCEBYTES` (24) bytes = box nonce; remainder = ciphertext
4. Decrypt: `crypto_box_open(ciphertext, boxNonce, keypair)` → plaintext nonce (32 bytes)
5. Submit the result as a 64-character hex string

---

## Endpoints

### Authentication

---

#### `POST /auth/register`

Begin account registration. Creates the account and first device key, then returns a PoP challenge.

**Request body**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `email` | string | ✓ | Must be unique |
| `password` | string | ✓ | Stored as bcrypt hash |
| `identity_uuid` | string | ✓ | Client-generated identifier for this identity |
| `device_public_key` | string | ✓ | 64-char hex Ed25519 public key (32 bytes) |

```json
{
  "email": "alice@example.com",
  "password": "correct-horse-battery",
  "identity_uuid": "550e8400-e29b-41d4-a716-446655440000",
  "device_public_key": "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
}
```

**Response `201`**

```json
{
  "data": {
    "account_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
    "challenge": {
      "encrypted_nonce": "a1b2c3d4...",
      "server_public_key": "e5f6a7b8..."
    }
  }
}
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Any required field absent |
| 400 | `INVALID_DEVICE_KEY` | Not a 64-char hex string |
| 409 | `EMAIL_EXISTS` | Email already registered |

---

#### `POST /auth/register/verify`

Submit the decrypted nonce to prove key ownership. Marks the device as verified and issues a session token.

**Request body**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `device_public_key` | string | ✓ | Same key sent to `/auth/register` |
| `nonce` | string | ✓ | 64-char hex — plaintext nonce decrypted from challenge |

```json
{
  "device_public_key": "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
  "nonce": "c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00"
}
```

**Response `200`**

```json
{
  "data": {
    "account_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
    "session_token": "3d0a7e5b9c2f1a4d..."
  }
}
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Any required field absent |
| 404 | `NO_CHALLENGE` | No pending registration challenge for this key |
| 403 | `INVALID_NONCE` | Decrypted value does not match stored nonce |

---

#### `POST /auth/login`

Password login. Returns a new session token.

**Request body**

| Field | Type | Required |
|-------|------|----------|
| `email` | string | ✓ |
| `password` | string | ✓ |

```json
{ "email": "alice@example.com", "password": "correct-horse-battery" }
```

**Response `200`**

```json
{ "data": { "session_token": "3d0a7e5b9c2f1a4d..." } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Any required field absent |
| 401 | `INVALID_CREDENTIALS` | Wrong email or password |
| 403 | `ACCOUNT_DELETED` | Account is flagged for deletion |

---

#### `POST /auth/logout`  🔒

Invalidates the current session token.

**Request body:** none

**Response `200`**

```json
{ "data": { "ok": true } }
```

---

#### `POST /auth/reset-password`

Requests a password reset. Always returns `200` regardless of whether the email exists (prevents enumeration). The reset token is stored server-side; email delivery is not yet implemented.

**Request body**

| Field | Type | Required |
|-------|------|----------|
| `email` | string | ✓ |

**Response `200`** (always)

```json
{ "data": { "message": "If the email exists, a reset link has been sent" } }
```

---

#### `POST /auth/reset-password/confirm`

Sets a new password using a valid reset token. Tokens expire after 1 hour and are single-use.

**Request body**

| Field | Type | Required |
|-------|------|----------|
| `token` | string | ✓ | Reset token from `/auth/reset-password` |
| `new_password` | string | ✓ | |

```json
{ "token": "abc123...", "new_password": "new-secure-password" }
```

**Response `200`**

```json
{ "data": { "ok": true } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Any required field absent |
| 404 | `INVALID_TOKEN` | Token not found or expired |

---

### Account & Devices

---

#### `GET /account`  🔒

Returns account info, all registered device keys, and current storage usage.

**Response `200`**

```json
{
  "data": {
    "account_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
    "email": "alice@example.com",
    "identity_uuid": "550e8400-e29b-41d4-a716-446655440000",
    "role": "user",
    "device_keys": [
      {
        "device_public_key": "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        "verified": true,
        "added_at": "2026-01-15 10:30:00"
      }
    ],
    "storage_used": 1048576,
    "flagged_for_deletion": null,
    "created_at": "2026-01-15 10:29:00"
  }
}
```

---

#### `DELETE /account`  🔒

Flags the account for deletion. The account and all associated data are permanently deleted after the 90-day grace period by the cleanup cron. The account can be reactivated by logging in again before the grace period expires.

**Response `200`**

```json
{ "data": { "message": "Account flagged for deletion" } }
```

---

#### `POST /account/devices`  🔒

Registers a new device key on the authenticated account and returns a PoP challenge. The device is not usable until verified via `/account/devices/verify`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `device_public_key` | string | ✓ | 64-char hex Ed25519 public key |

```json
{ "device_public_key": "a4d1e2f3..." }
```

**Response `201`**

```json
{
  "data": {
    "challenge": {
      "encrypted_nonce": "a1b2c3d4...",
      "server_public_key": "e5f6a7b8..."
    }
  }
}
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Field absent |
| 400 | `INVALID_DEVICE_KEY` | Not a 64-char hex string |
| 409 | `KEY_EXISTS` | Device key already registered to any account |

---

#### `POST /account/devices/verify`  🔒

Verifies a newly added device by solving its PoP challenge. Uses the same decryption procedure as `/auth/register/verify`. The device must belong to the authenticated account.

**Request body**

| Field | Type | Required |
|-------|------|----------|
| `device_public_key` | string | ✓ |
| `nonce` | string | ✓ | 64-char hex decrypted nonce |

**Response `200`**

```json
{ "data": { "ok": true } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Any required field absent |
| 404 | `NO_CHALLENGE` | No pending `device_add` challenge for this key |
| 403 | `FORBIDDEN` | Challenge belongs to a different account |
| 403 | `INVALID_NONCE` | Decrypted value does not match |

---

#### `DELETE /account/devices/{device_key}`  🔒

Removes a device key from the account. The `{device_key}` path parameter is the 64-char hex public key.

**Response `200`**

```json
{ "data": { "ok": true } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 404 | `NOT_FOUND` | Key not found on this account |

---

### Mailboxes

A mailbox registers an account's interest in bundles for a given workspace. Bundles addressed to a device key whose account has a mailbox for that workspace are routed to that account.

---

#### `POST /mailboxes`  🔒

Registers a mailbox for a workspace.

**Request body**

| Field | Type | Required |
|-------|------|----------|
| `workspace_id` | string | ✓ | Opaque client-defined workspace identifier |

```json
{ "workspace_id": "ws-7f3a9c2e" }
```

**Response `201`**

```json
{ "data": { "workspace_id": "ws-7f3a9c2e" } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | Field absent |

---

#### `DELETE /mailboxes/{workspace_id}`  🔒

Removes a mailbox. Does not delete bundles already routed to the account.

**Response `200`**

```json
{ "data": { "ok": true } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 404 | `NOT_FOUND` | Mailbox not found on this account |

---

#### `GET /mailboxes`  🔒

Lists all mailboxes for the account.

**Response `200`**

```json
{
  "data": [
    {
      "workspace_id": "ws-7f3a9c2e",
      "registered_at": "2026-02-01 08:00:00",
      "pending_bundles": 3,
      "storage_used": 524288
    }
  ]
}
```

---

### Bundles

Bundles are end-to-end encrypted blobs routed from a sender device to one or more recipient devices. The relay never sees plaintext content.

---

#### `POST /bundles`  🔒

Uploads a bundle and routes it to all specified recipient device keys that are registered in the relay. Recipients not found in the relay are silently skipped. The sender's own device key is always skipped even if included in `recipient_device_keys`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `header` | string | ✓ | JSON-encoded routing header (see below) |
| `payload` | string | ✓ | Base64-encoded encrypted bundle data |

**Header JSON fields**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `workspace_id` | string | ✓ | Target workspace |
| `sender_device_key` | string | ✓ | 64-char hex sender key |
| `recipient_device_keys` | string[] | ✓ | Array of 64-char hex recipient keys |
| `mode` | string | — | One of `delta` (default), `snapshot`, `invite`, `accept` |

```json
{
  "header": "{\"workspace_id\":\"ws-7f3a9c2e\",\"sender_device_key\":\"d75a98...\",\"recipient_device_keys\":[\"a4d1e2...\"],\"mode\":\"delta\"}",
  "payload": "base64encodedencrypteddata=="
}
```

**Response `201`**

```json
{
  "data": {
    "routed_to": 1,
    "bundle_ids": ["b8f3c2d1-..."],
    "skipped": {
      "unverified": [],
      "unknown": [],
      "quota_exceeded": []
    }
  }
}
```

`routed_to` is the number of recipients a copy was created for. `skipped` is always present and contains three arrays of device keys that were not routed:

| Key | Meaning | Suggested client message |
|-----|---------|--------------------------|
| `skipped.unverified` | Key is registered but the owner has not completed device verification | "Waiting for recipient to verify their device" |
| `skipped.unknown` | Key is not registered with this relay | "Recipient has not registered with the relay" |
| `skipped.quota_exceeded` | Recipient's account has reached its storage limit | "Recipient's storage is full" |

The sender's own key is always excluded silently and is not counted in any `skipped` category.

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | `header` or `payload` absent |
| 400 | `INVALID_PAYLOAD` | `payload` is not valid base64 |
| 400 | `INVALID_HEADER` | Header JSON malformed or missing required fields |
| 413 | `BUNDLE_TOO_LARGE` | Decoded payload exceeds 10 MB |

---

#### `GET /bundles`  🔒 ⏱

Lists all bundles waiting for any verified device key on this account. Only metadata is returned; use `GET /bundles/{bundle_id}` to download the payload.

This endpoint is **rate-limited**: at most one call per 60 seconds per account. Subsequent calls within the window return `429`.

**Response `200`**

```json
{
  "data": [
    {
      "bundle_id": "b8f3c2d1-...",
      "workspace_id": "ws-7f3a9c2e",
      "sender_device_key": "d75a98...",
      "mode": "delta",
      "size_bytes": 2048,
      "created_at": "2026-03-01 12:00:00"
    }
  ]
}
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 429 | `RATE_LIMITED` | Called again within 60 seconds; includes `retry_after` (seconds) field and `Retry-After` header |

---

#### `GET /bundles/{bundle_id}`  🔒

Downloads a single bundle's payload. Only the recipient account may download it.

**Response `200`**

```json
{
  "data": {
    "bundle_id": "b8f3c2d1-...",
    "workspace_id": "ws-7f3a9c2e",
    "sender_device_key": "d75a98...",
    "mode": "delta",
    "payload": "base64encodedencrypteddata=="
  }
}
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 404 | `NOT_FOUND` | Bundle not found or file missing |
| 403 | `FORBIDDEN` | Bundle belongs to a different account |

---

#### `DELETE /bundles/{bundle_id}`  🔒

Deletes a bundle after the client has processed it. Also decrements the recipient account's storage usage.

**Response `200`**

```json
{ "data": { "ok": true } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 404 | `NOT_FOUND` | Bundle not found |
| 403 | `FORBIDDEN` | Bundle belongs to a different account |

---

### Invites

Invites allow an authenticated user to share an encrypted blob via a single public URL. The recipient does not need an account. The inviter creates an invite with an expiry; the relay holds the blob until fetched or expired.

---

#### `POST /invites`  🔒

Uploads an encrypted invite blob and returns a shareable URL.

**Request body**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `payload` | string | ✓ | Base64-encoded encrypted invite blob |
| `expires_at` | string | ✓ | ISO 8601 UTC datetime, e.g. `2026-06-01T00:00:00Z`; must be in the future and at most 90 days from now |

```json
{
  "payload": "base64encodedencryptedblob==",
  "expires_at": "2026-06-01T00:00:00Z"
}
```

**Response `201`**

```json
{
  "data": {
    "invite_id": "9a3f1b2c-...",
    "token": "1f177c2ee1861dc6...",
    "url": "https://swarm.krillnotes.org/invites/1f177c2ee1861dc6...",
    "expires_at": "2026-06-01T00:00:00Z"
  }
}
```

`token` is a 64-char hex string (256-bit random). The `url` is the public shareable link.

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 400 | `MISSING_FIELDS` | `payload` or `expires_at` absent |
| 400 | `INVALID_PAYLOAD` | `payload` is not valid base64 |
| 400 | `INVALID_EXPIRY` | `expires_at` is in the past, malformed, or more than 90 days away |
| 413 | `PAYLOAD_TOO_LARGE` | Decoded payload exceeds 10 MB |

---

#### `GET /invites`  🔒

Lists all invites created by the authenticated account.

**Response `200`**

```json
{
  "data": [
    {
      "invite_id": "9a3f1b2c-...",
      "token": "1f177c2ee1861dc6...",
      "url": "https://swarm.krillnotes.org/invites/1f177c2ee1861dc6...",
      "expires_at": "2026-06-01 00:00:00",
      "download_count": 2,
      "created_at": "2026-03-14 09:00:00"
    }
  ]
}
```

`download_count` counts only JSON fetches (app downloads), not browser page views.

---

#### `DELETE /invites/{token}`  🔒

Revokes an invite immediately. Deletes the blob from storage. Only the owning account may revoke.

**Response `200`**

```json
{ "data": { "ok": true } }
```

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 404 | `NOT_FOUND` | Token not found |
| 403 | `FORBIDDEN` | Invite belongs to a different account |

---

#### `GET /invites/{token}`

Fetches an invite. **No authentication required.** The response format depends on the `Accept` header.

**Content negotiation**

| `Accept` contains | Response |
|-------------------|----------|
| `application/json` | JSON envelope with base64 payload |
| anything else | HTML landing page for browsers |

Only JSON fetches increment the `download_count`.

**JSON response `200`** (`Accept: application/json`)

```json
{
  "data": {
    "payload": "base64encodedencryptedblob==",
    "expires_at": "2026-06-01 00:00:00"
  }
}
```

**HTML response `200`**

A minimal landing page instructing the recipient to open the URL in the KrillNotes app.

**Errors**

| Status | Code | Cause |
|--------|------|-------|
| 404 | `NOT_FOUND` | Token does not exist (JSON) or "no longer valid" page (HTML) |
| 410 | `GONE` | Invite has expired (JSON) or "no longer valid" page (HTML) |

---

## Error reference

All error responses use this shape:

```json
{ "error": { "code": "SNAKE_CASE", "message": "Human-readable string" } }
```

Some errors include additional fields:

| Code | Extra fields |
|------|-------------|
| `RATE_LIMITED` | `retry_after` (int, seconds); also sets `Retry-After` response header |

---

## Limits

| Setting | Default |
|---------|---------|
| Session lifetime | 30 days |
| Challenge lifetime | 5 minutes |
| Password reset token lifetime | 1 hour |
| Max bundle / invite payload size | 10 MB |
| Max storage per account | 100 MB |
| Bundle retention | 30 days |
| Invite max expiry | 90 days |
| Account deletion grace period | 90 days |
| Minimum bundle poll interval | 60 seconds |
