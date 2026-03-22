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
  isRootOwner?: boolean;
  onAddChild: () => void;
  onAddSibling: () => void;
  onAddRoot: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onPasteAsChild: () => void;
  onPasteAsSibling: () => void;
  onTreeAction: (label: string) => void;
  onInviteToSubtree?: (noteId: string) => void;
  onShareSubtree?: (noteId: string) => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({
  x, y, noteId, copiedNoteId, isLeaf, treeActions,
  effectiveRole, isRootOwner,
  onAddChild, onAddSibling, onAddRoot,
  onEdit, onCopy, onPasteAsChild, onPasteAsSibling,
  onTreeAction, onInviteToSubtree, onShareSubtree, onDelete, onClose,
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

  // Permission flags for note menu
  // When effectiveRole is null/undefined, RBAC is not active — allow everything
  const canWrite = !effectiveRole || effectiveRole === 'owner' || effectiveRole === 'root_owner' || effectiveRole === 'writer';
  const canManage = !effectiveRole || effectiveRole === 'owner' || effectiveRole === 'root_owner';
  const isGhost = effectiveRole === 'none';

  return createPortal(
    <div
      ref={menuRef}
      className="fixed bg-background border border-secondary rounded shadow-lg z-50 py-1 min-w-[160px]"
      style={{ top: y, left: x }}
    >
      {noteId === null ? (
        // Background context menu — root note creation only
        <button
          onClick={isRootOwner !== false ? () => { onAddRoot(); onClose(); } : undefined}
          disabled={isRootOwner === false}
          className={`w-full text-left px-3 py-1.5 text-sm ${
            isRootOwner !== false
              ? 'hover:bg-zinc-100 dark:hover:bg-zinc-700'
              : 'opacity-40 cursor-not-allowed'
          }`}
        >
          {t('notes.addRoot')}
        </button>
      ) : (
        // Note context menu
        <>
          {noteId && isGhost && (
            <p className="px-3 py-1.5 text-xs text-zinc-400 italic">
              {t('contextMenu.noAccess', 'No access')}
            </p>
          )}
          {!isGhost && (
            <>
              <button
                disabled={!canWrite || isLeaf}
                className={`w-full text-left px-3 py-1.5 text-sm ${(!canWrite || isLeaf) ? 'opacity-40 cursor-not-allowed' : 'hover:bg-secondary'}`}
                onClick={canWrite && !isLeaf ? () => { onAddChild(); onClose(); } : undefined}
              >
                {t('notes.addChildShort')}
              </button>
              <button
                disabled={!canWrite}
                className={`w-full text-left px-3 py-1.5 text-sm ${!canWrite ? 'opacity-40 cursor-not-allowed' : 'hover:bg-secondary'}`}
                onClick={canWrite ? () => { onAddSibling(); onClose(); } : undefined}
              >
                {t('notes.addSiblingShort')}
              </button>
              <button
                disabled={!canWrite}
                className={`w-full text-left px-3 py-1.5 text-sm ${!canWrite ? 'opacity-40 cursor-not-allowed' : 'hover:bg-secondary'}`}
                onClick={canWrite ? () => { onEdit(); onClose(); } : undefined}
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
              {noteId && canManage && onShareSubtree && (
                <button
                  onClick={() => { onShareSubtree(noteId); onClose(); }}
                  className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
                >
                  {t('contextMenu.shareSubtree', 'Share subtree\u2026')}
                </button>
              )}
              <div className="border-t border-secondary my-1" />
              <button
                disabled={!canWrite}
                className={`w-full text-left px-3 py-1.5 text-sm text-red-500 ${!canWrite ? 'opacity-40 cursor-not-allowed' : 'hover:bg-secondary'}`}
                onClick={canWrite ? () => { onDelete(); onClose(); } : undefined}
              >
                {t('common.delete')}
              </button>
            </>
          )}
        </>
      )}
    </div>,
    document.body
  );
}

export default ContextMenu;
