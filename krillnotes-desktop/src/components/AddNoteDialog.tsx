import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, SchemaInfo } from '../types';
import { getAvailableTypes, type NotePosition } from '../utils/noteTypes';

interface AddNoteDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onNoteCreated: (noteId: string) => void;
  referenceNoteId: string | null;  // null = creating root note
  position: NotePosition;
  notes: Note[];
  schemas: Record<string, SchemaInfo>;
}

function AddNoteDialog({ isOpen, onClose, onNoteCreated, referenceNoteId, position, notes, schemas }: AddNoteDialogProps) {
  const [nodeType, setNodeType] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [loading, setLoading] = useState(false);

  const availableTypes = useMemo(
    () => getAvailableTypes(position, referenceNoteId, notes, schemas),
    [position, referenceNoteId, notes, schemas]
  );

  useEffect(() => {
    if (availableTypes.length > 0 && !availableTypes.includes(nodeType)) {
      setNodeType(availableTypes[0]);
    }
  }, [availableTypes, nodeType]);

  useEffect(() => {
    if (isOpen) {
      setError('');
      setLoading(false);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleCreate = async () => {
    setLoading(true);
    setError('');
    try {
      const note = await invoke<Note>('create_note_with_type', {
        parentId: position === 'root' ? null : referenceNoteId,
        position: position === 'root' ? 'child' : position,
        nodeType,
      });
      onNoteCreated(note.id);
      onClose();
    } catch (err) {
      setError(`Failed to create note: ${err}`);
    } finally {
      setLoading(false);
    }
  };

  const title = position === 'root' ? 'Add Root Note'
    : position === 'child' ? 'Add Child Note'
    : 'Add Sibling Note';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{title}</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">Note Type</label>
          {availableTypes.length === 0 ? (
            <p className="text-sm text-amber-600 py-2">No note types are allowed at this position.</p>
          ) : (
            <select
              value={nodeType}
              onChange={(e) => setNodeType(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            >
              {availableTypes.map(type => (
                <option key={type} value={type}>{type}</option>
              ))}
            </select>
          )}
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={loading}
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={loading || !nodeType || availableTypes.length === 0}
          >
            {loading ? 'Creating...' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  );
}

export default AddNoteDialog;
