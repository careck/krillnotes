# File Attachments — Design Document

**Issue:** #23
**Date:** 2026-03-01
**Status:** Approved

---

## Overview

Allow users to attach arbitrary files (images, documents, audio, etc.) to any note. Attachments are encrypted at rest using ChaCha20-Poly1305, consistent with the existing SQLCipher-encrypted database. Each workspace is restructured into a dedicated folder to co-locate its database and encrypted attachment files.

---

## 1. Workspace Folder Structure

### Before (current layout)
```
<workspace_dir>/
  my-notes.db
  journal.db
```

### After (new layout)
```
<workspace_dir>/
  my-notes/
    notes.db              ← SQLCipher-encrypted SQLite (was my-notes.db)
    attachments/
      a1b2c3d4.enc        ← encrypted attachment (UUID filename)
      e5f6g7h8.enc
  journal/
    notes.db
    attachments/
```

`list_workspace_files` is updated to scan for subdirectories containing `notes.db` instead of flat `*.db` files.

### Auto-migration on first launch

On startup, before any workspace is opened, the app scans the workspace directory for flat `*.db` files and migrates each one:
1. Create `<stem>/` directory
2. Move `<name>.db` → `<stem>/notes.db`
3. Create `<stem>/attachments/`

If a file is locked or the move fails, log a warning and skip — the workspace remains openable in its old location until the next launch.

---

## 2. Encryption Scheme

### Crates (added to `krillnotes-core`)
- `chacha20poly1305 = "0.10"` — authenticated encryption
- `hkdf = "0.12"` — key derivation
- `sha2 = "0.10"` — hash for HKDF and integrity checks

Pure-Rust RustCrypto crates; no native build dependencies, portable across all Tauri targets.

### Workspace attachment key (derived at open time)

At `Workspace::create`: generate a UUID, write to `workspace_meta` as `workspace_id`.
At `Workspace::open`: read `workspace_id` from `workspace_meta`, then:

```
attachment_key = HKDF-SHA256(
    ikm  = password_bytes,
    salt = workspace_id_bytes,
    info = b"krillnotes-attachment-v1"
)
```

The derived 32-byte key is stored in the `Workspace` struct. The raw password is **not** retained after open. For unencrypted workspaces (empty password): `attachment_key = None`; attachments are stored plaintext.

### Per-file encryption

```
file_key = HKDF-SHA256(
    ikm  = attachment_key,
    salt = random_32_byte_salt,
    info = b"krillnotes-file-v1"
)
ciphertext = ChaCha20-Poly1305(file_key, random_12_byte_nonce, plaintext)
```

**Disk format (`.enc` file):**
```
[ 12-byte nonce ][ 32-byte salt ][ ciphertext + 16-byte auth tag ]
```

### Unencrypted workspace fallback

If `attachment_key` is `None` (empty password), attachments are written as raw bytes with a `.enc` extension but no encryption header. A magic byte prefix distinguishes encrypted from unencrypted files on read.

---

## 3. Database Schema

New table added via the existing migration system:

```sql
CREATE TABLE attachments (
    id          TEXT PRIMARY KEY,   -- UUID, matches .enc filename on disk
    note_id     TEXT NOT NULL,
    filename    TEXT NOT NULL,      -- original filename (e.g. "photo.jpg")
    mime_type   TEXT,               -- e.g. "image/jpeg"
    size_bytes  INTEGER NOT NULL,   -- original unencrypted size
    hash_sha256 TEXT NOT NULL,      -- SHA-256 of plaintext bytes (integrity)
    salt        BLOB NOT NULL,      -- 32-byte HKDF per-file salt
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
);
CREATE INDEX idx_attachments_note_id ON attachments(note_id);
```

### Workspace size limit

Stored as a `workspace_meta` key: `attachment_max_size_bytes` (integer or absent = unlimited). Enforced at attach time — attempting to add a file exceeding the limit returns an error surfaced in the UI. No settings UI in v1; configurable programmatically or via a future workspace settings tab.

