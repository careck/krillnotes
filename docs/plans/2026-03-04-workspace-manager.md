# Workspace Manager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the minimal `OpenWorkspaceDialog` with a full `WorkspaceManagerDialog` supporting Info, Open, Delete, and Duplicate — with metadata cached in an unencrypted `info.json` sidecar so no password is needed for basic info.

**Architecture:** A new `write_info_json()` method on `Workspace` writes `{ created_at, note_count, attachment_count }` to `<workspace_root>/info.json` on create/open and on window close. `list_workspace_files` reads `info.json` alongside filesystem stats so all metadata is available in one call. Two new Tauri commands (`delete_workspace`, `duplicate_workspace`) handle the destructive operations. The frontend replaces `OpenWorkspaceDialog.tsx` with `WorkspaceManagerDialog.tsx`.

**Tech Stack:** Rust (thiserror, serde_json, tempfile), Tauri v2, React 19, TypeScript, Tailwind v4

---

### Task 1: Add `write_info_json` to `krillnotes-core`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write the failing test**

Add to the `#[cfg(test)]` block at the bottom of `workspace.rs`:

```rust
#[test]
fn test_write_info_json_creates_file() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "").unwrap();
    ws.write_info_json().unwrap();

    let info_path = dir.path().join("info.json");
    assert!(info_path.exists(), "info.json should be created");

    let content = std::fs::read_to_string(&info_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(v["created_at"].is_number());
    assert_eq!(v["note_count"].as_u64().unwrap(), 0); // root excluded
    assert_eq!(v["attachment_count"].as_u64().unwrap(), 0);
}

#[test]
fn test_write_info_json_counts_notes() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let mut ws = Workspace::create(&db_path, "").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    ws.create_note(&root.id, crate::core::workspace::AddPosition::AsChild, "TextNote").unwrap();
    ws.create_note(&root.id, crate::core::workspace::AddPosition::AsChild, "TextNote").unwrap();
    ws.write_info_json().unwrap();

    let content = std::fs::read_to_string(dir.path().join("info.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(v["note_count"].as_u64().unwrap(), 2);
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_write_info_json
```
Expected: FAIL — `write_info_json` method not found.

**Step 3: Add `WorkspaceInfo` struct and `write_info_json` method**

In `workspace.rs`, find the `impl Workspace` block and add after the `workspace_root()` method (around line 365):

```rust
/// Writes `info.json` to the workspace root with cached metadata.
/// Called on open, create, and window close so the workspace manager
/// can display counts without opening the encrypted database.
pub fn write_info_json(&self) -> Result<()> {
    let note_count: i64 = self.connection()
        .query_row(
            "SELECT COUNT(*) FROM notes WHERE parent_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let attachment_count: i64 = self.connection()
        .query_row("SELECT COUNT(*) FROM attachments", [], |row| row.get(0))
        .unwrap_or(0);

    // created_at = root note's created_at (best proxy for workspace age)
    let created_at: i64 = self.connection()
        .query_row(
            "SELECT created_at FROM notes WHERE parent_id IS NULL LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| chrono::Utc::now().timestamp());

    let info = serde_json::json!({
        "created_at": created_at,
        "note_count": note_count,
        "attachment_count": attachment_count,
    });

    let path = self.workspace_root().join("info.json");
    std::fs::write(&path, serde_json::to_string(&info).map_err(|e| {
        KrillnotesError::Other(format!("Failed to serialise info.json: {e}"))
    })?)
    .map_err(|e| KrillnotesError::Other(format!("Failed to write info.json: {e}")))?;

    Ok(())
}
```

**Step 4: Run tests to verify they pass**

```bash
cargo test -p krillnotes-core test_write_info_json
```
Expected: 2 tests PASS.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add write_info_json to cache workspace metadata"
```

---

### Task 2: Call `write_info_json` on create and open

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_info_json_written_on_create() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    Workspace::create(&db_path, "").unwrap();
    assert!(dir.path().join("info.json").exists(), "info.json must exist after create");
}

#[test]
fn test_info_json_written_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    Workspace::create(&db_path, "").unwrap();
    std::fs::remove_file(dir.path().join("info.json")).unwrap(); // remove it
    Workspace::open(&db_path, "").unwrap();
    assert!(dir.path().join("info.json").exists(), "info.json must be rewritten on open");
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_info_json_written
```
Expected: FAIL.

