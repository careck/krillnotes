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
        className="flex items-center justify-center h-full text-muted-foreground text-sm focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
        tabIndex={0}
        onKeyDown={onKeyDown}
      >
        No notes yet
      </div>
    );
  }

  return (
    <div
      className="overflow-y-auto h-full focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
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
