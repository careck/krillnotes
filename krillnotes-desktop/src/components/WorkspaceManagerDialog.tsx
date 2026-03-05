import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { WorkspaceEntry, IdentityRef } from '../types';
import UnlockIdentityDialog from './UnlockIdentityDialog';
import { slugify } from '../utils/slugify';

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

  // Identity state
  const [identityFilter, setIdentityFilter] = useState<string>('all');
  const [identities, setIdentities] = useState<IdentityRef[]>([]);
  const [unlockedIds, setUnlockedIds] = useState<string[]>([]);
  const [unlockTarget, setUnlockTarget] = useState<{ uuid: string; name: string; workspacePath: string } | null>(null);

  // Open flow
  const [opening, setOpening] = useState(false);

  // Delete confirmation
  const [deleting, setDeleting] = useState(false);

  // Duplicate form
  const [dupNewName, setDupNewName] = useState('');
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

  const loadIdentities = useCallback(() => {
    Promise.all([
      invoke<IdentityRef[]>('list_identities'),
      invoke<string[]>('get_unlocked_identities'),
    ]).then(([ids, unlocked]) => {
      setIdentities(ids);
      setUnlockedIds(unlocked);
    }).catch(err => console.error('Failed to load identities:', err));
  }, []);

  useEffect(() => {
    if (isOpen) {
      setSelected(null);
      setActiveView('list');
      setError('');
      setOpening(false);
      setDeleting(false);
      setDuplicating(false);
      setUnlockTarget(null);
      loadEntries();
      loadIdentities();
    }
  }, [isOpen, loadEntries, loadIdentities]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (activeView !== 'list') {
          setActiveView('list');
        } else if (!unlockTarget) {
          onClose();
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, activeView, unlockTarget]);

  if (!isOpen) return null;

  // --- Sorting + filtering ---
  const sortedEntries = [...entries].sort((a, b) => {
    if (sortKey === 'name') return a.name.localeCompare(b.name);
    // 'modified' — descending (newest first)
    return b.lastModified - a.lastModified;
  });

  const filteredEntries = identityFilter === 'all'
    ? sortedEntries
    : sortedEntries.filter(e => e.identityUuid === identityFilter);

  // --- Open flow ---
  const handleOpen = async (target: WorkspaceEntry = selected!) => {
    if (!target || target.isOpen) return;
    setOpening(true);
    setError('');

    try {
      await invoke('open_workspace', { path: target.path });
      onClose();
    } catch (err) {
      const errStr = String(err);
      if (errStr.startsWith('IDENTITY_LOCKED:')) {
        const identityUuid = errStr.split(':')[1];
        const identity = identities.find(i => i.uuid === identityUuid);
        setUnlockTarget({
          uuid: identityUuid,
          name: identity?.displayName ?? 'Unknown',
          workspacePath: target.path,
        });
      } else if (errStr === 'IDENTITY_REQUIRED') {
        setError('This workspace is not bound to any identity.');
      } else if (!errStr.includes('focused_existing')) {
        setError(errStr);
      } else {
        onClose();
      }
      setOpening(false);
    }
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
    setDupNewName(t('workspaceManager.duplicateCopyPrefix', { name: selected.name }));
    setDupError('');
    setActiveView('duplicate-form');
  };

  const handleDuplicateConfirm = async () => {
    if (!selected) return;

    if (!dupNewName.trim()) {
      setDupError(t('workspaceManager.duplicateNameRequired'));
      return;
    }

    if (!selected.identityUuid) {
      setDupError(t('identity.workspaceNotBound'));
      return;
    }

    setDuplicating(true);
    setDupError('');
    try {
      await invoke('duplicate_workspace', {
        sourcePath: selected.path,
        identityUuid: selected.identityUuid,
        newName: slugify(dupNewName.trim()),
      });
      setActiveView('list');
      loadEntries();
    } catch (err) {
      setDupError(String(err));
    } finally {
      setDuplicating(false);
    }
  };

  // --- Unlock dialog callback ---
  const handleUnlocked = async () => {
    const savedPath = unlockTarget!.workspacePath;
    setUnlockTarget(null);
    const updatedUnlocked = await invoke<string[]>('get_unlocked_identities');
    setUnlockedIds(updatedUnlocked);
    // Retry the open
    const target = entries.find(e => e.path === savedPath);
    if (target) await handleOpen(target);
  };

  // --- Main dialog ---
  return (
    <>
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-background border border-secondary rounded-lg w-[560px] max-h-[80vh] flex flex-col">
          {/* Header */}
          <div className="px-6 pt-6 pb-3 flex items-center justify-between">
            <h2 className="text-xl font-bold">{t('workspaceManager.title')}</h2>

            {/* Sort toggle + identity filter — only in list view */}
            {activeView === 'list' && (
              <div className="flex items-center gap-2 text-sm flex-wrap justify-end">
                {identities.length > 0 && (
                  <select
                    value={identityFilter}
                    onChange={e => setIdentityFilter(e.target.value)}
                    className="bg-secondary border border-secondary rounded px-2 py-1 text-sm"
                  >
                    <option value="all">{t('identity.allIdentities')}</option>
                    {identities.map(i => (
                      <option key={i.uuid} value={i.uuid}>{i.displayName}</option>
                    ))}
                  </select>
                )}
                <div className="flex items-center gap-1">
                  <span className="text-muted-foreground mr-1">{t('workspaceManager.sortLabel')}</span>
                  <button
                    onClick={() => setSortKey('name')}
                    className={`px-2 py-1 rounded ${sortKey === 'name' ? 'bg-secondary font-medium' : 'hover:bg-secondary/50'}`}
                  >
                    {t('workspaceManager.sortName')}
                  </button>
                  <button
                    onClick={() => setSortKey('modified')}
                    className={`px-2 py-1 rounded ${sortKey === 'modified' ? 'bg-secondary font-medium' : 'hover:bg-secondary/50'}`}
                  >
                    {t('workspaceManager.sortModified')}
                  </button>
                </div>
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
                {filteredEntries.map(entry => (
                  <button
                    key={entry.path}
                    onClick={() => setSelected(entry)}
                    onDoubleClick={() => { setSelected(entry); handleOpen(entry); }}
                    className={`w-full text-left px-3 py-2 rounded-md flex items-center justify-between transition-colors ${
                      selected?.path === entry.path
                        ? 'bg-primary/15 border border-primary/30'
                        : 'hover:bg-secondary/50 border border-transparent'
                    }`}
                  >
                    <div className="flex flex-col min-w-0">
                      <span className="font-medium truncate">{entry.name}</span>
                      {entry.identityName && (
                        <span className="text-xs text-muted-foreground">
                          {unlockedIds.includes(entry.identityUuid!) ? '\uD83D\uDD13' : '\uD83D\uDD12'} {entry.identityName}
                        </span>
                      )}
                    </div>
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
                  <div className="text-muted-foreground text-xs mb-0.5">{t('workspaceManager.infoCreated')}</div>
                  <div className="font-medium">
                    {selected.createdAt !== null ? formatDate(selected.createdAt) : '—'}
                  </div>
                </div>
                <div>
                  <div className="text-muted-foreground text-xs mb-0.5">{t('workspaceManager.infoModified')}</div>
                  <div className="font-medium">{formatDate(selected.lastModified)}</div>
                </div>
                <div>
                  <div className="text-muted-foreground text-xs mb-0.5">{t('workspaceManager.infoNotes')}</div>
                  <div className="font-medium">
                    {selected.noteCount !== null ? selected.noteCount : '—'}
                  </div>
                </div>
                <div>
                  <div className="text-muted-foreground text-xs mb-0.5">{t('workspaceManager.infoAttachments')}</div>
                  <div className="font-medium">
                    {selected.attachmentCount !== null ? selected.attachmentCount : '—'}
                  </div>
                </div>
                <div>
                  <div className="text-muted-foreground text-xs mb-0.5">{t('workspaceManager.infoSize')}</div>
                  <div className="font-medium">{formatSize(selected.sizeBytes)}</div>
                </div>
              </div>
            </div>
          )}

          {/* Delete confirmation banner */}
          {activeView === 'delete-confirm' && selected && (
            <div className="mx-6 mt-2 p-4 bg-red-500/10 border border-red-500/30 rounded text-sm">
              <p className="text-red-500 font-medium mb-1">{t('workspaceManager.deleteConfirmTitle')}</p>
              <p className="text-muted-foreground mb-3">
                {t('workspaceManager.deleteConfirmBody', { name: selected.name })}
              </p>
              <div className="flex gap-2 justify-end">
                <button
                  onClick={() => setActiveView('list')}
                  disabled={deleting}
                  className="px-3 py-1.5 border border-secondary rounded hover:bg-secondary text-sm"
                >
                  {t('common.cancel')}
                </button>
                <button
                  onClick={handleDeleteConfirm}
                  disabled={deleting}
                  className="px-3 py-1.5 bg-red-600 text-white rounded hover:bg-red-700 disabled:opacity-50 text-sm"
                >
                  {deleting ? t('workspaceManager.deleting') : t('workspaceManager.deleteForever')}
                </button>
              </div>
            </div>
          )}

          {/* Duplicate form */}
          {activeView === 'duplicate-form' && selected && (
            <div className="mx-6 mt-2 p-4 border border-secondary rounded text-sm space-y-3">
              <p className="font-medium">{t('workspaceManager.duplicateFormTitle', { name: selected.name })}</p>

              <div>
                <label className="block text-xs text-muted-foreground mb-1">{t('workspaceManager.duplicateNewName')}</label>
                <input
                  type="text"
                  value={dupNewName}
                  onChange={e => setDupNewName(e.target.value)}
                  disabled={duplicating}
                  className="w-full bg-secondary border border-secondary rounded px-3 py-1.5 text-sm"
                  autoFocus
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
                  {t('common.cancel')}
                </button>
                <button
                  onClick={handleDuplicateConfirm}
                  disabled={duplicating || !dupNewName.trim()}
                  className="px-3 py-1.5 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 text-sm"
                >
                  {duplicating ? t('workspaceManager.duplicating') : t('workspaceManager.duplicate')}
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
                onClick={() => handleOpen()}
                disabled={!selected || selected.isOpen || opening}
                className="px-3 py-1.5 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 text-sm"
              >
                {opening ? t('workspaceManager.openingWorkspace') : t('workspaceManager.open')}
              </button>
              <button
                onClick={handleDuplicateBegin}
                disabled={!selected}
                className="px-3 py-1.5 border border-secondary rounded hover:bg-secondary disabled:opacity-50 text-sm"
              >
                {t('workspaceManager.duplicate')}
              </button>
              <button
                onClick={handleDeleteBegin}
                disabled={!selected || selected.isOpen}
                className="px-3 py-1.5 border border-red-500/40 text-red-500 rounded hover:bg-red-500/10 disabled:opacity-50 text-sm"
              >
                {t('workspaceManager.delete')}
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
              {t('workspaceManager.newWorkspace')}
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

      {/* Unlock identity dialog — rendered outside main dialog to avoid z-index conflicts */}
      {unlockTarget && (
        <UnlockIdentityDialog
          isOpen={true}
          identityUuid={unlockTarget.uuid}
          identityName={unlockTarget.name}
          onUnlocked={handleUnlocked}
          onCancel={() => { setUnlockTarget(null); setOpening(false); }}
        />
      )}
    </>
  );
}

export default WorkspaceManagerDialog;
