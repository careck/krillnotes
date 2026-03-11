// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useEffect, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open, save, confirm } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceView from './components/WorkspaceView';
import EmptyState from './components/EmptyState';
import StatusMessage from './components/StatusMessage';
import NewWorkspaceDialog from './components/NewWorkspaceDialog';
import WorkspaceManagerDialog from './components/WorkspaceManagerDialog';
import SettingsDialog from './components/SettingsDialog';
import type { WorkspaceInfo as WorkspaceInfoType, AppSettings, IdentityRef } from './types';
import CreateIdentityDialog from './components/CreateIdentityDialog';
import IdentityManagerDialog from './components/IdentityManagerDialog';
import SwarmInviteDialog from './components/SwarmInviteDialog';
import SwarmOpenDialog from './components/SwarmOpenDialog';
import WorkspacePeersDialog from './components/WorkspacePeersDialog';
import './styles/globals.css';
import { ThemeProvider } from './contexts/ThemeContext';
import i18n from './i18n';
import { useTranslation } from 'react-i18next';

function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

interface ImportState {
  zipPath: string;
  noteCount: number;
  scriptCount: number;
}

const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setShowExportPasswordDialog: (show: boolean) => void,
  setShowIdentityManager: (show: boolean) => void,
  doImport: (zipPath: string) => void,
  setShowSwarmInvite: (show: boolean) => void,
  setSwarmFilePath: (path: string | null) => void,
  setShowSwarmOpen: (show: boolean) => void,
  setShowWorkspacePeers: (show: boolean) => void,
) => ({
  'File > New Workspace clicked': () => {
    setShowNewWorkspace(true);
  },

  'File > Open Workspace clicked': () => {
    setShowIdentityManager(false);
    setShowOpenWorkspace(true);
  },

  'File > Export Workspace clicked': () => {
    setShowExportPasswordDialog(true);
  },

  'File > Import Workspace clicked': async () => {
    try {
      const zipPath = await open({
        filters: [{ name: 'Krillnotes Export', extensions: ['krillnotes'] }],
        multiple: false,
        title: 'Import Workspace',
      });
      if (!zipPath || Array.isArray(zipPath)) return;
      doImport(zipPath as string);
    } catch (error) {
      setStatus(`Import failed: ${error}`, true);
    }
  },

  'Edit > Settings clicked': () => {
    setShowSettings(true);
  },

  'File > Manage Identities clicked': () => {
    setShowOpenWorkspace(false);
    setShowIdentityManager(true);
  },

  'File > Invite Peer clicked': () => {
    setShowSwarmInvite(true);
  },

  'Edit > Workspace Peers clicked': () => {
    setShowWorkspacePeers(true);
  },

  'File > Open Swarm File clicked': async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({
        filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
        multiple: false,
        title: 'Open .swarm file',
      });
      if (!picked || Array.isArray(picked)) return;
      setSwarmFilePath(picked as string);
      setShowSwarmOpen(true);
    } catch {
      // user cancelled
    }
  },
});

