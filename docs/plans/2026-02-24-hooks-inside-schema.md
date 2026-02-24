# Hooks Inside Schema — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move `on_save` / `on_view` hook storage from `HookRegistry` into `SchemaRegistry`, change the Rhai syntax so hooks are defined inside `schema()`, and update all system scripts.

**Architecture:** `SchemaRegistry` gains two parallel `HashMap<String, HookEntry>` side-tables (on_save, on_view) populated by the `schema()` host function. `HookRegistry` is stripped of those maps and kept as an empty placeholder for future global/lifecycle hooks. The `schema()` host function extracts `FnPtr` closures from the Rhai map and inserts them alongside the schema. Standalone `on_save()` / `on_view()` Rhai host functions are removed.

**Tech Stack:** Rust, Rhai scripting engine, `Arc<Mutex<HashMap>>` for shared mutable state across closures.

---

## Task 1: Create git worktree

**Files:**
- Worktree: `Krillnotes/.worktrees/feat/hooks-inside-schema/`

**Step 1: Create the worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add \
  .worktrees/feat/hooks-inside-schema \
  -b feat/hooks-inside-schema
```

Expected: `Preparing worktree (new branch 'feat/hooks-inside-schema')`

**Step 2: Verify worktree exists**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/hooks-inside-schema/krillnotes-core/src/core/scripting/
```

Expected: `display_helpers.rs  hooks.rs  mod.rs  schema.rs`

All remaining work happens inside the worktree path.

---

## Task 2: Extend `schema.rs` — add HookEntry, conversion utilities, hook maps, execution methods

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs`

This task moves `HookEntry`, `field_value_to_dynamic`, and `dynamic_to_field_value` from `hooks.rs` into `schema.rs`, then adds hook storage and execution to `SchemaRegistry`.

**Step 1: Write a failing test confirming new syntax is not yet supported**

Add at the top of the test module in `mod.rs` (so we can see it fail before changing schema.rs):

```rust
#[test]
fn test_on_save_inside_schema_sets_title() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Person", #{
            fields: [
                #{ name: "first", type: "text", required: false },
                #{ name: "last",  type: "text", required: false },
            ],
            on_save: |note| {
                note.title = note.fields["last"] + ", " + note.fields["first"];
                note
            }
        });
    "#).unwrap();

    let mut fields = std::collections::HashMap::new();
    fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
    fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

    let result = registry
        .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
        .unwrap();

    assert!(result.is_some());
    let (new_title, _) = result.unwrap();
    assert_eq!(new_title, "Doe, John");
}

#[test]
fn test_on_view_inside_schema_returns_html() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Folder", #{
            fields: [],
            on_view: |note| {
                text("hello from view")
            }
        });
    "#).unwrap();

    use crate::Note;
    let note = Note {
        id: "n1".to_string(), node_type: "Folder".to_string(),
        title: "F".to_string(), parent_id: None, position: 0,
        created_at: 0, modified_at: 0, created_by: 0, modified_by: 0,
        fields: std::collections::HashMap::new(), is_expanded: false,
    };
    let ctx = QueryContext {
        notes_by_id: std::collections::HashMap::new(),
        children_by_id: std::collections::HashMap::new(),
        notes_by_type: std::collections::HashMap::new(),
    };
    let html = registry.run_on_view_hook(&note, ctx).unwrap();
    assert!(html.is_some());
    assert!(html.unwrap().contains("hello from view"));
}
```

**Step 2: Run tests to confirm they fail**

```bash
cargo test -p krillnotes-core test_on_save_inside_schema 2>&1 | tail -20
cargo test -p krillnotes-core test_on_view_inside_schema 2>&1 | tail -20
```

Expected: both FAIL — `run_on_save_hook` returns `None` (no hook registered).

**Step 3: Replace `schema.rs` with extended version**

Replace the entire file at `krillnotes-core/src/core/scripting/schema.rs`. The key additions are at the top (new imports + HookEntry + conversion utils) and at the bottom (extended SchemaRegistry):

```rust
//! Schema definitions and the private schema store for Krillnotes note types.

