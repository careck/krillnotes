import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { AppSettings } from '../types';
import { useTheme } from '../contexts/ThemeContext';
import ManageThemesDialog from './ManageThemesDialog';

interface SettingsDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function SettingsDialog({ isOpen, onClose }: SettingsDialogProps) {
  const [workspaceDir, setWorkspaceDir] = useState('');
  const [cachePasswords, setCachePasswords] = useState(false);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const { activeMode, lightThemeName, darkThemeName, themes, setMode, setLightTheme, setDarkTheme } = useTheme();
  const [manageThemesOpen, setManageThemesOpen] = useState(false);

  useEffect(() => {
    if (isOpen) {
      invoke<AppSettings>('get_settings')
        .then(s => {
          setWorkspaceDir(s.workspaceDirectory);
          setCachePasswords(s.cacheWorkspacePasswords);
          setError('');
        })
        .catch(err => setError(`Failed to load settings: ${err}`));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleBrowse = async () => {
    const selected = await open({
      directory: true,
      title: 'Choose Workspace Directory',
      defaultPath: workspaceDir,
    });
    if (selected && typeof selected === 'string') {
      setWorkspaceDir(selected);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      await invoke('update_settings', {
        patch: {
          workspaceDirectory: workspaceDir,
          cacheWorkspacePasswords: cachePasswords,
        },
      });
      onClose();
    } catch (err) {
      setError(`Failed to save settings: ${err}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-[500px]">
        <h2 className="text-xl font-bold mb-4">Settings</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">
            Default Workspace Directory
          </label>
          <div className="flex gap-2">
            <input
              type="text"
              value={workspaceDir}
              readOnly
              className="flex-1 bg-secondary border border-secondary rounded px-3 py-2 text-sm"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
            />
            <button
              onClick={handleBrowse}
              className="px-3 py-2 border border-secondary rounded hover:bg-secondary text-sm"
            >
              Browse...
            </button>
          </div>
          <p className="text-xs text-muted-foreground mt-1">
            New workspaces will be created in this directory.
          </p>
        </div>

        <div className="mb-4">
          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={cachePasswords}
              onChange={e => setCachePasswords(e.target.checked)}
              className="w-4 h-4"
            />
            <div>
              <span className="block text-sm font-medium">Remember workspace passwords for this session</span>
              <span className="block text-xs text-muted-foreground mt-0.5">
                Passwords are kept in memory until the app closes. Off by default.
              </span>
            </div>
          </label>
        </div>

        {/* Appearance */}
        <div className="border-t border-border pt-4 mt-4">
          <h3 className="text-sm font-semibold text-foreground mb-3">Appearance</h3>

          {/* Mode toggle */}
          <div className="flex items-center gap-2 mb-3">
            <span className="text-sm text-muted-foreground w-24">Mode</span>
            <div className="flex rounded border border-border overflow-hidden">
              {(['light', 'dark', 'system'] as const).map(m => (
                <button
                  key={m}
                  onClick={() => setMode(m)}
                  className={`px-3 py-1 text-sm capitalize ${
                    activeMode === m
                      ? 'bg-primary text-primary-foreground'
                      : 'text-muted-foreground hover:text-foreground hover:bg-secondary'
                  }`}
                >
                  {m}
                </button>
              ))}
            </div>
          </div>

          {/* Light theme picker */}
          <div className="flex items-center gap-2 mb-2">
            <span className="text-sm text-muted-foreground w-24">Light theme</span>
            <select
              value={lightThemeName}
              onChange={e => setLightTheme(e.target.value)}
              className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
            >
              <option value="light">light (built-in)</option>
              {themes.filter(t => t.hasLight).map(t => (
                <option key={t.filename} value={t.name}>{t.name}</option>
              ))}
            </select>
          </div>

          {/* Dark theme picker */}
          <div className="flex items-center gap-2 mb-3">
            <span className="text-sm text-muted-foreground w-24">Dark theme</span>
            <select
              value={darkThemeName}
              onChange={e => setDarkTheme(e.target.value)}
              className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
            >
              <option value="dark">dark (built-in)</option>
              {themes.filter(t => t.hasDark).map(t => (
                <option key={t.filename} value={t.name}>{t.name}</option>
              ))}
            </select>
          </div>

          <button
            onClick={() => setManageThemesOpen(true)}
            className="text-sm text-muted-foreground hover:text-foreground underline"
          >
            Manage Themes…
          </button>
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2 mt-4">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={saving}
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={saving}
          >
            {saving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
      <ManageThemesDialog isOpen={manageThemesOpen} onClose={() => setManageThemesOpen(false)} />
    </div>
  );
}

export default SettingsDialog;
