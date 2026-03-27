// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { IdentityRef, InviteFileData } from '../types';

export interface ImportState {
  zipPath: string;
  noteCount: number;
  scriptCount: number;
}

export function useDialogState() {
  const [status, setStatus] = useState('');
  const [isError, setIsError] = useState(false);
  const [showNewWorkspace, setShowNewWorkspace] = useState(false);
  const [showOpenWorkspace, setShowOpenWorkspace] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [importState, setImportState] = useState<ImportState | null>(null);
  const [importName, setImportName] = useState('');
  const [importError, setImportError] = useState('');
  const [importing, setImporting] = useState(false);
  const [importIdentities, setImportIdentities] = useState<IdentityRef[]>([]);
  const [importSelectedIdentity, setImportSelectedIdentity] = useState<string>('');
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
  const [pendingInvitePath, setPendingInvitePath] = useState<string | null>(null);
  const [pendingInviteData, setPendingInviteData] = useState<InviteFileData | null>(null);
  const [showWorkspacePeers, setShowWorkspacePeers] = useState(false);
  const [showCreateDeltaDialog, setShowCreateDeltaDialog] = useState(false);

  const statusSetter = useCallback((msg: string, error = false) => {
    setStatus(msg);
    setIsError(error);
    setTimeout(() => setStatus(''), 5000);
  }, []);

  // Reset import dialog state when it opens and load unlocked identities
  useEffect(() => {
    if (importState) {
      setImportName('imported-workspace');
      setImportError('');
      setImporting(false);
      Promise.all([
        invoke<IdentityRef[]>('list_identities'),
        invoke<string[]>('get_unlocked_identities'),
      ]).then(([ids, unlocked]) => {
        const unlockedIds = ids.filter(i => unlocked.includes(i.uuid));
        setImportIdentities(unlockedIds);
        setImportSelectedIdentity(unlockedIds.length > 0 ? unlockedIds[0].uuid : '');
      }).catch(() => {});
    }
  }, [importState]);

  return {
    status, setStatus,
    isError, setIsError,
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
    showCreateFirstIdentity, setShowCreateFirstIdentity,
    showIdentityManager, setShowIdentityManager,
    showSwarmInvite, setShowSwarmInvite,
    showSwarmOpen, setShowSwarmOpen,
    swarmFilePath, setSwarmFilePath,
    pendingInvitePath, setPendingInvitePath,
    pendingInviteData, setPendingInviteData,
    showWorkspacePeers, setShowWorkspacePeers,
    showCreateDeltaDialog, setShowCreateDeltaDialog,
    statusSetter,
  };
}