use crate::{FieldValue, KrillnotesError, Result};
use chrono::NaiveDate;
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A stored hook closure: the Rhai FnPtr and the AST it was compiled from.
///
/// Moved here from `hooks.rs` so `SchemaRegistry` can own schema-bound hooks
/// without a circular module dependency.
#[derive(Clone, Debug)]
pub(super) struct HookEntry {
    pub(super) fn_ptr: FnPtr,
    pub(super) ast: AST,
}

// ── FieldValue ↔ Rhai Dynamic conversions ──────────────────────────────────

/// Converts a [`FieldValue`] to a Rhai [`Dynamic`] for passing into hook closures.
///
/// `Date(None)` maps to `Dynamic::UNIT` (`()`).
/// `Date(Some(d))` maps to an ISO 8601 string `"YYYY-MM-DD"`.
/// All other variants map to their natural Rhai primitive.
pub(crate) fn field_value_to_dynamic(fv: &FieldValue) -> Dynamic {
    match fv {
        FieldValue::Text(s)    => Dynamic::from(s.clone()),
        FieldValue::Number(n)  => Dynamic::from(*n),
        FieldValue::Boolean(b) => Dynamic::from(*b),
        FieldValue::Date(None) => Dynamic::UNIT,
        FieldValue::Date(Some(d)) => Dynamic::from(d.format("%Y-%m-%d").to_string()),
        FieldValue::Email(s)   => Dynamic::from(s.clone()),
    }
}

