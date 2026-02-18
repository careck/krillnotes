# Workspace Integration Design (Phase 2)

**Date:** 2026-02-18
**Status:** Approved
**Implements:** Phase 2 from CHECKPOINT.md

## Overview

Phase 2 adds workspace integration with multi-window support, file picker dialogs, and minimal workspace information display. Each window can open a separate workspace, with automatic handling of duplicate file opens.

**Goals:**
- Multi-window support (multiple workspaces simultaneously)
- Native file pickers for create/open workspace
- Window labels derived from workspace filenames
- Minimal workspace info UI (path, note count, status)
- Clean startup flow with welcome dialog

**Non-Goals (Future Phases):**
- Tree view for browsing notes (Phase 3)
- Note editing UI (Phase 4)
- Keyboard shortcuts or advanced UI

## Architecture Overview

### Multi-Window State Management

The app uses Tauri's managed state pattern with a global workspace map:

```rust
pub struct AppState {
    workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
}
```

**Key Concepts:**
- Each Tauri window has a unique label derived from the workspace filename
- When a workspace is opened/created, it's stored in the HashMap keyed by window label
- Commands receive the `Window` parameter to determine which workspace to operate on
- Window close events trigger cleanup (remove workspace from HashMap)

**Window Labeling Strategy:**
- Window labels are derived from workspace filename (e.g., `"my-notes.db"` → label = `"my-notes"`)
- If filename conflicts exist, append number: `"my-notes-2"`, `"my-notes-3"`
- Before opening a file, check if that exact path is already open
- If duplicate path detected, focus existing window instead of opening new one
- Window title is set to `"Krillnotes - {filename}"`

**Workspace Lifecycle:**
1. App starts → main window (label `"main"`, no workspace, shows welcome dialog)
2. User dismisses welcome → empty state shown
3. File > New/Open Workspace → file picker → create new window with filename-based label
4. First workspace closes the `"main"` window (replaced by workspace window)
5. Subsequent workspaces → create additional windows with unique labels
6. User closes window → cleanup event removes workspace from map

**Benefits:**
- Each window operates independently
- Fast operations (workspace stays in memory, SQLite on disk)
- Clean separation (window label as key)
- Prevents duplicate file opens
- User-friendly window titles

## Rust Backend (Tauri Commands)

### App State Structure

```rust
pub struct AppState {
    workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
}
```

### Core Tauri Commands

**create_workspace:**
```rust
#[tauri::command]
async fn create_workspace(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    // Validate path doesn't exist
    if path_buf.exists() {
        return Err("File already exists. Use Open Workspace instead.".to_string());
    }

    // Check if this path is already open
    match find_window_for_path(&state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &path_buf);
            let workspace = Workspace::create(&path_buf)
                .map_err(|e| format!("Failed to create: {}", e))?;

            let new_window = create_workspace_window(&app, &label)?;
            store_workspace(&state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {}", label))
                .map_err(|e| e.to_string())?;

            // Close main window if this is first workspace
            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}
```

**open_workspace:**
```rust
#[tauri::command]
async fn open_workspace(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<WorkspaceInfo, String> {
    let path_buf = PathBuf::from(&path);

    // Validate path exists
    if !path_buf.exists() {
        return Err("File does not exist".to_string());
    }

    // Check for duplicate open
    match find_window_for_path(&state, &path_buf) {
        Some(existing_label) => {
            focus_window(&app, &existing_label)?;
            Err("focused_existing".to_string())
        }
        None => {
            let label = generate_unique_label(&state, &path_buf);
            let workspace = Workspace::open(&path_buf)
                .map_err(|e| format!("Failed to open: {}", e))?;

            let new_window = create_workspace_window(&app, &label)?;
            store_workspace(&state, label.clone(), workspace, path_buf.clone());

            new_window.set_title(&format!("Krillnotes - {}", label))
                .map_err(|e| e.to_string())?;

            if window.label() == "main" {
                window.close().map_err(|e| e.to_string())?;
            }

            get_workspace_info_internal(&state, &label)
        }
    }
}
```

**get_workspace_info:**
```rust
#[tauri::command]
fn get_workspace_info(
    window: Window,
    state: State<'_, AppState>,
) -> Result<WorkspaceInfo, String> {
    get_workspace_info_internal(&state, window.label())
}
```

