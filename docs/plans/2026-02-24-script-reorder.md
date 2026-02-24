# Script Load-Order Drag Reordering Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add drag-handle reordering to the Script Manager dialog so users can change script load order by dragging rows.

**Architecture:** Three tasks in sequence — add the Rust batch-reorder method, wire it as a Tauri command, then update the React dialog with native HTML5 drag-and-drop. No new npm dependencies. Verify with `cargo check` after Rust tasks and `npm run build` after the frontend task.

**Tech Stack:** Rust / rusqlite (backend), Tauri v2 IPC, React 19 / TypeScript / Tailwind, lucide-react (grip icon already available)

---

### Task 1: Add `reorder_all_user_scripts` to `workspace.rs`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (insert after line 1325, after the `reorder_user_script` method)

**Step 1: Insert the new method**

Open `krillnotes-core/src/core/workspace.rs`. After the closing `}` of `reorder_user_script` (line 1325), insert:

```rust
/// Re-assigns sequential load_order (1-based) to all scripts given in `ids` order, then reloads.
pub fn reorder_all_user_scripts(&mut self, ids: &[String]) -> Result<()> {
    {
        let conn = self.storage.connection_mut();
        let tx = conn.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE user_scripts SET load_order = ?1 WHERE id = ?2",
                rusqlite::params![i as i32 + 1, id],
            )?;
        }
        tx.commit()?;
    }
    self.reload_scripts()
}
```

Note: No operation log entry per script — this is a presentation-only reorder, not meaningful sync data. The existing `reload_scripts()` call handles re-registering schemas in the new order.

**Step 2: Verify it compiles**

```bash
cargo check --manifest-path /Users/careck/Source/Krillnotes/krillnotes-core/Cargo.toml
```

Expected: no errors.

**Step 3: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/workspace.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: add reorder_all_user_scripts batch method to workspace"
```

---

### Task 2: Add `reorder_all_user_scripts` Tauri command in `lib.rs`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
  - Insert new command after `reorder_user_script` fn (around line 727)
  - Add to `invoke_handler` list (around line 1055)

**Step 1: Insert the Tauri command function**

After the closing `}` of `fn reorder_user_script` (line 727), insert:

```rust
/// Reassigns sequential load order to all scripts given in order, then reloads.
#[tauri::command]
fn reorder_all_user_scripts(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_ids: Vec<String>,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.reorder_all_user_scripts(&script_ids)
        .map_err(|e| e.to_string())
}
```

**Step 2: Register in invoke_handler**

Find the `.invoke_handler(tauri::generate_handler![...])` block (around line 1055). Add `reorder_all_user_scripts` after `reorder_user_script`:

```rust
            reorder_user_script,
            reorder_all_user_scripts,   // ← add this line
```

**Step 3: Verify it compiles**

```bash
cargo check --manifest-path /Users/careck/Source/Krillnotes/krillnotes-desktop/src-tauri/Cargo.toml
```

Expected: no errors.

**Step 4: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-desktop/src-tauri/src/lib.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: add reorder_all_user_scripts Tauri command"
```

---

### Task 3: Add drag-handle reordering to `ScriptManagerDialog.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`

**Step 1: Add drag state and import `GripVertical`**

At the top of the file, `GripVertical` is in `lucide-react` which is already a dependency. Add the import:

```tsx
import { GripVertical } from 'lucide-react';
```

Inside the `ScriptManagerDialog` function, after the existing `useState` declarations (line 30), add:

```tsx
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
```

**Step 2: Add drag handler functions**

After the `handleDelete` function (after line 132), insert:

