import { useState, useEffect } from 'react';
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

type Step = 'configure' | 'share';

export function CreateInviteDialog({ identityUuid, workspaceName, onCreated, onClose }: Props) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>('configure');
  const [expiryDays, setExpiryDays] = useState<number | null>(null);
  const [customDays, setCustomDays] = useState('');
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createdInvite, setCreatedInvite] = useState<InviteInfo | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [hasRelay, setHasRelay] = useState(false);
  const [relayUrl, setRelayUrl] = useState<string | null>(null);
  const [copyingLink, setCopyingLink] = useState(false);
  const [linkCopied, setLinkCopied] = useState(false);
  const [savingFile, setSavingFile] = useState(false);
  const [fileSaved, setFileSaved] = useState(false);

  const effectiveDays =
    expiryDays === -1 ? (parseInt(customDays) || null) : expiryDays;

  // Step 1: ask for save path and generate the invite file
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
      setCreatedInvite(invite);
      setSavedPath(savePath);
      onCreated(invite);
      setStep('share');
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  // Check relay credentials when the share step is shown
  useEffect(() => {
    if (step === 'share') {
      invoke<boolean>('has_relay_credentials').then(setHasRelay).catch(() => setHasRelay(false));
    }
  }, [step]);

  // Step 2: upload invite to relay and copy URL to clipboard
  const handleCopyLink = async () => {
    if (!createdInvite) return;
    setCopyingLink(true);
    setError(null);
    try {
      const url = await invoke<string>('create_relay_invite', { token: createdInvite.inviteId });
      setRelayUrl(url);
      await navigator.clipboard.writeText(url);
      setLinkCopied(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setCopyingLink(false);
    }
  };

  // Step 2: save (or re-save) the .swarm file
  const handleSaveFile = async () => {
    if (!createdInvite) return;
    setSavingFile(true);
    setError(null);
    try {
      const defaultPath = savedPath ?? `invite_${workspaceName.replace(/\s+/g, '_')}.swarm`;
      const newSavePath = await save({
        defaultPath,
        filters: [{ name: 'Swarm Invite', extensions: ['swarm'] }],
      });
      if (!newSavePath) { setSavingFile(false); return; }
      await invoke('save_invite_file', { inviteId: createdInvite.inviteId, savePath: newSavePath });
      setSavedPath(newSavePath);
      setFileSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingFile(false);
    }
  };

  // Step 2: do both — copy relay link AND save file concurrently
  const handleBoth = async () => {
    await Promise.allSettled([handleCopyLink(), handleSaveFile()]);
  };

  if (step === 'share') {
    return (
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
          <h2 className="text-lg font-semibold mb-2">{t('invite.shareTitle', 'Share Invite')}</h2>
          <p className="text-sm text-zinc-500 mb-5">
            {t('invite.shareDescription', 'Your invite file has been saved. You can also share it via a relay link.')}
          </p>

          <div className="flex flex-col gap-3 mb-4">
            {hasRelay && (
              <button
                onClick={handleCopyLink}
                disabled={copyingLink || savingFile || linkCopied}
                className="flex items-center justify-center gap-2 px-4 py-2.5 text-sm rounded border dark:border-zinc-600 hover:bg-zinc-50 dark:hover:bg-zinc-800 disabled:opacity-50"
              >
                {linkCopied
                  ? t('invite.linkCopied', 'Link copied!')
                  : copyingLink
                    ? t('common.saving', 'Saving…')
                    : t('invite.copyLink', 'Copy link')}
              </button>
            )}

            <button
              onClick={handleSaveFile}
              disabled={savingFile || copyingLink}
              className="flex items-center justify-center gap-2 px-4 py-2.5 text-sm rounded border dark:border-zinc-600 hover:bg-zinc-50 dark:hover:bg-zinc-800 disabled:opacity-50"
            >
              {fileSaved
                ? t('invite.fileSaved', 'File saved!')
                : savingFile
                  ? t('common.saving', 'Saving…')
                  : t('invite.saveFile', 'Save .swarm file')}
            </button>

            {hasRelay && (
              <button
                onClick={handleBoth}
                disabled={copyingLink || savingFile || (linkCopied && fileSaved)}
                className="flex items-center justify-center gap-2 px-4 py-2.5 text-sm rounded border dark:border-zinc-600 hover:bg-zinc-50 dark:hover:bg-zinc-800 disabled:opacity-50"
              >
                {copyingLink || savingFile
                  ? t('common.saving', 'Saving…')
                  : t('invite.both', 'Copy link & Save file')}
              </button>
            )}
          </div>

          {relayUrl && (
            <p className="text-xs font-mono text-zinc-500 break-all mb-3">{relayUrl}</p>
          )}

          {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

          <div className="flex justify-end">
            <button onClick={onClose} className="px-4 py-2 text-sm rounded bg-blue-600 text-white">
              {t('common.done', 'Done')}
            </button>
          </div>
        </div>
      </div>
    );
  }

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