**list_notes (for future use):**
```rust
#[tauri::command]
fn list_notes(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Vec<Note>, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_all_notes()
        .map_err(|e| e.to_string())
}
```

### Helper Functions

**generate_unique_label:**
```rust
fn generate_unique_label(state: &AppState, path: &PathBuf) -> String {
    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");

    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");

    let mut label = filename.to_string();
    let mut counter = 2;

    while workspaces.contains_key(&label) {
        label = format!("{}-{}", filename, counter);
        counter += 1;
    }

    label
}
```

**find_window_for_path:**
```rust
fn find_window_for_path(state: &AppState, path: &PathBuf) -> Option<String> {
    state.workspace_paths.lock()
        .expect("Mutex poisoned")
        .iter()
        .find(|(_, p)| *p == path)
        .map(|(label, _)| label.clone())
}
```

**focus_window:**
```rust
fn focus_window(app: &AppHandle, label: &str) -> Result<(), String> {
    use tauri::Manager;

    app.get_webview_window(label)
        .ok_or_else(|| "Window not found".to_string())
        .and_then(|window| {
            window.set_focus()
                .map_err(|e| format!("Failed to focus: {}", e))
        })
}
```

**create_workspace_window:**
```rust
fn create_workspace_window(
    app: &AppHandle,
    label: &str
) -> Result<Window, String> {
    use tauri::Manager;

    tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App("index.html".into())
    )
    .title(&format!("Krillnotes - {}", label))
    .inner_size(1024.0, 768.0)
    .build()
    .map_err(|e| format!("Failed to create window: {}", e))
}
```

**store_workspace:**
```rust
fn store_workspace(
    state: &AppState,
    label: String,
    workspace: Workspace,
    path: PathBuf,
) {
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let mut paths = state.workspace_paths.lock()
        .expect("Mutex poisoned");

    workspaces.insert(label.clone(), workspace);
    paths.insert(label, path);
}
```

**get_workspace_info_internal:**
```rust
fn get_workspace_info_internal(
    state: &AppState,
    label: &str
) -> Result<WorkspaceInfo, String> {
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let paths = state.workspace_paths.lock()
        .expect("Mutex poisoned");

    let workspace = workspaces.get(label)
        .ok_or("No workspace found")?;
    let path = paths.get(label)
        .ok_or("No path found")?;

    let note_count = workspace.list_all_notes()
        .map(|notes| notes.len())
        .unwrap_or(0);

    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    Ok(WorkspaceInfo {
        filename,
        path: path.display().to_string(),
        note_count,
    })
}
```

### Return Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub filename: String,
    pub path: String,
    pub note_count: usize,
}
```

### Menu Handler (Declarative Mapping)

```rust
const MENU_MESSAGES: &[(&str, &str)] = &[
    ("file_new", "File > New Workspace clicked"),
    ("file_open", "File > Open Workspace clicked"),
    ("edit_add_note", "Edit > Add Note clicked"),
    ("edit_delete_note", "Edit > Delete Note clicked"),
    ("view_refresh", "View > Refresh clicked"),
    ("help_about", "Help > About Krillnotes clicked"),
];

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    MENU_MESSAGES.iter()
        .find(|(id, _)| id == &event.id().as_ref())
        .map(|(_, message)| app.emit("menu-action", message))
        .transpose()
        .ok();
}
```

## Frontend (React UI)

### Component Structure

```
src/
├── App.tsx                          # Root component, workspace state
├── types.ts                         # TypeScript type definitions
├── components/
│   ├── WorkspaceInfo.tsx           # Display workspace info
│   ├── WelcomeDialog.tsx           # Startup welcome dialog
│   ├── EmptyState.tsx              # "No workspace" placeholder
│   └── StatusMessage.tsx           # Status/error messages (existing)
└── styles/
    └── globals.css                  # Existing Tailwind styles
```

### App.tsx (Root Component)

```typescript
import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceInfo from './components/WorkspaceInfo';
import WelcomeDialog from './components/WelcomeDialog';
import EmptyState from './components/EmptyState';
import StatusMessage from './components/StatusMessage';
import type { WorkspaceInfo as WorkspaceInfoType } from './types';

