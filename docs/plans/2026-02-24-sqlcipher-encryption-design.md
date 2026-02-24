# SQLCipher Database Encryption — Design

## Summary

Add transparent AES-256 encryption to all Krillnotes workspace files (`.krillnotes`) using SQLCipher. Users set a password when creating a workspace and enter it when opening one. Old unencrypted workspaces are rejected with a clear migration message. Session password caching is configurable in settings.

## Approach

Use `rusqlite` with the `bundled-sqlcipher-vendored-openssl` feature flag, replacing the current `bundled` flag. This statically bundles both SQLCipher and OpenSSL into the binary — no system dependencies on macOS, Windows, or Linux.

## Architecture

### Crypto Backend

- **Feature flag:** `rusqlite = { version = "0.38", features = ["bundled-sqlcipher-vendored-openssl"] }`
- **Encryption:** AES-256-CBC (SQLCipher v4 default)
- **Key derivation:** PBKDF2-HMAC-SHA512, 256,000 iterations (SQLCipher v4 default)
- `PRAGMA key = 'password'` is set as the very first SQL operation after opening any connection, before any schema access

### Storage Layer Changes (`krillnotes-core/src/core/storage.rs`)

- `Storage::create(path, password: &str)` — creates an encrypted database
- `Storage::open(path, password: &str)` — opens an encrypted database; detects unencrypted files

**Old workspace detection algorithm:**
1. Open connection, set `PRAGMA key` with the provided password
2. Run `SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('notes', 'operations', 'workspace_meta')`
3. If count ≠ 3: open a second connection to the same file *without* setting any `PRAGMA key`
4. If that plain-SQLite connection sees the 3 expected tables → return `KrillnotesError::UnencryptedWorkspace`
5. Otherwise → return `KrillnotesError::WrongPassword`

### Workspace Layer Changes (`krillnotes-core/src/core/workspace.rs`)

- `Workspace::create(path, password: &str)` — passes password to `Storage::create`
- `Workspace::open(path, password: &str)` — passes password to `Storage::open`

### Tauri Command Changes (`krillnotes-desktop/src-tauri/src/lib.rs`)

- `create_workspace(path, name, password)` — new `password` parameter
- `open_workspace(path, password)` — new `password` parameter
- `import_workspace_cmd(...)` — new `workspace_password` parameter for the new encrypted workspace DB
- Error mapping: `UnencryptedWorkspace` → sentinel string `"UNENCRYPTED_WORKSPACE"` for frontend handling, `WrongPassword` → `"WRONG_PASSWORD"`

### AppState Changes

Add `workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>` for in-memory session caching.

### Settings Changes

Add `cache_workspace_passwords: bool` (default: `false`) to `AppSettings`.

## UI Changes

### New Dialogs

1. **SetPasswordDialog** — used on workspace create and import; two fields (password + confirm); confirm must match before proceeding
2. **EnterPasswordDialog** — used on workspace open; single password field with inline error on wrong password

### Updated Flows

| Scenario | Flow |
|---|---|
| Create workspace | name dialog → **SetPasswordDialog** → workspace opens |
| Open workspace | workspace picker → **EnterPasswordDialog** → workspace opens |
| Import workspace | zip-password dialog (existing) → **SetPasswordDialog** → workspace opens |
| Old unencrypted workspace | error dialog: *"This workspace was created with an older version of Krillnotes. Please open it in the previous version, export it via File → Export Workspace, then import it here."* |
| Wrong password | inline error on EnterPasswordDialog, stays open |

### Settings

New toggle in the Settings dialog: **"Remember workspace passwords for this session"** (default: off).
When enabled, the password is stored in `AppState.workspace_passwords` (keyed by workspace path) after a successful open. On subsequent opens of the same workspace within the same session, the cached password is used without prompting. Cache is cleared when the app quits.

## What Does Not Change

- ZIP export/import encryption (separate feature, uses the `zip` crate's own AES — unchanged)
- Database schema, tables, and all SQL queries
- All rusqlite API usage (SQLCipher is API-identical to SQLite)
- All Tauri commands except `create_workspace`, `open_workspace`, and `import_workspace_cmd`

## Migration Path for Old Workspaces

Old workspaces cannot be opened in the new version. Users must:
1. Open the workspace in an older version of Krillnotes
2. Export via **File → Export Workspace**
3. Import the exported `.zip` file in this version via **File → Import Workspace**
