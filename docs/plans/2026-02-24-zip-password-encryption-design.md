# Design: ZIP Password Encryption for Export/Import

**Date:** 2026-02-24
**Status:** Approved

## Summary

Add optional AES-256 password encryption to workspace exports. On import, detect encrypted archives automatically and prompt the user for the password before proceeding.

## Approach

Use the `zip` crate's built-in `aes-crypto` feature (WinZip AES-256 standard). No new dependencies — just enabling a Cargo feature flag. Encrypted zips can also be opened by third-party tools (7-Zip, WinZip, etc.).

## Architecture

### Rust Core (`krillnotes-core`)

**Cargo.toml:** Add `aes-crypto` to the `zip` feature list alongside the existing `deflate`.

**`ExportError` enum:** Two new variants:
- `EncryptedArchive` — archive requires a password (none was provided)
- `InvalidPassword` — a password was provided but is incorrect

**Core function signatures** (all gain `password: Option<&str>`):
```rust
pub fn export_workspace<W: Write + Seek>(workspace: &Workspace, writer: W, password: Option<&str>) -> Result<(), ExportError>
pub fn peek_import<R: Read + Seek>(reader: R, password: Option<&str>) -> Result<ImportResult, ExportError>
pub fn import_workspace<R: Read + Seek>(reader: R, db_path: &Path, password: Option<&str>) -> Result<ImportResult, ExportError>
```

**Export with password:** Each zip entry is written with `SimpleFileOptions::default().with_aes_encryption(AesMode::Aes256, password)`.

**Import detection:** Before reading `notes.json`, call `file.encrypted()` on the `ZipFile`. If `true` and no password was supplied → `Err(ExportError::EncryptedArchive)`. If a password is supplied and decryption fails → `Err(ExportError::InvalidPassword)`.

### Tauri Commands (`src-tauri/src/lib.rs`)

All three export/import commands gain `password: Option<String>`:
- `export_workspace_cmd(path, password)`
- `peek_import_cmd(zip_path, password)`
- `execute_import(zip_path, db_path, password)`

Sentinel error strings returned to frontend:
- `"ENCRYPTED_ARCHIVE"` — frontend shows password dialog
- `"INVALID_PASSWORD"` — frontend shows inline error in password dialog

### Frontend (`App.tsx`)

**Export flow:**
1. Menu: "File > Export Workspace clicked"
2. Show **export password dialog** (inline, same pattern as workspace-name dialog):
   - Title: "Protect with a password?"
   - Password field + Confirm field
   - "Encrypt" button (disabled until both fields are non-empty and match)
   - "Skip — no encryption" button/link
3. Native save dialog opens
4. `export_workspace_cmd(path, password | null)` is called

**Import flow:**
1. Menu: "File > Import Workspace clicked"
2. Native open dialog → select zip
3. `peek_import_cmd(zipPath, null)` called
4. If error === `"ENCRYPTED_ARCHIVE"`:
   - Show **import password dialog**:
     - Title: "This archive is password-protected"
     - Single password field
     - "Decrypt" button, "Cancel" button
   - On submit: `peek_import_cmd(zipPath, password)`
   - If error === `"INVALID_PASSWORD"`: show inline error "Incorrect password — try again" (dialog stays open)
   - On success: continue with the password in hand
5. Normal workspace-name dialog → `execute_import(zipPath, dbPath, password | null)`

## Components

No new files. Both password dialogs are inline state in `App.tsx`, controlled by:
- `showExportPasswordDialog: boolean`
- `showImportPasswordDialog: boolean`
- `pendingExportPassword: string | null`
- `importPassword: string | null`

Following the same conditional-render pattern as the existing workspace-name dialog.

## Error Handling

| Scenario | Behaviour |
|---|---|
| Wrong password on import | Inline error in password dialog; dialog stays open for retry |
| Corrupt/invalid zip | Existing error path (status message), unchanged |
| IO error on export | Existing error path (status message), unchanged |
| Password mismatch on export | "Encrypt" button stays disabled; no submit possible |
| User cancels password dialog | Import/export aborted (no file written, no window opened) |

## Testing

Extend the existing round-trip test in `export.rs` with:
1. Export with password → `peek_import` with correct password → succeeds
2. Export with password → `peek_import` with no password → `EncryptedArchive`
3. Export with password → `peek_import` with wrong password → `InvalidPassword`
4. Export without password → `peek_import` with no password → succeeds (existing behaviour unchanged)
