import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';

interface UnlockIdentityDialogProps {
  isOpen: boolean;
  identityUuid: string;
  identityName: string;
  onUnlocked: () => void;
  onCancel: () => void;
}

function UnlockIdentityDialog({ isOpen, identityUuid, identityName, onUnlocked, onCancel }: UnlockIdentityDialogProps) {
  const { t } = useTranslation();
  const [passphrase, setPassphrase] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (isOpen) {
      setPassphrase('');
      setError('');
      setLoading(false);
      // autoFocus via ref to ensure focus after state reset
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    setLoading(true);
    try {
      await invoke('unlock_identity', { identityUuid, passphrase });
      onUnlocked();
    } catch (err) {
      const msg = String(err);
      if (msg === 'WRONG_PASSPHRASE') {
        setError(t('identity.wrongPassphrase'));
        setPassphrase('');
        inputRef.current?.focus();
      } else {
        setError(msg);
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{t('identity.enterPassphrase', { name: identityName })}</h2>

        <form onSubmit={handleSubmit}>
          <div className="mb-4">
            <label className="block text-sm font-medium mb-2">{t('identity.passphrase')}</label>
            <input
              ref={inputRef}
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              autoFocus
              disabled={loading}
            />
          </div>

          {error && (
            <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {error}
            </div>
          )}

          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={onCancel}
              className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
              disabled={loading}
            >
              {t('common.cancel')}
            </button>
            <button
              type="submit"
              className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
              disabled={loading || !passphrase}
            >
              {loading ? t('common.saving') : t('identity.unlock')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default UnlockIdentityDialog;
