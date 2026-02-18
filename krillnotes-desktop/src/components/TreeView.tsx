import TreeNode from './TreeNode';
import type { TreeNode as TreeNodeType } from '../types';

interface TreeViewProps {
  tree: TreeNodeType[];
  selectedNoteId: string | null;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
}

function TreeView({ tree, selectedNoteId, onSelect, onToggleExpand }: TreeViewProps) {
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
        />
      ))}
    </div>
  );
}

export default TreeView;
