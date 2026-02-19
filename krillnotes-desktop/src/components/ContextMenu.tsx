import { useEffect } from 'react';
import { createPortal } from 'react-dom';

interface ContextMenuProps {
  x: number;
  y: number;
  noteId: string;
  onAddNote: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({ x, y, onAddNote, onEdit, onDelete, onClose }: ContextMenuProps) {
  useEffect(() => {
    const handleMouseDown = () => onClose();
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
      className="fixed bg-background border border-secondary rounded shadow-lg z-50 py-1 min-w-[160px]"
      style={{ top: y, left: x }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
        onClick={onAddNote}
      >
        Add Note
      </button>
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
        onClick={onEdit}
      >
        Edit
      </button>
      <div className="border-t border-secondary my-1" />
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary text-red-500"
        onClick={onDelete}
      >
        Delete
      </button>
    </div>,
    document.body
  );
}

export default ContextMenu;
