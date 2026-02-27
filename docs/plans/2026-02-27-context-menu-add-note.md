# Context Menu Add Note Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split "Add Note" context menu into "Add Child" / "Add Sibling" / "Add Root Note" actions, remove the position-picker from `AddNoteDialog`, and skip the dialog entirely when only one valid type exists.

**Architecture:** Extract a shared `getAvailableTypes()` utility; refactor `AddNoteDialog` to accept a pre-determined position; update `ContextMenu` to support a "background" (no note selected) variant; add `onContextMenu` on the `TreeView` background with `stopPropagation` on `TreeNode` to prevent bubbling.

**Tech Stack:** React 18, TypeScript, Tauri v2 (`invoke`)

---

### Task 1: Extract `getAvailableTypes` utility

**Files:**
- Create: `krillnotes-desktop/src/utils/noteTypes.ts`

**Step 1: Create the utility file**

```typescript
import type { Note, SchemaInfo } from '../types';

export type NotePosition = 'child' | 'sibling' | 'root';

/**
 * Returns the note types that are valid to create at a given position.
 * - 'root'    : referenceNoteId ignored; types with no allowedParentTypes restriction
 * - 'child'   : referenceNoteId is the intended parent
 * - 'sibling' : referenceNoteId is the intended sibling (its parent becomes the effective parent)
 */
export function getAvailableTypes(
  position: NotePosition,
  referenceNoteId: string | null,
  notes: Note[],
  schemas: Record<string, SchemaInfo>
): string[] {
  const allTypes = Object.keys(schemas);

  if (position === 'root' || referenceNoteId === null) {
    return allTypes.filter(t => (schemas[t]?.allowedParentTypes ?? []).length === 0);
  }

  const referenceNote = notes.find(n => n.id === referenceNoteId);
  if (!referenceNote) return allTypes;

  let effectiveParentType: string | null;
  if (position === 'child') {
    effectiveParentType = referenceNote.nodeType;
  } else {
    // sibling: effective parent is referenceNote's parent
    const parentNote = notes.find(n => n.id === referenceNote.parentId);
    effectiveParentType = parentNote ? parentNote.nodeType : null;
  }

  return allTypes.filter(type => {
    const apt = schemas[type]?.allowedParentTypes ?? [];
    if (apt.length > 0) {
      if (effectiveParentType === null) return false;
      if (!apt.includes(effectiveParentType)) return false;
    }
    if (effectiveParentType !== null) {
      const act = schemas[effectiveParentType]?.allowedChildrenTypes ?? [];
      if (act.length > 0 && !act.includes(type)) return false;
    }
    return true;
  });
}
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors (file is not yet imported anywhere)

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/noteTypes.ts
git commit -m "feat: extract getAvailableTypes utility for note type filtering"
```

---

### Task 2: Refactor `AddNoteDialog` — remove position picker, accept position prop

**Files:**
- Modify: `krillnotes-desktop/src/components/AddNoteDialog.tsx`

**Step 1: Replace the props interface and internal state**

Replace the entire file with the following (keeps all logic, removes position state and `hasNotes` prop, imports utility):

