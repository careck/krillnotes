# Design: Context Menu Add Note (Issues #24 + #25)

## Summary

Improve note creation UX by splitting the generic "Add Note" context menu action into
position-specific actions ("Add Child", "Add Sibling", "Add Root Note") and removing
the position-picker step from `AddNoteDialog`. When only one valid type exists for a
given position, skip the dialog entirely and create the note immediately.

## Approach

Refactor `AddNoteDialog` in-place (no new files). The dialog loses its position picker
and becomes a pure type-picker. The caller is always responsible for knowing the
position before opening the dialog.

## Component Changes

| File | Change |
|---|---|
| `AddNoteDialog.tsx` | Remove position picker; accept `position` prop; skip rendering if only 1 type (fire command directly) |
| `ContextMenu.tsx` | Split "Add Note" → "Add Child" + "Add Sibling"; accept nullable `noteId` to support root context |
| `TreeView.tsx` | Add `onContextMenu` on background div; TreeNode adds `stopPropagation` so background handler only fires for genuine empty-space clicks |
| `WorkspaceView.tsx` | Wire new context menu items; pass determined `position` into dialog |

## Interaction Flow

### Right-click on existing note
1. `TreeNode.onContextMenu` fires with `note.id` + `stopPropagation()`
2. Context menu shows: **Add Child**, **Add Sibling**, Edit, Copy, Paste, Delete
3. On "Add Child" or "Add Sibling": filter valid types for that position + parent
   - 1 valid type → call `create_note_with_type` directly, no dialog
   - >1 valid types → open simplified `AddNoteDialog` with position pre-set

### Right-click on empty tree background
1. `TreeView` background `onContextMenu` fires (bubbling blocked by TreeNode `stopPropagation`)
2. Context menu shows: **Add Root Note** only
3. On click: filter types with empty `allowedParentTypes`
   - 1 type → create directly
   - >1 types → open `AddNoteDialog` with `position="root"`

### `AddNoteDialog` (simplified)
- Accepts `position: "child" | "sibling" | "root"` prop (replaces internal position state)
- Renders type list + confirm button only
- If invoked with exactly 1 valid type, fires the Tauri command immediately and never renders

## Out of Scope
- Keyboard shortcut for Add Child / Add Sibling
- Context menu submenus (types as inline submenu items)
- Changes to the `Edit > Add Note` menu bar item (can be updated in a follow-up)