/// Converts a Rhai [`Dynamic`] back to a [`FieldValue`] given the field's type string.
fn dynamic_to_field_value(d: Dynamic, field_type: &str) -> Result<FieldValue> {
    match field_type {
        "text" | "textarea" => {
            if d.is_unit() { return Ok(FieldValue::Text(String::new())); }
            let s = d.try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("text field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "number" => {
            if d.is_unit() { return Ok(FieldValue::Number(0.0)); }
            let n = d.try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("number field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        "boolean" => {
            if d.is_unit() { return Ok(FieldValue::Boolean(false)); }
            let b = d.try_cast::<bool>()
                .ok_or_else(|| KrillnotesError::Scripting("boolean field must be a bool".into()))?;
            Ok(FieldValue::Boolean(b))
        }
        "date" => {
            if d.is_unit() { return Ok(FieldValue::Date(None)); }
            let s = d.try_cast::<String>().ok_or_else(|| {
                KrillnotesError::Scripting("date field must be a string or ()".into())
            })?;
            let nd = NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|e| {
                KrillnotesError::Scripting(format!("invalid date '{s}': {e}"))
            })?;
            Ok(FieldValue::Date(Some(nd)))
        }
        "email" => {
            if d.is_unit() { return Ok(FieldValue::Email(String::new())); }
            let s = d.try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("email field must be a string".into()))?;
            Ok(FieldValue::Email(s))
        }
        "select" => {
            if d.is_unit() { return Ok(FieldValue::Text(String::new())); }
            let s = d.try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("select field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "rating" => {
            if d.is_unit() { return Ok(FieldValue::Number(0.0)); }
            let n = d.try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("rating field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        _ => Ok(FieldValue::Text(String::new())),
    }
}
```

Then keep the existing `FieldDefinition`, `Schema`, and `Schema::impl` block unchanged (lines 9–210 of the current file), and replace the `SchemaRegistry` block (lines 212–255) with:

```rust
/// Private store for registered schemas and their schema-bound hooks.
#[derive(Debug)]
pub(super) struct SchemaRegistry {
    schemas:       Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
    on_view_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self {
            schemas:       Arc::new(Mutex::new(HashMap::new())),
            on_save_hooks: Arc::new(Mutex::new(HashMap::new())),
            on_view_hooks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(super) fn schemas_arc(&self) -> Arc<Mutex<HashMap<String, Schema>>> {
        Arc::clone(&self.schemas)
    }

    pub(super) fn on_save_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_save_hooks)
    }

    pub(super) fn on_view_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_view_hooks)
    }

    pub(super) fn get(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .map_err(|_| KrillnotesError::Scripting("Schema registry lock poisoned".to_string()))?
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    pub(super) fn exists(&self, name: &str) -> bool {
        self.schemas.lock().unwrap().contains_key(name)
    }

    pub(super) fn list(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
    }

    pub(super) fn all(&self) -> HashMap<String, Schema> {
        self.schemas.lock().unwrap().clone()
    }

    /// Removes all registered schemas and hooks.
    pub(super) fn clear(&self) {
        self.schemas.lock().unwrap().clear();
        self.on_save_hooks.lock().unwrap().clear();
        self.on_view_hooks.lock().unwrap().clear();
    }

    pub(super) fn has_hook(&self, schema_name: &str) -> bool {
        self.on_save_hooks.lock().unwrap().contains_key(schema_name)
    }

    pub(super) fn has_view_hook(&self, schema_name: &str) -> bool {
        self.on_view_hooks.lock().unwrap().contains_key(schema_name)
    }

    /// Runs the on_save hook for `schema_name`, if registered.
    ///
    /// Returns `Ok(None)` if no hook is registered.
    /// Returns `Ok(Some((title, fields)))` with the hook's output on success.
    pub(super) fn run_on_save_hook(
        &self,
        engine: &Engine,
        schema: &Schema,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        let entry = {
            let hooks = self.on_save_hooks
                .lock()
                .map_err(|_| KrillnotesError::Scripting("on_save hook lock poisoned".to_string()))?;
            hooks.get(&schema.name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        let mut fields_map = Map::new();
        for (k, v) in fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut note_map = Map::new();
        note_map.insert("id".into(),        Dynamic::from(note_id.to_string()));
        note_map.insert("node_type".into(), Dynamic::from(node_type.to_string()));
        note_map.insert("title".into(),     Dynamic::from(title.to_string()));
        note_map.insert("fields".into(),    Dynamic::from(fields_map));

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("on_save hook error: {e}")))?;

        let result_map = result.try_cast::<Map>().ok_or_else(|| {
            KrillnotesError::Scripting("on_save hook must return the note map".to_string())
        })?;

        let new_title = result_map
            .get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result 'title' must be a string".to_string()))?;

        let new_fields_dyn = result_map
            .get("fields")
            .and_then(|v| v.clone().try_cast::<Map>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result 'fields' must be a map".to_string()))?;

        let mut new_fields = HashMap::new();
        for field_def in &schema.fields {
            let dyn_val = new_fields_dyn
                .get(field_def.name.as_str())
                .cloned()
                .unwrap_or(Dynamic::UNIT);
            let fv = dynamic_to_field_value(dyn_val, &field_def.field_type).map_err(|e| {
                KrillnotesError::Scripting(format!("field '{}': {}", field_def.name, e))
            })?;
            new_fields.insert(field_def.name.clone(), fv);
        }

        Ok(Some((new_title, new_fields)))
    }

    /// Runs the on_view hook for the schema identified by `node_type` in `note_map`, if registered.
    ///
    /// Returns `Ok(None)` if no hook is registered.
    /// Returns `Ok(Some(html))` with the generated HTML string on success.
    pub(super) fn run_on_view_hook(
        &self,
        engine: &Engine,
        note_map: Map,
    ) -> Result<Option<String>> {
        let schema_name = note_map
            .get("node_type")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();

        let entry = {
            let hooks = self.on_view_hooks
                .lock()
                .map_err(|_| KrillnotesError::Scripting("on_view hook lock poisoned".to_string()))?;
            hooks.get(&schema_name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("on_view hook error: {e}")))?;

        let html = result.try_cast::<String>().ok_or_else(|| {
            KrillnotesError::Scripting("on_view hook must return a string".to_string())
        })?;

        Ok(Some(html))
    }
}
```

**Step 4: Run the two new tests**

```bash
cargo test -p krillnotes-core test_on_save_inside_schema 2>&1 | tail -20
cargo test -p krillnotes-core test_on_view_inside_schema 2>&1 | tail -20
```

Expected: still FAIL — the `schema()` host function in `mod.rs` doesn't yet extract the hooks.

---

## Task 3: Update `mod.rs` — wire `schema()` host fn, remove standalone `on_save()`/`on_view()`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Update imports at top of `mod.rs`**

Replace lines 10–20 of the current file:

```rust
pub use hooks::HookRegistry;
pub(crate) use hooks::field_value_to_dynamic;   // ← REMOVE this line
pub use schema::{FieldDefinition, Schema};
// StarterScript is defined in this file and re-exported via lib.rs.

use crate::{FieldValue, KrillnotesError, Note, Result};
use hooks::HookEntry;                             // ← REMOVE this line
use include_dir::{include_dir, Dir};
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
```

New version (replace the block from `pub use hooks::HookRegistry;` through `use std::sync::...`):

```rust
pub use hooks::HookRegistry;
pub(crate) use schema::field_value_to_dynamic;
pub use schema::{FieldDefinition, Schema};
// StarterScript is defined in this file and re-exported via lib.rs.

use crate::{FieldValue, KrillnotesError, Note, Result};
use schema::HookEntry;
use include_dir::{include_dir, Dir};
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
```

**Step 2: Update `ScriptRegistry::new()` — replace the `schema()` host fn and remove `on_save()`/`on_view()` host fns**

Replace the entire block from the comment `// Register schema() host function` through the closing `});` of the `on_view()` registration (lines 68–99 in the current file) with:

```rust
        // Register schema() host function — writes schema and hooks into SchemaRegistry.
        let schemas_arc    = schema_registry.schemas_arc();
        let on_save_arc    = schema_registry.on_save_hooks_arc();
        let on_view_arc    = schema_registry.on_view_hooks_arc();
        let schema_ast_arc = Arc::clone(&current_loading_ast);
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let s = Schema::parse_from_rhai(&name, &def)
                .map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
            schemas_arc.lock().unwrap().insert(name.clone(), s);

            // Extract optional on_save closure.
            if let Some(fn_ptr) = def.get("on_save").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                on_save_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast });
            }

            // Extract optional on_view closure.
            if let Some(fn_ptr) = def.get("on_view").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                on_view_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast });
            }

            Ok(Dynamic::UNIT)
        });
```

Note: the entire `on_save()` registration block (lines 77–87) and the entire `on_view()` registration block (lines 89–99) are **deleted** — not replaced, just removed.

**Step 3: Add `has_hook()` to `ScriptRegistry` and update delegation methods**

In the `impl ScriptRegistry` block, make these changes:

a) **Add `has_hook()` method** (after `has_view_hook()`):

```rust
    /// Returns `true` if an on_save hook is registered for `schema_name`.
    pub fn has_hook(&self, schema_name: &str) -> bool {
        self.schema_registry.has_hook(schema_name)
    }
```

b) **Update `has_view_hook()`** (currently delegates to `hook_registry`):

