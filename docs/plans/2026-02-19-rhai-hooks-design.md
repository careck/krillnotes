# Rhai Hooks Design

**Date:** 2026-02-19
**Status:** Approved

## Overview

Add a pre-save hook system to the Rhai scripting layer. Hooks let schema scripts register
functions that transform note data before it is written to the database. The initial use case
is the Contact schema automatically deriving `title` from `last_name` and `first_name`.

## Rhai Script API

Two host functions are registered in the Rhai engine: the existing `schema()` and a new `on_save()`.

```rhai
// contact.rhai
schema("Contact", #{
    fields: [...]
});

on_save("Contact", |note| {
    let last  = note.fields["last_name"];
    let first = note.fields["first_name"];
    note.title = last + ", " + first;
    note  // hook must return the (possibly modified) note map
});
```

`on_save` is a separate statement from `schema()` — it wires behaviour to a named schema.
The hook must return the note map (modified or unchanged).

## Note Map Shape

The map passed into the hook:

```rhai
#{
    id:        "550e8400-...",     // String, read-only by convention
    node_type: "Contact",         // String, read-only by convention
    title:     "current title",   // String, writable
    fields: #{
        "first_name": "John",     // Text  → Rhai String
        "last_name":  "Doe",      // Text  → Rhai String
        "birthdate":  "1990-05-12", // Date  → ISO 8601 String, or () if unset
        "email":      "j@d.com",    // Email → Rhai String
        // Number → Rhai f64, Boolean → Rhai bool
    }
}
```

On return, Rust reads `title` (String) and `fields` (Map), using the schema's
`FieldDefinition` list to parse each value back into the correct `FieldValue` variant.
Extra keys in the returned map are silently dropped.

## SchemaRegistry Changes (Rust)

```rust
struct HookEntry {
    fn_ptr: FnPtr,
    ast: AST,   // AST of the script that defined the closure
}
```

Added to `SchemaRegistry`:
- `hooks: Arc<Mutex<HashMap<String, HookEntry>>>`

### load_script change

`load_script()` changes from `engine.eval(script)` to:
1. `engine.compile(script)` — produces an `AST`
2. `engine.eval_ast(&ast)` — executes the script (registers schema + hook)
3. Store the `AST` alongside any `HookEntry` registered during evaluation

### New host function: on_save

```rust
// registered in engine setup
engine.register_fn("on_save", move |name: &str, fn_ptr: FnPtr| {
    // stores HookEntry { fn_ptr, ast: current_ast } into hooks map
});
```

Because `on_save` is called during script evaluation (inside `eval_ast`), the current AST
is made available to the registered closure via the `Arc<Mutex<...>>` shared state pattern
already used for schema registration.

### New public method: run_on_save_hook

```rust
pub fn run_on_save_hook(
    &self,
    schema_name: &str,
    id: &str,
    node_type: &str,
    title: &str,
    fields: &HashMap<String, FieldValue>,
) -> Result<Option<(String, HashMap<String, FieldValue>)>>
```

Returns `None` if no hook is registered for the given schema. Returns
`Some((new_title, new_fields))` with the hook's output otherwise.

## FieldValue ↔ Rhai Dynamic Conversion

| FieldValue variant | Rhai Dynamic |
|--------------------|--------------|
| `Text(s)` | `String` |
| `Number(n)` | `f64` |
| `Boolean(b)` | `bool` |
| `Date(Some(d))` | `String` (ISO 8601 `"YYYY-MM-DD"`) |
| `Date(None)` | `()` (Rhai unit) |
| `Email(s)` | `String` |

Conversion back from Dynamic uses the schema's `FieldDefinition` to determine the target
type. Unknown field names in the returned map are dropped.

## Save Pipeline Change

Only `update_note()` in `workspace.rs` calls the hook. `create_note()` is excluded because
all fields are empty defaults at creation time, which would produce malformed titles.

New steps in `update_note()`:

```
1. SELECT node_type FROM notes WHERE id=?    ← new query
2. schema_registry.run_on_save_hook(node_type, id, node_type, title, fields)
3. If Some((new_title, new_fields)) → replace title/fields with hook output
4. SQL UPDATE                                ← unchanged
5. Log operations                            ← unchanged
```

## Error Handling

| Situation | Behaviour |
|-----------|-----------|
| No hook registered for schema | `None` returned — save proceeds unchanged |
| Hook throws a Rhai runtime error | `Err(KrillnotesError::Scripting(...))` — save aborted |
| Hook returns wrong type | `Err(KrillnotesError::Scripting(...))` — save aborted |
| Extra field keys in returned map | Silently dropped |

## Out of Scope

- `on_create` / `on_delete` hooks (future work)
- Hook registration for user-defined scripts (future work, same mechanism)
- Hook execution order for multiple hooks on the same schema (not needed yet)
