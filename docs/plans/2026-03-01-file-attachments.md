# File Attachments Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add encrypted file attachment support to notes — per-workspace folder layout, ChaCha20-Poly1305 encryption, InfoPanel UI, and full export/import support.

**Architecture:** Each workspace becomes a folder (`<name>/notes.db` + `<name>/attachments/<uuid>.enc`). A workspace-scoped attachment key is derived via HKDF from the master password + a stable workspace UUID. Encrypted sidecar files live outside the SQLite database to keep the DB fast. The app auto-migrates flat `.db` files to the new folder layout on startup.

**Tech Stack:** Rust (`chacha20poly1305`, `hkdf`, `sha2`, `rand`), Tauri v2, React/TypeScript, `@tauri-apps/plugin-dialog` (file picker), `@tauri-apps/plugin-opener` (open attachments)

---

## Task 1: Add crypto crates to `krillnotes-core`

**Files:**
- Modify: `krillnotes-core/Cargo.toml`

**Step 1: Add dependencies**

```toml
# Encryption
chacha20poly1305 = "0.10"
hkdf = "0.12"
sha2 = "0.10"
rand = "0.8"
```

**Step 2: Build to confirm it compiles**

```bash
cargo build -p krillnotes-core
```

Expected: success (or only pre-existing warnings)

**Step 3: Commit**

```bash
git add krillnotes-core/Cargo.toml Cargo.lock
git commit -m "chore: add chacha20poly1305, hkdf, sha2, rand crates for attachment encryption"
```

---

## Task 2: Add error variants for attachments

**Files:**
- Modify: `krillnotes-core/src/core/error.rs`

**Step 1: Write the failing test**

Add to `error.rs` tests:

```rust
#[test]
fn test_attachment_error_variants_exist() {
    let e = KrillnotesError::AttachmentEncryption("bad key".to_string());
    assert!(e.to_string().contains("encryption") || e.to_string().contains("Encryption"));

    let e2 = KrillnotesError::AttachmentTooLarge { size: 200, limit: 100 };
    assert!(e2.to_string().contains("200"));
}
```

**Step 2: Run to verify it fails**

```bash
cargo test -p krillnotes-core -- test_attachment_error_variants_exist 2>&1 | tail -5
```

Expected: compile error (variants don't exist)

**Step 3: Add variants to `KrillnotesError`**

In `error.rs`, add inside the enum:

```rust
/// Attachment encryption or decryption failed.
#[error("Attachment encryption error: {0}")]
AttachmentEncryption(String),

/// Attachment exceeds the workspace size limit.
#[error("Attachment too large: {size} bytes (limit: {limit} bytes)")]
AttachmentTooLarge { size: u64, limit: u64 },
```

Also add to `user_message()`:

```rust
Self::AttachmentEncryption(_) => "Could not encrypt or decrypt the attachment".to_string(),
Self::AttachmentTooLarge { size, limit } => {
    format!("File too large ({} bytes). This workspace limits attachments to {} bytes.", size, limit)
}
```

**Step 4: Run test**

```bash
cargo test -p krillnotes-core -- test_attachment_error_variants_exist
```

Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/error.rs
git commit -m "feat: add AttachmentEncryption and AttachmentTooLarge error variants"
```

---

## Task 3: DB schema — `attachments` table

**Files:**
- Modify: `krillnotes-core/src/core/schema.sql`
- Modify: `krillnotes-core/src/core/storage.rs`

**Step 1: Write the failing migration test**

Add to `storage.rs` tests:

```rust
#[test]
fn test_attachments_table_exists_on_new_workspace() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Storage::create(temp.path(), "").unwrap();
    let count: i64 = storage.connection().query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_attachments_table_migration_on_existing_workspace() {
    let temp = NamedTempFile::new().unwrap();
    // Create raw DB without attachments table
    {
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, node_type TEXT NOT NULL,
             parent_id TEXT, position INTEGER NOT NULL, created_at INTEGER NOT NULL,
             modified_at INTEGER NOT NULL, created_by INTEGER NOT NULL DEFAULT 0,
             modified_by INTEGER NOT NULL DEFAULT 0, fields_json TEXT NOT NULL DEFAULT '{}',
             is_expanded INTEGER DEFAULT 1);
             CREATE TABLE operations (id INTEGER PRIMARY KEY AUTOINCREMENT,
             operation_id TEXT UNIQUE NOT NULL, timestamp INTEGER NOT NULL,
             device_id TEXT NOT NULL, operation_type TEXT NOT NULL,
             operation_data TEXT NOT NULL, synced INTEGER DEFAULT 0);
             CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
        ).unwrap();
    }
    let storage = Storage::open(temp.path(), "").unwrap();
    let count: i64 = storage.connection().query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
