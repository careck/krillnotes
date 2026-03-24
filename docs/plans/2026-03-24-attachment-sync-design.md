# Attachment Sync via Operations Log — Design Spec

**Date:** 2026-03-24
**Ticket:** #117 — Attachments are not added via OperationsLog and are not transferred in delta bundles

## Problem

Adding or removing an attachment does not log an operation. Since the sync system replicates via the operations log, attachment mutations are invisible to delta bundles and never reach peers.

Snapshots already include attachment metadata and encrypted file bytes (`attachments/<id>.enc` in the ZIP). But delta sync — the incremental, ongoing mechanism — has no awareness of attachments.

## Solution

Extend the operations log with two new signed operation variants (`AddAttachment`, `RemoveAttachment`) and extend the delta bundle format to carry attachment file bytes as sidecar files, mirroring the existing snapshot pattern.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Operation carries file bytes? | No — metadata only, bytes as ZIP sidecar | Keeps operations table lean, follows snapshot pattern |
| Deletion propagates to peers? | Yes | Matches `DeleteNote` semantics, keeps workspaces in sync |
| Operations signed? | Yes | Required for RBAC gating, consistent with all propagating ops |
| Content-addressable dedup? | No | Keep it simple; optimize later if needed |
| Backfill ops for existing attachments? | No | Snapshots already cover them; pre-1.0 acceptable gap |
| Protocol version bump? | No | Not needed |

## New Operation Variants

### `AddAttachment`

```rust
AddAttachment {
    operation_id: String,
    timestamp: HlcTimestamp,
    device_id: String,
    attachment_id: String,
    note_id: String,
    filename: String,
    mime_type: Option<String>,
    size_bytes: i64,
    hash_sha256: String,
    added_by: String,       // identity public key
    signature: String,
}
```

### `RemoveAttachment`

```rust
RemoveAttachment {
    operation_id: String,
    timestamp: HlcTimestamp,
    device_id: String,
    attachment_id: String,
    note_id: String,         // for RBAC scoping
    removed_by: String,      // identity public key
    signature: String,
}
```

Both variants are signed, carry the author's identity public key, and participate in RBAC (writer+ required).

## Delta Bundle Format Changes

**Current format:**
- `header.json`
- `payload.enc` (encrypted `Vec<Operation>` JSON)
- `signature.bin`

**Extended format** (when batch contains `AddAttachment` ops):
- `header.json` — gains `has_attachments: bool` (same field snapshots use)
- `payload.enc` — unchanged (encrypted operations JSON including attachment ops)
- `attachments/<attachment_id>.enc` — one file per `AddAttachment` op, encrypted with the shared symmetric key via `encrypt_blob()`
- `signature.bin` — signs header + payload (attachment authenticity from AES-GCM tags, same as snapshots)

### Crypto Function Changes

The delta codec currently uses `encrypt_for_recipients()` (discards the symmetric key) and `decrypt_payload()` (also discards it). To encrypt/decrypt attachment sidecar blobs, the delta path must switch to:

- **Build side:** `encrypt_for_recipients_with_key()` → returns `(ciphertext, sym_key, entries)` — same function snapshots use
- **Parse side:** `decrypt_payload_with_key()` → returns `(plaintext, sym_key)` — sym_key needed for `decrypt_blob()`

Additionally, `DeltaParams` gains a new field:
```rust
pub attachment_blobs: Vec<(String, Vec<u8>)>,  // (attachment_id, plaintext_bytes)
```

And `ParsedDelta` gains:
```rust
pub attachment_blobs: Vec<(String, Vec<u8>)>,  // (attachment_id, decrypted_bytes)
```

### Building a Delta

1. Fetch operations since watermark (existing logic)
2. Scan batch for `AddAttachment` ops
3. For each: `workspace.get_attachment_bytes(&attachment_id)` → plaintext
4. Pass `attachment_blobs: Vec<(String, Vec<u8>)>` to `create_delta_bundle()` via `DeltaParams`
5. `create_delta_bundle()` calls `encrypt_for_recipients_with_key()` (instead of `encrypt_for_recipients()`) to obtain the `sym_key`
6. Encrypts each blob with `encrypt_blob(&sym_key, &plaintext)`, writes to `attachments/<id>.enc` in ZIP

### Applying a Delta

1. `parse_delta_bundle()` calls `decrypt_payload_with_key()` (instead of `decrypt_payload()`) to recover the `sym_key`
2. Scans ZIP for `attachments/*.enc` entries, decrypts each with `decrypt_blob(&sym_key, &ct)`
3. Returns decrypted blobs in `ParsedDelta.attachment_blobs`
4. For `AddAttachment` ops: match blob by ID, call `attach_file_with_id()` to store locally
5. For `RemoveAttachment` ops: hard-delete `.enc` file + remove DB row (no soft-delete for remote ops)
6. Missing blob for an `AddAttachment`: log warning, don't fail (record the op for log consistency)
7. `RemoveAttachment` for nonexistent attachment: no-op (idempotent)
8. `AddAttachment` for a `note_id` that no longer exists (deleted note): skip the file write, still record the operation in the log for consistency

