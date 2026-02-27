// ThemeContext.tsx — React context for the active theme state.

import { createContext, useContext, useEffect, useState, useCallback, useRef, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { themeManager, watchSystem, systemVariant, type ThemeVariant } from '../utils/themeManager';
import type { ThemeMeta } from '../utils/theme';
import type { AppSettings } from '../types';

interface ThemeContextValue {
  activeMode: string;
  lightThemeName: string;
  darkThemeName: string;
  themes: ThemeMeta[];
  setMode: (mode: string) => Promise<void>;
  setLightTheme: (name: string) => Promise<void>;
  setDarkTheme: (name: string) => Promise<void>;
  reloadThemes: () => Promise<void>;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [activeMode, setActiveMode] = useState('system');
  const [lightThemeName, setLightThemeName] = useState('light');
  const [darkThemeName, setDarkThemeName] = useState('dark');
  const [themes, setThemes] = useState<ThemeMeta[]>([]);
  const settingsRef = useRef<AppSettings | null>(null);

  const reloadThemes = useCallback(async () => {
    try {
      const result = await invoke<ThemeMeta[]>('list_themes');
      setThemes(result);
    } catch {
      setThemes([]);
    }
  }, []);

  const applyCurrentTheme = useCallback(
    async (mode: string, lightName: string, darkName: string) => {
      const variant: ThemeVariant =
        mode === 'system' ? systemVariant() : (mode as ThemeVariant);
      const name = variant === 'dark' ? darkName : lightName;
      await themeManager.loadAndApply(name, variant);
    },
    [],
  );

  // Load settings and apply theme on mount; also listen for theme changes
  // broadcast by other windows so all windows stay in sync.
  useEffect(() => {
    let mounted = true;

    (async () => {
      try {
        const settings = await invoke<AppSettings>('get_settings');
        if (!mounted) return;
        settingsRef.current = settings;
        const mode = settings.activeThemeMode ?? 'system';
        const light = settings.lightTheme ?? 'light';
        const dark = settings.darkTheme ?? 'dark';
        setActiveMode(mode);
        setLightThemeName(light);
        setDarkThemeName(dark);
        await applyCurrentTheme(mode, light, dark);
        await reloadThemes();
      } catch (e) {
        console.error('ThemeContext: failed to load settings', e);
      }
    })();

    const unlistenPromise = listen<{ activeThemeMode: string; lightTheme: string; darkTheme: string }>(
      'krillnotes://theme-changed',
      (event) => {
        if (!mounted) return;
        const { activeThemeMode, lightTheme, darkTheme } = event.payload;
        setActiveMode(activeThemeMode);
        setLightThemeName(lightTheme);
        setDarkThemeName(darkTheme);
        applyCurrentTheme(activeThemeMode, lightTheme, darkTheme);
      },
    );

    return () => {
      mounted = false;
      unlistenPromise.then(unlisten => unlisten());
    };
  }, [applyCurrentTheme, reloadThemes]);

  // Watch system preference when mode === "system".
  useEffect(() => {
    if (activeMode !== 'system') return;
    const unwatch = watchSystem(async (variant) => {
      const name = variant === 'dark' ? darkThemeName : lightThemeName;
      await themeManager.loadAndApply(name, variant);
    });
    return unwatch;
  }, [activeMode, lightThemeName, darkThemeName]);

  const persistSettings = useCallback(
    async (patch: Partial<Pick<AppSettings, 'activeThemeMode' | 'lightTheme' | 'darkTheme'>>) => {
      const base = settingsRef.current ?? await invoke<AppSettings>('get_settings');
      const updated = { ...base, ...patch };
      settingsRef.current = updated;
      await invoke('update_settings', { patch: updated });
      await emit('krillnotes://theme-changed', {
        activeThemeMode: updated.activeThemeMode,
        lightTheme: updated.lightTheme,
        darkTheme: updated.darkTheme,
      });
    },
    [],
  );

  const setMode = useCallback(
    async (mode: string) => {
      setActiveMode(mode);
      await persistSettings({ activeThemeMode: mode });
      await applyCurrentTheme(mode, lightThemeName, darkThemeName);
    },
    [lightThemeName, darkThemeName, applyCurrentTheme, persistSettings],
  );

  const setLightTheme = useCallback(
    async (name: string) => {
      setLightThemeName(name);
      await persistSettings({ lightTheme: name });
      await applyCurrentTheme(activeMode, name, darkThemeName);
    },
    [activeMode, darkThemeName, applyCurrentTheme, persistSettings],
  );

  const setDarkTheme = useCallback(
    async (name: string) => {
      setDarkThemeName(name);
      await persistSettings({ darkTheme: name });
      await applyCurrentTheme(activeMode, lightThemeName, name);
    },
    [activeMode, lightThemeName, applyCurrentTheme, persistSettings],
  );

  return (
    <ThemeContext.Provider value={{
      activeMode, lightThemeName, darkThemeName, themes,
      setMode, setLightTheme, setDarkTheme, reloadThemes,
    }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used inside ThemeProvider');
  return ctx;
}