```

**Step 2: Run to verify they fail**

```bash
cargo test -p krillnotes-core -- test_attachments_table 2>&1 | tail -10
```

Expected: FAIL (table not found)

**Step 3: Add to `schema.sql`**

Append at the end of `krillnotes-core/src/core/schema.sql`:

```sql
-- Attachment metadata (encrypted files live on disk in attachments/ directory)
CREATE TABLE IF NOT EXISTS attachments (
    id          TEXT PRIMARY KEY,
    note_id     TEXT NOT NULL,
    filename    TEXT NOT NULL,
    mime_type   TEXT,
    size_bytes  INTEGER NOT NULL,
    hash_sha256 TEXT NOT NULL,
    salt        BLOB NOT NULL,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_attachments_note_id ON attachments(note_id);
```

**Step 4: Add migration to `storage.rs` `run_migrations`**

After the `note_links` migration block, add:

```rust
// Migration: add attachments table if absent.
let attachments_exists: bool = conn.query_row(
    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
    [],
    |row| row.get::<_, i64>(0).map(|c| c > 0),
)?;
if !attachments_exists {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS attachments (
            id          TEXT PRIMARY KEY,
            note_id     TEXT NOT NULL,
            filename    TEXT NOT NULL,
            mime_type   TEXT,
            size_bytes  INTEGER NOT NULL,
            hash_sha256 TEXT NOT NULL,
            salt        BLOB NOT NULL,
            created_at  INTEGER NOT NULL,
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_attachments_note_id ON attachments(note_id);",
    )?;
}
```

**Step 5: Run tests**

```bash
cargo test -p krillnotes-core -- test_attachments_table
```

Expected: both PASS

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/schema.sql krillnotes-core/src/core/storage.rs
git commit -m "feat: add attachments table to schema and migration"
```

---

## Task 4: New `attachment.rs` module — crypto primitives

**Files:**
- Create: `krillnotes-core/src/core/attachment.rs`
- Modify: `krillnotes-core/src/core/mod.rs`

**Step 1: Write failing tests first**

Create `krillnotes-core/src/core/attachment.rs` with tests only:

```rust
//! Attachment crypto primitives and metadata types.

use crate::{KrillnotesError, Result};
use hkdf::Hkdf;
use sha2::Sha256;
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Metadata for a single file attachment stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub id: String,
    pub note_id: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub hash_sha256: String,
    /// 32-byte HKDF per-file salt (hex-encoded for Tauri serialisation).
    pub salt: String,
    pub created_at: i64,
}

/// Derives a 32-byte workspace attachment key from the master password and a
/// workspace-unique UUID string (used as the HKDF salt).
pub fn derive_attachment_key(password: &str, workspace_id: &str) -> [u8; 32] {
    // TODO: implement
    [0u8; 32]
}

/// Derives a 32-byte per-file key from the workspace attachment key and a
/// random per-file salt.
fn derive_file_key(attachment_key: &[u8; 32], file_salt: &[u8; 32]) -> [u8; 32] {
    // TODO: implement
    [0u8; 32]
}

/// Encrypts `plaintext` using ChaCha20-Poly1305.
///
/// If `key` is `None` (unencrypted workspace), bytes are returned unchanged.
/// Otherwise the output format is: `[12-byte nonce][32-byte salt][ciphertext+tag]`.
pub fn encrypt_attachment(plaintext: &[u8], key: Option<&[u8; 32]>) -> Result<(Vec<u8>, [u8; 32])> {
    // TODO: implement
    // Returns (ciphertext_or_plaintext, file_salt)
    Err(KrillnotesError::AttachmentEncryption("not implemented".to_string()))
}

/// Decrypts bytes previously encrypted by `encrypt_attachment`.
///
/// If `key` is `None`, bytes are returned unchanged (unencrypted workspace).
pub fn decrypt_attachment(data: &[u8], key: Option<&[u8; 32]>, salt: &[u8]) -> Result<Vec<u8>> {
    // TODO: implement
    Err(KrillnotesError::AttachmentEncryption("not implemented".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_attachment_key_is_deterministic() {
        let k1 = derive_attachment_key("hunter2", "workspace-uuid-abc");
        let k2 = derive_attachment_key("hunter2", "workspace-uuid-abc");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_attachment_key_differs_by_password() {
        let k1 = derive_attachment_key("pass1", "same-uuid");
        let k2 = derive_attachment_key("pass2", "same-uuid");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_derive_attachment_key_differs_by_workspace() {
        let k1 = derive_attachment_key("pass", "uuid-a");
        let k2 = derive_attachment_key("pass", "uuid-b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let key = derive_attachment_key("testpass", "test-uuid");
        let plaintext = b"Hello, attachments!";
        let (ciphertext, salt) = encrypt_attachment(plaintext, Some(&key)).unwrap();
        let recovered = decrypt_attachment(&ciphertext, Some(&key), &salt).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_output_each_time() {
        let key = derive_attachment_key("testpass", "test-uuid");
        let plaintext = b"same content";
        let (ct1, _) = encrypt_attachment(plaintext, Some(&key)).unwrap();
        let (ct2, _) = encrypt_attachment(plaintext, Some(&key)).unwrap();
        // Due to random nonce, ciphertexts must differ
        assert_ne!(ct1, ct2);
    }

    #[test]
    fn test_unencrypted_workspace_passthrough() {
        let plaintext = b"unencrypted content";
        let (stored, _salt) = encrypt_attachment(plaintext, None).unwrap();
        let recovered = decrypt_attachment(&stored, None, &[]).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let key = derive_attachment_key("correct", "uuid");
        let plaintext = b"secret data";
        let (ciphertext, salt) = encrypt_attachment(plaintext, Some(&key)).unwrap();

        let wrong_key = derive_attachment_key("wrong", "uuid");
        let result = decrypt_attachment(&ciphertext, Some(&wrong_key), &salt);
        assert!(result.is_err());
    }
}
```

**Step 2: Add the module to `mod.rs`**

In `krillnotes-core/src/core/mod.rs`, add:

```rust
pub mod attachment;
```

And add re-export:

```rust
#[doc(inline)]
pub use attachment::AttachmentMeta;
```

Also add `AttachmentMeta` to `krillnotes-core/src/lib.rs` re-exports:

```rust
pub use core::{
    // ... existing imports ...
    attachment::AttachmentMeta,
};
```

**Step 3: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core -- attachment:: 2>&1 | tail -15
```

Expected: compile OK but tests FAIL (functions return errors/wrong values)

**Step 4: Implement the functions**

Replace the TODOs with real implementations:

```rust
pub fn derive_attachment_key(password: &str, workspace_id: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(
        Some(workspace_id.as_bytes()),
        password.as_bytes(),
    );
    let mut key = [0u8; 32];
    hk.expand(b"krillnotes-attachment-v1", &mut key)
        .expect("HKDF expand cannot fail for 32-byte output");
    key
}

fn derive_file_key(attachment_key: &[u8; 32], file_salt: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(file_salt), attachment_key);
    let mut key = [0u8; 32];
    hk.expand(b"krillnotes-file-v1", &mut key)
        .expect("HKDF expand cannot fail for 32-byte output");
    key
}

pub fn encrypt_attachment(plaintext: &[u8], key: Option<&[u8; 32]>) -> Result<(Vec<u8>, [u8; 32])> {
    let Some(attachment_key) = key else {
        // Unencrypted workspace — store plaintext, return zero salt
        return Ok((plaintext.to_vec(), [0u8; 32]));
    };

    let mut nonce_bytes = [0u8; 12];
    let mut file_salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    rand::thread_rng().fill_bytes(&mut file_salt);

    let file_key = derive_file_key(attachment_key, &file_salt);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&file_key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| KrillnotesError::AttachmentEncryption(e.to_string()))?;

    // Format: [12-byte nonce][ciphertext+16-byte tag]
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok((output, file_salt))
}

pub fn decrypt_attachment(data: &[u8], key: Option<&[u8; 32]>, salt: &[u8]) -> Result<Vec<u8>> {
    let Some(attachment_key) = key else {
        return Ok(data.to_vec());
    };

    if data.len() < 12 {
        return Err(KrillnotesError::AttachmentEncryption(
            "File too short to contain nonce".to_string(),
        ));
    }

    let nonce_bytes = &data[..12];
    let ciphertext = &data[12..];

    let salt_array: [u8; 32] = salt
        .try_into()
        .map_err(|_| KrillnotesError::AttachmentEncryption("Invalid salt length".to_string()))?;

    let file_key = derive_file_key(attachment_key, &salt_array);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&file_key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| KrillnotesError::AttachmentEncryption(e.to_string()))
}
```

**Step 5: Run tests**

```bash
cargo test -p krillnotes-core -- attachment::
```

Expected: all PASS

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/attachment.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/lib.rs
git commit -m "feat: attachment.rs — ChaCha20-Poly1305 + HKDF crypto primitives"
```

