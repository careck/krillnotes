// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useEffect, useRef } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import type { WorkspaceInfo as WorkspaceInfoType } from '../types';

export interface MenuEventCallbacks {
  setShowNewWorkspace: (show: boolean) => void;
  setShowOpenWorkspace: (show: boolean) => void;
  setShowSettings: (show: boolean) => void;
  setShowExportPasswordDialog: (show: boolean) => void;
  setShowIdentityManager: (show: boolean) => void;
  setShowSwarmInvite: (show: boolean) => void;
  setShowAcceptInvite: (show: boolean) => void;
  setShowWorkspacePeers: (show: boolean) => void;
  setShowCreateDeltaDialog: (show: boolean) => void;
  statusSetter: (msg: string, isError?: boolean) => void;
  proceedWithImport: (zipPath: string, password: string | null) => Promise<void>;
  openSwarmFile: (path: string) => void;
}

function createMenuHandlers(callbacks: MenuEventCallbacks) {
  const {
    setShowNewWorkspace,
    setShowOpenWorkspace,
    setShowSettings,
    setShowExportPasswordDialog,
    setShowIdentityManager,
    setShowSwarmInvite,
    setShowAcceptInvite,
    setShowWorkspacePeers,
    setShowCreateDeltaDialog,
    statusSetter,
    proceedWithImport,
    openSwarmFile,
  } = callbacks;

  return {
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
        proceedWithImport(zipPath as string, null);
      } catch (error) {
        statusSetter(`Import failed: ${error}`, true);
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

    'File > Accept Invite clicked': () => {
      setShowAcceptInvite(true);
    },

    'Edit > Workspace Peers clicked': () => {
      setShowWorkspacePeers(true);
    },

    'Edit > Create delta Swarm clicked': () => {
      setShowCreateDeltaDialog(true);
    },

    'File > Open Swarm File clicked': async () => {
      try {
        const picked = await open({
          filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
          multiple: false,
          title: 'Open .swarm file',
        });
        if (!picked || Array.isArray(picked)) return;
        openSwarmFile(picked as string);
      } catch {
        // user cancelled
      }
    },
  };
}

export function useMenuEvents(
  workspace: WorkspaceInfoType | null, // triggers listener re-registration on workspace change
  callbacks: MenuEventCallbacks,
): void {
  const callbacksRef = useRef(callbacks);
  useEffect(() => { callbacksRef.current = callbacks; }); // sync every render, no dep array

  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
      const handlers = createMenuHandlers(callbacksRef.current);
      const handler = handlers[event.payload as keyof typeof handlers];
      if (handler) handler();
    });

    return () => { unlisten.then(f => f()); };
  }, [workspace]);
}
