# Drag and Drop Tree Reordering — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable drag-and-drop reordering and reparenting of notes in the tree.

**Architecture:** A new `Workspace::move_note` method handles the atomic move in SQLite (gap closing, gap opening, parent/position update, operation logging). A new Tauri command exposes it. The frontend uses native HTML5 drag events on TreeNode rows with three drop zones (before/child/after) and visual indicators.

**Tech Stack:** Rust/rusqlite (backend), Tauri v2 command (bridge), React/TypeScript with native HTML5 DnD (frontend)

---

### Task 1: Add `InvalidMove` error variant

**Files:**
- Modify: `krillnotes-core/src/core/error.rs`

**Step 1: Add the variant**

In `error.rs`, add a new variant to `KrillnotesError` after `ValidationFailed`:

```rust
    /// A move operation would create a cycle or is otherwise invalid.
    #[error("Invalid move: {0}")]
    InvalidMove(String),
```

And add the user_message arm in the `user_message()` method:

```rust
            Self::InvalidMove(msg) => msg.clone(),
```

**Step 2: Verify it compiles**

Run: `cargo check -p krillnotes-core`
Expected: compiles cleanly

**Step 3: Commit**

```bash
git add krillnotes-core/src/core/error.rs
git commit -m "feat: add InvalidMove error variant"
```

---

### Task 2: Implement `Workspace::move_note` with tests

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write the failing tests**

Add these tests at the bottom of the `mod tests` block in `workspace.rs`:

```rust
    // ── move_note tests ──────────────────────────────────────────

    /// Helper: create a workspace with a root note and N children under it.
    /// Returns (workspace, root_id, vec_of_child_ids).
    fn setup_with_children(n: usize) -> (Workspace, String, Vec<String>) {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path()).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let mut child_ids = Vec::new();
        for _ in 0..n {
            let id = ws
                .create_note(&root.id, AddPosition::AsChild, "TextNote")
                .unwrap();
            child_ids.push(id);
        }
        // Children are inserted at position 0, so reverse to get insertion order
        child_ids.reverse();
        (ws, root.id, child_ids)
    }

    #[test]
    fn test_move_note_reorder_siblings() {
        let (mut ws, root_id, children) = setup_with_children(3);
        // children[0]=pos0, children[1]=pos1, children[2]=pos2
        // Move children[2] to position 0
        ws.move_note(&children[2], Some(&root_id), 0).unwrap();
        let kids = ws.get_children(&root_id).unwrap();
        assert_eq!(kids[0].id, children[2]);
        assert_eq!(kids[1].id, children[0]);
        assert_eq!(kids[2].id, children[1]);
        // Positions should be gapless: 0, 1, 2
        for (i, kid) in kids.iter().enumerate() {
            assert_eq!(kid.position, i as i32, "Position mismatch at index {i}");
        }
    }

    #[test]
    fn test_move_note_to_different_parent() {
        let (mut ws, root_id, children) = setup_with_children(2);
        // Move children[1] to be a child of children[0]
        ws.move_note(&children[1], Some(&children[0]), 0).unwrap();

        // Root should now have 1 child
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[0]);
        assert_eq!(root_kids[0].position, 0);

        // children[0] should have 1 child
        let grandkids = ws.get_children(&children[0]).unwrap();
        assert_eq!(grandkids.len(), 1);
        assert_eq!(grandkids[0].id, children[1]);
        assert_eq!(grandkids[0].position, 0);
    }

    #[test]
    fn test_move_note_to_root() {
        let (mut ws, root_id, children) = setup_with_children(2);
        // Move children[0] to root level
        ws.move_note(&children[0], None, 1).unwrap();

        // Root should have 1 child left
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[1]);
        assert_eq!(root_kids[0].position, 0);

        // children[0] is now at root level
        let moved = ws.get_note(&children[0]).unwrap();
        assert_eq!(moved.parent_id, None);
        assert_eq!(moved.position, 1);
    }

    #[test]
    fn test_move_note_prevents_cycle() {
        let (mut ws, _root_id, children) = setup_with_children(1);
        // Make a grandchild under children[0]
        let grandchild_id = ws
            .create_note(&children[0], AddPosition::AsChild, "TextNote")
            .unwrap();
        // Try to move children[0] into its own grandchild — should fail
        let result = ws.move_note(&children[0], Some(&grandchild_id), 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cycle"), "Expected cycle error, got: {err}");
    }

    #[test]
    fn test_move_note_prevents_self_move() {
        let (mut ws, _root_id, children) = setup_with_children(1);
        let result = ws.move_note(&children[0], Some(&children[0]), 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_move_note_logs_operation() {
        let (mut ws, root_id, children) = setup_with_children(2);
        // Move children[1] to position 0
        ws.move_note(&children[1], Some(&root_id), 0).unwrap();

        let ops = ws.list_operations(None, None, None).unwrap();
        let move_ops: Vec<_> = ops
            .iter()
            .filter(|o| o.operation_type == "MoveNote")
            .collect();
        assert_eq!(move_ops.len(), 1, "Expected exactly one MoveNote operation");
    }

    #[test]
    fn test_move_note_positions_gapless_after_cross_parent_move() {
        let (mut ws, root_id, children) = setup_with_children(4);
        // Move children[1] out to be child of children[0]
        ws.move_note(&children[1], Some(&children[0]), 0).unwrap();

        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 3);
        for (i, kid) in root_kids.iter().enumerate() {
            assert_eq!(kid.position, i as i32, "Gap at index {i}");
        }
    }
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core -- move_note`
Expected: FAIL — `move_note` method does not exist

