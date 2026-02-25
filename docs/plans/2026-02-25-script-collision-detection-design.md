# Script Collision Detection — Design

**Date:** 2026-02-25
**Issue:** #6
**Branch:** `feat/script-collision-detection`

## Problem

When two scripts both call `schema("Contact", ...)`, the second silently overwrites the first. There is no detection, no error, and no user feedback. The winner is whoever happens to load last, which is non-deterministic from the user's perspective.

## Approach

### First-Script Wins, Second Gets an Error

When `schema()` is called during script loading, check whether that schema name is already registered. If it is, return a Rhai error. The error propagates out of `load_script()` and is collected.

Scripts already execute in deterministic order: `load_order ASC, created_at ASC`. So "first" is well-defined and stable.

### Owner Tracking

To produce a helpful error message ("already defined by 'Script A'"), the registry must remember which script registered each schema. A `schema_owners: HashMap<String, String>` (schema name → script name) is added to `ScriptRegistry` alongside the existing `schemas` map.

### Error Collection and Surfacing

`reload_scripts()` currently swallows errors to `eprintln!`. The approach:
- `reload_scripts()` returns `Vec<ScriptError>` (collected from all failed scripts)
- Workspace mutation methods (`update_user_script`, `delete_user_script`, etc.) bundle those errors into their return value
- Tauri commands pass them through so the frontend can display a toast/alert at the moment of saving

### Error Type

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ScriptError {
    pub script_name: String,
    pub message: String,
}
```

### No Partial Schema Registration

If a script's `schema()` call fails due to collision, the error is returned from the Rhai host function immediately. No schema, hooks, or any other state from that call is registered. The `load_script()` function already clears `current_loading_ast` on error, preventing stale hook entries.

## Decisions Not Made

- **UI display format:** Decided by the frontend team; the backend just returns `Vec<ScriptError>`.
- **Starter script collisions:** Starter scripts are bundled and controlled by the app — they should not collide in practice. No special handling needed now.
