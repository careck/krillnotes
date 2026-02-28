# Design: Operations Log Sync Gate

**Issue:** #48
**Date:** 2026-02-28
**Status:** Approved

## Summary

Gate the operations log behind a per-workspace sync flag. Since sync is not yet implemented, the flag is always `false` for now. This delivers the correct immediate behavior (no log writes, greyed-out menu) while laying the groundwork for future per-workspace sync configuration.

## Goals

- Stop writing to the operations log for local-only workspaces
- Grey out the "Operations Log" menu item when sync is off
- Show a "Sync" placeholder tab in the Settings dialog (visible but locked)
- Keep the `operations` DB table intact
- Make enabling sync per-workspace a minimal future change

## Non-Goals

- Actual sync implementation
- Per-workspace settings storage (deferred until sync is real)
- Enabling or disabling sync at runtime

## Architecture

### `Workspace` — `Option<OperationLog>`

Change `operation_log: OperationLog` to `operation_log: Option<OperationLog>` on the `Workspace` struct in `krillnotes-core/src/core/workspace.rs`.

- `None` → sync off, no writes
- `Some(log)` → sync on, full logging

`create()` and `open()` always produce `None` for now.

All call sites:
- `self.operation_log.log(...)` → `if let Some(log) = &self.operation_log { log.log(&tx, &op)?; }`
- `self.operation_log.purge_if_needed(...)` → same guard
- `list_operations()` → returns `Ok(vec![])` when `None`
- `purge_all_operations()` → returns `Ok(0)` when `None`

### Menu — permanently disabled

Remove `view_operations_log` from the `workspace_items` list in `lib.rs` (items that auto-enable when a workspace opens). The item stays permanently disabled. A future sync-on code path can explicitly enable it per workspace.

### Settings UI — Sync tab (placeholder)

Add a **"Sync"** tab to the existing `SettingsDialog.tsx`. Contents:

- A disabled toggle labelled **"Sync"**
- A short note: *"Sync keeps your notes up to date across devices. Coming soon."*

No backend storage. No new `AppSettings` fields. The tab is purely visual.

## Future Extension Point

When per-workspace sync is implemented:

1. Add `sync_enabled: bool` to `workspace_meta` (workspace DB)
2. Read it in `open()` / `create()` and pass `Some(OperationLog::new(...))` or `None`
3. Unlock the Settings Sync tab and wire it to the workspace setting
4. Enable the Operations Log menu item when `sync_enabled = true`

## Files Changed

| File | Change |
|------|--------|
| `krillnotes-core/src/core/workspace.rs` | `operation_log: Option<OperationLog>`; guard all call sites |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Remove `view_operations_log` from `workspace_items` |
| `krillnotes-desktop/src/components/SettingsDialog.tsx` | Add "Sync" tab with disabled toggle |
