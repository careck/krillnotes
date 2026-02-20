# Tree Keyboard Navigation Design

**Date:** 2026-02-20
**Status:** Approved

## Overview

Add keyboard navigation to the Krill Notes tree panel so users can browse and open notes without the mouse.

## Key Bindings

| Key | Behavior |
|-----|----------|
| Down | Move selection to next visible node (depth-first) |
| Up | Move selection to previous visible node (depth-first) |
| Right | Collapsed node with children → expand and stay. Expanded node → move to first child. Leaf → no-op. |
| Left | Expanded node → collapse and stay. Collapsed node → move to parent. Root at top level → no-op. |
| Enter | Trigger edit mode for the selected note |

Right/Left matches VS Code file explorer conventions. Expanding a node and entering it requires two Right presses. This avoids async timing issues since `toggle_note_expansion` is a Tauri backend roundtrip.

## Architecture

**Option A selected:** logic lives in `WorkspaceView` alongside the state it manipulates (`selectedNoteId`, `tree`, `handleSelectNote`, `handleToggleExpand`, `setRequestEditMode`). The handler is passed as an `onKeyDown` prop to `TreeView`.

## Focus Model

- Tree keyboard navigation is active only when the `TreeView` container has DOM focus.
- `TreeView` gets `tabIndex={0}` to make it focusable.
- Arrow keys call `e.preventDefault()` to suppress browser scroll.
- When focus is in the InfoPanel (edit mode text fields), keys go to those inputs as normal.

## Scroll Behavior

After selection changes via keyboard, the newly selected node is scrolled into view:

```ts
document.querySelector(`[data-note-id="${newId}"]`)?.scrollIntoView({ block: 'nearest' });
```

Each `TreeNode` div gets a `data-note-id` attribute for this purpose.

## Files to Change

| File | Change |
|------|--------|
| `src/utils/tree.ts` | Add `flattenVisibleTree(nodes: TreeNode[]): TreeNode[]` — depth-first traversal of expanded nodes only |
| `src/components/TreeNode.tsx` | Add `data-note-id={note.id}` to the node's root div |
| `src/components/TreeView.tsx` | Add `onKeyDown` prop, `tabIndex={0}`, and focus ring styling to the container div |
| `src/components/WorkspaceView.tsx` | Add keyboard handler using `flattenVisibleTree`, `findNoteInTree`, and `note.parentId`; pass to `TreeView` |

## Helper Logic

### `flattenVisibleTree`

```ts
function flattenVisibleTree(nodes: TreeNode[]): TreeNode[] {
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

### Keyboard Handler (WorkspaceView)

```ts
const handleTreeKeyDown = (e: React.KeyboardEvent) => {
  if (!selectedNoteId || !tree) return;
  const flat = flattenVisibleTree(tree);
  const idx = flat.findIndex(n => n.note.id === selectedNoteId);
  if (idx === -1) return;
  const current = flat[idx];

  switch (e.key) {
    case 'ArrowDown': {
      e.preventDefault();
      if (idx < flat.length - 1) handleSelectNote(flat[idx + 1].note.id);
      break;
    }
    case 'ArrowUp': {
      e.preventDefault();
      if (idx > 0) handleSelectNote(flat[idx - 1].note.id);
      break;
    }
    case 'ArrowRight': {
      e.preventDefault();
      if (current.children.length > 0) {
        if (!current.note.isExpanded) {
          handleToggleExpand(current.note.id);
        } else {
          handleSelectNote(current.children[0].note.id);
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
        if (parent) handleSelectNote(parent.note.id);
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

## Non-Goals

- No wrapping (Down on last node does nothing; Up on first node does nothing)
- No Page Up/Down or Home/End shortcuts
- No multi-select