**Step 3: Implement `move_note`**

Add the following method to `impl Workspace` in `workspace.rs`, after the `set_selected_note` method (around line 470) and before `get_children`:

```rust
    /// Moves `note_id` to a new parent and/or position within a single transaction.
    ///
    /// When `new_parent_id` is `None` the note becomes a root-level note.
    /// Sibling positions in both the old and new parent groups are kept gapless.
    ///
    /// A `MoveNote` operation is appended to the operation log on success.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::InvalidMove`] if the move would create a
    /// cycle (moving a note into its own subtree) or the note tries to become
    /// its own parent. Returns [`KrillnotesError::NoteNotFound`] if `note_id`
    /// does not exist.
    pub fn move_note(
        &mut self,
        note_id: &str,
        new_parent_id: Option<&str>,
        new_position: i32,
    ) -> Result<()> {
        // Self-move check
        if new_parent_id == Some(note_id) {
            return Err(KrillnotesError::InvalidMove(
                "A note cannot be its own parent".to_string(),
            ));
        }

        // Cycle check: walk ancestor chain of new_parent_id
        if let Some(target_parent) = new_parent_id {
            let mut cursor = Some(target_parent.to_string());
            while let Some(ref ancestor_id) = cursor {
                if ancestor_id == note_id {
                    return Err(KrillnotesError::InvalidMove(
                        "Moving a note into its own subtree would create a cycle".to_string(),
                    ));
                }
                let parent: Option<String> = self.connection().query_row(
                    "SELECT parent_id FROM notes WHERE id = ?",
                    [ancestor_id.as_str()],
                    |row| row.get(0),
                )?;
                cursor = parent;
            }
        }

        let note = self.get_note(note_id)?;
        let old_parent_id = note.parent_id.clone();
        let old_position = note.position;

        let now = chrono::Utc::now().timestamp();
        let tx = self.storage.connection_mut().transaction()?;

        // 1. Close the gap in the old sibling group
        if let Some(ref old_pid) = old_parent_id {
            tx.execute(
                "UPDATE notes SET position = position - 1 WHERE parent_id = ? AND position > ?",
                rusqlite::params![old_pid, old_position],
            )?;
        } else {
            tx.execute(
                "UPDATE notes SET position = position - 1 WHERE parent_id IS NULL AND position > ?",
                rusqlite::params![old_position],
            )?;
        }

        // 2. Open a gap in the new sibling group
        if let Some(ref new_pid) = new_parent_id {
            tx.execute(
                "UPDATE notes SET position = position + 1 WHERE parent_id = ? AND position >= ?",
                rusqlite::params![new_pid, new_position],
            )?;
        } else {
            tx.execute(
                "UPDATE notes SET position = position + 1 WHERE parent_id IS NULL AND position >= ?",
                rusqlite::params![new_position],
            )?;
        }

        // 3. Move the note
        tx.execute(
            "UPDATE notes SET parent_id = ?, position = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![new_parent_id, new_position, now, note_id],
        )?;

        // 4. Log the operation
        let op = Operation::MoveNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: now,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            new_parent_id: new_parent_id.map(|s| s.to_string()),
            new_position,
        };
        self.operation_log.log(&tx, &op)?;
        self.operation_log.purge_if_needed(&tx)?;

        tx.commit()?;
        Ok(())
    }
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core -- move_note`
Expected: all 7 move_note tests PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: implement Workspace::move_note with cycle prevention and operation logging"
```

