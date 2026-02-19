# Context Menu Implementation Plan

> **Status:** ✅ COMPLETED 2026-02-19 — all 4 tasks implemented, TypeScript clean, manually verified.

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a right-click context menu to each tree node with Add Note, Edit, and Delete actions.

**Architecture:** A new `ContextMenu` component renders via a React portal into `document.body` (avoiding `overflow: hidden` clipping). All context menu + delete state lives in `WorkspaceView` (Approach A from design). The `DeleteConfirmDialog` is lifted from `InfoPanel` to `WorkspaceView`; `InfoPanel` gets a `requestEditMode` counter prop to signal when it should enter edit mode.

**Tech Stack:** React 18, TypeScript, Tauri v2, Tailwind CSS. No frontend test suite — verification is manual (run `npm run tauri dev` in `krillnotes-desktop/`).

---

### Task 1: Create `ContextMenu.tsx`

**Files:**
- Create: `krillnotes-desktop/src/components/ContextMenu.tsx`

**Step 1: Create the file with this exact content**

```tsx
import { useEffect } from 'react';
import { createPortal } from 'react-dom';

interface ContextMenuProps {
  x: number;
  y: number;
  noteId: string;
  onAddNote: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({ x, y, onAddNote, onEdit, onDelete, onClose }: ContextMenuProps) {
  useEffect(() => {
    const handleMouseDown = () => onClose();
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
      className="fixed bg-background border border-secondary rounded shadow-lg z-50 py-1 min-w-[160px]"
      style={{ top: y, left: x }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
        onClick={onAddNote}
      >
        Add Note
      </button>
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
        onClick={onEdit}
      >
        Edit
      </button>
      <div className="border-t border-secondary my-1" />
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary text-red-500"
        onClick={onDelete}
      >
        Delete
      </button>
    </div>,
    document.body
  );
}

export default ContextMenu;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/ContextMenu.tsx
git commit -m "feat(frontend): add ContextMenu component"
```

---

### Task 2: Thread `onContextMenu` through `TreeNode` and `TreeView`

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx`
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`

**Step 1: Update `TreeNode.tsx`**

Replace the entire file with:

```tsx
import type { TreeNode as TreeNodeType } from '../types';

interface TreeNodeProps {
  node: TreeNodeType;
  selectedNoteId: string | null;
  level: number;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
}

function TreeNode({ node, selectedNoteId, level, onSelect, onToggleExpand, onContextMenu }: TreeNodeProps) {
  const hasChildren = node.children.length > 0;
  const isSelected = node.note.id === selectedNoteId;
  const isExpanded = node.note.isExpanded;

  return (
    <div>
      <div
        className={`flex items-center py-1 px-2 cursor-pointer hover:bg-secondary/50 ${
          isSelected ? 'bg-secondary' : ''
        }`}
        style={{ paddingLeft: `${level * 20 + 8}px` }}
        onClick={() => onSelect(node.note.id)}
        onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, node.note.id); }}
      >
        {hasChildren && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onToggleExpand(node.note.id);
            }}
            className="mr-1 text-muted-foreground hover:text-foreground"
          >
            {isExpanded ? '▼' : '▶'}
          </button>
        )}
        {!hasChildren && <span className="w-4 mr-1" />}
        <span className="text-sm truncate">{node.note.title}</span>
      </div>

      {hasChildren && isExpanded && (
        <div>
          {node.children.map(child => (
            <TreeNode
              key={child.note.id}
              node={child}
              selectedNoteId={selectedNoteId}
              level={level + 1}
              onSelect={onSelect}
              onToggleExpand={onToggleExpand}
              onContextMenu={onContextMenu}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export default TreeNode;
```

**Step 2: Update `TreeView.tsx`**

Replace the entire file with:

