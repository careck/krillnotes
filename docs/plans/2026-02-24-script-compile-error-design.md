# Design: Script Compile Error on Save

**Date**: 2026-02-24
**Feature**: Show compile error when saving a broken Rhai script; do not save the script.

## Problem

When a user saves a Rhai script in the script editor that has a syntax or runtime error, the app fails silently:

- **Create flow**: `load_script()` result is only used to set `enabled = false`. The broken script is saved to the database (as disabled) and the frontend receives `Ok(UserScript)` — no error shown.
- **Update flow**: No compilation check at all. The broken script is always saved. `reload_scripts()` swallows errors with `eprintln!()` only.

The frontend already has a red error box that renders on `catch (err)` — it just never fires for compile failures.

## Approach: Reject on Compile Error (backend-only)

Return `Err(KrillnotesError::Scripting(...))` from both `create_user_script` and `update_user_script` when compilation fails. The script is not written to the database. The frontend `catch` block in `handleSave()` surfaces the Rhai error message in the existing red error box.

## Architecture

### Backend (`krillnotes-core/src/core/workspace.rs`)

**`create_user_script`**:
- Replace `let compile_ok = self.script_registry.load_script(source_code).is_ok();` with `self.script_registry.load_script(source_code)?;` (early return on error).
- Remove the `compile_ok` flag; always insert with `enabled = true` (we only reach the INSERT if compilation succeeded).
- Remove the `if !compile_ok { self.reload_scripts()?; }` cleanup block (no longer needed for the failure path).
- Add `self.reload_scripts()?` on the success path after the transaction (to restore clean registry state, since `load_script` mutates the registry during validation).

**`update_user_script`**:
- Add `self.script_registry.load_script(source_code)?;` before the DB transaction.
- On failure, return early (registry is dirty from the failed load, so call `reload_scripts()` first to restore previous state).
- On success, the existing `self.reload_scripts()?` at the end handles cleanup.

### Frontend (`krillnotes-desktop/src/components/ScriptManagerDialog.tsx`)

No changes needed. `handleSave()` already:
```typescript
} catch (err) {
  setError(`${err}`);
}
```
The error string from Rust is forwarded as-is and rendered in the red error box.

## Error Message Format

Rhai errors include position info:
```
Script error: syntax error near 'schema' (line 5, position 3)
```

No additional formatting needed.

## Data Integrity

- A script is only written to the database if it compiled and executed successfully.
- If compilation fails on update, the previously saved source code is untouched.
- `reload_scripts()` is always called after any mutation to keep the in-memory registry consistent with the database.
