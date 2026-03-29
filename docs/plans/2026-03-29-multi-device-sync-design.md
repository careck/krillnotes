# Multi-Device Sync for Same Identity — Design Spec

**Date:** 2026-03-29
**Issue:** #112
**Status:** Draft

## Problem

A user can export their identity (`.swarmid`) to another device, but there is no mechanism to:
1. Send a workspace snapshot to that other device (snapshots currently only go to *different* peer identities via the invite flow)
2. Keep two devices running the same identity in continuous sync (the delta echo filter and peer registry cannot distinguish them)

## Root Cause

`device_id` in every operation is set to `identity_uuid` (not hardware device ID). This was a deliberate choice so multiple identities on one machine get distinct device IDs. But it means two devices running the same identity share the same `device_id`, causing:

- **Delta echo filter** (`operations_since`) excludes ops where `device_id == exclude_device_id` — a delta from Device A to Device B excludes ALL ops from the shared identity
- **Peer registry** is keyed by `peer_device_id` (= identity UUID) — Device A and Device B collide as one peer with shared watermarks
- **Relay routing** is by `recipient_device_key` (Ed25519 public key) — same identity = same key = can't route to a specific device

## Design Decisions

- **Continuous sync**, not one-time clone — both devices stay in sync via normal delta exchange
- **Explicit per-workspace bootstrap** — user manually triggers "send this workspace to my other device"; no automatic workspace discovery
- **Relay AND manual delivery** — supports both relay upload and file export for the initial snapshot; channel type is determined by delivery method
- **Breaking change acceptable** — no backwards compatibility needed; only developer is testing
- **Relay codebase changes included** — relay server at `~/Source/krillnotes-relay` will be modified

## 1. Device Identity

### Device UUID Generation

Each identity gets a stable random UUID per machine, generated once and persisted at:

```
~/.config/krillnotes/identities/<identity_uuid>/device_id
```

This file contains a plain UUID string (e.g. `"f47ac10b-58cc-4372-a567-0e02b2c3d479"`). Created on first workspace open if absent.

### Composite device_id

The `device_id` stored in `workspace_meta` and stamped on every operation becomes:

```
{identity_uuid}:{device_uuid}
```

Example: `"a1b2c3d4-...:f47ac10b-..."`

This preserves the ability to extract identity from device_id (split on `:`, take first half) while making each device distinguishable.

### Code Changes

- `Workspace::create()` and `Workspace::open()` in `workspace/mod.rs`: read the device UUID from the identity directory, compose `{identity_uuid}:{device_uuid}` as `device_id`
- `identity.rs`: new function `ensure_device_uuid(identity_dir: &Path) -> Result<String>` that reads or creates `device_id` file
- Helper function `identity_from_device_id(device_id: &str) -> &str` to extract the identity UUID prefix

## 2. RegisterDevice Operation

New variant added to the `Operation` enum:

```rust
RegisterDevice {
    operation_id: String,
    timestamp: HlcTimestamp,
    device_id: String,
    identity_public_key: String,
    device_uuid: String,
    device_name: String,
    signature: String,
}
```

### Emission

On workspace open, check if a `RegisterDevice` operation exists in the log for the current `device_uuid`. If not, emit one. This is a one-time event per (workspace, device) pair.

### device_name Resolution

1. Use `gethostname()` (via `libc` or `hostname::get()`)
2. If result is empty, `"localhost"`, or `"unknown"` — fall back to `"{OS} Device"` using `std::env::consts::OS` (e.g. `"macos Device"`, `"linux Device"`)
3. The name is a display string only, not an identifier — editable in future UI

### Semantics

- Travels through normal sync like any other operation
- Other peers can display it (e.g. "Alice — MacBook Pro") but don't need to act on it
- RBAC ignores it — no permission implications
- Signed with the identity's Ed25519 key like all other operations

## 3. Self-Snapshot Bootstrap

### Sending (Device A)

A "Send to My Device" button in the peer list dialog triggers:

1. Create a workspace snapshot via existing `Workspace::to_snapshot_json()`
2. Encrypt to the user's own identity key (same key both devices hold)
3. Deliver via:
   - **Relay:** Upload with the sender's own public key as both sender and recipient, but with distinct `recipient_device_id` values (see Section 5)
   - **Manual:** Export as `.swarm` file for physical transfer

No invite handshake — same identity = implicit trust.

### Receiving (Device B)

When Device B receives and applies a snapshot where `sender_device_key == local identity key`:

1. Apply snapshot normally (create workspace, import notes/scripts/attachments)
2. Register the sender as a self-peer:
   - `peer_device_id` = sender's composite device_id (from bundle header `sender_device_id`)
   - `peer_identity_id` = own public key
   - `channel_type` = `"relay"` or `"manual"` depending on delivery method
   - `channel_params` = relay URL/account info if relay, empty if manual
3. Emit a `RegisterDevice` operation for the local device
4. Normal delta sync begins on next sync cycle

### Distinguishing Self-Snapshot from Peer Snapshot

A self-snapshot is identified by: `sender_device_key == local identity public key`. No new bundle mode needed — it's just a `"snapshot"` mode bundle where sender and recipient happen to be the same identity.

## 4. Peer Registry Changes

