import type { Note, TreeNode } from '../types';

/**
 * Builds a tree structure from a flat array of notes.
 * Notes are expected to have parentId and position fields for hierarchy and ordering.
 */
export function buildTree(notes: Note[]): TreeNode[] {
  // 1. Group children by parent_id
  const childrenMap = new Map<string | null, Note[]>();

  notes.forEach(note => {
    const parentId = note.parentId;
    if (!childrenMap.has(parentId)) {
      childrenMap.set(parentId, []);
    }
    childrenMap.get(parentId)!.push(note);
  });

  // 2. Sort siblings by position
  childrenMap.forEach(children => {
    children.sort((a, b) => a.position - b.position);
  });

  // 3. Recursive builder
  function buildNode(note: Note): TreeNode {
    const children = childrenMap.get(note.id) || [];
    return {
      note,
      children: children.map(buildNode)
    };
  }

  // 4. Return root-level nodes (parentId = null)
  const roots = childrenMap.get(null) || [];
  return roots.map(buildNode);
}

/**
 * Finds a note in the tree by ID (depth-first search)
 */
export function findNoteInTree(tree: TreeNode[], noteId: string): TreeNode | null {
  for (const node of tree) {
    if (node.note.id === noteId) {
      return node;
    }
    const found = findNoteInTree(node.children, noteId);
    if (found) {
      return found;
    }
  }
  return null;
}

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
