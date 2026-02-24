# Copy and Paste Notes — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow users to copy any note (and all its descendants) and paste it as a child or sibling of any compatible target note, accessible via context menu, Edit menu, and keyboard shortcuts.

**Architecture:** A new Rust `deep_copy_note` method recursively clones a note subtree in a single transaction. Clipboard state (`copiedNoteId`) lives in per-window React state in `WorkspaceView`. The native Edit menu paste items are kept in sync via a `set_paste_menu_enabled` Tauri command that stores `MenuItem` handles in `AppState`.

**Tech Stack:** Rust / Tauri v2, React / TypeScript, SQLite via rusqlite, `tauri::menu::MenuItem`

**Worktree:** `.worktrees/feat/copy-paste-notes/`

---

## Task 1: Rust — `deep_copy_note` in `workspace.rs`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

### Step 1: Write the failing test

At the bottom of `workspace.rs`, inside the `#[cfg(test)]` module, add:

```rust
#[test]
fn test_deep_copy_note_as_child() {
    // Build an in-memory workspace with two notes: parent → child
    // Call deep_copy_note(child_id, parent_id, AddPosition::AsChild)
    // Assert the copy exists with a new ID, same title and fields
    // Assert the original child is unchanged
}

#[test]
fn test_deep_copy_note_recursive() {
    // Build: root → note_a → note_b (grandchild)
    // Copy note_a as a child of root
    // Assert both note_a copy and note_b copy exist with new IDs
    // Assert parent_id of note_b copy points to the copy of note_a (not original)
}
```

> **Note on test setup:** Look at existing tests in workspace.rs for the helper that creates an in-memory `Workspace`. Use the same pattern.

### Step 2: Run tests to see them fail (compile error is fine at this stage)

```bash
cargo test -p krillnotes-core -- deep_copy 2>&1 | head -30
```

### Step 3: Implement `deep_copy_note`

Add this method to `impl Workspace` in `krillnotes-core/src/core/workspace.rs`, near the `create_note` method:

