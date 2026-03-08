// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { WorkspaceInfo } from '../types';

interface Props {
  isOpen: boolean;
  onClose: () => void;
  workspaceInfo: WorkspaceInfo | null;
  unlockedIdentityUuid: string | null;
  deviceId: string;
}

export default function SwarmInviteDialog({
  isOpen, onClose, workspaceInfo, unlockedIdentityUuid, deviceId,
}: Props) {
  const { t } = useTranslation();
  const [contactName, setContactName] = useState('');
  const [publicKey, setPublicKey] = useState('');
  const [role, setRole] = useState('writer');
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');

  useEffect(() => {
    if (!isOpen) {
      setContactName(''); setPublicKey(''); setRole('writer');
      setError(''); setSuccess(''); setCreating(false);
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleCreate = async () => {
    if (!workspaceInfo) return;
    if (!unlockedIdentityUuid) { setError(t('swarm.identityLocked')); return; }
    if (!contactName.trim()) { setError(t('swarm.contactNameLabel') + ' required'); return; }
    if (!publicKey.trim()) { setError(t('swarm.publicKeyLabel') + ' required'); return; }

    const savePath = await save({
      filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
      defaultPath: `invite-${contactName.trim().replace(/\s+/g, '-')}.swarm`,
    });
    if (!savePath) { setError(t('swarm.saveCancelled')); return; }

    setCreating(true); setError('');
    try {
      await invoke('create_invite_bundle_cmd', {
        workspaceId: workspaceInfo.path,
        workspaceName: workspaceInfo.filename,
        contactName: contactName.trim(),
        contactPublicKey: publicKey.trim(),
        offeredRole: role,
        offeredScope: null,
        sourceDeviceId: deviceId,
        identityUuid: unlockedIdentityUuid,
        savePath,
      });
      setSuccess(t('swarm.inviteSaved', { name: contactName.trim() }));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg === 'IDENTITY_LOCKED' ? t('swarm.identityLocked') : msg);
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary rounded-lg shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">{t('swarm.inviteDialogTitle')}</h2>

        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium mb-1">{t('swarm.contactNameLabel')}</label>
            <input
              className="w-full border border-secondary rounded px-3 py-2 bg-background"
              value={contactName}
              onChange={e => setContactName(e.target.value)}
              placeholder={t('swarm.contactNamePlaceholder')}
              disabled={creating}
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-1">{t('swarm.publicKeyLabel')}</label>
            <textarea
              className="w-full border border-secondary rounded px-3 py-2 bg-background font-mono text-xs"
              rows={3}
              value={publicKey}
              onChange={e => setPublicKey(e.target.value)}
              placeholder={t('swarm.publicKeyPlaceholder')}
              disabled={creating}
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-1">{t('swarm.roleLabel')}</label>
            <select
              className="w-full border border-secondary rounded px-3 py-2 bg-background"
              value={role}
              onChange={e => setRole(e.target.value)}
              disabled={creating}
            >
              <option value="owner">{t('swarm.roleOwner')}</option>
              <option value="writer">{t('swarm.roleWriter')}</option>
              <option value="reader">{t('swarm.roleReader')}</option>
            </select>
          </div>
        </div>

        {error && <p className="mt-3 text-sm text-red-500">{error}</p>}
        {success && <p className="mt-3 text-sm text-green-600">{success}</p>}

        <div className="flex justify-end gap-3 mt-6">
          <button
            className="px-4 py-2 rounded border border-secondary hover:bg-secondary"
            onClick={onClose}
            disabled={creating}
          >
            {success ? t('common.close', 'Close') : t('common.cancel', 'Cancel')}
          </button>
          {!success && (
            <button
              className="px-4 py-2 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
              onClick={handleCreate}
              disabled={creating || !contactName.trim() || !publicKey.trim()}
            >
              {creating ? '…' : t('swarm.createInviteButton')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