```tsx
import TreeNode from './TreeNode';
import type { TreeNode as TreeNodeType } from '../types';

interface TreeViewProps {
  tree: TreeNodeType[];
  selectedNoteId: string | null;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
}

function TreeView({ tree, selectedNoteId, onSelect, onToggleExpand, onContextMenu }: TreeViewProps) {
  if (tree.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
        No notes yet
      </div>
    );
  }

  return (
    <div className="overflow-y-auto h-full">
      {tree.map(node => (
        <TreeNode
          key={node.note.id}
          node={node}
          selectedNoteId={selectedNoteId}
          level={0}
          onSelect={onSelect}
          onToggleExpand={onToggleExpand}
          onContextMenu={onContextMenu}
        />
      ))}
    </div>
  );
}

export default TreeView;
```

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/TreeNode.tsx krillnotes-desktop/src/components/TreeView.tsx
git commit -m "feat(frontend): thread onContextMenu prop through TreeView and TreeNode"
```

---

### Task 3: Refactor `InfoPanel` — lift delete dialog, add `requestEditMode`

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Replace the entire file with this updated version**

Key changes:
- Remove `DeleteConfirmDialog`, `DeleteResult`, `DeleteStrategy` imports
- Remove delete state (`showDeleteDialog`, `childCount`, `deleteTargetId`, `isDeleting`) and delete handlers
- Add `onDeleteRequest` and `requestEditMode` props
- Add `titleInputRef` on the title input
- Add two new effects for edit-mode triggering and input focus

```tsx
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, FieldDefinition, FieldValue } from '../types';
import FieldDisplay from './FieldDisplay';
import FieldEditor from './FieldEditor';

interface InfoPanelProps {
  selectedNote: Note | null;
  onNoteUpdated: () => void;
  onDeleteRequest: (noteId: string) => void;
  requestEditMode: number;
}

function InfoPanel({ selectedNote, onNoteUpdated, onDeleteRequest, requestEditMode }: InfoPanelProps) {
  const [schemaFields, setSchemaFields] = useState<FieldDefinition[]>([]);
  const [isEditing, setIsEditing] = useState(false);
  const [editedTitle, setEditedTitle] = useState('');
  const [editedFields, setEditedFields] = useState<Record<string, FieldValue>>({});
  const [isDirty, setIsDirty] = useState(false);
  const titleInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!selectedNote) {
      setSchemaFields([]);
      setIsEditing(false);
      return;
    }

    invoke<FieldDefinition[]>('get_schema_fields', { nodeType: selectedNote.nodeType })
      .then(fields => setSchemaFields(fields))
      .catch(err => {
        console.error('Failed to fetch schema fields:', err);
        setSchemaFields([]);
      });
  }, [selectedNote?.id]);

  useEffect(() => {
    if (selectedNote) {
      setIsEditing(false);
      setEditedTitle(selectedNote.title);
      setEditedFields({ ...selectedNote.fields });
      setIsDirty(false);
    }
  }, [selectedNote?.id]);

  // Enter edit mode when WorkspaceView requests it (e.g. via context menu "Edit")
  useEffect(() => {
    if (requestEditMode > 0 && selectedNote) {
      setIsEditing(true);
    }
  }, [requestEditMode]);

  // Focus title input whenever edit mode activates
  useEffect(() => {
    if (isEditing && titleInputRef.current) {
      titleInputRef.current.focus();
    }
  }, [isEditing]);

  const handleEdit = () => {
    setIsEditing(true);
  };

  const handleCancel = () => {
    if (isDirty) {
      if (!confirm('Discard changes?')) {
        return;
      }
    }
    setIsEditing(false);
    setEditedTitle(selectedNote!.title);
    setEditedFields({ ...selectedNote!.fields });
    setIsDirty(false);
  };

  const handleSave = async () => {
    if (!selectedNote) return;

    try {
      await invoke('update_note', {
        noteId: selectedNote.id,
        title: editedTitle,
        fields: editedFields,
      });
      setIsEditing(false);
      setIsDirty(false);
      onNoteUpdated();
    } catch (err) {
      alert(`Failed to save: ${err}`);
    }
  };

  const handleFieldChange = (fieldName: string, value: FieldValue) => {
    setEditedFields(prev => ({ ...prev, [fieldName]: value }));
    setIsDirty(true);
  };

  if (!selectedNote) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Select a note to view details
      </div>
    );
  }

  const formatTimestamp = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleString();
  };

  const schemaFieldNames = new Set(schemaFields.map(f => f.name));
  const allFieldNames = Object.keys(selectedNote.fields);
  const legacyFieldNames = allFieldNames.filter(name => !schemaFieldNames.has(name));

  return (
    <div className={`p-6 ${isEditing ? 'border-2 border-primary rounded-lg' : ''}`}>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        {isEditing ? (
          <input
            ref={titleInputRef}
            type="text"
            value={editedTitle}
            onChange={(e) => {
              setEditedTitle(e.target.value);
              setIsDirty(true);
            }}
            className="text-4xl font-bold bg-background border border-border rounded-md px-2 py-1 flex-1"
          />
        ) : (
          <h1 className="text-4xl font-bold">{selectedNote.title}</h1>
        )}
        <div className="flex gap-2 ml-4">
          {isEditing ? (
            <>
              <button
                onClick={handleSave}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                Save
              </button>
              <button
                onClick={handleCancel}
                className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              <button
                onClick={handleEdit}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                Edit
              </button>
              <button
                onClick={() => onDeleteRequest(selectedNote.id)}
                className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
              >
                Delete
              </button>
            </>
          )}
        </div>
      </div>

      {/* Fields Section */}
      <div className="mb-6">
        <h2 className="text-xl font-semibold mb-4">Fields</h2>

        {schemaFields.map(field => (
          isEditing ? (
            <FieldEditor
              key={field.name}
              fieldName={field.name}
              value={editedFields[field.name] || { Text: '' }}
              required={field.required}
              onChange={(value) => handleFieldChange(field.name, value)}
            />
          ) : (
            <FieldDisplay
              key={field.name}
              fieldName={field.name}
              value={selectedNote.fields[field.name] || { Text: '' }}
            />
          )
        ))}

        {legacyFieldNames.length > 0 && (
          <>
            <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
              Legacy Fields
            </h3>
            {legacyFieldNames.map(name => (
              isEditing ? (
                <FieldEditor
                  key={name}
                  fieldName={`${name} (legacy)`}
                  value={editedFields[name] || { Text: '' }}
                  required={false}
                  onChange={(value) => handleFieldChange(name, value)}
                />
              ) : (
                <FieldDisplay
                  key={name}
                  fieldName={`${name} (legacy)`}
                  value={selectedNote.fields[name]}
                />
              )
            ))}
          </>
        )}

        {schemaFields.length === 0 && legacyFieldNames.length === 0 && (
          <p className="text-muted-foreground italic">No fields</p>
        )}
      </div>

      {/* Metadata Section */}
      <div className="bg-secondary p-6 rounded-lg space-y-4">
        <div>
          <p className="text-sm text-muted-foreground">Type</p>
          <p className="text-lg">{selectedNote.nodeType}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">Created</p>
          <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">Modified</p>
          <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">ID</p>
          <p className="text-xs font-mono">{selectedNote.id}</p>
        </div>
      </div>
    </div>
  );
}