```rust
/// Deep-copies the note at `source_id` and its entire descendant subtree,
/// placing the copy at `target_id` with the given `position`.
///
/// Returns the ID of the new root note.
///
/// All notes in the subtree receive fresh UUIDs and current timestamps.
/// Schema constraints (`allowed_parent_types`, `allowed_children_types`) are
/// validated only for the root of the copy against the paste target.
/// Children's internal parent/child relationships are trusted and not re-validated.
pub fn deep_copy_note(
    &mut self,
    source_id: &str,
    target_id: &str,
    position: AddPosition,
) -> Result<String> {
    // 1. Load the full subtree rooted at source_id using an iterative BFS.
    let mut subtree: Vec<Note> = Vec::new();
    let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    queue.push_back(source_id.to_string());
    while let Some(current_id) = queue.pop_front() {
        let note = self.get_note(&current_id)?;
        // Enqueue children
        let child_ids: Vec<String> = self
            .connection()
            .prepare("SELECT id FROM notes WHERE parent_id = ? ORDER BY position")?
            .query_map([&current_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for cid in child_ids {
            queue.push_back(cid);
        }
        subtree.push(note);
    }

    if subtree.is_empty() {
        return Err(KrillnotesError::NoteNotFound(source_id.to_string()));
    }

    // 2. Validate the paste location for the root note only.
    let root_source = &subtree[0];
    let root_schema = self.script_registry.get_schema(&root_source.node_type)?;
    let target_note = self.get_note(target_id)?;

    let (new_parent_id, new_position) = match position {
        AddPosition::AsChild => (Some(target_note.id.clone()), 0i32),
        AddPosition::AsSibling => (target_note.parent_id.clone(), target_note.position + 1),
    };

    // Validate allowed_parent_types for the root copy
    if !root_schema.allowed_parent_types.is_empty() {
        match &new_parent_id {
            None => return Err(KrillnotesError::InvalidMove(format!(
                "Note type '{}' cannot be placed at root level", root_source.node_type
            ))),
            Some(pid) => {
                let parent = self.get_note(pid)?;
                if !root_schema.allowed_parent_types.contains(&parent.node_type) {
                    return Err(KrillnotesError::InvalidMove(format!(
                        "Note type '{}' cannot be placed under '{}'",
                        root_source.node_type, parent.node_type
                    )));
                }
            }
        }
    }

    // Validate allowed_children_types on the paste parent
    if let Some(pid) = &new_parent_id {
        let parent = self.get_note(pid)?;
        let parent_schema = self.script_registry.get_schema(&parent.node_type)?;
        if !parent_schema.allowed_children_types.is_empty()
            && !parent_schema.allowed_children_types.contains(&root_source.node_type)
        {
            return Err(KrillnotesError::InvalidMove(format!(
                "Note type '{}' is not allowed as a child of '{}'",
                root_source.node_type, parent.node_type
            )));
        }
    }

    // 3. Build old_id → new_id remap table.
    let mut id_map: HashMap<String, String> = HashMap::new();
    for note in &subtree {
        id_map.insert(note.id.clone(), Uuid::new_v4().to_string());
    }

    let now = chrono::Utc::now().timestamp();

    // 4. Insert all cloned notes in a single transaction.
    let tx = self.storage.connection_mut().transaction()?;

    // If pasting as sibling, bump positions of following siblings to make room.
    if let AddPosition::AsSibling = position {
        tx.execute(
            "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ?",
            rusqlite::params![new_parent_id, new_position],
        )?;
    }

    let root_new_id = id_map[source_id].clone();

    for note in &subtree {
        let new_id = id_map[&note.id].clone();
        let new_parent = if note.id == source_id {
            // Root of the copy gets the paste target as parent
            match &new_parent_id {
                Some(p) => Some(p.clone()),
                None => None,
            }
        } else {
            // Children remap their parent_id through the id_map
            note.parent_id.as_ref().and_then(|pid| id_map.get(pid).cloned())
        };
        let this_position = if note.id == source_id { new_position } else { note.position };

        tx.execute(
            "INSERT INTO notes (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                new_id,
                note.title,
                note.node_type,
                new_parent,
                this_position,
                now,
                now,
                self.current_user_id,
                self.current_user_id,
                serde_json::to_string(&note.fields)?,
                note.is_expanded,
            ],
        )?;

        // Log a CreateNote operation for each inserted note.
        let op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            note_id: new_id.clone(),
            parent_id: new_parent,
            position: this_position,
            node_type: note.node_type.clone(),
            title: note.title.clone(),
            fields: note.fields.clone(),
            created_by: self.current_user_id,
        };
        self.operation_log.log(&tx, &op)?;
    }

    self.operation_log.purge_if_needed(&tx)?;
    tx.commit()?;

    Ok(root_new_id)
}
```

### Step 4: Run tests to verify they pass

```bash
cargo test -p krillnotes-core -- deep_copy 2>&1
```

Expected: all `deep_copy` tests PASS.

### Step 5: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-core/src/core/workspace.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: add deep_copy_note to workspace"
```

---

## Task 2: Tauri command — `deep_copy_note` in `lib.rs`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

### Step 1: Add the Tauri command

Find the block of `#[tauri::command]` functions and add this near `move_note`:

```rust
#[tauri::command]
fn deep_copy_note_cmd(
    state: State<'_, AppState>,
    window: tauri::Window,
    source_note_id: String,
    target_note_id: String,
    position: String, // "child" or "sibling"
) -> Result<String, String> {
    let label = window.label().to_string();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let ws = workspaces
        .get_mut(&label)
        .ok_or_else(|| "No workspace open".to_string())?;
    let pos = if position == "child" {
        AddPosition::AsChild
    } else {
        AddPosition::AsSibling
    };
    ws.deep_copy_note(&source_note_id, &target_note_id, pos)
        .map_err(|e| e.to_string())
}
```

### Step 2: Register the command

In the `invoke_handler!` macro call at the bottom of `run()`, add `deep_copy_note_cmd` to the list alongside `move_note`.

### Step 3: Build to verify it compiles

```bash
cargo build -p krillnotes-desktop 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

### Step 4: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src-tauri/src/lib.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: add deep_copy_note_cmd Tauri command"
```

