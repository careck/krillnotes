# SQLCipher Database Encryption Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Encrypt all Krillnotes workspace files at rest using SQLCipher AES-256, requiring a user password on create and open, with configurable session caching.

**Architecture:** Swap `rusqlite`'s `bundled` feature for `bundled-sqlcipher-vendored-openssl` (self-contained across all platforms). Add a `password: &str` parameter to `Storage::create` and `Storage::open`; set `PRAGMA key` as the first SQL operation. Propagate the password through `Workspace`, Tauri commands, and export/import. Add two new frontend dialogs (`SetPasswordDialog`, `EnterPasswordDialog`) and wire them into existing workspace flows.

**Tech Stack:** Rust/rusqlite/SQLCipher, Tauri v2, React/TypeScript, Tailwind CSS

---

## Task 0: Create the feature worktree

**Files:**
- No files yet — sets up the isolated branch

**Step 1: Create the worktree and branch**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/sqlcipher-encryption -b feat/sqlcipher-encryption
```

**Step 2: Verify**

```bash
git -C /Users/careck/Source/Krillnotes worktree list
```
Expected: new entry `.worktrees/feat/sqlcipher-encryption  <sha>  [feat/sqlcipher-encryption]`

All subsequent work happens inside `/Users/careck/Source/Krillnotes/.worktrees/feat/sqlcipher-encryption/`.

---

## Task 1: Swap the rusqlite feature flag

**Files:**
- Modify: `Cargo.toml:6`

**Step 1: Change the feature flag**

In `Cargo.toml` line 6, replace:
```toml
rusqlite = { version = "0.38", features = ["bundled"] }
```
with:
```toml
rusqlite = { version = "0.38", features = ["bundled-sqlcipher-vendored-openssl"] }
```

**Step 2: Verify it compiles (tests will fail — that's expected)**

```bash
cargo build -p krillnotes-core 2>&1 | tail -20
```
Expected: build succeeds (may be slow — it compiles OpenSSL from source). Linker errors or `PRAGMA key` errors come later.

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: swap rusqlite bundled for bundled-sqlcipher-vendored-openssl"
```

---

## Task 2: Add new error variants

**Files:**
- Modify: `krillnotes-core/src/core/error.rs`

**Step 1: Write the failing test**

Add to `krillnotes-core/src/core/error.rs` in the `#[cfg(test)]` block (or create one):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrong_password_variant_exists() {
        let e = KrillnotesError::WrongPassword;
        assert!(e.to_string().contains("password"));
    }

    #[test]
    fn test_unencrypted_workspace_variant_exists() {
        let e = KrillnotesError::UnencryptedWorkspace;
        assert!(e.to_string().contains("encrypted") || e.to_string().contains("old version"));
    }
}
```

**Step 2: Run to confirm failure**

```bash
cargo test -p krillnotes-core error 2>&1 | tail -20
```
Expected: compile error — `WrongPassword` and `UnencryptedWorkspace` don't exist yet.

**Step 3: Add the variants**

In `krillnotes-core/src/core/error.rs`, add to the `KrillnotesError` enum:
```rust
    /// The supplied password is wrong for this workspace.
    #[error("Wrong password for this workspace")]
    WrongPassword,

    /// The file is a valid but unencrypted (pre-encryption) workspace.
    #[error("This workspace was created with an older version of Krillnotes and cannot be opened here")]
    UnencryptedWorkspace,
```

Also add human-readable messages to `user_message()`:
```rust
            Self::WrongPassword => "Wrong password — please try again".to_string(),
            Self::UnencryptedWorkspace => "This workspace was created with an older version of Krillnotes. Please open it in the previous version, export it via File → Export Workspace, then import it here.".to_string(),
```

**Step 4: Run tests to verify pass**

```bash
cargo test -p krillnotes-core error 2>&1 | tail -20
```
Expected: 2 tests pass.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/error.rs
git commit -m "feat: add WrongPassword and UnencryptedWorkspace error variants"
```

---

