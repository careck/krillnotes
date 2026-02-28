# Design: Settings Appearance Tab + Sync i18n Fix

**Date:** 2026-02-28
**Issues addressed:** Sync settings untranslated (bug), Appearance section → own tab (feature)

---

## Problem 1 — Bug: Sync settings not translated

PR #50 added four new translation keys to `en.json`:

```
settings.tabGeneral
settings.tabSync
settings.sync
settings.syncHint
```

All six non-English locale files (`de`, `fr`, `es`, `ja`, `ko`, `zh`) received the **English text copy-pasted verbatim**. Switching to any non-English language leaves the General/Sync tab labels, and the Sync tab content, in English.

### Fix

Add proper translations for those four keys in all six locale files. No code changes needed — only JSON.

---

## Problem 2 — Feature: Appearance section → own tab

### Current layout

The Settings dialog has two tabs (General, Sync). The General tab contains:

1. Workspace Directory
2. Cache Passwords
3. *(border separator)*
4. **Appearance section** — Language, Mode, Light theme, Dark theme, Manage Themes

### Desired layout

Three tabs: **General | Appearance | Sync**

- **General tab** — Workspace Directory + Cache Passwords (unchanged)
- **Appearance tab** — Language, Mode, Light theme, Dark theme, Manage Themes
- **Sync tab** — unchanged placeholder

### Component changes (SettingsDialog.tsx)

1. Extend `activeTab` type: `'general' | 'appearance' | 'sync'`
2. Add an "Appearance" tab button in the tab bar (between General and Sync)
3. Move the Appearance JSX block to render under `activeTab === 'appearance'`
4. Remove the `border-t border-border pt-4 mt-4` wrapper and the `<h3>` heading — the tab label provides the visual context, so the heading is redundant
5. No changes to state shape, save logic, or dialog dimensions (`w-[500px]` stays)

### New translation key

Add `settings.tabAppearance` to all 7 locale files:

| Locale | Translation |
|--------|-------------|
| en | Appearance |
| de | Erscheinungsbild |
| fr | Apparence |
| es | Apariencia |
| ja | 外観 |
| ko | 외관 |
| zh | 外观 |

---

## Files affected

| File | Change |
|------|--------|
| `krillnotes-desktop/src/components/SettingsDialog.tsx` | Add Appearance tab; restructure tab bar and conditionals |
| `krillnotes-desktop/src/i18n/locales/en.json` | Add `tabAppearance` |
| `krillnotes-desktop/src/i18n/locales/de.json` | Fix 4 existing keys + add `tabAppearance` |
| `krillnotes-desktop/src/i18n/locales/fr.json` | Same |
| `krillnotes-desktop/src/i18n/locales/es.json` | Same |
| `krillnotes-desktop/src/i18n/locales/ja.json` | Same |
| `krillnotes-desktop/src/i18n/locales/ko.json` | Same |
| `krillnotes-desktop/src/i18n/locales/zh.json` | Same |

No Rust changes, no new Tauri commands.
