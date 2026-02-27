# Theme Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a full `.krilltheme` JSON token system with built-in light/dark bases, runtime CSS-var application, system auto-switching, Manage Themes dialog, and appearance pickers in Settings.

**Architecture:** User themes are JSON files stored in `~/.config/krillnotes/themes/`. They define `light-theme` and/or `dark-theme` token blocks that deep-merge onto hardcoded TypeScript base themes. ThemeManager applies the merged tokens as CSS custom properties on `<html>`. ThemeContext exposes the active state to React. Settings holds three new fields: `activeThemeMode`, `lightTheme`, `darkTheme`.

**Tech Stack:** Rust/Tauri v2, React 19, TypeScript, Tailwind CSS v4, CodeMirror 6.

---

### Task 1: Add theme fields to AppSettings (Rust)

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/settings.rs`

**Step 1: Add three fields to `AppSettings`**

In `settings.rs`, add to the `AppSettings` struct (after `cache_workspace_passwords`):

```rust
/// Current theme mode: "light", "dark", or "system".
#[serde(default = "default_theme_mode")]
pub active_theme_mode: String,
/// Name of the theme to use in light mode.
#[serde(default = "default_light_theme")]
pub light_theme: String,
/// Name of the theme to use in dark mode.
#[serde(default = "default_dark_theme")]
pub dark_theme: String,
```

Add the three default fns after `default_workspace_directory()`:

```rust
fn default_theme_mode() -> String { "system".to_string() }
fn default_light_theme() -> String { "light".to_string() }
fn default_dark_theme() -> String { "dark".to_string() }
```

Also add to `impl Default for AppSettings`:

```rust
active_theme_mode: default_theme_mode(),
light_theme: default_light_theme(),
dark_theme: default_dark_theme(),
```

**Step 2: Add backward-compat deserialization test**

At the bottom of `settings.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_settings_without_theme_fields() {
        let json = r#"{"workspaceDirectory":"/tmp","cacheWorkspacePasswords":false}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_theme_mode, "system");
        assert_eq!(s.light_theme, "light");
        assert_eq!(s.dark_theme, "dark");
    }
}
```

**Step 3: Run the test**

```bash
cd krillnotes-desktop/src-tauri && cargo test settings
```
Expected: 1 test passes.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/settings.rs
git commit -m "feat(settings): add theme mode and light/dark theme name fields"
```

---

### Task 2: Theme file storage module (Rust)

**Files:**
- Create: `krillnotes-desktop/src-tauri/src/themes.rs`

**Step 1: Write the failing test first**

Create `themes.rs` with the tests module only:

```rust
//! App-level theme file storage.
//!
//! Themes are stored as `.krilltheme` JSON files in the same config
//! directory as `settings.json`.

use std::fs;
use std::path::PathBuf;

/// Metadata returned when listing themes (excludes raw JSON content).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeMeta {
    pub name: String,
    pub filename: String,
    pub has_light: bool,
    pub has_dark: bool,
}

/// Returns the themes directory path, creating it if absent.
pub fn themes_dir() -> PathBuf {
    let base = {
        #[cfg(target_os = "windows")]
        { dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Krillnotes") }
        #[cfg(not(target_os = "windows"))]
        { dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".config").join("krillnotes") }
    };
    let dir = base.join("themes");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Lists all `.krilltheme` files in the themes directory.
pub fn list_themes() -> Result<Vec<ThemeMeta>, String> {
    let dir = themes_dir();
    let mut metas = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("krilltheme") {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("Invalid JSON in {:?}: {e}", path))?;
        let name = json.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed")
            .to_string();
        let has_light = json.get("light-theme").is_some();
        let has_dark = json.get("dark-theme").is_some();
        let filename = path.file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("")
            .to_string();
        metas.push(ThemeMeta { name, filename, has_light, has_dark });
    }
    metas.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(metas)
}

/// Returns the raw JSON content of a theme file.
pub fn read_theme(filename: &str) -> Result<String, String> {
    let path = themes_dir().join(filename);
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Writes (creates or overwrites) a theme file.
pub fn write_theme(filename: &str, content: &str) -> Result<(), String> {
    // Validate JSON before saving.
    let _: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    let path = themes_dir().join(filename);
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// Deletes a theme file.
pub fn delete_theme(filename: &str) -> Result<(), String> {
    let path = themes_dir().join(filename);
    fs::remove_file(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_themes_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn write_and_read_theme() {
        let dir = temp_themes_dir();
        let path = dir.path().join("test.krilltheme");
        let content = r#"{"name":"Test","dark-theme":{"colors":{}}}"#;
        fs::write(&path, content).unwrap();
        let read = fs::read_to_string(&path).unwrap();
        assert_eq!(read, content);
    }

    #[test]
    fn list_themes_returns_has_light_has_dark() {
        let dir = temp_themes_dir();
        fs::write(
            dir.path().join("both.krilltheme"),
            r#"{"name":"Both","light-theme":{},"dark-theme":{}}"#,
        ).unwrap();
        // Use the low-level dir directly to test parsing logic.
        let content = fs::read_to_string(dir.path().join("both.krilltheme")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(json.get("light-theme").is_some());
        assert!(json.get("dark-theme").is_some());
    }
}
```

