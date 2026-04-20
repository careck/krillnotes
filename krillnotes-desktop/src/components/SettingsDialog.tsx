// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

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
  onSaved?: () => void;
}

function SettingsDialog({ isOpen, onClose, onSaved }: SettingsDialogProps) {
  const { t } = useTranslation();
  const [homeDir, setHomeDir] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [language, setLanguage] = useState(() => i18n.language ?? 'en');
  const [originalLanguage, setOriginalLanguage] = useState(() => i18n.language ?? 'en');
  const { activeMode, lightThemeName, darkThemeName, themes, setMode, setLightTheme, setDarkTheme } = useTheme();
  const [manageThemesOpen, setManageThemesOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<'general' | 'appearance'>('general');
  const [undoLimit, setUndoLimit] = useState<number | undefined>(undefined);
  const [sharingIndicatorMode, setSharingIndicatorMode] = useState<'off' | 'auto' | 'on'>('auto');
  const [syncOnClose, setSyncOnClose] = useState('ask');

  useEffect(() => {
    if (isOpen) {
      invoke<string>('get_home_dir_path').then(setHomeDir).catch(() => {});
      invoke<AppSettings>('get_settings')
        .then(s => {
          setLanguage(s.language ?? 'en');
          setOriginalLanguage(s.language ?? 'en');
          setSharingIndicatorMode((s.sharingIndicatorMode ?? 'auto') as 'off' | 'auto' | 'on');
          setUndoLimit(s.undoHistoryLimit ?? 50);
          setSyncOnClose(s.syncOnClose ?? 'ask');
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

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      await invoke('update_settings', {
        patch: {
          language,
          sharingIndicatorMode,
          undoHistoryLimit: undoLimit ?? 50,
          syncOnClose,
        },
      });
      if (homeDir) {
        await invoke('set_home_dir_path', { path: homeDir });
      }
      setOriginalLanguage(language); // committed — no revert on close
      onSaved?.();
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
            onClick={() => setActiveTab('appearance')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'appearance'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabAppearance')}
          </button>
        </div>

        {activeTab === 'general' && (
          <>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">
                {t('settings.homeFolder')}
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  readOnly
                  value={homeDir}
                  className="flex-1 text-sm bg-background border border-input rounded px-3 py-2 select-all"
                />
                <button
                  onClick={async () => {
                    const selected = await open({ directory: true, defaultPath: homeDir || undefined });
                    if (selected && typeof selected === 'string') setHomeDir(selected);
                  }}
                  className="px-3 py-2 text-sm border border-input rounded hover:bg-secondary"
                >
                  {t('common.browse')}
                </button>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                {t('settings.homeFolderHint')}
              </p>
            </div>

            <div className="flex flex-col gap-1">
              <label className="text-sm font-medium">
                {t('settings.undoHistoryLimit')}
              </label>
              <input
                type="number"
                min={1}
                max={500}
                value={undoLimit ?? 50}
                disabled={undoLimit === undefined}
                onChange={e => {
                  const v = Number(e.target.value);
                  if (!isNaN(v)) setUndoLimit(Math.max(1, Math.min(500, v)));
                }}
                className="bg-background border border-input rounded px-2 py-1 text-sm w-24 disabled:opacity-50"
              />
              <p className="text-xs text-muted-foreground">
                {t('settings.undoHistoryLimitHint')}
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-1">
                {t('settings.syncOnClose')}
              </label>
              <select
                className="w-full px-3 py-2 border border-secondary rounded bg-background text-foreground"
                value={syncOnClose}
                onChange={e => setSyncOnClose(e.target.value)}
              >
                <option value="always">{t('settings.syncOnCloseAlways')}</option>
                <option value="ask">{t('settings.syncOnCloseAsk')}</option>
                <option value="never">{t('settings.syncOnCloseNever')}</option>
              </select>
            </div>

          </>
        )}

        {activeTab === 'appearance' && (
          <div className="py-2">
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

            {/* Sharing indicators toggle */}
            <div className="flex items-center gap-2 mb-3">
              <span className="text-sm text-muted-foreground w-24">{t('settings.sharingIndicators')}</span>
              <div className="flex rounded border border-border overflow-hidden">
                {(['off', 'auto', 'on'] as const).map(m => (
                  <button
                    key={m}
                    onClick={() => setSharingIndicatorMode(m)}
                    className={`px-3 py-1 text-sm ${
                      sharingIndicatorMode === m
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground hover:bg-secondary'
                    }`}
                  >
                    {t(`settings.sharingIndicators${m.charAt(0).toUpperCase() + m.slice(1)}`)}
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