---

## Task 5: Add `workspace_root` and `attachment_key` to `Workspace`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write the failing test**

Add to workspace tests:

```rust
#[test]
fn test_workspace_has_attachment_key_when_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let ws = Workspace::create(&db_path, "hunter2").unwrap();
    assert!(ws.attachment_key().is_some(), "Encrypted workspace must have attachment_key");
}

#[test]
fn test_workspace_has_no_attachment_key_when_unencrypted() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let ws = Workspace::create(&db_path, "").unwrap();
    assert!(ws.attachment_key().is_none(), "Unencrypted workspace must have no attachment_key");
}

#[test]
fn test_workspace_creates_attachments_directory() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    Workspace::create(&db_path, "").unwrap();
    assert!(dir.path().join("attachments").is_dir());
}

#[test]
fn test_workspace_attachment_key_stable_across_open() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let ws1 = Workspace::create(&db_path, "mypass").unwrap();
    let key1 = ws1.attachment_key().unwrap();
    drop(ws1);
    let ws2 = Workspace::open(&db_path, "mypass").unwrap();
    let key2 = ws2.attachment_key().unwrap();
    assert_eq!(key1, key2, "Key must be derived deterministically from password + workspace_id");
}
```

Note: these tests use `tempfile::tempdir()` — add `tempfile` to `[dev-dependencies]` if not already there (it is).

**Step 2: Run to verify they fail**

```bash
cargo test -p krillnotes-core -- test_workspace_has_attachment test_workspace_creates test_workspace_attachment_key_stable 2>&1 | tail -10
```

Expected: compile error (method `attachment_key` not found)

**Step 3: Add fields to `Workspace` struct**

Find the `Workspace` struct definition (~line 42) and add:

```rust
pub struct Workspace {
    storage: Storage,
    script_registry: ScriptRegistry,
    operation_log: Option<OperationLog>,
    device_id: String,
    current_user_id: i64,
    /// Root directory for this workspace (parent of `notes.db`).
    workspace_root: PathBuf,
    /// ChaCha20-Poly1305 attachment key derived from password + workspace_id.
    /// `None` for unencrypted workspaces (empty password).
    attachment_key: Option<[u8; 32]>,
}
```

Add accessor:

```rust
/// Returns the derived attachment encryption key, or `None` for an unencrypted workspace.
pub fn attachment_key(&self) -> Option<&[u8; 32]> {
    self.attachment_key.as_ref()
}

/// Returns the workspace root directory (parent of `notes.db`).
pub fn workspace_root(&self) -> &Path {
    &self.workspace_root
}
```

**Step 4: Update `Workspace::create`**

At the top of `create`, after the existing storage/script_registry lines:

```rust
// Derive workspace root from db path
let workspace_root = path.as_ref()
    .parent()
    .unwrap_or_else(|| std::path::Path::new("."))
    .to_path_buf();
// Create attachments directory (idempotent)
let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

// Generate and store a stable workspace ID (used for attachment key derivation)
let workspace_id = Uuid::new_v4().to_string();
storage.connection().execute(
    "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
    ["workspace_id", &workspace_id],
)?;
```

Also at the beginning, update `Workspace::create` to take `password: &str` (already does), store it temporarily for key derivation, then at the return value:

```rust
// Derive attachment key
let attachment_key = if !password.is_empty() {
    Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
} else {
    None
};
```

Update the final `Ok(...)` to include both new fields:

```rust
Ok(Workspace {
    storage,
    script_registry,
    operation_log,
    device_id,
    current_user_id,
    workspace_root,
    attachment_key,
})
```

**Step 5: Update `Workspace::open`**

After the existing `storage` and `device_id` / `current_user_id` reads, add:

```rust
let workspace_root = path.as_ref()
    .parent()
    .unwrap_or_else(|| std::path::Path::new("."))
    .to_path_buf();
let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

// Read workspace_id for key derivation; generate one if absent (older workspaces)
let workspace_id: String = {
    let existing: std::result::Result<String, _> = storage.connection().query_row(
        "SELECT value FROM workspace_meta WHERE key = 'workspace_id'",
        [],
        |row| row.get(0),
    );
    match existing {
        Ok(id) => id,
        Err(_) => {
            // Older workspace — generate and persist a new workspace_id
            let id = Uuid::new_v4().to_string();
            storage.connection().execute(
                "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
                ["workspace_id", &id],
            )?;
            id
        }
    }
};

let attachment_key = if !password.is_empty() {
    Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
} else {
    None
};
```

Update the return value to include both new fields. Also: the `Workspace::open` function still needs `password: &str` as a parameter (it already does).

**Step 6: Fix existing workspace tests that use `NamedTempFile`**

In the workspace module, the root-note title is derived from `path.file_stem()`. After moving to a folder layout, the DB is always `notes.db` so stem would be "notes". Fix this in `Workspace::create`:

```rust
// Derive root note title from workspace folder name (parent of notes.db), not the db filename
let filename = path.as_ref()
    .parent()
    .and_then(|p| p.file_name())
    .and_then(|s| s.to_str())
    .unwrap_or_else(|| {
        // Fallback: use the db filename stem for backwards compat / tests
        path.as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
    });
```

This keeps existing `NamedTempFile` tests working (they use the temp filename as a title) while making real workspaces use the folder name.

**Step 7: Run all workspace tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -10
```

Expected: all PASS (existing tests unaffected because `NamedTempFile` parent is `/tmp` which is writable)

**Step 8: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: add workspace_root and attachment_key fields to Workspace"
```

---

## Task 6: Workspace attachment methods

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write failing tests**

Add to workspace tests:

