import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteInfo } from '../types';

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
  // Import response from link state
  const [responseUrl, setResponseUrl] = useState('');
  const [fetchingResponse, setFetchingResponse] = useState(false);

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

  const handleDelete = async (inviteId: string) => {
    try {
      await invoke('delete_invite', { identityUuid, inviteId });
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  const handlePurgeRevoked = async () => {
    try {
      await invoke('delete_revoked_invites', { identityUuid });
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  const hasRevoked = invites.some(i => i.revoked);

  const extractRelayToken = (url: string): string | null => {
    try {
      const parsed = new URL(url.trim());
      const segments = parsed.pathname.split('/').filter(Boolean);
      return segments[segments.length - 1] ?? null;
    } catch {
      return null;
    }
  };

  const handleFetchResponseFromLink = async () => {
    const token = extractRelayToken(responseUrl);
    if (!token) {
      setError(t('invite.invalidRelayUrl'));
      return;
    }
    setFetchingResponse(true);
    setError(null);
    try {
      await invoke('fetch_relay_invite_response', {
        identityUuid,
        token,
      });
      setResponseUrl('');
      load();
    } catch (e) {
      setError(String(e));
    } finally {
      setFetchingResponse(false);
    }
  };

  const handleImportResponse = async () => {
    const path = await open({ filters: [{ name: 'Swarm Response', extensions: ['swarm'] }] });
    if (!path) return;
    try {
      await invoke('import_invite_response', {
        identityUuid,
        path: typeof path === 'string' ? path : path[0],
      });
      load();
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
      <div className="fixed inset-0 z-60 flex items-center justify-center bg-black/50">
        <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg shadow-xl w-[520px] max-h-[80vh] flex flex-col">

          {/* Header */}
          <div className="flex items-center justify-between p-4 border-b border-[var(--color-border)]">
            <h2 className="text-lg font-semibold">
              {workspaceName} — {t('invite.manageTitle')}
            </h2>
            <button
              onClick={onClose}
              className="text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)] px-2"
            >
              ✕
            </button>
          </div>

          {/* Status messages */}
          <div className="px-4 pt-3">
            {error && <p className="text-sm text-red-500 p-2 rounded bg-red-500/10 mb-2">{error}</p>}
          </div>

          {/* Action buttons */}
          <div className="flex gap-2 px-4 py-3">
            <button
              onClick={handleImportResponse}
              className="px-3 py-1.5 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
            >
              {t('invite.importResponse')}
            </button>
            {hasRevoked && (
              <button
                onClick={handlePurgeRevoked}
                className="px-3 py-1.5 text-sm rounded border border-red-500/30 text-red-500 hover:bg-red-500/10"
              >
                {t('invite.purgeRevoked', 'Purge Revoked')}
              </button>
            )}
          </div>

          {/* Import Response from Link */}
          <div className="mx-4 mb-3 p-3 border border-[var(--color-border)] rounded-md bg-[var(--color-secondary)]/30">
            <p className="text-xs font-medium text-[var(--color-muted-foreground)] mb-1">
              {t('invite.importResponseFromLink')}
            </p>
            <div className="flex gap-2">
              <input
                type="url"
                value={responseUrl}
                onChange={e => setResponseUrl(e.target.value)}
                placeholder={t('invite.pasteResponseUrl')}
                className="flex-1 border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                disabled={fetchingResponse}
              />
              <button
                onClick={handleFetchResponseFromLink}
                disabled={!responseUrl.trim() || fetchingResponse}
                className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
              >
                {fetchingResponse ? t('common.loading') : t('invite.fetchResponse')}
              </button>
            </div>
          </div>

          {/* Invite list */}
          <div className="overflow-y-auto flex-1 px-4 pb-4">
            {loading ? (
              <p className="text-sm text-[var(--color-muted-foreground)] text-center py-8">{t('common.loading')}</p>
            ) : invites.length === 0 ? (
              <p className="text-sm text-[var(--color-muted-foreground)] text-center py-8">{t('invite.noInvites')}</p>
            ) : (
              <ul className="space-y-2">
                {invites.map(invite => (
                  <li
                    key={invite.inviteId}
                    className="flex items-center justify-between p-3 rounded-md border border-[var(--color-border)] bg-[var(--color-secondary)]/30"
                  >
                    <div className="flex-1 min-w-0">
                      <p className="text-sm">{formatExpiry(invite)}</p>
                      <div className="flex items-center gap-2 flex-wrap mt-0.5">
                        <p className="text-xs text-[var(--color-muted-foreground)]">
                          {t('invite.usedCount', { count: invite.useCount })}
                          {invite.revoked && (
                            <span className="ml-2 text-red-500">{t('invite.revoked')}</span>
                          )}
                          {invite.scopeNoteId && (
                            <span className="ml-2 text-xs text-zinc-400">
                              → {invite.scopeNoteTitle ?? invite.scopeNoteId}
                            </span>
                          )}
                        </p>
                        {invite.offeredRole && (
                          <span className={`px-2 py-0.5 rounded text-xs font-medium ${
                            invite.offeredRole === 'owner' ? 'bg-purple-500/20 text-purple-300' :
                            invite.offeredRole === 'writer' ? 'bg-green-500/20 text-green-300' :
                            'bg-blue-500/20 text-blue-300'
                          }`}>
                            {t(`roles.${invite.offeredRole}Short`)}
                          </span>
                        )}
                      </div>
                      {invite.relayUrl && (
                        <p className="text-xs text-[var(--color-muted-foreground)] font-mono truncate mt-0.5" title={invite.relayUrl}>
                          {invite.relayUrl}
                        </p>
                      )}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {invite.relayUrl && (
                        <button
                          onClick={() => { try { navigator.clipboard.writeText(invite.relayUrl!); } catch { /* fallback: URL is visible */ } }}
                          className="text-xs px-2 py-1 rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
                        >
                          {t('invite.copyLink')}
                        </button>
                      )}
                      {!invite.revoked && (
                        <button
                          onClick={() => handleRevoke(invite.inviteId)}
                          className="text-xs text-red-500 hover:underline"
                        >
                          {t('invite.revoke')}
                        </button>
                      )}
                      {invite.revoked && (
                        <button
                          onClick={() => handleDelete(invite.inviteId)}
                          className="p-1.5 rounded hover:bg-[var(--color-secondary)] text-[var(--color-muted-foreground)] hover:text-red-500 text-sm"
                          title={t('invite.deleteInvite', 'Delete invite')}
                        >
                          🗑
                        </button>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
