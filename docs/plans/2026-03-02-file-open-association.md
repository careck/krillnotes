# File Open Association — Implementation Plan

**Issue:** #38
**Design:** `2026-03-02-file-open-association-design.md`
**Branch:** `feat/file-open-association`
**Worktree:** `.worktrees/feat/file-open-association/`

---

## Task 1 — Create worktree and branch

```bash
git -C /Users/careck/Source/Krillnotes worktree add \
  .worktrees/feat/file-open-association -b feat/file-open-association
```

All code changes below happen inside the worktree.

---

## Task 2 — Add `tauri-plugin-deep-link` dependency

**File:** `krillnotes-desktop/src-tauri/Cargo.toml`

```toml
[dependencies]
tauri-plugin-deep-link = "2"
```

**File:** `krillnotes-desktop/package.json` — add npm dependency:

```bash
npm install @tauri-apps/plugin-deep-link
```

---

## Task 3 — Register file association in `tauri.conf.json`

**File:** `krillnotes-desktop/src-tauri/tauri.conf.json`

Add `fileAssociations` inside the existing `bundle` object:

```json
"fileAssociations": [
  {
    "ext": ["krillnotes"],
    "name": "Krillnotes Archive",
    "description": "Krillnotes Workspace Archive",
    "mimeType": "application/x-krillnotes",
    "role": "Editor"
  }
]
```

---

## Task 4 — Add `deep-link:default` capability

**File:** `krillnotes-desktop/src-tauri/capabilities/default.json`

Add `"deep-link:default"` to the `permissions` array.

---

## Task 5 — Extend `AppState` with `pending_file_open`

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

In the `AppState` struct, add:

```rust
pub pending_file_open: Arc<Mutex<Option<PathBuf>>>,
```

In the `AppState` constructor / `Default` impl, initialise:

```rust
pending_file_open: Arc::new(Mutex::new(None)),
```

---

## Task 6 — Add dispatch and handler functions

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

Add the following functions (outside any command, near the window creation helpers):

```rust
fn handle_file_opened(app: &AppHandle, state: &AppState, path: PathBuf) {
    match path.extension().and_then(|e| e.to_str()) {
        Some("krillnotes") => handle_krillnotes_open(app, state, path),
        // future: Some("swarm") => handle_swarm_open(app, state, path),
        _ => {}
    }
}

fn handle_krillnotes_open(app: &AppHandle, state: &AppState, path: PathBuf) {
    // Store for cold-start poll
    {
        let mut pending = state.pending_file_open.lock().unwrap();
        *pending = Some(path.clone());
    }

    if let Some(win) = app.get_webview_window("main") {
        // App is warm — JS is listening; emit directly.
        // The frontend listener calls consume_pending_file_open to clear the slot.
        win.emit("file-opened", path.to_string_lossy().to_string()).ok();
    } else {
        // No launcher window — create one; its mount effect will poll
        // consume_pending_file_open and trigger the import flow.
        create_main_window(app);
    }
}
```

---

## Task 7 — Parse CLI args in `setup()` (Windows / Linux cold-start)

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

At the end of the `setup` closure, after `AppState` is registered, add:

```rust
let state_ref = app.state::<AppState>();
let file_args: Vec<PathBuf> = std::env::args()
    .skip(1)
    .filter_map(|a| {
        let p = PathBuf::from(&a);
        if p.exists() { Some(p) } else { None }
    })
    .collect();

for path in file_args {
    handle_file_opened(app, &state_ref, path);
}
```

---

## Task 8 — Handle `RunEvent::Opened` in `app.run()` (macOS)

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

Change the existing `.run(tauri::generate_context!())` call to use a callback:

```rust
app.run(|app_handle, event| {
    #[allow(clippy::single_match)]
    match event {
        tauri::RunEvent::Opened { urls } => {
            let state = app_handle.state::<AppState>();
            for url in &urls {
                if url.scheme() == "file" {
                    if let Ok(path) = url.to_file_path() {
                        handle_file_opened(app_handle, &state, path);
                    }
                }
            }
        }
        _ => {}
    }
});
```