---

### Task 3: Add `move_note` Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add the command function**

Add this function right after the `delete_note` command (around line 525), before the user-script commands section:

```rust
/// Moves a note to a new parent and/or position.
#[tauri::command]
fn move_note(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    new_parent_id: Option<String>,
    new_position: i32,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.move_note(
        &note_id,
        new_parent_id.as_deref(),
        new_position,
    )
    .map_err(|e| e.to_string())
}
```

**Step 2: Register in the handler**

Add `move_note` to the `tauri::generate_handler!` list, after `delete_note`:

```rust
            delete_note,
            move_note,
```

**Step 3: Verify it compiles**

Run: `cargo check -p krillnotes-desktop`
Expected: compiles cleanly

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add move_note Tauri command"
```

---

### Task 4: Add `getDescendantIds` tree utility

**Files:**
- Modify: `krillnotes-desktop/src/utils/tree.ts`

**Step 1: Add the helper**

Append this function to the end of `tree.ts`:

```typescript
/**
 * Returns all descendant IDs of the given noteId (children, grandchildren, etc.).
 * Used for client-side cycle prevention during drag-and-drop.
 */
export function getDescendantIds(notes: Note[], noteId: string): Set<string> {
  const descendants = new Set<string>();
  const queue = [noteId];
  while (queue.length > 0) {
    const current = queue.pop()!;
    for (const note of notes) {
      if (note.parentId === current && !descendants.has(note.id)) {
        descendants.add(note.id);
        queue.push(note.id);
      }
    }
  }
  return descendants;
}
```

**Step 2: Verify it compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/tree.ts
git commit -m "feat: add getDescendantIds tree utility for cycle prevention"
```

---

### Task 5: Add drag state and handler to WorkspaceView

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Add drag state and types**

At the top of the file, after the existing imports, add the `getDescendantIds` import:

```typescript
import { buildTree, flattenVisibleTree, findNoteInTree, getAncestorIds, getDescendantIds } from '../utils/tree';
```

Inside the `WorkspaceView` function, after the existing state declarations (around line 48), add:

```typescript
  // Drag and drop state
  const [draggedNoteId, setDraggedNoteId] = useState<string | null>(null);
  const [dropIndicator, setDropIndicator] = useState<{ noteId: string; position: 'before' | 'after' | 'child' } | null>(null);
```

**Step 2: Add the move handler**

After the `handleToggleExpand` function, add:

```typescript
  const handleMoveNote = async (noteId: string, newParentId: string | null, newPosition: number) => {
    try {
      await invoke('move_note', { noteId, newParentId, newPosition });
      await loadNotes();
    } catch (err) {
      console.error('Failed to move note:', err);
    }
  };
```

