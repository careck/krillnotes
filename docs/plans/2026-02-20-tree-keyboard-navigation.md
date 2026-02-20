# Tree Keyboard Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Up/Down/Left/Right/Enter keyboard navigation to the tree panel so users can browse notes and enter edit mode without the mouse.

**Architecture:** A `handleTreeKeyDown` handler lives in `WorkspaceView` (where all selection state lives) and is passed as a prop to `TreeView`. A new `flattenVisibleTree` utility produces a depth-first flat list of all currently-visible nodes for Up/Down traversal. `TreeView` gets `tabIndex={0}` so it can receive keyboard focus; nav is only active when the tree panel is focused.

**Tech Stack:** React 19, TypeScript 5, Tauri v2. No test framework exists — verify with `npx tsc --noEmit`.

---

### Task 1: Add `flattenVisibleTree` to `src/utils/tree.ts`

**Files:**
- Modify: `krillnotes-desktop/src/utils/tree.ts:52` (add after `findNoteInTree`)

**Step 1: Add the function**

Open `krillnotes-desktop/src/utils/tree.ts`. After the closing `}` of `findNoteInTree` (line 52), add:

```typescript
/**
 * Returns a flat depth-first list of all currently-visible nodes.
 * Only expanded nodes' children are included.
 */
export function flattenVisibleTree(nodes: TreeNode[]): TreeNode[] {
  const result: TreeNode[] = [];
  for (const node of nodes) {
    result.push(node);
    if (node.note.isExpanded && node.children.length > 0) {
      result.push(...flattenVisibleTree(node.children));
    }
  }
  return result;
}
```

**Step 2: Type-check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/tree.ts
git commit -m "feat: add flattenVisibleTree utility for keyboard navigation"
```

---

### Task 2: Add `data-note-id` attribute to `TreeNode.tsx`

This attribute lets the keyboard handler call `scrollIntoView` after selection changes.

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx:19`

**Step 1: Add the attribute**

In `TreeNode.tsx`, the inner clickable row div starts at line 19:

```tsx
<div
  className={`flex items-center py-1 px-2 cursor-pointer hover:bg-secondary/50 ${
    isSelected ? 'bg-secondary' : ''
  }`}
  style={{ paddingLeft: `${level * 20 + 8}px` }}
  onClick={() => onSelect(node.note.id)}
  onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, node.note.id); }}
>
```

Add `data-note-id={node.note.id}` as the first attribute:

```tsx
<div
  data-note-id={node.note.id}
  className={`flex items-center py-1 px-2 cursor-pointer hover:bg-secondary/50 ${
    isSelected ? 'bg-secondary' : ''
  }`}
  style={{ paddingLeft: `${level * 20 + 8}px` }}
  onClick={() => onSelect(node.note.id)}
  onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, node.note.id); }}
>
```

**Step 2: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/TreeNode.tsx
git commit -m "feat: add data-note-id attribute to tree nodes for scroll-into-view"
```

---

### Task 3: Add `onKeyDown` prop and focus to `TreeView.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`

**Step 1: Update the file**

Replace the entire file content with:

```tsx
import TreeNode from './TreeNode';
import type { TreeNode as TreeNodeType } from '../types';

interface TreeViewProps {
  tree: TreeNodeType[];
  selectedNoteId: string | null;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
}

function TreeView({ tree, selectedNoteId, onSelect, onToggleExpand, onContextMenu, onKeyDown }: TreeViewProps) {
  if (tree.length === 0) {
    return (
      <div
        className="flex items-center justify-center h-full text-muted-foreground text-sm focus:outline-none"
        tabIndex={0}
        onKeyDown={onKeyDown}
      >
        No notes yet
      </div>
    );
  }

  return (
    <div
      className="overflow-y-auto h-full focus:outline-none"
      tabIndex={0}
      onKeyDown={onKeyDown}
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
        />
      ))}
    </div>
  );
}

