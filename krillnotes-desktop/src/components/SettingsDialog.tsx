import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { AppSettings } from '../types';
import { useTheme } from '../contexts/ThemeContext';
import ManageThemesDialog from './ManageThemesDialog';
import i18n from '../i18n';
import { useTranslation } from 'react-i18next';

interface SettingsDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function SettingsDialog({ isOpen, onClose }: SettingsDialogProps) {
  const { t } = useTranslation();
  const [workspaceDir, setWorkspaceDir] = useState('');
  const [cachePasswords, setCachePasswords] = useState(false);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [language, setLanguage] = useState(() => i18n.language ?? 'en');
  const [originalLanguage, setOriginalLanguage] = useState(() => i18n.language ?? 'en');
  const { activeMode, lightThemeName, darkThemeName, themes, setMode, setLightTheme, setDarkTheme } = useTheme();
  const [manageThemesOpen, setManageThemesOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<'general' | 'sync'>('general');

  useEffect(() => {
    if (isOpen) {
      invoke<AppSettings>('get_settings')
        .then(s => {
          setWorkspaceDir(s.workspaceDirectory);
          setCachePasswords(s.cacheWorkspacePasswords);
          setLanguage(s.language ?? 'en');
          setOriginalLanguage(s.language ?? 'en');
          setError('');
        })
        .catch(err => setError(t('settings.failedLoad', { error: String(err) })));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        i18n.changeLanguage(originalLanguage);
        onClose();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, originalLanguage]);

  const handleClose = () => {
    i18n.changeLanguage(originalLanguage); // revert preview
    onClose();
  };

  const handleLanguageChange = (lang: string) => {
    setLanguage(lang);
    i18n.changeLanguage(lang); // live preview — UI updates immediately
  };

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
          language,
        },
      });
      setOriginalLanguage(language); // committed — no revert on close
      onClose();
    } catch (err) {
      setError(t('settings.failedSave', { error: String(err) }));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-[500px]">
        <h2 className="text-xl font-bold mb-4">{t('settings.title')}</h2>

        {/* Tab bar */}
        <div className="flex border-b border-border mb-4">
          <button
            onClick={() => setActiveTab('general')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'general'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabGeneral')}
          </button>
          <button
            onClick={() => setActiveTab('sync')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'sync'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabSync')}
          </button>
        </div>

        {activeTab === 'general' && (
          <>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">
                {t('settings.workspaceDir')}
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
                  {t('common.browse')}
                </button>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                {t('settings.workspaceDirHint')}
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
                  <span className="block text-sm font-medium">{t('settings.rememberPasswords')}</span>
                  <span className="block text-xs text-muted-foreground mt-0.5">
                    {t('settings.rememberPasswordsHint')}
                  </span>
                </div>
              </label>
            </div>

            {/* Appearance */}
            <div className="border-t border-border pt-4 mt-4">
              <h3 className="text-sm font-semibold text-foreground mb-3">{t('settings.appearance')}</h3>

              {/* Language picker */}
              <div className="flex items-center gap-2 mb-3">
                <span className="text-sm text-muted-foreground w-24">{t('settings.language')}</span>
                <select
                  value={language}
                  onChange={e => handleLanguageChange(e.target.value)}
                  className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
                >
                  <option value="en">English</option>
                  <option value="de">Deutsch (de)</option>
                  <option value="fr">Français (fr)</option>
                  <option value="es">Español (es)</option>
                  <option value="ja">日本語 (ja)</option>
                  <option value="ko">한국어 (ko)</option>
                  <option value="zh">中文 (zh)</option>
                </select>
              </div>

              {/* Mode toggle */}
              <div className="flex items-center gap-2 mb-3">
                <span className="text-sm text-muted-foreground w-24">{t('settings.mode')}</span>
                <div className="flex rounded border border-border overflow-hidden">
                  {(['light', 'dark', 'system'] as const).map(m => (
                    <button
                      key={m}
                      onClick={() => setMode(m)}
                      className={`px-3 py-1 text-sm ${
                        activeMode === m
                          ? 'bg-primary text-primary-foreground'
                          : 'text-muted-foreground hover:text-foreground hover:bg-secondary'
                      }`}
                    >
                      {t(`settings.mode${m.charAt(0).toUpperCase() + m.slice(1)}`)}
                    </button>
                  ))}
                </div>
              </div>

              {/* Light theme picker */}
              <div className="flex items-center gap-2 mb-2">
                <span className="text-sm text-muted-foreground w-24">{t('settings.lightTheme')}</span>
                <select
                  value={lightThemeName}
                  onChange={e => setLightTheme(e.target.value)}
                  className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
                >
                  <option value="light">{t('settings.lightBuiltIn')}</option>
                  {themes.filter(theme => theme.hasLight).map(theme => (
                    <option key={theme.filename} value={theme.name}>{theme.name}</option>
                  ))}
                </select>
              </div>

              {/* Dark theme picker */}
              <div className="flex items-center gap-2 mb-3">
                <span className="text-sm text-muted-foreground w-24">{t('settings.darkTheme')}</span>
                <select
                  value={darkThemeName}
                  onChange={e => setDarkTheme(e.target.value)}
                  className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
                >
                  <option value="dark">{t('settings.darkBuiltIn')}</option>
                  {themes.filter(theme => theme.hasDark).map(theme => (
                    <option key={theme.filename} value={theme.name}>{theme.name}</option>
                  ))}
                </select>
              </div>

              <button
                onClick={() => setManageThemesOpen(true)}
                className="text-sm text-muted-foreground hover:text-foreground underline"
              >
                {t('settings.manageThemes')}
              </button>
            </div>
          </>
        )}

        {activeTab === 'sync' && (
          <div className="py-2">
            <label className="flex items-center gap-3 opacity-50 cursor-not-allowed">
              <input
                type="checkbox"
                checked={false}
                disabled
                className="w-4 h-4"
              />
              <div>
                <span className="block text-sm font-medium">{t('settings.sync')}</span>
                <span className="block text-xs text-muted-foreground mt-0.5">
                  {t('settings.syncHint')}
                </span>
              </div>
            </label>
          </div>
        )}

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2 mt-4">
          <button
            onClick={handleClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={saving}
          >
            {t('common.cancel')}
          </button>
          <button
            onClick={handleSave}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={saving}
          >
            {saving ? t('common.saving') : t('common.save')}
          </button>
        </div>
      </div>
      <ManageThemesDialog isOpen={manageThemesOpen} onClose={() => setManageThemesOpen(false)} />
    </div>
  );
}

export default SettingsDialog;
