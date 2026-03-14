import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteFileData, IdentityRef } from '../types';

interface Props {
  initialIdentityUuid?: string;
  invitePath: string;
  inviteData: InviteFileData;
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

  // Use relay-fetched data if available, otherwise fall back to file-based data
  const effectiveInviteData = relayInviteData ?? inviteData;
  const effectiveInvitePath = relayInvitePath ?? invitePath;

  useEffect(() => {
    Promise.all([
      invoke<IdentityRef[]>('list_identities'),
      invoke<string[]>('get_unlocked_identities'),
    ]).then(([all, unlockedUuids]) => {
      const unlocked = all.filter(id => unlockedUuids.includes(id.uuid));
      setUnlockedIdentities(unlocked);
      // Pick initial selection: prefer the hint, fall back to first unlocked.
      if (unlocked.length > 0) {
        const hint = unlocked.find(id => id.uuid === initialIdentityUuid);
        setSelectedUuid(hint ? hint.uuid : unlocked[0].uuid);
      }
    }).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const isExpired = effectiveInviteData.expiresAt
    ? new Date(effectiveInviteData.expiresAt) < new Date()
    : false;

  // Extract the token from a relay invite URL (last path segment)
  const extractRelayToken = (url: string): string | null => {
    try {
      const parsed = new URL(url.trim());
      const segments = parsed.pathname.split('/').filter(Boolean);
      return segments[segments.length - 1] ?? null;
    } catch {
      return null;
    }
  };

  const handleFetchRelay = async () => {
    const token = extractRelayToken(relayUrl);
    if (!token) {
      setError(t('invite.relayInvalidUrl', 'Invalid relay URL'));
      return;
    }
    setFetchingRelay(true);
    setError(null);
    try {
      const bytes = await invoke<number[]>('fetch_relay_invite', { token });
      const parsed = await invoke<InviteFileData>('parse_invite_bytes', { bytes });
      const tempPath = await invoke<string>('write_temp_swarm_bytes', { bytes });
      setRelayInviteData(parsed);
      setRelayInvitePath(tempPath);
    } catch (e) {
      setError(String(e));
    } finally {
      setFetchingRelay(false);
    }
  };

  const handleRespond = async () => {
    if (!selectedUuid) {
      setError(t('swarm.identityLocked'));
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const savePath = await save({
        defaultPath: `response_${effectiveInviteData.workspaceName.replace(/\s+/g, '_')}.swarm`,
        filters: [{ name: 'Swarm Response', extensions: ['swarm'] }],
      });
      if (!savePath) { setLoading(false); return; }

      await invoke('respond_to_invite', {
        identityUuid: selectedUuid,
        invitePath: effectiveInvitePath,
        savePath,
      });
      onResponded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-lg">
        <h2 className="text-lg font-semibold mb-1">{t('invite.importTitle')}</h2>
        <p className="text-sm text-zinc-500 mb-4">{t('invite.importSubtitle')}</p>

        {/* Relay URL import */}
        <div className="mb-4 p-3 border rounded dark:border-zinc-700">
          <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
            {t('invite.relayUrlLabel', 'Or paste a relay invite URL')}
          </label>
          <div className="flex gap-2">
            <input
              type="url"
              value={relayUrl}
              onChange={e => setRelayUrl(e.target.value)}
              placeholder="https://relay.example.com/i/abc123"
              className="flex-1 border border-zinc-300 dark:border-zinc-600 rounded px-3 py-1.5 text-sm bg-white dark:bg-zinc-800"
              disabled={fetchingRelay || loading}
            />
            <button
              onClick={handleFetchRelay}
              disabled={!relayUrl.trim() || fetchingRelay || loading}
              className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
            >
              {fetchingRelay
                ? t('common.loading', 'Loading…')
                : t('invite.fetchRelay', 'Fetch')}
            </button>
          </div>
          {relayInviteData && (
            <p className="text-xs text-green-600 dark:text-green-400 mt-1">
              {t('invite.relayFetched', 'Invite loaded from relay.')}
            </p>
          )}
        </div>

        <div className="mb-4 p-4 border rounded dark:border-zinc-700 space-y-1">
          <p className="font-medium">{effectiveInviteData.workspaceName}</p>
          {effectiveInviteData.workspaceDescription && (
            <p className="text-sm text-zinc-500">{effectiveInviteData.workspaceDescription}</p>
          )}
          {effectiveInviteData.workspaceAuthorName && (
            <p className="text-xs text-zinc-500">
              {t('invite.by')} {effectiveInviteData.workspaceAuthorName}
              {effectiveInviteData.workspaceAuthorOrg && ` (${effectiveInviteData.workspaceAuthorOrg})`}
            </p>
          )}
          {effectiveInviteData.workspaceLicense && (
            <p className="text-xs text-zinc-400">{t('invite.license')}: {effectiveInviteData.workspaceLicense}</p>
          )}
          {effectiveInviteData.workspaceTags.length > 0 && (
            <div className="flex flex-wrap gap-1 mt-1">
              {effectiveInviteData.workspaceTags.map(tag => (
                <span key={tag} className="text-xs bg-zinc-100 dark:bg-zinc-800 px-2 py-0.5 rounded-full">
                  {tag}
                </span>
              ))}
            </div>
          )}
        </div>

        <div className="mb-4 p-3 bg-zinc-100 dark:bg-zinc-800 rounded">
          <p className="text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
            {t('invite.invitedBy')}
          </p>
          <p className="text-sm font-medium">{effectiveInviteData.inviterDeclaredName}</p>
          <p className="text-xs font-mono text-zinc-500 mt-1">{effectiveInviteData.inviterFingerprint}</p>
        </div>

        <div className="mb-4">
          <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
            {t('invite.respondAs', 'Respond as')}
          </label>
          {unlockedIdentities.length === 0 ? (
            <p className="text-sm text-amber-600 dark:text-amber-400">
              {t('swarm.identityLocked')}
            </p>
          ) : (
            <select
              value={selectedUuid}
              onChange={e => setSelectedUuid(e.target.value)}
              className="w-full border border-zinc-300 dark:border-zinc-600 rounded px-3 py-2 bg-white dark:bg-zinc-800 text-sm"
              disabled={loading}
            >
              {unlockedIdentities.map(id => (
                <option key={id.uuid} value={id.uuid}>{id.displayName}</option>
              ))}
            </select>
          )}
        </div>

        <p className="text-sm text-amber-600 dark:text-amber-400 mb-3">
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
        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="px-4 py-2 text-sm rounded border dark:border-zinc-700">
            {t('common.cancel')}
          </button>
          <button
            onClick={handleRespond}
            disabled={loading || !fingerprintConfirmed || isExpired || !selectedUuid}
            className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
          >
            {loading ? t('common.saving') : t('invite.respond')}
          </button>
        </div>
      </div>
    </div>
  );
}
