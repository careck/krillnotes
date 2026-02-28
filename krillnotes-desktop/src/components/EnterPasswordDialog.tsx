import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';

interface EnterPasswordDialogProps {
  isOpen: boolean;
  workspaceName: string;
  error?: string;
  onConfirm: (password: string) => void;
  onCancel: () => void;
}

function EnterPasswordDialog({ isOpen, workspaceName, error: externalError, onConfirm, onCancel }: EnterPasswordDialogProps) {
  const { t } = useTranslation();
  const [password, setPassword] = useState('');

  useEffect(() => {
    if (isOpen) setPassword('');
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onCancel]);

  if (!isOpen) return null;

  const handleConfirm = () => {
    if (password) onConfirm(password);
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-1">{t('dialogs.password.enterTitle')}</h2>
        <p className="text-sm text-muted-foreground mb-4">"{workspaceName}"</p>

        <div className="mb-4">
          <input
            type="password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleConfirm()}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder={t('dialogs.password.workspacePasswordPlaceholder')}
          />
        </div>

        {externalError === 'WRONG_PASSWORD' && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {t('dialogs.password.wrongPassword')}
          </div>
        )}

        {externalError === 'UNENCRYPTED_WORKSPACE' && (
          <div className="mb-4 p-3 bg-amber-500/10 border border-amber-500/20 text-amber-600 rounded text-sm">
            {t('dialogs.password.legacyWorkspace')}
          </div>
        )}

        {externalError && externalError !== 'WRONG_PASSWORD' && externalError !== 'UNENCRYPTED_WORKSPACE' && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {externalError}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="px-4 py-2 border border-secondary rounded hover:bg-secondary">
            {t('common.cancel')}
          </button>
          <button
            onClick={handleConfirm}
            disabled={!password}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50"
          >
            {t('common.open')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default EnterPasswordDialog;