**Step 2: Add `tempfile` dev-dependency**

In `krillnotes-desktop/src-tauri/Cargo.toml`, under `[dev-dependencies]`:

```toml
tempfile = "3"
```

**Step 3: Run tests**

```bash
cd krillnotes-desktop/src-tauri && cargo test themes
```
Expected: 2 tests pass.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/themes.rs krillnotes-desktop/src-tauri/Cargo.toml
git commit -m "feat(themes): add theme file storage module with list/read/write/delete"
```

---

### Task 3: Register theme Tauri commands (Rust)

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Declare the module**

At the top of `lib.rs` where other `mod` declarations are, add:

```rust
mod themes;
```

**Step 2: Add four Tauri command functions**

Find the `// ── Settings commands` section (~line 1080). Add a new block just before it:

```rust
// ── Theme commands ────────────────────────────────────────────────

/// Lists all user theme files in the themes directory.
#[tauri::command]
fn list_themes() -> std::result::Result<Vec<themes::ThemeMeta>, String> {
    themes::list_themes()
}

/// Returns the raw JSON content of a theme file.
#[tauri::command]
fn read_theme(filename: String) -> std::result::Result<String, String> {
    themes::read_theme(&filename)
}

/// Writes (creates or overwrites) a theme file.
#[tauri::command]
fn write_theme(filename: String, content: String) -> std::result::Result<(), String> {
    themes::write_theme(&filename, &content)
}

/// Deletes a theme file.
#[tauri::command]
fn delete_theme(filename: String) -> std::result::Result<(), String> {
    themes::delete_theme(&filename)
}
```

**Step 3: Register in `generate_handler!`**

In the `invoke_handler` block (~line 1266), add after `update_settings`:

```rust
list_themes,
read_theme,
write_theme,
delete_theme,
```

**Step 4: Build to verify no errors**

```bash
cd krillnotes-desktop/src-tauri && cargo build 2>&1 | tail -5
```
Expected: `Finished` with no errors.

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(themes): expose list/read/write/delete_theme as Tauri commands"
```

---

### Task 4: TypeScript token schema + hardcoded base themes

**Files:**
- Create: `krillnotes-desktop/src/utils/theme.ts`

**Step 1: Create the file**

```typescript
// theme.ts — Token schema types and hardcoded base themes.
//
// All fields in ThemeVariant are optional. Anything omitted inherits
// from the matching base (LIGHT_BASE or DARK_BASE).

export interface ThemeColors {
  background?: string;
  foreground?: string;
  primary?: string;
  primaryForeground?: string;
  secondary?: string;
  secondaryForeground?: string;
  muted?: string;
  mutedForeground?: string;
  accent?: string;
  accentForeground?: string;
  border?: string;
  input?: string;
  ring?: string;
}

export interface ThemeTypography {
  fontFamily?: string;
  fontSize?: string;
  lineHeight?: string;
}

export interface ThemeSpacing {
  scale?: number;
}

export interface ThemeVariant {
  colors?: ThemeColors;
  typography?: ThemeTypography;
  spacing?: ThemeSpacing;
  iconSize?: string;
}

export interface ThemeFile {
  name: string;
  'light-theme'?: ThemeVariant;
  'dark-theme'?: ThemeVariant;
}