**Step 3: Pass drag props to TreeView**

Update the `<TreeView>` JSX to pass the new props:

```tsx
          <TreeView
            tree={tree}
            selectedNoteId={selectedNoteId}
            onSelect={handleSelectNote}
            onToggleExpand={handleToggleExpand}
            onContextMenu={handleContextMenu}
            onKeyDown={handleTreeKeyDown}
            notes={notes}
            draggedNoteId={draggedNoteId}
            setDraggedNoteId={setDraggedNoteId}
            dropIndicator={dropIndicator}
            setDropIndicator={setDropIndicator}
            onMoveNote={handleMoveNote}
          />
```

**Step 4: Verify it compiles (will have type errors until Task 6)**

This step will produce type errors because `TreeView` doesn't accept the new props yet. That's expected — we'll fix it in Task 6.

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: add drag state and move handler to WorkspaceView"
```

---

### Task 6: Add drag-and-drop to TreeView and TreeNode

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx`

**Step 1: Update TreeView**

Replace the entire contents of `TreeView.tsx` with:

```typescript
import TreeNode from './TreeNode';
import type { TreeNode as TreeNodeType, Note } from '../types';

interface DropIndicator {
  noteId: string;
  position: 'before' | 'after' | 'child';
}

interface TreeViewProps {
  tree: TreeNodeType[];
  selectedNoteId: string | null;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  notes: Note[];
  draggedNoteId: string | null;
  setDraggedNoteId: (id: string | null) => void;
  dropIndicator: DropIndicator | null;
  setDropIndicator: (indicator: DropIndicator | null) => void;
  onMoveNote: (noteId: string, newParentId: string | null, newPosition: number) => void;
}

function TreeView({
  tree, selectedNoteId, onSelect, onToggleExpand, onContextMenu, onKeyDown,
  notes, draggedNoteId, setDraggedNoteId, dropIndicator, setDropIndicator, onMoveNote,
}: TreeViewProps) {

  const handleRootDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    // Only show root drop indicator when over empty space (not bubbled from a node)
    if (e.target === e.currentTarget) {
      setDropIndicator({ noteId: '__root__', position: 'after' });
    }
  };

  const handleRootDrop = (e: React.DragEvent) => {
    e.preventDefault();
    if (!draggedNoteId) return;
    // Count current root notes for position
    const rootCount = notes.filter(n => n.parentId === null).length;
    onMoveNote(draggedNoteId, null, rootCount);
    setDraggedNoteId(null);
    setDropIndicator(null);
  };

  const handleRootDragLeave = (e: React.DragEvent) => {
    if (e.target === e.currentTarget) {
      setDropIndicator(null);
    }
  };

  if (tree.length === 0) {
    return (
      <div
        className="flex items-center justify-center h-full text-muted-foreground text-sm focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
        tabIndex={0}
        onKeyDown={onKeyDown}
        onDragOver={handleRootDragOver}
        onDrop={handleRootDrop}
        onDragLeave={handleRootDragLeave}
      >
        No notes yet
      </div>
    );
  }

  return (
    <div
      className="h-full focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
      tabIndex={0}
      onKeyDown={onKeyDown}
      onDragOver={handleRootDragOver}
      onDrop={handleRootDrop}
      onDragLeave={handleRootDragLeave}
    >
      {tree.map(node => (
        <TreeNode
          key={node.note.id}
          node={node}
          selectedNoteId={selectedNoteId}
          level={0}
          onSelect={onSelect}
          onToggleExpand={onToggleExpand}
          onContextMenu={onContextMenu}
          notes={notes}
          draggedNoteId={draggedNoteId}
          setDraggedNoteId={setDraggedNoteId}
          dropIndicator={dropIndicator}
          setDropIndicator={setDropIndicator}
          onMoveNote={onMoveNote}
        />
      ))}
      {/* Root drop zone at bottom */}
      {draggedNoteId && dropIndicator?.noteId === '__root__' && (
        <div className="h-0.5 bg-blue-500 mx-2 my-1" />
      )}
    </div>
  );
}

export default TreeView;
```

