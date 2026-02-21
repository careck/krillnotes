import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ScriptEditor from './ScriptEditor';
import type { UserScript } from '../types';

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

function ScriptManagerDialog({ isOpen, onClose, onScriptsChanged }: ScriptManagerDialogProps) {
  const [view, setView] = useState<View>('list');
  const [scripts, setScripts] = useState<UserScript[]>([]);
  const [editingScript, setEditingScript] = useState<UserScript | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);

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
    setEditingScript(null);
    setEditorContent(NEW_SCRIPT_TEMPLATE);
    setError('');
    setView('editor');
  };

  const handleEdit = (script: UserScript) => {
    setEditingScript(script);
    setEditorContent(script.sourceCode);
    setError('');
    setView('editor');
  };

  const handleToggle = async (script: UserScript) => {
    try {
      await invoke('toggle_user_script', { scriptId: script.id, enabled: !script.enabled });
      await loadScripts();
      onScriptsChanged?.();
    } catch (err) {
      setError(`Failed to toggle script: ${err}`);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      if (editingScript) {
        await invoke<UserScript>('update_user_script', {
          scriptId: editingScript.id,
          sourceCode: editorContent,
        });
      } else {
        await invoke<UserScript>('create_user_script', {
          sourceCode: editorContent,
        });
      }
      await loadScripts();
      setView('list');
      onScriptsChanged?.();
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
      await invoke('delete_user_script', { scriptId: editingScript.id });
      await loadScripts();
      setView('list');
      setError('');
      onScriptsChanged?.();
    } catch (err) {
      setError(`Failed to delete: ${err}`);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg w-[700px] max-h-[80vh] flex flex-col">
        {view === 'list' ? (
          <>
            {/* List View Header */}
            <div className="flex items-center justify-between p-4 border-b border-border">
              <h2 className="text-xl font-bold">User Scripts</h2>
              <button
                onClick={handleAdd}
                className="px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
              >
                + Add
              </button>
            </div>

            {/* Script List */}
            <div className="flex-1 overflow-y-auto p-4">
              {scripts.length === 0 ? (
                <p className="text-muted-foreground text-center py-8">
                  No user scripts yet. Click "+ Add" to create one.
                </p>
              ) : (
                <div className="space-y-2">
                  {scripts.map(script => (
                    <div
                      key={script.id}
                      className="flex items-center gap-3 p-3 border border-border rounded-md hover:bg-secondary/50"
                    >
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
                      <span className="text-xs text-muted-foreground shrink-0">
                        #{script.loadOrder}
                      </span>
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

            {/* Editor */}
            <div className="flex-1 overflow-hidden p-4">
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
                  onClick={() => { setView('list'); setError(''); }}
                  className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
                  disabled={saving}
                >
                  Cancel
                </button>
                <button
                  onClick={handleSave}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
                  disabled={saving}
                >
                  {saving ? 'Saving...' : 'Save'}
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