```tsx
import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, SchemaInfo } from '../types';
import { getAvailableTypes, type NotePosition } from '../utils/noteTypes';

interface AddNoteDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onNoteCreated: (noteId: string) => void;
  referenceNoteId: string | null;  // null = creating root note
  position: NotePosition;
  notes: Note[];
  schemas: Record<string, SchemaInfo>;
}

function AddNoteDialog({ isOpen, onClose, onNoteCreated, referenceNoteId, position, notes, schemas }: AddNoteDialogProps) {
  const [nodeType, setNodeType] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [loading, setLoading] = useState(false);

  const availableTypes = useMemo(
    () => getAvailableTypes(position, referenceNoteId, notes, schemas),
    [position, referenceNoteId, notes, schemas]
  );

  useEffect(() => {
    if (availableTypes.length > 0 && !availableTypes.includes(nodeType)) {
      setNodeType(availableTypes[0]);
    }
  }, [availableTypes]);

  if (!isOpen) return null;

  const handleCreate = async () => {
    setLoading(true);
    setError('');
    try {
      const note = await invoke<Note>('create_note_with_type', {
        parentId: position === 'root' ? null : referenceNoteId,
        position: position === 'root' ? 'child' : position,
        nodeType,
      });
      onNoteCreated(note.id);
      onClose();
    } catch (err) {
      setError(`Failed to create note: ${err}`);
    } finally {
      setLoading(false);
    }
  };

  const title = position === 'root' ? 'Add Root Note'
    : position === 'child' ? 'Add Child Note'
    : 'Add Sibling Note';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{title}</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">Note Type</label>
          {availableTypes.length === 0 ? (
            <p className="text-sm text-amber-600 py-2">No note types are allowed at this position.</p>
          ) : (
            <select
              value={nodeType}
              onChange={(e) => setNodeType(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            >
              {availableTypes.map(type => (
                <option key={type} value={type}>{type}</option>
              ))}
            </select>
          )}
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={loading}
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={loading || !nodeType || availableTypes.length === 0}
          >
            {loading ? 'Creating...' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  );
}

export default AddNoteDialog;
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: errors in `WorkspaceView.tsx` because it still passes old props — that's fine, will fix in Task 5.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/AddNoteDialog.tsx
git commit -m "feat: refactor AddNoteDialog to accept position prop, remove position picker"
```

---

### Task 3: Add `stopPropagation` to `TreeNode` context menu handler

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx:193`

**Step 1: Add `stopPropagation`**

Find line 193:
```tsx
onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, node.note.id); }}
```

Change to:
```tsx
onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e, node.note.id); }}
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: same errors as before (WorkspaceView still broken), no new errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/TreeNode.tsx
git commit -m "fix: stopPropagation on TreeNode context menu to prevent background handler firing"
```

---

### Task 4: Add background `onContextMenu` to `TreeView`

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`

**Step 1: Add `onBackgroundContextMenu` to props interface**

Add to `TreeViewProps`:
```tsx
onBackgroundContextMenu: (e: React.MouseEvent) => void;
```

Add to the destructured parameter list in the function signature.

**Step 2: Add `onContextMenu` handler to both root divs**

Both the empty-tree `<div>` (line 63) and the non-empty root `<div>` (line 77) need:
```tsx
onContextMenu={(e) => { e.preventDefault(); onBackgroundContextMenu(e); }}
```

After the change, both divs look like:

```tsx
// Empty tree div (line 63):
<div
  className="flex items-center justify-center h-full text-muted-foreground text-sm focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
  tabIndex={0}
  onKeyDown={onKeyDown}
  onDragOver={handleRootDragOver}
  onDrop={handleRootDrop}
  onDragLeave={handleRootDragLeave}
  onContextMenu={(e) => { e.preventDefault(); onBackgroundContextMenu(e); }}
>
  No notes yet
</div>

// Non-empty root div (line 77):
<div
  className="h-full focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
  tabIndex={0}
  onKeyDown={onKeyDown}
  onDragOver={handleRootDragOver}
  onDrop={handleRootDrop}
  onDragLeave={handleRootDragLeave}
  onContextMenu={(e) => { e.preventDefault(); onBackgroundContextMenu(e); }}
>
```

**Step 3: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: new error in WorkspaceView about missing `onBackgroundContextMenu` prop — will fix in Task 6.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/TreeView.tsx
git commit -m "feat: add background onContextMenu to TreeView for root note creation"
```

---

### Task 5: Refactor `ContextMenu` — split items, support background variant

**Files:**
- Modify: `krillnotes-desktop/src/components/ContextMenu.tsx`

**Step 1: Replace the entire file**

```tsx
import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

