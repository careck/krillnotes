// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, useCallback } from 'react';
import { GripVertical } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { open, confirm } from '@tauri-apps/plugin-dialog';
import ScriptEditor from './ScriptEditor';
import type { UserScript, ScriptError, ScriptMutationResult, ScriptWarning } from '../types';
import { useTranslation } from 'react-i18next';
import { parseFrontMatterName } from '../utils/scriptHelpers';

interface ScriptManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onScriptsChanged?: () => void;
}

const SCHEMA_TEMPLATE = `// @name: MyType
// @description: Describe your note type here

schema("MyType", #{
    version: 1,
    fields: [
        #{ name: "body", type: "textarea" },
    ],
    on_save: |note| {
        commit();
    }
});
`;

const PRESENTATION_TEMPLATE = `// @name: MyType Views
// @description: Views and actions for MyType

register_view("MyType", "Summary", |note| {
    text("Custom view for " + note.title)
});
`;

type View = 'list' | 'editor';

function ScriptManagerDialog({ isOpen, onClose, onScriptsChanged }: ScriptManagerDialogProps) {
  const { t } = useTranslation();
  const [view, setView] = useState<View>('list');
  const [scripts, setScripts] = useState<UserScript[]>([]);
  const [editingScript, setEditingScript] = useState<UserScript | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [importConflict, setImportConflict] = useState<UserScript | null>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
  const [canScriptUndo, setCanScriptUndo] = useState(false);
  const [canScriptRedo, setCanScriptRedo] = useState(false);
  const [newScriptCategory, setNewScriptCategory] = useState<string>('schema');
  const [warnings, setWarnings] = useState<ScriptWarning[]>([]);
  const [isOwner, setIsOwner] = useState(true); // optimistic default

  const loadScripts = useCallback(async () => {
    try {
      const result = await invoke<UserScript[]>('list_user_scripts');
      setScripts(result);
    } catch (err) {
      setError(t('scripts.failedLoad', { error: String(err) }));
    }
  }, [t]);

  const refreshScriptUndoState = useCallback(async () => {
    const [u, r] = await Promise.all([
      invoke<boolean>('can_script_undo'),
      invoke<boolean>('can_script_redo'),
    ]);
    setCanScriptUndo(u);
    setCanScriptRedo(r);
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadScripts();
      refreshScriptUndoState();
      invoke<ScriptWarning[]>('get_script_warnings').then(setWarnings).catch(() => setWarnings([]));
      invoke<boolean>('is_workspace_owner').then(setIsOwner).catch(() => setIsOwner(false));
      setView('list');
      setError('');
      setImportConflict(null);
    }
  }, [isOpen, loadScripts, refreshScriptUndoState]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (view === 'editor') {
          setImportConflict(null);
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
    setEditorContent(newScriptCategory === 'schema' ? SCHEMA_TEMPLATE : PRESENTATION_TEMPLATE);
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
        setError(t('scripts.reloadErrors', { errors: formatLoadErrors(loadErrors) }));
      }
    } catch (err) {
      setError(t('scripts.failedToggle', { error: String(err) }));
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
          category: newScriptCategory,
        });
        loadErrors = result.loadErrors;
      }
      await loadScripts();
      await refreshScriptUndoState();
      setImportConflict(null);
      setView('list');
      onScriptsChanged?.();
      if (loadErrors.length > 0) {
        setError(t('scripts.reloadErrors', { errors: formatLoadErrors(loadErrors) }));
      }
    } catch (err) {
      setError(`${err}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!editingScript) return;
    const confirmed = await confirm(t('scripts.deleteWarning'));
    if (!confirmed) return;
    try {
      const loadErrors = await invoke<ScriptError[]>('delete_user_script', { scriptId: editingScript.id });
      await loadScripts();
      await refreshScriptUndoState();
      setImportConflict(null);
      setView('list');
      setError(loadErrors.length > 0 ? t('scripts.reloadErrors', { errors: formatLoadErrors(loadErrors) }) : '');
      onScriptsChanged?.();
    } catch (err) {
      setError(t('scripts.failedDelete', { error: String(err) }));
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
      const freshScripts = await invoke<UserScript[]>('list_user_scripts');
      setScripts(freshScripts);
      const name = parseFrontMatterName(content);
      const conflict = name ? (freshScripts.find(s => s.name === name) ?? null) : null;
      const isSchema = typeof path === 'string' && path.endsWith('.schema.rhai');
      setNewScriptCategory(isSchema ? 'schema' : 'presentation');
      setImportConflict(conflict);
      setEditingScript(conflict ?? null);
      setEditorContent(content);
      setView('editor');
    } catch (e) {
      setError(t('scripts.failedImport', { error: String(e) }));
    }
  };

  const handleSaveOrReplace = async () => {
    if (importConflict) {
      const confirmed = await confirm(t('scripts.replaceConfirm', { name: importConflict.name }));
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
        setError(t('scripts.reloadErrors', { errors: formatLoadErrors(loadErrors) }));
      }
    } catch (err) {
      setError(t('scripts.failedReorder', { error: String(err) }));
      await loadScripts();
    }
  };

  const handleDragEnd = () => {
    setDragIndex(null);
    setDragOverIndex(null);
  };

  const handleScriptUndo = async () => {
    try {
      await invoke('script_undo');
      await loadScripts();
      await refreshScriptUndoState();
      onScriptsChanged?.();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleScriptRedo = async () => {
    try {
      await invoke('script_redo');
      await loadScripts();
      await refreshScriptUndoState();
      onScriptsChanged?.();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className={`bg-background border border-border rounded-lg h-[80vh] flex flex-col transition-[width] duration-200 ${view === 'list' ? 'w-[700px]' : 'w-[90vw]'}`}>
        {view === 'list' ? (
          <>
            {/* List View Header */}
            <div className="flex items-center justify-between p-4 border-b border-border">
              <h2 className="text-xl font-bold">{t('scripts.title')}</h2>
              <div className="flex items-center gap-2">
                <div className="flex items-center gap-1.5 text-sm mr-1">
                  <label className="flex items-center gap-1 cursor-pointer">
                    <input
                      type="radio"
                      name="newCategory"
                      checked={newScriptCategory === 'schema'}
                      onChange={() => setNewScriptCategory('schema')}
                      disabled={!isOwner}
                    />
                    {t('scripts.schema')}
                  </label>
                  <label className="flex items-center gap-1 cursor-pointer">
                    <input
                      type="radio"
                      name="newCategory"
                      checked={newScriptCategory === 'presentation'}
                      onChange={() => setNewScriptCategory('presentation')}
                      disabled={!isOwner}
                    />
                    {t('scripts.library')}
                  </label>
                </div>
                <button
                  onClick={handleAdd}
                  disabled={!isOwner}
                  className="px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {t('common.add')}
                </button>
                <button
                  onClick={handleImportFromFile}
                  disabled={!isOwner}
                  className="px-3 py-1.5 border border-border rounded-md hover:bg-secondary text-sm disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {t('scripts.importFromFile')}
                </button>
              </div>
            </div>

            {!isOwner && (
              <div className="mx-4 mt-3 p-2 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-400 text-xs">
                {t('scripts.ownerOnly', 'Only the workspace owner can modify scripts.')}
              </div>
            )}

            {/* Script List */}
            <div className="flex-1 overflow-y-auto p-4">
              {scripts.length === 0 ? (
                <p className="text-muted-foreground text-center py-8">
                  {t('scripts.noScripts')}
                </p>
              ) : (
                <div className="space-y-2">
                  {scripts.map((script, index) => (
                    <div
                      key={script.id}
                      draggable={isOwner}
                      onDragStart={isOwner ? () => handleDragStart(index) : undefined}
                      onDragOver={isOwner ? (e) => handleDragOver(e, index) : undefined}
                      onDrop={isOwner ? (e) => handleDrop(e, index) : undefined}
                      onDragEnd={isOwner ? handleDragEnd : undefined}
                      className={[
                        'flex items-center gap-3 p-3 border border-border rounded-md hover:bg-secondary/50 transition-opacity',
                        dragIndex === index ? 'opacity-40' : '',
                        dragOverIndex === index && dragIndex !== index ? 'border-t-2 border-t-primary' : '',
                      ].join(' ')}
                    >
                      <GripVertical
                        size={16}
                        className={`shrink-0 text-muted-foreground ${isOwner ? 'cursor-grab active:cursor-grabbing' : 'opacity-40'}`}
                      />
                      <input
                        type="checkbox"
                        checked={script.enabled}
                        onChange={() => handleToggle(script)}
                        className="shrink-0"
                        disabled={!isOwner}
                        title={script.enabled ? t('scripts.disableScript') : t('scripts.enableScript')}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="font-medium truncate flex items-center gap-2">
                          {script.name || t('scripts.unnamed')}
                          <span className={`text-xs px-1.5 py-0.5 rounded ${
                            script.category === 'schema'
                              ? 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300'
                              : 'bg-amber-100 text-amber-700 dark:bg-amber-900 dark:text-amber-300'
                          }`}>
                            {script.category === 'schema' ? t('scripts.schema') : t('scripts.library')}
                          </span>
                          {warnings.filter(w => w.scriptName === script.name).length > 0 && (
                            <span
                              title={warnings.filter(w => w.scriptName === script.name).map(w => w.message).join('\n')}
                              className="text-amber-500 cursor-help font-bold text-xs"
                            >
                              ⚠
                            </span>
                          )}
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
                        {t('common.edit')}
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
            <div className="flex justify-between items-center p-4 border-t border-border">
              <div className="flex gap-2">
                <button
                  onClick={handleScriptUndo}
                  disabled={!canScriptUndo || !isOwner}
                  title={t('scripts.undoLastSave')}
                  className="px-3 py-1.5 text-sm border border-border rounded-md hover:bg-secondary disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {t('common.undo')}
                </button>
                <button
                  onClick={handleScriptRedo}
                  disabled={!canScriptRedo || !isOwner}
                  title={t('scripts.redoLastSave')}
                  className="px-3 py-1.5 text-sm border border-border rounded-md hover:bg-secondary disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {t('common.redo')}
                </button>
              </div>
              <button
                onClick={onClose}
                className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
              >
                {t('common.close')}
              </button>
            </div>
          </>
        ) : (
          <>
            {/* Editor View Header */}
            <div className="p-4 border-b border-border">
              <h2 className="text-xl font-bold">
                {editingScript ? t('scripts.editing', { name: editingScript.name }) : t('scripts.newScript')}
              </h2>
            </div>

            {importConflict && (
              <div className="px-4 py-2 text-sm text-yellow-700 bg-yellow-50 border-b border-yellow-200 dark:bg-yellow-900/20 dark:text-yellow-300">
                {t('scripts.conflictWarning', { name: importConflict.name })}
              </div>
            )}

            {/* Editor */}
            <div className="flex-1 min-h-0 overflow-hidden p-4 flex flex-col">
              <ScriptEditor value={editorContent} onChange={setEditorContent} readOnly={!isOwner} />
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
                    className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600 disabled:opacity-40 disabled:cursor-not-allowed"
                    disabled={saving || !isOwner}
                  >
                    {t('common.delete')}
                  </button>
                )}
              </div>
              <div className="flex gap-2">
                <button
                  onClick={() => { setImportConflict(null); setView('list'); setError(''); }}
                  className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
                  disabled={saving}
                >
                  {t('common.cancel')}
                </button>
                <button
                  onClick={handleSaveOrReplace}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-40 disabled:cursor-not-allowed"
                  disabled={saving || !isOwner}
                >
                  {saving ? t('common.saving') : (importConflict ? t('common.replace') : t('common.save'))}
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
