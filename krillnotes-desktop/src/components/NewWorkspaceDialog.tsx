import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { AppSettings, WorkspaceInfo } from '../types';
import SetPasswordDialog from './SetPasswordDialog';

function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

interface NewWorkspaceDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function NewWorkspaceDialog({ isOpen, onClose }: NewWorkspaceDialogProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState<'name' | 'password'>('name');
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);
  const [workspaceDir, setWorkspaceDir] = useState('');

  useEffect(() => {
    if (isOpen) {
      setStep('name');
      setName('');
      setError('');
      setCreating(false);
      invoke<AppSettings>('get_settings')
        .then(s => setWorkspaceDir(s.workspaceDirectory))
        .catch(err => setError(`Failed to load settings: ${err}`));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !creating && step === 'name') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, creating, step]);

  if (!isOpen) return null;

  const handleNameNext = () => {
    const trimmed = name.trim();
    if (!trimmed) { setError(t('workspace.nameRequired')); return; }
    const slug = slugify(trimmed);
    if (!slug) { setError(t('workspace.nameInvalid')); return; }
    setError('');
    setStep('password');
  };

  const handlePasswordConfirm = async (password: string) => {
    const slug = slugify(name.trim());
    const path = `${workspaceDir}/${slug}.db`;
    setCreating(true);
    try {
      await invoke<WorkspaceInfo>('create_workspace', { path, password });
      onClose();
    } catch (err) {
      if (err !== 'focused_existing') {
        setError(`${err}`);
        setStep('name');
      }
      setCreating(false);
    }
  };

  if (step === 'password') {
    return (
      <SetPasswordDialog
        isOpen={true}
        title="Set Workspace Password"
        onConfirm={handlePasswordConfirm}
        onCancel={() => setStep('name')}
      />
    );
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{t('workspace.newTitle')}</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">{t('workspace.nameLabel')}</label>
          <input
            type="text"
            value={name}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && !creating && handleNameNext()}
            placeholder={t('workspace.namePlaceholder')}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            disabled={creating}
          />
          {workspaceDir && (
            <p className="text-xs text-muted-foreground mt-1">
              {t('workspace.savedTo', { path: `${workspaceDir}/${slugify(name.trim()) || '...'}.db` })}
            </p>
          )}
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="px-4 py-2 border border-secondary rounded hover:bg-secondary" disabled={creating}>
            {t('common.cancel')}
          </button>
          <button
            onClick={handleNameNext}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={creating || !name.trim()}
          >
            {t('common.next')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default NewWorkspaceDialog;