interface ContextMenuProps {
  x: number;
  y: number;
  noteId: string | null;  // null = background (no note right-clicked)
  copiedNoteId: string | null;
  treeActions: string[];
  onAddChild: () => void;
  onAddSibling: () => void;
  onAddRoot: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onPasteAsChild: () => void;
  onPasteAsSibling: () => void;
  onTreeAction: (label: string) => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({
  x, y, noteId, copiedNoteId, treeActions,
  onAddChild, onAddSibling, onAddRoot,
  onEdit, onCopy, onPasteAsChild, onPasteAsSibling,
  onTreeAction, onDelete, onClose,
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('mousedown', handleMouseDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handleMouseDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={menuRef}
      className="fixed bg-background border border-secondary rounded shadow-lg z-50 py-1 min-w-[160px]"
      style={{ top: y, left: x }}
    >
      {noteId === null ? (
        // Background context menu — root note creation only
        <button
          className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
          onClick={() => { onAddRoot(); onClose(); }}
        >
          Add Root Note
        </button>
      ) : (
        // Note context menu
        <>
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onAddChild(); onClose(); }}
          >
            Add Child
          </button>
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onAddSibling(); onClose(); }}
          >
            Add Sibling
          </button>
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onEdit(); onClose(); }}
          >
            Edit
          </button>
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
          {treeActions.length > 0 && (
            <>
              <div className="border-t border-secondary my-1" />
              {treeActions.map((label) => (
                <button
                  key={label}
                  className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
                  onClick={() => { onTreeAction(label); onClose(); }}
                >
                  {label}
                </button>
              ))}
            </>
          )}
          <div className="border-t border-secondary my-1" />
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary text-red-500"
            onClick={() => { onDelete(); onClose(); }}
          >
            Delete
          </button>
        </>
      )}
    </div>,
    document.body
  );
}

export default ContextMenu;
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: errors in WorkspaceView (old props) — will fix next.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/ContextMenu.tsx
git commit -m "feat: split ContextMenu Add Note into Add Child/Sibling/Root Note"
```

---

### Task 6: Update `WorkspaceView` — wire up new props and skip-dialog logic

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Add imports**

At the top of the file, add:
```tsx
import { getAvailableTypes, type NotePosition } from '../utils/noteTypes';
```

**Step 2: Add dialog position/reference state**

Near `showAddDialog` state (around line 31), add:
```tsx
const [addDialogNoteId, setAddDialogNoteId] = useState<string | null>(null);
const [addDialogPosition, setAddDialogPosition] = useState<NotePosition>('child');
```

**Step 3: Add `openAddDialog` helper**

This helper handles the skip-dialog logic. Add near the other context menu handlers (around line 441):

```tsx
// Opens AddNoteDialog or creates directly if only one type is available
const openAddDialog = (position: NotePosition, referenceNoteId: string | null) => {
  const available = getAvailableTypes(position, referenceNoteId, notes, schemas);
  if (available.length === 0) return;
  if (available.length === 1) {
    const parentId = position === 'root' ? null : referenceNoteId;
    const tauriPosition = position === 'root' ? 'child' : position;
    invoke<Note>('create_note_with_type', { parentId, position: tauriPosition, nodeType: available[0] })
      .then(note => handleNoteCreated(note.id))
      .catch(err => console.error('Failed to create note:', err));
    return;
  }
  setAddDialogNoteId(referenceNoteId);
  setAddDialogPosition(position);
  setShowAddDialog(true);
};
```

**Step 4: Replace `handleContextAddNote` with three handlers**

Remove `handleContextAddNote` entirely. Add:

```tsx
const handleContextAddChild = (noteId: string) => {
  setContextMenu(null);
  openAddDialog('child', noteId);
};

const handleContextAddSibling = (noteId: string) => {
  setContextMenu(null);
  openAddDialog('sibling', noteId);
};

const handleContextAddRoot = () => {
  setContextMenu(null);
  openAddDialog('root', null);
};

