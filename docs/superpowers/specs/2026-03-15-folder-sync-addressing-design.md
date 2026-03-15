# Folder Sync Addressing — Design Spec

**Date:** 2026-03-15
**Status:** Approved

## Problem

The folder sync channel (`FolderChannel`) uses a flat shared directory where all peers read and write `.swarm` bundle files. Two bugs exist:

1. **No inbox filtering:** Every device in the shared folder tries to decrypt every bundle it didn't write itself. When Alice and Charlie share the same folder, Alice reads Charlie's bundles and gets "no recipient entry matched our key" errors. These bundles are never deleted (correctly), so the error repeats on every poll cycle forever.

2. **Silent channel config failure:** `update_channel_config` issues a SQL `UPDATE … WHERE peer_device_id = ?`. If the device ID is stale (e.g. the peer row was recently consolidated from a placeholder `identity:<pubkey>` to a real device ID via `upsert_peer_from_delta`), the UPDATE matches 0 rows and silently does nothing. The caller receives no error and the config change is lost.

## Design

### Fix 1 — Recipient-prefixed filenames

**New filename format:**

```
{RECIPIENT_identity_short}_{timestamp}_{uuid_short}.swarm
```

where `RECIPIENT_identity_short` = first 8 chars of the recipient peer's `peer_identity_id` (base64 Ed25519 public key).

**Sender side (`send_bundle`):**
Derive `recipient_short` from `peer.peer_identity_id.chars().take(8)`. Write the bundle to `dir/{recipient_short}_{timestamp}_{uuid_short}.swarm`.

**Receiver side (`receive_bundles_from_dir`):**
Replace the old "skip own files" sender-prefix filter with an inbox filter: only collect files whose filename starts with `{MY_identity_short}_`. Files not matching this prefix are silently skipped — this naturally handles old-format files (written before this change) and bundles addressed to other peers.

**Delete on success:**
A device that successfully applies a bundle deletes it via `acknowledge()`. This is safe: the file was addressed to this device, so no other peer needs it.

**Why this works for shared folders:**
- Alice reads only files starting with `ALICE_SHORT_`. Charlie reads only `CHARLIE_SHORT_`.
- No cross-decryption attempts, no "not for us" errors.
- Old-format files (`{SENDER_SHORT}_{DEVICE_SHORT}_{ts}_{uuid}.swarm`) don't start with any active device's identity prefix and are silently ignored.
- No subdirectories, no sidecar metadata — flat folder is preserved.

**Backward compatibility note:**
Old-format bundles are ignored by the new receiver. Any `.swarm` files already in the folder from before this change will remain but never be processed. Users should clear the sync folder once after upgrading.

### Fix 2 — Detect silent channel config failure

In `PeerRegistry::update_channel_config`, check the row count returned by `conn.execute()`. If 0 rows were affected, return `KrillnotesError::Sync("peer not found: {peer_device_id}")`. This surfaces through the Tauri command as a user-visible error so the UI can reload peers and retry.

## Files Changed

| File | Change |
|------|--------|
| `krillnotes-core/src/core/sync/folder.rs` | New filename format in `send_bundle`; inbox-prefix filter in `receive_bundles_from_dir` |
| `krillnotes-core/src/core/peer_registry.rs` | Check row count in `update_channel_config` |

## Non-Changes

- No new error variants needed.
- No subdirectory creation logic.
- No changes to bundle encryption, headers, or the sync engine dispatch loop.
- The `acknowledge` (delete) behaviour on successful apply is unchanged.

## Testing

- Existing `test_folder_channel_send_creates_file`: update expected filename pattern.
- Existing `test_folder_channel_receive_filters_own_bundles`: rewrite to test inbox-prefix filtering instead of sender-prefix filtering.
- New test: `test_folder_channel_ignores_other_recipient_files` — place a file addressed to a different identity short in the folder, verify it is not returned.
- New test: `test_folder_channel_ignores_old_format_files` — place an old-format file in the folder, verify it is silently skipped.
- New test: `test_update_channel_config_unknown_peer_returns_error` — call `update_channel_config` with a non-existent device ID, verify `Err` is returned.
