# Workspace Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement multi-window workspace integration with file pickers and minimal workspace info UI

**Architecture:** Global AppState map with filename-based window labels, native file pickers, welcome dialog startup flow

**Tech Stack:** Rust (Tauri v2, rusqlite), TypeScript/React, Tauri dialog plugin

---

## Task 1: Add Storage::open() Method

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs`
- Test: existing tests will verify

**Step 1: Write test for opening existing database**

Add to `krillnotes-core/src/core/storage.rs` tests section:

```rust
#[test]
fn test_open_existing_storage() {
    let temp = NamedTempFile::new().unwrap();

    // Create database first
    Storage::create(temp.path()).unwrap();

    // Open it
    let storage = Storage::open(temp.path()).unwrap();

    // Verify tables exist
    let tables: Vec<String> = storage
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(tables.contains(&"notes".to_string()));
    assert!(tables.contains(&"operations".to_string()));
    assert!(tables.contains(&"workspace_meta".to_string()));
}

#[test]
fn test_open_invalid_database() {
    let temp = NamedTempFile::new().unwrap();

    // Create empty file (not a valid Krillnotes DB)
    std::fs::write(temp.path(), "not a database").unwrap();

    let result = Storage::open(temp.path());
    assert!(result.is_err());
}
```

**Step 2: Run tests to verify failure**

Run: `cargo test -p krillnotes-core storage::tests::test_open`
Expected: FAIL with "function not defined"

**Step 3: Implement Storage::open()**

Add to `krillnotes-core/src/core/storage.rs` in the `impl Storage` block:

```rust
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
```

**Step 4: Run tests to verify pass**

Run: `cargo test -p krillnotes-core storage::tests::test_open`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/storage.rs
git commit -m "feat(core): add Storage::open() with validation

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 2: Add Workspace::open() Method

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write test for opening existing workspace**

Add to `krillnotes-core/src/core/workspace.rs` tests section:

```rust
#[test]
fn test_open_existing_workspace() {
    let temp = NamedTempFile::new().unwrap();

    // Create workspace first
    {
        let ws = Workspace::create(temp.path()).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert_eq!(root.node_type, "TextNote");
    }

    // Open it
    let ws = Workspace::open(temp.path()).unwrap();

    // Verify we can read notes
    let notes = ws.list_all_notes().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].node_type, "TextNote");
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p krillnotes-core workspace::tests::test_open`
Expected: FAIL with "function not defined"

**Step 3: Implement Workspace::open()**

Add to `krillnotes-core/src/core/workspace.rs` in the `impl Workspace` block:

```rust
pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
    let mut storage = Storage::open(&path)?;
    let registry = SchemaRegistry::new()?;
    let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 1000 });

    // Read metadata from database
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
```

**Step 4: Run test to verify pass**

Run: `cargo test -p krillnotes-core workspace::tests::test_open`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add Workspace::open() method

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 3: Add tauri-plugin-dialog Dependency

**Files:**
- Modify: `krillnotes-desktop/src-tauri/Cargo.toml`

**Step 1: Add dependency**

Add to `[dependencies]` section in `krillnotes-desktop/src-tauri/Cargo.toml`:

```toml
tauri-plugin-dialog = "2.0"
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/Cargo.toml
git commit -m "build: add tauri-plugin-dialog dependency

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 4: Add AppState and WorkspaceInfo Types

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add imports and AppState struct**

Add at the top of `krillnotes-desktop/src-tauri/src/lib.rs` after existing imports:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State, Window};

