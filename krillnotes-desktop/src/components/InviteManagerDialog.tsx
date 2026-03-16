import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteInfo, PendingPeer, ContactInfo } from '../types';
import { CreateInviteDialog } from './CreateInviteDialog';
import { AcceptPeerDialog } from './AcceptPeerDialog';
import { PostAcceptDialog } from './PostAcceptDialog';
import { SendSnapshotDialog } from './SendSnapshotDialog';
import AddRelayAccountDialog from './AddRelayAccountDialog';

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
  const [postAcceptPeer, setPostAcceptPeer] = useState<{ name: string; publicKey: string } | null>(null);
  const [showSendSnapshot, setShowSendSnapshot] = useState(false);
  const [sendSnapshotFor, setSendSnapshotFor] = useState<string[]>([]);
  // Share Invite Link state
  const [sharingLink, setSharingLink] = useState(false);
  const [shareError, setShareError] = useState<string | null>(null);
  const [shareSuccess, setShareSuccess] = useState<string | null>(null);
  const [showRelaySetup, setShowRelaySetup] = useState(false);
  const [pendingShareAction, setPendingShareAction] = useState(false);
  // Upload to relay state (per-invite)
  const [uploadingRelayFor, setUploadingRelayFor] = useState<string | null>(null);
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

  const handleShareInviteLink = async () => {
    setSharingLink(true);
    setShareError(null);
    setShareSuccess(null);
    try {
      const hasRelay = await invoke<boolean>('has_relay_credentials');
      if (!hasRelay) {
        setPendingShareAction(true);
        setShowRelaySetup(true);
        setSharingLink(false);
        return;
      }
      await doShareInviteLink();
    } catch (e) {
      setShareError(String(e));
      setSharingLink(false);
    }
  };

  const doShareInviteLink = async () => {
    setSharingLink(true);
    setShareError(null);
    try {
      const info = await invoke<{ relayUrl: string | null }>('share_invite_link', {
        identityUuid,
        workspaceName,
        expiresInDays: 7,
      });
      if (info.relayUrl) {
        try {
          await navigator.clipboard.writeText(info.relayUrl);
          setShareSuccess(t('invite.linkCopied'));
        } catch {
          // WKWebView blocks clipboard after async — show URL as fallback
          setShareSuccess(info.relayUrl);
        }
      }
      await load();
    } catch (e) {
      setShareError(String(e));
    } finally {
      setSharingLink(false);
    }
  };

  const handleUploadToRelay = async (inviteId: string) => {
    setUploadingRelayFor(inviteId);
    setError(null);
    try {
      const hasRelay = await invoke<boolean>('has_relay_credentials');
      if (!hasRelay) {
        setError(t('workspacePeers.noRelayAccounts'));
        return;
      }
      const url = await invoke<string>('create_relay_invite', {
        identityUuid,
        inviteId,
      });
      try { await navigator.clipboard.writeText(url); } catch { /* WKWebView fallback */ }
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setUploadingRelayFor(null);
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
      const peer = await invoke<PendingPeer>('fetch_relay_invite_response', {
        identityUuid,
        token,
      });
      setPendingPeer(peer);
      setShowAccept(true);
      setResponseUrl('');
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
          {shareSuccess && (
            shareSuccess.startsWith('http') ? (
              <div className="mb-3">
                <p className="text-green-500 text-sm mb-1">{t('invite.linkCopied')}</p>
                <input readOnly value={shareSuccess} className="w-full text-xs font-mono p-1 rounded border border-[var(--color-border)] bg-[var(--color-background)] select-all" onClick={e => (e.target as HTMLInputElement).select()} />
              </div>
            ) : (
              <p className="text-green-500 text-sm mb-3">{shareSuccess}</p>
            )
          )}
          {shareError && <p className="text-red-500 text-sm mb-3">{shareError}</p>}

          <div className="flex gap-2 mb-4">
            <button
              onClick={() => setShowCreate(true)}
              className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white"
            >
              {t('invite.createInvite')}
            </button>
            <button
              onClick={handleShareInviteLink}
              disabled={sharingLink}
              className="px-3 py-1.5 text-sm rounded border dark:border-zinc-700 disabled:opacity-50"
            >
              {sharingLink ? t('invite.sharing') : t('invite.shareInviteLink')}
            </button>
            <button
              onClick={handleImportResponse}
              className="px-3 py-1.5 text-sm rounded border dark:border-zinc-700"
            >
              {t('invite.importResponse')}
            </button>
            {hasRevoked && (
              <button
                onClick={handlePurgeRevoked}
                className="px-3 py-1.5 text-sm rounded border border-red-300 text-red-500 hover:bg-red-50 dark:border-red-800 dark:hover:bg-red-950"
              >
                {t('invite.purgeRevoked', 'Purge Revoked')}
              </button>
            )}
          </div>

          {/* Import Response from Link */}
          <div className="mb-4 p-3 border rounded dark:border-zinc-700">
            <p className="text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
              {t('invite.importResponseFromLink')}
            </p>
            <div className="flex gap-2">
              <input
                type="url"
                value={responseUrl}
                onChange={e => setResponseUrl(e.target.value)}
                placeholder={t('invite.pasteResponseUrl')}
                className="flex-1 border border-zinc-300 dark:border-zinc-600 rounded px-3 py-1.5 text-sm bg-white dark:bg-zinc-800"
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
                    <div className="flex-1 min-w-0">
                      <p className="text-sm">{formatExpiry(invite)}</p>
                      <p className="text-xs text-zinc-500">
                        {t('invite.usedCount', { count: invite.useCount })}
                        {invite.revoked && (
                          <span className="ml-2 text-red-500">{t('invite.revoked')}</span>
                        )}
                      </p>
                      {invite.relayUrl && (
                        <p className="text-xs text-zinc-400 font-mono truncate mt-0.5" title={invite.relayUrl}>
                          {invite.relayUrl}
                        </p>
                      )}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {invite.relayUrl && (
                        <button
                          onClick={() => navigator.clipboard.writeText(invite.relayUrl!)}
                          className="text-xs px-2 py-1 rounded border dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                        >
                          {t('invite.copyLink')}
                        </button>
                      )}
                      {!invite.relayUrl && !invite.revoked && (
                        <button
                          onClick={() => handleUploadToRelay(invite.inviteId)}
                          disabled={uploadingRelayFor === invite.inviteId}
                          className="text-xs px-2 py-1 rounded border dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 disabled:opacity-50"
                        >
                          {uploadingRelayFor === invite.inviteId ? t('common.loading') : t('invite.uploadToRelay')}
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
          onAccepted={(contact: ContactInfo) => {
            load();
            const peerName = contact.localName || contact.declaredName;
            setPostAcceptPeer({ name: peerName, publicKey: contact.publicKey });
          }}
          onClose={() => { setShowAccept(false); setPendingPeer(null); }}
        />
      )}

      <PostAcceptDialog
        open={postAcceptPeer !== null}
        peerName={postAcceptPeer?.name ?? ''}
        onSendNow={() => {
          setSendSnapshotFor([postAcceptPeer!.publicKey]);
          setPostAcceptPeer(null);
          setShowSendSnapshot(true);
        }}
        onLater={() => setPostAcceptPeer(null)}
      />

      <SendSnapshotDialog
        open={showSendSnapshot}
        identityUuid={identityUuid}
        preSelectedPublicKeys={sendSnapshotFor}
        onClose={() => setShowSendSnapshot(false)}
        onSuccess={() => {}}
      />

      {showRelaySetup && (
        <AddRelayAccountDialog
          identityUuid={identityUuid}
          onClose={() => {
            setShowRelaySetup(false);
            setPendingShareAction(false);
          }}
          onCreated={async () => {
            setShowRelaySetup(false);
            if (pendingShareAction) {
              setPendingShareAction(false);
              await doShareInviteLink();
            }
          }}
        />
      )}
    </>
  );
}