## Task 3: Update Storage — create with password

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs`

**Step 1: Write the failing test**

Replace `test_create_storage` in `storage.rs` tests with:
```rust
#[test]
fn test_create_encrypted_storage() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Storage::create(temp.path(), "hunter2").unwrap();

    let tables: Vec<String> = storage
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();

    assert!(tables.contains(&"notes".to_string()));
    assert!(tables.contains(&"operations".to_string()));
    assert!(tables.contains(&"workspace_meta".to_string()));
}
```

**Step 2: Run to confirm failure**

```bash
cargo test -p krillnotes-core test_create_encrypted_storage 2>&1 | tail -20
```
Expected: compile error — `Storage::create` takes 1 argument, not 2.

**Step 3: Update `Storage::create`**

Replace the `create` function in `storage.rs:25-29`:
```rust
pub fn create<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
    let conn = Connection::open(path)?;
    let escaped = password.replace('\'', "''");
    conn.execute_batch(&format!("PRAGMA key = '{escaped}';\n"))?;
    conn.execute_batch(include_str!("schema.sql"))?;
    Ok(Self { conn })
}
```

**Step 4: Run test to verify pass**

```bash
cargo test -p krillnotes-core test_create_encrypted_storage 2>&1 | tail -20
```
Expected: PASS.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/storage.rs
git commit -m "feat: add password parameter to Storage::create"
```

---

## Task 4: Update Storage — open with password + unencrypted detection

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs`

**Step 1: Write failing tests**

Add to `storage.rs` tests:
```rust
#[test]
fn test_open_encrypted_storage_correct_password() {
    let temp = NamedTempFile::new().unwrap();
    Storage::create(temp.path(), "correct").unwrap();
    let storage = Storage::open(temp.path(), "correct").unwrap();
    let count: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('notes','operations','workspace_meta')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn test_open_encrypted_storage_wrong_password() {
    let temp = NamedTempFile::new().unwrap();
    Storage::create(temp.path(), "correct").unwrap();
    let result = Storage::open(temp.path(), "wrong");
    assert!(matches!(result, Err(crate::KrillnotesError::WrongPassword)));
}

#[test]
fn test_open_unencrypted_workspace_returns_specific_error() {
    let temp = NamedTempFile::new().unwrap();
    // Create a plain (unencrypted) SQLite database with the expected tables
    {
        let conn = rusqlite::Connection::open(temp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, node_type TEXT NOT NULL, parent_id TEXT, position INTEGER NOT NULL, created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL, created_by INTEGER NOT NULL DEFAULT 0, modified_by INTEGER NOT NULL DEFAULT 0, fields_json TEXT NOT NULL DEFAULT '{}', is_expanded INTEGER DEFAULT 1);
             CREATE TABLE operations (id INTEGER PRIMARY KEY AUTOINCREMENT, operation_id TEXT UNIQUE NOT NULL, timestamp INTEGER NOT NULL, device_id TEXT NOT NULL, operation_type TEXT NOT NULL, operation_data TEXT NOT NULL, synced INTEGER DEFAULT 0);
             CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE user_scripts (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '', source_code TEXT NOT NULL, load_order INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, modified_at INTEGER NOT NULL);",
        ).unwrap();
    }
    let result = Storage::open(temp.path(), "any_password");
    assert!(
        matches!(result, Err(crate::KrillnotesError::UnencryptedWorkspace)),
        "Expected UnencryptedWorkspace, got: {:?}", result
    );
}
```

Also update `test_open_existing_storage` to pass a password:
```rust
#[test]
fn test_open_existing_storage() {
    let temp = NamedTempFile::new().unwrap();
    Storage::create(temp.path(), "testpass").unwrap();
    let storage = Storage::open(temp.path(), "testpass").unwrap();
    // ... same assertions as before
}
```

And update `test_open_invalid_database` — no password change needed since it still returns a DB error before the table check.

**Step 2: Run to confirm failures**

```bash
cargo test -p krillnotes-core storage 2>&1 | tail -30
```
Expected: compile errors — `Storage::open` takes 1 arg not 2.

**Step 3: Rewrite `Storage::open`**

Replace the entire `open` function in `storage.rs:41-92`:
```rust
pub fn open<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
    let conn = Connection::open(path.as_ref())?;
    let escaped = password.replace('\'', "''");
    conn.execute_batch(&format!("PRAGMA key = '{escaped}';\n"))?;

    // Attempt to read the schema. With a wrong password, SQLCipher returns
    // garbage bytes and the query either errors or returns zero matching tables.
    let table_count: std::result::Result<i64, rusqlite::Error> = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table'
         AND name IN ('notes', 'operations', 'workspace_meta')",
        [],
        |row| row.get(0),
    );

    match table_count {
        Ok(3) => {
            // Correct password and valid workspace — run migrations.
            Self::run_migrations(&conn)?;
            Ok(Self { conn })
        }
        Ok(_) | Err(_) => {
            // Either wrong password or not a Krillnotes workspace.
            // Check if the file is a plain (unencrypted) SQLite database.
            let plain_conn = Connection::open(path.as_ref())?;
            // No PRAGMA key — opens as plaintext
            let plain_count: std::result::Result<i64, rusqlite::Error> = plain_conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table'
                 AND name IN ('notes', 'operations', 'workspace_meta')",
                [],
                |row| row.get(0),
            );
            match plain_count {
                Ok(3) => Err(crate::KrillnotesError::UnencryptedWorkspace),
                _ => Err(crate::KrillnotesError::WrongPassword),
            }
        }
    }
}