**Step 2: Update TreeNode**

Replace the entire contents of `TreeNode.tsx` with:

```typescript
import { useCallback } from 'react';
import type { TreeNode as TreeNodeType, Note } from '../types';
import { getDescendantIds } from '../utils/tree';

interface DropIndicator {
  noteId: string;
  position: 'before' | 'after' | 'child';
}

interface TreeNodeProps {
  node: TreeNodeType;
  selectedNoteId: string | null;
  level: number;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
  notes: Note[];
  draggedNoteId: string | null;
  setDraggedNoteId: (id: string | null) => void;
  dropIndicator: DropIndicator | null;
  setDropIndicator: (indicator: DropIndicator | null) => void;
  onMoveNote: (noteId: string, newParentId: string | null, newPosition: number) => void;
}

function TreeNode({
  node, selectedNoteId, level, onSelect, onToggleExpand, onContextMenu,
  notes, draggedNoteId, setDraggedNoteId, dropIndicator, setDropIndicator, onMoveNote,
}: TreeNodeProps) {
  const hasChildren = node.children.length > 0;
  const isSelected = node.note.id === selectedNoteId;
  const isExpanded = node.note.isExpanded;
  const isDragged = node.note.id === draggedNoteId;
  const isDropTarget = dropIndicator?.noteId === node.note.id;

  const handleDragStart = useCallback((e: React.DragEvent) => {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', node.note.id);
    setDraggedNoteId(node.note.id);
  }, [node.note.id, setDraggedNoteId]);

  const handleDragEnd = useCallback(() => {
    setDraggedNoteId(null);
    setDropIndicator(null);
  }, [setDraggedNoteId, setDropIndicator]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (!draggedNoteId || draggedNoteId === node.note.id) return;

    // Cycle check: can't drop onto a descendant
    const descendants = getDescendantIds(notes, draggedNoteId);
    if (descendants.has(node.note.id)) return;

    e.dataTransfer.dropEffect = 'move';

    const rect = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - rect.top;
    const height = rect.height;
    const ratio = y / height;

    let position: 'before' | 'after' | 'child';
    if (ratio < 0.25) {
      position = 'before';
    } else if (ratio > 0.75) {
      position = 'after';
    } else {
      position = 'child';
    }

    setDropIndicator({ noteId: node.note.id, position });
  }, [draggedNoteId, node.note.id, notes, setDropIndicator]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    // Only clear if leaving this node (not entering a child)
    const related = e.relatedTarget as HTMLElement | null;
    if (!e.currentTarget.contains(related)) {
      if (dropIndicator?.noteId === node.note.id) {
        setDropIndicator(null);
      }
    }
  }, [node.note.id, dropIndicator, setDropIndicator]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (!draggedNoteId || draggedNoteId === node.note.id) return;

    const descendants = getDescendantIds(notes, draggedNoteId);
    if (descendants.has(node.note.id)) return;

    const rect = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - rect.top;
    const height = rect.height;
    const ratio = y / height;

    let newParentId: string | null;
    let newPosition: number;

    if (ratio < 0.25) {
      // Before: same parent, this node's position
      newParentId = node.note.parentId;
      newPosition = node.note.position;
    } else if (ratio > 0.75) {
      // After: same parent, this node's position + 1
      newParentId = node.note.parentId;
      newPosition = node.note.position + 1;
    } else {
      // Child: this node becomes parent, position 0
      newParentId = node.note.id;
      newPosition = 0;
      // Auto-expand if collapsed
      if (!isExpanded && hasChildren) {
        onToggleExpand(node.note.id);
      }
    }

    // No-op: skip if same location
    const dragged = notes.find(n => n.id === draggedNoteId);
    if (dragged && dragged.parentId === newParentId && dragged.position === newPosition) {
      setDraggedNoteId(null);
      setDropIndicator(null);
      return;
    }

    onMoveNote(draggedNoteId, newParentId, newPosition);
    setDraggedNoteId(null);
    setDropIndicator(null);
  }, [draggedNoteId, node, notes, isExpanded, hasChildren, onToggleExpand, onMoveNote, setDraggedNoteId, setDropIndicator]);

  const indentPx = level * 20 + 8;

  return (
    <div>
      {/* Drop indicator line: before */}
      {isDropTarget && dropIndicator?.position === 'before' && (
        <div className="h-0.5 bg-blue-500" style={{ marginLeft: `${indentPx}px` }} />
      )}

      <div
        data-note-id={node.note.id}
        draggable
        onDragStart={handleDragStart}
        onDragEnd={handleDragEnd}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        className={`flex items-center py-1 px-2 cursor-pointer hover:bg-secondary/50 ${
          isSelected ? 'bg-secondary' : ''
        } ${isDragged ? 'opacity-40' : ''} ${
          isDropTarget && dropIndicator?.position === 'child' ? 'bg-blue-500/20 ring-1 ring-blue-500/40' : ''
        }`}
        style={{ paddingLeft: `${indentPx}px` }}
        onClick={() => onSelect(node.note.id)}
        onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, node.note.id); }}
      >
        {hasChildren && (
          <button
            tabIndex={-1}
            onClick={(e) => {
              e.stopPropagation();
              onToggleExpand(node.note.id);
            }}
            className="mr-1 text-muted-foreground hover:text-foreground"
            aria-label={isExpanded ? 'Collapse' : 'Expand'}
            aria-expanded={isExpanded}
          >
            {isExpanded ? '▼' : '▶'}
          </button>
        )}
        {!hasChildren && <span className="w-4 mr-1" />}
        <span className="text-sm truncate">{node.note.title}</span>
      </div>

      {/* Drop indicator line: after */}
      {isDropTarget && dropIndicator?.position === 'after' && (
        <div className="h-0.5 bg-blue-500" style={{ marginLeft: `${indentPx}px` }} />
      )}

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
              notes={notes}
              draggedNoteId={draggedNoteId}
              setDraggedNoteId={setDraggedNoteId}
              dropIndicator={dropIndicator}
              setDropIndicator={setDropIndicator}
              onMoveNote={onMoveNote}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export default TreeNode;
```