```rust
#[test]
fn test_attach_file_stores_metadata_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "testpass").unwrap();
    let notes = ws.list_all_notes().unwrap();
    let root_id = &notes[0].id;

    let data = b"hello attachment";
    let meta = ws.attach_file(root_id, "test.txt", Some("text/plain"), data).unwrap();
    assert_eq!(meta.filename, "test.txt");
    assert_eq!(meta.size_bytes, data.len() as i64);

    let enc_path = dir.path().join("attachments").join(format!("{}.enc", meta.id));
    assert!(enc_path.exists(), "Encrypted file must exist on disk");
}

#[test]
fn test_get_attachment_bytes_decrypts_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "testpass").unwrap();
    let root_id = ws.list_all_notes().unwrap()[0].id.clone();

    let data = b"secret file content";
    let meta = ws.attach_file(&root_id, "doc.txt", None, data).unwrap();
    let recovered = ws.get_attachment_bytes(&meta.id).unwrap();
    assert_eq!(recovered, data);
}

#[test]
fn test_get_attachments_returns_metadata_list() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "").unwrap();
    let root_id = ws.list_all_notes().unwrap()[0].id.clone();

    ws.attach_file(&root_id, "a.pdf", None, b"data a").unwrap();
    ws.attach_file(&root_id, "b.pdf", None, b"data b").unwrap();

    let attachments = ws.get_attachments(&root_id).unwrap();
    assert_eq!(attachments.len(), 2);
    let names: Vec<&str> = attachments.iter().map(|a| a.filename.as_str()).collect();
    assert!(names.contains(&"a.pdf"));
    assert!(names.contains(&"b.pdf"));
}

#[test]
fn test_delete_attachment_removes_file_and_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "testpass").unwrap();
    let root_id = ws.list_all_notes().unwrap()[0].id.clone();

    let meta = ws.attach_file(&root_id, "bye.txt", None, b"temp").unwrap();
    let enc_path = dir.path().join("attachments").join(format!("{}.enc", meta.id));
    assert!(enc_path.exists());

    ws.delete_attachment(&meta.id).unwrap();
    assert!(!enc_path.exists(), "File must be deleted from disk");
    assert!(ws.get_attachments(&root_id).unwrap().is_empty());
}

#[test]
fn test_attach_file_enforces_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "").unwrap();
    let root_id = ws.list_all_notes().unwrap()[0].id.clone();

    ws.set_attachment_max_size_bytes(Some(10)).unwrap();
    let big_data = vec![0u8; 100];
    let result = ws.attach_file(&root_id, "big.bin", None, &big_data);
    assert!(matches!(result, Err(KrillnotesError::AttachmentTooLarge { .. })));
}
```

**Step 2: Run to verify they fail**

```bash
cargo test -p krillnotes-core -- test_attach_file test_get_attachment test_delete_attachment 2>&1 | tail -10
```

Expected: compile error (methods not found)

**Step 3: Implement methods**

Add to `impl Workspace` in `workspace.rs`. First, add necessary imports at the top of the file:

```rust
use crate::core::attachment::{
    decrypt_attachment, encrypt_attachment, AttachmentMeta,
};
use sha2::{Digest, Sha256};
```

Then add these methods:

```rust
/// Attaches a file to a note. Encrypts the bytes and writes them to
/// `<workspace_root>/attachments/<uuid>.enc`, then inserts a DB metadata row.
pub fn attach_file(
    &mut self,
    note_id: &str,
    filename: &str,
    mime_type: Option<&str>,
    data: &[u8],
) -> Result<AttachmentMeta> {
    // Enforce workspace size limit
    if let Some(limit) = self.attachment_max_size_bytes()? {
        if data.len() as u64 > limit {
            return Err(KrillnotesError::AttachmentTooLarge {
                size: data.len() as u64,
                limit,
            });
        }
    }

    // SHA-256 hash for integrity
    let hash = {
        let mut h = Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    };

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    let (encrypted_bytes, file_salt) =
        encrypt_attachment(data, self.attachment_key.as_ref())?;

    // Write to disk
    let enc_path = self.workspace_root.join("attachments").join(format!("{id}.enc"));
    std::fs::write(&enc_path, &encrypted_bytes)?;

    // Insert DB row
    let salt_hex = hex::encode(file_salt);
    self.storage.connection().execute(
        "INSERT INTO attachments (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            id, note_id, filename, mime_type,
            data.len() as i64, hash, file_salt.as_slice(), now
        ],
    )?;

    Ok(AttachmentMeta {
        id,
        note_id: note_id.to_string(),
        filename: filename.to_string(),
        mime_type: mime_type.map(|s| s.to_string()),
        size_bytes: data.len() as i64,
        hash_sha256: hash,
        salt: salt_hex,
        created_at: now,
    })
}

/// Returns all attachment metadata for a note (no file I/O).
pub fn get_attachments(&self, note_id: &str) -> Result<Vec<AttachmentMeta>> {
    let mut stmt = self.storage.connection().prepare(
        "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
         FROM attachments WHERE note_id = ? ORDER BY created_at ASC",
    )?;
    let results = stmt.query_map([note_id], |row| {
        let salt_bytes: Vec<u8> = row.get(6)?;
        Ok(AttachmentMeta {
            id: row.get(0)?,
            note_id: row.get(1)?,
            filename: row.get(2)?,
            mime_type: row.get(3)?,
            size_bytes: row.get(4)?,
            hash_sha256: row.get(5)?,
            salt: hex::encode(&salt_bytes),
            created_at: row.get(7)?,
        })
    })?
    .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(results)
}

/// Returns all attachments in the workspace (used for export).
pub fn list_all_attachments(&self) -> Result<Vec<AttachmentMeta>> {
    let mut stmt = self.storage.connection().prepare(
        "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
         FROM attachments ORDER BY created_at ASC",
    )?;
    let results = stmt.query_map([], |row| {
        let salt_bytes: Vec<u8> = row.get(6)?;
        Ok(AttachmentMeta {
            id: row.get(0)?,
            note_id: row.get(1)?,
            filename: row.get(2)?,
            mime_type: row.get(3)?,
            size_bytes: row.get(4)?,
            hash_sha256: row.get(5)?,
            salt: hex::encode(&salt_bytes),
            created_at: row.get(7)?,
        })
    })?
    .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(results)
}

/// Decrypts and returns the plaintext bytes for an attachment.
pub fn get_attachment_bytes(&self, attachment_id: &str) -> Result<Vec<u8>> {
    let (salt_bytes, _): (Vec<u8>, i64) = self.storage.connection().query_row(
        "SELECT salt, size_bytes FROM attachments WHERE id = ?",
        [attachment_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| KrillnotesError::NoteNotFound(attachment_id.to_string()))?;

    let enc_path = self
        .workspace_root
        .join("attachments")
        .join(format!("{attachment_id}.enc"));
    let encrypted_bytes = std::fs::read(&enc_path)?;
    decrypt_attachment(&encrypted_bytes, self.attachment_key.as_ref(), &salt_bytes)
}

/// Deletes an attachment: removes the `.enc` file and the DB row.
pub fn delete_attachment(&mut self, attachment_id: &str) -> Result<()> {
    let enc_path = self
        .workspace_root
        .join("attachments")
        .join(format!("{attachment_id}.enc"));
    if enc_path.exists() {
        std::fs::remove_file(&enc_path)?;
    }
    self.storage.connection().execute(
        "DELETE FROM attachments WHERE id = ?",
        [attachment_id],
    )?;
    Ok(())
}

/// Returns the workspace-level max attachment size in bytes, or `None` if unlimited.
pub fn attachment_max_size_bytes(&self) -> Result<Option<u64>> {
    let val: std::result::Result<String, _> = self.storage.connection().query_row(
        "SELECT value FROM workspace_meta WHERE key = 'attachment_max_size_bytes'",
        [],
        |row| row.get(0),
    );
    match val {
        Ok(s) => Ok(s.parse::<u64>().ok()),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Sets or clears the workspace-level max attachment size.
pub fn set_attachment_max_size_bytes(&mut self, limit: Option<u64>) -> Result<()> {
    match limit {
        Some(n) => {
            self.storage.connection().execute(
                "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('attachment_max_size_bytes', ?)",
                [n.to_string()],
            )?;
        }
        None => {
            self.storage.connection().execute(
                "DELETE FROM workspace_meta WHERE key = 'attachment_max_size_bytes'",
                [],
            )?;
        }
    }
    Ok(())
}
```

