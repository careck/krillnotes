import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import type { TreeNode as TreeNodeType, Note, DropIndicator, SchemaInfo } from '../types';

interface TreeNodeProps {
  node: TreeNodeType;
  selectedNoteId: string | null;
  level: number;
  onSelect: (noteId: string) => void;
  onToggleExpand: (noteId: string) => void;
  onContextMenu: (e: React.MouseEvent, noteId: string) => void;
  notes: Note[];
  schemas: Record<string, SchemaInfo>;
  draggedNoteId: string | null;
  setDraggedNoteId: (id: string | null) => void;
  dropIndicator: DropIndicator | null;
  setDropIndicator: (indicator: DropIndicator | null) => void;
  dragDescendants: Set<string>;
  onMoveNote: (noteId: string, newParentId: string | null, newPosition: number) => void;
  onHoverStart: (noteId: string, anchorY: number) => void;
  onHoverEnd: () => void;
}

function TreeNode({
  node, selectedNoteId, level, onSelect, onToggleExpand, onContextMenu,
  notes, schemas, draggedNoteId, setDraggedNoteId, dropIndicator, setDropIndicator, dragDescendants, onMoveNote,
  onHoverStart, onHoverEnd,
}: TreeNodeProps) {
  const { t } = useTranslation();
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
    if (dragDescendants.has(node.note.id)) return;

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

    // Schema constraint checks: suppress drop indicator for invalid placements
    const draggedNote = notes.find(n => n.id === draggedNoteId);
    if (draggedNote) {
      // Compute prospective parent type once for both checks
      let prospectiveParentType: string | null;
      if (position === 'child') {
        prospectiveParentType = node.note.nodeType;
      } else {
        const parentNote = node.note.parentId ? notes.find(n => n.id === node.note.parentId) : null;
        prospectiveParentType = parentNote ? parentNote.nodeType : null;
      }

      // Child constraint: dragged type's allowedParentTypes
      const apt = schemas[draggedNote.nodeType]?.allowedParentTypes ?? [];
      if (apt.length > 0) {
        if (!prospectiveParentType || !apt.includes(prospectiveParentType)) return;
      }

      // Parent constraint: prospective parent's allowedChildrenTypes
      if (prospectiveParentType !== null) {
        const act = schemas[prospectiveParentType]?.allowedChildrenTypes ?? [];
        if (act.length > 0 && !act.includes(draggedNote.nodeType)) return;
      }
    }

    e.dataTransfer.dropEffect = 'move';
    setDropIndicator({ noteId: node.note.id, position });
  }, [draggedNoteId, node.note, notes, schemas, dragDescendants, setDropIndicator]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    const related = e.relatedTarget as HTMLElement | null;
    if (!e.currentTarget.contains(related)) {
      setDropIndicator(null);
    }
  }, [setDropIndicator]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (!draggedNoteId || draggedNoteId === node.note.id) return;

    if (dragDescendants.has(node.note.id)) return;

    const rect = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - rect.top;
    const height = rect.height;
    const ratio = y / height;

    let newParentId: string | null;
    let newPosition: number;

    if (ratio < 0.25) {
      newParentId = node.note.parentId;
      newPosition = node.note.position;
    } else if (ratio > 0.75) {
      newParentId = node.note.parentId;
      newPosition = node.note.position + 1;
    } else {
      newParentId = node.note.id;
      newPosition = 0;
      if (!isExpanded && hasChildren) {
        onToggleExpand(node.note.id);
      }
    }

    // Schema constraint checks: block invalid drops
    const draggedNote = notes.find(n => n.id === draggedNoteId);
    if (draggedNote) {
      const parentNote = newParentId ? notes.find(n => n.id === newParentId) : null;

      // Child constraint: dragged type's allowedParentTypes
      const apt = schemas[draggedNote.nodeType]?.allowedParentTypes ?? [];
      if (apt.length > 0) {
        if (!parentNote || !apt.includes(parentNote.nodeType)) {
          setDraggedNoteId(null);
          setDropIndicator(null);
          return;
        }
      }

      // Parent constraint: new parent's allowedChildrenTypes
      if (parentNote) {
        const act = schemas[parentNote.nodeType]?.allowedChildrenTypes ?? [];
        if (act.length > 0 && !act.includes(draggedNote.nodeType)) {
          setDraggedNoteId(null);
          setDropIndicator(null);
          return;
        }
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
  }, [draggedNoteId, node, notes, schemas, dragDescendants, isExpanded, hasChildren, onToggleExpand, onMoveNote, setDraggedNoteId, setDropIndicator]);

  const indentPx = level * 20 + 8;

  return (
    <div>
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
        onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e, node.note.id); }}
        onMouseEnter={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          onHoverStart(node.note.id, rect.top + rect.height / 2);
        }}
        onMouseLeave={() => onHoverEnd()}
        onMouseDown={() => onHoverEnd()}
      >
        {hasChildren && (
          <button
            tabIndex={-1}
            onClick={(e) => {
              e.stopPropagation();
              onToggleExpand(node.note.id);
            }}
            className="mr-1 text-muted-foreground hover:text-foreground"
            aria-label={isExpanded ? t('tree.collapse') : t('tree.expand')}
            aria-expanded={isExpanded}
          >
            {isExpanded ? '\u25BC' : '\u25B6'}
          </button>
        )}
        {!hasChildren && <span className="w-4 mr-1" />}
        <span className="text-sm truncate">{node.note.title}</span>
      </div>

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
              schemas={schemas}
              draggedNoteId={draggedNoteId}
              setDraggedNoteId={setDraggedNoteId}
              dropIndicator={dropIndicator}
              setDropIndicator={setDropIndicator}
              dragDescendants={dragDescendants}
              onMoveNote={onMoveNote}
              onHoverStart={onHoverStart}
              onHoverEnd={onHoverEnd}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export default TreeNode;
