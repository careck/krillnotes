import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteInfo, PendingPeer, ContactInfo } from '../types';
import { CreateInviteDialog } from './CreateInviteDialog';
import { AcceptPeerDialog } from './AcceptPeerDialog';

interface Props {
  identityUuid: string;
  workspaceName: string;
  onClose: () => void;
}

export function InviteManagerDialog({ identityUuid, workspaceName, onClose }: Props) {
  const { t } = useTranslation();
  const [invites, setInvites] = useState<InviteInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [pendingPeer, setPendingPeer] = useState<PendingPeer | null>(null);
  const [showAccept, setShowAccept] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      const list = await invoke<InviteInfo[]>('list_invites', { identityUuid });
      setInvites(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, [identityUuid]);

  const handleRevoke = async (inviteId: string) => {
    try {
      await invoke('revoke_invite', { identityUuid, inviteId });
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleImportResponse = async () => {
    const path = await open({ filters: [{ name: 'Swarm Response', extensions: ['swarm'] }] });
    if (!path) return;
    try {
      const peer = await invoke<PendingPeer>('import_invite_response', {
        identityUuid,
        path: typeof path === 'string' ? path : path[0],
      });
      setPendingPeer(peer);
      setShowAccept(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const formatExpiry = (invite: InviteInfo) => {
    if (!invite.expiresAt) return t('invite.noExpiry');
    const date = new Date(invite.expiresAt);
    const expired = date < new Date();
    return expired
      ? t('invite.expiredOn', { date: date.toLocaleDateString() })
      : t('invite.expiresOn', { date: date.toLocaleDateString() });
  };

  return (
    <>
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-lg max-h-[80vh] flex flex-col">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold">
              {workspaceName} — {t('invite.manageTitle')}
            </h2>
            <button onClick={onClose} className="text-zinc-400 hover:text-zinc-600 text-xl leading-none">×</button>
          </div>

          {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

          <div className="flex gap-2 mb-4">
            <button
              onClick={() => setShowCreate(true)}
              className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white"
            >
              {t('invite.createInvite')}
            </button>
            <button
              onClick={handleImportResponse}
              className="px-3 py-1.5 text-sm rounded border dark:border-zinc-700"
            >
              {t('invite.importResponse')}
            </button>
          </div>

          <div className="overflow-y-auto flex-1">
            {loading ? (
              <p className="text-sm text-zinc-500 text-center py-8">{t('common.loading')}</p>
            ) : invites.length === 0 ? (
              <p className="text-sm text-zinc-500 text-center py-8">{t('invite.noInvites')}</p>
            ) : (
              <ul className="space-y-2">
                {invites.map(invite => (
                  <li
                    key={invite.inviteId}
                    className="flex items-center justify-between p-3 border rounded dark:border-zinc-700"
                  >
                    <div>
                      <p className="text-sm">{formatExpiry(invite)}</p>
                      <p className="text-xs text-zinc-500">
                        {t('invite.usedCount', { count: invite.useCount })}
                        {invite.revoked && (
                          <span className="ml-2 text-red-500">{t('invite.revoked')}</span>
                        )}
                      </p>
                    </div>
                    {!invite.revoked && (
                      <button
                        onClick={() => handleRevoke(invite.inviteId)}
                        className="text-xs text-red-500 hover:underline"
                      >
                        {t('invite.revoke')}
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>

      {showCreate && (
        <CreateInviteDialog
          identityUuid={identityUuid}
          workspaceName={workspaceName}
          onCreated={() => load()}
          onClose={() => setShowCreate(false)}
        />
      )}

      {showAccept && (
        <AcceptPeerDialog
          identityUuid={identityUuid}
          pendingPeer={pendingPeer}
          onAccepted={(_contact: ContactInfo) => { load(); }}
          onClose={() => { setShowAccept(false); setPendingPeer(null); }}
        />
      )}
    </>
  );
}