Note: `hex::encode` requires the `hex` crate. Add to `krillnotes-core/Cargo.toml`:

```toml
hex = "0.4"
```

**Step 4: Run tests**

```bash
cargo test -p krillnotes-core -- test_attach_file test_get_attachment test_delete_attachment
```

Expected: all PASS

**Step 5: Run full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all PASS (check the count matches or exceeds previous count)

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs krillnotes-core/Cargo.toml Cargo.lock
git commit -m "feat: attach_file, get_attachments, get_attachment_bytes, delete_attachment on Workspace"
```

---

## Task 7: Auto-migrate workspace folder structure + update `list_workspace_files`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

This task restructures how workspaces are stored on disk. It is **backward-compatible** — migration runs once on startup.

**Step 1: Add auto-migration in `setup` closure**

Find the setup block in `lib.rs` that ends with:

```rust
// Ensure default workspace directory exists on startup
let app_settings = settings::load_settings();
let dir = std::path::Path::new(&app_settings.workspace_directory);
if !dir.exists() {
    std::fs::create_dir_all(dir).ok();
}
```

Directly after that block, add:

```rust
// Auto-migrate flat *.db files to per-workspace folders
for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
    let path = entry.path();
    if path.extension().map(|e| e == "db").unwrap_or(false) {
        let stem = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if stem.is_empty() { continue; }
        let new_folder = dir.join(stem);
        if new_folder.exists() { continue; } // already migrated
        if std::fs::create_dir_all(&new_folder).is_ok() {
            if let Err(e) = std::fs::rename(&path, new_folder.join("notes.db")) {
                eprintln!("[migration] Failed to move {:?}: {e}", path);
                let _ = std::fs::remove_dir(&new_folder); // rollback folder
            } else {
                let _ = std::fs::create_dir_all(new_folder.join("attachments"));
                eprintln!("[migration] Migrated {:?} → {:?}", path, new_folder);
            }
        }
    }
}
```

**Step 2: Update `list_workspace_files` to scan for subdirectories**

Find the `list_workspace_files` function and replace the directory scanning loop:

```rust
// OLD: scans for *.db files
for entry in read_dir.flatten() {
    let path = entry.path();
    if path.extension().map(|e| e == "db").unwrap_or(false) {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let is_open = open_paths.iter().any(|p| *p == path);
            entries.push(WorkspaceEntry { name: stem.to_string(), path: path.display().to_string(), is_open });
        }
    }
}
```

Replace with:

```rust
// NEW: scans for subdirectories containing notes.db
for entry in read_dir.flatten() {
    let folder = entry.path();
    if !folder.is_dir() { continue; }
    let db_file = folder.join("notes.db");
    if !db_file.exists() { continue; }
    if let Some(name) = folder.file_name().and_then(|s| s.to_str()) {
        let is_open = open_paths.iter().any(|p| *p == folder);
        entries.push(WorkspaceEntry {
            name: name.to_string(),
            path: folder.display().to_string(),
            is_open,
        });
    }
}
```

Note: `open_paths` (from `workspace_paths`) now stores **folder paths** not `.db` paths — see Task 8.

**Step 3: Update `create_workspace` command**

The command receives `path: String` which is now the workspace **folder path** (e.g., `/Users/foo/Krillnotes/my-notes`).

Find the `create_workspace` Tauri command. Change:

```rust
// OLD
let path_buf = PathBuf::from(&path);
if path_buf.exists() {
    return Err("File already exists. Use Open Workspace instead.".to_string());
}
// ...
let workspace = Workspace::create(&path_buf, &password)
    .map_err(|e| format!("Failed to create: {e}"))?;
// Cache password if setting is enabled
if settings.cache_workspace_passwords {
    state.workspace_passwords.lock().expect("Mutex poisoned")
        .insert(path_buf.clone(), password);
}
// ...
store_workspace(&state, label.clone(), workspace, path_buf.clone());
```

To:

```rust
// NEW: path is the workspace folder; DB lives at <folder>/notes.db
let folder = PathBuf::from(&path);
if folder.exists() {
    return Err("Workspace already exists. Use Open Workspace instead.".to_string());
}
std::fs::create_dir_all(&folder)
    .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
let db_path = folder.join("notes.db");
let workspace = Workspace::create(&db_path, &password)
    .map_err(|e| format!("Failed to create: {e}"))?;

let settings = settings::load_settings();
if settings.cache_workspace_passwords {
    state.workspace_passwords.lock().expect("Mutex poisoned")
        .insert(folder.clone(), password);
}
// ...
store_workspace(&state, label.clone(), workspace, folder.clone());
```

Also update the label generation (still uses path to derive a unique name) — `generate_unique_label` uses the file stem from the path, which for a folder is the folder name itself. No change needed there.

**Step 4: Update `open_workspace` command**

Same pattern: `path` is now the workspace folder.

```rust
// NEW
let folder = PathBuf::from(&path);
if !folder.is_dir() {
    return Err("Workspace folder does not exist".to_string());
}
let db_path = folder.join("notes.db");
let workspace = Workspace::open(&db_path, &password)
    .map_err(|e| match e {
        KrillnotesError::WrongPassword => "WRONG_PASSWORD".to_string(),
        KrillnotesError::UnencryptedWorkspace => "UNENCRYPTED_WORKSPACE".to_string(),
        other => format!("Failed to open: {other}"),
    })?;

