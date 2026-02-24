# Design: Copy and Paste of Notes

**Date:** 2026-02-24
**Feature:** Deep copy of notes and their descendants via context menu, Edit menu, and keyboard shortcuts.

---

## Summary

Allow users to copy any note (and its entire subtree of children) and paste it as a child or sibling of any compatible target note. Cut-and-paste is not needed because note moving is already covered by drag-and-drop.

---

## User-Facing Behaviour

### Copy
- Copies the selected note and all its descendants (deep copy).
- The original note is not modified in any way.
- The clipboard state is per-workspace-window (React state). Copying in one workspace window does not affect another.

### Paste
- Two paste modes: **Paste as Child** (most common) and **Paste as Sibling**.
- Pastes a full deep copy with fresh UUIDs and new timestamps.
- Schema constraints (`allowedParentTypes`, `allowedChildrenTypes`) are validated for the root of the pasted subtree against the target. Children's internal relationships are trusted and not re-validated.
- If the target location violates a schema constraint, a `StatusMessage` error is shown and nothing is pasted.
- After a successful paste the newly pasted root note is selected and the tree scrolls to reveal it.

### Access Points

| Action | Context menu | Edit menu | Keyboard (tree focused) |
|---|---|---|---|
| Copy Note | ✓ | ✓ | Cmd+C |
| Paste as Child | ✓ (greyed if empty) | ✓ (greyed if empty) | Cmd+V |
| Paste as Sibling | ✓ (greyed if empty) | ✓ (greyed if empty) | Cmd+Shift+V |

Keyboard shortcuts only fire when focus is **not** on an `<input>` or `<textarea>` element, so system text copy/paste (Cmd+C / Cmd+V) continues to work normally in text fields.

The Edit menu paste items have **no native accelerator** to avoid conflicting with `PredefinedMenuItem::copy` and `PredefinedMenuItem::paste` (which handle OS-level text copy/paste and remain in the menu unchanged). The keyboard shortcuts are handled entirely by a React `keydown` listener.

---

## Architecture

### Backend — `krillnotes-core/src/core/workspace.rs`

New method:

```rust
pub fn deep_copy_note(
    &mut self,
    source_id: &str,
    target_id: &str,
    position: AddPosition,
) -> Result<String>  // returns ID of new root note
```

Steps:
1. Load the full subtree rooted at `source_id` by querying all notes with the source as ancestor (recursive CTE or iterative BFS).
2. Validate the paste location for the root note using the existing `allowedParentTypes` / `allowedChildrenTypes` checks (same logic as `create_note`).
3. Build a remapping table: `old_uuid → new_uuid` for every note in the subtree.
4. Clone each note with: new UUID, new `created_at` / `modified_at` timestamps, remapped `parent_id`, same `node_type`, `title`, `fields`, and `is_expanded`.
5. Set the new root note's `parent_id` and `position` based on `target_id` + `AddPosition`. Bump sibling positions if inserting as sibling (same as `create_note`).
6. Insert all cloned notes in a **single transaction**, logging a `CreateNote` operation for each.
7. Return the new root note's ID.

New Tauri command in `krillnotes-desktop/src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn deep_copy_note(workspace_id, source_note_id, target_note_id, position) -> Result<String, String>
```

### Frontend — Clipboard State

`copiedNoteId: string | null` added to `WorkspaceView.tsx` local state. This is naturally per-window (each workspace window is an independent React app instance).

Whenever `copiedNoteId` changes, the frontend calls a new Tauri command `set_paste_menu_enabled(enabled: bool)` to update the native Edit menu item states. It also calls this on `WindowEvent` focus gain so that switching between workspace windows re-syncs the global macOS menu bar.

### Frontend — Context Menu (`ContextMenu.tsx`)

New props: `copiedNoteId: string | null`, `onCopy`, `onPasteAsChild`, `onPasteAsSibling`.

"Paste as Child" and "Paste as Sibling" render with `opacity-40 pointer-events-none` CSS classes when `copiedNoteId` is null.

Layout:

```
┌───────────────────┐
│ Add Note          │
│ Edit              │
│ Copy Note         │
│ Paste as Child    │  ← greyed if copiedNoteId is null
│ Paste as Sibling  │  ← greyed if copiedNoteId is null
├───────────────────┤
│ Delete            │
└───────────────────┘
```

### Frontend — Keyboard Shortcuts (`WorkspaceView.tsx`)

`keydown` event listener at document level:

```ts
if (e.key === 'c' && (e.metaKey || e.ctrlKey) && !e.shiftKey) {
  if (!isInputFocused()) { copySelectedNote(); e.preventDefault(); }
}
if (e.key === 'v' && (e.metaKey || e.ctrlKey) && !e.shiftKey) {
  if (!isInputFocused()) { pasteAsChild(); e.preventDefault(); }
}
if (e.key === 'v' && (e.metaKey || e.ctrlKey) && e.shiftKey) {
  if (!isInputFocused()) { pasteAsSibling(); e.preventDefault(); }
}
```

`isInputFocused()` checks `document.activeElement` against `INPUT`, `TEXTAREA`, and `[contenteditable]`.

### Edit Menu — `menu.rs`

Three new custom items inserted between the Delete Note separator and the Undo/Redo block:

```
Add Note             ⌘⇧N
Delete Note          ⌘⌫
─────────────────────────
Copy Note                   ← no accelerator
Paste as Child              ← no accelerator
Paste as Sibling            ← no accelerator
─────────────────────────
Undo                 ⌘Z
Redo                 ⌘⇧Z
Copy                 ⌘C    (system text copy, unchanged)
Paste                ⌘V    (system text paste, unchanged)
```

IDs: `edit_copy_note`, `edit_paste_as_child`, `edit_paste_as_sibling`.

`handle_menu_event` in `lib.rs` maps these IDs to `menu-action` events routed to the focused window (existing pattern). The frontend's `menu-action` listener calls the same handlers as keyboard shortcuts.

### Dynamic Menu Greying — `lib.rs` + `AppState`

To enable/disable native menu items at runtime, MenuItem handles must be stored in `AppState`.

**macOS** (global menu bar, shared across all windows):
- `AppState` holds `Arc<Mutex<(MenuItem, MenuItem)>>` for the two paste items.
- `set_paste_menu_enabled(enabled: bool)` command calls `.set_enabled(enabled)` on both.
- On `WindowEvent::Focused(true)` the focused window's frontend immediately calls `set_paste_menu_enabled` with its current `copiedNoteId != null` state to re-sync the global menu.

**Windows** (per-window menu bar):
- `AppState` holds `Arc<Mutex<HashMap<String, (MenuItem, MenuItem)>>>` keyed by window label.
- `set_paste_menu_enabled(enabled: bool)` command resolves the calling window's label and updates only that window's handles.
- Window labels are available from the Tauri command invocation context.

---

## Error Handling

- Schema constraint violation on paste → `StatusMessage` error, no data written.
- Source note not found (e.g. deleted between copy and paste) → `StatusMessage` error.
- Database error during deep copy → transaction rolled back, `StatusMessage` error.

---

## What Is Not In Scope

- Cut-and-paste (move is already covered by drag-and-drop).
- Cross-workspace paste (the copied note ID is per-window React state; schemas may not exist in another workspace).
- Persisting the clipboard across app restarts.