#[derive(Clone)]
pub struct AppState {
    pub workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub filename: String,
    pub path: String,
    pub note_count: usize,
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add AppState and WorkspaceInfo types

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 5: Add Helper Function - generate_unique_label

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add function**

Add before the `run()` function in `krillnotes-desktop/src-tauri/src/lib.rs`:

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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add generate_unique_label helper

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 6: Add Helper Function - find_window_for_path

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add function**

Add after `generate_unique_label`:

```rust
fn find_window_for_path(state: &AppState, path: &PathBuf) -> Option<String> {
    state.workspace_paths.lock()
        .expect("Mutex poisoned")
        .iter()
        .find(|(_, p)| *p == path)
        .map(|(label, _)| label.clone())
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add find_window_for_path helper

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 7: Add Helper Function - focus_window

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add function**

Add after `find_window_for_path`:

```rust
fn focus_window(app: &AppHandle, label: &str) -> Result<(), String> {
    app.get_webview_window(label)
        .ok_or_else(|| "Window not found".to_string())
        .and_then(|window| {
            window.set_focus()
                .map_err(|e| format!("Failed to focus: {}", e))
        })
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add focus_window helper

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 8: Add Helper Function - create_workspace_window

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add function**

Add after `focus_window`:

```rust
fn create_workspace_window(
    app: &AppHandle,
    label: &str
) -> Result<Window, String> {
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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add create_workspace_window helper

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 9: Add Helper Function - store_workspace

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add function**

Add after `create_workspace_window`:

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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add store_workspace helper

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 10: Add Helper Function - get_workspace_info_internal

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add function**

Add after `store_workspace`:

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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add get_workspace_info_internal helper

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 11: Add Tauri Command - create_workspace

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add command**

Add after helper functions, before the `run()` function:

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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add create_workspace command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 12: Add Tauri Command - open_workspace

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add command**

Add after `create_workspace`:

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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add open_workspace command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 13: Add Tauri Command - get_workspace_info

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add command**

Add after `open_workspace`:

```rust
#[tauri::command]
fn get_workspace_info(
    window: Window,
    state: State<'_, AppState>,
) -> Result<WorkspaceInfo, String> {
    get_workspace_info_internal(&state, window.label())
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add get_workspace_info command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 14: Add Tauri Command - list_notes

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add command**

Add after `get_workspace_info`:

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

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add list_notes command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 15: Add Window Cleanup Helper

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add cleanup functions**

Add after the commands:

```rust
fn setup_window_cleanup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let state_handle = app.state::<AppState>();
    let state_clone = state_handle.inner().clone();

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
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add window cleanup functions

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 16: Update Menu Handler with Declarative Mapping

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add menu message mapping constant**

Add before the `run()` function:

```rust
const MENU_MESSAGES: &[(&str, &str)] = &[
    ("file_new", "File > New Workspace clicked"),
    ("file_open", "File > Open Workspace clicked"),
    ("edit_add_note", "Edit > Add Note clicked"),
    ("edit_delete_note", "Edit > Delete Note clicked"),
    ("view_refresh", "View > Refresh clicked"),
    ("help_about", "Help > About Krillnotes clicked"),
];

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    MENU_MESSAGES.iter()
        .find(|(id, _)| id == &event.id().as_ref())
        .map(|(_, message)| app.emit("menu-action", message))
        .transpose()
        .ok();
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add declarative menu handler

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 17: Update run() Function with AppState and Handlers

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Replace run() function**

Replace the existing `run()` function:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            workspace_paths: Arc::new(Mutex::new(HashMap::new())),
        })
        .setup(|app| {
            let menu = menu::build_menu(app.handle())?;
            app.set_menu(menu)?;
            setup_window_cleanup(app)?;
            Ok(())
        })
        .on_menu_event(handle_menu_event)
        .invoke_handler(tauri::generate_handler![
            greet,
            create_workspace,
            open_workspace,
            get_workspace_info,
            list_notes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): wire up AppState and commands in run()

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 18: Create Frontend Types File

**Files:**
- Create: `krillnotes-desktop/src/types.ts`

**Step 1: Create types file**

Create `krillnotes-desktop/src/types.ts`:

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

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(frontend): add TypeScript type definitions

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 19: Create WelcomeDialog Component

**Files:**
- Create: `krillnotes-desktop/src/components/WelcomeDialog.tsx`

**Step 1: Create component**

Create `krillnotes-desktop/src/components/WelcomeDialog.tsx`:

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

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WelcomeDialog.tsx
git commit -m "feat(frontend): add WelcomeDialog component

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 20: Create EmptyState Component

**Files:**
- Create: `krillnotes-desktop/src/components/EmptyState.tsx`

**Step 1: Create component**

Create `krillnotes-desktop/src/components/EmptyState.tsx`:

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

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/EmptyState.tsx
git commit -m "feat(frontend): add EmptyState component

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 21: Create WorkspaceInfo Component

**Files:**
- Create: `krillnotes-desktop/src/components/WorkspaceInfo.tsx`

**Step 1: Create component**

Create `krillnotes-desktop/src/components/WorkspaceInfo.tsx`:

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

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceInfo.tsx
git commit -m "feat(frontend): add WorkspaceInfo component

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 22: Update StatusMessage with Error Styling

**Files:**
- Modify: `krillnotes-desktop/src/components/StatusMessage.tsx`

**Step 1: Read current StatusMessage**

Run: `cat krillnotes-desktop/src/components/StatusMessage.tsx`
Expected: Shows existing component

**Step 2: Update component with error prop**

Replace `krillnotes-desktop/src/components/StatusMessage.tsx`:

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

**Step 3: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/StatusMessage.tsx
git commit -m "feat(frontend): add error styling to StatusMessage

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 23: Update App.tsx with Workspace Logic (Part 1 - Menu Handlers)

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Add imports**

Replace the imports at the top of `krillnotes-desktop/src/App.tsx`:

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
import './styles/globals.css';
```

**Step 2: Add menu handlers function**

Add before the `App` function:

```typescript
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
```

**Step 3: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat(frontend): add menu handlers to App

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 24: Update App.tsx with Workspace Logic (Part 2 - Component)

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Replace App function**

Replace the `App` function in `krillnotes-desktop/src/App.tsx`:

```typescript
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

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat(frontend): complete App component with workspace state

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 25: Manual Integration Testing

**Files:**
- None (testing)

**Step 1: Start development server**

Run: `cd krillnotes-desktop && npm run tauri dev`
Expected: App window opens

**Step 2: Test welcome dialog**

1. App should show welcome dialog
2. Click OK button
3. Should show empty state
4. Close app and reopen
5. Should NOT show welcome dialog (localStorage persists)

**Step 3: Test create workspace**

1. Clear localStorage: Open browser DevTools → Storage → localStorage → Clear
2. Restart app → dismiss welcome
3. Click File > New Workspace
4. Choose location: `/tmp/test-workspace.db`
5. Should create new window with title "Krillnotes - test-workspace"
6. Main window should close
7. Should show workspace info (1 note count)

**Step 4: Test open workspace**

1. Click File > New Workspace again
2. Choose `/tmp/second-workspace.db`
3. Should create second window with title "Krillnotes - second-workspace"
4. Both windows should be open

**Step 5: Test duplicate file open**

1. From second-workspace window, click File > Open Workspace
2. Choose `/tmp/test-workspace.db` (already open)
3. Should focus the first window (test-workspace)
4. No error message shown

**Step 6: Test filename conflicts**

1. Create `/tmp/notes.db` via File > New Workspace
2. Create `/tmp/subfolder/notes.db` via File > New Workspace
3. Second should have label "notes-2" (visible in window title)

**Step 7: Test window cleanup**

1. Close one workspace window
2. Try to open that same file again
3. Should open successfully (label was freed)

**Step 8: Test error handling**

1. Click File > Open Workspace
2. Choose a non-existent file (cancel, then manually type invalid path)
3. Should show red error message
4. Error should auto-clear after 5 seconds

**Step 9: Document results**

No commit needed - manual testing complete

---

## Task 26: Update CHECKPOINT.md

**Files:**
- Modify: `CHECKPOINT.md`

**Step 1: Update checkpoint file**

Update the "Next Steps" section in `CHECKPOINT.md`:

```markdown
## 🎯 Next Steps: Functional Features

**Phase 2: Workspace Integration (Complete)** ✅
- ✅ Multi-window support with filename-based labels
- ✅ File picker integration (create/open .db files)
- ✅ Display workspace info in UI
- ✅ Welcome dialog and empty state
- ✅ Window cleanup on close

**Phase 3: Tree View (Next)**
- Display hierarchical note list
- Note selection handling
- Tree view component

**Phase 4: Detail View**
- Edit note title and fields
- Auto-save functionality
- Schema-driven field rendering
```

**Step 2: Commit**

```bash
git add CHECKPOINT.md
git commit -m "docs: mark Phase 2 complete in checkpoint

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Summary

**Phase 2 Implementation Complete!** 🎉

**Delivered:**
- ✅ Core library extensions (Storage::open, Workspace::open)
- ✅ Multi-window support with filename-based labels
- ✅ Native file picker integration
- ✅ Workspace lifecycle management
- ✅ Welcome dialog and empty state UI
- ✅ Workspace info display (path, note count)
- ✅ Error handling with visual feedback
- ✅ Window cleanup on close

**Lines of Code:** ~800 (estimated)

**Tasks Completed:** 26

**Next Phase:** Tree View (Phase 3)
