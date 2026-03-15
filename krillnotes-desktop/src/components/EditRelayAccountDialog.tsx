import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { RelayAccountInfo } from '../types';

interface Props {
  identityUuid: string;
  account: RelayAccountInfo;
  onClose: () => void;
  onDeleted: () => void;
}

export default function EditRelayAccountDialog({ identityUuid, account, onClose, onDeleted }: Props) {
  const { t } = useTranslation();
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  async function handleDelete() {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    setDeleting(true);
    try {
      await invoke('delete_relay_account', {
        identityUuid,
        relayAccountId: account.relayAccountId,
      });
      onDeleted();
    } catch (e) {
      setError(String(e));
      setDeleting(false);
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
      <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg p-6 w-full max-w-md shadow-xl">
        <h2 className="text-lg font-semibold mb-4">{t('editRelay.title')}</h2>

        <div className="space-y-4">
          {/* Read-only fields */}
          <div className="rounded border border-[var(--color-border)] p-3 space-y-2">
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-[var(--color-muted-foreground)]">
                {t('addRelay.relayUrl')}
              </p>
              <p className="text-sm font-mono truncate">{account.relayUrl}</p>
            </div>
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-[var(--color-muted-foreground)]">
                {t('addRelay.email')}
              </p>
              <p className="text-sm">{account.email}</p>
            </div>
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-[var(--color-muted-foreground)]">
                {t('editRelay.status')}
              </p>
              <span className="flex items-center gap-1.5 text-sm">
                <span
                  className={`inline-block w-2 h-2 rounded-full ${
                    account.sessionValid ? 'bg-green-500' : 'bg-red-500'
                  }`}
                />
                {account.sessionValid ? t('relayBook.sessionValid') : t('relayBook.sessionExpired')}
              </span>
            </div>
          </div>

          {error && <p className="text-sm text-red-500">{error}</p>}
        </div>

        <div className="flex justify-between mt-6">
          <button
            onClick={handleDelete}
            disabled={deleting}
            className={`px-4 py-2 text-sm rounded ${
              confirmDelete
                ? 'bg-red-600 text-white hover:bg-red-700'
                : 'border border-red-400 text-red-500 hover:bg-red-50/10'
            } disabled:opacity-50`}
          >
            {deleting
              ? t('common.deleting')
              : confirmDelete
                ? t('editRelay.confirmDelete')
                : t('editRelay.delete')}
          </button>
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
          >
            {t('common.close')}
          </button>
        </div>
      </div>
    </div>
  );
}