```rust
    pub fn has_view_hook(&self, schema_name: &str) -> bool {
        self.schema_registry.has_view_hook(schema_name)
    }
```

c) **Update `run_on_save_hook()`** (currently delegates to `hook_registry`):

```rust
    pub fn run_on_save_hook(
        &self,
        schema_name: &str,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        let schema = self.schema_registry.get(schema_name)?;
        self.schema_registry.run_on_save_hook(&self.engine, &schema, note_id, node_type, title, fields)
    }
```

d) **Update `run_on_view_hook()`** (currently uses `self.hook_registry.run_on_view_hook`):

Replace the line `let result = self.hook_registry.run_on_view_hook(&self.engine, note_map);` with:

```rust
        let result = self.schema_registry.run_on_view_hook(&self.engine, note_map);
```

e) **Update `clear_all()`** (currently calls `self.hook_registry.clear()`):

```rust
    pub fn clear_all(&self) {
        self.schema_registry.clear();
        *self.query_context.lock().unwrap() = None;
    }
```

(Remove the `self.hook_registry.clear()` line — `schema_registry.clear()` now clears hooks too.)

**Step 4: Run the two new tests**

```bash
cargo test -p krillnotes-core test_on_save_inside_schema 2>&1 | tail -20
cargo test -p krillnotes-core test_on_view_inside_schema 2>&1 | tail -20
```

Expected: both PASS.

---

## Task 4: Gut `HookRegistry` in `hooks.rs`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/hooks.rs`

`HookEntry`, `field_value_to_dynamic`, and `dynamic_to_field_value` have moved to `schema.rs`. `HookRegistry` loses all schema-bound hook storage. The file is reduced to a near-empty shell kept for future global hooks.

**Step 1: Replace `hooks.rs` entirely**

