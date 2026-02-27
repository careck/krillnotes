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
