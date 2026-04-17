// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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
  onToggleChecked: (noteId: string, checked: boolean) => void;
  effectiveRoles?: Record<string, string>;
  shareAnchorIds?: Set<string>;
  showSharingIndicators?: boolean;
}

function TreeNode({
  node, selectedNoteId, level, onSelect, onToggleExpand, onContextMenu,
  notes, schemas, draggedNoteId, setDraggedNoteId, dropIndicator, setDropIndicator, dragDescendants, onMoveNote,
  onHoverStart, onHoverEnd, onToggleChecked, effectiveRoles, shareAnchorIds, showSharingIndicators,
}: TreeNodeProps) {
  const { t } = useTranslation();
  const hasChildren = node.children.length > 0;
  const isSelected = node.note.id === selectedNoteId;

  const noteId = node.note.id;
  const role = effectiveRoles?.[noteId] ?? null;
  const isGhost = role === 'none' || (effectiveRoles && Object.keys(effectiveRoles).length > 0 && !role);
  const isShareAnchor = shareAnchorIds?.has(noteId) ?? false;
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
        prospectiveParentType = node.note.schema;
      } else {
        const parentNote = node.note.parentId ? notes.find(n => n.id === node.note.parentId) : null;
        prospectiveParentType = parentNote ? parentNote.schema : null;
      }

      // is_leaf blocks all drops onto this note as parent
      if (prospectiveParentType !== null && schemas[prospectiveParentType]?.isLeaf) {
        setDropIndicator(null);
        return;
      }

      // Child constraint: dragged type's allowedParentSchemas
      const apt = schemas[draggedNote.schema]?.allowedParentSchemas ?? [];
      if (apt.length > 0) {
        if (!prospectiveParentType || !apt.includes(prospectiveParentType)) return;
      }

      // Parent constraint: prospective parent's allowedChildrenSchemas
      if (prospectiveParentType !== null) {
        const act = schemas[prospectiveParentType]?.allowedChildrenSchemas ?? [];
        if (act.length > 0 && !act.includes(draggedNote.schema)) return;
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

      // is_leaf: prospective parent cannot have any children
      if (parentNote && schemas[parentNote.schema]?.isLeaf) {
        setDraggedNoteId(null);
        setDropIndicator(null);
        return;
      }

      // Child constraint: dragged type's allowedParentSchemas
      const apt = schemas[draggedNote.schema]?.allowedParentSchemas ?? [];
      if (apt.length > 0) {
        if (!parentNote || !apt.includes(parentNote.schema)) {
          setDraggedNoteId(null);
          setDropIndicator(null);
          return;
        }
      }

      // Parent constraint: new parent's allowedChildrenSchemas
      if (parentNote) {
        const act = schemas[parentNote.schema]?.allowedChildrenSchemas ?? [];
        if (act.length > 0 && !act.includes(draggedNote.schema)) {
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

  const schema = schemas[node.note.schema];
  const hasHoverContent = (schema?.hasHover ?? false) || (schema?.fields.some(f => f.showOnHover) ?? false);

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
        onClick={isGhost ? undefined : () => onSelect(noteId)}
        onContextMenu={isGhost ? undefined : (e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e, noteId); }}
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
        {showSharingIndicators && !isGhost && role && (
          <span className={`text-[10px] mr-1 flex-shrink-0 ${
            role === 'owner' || role === 'root_owner' ? 'text-green-500' :
            role === 'writer' ? 'text-orange-500' :
            role === 'reader' ? 'text-yellow-500' : ''
          }`}>●</span>
        )}
        {showSharingIndicators && isShareAnchor && (role === 'owner' || role === 'root_owner') && (
          <span className="text-[10px] mr-1 flex-shrink-0 text-zinc-400" title={t('tree.sharedSubtree', 'Shared subtree')}>👥</span>
        )}
        {schemas[node.note.schema]?.showCheckbox && (
          <input
            type="checkbox"
            checked={node.note.isChecked}
            onChange={(e) => {
              e.stopPropagation();
              onToggleChecked(node.note.id, e.target.checked);
            }}
            onClick={(e) => e.stopPropagation()}
            className="mr-1.5 h-3.5 w-3.5 rounded border-muted-foreground/50 accent-primary flex-shrink-0"
            aria-label={node.note.isChecked ? t('tree.uncheckNote') : t('tree.checkNote')}
          />
        )}
        <span className={`text-sm truncate flex-1 min-w-0 ${isGhost ? 'text-zinc-400 italic' : ''} ${node.note.isChecked && schemas[node.note.schema]?.showCheckbox ? 'line-through text-muted-foreground' : ''}`}>{node.note.title}</span>
        {hasHoverContent && <span className="ml-1 text-xs text-muted-foreground/40 select-none">›</span>}
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
              onToggleChecked={onToggleChecked}
              effectiveRoles={effectiveRoles}
              shareAnchorIds={shareAnchorIds}
              showSharingIndicators={showSharingIndicators}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export default TreeNode;