let settings = settings::load_settings();
if settings.cache_workspace_passwords {
    state.workspace_passwords.lock().expect("Mutex poisoned")
        .insert(folder.clone(), password);
}
// ...
store_workspace(&state, label.clone(), workspace, folder.clone());
```

**Step 5: Update `execute_import` command**

`db_path` parameter now represents the workspace folder. Rename it `folder_path` in the command for clarity, and construct `notes.db` inside it:

```rust
async fn execute_import(
    // ...
    folder_path: String,  // was db_path
    // ...
) {
    let folder = PathBuf::from(&folder_path);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
    let db_path_buf = folder.join("notes.db");
    let result = import_workspace(reader, &db_path_buf, password.as_deref(), &workspace_password)
        ...
    // After import: open workspace using folder path
    store_workspace(&state, label.clone(), workspace, folder.clone());
```

Note: the Tauri parameter `db_path` must be renamed to `folder_path` in the function signature AND in `generate_handler!`. The frontend sends `folderPath` (camelCase) — update `App.tsx` in Task 9.

**Step 6: Build to verify it compiles**

```bash
cargo build -p krillnotes-desktop 2>&1 | grep -E "error|warning" | head -20
```

Fix any compile errors. Run the app manually to verify:
- Existing flat `.db` workspaces auto-migrate on first launch (check console output)
- `list_workspace_files` returns the migrated workspaces
- Create and Open workspace dialogs still work

**Step 7: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: auto-migrate flat .db workspaces to folder layout, update list/create/open/import commands"
```

---

## Task 8: Update frontend path conventions

**Files:**
- Modify: `krillnotes-desktop/src/components/NewWorkspaceDialog.tsx`
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Update `NewWorkspaceDialog.tsx`**

Line 61: change path construction from `.db` to folder path:

```tsx
// OLD
const path = `${workspaceDir}/${slug}.db`;

// NEW
const path = `${workspaceDir}/${slug}`;
```

Also update the "savedTo" hint at line 108:

```tsx
// OLD
{t('workspace.savedTo', { path: `${workspaceDir}/${slugify(name.trim()) || '...'}.db` })}

// NEW
{t('workspace.savedTo', { path: `${workspaceDir}/${slugify(name.trim()) || '...'}` })}
```

**Step 2: Update `App.tsx`**

Line 171: change `dbPath` to `folderPath`:

```tsx
// OLD
const dbPath = `${settings.workspaceDirectory}/${slug}.db`;
// ...
dbPath,

// NEW
const folderPath = `${settings.workspaceDirectory}/${slug}`;
// ...
folderPath,
```

Update the invoke call to use `folderPath`:

```tsx
await invoke<WorkspaceInfoType>('execute_import', {
  zipPath: pendingImportArgs.zipPath,
  folderPath: pendingImportArgs.folderPath,  // was dbPath
  password: pendingImportArgs.zipPassword ?? null,
  workspacePassword: wsPassword,
});
```

Also update the `pendingImportArgs` type:

```tsx
const [pendingImportArgs, setPendingImportArgs] = useState<{
  zipPath: string;
  folderPath: string;  // was dbPath
  zipPassword?: string;
} | null>(null);
```

**Step 3: Build frontend**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

Expected: no TypeScript errors

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/NewWorkspaceDialog.tsx krillnotes-desktop/src/App.tsx
git commit -m "feat: update frontend path conventions to workspace folder (not .db file)"
```

---

## Task 9: Tauri commands for attachments

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add attachment commands**

Add these Tauri commands to `lib.rs`:

```rust
/// Attaches a file to a note. Reads the file from disk, encrypts it, and stores it.
#[tauri::command]
fn attach_file(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    file_path: String,
) -> std::result::Result<AttachmentMeta, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    let path = std::path::Path::new(&file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid file path")?
        .to_string();

    let mime_type = mime_guess::from_path(path)
        .first()
        .map(|m| m.to_string());

    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    workspace
        .attach_file(&note_id, &filename, mime_type.as_deref(), &data)
        .map_err(|e| e.to_string())
}

/// Returns attachment metadata for all attachments on a note.
#[tauri::command]
fn get_attachments(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Vec<AttachmentMeta>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.get_attachments(&note_id).map_err(|e| e.to_string())
}

/// Returns the decrypted base64-encoded bytes of an attachment (for display in UI).
#[tauri::command]
fn get_attachment_data(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<String, String> {
    use base64::Engine;
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    let bytes = workspace
        .get_attachment_bytes(&attachment_id)
        .map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// Deletes an attachment from a note.
#[tauri::command]
fn delete_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;
    workspace
        .delete_attachment(&attachment_id)
        .map_err(|e| e.to_string())
}

/// Decrypts an attachment to a temp file and opens it with the default system application.
#[tauri::command]
async fn open_attachment(
    window: tauri::Window,
    state: State<'_, AppState>,
    attachment_id: String,
    filename: String,
) -> std::result::Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let bytes = {
        let label = window.label();
        let workspaces = state.workspaces.lock().expect("Mutex poisoned");
        let workspace = workspaces.get(label).ok_or("No workspace open")?;
        workspace
            .get_attachment_bytes(&attachment_id)
            .map_err(|e| e.to_string())?
    };

    let tmp_dir = std::env::temp_dir().join("krillnotes-attachments");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let tmp_path = tmp_dir.join(&filename);
    std::fs::write(&tmp_path, &bytes).map_err(|e| e.to_string())?;

    window
        .app_handle()
        .opener()
        .open_path(tmp_path.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|e| e.to_string())
}
```

Note: `mime_guess` and `base64` crates are needed. Add to `krillnotes-desktop/src-tauri/Cargo.toml`:

```toml
mime_guess = "2"
base64 = "0.22"
```

Also add `AttachmentMeta` to the imports at the top of `lib.rs`:

```rust
use krillnotes_core::AttachmentMeta;
```

**Step 2: Register commands**

In the `invoke_handler!` macro, add:

```rust
attach_file,
get_attachments,
get_attachment_data,
delete_attachment,
open_attachment,
```

**Step 3: Build to verify**

```bash
cargo build -p krillnotes-desktop 2>&1 | grep "^error" | head -10
```

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/Cargo.toml Cargo.lock
git commit -m "feat: Tauri commands for file attachments (attach, list, read, delete, open)"
```

---

## Task 10: Update export to include attachments

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`

**Step 1: Write failing test**

Add to export tests:

```rust
#[test]
fn test_export_includes_attachments() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "").unwrap();
    let root_id = ws.list_all_notes().unwrap()[0].id.clone();

    ws.attach_file(&root_id, "hello.txt", Some("text/plain"), b"hello world").unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

    let mut archive = zip::ZipArchive::new(Cursor::new(&buf)).unwrap();
    // Must have attachments.json
    assert!(archive.by_name("attachments.json").is_ok(), "Must have attachments.json");
    // Must have the attachment file
    let found = (0..archive.len()).any(|i| {
        archive.by_index(i).ok()
            .map(|f| f.name().ends_with("hello.txt"))
            .unwrap_or(false)
    });
    assert!(found, "Attachment file must be in the zip");
}