---

## Task 3: AppState — store paste `MenuItem` handles

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

### Step 1: Add paste handles to `AppState`

In `lib.rs`, update the `AppState` struct to add a field for the paste menu item handles:

```rust
use tauri::menu::MenuItem;

pub struct AppState {
    pub workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
    pub focused_window: Arc<Mutex<Option<String>>>,
    pub workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>,
    /// Paste menu item handles for dynamic enable/disable.
    /// On macOS: one global pair (the menu is shared).
    /// On Windows: keyed by window label (each window owns its menu).
    pub paste_menu_items: Arc<Mutex<HashMap<String, (MenuItem<tauri::Wry>, MenuItem<tauri::Wry>)>>>,
}
```

Update the `AppState` initialisation in `run()` to include the new field:

```rust
.manage(AppState {
    workspaces: Arc::new(Mutex::new(HashMap::new())),
    workspace_paths: Arc::new(Mutex::new(HashMap::new())),
    focused_window: Arc::new(Mutex::new(None)),
    workspace_passwords: Arc::new(Mutex::new(HashMap::new())),
    paste_menu_items: Arc::new(Mutex::new(HashMap::new())),
})
```

### Step 2: Return paste handles from `build_menu`

In `menu.rs`, change `build_edit_menu` to return the two paste `MenuItem`s alongside the menu. The simplest approach is to return a struct:

```rust
pub struct EditMenuResult<R: tauri::Runtime> {
    pub submenu: Submenu<R>,
    pub paste_as_child: MenuItem<R>,
    pub paste_as_sibling: MenuItem<R>,
}
```

Update `build_edit_menu` signature to return `Result<EditMenuResult<R>, tauri::Error>`.

Build the three new items inside `build_edit_menu`:

```rust
let copy_note = MenuItemBuilder::with_id("edit_copy_note", "Copy Note")
    .build(app)?;
let paste_child = MenuItemBuilder::with_id("edit_paste_as_child", "Paste as Child")
    .enabled(false)   // disabled until a note is copied
    .build(app)?;
let paste_sibling = MenuItemBuilder::with_id("edit_paste_as_sibling", "Paste as Sibling")
    .enabled(false)
    .build(app)?;
```

Insert them between the `sep1` and `undo` items in the `SubmenuBuilder`:

```rust
builder.items(&[
    &add_note, &delete_note, &sep1,
    &copy_note, &paste_child, &paste_sibling,
    &PredefinedMenuItem::separator(app)?,
    &undo, &redo, &copy, &paste,
])
```

Return the struct:

```rust
Ok(EditMenuResult {
    submenu: builder.build()?,
    paste_as_child: paste_child,
    paste_as_sibling: paste_sibling,
})
```

Update `build_menu` to unwrap the `EditMenuResult` and use `.submenu` where `edit_menu` was previously used.

### Step 3: Store handles in `AppState` during setup

In `run()`, inside the `.setup(|app|)` closure, after `build_menu` is called:

```rust
.setup(|app| {
    let menu_result = menu::build_menu(app.handle())?;
    app.set_menu(menu_result.menu)?;

    // Store paste item handles for macOS (global key "macos")
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<AppState>();
        state.paste_menu_items.lock().expect("Mutex poisoned")
            .insert("macos".to_string(), (menu_result.paste_as_child, menu_result.paste_as_sibling));
    }

    // ... rest of setup unchanged
    Ok(())
})
```

> **Windows note:** On Windows, paste handles are stored per workspace window. This is done in `open_workspace_window` (Task 4, Step 1).

### Step 4: Build to verify it compiles

```bash
cargo build -p krillnotes-desktop 2>&1 | grep -E "^error" | head -20
```