const createMenuHandlers = (
  setWorkspace: (info: WorkspaceInfoType | null) => void,
  setStatus: (msg: string, isError?: boolean) => void
) => ({
  'File > New Workspace clicked': async () => {
    const path = await save({
      filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
      defaultPath: 'workspace.db',
      title: 'Create New Workspace'
    });

    if (!path) return;

    await invoke<WorkspaceInfoType>('create_workspace', { path })
      .then(info => {
        setWorkspace(info);
        setStatus(`Created: ${info.filename}`);
      })
      .catch(err => err !== 'focused_existing' && setStatus(`Error: ${err}`, true));
  },

  'File > Open Workspace clicked': async () => {
    const path = await open({
      filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
      multiple: false,
      title: 'Open Workspace'
    });

    if (!path || Array.isArray(path)) return;

    await invoke<WorkspaceInfoType>('open_workspace', { path })
      .then(info => {
        setWorkspace(info);
        setStatus(`Opened: ${info.filename}`);
      })
      .catch(err => err !== 'focused_existing' && setStatus(`Error: ${err}`, true));
  },
});

function App() {
  const [showWelcome, setShowWelcome] = useState(true);
  const [workspace, setWorkspace] = useState<WorkspaceInfoType | null>(null);
  const [status, setStatus] = useState('');
  const [isError, setIsError] = useState(false);

  useEffect(() => {
    const welcomed = localStorage.getItem('krillnotes_welcomed');
    if (welcomed === 'true') {
      setShowWelcome(false);
    }
  }, []);

  useEffect(() => {
    const handlers = createMenuHandlers(
      setWorkspace,
      (msg, error = false) => {
        setStatus(msg);
        setIsError(error);
        setTimeout(() => setStatus(''), 5000);
      }
    );

    const unlisten = listen<string>('menu-action', (event) =>
      handlers[event.payload as keyof typeof handlers]?.()
    );

    return () => { unlisten.then(f => f()); };
  }, []);

  const handleDismissWelcome = () => {
    localStorage.setItem('krillnotes_welcomed', 'true');
    setShowWelcome(false);
  };

  if (showWelcome) {
    return <WelcomeDialog onDismiss={handleDismissWelcome} />;
  }

  return (
    <div className="min-h-screen bg-background text-foreground p-8">
      {workspace ? <WorkspaceInfo info={workspace} /> : <EmptyState />}
      {status && <StatusMessage message={status} isError={isError} />}
    </div>
  );
}

export default App;
```

### WorkspaceInfo.tsx

```typescript
import type { WorkspaceInfo as WorkspaceInfoType } from '../types';

interface WorkspaceInfoProps {
  info: WorkspaceInfoType;
}

function WorkspaceInfo({ info }: WorkspaceInfoProps) {
  return (
    <div className="max-w-2xl mx-auto">
      <h1 className="text-4xl font-bold mb-2">{info.filename}</h1>
      <p className="text-muted-foreground mb-6">{info.path}</p>

      <div className="bg-secondary p-6 rounded-lg">
        <div className="grid grid-cols-2 gap-4">
          <div>
            <p className="text-sm text-muted-foreground">Notes</p>
            <p className="text-2xl font-semibold">{info.noteCount}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">Status</p>
            <p className="text-lg">Ready</p>
          </div>
        </div>
      </div>

      <p className="mt-6 text-sm text-muted-foreground">
        Phase 3 will add tree view for browsing notes
      </p>
    </div>
  );
}

export default WorkspaceInfo;
```

### WelcomeDialog.tsx

```typescript
interface WelcomeDialogProps {
  onDismiss: () => void;
}

function WelcomeDialog({ onDismiss }: WelcomeDialogProps) {
  return (
    <div className="min-h-screen bg-background text-foreground flex items-center justify-center">
      <div className="max-w-md bg-secondary p-8 rounded-lg text-center">
        <h1 className="text-3xl font-bold mb-4">Welcome to Krillnotes</h1>
        <p className="text-muted-foreground mb-6">
          You can start a new Workspace or load an existing one from the File menu
        </p>
        <button
          onClick={onDismiss}
          className="bg-primary text-primary-foreground px-6 py-2 rounded-md hover:bg-primary/90"
        >
          OK
        </button>
      </div>
    </div>
  );
}