### Self-Peers

A self-peer is a peer entry where `peer_identity_id == local identity public key`. Code can use this to:

- Group self-peers under "My Devices" in the UI
- Skip RBAC checks (same identity = same permissions)
- Display device names from `RegisterDevice` operations

### No Schema Changes

The `sync_peers` table already has all needed columns. Self-peers are just regular peer entries with a recognisable `peer_identity_id`. The composite `device_id` ensures each device gets its own row.

## 5. Relay Routing Changes

### Bundle Header

Add two new fields to `BundleHeader`:

```rust
pub struct BundleHeader {
    pub workspace_id: String,
    pub sender_device_key: String,
    pub sender_device_id: String,              // NEW
    pub recipient_device_keys: Vec<String>,
    pub recipient_device_ids: Vec<String>,      // NEW — parallel array, one per recipient
    pub mode: Option<String>,
}
```

### Relay Database Changes (krillnotes-relay)

New migration `009_add_device_id.sql`:

```sql
ALTER TABLE bundles ADD COLUMN recipient_device_id TEXT;
CREATE INDEX idx_bundles_device_id ON bundles(recipient_device_id);

ALTER TABLE device_keys ADD COLUMN device_id TEXT;
```

### Relay Routing Changes

**Upload (`BundleRoutingService::routeBundle()`):**
- Parse `recipient_device_ids` from header (parallel array with `recipient_device_keys`)
- Store `recipient_device_id` in the `bundles` table alongside `recipient_device_key`

**Polling (`BundleRepository::listForRecipientKeys()`):**
- Accept required `device_id` parameter from the polling client
- Filter: `WHERE recipient_device_key IN (...) AND recipient_device_id = ?`
- No backwards-compat fallback needed (breaking changes acceptable)

**Device registration:**
- When a device key is registered or verified, the client can optionally provide its `device_id`
- Stored in `device_keys.device_id` for future reference

### Client Polling Changes

`receive_poll.rs` sends `device_id` as a query parameter or header when polling, so the relay can filter bundles for this specific device.

## 6. Delta Echo Filter

**No code changes needed to `operations_since()`.** The existing SQL logic:

```sql
WHERE device_id != ?  -- exclude_device_id
  AND (received_from_peer IS NULL OR received_from_peer != ?)
```

Works correctly with composite device_ids because Device A (`alice:macbook`) and Device B (`alice:desktop`) have distinct `device_id` values. When generating a delta for Device B, we pass `exclude_device_id = "alice:desktop"` — Device A's ops (`"alice:macbook"`) pass through.

## 7. UI Changes

### Peer List Dialog

**New button: "Send to My Device"**

Placement: In the peer list dialog, above or below the peer list.

Behavior:
1. Click opens a choice: "Via Relay" or "Export File"
2. **Via Relay:** Creates snapshot, uploads to relay addressed to own identity + a target device_id. If no other device is known yet (first time), the user must provide the device_id from Device B or use the file method first.
3. **Export File:** Creates snapshot, saves as `.swarm` file via system save dialog.

**Bootstrap via relay:** Device B doesn't exist in the peer registry yet, so Device A doesn't know its device_id. The flow is:
1. User imports `.swarmid` on Device B and opens the app
2. Device B registers its device key with the relay (existing flow) — this includes the new `device_id` field
3. On Device A, user clicks "Send to My Device" → "Via Relay"
4. Device A queries the relay for other devices on the same account (relay already has `device_keys` table with multiple entries per account — add a `GET /account/devices` endpoint)
5. Device A shows a picker if multiple other devices exist, or auto-selects if there's exactly one
6. Snapshot is uploaded addressed to the selected device

**Bootstrap via file:** Always works without relay. Device A exports `.swarm` file, user transfers it to Device B, Device B imports it. After first delta exchange, both devices learn each other's device_id.

### "My Devices" Grouping

In the peer list, peers where `peer_identity_id == local identity public key` are displayed in a separate "My Devices" section showing:
- Device name (from `RegisterDevice` operation)
- Channel type (relay / manual)
- Last sync time
- Sync status

## 8. Security Considerations

- **No new trust model:** Same identity = same key = full trust. No additional verification needed.
- **Snapshot encryption:** Self-snapshots use the same ECDH key encapsulation as peer snapshots. The sender encrypts to their own public key; the recipient (same key on different device) decrypts with the same private key.
- **Operation signatures:** Both devices sign with the same Ed25519 key. All signatures are valid from any recipient's perspective. The `device_id` field distinguishes which device authored each operation, but cryptographically they are the same author.
- **Relay security:** The relay never sees plaintext. Adding device_id routing doesn't change the threat model — the relay already routes by recipient key, device_id is just a sub-routing discriminator.

## 9. What This Does NOT Cover

- **Automatic workspace discovery** — user must manually send each workspace to Device B. A future enhancement could add a workspace manifest to the relay.
- **Device management UI** — no rename/revoke device screen. Just the "My Devices" section in the peer list.
- **Conflict resolution** — if the same note is edited on both devices simultaneously, the existing HLC-based last-writer-wins applies. No new conflict model.
- **Three+ devices** — works naturally. Each device has a unique composite device_id and acts as an independent peer. Snapshots must be sent to each new device individually.
