# Script Load-Order Drag Reordering — Design

**Date:** 2026-02-24
**Status:** Approved

## Summary

Allow users to change the loading order of user scripts in the Script Manager dialog via drag handles. The visual order in the list is the source of truth; after every drag-drop a batch Rust command renumbers all scripts sequentially and reloads the script engine once.

## Approach

**Native HTML5 drag-and-drop** (no new dependencies) + a new **`reorder_all_user_scripts`** Tauri command that accepts the full ordered array of script IDs, renumbers them 1–N in a single DB pass, and calls `reload_scripts()` once.

This avoids adding a third-party D&D library (consistent with the tree D&D implementation) and avoids the N-reload waste of calling the existing single-script `reorder_user_script` command N times.

## Frontend — `ScriptManagerDialog.tsx`

- Add `GripVertical` (lucide-react) as a drag handle on the left of each script row.
- Mark each row `draggable={true}`.
- Track three pieces of local state: `dragIndex`, `dragOverIndex`, `isDragging`.
- Show a thin insertion-line indicator between rows during drag.
- On `onDrop`: reorder the local `scripts` array optimistically, then call `invoke('reorder_all_user_scripts', { scriptIds })` with IDs in new order.
- Remove the `#{script.loadOrder}` number badge — the visual order is now self-evident.

## Backend — `workspace.rs`

Add `reorder_all_user_scripts(&mut self, ids: &[String]) -> Result<()>`:

```rust
pub fn reorder_all_user_scripts(&mut self, ids: &[String]) -> Result<()> {
    let conn = self.connection();
    for (i, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE user_scripts SET load_order = ?1, modified_at = ?2 WHERE id = ?3",
            rusqlite::params![i as i32 + 1, now_millis(), id],
        )?;
    }
    self.reload_scripts()
}
```

One sequential DB update per script, one script reload at the end.

## Tauri Command — `lib.rs`

New command: `reorder_all_user_scripts(app, window, script_ids: Vec<String>) -> Result<()>`
Scoped to the window's active workspace, same pattern as all other script commands.

## What Does NOT Change

- The `reorder_user_script` single-script command remains (unused by UI but keep it).
- Database schema is unchanged (`load_order INTEGER` column already exists).
- Script loading query (`ORDER BY load_order ASC, created_at ASC`) is unchanged.
