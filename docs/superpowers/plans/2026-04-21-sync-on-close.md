# Sync on Close — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Intercept workspace window close, check for unsynchronized operations against relay/folder peers, and either prompt or auto-sync based on a global setting.

**Architecture:** Frontend-driven approach. Tauri's `CloseRequested` event prevents the close and emits to the React frontend. A hook reads the `syncOnClose` setting, checks pending ops via existing commands, then either closes, shows a dialog, or auto-syncs. A `closing_windows` set in `AppState` prevents re-interception when the frontend intentionally destroys the window.

**Tech Stack:** Rust/Tauri v2 (backend), React 19/TypeScript (frontend), i18next (i18n), Tailwind v4 (styling)

---

### Task 1: Add `sync_on_close` to AppSettings (Rust)

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/settings.rs:20-55`

- [ ] **Step 1: Add the field and default function**

In `settings.rs`, add a default function and the new field to `AppSettings`:

```rust
fn default_sync_on_close() -> String {
    "ask".to_string()
}
```

Add this field to the `AppSettings` struct after `undo_history_limit`:

```rust
    #[serde(default = "default_sync_on_close")]
    pub sync_on_close: String,
```

Add to the `Default` impl:

```rust
            sync_on_close: default_sync_on_close(),
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p krillnotes-desktop`
Expected: compiles with no errors. Existing `settings.json` files without `sync_on_close` will deserialize with default `"ask"`.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/settings.rs
git commit -m "feat: add sync_on_close field to AppSettings"
```

---

### Task 2: Add `closing_windows` to AppState and CloseRequested handler (Rust)

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:39-86` (AppState)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:181-199` (AppState init)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:200-241` (on_window_event)

- [ ] **Step 1: Add `closing_windows` field to AppState**

Add after the `pending_file_open` field in the `AppState` struct:

```rust
    /// Window labels that have been approved for closing by the frontend.
    /// When a label is in this set, the next `CloseRequested` event for
    /// that window is allowed through without interception.
    pub closing_windows: Arc<Mutex<HashSet<String>>>,
```

Add `use std::collections::HashSet;` to imports if not already present.

- [ ] **Step 2: Initialize the field in `.manage(AppState { ... })`**

Add after `pending_file_open: ...`:

```rust
    closing_windows: Arc::new(Mutex::new(HashSet::new())),
```

- [ ] **Step 3: Add CloseRequested handler in on_window_event**

In the `match event` block, add a new arm **before** the `Destroyed` arm:

```rust
        tauri::WindowEvent::CloseRequested { api, .. } => {
            let mut closing = state.closing_windows.lock().expect("Mutex poisoned");
            if closing.remove(&label) {
                // Frontend approved this close — let it proceed.
            } else {
                // Intercept: hold the window open and notify the frontend.
                api.prevent_close();
                let _ = window.emit("krillnotes://close-requested", ());
            }
        }
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p krillnotes-desktop`
Expected: compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: intercept CloseRequested and emit to frontend"
```

---

### Task 3: Add `close_window` Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/workspace.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (generate_handler)

- [ ] **Step 1: Add the command function**

At the end of `commands/workspace.rs`, add:

```rust
#[tauri::command]
pub fn close_window(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let label = window.label().to_string();
    state.closing_windows.lock().expect("Mutex poisoned").insert(label);
    window.destroy().map_err(|e| format!("Failed to close window: {e}"))
}
```

- [ ] **Step 2: Register in generate_handler**

In `lib.rs`, add `close_window` to the `tauri::generate_handler![...]` list, after `update_settings`:

```rust
    close_window,
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p krillnotes-desktop`
Expected: compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/workspace.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add close_window Tauri command"
```

---

### Task 4: Add `syncOnClose` to TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:166-173`

- [ ] **Step 1: Add the field**

Add `syncOnClose?: string;` to the `AppSettings` interface after `undoHistoryLimit`:

