# Auto-select and Edit Mode on Note Creation — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When a note is created via AddNoteDialog, it is immediately selected in the tree and InfoPanel enters edit mode.

**Architecture:** Thread the new note's ID from AddNoteDialog back to WorkspaceView via the existing `onNoteCreated` callback, then use the existing `handleSelectNote` and `requestEditMode` mechanisms to select and enter edit mode.

**Tech Stack:** React (TypeScript), Tauri invoke for backend calls. No backend changes required.

---

### Task 1: Update `AddNoteDialog` to pass the new note ID to its callback

**Files:**
- Modify: `krillnotes-desktop/src/components/AddNoteDialog.tsx:4-10` (props interface)
- Modify: `krillnotes-desktop/src/components/AddNoteDialog.tsx:35-52` (handleCreate)

**Context:**
- The `invoke('create_note_with_type', ...)` call at line 40 already returns the created note from the backend — it's just being discarded with `await` and no capture.
- The `Note` type is defined in `../types` and has an `id: string` field.
- The `onNoteCreated` prop at line 7 is currently `() => void`.

**Step 1: Update the props interface**

In `AddNoteDialog.tsx`, change line 7 from:
```typescript
  onNoteCreated: () => void;
```
to:
```typescript
  onNoteCreated: (noteId: string) => void;
```

**Step 2: Add Note import**

At the top of `AddNoteDialog.tsx`, add the Note type import (after existing imports):
```typescript
import type { Note } from '../types';
```

**Step 3: Capture the returned note and pass its ID to the callback**

In `handleCreate` (lines 40–45), change:
```typescript
      await invoke('create_note_with_type', {
        parentId: hasNotes ? selectedNoteId : null,
        position: hasNotes ? position : 'child',
        nodeType
      });
      onNoteCreated();
```
to:
```typescript
      const note = await invoke<Note>('create_note_with_type', {
        parentId: hasNotes ? selectedNoteId : null,
        position: hasNotes ? position : 'child',
        nodeType
      });
      onNoteCreated(note.id);
```

**Step 4: Verify the file compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/AddNoteDialog.tsx
git commit -m "feat: pass new note ID through onNoteCreated callback"
```

---

### Task 2: Update `WorkspaceView` to auto-select and enter edit mode

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx:107-109` (handleNoteCreated)
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx:253` (onNoteCreated prop on AddNoteDialog)

**Context:**
- `handleSelectNote(noteId)` at line 89 sets `selectedNoteId` state and persists to backend.
- `requestEditMode` at line 38 is a counter; incrementing it signals `InfoPanel` to call `setIsEditing(true)` via a useEffect.
- `loadNotes()` is async and must complete before we select, so we `await` it first.
- The `onNoteCreated` prop passed to `<AddNoteDialog>` at line 253 still has the old type — TypeScript will error until it's updated too.

**Step 1: Update `handleNoteCreated` signature and body**

Change lines 107–109 from:
```typescript
  const handleNoteCreated = async () => {
    await loadNotes();
  };
```
to:
```typescript
  const handleNoteCreated = async (noteId: string) => {
    await loadNotes();
    await handleSelectNote(noteId);
    setRequestEditMode(prev => prev + 1);
  };
```

**Step 2: Verify the file compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors (the `onNoteCreated={handleNoteCreated}` prop at line 253 now matches the updated callback type automatically)

**Step 3: Manual smoke test**

Run the app: `cd krillnotes-desktop && npm run tauri dev`

Verify:
1. Open a workspace with existing notes
2. Press `Cmd+Shift+N` (or use the menu) to open Add Note dialog
3. Select a type and click Create
4. The dialog closes, the new note appears selected (highlighted) in the tree, and InfoPanel is immediately in edit mode (title input is focused, Edit button is not visible)
5. Right-click a note → Add Note → same behavior

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: auto-select new note and enter edit mode on creation"
```