export default InfoPanel;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "refactor(frontend): lift delete dialog out of InfoPanel, add requestEditMode prop"
```

---

### Task 4: Wire `WorkspaceView` — add context menu + delete state, render new components

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Replace the entire file with this updated version**

Key changes:
- Import `ContextMenu`, `DeleteConfirmDialog`, `DeleteStrategy`, `DeleteResult`
- Add context menu state and delete state
- Add five new handlers: `handleContextMenu`, `handleContextAddNote`, `handleContextEdit`, `handleContextDelete`, `handleDeleteRequest`, `handleDeleteConfirm`, `handleDeleteCancel`
- Pass `onContextMenu` to `TreeView`
- Pass `onDeleteRequest` and `requestEditMode` to `InfoPanel`
- Render `ContextMenu` and `DeleteConfirmDialog`

```tsx
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import TreeView from './TreeView';
import InfoPanel from './InfoPanel';
import AddNoteDialog from './AddNoteDialog';
import ContextMenu from './ContextMenu';
import DeleteConfirmDialog from './DeleteConfirmDialog';
import type { Note, TreeNode, WorkspaceInfo, DeleteResult } from '../types';
import { DeleteStrategy } from '../types';
import { buildTree } from '../utils/tree';

interface WorkspaceViewProps {
  workspaceInfo: WorkspaceInfo;
}