```tsx
  const handleDragStart = (index: number) => {
    setDragIndex(index);
  };

  const handleDragOver = (e: React.DragEvent, index: number) => {
    e.preventDefault();
    setDragOverIndex(index);
  };

  const handleDrop = async (e: React.DragEvent, dropIndex: number) => {
    e.preventDefault();
    if (dragIndex === null || dragIndex === dropIndex) {
      setDragIndex(null);
      setDragOverIndex(null);
      return;
    }

    // Reorder local array optimistically
    const reordered = [...scripts];
    const [moved] = reordered.splice(dragIndex, 1);
    reordered.splice(dropIndex, 0, moved);
    setScripts(reordered);
    setDragIndex(null);
    setDragOverIndex(null);

    try {
      await invoke('reorder_all_user_scripts', {
        scriptIds: reordered.map(s => s.id),
      });
      onScriptsChanged?.();
    } catch (err) {
      setError(`Failed to reorder scripts: ${err}`);
      await loadScripts(); // revert to server state on failure
    }
  };

  const handleDragEnd = () => {
    setDragIndex(null);
    setDragOverIndex(null);
  };
```

**Step 3: Update the script list rows**

Replace the script list section (the `{scripts.map(script => (` block, lines 158–189) with the version below. Key changes:
- Add `draggable` and drag event props to the row div
- Add `GripVertical` handle as the first child (before the checkbox)
- Remove the `#{script.loadOrder}` badge
- Style the dragged item with reduced opacity and highlight the drop target with a top border

```tsx
                  {scripts.map((script, index) => (
                    <div
                      key={script.id}
                      draggable
                      onDragStart={() => handleDragStart(index)}
                      onDragOver={(e) => handleDragOver(e, index)}
                      onDrop={(e) => handleDrop(e, index)}
                      onDragEnd={handleDragEnd}
                      className={[
                        'flex items-center gap-3 p-3 border border-border rounded-md hover:bg-secondary/50 transition-opacity',
                        dragIndex === index ? 'opacity-40' : '',
                        dragOverIndex === index && dragIndex !== index ? 'border-t-2 border-t-primary' : '',
                      ].join(' ')}
                    >
                      <GripVertical
                        size={16}
                        className="shrink-0 text-muted-foreground cursor-grab active:cursor-grabbing"
                      />
                      <input
                        type="checkbox"
                        checked={script.enabled}
                        onChange={() => handleToggle(script)}
                        className="shrink-0"
                        title={script.enabled ? 'Disable script' : 'Enable script'}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="font-medium truncate">
                          {script.name || '(unnamed)'}
                        </div>
                        {script.description && (
                          <div className="text-sm text-muted-foreground truncate">
                            {script.description}
                          </div>
                        )}
                      </div>
                      <button
                        onClick={() => handleEdit(script)}
                        className="px-2 py-1 text-sm border border-border rounded hover:bg-secondary"
                      >
                        Edit
                      </button>
                    </div>
                  ))}
```

**Step 4: Verify TypeScript build**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build
```

Expected: exits 0 with no type errors.

**Step 5: Manual smoke test**

Run the app in dev mode:
```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run tauri dev
```

- Open a workspace → open Script Manager
- If there are 2+ scripts, drag a grip handle to reorder them
- Close and reopen the dialog — verify the new order persists
- Disable a script by unchecking it, then reorder — verify disabled scripts keep their position

**Step 6: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-desktop/src/components/ScriptManagerDialog.tsx
git -C /Users/careck/Source/Krillnotes commit -m "feat: add drag-handle reordering to Script Manager dialog"
```

---

### Task 4: Mark TODO as done

**Files:**
- Modify: `TODO.md`

Change:
```
[ ] user scripts have a loading order, but currently there is no way in the manage_script dialog to change this order. I think it would be cool if the order of scripts could be changed via drag handles.
```
to:
```
✅ DONE! user scripts have a loading order, but currently there is no way in the manage_script dialog to change this order. I think it would be cool if the order of scripts could be changed via drag handles.
```

Then commit:
```bash
git -C /Users/careck/Source/Krillnotes add TODO.md
git -C /Users/careck/Source/Krillnotes commit -m "chore: mark script reorder drag handles task as done"
```