// Metadata returned from the Rust list_themes command.
export interface ThemeMeta {
  name: string;
  filename: string;
  hasLight: boolean;
  hasDark: boolean;
}

// ── Resolved token type (all fields present after merge) ──────────

export interface ResolvedThemeColors {
  background: string;
  foreground: string;
  primary: string;
  primaryForeground: string;
  secondary: string;
  secondaryForeground: string;
  muted: string;
  mutedForeground: string;
  accent: string;
  accentForeground: string;
  border: string;
  input: string;
  ring: string;
}

export interface ResolvedThemeTypography {
  fontFamily: string;
  fontSize: string;
  lineHeight: string;
}

export interface ResolvedTheme {
  colors: ResolvedThemeColors;
  typography: ResolvedThemeTypography;
  spacing: { scale: number };
  iconSize: string;
}

// ── Hardcoded base themes (extracted from globals.css) ────────────

export const LIGHT_BASE: ResolvedTheme = {
  colors: {
    background:          'oklch(100% 0 0)',
    foreground:          'oklch(9.8% 0.041 222.2)',
    primary:             'oklch(22.4% 0.053 222.2)',
    primaryForeground:   'oklch(98% 0.04 210)',
    secondary:           'oklch(96.1% 0.04 210)',
    secondaryForeground: 'oklch(22.4% 0.053 222.2)',
    muted:               'oklch(96.1% 0.04 210)',
    mutedForeground:     'oklch(57.5% 0.025 215.4)',
    accent:              'oklch(96.1% 0.04 210)',
    accentForeground:    'oklch(22.4% 0.053 222.2)',
    border:              'oklch(91.4% 0.032 214.3)',
    input:               'oklch(91.4% 0.032 214.3)',
    ring:                'oklch(9.8% 0.041 222.2)',
  },
  typography: {
    fontFamily: 'system-ui, sans-serif',
    fontSize:   '14px',
    lineHeight: '1.5',
  },
  spacing: { scale: 1.0 },
  iconSize: '16px',
};

export const DARK_BASE: ResolvedTheme = {
  colors: {
    background:          'oklch(9.8% 0.041 222.2)',
    foreground:          'oklch(98% 0.04 210)',
    primary:             'oklch(98% 0.04 210)',
    primaryForeground:   'oklch(22.4% 0.053 222.2)',
    secondary:           'oklch(30.3% 0.033 217.2)',
    secondaryForeground: 'oklch(98% 0.04 210)',
    muted:               'oklch(30.3% 0.033 217.2)',
    mutedForeground:     'oklch(72.3% 0.026 215)',
    accent:              'oklch(30.3% 0.033 217.2)',
    accentForeground:    'oklch(98% 0.04 210)',
    border:              'oklch(30.3% 0.033 217.2)',
    input:               'oklch(30.3% 0.033 217.2)',
    ring:                'oklch(88.2% 0.027 212.7)',
  },
  typography: {
    fontFamily: 'system-ui, sans-serif',
    fontSize:   '14px',
    lineHeight: '1.5',
  },
  spacing: { scale: 1.0 },
  iconSize: '16px',
};

// ── Deep merge helper ──────────────────────────────────────────────

export function mergeTheme(
  base: ResolvedTheme,
  overrides: ThemeVariant,
): ResolvedTheme {
  return {
    colors:     { ...base.colors,     ...(overrides.colors     ?? {}) },
    typography: { ...base.typography, ...(overrides.typography ?? {}) },
    spacing:    { ...base.spacing,    ...(overrides.spacing    ?? {}) },
    iconSize:   overrides.iconSize ?? base.iconSize,
  };
}
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/theme.ts
git commit -m "feat(theme): add token schema types, resolved types, base themes, and merge helper"
```

---

### Task 5: ThemeManager singleton

**Files:**
- Create: `krillnotes-desktop/src/utils/themeManager.ts`

**Step 1: Create the file**

```typescript
// themeManager.ts — Load, merge, and apply themes as CSS custom properties.

import { invoke } from '@tauri-apps/api/core';
import {
  LIGHT_BASE, DARK_BASE, mergeTheme,
  type ThemeFile, type ResolvedTheme,
} from './theme';

export type ThemeVariant = 'light' | 'dark';

