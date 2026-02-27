import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open, confirm } from '@tauri-apps/plugin-dialog';
import { EditorView, keymap, lineNumbers, highlightActiveLine } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { json } from '@codemirror/lang-json';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching } from '@codemirror/language';
import { useTheme } from '../contexts/ThemeContext';
import type { ThemeMeta } from '../utils/theme';

const NEW_THEME_TEMPLATE = `{
  "name": "My Theme",

  // Delete whichever variant you don't need (or keep both for a complete theme).

  "light-theme": {
    "colors": {
      // "background": "oklch(97% 0.02 210)",
      // "foreground": "oklch(10% 0.04 222)",
      // "primary":    "oklch(35% 0.10 240)"
    },
    "typography": {
      // "fontFamily": "\\"Georgia\\", serif",
      // "fontSize":   "14px",
      // "lineHeight": "1.6"
    },
    "spacing": {
      // "scale": 1.0
    }
    // "iconSize": "16px"
  },

  "dark-theme": {
    "colors": {
      // "background": "oklch(10% 0.04 240)",
      // "foreground": "oklch(95% 0.02 210)",
      // "primary":    "oklch(65% 0.15 240)"
    },
    "typography": {
      // "fontFamily": "\\"JetBrains Mono\\", monospace",
      // "fontSize":   "14px",
      // "lineHeight": "1.6"
    },
    "spacing": {
      // "scale": 1.0
    }
    // "iconSize": "16px"
  }
}
`;

const BUILT_IN_NAMES = ['light', 'dark'];

interface Props {
  isOpen: boolean;
  onClose: () => void;
}

type View = 'list' | 'editor';

