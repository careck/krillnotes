# ZIP Password Encryption Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add optional AES-256 password encryption to workspace exports, with automatic detection and password prompt on import.

**Architecture:** Enable the `aes-crypto` feature in the existing `zip` v8 crate (zero new dependencies). Add `EncryptedArchive` / `InvalidPassword` error variants, thread an `Option<&str> password` through all three core functions and their matching Tauri commands, then add two inline dialogs in `App.tsx` following the existing workspace-name dialog pattern.

**Tech Stack:** Rust (`zip` v8 + `aes-crypto` feature, `thiserror`), Tauri v2, TypeScript + React (Tailwind CSS inline dialogs)

---

## Task 1: Enable aes-crypto + add ExportError variants

**Files:**
- Modify: `krillnotes-core/Cargo.toml:19`
- Modify: `krillnotes-core/src/core/export.rs:54-70`

**Step 1: Add the aes-crypto feature to the zip dependency**

In `krillnotes-core/Cargo.toml`, change line 19 from:
```toml
zip = { version = "8", default-features = false, features = ["deflate"] }
```
to:
```toml
zip = { version = "8", default-features = false, features = ["deflate", "aes-crypto"] }
```

**Step 2: Add two new ExportError variants**

In `krillnotes-core/src/core/export.rs`, extend `ExportError` (currently ends at line 70) to add two new arms after the `Database` variant:

```rust
    #[error("Archive is password-protected; provide a password to decrypt")]
    EncryptedArchive,

    #[error("Incorrect password")]
    InvalidPassword,
```

The full enum should now look like:
```rust
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid export format: {0}")]
    InvalidFormat(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Archive is password-protected; provide a password to decrypt")]
    EncryptedArchive,

    #[error("Incorrect password")]
    InvalidPassword,
}
```

**Step 3: Verify it compiles**

```bash
cargo test -p krillnotes-core 2>&1 | head -20
```
Expected: all existing tests still pass. No new tests yet.

**Step 4: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/Cargo.toml krillnotes-core/src/core/export.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: add aes-crypto feature and EncryptedArchive/InvalidPassword error variants"
```

---

## Task 2: Add read_entry helpers + extend export_workspace

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`

### Step 1: Add the failing test

Add to the `#[cfg(test)]` block at the bottom of `export.rs`:

```rust
#[test]
fn test_export_with_password_creates_encrypted_zip() {
    let temp = NamedTempFile::new().unwrap();
    let ws = Workspace::create(temp.path()).unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), Some("hunter2")).unwrap();

    // notes.json should be marked as encrypted
    let reader = Cursor::new(&buf);
    let mut archive = ZipArchive::new(reader).unwrap();
    let notes_file = archive.by_name("notes.json").unwrap();
    assert!(notes_file.encrypted(), "notes.json must be encrypted when password is provided");
}

#[test]
fn test_export_without_password_creates_plain_zip() {
    let temp = NamedTempFile::new().unwrap();
    let ws = Workspace::create(temp.path()).unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

    let reader = Cursor::new(&buf);
    let mut archive = ZipArchive::new(reader).unwrap();
    let notes_file = archive.by_name("notes.json").unwrap();
    assert!(!notes_file.encrypted(), "notes.json must be plain when no password given");
}
```

### Step 2: Run to see it fail

```bash
cargo test -p krillnotes-core test_export_with_password 2>&1
```
Expected: compile error — `export_workspace` doesn't accept the `password` argument yet.

### Step 3: Add the helper functions and update imports

Near the top of `export.rs`, after the existing `use zip::write::SimpleFileOptions;` line, add:
```rust
use zip::write::AesMode;
```

After the `slugify_script_name` function (around line 85) and before `export_workspace`, add these two private helpers:

```rust
/// Opens a named entry, decrypting with `password` if provided.
/// Returns `ExportError::InvalidPassword` if the password is wrong.
/// Returns `ExportError::InvalidFormat` if the entry doesn't exist.
fn read_entry<'a, R: Read + Seek>(
    archive: &'a mut ZipArchive<R>,
    name: &str,
    password: Option<&str>,
) -> Result<zip::read::ZipFile<'a>, ExportError> {
    if let Some(pwd) = password {
        archive
            .by_name_decrypt(name, pwd.as_bytes())
            .map_err(|_| ExportError::InvalidFormat(format!("Missing '{name}' in archive")))?
            .map_err(|_| ExportError::InvalidPassword)
    } else {
        archive
            .by_name(name)
            .map_err(|_| ExportError::InvalidFormat(format!("Missing '{name}' in archive")))
    }
}

/// Like `read_entry` but returns `None` instead of an error when the entry is absent.
fn try_read_entry<'a, R: Read + Seek>(
    archive: &'a mut ZipArchive<R>,
    name: &str,
    password: Option<&str>,
) -> Option<zip::read::ZipFile<'a>> {
    if let Some(pwd) = password {
        archive.by_name_decrypt(name, pwd.as_bytes()).ok()?.ok()
    } else {
        archive.by_name(name).ok()
    }
}
```

### Step 4: Update export_workspace signature and body

Change the function signature:
```rust
pub fn export_workspace<W: Write + Seek>(
    workspace: &Workspace,
    writer: W,
    password: Option<&str>,
) -> Result<(), ExportError> {
```

Inside the function, replace the `options` line:
```rust
// OLD:
let options =
    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

// NEW:
let options = match password {
    Some(pwd) => SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .with_aes_encryption(AesMode::Aes256, pwd),
    None => SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated),
};
```

No other changes to `export_workspace` — `options` is already used for all three `start_file` calls.

### Step 5: Fix the existing tests that call export_workspace

Update every existing call to `export_workspace` in the `#[cfg(test)]` block to pass `None` as the third argument:
- `test_export_workspace_creates_valid_zip`: `export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();`
- `test_peek_import_reads_metadata`: `export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();`
- `test_round_trip_export_import`: `export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();`