fn run_migrations(conn: &Connection) -> Result<()> {
    // Migration: add is_expanded column if absent.
    let column_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
        [],
        |row| row.get::<_, i64>(0).map(|c| c > 0),
    )?;
    if !column_exists {
        conn.execute("ALTER TABLE notes ADD COLUMN is_expanded INTEGER DEFAULT 1", [])?;
    }

    // Migration: add user_scripts table if absent.
    let user_scripts_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='user_scripts'",
        [],
        |row| row.get::<_, i64>(0).map(|c| c > 0),
    )?;
    if !user_scripts_exists {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_scripts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                source_code TEXT NOT NULL,
                load_order INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                modified_at INTEGER NOT NULL
            )",
        )?;
    }
    Ok(())
}
```

**Step 4: Run tests to verify all pass**

```bash
cargo test -p krillnotes-core storage 2>&1 | tail -30
```
Expected: all storage tests pass (including the 3 new ones + updated existing ones).

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/storage.rs
git commit -m "feat: add password + unencrypted detection to Storage::open"
```

---

## Task 5: Update Workspace — propagate password

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs:48-49` and `148-149`

**Step 1: Write failing tests**

In `workspace.rs` (look for `#[cfg(test)]` block at the bottom, or add one):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "secret").unwrap();
        // Should have at least one note (the root note)
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret").unwrap();
        let ws = Workspace::open(temp.path(), "secret").unwrap();
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_wrong_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret").unwrap();
        let result = Workspace::open(temp.path(), "wrong");
        assert!(matches!(result, Err(KrillnotesError::WrongPassword)));
    }
}
```

**Step 2: Run to confirm failures**

```bash
cargo test -p krillnotes-core workspace 2>&1 | tail -20
```
Expected: compile errors — `Workspace::create` and `Workspace::open` don't accept a password yet.

**Step 3: Update `Workspace::create` signature**

In `workspace.rs:48`, change:
```rust
pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
    let mut storage = Storage::create(&path)?;
```
to:
```rust
pub fn create<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
    let mut storage = Storage::create(&path, password)?;
```

**Step 4: Update `Workspace::open` signature**

In `workspace.rs:148`, change:
```rust
pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
    let storage = Storage::open(&path)?;