If `app.run()` already exists with a callback (check the current code), add the
`RunEvent::Opened` arm to the existing match.

---

## Task 9 — Initialise `tauri-plugin-deep-link` in the builder

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

In the `tauri::Builder::default()` chain, add:

```rust
.plugin(tauri_plugin_deep_link::init())
```

This is infrastructure only — no URL-scheme listeners are registered in this feature.

---

## Task 10 — Add `consume_pending_file_open` Tauri command

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

Add the command:

```rust
#[tauri::command]
fn consume_pending_file_open(state: State<'_, AppState>) -> Option<String> {
    let mut p = state.pending_file_open.lock().unwrap();
    p.take().map(|path| path.to_string_lossy().into_owned())
}
```

Register it in the `invoke_handler` alongside the other commands.

---

## Task 11 — Wire up the frontend in `App.tsx`

**File:** `krillnotes-desktop/src/App.tsx`

### Mount-effect (cold-start poll)

Add inside the component, after the existing workspace-info `useEffect`:

```tsx
// Cold-start: file was opened before JS listeners were ready
useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label() !== 'main') return;
    invoke<string | null>('consume_pending_file_open').then(path => {
        if (path) proceedWithImport(path, null);
    });
}, []);
```

### Event listener (warm-start)

```tsx
// Warm-start: "main" window already existed when the file was opened
useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label() !== 'main') return;
    const unlisten = win.listen<string>('file-opened', () => {
        invoke<string | null>('consume_pending_file_open').then(p => {
            if (p) proceedWithImport(p, null);
        });
    });
    return () => { unlisten.then(f => f()); };
}, []);
```

Both `useEffect` hooks have empty dependency arrays and are guarded by `win.label() !== 'main'` so they are no-ops in workspace windows.

---

## Task 12 — Verify TypeScript build

```bash
cd krillnotes-desktop && npm run build
```

Fix any type errors (e.g., if `invoke` return type needs `| null` annotation).

---

## Task 13 — Test matrix

| Scenario | Platform | Expected |
|---|---|---|
| Double-click `.krillnotes`, app not running | macOS | App launches → import dialogs start |
| Double-click `.krillnotes`, app not running | Windows | App launches → import dialogs start |
| Double-click `.krillnotes`, app not running | Linux | App launches → import dialogs start |
| Double-click `.krillnotes`, launcher open | macOS | Existing launcher shows import dialogs |
| Double-click `.krillnotes`, workspace(s) open, no launcher | macOS | New launcher window opens → import dialogs start |
| Double-click `.krillnotes`, workspace(s) open | Windows | New app instance opens → import dialogs start |
| Double-click `.krillnotes`, app not running, **encrypted** | all | Password prompt appears first |
| Right-click → Open With → Krillnotes | macOS | Same as double-click |

---

## Task 14 — Commit, push, open PR

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/file-open-association \
  add krillnotes-desktop/src-tauri/src/lib.rs \
      krillnotes-desktop/src-tauri/tauri.conf.json \
      krillnotes-desktop/src-tauri/Cargo.toml \
      krillnotes-desktop/src-tauri/Cargo.lock \
      krillnotes-desktop/src-tauri/capabilities/default.json \
      krillnotes-desktop/src/App.tsx \
      krillnotes-desktop/package.json \
      krillnotes-desktop/package-lock.json

git -C /Users/careck/Source/Krillnotes/.worktrees/feat/file-open-association \
  commit -m "feat: OS file association for .krillnotes — closes #38"

git -C /Users/careck/Source/Krillnotes push -u github-https feat/file-open-association
gh pr create --repo careck/krillnotes --base master \
  --title "feat: OS file association for .krillnotes (closes #38)" \
  --body "..."
```
