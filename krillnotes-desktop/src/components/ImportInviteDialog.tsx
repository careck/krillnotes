import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteFileData, IdentityRef, FetchedRelayInvite } from '../types';
import AddRelayAccountDialog from './AddRelayAccountDialog';

interface Props {
  initialIdentityUuid?: string;
  invitePath?: string;
  inviteData?: InviteFileData;
  onResponded: () => void;
  onClose: () => void;
}

export function ImportInviteDialog({ initialIdentityUuid, invitePath, inviteData, onResponded, onClose }: Props) {
  const { t } = useTranslation();
  const [fingerprintConfirmed, setFingerprintConfirmed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [unlockedIdentities, setUnlockedIdentities] = useState<IdentityRef[]>([]);
  const [selectedUuid, setSelectedUuid] = useState(initialIdentityUuid ?? '');

  // Relay URL import state
  const [relayUrl, setRelayUrl] = useState('');
  const [fetchingRelay, setFetchingRelay] = useState(false);
  const [relayInviteData, setRelayInviteData] = useState<InviteFileData | null>(null);
  const [relayInvitePath, setRelayInvitePath] = useState<string | null>(null);

  // File-based import state (when opened from menu without pre-loaded data)
  const [fileInviteData, setFileInviteData] = useState<InviteFileData | null>(null);
  const [fileInvitePath, setFileInvitePath] = useState<string | null>(null);

  // Relay response state
  const [sendingViaRelay, setSendingViaRelay] = useState(false);
  const [responseRelayUrl, setResponseRelayUrl] = useState<string | null>(null);
  const [responseShared, setResponseShared] = useState(false);
  const [showRelaySetup, setShowRelaySetup] = useState(false);
  const [pendingRelayRespond, setPendingRelayRespond] = useState(false);

  // Priority: relay-fetched > file-picked > prop-provided
  const effectiveInviteData = relayInviteData ?? fileInviteData ?? inviteData ?? null;
  const isStandalone = !invitePath && !inviteData;

  useEffect(() => {
    Promise.all([
      invoke<IdentityRef[]>('list_identities'),
      invoke<string[]>('get_unlocked_identities'),
    ]).then(([all, unlockedUuids]) => {
      const unlocked = all.filter(id => unlockedUuids.includes(id.uuid));
      setUnlockedIdentities(unlocked);
      if (unlocked.length > 0) {
        const hint = unlocked.find(id => id.uuid === initialIdentityUuid);
        setSelectedUuid(hint ? hint.uuid : unlocked[0].uuid);
      }
    }).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const isExpired = effectiveInviteData?.expiresAt
    ? new Date(effectiveInviteData.expiresAt) < new Date()
    : false;

  const extractRelayToken = (url: string): string | null => {
    try {
      const parsed = new URL(url.trim());
      const segments = parsed.pathname.split('/').filter(Boolean);
      return segments[segments.length - 1] ?? null;
    } catch {
      return null;
    }
  };

  const handleFetchRelay = useCallback(async () => {
    const token = extractRelayToken(relayUrl);
    if (!token) {
      setError(t('invite.invalidRelayUrl'));
      return;
    }
    setFetchingRelay(true);
    setError(null);
    try {
      const result = await invoke<FetchedRelayInvite>('fetch_relay_invite', { token });
      setRelayInviteData(result.invite);
      setRelayInvitePath(result.tempPath);
    } catch (e) {
      setError(String(e));
    } finally {
      setFetchingRelay(false);
    }
  }, [relayUrl, t]);

  const handleOpenFile = useCallback(async () => {
    try {
      const picked = await open({
        filters: [{ name: 'Swarm Invite', extensions: ['swarm'] }],
        multiple: false,
        title: t('invite.openSwarmFile', 'Open .swarm file'),
      });
      if (!picked || Array.isArray(picked)) return;
      setLoading(true);
      setError(null);
      const data = await invoke<InviteFileData>('import_invite', { path: picked });
      setFileInviteData(data);
      setFileInvitePath(picked as string);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [t]);

  // Plain functions (not useCallback) to always read fresh state via effectiveInvitePath
  const doSendViaRelay = async () => {
    const path = relayInvitePath ?? fileInvitePath ?? invitePath;
    if (!path) {
      setError('No invite file path available');
      return;
    }
    setSendingViaRelay(true);
    setError(null);
    try {
      const url = await invoke<string>('send_invite_response_via_relay', {
        identityUuid: selectedUuid,
        tempPath: path,
        expiresInDays: 7,
      });
      try { await navigator.clipboard.writeText(url); } catch { /* WKWebView fallback */ }
      setResponseRelayUrl(url);
      setResponseShared(true);
      // Don't call onResponded() here — let user see + copy the URL first
      try {
        await invoke("save_accepted_invite", {
          identityUuid: selectedUuid,
          inviteId: effectiveInviteData!.inviteId,
          workspaceId: effectiveInviteData!.workspaceId,
          workspaceName: effectiveInviteData!.workspaceName,
          inviterPublicKey: effectiveInviteData!.inviterPublicKey,
          inviterDeclaredName: effectiveInviteData!.inviterDeclaredName,
          responseRelayUrl: url,
        });
      } catch (e) {
        console.warn("Failed to save accepted invite:", e);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setSendingViaRelay(false);
    }
  };

  const handleSendViaRelay = async () => {
    if (!selectedUuid) {
      setError(t('swarm.identityLocked'));
      return;
    }
    setSendingViaRelay(true);
    setError(null);
    try {
      const hasRelay = await invoke<boolean>('has_relay_credentials', { identityUuid: selectedUuid });
      if (!hasRelay) {
        setPendingRelayRespond(true);
        setShowRelaySetup(true);
        setSendingViaRelay(false);
        return;
      }
      await doSendViaRelay();
    } catch (e) {
      setError(String(e));
      setSendingViaRelay(false);
    }
  };

  const handleRespond = async () => {
    const data = relayInviteData ?? fileInviteData ?? inviteData;
    const path = relayInvitePath ?? fileInvitePath ?? invitePath;
    if (!selectedUuid || !data || !path) {
      setError(t('swarm.identityLocked'));
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const savePath = await save({
        defaultPath: `response_${data.workspaceName.replace(/\s+/g, '_')}.swarm`,
        filters: [{ name: 'Swarm Response', extensions: ['swarm'] }],
      });
      if (!savePath) { setLoading(false); return; }

      await invoke('respond_to_invite', {
        identityUuid: selectedUuid,
        invitePath: path,
        savePath,
      });
      try {
        await invoke("save_accepted_invite", {
          identityUuid: selectedUuid,
          inviteId: data.inviteId,
          workspaceId: data.workspaceId,
          workspaceName: data.workspaceName,
          inviterPublicKey: data.inviterPublicKey,
          inviterDeclaredName: data.inviterDeclaredName,
          responseRelayUrl: null,
        });
      } catch (e) {
        console.warn("Failed to save accepted invite:", e);
      }
      onResponded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  // Success state: response was shared via relay
  if (responseShared && responseRelayUrl) {
    return (
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg shadow-xl p-6 w-full max-w-lg">
          <h2 className="text-lg font-semibold mb-2">{t('invite.respond')}</h2>
          <p className="text-sm text-green-600 mb-3">
            {t('invite.responseShared')}
          </p>
          <p className="text-xs font-mono text-[var(--color-muted-foreground)] break-all mb-2 p-2 bg-[var(--color-secondary)]/50 rounded">
            {responseRelayUrl}
          </p>
          <p className="text-xs text-[var(--color-muted-foreground)] mb-4">
            {t('invite.shareResponseUrlWithInviter')}
          </p>
          <div className="flex justify-end gap-2">
            <button
              onClick={() => { try { navigator.clipboard.writeText(responseRelayUrl); } catch { /* fallback: URL is visible above */ } }}
              className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
            >
              {t('invite.copyLink')}
            </button>
            <button onClick={onClose} className="px-4 py-2 text-sm rounded bg-blue-600 text-white">
              {t('common.close')}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <>
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg shadow-xl p-6 w-full max-w-lg">
          <h2 className="text-lg font-semibold mb-1">{t('invite.acceptInvite', 'Accept Invite')}</h2>
          <p className="text-sm text-[var(--color-muted-foreground)] mb-4">
            {effectiveInviteData
              ? t('invite.importSubtitle')
              : t('invite.acceptSubtitle', 'Paste a relay invite link or open an invite file.')}
          </p>

          {/* Relay URL input — always shown until invite is loaded */}
          {!effectiveInviteData && (
            <div className="mb-4 p-3 border border-[var(--color-border)] rounded-md bg-[var(--color-secondary)]/30">
              <label className="block text-xs font-medium text-[var(--color-muted-foreground)] mb-1">
                {t('invite.relayUrlLabel', 'Paste a relay invite URL')}
              </label>
              <div className="flex gap-2">
                <input
                  type="url"
                  value={relayUrl}
                  onChange={e => setRelayUrl(e.target.value)}
                  placeholder="https://swarm.krillnotes.org/invites/..."
                  className="flex-1 border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                  disabled={fetchingRelay || loading}
                />
                <button
                  onClick={handleFetchRelay}
                  disabled={!relayUrl.trim() || fetchingRelay || loading}
                  className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
                >
                  {fetchingRelay ? t('common.loading', 'Loading…') : t('invite.fetchRelay', 'Fetch')}
                </button>
              </div>

              <div className="flex items-center gap-3 my-3">
                <div className="flex-1 border-t border-[var(--color-border)]" />
                <span className="text-xs text-[var(--color-muted-foreground)]">{t('common.or', 'or')}</span>
                <div className="flex-1 border-t border-[var(--color-border)]" />
              </div>

              <button
                onClick={handleOpenFile}
                disabled={loading}
                className="w-full px-3 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)] disabled:opacity-50"
              >
                {t('invite.openSwarmFile', 'Open .swarm Invite File…')}
              </button>
            </div>
          )}

          {/* Once fetched via relay, show a small "change" link */}
          {effectiveInviteData && isStandalone && (
            <div className="mb-3 p-3 border border-[var(--color-border)] rounded-md bg-[var(--color-secondary)]/30">
              <div className="flex gap-2">
                <input
                  type="url"
                  value={relayUrl}
                  onChange={e => setRelayUrl(e.target.value)}
                  placeholder="https://swarm.krillnotes.org/invites/..."
                  className="flex-1 border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                  disabled={fetchingRelay || loading}
                />
                <button
                  onClick={handleFetchRelay}
                  disabled={!relayUrl.trim() || fetchingRelay || loading}
                  className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
                >
                  {fetchingRelay ? t('common.loading', 'Loading…') : t('invite.fetchRelay', 'Fetch')}
                </button>
              </div>
            </div>
          )}

          {/* Invite details */}
          {effectiveInviteData && (
            <>
              <div className="mb-4 p-4 border border-[var(--color-border)] rounded-md space-y-1">
                <p className="font-medium">{effectiveInviteData.workspaceName}</p>
                {effectiveInviteData.workspaceDescription && (
                  <p className="text-sm text-[var(--color-muted-foreground)]">{effectiveInviteData.workspaceDescription}</p>
                )}
                {effectiveInviteData.workspaceAuthorName && (
                  <p className="text-xs text-[var(--color-muted-foreground)]">
                    {t('invite.by')} {effectiveInviteData.workspaceAuthorName}
                    {effectiveInviteData.workspaceAuthorOrg && ` (${effectiveInviteData.workspaceAuthorOrg})`}
                  </p>
                )}
                {effectiveInviteData.workspaceLicense && (
                  <p className="text-xs text-[var(--color-muted-foreground)]">{t('invite.license')}: {effectiveInviteData.workspaceLicense}</p>
                )}
                {effectiveInviteData.workspaceTags.length > 0 && (
                  <div className="flex flex-wrap gap-1 mt-1">
                    {effectiveInviteData.workspaceTags.map(tag => (
                      <span key={tag} className="text-xs bg-[var(--color-secondary)] px-2 py-0.5 rounded-full">
                        {tag}
                      </span>
                    ))}
                  </div>
                )}
              </div>

              <div className="mb-4 p-3 bg-[var(--color-secondary)]/50 rounded-md">
                <p className="text-xs font-medium text-[var(--color-muted-foreground)] mb-1">
                  {t('invite.invitedBy')}
                </p>
                <p className="text-sm font-medium">{effectiveInviteData.inviterDeclaredName}</p>
                <p className="text-xs font-mono text-[var(--color-muted-foreground)] mt-1">{effectiveInviteData.inviterFingerprint}</p>
              </div>

              <div className="mb-4">
                <label className="block text-xs font-medium text-[var(--color-muted-foreground)] mb-1">
                  {t('invite.respondAs', 'Respond as')}
                </label>
                {unlockedIdentities.length === 0 ? (
                  <p className="text-sm text-amber-600">
                    {t('swarm.identityLocked')}
                  </p>
                ) : (
                  <select
                    value={selectedUuid}
                    onChange={e => setSelectedUuid(e.target.value)}
                    className="w-full border border-[var(--color-border)] rounded px-3 py-2 bg-[var(--color-background)] text-sm"
                    disabled={loading || sendingViaRelay}
                  >
                    {unlockedIdentities.map(id => (
                      <option key={id.uuid} value={id.uuid}>{id.displayName}</option>
                    ))}
                  </select>
                )}
              </div>

              <p className="text-sm text-amber-600 mb-3">
                {t('invite.fingerprintVerifyPrompt')}
              </p>

              <label className="flex items-center gap-2 mb-4 text-sm cursor-pointer">
                <input
                  type="checkbox"
                  checked={fingerprintConfirmed}
                  onChange={e => setFingerprintConfirmed(e.target.checked)}
                />
                {t('invite.fingerprintConfirm')}
              </label>

              {isExpired && (
                <p className="text-red-500 text-sm mb-3">{t('invite.expired')}</p>
              )}
            </>
          )}

          {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

          <div className="flex justify-end gap-2">
            <button onClick={onClose} className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]">
              {t('common.cancel')}
            </button>
            {effectiveInviteData && (
              <>
                {/* Send via Relay — primary option */}
                <button
                  onClick={handleSendViaRelay}
                  disabled={sendingViaRelay || loading || !fingerprintConfirmed || isExpired || !selectedUuid}
                  className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
                >
                  {sendingViaRelay ? t('common.saving') : t('invite.sendViaRelay')}
                </button>
                {/* Save Response File — secondary option */}
                <button
                  onClick={handleRespond}
                  disabled={loading || sendingViaRelay || !fingerprintConfirmed || isExpired || !selectedUuid}
                  className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)] disabled:opacity-50"
                >
                  {loading ? t('common.saving') : t('invite.saveResponseFile')}
                </button>
              </>
            )}
          </div>
        </div>
      </div>

      {showRelaySetup && (
        <AddRelayAccountDialog
          identityUuid={selectedUuid}
          onClose={() => {
            setShowRelaySetup(false);
            setPendingRelayRespond(false);
          }}
          onCreated={async () => {
            setShowRelaySetup(false);
            if (pendingRelayRespond) {
              setPendingRelayRespond(false);
              await doSendViaRelay();
            }
          }}
        />
      )}
    </>
  );
}
