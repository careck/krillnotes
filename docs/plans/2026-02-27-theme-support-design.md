# Theme Support — Design Document

**Date:** 2026-02-27
**Issue:** [#21](https://github.com/careck/krillnotes/issues/21)
**Status:** Approved

## Summary

Add a full theme system to Krillnotes that lets users control colours, typography, spacing scale, and icon sizes via `.krilltheme` JSON files. The system is designed for maximum creativity while keeping the simple case easy — a user can change one colour in five lines without authoring a 200-line stylesheet.

Phase 1 ships the complete infrastructure (schema, runtime, storage, UI) with built-in light/dark themes and system auto-switching. A community gallery and visual colour-picker editor are deferred.

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│              .krilltheme files                   │
│  ~/.config/krillnotes/themes/ocean.krilltheme    │
│  (user themes on disk; built-ins are hardcoded)  │
└──────────────────┬──────────────────────────────┘
                   │  Tauri commands
                   ▼
┌─────────────────────────────────────────────────┐
│  Rust: theme commands                            │
│  list_themes / read_theme / write_theme /        │
│  delete_theme                                    │
│  AppSettings: active_theme_mode,                 │
│               light_theme, dark_theme            │
└──────────────────┬──────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────┐
│  TS: ThemeManager (singleton module)             │
│  load(name, variant) → merged token map          │
│  apply(tokens) → CSS vars on <html>              │
│  watchSystem() → OS preference listener          │
└──────────────────┬──────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────┐
│  ThemeContext (React)                            │
│  activeMode, lightThemeName, darkThemeName,      │
│  themes[], setMode, setLightTheme, setDarkTheme  │
└──────────────────┬──────────────────────────────┘
                   │
          ┌────────┴────────┐
          ▼                 ▼
  SettingsDialog    ManageThemesDialog
  (pickers + mode)  (list + JSON editor)
```

---

## Token Schema

A `.krilltheme` file is JSON. The top-level keys `light-theme` and `dark-theme` determine which picker(s) the theme appears in. A file may define one or both variants. All token fields are optional — anything omitted inherits from the hardcoded built-in base.

```json
{
  "name": "Ocean Breeze",
  "light-theme": {
    "colors": {
      "background": "oklch(97% 0.02 210)",
      "foreground": "oklch(10% 0.04 222)",
      "primary":    "oklch(35% 0.10 240)",
      "secondary":  "oklch(94% 0.03 210)",
      "muted":      "oklch(94% 0.03 210)",
      "accent":     "oklch(90% 0.06 200)",
      "border":     "oklch(88% 0.03 214)"
    },
    "typography": {
      "fontFamily": "\"Georgia\", serif",
      "fontSize":   "14px",
      "lineHeight": "1.6"
    },
    "spacing": { "scale": 1.0 },
    "iconSize": "16px"
  },
  "dark-theme": {
    "colors": {
      "background": "oklch(15% 0.05 240)",
      "foreground": "oklch(95% 0.02 210)"
    },
    "typography": {
      "fontFamily": "\"Georgia\", serif"
    }
  }
}
```

### Picker routing

| File contains    | Appears in        |
|------------------|-------------------|
| `light-theme` only | light picker only |
| `dark-theme` only  | dark picker only  |
| both keys          | both pickers      |

### Built-in themes

`light` and `dark` are hardcoded TypeScript objects (not files on disk). They cannot be deleted or edited. They serve as the immutable merge base for all custom themes. They appear at the top of both pickers and in Manage Themes as read-only.

---

## Runtime: ThemeManager

Singleton module at `src/utils/themeManager.ts`.

**`load(name: string, variant: 'light' | 'dark'): ThemeTokens`**
- If `name === 'light'` or `'dark'`: return hardcoded base directly.
- Otherwise: read file via `read_theme`, parse JSON, extract the matching variant block, deep-merge onto the hardcoded base for that variant.

**`apply(tokens: ThemeTokens): void`**
- Iterate the flattened token map and write each as a CSS custom property on `document.documentElement`.
- Example: `tokens.colors.background` → `--color-background`.

**`watchSystem(): void`**
- Attaches a `window.matchMedia('(prefers-color-scheme: dark)')` listener.
- On change, re-evaluates which theme to apply when `activeMode === 'system'`.

---

## Storage: Rust Layer

**File location:** `~/.config/krillnotes/themes/*.krilltheme`

**Tauri commands:**

| Command | Signature | Notes |
|---------|-----------|-------|
| `list_themes` | `→ Vec<ThemeMeta>` | `{ name, filename, has_light, has_dark }` |
| `read_theme` | `(filename: String) → String` | Raw JSON content |
| `write_theme` | `(filename: String, content: String) → ()` | Creates or overwrites |
| `delete_theme` | `(filename: String) → ()` | User themes only |

**AppSettings additions** (all `#[serde(default)]` for backward compat):

```rust
pub active_theme_mode: String,  // "light" | "dark" | "system"  (default: "system")
pub light_theme: String,        // theme name  (default: "light")
pub dark_theme: String,         // theme name  (default: "dark")
```

---

## UI: Settings Dialog

New "Appearance" section in `SettingsDialog`:

```
Appearance
  Mode:         [ Light ]  [ Dark ]  [ System ]

  Light theme:  [ light ▾ ]
  Dark theme:   [ dark  ▾ ]

                [ Manage Themes... ]
```

- Mode buttons persist `active_theme_mode`.
- Pickers list all themes that have the matching variant key. Built-ins always appear first.
- "Manage Themes" opens `ManageThemesDialog`.

---

## UI: Manage Themes Dialog

Mirrors `ScriptManagerDialog` — same two-view pattern (`list` → `editor`).

**List view:**
- Built-ins pinned at top with a "built-in" read-only badge.
- User themes listed below, each row showing: name, variant chips (light / dark / both).
- Row actions: Edit, Delete.
- Global actions: New, Import (file picker for `.krilltheme`).

**Editor view:**
- CodeMirror with JSON syntax highlighting (re-uses `ScriptEditor` component).
- "Save" button; built-ins shown read-only with an explanatory notice.
- New theme pre-fills with a fully commented template listing every available field.

---

## CSS Wiring

The existing CSS custom property names in `globals.css` become the target map. ThemeManager writes directly to these same variable names, so all existing Tailwind utilities (`bg-background`, `text-foreground`, etc.) and hand-written `-kn-*` classes respond automatically with no component changes.

Additional CSS variables to add for the new tokens:
- `--typography-font-family`
- `--typography-font-size`
- `--typography-line-height`
- `--spacing-scale`
- `--icon-size`

---

## CodeMirror Theming

`ScriptEditor` currently uses CodeMirror's default light theme. In dark mode it should switch to a dark variant (e.g. `@codemirror/theme-one-dark`). ThemeContext provides the current resolved variant; ScriptEditor reads it and selects the appropriate CodeMirror theme.

---

## Phase 1 Scope

**In:**
- Full token schema and TypeScript types
- Hardcoded `light` and `dark` base themes
- ThemeManager singleton (load, apply, watchSystem)
- ThemeContext
- Manage Themes dialog
- Settings appearance section (mode + pickers)
- System auto-switch listener
- CSS var additions to `globals.css`
- CodeMirror dark theme in ScriptEditor
- AppSettings Rust additions + Tauri theme commands

**Deferred:**
- Visual colour-picker editor
- Community / theme gallery
- Export file picker
- Live preview while editing JSON in Manage Themes
