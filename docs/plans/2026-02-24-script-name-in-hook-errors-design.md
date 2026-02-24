# Design: Script Name in Hook Runtime Errors

**Date:** 2026-02-24
**Status:** Approved

## Summary

When an `on_save` or `on_view` hook throws a runtime error, the error popup currently includes the hook type and the Rhai error message (with line number), but not the name of the script the hook came from. This makes it hard for users to know which script to look at.

The fix adds the script name to the error message, e.g.:

```
on_save hook error in 'Contact Manager': Undefined variable `x` at line 5
```

## Approach

Follow the existing `current_loading_ast: Arc<Mutex<Option<AST>>>` pattern already used in `ScriptRegistry`.

### 1. Extend `HookEntry`

Add `script_name: String` to `HookEntry` in `krillnotes-core/src/core/scripting/schema.rs`:

```rust
pub struct HookEntry {
    pub fn_ptr: FnPtr,
    pub ast: AST,
    pub script_name: String,
}
```

### 2. Thread script name through loading

Add to `ScriptRegistry` in `krillnotes-core/src/core/scripting/mod.rs`:

```rust
current_loading_script_name: Arc<Mutex<Option<String>>>,
```

In `load_script()`, set this to the script's `name` before evaluating and clear it after — mirroring how `current_loading_ast` works. The `schema()` host function closure captures this Arc and reads the name when constructing `HookEntry`.

### 3. Update error messages

In `run_on_save_hook()` and `run_on_view_hook()` in `schema.rs`, change the error format strings:

- Before: `"on_save hook error: {e}"`
- After: `"on_save hook error in '{script_name}': {e}"`

Same for `on_view`.

## Scope

- Changes confined to `krillnotes-core/src/core/scripting/` (two files: `mod.rs`, `schema.rs`)
- No frontend changes
- No changes to error propagation path
- The "two scripts registering the same schema name" collision is a known gap but out of scope here

## Architecture Decision

A post-load annotation approach was considered (scan hooks after each script load, backfill names) but rejected in favour of the Arc pattern, which is already established in this codebase and is more explicit about the data flow.
