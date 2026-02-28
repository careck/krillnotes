import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';

interface SetPasswordDialogProps {
  isOpen: boolean;
  title?: string;
  onConfirm: (password: string) => void;
  onCancel: () => void;
}

function SetPasswordDialog({ isOpen, title, onConfirm, onCancel }: SetPasswordDialogProps) {
  const { t } = useTranslation();
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    if (isOpen) {
      setPassword('');
      setConfirm('');
      setError('');
    }
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
    if (!password) {
      setError(t('dialogs.password.required'));
      return;
    }
    if (password !== confirm) {
      setError(t('dialogs.password.mismatch'));
      return;
    }
    onConfirm(password);
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleConfirm();
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{title ?? t('dialogs.password.setTitle')}</h2>

        <div className="mb-3">
          <label className="block text-sm font-medium mb-2">{t('dialogs.password.passwordLabel')}</label>
          <input
            type="password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            onKeyDown={handleKeyPress}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder={t('dialogs.password.passwordPlaceholder')}
          />
        </div>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">{t('dialogs.password.confirmLabel')}</label>
          <input
            type="password"
            value={confirm}
            onChange={e => setConfirm(e.target.value)}
            onKeyDown={handleKeyPress}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder={t('dialogs.password.repeatPlaceholder')}
          />
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="px-4 py-2 border border-secondary rounded hover:bg-secondary">
            {t('common.cancel')}
          </button>
          <button
            onClick={handleConfirm}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
          >
            {t('common.confirm')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default SetPasswordDialog;