```
to:
```rust
pub fn open<P: AsRef<Path>>(path: P, password: &str) -> Result<Self> {
    let storage = Storage::open(&path, password)?;
```

**Step 5: Run tests to verify pass**

```bash
cargo test -p krillnotes-core workspace 2>&1 | tail -20
```
Expected: all 3 new tests pass.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: propagate password through Workspace::create and Workspace::open"
```

---

## Task 6: Update export.rs — import with workspace password

**Files:**
- Modify: `krillnotes-core/src/core/export.rs:270-340` (the `import_workspace` function)

**Step 1: Find the `import_workspace` signature**

Open `krillnotes-core/src/core/export.rs` and find the `import_workspace` function. It currently looks like:
```rust
pub fn import_workspace<R: Read + Seek>(
    reader: R,
    db_path: &Path,
    password: Option<&str>,
) -> Result<ImportResult, ExportError>
```
Rename `password` → `zip_password` and add `workspace_password: &str`.

**Step 2: Update the signature and the `Storage::create` call**

Change the function signature:
```rust
pub fn import_workspace<R: Read + Seek>(
    reader: R,
    db_path: &Path,
    zip_password: Option<&str>,
    workspace_password: &str,
) -> Result<ImportResult, ExportError>
```

At the line that calls `Storage::create(db_path)` (around line 335), change to:
```rust
let mut storage = Storage::create(db_path, workspace_password)
    .map_err(|e| ExportError::Database(e.to_string()))?;
```

Also rename all remaining uses of the old `password` parameter within the function body to `zip_password`.

**Step 3: Verify it compiles**

```bash
cargo build -p krillnotes-core 2>&1 | tail -20
```
Expected: build succeeds (the Tauri layer will now fail to compile — that's fine for now).

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/export.rs
git commit -m "feat: add workspace_password parameter to import_workspace"
```

---

## Task 7: Update AppSettings and AppState

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/settings.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:26-35`

**Step 1: Add `cache_workspace_passwords` to AppSettings**

In `settings.rs`, update the `AppSettings` struct:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub workspace_directory: String,
    #[serde(default)]
    pub cache_workspace_passwords: bool,
}
```

The `#[serde(default)]` makes old settings files load without error (defaults to `false`).

The `Default` impl doesn't need to change (it will use the field's own default of `false` via the derive).

**Step 2: Add `workspace_passwords` to AppState**

In `lib.rs:26-35`, update `AppState`:
```rust
pub struct AppState {
    pub workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
    pub focused_window: Arc<Mutex<Option<String>>>,
    /// In-memory password cache keyed by workspace file path.
    /// Populated only when settings.cacheWorkspacePasswords is true.
    pub workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>,
}
```

Find where `AppState` is constructed (in the `run()` function near the bottom of `lib.rs`) and add the new field:
```rust
workspace_passwords: Arc::new(Mutex::new(HashMap::new())),
```

**Step 3: Verify compilation**

```bash
cargo build -p krillnotes-desktop 2>&1 | grep "^error" | head -20
```
Expected: only errors about `create_workspace`, `open_workspace`, `execute_import` missing password args — all in lib.rs. No new errors from settings.rs.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/settings.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add cacheWorkspacePasswords to settings and workspace_passwords cache to AppState"
```

---

## Task 8: Update Tauri commands — create_workspace, open_workspace, execute_import

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:183-232` and `781-809`

**Step 1: Update `create_workspace` (lib.rs ~line 183)**

Add `password: String` parameter and pass it to `Workspace::create`. Also cache it if the setting is enabled:
```rust
#[tauri::command]
async fn create_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    if path_buf.exists() {
        return Err("File already exists. Use Open Workspace instead.".to_string());
    }

    match find_window_for_path(&state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &path_buf);
            let workspace = Workspace::create(&path_buf, &password)
                .map_err(|e| format!("Failed to create: {e}"))?;

            // Cache password if setting is enabled
            let settings = settings::load_settings();
            if settings.cache_workspace_passwords {
                state.workspace_passwords.lock().expect("Mutex poisoned")
                    .insert(path_buf.clone(), password);
            }

            let new_window = create_workspace_window(&app, &label)?;
            store_workspace(&state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {label}"))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}
```

**Step 2: Update `open_workspace` (lib.rs ~line 209)**

Add `password: String` parameter, handle cached passwords, and map new error variants:
```rust
#[tauri::command]
async fn open_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    if !path_buf.exists() {
        return Err("File does not exist".to_string());
    }

    match find_window_for_path(&state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &path_buf);
            let workspace = Workspace::open(&path_buf, &password)
                .map_err(|e| match e {
                    KrillnotesError::WrongPassword => "WRONG_PASSWORD".to_string(),
                    KrillnotesError::UnencryptedWorkspace => "UNENCRYPTED_WORKSPACE".to_string(),
                    other => format!("Failed to open: {other}"),
                })?;

            // Cache password if setting is enabled
            let settings = settings::load_settings();
            if settings.cache_workspace_passwords {
                state.workspace_passwords.lock().expect("Mutex poisoned")
                    .insert(path_buf.clone(), password);
            }

            let new_window = create_workspace_window(&app, &label)?;
            store_workspace(&state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {label}"))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}
```

