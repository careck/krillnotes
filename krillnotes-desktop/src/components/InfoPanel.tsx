import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, FieldDefinition, FieldValue } from '../types';
import FieldDisplay from './FieldDisplay';
import FieldEditor from './FieldEditor';

interface InfoPanelProps {
  selectedNote: Note | null;
  onNoteUpdated: () => void;
  onDeleteRequest: (noteId: string) => void;
  requestEditMode: number;
}

function InfoPanel({ selectedNote, onNoteUpdated, onDeleteRequest, requestEditMode }: InfoPanelProps) {
  const [schemaFields, setSchemaFields] = useState<FieldDefinition[]>([]);
  const [isEditing, setIsEditing] = useState(false);
  const [editedTitle, setEditedTitle] = useState('');
  const [editedFields, setEditedFields] = useState<Record<string, FieldValue>>({});
  const [isDirty, setIsDirty] = useState(false);
  const titleInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!selectedNote) {
      setSchemaFields([]);
      setIsEditing(false);
      return;
    }

    invoke<FieldDefinition[]>('get_schema_fields', { nodeType: selectedNote.nodeType })
      .then(fields => setSchemaFields(fields))
      .catch(err => {
        console.error('Failed to fetch schema fields:', err);
        setSchemaFields([]);
      });
  }, [selectedNote?.id]);

  useEffect(() => {
    if (selectedNote) {
      setIsEditing(false);
      setEditedTitle(selectedNote.title);
      setEditedFields({ ...selectedNote.fields });
      setIsDirty(false);
    }
  }, [selectedNote?.id]);

  // Enter edit mode when WorkspaceView requests it (e.g. via context menu, note creation).
  // NOTE: This effect must be declared AFTER the selectedNote?.id effect above.
  // When note creation triggers both a selection change and a requestEditMode increment,
  // the IPC await in handleSelectNote separates them into different renders (selection
  // resets isEditing first, then this effect sets it to true). The declaration order
  // ensures correct behaviour if they ever land in the same render.
  useEffect(() => {
    if (requestEditMode > 0 && selectedNote) {
      setIsEditing(true);
    }
  }, [requestEditMode]);

  // Focus title input whenever edit mode activates
  useEffect(() => {
    if (isEditing && titleInputRef.current) {
      titleInputRef.current.focus();
    }
  }, [isEditing]);

  const handleEdit = () => {
    setIsEditing(true);
  };

  const handleCancel = () => {
    if (isDirty) {
      if (!confirm('Discard changes?')) {
        return;
      }
    }
    setIsEditing(false);
    setEditedTitle(selectedNote!.title);
    setEditedFields({ ...selectedNote!.fields });
    setIsDirty(false);
  };

  const handleSave = async () => {
    if (!selectedNote) return;

    try {
      const updatedNote = await invoke<Note>('update_note', {
        noteId: selectedNote.id,
        title: editedTitle,
        fields: editedFields,
      });
      setEditedTitle(updatedNote.title);
      setEditedFields({ ...updatedNote.fields });
      setIsEditing(false);
      setIsDirty(false);
      onNoteUpdated();
    } catch (err) {
      alert(`Failed to save: ${err}`);
    }
  };

  const handleFieldChange = (fieldName: string, value: FieldValue) => {
    setEditedFields(prev => ({ ...prev, [fieldName]: value }));
    setIsDirty(true);
  };

  if (!selectedNote) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Select a note to view details
      </div>
    );
  }

  const formatTimestamp = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleString();
  };

  const schemaFieldNames = new Set(schemaFields.map(f => f.name));
  const allFieldNames = Object.keys(selectedNote.fields);
  const legacyFieldNames = allFieldNames.filter(name => !schemaFieldNames.has(name));

  return (
    <div className={`p-6 ${isEditing ? 'border-2 border-primary rounded-lg' : ''}`}>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        {isEditing ? (
          <input
            ref={titleInputRef}
            type="text"
            value={editedTitle}
            onChange={(e) => {
              setEditedTitle(e.target.value);
              setIsDirty(true);
            }}
            className="text-4xl font-bold bg-background border border-border rounded-md px-2 py-1 flex-1"
          />
        ) : (
          <h1 className="text-4xl font-bold">{selectedNote.title}</h1>
        )}
        <div className="flex gap-2 ml-4">
          {isEditing ? (
            <>
              <button
                onClick={handleSave}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                Save
              </button>
              <button
                onClick={handleCancel}
                className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              <button
                onClick={handleEdit}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                Edit
              </button>
              <button
                onClick={() => onDeleteRequest(selectedNote.id)}
                className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
              >
                Delete
              </button>
            </>
          )}
        </div>
      </div>

      {/* Fields Section */}
      <div className="mb-6">
        <h2 className="text-xl font-semibold mb-4">Fields</h2>

        {schemaFields.map(field => (
          isEditing ? (
            <FieldEditor
              key={field.name}
              fieldName={field.name}
              value={editedFields[field.name] || { Text: '' }}
              required={field.required}
              onChange={(value) => handleFieldChange(field.name, value)}
            />
          ) : (
            <FieldDisplay
              key={field.name}
              fieldName={field.name}
              value={selectedNote.fields[field.name] || { Text: '' }}
            />
          )
        ))}

        {legacyFieldNames.length > 0 && (
          <>
            <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
              Legacy Fields
            </h3>
            {legacyFieldNames.map(name => (
              isEditing ? (
                <FieldEditor
                  key={name}
                  fieldName={`${name} (legacy)`}
                  value={editedFields[name] || { Text: '' }}
                  required={false}
                  onChange={(value) => handleFieldChange(name, value)}
                />
              ) : (
                <FieldDisplay
                  key={name}
                  fieldName={`${name} (legacy)`}
                  value={selectedNote.fields[name]}
                />
              )
            ))}
          </>
        )}

        {schemaFields.length === 0 && legacyFieldNames.length === 0 && (
          <p className="text-muted-foreground italic">No fields</p>
        )}
      </div>

      {/* Metadata Section */}
      <div className="bg-secondary p-6 rounded-lg space-y-4">
        <div>
          <p className="text-sm text-muted-foreground">Type</p>
          <p className="text-lg">{selectedNote.nodeType}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">Created</p>
          <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">Modified</p>
          <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">ID</p>
          <p className="text-xs font-mono">{selectedNote.id}</p>
        </div>
      </div>
    </div>
  );
}

export default InfoPanel;