function WorkspaceView({ workspaceInfo }: WorkspaceViewProps) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [selectedNoteId, setSelectedNoteId] = useState<string | null>(null);
  const selectedNoteIdRef = useRef(selectedNoteId);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [error, setError] = useState<string>('');
  const selectionInitialized = useRef(false);
  const isRefreshing = useRef(false);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; noteId: string } | null>(null);

  // Delete dialog state (lifted from InfoPanel)
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [pendingDeleteChildCount, setPendingDeleteChildCount] = useState(0);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);

  // Incremented to signal InfoPanel to enter edit mode
  const [requestEditMode, setRequestEditMode] = useState(0);

  selectedNoteIdRef.current = selectedNoteId;

  // Load notes on mount
  useEffect(() => {
    loadNotes();
  }, []);

  // Set up menu listener
  useEffect(() => {
    const unlisten = listen<string>('menu-action', async (event) => {
      const isFocused = await getCurrentWebviewWindow().isFocused();
      if (!isFocused) return;

      if (event.payload === 'Edit > Add Note clicked') {
        setShowAddDialog(true);
      }
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  const loadNotes = async (): Promise<Note[]> => {
    try {
      const fetchedNotes = await invoke<Note[]>('list_notes');
      setNotes(fetchedNotes);

      const builtTree = buildTree(fetchedNotes);
      setTree(builtTree);

      if (!selectionInitialized.current) {
        selectionInitialized.current = true;
        if (workspaceInfo.selectedNoteId) {
          setSelectedNoteId(workspaceInfo.selectedNoteId);
        } else if (builtTree.length > 0) {
          const firstRootId = builtTree[0].note.id;
          setSelectedNoteId(firstRootId);
          await invoke('set_selected_note', { noteId: firstRootId });
        }
      }

      return fetchedNotes;
    } catch (err) {
      setError(`Failed to load notes: ${err}`);
      return [];
    }
  };

  const handleSelectNote = async (noteId: string) => {
    setSelectedNoteId(noteId);
    try {
      await invoke('set_selected_note', { noteId });
    } catch (err) {
      console.error('Failed to save selection:', err);
    }
  };

  const handleToggleExpand = async (noteId: string) => {
    try {
      await invoke('toggle_note_expansion', { noteId });
      await loadNotes();
    } catch (err) {
      console.error('Failed to toggle expansion:', err);
    }
  };

  const handleNoteCreated = async () => {
    await loadNotes();
  };

  const handleNoteUpdated = async () => {
    if (isRefreshing.current) return;
    isRefreshing.current = true;
    try {
      const currentId = selectedNoteIdRef.current;
      const freshNotes = await loadNotes();

      if (currentId && !freshNotes.some(n => n.id === currentId)) {
        const freshTree = buildTree(freshNotes);
        const firstId = freshTree.length > 0 ? freshTree[0].note.id : null;

        if (firstId) {
          setSelectedNoteId(firstId);
          try {
            await invoke('set_selected_note', { noteId: firstId });
          } catch (err) {
            console.error('Failed to save auto-selection:', err);
          }
        } else {
          setSelectedNoteId(null);
        }
      }
    } finally {
      isRefreshing.current = false;
    }
  };

  // --- Context menu handlers ---

  const handleContextMenu = (e: React.MouseEvent, noteId: string) => {
    setContextMenu({ x: e.clientX, y: e.clientY, noteId });
  };

  const handleContextAddNote = (noteId: string) => {
    setContextMenu(null);
    setSelectedNoteId(noteId);
    setShowAddDialog(true);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
  };

  const handleContextEdit = (noteId: string) => {
    setContextMenu(null);
    setSelectedNoteId(noteId);
    setRequestEditMode(prev => prev + 1);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
  };

  const handleContextDelete = (noteId: string) => {
    setContextMenu(null);
    setSelectedNoteId(noteId);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
    handleDeleteRequest(noteId);
  };

  // --- Delete handlers (lifted from InfoPanel) ---

  const handleDeleteRequest = async (noteId: string) => {
    try {
      const count = await invoke<number>('count_children', { noteId });
      setPendingDeleteChildCount(count);
      setPendingDeleteId(noteId);
      setShowDeleteDialog(true);
    } catch (err) {
      alert(`Failed to check children: ${err}`);
    }
  };

  const handleDeleteConfirm = async (strategy: DeleteStrategy) => {
    if (!pendingDeleteId || isDeleting) return;
    setIsDeleting(true);
    try {
      await invoke<DeleteResult>('delete_note', {
        noteId: pendingDeleteId,
        strategy,
      });
      setShowDeleteDialog(false);
      setPendingDeleteId(null);
      setIsDeleting(false);
      handleNoteUpdated();
    } catch (err) {
      alert(`Failed to delete: ${err}`);
      setShowDeleteDialog(false);
      setPendingDeleteId(null);
      setIsDeleting(false);
    }
  };

  const handleDeleteCancel = () => {
    setShowDeleteDialog(false);
    setPendingDeleteId(null);
    setIsDeleting(false);
  };

  const selectedNote = selectedNoteId
    ? notes.find(n => n.id === selectedNoteId) || null
    : null;

  const pendingDeleteNote = pendingDeleteId
    ? notes.find(n => n.id === pendingDeleteId) || null
    : null;

  if (error) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-red-500">{error}</div>
      </div>
    );
  }

  return (
    <div className="flex h-screen">
      {/* Left sidebar - Tree */}
      <div className="w-[300px] border-r border-secondary bg-background overflow-hidden">
        <TreeView
          tree={tree}
          selectedNoteId={selectedNoteId}
          onSelect={handleSelectNote}
          onToggleExpand={handleToggleExpand}
          onContextMenu={handleContextMenu}
        />
      </div>

      {/* Right panel - Info */}
      <div className="flex-1 overflow-y-auto">
        <InfoPanel
          selectedNote={selectedNote}
          onNoteUpdated={handleNoteUpdated}
          onDeleteRequest={handleDeleteRequest}
          requestEditMode={requestEditMode}
        />
      </div>

      {/* Add Note Dialog */}
      <AddNoteDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onNoteCreated={handleNoteCreated}
        selectedNoteId={selectedNoteId}
        hasNotes={notes.length > 0}
      />

      {/* Context Menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          noteId={contextMenu.noteId}
          onAddNote={() => handleContextAddNote(contextMenu.noteId)}
          onEdit={() => handleContextEdit(contextMenu.noteId)}
          onDelete={() => handleContextDelete(contextMenu.noteId)}
          onClose={() => setContextMenu(null)}
        />
      )}

      {/* Delete Confirm Dialog (handles both InfoPanel button and context menu) */}
      {showDeleteDialog && pendingDeleteNote && (
        <DeleteConfirmDialog
          noteTitle={pendingDeleteNote.title}
          childCount={pendingDeleteChildCount}
          onConfirm={handleDeleteConfirm}
          onCancel={handleDeleteCancel}
          disabled={isDeleting}
        />
      )}
    </div>
  );
}

export default WorkspaceView;
```

**Step 2: Build and verify TypeScript compiles cleanly**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Run the app and manually verify**

```bash
npm run tauri dev
```

Verify:
- Right-click any tree node → context menu appears at cursor with "Add Note", "Edit", "Delete"
- Click outside or press Escape → menu dismisses
- **Add Note**: right-click a node → Add Note → `AddNoteDialog` opens with that node selected
- **Edit**: right-click a node → Edit → InfoPanel shows that note with title input focused and in edit mode
- **Delete**: right-click a node → Delete → `DeleteConfirmDialog` appears; confirm deletes the note
- InfoPanel Delete button still works (uses the same `handleDeleteRequest` path)
- InfoPanel Edit button still works as before

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat(frontend): wire context menu into WorkspaceView, lift delete dialog from InfoPanel"
```

---

## Summary

**Context Menu Implementation Complete!**

**Delivered:**
- ✅ `ContextMenu` component rendering via React portal into `document.body`
- ✅ Dismiss on click-outside (ref-based) and Escape key
- ✅ `onContextMenu` prop threaded through `TreeView` → `TreeNode`
- ✅ `InfoPanel` refactored: delete dialog lifted out, `requestEditMode` counter prop added
- ✅ `WorkspaceView` wired: context menu state, delete state, all handlers
- ✅ TypeScript compiles cleanly (`npx tsc --noEmit` — no errors)
- ✅ Manually verified: right-click Add Note / Edit / Delete all work; InfoPanel buttons unchanged

**Tasks Completed:** 4

**Commits:**
- `feat(frontend): add ContextMenu component`
- `fix(frontend): fix ContextMenu dismiss logic and remove unused noteId prop`
- `feat(frontend): thread onContextMenu prop through TreeView and TreeNode`
- `refactor(frontend): lift delete dialog out of InfoPanel, add requestEditMode prop`
- `feat(frontend): wire context menu into WorkspaceView, lift delete dialog from InfoPanel`