Also update the call in `test_import_missing_notes_json` (that one doesn't call `export_workspace`, but double-check).

### Step 6: Run the tests

```bash
cargo test -p krillnotes-core 2>&1
```
Expected: all tests pass, including the two new ones.

### Step 7: Commit

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/export.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: export_workspace accepts optional AES-256 password"
```

---

## Task 3: Extend peek_import to detect and decrypt

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`

### Step 1: Add the three failing tests

Add to the `#[cfg(test)]` block:

```rust
#[test]
fn test_peek_import_returns_encrypted_archive_error_when_no_password() {
    let temp = NamedTempFile::new().unwrap();
    let ws = Workspace::create(temp.path()).unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

    let err = peek_import(Cursor::new(&buf), None).unwrap_err();
    assert!(matches!(err, ExportError::EncryptedArchive), "got: {err:?}");
}

#[test]
fn test_peek_import_with_correct_password_succeeds() {
    let temp = NamedTempFile::new().unwrap();
    let ws = Workspace::create(temp.path()).unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

    let result = peek_import(Cursor::new(&buf), Some("s3cr3t")).unwrap();
    assert_eq!(result.app_version, APP_VERSION);
    assert!(result.note_count >= 1);
}

#[test]
fn test_peek_import_with_wrong_password_returns_invalid_password() {
    let temp = NamedTempFile::new().unwrap();
    let ws = Workspace::create(temp.path()).unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

    let err = peek_import(Cursor::new(&buf), Some("wrong-password")).unwrap_err();
    assert!(matches!(err, ExportError::InvalidPassword), "got: {err:?}");
}
```

### Step 2: Run to see them fail

```bash
cargo test -p krillnotes-core test_peek_import_returns_encrypted 2>&1
```
Expected: compile error — `peek_import` doesn't accept `password` yet.

### Step 3: Update peek_import

Replace the entire `peek_import` function with:

```rust
pub fn peek_import<R: Read + Seek>(reader: R, password: Option<&str>) -> Result<ImportResult, ExportError> {
    let mut archive = ZipArchive::new(reader)?;

    // Detect encryption before trying to read data
    {
        let check = archive.by_name("notes.json").map_err(|_| {
            ExportError::InvalidFormat("Missing notes.json in archive".to_string())
        })?;
        if check.encrypted() && password.is_none() {
            return Err(ExportError::EncryptedArchive);
        }
    }

    let notes_file = read_entry(&mut archive, "notes.json", password)?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_file)?;

    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    let script_count = match try_read_entry(&mut archive, "scripts/scripts.json", password) {
        Some(manifest_file) => {
            let manifest: ScriptManifest = serde_json::from_reader(manifest_file)?;
            manifest.scripts.len()
        }
        None => 0,
    };

    Ok(ImportResult {
        app_version: export_notes.app_version,
        note_count: export_notes.notes.len(),
        script_count,
    })
}
```

### Step 4: Fix the existing peek_import test call

In `test_peek_import_reads_metadata`, update:
```rust
let result = peek_import(Cursor::new(&buf), None).unwrap();
```

### Step 5: Run the tests

```bash
cargo test -p krillnotes-core 2>&1
```
Expected: all tests pass.

### Step 6: Commit

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/export.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: peek_import detects encrypted archives and accepts password"
```

---

## Task 4: Extend import_workspace to decrypt

**Files:**
- Modify: `krillnotes-core/src/core/export.rs`

### Step 1: Add the failing test

Add to the `#[cfg(test)]` block:

```rust
#[test]
fn test_encrypted_round_trip_import() {
    // Build a workspace with a child note
    let temp_src = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp_src.path()).unwrap();
    let root = ws.list_all_notes().unwrap()[0].clone();
    ws.update_note_title(&root.id, "Encrypted Root".to_string()).unwrap();

    // Export with password
    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), Some("mypass")).unwrap();

    // Import with correct password → should succeed
    let temp_dst = NamedTempFile::new().unwrap();
    let result = import_workspace(Cursor::new(&buf), temp_dst.path(), Some("mypass")).unwrap();
    assert_eq!(result.note_count, 1);

    // Verify imported note title
    let imported_ws = Workspace::open(temp_dst.path()).unwrap();
    let notes = imported_ws.list_all_notes().unwrap();
    assert!(notes.iter().any(|n| n.title == "Encrypted Root"));
}
```

### Step 2: Run to see it fail

```bash
cargo test -p krillnotes-core test_encrypted_round_trip_import 2>&1
```
Expected: compile error — `import_workspace` doesn't accept `password` yet.

### Step 3: Update import_workspace signature

Change the signature to:
```rust
pub fn import_workspace<R: Read + Seek>(reader: R, db_path: &Path, password: Option<&str>) -> Result<ImportResult, ExportError> {
```

### Step 4: Add encryption detection + use read_entry helpers in import_workspace

Replace the start of `import_workspace` (the part that opens the archive and reads `notes.json`) with:

```rust
    let mut archive = ZipArchive::new(reader)?;

    // Detect encryption before trying to read data
    {
        let check = archive.by_name("notes.json").map_err(|_| {
            ExportError::InvalidFormat("Missing notes.json in archive".to_string())
        })?;
        if check.encrypted() && password.is_none() {
            return Err(ExportError::EncryptedArchive);
        }
    }

    let notes_file = read_entry(&mut archive, "notes.json", password)?;
    let export_notes: ExportNotes = serde_json::from_reader(notes_file)?;

    if export_notes.version != 1 {
        return Err(ExportError::InvalidFormat(format!(
            "Unsupported export format version: {}",
            export_notes.version
        )));
    }

    // Read script manifest and source files
    let manifest = match try_read_entry(&mut archive, "scripts/scripts.json", password) {
        Some(manifest_file) => {
            let m: ScriptManifest = serde_json::from_reader(manifest_file)?;
            Some(m)
        }
        None => None,
    };
```

Then update the individual `.rhai` file reads inside the `for entry in &manifest.scripts` loop — replace `archive.by_name(&path)` with `read_entry(&mut archive, &path, password)`:

```rust
        for entry in &manifest.scripts {
            let path = format!("scripts/{}", entry.filename);
            let mut rhai_file = read_entry(&mut archive, &path, password).map_err(|e| {
                ExportError::InvalidFormat(format!(
                    "Script file '{}' referenced in manifest but missing from archive: {}",
                    path, e
                ))
            })?;
            let mut source = String::new();
            rhai_file.read_to_string(&mut source)?;
            script_sources.push((source, entry.load_order, entry.enabled));
        }
```

### Step 5: Fix the existing round-trip test call

In `test_round_trip_export_import` and `test_import_invalid_zip` and `test_import_missing_notes_json`, update all calls:
```rust
import_workspace(Cursor::new(&buf), temp_dst.path(), None).unwrap()
import_workspace(Cursor::new(garbage), Path::new("/tmp/invalid.db"), None)
import_workspace(Cursor::new(&buf), Path::new("/tmp/missing_notes.db"), None)
```

### Step 6: Run all tests

```bash
cargo test -p krillnotes-core 2>&1
```
Expected: all tests pass.

### Step 7: Commit

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/export.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: import_workspace accepts password, detects encrypted archives"
```

---

## Task 5: Update Tauri commands to pass password through

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:750-804`

No new tests needed — the Rust compiler will verify correctness, and end-to-end behaviour is covered by the frontend integration.

### Step 1: Update export_workspace_cmd

Replace the current `export_workspace_cmd` function (lines 750-761):

```rust
#[tauri::command]
fn export_workspace_cmd(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
) -> std::result::Result<(), String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    export_workspace(workspace, file, password.as_deref()).map_err(|e| e.to_string())
}
```

### Step 2: Update peek_import_cmd — add password + sentinel errors

Replace the current `peek_import_cmd` function (lines 765-771):

```rust
#[tauri::command]
fn peek_import_cmd(
    zip_path: String,
    password: Option<String>,
) -> std::result::Result<ImportResult, String> {
    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    peek_import(reader, password.as_deref()).map_err(|e| match e {
        krillnotes_core::core::export::ExportError::EncryptedArchive => {
            "ENCRYPTED_ARCHIVE".to_string()
        }
        krillnotes_core::core::export::ExportError::InvalidPassword => {
            "INVALID_PASSWORD".to_string()
        }
        other => other.to_string(),
    })
}
```

> **Note:** Check the existing `use` imports at the top of `lib.rs` to see how `ExportError` is currently imported. If `export::ExportError` is already in scope via a `use` statement, you can shorten the match arms. If only `peek_import` (the function) is imported, you need the full path. Use whichever matches the existing import style.

### Step 3: Update execute_import — add password parameter

Replace the current `execute_import` function (lines 774-804):

```rust
#[tauri::command]
async fn execute_import(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    db_path: String,
    password: Option<String>,
) -> std::result::Result<WorkspaceInfo, String> {
    let db_path_buf = PathBuf::from(&db_path);

    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    import_workspace(reader, &db_path_buf, password.as_deref()).map_err(|e| e.to_string())?;

    let workspace = Workspace::open(&db_path_buf).map_err(|e| e.to_string())?;
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

### Step 4: Build to confirm no errors

```bash
cargo build -p krillnotes-desktop 2>&1
```
Expected: builds cleanly.

### Step 5: Commit

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-desktop/src-tauri/src/lib.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: pass password through all Tauri export/import commands"
```

---

## Task 6: Frontend — export password dialog

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

### Step 1: Add state variables

In the `App` function body (after the existing `importing` state, around line 114), add:

```typescript
  const [showExportPasswordDialog, setShowExportPasswordDialog] = useState(false);
  const [exportPassword, setExportPassword] = useState('');
  const [exportPasswordConfirm, setExportPasswordConfirm] = useState('');
```

### Step 2: Thread setShowExportPasswordDialog into createMenuHandlers

`createMenuHandlers` is called with setter functions. We need to pass `setShowExportPasswordDialog` into it so the export handler can show the dialog.

**Update the `createMenuHandlers` signature** (line 28–35) to add the new setter:

```typescript
const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setImportState: (state: ImportState | null) => void,
  setShowExportPasswordDialog: (show: boolean) => void,
  workspace: WorkspaceInfoType | null,
) => ({
```

**Update the call site** in the `useEffect` (around line 143–150):

```typescript
    const handlers = createMenuHandlers(
      statusSetter,
      setShowNewWorkspace,
      setShowOpenWorkspace,
      setShowSettings,
      setImportState,
      setShowExportPasswordDialog,
      workspace,
    );
```

### Step 3: Update the export handler to show the dialog

In `createMenuHandlers`, replace the `'File > Export Workspace clicked'` handler:

```typescript
  'File > Export Workspace clicked': () => {
    setShowExportPasswordDialog(true);
  },
```

The actual export logic (save dialog + invoke) moves to a new `handleExportConfirm` function inside `App`.

### Step 4: Add handleExportConfirm

Add a new function after `handleImportConfirm` (around line 197):

```typescript
  const handleExportConfirm = async (password: string | null) => {
    setShowExportPasswordDialog(false);
    setExportPassword('');
    setExportPasswordConfirm('');

    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        defaultPath: `${(workspace?.filename ?? 'workspace').replace(/\.db$/, '')}.krillnotes.zip`,
        title: 'Export Workspace',
      });

      if (!path) return;

      await invoke('export_workspace_cmd', { path, password });
      setStatus('Workspace exported successfully');
    } catch (error) {
      setStatus(`Export failed: ${error}`, true);
    }
  };
```

### Step 5: Add the export password dialog JSX

In the return statement, add the export password dialog after the `<SettingsDialog>` block (around line 224), before the import name dialog:

```tsx
      {/* Export password dialog */}
      {showExportPasswordDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">Protect with a password?</h2>
            <p className="text-sm text-muted-foreground mb-4">
              Leave blank to export without encryption.
            </p>
            <div className="mb-3">
              <label className="block text-sm font-medium mb-2">Password</label>
              <input
                type="password"
                value={exportPassword}
                onChange={(e) => setExportPassword(e.target.value)}
                placeholder="Optional password"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
              />
            </div>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">Confirm password</label>
              <input
                type="password"
                value={exportPasswordConfirm}
                onChange={(e) => setExportPasswordConfirm(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const pwd = exportPassword.trim();
                    if (!pwd || pwd === exportPasswordConfirm) {
                      handleExportConfirm(pwd || null);
                    }
                  }
                }}
                placeholder="Confirm password"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              />
            </div>
            {exportPassword && exportPasswordConfirm && exportPassword !== exportPasswordConfirm && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                Passwords do not match.
              </div>
            )}
            <div className="flex justify-between items-center">
              <button
                onClick={() => {
                  setShowExportPasswordDialog(false);
                  setExportPassword('');
                  setExportPasswordConfirm('');
                }}
                className="text-sm text-muted-foreground hover:text-foreground underline"
              >
                Cancel
              </button>
              <div className="flex gap-2">
                <button
                  onClick={() => handleExportConfirm(null)}
                  className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
                >
                  Skip — no encryption
                </button>
                <button
                  onClick={() => handleExportConfirm(exportPassword)}
                  disabled={!exportPassword || exportPassword !== exportPasswordConfirm}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  Encrypt
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
```

### Step 6: Build and manual smoke test

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build 2>&1
```
Expected: builds cleanly. Then launch the app and:
1. File → Export Workspace
2. Verify the password dialog appears
3. Click "Skip — no encryption" → save dialog opens → export completes → status message appears
4. Try with a password → export completes

### Step 7: Commit

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-desktop/src/App.tsx
git -C /Users/careck/Source/Krillnotes commit -m "feat: add export password dialog (AES-256 optional encryption)"
```

---

## Task 7: Frontend — import password dialog

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

### Step 1: Add state variables for import password

After the existing import-related state (around line 113), add:

```typescript
  const [showImportPasswordDialog, setShowImportPasswordDialog] = useState(false);
  const [importPassword, setImportPassword] = useState('');
  const [importPasswordError, setImportPasswordError] = useState('');
  const [pendingImportZipPath, setPendingImportZipPath] = useState<string | null>(null);
  const [pendingImportPassword, setPendingImportPassword] = useState<string | null>(null);
```

### Step 2: Thread setPendingImportPassword into the import handler

The import handler in `createMenuHandlers` needs to:
1. Open the zip picker
2. Call `peek_import_cmd` with `password: null`
3. If error is `"ENCRYPTED_ARCHIVE"`, store the zip path and show the password dialog
4. Otherwise, continue to the version check and workspace-name dialog

We also need to propagate the resolved `pendingImportPassword` through to `execute_import`.

**Update `createMenuHandlers` signature** to add `setPendingImportZipPath` and `setShowImportPasswordDialog`:

```typescript
const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setImportState: (state: ImportState | null) => void,
  setShowExportPasswordDialog: (show: boolean) => void,
  setPendingImportZipPath: (path: string | null) => void,
  setShowImportPasswordDialog: (show: boolean) => void,
  workspace: WorkspaceInfoType | null,
) => ({
```

**Update the call site** in `useEffect`:

```typescript
    const handlers = createMenuHandlers(
      statusSetter,
      setShowNewWorkspace,
      setShowOpenWorkspace,
      setShowSettings,
      setImportState,
      setShowExportPasswordDialog,
      setPendingImportZipPath,
      setShowImportPasswordDialog,
      workspace,
    );
```

### Step 3: Extract peek logic into a reusable helper

Extract the "peek and proceed to workspace-name dialog" logic into a standalone function inside `App`, so it can be called both from the initial menu handler and from the password dialog "Decrypt" button:

Add this function after `handleExportConfirm`:

```typescript
  const proceedWithImport = async (zipPath: string, password: string | null) => {
    try {
      const result = await invoke<{ appVersion: string; noteCount: number; scriptCount: number }>(
        'peek_import_cmd', { zipPath, password }
      );

      const currentVersion = await invoke<string>('get_app_version');
      if (result.appVersion > currentVersion) {
        const { confirm } = await import('@tauri-apps/plugin-dialog');
        const proceed = await confirm(
          `This export was created with Krillnotes v${result.appVersion}, but you are running v${currentVersion}. Some data may not import correctly.\n\nImport anyway?`,
          { title: 'Version Mismatch', kind: 'warning' }
        );
        if (!proceed) return;
      }

      setPendingImportPassword(password);
      setImportState({
        zipPath,
        noteCount: result.noteCount,
        scriptCount: result.scriptCount,
      });
    } catch (error) {
      const errStr = `${error}`;
      if (errStr === 'ENCRYPTED_ARCHIVE') {
        setPendingImportZipPath(zipPath);
        setImportPassword('');
        setImportPasswordError('');
        setShowImportPasswordDialog(true);
      } else if (errStr === 'INVALID_PASSWORD') {
        setImportPasswordError('Incorrect password — try again.');
      } else {
        setStatus(`Import failed: ${errStr}`, true);
      }
    }
  };
```

### Step 4: Update the 'File > Import Workspace clicked' handler

Replace the existing handler in `createMenuHandlers`:

```typescript
  'File > Import Workspace clicked': async () => {
    try {
      const zipPath = await open({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        multiple: false,
        title: 'Import Workspace',
      });

      if (!zipPath || Array.isArray(zipPath)) return;

      // proceedWithImport is defined in App and handles encryption detection
      // We call it via the setter approach — see handleImportFromMenu below
      setPendingImportZipPath(zipPath as string);
      // Trigger the peek (we'll do it in a useEffect or call directly)
    } catch (error) {
      setStatus(`Import failed: ${error}`, true);
    }
  },
```

> **Implementation note:** Because `proceedWithImport` is defined inside `App` and `createMenuHandlers` is defined outside, the cleanest approach is to pass `proceedWithImport` as a callback into `createMenuHandlers`, similar to how `setStatus` is passed. Add a `doImport: (zipPath: string) => void` parameter to `createMenuHandlers` and use it in the handler:

```typescript
// Simplified handler after adding doImport callback:
  'File > Import Workspace clicked': async () => {
    try {
      const zipPath = await open({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        multiple: false,
        title: 'Import Workspace',
      });
      if (!zipPath || Array.isArray(zipPath)) return;
      doImport(zipPath as string);
    } catch (error) {
      setStatus(`Import failed: ${error}`, true);
    }
  },
```

**Update `createMenuHandlers` signature** to include `doImport`:

```typescript
const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setImportState: (state: ImportState | null) => void,
  setShowExportPasswordDialog: (show: boolean) => void,
  doImport: (zipPath: string) => void,
  workspace: WorkspaceInfoType | null,
) => ({
```

This simplifies the signature (remove `setPendingImportZipPath` and `setShowImportPasswordDialog` since those are called inside `proceedWithImport`).

**Update the call site:**

```typescript
    const handlers = createMenuHandlers(
      statusSetter,
      setShowNewWorkspace,
      setShowOpenWorkspace,
      setShowSettings,
      setImportState,
      setShowExportPasswordDialog,
      (zipPath) => proceedWithImport(zipPath, null),
      workspace,
    );
```

### Step 5: Update handleImportConfirm to thread the password through

In `handleImportConfirm`, update the `execute_import` call to pass the password:

```typescript
      await invoke('execute_import', { zipPath: importState.zipPath, dbPath, password: pendingImportPassword });
```

Also reset `pendingImportPassword` after import completes:
```typescript
      setPendingImportPassword(null);
```

### Step 6: Add the import password dialog JSX

Add after the export password dialog block in the return statement:

```tsx
      {/* Import password dialog */}
      {showImportPasswordDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">This archive is password-protected</h2>
            <p className="text-sm text-muted-foreground mb-4">
              Enter the password used when the workspace was exported.
            </p>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">Password</label>
              <input
                type="password"
                value={importPassword}
                onChange={(e) => setImportPassword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && importPassword && pendingImportZipPath) {
                    setShowImportPasswordDialog(false);
                    proceedWithImport(pendingImportZipPath, importPassword);
                  }
                }}
                placeholder="Enter password"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
              />
            </div>
            {importPasswordError && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {importPasswordError}
              </div>
            )}
            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setShowImportPasswordDialog(false);
                  setPendingImportZipPath(null);
                  setImportPassword('');
                  setImportPasswordError('');
                }}
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  if (!pendingImportZipPath) return;
                  setShowImportPasswordDialog(false);
                  proceedWithImport(pendingImportZipPath, importPassword);
                }}
                disabled={!importPassword}
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Decrypt
              </button>
            </div>
          </div>
        </div>
      )}
```

### Step 7: Build and manual end-to-end test

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build 2>&1
```

Then manually test the full flow:
1. Export with a password (Task 6 flow)
2. File → Import → select the encrypted zip
3. Verify "This archive is password-protected" dialog appears
4. Enter wrong password → verify "Incorrect password" error stays in dialog
5. Enter correct password → verify workspace-name dialog appears → import completes
6. Import a plain (unencrypted) zip → verify it goes straight to workspace-name dialog (no password dialog)

### Step 8: Commit

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-desktop/src/App.tsx
git -C /Users/careck/Source/Krillnotes commit -m "feat: add import password dialog for encrypted archives"
```

---

## Wrap-up

Run the full test suite one final time:
```bash
cargo test -p krillnotes-core 2>&1
```

All tests should pass. The feature is complete.