**Step 3: Verify it compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/TreeNode.tsx krillnotes-desktop/src/components/TreeView.tsx
git commit -m "feat: add drag-and-drop to tree with visual indicators"
```

---

### Task 7: Manual smoke test and edge case fixes

**Step 1: Run the app**

Run: `cd krillnotes-desktop && npm run tauri dev`

**Step 2: Test these scenarios manually**

1. Drag a note up/down among siblings — positions reorder correctly
2. Drag a note onto another note (middle zone) — becomes a child
3. Drag a note to empty space at the bottom — becomes a root node
4. Drag a note onto a collapsed parent — parent expands, note inserted
5. Drag a parent onto its own child — nothing happens (cycle prevention)
6. Drag a note onto itself — nothing happens
7. Verify the moved note stays selected after the drop

**Step 3: Fix any issues found**

Address any visual or behavioral issues discovered during testing.

**Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: address drag-and-drop edge cases from manual testing"
```

---

### Task 8: Run full test suite and final commit

**Step 1: Run backend tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

**Step 2: Run frontend type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

**Step 3: Mark TODO as done**

In `TODO.md`, change the drag-and-drop task from `[ ]` to `✅ DONE!`.

**Step 4: Commit**

```bash
git add TODO.md
git commit -m "feat: drag-and-drop tree reordering done"
```