// ── CSS var mapping ───────────────────────────────────────────────

function applyTokens(tokens: ResolvedTheme, variant: ThemeVariant): void {
  const root = document.documentElement;

  // Colors
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

  // Typography
  root.style.setProperty('--typography-font-family', tokens.typography.fontFamily);
  root.style.setProperty('--typography-font-size',   tokens.typography.fontSize);
  root.style.setProperty('--typography-line-height', tokens.typography.lineHeight);

  // Spacing + icon
  root.style.setProperty('--spacing-scale', String(tokens.spacing.scale));
  root.style.setProperty('--icon-size',     tokens.iconSize);

  // Toggle dark class for Tailwind utilities
  if (variant === 'dark') {
    root.classList.add('dark');
  } else {
    root.classList.remove('dark');
  }
}

// ── Load & apply ──────────────────────────────────────────────────

async function loadAndApply(name: string, variant: ThemeVariant): Promise<void> {
  const base = variant === 'dark' ? DARK_BASE : LIGHT_BASE;

  if (name === 'light' || name === 'dark') {
    applyTokens(base, variant);
    return;
  }

  try {
    // filename convention: <name>.krilltheme  (spaces → hyphens, lowercase)
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
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/themeManager.ts
git commit -m "feat(theme): add ThemeManager with load, apply, and system watcher"
```

---

### Task 6: Add new CSS variables to globals.css

**Files:**
- Modify: `krillnotes-desktop/src/styles/globals.css`

**Step 1: Add new vars to the `@theme` block**

Inside the `@theme { }` block (after `--radius-sm`), add:

```css
  /* Typography */
  --typography-font-family: system-ui, sans-serif;
  --typography-font-size: 14px;
  --typography-line-height: 1.5;

  /* Spacing scale (unitless multiplier) */
  --spacing-scale: 1;

  /* Icon size */
  --icon-size: 16px;
```

**Step 2: Wire font vars into `body`**

In the existing `body { }` rule (~line 56), add:

```css
  font-family: var(--typography-font-family);
  font-size: var(--typography-font-size);
  line-height: var(--typography-line-height);
```

**Step 3: Verify the app builds**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```
Expected: build succeeds.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/styles/globals.css
git commit -m "feat(theme): add typography, spacing-scale, and icon-size CSS variables"
```

---

### Task 7: ThemeContext

**Files:**
- Create: `krillnotes-desktop/src/contexts/ThemeContext.tsx`

**Step 1: Create the contexts directory and file**

```bash
mkdir -p krillnotes-desktop/src/contexts
```

```tsx
// ThemeContext.tsx — React context for the active theme state.

import { createContext, useContext, useEffect, useState, useCallback, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { themeManager, watchSystem, systemVariant, type ThemeVariant } from '../utils/themeManager';
import type { ThemeMeta } from '../utils/theme';
import type { AppSettings } from '../types';

interface ThemeContextValue {
  activeMode: string;           // "light" | "dark" | "system"
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

  // Load settings and apply theme on mount.
  useEffect(() => {
    (async () => {
      const settings = await invoke<AppSettings>('get_settings');
      const mode = settings.activeThemeMode ?? 'system';
      const light = settings.lightTheme ?? 'light';
      const dark = settings.darkTheme ?? 'dark';
      setActiveMode(mode);
      setLightThemeName(light);
      setDarkThemeName(dark);
      await applyCurrentTheme(mode, light, dark);
      await reloadThemes();
    })();
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
      const current = await invoke<AppSettings>('get_settings');
      await invoke('update_settings', { settings: { ...current, ...patch } });
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
```

**Step 2: Add missing fields to the `AppSettings` type**

The frontend `AppSettings` type lives in `src/types.ts` (or similar). Find it and add:

```typescript
activeThemeMode?: string;
lightTheme?: string;
darkTheme?: string;
```

Search for the type: `grep -rn "AppSettings" krillnotes-desktop/src --include="*.ts" --include="*.tsx"`

**Step 3: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/contexts/ThemeContext.tsx krillnotes-desktop/src/types.ts
git commit -m "feat(theme): add ThemeContext with load-on-mount and system watcher"
```

---

### Task 8: Wire ThemeProvider into App

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Import and wrap**

At the top of `App.tsx`, add:

```tsx
import { ThemeProvider } from './contexts/ThemeContext';
```

Wrap the root JSX return in `<ThemeProvider>`:

```tsx
return (
  <ThemeProvider>
    {/* existing JSX unchanged */}
  </ThemeProvider>
);
```

**Step 2: Run the dev server to smoke-test**

```bash
cd krillnotes-desktop && npm run dev
```

Open the app. It should start in system-default appearance. Check DevTools: `document.documentElement.style` should show `--color-background` etc. set inline.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat(theme): wrap App in ThemeProvider so theme applies on startup"
```

---

### Task 9: ManageThemesDialog

**Files:**
- Create: `krillnotes-desktop/src/components/ManageThemesDialog.tsx`

**Step 1: Install `@codemirror/lang-json`**

```bash
cd krillnotes-desktop && npm install @codemirror/lang-json
```

**Step 2: Create the component**

```tsx
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { EditorView, keymap, lineNumbers, highlightActiveLine } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { json } from '@codemirror/lang-json';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching } from '@codemirror/language';
import { useRef } from 'react';
import { useTheme } from '../contexts/ThemeContext';
import type { ThemeMeta } from '../utils/theme';

const NEW_THEME_TEMPLATE = `{
  "name": "My Theme",
  "dark-theme": {
    "colors": {
      // "background": "oklch(10% 0.04 240)",
      // "foreground": "oklch(95% 0.02 210)",
      // "primary":    "oklch(65% 0.15 240)"
    },
    "typography": {
      // "fontFamily": "\\"JetBrains Mono\\", monospace",
      // "fontSize":   "14px",
      // "lineHeight": "1.6"
    },
    "spacing": {
      // "scale": 1.0
    }
    // "iconSize": "16px"
  }
}
`;

const BUILT_IN_NAMES = ['light', 'dark'];

interface Props {
  isOpen: boolean;
  onClose: () => void;
}

type View = 'list' | 'editor';

export default function ManageThemesDialog({ isOpen, onClose }: Props) {
  const { themes, reloadThemes, lightThemeName, darkThemeName, setLightTheme, setDarkTheme } = useTheme();
  const [view, setView] = useState<View>('list');
  const [editingMeta, setEditingMeta] = useState<ThemeMeta | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const contentRef = useRef(editorContent);
  contentRef.current = editorContent;

  useEffect(() => {
    if (isOpen) { reloadThemes(); setView('list'); setError(''); }
  }, [isOpen, reloadThemes]);

  // CodeMirror lifecycle
  useEffect(() => {
    if (view !== 'editor' || !containerRef.current) return;
    viewRef.current?.destroy();
    const isBuiltIn = editingMeta ? BUILT_IN_NAMES.includes(editingMeta.name) : false;
    const state = EditorState.create({
      doc: editorContent,
      extensions: [
        lineNumbers(), highlightActiveLine(), history(), bracketMatching(),
        json(),
        syntaxHighlighting(defaultHighlightStyle),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.editable.of(!isBuiltIn),
        EditorView.updateListener.of(update => {
          if (update.docChanged) setEditorContent(update.state.doc.toString());
        }),
        EditorView.theme({
          '&': { height: '100%', fontSize: '13px' },
          '.cm-scroller': { overflow: 'auto', fontFamily: 'monospace' },
        }),
      ],
    });
    viewRef.current = new EditorView({ state, parent: containerRef.current });
    return () => { viewRef.current?.destroy(); viewRef.current = null; };
  }, [view, editingMeta]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { view === 'editor' ? setView('list') : onClose(); }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [isOpen, view, onClose]);

  if (!isOpen) return null;

  const handleNew = () => {
    setEditingMeta(null);
    setEditorContent(NEW_THEME_TEMPLATE);
    setError('');
    setView('editor');
  };

  const handleEdit = async (meta: ThemeMeta) => {
    if (BUILT_IN_NAMES.includes(meta.name)) {
      setEditingMeta(meta);
      setEditorContent(`// Built-in theme "${meta.name}" cannot be edited.`);
      setError('');
      setView('editor');
      return;
    }
    try {
      const content = await invoke<string>('read_theme', { filename: meta.filename });
      setEditingMeta(meta);
      setEditorContent(content);
      setError('');
      setView('editor');
    } catch (e) {
      setError(`Failed to read theme: ${e}`);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      let parsed: { name?: string };
      try { parsed = JSON.parse(editorContent); }
      catch { throw new Error('Invalid JSON — check for syntax errors.'); }
      const name = parsed.name ?? 'unnamed';
      const filename = editingMeta?.filename ?? `${name.toLowerCase().replace(/\s+/g, '-')}.krilltheme`;
      await invoke('write_theme', { filename, content: editorContent });
      await reloadThemes();
      setView('list');
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (meta: ThemeMeta) => {
    if (!confirm(`Delete theme "${meta.name}"?`)) return;
    try {
      await invoke('delete_theme', { filename: meta.filename });
      await reloadThemes();
    } catch (e) {
      setError(`Failed to delete: ${e}`);
    }
  };

  const BUILT_IN_METAS: ThemeMeta[] = [
    { name: 'light', filename: '', hasLight: true, hasDark: false },
    { name: 'dark',  filename: '', hasLight: false, hasDark: true },
  ];

  const allThemes = [...BUILT_IN_METAS, ...themes];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-background border border-border rounded-lg w-[700px] max-h-[80vh] flex flex-col shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="font-semibold text-foreground">
            {view === 'list' ? 'Manage Themes' : (editingMeta ? `Editing: ${editingMeta.name}` : 'New Theme')}
          </h2>
          <button onClick={view === 'editor' ? () => setView('list') : onClose}
            className="text-muted-foreground hover:text-foreground text-sm">
            {view === 'editor' ? '← Back' : '✕'}
          </button>
        </div>

        {/* Error */}
        {error && <div className="px-4 py-2 text-sm text-red-600 bg-red-50 border-b border-border">{error}</div>}

        {/* List view */}
        {view === 'list' && (
          <>
            <div className="flex-1 overflow-y-auto">
              {allThemes.map((meta) => {
                const isBuiltIn = BUILT_IN_NAMES.includes(meta.name);
                const isActiveLight = lightThemeName === meta.name;
                const isActiveDark  = darkThemeName  === meta.name;
                return (
                  <div key={meta.filename || meta.name}
                    className="flex items-center justify-between px-4 py-2 border-b border-border hover:bg-secondary/50">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="font-medium text-foreground truncate">{meta.name}</span>
                      {isBuiltIn && <span className="text-xs px-1.5 py-0.5 rounded bg-muted text-muted-foreground">built-in</span>}
                      {meta.hasLight && <span className="text-xs px-1.5 py-0.5 rounded bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">light</span>}
                      {meta.hasDark  && <span className="text-xs px-1.5 py-0.5 rounded bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">dark</span>}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {meta.hasLight && (
                        <button onClick={() => setLightTheme(meta.name)}
                          className={`text-xs px-2 py-1 rounded border ${isActiveLight ? 'bg-primary text-primary-foreground border-primary' : 'border-border text-muted-foreground hover:text-foreground'}`}>
                          {isActiveLight ? '✓ Light' : 'Set Light'}
                        </button>
                      )}
                      {meta.hasDark && (
                        <button onClick={() => setDarkTheme(meta.name)}
                          className={`text-xs px-2 py-1 rounded border ${isActiveDark ? 'bg-primary text-primary-foreground border-primary' : 'border-border text-muted-foreground hover:text-foreground'}`}>
                          {isActiveDark ? '✓ Dark' : 'Set Dark'}
                        </button>
                      )}
                      <button onClick={() => handleEdit(meta)}
                        className="text-xs text-muted-foreground hover:text-foreground">
                        {isBuiltIn ? 'View' : 'Edit'}
                      </button>
                      {!isBuiltIn && (
                        <button onClick={() => handleDelete(meta)}
                          className="text-xs text-red-500 hover:text-red-700">Delete</button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            <div className="px-4 py-3 border-t border-border flex justify-between">
              <button onClick={handleNew}
                className="text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:opacity-90">
                + New Theme
              </button>
              <button onClick={onClose} className="text-sm text-muted-foreground hover:text-foreground">Close</button>
            </div>
          </>
        )}

        {/* Editor view */}
        {view === 'editor' && (
          <>
            {editingMeta && BUILT_IN_NAMES.includes(editingMeta.name) && (
              <div className="px-4 py-2 text-sm text-muted-foreground bg-muted border-b border-border">
                Built-in themes are read-only. Create a new theme that overrides only what you need.
              </div>
            )}
            <div ref={containerRef} className="flex-1 overflow-hidden border-b border-border" />
            {(!editingMeta || !BUILT_IN_NAMES.includes(editingMeta.name)) && (
              <div className="px-4 py-3 flex justify-end gap-2">
                <button onClick={() => setView('list')} className="text-sm text-muted-foreground hover:text-foreground">Cancel</button>
                <button onClick={handleSave} disabled={saving}
                  className="text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50">
                  {saving ? 'Saving…' : 'Save'}
                </button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
```

**Step 3: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/ManageThemesDialog.tsx package.json package-lock.json
git commit -m "feat(theme): add ManageThemesDialog with list view and JSON editor"
```

---

### Task 10: Appearance section in SettingsDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/SettingsDialog.tsx`

**Step 1: Add imports at the top of SettingsDialog.tsx**

```tsx
import { useTheme } from '../contexts/ThemeContext';
import ManageThemesDialog from './ManageThemesDialog';
```

**Step 2: Inside the component, read from ThemeContext**

```tsx
const { activeMode, lightThemeName, darkThemeName, themes, setMode, setLightTheme, setDarkTheme } = useTheme();
const [manageThemesOpen, setManageThemesOpen] = useState(false);
```

**Step 3: Add the Appearance section to the JSX**

Find where the settings form content ends (before the Save/Cancel buttons) and insert:

```tsx
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

<ManageThemesDialog isOpen={manageThemesOpen} onClose={() => setManageThemesOpen(false)} />
```

**Step 4: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

**Step 5: Smoke-test in dev**

```bash
cd krillnotes-desktop && npm run dev
```

Open Settings. The Appearance section should be visible. Toggle mode, change theme — the app should switch in real time.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/SettingsDialog.tsx
git commit -m "feat(theme): add Appearance section to SettingsDialog with mode/theme pickers"
```

---

### Task 11: CodeMirror dark theme in ScriptEditor

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptEditor.tsx`

**Step 1: Install `@codemirror/theme-one-dark`**

```bash
cd krillnotes-desktop && npm install @codemirror/theme-one-dark
```

**Step 2: Import and apply**

At the top of `ScriptEditor.tsx`, add:

```tsx
import { oneDark } from '@codemirror/theme-one-dark';
import { useTheme } from '../contexts/ThemeContext';
import { systemVariant } from '../utils/themeManager';
```

Inside the component (before the `useEffect` that creates the editor), add:

```tsx
const { activeMode } = useTheme();
const isDark = activeMode === 'dark' || (activeMode === 'system' && systemVariant() === 'dark');
```

In the `EditorState.create` extensions array, conditionally add the dark theme:

```tsx
...(isDark ? [oneDark] : []),
```

Add `isDark` to the `useEffect` dependency array so the editor re-creates when the theme changes:

```tsx
}, [value, isDark]);
```

**Step 3: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

**Step 4: Test in dev**

Open the app, toggle to dark mode in Settings, then open a script editor. The editor should use the dark theme.

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptEditor.tsx package.json package-lock.json
git commit -m "feat(theme): apply CodeMirror one-dark theme when app is in dark mode"
```

---

### Task 12: Final build verification

**Step 1: Run the full Rust test suite**

```bash
cd krillnotes-desktop/src-tauri && cargo test 2>&1 | tail -10
```
Expected: all tests pass, no regressions.

**Step 2: Run TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```
Expected: no errors.

**Step 3: Production build**

```bash
cd krillnotes-desktop && npm run build
```
Expected: build succeeds with no errors.

**Step 4: Manual smoke-test checklist**
- [ ] App starts in system-default appearance
- [ ] Toggle Light / Dark / System in Settings → app switches instantly
- [ ] Open Manage Themes → built-ins listed as read-only
- [ ] Create a new theme with a custom background colour → save → set as active → colour changes
- [ ] Restart app → theme is remembered
- [ ] Open script manager → editor uses dark theme when in dark mode

**Step 5: Commit summary if any straggling changes**

```bash
git status
```
