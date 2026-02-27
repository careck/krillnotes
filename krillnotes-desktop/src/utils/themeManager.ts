// themeManager.ts — Load, merge, and apply themes as CSS custom properties.

import { invoke } from '@tauri-apps/api/core';
import {
  LIGHT_BASE, DARK_BASE, mergeTheme,
  type ThemeFile, type ResolvedTheme,
} from './theme';

export type ThemeVariant = 'light' | 'dark';

// ── CSS var application ───────────────────────────────────────────

function applyTokens(tokens: ResolvedTheme, variant: ThemeVariant): void {
  const root = document.documentElement;
  const c = tokens.colors;

  root.style.setProperty('--color-background',            c.background);
  root.style.setProperty('--color-foreground',            c.foreground);
  root.style.setProperty('--color-primary',               c.primary);
  root.style.setProperty('--color-primary-foreground',    c.primaryForeground);
  root.style.setProperty('--color-secondary',             c.secondary);
  root.style.setProperty('--color-secondary-foreground',  c.secondaryForeground);
  root.style.setProperty('--color-muted',                 c.muted);
  root.style.setProperty('--color-muted-foreground',      c.mutedForeground);
  root.style.setProperty('--color-accent',                c.accent);
  root.style.setProperty('--color-accent-foreground',     c.accentForeground);
  root.style.setProperty('--color-border',                c.border);
  root.style.setProperty('--color-input',                 c.input);
  root.style.setProperty('--color-ring',                  c.ring);

  root.style.setProperty('--typography-font-family', tokens.typography.fontFamily);
  root.style.setProperty('--typography-font-size',   tokens.typography.fontSize);
  root.style.setProperty('--typography-line-height', tokens.typography.lineHeight);

  root.style.setProperty('--spacing-scale', String(tokens.spacing.scale));
  root.style.setProperty('--icon-size',     tokens.iconSize);

  if (variant === 'dark') {
    root.classList.add('dark');
  } else {
    root.classList.remove('dark');
  }
}

// ── Load and apply ────────────────────────────────────────────────

async function loadAndApply(name: string, variant: ThemeVariant): Promise<void> {
  const base = variant === 'dark' ? DARK_BASE : LIGHT_BASE;

  if (name === 'light' || name === 'dark') {
    applyTokens(base, variant);
    return;
  }

  try {
    const filename = `${name.toLowerCase().replace(/\s+/g, '-')}.krilltheme`;
    const raw = await invoke<string>('read_theme', { filename });
    const file: ThemeFile = JSON.parse(raw);
    const block = variant === 'dark' ? file['dark-theme'] : file['light-theme'];
    const tokens = block ? mergeTheme(base, block) : base;
    applyTokens(tokens, variant);
  } catch {
    // Fallback to base on any error.
    applyTokens(base, variant);
  }
}

// ── System preference watcher ─────────────────────────────────────

let _mql: MediaQueryList | null = null;
let _onSystemChange: (() => void) | null = null;

export function watchSystem(callback: (variant: ThemeVariant) => void): () => void {
  if (_mql && _onSystemChange) {
    _mql.removeEventListener('change', _onSystemChange);
  }
  _mql = window.matchMedia('(prefers-color-scheme: dark)');
  _onSystemChange = () => callback(_mql!.matches ? 'dark' : 'light');
  _mql.addEventListener('change', _onSystemChange);
  return () => {
    if (_mql && _onSystemChange) {
      _mql.removeEventListener('change', _onSystemChange);
    }
  };
}

export function systemVariant(): ThemeVariant {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export const themeManager = { loadAndApply, applyTokens };