export default TreeView;
```

Key changes:
- New `onKeyDown` prop in interface and destructuring
- Both return paths get `tabIndex={0}`, `onKeyDown`, and `focus:outline-none`

**Step 2: Type-check**

```bash
npx tsc --noEmit
```

Expected: one error — `WorkspaceView` doesn't pass `onKeyDown` yet. That's fine, we fix it next.

**Step 3: Commit after Task 4 passes tsc**

Hold the commit until WorkspaceView is updated.

---

### Task 4: Add keyboard handler to `WorkspaceView.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Update the import line**

Line 12 currently imports only `buildTree`:

```typescript
import { buildTree } from '../utils/tree';
```

Change to:

```typescript
import { buildTree, flattenVisibleTree, findNoteInTree } from '../utils/tree';
```

**Step 2: Add `handleTreeKeyDown` function**

Add this function after `handleToggleExpand` (after line 105, before `handleNoteCreated`):

```typescript
const handleTreeKeyDown = (e: React.KeyboardEvent) => {
  if (!selectedNoteId) return;
  const flat = flattenVisibleTree(tree);
  const idx = flat.findIndex(n => n.note.id === selectedNoteId);
  if (idx === -1) return;
  const current = flat[idx];

  const selectAndScroll = (noteId: string) => {
    handleSelectNote(noteId);
    requestAnimationFrame(() => {
      document.querySelector(`[data-note-id="${noteId}"]`)?.scrollIntoView({ block: 'nearest' });
    });
  };

  switch (e.key) {
    case 'ArrowDown': {
      e.preventDefault();
      if (idx < flat.length - 1) selectAndScroll(flat[idx + 1].note.id);
      break;
    }
    case 'ArrowUp': {
      e.preventDefault();
      if (idx > 0) selectAndScroll(flat[idx - 1].note.id);
      break;
    }
    case 'ArrowRight': {
      e.preventDefault();
      if (current.children.length > 0) {
        if (!current.note.isExpanded) {
          handleToggleExpand(current.note.id);
        } else {
          selectAndScroll(current.children[0].note.id);
        }
      }
      break;
    }
    case 'ArrowLeft': {
      e.preventDefault();
      if (current.note.isExpanded) {
        handleToggleExpand(current.note.id);
      } else if (current.note.parentId) {
        const parent = findNoteInTree(tree, current.note.parentId);
        if (parent) selectAndScroll(parent.note.id);
      }
      break;
    }
    case 'Enter': {
      e.preventDefault();
      setRequestEditMode(prev => prev + 1);
      break;
    }
  }
};
```

**Step 3: Pass `onKeyDown` to `TreeView`**

In the JSX (around line 233), change:

```tsx
<TreeView
  tree={tree}
  selectedNoteId={selectedNoteId}
  onSelect={handleSelectNote}
  onToggleExpand={handleToggleExpand}
  onContextMenu={handleContextMenu}
/>
```

to:

```tsx
<TreeView
  tree={tree}
  selectedNoteId={selectedNoteId}
  onSelect={handleSelectNote}
  onToggleExpand={handleToggleExpand}
  onContextMenu={handleContextMenu}
  onKeyDown={handleTreeKeyDown}
/>
```

**Step 4: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors.

**Step 5: Commit Tasks 3 and 4 together**

```bash
git add krillnotes-desktop/src/components/TreeView.tsx krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: add keyboard navigation to tree panel (arrow keys + enter)"
```

---

### Task 5: Manual smoke test

Run the app:

```bash
cd krillnotes-desktop && npm run tauri dev
```

Verify:
1. Click the tree panel to focus it
2. Down/Up moves selection through all visible nodes depth-first
3. Right on a collapsed node expands it (two Right presses to expand then enter first child)
4. Left on an expanded node collapses it; Left on a collapsed non-root node moves to parent
5. Enter opens edit mode in the info panel
6. When typing in the edit panel, arrow keys do not navigate the tree
7. Newly selected node scrolls into view when off-screen

---

### Task 6: Mark TODO complete and commit

In `TODO.md`, change line 9:

```
[ ] tree navigation via keyboard, eg. arrow keys...
```

to:

```
[x] tree navigation via keyboard, eg. arrow keys...
```

```bash
git add TODO.md
git commit -m "chore: mark tree keyboard navigation as done"
```
