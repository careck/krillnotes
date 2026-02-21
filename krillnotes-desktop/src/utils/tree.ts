import type { Note, TreeNode } from '../types';

/**
 * Builds a tree structure from a flat array of notes.
 * Notes are expected to have parentId and position fields for hierarchy and ordering.
 * When sortConfig is provided, children are sorted according to their parent's schema:
 * - "asc": alphabetical by title (A→Z)
 * - "desc": reverse alphabetical by title (Z→A)
 * - "none" (default): by position (manual order)
 */
export function buildTree(
  notes: Note[],
  sortConfig?: Record<string, 'asc' | 'desc' | 'none'>
): TreeNode[] {
  // 1. Group children by parent_id
  const childrenMap = new Map<string | null, Note[]>();

  notes.forEach(note => {
    const parentId = note.parentId;
    if (!childrenMap.has(parentId)) {
      childrenMap.set(parentId, []);
    }
    childrenMap.get(parentId)!.push(note);
  });

  // 2. Recursive builder — sorts children based on parent's schema
  function buildNode(note: Note): TreeNode {
    const children = childrenMap.get(note.id) || [];
    const mode = sortConfig?.[note.nodeType] ?? 'none';
    if (mode === 'asc') {
      children.sort((a, b) => a.title.localeCompare(b.title));
    } else if (mode === 'desc') {
      children.sort((a, b) => b.title.localeCompare(a.title));
    } else {
      children.sort((a, b) => a.position - b.position);
    }
    return {
      note,
      children: children.map(buildNode)
    };
  }

  // 3. Sort root-level notes by position (roots have no parent schema)
  const roots = childrenMap.get(null) || [];
  roots.sort((a, b) => a.position - b.position);
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
