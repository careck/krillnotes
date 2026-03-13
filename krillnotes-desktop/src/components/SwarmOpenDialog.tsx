// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import UnlockIdentityDialog from './UnlockIdentityDialog';
import { SendSnapshotDialog } from './SendSnapshotDialog';

interface InviteInfo {
  mode: 'invite';
  workspaceName: string;
  offeredRole: string;
  offeredScope: string | null;
  inviterDisplayName: string;
  inviterFingerprint: string;
  pairingToken: string;
  targetIdentityUuid: string | null;
  targetIdentityName: string | null;
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
  targetIdentityUuid: string | null;
  targetIdentityName: string | null;
}

interface DeltaInfo {
  mode: 'delta';
  workspaceName: string;
  localWorkspaceName: string | null;
  senderDisplayName: string;
  senderFingerprint: string;
  sinceOperationId: string | null;
  targetIdentityUuid: string | null;
  targetIdentityName: string | null;
}

type SwarmFileInfo = InviteInfo | AcceptInfo | SnapshotInfo | DeltaInfo;

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
  const [unlockTarget, setUnlockTarget] = useState<{ uuid: string; name: string } | null>(null);
  const [showSendSnapshot, setShowSendSnapshot] = useState(false);

  useEffect(() => {
    if (!isOpen || !swarmFilePath) return;
    setLoading(true); setError(''); setFileInfo(null); setSuccess('');
    invoke<SwarmFileInfo>('open_swarm_file_cmd', { path: swarmFilePath })
      .then(async info => {
        setFileInfo(info);
        // If we know which local identity is required, check if it's already unlocked.
        // Only prompt the user if it genuinely needs to be unlocked.
        if ((info.mode === 'invite' || info.mode === 'snapshot' || info.mode === 'delta') &&
            info.targetIdentityUuid && info.targetIdentityName) {
          const alreadyUnlocked = await invoke<boolean>('is_identity_unlocked', {
            identityUuid: info.targetIdentityUuid,
          }).catch(() => false);
          if (!alreadyUnlocked) {
            setUnlockTarget({ uuid: info.targetIdentityUuid, name: info.targetIdentityName });
          }
        }
        if (info.mode === 'snapshot') setWorkspaceName(info.workspaceName);
      })
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [isOpen, swarmFilePath]);

  useEffect(() => {
    if (!isOpen) {
      setFileInfo(null); setError(''); setSuccess('');
      setWorkspaceName(''); setDeclaredName(''); setUnlockTarget(null);
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleAcceptInvite = async (identityUuid?: string) => {
    if (!fileInfo || fileInfo.mode !== 'invite' || !swarmFilePath) return;
    if (!declaredName.trim()) { setError(t('swarm.contactNameLabel') + ' required'); return; }
    const uuid = identityUuid ?? (fileInfo as InviteInfo).targetIdentityUuid ?? unlockedIdentityUuid;
    if (!uuid) {
      if (fileInfo.targetIdentityUuid && fileInfo.targetIdentityName) {
        setUnlockTarget({ uuid: fileInfo.targetIdentityUuid, name: fileInfo.targetIdentityName });
      } else {
        setError(t('swarm.identityLocked'));
      }
      return;
    }
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
        identityUuid: uuid,
        savePath,
      });
      setSuccess(t('swarm.acceptSaved', { name: fileInfo.inviterDisplayName }));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg === 'IDENTITY_LOCKED' && fileInfo.targetIdentityUuid && fileInfo.targetIdentityName) {
        setUnlockTarget({ uuid: fileInfo.targetIdentityUuid, name: fileInfo.targetIdentityName });
      } else {
        setError(msg);
      }
    } finally { setProcessing(false); }
  };

  const handleSendSnapshot = () => {
    setShowSendSnapshot(true);
  };

  const handleApplyDelta = async (identityUuid?: string) => {
    if (!fileInfo || fileInfo.mode !== 'delta' || !swarmFilePath) return;
    const uuid = identityUuid ?? (fileInfo as DeltaInfo).targetIdentityUuid ?? unlockedIdentityUuid;
    if (!uuid) {
      if (fileInfo.targetIdentityUuid && fileInfo.targetIdentityName) {
        setUnlockTarget({ uuid: fileInfo.targetIdentityUuid, name: fileInfo.targetIdentityName });
      } else {
        setError(t('swarm.identityLocked'));
      }
      return;
    }
    setProcessing(true); setError('');
    try {
      const resultJson = await invoke<string>('apply_swarm_delta', {
        path: swarmFilePath,
        identityUuid: uuid,
      });
      const result = JSON.parse(resultJson);
      setSuccess(`Applied ${result.operationsApplied} operation(s).`);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setProcessing(false); }
  };

  const handleCreateWorkspace = async (identityUuid?: string) => {
    if (!fileInfo || fileInfo.mode !== 'snapshot' || !swarmFilePath) return;
    const uuid = identityUuid ?? (fileInfo as SnapshotInfo).targetIdentityUuid ?? unlockedIdentityUuid;
    if (!uuid) {
      // If we know which identity is needed, prompt to unlock it.
      if (fileInfo.targetIdentityUuid && fileInfo.targetIdentityName) {
        setUnlockTarget({ uuid: fileInfo.targetIdentityUuid, name: fileInfo.targetIdentityName });
      } else {
        setError(t('swarm.identityLocked'));
      }
      return;
    }
    setProcessing(true); setError('');
    try {
      await invoke('apply_swarm_snapshot', {
        path: swarmFilePath,
        identityUuid: uuid,
        workspaceNameOverride: workspaceName.trim() || undefined,
      });
      onClose();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg === 'IDENTITY_LOCKED' && fileInfo.targetIdentityUuid && fileInfo.targetIdentityName) {
        setUnlockTarget({ uuid: fileInfo.targetIdentityUuid, name: fileInfo.targetIdentityName });
      } else {
        setError(msg);
      }
    } finally { setProcessing(false); }
  };

  const FingerprintBadge = ({ fp }: { fp: string }) => (
    <code className="block mt-1 text-xs font-mono bg-secondary px-2 py-1 rounded tracking-wide">
      {fp}
    </code>
  );

  const renderContent = () => {
    if (loading) return <p className="text-sm text-muted-foreground">{t('swarm.loading')}</p>;
    if (!fileInfo) return null;

    if (fileInfo.mode === 'invite') return (
      <div className="space-y-3">
        <h3 className="font-medium">{t('swarm.inviteModeHeading')}</h3>
        <div className="text-sm space-y-1">
          <p><span className="text-muted-foreground">{t('swarm.inviteWorkspace')}: </span>{fileInfo.workspaceName}</p>
          <p><span className="text-muted-foreground">{t('swarm.inviteFrom')}: </span>{fileInfo.inviterDisplayName}</p>
          <p><span className="text-muted-foreground">{t('swarm.inviteOfferedRole')}: </span>{fileInfo.offeredRole}</p>
          <p className="text-muted-foreground text-xs">{t('swarm.inviteFingerprint')}:</p>
          <FingerprintBadge fp={fileInfo.inviterFingerprint} />
        </div>
        <div>
          <label className="block text-sm font-medium mb-1">{t('swarm.contactNameLabel')}</label>
          <input
            className="w-full border border-secondary rounded px-3 py-2 bg-background text-sm"
            value={declaredName}
            onChange={e => setDeclaredName(e.target.value)}
            placeholder={t('swarm.contactNamePlaceholder')}
            disabled={processing}
          />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
          onClick={() => handleAcceptInvite()}
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
          <p><span className="text-muted-foreground">{t('swarm.inviteWorkspace')}: </span>{fileInfo.workspaceName}</p>
          <p className="text-muted-foreground text-xs">{t('swarm.acceptorFingerprint')}</p>
          <FingerprintBadge fp={fileInfo.acceptorFingerprint} />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
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
          <p className="text-muted-foreground text-xs">{t('swarm.inviteFingerprint')}:</p>
          <FingerprintBadge fp={fileInfo.senderFingerprint} />
        </div>
        <div>
          <label className="block text-sm font-medium mb-1">{t('swarm.snapshotWorkspaceNameLabel')}</label>
          <input
            className="w-full border border-secondary rounded px-3 py-2 bg-background text-sm"
            value={workspaceName}
            onChange={e => setWorkspaceName(e.target.value)}
            disabled={processing}
          />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
          onClick={() => handleCreateWorkspace()}
          disabled={processing || !workspaceName.trim()}
        >
          {processing ? '…' : t('swarm.createWorkspaceButton')}
        </button>
      </div>
    );

    if (fileInfo.mode === 'delta') return (
      <div className="space-y-3">
        <h3 className="font-medium">Delta from {fileInfo.senderDisplayName}</h3>
        <div className="text-sm space-y-1">
          <p><span className="text-muted-foreground">Workspace: </span>{fileInfo.localWorkspaceName ?? fileInfo.workspaceName}</p>
          <p className="text-muted-foreground text-xs">Sender fingerprint:</p>
          <FingerprintBadge fp={fileInfo.senderFingerprint} />
        </div>
        <button
          className="w-full px-4 py-2 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
          onClick={() => handleApplyDelta()}
          disabled={processing}
        >
          {processing ? '…' : 'Apply Delta'}
        </button>
      </div>
    );

    return null;
  };

  return (
    <>
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-background border border-secondary rounded-lg shadow-xl p-6 w-full max-w-md">
          <h2 className="text-lg font-semibold mb-4">{t('swarm.openDialogTitle')}</h2>

          {renderContent()}

          {error && <p className="mt-3 text-sm text-red-500">{error}</p>}
          {success && <p className="mt-3 text-sm text-green-600">{success}</p>}

          <div className="flex justify-end mt-4">
            <button
              className="px-4 py-2 rounded border border-secondary hover:bg-secondary"
              onClick={onClose}
              disabled={processing}
            >
              {t('common.close', 'Close')}
            </button>
          </div>
        </div>
      </div>

      {unlockTarget && (
        <UnlockIdentityDialog
          isOpen={true}
          identityUuid={unlockTarget.uuid}
          identityName={unlockTarget.name}
          onUnlocked={() => {
            const uuid = unlockTarget.uuid;
            setUnlockTarget(null);
            if (fileInfo?.mode === 'invite') handleAcceptInvite(uuid);
            else if (fileInfo?.mode === 'delta') handleApplyDelta(uuid);
            else handleCreateWorkspace(uuid);
          }}
          onCancel={() => setUnlockTarget(null)}
        />
      )}

      {fileInfo?.mode === 'accept' && (
        <SendSnapshotDialog
          open={showSendSnapshot}
          identityUuid={unlockedIdentityUuid ?? ''}
          preSelectedPublicKeys={fileInfo.acceptorPublicKey ? [fileInfo.acceptorPublicKey] : []}
          onClose={() => setShowSendSnapshot(false)}
          onSuccess={() => { setShowSendSnapshot(false); setSuccess('Snapshot saved.'); onClose(); }}
        />
      )}
    </>
  );
}