#[test]
fn test_import_restores_attachments() {
    let dir_src = tempfile::tempdir().unwrap();
    let db_src = dir_src.path().join("notes.db");
    let mut ws = Workspace::create(&db_src, "pass").unwrap();
    let root_id = ws.list_all_notes().unwrap()[0].id.clone();

    ws.attach_file(&root_id, "data.txt", None, b"attachment content").unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

    let dir_dst = tempfile::tempdir().unwrap();
    let db_dst = dir_dst.path().join("notes.db");
    import_workspace(Cursor::new(&buf), &db_dst, None, "newpass").unwrap();

    let ws2 = Workspace::open(&db_dst, "newpass").unwrap();
    let notes = ws2.list_all_notes().unwrap();
    let root = notes.iter().find(|n| n.parent_id.is_none()).unwrap();
    let attachments = ws2.get_attachments(&root.id).unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename, "data.txt");

    let recovered = ws2.get_attachment_bytes(&attachments[0].id).unwrap();
    assert_eq!(recovered, b"attachment content");
}
```

**Step 2: Run to verify failure**

```bash
cargo test -p krillnotes-core -- test_export_includes_attachments test_import_restores 2>&1 | tail -10
```

Expected: FAIL (no attachments.json in zip)

**Step 3: Update `export_workspace` to include attachments**

Add `AttachmentMeta` import at top of `export.rs`:

```rust
use crate::core::attachment::{decrypt_attachment, AttachmentMeta};
```

After the `zip.finish()?;` line and before `Ok(())`, add the attachment export block. But actually, attachments need to be added BEFORE `zip.finish()`. Insert before the finish call:

```rust
// Write attachments
let all_attachments = workspace
    .list_all_attachments()
    .map_err(|e| ExportError::Database(e.to_string()))?;