const handleBackgroundContextMenu = (e: React.MouseEvent) => {
  setContextMenu({ x: e.clientX, y: e.clientY, noteId: null, noteType: '' });
};
```

**Step 5: Update contextMenu state type**

Change line 37:
```tsx
const [contextMenu, setContextMenu] = useState<{ x: number; y: number; noteId: string; noteType: string } | null>(null);
```
to:
```tsx
const [contextMenu, setContextMenu] = useState<{ x: number; y: number; noteId: string | null; noteType: string } | null>(null);
```

**Step 6: Update `Edit > Add Note` menu listener**

Around line 130, change:
```tsx
if (event.payload === 'Edit > Add Note clicked') {
  setShowAddDialog(true);
}
```
to:
```tsx
if (event.payload === 'Edit > Add Note clicked') {
  if (notes.length === 0) {
    openAddDialog('root', null);
  } else {
    openAddDialog('child', selectedNoteId);
  }
}
```

**Step 7: Update `AddNoteDialog` usage**

Find the `<AddNoteDialog ...>` block (around line 612) and replace its props:

```tsx
<AddNoteDialog
  isOpen={showAddDialog}
  onClose={() => setShowAddDialog(false)}
  onNoteCreated={handleNoteCreated}
  referenceNoteId={addDialogNoteId}
  position={addDialogPosition}
  notes={notes}
  schemas={schemas}
/>
```

**Step 8: Update `ContextMenu` usage**

Find the `<ContextMenu ...>` block (around line 624) and update props:

```tsx
{contextMenu && (
  <ContextMenu
    x={contextMenu.x}
    y={contextMenu.y}
    noteId={contextMenu.noteId}
    copiedNoteId={copiedNoteId}
    treeActions={contextMenu.noteId ? (treeActionMap[contextMenu.noteType] ?? []) : []}
    onAddChild={() => contextMenu.noteId && handleContextAddChild(contextMenu.noteId)}
    onAddSibling={() => contextMenu.noteId && handleContextAddSibling(contextMenu.noteId)}
    onAddRoot={handleContextAddRoot}
    onEdit={() => contextMenu.noteId && handleContextEdit(contextMenu.noteId)}
    onCopy={() => contextMenu.noteId && copyNote(contextMenu.noteId)}
    onPasteAsChild={() => pasteNote('child')}
    onPasteAsSibling={() => pasteNote('sibling')}
    onTreeAction={(label) => contextMenu.noteId && handleTreeAction(contextMenu.noteId, label)}
    onDelete={() => contextMenu.noteId && handleContextDelete(contextMenu.noteId)}
    onClose={() => setContextMenu(null)}
  />
)}
```

**Step 9: Pass `onBackgroundContextMenu` to `TreeView`**

Find the `<TreeView ...>` usage (around line 553) and add:
```tsx
onBackgroundContextMenu={handleBackgroundContextMenu}
```

**Step 10: Verify TypeScript compiles clean**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 11: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: wire up Add Child/Sibling/Root context menu actions with skip-dialog logic"
```

---

### Task 7: Smoke test

Start the app and verify all paths work:

```bash
cd krillnotes-desktop && npm run tauri dev
```

Test checklist:
1. **Right-click empty tree** → context menu shows "Add Root Note" only
   - With >1 root types: dialog opens showing type list (no position radio buttons)
   - With 1 root type: note created immediately, no dialog
2. **Right-click existing note → Add Child**
   - With >1 child types: dialog opens with title "Add Child Note"
   - With 1 child type: note created immediately
3. **Right-click existing note → Add Sibling**
   - Same skip-dialog behaviour
4. **Right-click existing note** → no "Add Note" item (it's gone); Edit / Copy / Paste / Delete still present
5. **Edit > Add Note menu bar item** → still works (opens dialog or skips)
6. **Right-click on note does NOT trigger background context menu** (stopPropagation check)

**Step 2: Commit any fixes found during smoke test**

```bash
git add -p
git commit -m "fix: <describe what was fixed>"
```