```typescript
export interface AppSettings {
  activeThemeMode?: string;
  lightTheme?: string;
  darkTheme?: string;
  language?: string;
  sharingIndicatorMode?: string;
  undoHistoryLimit?: number;
  syncOnClose?: string;
}
```

- [ ] **Step 2: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add syncOnClose to AppSettings TS interface"
```

---

### Task 5: Add i18n keys to all 7 locales

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`
- Modify: `krillnotes-desktop/src/i18n/locales/de.json`
- Modify: `krillnotes-desktop/src/i18n/locales/es.json`
- Modify: `krillnotes-desktop/src/i18n/locales/fr.json`
- Modify: `krillnotes-desktop/src/i18n/locales/ja.json`
- Modify: `krillnotes-desktop/src/i18n/locales/ko.json`
- Modify: `krillnotes-desktop/src/i18n/locales/zh.json`

- [ ] **Step 1: Add English keys**

In `en.json`, add to the `"settings"` section (after `"sharingIndicatorsOn"`):

```json
    "syncOnClose": "Sync on Close",
    "syncOnCloseAlways": "Always sync",
    "syncOnCloseAsk": "Ask before closing",
    "syncOnCloseNever": "Never sync"
```

Add a new top-level `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "This workspace has unsynchronized changes. Sync with peers before closing?",
    "syncAndClose": "Sync & Close",
    "closeWithoutSync": "Close Without Syncing",
    "cancel": "Cancel",
    "syncing": "Syncing…",
    "errorTitle": "Sync failed",
    "closeAnyway": "Close Anyway"
  }
```

- [ ] **Step 2: Add German keys**

In `de.json`, add to `"settings"`:

```json
    "syncOnClose": "Synchronisierung beim Schließen",
    "syncOnCloseAlways": "Immer synchronisieren",
    "syncOnCloseAsk": "Vor dem Schließen fragen",
    "syncOnCloseNever": "Nie synchronisieren"
```

Add `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "Dieser Arbeitsbereich hat nicht synchronisierte Änderungen. Vor dem Schließen mit Peers synchronisieren?",
    "syncAndClose": "Synchronisieren & Schließen",
    "closeWithoutSync": "Ohne Synchronisierung schließen",
    "cancel": "Abbrechen",
    "syncing": "Synchronisiere…",
    "errorTitle": "Synchronisierung fehlgeschlagen",
    "closeAnyway": "Trotzdem schließen"
  }
```

- [ ] **Step 3: Add Spanish keys**

In `es.json`, add to `"settings"`:

```json
    "syncOnClose": "Sincronizar al cerrar",
    "syncOnCloseAlways": "Sincronizar siempre",
    "syncOnCloseAsk": "Preguntar antes de cerrar",
    "syncOnCloseNever": "Nunca sincronizar"
```

Add `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "Este espacio de trabajo tiene cambios sin sincronizar. ¿Sincronizar con los pares antes de cerrar?",
    "syncAndClose": "Sincronizar y cerrar",
    "closeWithoutSync": "Cerrar sin sincronizar",
    "cancel": "Cancelar",
    "syncing": "Sincronizando…",
    "errorTitle": "Error de sincronización",
    "closeAnyway": "Cerrar de todos modos"
  }
```

- [ ] **Step 4: Add French keys**

In `fr.json`, add to `"settings"`:

```json
    "syncOnClose": "Synchroniser à la fermeture",
    "syncOnCloseAlways": "Toujours synchroniser",
    "syncOnCloseAsk": "Demander avant de fermer",
    "syncOnCloseNever": "Ne jamais synchroniser"
```

Add `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "Cet espace de travail contient des modifications non synchronisées. Synchroniser avec les pairs avant de fermer ?",
    "syncAndClose": "Synchroniser et fermer",
    "closeWithoutSync": "Fermer sans synchroniser",
    "cancel": "Annuler",
    "syncing": "Synchronisation…",
    "errorTitle": "Échec de la synchronisation",
    "closeAnyway": "Fermer quand même"
  }
```