if !all_attachments.is_empty() {
    // Write attachments.json manifest
    zip.start_file("attachments.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &all_attachments)?;

    // Write each attachment file (plaintext — zip AES password protects them)
    for meta in &all_attachments {
        let plaintext = workspace
            .get_attachment_bytes(&meta.id)
            .map_err(|e| ExportError::Database(e.to_string()))?;
        // Store as attachments/<uuid>/<original_filename> to avoid name collisions
        zip.start_file(
            format!("attachments/{}/{}", meta.id, meta.filename),
            options,
        )?;
        zip.write_all(&plaintext)?;
    }
}
```

**Step 4: Update `import_workspace` to restore attachments**

Add `AttachmentMeta` import. After the `workspace.rebuild_note_links_index()` call (which already opens the workspace at line ~471), add:

```rust
// Restore attachments if the archive contains them
if let Some(attachments_cursor) = try_read_entry(&mut archive, "attachments.json", zip_password) {
    let attachment_metas: Vec<AttachmentMeta> =
        serde_json::from_reader(attachments_cursor).unwrap_or_default();

    for meta in attachment_metas {
        let zip_path = format!("attachments/{}/{}", meta.id, meta.filename);
        if let Some(mut file_cursor) = try_read_entry(&mut archive, &zip_path, zip_password) {
            let mut plaintext = Vec::new();
            file_cursor.read_to_end(&mut plaintext).ok();
            // Re-encrypt and store using the new workspace's attachment key
            let _ = workspace.attach_file_with_id(
                &meta.id,
                &meta.note_id,
                &meta.filename,
                meta.mime_type.as_deref(),
                &plaintext,
            );
        }
    }
}
```

This requires a new method `attach_file_with_id` on `Workspace` that takes an explicit ID (for import, to preserve IDs from the source workspace). Add to `workspace.rs`:

```rust
/// Import-only: attach a file with a pre-specified ID (preserves IDs from export).
/// Does NOT enforce size limits (the size was already validated at export time).
pub fn attach_file_with_id(
    &mut self,
    id: &str,
    note_id: &str,
    filename: &str,
    mime_type: Option<&str>,
    data: &[u8],
) -> Result<()> {
    use sha2::{Digest, Sha256};
    let hash = {
        let mut h = Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    };
    let now = chrono::Utc::now().timestamp();
    let (encrypted_bytes, file_salt) = encrypt_attachment(data, self.attachment_key.as_ref())?;
    let enc_path = self.workspace_root.join("attachments").join(format!("{id}.enc"));
    std::fs::write(&enc_path, &encrypted_bytes)?;
    let _ = self.storage.connection().execute(
        "INSERT OR IGNORE INTO attachments (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![id, note_id, filename, mime_type, data.len() as i64, hash, file_salt.as_slice(), now],
    );
    Ok(())
}
```

**Step 5: Run tests**

```bash
cargo test -p krillnotes-core -- test_export_includes_attachments test_import_restores
```

Expected: both PASS

**Step 6: Run full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/export.rs krillnotes-core/src/core/workspace.rs
git commit -m "feat: include attachments in export/import zip archive"
```

---

## Task 11: Frontend — add `AttachmentMeta` type

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Add type**

Append to `types.ts`:

```typescript
export interface AttachmentMeta {
  id: string;
  noteId: string;
  filename: string;
  mimeType: string | null;
  sizeBytes: number;
  hashSha256: string;
  salt: string;
  createdAt: number;
}
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add AttachmentMeta type"
```

---

## Task 12: `AttachmentsSection.tsx` component

**Files:**
- Create: `krillnotes-desktop/src/components/AttachmentsSection.tsx`

**Step 1: Create the component**

```tsx
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openFilePicker } from '@tauri-apps/plugin-dialog';
import { confirm } from '@tauri-apps/plugin-dialog';
import { Paperclip, Trash2, FileText, Image } from 'lucide-react';
import type { AttachmentMeta } from '../types';

interface AttachmentsSectionProps {
  noteId: string | null;
  windowLabel: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function isImageMime(mime: string | null): boolean {
  return mime?.startsWith('image/') ?? false;
}

export default function AttachmentsSection({ noteId, windowLabel }: AttachmentsSectionProps) {
  const [attachments, setAttachments] = useState<AttachmentMeta[]>([]);
  const [thumbnails, setThumbnails] = useState<Record<string, string>>({});
  const [error, setError] = useState('');
  const [dragging, setDragging] = useState(false);

  const loadAttachments = async () => {
    if (!noteId) { setAttachments([]); return; }
    try {
      const list = await invoke<AttachmentMeta[]>('get_attachments', { noteId });
      setAttachments(list);
      // Load thumbnails for images
      for (const att of list) {
        if (isImageMime(att.mimeType) && !thumbnails[att.id]) {
          invoke<string>('get_attachment_data', { attachmentId: att.id })
            .then(b64 => {
              setThumbnails(prev => ({ ...prev, [att.id]: `data:${att.mimeType};base64,${b64}` }));
            })
            .catch(() => {});
        }
      }
    } catch (e) {
      setError(`${e}`);
    }
  };

  useEffect(() => { loadAttachments(); }, [noteId]);

  const handleAdd = async () => {
    if (!noteId) return;
    setError('');
    try {
      const selected = await openFilePicker({ multiple: true });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      for (const filePath of paths) {
        await invoke('attach_file', { noteId, filePath });
      }
      await loadAttachments();
    } catch (e) {
      setError(`Failed to attach: ${e}`);
    }
  };

  const handleOpen = async (att: AttachmentMeta) => {
    try {
      await invoke('open_attachment', { attachmentId: att.id, filename: att.filename });
    } catch (e) {
      setError(`Failed to open: ${e}`);
    }
  };

  const handleDelete = async (att: AttachmentMeta) => {
    const ok = await confirm(`Delete attachment "${att.filename}"?`, { title: 'Delete Attachment' });
    if (!ok) return;
    try {
      await invoke('delete_attachment', { attachmentId: att.id });
      setAttachments(prev => prev.filter(a => a.id !== att.id));
      setThumbnails(prev => { const copy = { ...prev }; delete copy[att.id]; return copy; });
    } catch (e) {
      setError(`Failed to delete: ${e}`);
    }
  };

  const handleDragOver = (e: React.DragEvent) => { e.preventDefault(); setDragging(true); };
  const handleDragLeave = () => setDragging(false);
  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    if (!noteId) return;
    const files = Array.from(e.dataTransfer.files);
    for (const file of files) {
      // Tauri drag-drop gives us the path via the file object's path property
      const filePath = (file as any).path;
      if (filePath) {
        try {
          await invoke('attach_file', { noteId, filePath });
        } catch (err) {
          setError(`Failed to attach ${file.name}: ${err}`);
        }
      }
    }
    await loadAttachments();
  };

  if (!noteId) return null;

  return (
    <div
      className={`border-t border-border pt-3 mt-3 ${dragging ? 'ring-2 ring-primary rounded' : ''}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wide flex items-center gap-1">
          <Paperclip size={12} /> Attachments {attachments.length > 0 && `(${attachments.length})`}
        </span>
        <button
          onClick={handleAdd}
          className="text-xs text-primary hover:text-primary/80 px-2 py-1 rounded hover:bg-secondary"
        >
          + Add
        </button>
      </div>

      {error && (
        <p className="text-xs text-red-500 mb-2">{error}</p>
      )}

      {attachments.length === 0 ? (
        <p className="text-xs text-muted-foreground italic">
          {dragging ? 'Drop files here' : 'No attachments — drop files or click Add'}
        </p>
      ) : (
        <div className="space-y-1">
          {attachments.map(att => (
            <div
              key={att.id}
              className="flex items-center gap-2 group rounded p-1 hover:bg-secondary/50"
            >
              {isImageMime(att.mimeType) && thumbnails[att.id] ? (
                <img
                  src={thumbnails[att.id]}
                  alt={att.filename}
                  className="w-10 h-10 object-cover rounded flex-shrink-0 cursor-pointer"
                  onClick={() => handleOpen(att)}
                />
              ) : (
                <div
                  className="w-10 h-10 rounded flex-shrink-0 bg-secondary flex items-center justify-center cursor-pointer"
                  onClick={() => handleOpen(att)}
                >
                  {isImageMime(att.mimeType)
                    ? <Image size={18} className="text-muted-foreground" />
                    : <FileText size={18} className="text-muted-foreground" />
                  }
                </div>
              )}
              <div className="flex-1 min-w-0 cursor-pointer" onClick={() => handleOpen(att)}>
                <p className="text-xs font-medium truncate">{att.filename}</p>
                <p className="text-xs text-muted-foreground">{formatBytes(att.sizeBytes)}</p>
              </div>
              <button
                onClick={() => handleDelete(att)}
                className="opacity-0 group-hover:opacity-100 p-1 rounded hover:bg-red-500/20 hover:text-red-500 flex-shrink-0"
                title="Delete attachment"
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
```

**Step 2: Build to check TypeScript**

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep -E "error TS" | head -10
```

Fix any type errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/AttachmentsSection.tsx
git commit -m "feat: AttachmentsSection component for InfoPanel"
```

---

## Task 13: Integrate `AttachmentsSection` into `InfoPanel.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Add import**

Near the top of `InfoPanel.tsx`, add:

```tsx
import AttachmentsSection from './AttachmentsSection';
```

**Step 2: Render in view mode**

Locate the section in the InfoPanel JSX that renders tags (in view mode). After the tags block, add the `AttachmentsSection`:

```tsx
{/* Attachments section — always visible when a note is selected */}
<AttachmentsSection
  noteId={selectedNote?.id ?? null}
  windowLabel={window.location.hostname}  // not used for routing, just context
/>
```

Note: the `windowLabel` is used by Tauri commands implicitly via the window handle on the backend; the component doesn't need to pass it explicitly since `invoke` calls are automatically routed to the calling window. Remove that prop from the component interface if it's unused.

**Step 3: Simplify the component interface**

Update `AttachmentsSection` to not require `windowLabel` — Tauri routes commands to the correct window automatically. Remove `windowLabel` from the props interface and all usages.

**Step 4: Build frontend**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

Expected: no errors

**Step 5: Run all Rust tests one final time**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all PASS

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx krillnotes-desktop/src/components/AttachmentsSection.tsx
git commit -m "feat: integrate AttachmentsSection into InfoPanel"
```

---

## Final verification

Run the full build and test pass before considering this feature complete:

```bash
# Rust tests
cargo test 2>&1 | tail -5

# TypeScript build
cd krillnotes-desktop && npm run build

# Manual smoke test:
# 1. Launch app — verify existing workspaces auto-migrated (console: [migration] ...)
# 2. Create a new workspace — verify folder structure: <name>/notes.db + <name>/attachments/
# 3. Open workspace, select a note, attach an image and a text file
# 4. Verify thumbnail for image, file icon for text
# 5. Click attachment → opens in system app
# 6. Export workspace as .krillnotes archive
# 7. Import archive into a new workspace → verify attachments present and openable
# 8. Delete an attachment → verify removed from UI and disk
```
