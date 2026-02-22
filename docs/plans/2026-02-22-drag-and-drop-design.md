# Drag and Drop Tree Reordering — Design

## Summary

Add mouse drag-and-drop to the note tree, allowing users to reorder notes among siblings and move notes (with all children) to a different parent — including to/from root level.

## Approach

Native HTML5 drag and drop — zero dependencies. Uses `draggable`, `dragstart`, `dragover`, `dragenter`, `drop`, and `dragend` events on existing `TreeNode` rows. No external library.

## Backend

### `Workspace::move_note(note_id, new_parent_id, new_position)`

A new method on `Workspace` that runs in a single SQLite transaction:

1. Read the note's current `parent_id` and `position`.
2. **Cycle check**: walk the ancestor chain of `new_parent_id` — if `note_id` appears, return an error.
3. **Self-move check**: if `note_id == new_parent_id`, return an error.
4. Decrement positions of all former siblings that came after the note (close the gap).
5. Increment positions of all new siblings at or after `new_position` (make room).
6. Update the note's `parent_id` to `new_parent_id` and `position` to `new_position`.
7. Log a `MoveNote` operation (the variant already exists in the `Operation` enum).
8. Purge old operations per the configured strategy.

### Tauri command

```rust
move_note(noteId: String, newParentId: Option<String>, newPosition: i32)
```

Registered in `generate_handler!` alongside existing commands.

## Frontend

### Drop zone detection

Each `TreeNode` row is both a drag source and a drop target. The drop position is determined by the cursor's vertical position within the row's bounding box:

```
┌─────────────────────────────┐
│  Top 25%    → insert BEFORE │  (sibling above)
├─────────────────────────────┤
│  Middle 50% → drop AS CHILD │  (reparent into this node)
├─────────────────────────────┤
│  Bottom 25% → insert AFTER  │  (sibling below)
└─────────────────────────────┘
```

### Visual indicators

- **Before/After**: A 2px blue horizontal line at the top or bottom edge of the row, indented to match the node's tree level.
- **As child**: The entire row gets a subtle blue background highlight.
- **Dragged node**: Reduced opacity (0.4) during the drag.

### Root-level drops

- Dropping on the empty space below the last tree node makes the note a root node at the end.
- The `TreeView` container acts as a drop target for this case.

### Position calculation

| Drop zone | `newParentId` | `newPosition` |
|-----------|---------------|---------------|
| Before node N | N's `parentId` | N's `position` |
| After node N | N's `parentId` | N's `position + 1` |
| As child of N | N's `id` | `0` (first child) |
| Root zone (bottom) | `null` | root count |

### Schema-sorted parents

When dropping onto a parent whose schema has `children_sort: "asc"` or `"desc"`, the drop is allowed and the note is reparented. The displayed position is determined by the sort order, not the manual position.

### Invalid drop prevention (client-side)

- Cannot drop a note onto itself.
- Cannot drop a note onto any of its own descendants (cycle prevention — walk the `parentId` chain in the flat notes array).

Both checks are also enforced server-side as a safety net.

### No-op optimization

If the computed `(newParentId, newPosition)` is identical to the note's current state, skip the backend call.

### After a successful move

Call `loadNotes()` to refresh the tree. The moved note stays selected.

When dropping "as child" onto a collapsed node, auto-expand it so the user sees the result.

### State management

Drag state is managed in `WorkspaceView` and passed down as props:

- `draggedNoteId: string | null` — set on `dragstart`, cleared on `dragend`
- `dropIndicator: { noteId: string, position: 'before' | 'after' | 'child' } | null` — updated on `dragover`, cleared on `drop`/`dragleave`/`dragend`

A helper `getDescendantIds(notes, noteId)` is added to `utils/tree.ts` for the client-side cycle check.

## Testing

Backend unit tests in `workspace.rs`:

- Move a note between siblings (reorder within same parent)
- Move a note to a different parent
- Move a note to root level (`new_parent_id: None`)
- Move a note to position 0 (first child)
- Cycle prevention: moving a parent into its own descendant returns an error
- Self-move returns an error
- Positions are gapless after each move
- A `MoveNote` operation is logged after each successful move

## Files changed

| File | Change |
|------|--------|
| `krillnotes-core/src/core/workspace.rs` | Add `move_note` method + unit tests |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Add `move_note` Tauri command, register in handler |
| `krillnotes-desktop/src/components/TreeNode.tsx` | Add drag source + drop target events, visual indicators |
| `krillnotes-desktop/src/components/TreeView.tsx` | Add root-level drop zone, pass drag props |
| `krillnotes-desktop/src/components/WorkspaceView.tsx` | Add drag state, `handleMoveNote` handler, pass props down |
| `krillnotes-desktop/src/utils/tree.ts` | Add `getDescendantIds` helper |
