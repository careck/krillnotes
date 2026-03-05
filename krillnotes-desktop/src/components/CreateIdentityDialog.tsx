import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { IdentityRef } from '../types';

interface CreateIdentityDialogProps {
  isOpen: boolean;
  isFirstLaunch?: boolean;
  onCreated: (identity: IdentityRef) => void;
  onCancel: () => void;
}

function CreateIdentityDialog({ isOpen, isFirstLaunch = false, onCreated, onCancel }: CreateIdentityDialogProps) {
  const { t } = useTranslation();
  const [displayName, setDisplayName] = useState('');
  const [passphrase, setPassphrase] = useState('');
  const [confirmPassphrase, setConfirmPassphrase] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (isOpen) {
      setDisplayName('');
      setPassphrase('');
      setConfirmPassphrase('');
      setError('');
      setLoading(false);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    if (!displayName.trim()) {
      setError(t('identity.nameRequired'));
      return;
    }
    if (!passphrase) {
      setError(t('identity.passphraseRequired'));
      return;
    }
    if (passphrase !== confirmPassphrase) {
      setError(t('identity.passphraseMismatch'));
      return;
    }

    setLoading(true);
    try {
      const identity = await invoke<IdentityRef>('create_identity', {
        displayName: displayName.trim(),
        passphrase,
      });
      onCreated(identity);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        {isFirstLaunch ? (
          <div className="mb-6">
            <h2 className="text-xl font-bold mb-2">{t('identity.createFirst')}</h2>
            <p className="text-sm text-muted-foreground">{t('identity.createFirstDescription')}</p>
          </div>
        ) : (
          <h2 className="text-xl font-bold mb-4">{t('identity.create')}</h2>
        )}

        <form onSubmit={handleSubmit}>
          <div className="mb-4">
            <label className="block text-sm font-medium mb-2">{t('identity.displayName')}</label>
            <input
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              autoFocus
              disabled={loading}
            />
          </div>

          <div className="mb-4">
            <label className="block text-sm font-medium mb-2">{t('identity.passphrase')}</label>
            <input
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              disabled={loading}
            />
          </div>

          <div className="mb-4">
            <label className="block text-sm font-medium mb-2">{t('identity.confirmPassphrase')}</label>
            <input
              type="password"
              value={confirmPassphrase}
              onChange={(e) => setConfirmPassphrase(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              disabled={loading}
            />
          </div>

          {error && (
            <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {error}
            </div>
          )}

          <div className="flex justify-end gap-2">
            {!isFirstLaunch && (
              <button
                type="button"
                onClick={onCancel}
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                disabled={loading}
              >
                {t('common.cancel')}
              </button>
            )}
            <button
              type="submit"
              className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
              disabled={loading}
            >
              {loading ? t('common.creating') : t('common.create')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default CreateIdentityDialog;