export default WelcomeDialog;
```

### EmptyState.tsx

```typescript
function EmptyState() {
  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="text-center">
        <h1 className="text-4xl font-bold mb-4">Krillnotes</h1>
        <p className="text-muted-foreground">
          Use File &gt; New Workspace or File &gt; Open Workspace to get started
        </p>
      </div>
    </div>
  );
}

export default EmptyState;
```

### StatusMessage.tsx (Updated)

```typescript
interface StatusMessageProps {
  message: string;
  isError?: boolean;
}

function StatusMessage({ message, isError = false }: StatusMessageProps) {
  return (
    <div className={`mt-4 p-4 rounded-lg ${
      isError
        ? 'bg-red-500/10 border border-red-500/20 text-red-500'
        : 'bg-secondary'
    }`}>
      <p className="text-sm">{message}</p>
    </div>
  );
}

export default StatusMessage;
```

### types.ts

```typescript
export interface WorkspaceInfo {
  filename: string;
  path: string;
  noteCount: number;
}

export interface Note {
  id: string;
  title: string;
  nodeType: string;
  parentId: string | null;
  position: number;
  createdAt: number;
  modifiedAt: number;
  createdBy: number;
  modifiedBy: number;
  fields: Record<string, FieldValue>;
}

export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean };
```

## Window Management

### Window Lifecycle

```
1. App starts
   → Main window (label: "main")
   → Shows WelcomeDialog
   → User clicks OK → localStorage remembers, shows EmptyState

2. User clicks File > New Workspace (first time)
   → File picker: "/Users/alice/notes.db"
   → Generate label: "notes"
   → Create NEW window with label "notes"
   → Close "main" window
   → Window "notes" shows WorkspaceInfo

3. User clicks File > Open Workspace
   → File picker: "/Users/alice/work.db"
   → Generate label: "work"
   → Create NEW window with label "work"
   → Now have two windows: "notes" and "work"

4. User opens "/Users/alice/notes.db" again
   → find_window_for_path() finds "notes"
   → focus_window() brings "notes" to front
   → No new window created, no error shown

5. User opens "/Users/bob/notes.db" (different path, same filename)
   → Generate label: "notes-2" (avoid conflict)
   → Create new window with label "notes-2"
   → Now have three windows: "notes", "work", "notes-2"

6. User closes window "work"
   → CloseRequested event fires
   → cleanup_workspace() removes from maps
   → Workspace Drop closes SQLite connection
   → Label "work" now available for reuse
```

### Window Cleanup

```rust
// In lib.rs run() function
fn setup_window_cleanup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let state_clone = state.inner().clone();

    app.on_window_event(move |window, event| {
        match event {
            tauri::WindowEvent::CloseRequested { .. } |
            tauri::WindowEvent::Destroyed => {
                let label = window.label().to_string();
                cleanup_workspace(&state_clone, &label);
            }
            _ => {}
        }
    });

    Ok(())
}

