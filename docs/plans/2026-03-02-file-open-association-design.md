# File Open Association — Design

**Issue:** #38
**Date:** 2026-03-02

## Summary

Register `.krillnotes` as an OS-level file type on macOS, Windows, and Linux so that
double-clicking (or "Open With") launches Krillnotes and triggers the existing import flow,
pre-loaded with the selected file path. The architecture must not foreclose handling a
future `.swarm` file type that triggers a different action.

## Approach

Three layers:

1. **OS registration** — `bundle.fileAssociations` in `tauri.conf.json` tells the Tauri
   bundler to embed the necessary metadata (macOS `Info.plist` `CFBundleDocumentTypes`,
   Windows registry entries via NSIS/MSI, Linux `.desktop` MimeType via deb/rpm).

2. **Runtime delivery** — platform-specific mechanisms deliver the file path to the
   running (or newly started) Krillnotes process:
   - **macOS cold-start & warm-start**: `tauri::RunEvent::Opened` fires in `app.run()`.
   - **Windows & Linux cold-start**: the file path arrives as a CLI argument in
     `std::env::args()`; the OS spawns a fresh process per open, which is the desired
     multi-window behavior.
   - **Windows & Linux warm-start** (app already running): same as cold-start — a second
     process is spawned, it opens a new workspace window. No single-instance plugin
     required.

3. **`tauri-plugin-deep-link`** — added to the project for URL-scheme extensibility
   (e.g., a future `krillnotes://` inter-app link or a `.swarm` action that arrives as a
   custom URL). It does **not** handle file associations, which use the native mechanisms
   above. Initialised in the builder but no listeners registered in this feature.

## Dispatch Layer (extensibility hook)

A single `handle_file_opened(app, state, path)` function in `lib.rs` pattern-matches on
the file extension. All future file types are added here — nothing else needs to change:

```rust
fn handle_file_opened(app: &AppHandle, state: &AppState, path: PathBuf) {
    match path.extension().and_then(|e| e.to_str()) {
        Some("krillnotes") => handle_krillnotes_open(app, state, path),
        // future: Some("swarm") => handle_swarm_open(app, state, path),
        _ => {}
    }
}
```

## Architecture

### AppState — new field

```rust
pub struct AppState {
    // ... existing fields ...
    pub pending_file_open: Arc<Mutex<Option<PathBuf>>>,
}
```

This stores a file path that arrived before the frontend was ready to receive it.
It is read-and-cleared by the `consume_pending_file_open` Tauri command.

### Rust — `handle_krillnotes_open`

```rust
fn handle_krillnotes_open(app: &AppHandle, state: &AppState, path: PathBuf) {
    // 1. Store in AppState (handles the cold-start case where JS isn't ready yet)
    {
        let mut p = state.pending_file_open.lock().unwrap();
        *p = Some(path.clone());
    }

    // 2. If "main" window exists and JS is already running, emit directly.
    //    The frontend listener clears the pending entry via consume_pending_file_open.
    if let Some(win) = app.get_webview_window("main") {
        win.emit("file-opened", path.to_string_lossy().to_string()).ok();
    } else {
        // 3. No launcher window — create one. It will call consume_pending_file_open
        //    in its mount effect and start the import flow.
        create_main_window(app);
    }
}
```

### Rust — cold-start detection in `setup()`

```rust
// After AppState is initialised, before returning from setup():
let file_args: Vec<PathBuf> = std::env::args()
    .skip(1)
    .filter_map(|a| {
        let p = PathBuf::from(&a);
        if p.exists() { Some(p) } else { None }
    })
    .collect();

for path in file_args {
    handle_file_opened(&app_handle, &state, path);
}
```

### Rust — warm-start detection in `app.run()`

```rust
app.run(|app_handle, event| {
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

Note: `RunEvent::Opened` fires on macOS for both cold-start and warm-start. For
cold-start, the JS may not be listening yet — the AppState store + mount-effect poll
handles that case.

### Rust — new Tauri command

```rust
#[tauri::command]
fn consume_pending_file_open(state: State<'_, AppState>) -> Option<String> {
    let mut p = state.pending_file_open.lock().unwrap();
    p.take().map(|path| path.to_string_lossy().into_owned())
}
```

### Frontend — `App.tsx`

Two mechanisms, both calling the same function:

```tsx
// On mount — handles cold-start where the event fired before JS was ready
useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label() !== 'main') return;
    invoke<string | null>('consume_pending_file_open').then(path => {
        if (path) proceedWithImport(path, null);
    });
}, []);

// Event listener — handles warm-start where "main" window already existed
useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label() !== 'main') return;
    const unlisten = win.listen<string>('file-opened', () => {
        // Path is in the event, but consume_pending_file_open is the canonical source
        // so both paths go through the same cleared-on-read store.
        invoke<string | null>('consume_pending_file_open').then(p => {
            if (p) proceedWithImport(p, null);
        });
    });
    return () => { unlisten.then(f => f()); };
}, []);
```

`proceedWithImport(path, null)` is the existing function already used by File > Import.
No changes to the import dialog sequence are needed.

## `tauri.conf.json` changes

```json
{
  "bundle": {
    "fileAssociations": [
      {
        "ext": ["krillnotes"],
        "name": "Krillnotes Archive",
        "description": "Krillnotes Workspace Archive",
        "mimeType": "application/x-krillnotes",
        "role": "Editor"
      }
    ]
  }
}
```

The `role: "Editor"` field maps to `LSHandlerRank: Owner` on macOS, making Krillnotes
the default opener (a missing `LSHandlerRank` is a known Tauri bug fixed in v2; setting
`role` is the workaround).

## Key Decisions

- **Multi-instance on Windows/Linux** — no single-instance plugin. Each double-click
  spawns a new process with a new workspace window. This is consistent with the existing
  multi-window model.
- **`tauri-plugin-deep-link` scope** — added as infrastructure for future URL scheme
  support, not used for file associations in this feature. The plugin is initialised in
  the Tauri builder but has no active listeners.
- **`pending_file_open` is a single slot** — if multiple files are opened simultaneously
  (e.g., user selects several and hits Open), only the last one is queued. Multi-file
  open is not a supported use-case. Each file triggers its own process on Windows/Linux;
  on macOS `RunEvent::Opened` delivers all URLs in one call — loop handles them
  sequentially, each creating its own main window.
- **Cold-start race handled by pull** — `consume_pending_file_open` (pull) guarantees the
  import starts even if the push event fired before the JS listener was registered.
- **No changes to import dialogs** — the feature wires into `proceedWithImport()` at the
  same entry point as File > Import. Password prompts, workspace naming, and workspace
  password setup are unchanged.