export default function ManageThemesDialog({ isOpen, onClose }: Props) {
  const { themes, reloadThemes, lightThemeName, darkThemeName, setLightTheme, setDarkTheme } = useTheme();
  const [view, setView] = useState<View>('list');
  const [editingMeta, setEditingMeta] = useState<ThemeMeta | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [importConflict, setImportConflict] = useState<ThemeMeta | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);

  useEffect(() => {
    if (isOpen) { reloadThemes(); setView('list'); setError(''); setImportConflict(null); }
  }, [isOpen, reloadThemes]);

  // CodeMirror lifecycle
  useEffect(() => {
    if (view !== 'editor' || !containerRef.current) return;
    viewRef.current?.destroy();
    const isBuiltIn = editingMeta ? BUILT_IN_NAMES.includes(editingMeta.name) : false;
    const state = EditorState.create({
      doc: editorContent,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        history(),
        bracketMatching(),
        json(),
        syntaxHighlighting(defaultHighlightStyle),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.editable.of(!isBuiltIn),
        EditorView.updateListener.of(update => {
          if (update.docChanged) setEditorContent(update.state.doc.toString());
        }),
        EditorView.theme({
          '&': { height: '100%', fontSize: '13px' },
          '.cm-scroller': { overflow: 'auto', fontFamily: 'monospace' },
        }),
      ],
    });
    viewRef.current = new EditorView({ state, parent: containerRef.current });
    return () => { viewRef.current?.destroy(); viewRef.current = null; };
  }, [view, editingMeta]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => {
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
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [isOpen, view, onClose]);

  if (!isOpen) return null;

  const handleNew = () => {
    setEditingMeta(null);
    setEditorContent(NEW_THEME_TEMPLATE);
    setError('');
    setImportConflict(null);
    setView('editor');
  };

  const handleEdit = async (meta: ThemeMeta) => {
    if (BUILT_IN_NAMES.includes(meta.name)) {
      setEditingMeta(meta);
      const preview = JSON.stringify(
        meta.name === 'light'
          ? { name: 'light (built-in)', note: 'This is the default light theme. Create a new theme that extends it by setting "light-theme": {}.' }
          : { name: 'dark (built-in)', note: 'This is the default dark theme. Create a new theme that extends it by setting "dark-theme": {}.' },
        null, 2
      );
      setEditorContent(preview);
      setError('');
      setImportConflict(null);
      setView('editor');
      return;
    }
    try {
      const content = await invoke<string>('read_theme', { filename: meta.filename });
      setEditingMeta(meta);
      setEditorContent(content);
      setError('');
      setImportConflict(null);
      setView('editor');
    } catch (e) {
      setError(`Failed to read theme: ${e}`);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      let parsed: { name?: string };
      let cleaned: string;
      try {
        cleaned = editorContent
          .split('\n')
          .filter(line => !/^\s*\/\//.test(line))
          .join('\n')
          .replace(/,(\s*[}\]])/g, '$1');
        parsed = JSON.parse(cleaned);
      } catch {
        throw new Error('Invalid JSON — check for syntax errors.');
      }
      const name = parsed.name ?? 'unnamed';
      const filename = editingMeta?.filename ?? `${name.toLowerCase().replace(/\s+/g, '-')}.krilltheme`;
      await invoke('write_theme', { filename, content: cleaned });
      await reloadThemes();
      setImportConflict(null);
      setView('list');
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (meta: ThemeMeta) => {
    if (!await confirm(`Delete theme "${meta.name}"?`)) return;
    try {
      await invoke('delete_theme', { filename: meta.filename });
      await reloadThemes();
    } catch (e) {
      setError(`Failed to delete: ${e}`);
    }
  };

  const handleImportFromFile = async () => {
    setError('');
    const path = await open({
      filters: [{ name: 'Krillnotes Theme', extensions: ['krilltheme'] }],
      multiple: false,
    });
    if (!path) return;
    try {
      const content = await invoke<string>('read_file_content', { path });
      const cleaned = content
        .split('\n')
        .filter(line => !/^\s*\/\//.test(line))
        .join('\n')
        .replace(/,(\s*[}\]])/g, '$1');
      let parsed: { name?: string };
      try {
        parsed = JSON.parse(cleaned);
      } catch {
        setError('Invalid theme file — could not parse JSON.');
        return;
      }
      const name = parsed.name ?? 'unnamed';
      const conflict = themes.find(t => t.name === name) ?? null;
      setImportConflict(conflict);
      setEditingMeta(conflict ?? null);
      setEditorContent(content);
      setError('');
      setView('editor');
    } catch (e) {
      setError(`Failed to read file: ${e}`);
    }
  };

  const handleSaveOrReplace = async () => {
    if (importConflict) {
      const confirmed = await confirm(`Replace theme "${importConflict.name}"? This cannot be undone.`);
      if (!confirmed) return;
    }
    await handleSave();
  };

  const BUILT_IN_METAS: ThemeMeta[] = [
    { name: 'light', filename: '', hasLight: true, hasDark: false },
    { name: 'dark',  filename: '', hasLight: false, hasDark: true },
  ];

  const allThemes = [...BUILT_IN_METAS, ...themes];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-background border border-border rounded-lg w-[700px] max-h-[80vh] flex flex-col shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="font-semibold text-foreground">
            {view === 'list' ? 'Manage Themes' : (editingMeta ? `Editing: ${editingMeta.name}` : 'New Theme')}
          </h2>
          <button
            onClick={view === 'editor' ? () => { setView('list'); setImportConflict(null); setError(''); } : onClose}
            className="text-muted-foreground hover:text-foreground text-sm"
          >
            {view === 'editor' ? '← Back' : '✕'}
          </button>
        </div>

        {/* Error */}
        {error && (
          <div className="px-4 py-2 text-sm text-red-600 bg-red-50 border-b border-border">{error}</div>
        )}

        {/* List view */}
        {view === 'list' && (
          <>
            <div className="flex-1 overflow-y-auto">
              {allThemes.map((meta) => {
                const isBuiltIn = BUILT_IN_NAMES.includes(meta.name);
                const isActiveLight = lightThemeName === meta.name;
                const isActiveDark  = darkThemeName  === meta.name;
                return (
                  <div
                    key={meta.filename || meta.name}
                    className="flex items-center justify-between px-4 py-2 border-b border-border hover:bg-secondary/50"
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="font-medium text-foreground truncate">{meta.name}</span>
                      {isBuiltIn && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-muted text-muted-foreground">built-in</span>
                      )}
                      {meta.hasLight && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">light</span>
                      )}
                      {meta.hasDark && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">dark</span>
                      )}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {meta.hasLight && (
                        <button
                          onClick={() => setLightTheme(meta.name)}
                          className={`text-xs px-2 py-1 rounded border ${
                            isActiveLight
                              ? 'bg-primary text-primary-foreground border-primary'
                              : 'border-border text-muted-foreground hover:text-foreground'
                          }`}
                        >
                          {isActiveLight ? '✓ Light' : 'Set Light'}
                        </button>
                      )}
                      {meta.hasDark && (
                        <button
                          onClick={() => setDarkTheme(meta.name)}
                          className={`text-xs px-2 py-1 rounded border ${
                            isActiveDark
                              ? 'bg-primary text-primary-foreground border-primary'
                              : 'border-border text-muted-foreground hover:text-foreground'
                          }`}
                        >
                          {isActiveDark ? '✓ Dark' : 'Set Dark'}
                        </button>
                      )}
                      <button
                        onClick={() => handleEdit(meta)}
                        className="text-xs text-muted-foreground hover:text-foreground"
                      >
                        {isBuiltIn ? 'View' : 'Edit'}
                      </button>
                      {!isBuiltIn && (
                        <button
                          onClick={() => handleDelete(meta)}
                          className="text-xs text-red-500 hover:text-red-700"
                        >
                          Delete
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            <div className="px-4 py-3 border-t border-border flex justify-between">
              <div className="flex gap-2">
                <button
                  onClick={handleNew}
                  className="text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:opacity-90"
                >
                  + New Theme
                </button>
                <button
                  onClick={handleImportFromFile}
                  className="text-sm px-3 py-1.5 rounded border border-border text-foreground hover:bg-secondary"
                >
                  Import from file…
                </button>
              </div>
              <button onClick={onClose} className="text-sm text-muted-foreground hover:text-foreground">
                Close
              </button>
            </div>
          </>
        )}

        {/* Editor view */}
        {view === 'editor' && (
          <>
            {editingMeta && BUILT_IN_NAMES.includes(editingMeta.name) && (
              <div className="px-4 py-2 text-sm text-muted-foreground bg-muted border-b border-border">
                Built-in themes are read-only. Create a new theme to customise colours and typography.
              </div>
            )}
            {importConflict && (
              <div className="px-4 py-2 text-sm text-yellow-700 bg-yellow-50 border-b border-yellow-200 dark:bg-yellow-900/20 dark:text-yellow-300">
                A theme named "{importConflict.name}" already exists. Saving will replace it.
              </div>
            )}
            <div ref={containerRef} className="flex-1 overflow-hidden border-b border-border" />
            {(!editingMeta || !BUILT_IN_NAMES.includes(editingMeta.name)) && (
              <div className="px-4 py-3 flex justify-end gap-2">
                <button
                  onClick={() => { setView('list'); setImportConflict(null); setError(''); }}
                  className="text-sm text-muted-foreground hover:text-foreground"
                >
                  Cancel
                </button>
                <button
                  onClick={handleSaveOrReplace}
                  disabled={saving}
                  className="text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
                >
                  {saving ? 'Saving…' : (importConflict ? 'Replace' : 'Save')}
                </button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
