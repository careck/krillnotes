import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

interface ContextMenuProps {
  x: number;
  y: number;
  noteId: string | null;  // null = background (no note right-clicked)
  copiedNoteId: string | null;
  treeActions: string[];
  onAddChild: () => void;
  onAddSibling: () => void;
  onAddRoot: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onPasteAsChild: () => void;
  onPasteAsSibling: () => void;
  onTreeAction: (label: string) => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({
  x, y, noteId, copiedNoteId, treeActions,
  onAddChild, onAddSibling, onAddRoot,
  onEdit, onCopy, onPasteAsChild, onPasteAsSibling,
  onTreeAction, onDelete, onClose,
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
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
            onClick={() => { onAddChild(); onClose(); }}
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
            className={`w-full text-left px-3 py-1.5 text-sm ${copiedNoteId ? 'hover:bg-secondary' : 'opacity-40 cursor-not-allowed'}`}
            onClick={() => { if (copiedNoteId) { onPasteAsChild(); onClose(); } }}
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