**Step 3: Update `execute_import` (lib.rs ~line 781)**

Add `workspace_password: String` parameter:
```rust
#[tauri::command]
async fn execute_import(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    db_path: String,
    password: Option<String>,
    workspace_password: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let db_path_buf = PathBuf::from(&db_path);

    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    import_workspace(reader, &db_path_buf, password.as_deref(), &workspace_password)
        .map_err(|e| e.to_string())?;

    let workspace = Workspace::open(&db_path_buf, &workspace_password)
        .map_err(|e| e.to_string())?;
    let label = generate_unique_label(&state, &db_path_buf);

    let new_window = create_workspace_window(&app, &label)?;
    store_workspace(&state, label.clone(), workspace, db_path_buf);

    new_window.set_title(&format!("Krillnotes - {label}"))
        .map_err(|e| e.to_string())?;

    if window.label() == "main" {
        window.close().map_err(|e| e.to_string())?;
    }

    get_workspace_info_internal(&state, &label)
}
```

**Step 4: Verify compilation**

```bash
cargo build -p krillnotes-desktop 2>&1 | grep "^error" | head -20
```
Expected: clean build.

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add password parameters to create_workspace, open_workspace, execute_import commands"
```

---

## Task 9: Frontend — Add password dialog components

**Files:**
- Create: `krillnotes-desktop/src/components/SetPasswordDialog.tsx`
- Create: `krillnotes-desktop/src/components/EnterPasswordDialog.tsx`

**Step 1: Create `SetPasswordDialog.tsx`**

```tsx
import { useState, useEffect } from 'react';

interface SetPasswordDialogProps {
  isOpen: boolean;
  title?: string;
  onConfirm: (password: string) => void;
  onCancel: () => void;
}

function SetPasswordDialog({ isOpen, title = 'Set Password', onConfirm, onCancel }: SetPasswordDialogProps) {
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    if (isOpen) {
      setPassword('');
      setConfirm('');
      setError('');
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onCancel]);

  if (!isOpen) return null;

  const handleConfirm = () => {
    if (!password) {
      setError('Please enter a password.');
      return;
    }
    if (password !== confirm) {
      setError('Passwords do not match.');
      return;
    }
    onConfirm(password);
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleConfirm();
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{title}</h2>

        <div className="mb-3">
          <label className="block text-sm font-medium mb-2">Password</label>
          <input
            type="password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            onKeyDown={handleKeyPress}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            placeholder="Enter password"
          />
        </div>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">Confirm Password</label>
          <input
            type="password"
            value={confirm}
            onChange={e => setConfirm(e.target.value)}
            onKeyDown={handleKeyPress}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            placeholder="Repeat password"
          />
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="px-4 py-2 border border-secondary rounded hover:bg-secondary">
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
          >
            Confirm
          </button>
        </div>
      </div>
    </div>
  );
}

export default SetPasswordDialog;
```

**Step 2: Create `EnterPasswordDialog.tsx`**

```tsx
import { useState, useEffect } from 'react';

interface EnterPasswordDialogProps {
  isOpen: boolean;
  workspaceName: string;
  error?: string;
  onConfirm: (password: string) => void;
  onCancel: () => void;
}

function EnterPasswordDialog({ isOpen, workspaceName, error: externalError, onConfirm, onCancel }: EnterPasswordDialogProps) {
  const [password, setPassword] = useState('');

  useEffect(() => {
    if (isOpen) setPassword('');
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onCancel]);

  if (!isOpen) return null;

  const handleConfirm = () => {
    if (password) onConfirm(password);
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-1">Enter Password</h2>
        <p className="text-sm text-muted-foreground mb-4">"{workspaceName}"</p>

        <div className="mb-4">
          <input
            type="password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleConfirm()}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            placeholder="Workspace password"
          />
        </div>

        {externalError === 'WRONG_PASSWORD' && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            Wrong password — please try again.
          </div>
        )}

        {externalError === 'UNENCRYPTED_WORKSPACE' && (
          <div className="mb-4 p-3 bg-amber-500/10 border border-amber-500/20 text-amber-600 rounded text-sm">
            This workspace was created with an older version of Krillnotes.
            Please open it in the previous version, export it via <strong>File → Export Workspace</strong>,
            then import it here.
          </div>
        )}

        {externalError && externalError !== 'WRONG_PASSWORD' && externalError !== 'UNENCRYPTED_WORKSPACE' && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {externalError}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="px-4 py-2 border border-secondary rounded hover:bg-secondary">
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            disabled={!password}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50"
          >
            Open
          </button>
        </div>
      </div>
    </div>
  );
}

