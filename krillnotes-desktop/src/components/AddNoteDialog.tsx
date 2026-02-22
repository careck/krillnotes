import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, SchemaInfo } from '../types';

interface AddNoteDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onNoteCreated: (noteId: string) => void;
  selectedNoteId: string | null;
  hasNotes: boolean;
  notes: Note[];
  schemas: Record<string, SchemaInfo>;
}

function AddNoteDialog({ isOpen, onClose, onNoteCreated, selectedNoteId, hasNotes, notes, schemas }: AddNoteDialogProps) {
  const [position, setPosition] = useState<'child' | 'sibling'>('child');
  const [nodeType, setNodeType] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [loading, setLoading] = useState(false);

  // Derive the list of allowed note types based on position and selected note
  const availableTypes = useMemo(() => {
    const allTypes = Object.keys(schemas);
    if (!hasNotes || !selectedNoteId) {
      // Creating the first note (root): only types with no parent restriction
      return allTypes.filter(t => (schemas[t]?.allowedParentTypes ?? []).length === 0);
    }

    const selectedNote = notes.find(n => n.id === selectedNoteId);
    if (!selectedNote) return allTypes;

    let effectiveParentType: string | null;
    if (position === 'child') {
      effectiveParentType = selectedNote.nodeType;
    } else {
      const parentNote = notes.find(n => n.id === selectedNote.parentId);
      effectiveParentType = parentNote ? parentNote.nodeType : null;
    }

    return allTypes.filter(type => {
      const apt = schemas[type]?.allowedParentTypes ?? [];
      if (apt.length === 0) return true;
      if (effectiveParentType === null) return false;
      return apt.includes(effectiveParentType);
    });
  }, [schemas, notes, selectedNoteId, hasNotes, position]);

  // Keep nodeType in sync when available types change
  useEffect(() => {
    if (availableTypes.length > 0 && !availableTypes.includes(nodeType)) {
      setNodeType(availableTypes[0]);
    }
  }, [availableTypes]);

  if (!isOpen) return null;

  const handleCreate = async () => {
    setLoading(true);
    setError('');

    try {
      const note = await invoke<Note>('create_note_with_type', {
        parentId: hasNotes ? selectedNoteId : null,
        position: hasNotes ? position : 'child',
        nodeType
      });
      onNoteCreated(note.id);
      onClose();
    } catch (err) {
      setError(`Failed to create note: ${err}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">
          {hasNotes ? 'Add Note' : 'Creating First Note'}
        </h2>

        {hasNotes && (
          <div className="mb-4">
            <label className="block text-sm font-medium mb-2">Position</label>
            <div className="space-y-2">
              <label className="flex items-center">
                <input
                  type="radio"
                  value="child"
                  checked={position === 'child'}
                  onChange={(e) => setPosition(e.target.value as 'child')}
                  className="mr-2"
                />
                As child of selected note
              </label>
              <label className="flex items-center">
                <input
                  type="radio"
                  value="sibling"
                  checked={position === 'sibling'}
                  onChange={(e) => setPosition(e.target.value as 'sibling')}
                  className="mr-2"
                />
                As sibling of selected note
              </label>
            </div>
          </div>
        )}

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
