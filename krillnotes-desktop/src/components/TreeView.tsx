import TreeNode from './TreeNode';
import type { TreeNode as TreeNodeType, Note, DropIndicator, SchemaInfo } from '../types';

interface TreeViewProps {
  tree: TreeNodeType[];
  selectedNoteId: string | null;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  notes: Note[];
  schemas: Record<string, SchemaInfo>;
  draggedNoteId: string | null;
  setDraggedNoteId: (id: string | null) => void;
  dropIndicator: DropIndicator | null;
  setDropIndicator: (indicator: DropIndicator | null) => void;
  dragDescendants: Set<string>;
  onMoveNote: (noteId: string, newParentId: string | null, newPosition: number) => void;
  onBackgroundContextMenu: (e: React.MouseEvent) => void;
}

function TreeView({
  tree, selectedNoteId, onSelect, onToggleExpand, onContextMenu, onKeyDown,
  notes, schemas, draggedNoteId, setDraggedNoteId, dropIndicator, setDropIndicator, dragDescendants, onMoveNote,
  onBackgroundContextMenu,
}: TreeViewProps) {

  const handleRootDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    if (e.target === e.currentTarget) {
      setDropIndicator({ noteId: '__root__', position: 'after' });
    }
  };

  const handleRootDrop = (e: React.DragEvent) => {
    e.preventDefault();
    if (!draggedNoteId) return;

    // Block if dragged note's schema restricts parent types (root has no parent)
    const draggedNote = notes.find(n => n.id === draggedNoteId);
    if (draggedNote) {
      const apt = schemas[draggedNote.nodeType]?.allowedParentTypes ?? [];
      if (apt.length > 0) {
        setDraggedNoteId(null);
        setDropIndicator(null);
        return;
      }
    }

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
        onContextMenu={(e) => { e.preventDefault(); onBackgroundContextMenu(e); }}
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
      onContextMenu={(e) => { e.preventDefault(); onBackgroundContextMenu(e); }}
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
          schemas={schemas}
          draggedNoteId={draggedNoteId}
          setDraggedNoteId={setDraggedNoteId}
          dropIndicator={dropIndicator}
          setDropIndicator={setDropIndicator}
          dragDescendants={dragDescendants}
          onMoveNote={onMoveNote}
        />
      ))}
      {draggedNoteId && dropIndicator?.noteId === '__root__' && (
        <div className="h-0.5 bg-blue-500 mx-2 my-1" />
      )}
    </div>
  );
}

export default TreeView;
