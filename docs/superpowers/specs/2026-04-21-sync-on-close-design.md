# Sync on Close — Design Spec

## Overview

When a user closes a workspace that has unsynchronized changes (pending operations for relay or folder-sync peers), the app intercepts the close and either prompts or auto-syncs, depending on a global setting.

## Global Setting

**Field**: `sync_on_close: String` in `AppSettings` (Rust) / `syncOnClose?: string` in `AppSettings` (TS).

| Value | Behavior |
|-------|----------|
| `"always"` | Auto-sync on close. Show syncing overlay with spinner. On error, show error state with Close Anyway / Cancel. |
| `"ask"` (default) | Show `SyncOnCloseDialog` with three buttons: Sync & Close, Close Without Syncing, Cancel. |
| `"never"` | Close immediately, no check, no prompt. |

**UI**: Dropdown in SettingsDialog General tab, labeled "Sync on Close" with options "Always sync", "Ask before closing", "Never sync".

## Close Interception Flow

### Backend (lib.rs)

1. Add `closing_windows: Arc<Mutex<HashSet<String>>>` to `AppState`.
2. In `on_window_event`, add `CloseRequested` handler:
   - Check if window label is in `closing_windows` set.
   - If yes: remove from set, allow close (do nothing — existing `Destroyed` handler runs).
   - If no: call `event.prevent_close()`, emit `krillnotes://close-requested` to the window.
3. New Tauri command `close_window(window)`:
   - Add window label to `closing_windows`.
   - Call `window.destroy()`.

### Frontend

A hook (in `App.tsx` or `useWorkspaceLifecycle.ts`) listens for `krillnotes://close-requested`:

1. Read `syncOnClose` from `get_settings()`.
2. Call `has_pending_sync_ops` to check for unsent operations.
3. Decision matrix:

| Setting | Pending ops? | Action |
|---------|-------------|--------|
| any | no | `close_window` immediately |
| `"never"` | yes | `close_window` immediately |
| `"always"` | yes | Show syncing overlay → `poll_sync` → close on success, error state on failure |
| `"ask"` | yes | Show `SyncOnCloseDialog` |

## SyncOnCloseDialog Component

### Props

```typescript
interface SyncOnCloseDialogProps {
  mode: "ask" | "syncing";  // "ask" shows prompt, "syncing" shows spinner (for "always" mode)
  onSyncAndClose: () => void;
  onCloseWithoutSync: () => void;
  onCancel: () => void;
}
```

### States

**Prompt** (mode="ask"): Message "This workspace has unsynchronized changes. Sync with peers before closing?" with buttons:
- **Sync & Close** (primary)
- **Close Without Syncing** (secondary)
- **Cancel** (secondary)

**Syncing**: Spinner + "Syncing..." text. All buttons disabled. Shown when user clicks "Sync & Close" or when the hook enters "always" mode (the same component is rendered directly in syncing state — no prompt is shown).

**Error**: Error message from `poll_sync` failure. Buttons:
- **Close Anyway** (secondary/warning)
- **Cancel** (secondary)

### Keyboard

- Escape → Cancel (dismiss dialog, window stays open)

### Styling

Follows existing dialog patterns: `fixed inset-0 bg-black/50` overlay, `bg-background border border-border rounded-lg` container, standard button classes.

## Changes by File

### Backend (Rust)

| File | Change |
|------|--------|
| `src-tauri/src/lib.rs` | Add `closing_windows` to `AppState`. Add `CloseRequested` handler. Register `close_window` command. |
| `src-tauri/src/settings.rs` | Add `sync_on_close: String` to `AppSettings` with `#[serde(default)]` defaulting to `"ask"`. |
| `src-tauri/src/commands/workspace.rs` | Add `close_window` command. |

### Frontend (React/TypeScript)

| File | Change |
|------|--------|
| `src/types.ts` | Add `syncOnClose?: string` to `AppSettings` interface. |
| `src/components/SyncOnCloseDialog.tsx` | New component. |
| `src/components/SettingsDialog.tsx` | Add "Sync on Close" dropdown to General tab. |
| `src/App.tsx` or `src/hooks/useWorkspaceLifecycle.ts` | Listen for `krillnotes://close-requested`, implement decision logic. |

### i18n (all 7 locales)

New keys (under `settings` and `syncOnClose` sections):

```
settings.syncOnClose           — "Sync on Close"
settings.syncOnCloseAlways     — "Always sync"
settings.syncOnCloseAsk        — "Ask before closing"
settings.syncOnCloseNever      — "Never sync"
syncOnClose.message            — "This workspace has unsynchronized changes. Sync with peers before closing?"
syncOnClose.syncAndClose       — "Sync & Close"
syncOnClose.closeWithoutSync   — "Close Without Syncing"
syncOnClose.cancel             — "Cancel"
syncOnClose.syncing            — "Syncing..."
syncOnClose.errorTitle         — "Sync failed"
syncOnClose.closeAnyway        — "Close Anyway"
```

## Peer Filtering

Only **relay** and **folder** peers are considered. Manual peers are excluded from both the pending-ops check and the sync itself. This is already the behavior of the existing infrastructure:

- `has_pending_ops_for_any_peer()` calls `get_active_sync_peers()`, which filters via `list_peers_by_channel_not("manual")`.
- `poll_sync()` builds its `SyncEngine` from the same active peer set.

No new filtering logic is needed — the existing commands already do the right thing.

## Not in Scope

- Per-peer sync selection (sync all peers at once via existing `poll_sync`)
- Changes to `krillnotes-core` (this is desktop UI only)
- Sync progress details (just a spinner, no per-peer breakdown)