---

## 4. Workspace Struct Changes

### New fields

```rust
pub struct Workspace {
    // ... existing fields unchanged ...
    attachment_key: Option<[u8; 32]>,  // None = unencrypted workspace
    workspace_root: PathBuf,           // workspace folder (parent of notes.db)
}
```

`workspace_root` is used to derive the attachments directory: `workspace_root/attachments/`.

### New public methods

| Method | Description |
|---|---|
| `attach_file(note_id, filename, mime_type, bytes) -> Result<AttachmentMeta>` | Encrypt + write to disk, insert DB row. Enforces size limit. |
| `get_attachments(note_id) -> Result<Vec<AttachmentMeta>>` | Query DB for metadata, no disk I/O. |
| `get_attachment_bytes(id) -> Result<Vec<u8>>` | Read `.enc` file, decrypt, verify hash. |
| `delete_attachment(id) -> Result<()>` | Delete DB row + `.enc` file. |
| `attachment_max_size_bytes() -> Option<u64>` | Read workspace_meta. |
| `set_attachment_max_size_bytes(limit: Option<u64>) -> Result<()>` | Write workspace_meta. |

### New Tauri commands

| Command | Description |
|---|---|
| `attach_file(window, note_id, file_path)` | Reads file from disk, calls `workspace.attach_file` |
| `get_attachments(window, note_id)` | Returns `Vec<AttachmentMeta>` |
| `get_attachment_bytes(window, attachment_id)` | Returns base64-encoded bytes for UI display |
| `delete_attachment(window, attachment_id)` | Deletes attachment |
| `open_attachment(window, attachment_id)` | Decrypts to temp file, opens via `plugin-opener` |

---

## 5. Export / Import

### Export (`.krillnotes` zip)

After existing `notes.json` and `scripts/` entries:
1. Query all attachments in the workspace DB
2. For each: decrypt bytes, add to zip as `attachments/<uuid>/<original_filename>`
   (UUID subfolder avoids name collisions across notes)
3. Add an `attachments.json` manifest listing all attachment metadata (id, note_id, filename, mime_type, size_bytes, hash_sha256) — used during import to restore DB rows

Attachments are stored **plaintext** inside the zip (relying on the zip's AES password for at-rest protection).

### Import

1. Parse `attachments.json` from archive
2. For each entry: read plaintext bytes from `attachments/<uuid>/<filename>`, re-encrypt using the new workspace's `attachment_key`, write to `<workspace_root>/attachments/<uuid>.enc`, insert DB row

---

## 6. UI

### InfoPanel — Attachments Section

A new collapsible "Attachments" section below the tags section in `InfoPanel.tsx`:

- **Add button** — opens a file picker via `@tauri-apps/plugin-dialog`; also accepts drag-and-drop onto the InfoPanel
- **Image attachments** — shown as thumbnails (decoded in-memory, rendered as `<img>` via data URL)
- **Non-image attachments** — file-type icon + original filename + human-readable size
- **Open** — click thumbnail/filename → `open_attachment` command (decrypts to temp, calls plugin-opener)
- **Delete** — trash icon per attachment; confirmation dialog via `@tauri-apps/plugin-dialog`
- Size limit violation shown as an inline error toast

---

## 7. Considerations & Future Work

- **Orphan cleanup:** On workspace open, scan `attachments/` for `.enc` files with no matching DB row and delete them. Also log DB rows pointing to missing files.
- **Large files:** v1 decrypts fully in memory. Chunked streaming can be added in a follow-up if needed for video support.
- **Sync:** The derived `attachment_key` is workspace-scoped, enabling peers to exchange attachments without sharing the master password — key is passed alongside the attachment data during sync.
- **Thumbnails:** Pre-generated encrypted thumbnails for fast list preview can be added as a follow-up.
- **Backup:** Documenting that a complete backup requires both `notes.db` and `attachments/` is deferred to a docs update.