fn cleanup_workspace(state: &AppState, label: &str) {
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let mut paths = state.workspace_paths.lock()
        .expect("Mutex poisoned");

    workspaces.remove(label);
    paths.remove(label);

    // Workspace Drop impl closes SQLite connection
}
```

## File Picker Integration & Error Handling

### File Picker Setup

```toml
# Cargo.toml (src-tauri/Cargo.toml)
[dependencies]
tauri-plugin-dialog = "2.0"
```

```rust
// lib.rs
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // ... rest
}
```

### Error Handling

| Error | Rust Response | Frontend Behavior |
|-------|---------------|-------------------|
| User cancels picker | `path = null` | No error, silent return |
| Duplicate file open | `Err("focused_existing")` | No error, focus existing window |
| File doesn't exist (open) | `Err("File does not exist")` | Show error in StatusMessage |
| File exists (create) | `Err("File already exists...")` | Show error in StatusMessage |
| Corrupt database | `Err("Failed to open...")` | Show error in StatusMessage |
| Window creation fails | `Err("Failed to create window...")` | Show error in StatusMessage |

**Error Display:**
- Errors shown in red StatusMessage component
- Auto-clear after 5 seconds
- "focused_existing" is not shown to user (silent focus)
- User cancellation is silent (no error message)

## Missing Core Functionality

### Add to krillnotes-core

**Workspace::open()** in `workspace.rs`:
```rust
impl Workspace {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut storage = Storage::open(&path)?;
        let registry = SchemaRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 1000 });

        let device_id = storage.connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'device_id'",
                [],
                |row| row.get::<_, String>(0)
            )?;

        let current_user_id = storage.connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'current_user_id'",
                [],
                |row| row.get::<_, String>(0)
            )?
            .parse::<i64>()
            .unwrap_or(0);

        Ok(Self {
            storage,
            registry,
            operation_log,
            device_id,
            current_user_id,
        })
    }
}
```

**Storage::open()** in `storage.rs`:
```rust
impl Storage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Validate database structure
        let table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table'
             AND name IN ('notes', 'operations', 'workspace_meta')",
            [],
            |row| row.get(0)
        )?;

        if table_count != 3 {
            return Err(crate::KrillnotesError::InvalidWorkspace(
                "Not a valid Krillnotes database".to_string()
            ));
        }

        Ok(Self { conn })
    }
}
```

## Implementation Summary

### Changes Required

**1. krillnotes-core (Rust library):**
- Add `Workspace::open()` method
- Add `Storage::open()` with validation
- Ensure `Storage::connection_mut()` exists (already implemented)

**2. krillnotes-desktop/src-tauri (Rust):**
- Add `AppState` struct with workspace maps
- Implement commands: `create_workspace`, `open_workspace`, `get_workspace_info`, `list_notes`
- Add helper functions: `generate_unique_label`, `find_window_for_path`, `focus_window`, etc.
- Add `WorkspaceInfo` serializable struct
- Update menu handler with declarative mapping
- Add window cleanup listener in `setup()`
- Update `invoke_handler` with new commands

**3. krillnotes-desktop/src (TypeScript/React):**
- Create `types.ts` with TypeScript interfaces
- Create `WorkspaceInfo.tsx` component
- Create `WelcomeDialog.tsx` component
- Create `EmptyState.tsx` component
- Update `StatusMessage.tsx` with error styling
- Update `App.tsx` with menu handlers and state management
- Add localStorage for welcome dialog dismissal

### File Changes

**New Files:**
- `krillnotes-desktop/src/types.ts`
- `krillnotes-desktop/src/components/WorkspaceInfo.tsx`
- `krillnotes-desktop/src/components/WelcomeDialog.tsx`
- `krillnotes-desktop/src/components/EmptyState.tsx`

**Modified Files:**
- `krillnotes-core/src/core/workspace.rs` (add `open()`)
- `krillnotes-core/src/core/storage.rs` (add `open()`)
- `krillnotes-desktop/src-tauri/src/lib.rs` (add commands and state)
- `krillnotes-desktop/src-tauri/Cargo.toml` (add `tauri-plugin-dialog`)
- `krillnotes-desktop/src/App.tsx` (add workspace logic)
- `krillnotes-desktop/src/components/StatusMessage.tsx` (add error styling)

### Testing Strategy

**Manual Testing:**
1. App startup → verify welcome dialog appears
2. Dismiss welcome → verify localStorage persists
3. Create first workspace → verify main window replaced
4. Create second workspace → verify new window created
5. Open same file twice → verify focus existing window
6. Open different file with same name → verify unique label generated
7. Close workspace window → verify cleanup (check with subsequent open)
8. Invalid file open → verify error shown

**Future Automated Tests:**
- Unit tests for `generate_unique_label()`
- Unit tests for `find_window_for_path()`
- Integration tests for workspace CRUD operations
- E2E tests for multi-window scenarios (Tauri WebDriver)

## Decision Log

**Q: Why global state map instead of per-window state?**
A: Tauri's per-window state APIs are less mature. Global map with window labels is a proven pattern and provides centralized state management.

**Q: Why close main window on first workspace open?**
A: Cleaner UX - user thinks of "opening a workspace" not "replacing welcome screen". Subsequent workspaces naturally create new windows.

**Q: Why filename-based labels instead of arbitrary labels?**
A: Better UX (window title matches content), prevents duplicate opens naturally, easier debugging.

**Q: Why focus existing window instead of showing error on duplicate open?**
A: Better UX - user gets what they want (the workspace) without interruption. Matches behavior of modern apps.

**Q: Why localStorage for welcome dialog instead of settings file?**
A: Simple, fast, browser-native. Settings system can be added in Phase 4+ if needed.

---

**Next Step:** Create implementation plan with detailed tasks
