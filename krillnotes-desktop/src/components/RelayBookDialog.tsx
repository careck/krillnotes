import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { RelayAccountInfo } from '../types';
import AddRelayAccountDialog from './AddRelayAccountDialog';
import EditRelayAccountDialog from './EditRelayAccountDialog';

interface Props {
  identityUuid: string;
  identityName: string;
  onClose: () => void;
}

export default function RelayBookDialog({ identityUuid, identityName, onClose }: Props) {
  const { t } = useTranslation();
  const [accounts, setAccounts] = useState<RelayAccountInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<RelayAccountInfo | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid });
      setAccounts(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [identityUuid]);

  useEffect(() => { load(); }, [load]);

  function handleCreated() {
    setShowAdd(false);
    load();
  }

  function handleDeleted() {
    setEditing(null);
    load();
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-60">
      <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg w-full max-w-lg shadow-xl flex flex-col" style={{ maxHeight: '80vh' }}>
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[var(--color-border)]">
          <div>
            <h2 className="text-lg font-semibold">{t('relayBook.title')}</h2>
            <p className="text-xs text-[var(--color-muted-foreground)]">{identityName}</p>
          </div>
          <button
            onClick={onClose}
            className="text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)] px-2"
          >
            ✕
          </button>
        </div>

        {/* Add button */}
        <div className="flex justify-end p-3 border-b border-[var(--color-border)]">
          <button
            onClick={() => setShowAdd(true)}
            className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white hover:bg-blue-700 whitespace-nowrap"
          >
            {t('relayBook.addRelay')}
          </button>
        </div>

        {/* Account list */}
        <div className="overflow-y-auto flex-1">
          {loading && (
            <p className="text-sm text-center text-[var(--color-muted-foreground)] py-8">{t('common.loading')}</p>
          )}
          {!loading && error && (
            <p className="text-sm text-center text-red-500 py-8">{error}</p>
          )}
          {!loading && !error && accounts.length === 0 && (
            <p className="text-sm text-center text-[var(--color-muted-foreground)] py-8">
              {t('relayBook.noRelays')}
            </p>
          )}
          {accounts.map(account => (
            <button
              key={account.relayAccountId}
              onClick={() => setEditing(account)}
              className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-[var(--color-secondary)] border-b border-[var(--color-border)] last:border-0"
            >
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium truncate">{account.relayUrl}</p>
                <p className="text-xs text-[var(--color-muted-foreground)] truncate">{account.email}</p>
              </div>
              <span className="flex items-center gap-1.5 text-xs">
                <span
                  className={`inline-block w-2 h-2 rounded-full ${
                    account.sessionValid ? 'bg-green-500' : 'bg-red-500'
                  }`}
                />
                {account.sessionValid ? t('relayBook.sessionValid') : t('relayBook.sessionExpired')}
              </span>
            </button>
          ))}
        </div>
      </div>

      {/* Sub-dialogs */}
      {showAdd && (
        <AddRelayAccountDialog
          identityUuid={identityUuid}
          onCreated={handleCreated}
          onClose={() => setShowAdd(false)}
        />
      )}
      {editing && (
        <EditRelayAccountDialog
          key={editing.relayAccountId}
          identityUuid={identityUuid}
          account={editing}
          onDeleted={handleDeleted}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  );
}