### Step 5: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/src/menu.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: store paste MenuItem handles in AppState"
```

---

## Task 4: Tauri command — `set_paste_menu_enabled`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

### Step 1: Add the command

```rust
#[tauri::command]
fn set_paste_menu_enabled(
    state: State<'_, AppState>,
    window: tauri::Window,
    enabled: bool,
) -> Result<(), String> {
    let label = window.label().to_string();

    #[cfg(target_os = "macos")]
    {
        let items = state.paste_menu_items.lock().expect("Mutex poisoned");
        if let Some((child_item, sibling_item)) = items.get("macos") {
            child_item.set_enabled(enabled).map_err(|e| e.to_string())?;
            sibling_item.set_enabled(enabled).map_err(|e| e.to_string())?;
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let items = state.paste_menu_items.lock().expect("Mutex poisoned");
        if let Some((child_item, sibling_item)) = items.get(&label) {
            child_item.set_enabled(enabled).map_err(|e| e.to_string())?;
            sibling_item.set_enabled(enabled).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}
```

### Step 2: Store Windows handles when a workspace window is created

Find `open_workspace_window` in `lib.rs`. After `.menu(menu)` is called on `WebviewWindowBuilder`, store the paste handles in `AppState`:

```rust
#[cfg(not(target_os = "macos"))]
{
    let menu_result = menu::build_menu(app)?;
    // ... (use menu_result.menu for the window)
    let state = app.state::<AppState>();
    state.paste_menu_items.lock().expect("Mutex poisoned")
        .insert(label.clone(), (menu_result.paste_as_child, menu_result.paste_as_sibling));
}
```

> **Read `open_workspace_window` carefully first** — it's in `lib.rs` around line 103–130. The Windows branch already calls `menu::build_menu` separately; you are plugging into that existing call.

### Step 3: Register the command in `invoke_handler!`

Add `set_paste_menu_enabled` to the list.

### Step 4: Build to verify it compiles

```bash
cargo build -p krillnotes-desktop 2>&1 | grep -E "^error" | head -20
```

### Step 5: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src-tauri/src/lib.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: add set_paste_menu_enabled Tauri command"
```

---

## Task 5: Menu event routing — `menu.rs` + `lib.rs`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

### Step 1: Add the three new IDs to `MENU_MESSAGES`

Find the `MENU_MESSAGES` constant array and append:

```rust
("edit_copy_note",       "Edit > Copy Note clicked"),
("edit_paste_as_child",  "Edit > Paste as Child clicked"),
("edit_paste_as_sibling","Edit > Paste as Sibling clicked"),
```

### Step 2: Build to verify it compiles

```bash
cargo build -p krillnotes-desktop 2>&1 | grep -E "^error" | head -20
```

### Step 3: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src-tauri/src/lib.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: route copy/paste menu events to frontend"
```

---

## Task 6: Frontend state + keyboard shortcuts in `WorkspaceView.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

### Step 1: Add `copiedNoteId` state

Near the existing `selectedNoteId` state declaration, add:

```ts
const [copiedNoteId, setCopiedNoteId] = useState<string | null>(null);
```

### Step 2: Add `copyNote` handler

```ts
const copyNote = useCallback((noteId: string) => {
  setCopiedNoteId(noteId);
  invoke('set_paste_menu_enabled', { enabled: true }).catch(console.error);
}, []);
```

### Step 3: Add `pasteNote` handler

```ts
const pasteNote = useCallback(async (position: 'child' | 'sibling') => {
  if (!copiedNoteId || !selectedNoteId) return;
  try {
    const newId = await invoke<string>('deep_copy_note_cmd', {
      sourceNoteId: copiedNoteId,
      targetNoteId: selectedNoteId,
      position,
    });
    const freshNotes = await loadNotes();
    // Expand target so the paste is visible
    if (position === 'child') {
      await invoke('toggle_note_expansion', { noteId: selectedNoteId, expanded: true });
    }
    setSelectedNoteId(newId);
  } catch (err) {
    statusSetter(`Paste failed: ${err}`, true);
  }
}, [copiedNoteId, selectedNoteId, loadNotes]);
```

> **Check:** `statusSetter` and `loadNotes` are already defined in `WorkspaceView.tsx`. Just reference them.

### Step 4: Add keyboard shortcut listener

Inside the main `useEffect` that sets up keyboard navigation (look for the `keydown` handler that handles arrow keys), add the copy/paste shortcuts. Create a dedicated `useEffect` for clarity:

```ts
useEffect(() => {
  const isInputFocused = () => {
    const el = document.activeElement;
    if (!el) return false;
    const tag = el.tagName.toLowerCase();
    return tag === 'input' || tag === 'textarea' || (el as HTMLElement).isContentEditable;
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (!(e.metaKey || e.ctrlKey)) return;
    if (isInputFocused()) return;

    if (e.key === 'c' && !e.shiftKey) {
      if (selectedNoteId) { copyNote(selectedNoteId); e.preventDefault(); }
    } else if (e.key === 'v' && !e.shiftKey) {
      pasteNote('child'); e.preventDefault();
    } else if (e.key === 'v' && e.shiftKey) {
      pasteNote('sibling'); e.preventDefault();
    }
  };

  document.addEventListener('keydown', handleKeyDown);
  return () => document.removeEventListener('keydown', handleKeyDown);
}, [selectedNoteId, copiedNoteId, copyNote, pasteNote]);
```

### Step 5: Sync menu state on window focus

Add a `useEffect` that fires once on mount to sync the paste menu state when this window gains focus (important for macOS multi-window):

```ts
useEffect(() => {
  const win = getCurrentWebviewWindow();
  let unlisten: (() => void) | null = null;
  win.onFocusChanged(({ payload: focused }) => {
    if (focused) {
      invoke('set_paste_menu_enabled', { enabled: copiedNoteId !== null }).catch(console.error);
    }
  }).then(fn => { unlisten = fn; });
  return () => { unlisten?.(); };
}, [copiedNoteId]);
```

> **Import:** Add `getCurrentWebviewWindow` import at the top if not already present (check — it may already be imported in `App.tsx` but not `WorkspaceView.tsx`).

### Step 6: Build frontend to verify TypeScript

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes/krillnotes-desktop && npm run build 2>&1 | grep -E "error TS|Error" | head -20
```

### Step 7: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src/components/WorkspaceView.tsx
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: add copiedNoteId state, keyboard shortcuts and menu sync"
```

---

## Task 7: Context menu — `ContextMenu.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/ContextMenu.tsx`

### Step 1: Update props interface

```ts
interface ContextMenuProps {
  x: number;
  y: number;
  copiedNoteId: string | null;
  onAddNote: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onPasteAsChild: () => void;
  onPasteAsSibling: () => void;
  onDelete: () => void;
  onClose: () => void;
}
```

### Step 2: Update the component signature and JSX

```tsx
function ContextMenu({
  x, y, copiedNoteId,
  onAddNote, onEdit, onCopy, onPasteAsChild, onPasteAsSibling, onDelete, onClose
}: ContextMenuProps) {
```

In the JSX, add the three new items between Edit and the separator before Delete:

```tsx
<button
  className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
  onClick={() => { onCopy(); onClose(); }}
>
  Copy Note
</button>
<button
  className={`w-full text-left px-3 py-1.5 text-sm ${copiedNoteId ? 'hover:bg-secondary' : 'opacity-40 cursor-not-allowed'}`}
  onClick={() => { if (copiedNoteId) { onPasteAsChild(); onClose(); } }}
>
  Paste as Child
</button>
<button
  className={`w-full text-left px-3 py-1.5 text-sm ${copiedNoteId ? 'hover:bg-secondary' : 'opacity-40 cursor-not-allowed'}`}
  onClick={() => { if (copiedNoteId) { onPasteAsSibling(); onClose(); } }}
>
  Paste as Sibling
</button>
```

### Step 3: Build to verify TypeScript

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes/krillnotes-desktop && npm run build 2>&1 | grep -E "error TS|Error" | head -20
```

### Step 4: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src/components/ContextMenu.tsx
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: add copy/paste items to ContextMenu"
```

---

## Task 8: Wire callbacks in `WorkspaceView.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

### Step 1: Pass `copiedNoteId` and callbacks into ContextMenu

Find where `<ContextMenu>` is rendered in `WorkspaceView.tsx` (around line 460–480) and add the new props:

```tsx
<ContextMenu
  x={contextMenu.x}
  y={contextMenu.y}
  copiedNoteId={copiedNoteId}
  onAddNote={() => handleContextAdd(contextMenu.noteId)}
  onEdit={() => handleContextEdit(contextMenu.noteId)}
  onCopy={() => copyNote(contextMenu.noteId)}
  onPasteAsChild={() => pasteNote('child')}
  onPasteAsSibling={() => pasteNote('sibling')}
  onDelete={() => handleContextDelete(contextMenu.noteId)}
  onClose={() => setContextMenu(null)}
/>
```

### Step 2: Build to verify TypeScript

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes/krillnotes-desktop && npm run build 2>&1 | grep -E "error TS|Error" | head -20
```

### Step 3: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src/components/WorkspaceView.tsx
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: wire copy/paste callbacks into ContextMenu"
```

---

## Task 9: Handle Edit menu actions in `App.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

### Step 1: Add menu handlers to `createMenuHandlers`

The copy/paste operations need access to `WorkspaceView`'s internal state, which is not available in `App.tsx`. The cleanest solution is to have `WorkspaceView` listen for the `menu-action` events itself and handle copy/paste there — exactly like keyboard shortcuts.

Open `WorkspaceView.tsx` and **extend the existing `menu-action` useEffect** (or add a new one) to handle the copy/paste menu actions:

```ts
useEffect(() => {
  const win = getCurrentWebviewWindow();
  const unlisten = win.listen<string>('menu-action', (event) => {
    switch (event.payload) {
      case 'Edit > Copy Note clicked':
        if (selectedNoteId) copyNote(selectedNoteId);
        break;
      case 'Edit > Paste as Child clicked':
        pasteNote('child');
        break;
      case 'Edit > Paste as Sibling clicked':
        pasteNote('sibling');
        break;
    }
  });
  return () => { unlisten.then(f => f()); };
}, [selectedNoteId, copiedNoteId, copyNote, pasteNote]);
```

> **Check first:** Does `WorkspaceView.tsx` already have a `menu-action` listener? If yes, add the new cases inside the existing switch/if-else rather than creating a duplicate listener.

### Step 2: Build to verify TypeScript

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes/krillnotes-desktop && npm run build 2>&1 | grep -E "error TS|Error" | head -20
```

### Step 3: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes add krillnotes-desktop/src/components/WorkspaceView.tsx
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes commit -m "feat: handle copy/paste Edit menu actions in WorkspaceView"
```

---

## Task 10: Full build + smoke test

### Step 1: Run all Rust tests

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: all tests PASS.

### Step 2: Full Tauri dev build

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/copy-paste-notes/krillnotes-desktop && npm run tauri dev
```

**Manual smoke test checklist:**
- [ ] Right-click a note → "Copy Note", "Paste as Child" (grey), "Paste as Sibling" (grey) visible
- [ ] Copy a note → paste items in context menu become active (not grey)
- [ ] Edit menu → "Copy Note", "Paste as Child" (grey), "Paste as Sibling" (grey) visible
- [ ] Copy a note → Edit menu paste items become enabled
- [ ] Cmd+C on a selected tree node → note is marked as copied
- [ ] Cmd+V → note pasted as child, new note selected in tree
- [ ] Cmd+Shift+V → note pasted as sibling
- [ ] Cmd+C inside a text input → text is copied (system behaviour unchanged)
- [ ] Multi-level note (with children) copied → all children appear under the paste
- [ ] Schema-constrained note paste to invalid location → error message shown
- [ ] Open two workspace windows → copying in window A does not affect paste state in window B
- [ ] Switch focus from window A (has copy) to window B (no copy) → Edit menu paste greys out
- [ ] Switch back to window A → Edit menu paste re-enables

### Step 3: Commit any final fixes, then mark TODO done

In `TODO.md` in the **main checkout** (not the worktree), mark the task done:

```
✅ DONE! implement copy and paste of notes. ...
```

```bash
git -C /Users/careck/Source/Krillnotes add TODO.md
git -C /Users/careck/Source/Krillnotes commit -m "chore: mark copy-paste notes task as done"
```

### Step 4: Merge the feature branch

Follow `superpowers:finishing-a-development-branch` to merge and clean up the worktree.
