// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software


import { useState, useEffect } from 'react';
import { save, confirm } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceView from './components/WorkspaceView';
import EmptyState from './components/EmptyState';
import StatusMessage from './components/StatusMessage';
import NewWorkspaceDialog from './components/NewWorkspaceDialog';
import WorkspaceManagerDialog from './components/WorkspaceManagerDialog';
import SettingsDialog from './components/SettingsDialog';
import type { AppSettings, WorkspaceInfo as WorkspaceInfoType } from './types';
import IdentityManagerDialog from './components/IdentityManagerDialog';
import SwarmOpenDialog from './components/SwarmOpenDialog';
import WorkspacePeersDialog from './components/WorkspacePeersDialog';
import { AcceptInviteWorkflow } from './components/AcceptInviteWorkflow';
import { CreateDeltaDialog } from './components/CreateDeltaDialog';
import './styles/globals.css';
import { ThemeProvider } from './contexts/ThemeContext';
import { useTranslation } from 'react-i18next';
import { slugify } from './utils/slugify';
import { useMenuEvents } from './hooks/useMenuEvents';
import { useWorkspaceLifecycle } from './hooks/useWorkspaceLifecycle';
import { useDialogState } from './hooks/useDialogState';
import { useGlobalSnapshotPolling } from './hooks/useIdentityPolling';
import { useSyncOnClose } from './hooks/useSyncOnClose';
import SyncOnCloseDialog from './components/SyncOnCloseDialog';

function App() {
  const { t } = useTranslation();

  const {
    status,
    isError,
    showNewWorkspace, setShowNewWorkspace,
    showOpenWorkspace, setShowOpenWorkspace,
    showSettings, setShowSettings,
    importState, setImportState,
    importName, setImportName,
    importError, setImportError,
    importing, setImporting,
    importIdentities,
    importSelectedIdentity, setImportSelectedIdentity,
    showImportPasswordDialog, setShowImportPasswordDialog,
    importPassword, setImportPassword,
    importPasswordError, setImportPasswordError,
    pendingImportZipPath, setPendingImportZipPath,
    pendingImportPassword, setPendingImportPassword,
    showExportPasswordDialog, setShowExportPasswordDialog,
    exportPassword, setExportPassword,
    exportPasswordConfirm, setExportPasswordConfirm,
    showIdentityManager, setShowIdentityManager,
    showSwarmOpen, setShowSwarmOpen,
    swarmFilePath, setSwarmFilePath,
    pendingInvitePath, setPendingInvitePath,
    pendingInviteData, setPendingInviteData,
    showWorkspacePeers, setShowWorkspacePeers,
    showCreateDeltaDialog, setShowCreateDeltaDialog,
    statusSetter,
  } = useDialogState();

  // Global snapshot polling for all unlocked identities (no workspace needed).
  useGlobalSnapshotPolling();

  const [sharingIndicatorMode, setSharingIndicatorMode] = useState<'off' | 'auto' | 'on'>('auto');
  const refreshSharingIndicatorMode = () => {
    invoke<AppSettings>('get_settings')
      .then(s => setSharingIndicatorMode((s.sharingIndicatorMode ?? 'auto') as 'off' | 'auto' | 'on'))
      .catch(() => {});
  };
  useEffect(refreshSharingIndicatorMode, []);

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

    if (!importSelectedIdentity) {
      setImportError(t('identity.noUnlockedIdentities'));
      return;
    }

    setImporting(true);
    setImportError('');

    try {
      const prev = importState;
      await invoke<WorkspaceInfoType>('execute_import', {
        zipPath: importState.zipPath,
        name: slug,
        password: pendingImportPassword ?? null,
        identityUuid: importSelectedIdentity,
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

  const { workspace, unlockedIdentityUuid, refreshUnlockedIdentity, openSwarmFile } =
    useWorkspaceLifecycle({
      setShowIdentityManager,
      setShowSwarmOpen,
      showSwarmOpen,
      proceedWithImport,
      setPendingInvitePath,
      setPendingInviteData,
      setSwarmFilePath,
    });


  useMenuEvents(workspace, {
    setShowNewWorkspace, setShowOpenWorkspace, setShowSettings,
    setShowExportPasswordDialog, setShowIdentityManager,
    setShowWorkspacePeers, setShowCreateDeltaDialog,
    statusSetter, proceedWithImport, openSwarmFile,
  });

  const {
    syncOnCloseState,
    handleSyncAndClose,
    handleCloseWithoutSync,
    handleCancel,
  } = useSyncOnClose();

  return (
    <ThemeProvider>
    <div className="min-h-screen bg-background text-foreground">
      {workspace ? <WorkspaceView workspaceInfo={workspace} onOpenWorkspacePeers={() => setShowWorkspacePeers(true)} sharingIndicatorMode={sharingIndicatorMode} /> : <div className="p-8"><EmptyState /></div>}
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
        onSaved={refreshSharingIndicatorMode}
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

            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">{t('identity.selectIdentity')}</label>
              {importIdentities.length > 0 ? (
                <select
                  value={importSelectedIdentity}
                  onChange={(e) => setImportSelectedIdentity(e.target.value)}
                  className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                  disabled={importing}
                >
                  {importIdentities.map(i => (
                    <option key={i.uuid} value={i.uuid}>{i.displayName}</option>
                  ))}
                </select>
              ) : (
                <p className="text-sm text-muted-foreground p-2 bg-secondary/30 rounded border border-secondary">
                  {t('identity.noUnlockedIdentities')}
                </p>
              )}
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
                disabled={importing || !importName.trim() || !importSelectedIdentity}
              >
                {importing ? t('common.importing') : t('common.import')}
              </button>
            </div>
          </div>
        </div>
      )}
      <IdentityManagerDialog
        isOpen={showIdentityManager}
        onClose={() => { setShowIdentityManager(false); refreshUnlockedIdentity(); }}
      />
      <SwarmOpenDialog
        isOpen={showSwarmOpen}
        onClose={() => { setShowSwarmOpen(false); setSwarmFilePath(null); }}
        swarmFilePath={swarmFilePath}
        unlockedIdentityUuid={unlockedIdentityUuid}
        deviceId={unlockedIdentityUuid ?? ''}
      />
      {pendingInviteData && pendingInvitePath && (
        <AcceptInviteWorkflow
          identityUuid={unlockedIdentityUuid ?? ''}
          identityName={''}
          preloadedInviteData={pendingInviteData}
          preloadedPath={pendingInvitePath}
          onResponded={() => { setPendingInvitePath(null); setPendingInviteData(null); }}
          onClose={() => { setPendingInvitePath(null); setPendingInviteData(null); }}
        />
      )}
      {showWorkspacePeers && (
        <WorkspacePeersDialog
          identityUuid={workspace?.identityUuid ?? unlockedIdentityUuid ?? ''}
          workspaceInfo={workspace}
          unlockedIdentityUuid={unlockedIdentityUuid}
          onClose={() => setShowWorkspacePeers(false)}
        />
      )}
      {showCreateDeltaDialog && (
        <CreateDeltaDialog onClose={() => setShowCreateDeltaDialog(false)} />
      )}
      {syncOnCloseState.phase !== 'idle' && (
        <SyncOnCloseDialog
          mode={syncOnCloseState.phase === 'asking' ? 'ask' : 'syncing'}
          syncError={syncOnCloseState.phase === 'syncing' ? syncOnCloseState.error : null}
          onSyncAndClose={handleSyncAndClose}
          onCloseWithoutSync={handleCloseWithoutSync}
          onCancel={handleCancel}
        />
      )}
    </div>
    </ThemeProvider>
  );
}

export default App;
