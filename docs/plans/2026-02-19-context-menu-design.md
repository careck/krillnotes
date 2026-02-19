# Context Menu for Tree Nodes — Design

**Date:** 2026-02-19

## Goal

Add a right-click context menu to each tree node with three actions: Add Note, Edit, Delete.

## Behaviour

- Right-clicking any tree node opens a small context menu at the cursor position.
- The menu has three items:
  - **Add Note** — selects the right-clicked note and opens the existing `AddNoteDialog` (the user picks position and type in the dialog as normal)
  - **Edit** — selects the note and puts the InfoPanel title field into edit mode with focus
  - **Delete** — selects the note and opens `DeleteConfirmDialog`
- The menu dismisses on click-outside or `Escape`.

## Architecture

### New file: `ContextMenu.tsx`

A portal-rendered overlay (`createPortal` into `document.body`) to avoid clipping from the tree panel's `overflow: hidden`.

Props:
```typescript
{
  x: number;
  y: number;
  noteId: string;
  onAddNote: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onClose: () => void;
}
```

Dismissal: a `mousedown` listener on `document` calls `onClose`; the menu's own `onMouseDown` stops propagation so clicking a menu item does not immediately dismiss. Each item's `onClick` calls its handler (which closes the menu as a side-effect via WorkspaceView state).

Menu items: Add Note, Edit, — (separator) —, Delete (destructive colour).

### WorkspaceView changes

**New state:**
```typescript
contextMenu: { x: number; y: number; noteId: string } | null
pendingDeleteId: string | null
pendingDeleteChildCount: number
showDeleteDialog: boolean
isDeleting: boolean
requestEditMode: number   // increment to signal InfoPanel to enter edit mode
```

**New imports:** `ContextMenu`, `DeleteConfirmDialog`

**New handlers:**
- `handleContextMenu(e, noteId)` — `e.preventDefault()`, sets `contextMenu`
- `handleContextAddNote(noteId)` — clears menu, calls `handleSelectNote(noteId)`, sets `showAddDialog = true`
- `handleContextEdit(noteId)` — clears menu, calls `handleSelectNote(noteId)`, increments `requestEditMode`
- `handleContextDelete(noteId)` — clears menu, calls `handleSelectNote(noteId)`, fetches `count_children`, sets pending delete state, sets `showDeleteDialog = true`
- `handleDeleteConfirm(strategy)` — calls `delete_note`, then `handleNoteUpdated`, clears delete state
- `handleDeleteCancel()` — clears delete state

**Render additions:**
- `<ContextMenu>` rendered when `contextMenu !== null`
- `<DeleteConfirmDialog>` rendered when `showDeleteDialog` (moved here from `InfoPanel`)

**Prop changes:**
- `TreeView`: add `onContextMenu` prop
- `InfoPanel`: add `onDeleteRequest(noteId)` prop and `requestEditMode: number` prop

### TreeView changes

Thread `onContextMenu: (e: React.MouseEvent, noteId: string) => void` through to each `TreeNode`.

### TreeNode changes

Add `onContextMenu` prop. On the row `<div>`:
```tsx
onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, node.note.id); }}
```

### InfoPanel changes

**Remove:** internal delete state (`showDeleteDialog`, `childCount`, `deleteTargetId`, `isDeleting`), delete handlers, `DeleteConfirmDialog` import and render.

**Add:**
- `onDeleteRequest: (noteId: string) => void` prop — delete button calls this instead
- `requestEditMode: number` prop
- `titleInputRef` on the title `<input>`

**New effects:**
```typescript
// Enter edit mode when WorkspaceView requests it
useEffect(() => {
  if (requestEditMode > 0 && selectedNote) setIsEditing(true);
}, [requestEditMode]);

// Focus title input when edit mode activates
useEffect(() => {
  if (isEditing && titleInputRef.current) titleInputRef.current.focus();
}, [isEditing]);
```

The existing `useEffect([selectedNote?.id])` resets `isEditing` to `false` when the selection changes; because React 18 batches state updates from effects and runs effects in definition order, the `requestEditMode` effect (defined after) wins and the final state is `isEditing = true` when edit is triggered via context menu on a newly-selected note.

### AddNoteDialog changes

None. The dialog's existing position radio buttons (child / sibling) handle placement.

## Files touched

| File | Change |
|------|--------|
| `src/components/ContextMenu.tsx` | **New** |
| `src/components/WorkspaceView.tsx` | Add context menu + delete state/handlers, render new components, update prop threading |
| `src/components/TreeView.tsx` | Thread `onContextMenu` prop |
| `src/components/TreeNode.tsx` | Add `onContextMenu` handler to row div |
| `src/components/InfoPanel.tsx` | Remove delete dialog, add `onDeleteRequest` + `requestEditMode` props, add title ref + focus effects |
