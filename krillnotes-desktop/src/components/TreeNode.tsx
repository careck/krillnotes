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
        data-note-id={node.note.id}
        className={`flex items-center py-1 px-2 cursor-pointer hover:bg-secondary/50 ${
          isSelected ? 'bg-secondary' : ''
        }`}
        style={{ paddingLeft: `${level * 20 + 8}px` }}
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
