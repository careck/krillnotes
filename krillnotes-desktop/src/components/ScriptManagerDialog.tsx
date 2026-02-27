import { useState, useEffect, useCallback } from 'react';
import { GripVertical } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import ScriptEditor from './ScriptEditor';
import type { UserScript, ScriptError, ScriptMutationResult } from '../types';

interface ScriptManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onScriptsChanged?: () => void;
}

const NEW_SCRIPT_TEMPLATE = `// @name: New Script
// @description:

schema("NewType", #{
    fields: [
        #{ name: "body", type: "textarea" },
    ]
});
`;

type View = 'list' | 'editor';

function parseFrontMatterName(source: string): string {
  for (const line of source.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('//')) {
      if (trimmed === '') continue;
      break;
    }
    const body = trimmed.replace(/^\/\/\s*/, '');
    if (body.startsWith('@name:')) {
      return body.slice('@name:'.length).trim();
    }
  }
  return '';
}

function ScriptManagerDialog({ isOpen, onClose, onScriptsChanged }: ScriptManagerDialogProps) {
  const [view, setView] = useState<View>('list');
  const [scripts, setScripts] = useState<UserScript[]>([]);
  const [editingScript, setEditingScript] = useState<UserScript | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [importConflict, setImportConflict] = useState<UserScript | null>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);

  const loadScripts = useCallback(async () => {
    try {
      const result = await invoke<UserScript[]>('list_user_scripts');
      setScripts(result);
    } catch (err) {
      setError(`Failed to load scripts: ${err}`);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadScripts();
      setView('list');
      setError('');
    }
  }, [isOpen, loadScripts]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (view === 'editor') {
          setView('list');
          setError('');
        } else {
          onClose();
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, view, onClose]);

  if (!isOpen) return null;

  const handleAdd = () => {
    setImportConflict(null);
    setEditingScript(null);
    setEditorContent(NEW_SCRIPT_TEMPLATE);
    setError('');
    setView('editor');
  };

  const handleEdit = (script: UserScript) => {
    setImportConflict(null);
    setEditingScript(script);
    setEditorContent(script.sourceCode);
    setError('');
    setView('editor');
  };

  const formatLoadErrors = (loadErrors: ScriptError[]): string =>
    loadErrors.map(e => `Script "${e.scriptName}": ${e.message}`).join('\n');

  const handleToggle = async (script: UserScript) => {
    try {
      const loadErrors = await invoke<ScriptError[]>('toggle_user_script', {
        scriptId: script.id,
        enabled: !script.enabled,
      });
      await loadScripts();
      onScriptsChanged?.();
      if (loadErrors.length > 0) {
        setError(`Script reload errors:\n${formatLoadErrors(loadErrors)}`);
      }
    } catch (err) {
      setError(`Failed to toggle script: ${err}`);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      let loadErrors: ScriptError[];
      if (editingScript) {
        const result = await invoke<ScriptMutationResult<UserScript>>('update_user_script', {
          scriptId: editingScript.id,
          sourceCode: editorContent,
        });
        loadErrors = result.loadErrors;
      } else {
        const result = await invoke<ScriptMutationResult<UserScript>>('create_user_script', {
          sourceCode: editorContent,
        });
        loadErrors = result.loadErrors;
      }
      await loadScripts();
      setImportConflict(null);
      setView('list');
      onScriptsChanged?.();
      if (loadErrors.length > 0) {
        setError(`Script reload errors:\n${formatLoadErrors(loadErrors)}`);
      }
    } catch (err) {
      setError(`${err}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!editingScript) return;
    const confirmed = window.confirm(
      "Deleting this script may remove schema definitions used by existing notes. " +
      "Their data will be preserved in the database but may not display correctly " +
      "until a compatible schema is re-registered. Delete anyway?"
    );
    if (!confirmed) return;
    try {
      const loadErrors = await invoke<ScriptError[]>('delete_user_script', { scriptId: editingScript.id });
      await loadScripts();
      setView('list');
      setError(loadErrors.length > 0 ? `Script reload errors:\n${formatLoadErrors(loadErrors)}` : '');
      onScriptsChanged?.();
    } catch (err) {
      setError(`Failed to delete: ${err}`);
    }
  };

  const handleImportFromFile = async () => {
    setError('');
    const path = await open({
      filters: [{ name: 'Rhai Script', extensions: ['rhai'] }],
      multiple: false,
    });
    if (!path) return;
    try {
      const content = await invoke<string>('read_file_content', { path });
      const name = parseFrontMatterName(content);
      const conflict = name ? (scripts.find(s => s.name === name) ?? null) : null;
      setImportConflict(conflict);
      setEditingScript(conflict ?? null);
      setEditorContent(content);
      setError('');
      setView('editor');
    } catch (e) {
      setError(`Failed to read file: ${e}`);
    }
  };

  const handleSaveOrReplace = async () => {
    if (importConflict) {
      const confirmed = confirm(`Replace script "${importConflict.name}"? This cannot be undone.`);
      if (!confirmed) return;
    }
    await handleSave();
  };

  const handleDragStart = (index: number) => {
    setDragIndex(index);
  };

  const handleDragOver = (e: React.DragEvent, index: number) => {
    e.preventDefault();
    setDragOverIndex(index);
  };

  const handleDrop = async (e: React.DragEvent, dropIndex: number) => {
    e.preventDefault();
    if (dragIndex === null || dragIndex === dropIndex) {
      setDragIndex(null);
      setDragOverIndex(null);
      return;
    }

    const reordered = [...scripts];
    const [moved] = reordered.splice(dragIndex, 1);
    reordered.splice(dropIndex, 0, moved);
    setScripts(reordered);
    setDragIndex(null);
    setDragOverIndex(null);

    try {
      const loadErrors = await invoke<ScriptError[]>('reorder_all_user_scripts', {
        scriptIds: reordered.map(s => s.id),
      });
      onScriptsChanged?.();
      if (loadErrors.length > 0) {
        setError(`Script reload errors:\n${formatLoadErrors(loadErrors)}`);
      }
    } catch (err) {
      setError(`Failed to reorder scripts: ${err}`);
      await loadScripts();
    }
  };

  const handleDragEnd = () => {
    setDragIndex(null);
    setDragOverIndex(null);
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg w-[700px] h-[80vh] flex flex-col">
        {view === 'list' ? (
          <>
            {/* List View Header */}
            <div className="flex items-center justify-between p-4 border-b border-border">
              <h2 className="text-xl font-bold">User Scripts</h2>
              <div className="flex items-center gap-2">
                <button
                  onClick={handleAdd}
                  className="px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
                >
                  + Add
                </button>
                <button
                  onClick={handleImportFromFile}
                  className="px-3 py-1.5 border border-border rounded-md hover:bg-secondary text-sm"
                >
                  Import from file…
                </button>
              </div>
            </div>

            {/* Script List */}
            <div className="flex-1 overflow-y-auto p-4">
              {scripts.length === 0 ? (
                <p className="text-muted-foreground text-center py-8">
                  No user scripts yet. Click "+ Add" to create one.
                </p>
              ) : (
                <div className="space-y-2">
                  {scripts.map((script, index) => (
                    <div
                      key={script.id}
                      draggable
                      onDragStart={() => handleDragStart(index)}
                      onDragOver={(e) => handleDragOver(e, index)}
                      onDrop={(e) => handleDrop(e, index)}
                      onDragEnd={handleDragEnd}
                      className={[
                        'flex items-center gap-3 p-3 border border-border rounded-md hover:bg-secondary/50 transition-opacity',
                        dragIndex === index ? 'opacity-40' : '',
                        dragOverIndex === index && dragIndex !== index ? 'border-t-2 border-t-primary' : '',
                      ].join(' ')}
                    >
                      <GripVertical
                        size={16}
                        className="shrink-0 text-muted-foreground cursor-grab active:cursor-grabbing"
                      />
                      <input
                        type="checkbox"
                        checked={script.enabled}
                        onChange={() => handleToggle(script)}
                        className="shrink-0"
                        title={script.enabled ? 'Disable script' : 'Enable script'}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="font-medium truncate">
                          {script.name || '(unnamed)'}
                        </div>
                        {script.description && (
                          <div className="text-sm text-muted-foreground truncate">
                            {script.description}
                          </div>
                        )}
                      </div>
                      <button
                        onClick={() => handleEdit(script)}
                        className="px-2 py-1 text-sm border border-border rounded hover:bg-secondary"
                      >
                        Edit
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Error display */}
            {error && (
              <div className="px-4 pb-2">
                <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                  {error}
                </div>
              </div>
            )}

            {/* Footer */}
            <div className="flex justify-end p-4 border-t border-border">
              <button
                onClick={onClose}
                className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
              >
                Close
              </button>
            </div>
          </>
        ) : (
          <>
            {/* Editor View Header */}
            <div className="p-4 border-b border-border">
              <h2 className="text-xl font-bold">
                {editingScript ? `Edit: ${editingScript.name}` : 'New Script'}
              </h2>
            </div>

            {importConflict && (
              <div className="px-4 py-2 text-sm text-yellow-700 bg-yellow-50 border-b border-yellow-200 dark:bg-yellow-900/20 dark:text-yellow-300">
                A script named "{importConflict.name}" already exists. Saving will replace it.
              </div>
            )}

            {/* Editor */}
            <div className="flex-1 min-h-0 overflow-hidden p-4 flex flex-col">
              <ScriptEditor value={editorContent} onChange={setEditorContent} />
            </div>

            {/* Error display */}
            {error && (
              <div className="px-4 pb-2">
                <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm whitespace-pre-wrap">
                  {error}
                </div>
              </div>
            )}

            {/* Footer */}
            <div className="flex justify-between p-4 border-t border-border">
              <div>
                {editingScript && (
                  <button
                    onClick={handleDelete}
                    className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
                    disabled={saving}
                  >
                    Delete
                  </button>
                )}
              </div>
              <div className="flex gap-2">
                <button
                  onClick={() => { setImportConflict(null); setView('list'); setError(''); }}
                  className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
                  disabled={saving}
                >
                  Cancel
                </button>
                <button
                  onClick={handleSaveOrReplace}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
                  disabled={saving}
                >
                  {saving ? 'Saving...' : (importConflict ? 'Replace' : 'Save')}
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export default ScriptManagerDialog;
