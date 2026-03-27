// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { ReceivedResponseInfo } from '../types';

interface OnboardPeerDialogProps {
  open: boolean;
  response: ReceivedResponseInfo;
  identityUuid: string;
  onComplete: () => void;
  onClose: () => void;
}

export function OnboardPeerDialog({
  open, response, identityUuid, onComplete, onClose,
}: OnboardPeerDialogProps) {
  const { t } = useTranslation();
  const [processing, setProcessing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;

  const handleGrantAndSync = async () => {
    setProcessing(true);
    setError(null);
    try {
      // Step 1: Accept peer (add to workspace) — skip if already added
      if (response.status === 'pending') {
        await invoke('accept_peer', {
          identityUuid,
          inviteePublicKey: response.inviteePublicKey,
          declaredName: response.inviteeDeclaredName,
          trustLevel: 'Tofu',
          localName: null,
        });
      }

      // Step 2: Grant permission on the scoped subtree using the offered role
      if (response.scopeNoteId) {
        await invoke('set_permission', {
          noteId: response.scopeNoteId,
          userId: response.inviteePublicKey,
          role: response.offeredRole,
        });
      }

      // Step 2b: Set relay channel so ongoing sync routes via the same relay
      if (response.responseChannel === 'relay' && response.relayAccountId) {
        await invoke('set_peer_relay', {
          peerDeviceId: `identity:${response.inviteePublicKey}`,
          relayAccountId: response.relayAccountId,
        });
      }

      // Step 3: Send snapshot via the channel the response arrived on
      if (response.responseChannel === 'relay') {
        await invoke('send_snapshot_via_relay', {
          identityUuid,
          peerPublicKeys: [response.inviteePublicKey],
        });
      } else {
        const savePath = await save({
          defaultPath: `${response.workspaceName}-snapshot.swarm`,
          filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
        });
        if (!savePath) { setProcessing(false); return; }
        await invoke('create_snapshot_for_peers', {
          identityUuid,
          peerPublicKeys: [response.inviteePublicKey],
          savePath,
        });
      }

      // Step 4: Update response status
      await invoke('update_response_status', {
        identityUuid,
        responseId: response.responseId,
        status: 'snapshotSent',
      });

      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  const handleReject = async () => {
    setProcessing(true);
    try {
      await invoke('dismiss_response', {
        identityUuid,
        responseId: response.responseId,
      });
      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70" onClick={onClose}>
      <div className="bg-background border border-border rounded-xl shadow-xl p-6 w-full max-w-md" onClick={e => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">
            {t('onboard.title', 'Onboard Peer')}
          </h2>
          <button onClick={onClose} className="text-muted-foreground hover:text-foreground">
            ✕
          </button>
        </div>

        {/* Peer card */}
        <div className="bg-secondary rounded-lg p-3 mb-4">
          <p className="font-medium">{response.inviteeDeclaredName}</p>
          <p className="text-xs text-muted-foreground font-mono truncate">
            {response.inviteePublicKey.slice(0, 16)}…
          </p>
        </div>

        {/* Scope reminder */}
        {response.scopeNoteTitle && (
          <div className="mb-4">
            <label className="block text-sm font-medium text-muted-foreground mb-1">
              {t('onboard.scope', 'Invited to subtree')}
            </label>
            <p className="text-sm bg-secondary rounded px-3 py-1.5">
              {response.scopeNoteTitle}
            </p>
          </div>
        )}

        {/* Role (read-only badge) */}
        <div className="mb-4 text-sm">
          <span className="text-muted-foreground">{t('invite.role')}</span>
          <span className={`ml-2 px-2 py-0.5 rounded text-xs font-medium ${
            response.offeredRole === 'owner' ? 'bg-purple-500/20 text-purple-300' :
            response.offeredRole === 'writer' ? 'bg-green-500/20 text-green-300' :
            'bg-blue-500/20 text-blue-300'
          }`}>
            {t(`roles.${response.offeredRole}Short`)}
          </span>
        </div>

        {/* Channel (read-only display) */}
        <div className="mb-4 text-sm">
          <span className="text-muted-foreground">{t('invite.channel')}</span>
          <span className="ml-2">
            {response.responseChannel === 'relay' ? '🔗 Relay' : '💾 File'}
          </span>
        </div>

        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        {/* Actions */}
        <div className="flex justify-between">
          <button
            onClick={handleReject}
            disabled={processing}
            className="px-3 py-2 text-sm text-red-600 hover:bg-red-500/10 rounded disabled:opacity-50"
          >
            {t('onboard.reject', 'Reject')}
          </button>
          <button
            onClick={handleGrantAndSync}
            disabled={processing}
            className="px-4 py-2 text-sm rounded bg-primary text-primary-foreground disabled:opacity-50"
          >
            {processing
              ? t('common.saving', 'Saving…')
              : t('onboard.grantAndSync', 'Grant & sync')}
          </button>
        </div>
      </div>
    </div>
  );
}