**Step 3: Call `write_info_json` at end of `create()` and `open()`**

In `Workspace::create()` (around line 88), just before `Ok(workspace)`:
```rust
    let _ = workspace.write_info_json(); // best-effort; non-fatal
    Ok(workspace)
```

In `Workspace::open()` (around line 256), just before `Ok(workspace)`:
```rust
    let _ = workspace.write_info_json(); // best-effort; non-fatal
    Ok(workspace)
```

**Step 4: Run tests**

```bash
cargo test -p krillnotes-core test_info_json_written
```
Expected: 2 tests PASS.

**Step 5: Run full suite**

```bash
cargo test -p krillnotes-core
```
Expected: all existing tests still pass.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): write info.json on workspace create and open"
```

---

### Task 3: Write `info.json` on window close

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Find the `WindowEvent::Destroyed` handler** (~line 1885). It currently removes the workspace from AppState. Add a `write_info_json` call **before** removing:

```rust
tauri::WindowEvent::Destroyed => {
    // Persist cached metadata before dropping the workspace.
    if let Some(ws) = state.workspaces.lock().expect("Mutex poisoned").get(&label) {
        let _ = ws.write_info_json();
    }

    state.workspaces.lock().expect("Mutex poisoned").remove(&label);
    state.workspace_paths.lock().expect("Mutex poisoned").remove(&label);
    // ... rest of existing code unchanged
```

**Step 2: Build to verify it compiles**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "^error"
```
Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: write info.json on workspace window close"
```

---

### Task 4: Extend `WorkspaceEntry` and `list_workspace_files`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Update `WorkspaceEntry` struct** (around line 1564):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    name: String,
    path: String,
    is_open: bool,
    /// Unix timestamp of the workspace folder's last modification (mtime).
    last_modified: i64,
    /// Total size in bytes: notes.db + attachments/ directory.
    size_bytes: u64,
    /// From info.json: root note's created_at. None if info.json is missing.
    created_at: Option<i64>,
    /// From info.json: number of notes excluding root. None if info.json is missing.
    note_count: Option<usize>,
    /// From info.json: number of attachments. None if info.json is missing.
    attachment_count: Option<usize>,
}
```

**Step 2: Add helper functions** above `list_workspace_files`:

```rust
/// Returns the total size in bytes of all files directly inside `dir`,
/// including a recursive sum of the `attachments/` subdirectory.
fn dir_size_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                total += dir_size_bytes(&p);
            }
        }
    }
    total
}

/// Reads `info.json` from `workspace_dir` and returns the three optional fields.
fn read_info_json(workspace_dir: &Path) -> (Option<i64>, Option<usize>, Option<usize>) {
    let path = workspace_dir.join("info.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (None, None, None),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };
    let created_at = v["created_at"].as_i64();
    let note_count = v["note_count"].as_u64().map(|n| n as usize);
    let attachment_count = v["attachment_count"].as_u64().map(|n| n as usize);
    (created_at, note_count, attachment_count)
}
```

**Step 3: Update the entry construction in `list_workspace_files`**:

Replace the existing `entries.push(WorkspaceEntry { ... })` block with:

```rust
let is_open = open_paths.iter().any(|p| *p == folder);
let last_modified = std::fs::metadata(&folder)
    .and_then(|m| m.modified())
    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64)
    .unwrap_or(0);
let size_bytes = dir_size_bytes(&folder);
let (created_at, note_count, attachment_count) = read_info_json(&folder);

entries.push(WorkspaceEntry {
    name: name.to_string(),
    path: folder.display().to_string(),
    is_open,
    last_modified,
    size_bytes,
    created_at,
    note_count,
    attachment_count,
});
```

**Step 4: Update `WorkspaceEntry` in `types.ts`**:

```typescript
export interface WorkspaceEntry {
  name: string;
  path: string;
  isOpen: boolean;
  lastModified: number;       // Unix timestamp (seconds)
  sizeBytes: number;
  createdAt: number | null;
  noteCount: number | null;
  attachmentCount: number | null;
}
```

**Step 5: Build to verify**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "^error"
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src/types.ts
git commit -m "feat: extend WorkspaceEntry with metadata from filesystem and info.json"
```

---

### Task 5: Add `delete_workspace` Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add the command** (near the other workspace commands, around line 1579):

```rust
/// Permanently deletes a workspace folder and all its contents.
/// Returns an error if the workspace is currently open.
#[tauri::command]
fn delete_workspace(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<(), String> {
    let folder = PathBuf::from(&path);

    // Refuse to delete an open workspace.
    let is_open = state
        .workspace_paths
        .lock()
        .expect("Mutex poisoned")
        .values()
        .any(|p| *p == folder);

    if is_open {
        return Err("Close the workspace before deleting it.".to_string());
    }

    std::fs::remove_dir_all(&folder)
        .map_err(|e| format!("Failed to delete workspace: {e}"))
}
```

**Step 2: Register the command** in `tauri::generate_handler![...]` (around line 1983 — find the closing `]` and add `delete_workspace` to the list):

```rust
tauri::generate_handler![
    // ... existing commands ...
    delete_workspace,
]
```

**Step 3: Build to verify**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "^error"
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add delete_workspace Tauri command"
```

---

### Task 6: Add `duplicate_workspace` Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add the command** near `delete_workspace`:

```rust
/// Duplicates a workspace by exporting it to a temp file and importing it
/// under a new name. Does NOT open the duplicated workspace in a window.
#[tauri::command]
fn duplicate_workspace(
    state: State<'_, AppState>,
    source_path: String,
    source_password: String,
    new_name: String,
    new_password: String,
) -> std::result::Result<(), String> {
    let app_settings = settings::load_settings();
    let workspace_dir = PathBuf::from(&app_settings.workspace_directory);
    let dest_folder = workspace_dir.join(&new_name);

    if dest_folder.exists() {
        return Err(format!("A workspace named '{new_name}' already exists."));
    }

    // Open the source workspace (validates password).
    let source_db = PathBuf::from(&source_path).join("notes.db");
    let workspace = Workspace::open(&source_db, &source_password)
        .map_err(|e| e.to_string())?;

    // Export to a temp file.
    let mut tmp = tempfile::NamedTempFile::new()
        .map_err(|e| format!("Failed to create temp file: {e}"))?;
    export_workspace(&workspace, &mut tmp, Some(&source_password))
        .map_err(|e| e.to_string())?;

    // Rewind and import to dest.
    std::fs::create_dir_all(&dest_folder)
        .map_err(|e| format!("Failed to create destination: {e}"))?;
    let dest_db = dest_folder.join("notes.db");

    tmp.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Seek failed: {e}"))?;
    import_workspace(tmp, &dest_db, Some(&source_password), &new_password)
        .map_err(|e| e.to_string())?;

    // Write info.json for the new workspace (best-effort).
    if let Ok(new_ws) = Workspace::open(&dest_db, &new_password) {
        let _ = new_ws.write_info_json();
    }

    Ok(())
}
```

Note: `tempfile` is already a dependency in `krillnotes-desktop/src-tauri/Cargo.toml`. Check with:
```bash
grep "tempfile" krillnotes-desktop/src-tauri/Cargo.toml
```
If missing, add: `tempfile = "3"` to `[dependencies]`.

Also ensure `use std::io::Seek;` is imported at the top of `lib.rs` (check with `grep "use std::io" src-tauri/src/lib.rs`). Add if not present.

**Step 2: Register the command** in `tauri::generate_handler![...]`:

```rust
tauri::generate_handler![
    // ... existing commands ...
    delete_workspace,
    duplicate_workspace,
]
```

**Step 3: Build to verify**

```bash
cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "^error"
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/Cargo.toml
git commit -m "feat: add duplicate_workspace Tauri command"
```

---

### Task 7: Create `WorkspaceManagerDialog.tsx`

**Files:**
- Create: `krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx`
- Delete: `krillnotes-desktop/src/components/OpenWorkspaceDialog.tsx` (after wiring is complete in Task 8)

**Step 1: Create the component**

```tsx
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { WorkspaceEntry, WorkspaceInfo } from '../types';
import EnterPasswordDialog from './EnterPasswordDialog';

interface WorkspaceManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

type SortKey = 'name' | 'modified';
type View = 'list' | 'password' | 'duplicate' | 'delete-confirm';

function formatDate(ts: number | null): string {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleDateString();
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function WorkspaceManagerDialog({ isOpen, onClose }: WorkspaceManagerDialogProps) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<WorkspaceEntry[]>([]);
  const [selected, setSelected] = useState<WorkspaceEntry | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>('name');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [view, setView] = useState<View>('list');
  const [passwordError, setPasswordError] = useState('');
  const [passwordAction, setPasswordAction] = useState<'open' | 'duplicate'>('open');
  const [dupName, setDupName] = useState('');
  const [dupPassword, setDupPassword] = useState('');
  const [dupPasswordConfirm, setDupPasswordConfirm] = useState('');
  const [dupError, setDupError] = useState('');
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    setError('');
    invoke<WorkspaceEntry[]>('list_workspace_files')
      .then(list => {
        setEntries(list);
        // Keep selection if still present
        setSelected(prev => prev ? (list.find(e => e.path === prev.path) ?? null) : null);
      })
      .catch(err => setError(String(err)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (isOpen) {
      setView('list');
      setSelected(null);
      setSortKey('name');
      setError('');
      load();
    }
  }, [isOpen, load]);

  const sorted = [...entries].sort((a, b) => {
    if (sortKey === 'name') return a.name.localeCompare(b.name);
    return b.lastModified - a.lastModified;
  });

  // ── Open action ────────────────────────────────────────────────
  const handleOpen = async (entry: WorkspaceEntry, password: string) => {
    setBusy(true);
    setPasswordError('');
    try {
      await invoke<WorkspaceInfo>('open_workspace', { path: entry.path, password });
      onClose();
    } catch (err) {
      setPasswordError(String(err));
      setBusy(false);
    }
  };

  const handleOpenClick = async () => {
    if (!selected || selected.isOpen) return;
    try {
      const cached = await invoke<string | null>('get_cached_password', { path: selected.path });
      if (cached) {
        await handleOpen(selected, cached);
        return;
      }
    } catch { /* fall through */ }
    setPasswordAction('open');
    setView('password');
  };

  // ── Delete action ──────────────────────────────────────────────
  const handleDeleteConfirm = async () => {
    if (!selected) return;
    setBusy(true);
    setError('');
    try {
      await invoke('delete_workspace', { path: selected.path });
      setSelected(null);
      setView('list');
      load();
    } catch (err) {
      setError(String(err));
      setView('list');
    } finally {
      setBusy(false);
    }
  };

  // ── Duplicate action ───────────────────────────────────────────
  const handleDuplicateSubmit = async (sourcePassword: string) => {
    if (!selected) return;
    if (dupPassword !== dupPasswordConfirm) {
      setDupError('Passwords do not match.');
      return;
    }
    setBusy(true);
    setDupError('');
    try {
      await invoke('duplicate_workspace', {
        sourcePath: selected.path,
        sourcePassword,
        newName: dupName,
        newPassword: dupPassword,
      });
      setView('list');
      load();
    } catch (err) {
      setDupError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleDuplicateClick = async () => {
    if (!selected) return;
    setDupName(`Copy of ${selected.name}`);
    setDupPassword('');
    setDupPasswordConfirm('');
    setDupError('');
    // Try cached password; if not available, ask for it via the password view
    try {
      const cached = await invoke<string | null>('get_cached_password', { path: selected.path });
      if (cached !== null) {
        // We have the source password — go straight to the duplicate form
        // Store it temporarily in dupPasswordConfirm slot? No — use a separate state.
        // We'll request it in the form instead (simpler, avoids stale state).
      }
    } catch { /* ignore */ }
    setView('duplicate');
  };

  // ── New workspace ──────────────────────────────────────────────
  const handleNew = async () => {
    try {
      await invoke('create_workspace_dialog');
    } catch { /* handled by App.tsx / menu */ }
    onClose();
  };

  if (!isOpen) return null;

  // Password sub-view
  if (view === 'password') {
    return (
      <EnterPasswordDialog
        isOpen={true}
        workspaceName={selected?.name ?? ''}
        error={passwordError}
        onConfirm={async (pw) => {
          if (passwordAction === 'open' && selected) await handleOpen(selected, pw);
        }}
        onCancel={() => { setView('list'); setPasswordError(''); }}
      />
    );
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary rounded-lg w-[560px] max-h-[70vh] flex flex-col">

        {/* Header */}
        <div className="px-6 pt-6 pb-3 flex items-center justify-between">
          <h2 className="text-xl font-bold">{t('workspaceManager.title', 'Workspaces')}</h2>
          <div className="flex gap-2 text-sm">
            <button
              onClick={() => setSortKey('name')}
              className={`px-2 py-1 rounded ${sortKey === 'name' ? 'bg-secondary' : 'hover:bg-secondary/50'}`}
            >
              {t('workspaceManager.sortName', 'Name')}
            </button>
            <button
              onClick={() => setSortKey('modified')}
              className={`px-2 py-1 rounded ${sortKey === 'modified' ? 'bg-secondary' : 'hover:bg-secondary/50'}`}
            >
              {t('workspaceManager.sortModified', 'Modified')}
            </button>
          </div>
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto px-6 min-h-0">
          {loading ? (
            <p className="text-muted-foreground text-center py-8">{t('workspace.loading')}</p>
          ) : sorted.length === 0 ? (
            <p className="text-muted-foreground text-center py-8">{t('workspace.noWorkspaces')}</p>
          ) : (
            <div className="space-y-0.5">
              {sorted.map(entry => (
                <button
                  key={entry.path}
                  onClick={() => { setSelected(entry); setView('list'); }}
                  className={`w-full text-left px-3 py-2 rounded-md flex items-center justify-between text-sm ${
                    selected?.path === entry.path ? 'bg-secondary' : 'hover:bg-secondary/50'
                  }`}
                >
                  <span className="font-medium truncate flex-1">{entry.name}</span>
                  <span className="text-muted-foreground ml-4 shrink-0">
                    {formatDate(entry.lastModified)}
                  </span>
                  <span className="text-muted-foreground ml-4 w-16 text-right shrink-0">
                    {formatSize(entry.sizeBytes)}
                  </span>
                  {entry.isOpen && (
                    <span className="text-xs text-muted-foreground ml-3 shrink-0">{t('workspace.alreadyOpen')}</span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Info panel (when selected and not in an action view) */}
        {selected && view === 'list' && (
          <div className="px-6 py-3 border-t border-secondary text-sm grid grid-cols-2 gap-x-6 gap-y-1 text-muted-foreground">
            <span>{t('workspaceManager.created', 'Created')}: <span className="text-foreground">{formatDate(selected.createdAt)}</span></span>
            <span>{t('workspaceManager.modified', 'Modified')}: <span className="text-foreground">{formatDate(selected.lastModified)}</span></span>
            <span>{t('workspaceManager.notes', 'Notes')}: <span className="text-foreground">{selected.noteCount ?? '—'}</span></span>
            <span>{t('workspaceManager.attachments', 'Attachments')}: <span className="text-foreground">{selected.attachmentCount ?? '—'}</span></span>
            <span>{t('workspaceManager.size', 'Size')}: <span className="text-foreground">{formatSize(selected.sizeBytes)}</span></span>
          </div>
        )}

        {/* Delete confirmation banner */}
        {view === 'delete-confirm' && selected && (
          <div className="px-6 py-4 border-t border-red-500/30 bg-red-500/10">
            <p className="text-red-500 font-semibold mb-1">
              ⚠ {t('workspaceManager.deleteWarningTitle', 'This cannot be undone.')}
            </p>
            <p className="text-sm text-muted-foreground mb-3">
              {t('workspaceManager.deleteWarningBody', `Permanently delete "${selected.name}" and all its notes and attachments?`, { name: selected.name })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setView('list')}
                className="px-3 py-1.5 border border-secondary rounded text-sm hover:bg-secondary"
                disabled={busy}
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={handleDeleteConfirm}
                className="px-3 py-1.5 bg-red-600 hover:bg-red-700 text-white rounded text-sm"
                disabled={busy}
              >
                {busy ? t('workspaceManager.deleting', 'Deleting…') : t('workspaceManager.deleteForever', 'Delete forever')}
              </button>
            </div>
          </div>
        )}

        {/* Duplicate form */}
        {view === 'duplicate' && selected && (
          <div className="px-6 py-4 border-t border-secondary space-y-3">
            <p className="text-sm font-medium">{t('workspaceManager.duplicateTitle', 'Duplicate workspace')}</p>
            <div>
              <label className="block text-xs text-muted-foreground mb-1">{t('workspaceManager.newName', 'New name')}</label>
              <input
                type="text"
                value={dupName}
                onChange={e => setDupName(e.target.value)}
                className="w-full px-3 py-1.5 border border-secondary rounded text-sm bg-background"
                autoFocus
              />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="block text-xs text-muted-foreground mb-1">{t('workspaceManager.sourcePassword', 'Source password')}</label>
                <input
                  type="password"
                  value={dupPassword}
                  onChange={e => setDupPassword(e.target.value)}
                  placeholder={t('dialogs.password.optionalPlaceholder')}
                  className="w-full px-3 py-1.5 border border-secondary rounded text-sm bg-background"
                />
              </div>
              <div>
                <label className="block text-xs text-muted-foreground mb-1">{t('workspaceManager.newPassword', 'New password (optional)')}</label>
                <input
                  type="password"
                  value={dupPasswordConfirm}
                  onChange={e => setDupPasswordConfirm(e.target.value)}
                  placeholder={t('dialogs.password.optionalPlaceholder')}
                  className="w-full px-3 py-1.5 border border-secondary rounded text-sm bg-background"
                />
              </div>
            </div>
            {dupError && <p className="text-red-500 text-xs">{dupError}</p>}
            <div className="flex gap-2">
              <button onClick={() => setView('list')} className="px-3 py-1.5 border border-secondary rounded text-sm hover:bg-secondary" disabled={busy}>
                {t('common.cancel')}
              </button>
              <button
                onClick={() => handleDuplicateSubmit(dupPassword)}
                className="px-3 py-1.5 bg-primary text-primary-foreground rounded text-sm hover:opacity-90"
                disabled={busy || !dupName.trim()}
              >
                {busy ? t('workspaceManager.duplicating', 'Duplicating…') : t('workspaceManager.duplicate', 'Duplicate')}
              </button>
            </div>
          </div>
        )}

        {/* Error */}
        {error && (
          <div className="px-6 pt-2">
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {error}
            </div>
          </div>
        )}

        {/* Action toolbar */}
        {view === 'list' && (
          <div className="px-6 py-3 border-t border-secondary flex gap-2">
            <button
              onClick={handleOpenClick}
              disabled={!selected || selected.isOpen || busy}
              className="px-3 py-1.5 border border-secondary rounded text-sm hover:bg-secondary disabled:opacity-40"
            >
              {t('workspace.open', 'Open')}
            </button>
            <button
              onClick={handleDuplicateClick}
              disabled={!selected || busy}
              className="px-3 py-1.5 border border-secondary rounded text-sm hover:bg-secondary disabled:opacity-40"
            >
              {t('workspaceManager.duplicate', 'Duplicate')}
            </button>
            <button
              onClick={() => setView('delete-confirm')}
              disabled={!selected || selected.isOpen || busy}
              className="px-3 py-1.5 border border-red-500/40 text-red-500 rounded text-sm hover:bg-red-500/10 disabled:opacity-40"
            >
              {t('common.delete', 'Delete')}
            </button>
          </div>
        )}

        {/* Footer */}
        <div className="px-6 py-4 border-t border-secondary flex justify-between">
          <button
            onClick={handleNew}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
            disabled={busy}
          >
            {t('workspace.new', 'New')}
          </button>
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
            disabled={busy}
          >
            {t('common.close', 'Close')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default WorkspaceManagerDialog;
```

**Note on Duplicate password fields:** The form uses `dupPassword` for the *source* password and `dupPasswordConfirm` for the *new* workspace password — the variable names are confusing. Rename them to `sourcePassword` / `newPassword` / `newPasswordConfirm` for clarity while implementing.

**Step 2: TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx
git commit -m "feat: add WorkspaceManagerDialog component"
```

---

### Task 8: Wire `WorkspaceManagerDialog` into `App.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Find all references to `OpenWorkspaceDialog`**

```bash
grep -rn "OpenWorkspaceDialog" krillnotes-desktop/src/
```

**Step 2: Replace import and usage**

In `App.tsx`:
1. Replace: `import OpenWorkspaceDialog from './components/OpenWorkspaceDialog';`
   With: `import WorkspaceManagerDialog from './components/WorkspaceManagerDialog';`

2. Replace every `<OpenWorkspaceDialog` with `<WorkspaceManagerDialog`

**Step 3: TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 4: Delete the old dialog**

```bash
rm krillnotes-desktop/src/components/OpenWorkspaceDialog.tsx
```

**Step 5: TypeScript check again (confirms nothing else imports the old file)**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git rm krillnotes-desktop/src/components/OpenWorkspaceDialog.tsx
git commit -m "feat: replace OpenWorkspaceDialog with WorkspaceManagerDialog"
```

---

### Task 9: Fix duplicate password fields and i18n keys

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx`
- Modify: `krillnotes-desktop/src/i18n/locales/en.json` (and the other 6 locale files)

**Step 1: Clean up variable naming in duplicate form**

In `WorkspaceManagerDialog.tsx`, rename:
- `dupPassword` → `sourcePassword`
- `dupPasswordConfirm` → `newPassword`
- Add a third field `newPasswordConfirm` for new password confirmation

Update `handleDuplicateSubmit` accordingly:
```tsx
const handleDuplicateSubmit = async () => {
  if (newPassword !== newPasswordConfirm) {
    setDupError('Passwords do not match.');
    return;
  }
  // ...
  await invoke('duplicate_workspace', {
    sourcePath: selected.path,
    sourcePassword,
    newName: dupName,
    newPassword,
  });
};
```

**Step 2: Add i18n keys to `en.json`**

Find the translation file:
```bash
ls krillnotes-desktop/src/i18n/locales/
```

Add to `en.json` under a `"workspaceManager"` key:
```json
"workspaceManager": {
  "title": "Workspaces",
  "sortName": "Name",
  "sortModified": "Modified",
  "created": "Created",
  "modified": "Modified",
  "notes": "Notes",
  "attachments": "Attachments",
  "size": "Size",
  "open": "Open",
  "duplicate": "Duplicate",
  "duplicating": "Duplicating…",
  "duplicateTitle": "Duplicate workspace",
  "newName": "New name",
  "sourcePassword": "Source password",
  "newPassword": "New password (optional)",
  "newPasswordConfirm": "Confirm new password",
  "deleteWarningTitle": "This cannot be undone.",
  "deleteWarningBody": "Permanently delete \"{{name}}\" and all its notes and attachments?",
  "deleting": "Deleting…",
  "deleteForever": "Delete forever"
}
```

Add the same keys (untranslated, same English value) to all other locale files: `de.json`, `es.json`, `fr.json`, `it.json`, `ja.json`, `zh.json`. (Translations can be added later — having the keys prevents missing-key warnings.)

**Step 3: TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx krillnotes-desktop/src/i18n/
git commit -m "feat: fix duplicate form fields and add i18n keys for workspace manager"
```

---

### Task 10: Smoke test and PR

**Step 1: Start dev server**

```bash
cd krillnotes-desktop && npm run tauri dev
```

**Step 2: Manual smoke test checklist**

- [ ] Open Workspace Manager — list loads with name, modified date, size
- [ ] Selecting a workspace shows info panel (created, modified, notes, attachments, size)
- [ ] Workspaces without `info.json` show "—" for counts (test by manually deleting one)
- [ ] Sort by Name / Modified works
- [ ] Open button opens a workspace (password prompt if needed)
- [ ] Open button is disabled for already-open workspaces
- [ ] Delete button disabled for open workspaces
- [ ] Delete confirmation banner appears; Cancel returns to list; Delete forever removes folder and refreshes list
- [ ] Duplicate creates a new workspace visible in the list; can be opened with new password
- [ ] New button works

**Step 3: Final build check**

```bash
cargo test -p krillnotes-core
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: all tests pass, no TypeScript errors.

**Step 4: Push and open PR**

```bash
git push -u github-https feat/workspace-manager
gh pr create --title "feat: Workspace Manager with info, delete and duplicate (closes #65)" \
  --body "$(cat <<'EOF'
## Summary
- Replaces `OpenWorkspaceDialog` with a full `WorkspaceManagerDialog`
- Workspace list shows name, last-modified date, and size on disk
- Selecting a workspace shows info panel: created date, note count, attachment count, size
- Metadata cached in `info.json` sidecar — no password needed to view info
- Open / Delete / Duplicate actions with appropriate guards and confirmations
- Delete shows big red irreversible warning; refuses to delete open workspaces
- Duplicate uses the existing export → import pipeline; prompts for new name and password

## Test plan
- [ ] List loads correctly with all metadata columns
- [ ] Info panel shows correct counts (verify against a known workspace)
- [ ] Sort by name and modified date works
- [ ] Open action works with cached and uncached passwords
- [ ] Delete action blocked for open workspaces; works for closed workspaces
- [ ] Duplicate creates a valid openable workspace with the new password
- [ ] All Rust unit tests pass
- [ ] TypeScript builds clean
EOF
)"
```
