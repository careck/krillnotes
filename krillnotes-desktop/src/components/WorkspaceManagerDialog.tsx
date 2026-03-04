import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { WorkspaceEntry } from '../types';
import EnterPasswordDialog from './EnterPasswordDialog';

interface WorkspaceManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onNewWorkspace: () => void;
}

type SortKey = 'name' | 'modified';

type ActiveView = 'list' | 'delete-confirm' | 'duplicate-form';

function formatDate(timestampSeconds: number): string {
  const d = new Date(timestampSeconds * 1000);
  return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function WorkspaceManagerDialog({ isOpen, onClose, onNewWorkspace }: WorkspaceManagerDialogProps) {
  const { t } = useTranslation();

  const [entries, setEntries] = useState<WorkspaceEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const [selected, setSelected] = useState<WorkspaceEntry | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>('name');

  const [activeView, setActiveView] = useState<ActiveView>('list');

  // Open / password flow
  const [pendingOpen, setPendingOpen] = useState<WorkspaceEntry | null>(null);
  const [passwordError, setPasswordError] = useState('');
  const [opening, setOpening] = useState(false);

  // Delete confirmation
  const [deleting, setDeleting] = useState(false);

  // Duplicate form
  const [dupNewName, setDupNewName] = useState('');
  const [dupSourcePassword, setDupSourcePassword] = useState('');
  const [dupNewPassword, setDupNewPassword] = useState('');
  const [dupNewPasswordConfirm, setDupNewPasswordConfirm] = useState('');
  const [dupError, setDupError] = useState('');
  const [duplicating, setDuplicating] = useState(false);

  const loadEntries = useCallback(() => {
    setLoading(true);
    setError('');
    invoke<WorkspaceEntry[]>('list_workspace_files')
      .then(list => setEntries(list))
      .catch(err => setError(t('workspace.failedList', { error: String(err) })))
      .finally(() => setLoading(false));
  }, [t]);

  useEffect(() => {
    if (isOpen) {
      setSelected(null);
      setActiveView('list');
      setError('');
      setPendingOpen(null);
      setPasswordError('');
      setOpening(false);
      setDeleting(false);
      setDuplicating(false);
      loadEntries();
    }
  }, [isOpen, loadEntries]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (activeView !== 'list') {
          setActiveView('list');
        } else if (!pendingOpen) {
          onClose();
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, activeView, pendingOpen]);

  if (!isOpen) return null;

  // --- Sorting ---
  const sortedEntries = [...entries].sort((a, b) => {
    if (sortKey === 'name') return a.name.localeCompare(b.name);
    // 'modified' — descending (newest first)
    return b.lastModified - a.lastModified;
  });

  // --- Open flow ---
  const handleOpen = async () => {
    if (!selected || selected.isOpen) return;
    setOpening(true);
    setError('');

    try {
      const cached = await invoke<string | null>('get_cached_password', { path: selected.path });
      if (cached !== null) {
        try {
          await invoke('open_workspace', { path: selected.path, password: cached });
          onClose();
          return;
        } catch {
          setOpening(false);
          // Cached password no longer valid — fall through to password dialog.
        }
      }
    } catch {
      // Cache lookup failed — fall through to password dialog.
    }

    setOpening(false);
    setPendingOpen(selected);
    setPasswordError('');
  };

  const handlePasswordConfirm = async (password: string) => {
    if (!pendingOpen) return;
    setOpening(true);
    setPasswordError('');
    try {
      await invoke('open_workspace', { path: pendingOpen.path, password });
      setPendingOpen(null);
      onClose();
    } catch (err) {
      setPasswordError(String(err));
      setOpening(false);
    }
  };

  const handlePasswordCancel = () => {
    setPendingOpen(null);
    setPasswordError('');
  };

  // --- Delete flow ---
  const handleDeleteBegin = () => {
    if (!selected || selected.isOpen) return;
    setActiveView('delete-confirm');
  };

  const handleDeleteConfirm = async () => {
    if (!selected) return;
    setDeleting(true);
    setError('');
    try {
      await invoke('delete_workspace', { path: selected.path });
      setSelected(null);
      setActiveView('list');
      loadEntries();
    } catch (err) {
      setError(String(err));
      setActiveView('list');
    } finally {
      setDeleting(false);
    }
  };

  // --- Duplicate flow ---
  const handleDuplicateBegin = () => {
    if (!selected) return;
    setDupNewName(`Copy of ${selected.name}`);
    setDupSourcePassword('');
    setDupNewPassword('');
    setDupNewPasswordConfirm('');
    setDupError('');
    setActiveView('duplicate-form');
  };

  const handleDuplicateConfirm = async () => {
    if (!selected) return;

    if (!dupNewName.trim()) {
      setDupError('Please enter a name for the duplicate workspace.');
      return;
    }
    if (dupNewPassword.length > 0 && dupNewPassword !== dupNewPasswordConfirm) {
      setDupError('New passwords do not match.');
      return;
    }

    setDuplicating(true);
    setDupError('');
    try {
      await invoke('duplicate_workspace', {
        sourcePath: selected.path,
        sourcePassword: dupSourcePassword,
        newName: dupNewName.trim(),
        newPassword: dupNewPassword,
      });
      setActiveView('list');
      loadEntries();
    } catch (err) {
      setDupError(String(err));
    } finally {
      setDuplicating(false);
    }
  };

  // --- Password dialog overlay ---
  if (pendingOpen) {
    return (
      <EnterPasswordDialog
        isOpen={true}
        workspaceName={pendingOpen.name}
        error={passwordError}
        onConfirm={handlePasswordConfirm}
        onCancel={handlePasswordCancel}
      />
    );
  }

  // --- Main dialog ---
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary rounded-lg w-[560px] max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="px-6 pt-6 pb-3 flex items-center justify-between">
          <h2 className="text-xl font-bold">Manage Workspaces</h2>

          {/* Sort toggle — only in list view */}
          {activeView === 'list' && (
            <div className="flex items-center gap-1 text-sm">
              <span className="text-muted-foreground mr-1">Sort:</span>
              <button
                onClick={() => setSortKey('name')}
                className={`px-2 py-1 rounded ${sortKey === 'name' ? 'bg-secondary font-medium' : 'hover:bg-secondary/50'}`}
              >
                Name
              </button>
              <button
                onClick={() => setSortKey('modified')}
                className={`px-2 py-1 rounded ${sortKey === 'modified' ? 'bg-secondary font-medium' : 'hover:bg-secondary/50'}`}
              >
                Modified
              </button>
            </div>
          )}
        </div>

        {/* Workspace list */}
        <div className="flex-1 overflow-y-auto px-6">
          {loading ? (
            <p className="text-muted-foreground text-center py-8">{t('workspace.loading')}</p>
          ) : entries.length === 0 ? (
            <p className="text-muted-foreground text-center py-8 whitespace-pre-line">
              {t('workspace.noWorkspaces')}
            </p>
          ) : (
            <div className="space-y-1">
              {sortedEntries.map(entry => (
                <button
                  key={entry.path}
                  onClick={() => setSelected(entry)}
                  className={`w-full text-left px-3 py-2 rounded-md flex items-center justify-between transition-colors ${
                    selected?.path === entry.path
                      ? 'bg-primary/15 border border-primary/30'
                      : 'hover:bg-secondary/50 border border-transparent'
                  }`}
                >
                  <span className="font-medium truncate">{entry.name}</span>
                  <div className="flex items-center gap-3 text-xs text-muted-foreground ml-2 shrink-0">
                    {entry.isOpen && (
                      <span className="text-primary font-medium">{t('workspace.alreadyOpen')}</span>
                    )}
                    <span>{formatDate(entry.lastModified)}</span>
                    <span>{formatSize(entry.sizeBytes)}</span>
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Info panel — shown when a workspace is selected */}
        {selected && activeView === 'list' && (
          <div className="mx-6 mt-2 p-3 bg-secondary/30 border border-secondary rounded text-sm">
            <div className="grid grid-cols-5 gap-2 text-center">
              <div>
                <div className="text-muted-foreground text-xs mb-0.5">Created</div>
                <div className="font-medium">
                  {selected.createdAt !== null ? formatDate(selected.createdAt) : '—'}
                </div>
              </div>
              <div>
                <div className="text-muted-foreground text-xs mb-0.5">Modified</div>
                <div className="font-medium">{formatDate(selected.lastModified)}</div>
              </div>
              <div>
                <div className="text-muted-foreground text-xs mb-0.5">Notes</div>
                <div className="font-medium">
                  {selected.noteCount !== null ? selected.noteCount : '—'}
                </div>
              </div>
              <div>
                <div className="text-muted-foreground text-xs mb-0.5">Attachments</div>
                <div className="font-medium">
                  {selected.attachmentCount !== null ? selected.attachmentCount : '—'}
                </div>
              </div>
              <div>
                <div className="text-muted-foreground text-xs mb-0.5">Size</div>
                <div className="font-medium">{formatSize(selected.sizeBytes)}</div>
              </div>
            </div>
          </div>
        )}

        {/* Delete confirmation banner */}
        {activeView === 'delete-confirm' && selected && (
          <div className="mx-6 mt-2 p-4 bg-red-500/10 border border-red-500/30 rounded text-sm">
            <p className="text-red-500 font-medium mb-1">This cannot be undone.</p>
            <p className="text-muted-foreground mb-3">
              Permanently delete <strong>"{selected.name}"</strong> and all its data?
            </p>
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setActiveView('list')}
                disabled={deleting}
                className="px-3 py-1.5 border border-secondary rounded hover:bg-secondary text-sm"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirm}
                disabled={deleting}
                className="px-3 py-1.5 bg-red-600 text-white rounded hover:bg-red-700 disabled:opacity-50 text-sm"
              >
                {deleting ? 'Deleting…' : 'Delete forever'}
              </button>
            </div>
          </div>
        )}

        {/* Duplicate form */}
        {activeView === 'duplicate-form' && selected && (
          <div className="mx-6 mt-2 p-4 border border-secondary rounded text-sm space-y-3">
            <p className="font-medium">Duplicate "{selected.name}"</p>

            <div>
              <label className="block text-xs text-muted-foreground mb-1">New name</label>
              <input
                type="text"
                value={dupNewName}
                onChange={e => setDupNewName(e.target.value)}
                disabled={duplicating}
                className="w-full bg-secondary border border-secondary rounded px-3 py-1.5 text-sm"
                autoFocus
              />
            </div>

            <div>
              <label className="block text-xs text-muted-foreground mb-1">
                Source password
              </label>
              <input
                type="password"
                value={dupSourcePassword}
                onChange={e => setDupSourcePassword(e.target.value)}
                disabled={duplicating}
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-secondary border border-secondary rounded px-3 py-1.5 text-sm"
                placeholder="Password for source workspace"
              />
            </div>

            <div>
              <label className="block text-xs text-muted-foreground mb-1">
                New password <span className="text-muted-foreground/70">(optional)</span>
              </label>
              <input
                type="password"
                value={dupNewPassword}
                onChange={e => setDupNewPassword(e.target.value)}
                disabled={duplicating}
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-secondary border border-secondary rounded px-3 py-1.5 text-sm"
                placeholder="Leave blank to use same password"
              />
            </div>

            <div>
              <label className="block text-xs text-muted-foreground mb-1">
                Confirm new password
              </label>
              <input
                type="password"
                value={dupNewPasswordConfirm}
                onChange={e => setDupNewPasswordConfirm(e.target.value)}
                disabled={duplicating}
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-secondary border border-secondary rounded px-3 py-1.5 text-sm"
                placeholder="Confirm new password"
              />
            </div>

            {dupError && (
              <div className="p-2 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-xs">
                {dupError}
              </div>
            )}

            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setActiveView('list')}
                disabled={duplicating}
                className="px-3 py-1.5 border border-secondary rounded hover:bg-secondary text-sm"
              >
                Cancel
              </button>
              <button
                onClick={handleDuplicateConfirm}
                disabled={duplicating || !dupNewName.trim()}
                className="px-3 py-1.5 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 text-sm"
              >
                {duplicating ? 'Duplicating…' : 'Duplicate'}
              </button>
            </div>
          </div>
        )}

        {/* Global error */}
        {error && (
          <div className="mx-6 mt-2">
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {error}
            </div>
          </div>
        )}

        {/* Toolbar (only in list view) */}
        {activeView === 'list' && (
          <div className="px-6 pt-3 pb-2 flex gap-2">
            <button
              onClick={handleOpen}
              disabled={!selected || selected.isOpen || opening}
              className="px-3 py-1.5 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 text-sm"
            >
              {opening ? 'Opening…' : 'Open'}
            </button>
            <button
              onClick={handleDuplicateBegin}
              disabled={!selected}
              className="px-3 py-1.5 border border-secondary rounded hover:bg-secondary disabled:opacity-50 text-sm"
            >
              Duplicate
            </button>
            <button
              onClick={handleDeleteBegin}
              disabled={!selected || selected.isOpen}
              className="px-3 py-1.5 border border-red-500/40 text-red-500 rounded hover:bg-red-500/10 disabled:opacity-50 text-sm"
            >
              Delete
            </button>
          </div>
        )}

        {/* Footer */}
        <div className="flex justify-between px-6 py-4 pt-2 border-t border-secondary mt-2">
          <button
            onClick={() => {
              onClose();
              onNewWorkspace();
            }}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
          >
            New
          </button>
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
          >
            {t('common.close')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default WorkspaceManagerDialog;