export default EnterPasswordDialog;
```

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/SetPasswordDialog.tsx krillnotes-desktop/src/components/EnterPasswordDialog.tsx
git commit -m "feat: add SetPasswordDialog and EnterPasswordDialog components"
```

---

## Task 10: Frontend — Update NewWorkspaceDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/NewWorkspaceDialog.tsx`

The dialog needs a two-step flow: name entry → password entry. Implement this by adding a `step` state (`'name' | 'password'`) and rendering `SetPasswordDialog` after the name is entered.

**Step 1: Update `NewWorkspaceDialog.tsx`**

Replace the entire file content:
```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings, WorkspaceInfo } from '../types';
import SetPasswordDialog from './SetPasswordDialog';

function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

interface NewWorkspaceDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function NewWorkspaceDialog({ isOpen, onClose }: NewWorkspaceDialogProps) {
  const [step, setStep] = useState<'name' | 'password'>('name');
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);
  const [workspaceDir, setWorkspaceDir] = useState('');

  useEffect(() => {
    if (isOpen) {
      setStep('name');
      setName('');
      setError('');
      setCreating(false);
      invoke<AppSettings>('get_settings')
        .then(s => setWorkspaceDir(s.workspaceDirectory))
        .catch(err => setError(`Failed to load settings: ${err}`));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !creating && step === 'name') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, creating, step]);

  if (!isOpen) return null;

  const handleNameNext = () => {
    const trimmed = name.trim();
    if (!trimmed) { setError('Please enter a workspace name.'); return; }
    const slug = slugify(trimmed);
    if (!slug) { setError('Name must contain at least one letter or number.'); return; }
    setError('');
    setStep('password');
  };

  const handlePasswordConfirm = async (password: string) => {
    const slug = slugify(name.trim());
    const path = `${workspaceDir}/${slug}.db`;
    setCreating(true);
    try {
      await invoke<WorkspaceInfo>('create_workspace', { path, password });
      onClose();
    } catch (err) {
      if (err !== 'focused_existing') {
        setError(`${err}`);
        setStep('name');
      }
      setCreating(false);
    }
  };

  if (step === 'password') {
    return (
      <SetPasswordDialog
        isOpen={true}
        title="Set Workspace Password"
        onConfirm={handlePasswordConfirm}
        onCancel={() => setStep('name')}
      />
    );
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">New Workspace</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">Workspace Name</label>
          <input
            type="text"
            value={name}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && !creating && handleNameNext()}
            placeholder="My Workspace"
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            disabled={creating}
          />
          {workspaceDir && (
            <p className="text-xs text-muted-foreground mt-1">
              Will be saved to: {workspaceDir}/{slugify(name.trim()) || '...'}.db
            </p>
          )}
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="px-4 py-2 border border-secondary rounded hover:bg-secondary" disabled={creating}>
            Cancel
          </button>
          <button
            onClick={handleNameNext}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={creating || !name.trim()}
          >
            Next
          </button>
        </div>
      </div>
    </div>
  );
}

export default NewWorkspaceDialog;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/NewWorkspaceDialog.tsx
git commit -m "feat: add two-step password flow to NewWorkspaceDialog"
```

---

## Task 11: Frontend — Update OpenWorkspaceDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/OpenWorkspaceDialog.tsx`

Add a two-step flow: select workspace → enter password. The password dialog re-opens with an error message if the password is wrong. If the error is `UNENCRYPTED_WORKSPACE`, display the migration message and don't allow retry.

**Step 1: Update `OpenWorkspaceDialog.tsx`**

