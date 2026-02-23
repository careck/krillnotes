# Design: Move Schema-Bound Hooks Inside `schema()` Definition

**Date:** 2026-02-24
**Status:** Approved

## Summary

`on_save` and `on_view` hooks are currently registered as standalone top-level Rhai calls
(`on_save("Name", |note| {...})`) and stored in a separate `HookRegistry`. This creates
load-order ambiguity and allows multiple conflicting registrations for the same schema.

This change moves both hooks inside the `schema()` Rhai call and relocates their storage
from `HookRegistry` into `SchemaRegistry`, making each schema fully self-contained.

## Two-Tier Hook Architecture

The refactor establishes a clear two-tier system:

| Tier | Hooks | Owned by | Registered via |
|------|-------|----------|----------------|
| Schema-bound | `on_save`, `on_view` | `SchemaRegistry` | `schema()` call |
| Global / lifecycle | `on_load`, `on_export`, menu hooks (future) | `HookRegistry` | standalone functions |

`HookRegistry` is stripped of `on_save_hooks` and `on_view_hooks` and kept as an
empty-but-ready struct for future global/lifecycle hooks.

## Rhai Syntax Change

### Before

```rhai
schema("Contact", #{
    title_can_edit: false,
    fields: [ ... ]
});

on_save("Contact", |note| {
    note.title = note.fields["last_name"] + ", " + note.fields["first_name"];
    note
});
```

### After

```rhai
schema("Contact", #{
    title_can_edit: false,
    fields: [ ... ],
    on_save: |note| {
        note.title = note.fields["last_name"] + ", " + note.fields["first_name"];
        note
    }
});
```

Key differences:
- Hooks are defined as map keys `on_save:` and `on_view:` inside the `schema()` map.
- The closure signature changes from `|note|` with schema name as first arg to just `|note|`
  (schema identity is implicit).
- The standalone `on_save()` and `on_view()` Rhai host functions are removed entirely.

## Rust Architecture Changes

### `SchemaRegistry` (`schema.rs`)

Gains two parallel side-table maps populated during `schema()` parsing:

```rust
pub(super) struct SchemaRegistry {
    schemas:       Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,  // moved from HookRegistry
    on_view_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,  // moved from HookRegistry
}
```

Gains execution methods (moved from `HookRegistry`):
- `run_on_save_hook(engine, schema, note_id, node_type, title, fields)`
- `run_on_view_hook(engine, note_map)`
- `has_view_hook(schema_name)`

### `HookRegistry` (`hooks.rs`)

Loses `on_save_hooks` and `on_view_hooks`. The struct remains as a placeholder for future
global/lifecycle hooks. `HookEntry`, field conversion utilities, and display helper
registration remain in `hooks.rs`.

### `ScriptRegistry` (`mod.rs`)

- `schema()` host function: after parsing fields/flags, extracts optional `on_save` and
  `on_view` FnPtrs from the map. Uses the `current_loading_ast` arc (already available)
  to store a `HookEntry` in `SchemaRegistry`'s hook maps.
- Removes registration of standalone `on_save()` and `on_view()` Rhai host functions.
- Delegation methods `run_on_save_hook`, `run_on_view_hook`, `has_view_hook` now route
  through `SchemaRegistry` instead of `HookRegistry`.

### `workspace.rs` / `lib.rs` (tauri)

No changes to call sites — the public API surface of `ScriptRegistry` is preserved.

## Scripts to Update

All 7 system scripts must be updated. 6 have `on_save` hooks, 1 has an `on_view` hook:

| File | Hook(s) | Schema(s) affected |
|------|---------|-------------------|
| `00_text_note.rhai` | none | — |
| `01_contact.rhai` | `on_save`, `on_view` | `Contact`, `ContactsFolder` |
| `02_task.rhai` | `on_save` | `Task` |
| `03_project.rhai` | `on_save` | `Project` |
| `04_book.rhai` | `on_save` | `Book` |
| `05_recipe.rhai` | `on_save` | `Recipe` |
| `06_product.rhai` | `on_save` | `Product` |

## Migration

This is a clean break — no backward compatibility with the old `on_save()`/`on_view()`
standalone syntax. The app is in prototype stage with no deployed user data.
