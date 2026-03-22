// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

interface ContextMenuProps {
  x: number;
  y: number;
  noteId: string | null;  // null = background (no note right-clicked)
  copiedNoteId: string | null;
  isLeaf: boolean;
  treeActions: string[];
  effectiveRole?: string | null;       // "owner" | "writer" | "reader" | "root_owner" | "none" | null
  onAddChild: () => void;
  onAddSibling: () => void;
  onAddRoot: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onPasteAsChild: () => void;
  onPasteAsSibling: () => void;
  onTreeAction: (label: string) => void;
  onInviteToSubtree?: (noteId: string) => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({
  x, y, noteId, copiedNoteId, isLeaf, treeActions,
  effectiveRole,
  onAddChild, onAddSibling, onAddRoot,
  onEdit, onCopy, onPasteAsChild, onPasteAsSibling,
  onTreeAction, onInviteToSubtree, onDelete, onClose,
}: ContextMenuProps) {
  const { t } = useTranslation();
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('mousedown', handleMouseDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handleMouseDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={menuRef}
      className="fixed bg-background border border-secondary rounded shadow-lg z-50 py-1 min-w-[160px]"
      style={{ top: y, left: x }}
    >
      {noteId === null ? (
        // Background context menu — root note creation only
        <button
          className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
          onClick={() => { onAddRoot(); onClose(); }}
        >
          {t('notes.addRoot')}
        </button>
      ) : (
        // Note context menu
        <>
          <button
            className={`w-full text-left px-3 py-1.5 text-sm ${isLeaf ? 'opacity-40 cursor-not-allowed' : 'hover:bg-secondary'}`}
            onClick={() => { if (!isLeaf) { onAddChild(); onClose(); } }}
          >
            {t('notes.addChildShort')}
          </button>
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onAddSibling(); onClose(); }}
          >
            {t('notes.addSiblingShort')}
          </button>
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onEdit(); onClose(); }}
          >
            {t('common.edit')}
          </button>
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onCopy(); onClose(); }}
          >
            {t('notes.copyNote')}
          </button>
          <button
            className={`w-full text-left px-3 py-1.5 text-sm ${(copiedNoteId && !isLeaf) ? 'hover:bg-secondary' : 'opacity-40 cursor-not-allowed'}`}
            onClick={() => { if (copiedNoteId && !isLeaf) { onPasteAsChild(); onClose(); } }}
          >
            {t('notes.pasteAsChild')}
          </button>
          <button
            className={`w-full text-left px-3 py-1.5 text-sm ${copiedNoteId ? 'hover:bg-secondary' : 'opacity-40 cursor-not-allowed'}`}
            onClick={() => { if (copiedNoteId) { onPasteAsSibling(); onClose(); } }}
          >
            {t('notes.pasteAsSibling')}
          </button>
          {treeActions.length > 0 && (
            <>
              <div className="border-t border-secondary my-1" />
              {treeActions.map((label) => (
                <button
                  key={label}
                  className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
                  onClick={() => { onTreeAction(label); onClose(); }}
                >
                  {label}
                </button>
              ))}
            </>
          )}
          {noteId && effectiveRole && (effectiveRole === 'owner' || effectiveRole === 'root_owner') && onInviteToSubtree && (
            <>
              <div className="border-t border-secondary my-1" />
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
                onClick={() => { onInviteToSubtree(noteId); onClose(); }}
              >
                {t('contextMenu.inviteToSubtree', 'Invite to this subtree\u2026')}
              </button>
            </>
          )}
          <div className="border-t border-secondary my-1" />
          <button
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary text-red-500"
            onClick={() => { onDelete(); onClose(); }}
          >
            {t('common.delete')}
          </button>
        </>
      )}
    </div>,
    document.body
  );
}

export default ContextMenu;