## Workspace Method Changes

### `attach_file()` — new signature

```rust
pub fn attach_file(
    &mut self,
    note_id: &str,
    filename: &str,
    mime_type: Option<&str>,
    data: &[u8],
    signing_key: Option<&SigningKey>,
) -> Result<AttachmentMeta>
```

- `Some(key)`: write file + insert DB row + build/sign/log `AddAttachment` op
- `None`: write file + insert DB row only (import/snapshot path, op already exists)

### `delete_attachment()` — new signature

```rust
pub fn delete_attachment(
    &mut self,
    attachment_id: &str,
    signing_key: Option<&SigningKey>,
) -> Result<()>
```

- `Some(key)`: soft-delete file (`.trash`) + remove DB row + push undo entry + build/sign/log `RemoveAttachment` op
- `None`: hard-delete `.enc` file + remove DB row (remote apply, no undo)

### `attach_file_with_id()` — unchanged

Already used for import/snapshot apply. Called during delta apply for `AddAttachment` ops. No op logged.

### `apply_incoming_operation()` — new parameter

```rust
pub fn apply_incoming_operation(
    &mut self,
    op: &Operation,
    received_from: &str,
    attachment_blobs: &[(String, Vec<u8>)],
) -> Result<bool>
```

New match arms for `AddAttachment` and `RemoveAttachment`. Non-attachment ops ignore the blobs parameter.

## RBAC

- `AddAttachment`: writer+ on target `note_id`
- `RemoveAttachment`: writer+ on target `note_id`
- Same gating pattern as `UpdateField` / `DeleteNote`

## Undo

**Note:** The current `delete_attachment()` does NOT push an undo entry despite its doc comment claiming otherwise. The undo push must be added as part of this feature.

- **Local delete**: `delete_attachment(id, Some(key))` soft-deletes file + removes DB row + pushes `AttachmentRestore` undo entry (**new** — not currently implemented) + logs `RemoveAttachment` op (**new**)
- **Undo local delete**: `restore_attachment()` runs (existing) + logs `RetractOperation { propagate: true }` so peers undo the removal
- **Local add**: `attach_file(...)` logs `AddAttachment` op. Undo pushes `AttachmentSoftDelete` inverse + logs retract
- **Remote ops**: no undo entries — remote mutations are applied without local undo state
- Existing `RetractInverse` variants (`AttachmentRestore`, `AttachmentSoftDelete`) are reused — no new variants needed

## Tauri Command Layer

Commands in `commands/attachments.rs` need to pass the signing key to `attach_file()` and `delete_attachment()`. The signing key is available in `AppState` (loaded from identity). Small plumbing change.

## Backward Compatibility

- **No DB migration**: operations table stores JSON blobs with string discriminator; new variants are new discriminator values
- **Old peers**: will fail to deserialize unknown variants. Acceptable for pre-1.0; sync is v0.9+ only
- **Existing attachments**: no backfill. They continue working locally and sync via snapshots (which already include them)
- **No protocol version bump**: not needed

## Files to Modify

| File | Changes |
|------|---------|
| `krillnotes-core/src/core/operation.rs` | Add `AddAttachment`, `RemoveAttachment` variants + signing/verification |
| `krillnotes-core/src/core/workspace/attachments.rs` | Add `signing_key` param to `attach_file()`, `delete_attachment()`. Log ops when key provided. Add undo push to `delete_attachment()` |
| `krillnotes-core/src/core/workspace/sync.rs` | Add `attachment_blobs` param to `apply_incoming_operation()`. New match arms for both variants |
| `krillnotes-core/src/core/swarm/delta.rs` | Switch to `encrypt_for_recipients_with_key`/`decrypt_payload_with_key`. Add `attachment_blobs` to `DeltaParams` and `ParsedDelta`. Handle `attachments/*.enc` sidecar files |
| `krillnotes-core/src/core/swarm/sync.rs` | Update `generate_delta()` to scan ops for `AddAttachment`, collect blobs from workspace, pass to `create_delta_bundle()`. Update `apply_delta()` to extract blobs and pass to `apply_incoming_operation()` |
| `krillnotes-core/src/core/sync/mod.rs` | Update `SyncEngine::poll()` phase 2 op application loop — `PendingDelta` must carry `attachment_blobs`, pass them through to `apply_incoming_operation()` during merge-sort application |
| `krillnotes-core/src/core/swarm/header.rs` | Add `has_attachments` to delta header (if not already present) |
| `krillnotes-core/src/core/undo.rs` | Wire `AddAttachment`/`RemoveAttachment` into undo group logic |
| `krillnotes-desktop/src-tauri/src/commands/attachments.rs` | Pass signing key from `AppState` to workspace methods |
| `krillnotes-desktop/src-tauri/src/commands/swarm.rs` | Pass attachment blobs through delta create/apply paths |