- [ ] **Step 5: Add Japanese keys**

In `ja.json`, add to `"settings"`:

```json
    "syncOnClose": "閉じる時に同期",
    "syncOnCloseAlways": "常に同期",
    "syncOnCloseAsk": "閉じる前に確認",
    "syncOnCloseNever": "同期しない"
```

Add `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "このワークスペースには同期されていない変更があります。閉じる前にピアと同期しますか？",
    "syncAndClose": "同期して閉じる",
    "closeWithoutSync": "同期せずに閉じる",
    "cancel": "キャンセル",
    "syncing": "同期中…",
    "errorTitle": "同期に失敗しました",
    "closeAnyway": "そのまま閉じる"
  }
```

- [ ] **Step 6: Add Korean keys**

In `ko.json`, add to `"settings"`:

```json
    "syncOnClose": "닫을 때 동기화",
    "syncOnCloseAlways": "항상 동기화",
    "syncOnCloseAsk": "닫기 전에 확인",
    "syncOnCloseNever": "동기화 안 함"
```

Add `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "이 워크스페이스에 동기화되지 않은 변경 사항이 있습니다. 닫기 전에 피어와 동기화하시겠습니까?",
    "syncAndClose": "동기화 후 닫기",
    "closeWithoutSync": "동기화 없이 닫기",
    "cancel": "취소",
    "syncing": "동기화 중…",
    "errorTitle": "동기화 실패",
    "closeAnyway": "그래도 닫기"
  }
```

- [ ] **Step 7: Add Chinese keys**

In `zh.json`, add to `"settings"`:

```json
    "syncOnClose": "关闭时同步",
    "syncOnCloseAlways": "始终同步",
    "syncOnCloseAsk": "关闭前询问",
    "syncOnCloseNever": "从不同步"
```

Add `"syncOnClose"` section:

```json
  "syncOnClose": {
    "message": "此工作区有未同步的更改。关闭前是否与对等方同步？",
    "syncAndClose": "同步并关闭",
    "closeWithoutSync": "不同步直接关闭",
    "cancel": "取消",
    "syncing": "同步中…",
    "errorTitle": "同步失败",
    "closeAnyway": "仍然关闭"
  }
```

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/*.json
git commit -m "feat: add sync-on-close i18n keys for all 7 locales"
```

---

### Task 6: Create SyncOnCloseDialog component

**Files:**
- Create: `krillnotes-desktop/src/components/SyncOnCloseDialog.tsx`

- [ ] **Step 1: Create the component**

```tsx
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';

interface SyncOnCloseDialogProps {
  mode: 'ask' | 'syncing';
  syncError: string | null;
  onSyncAndClose: () => void;
  onCloseWithoutSync: () => void;
  onCancel: () => void;
}

export default function SyncOnCloseDialog({
  mode,
  syncError,
  onSyncAndClose,
  onCloseWithoutSync,
  onCancel,
}: SyncOnCloseDialogProps) {
  const { t } = useTranslation();

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onCancel();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onCancel]);

  const isSyncing = mode === 'syncing' && !syncError;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border p-6 rounded-lg w-[420px]">
        {syncError ? (
          <>
            <h3 className="text-lg font-semibold mb-3">{t('syncOnClose.errorTitle')}</h3>
            <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {syncError}
            </div>
            <div className="flex justify-end gap-2">
              <button
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                onClick={onCancel}
              >
                {t('syncOnClose.cancel')}
              </button>
              <button
                className="px-4 py-2 bg-orange-500 text-white rounded hover:bg-orange-600"
                onClick={onCloseWithoutSync}
              >
                {t('syncOnClose.closeAnyway')}
              </button>
            </div>
          </>
        ) : isSyncing ? (
          <div className="flex flex-col items-center py-4 gap-3">
            <div className="w-6 h-6 border-2 border-primary border-t-transparent rounded-full animate-spin" />
            <span className="text-sm text-muted-foreground">{t('syncOnClose.syncing')}</span>
          </div>
        ) : (
          <>
            <h3 className="text-lg font-semibold mb-3">{t('settings.syncOnClose')}</h3>
            <p className="text-sm text-muted-foreground mb-5">{t('syncOnClose.message')}</p>
            <div className="flex justify-end gap-2">
              <button
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                onClick={onCancel}
              >
                {t('syncOnClose.cancel')}
              </button>
              <button
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                onClick={onCloseWithoutSync}
              >
                {t('syncOnClose.closeWithoutSync')}
              </button>
              <button
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
                onClick={onSyncAndClose}
              >
                {t('syncOnClose.syncAndClose')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify TypeScript compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors (component is not yet wired in, but should type-check on its own).

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/SyncOnCloseDialog.tsx
git commit -m "feat: add SyncOnCloseDialog component"
```

---

### Task 7: Add close-interception hook and wire into App.tsx

**Files:**
- Create: `krillnotes-desktop/src/hooks/useSyncOnClose.ts`
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Create the useSyncOnClose hook**

```typescript
import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppSettings } from '../types';

type SyncOnCloseState =
  | { phase: 'idle' }
  | { phase: 'asking' }
  | { phase: 'syncing'; error: string | null };

export function useSyncOnClose() {
  const [state, setState] = useState<SyncOnCloseState>({ phase: 'idle' });
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    const unlisten = listen('krillnotes://close-requested', async () => {
      if (stateRef.current.phase !== 'idle') return;

      try {
        const settings = await invoke<AppSettings>('get_settings');
        const mode = settings.syncOnClose ?? 'ask';

        if (mode === 'never') {
          await invoke('close_window');
          return;
        }

        const hasPending = await invoke<boolean>('has_pending_sync_ops');
        if (!hasPending) {
          await invoke('close_window');
          return;
        }

        if (mode === 'always') {
          setState({ phase: 'syncing', error: null });
          try {
            await invoke('poll_sync');
            await invoke('close_window');
          } catch (err) {
            setState({ phase: 'syncing', error: String(err) });
          }
          return;
        }

        // mode === 'ask'
        setState({ phase: 'asking' });
      } catch {
        await invoke('close_window');
      }
    });

    return () => { unlisten.then(fn => fn()); };
  }, []);

  const handleSyncAndClose = useCallback(async () => {
    setState({ phase: 'syncing', error: null });
    try {
      await invoke('poll_sync');
      await invoke('close_window');
    } catch (err) {
      setState({ phase: 'syncing', error: String(err) });
    }
  }, []);

  const handleCloseWithoutSync = useCallback(async () => {
    setState({ phase: 'idle' });
    await invoke('close_window');
  }, []);

  const handleCancel = useCallback(() => {
    setState({ phase: 'idle' });
  }, []);

  return {
    syncOnCloseState: state,
    handleSyncAndClose,
    handleCloseWithoutSync,
    handleCancel,
  };
}
```

- [ ] **Step 2: Wire into App.tsx**

Add the import at the top of `App.tsx`:

```typescript
import { useSyncOnClose } from './hooks/useSyncOnClose';
import SyncOnCloseDialog from './components/SyncOnCloseDialog';
```

Inside the `App` component, after the existing hook calls (around line 76), add:

```typescript
const {
  syncOnCloseState,
  handleSyncAndClose,
  handleCloseWithoutSync,
  handleCancel,
} = useSyncOnClose();
```

In the JSX, add the dialog rendering before the closing `</>` of the outermost fragment (after all other dialogs):

```tsx
{syncOnCloseState.phase !== 'idle' && (
  <SyncOnCloseDialog
    mode={syncOnCloseState.phase === 'asking' ? 'ask' : 'syncing'}
    syncError={syncOnCloseState.phase === 'syncing' ? syncOnCloseState.error : null}
    onSyncAndClose={handleSyncAndClose}
    onCloseWithoutSync={handleCloseWithoutSync}
    onCancel={handleCancel}
  />
)}
```

- [ ] **Step 3: Verify TypeScript compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/hooks/useSyncOnClose.ts krillnotes-desktop/src/App.tsx
git commit -m "feat: wire close-interception hook and dialog into App"
```

---

### Task 8: Add "Sync on Close" dropdown to SettingsDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/SettingsDialog.tsx`

- [ ] **Step 1: Add state for the setting**

In `SettingsDialog`, add state alongside existing state variables (near `undoLimit` state):

```typescript
const [syncOnClose, setSyncOnClose] = useState('ask');
```

- [ ] **Step 2: Load the value on open**

In the `useEffect` that loads settings when `isOpen` becomes true, add after the `undoLimit` setter:

```typescript
setSyncOnClose(s.syncOnClose ?? 'ask');
```

- [ ] **Step 3: Include in the save patch**

In `handleSave`, add `syncOnClose` to the `patch` object passed to `update_settings`:

```typescript
await invoke('update_settings', {
  patch: {
    language,
    sharingIndicatorMode,
    undoHistoryLimit: undoLimit ?? 50,
    syncOnClose,
  },
});
```

- [ ] **Step 4: Add the dropdown UI in the General tab**

After the "Undo history limit" section in the General tab (after its closing `</div>`), add:

```tsx
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
```

- [ ] **Step 5: Verify TypeScript compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/SettingsDialog.tsx
git commit -m "feat: add Sync on Close dropdown to Settings General tab"
```

---

### Task 9: Manual testing

**Files:** None (testing only)

- [ ] **Step 1: Start dev server**

Run: `cd krillnotes-desktop && npm run tauri dev`

- [ ] **Step 2: Test "ask" mode (default)**

1. Open a workspace that has peers configured (relay or folder).
2. Make a change (create a note, edit a field).
3. Close the workspace window.
4. Expected: `SyncOnCloseDialog` appears with "Sync & Close", "Close Without Syncing", "Cancel".
5. Click "Cancel" — dialog dismisses, window stays open.
6. Close again, click "Close Without Syncing" — window closes without syncing.
7. Reopen, make a change, close again, click "Sync & Close" — spinner appears, sync runs, window closes.

- [ ] **Step 3: Test "always" mode**

1. Open Settings → General → set "Sync on Close" to "Always sync" → Save.
2. Open a workspace with peers, make a change.
3. Close the window.
4. Expected: spinner overlay appears briefly, sync runs, window closes automatically.

- [ ] **Step 4: Test "never" mode**

1. Open Settings → General → set "Sync on Close" to "Never sync" → Save.
2. Open a workspace with peers, make a change.
3. Close the window.
4. Expected: window closes immediately with no prompt.

- [ ] **Step 5: Test with no pending ops**

1. Set mode back to "ask".
2. Open a workspace, sync manually (File > Sync Now), then close.
3. Expected: window closes immediately with no prompt (no pending ops).

- [ ] **Step 6: Test with no peers**

1. Open a workspace with no peers configured (or only manual peers).
2. Make a change, close.
3. Expected: window closes immediately (has_pending_sync_ops returns false for manual-only).

- [ ] **Step 7: Test error state**

1. Set mode to "ask". Configure a relay peer with invalid credentials or disconnect network.
2. Make a change, close, click "Sync & Close".
3. Expected: spinner, then error message with "Close Anyway" and "Cancel" buttons.

- [ ] **Step 8: Test Escape key**

1. Trigger the dialog (ask mode with pending ops).
2. Press Escape.
3. Expected: dialog dismisses, window stays open.

- [ ] **Step 9: Verify i18n**

1. Change language to each of the 7 supported languages.
2. Open Settings — verify "Sync on Close" dropdown labels are translated.
3. Trigger the close dialog — verify all dialog strings are translated.