```rust
//! Hook registry for global / lifecycle hooks (on_load, on_export, menu hooks, …).
//!
//! Schema-bound hooks (`on_save`, `on_view`) are managed by
//! [`SchemaRegistry`](super::schema::SchemaRegistry) and registered via the
//! `schema()` Rhai host function.

/// Registry for global event hooks not tied to a specific schema.
///
/// Currently a placeholder — global hooks (on_load, on_export, menu hooks, …)
/// will be added here in a future task.
#[derive(Debug)]
pub struct HookRegistry {}

impl HookRegistry {
    pub(super) fn new() -> Self {
        Self {}
    }
}
```

**Step 2: Check it compiles**

```bash
cargo build -p krillnotes-core 2>&1 | grep -E "^error"
```

Expected: no errors. (Warnings about unused imports in mod.rs are OK; fix them next.)

**Step 3: Fix any remaining unused-import warnings in `mod.rs`**

Remove `FnPtr` and `AST` from the `use rhai::` line in `mod.rs` if they're now unused. They ARE still used (in the `schema()` host fn), so this step may be a no-op. Check:

```bash
cargo build -p krillnotes-core 2>&1 | grep "unused import"
```

Remove any flagged imports.

---

## Task 5: Update tests in `mod.rs` to use new Rhai syntax

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (test module, lines ~341–1185)

Every test that embeds an `on_save("Name", |note| {...})` or `on_view("Name", |note| {...})` call must move the hook inside the `schema()` definition. Every test that calls `registry.hooks().has_hook()` must change to `registry.has_hook()`.

**Step 1: Run the full test suite and note failures**

```bash
cargo test -p krillnotes-core 2>&1 | grep -E "FAILED|error\[" | head -30
```

Expected: many failures — tests use old `on_save()` / `on_view()` host functions which no longer exist.

**Step 2: Update `test_hooks_accessor_returns_hook_registry`**

