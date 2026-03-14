// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { invoke } from '@tauri-apps/api/core';
import i18n from '../i18n';
import type { WorkspaceInfo as WorkspaceInfoType, AppSettings, IdentityRef, InviteFileData } from '../types';

interface WorkspaceLifecycleCallbacks {
  setShowCreateFirstIdentity: (show: boolean) => void;
  setShowSwarmOpen: (show: boolean) => void;
  showSwarmInvite: boolean;
  showSwarmOpen: boolean;
  proceedWithImport: (zipPath: string, password: string | null) => Promise<void>;
  setPendingInvitePath: (path: string | null) => void;
  setPendingInviteData: (data: InviteFileData | null) => void;
  setSwarmFilePath: (path: string | null) => void;
}

export function useWorkspaceLifecycle(callbacks: WorkspaceLifecycleCallbacks) {
  const [workspace, setWorkspace] = useState<WorkspaceInfoType | null>(null);
  const [unlockedIdentityUuid, setUnlockedIdentityUuid] = useState<string | null>(null);

  // Keep callbacks ref current without triggering effect re-runs
  const callbacksRef = useRef(callbacks);
  useEffect(() => { callbacksRef.current = callbacks; });

  const refreshUnlockedIdentity = useCallback(() => {
    invoke<string[]>('get_unlocked_identities')
      .then(ids => setUnlockedIdentityUuid(ids.length > 0 ? ids[0] : null))
      .catch(() => {});
  }, []);

  // Route a .swarm file to the correct dialog: Phase C invite → ImportInviteDialog,
  // all other formats (WP-A header.json based) → SwarmOpenDialog.
  const openSwarmFile = useCallback((path: string) => {
    invoke<InviteFileData>('import_invite', { path })
      .then(data => {
        callbacksRef.current.setPendingInvitePath(path);
        callbacksRef.current.setPendingInviteData(data);
      })
      .catch(() => {
        // Not a Phase C invite file — fall through to the WP-A swarm open dialog.
        callbacksRef.current.setSwarmFilePath(path);
        callbacksRef.current.setShowSwarmOpen(true);
      });
  }, []);

  // Effect 1: Initial mount — workspace fetch, first-launch identity check
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
          callbacksRef.current.setShowCreateFirstIdentity(true);
        }
      }).catch(err => console.error('Failed to check identities:', err));
    }

    // Load first unlocked identity UUID for swarm operations
    refreshUnlockedIdentity();
  }, []);

  // Effect 2: Swarm dialog lifecycle — refresh unlocked identity whenever a swarm dialog opens
  useEffect(() => {
    if (callbacks.showSwarmInvite || callbacks.showSwarmOpen) refreshUnlockedIdentity();
  }, [callbacks.showSwarmInvite, callbacks.showSwarmOpen]);

  // Effect 3: Cold-start — pull any file path that arrived via OS file-open before JS
  // listeners were registered. Only the "main" (launcher) window handles imports.
  useEffect(() => {
    let cancelled = false;
    const win = getCurrentWebviewWindow();
    if (win.label !== 'main') return;
    invoke<string | null>('consume_pending_file_open').then(path => {
      if (!cancelled && path) callbacksRef.current.proceedWithImport(path, null);
    });
    invoke<string | null>('consume_pending_swarm_file').then(path => {
      if (!cancelled && path) openSwarmFile(path);
    });
    return () => { cancelled = true; };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Effect 4: Warm-start (macOS) — the backend emits "file-opened" when the app is already
  // running and the user opens a .krillnotes file from the OS.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label !== 'main') return;
    const unlisten = win.listen<string>('file-opened', () => {
      // Path is already stored in AppState; use the canonical pull command so
      // both paths (cold and warm start) share the same read-and-clear logic.
      invoke<string | null>('consume_pending_file_open').then(p => {
        if (p) callbacksRef.current.proceedWithImport(p, null);
      });
    });
    return () => { unlisten.then(f => f()); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Effect 5: Warm-start (macOS) — the backend emits "swarm-file-opened" when the app is already
  // running and the user opens a .swarm file from the OS.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label !== 'main') return;
    const unlisten = win.listen<string>('swarm-file-opened', (event) => {
      openSwarmFile(event.payload);
    });
    return () => { unlisten.then(f => f()); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Effect 6: Load settings language on startup
  useEffect(() => {
    invoke<AppSettings>('get_settings')
      .then(s => {
        if (s.language) {
          i18n.changeLanguage(s.language);
        }
      })
      .catch(err => console.error('Failed to load settings for language:', err));
  }, []);

  return {
    workspace,
    unlockedIdentityUuid,
    refreshUnlockedIdentity,
    openSwarmFile,
  };
}
