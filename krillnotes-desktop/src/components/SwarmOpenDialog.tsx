// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';

interface InviteInfo {
  mode: 'invite';
  workspaceName: string;
  offeredRole: string;
  offeredScope: string | null;
  inviterDisplayName: string;
  inviterFingerprint: string;
  pairingToken: string;
}

interface AcceptInfo {
  mode: 'accept';
  workspaceName: string;
  declaredName: string;
  acceptorFingerprint: string;
  acceptorPublicKey: string;
  pairingToken: string;
}

interface SnapshotInfo {
  mode: 'snapshot';
  workspaceName: string;
  senderDisplayName: string;
  senderFingerprint: string;
  asOfOperationId: string;
}

type SwarmFileInfo = InviteInfo | AcceptInfo | SnapshotInfo;

interface Props {
  isOpen: boolean;
  onClose: () => void;
  swarmFilePath: string | null;
  unlockedIdentityUuid: string | null;
  deviceId: string;
}

export default function SwarmOpenDialog({
  isOpen, onClose, swarmFilePath, unlockedIdentityUuid, deviceId,
}: Props) {
  const { t } = useTranslation();
  const [fileInfo, setFileInfo] = useState<SwarmFileInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');
  const [workspaceName, setWorkspaceName] = useState('');
  const [declaredName, setDeclaredName] = useState('');

  useEffect(() => {
    if (!isOpen || !swarmFilePath) return;
    setLoading(true); setError(''); setFileInfo(null); setSuccess('');
    invoke<SwarmFileInfo>('open_swarm_file_cmd', { path: swarmFilePath })
      .then(info => {
        setFileInfo(info);
        if (info.mode === 'snapshot') setWorkspaceName(info.workspaceName);
      })
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [isOpen, swarmFilePath]);

  useEffect(() => {
    if (!isOpen) {
      setFileInfo(null); setError(''); setSuccess('');
      setWorkspaceName(''); setDeclaredName('');
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleAcceptInvite = async () => {
    if (!fileInfo || fileInfo.mode !== 'invite' || !swarmFilePath) return;
    if (!unlockedIdentityUuid) { setError(t('swarm.identityLocked')); return; }
    if (!declaredName.trim()) { setError(t('swarm.contactNameLabel') + ' required'); return; }
    const savePath = await save({
      filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
      defaultPath: `accept-${fileInfo.workspaceName.replace(/\s+/g, '-')}.swarm`,
    });
    if (!savePath) return;
    setProcessing(true); setError('');
    try {
      await invoke('create_accept_bundle_cmd', {
        invitePath: swarmFilePath,
        declaredName: declaredName.trim(),
        sourceDeviceId: deviceId,
        identityUuid: unlockedIdentityUuid,
        savePath,
      });
      setSuccess(t('swarm.acceptSaved', { name: fileInfo.inviterDisplayName }));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg === 'IDENTITY_LOCKED' ? t('swarm.identityLocked') : msg);
    } finally { setProcessing(false); }
  };

  const handleSendSnapshot = async () => {
    if (!fileInfo || fileInfo.mode !== 'accept' || !swarmFilePath) return;
    if (!unlockedIdentityUuid) { setError(t('swarm.identityLocked')); return; }
    const savePath = await save({
      filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
      defaultPath: `snapshot-${fileInfo.workspaceName.replace(/\s+/g, '-')}.swarm`,
    });
    if (!savePath) return;
    setProcessing(true); setError('');
    try {
      await invoke('create_snapshot_bundle_cmd', {
        acceptPath: swarmFilePath,
        identityUuid: unlockedIdentityUuid,
        savePath,
      });
      setSuccess(t('swarm.snapshotSaved', { name: fileInfo.declaredName }));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg === 'IDENTITY_LOCKED' ? t('swarm.identityLocked') : msg);
    } finally { setProcessing(false); }
  };

  const handleCreateWorkspace = async () => {
    if (!fileInfo || fileInfo.mode !== 'snapshot' || !swarmFilePath) return;
    if (!unlockedIdentityUuid) { setError(t('swarm.identityLocked')); return; }
    setProcessing(true); setError('');
    try {
      await invoke('create_workspace_from_snapshot_cmd', {
        snapshotPath: swarmFilePath,
        workspaceName: workspaceName.trim() || fileInfo.workspaceName,
        identityUuid: unlockedIdentityUuid,
      });
      onClose();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg === 'IDENTITY_LOCKED' ? t('swarm.identityLocked') : msg);
    } finally { setProcessing(false); }
  };

  const FingerprintBadge = ({ fp }: { fp: string }) => (
    <code className="block mt-1 text-xs font-mono bg-[--color-code-bg] px-2 py-1 rounded tracking-wide">
      {fp}
    </code>
  );

  const renderContent = () => {
    if (loading) return <p className="text-sm text-[--color-muted]">{t('swarm.loading')}</p>;
    if (!fileInfo) return null;

    if (fileInfo.mode === 'invite') return (
      <div className="space-y-3">
        <h3 className="font-medium">{t('swarm.inviteModeHeading')}</h3>
        <div className="text-sm space-y-1">
          <p><span className="text-[--color-muted]">{t('swarm.inviteWorkspace')}: </span>{fileInfo.workspaceName}</p>
          <p><span className="text-[--color-muted]">{t('swarm.inviteFrom')}: </span>{fileInfo.inviterDisplayName}</p>
          <p><span className="text-[--color-muted]">{t('swarm.inviteOfferedRole')}: </span>{fileInfo.offeredRole}</p>
          <p className="text-[--color-muted] text-xs">{t('swarm.inviteFingerprint')}:</p>
          <FingerprintBadge fp={fileInfo.inviterFingerprint} />
        </div>
        <div>
          <label className="block text-sm font-medium mb-1">{t('swarm.contactNameLabel')}</label>
          <input
            className="w-full border border-[--color-border] rounded px-3 py-2 bg-[--color-bg] text-sm"
            value={declaredName}
            onChange={e => setDeclaredName(e.target.value)}
            placeholder={t('swarm.contactNamePlaceholder')}
            disabled={processing}
          />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-[--color-accent] text-white hover:opacity-90 disabled:opacity-50"
          onClick={handleAcceptInvite}
          disabled={processing || !declaredName.trim()}
        >
          {processing ? '…' : t('swarm.acceptButton')}
        </button>
      </div>
    );

    if (fileInfo.mode === 'accept') return (
      <div className="space-y-3">
        <h3 className="font-medium">{t('swarm.acceptModeHeading', { name: fileInfo.declaredName })}</h3>
        <div className="text-sm space-y-1">
          <p><span className="text-[--color-muted]">{t('swarm.inviteWorkspace')}: </span>{fileInfo.workspaceName}</p>
          <p className="text-[--color-muted] text-xs">{t('swarm.acceptorFingerprint')}</p>
          <FingerprintBadge fp={fileInfo.acceptorFingerprint} />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-[--color-accent] text-white hover:opacity-90 disabled:opacity-50"
          onClick={handleSendSnapshot}
          disabled={processing}
        >
          {processing ? '…' : t('swarm.sendSnapshotButton')}
        </button>
      </div>
    );

    if (fileInfo.mode === 'snapshot') return (
      <div className="space-y-3">
        <h3 className="font-medium">{t('swarm.snapshotModeHeading', { name: fileInfo.senderDisplayName })}</h3>
        <div className="text-sm space-y-1">
          <p className="text-[--color-muted] text-xs">{t('swarm.inviteFingerprint')}:</p>
          <FingerprintBadge fp={fileInfo.senderFingerprint} />
        </div>
        <div>
          <label className="block text-sm font-medium mb-1">{t('swarm.snapshotWorkspaceNameLabel')}</label>
          <input
            className="w-full border border-[--color-border] rounded px-3 py-2 bg-[--color-bg] text-sm"
            value={workspaceName}
            onChange={e => setWorkspaceName(e.target.value)}
            disabled={processing}
          />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-[--color-accent] text-white hover:opacity-90 disabled:opacity-50"
          onClick={handleCreateWorkspace}
          disabled={processing || !workspaceName.trim()}
        >
          {processing ? '…' : t('swarm.createWorkspaceButton')}
        </button>
      </div>
    );

    return null;
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-[--color-bg] border border-[--color-border] rounded-lg shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">{t('swarm.openDialogTitle')}</h2>

        {renderContent()}

        {error && <p className="mt-3 text-sm text-red-500">{error}</p>}
        {success && <p className="mt-3 text-sm text-green-600">{success}</p>}

        <div className="flex justify-end mt-4">
          <button
            className="px-4 py-2 rounded border border-[--color-border] hover:bg-[--color-hover]"
            onClick={onClose}
            disabled={processing}
          >
            {t('common.close', 'Close')}
          </button>
        </div>
      </div>
    </div>
  );
}
