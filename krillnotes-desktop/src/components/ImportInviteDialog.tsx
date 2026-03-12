import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteFileData } from '../types';

interface Props {
  identityUuid: string;
  invitePath: string;
  inviteData: InviteFileData;
  onResponded: () => void;
  onClose: () => void;
}

export function ImportInviteDialog({ identityUuid, invitePath, inviteData, onResponded, onClose }: Props) {
  const { t } = useTranslation();
  const [fingerprintConfirmed, setFingerprintConfirmed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isExpired = inviteData.expiresAt
    ? new Date(inviteData.expiresAt) < new Date()
    : false;

  const handleRespond = async () => {
    setLoading(true);
    setError(null);
    try {
      const savePath = await save({
        defaultPath: `response_${inviteData.workspaceName.replace(/\s+/g, '_')}.swarm`,
        filters: [{ name: 'Swarm Response', extensions: ['swarm'] }],
      });
      if (!savePath) { setLoading(false); return; }

      await invoke('respond_to_invite', {
        identityUuid,
        invitePath,
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

        <div className="mb-4 p-4 border rounded dark:border-zinc-700 space-y-1">
          <p className="font-medium">{inviteData.workspaceName}</p>
          {inviteData.workspaceDescription && (
            <p className="text-sm text-zinc-500">{inviteData.workspaceDescription}</p>
          )}
          {inviteData.workspaceAuthorName && (
            <p className="text-xs text-zinc-500">
              {t('invite.by')} {inviteData.workspaceAuthorName}
              {inviteData.workspaceAuthorOrg && ` (${inviteData.workspaceAuthorOrg})`}
            </p>
          )}
          {inviteData.workspaceLicense && (
            <p className="text-xs text-zinc-400">{t('invite.license')}: {inviteData.workspaceLicense}</p>
          )}
          {inviteData.workspaceTags.length > 0 && (
            <div className="flex flex-wrap gap-1 mt-1">
              {inviteData.workspaceTags.map(tag => (
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
          <p className="text-sm font-medium">{inviteData.inviterDeclaredName}</p>
          <p className="text-xs font-mono text-zinc-500 mt-1">{inviteData.inviterFingerprint}</p>
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
            disabled={loading || !fingerprintConfirmed || isExpired}
            className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
          >
            {loading ? t('common.saving') : t('invite.respond')}
          </button>
        </div>
      </div>
    </div>
  );
}
