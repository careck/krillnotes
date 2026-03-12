import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteInfo } from '../types';

interface Props {
  identityUuid: string;
  workspaceName: string;
  onCreated: (invite: InviteInfo) => void;
  onClose: () => void;
}

const EXPIRY_OPTIONS = [
  { label: 'No expiry', value: null },
  { label: '7 days', value: 7 },
  { label: '30 days', value: 30 },
  { label: 'Custom', value: -1 },
];

export function CreateInviteDialog({ identityUuid, workspaceName, onCreated, onClose }: Props) {
  const { t } = useTranslation();
  const [expiryDays, setExpiryDays] = useState<number | null>(null);
  const [customDays, setCustomDays] = useState('');
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const effectiveDays =
    expiryDays === -1 ? (parseInt(customDays) || null) : expiryDays;

  const handleCreate = async () => {
    setCreating(true);
    setError(null);
    try {
      const savePath = await save({
        defaultPath: `invite_${workspaceName.replace(/\s+/g, '_')}.swarm`,
        filters: [{ name: 'Swarm Invite', extensions: ['swarm'] }],
      });
      if (!savePath) { setCreating(false); return; }

      const invite = await invoke<InviteInfo>('create_invite', {
        identityUuid,
        workspaceName,
        expiresInDays: effectiveDays ?? undefined,
        savePath,
      });
      onCreated(invite);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">{t('invite.createTitle')}</h2>

        <p className="text-sm text-zinc-500 mb-4">
          {t('invite.createDescription', { workspaceName })}
        </p>

        <label className="block text-sm font-medium mb-1">{t('invite.expiry')}</label>
        <select
          className="w-full border rounded px-3 py-2 mb-4 dark:bg-zinc-800 dark:border-zinc-700"
          value={expiryDays ?? 'null'}
          onChange={e => setExpiryDays(e.target.value === 'null' ? null : parseInt(e.target.value))}
        >
          {EXPIRY_OPTIONS.map(opt => (
            <option key={String(opt.value)} value={String(opt.value)}>{opt.label}</option>
          ))}
        </select>

        {expiryDays === -1 && (
          <div className="mb-4">
            <label className="block text-sm font-medium mb-1">{t('invite.customDays')}</label>
            <input
              type="number"
              min="1"
              className="w-full border rounded px-3 py-2 dark:bg-zinc-800 dark:border-zinc-700"
              value={customDays}
              onChange={e => setCustomDays(e.target.value)}
            />
          </div>
        )}

        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="px-4 py-2 text-sm rounded border dark:border-zinc-700">
            {t('common.cancel')}
          </button>
          <button
            onClick={handleCreate}
            disabled={creating || (expiryDays === -1 && !parseInt(customDays))}
            className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
          >
            {creating ? t('common.saving') : t('invite.createAndSave')}
          </button>
        </div>
      </div>
    </div>
  );
}
