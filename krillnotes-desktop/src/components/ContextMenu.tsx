import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

interface ContextMenuProps {
  x: number;
  y: number;
  onAddNote: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onClose: () => void;
}

function ContextMenu({ x, y, onAddNote, onEdit, onDelete, onClose }: ContextMenuProps) {
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
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
        onClick={() => { onAddNote(); onClose(); }}
      >
        Add Note
      </button>
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
        onClick={() => { onEdit(); onClose(); }}
      >
        Edit
      </button>
      <div className="border-t border-secondary my-1" />
      <button
        className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary text-red-500"
        onClick={() => { onDelete(); onClose(); }}
      >
        Delete
      </button>
    </div>,
    document.body
  );
}

export default ContextMenu;