This test name is now misleading (there's no useful `hooks()` accessor for this). Rename and update:

```rust
#[test]
fn test_has_hook_after_schema_with_on_save() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Widget", #{
                fields: [ #{ name: "label", type: "text", required: false } ],
                on_save: |note| { note }
            });
        "#,
        )
        .unwrap();
    assert!(registry.has_hook("Widget"));
    assert!(!registry.has_hook("Missing"));
}
```

**Step 3: Update `test_run_on_save_hook_sets_title`**

Replace the inline script:

```rust
        registry
            .load_script(
                r#"
                schema("Person", #{
                    fields: [
                        #{ name: "first", type: "text", required: false },
                        #{ name: "last",  type: "text", required: false },
                    ],
                    on_save: |note| {
                        note.title = note.fields["last"] + ", " + note.fields["first"];
                        note
                    }
                });
            "#,
            )
            .unwrap();
```

**Step 4: Update `test_boolean_field_defaults_to_false_when_absent_from_hook_result`**

```rust
        registry
            .load_script(
                r#"
                schema("FlagNote", #{
                    fields: [
                        #{ name: "flag", type: "boolean", required: false },
                    ],
                    on_save: |note| {
                        // intentionally does NOT touch note.fields["flag"]
                        note
                    }
                });
            "#,
            )
            .unwrap();
```

**Step 5: Update `test_hooks_cleared_on_clear_all`**

```rust
    #[test]
    fn test_hooks_cleared_on_clear_all() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Hooked", #{
                fields: [#{ name: "x", type: "text" }],
                on_save: |note| { note }
            });
        "#).unwrap();
        assert!(registry.has_hook("Hooked"));

        registry.clear_all();
        assert!(!registry.has_hook("Hooked"));
    }
```

**Step 6: Update `test_contact_on_save_hook_derives_title`**

Change the assertion line from:

```rust
        assert!(registry.hooks().has_hook("Contact"), "Contact schema should have an on_save hook");
```

to:

```rust
        assert!(registry.has_hook("Contact"), "Contact schema should have an on_save hook");
```

(The script source itself, `01_contact.rhai`, will be updated in Task 6.)

**Step 7: Update `test_select_field_round_trips_through_hook`**

```rust
        registry.load_script(r#"
            schema("S", #{
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    note.fields.status = "B";
                    note
                }
            });
        "#).unwrap();
```

**Step 8: Update `test_rating_field_round_trips_through_hook`**

```rust
        registry.load_script(r#"
            schema("R", #{
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    note.fields.stars = 4.0;
                    note
                }
            });
        "#).unwrap();
```

**Step 9: Update `test_select_field_defaults_to_empty_text_when_absent_from_hook_result`**

```rust
        registry.load_script(r#"
            schema("S2", #{
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    // deliberately do NOT set note.fields.status
                    note
                }
            });
        "#).unwrap();
```

**Step 10: Update `test_rating_field_defaults_to_zero_when_absent_from_hook_result`**

```rust
        registry.load_script(r#"
            schema("R2", #{
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    // deliberately do NOT set note.fields.stars
                    note
                }
            });
        "#).unwrap();
```

**Step 11: Update `test_link_to_is_callable_from_on_view_script`**

```rust
        registry.load_script(r#"
            schema("LinkTest", #{
                fields: [#{ name: "ref_id", type: "text" }],
                on_view: |note| {
                    let target = #{ id: "target-id-123", title: "Target Note", fields: #{}, node_type: "TextNote" };
                    link_to(target)
                }
            });
        "#).unwrap();
```

**Step 12: Run all tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: all tests pass (the two new tests from Task 2 should also pass now). The `test_starter_scripts_load_without_error` test will FAIL because the system scripts still use old syntax — that is expected and will be fixed in Task 6.

**Step 13: Commit progress**

```bash
git add krillnotes-core/src/core/scripting/
git commit -m "refactor: move schema-bound hooks into schema() — Rust layer complete"
```

---

## Task 6: Update all system `.rhai` scripts

**Files:**
- Modify: `krillnotes-core/src/system_scripts/01_contact.rhai`
- Modify: `krillnotes-core/src/system_scripts/02_task.rhai`
- Modify: `krillnotes-core/src/system_scripts/03_project.rhai`
- Modify: `krillnotes-core/src/system_scripts/04_book.rhai`
- Modify: `krillnotes-core/src/system_scripts/05_recipe.rhai`
- Modify: `krillnotes-core/src/system_scripts/06_product.rhai`
- No change: `krillnotes-core/src/system_scripts/00_text_note.rhai` (no hooks)

**Step 1: Update `01_contact.rhai`**

`Contact` gets `on_save` inline. `ContactsFolder` gets `on_view` inline.
Note: `ContactsFolder` is defined first; its `on_view` goes inside it.

```rhai
// @name: Contacts
// @description: A contact folder and contact card with name, address, and communication details.

schema("ContactsFolder", #{
    children_sort: "asc",
    allowed_children_types: ["Contact"],
    fields: [
        #{ name: "notes", type: "textarea", required: false },
    ],
    on_view: |note| {
        let contacts = get_children(note.id);
        if contacts.len() == 0 {
            return text("No contacts yet. Add a contact using the context menu.");
        }
        let rows = contacts.map(|c| [
            link_to(c),
            c.fields.email  ?? "-",
            c.fields.phone  ?? "-",
            c.fields.mobile ?? "-"
        ]);
        let contacts_section = section(
            "Contacts (" + contacts.len() + ")",
            table(["Name", "Email", "Phone", "Mobile"], rows)
        );
        let notes_val = note.fields["notes"] ?? "";
        if notes_val == "" {
            contacts_section
        } else {
            stack([contacts_section, section("Notes", text(notes_val))])
        }
    }
});

schema("Contact", #{
    title_can_edit: false,
    allowed_parent_types: ["ContactsFolder"],
    fields: [
        #{ name: "first_name",      type: "text",  required: true  },
        #{ name: "middle_name",     type: "text",  required: false },
        #{ name: "last_name",       type: "text",  required: true  },
        #{ name: "phone",           type: "text",  required: false },
        #{ name: "mobile",          type: "text",  required: false },
        #{ name: "email",           type: "email", required: false },
        #{ name: "birthdate",       type: "date",  required: false },
        #{ name: "address_street",  type: "text",  required: false },
        #{ name: "address_city",    type: "text",  required: false },
        #{ name: "address_zip",     type: "text",  required: false },
        #{ name: "address_country", type: "text",  required: false },
        #{ name: "is_family",       type: "boolean",  required: false },
    ],
    on_save: |note| {
        let last  = note.fields["last_name"];
        let first = note.fields["first_name"];
        if last != "" || first != "" {
            note.title = last + ", " + first;
        }
        note
    }
});
```

**Step 2: Update `02_task.rhai`**

Move `on_save("Task", ...)` inside `schema("Task", #{ ... })`:

```rhai
// @name: Task
// @description: A trackable to-do item with status, priority, and due date.
//
// on_save hook:
//   - Computes the note title as a status symbol + task name, e.g. "[✓] Buy groceries"
//   - Derives `priority_label` (view-only) from the priority field

schema("Task", #{
    title_can_edit: false,
    fields: [
        #{ name: "name",           type: "text",     required: true                     },
        #{ name: "status",         type: "select",   required: true,
           options: ["TODO", "WIP", "DONE"]                                             },
        #{ name: "priority",       type: "select",   required: false,
           options: ["low", "medium", "high"]                                           },
        #{ name: "due_date",       type: "date",     required: false                    },
        #{ name: "assignee",       type: "text",     required: false                    },
        #{ name: "notes",          type: "textarea", required: false                    },
        #{ name: "priority_label", type: "text",     required: false,
           can_edit: false                                                               },
    ],
    on_save: |note| {
        let name   = note.fields["name"];
        let status = note.fields["status"];

        let symbol = if status == "DONE" { "✓" }
                     else if status == "WIP" { "→" }
                     else { " " };

        note.title = "[" + symbol + "] " + name;

        let priority = note.fields["priority"];
        note.fields["priority_label"] =
            if priority == "high"        { "🔴 High" }
            else if priority == "medium" { "🟡 Medium" }
            else if priority == "low"    { "🟢 Low" }
            else                         { "" };

        note
    }
});
```

**Step 3: Update `03_project.rhai`**

```rhai
// @name: Project
// @description: A piece of work with status, priority, and optional dates.
//
// on_save hook:
//   - Derives `health` (view-only) from the status field, e.g. "🚧 Active"

schema("Project", #{
    title_can_edit: true,
    fields: [
        #{ name: "status",      type: "select",   required: true,
           options: ["Planning", "Active", "On Hold", "Done"]                  },
        #{ name: "priority",    type: "select",   required: false,
           options: ["low", "medium", "high"]                                  },
        #{ name: "start_date",  type: "date",     required: false              },
        #{ name: "due_date",    type: "date",     required: false              },
        #{ name: "description", type: "textarea", required: false              },
        #{ name: "health",      type: "text",     required: false,
           can_edit: false                                                      },
    ],
    on_save: |note| {
        let status = note.fields["status"];

        note.fields["health"] =
            if status == "Done"          { "✅ Done" }
            else if status == "Active"   { "🚧 Active" }
            else if status == "On Hold"  { "⏸ On Hold" }
            else                         { "📋 Planning" };

        note
    }
});
```

**Step 4: Update `04_book.rhai`**

```rhai
// @name: Book
// @description: Reading tracker with star rating and derived read duration.
//
// on_save hook:
//   - Computes title as "Author: Book Title"
//   - Derives `read_duration` (view-only) from started + finished dates

schema("Book", #{
    title_can_edit: false,
    fields: [
        #{ name: "book_title",    type: "text",     required: true                  },
        #{ name: "author",        type: "text",     required: true                  },
        #{ name: "genre",         type: "text",     required: false                 },
        #{ name: "status",        type: "select",   required: false,
           options: ["To Read", "Reading", "Read"]                                  },
        #{ name: "rating",        type: "rating",   required: false, max: 5         },
        #{ name: "started",       type: "date",     required: false                 },
        #{ name: "finished",      type: "date",     required: false                 },
        #{ name: "notes",         type: "textarea", required: false                 },
        #{ name: "read_duration", type: "text",     required: false, can_edit: false },
    ],
    on_save: |note| {
        let title  = note.fields["book_title"];
        let author = note.fields["author"];

        note.title = if author != "" && title != "" {
            author + ": " + title
        } else if title != "" {
            title
        } else {
            "Untitled Book"
        };

        let started  = note.fields["started"];
        let finished = note.fields["finished"];
        note.fields["read_duration"] = if type_of(started) == "string" && started != ""
            && type_of(finished) == "string" && finished != "" {
            let s_parts = started.split("-");
            let f_parts = finished.split("-");
            let s_days = parse_int(s_parts[0]) * 365 + parse_int(s_parts[1]) * 30 + parse_int(s_parts[2]);
            let f_days = parse_int(f_parts[0]) * 365 + parse_int(f_parts[1]) * 30 + parse_int(f_parts[2]);
            let diff = f_days - s_days;
            if diff > 0 { diff.to_string() + " days" } else { "" }
        } else {
            ""
        };

        note
    }
});
```

**Step 5: Update `05_recipe.rhai`**

```rhai
// @name: Recipe
// @description: A cooking recipe with ingredients, steps, and derived total time.
//
// on_save hook:
//   - Derives `total_time` (view-only) from prep_time + cook_time

schema("Recipe", #{
    title_can_edit: true,
    fields: [
        #{ name: "servings",    type: "number",   required: false                  },
        #{ name: "prep_time",   type: "number",   required: false                  },
        #{ name: "cook_time",   type: "number",   required: false                  },
        #{ name: "difficulty",  type: "select",   required: false,
           options: ["Easy", "Medium", "Hard"]                                     },
        #{ name: "ingredients", type: "textarea", required: false                  },
        #{ name: "steps",       type: "textarea", required: false                  },
        #{ name: "total_time",  type: "text",     required: false, can_edit: false  },
    ],
    on_save: |note| {
        let prep  = note.fields["prep_time"];
        let cook  = note.fields["cook_time"];
        let total = (prep + cook).to_int();

        note.fields["total_time"] = if total <= 0 {
            ""
        } else if total < 60 {
            total.to_string() + " min"
        } else {
            let h = total / 60;
            let m = total % 60;
            if m == 0 { h.to_string() + "h" }
            else      { h.to_string() + "h " + m.to_string() + "min" }
        };

        note
    }
});
```

**Step 6: Update `06_product.rhai`**

```rhai
// @name: Product
// @description: An inventory item with auto-formatted title and stock status.
//
// on_save hook:
//   - Computes title as "Product Name (SKU)" (or just name if SKU is blank)
//   - Derives `stock_status` (view-only) from the stock count

schema("Product", #{
    title_can_edit: false,
    fields: [
        #{ name: "product_name", type: "text",     required: true                   },
        #{ name: "sku",          type: "text",     required: false                  },
        #{ name: "price",        type: "number",   required: false                  },
        #{ name: "stock",        type: "number",   required: false                  },
        #{ name: "category",     type: "text",     required: false                  },
        #{ name: "description",  type: "textarea", required: false                  },
        #{ name: "stock_status", type: "text",     required: false, can_edit: false  },
    ],
    on_save: |note| {
        let name = note.fields["product_name"];
        let sku  = note.fields["sku"];

        note.title = if sku != "" {
            name + " (" + sku + ")"
        } else {
            name
        };

        let stock = note.fields["stock"];
        note.fields["stock_status"] =
            if stock < 0.0         { "❌ Out of Stock" }
            else if stock == 0.0   { "" }
            else if stock < 5.0    { "⚠️ Low Stock" }
            else                   { "✅ In Stock" };

        note
    }
});
```

**Step 7: Run the full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: all tests PASS, including `test_starter_scripts_load_without_error`.

**Step 8: Also build the desktop crate to confirm no compilation errors there**

```bash
cargo build -p krillnotes-desktop 2>&1 | grep -E "^error"
```

Expected: no errors.

**Step 9: Commit**

```bash
git add krillnotes-core/src/system_scripts/
git commit -m "refactor: update all system scripts to new on_save/on_view-inside-schema syntax"
```

---

## Task 7: Final verification and TODO.md update

**Step 1: Run the full workspace test suite**

```bash
cargo test --workspace 2>&1 | tail -30
```

Expected: all tests PASS.

**Step 2: Mark TODO item as done**

In `TODO.md` at the main checkout (`/Users/careck/Source/Krillnotes/TODO.md`), change:

```
[ ] I thought about the design decision to have on_save() and on_view() hooks outside the schema definition…
```

to:

```
✅ DONE! I thought about the design decision to have on_save() and on_view() hooks outside the schema definition…
```

**Step 3: Commit TODO update**

```bash
git add TODO.md
git commit -m "chore: mark hooks-inside-schema task as done in TODO.md"
```

---

## Task 8: Finish the branch

> Use superpowers:finishing-a-development-branch to decide how to merge/PR.