function App() {
  const { t } = useTranslation();
  const [workspace, setWorkspace] = useState<WorkspaceInfoType | null>(null);
  const [status, setStatus] = useState('');
  const [isError, setIsError] = useState(false);
  const [showNewWorkspace, setShowNewWorkspace] = useState(false);
  const [showOpenWorkspace, setShowOpenWorkspace] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [importState, setImportState] = useState<ImportState | null>(null);
  const [importName, setImportName] = useState('');
  const [importError, setImportError] = useState('');
  const [importing, setImporting] = useState(false);
  const [showImportPasswordDialog, setShowImportPasswordDialog] = useState(false);
  const [importPassword, setImportPassword] = useState('');
  const [importPasswordError, setImportPasswordError] = useState('');
  const [pendingImportZipPath, setPendingImportZipPath] = useState<string | null>(null);
  const [pendingImportPassword, setPendingImportPassword] = useState<string | null>(null);
  const [showExportPasswordDialog, setShowExportPasswordDialog] = useState(false);
  const [exportPassword, setExportPassword] = useState('');
  const [exportPasswordConfirm, setExportPasswordConfirm] = useState('');
  const [showCreateFirstIdentity, setShowCreateFirstIdentity] = useState(false);
  const [showIdentityManager, setShowIdentityManager] = useState(false);
  const [showSwarmInvite, setShowSwarmInvite] = useState(false);
  const [showSwarmOpen, setShowSwarmOpen] = useState(false);
  const [swarmFilePath, setSwarmFilePath] = useState<string | null>(null);
  const [unlockedIdentityUuid, setUnlockedIdentityUuid] = useState<string | null>(null);
  const [showWorkspacePeers, setShowWorkspacePeers] = useState(false);

  const refreshUnlockedIdentity = () => {
    invoke<string[]>('get_unlocked_identities')
      .then(ids => setUnlockedIdentityUuid(ids.length > 0 ? ids[0] : null))
      .catch(() => {});
  };

  useEffect(() => {
    // If this is a workspace window (not "main"), fetch workspace info immediately
    {
      const window = getCurrentWebviewWindow();
      if (window.label !== 'main') {
        invoke<WorkspaceInfoType>('get_workspace_info')
          .then(info => {
            setWorkspace(info);
          })
          .catch(err => console.error('Failed to fetch workspace info:', err));
      }
    }

    // First-launch identity check: only on main window
    if (getCurrentWebviewWindow().label === 'main') {
      invoke<IdentityRef[]>('list_identities').then(identities => {
        if (identities.length === 0) {
          setShowCreateFirstIdentity(true);
        }
      }).catch(err => console.error('Failed to check identities:', err));
    }

    // Load first unlocked identity UUID for swarm operations
    refreshUnlockedIdentity();
  }, []);

  // Refresh unlocked identity whenever a swarm dialog opens, so a recently
  // unlocked identity is always picked up even if it happened after mount.
  useEffect(() => {
    if (showSwarmInvite || showSwarmOpen) refreshUnlockedIdentity();
  }, [showSwarmInvite, showSwarmOpen]);

  // Cold-start: pull any file path that arrived via OS file-open before JS
  // listeners were registered. Only the "main" (launcher) window handles imports.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label !== 'main') return;
    invoke<string | null>('consume_pending_file_open').then(path => {
      if (path) proceedWithImport(path, null);
    });
    invoke<string | null>('consume_pending_swarm_file').then(path => {
      if (path) {
        setSwarmFilePath(path);
        setShowSwarmOpen(true);
      }
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Warm-start (macOS): the backend emits "file-opened" when the app is already
  // running and the user opens a .krillnotes file from the OS.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label !== 'main') return;
    const unlisten = win.listen<string>('file-opened', () => {
      // Path is already stored in AppState; use the canonical pull command so
      // both paths (cold and warm start) share the same read-and-clear logic.
      invoke<string | null>('consume_pending_file_open').then(p => {
        if (p) proceedWithImport(p, null);
      });
    });
    return () => { unlisten.then(f => f()); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Warm-start (macOS): the backend emits "swarm-file-opened" when the app is already
  // running and the user opens a .swarm file from the OS.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label !== 'main') return;
    const unlisten = win.listen<string>('swarm-file-opened', (event) => {
      setSwarmFilePath(event.payload);
      setShowSwarmOpen(true);
    });
    return () => { unlisten.then(f => f()); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Apply saved language on startup
  useEffect(() => {
    invoke<AppSettings>('get_settings')
      .then(s => {
        if (s.language) {
          i18n.changeLanguage(s.language);
        }
      })
      .catch(err => console.error('Failed to load settings for language:', err));
  }, []);

  const statusSetter = (msg: string, error = false) => {
    setStatus(msg);
    setIsError(error);
    setTimeout(() => setStatus(''), 5000);
  };

  useEffect(() => {
    const handlers = createMenuHandlers(
      statusSetter,
      setShowNewWorkspace,
      setShowOpenWorkspace,
      setShowSettings,
      setShowExportPasswordDialog,
      setShowIdentityManager,
      (zipPath) => proceedWithImport(zipPath, null),
      setShowSwarmInvite,
      setSwarmFilePath,
      setShowSwarmOpen,
      setShowWorkspacePeers,
    );

    const unlisten = getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
      const handler = handlers[event.payload as keyof typeof handlers];
      if (handler) handler();
    });

    return () => { unlisten.then(f => f()); };
  }, [workspace]);

  // Reset import dialog state when it opens
  useEffect(() => {
    if (importState) {
      setImportName('imported-workspace');
      setImportError('');
      setImporting(false);
    }
  }, [importState]);

  const handleImportConfirm = async () => {
    if (!importState) return;

    const trimmed = importName.trim();
    if (!trimmed) {
      setImportError(t('workspace.nameRequired'));
      return;
    }

    const slug = slugify(trimmed);
    if (!slug) {
      setImportError(t('workspace.nameInvalid'));
      return;
    }

    setImporting(true);
    setImportError('');

    try {
      const settings = await invoke<AppSettings>('get_settings');
      const folderPath = `${settings.workspaceDirectory}/${slug}`;

      // Get the first unlocked identity to own this imported workspace.
      const unlockedIds = await invoke<string[]>('get_unlocked_identities');
      if (unlockedIds.length === 0) {
        setImportError(t('identity.noUnlockedIdentities'));
        setImporting(false);
        return;
      }
      const identityUuid = unlockedIds[0];

      const prev = importState;
      await invoke<WorkspaceInfoType>('execute_import', {
        zipPath: importState.zipPath,
        folderPath,
        password: pendingImportPassword ?? null,
        identityUuid,
      });
      setImportState(null);
      setPendingImportPassword(null);
      setImporting(false);
      if (prev) {
        statusSetter(t('workspace.importSuccess', { noteCount: prev.noteCount, scriptCount: prev.scriptCount }));
      }
    } catch (error) {
      setImportError(`${error}`);
      setImporting(false);
    }
  };

  const handleExportConfirm = async (password: string | null) => {
    setShowExportPasswordDialog(false);
    setExportPassword('');
    setExportPasswordConfirm('');

    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Export', extensions: ['krillnotes'] }],
        defaultPath: `${(workspace?.filename ?? 'workspace').replace(/\.db$/, '')}.krillnotes`,
        title: 'Export Workspace',
      });

      if (!path) return;

      await invoke('export_workspace_cmd', { path, password });
      statusSetter(t('workspace.exportSuccess'));
    } catch (error) {
      statusSetter(t('workspace.exportFailed', { error: String(error) }), true);
    }
  };

  const proceedWithImport = async (zipPath: string, password: string | null) => {
    try {
      const result = await invoke<{ appVersion: string; noteCount: number; scriptCount: number }>(
        'peek_import_cmd', { zipPath, password }
      );

      const currentVersion = await invoke<string>('get_app_version');
      if (result.appVersion > currentVersion) {
        const proceed = await confirm(
          t('dialogs.import.versionMismatch', { version: result.appVersion, currentVersion }),
          { title: t('dialogs.import.versionMismatchTitle'), kind: 'warning' }
        );
        if (!proceed) return;
      }

      setShowImportPasswordDialog(false);
      setImportPassword('');
      setPendingImportPassword(password);
      setImportState({
        zipPath,
        noteCount: result.noteCount,
        scriptCount: result.scriptCount,
      });
    } catch (error) {
      const errStr = `${error}`;
      if (errStr === 'ENCRYPTED_ARCHIVE') {
        setPendingImportZipPath(zipPath);
        setImportPassword('');
        setImportPasswordError('');
        setShowImportPasswordDialog(true);
      } else if (errStr === 'INVALID_PASSWORD') {
        setImportPasswordError(t('dialogs.password.incorrectTryAgain'));
      } else {
        statusSetter(t('workspace.importFailed', { error: errStr }), true);
      }
    }
  };

  return (
    <ThemeProvider>
    <div className="min-h-screen bg-background text-foreground">
      {workspace ? <WorkspaceView workspaceInfo={workspace} /> : <div className="p-8"><EmptyState /></div>}
      {status && <StatusMessage message={status} isError={isError} />}

      <NewWorkspaceDialog
        isOpen={showNewWorkspace}
        onClose={() => setShowNewWorkspace(false)}
      />
      <WorkspaceManagerDialog
        isOpen={showOpenWorkspace}
        onClose={() => setShowOpenWorkspace(false)}
        onNewWorkspace={() => {
          setShowOpenWorkspace(false);
          setShowNewWorkspace(true);
        }}
      />
      <SettingsDialog
        isOpen={showSettings}
        onClose={() => setShowSettings(false)}
      />

      {/* Export password dialog */}
      {showExportPasswordDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">{t('dialogs.password.protectPrompt')}</h2>
            <p className="text-sm text-muted-foreground mb-4">
              {t('dialogs.password.protectHint')}
            </p>
            <div className="mb-3">
              <label className="block text-sm font-medium mb-2">{t('dialogs.password.passwordLabel')}</label>
              <input
                type="password"
                value={exportPassword}
                onChange={(e) => setExportPassword(e.target.value)}
                placeholder={t('dialogs.password.optionalPlaceholder')}
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            </div>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">{t('dialogs.password.confirmLabel')}</label>
              <input
                type="password"
                value={exportPasswordConfirm}
                onChange={(e) => setExportPasswordConfirm(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    if (!exportPassword || exportPassword === exportPasswordConfirm) {
                      handleExportConfirm(exportPassword || null);
                    }
                  }
                }}
                placeholder={t('dialogs.password.confirmPlaceholder')}
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            </div>
            {exportPassword && exportPasswordConfirm && exportPassword !== exportPasswordConfirm && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {t('dialogs.password.mismatch')}
              </div>
            )}
            <div className="flex justify-between items-center">
              <button
                onClick={() => {
                  setShowExportPasswordDialog(false);
                  setExportPassword('');
                  setExportPasswordConfirm('');
                }}
                className="text-sm text-muted-foreground hover:text-foreground underline"
              >
                {t('common.cancel')}
              </button>
              <div className="flex gap-2">
                <button
                  onClick={() => handleExportConfirm(null)}
                  className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
                >
                  {t('dialogs.password.skipNoEncryption')}
                </button>
                <button
                  onClick={() => handleExportConfirm(exportPassword)}
                  disabled={!exportPassword || exportPassword !== exportPasswordConfirm}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {t('common.encrypt')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Import password dialog */}
      {showImportPasswordDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">{t('dialogs.password.archiveProtected')}</h2>
            <p className="text-sm text-muted-foreground mb-4">
              {t('dialogs.password.archiveHint')}
            </p>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">{t('dialogs.password.passwordLabel')}</label>
              <input
                type="password"
                value={importPassword}
                onChange={(e) => setImportPassword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && importPassword && pendingImportZipPath) {
                    setImportPasswordError('');
                    proceedWithImport(pendingImportZipPath, importPassword);
                  }
                }}
                placeholder={t('dialogs.password.passwordPlaceholder')}
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            </div>
            {importPasswordError && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {importPasswordError}
              </div>
            )}
            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setShowImportPasswordDialog(false);
                  setPendingImportZipPath(null);
                  setImportPassword('');
                  setImportPasswordError('');
                }}
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={() => {
                  if (!pendingImportZipPath) return;
                  setImportPasswordError('');
                  proceedWithImport(pendingImportZipPath, importPassword);
                }}
                disabled={!importPassword}
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {t('common.open')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Import name dialog — inline since it's a lightweight prompt */}
      {importState && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">{t('dialogs.import.title')}</h2>
            <p className="text-sm text-muted-foreground mb-4">
              {t('workspace.importingProgress', { noteCount: importState.noteCount, scriptCount: importState.scriptCount })}
            </p>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">
                {t('workspace.nameLabel')}
              </label>
              <input
                type="text"
                value={importName}
                onChange={(e) => setImportName(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && !importing) handleImportConfirm(); }}
                placeholder={t('dialogs.import.importedPlaceholder')}
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                autoFocus
                disabled={importing}
              />
            </div>

            {importError && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {importError}
              </div>
            )}

            <div className="flex justify-end gap-2">
              <button
                onClick={() => { setImportState(null); setPendingImportPassword(null); }}
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                disabled={importing}
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={handleImportConfirm}
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
                disabled={importing || !importName.trim()}
              >
                {importing ? t('common.importing') : t('common.import')}
              </button>
            </div>
          </div>
        </div>
      )}
      <CreateIdentityDialog
        isOpen={showCreateFirstIdentity}
        isFirstLaunch={true}
        onCreated={() => setShowCreateFirstIdentity(false)}
        onCancel={() => setShowCreateFirstIdentity(false)}
      />
      <IdentityManagerDialog
        isOpen={showIdentityManager}
        onClose={() => { setShowIdentityManager(false); refreshUnlockedIdentity(); }}
      />
      <SwarmInviteDialog
        isOpen={showSwarmInvite}
        onClose={() => setShowSwarmInvite(false)}
        workspaceInfo={workspace}
        unlockedIdentityUuid={unlockedIdentityUuid}
        deviceId={unlockedIdentityUuid ?? ''}
      />
      <SwarmOpenDialog
        isOpen={showSwarmOpen}
        onClose={() => { setShowSwarmOpen(false); setSwarmFilePath(null); }}
        swarmFilePath={swarmFilePath}
        unlockedIdentityUuid={unlockedIdentityUuid}
        deviceId={unlockedIdentityUuid ?? ''}
      />
      {showWorkspacePeers && (
        <WorkspacePeersDialog
          identityUuid={unlockedIdentityUuid ?? ''}
          workspaceInfo={workspace}
          unlockedIdentityUuid={unlockedIdentityUuid}
          onClose={() => setShowWorkspacePeers(false)}
        />
      )}
    </div>
    </ThemeProvider>
  );
}

export default App;