Replace the entire file:
```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { WorkspaceEntry, WorkspaceInfo } from '../types';
import EnterPasswordDialog from './EnterPasswordDialog';

interface OpenWorkspaceDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function OpenWorkspaceDialog({ isOpen, onClose }: OpenWorkspaceDialogProps) {
  const [entries, setEntries] = useState<WorkspaceEntry[]>([]);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [selectedEntry, setSelectedEntry] = useState<WorkspaceEntry | null>(null);
  const [passwordError, setPasswordError] = useState('');
  const [opening, setOpening] = useState(false);

  useEffect(() => {
    if (isOpen) {
      setError('');
      setSelectedEntry(null);
      setPasswordError('');
      setOpening(false);
      setLoading(true);
      invoke<WorkspaceEntry[]>('list_workspace_files')
        .then(setEntries)
        .catch(err => setError(`Failed to list workspaces: ${err}`))
        .finally(() => setLoading(false));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !opening && !selectedEntry) onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, opening, selectedEntry]);

  if (!isOpen) return null;

  const handleSelectEntry = (entry: WorkspaceEntry) => {
    if (entry.isOpen) return;
    setPasswordError('');
    setSelectedEntry(entry);
  };

  const handlePasswordConfirm = async (password: string) => {
    if (!selectedEntry) return;
    setOpening(true);
    setPasswordError('');
    try {
      await invoke<WorkspaceInfo>('open_workspace', { path: selectedEntry.path, password });
      onClose();
    } catch (err) {
      const errStr = `${err}`;
      setPasswordError(errStr);
      if (errStr !== 'WRONG_PASSWORD') {
        // For UNENCRYPTED_WORKSPACE or other errors, don't allow retrying — stay with error shown
      }
      setOpening(false);
    }
  };

  const handlePasswordCancel = () => {
    setSelectedEntry(null);
    setPasswordError('');
  };

  if (selectedEntry) {
    return (
      <EnterPasswordDialog
        isOpen={true}
        workspaceName={selectedEntry.name}
        error={passwordError}
        onConfirm={handlePasswordConfirm}
        onCancel={handlePasswordCancel}
      />
    );
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary rounded-lg w-[450px] max-h-[60vh] flex flex-col">
        <div className="p-6 pb-0">
          <h2 className="text-xl font-bold mb-4">Open Workspace</h2>
        </div>

        <div className="flex-1 overflow-y-auto px-6">
          {loading ? (
            <p className="text-muted-foreground text-center py-8">Loading...</p>
          ) : entries.length === 0 ? (
            <p className="text-muted-foreground text-center py-8">
              No workspaces found in the default directory.<br />
              Use "New Workspace" to create one.
            </p>
          ) : (
            <div className="space-y-1">
              {entries.map(entry => (
                <button
                  key={entry.path}
                  onClick={() => handleSelectEntry(entry)}
                  disabled={opening || entry.isOpen}
                  className={`w-full text-left px-3 py-2 rounded-md flex items-center justify-between ${
                    entry.isOpen
                      ? 'opacity-40 cursor-not-allowed'
                      : 'hover:bg-secondary/50 disabled:opacity-50'
                  }`}
                >
                  <span className="font-medium truncate">{entry.name}</span>
                  {entry.isOpen && (
                    <span className="text-xs text-muted-foreground ml-2">Already open</span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>

        {error && (
          <div className="px-6 pt-2">
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {error}
            </div>
          </div>
        )}

        <div className="flex justify-end p-6 pt-4">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={opening}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

export default OpenWorkspaceDialog;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/OpenWorkspaceDialog.tsx
git commit -m "feat: add password prompt flow to OpenWorkspaceDialog"
```

---

## Task 12: Frontend — Update SettingsDialog and types.ts

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:89-91`
- Modify: `krillnotes-desktop/src/components/SettingsDialog.tsx`

**Step 1: Update `types.ts` — add `cacheWorkspacePasswords`**

In `types.ts:89-91`, update the `AppSettings` interface:
```typescript
export interface AppSettings {
  workspaceDirectory: string;
  cacheWorkspacePasswords: boolean;
}
```

**Step 2: Update `SettingsDialog.tsx` — add the toggle**

Add a `cachePasswords` state variable, load/save it alongside `workspaceDirectory`, and render a toggle:

In `SettingsDialog.tsx`, add state:
```tsx
const [cachePasswords, setCachePasswords] = useState(false);
```

In the `useEffect` that loads settings, add:
```tsx
setCachePasswords(s.cacheWorkspacePasswords);
```

In `handleSave`, update the invoke call:
```tsx
await invoke('update_settings', {
  settings: {
    workspaceDirectory: workspaceDir,
    cacheWorkspacePasswords: cachePasswords,
  },
});
```

Add this UI block after the workspace directory section:
```tsx
<div className="mb-4">
  <label className="flex items-center gap-3 cursor-pointer">
    <input
      type="checkbox"
      checked={cachePasswords}
      onChange={e => setCachePasswords(e.target.checked)}
      className="w-4 h-4"
    />
    <div>
      <span className="block text-sm font-medium">Remember workspace passwords for this session</span>
      <span className="block text-xs text-muted-foreground mt-0.5">
        Passwords are kept in memory until the app closes. Off by default.
      </span>
    </div>
  </label>
