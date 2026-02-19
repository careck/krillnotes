# Scripting Refactor Design

**Date:** 2026-02-20

**Goal:** Separate the scripting engine, schema storage, and hook storage into distinct types with clear responsibilities. `SchemaRegistry` currently owns all three concerns; this refactor splits them cleanly.

---

## Problem

`SchemaRegistry` is doing too much:
- Owns the Rhai `Engine`
- Loads and compiles scripts
- Stores schemas
- Stores hooks and executes them

As the hook system grows (more event types), the class will become a maintenance burden. Schema and hook concerns are unrelated and should not share a container.

---

## Design

### Three types, one public façade

```
ScriptRegistry (pub)                     — replaces SchemaRegistry as the public type
  ├── engine: Engine                     (private)
  ├── current_loading_ast: …             (private)
  ├── schema_registry: SchemaRegistry    (private)
  └── hook_registry: HookRegistry        (pub via accessor)
```

`Workspace` owns `script_registry: ScriptRegistry` (field renamed from `registry`).

---

### `SchemaRegistry` (private)

Pure data store. No Rhai imports. Knows only `Schema` and `FieldDefinition`.

**Responsibilities:**
- Store and retrieve `Schema` objects by name
- List registered schema names

**Methods:** `insert`, `get`, `list`

---

### `HookRegistry` (pub)

Pure data store plus typed execution methods. Receives a `&Engine` from the caller — it does not own one.

**Responsibilities:**
- Store `HookEntry` objects (Rhai `FnPtr` + `AST`) keyed by schema name, per event type
- Execute stored hooks given an engine and typed input

**Public methods:**
- `has_hook(schema_name: &str) -> bool`
- `run_on_save_hook(&self, engine: &Engine, schema: &Schema, note_id, node_type, title, fields) -> Result<Option<(String, HashMap<String, FieldValue>)>>`

**Adding a new hook event type** (`on_create`, `on_delete`, etc.):
- Add a new collection to `HookRegistry` (e.g. `on_create_hooks`)
- Add the typed execution method to `HookRegistry`
- Add the typed delegation method to `ScriptRegistry`
- Register the Rhai host function in `ScriptRegistry::new()`

---

### `ScriptRegistry` (pub)

Orchestrator. Owns the `Engine`. Responsible for registering Rhai host functions, loading scripts, and delegating queries/execution to the sub-registries.

**Public methods:**
- `new() -> Result<Self>` — creates engine, registers `schema(...)` and `on_save(...)` host functions, loads system scripts
- `load_script(&mut self, script: &str) -> Result<()>`
- `get_schema(name: &str) -> Result<Schema>`
- `list_types() -> Result<Vec<String>>`
- `hooks(&self) -> &HookRegistry` — accessor for state queries (`has_hook`, future introspection)
- `run_on_save_hook(schema_name, note_id, node_type, title, fields) -> Result<Option<…>>` — typed delegation: calls `self.hook_registry.run_on_save_hook(&self.engine, schema, ...)`

**Trade-off acknowledged:** `ScriptRegistry` has a thin typed wrapper method per hook event that mirrors the method on `HookRegistry`. This duplication is intentional — it is type-safe and explicit. Deduplication can be addressed in a future refactor.

---

## File Layout

Split `scripting.rs` into a module directory:

```
krillnotes-core/src/core/scripting/
  mod.rs      — ScriptRegistry, new(), load_script(), host-fn registration, delegation methods
  schema.rs   — SchemaRegistry (private struct), Schema (pub), FieldDefinition (pub)
  hooks.rs    — HookRegistry (pub), HookEntry (private), run_on_save_hook, field_value_to_dynamic, dynamic_to_field_value
```

---

## Crate Re-exports

```rust
// Before
pub use scripting::{FieldDefinition, Schema, SchemaRegistry};

// After
pub use scripting::{FieldDefinition, HookRegistry, Schema, ScriptRegistry};
```

`SchemaRegistry` is no longer exported. `Schema` and `FieldDefinition` remain public unchanged.

---

## Workspace Changes

- Field rename: `registry: SchemaRegistry` → `script_registry: ScriptRegistry`
- All `self.registry.*` call sites updated to `self.script_registry.*`
- `update_note` hook call: `self.registry.run_on_save_hook(...)` → `self.script_registry.run_on_save_hook(...)`

---

## What Does Not Change

- `Schema`, `FieldDefinition` — unchanged public types
- `HookEntry` — stays private, moves to `hooks.rs`
- `field_value_to_dynamic`, `dynamic_to_field_value` — stay private free functions, move to `hooks.rs`
- All existing tests — updated import paths only, no logic changes
- Tauri command layer (`lib.rs`) — updates `SchemaRegistry` → `ScriptRegistry` in type references only
