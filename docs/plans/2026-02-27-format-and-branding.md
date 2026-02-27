# Format Extension + App Branding

**Date:** 2026-02-27
**Branch:** `fix/format-and-branding`

## Changes

### 1. `.krillnotes` file format
- Export save dialog: filter `['zip']` → `['krillnotes']`, defaultPath drops the `.zip` suffix
- Import open dialog: filter `['zip']` → `['krillnotes']`
- File: `krillnotes-desktop/src/App.tsx` (lines 52, 213–214)

### 2. App product name → "Krillnotes"
- `productName`: "krillnotes-desktop" → "Krillnotes"
- `identifier`: "com.careck.krillnotes-desktop" → "com.careck.krillnotes"
- Window `title`: "krillnotes-desktop" → "Krillnotes"
- File: `krillnotes-desktop/src-tauri/tauri.conf.json`