</div>
```

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/types.ts krillnotes-desktop/src/components/SettingsDialog.tsx
git commit -m "feat: add cacheWorkspacePasswords toggle to settings"
```

---

## Task 13: Frontend — Update the import workspace flow

The import flow lives in `App.tsx`. Find the section that calls `execute_import` and add a `SetPasswordDialog` step for the workspace password.

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx` (find the import flow section)

**Step 1: Locate the import flow**

Search `App.tsx` for `execute_import` or `peek_import_cmd`. The import flow typically:
1. Opens a file picker for the zip
2. Calls `peek_import_cmd` to detect if zip is encrypted → shows zip password dialog if needed
3. Calls `execute_import` with the zip path, db path, and zip password

**Step 2: Add workspace password state**

Add state near the import-related state:
```tsx
const [importWorkspacePassword, setImportWorkspacePassword] = useState('');
const [showImportPasswordDialog, setShowImportPasswordDialog] = useState(false);
const [pendingImportArgs, setPendingImportArgs] = useState<{zipPath: string, dbPath: string, zipPassword?: string} | null>(null);
```

**Step 3: Insert SetPasswordDialog step before calling `execute_import`**

Instead of calling `execute_import` directly after getting the zip password, store the args and show `SetPasswordDialog`:
```tsx
// When ready to call execute_import, instead do:
setPendingImportArgs({ zipPath, dbPath, zipPassword: zipPwd });
setShowImportPasswordDialog(true);
```

Add a handler for when the workspace password is confirmed:
```tsx
const handleImportWorkspacePassword = async (wsPassword: string) => {
  if (!pendingImportArgs) return;
  setShowImportPasswordDialog(false);
  try {
    await invoke<WorkspaceInfo>('execute_import', {
      zipPath: pendingImportArgs.zipPath,
      dbPath: pendingImportArgs.dbPath,
      password: pendingImportArgs.zipPassword ?? null,
      workspacePassword: wsPassword,
    });
    setPendingImportArgs(null);
  } catch (err) {
    // show error
  }
};
```

Add `SetPasswordDialog` to the render:
```tsx
<SetPasswordDialog
  isOpen={showImportPasswordDialog}
  title="Set Password for Imported Workspace"
  onConfirm={handleImportWorkspacePassword}
  onCancel={() => { setShowImportPasswordDialog(false); setPendingImportArgs(null); }}
/>
```

**Step 4: Verify the app builds**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -30
```
Expected: TypeScript compiles without errors.

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat: add workspace password step to import flow"
```

---

## Task 14: Manual smoke test and final commit

**Step 1: Start the app in dev mode**

```bash
cd krillnotes-desktop && npm run tauri dev
```

**Step 2: Test create + reopen**

1. Create a new workspace → enter a name → set a password → verify it opens
2. Close the workspace window
3. Open the same workspace from the list → enter the correct password → verify it opens
4. Close again → try wrong password → verify "Wrong password" error appears

**Step 3: Test unencrypted workspace detection**

Build without this PR's changes would be hard — skip unless you have an old `.db` file handy. If you do:
1. Try to open an old unencrypted `.db` → verify the migration message appears

**Step 4: Test import flow**

1. Export current workspace (any password or none)
2. Import the zip → enter zip password if set → set a workspace password → verify it opens

**Step 5: Test session caching**

1. Enable "Remember workspace passwords" in Settings
2. Open a workspace → enter password → close window
3. Re-open same workspace → verify no password prompt appears

**Step 6: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: address smoke test issues"
```
