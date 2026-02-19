# Scripting Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the monolithic `scripting.rs` into three focused types — `ScriptRegistry` (orchestrator, public), `SchemaRegistry` (private schema store), `HookRegistry` (public hook store + execution) — living in a `scripting/` module directory.

**Architecture:** `ScriptRegistry` owns the Rhai `Engine`, registers host functions, and delegates all queries and hook execution to private `SchemaRegistry` and public `HookRegistry`. `Workspace` renames its `registry: SchemaRegistry` field to `script_registry: ScriptRegistry`. The public crate API swaps `SchemaRegistry` for `ScriptRegistry` and adds `HookRegistry`.

**Tech Stack:** Rust, Rhai 1.24 (`sync` feature), `Arc<Mutex<...>>` for shared state across Rhai closures

---

### Task 1: Create `scripting/schema.rs`

**Files:**
- Create: `krillnotes-core/src/core/scripting/schema.rs`

Move `FieldDefinition`, `Schema`, and all schema-related logic out of `scripting.rs` into a new file. `SchemaRegistry` in this file is private to the `scripting` module — it has no Rhai dependency.

**Step 1: Create the file**

```rust
//! Schema definitions and the private schema store for Krillnotes note types.

use crate::{FieldValue, KrillnotesError, Result};
use rhai::Map;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Describes a single typed field within a note schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
}

/// A parsed note-type schema containing an ordered list of field definitions.
#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
}

impl Schema {
    /// Returns a map of field names to their zero-value defaults.
    pub fn default_fields(&self) -> HashMap<String, FieldValue> {
        let mut fields = HashMap::new();
        for field_def in &self.fields {
            let default_value = match field_def.field_type.as_str() {
                "text" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                "date" => FieldValue::Date(None),
                "email" => FieldValue::Email(String::new()),
                _ => FieldValue::Text(String::new()),
            };
            fields.insert(field_def.name.clone(), default_value);
        }
        fields
    }

    /// Parses a `Schema` from a Rhai object map produced by a `schema(...)` call.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the map is malformed.
    pub(super) fn parse_from_rhai(name: &str, def: &Map) -> Result<Self> {
        let fields_array = def
            .get("fields")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .ok_or_else(|| KrillnotesError::Scripting("Missing 'fields' array".to_string()))?;

        let mut fields = Vec::new();
        for field_item in fields_array {
            let field_map = field_item
                .try_cast::<Map>()
                .ok_or_else(|| KrillnotesError::Scripting("Field must be a map".to_string()))?;

            let field_name = field_map
                .get("name")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("Field missing 'name'".to_string()))?;

            let field_type = field_map
                .get("type")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("Field missing 'type'".to_string()))?;

            let required = field_map
                .get("required")
                .and_then(|v| v.clone().try_cast::<bool>())
                .unwrap_or(false);

            fields.push(FieldDefinition { name: field_name, field_type, required });
        }

        Ok(Schema { name: name.to_string(), fields })
    }
}

/// Private store for registered schemas. No Rhai dependency.
pub(super) struct SchemaRegistry {
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self { schemas: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn schemas_arc(&self) -> Arc<Mutex<HashMap<String, Schema>>> {
        Arc::clone(&self.schemas)
    }

    pub(super) fn get(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    pub(super) fn list(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
    }
}
```

**Step 2: Verify it compiles in isolation**

This file has no tests of its own — the types it exposes (`Schema`, `FieldDefinition`) are covered by tests in `mod.rs`. Just confirm it compiles:

```bash
cargo build -p krillnotes-core --manifest-path /Users/careck/Source/Krillnotes/Cargo.toml 2>&1 | head -20
```

Expected: build error because `scripting/mod.rs` does not exist yet — that is fine. The file itself must have no syntax errors (no `error[E...]` lines pointing into `schema.rs`).

**Step 3: Commit**

```bash
cd /Users/careck/Source/Krillnotes && \
git add krillnotes-core/src/core/scripting/schema.rs && \
git commit -m "$(cat <<'EOF'
refactor(core): add scripting/schema.rs with Schema, FieldDefinition, SchemaRegistry

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Create `scripting/hooks.rs`

**Files:**
- Create: `krillnotes-core/src/core/scripting/hooks.rs`

Move `HookEntry`, the hook store, `run_on_save_hook` execution logic, and the `field_value_to_dynamic` / `dynamic_to_field_value` conversion helpers out of `scripting.rs` into this file. `HookRegistry` is public.

**Step 1: Create the file**

```rust
//! Hook storage and execution for Krillnotes scripting events.

use super::schema::Schema;
use crate::{FieldValue, KrillnotesError, Result};
use chrono::NaiveDate;
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A stored pre-save hook: the Rhai closure and the AST it was defined in.
#[derive(Clone, Debug)]
pub(super) struct HookEntry {
    pub(super) fn_ptr: FnPtr,
    pub(super) ast: AST,
}

/// Public registry of event hooks loaded from Rhai scripts.
///
/// Execution methods accept a `&Engine` from the caller ([`ScriptRegistry`])
/// rather than owning one, keeping this type free of Rhai engine lifecycle concerns.
///
/// [`ScriptRegistry`]: super::ScriptRegistry
#[derive(Debug)]
pub struct HookRegistry {
    on_save_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
}

impl HookRegistry {
    pub(super) fn new() -> Self {
        Self { on_save_hooks: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn on_save_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_save_hooks)
    }

    /// Returns `true` if a pre-save hook is registered for `schema_name`.
    pub fn has_hook(&self, schema_name: &str) -> bool {
        self.on_save_hooks.lock().unwrap().contains_key(schema_name)
    }

    /// Runs the pre-save hook registered for `schema_name`, if any.
    ///
    /// Returns `Ok(None)` when no hook is registered for `schema_name`.
    /// Returns `Ok(Some((title, fields)))` with the hook's output on success.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the hook throws a Rhai error
    /// or returns a malformed map.
    pub fn run_on_save_hook(
        &self,
        engine: &Engine,
        schema: &Schema,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        // Clone the entry out of the mutex so the lock is not held during the call.
        let entry = {
            let hooks = self.on_save_hooks.lock().unwrap();
            hooks.get(&schema.name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        // Build the fields sub-map.
        let mut fields_map = Map::new();
        for (k, v) in fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }

        // Build the top-level note map.
        let mut note_map = Map::new();
        note_map.insert("id".into(), Dynamic::from(note_id.to_string()));
        note_map.insert("node_type".into(), Dynamic::from(node_type.to_string()));
        note_map.insert("title".into(), Dynamic::from(title.to_string()));
        note_map.insert("fields".into(), Dynamic::from(fields_map));

        // Call the closure.
        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("on_save hook error: {}", e)))?;

        // Parse the returned map.
        let result_map = result.try_cast::<Map>().ok_or_else(|| {
            KrillnotesError::Scripting("on_save hook must return the note map".to_string())
        })?;

        let new_title = result_map
            .get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| {
                KrillnotesError::Scripting("hook result 'title' must be a string".to_string())
            })?;

        let new_fields_dyn = result_map
            .get("fields")
            .and_then(|v| v.clone().try_cast::<Map>())
            .ok_or_else(|| {
                KrillnotesError::Scripting("hook result 'fields' must be a map".to_string())
            })?;

        // Convert each field back, guided by the schema's type definitions.
        // Fields present in the schema but absent from the hook result are passed
        // as Dynamic::UNIT to dynamic_to_field_value, yielding the type's zero value.
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
}

/// Converts a [`FieldValue`] to a Rhai [`Dynamic`] for passing into hook closures.
///
/// `Date(None)` maps to `Dynamic::UNIT` (`()`).
/// `Date(Some(d))` maps to an ISO 8601 string `"YYYY-MM-DD"`.
/// All other variants map to their natural Rhai primitive.
fn field_value_to_dynamic(fv: &FieldValue) -> Dynamic {
    match fv {
        FieldValue::Text(s) => Dynamic::from(s.clone()),
        FieldValue::Number(n) => Dynamic::from(*n),
        FieldValue::Boolean(b) => Dynamic::from(*b),
        FieldValue::Date(None) => Dynamic::UNIT,
        FieldValue::Date(Some(d)) => Dynamic::from(d.format("%Y-%m-%d").to_string()),
        FieldValue::Email(s) => Dynamic::from(s.clone()),
    }
}

/// Converts a Rhai [`Dynamic`] back to a [`FieldValue`] given the field's type string.
///
/// Returns [`KrillnotesError::Scripting`] if the Dynamic value cannot be
/// converted to the expected Rust type.
fn dynamic_to_field_value(d: Dynamic, field_type: &str) -> Result<FieldValue> {
    match field_type {
        "text" => {
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("text field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "number" => {
            let n = d
                .try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("number field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        "boolean" => {
            let b = d
                .try_cast::<bool>()
                .ok_or_else(|| KrillnotesError::Scripting("boolean field must be a bool".into()))?;
            Ok(FieldValue::Boolean(b))
        }
        "date" => {
            if d.is_unit() {
                Ok(FieldValue::Date(None))
            } else {
                let s = d.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("date field must be a string or ()".into())
                })?;
                let nd = NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|e| {
                    KrillnotesError::Scripting(format!("invalid date '{}': {}", s, e))
                })?;
                Ok(FieldValue::Date(Some(nd)))
            }
        }
        "email" => {
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("email field must be a string".into()))?;
            Ok(FieldValue::Email(s))
        }
        _ => Ok(FieldValue::Text(String::new())),
    }
}
```

**Step 2: Verify syntax (build will fail — that is expected)**

```bash
cargo build -p krillnotes-core --manifest-path /Users/careck/Source/Krillnotes/Cargo.toml 2>&1 | grep "^error" | grep "scripting/hooks.rs"
```

Expected: no lines (no errors inside `hooks.rs` itself).

**Step 3: Commit**

```bash
cd /Users/careck/Source/Krillnotes && \
git add krillnotes-core/src/core/scripting/hooks.rs && \
git commit -m "$(cat <<'EOF'
refactor(core): add scripting/hooks.rs with HookRegistry and conversion helpers

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Create `scripting/mod.rs` with `ScriptRegistry` and delete `scripting.rs`

**Files:**
- Create: `krillnotes-core/src/core/scripting/mod.rs`
- Delete: `krillnotes-core/src/core/scripting.rs`

This task wires everything together. `ScriptRegistry` owns the `Engine` and delegates to the sub-registries. All existing tests move here (import paths updated).

**Step 1: Write the failing test (new behaviour: `hooks()` accessor)**

The existing tests will be ported in Step 3. Write one new test for the `hooks()` accessor — the only truly new public API — so we have a failing test before implementing:

Add to the test block you will create in `mod.rs`:

```rust
#[test]
fn test_hooks_accessor_returns_hook_registry() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Widget", #{
                fields: [ #{ name: "label", type: "text", required: false } ]
            });
            on_save("Widget", |note| { note });
        "#,
        )
        .unwrap();
    assert!(registry.hooks().has_hook("Widget"));
    assert!(!registry.hooks().has_hook("Missing"));
}
```

**Step 2: Create `scripting/mod.rs`**

```rust
//! Rhai-based scripting registry for Krillnotes note types and hooks.
//!
//! [`ScriptRegistry`] is the public entry point. It owns the Rhai [`Engine`],
//! loads scripts, and delegates schema and hook concerns to internal sub-registries.

mod hooks;
mod schema;

pub use hooks::HookRegistry;
pub use schema::{FieldDefinition, Schema};

use crate::{FieldValue, KrillnotesError, Result};
use hooks::HookEntry;
use rhai::{Engine, FnPtr, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Orchestrating registry that owns the Rhai engine and delegates to
/// [`SchemaRegistry`](schema::SchemaRegistry) and [`HookRegistry`].
///
/// This is the primary scripting entry point used by [`Workspace`](crate::Workspace).
#[derive(Debug)]
pub struct ScriptRegistry {
    engine: Engine,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
    schema_registry: schema::SchemaRegistry,
    hook_registry: HookRegistry,
}

impl ScriptRegistry {
    /// Creates a new registry and loads the built-in system schemas.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if a bundled system script
    /// fails to parse or if any `schema(...)` call within it is malformed.
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        let schema_registry = schema::SchemaRegistry::new();
        let hook_registry = HookRegistry::new();
        let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));

        // Register schema() host function — writes into SchemaRegistry.
        let schemas_arc = schema_registry.schemas_arc();
        engine.register_fn("schema", move |name: String, def: rhai::Map| {
            let s = Schema::parse_from_rhai(&name, &def).unwrap();
            schemas_arc.lock().unwrap().insert(name, s);
        });

        // Register on_save() host function — writes into HookRegistry.
        let hooks_arc = hook_registry.on_save_hooks_arc();
        let ast_arc = Arc::clone(&current_loading_ast);
        engine.register_fn("on_save", move |name: String, fn_ptr: FnPtr| {
            let ast = ast_arc
                .lock()
                .unwrap()
                .clone()
                .expect("on_save called outside of load_script");
            hooks_arc
                .lock()
                .unwrap()
                .insert(name, HookEntry { fn_ptr, ast });
        });

        let mut registry = Self {
            engine,
            current_loading_ast,
            schema_registry,
            hook_registry,
        };
        registry.load_script(include_str!("../../system_scripts/text_note.rhai"))?;
        registry.load_script(include_str!("../../system_scripts/contact.rhai"))?;

        Ok(registry)
    }

    /// Evaluates `script` and registers any schemas and hooks it defines.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the script fails to evaluate.
    pub fn load_script(&mut self, script: &str) -> Result<()> {
        let ast = self
            .engine
            .compile(script)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;

        *self.current_loading_ast.lock().unwrap() = Some(ast.clone());

        let result = self
            .engine
            .eval_ast::<()>(&ast)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()));

        *self.current_loading_ast.lock().unwrap() = None;

        result
    }

    /// Returns the schema registered under `name`.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::SchemaNotFound`] if no schema with that name
    /// has been registered.
    pub fn get_schema(&self, name: &str) -> Result<Schema> {
        self.schema_registry.get(name)
    }

    /// Returns the names of all currently registered schemas.
    pub fn list_types(&self) -> Result<Vec<String>> {
        Ok(self.schema_registry.list())
    }

    /// Returns a reference to the [`HookRegistry`] for hook state queries.
    pub fn hooks(&self) -> &HookRegistry {
        &self.hook_registry
    }

    /// Runs the pre-save hook registered for `schema_name`, if any.
    ///
    /// Delegates to [`HookRegistry::run_on_save_hook`] with this registry's engine.
    ///
    /// Returns `Ok(None)` when no hook is registered for `schema_name`.
    /// Returns `Ok(Some((title, fields)))` with the hook's output on success.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the hook throws a Rhai error
    /// or returns a malformed map.
    pub fn run_on_save_hook(
        &self,
        schema_name: &str,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        let schema = self.schema_registry.get(schema_name)?;
        self.hook_registry
            .run_on_save_hook(&self.engine, &schema, note_id, node_type, title, fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── New test for the hooks() accessor ────────────────────────────────────

    #[test]
    fn test_hooks_accessor_returns_hook_registry() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("Widget", #{
                    fields: [ #{ name: "label", type: "text", required: false } ]
                });
                on_save("Widget", |note| { note });
            "#,
            )
            .unwrap();
        assert!(registry.hooks().has_hook("Widget"));
        assert!(!registry.hooks().has_hook("Missing"));
    }

    // ── Ported tests (previously in scripting.rs) ────────────────────────────

    #[test]
    fn test_schema_registration() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("TestNote", #{
                    fields: [
                        #{ name: "body", type: "text", required: false },
                        #{ name: "count", type: "number", required: false },
                    ]
                });
            "#,
            )
            .unwrap();
        let schema = registry.get_schema("TestNote").unwrap();
        assert_eq!(schema.name, "TestNote");
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "text");
    }

    #[test]
    fn test_default_fields() {
        let schema = Schema {
            name: "TestNote".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "body".to_string(),
                    field_type: "text".to_string(),
                    required: false,
                },
                FieldDefinition {
                    name: "count".to_string(),
                    field_type: "number".to_string(),
                    required: false,
                },
            ],
        };
        let defaults = schema.default_fields();
        assert_eq!(defaults.len(), 2);
        assert!(matches!(defaults.get("body"), Some(FieldValue::Text(_))));
        assert!(matches!(defaults.get("count"), Some(FieldValue::Number(_))));
    }

    #[test]
    fn test_text_note_schema_loaded() {
        let registry = ScriptRegistry::new().unwrap();
        let schema = registry.get_schema("TextNote").unwrap();
        assert_eq!(schema.name, "TextNote");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "text");
    }

    #[test]
    fn test_date_field_default() {
        let schema = Schema {
            name: "Test".to_string(),
            fields: vec![FieldDefinition {
                name: "birthday".to_string(),
                field_type: "date".to_string(),
                required: false,
            }],
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("birthday"), Some(FieldValue::Date(None))));
    }

    #[test]
    fn test_email_field_default() {
        let schema = Schema {
            name: "Test".to_string(),
            fields: vec![FieldDefinition {
                name: "email_addr".to_string(),
                field_type: "email".to_string(),
                required: false,
            }],
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("email_addr"), Some(FieldValue::Email(s)) if s.is_empty()));
    }

    #[test]
    fn test_contact_schema_loaded() {
        let registry = ScriptRegistry::new().unwrap();
        let schema = registry.get_schema("Contact").unwrap();
        assert_eq!(schema.name, "Contact");
        assert_eq!(schema.fields.len(), 11);
        let email_field = schema.fields.iter().find(|f| f.name == "email").unwrap();
        assert_eq!(email_field.field_type, "email");
        let birthdate_field = schema.fields.iter().find(|f| f.name == "birthdate").unwrap();
        assert_eq!(birthdate_field.field_type, "date");
        let first_name_field = schema.fields.iter().find(|f| f.name == "first_name").unwrap();
        assert!(first_name_field.required, "first_name should be required");
        let last_name_field = schema.fields.iter().find(|f| f.name == "last_name").unwrap();
        assert!(last_name_field.required, "last_name should be required");
    }

    #[test]
    fn test_hook_registered_via_on_save() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("Widget", #{
                    fields: [ #{ name: "label", type: "text", required: false } ]
                });
                on_save("Widget", |note| { note });
            "#,
            )
            .unwrap();
        assert!(registry.hooks().has_hook("Widget"));
        assert!(!registry.hooks().has_hook("Missing"));
    }

    // ── Conversion helper tests (tested via HookRegistry in hooks.rs) ────────
    // These were private free function tests in scripting.rs.
    // They are now exercised end-to-end through run_on_save_hook.
    // Dedicated unit tests for the helpers live in scripting/hooks.rs if needed.

    #[test]
    fn test_run_on_save_hook_sets_title() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("Person", #{
                    fields: [
                        #{ name: "first", type: "text", required: false },
                        #{ name: "last",  type: "text", required: false },
                    ]
                });
                on_save("Person", |note| {
                    note.title = note.fields["last"] + ", " + note.fields["first"];
                    note
                });
            "#,
            )
            .unwrap();

        let mut fields = HashMap::new();
        fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
        fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

        let result = registry
            .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
            .unwrap();

        assert!(result.is_some());
        let (new_title, new_fields) = result.unwrap();
        assert_eq!(new_title, "Doe, John");
        assert_eq!(new_fields.get("first"), Some(&FieldValue::Text("John".to_string())));
    }

    #[test]
    fn test_run_on_save_hook_no_hook_returns_none() {
        let registry = ScriptRegistry::new().unwrap();
        let fields = HashMap::new();
        let result = registry
            .run_on_save_hook("TextNote", "id-1", "TextNote", "title", &fields)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_contact_on_save_hook_derives_title() {
        let registry = ScriptRegistry::new().unwrap();
        assert!(registry.hooks().has_hook("Contact"), "Contact schema should have an on_save hook");

        let mut fields = HashMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("Jane".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Smith".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));

        let result = registry
            .run_on_save_hook("Contact", "id-1", "Contact", "", &fields)
            .unwrap()
            .unwrap();

        assert_eq!(result.0, "Smith, Jane");
    }
}
```

**Step 3: Delete `scripting.rs`**

```bash
rm /Users/careck/Source/Krillnotes/krillnotes-core/src/core/scripting.rs
```

**Step 4: Run the full test suite**

```bash
cargo test -p krillnotes-core --manifest-path /Users/careck/Source/Krillnotes/Cargo.toml 2>&1 | tail -15
```

Expected: compile errors about `SchemaRegistry` not found in `workspace.rs`, `mod.rs`, `lib.rs`. The `scripting` tests themselves should be structurally correct. Fix compile errors in Tasks 4 and 5.

**Step 5: Commit**

```bash
cd /Users/careck/Source/Krillnotes && \
git add krillnotes-core/src/core/scripting/mod.rs && \
git rm krillnotes-core/src/core/scripting.rs && \
git commit -m "$(cat <<'EOF'
refactor(core): add scripting/mod.rs with ScriptRegistry, remove scripting.rs

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Update `workspace.rs`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

Rename the `registry` field and its type, and update all call sites.

**Step 1: Update the import line**

Find:
```rust
    OperationLog, PurgeStrategy, Result, SchemaRegistry, Storage,
```
Replace with:
```rust
    OperationLog, PurgeStrategy, Result, ScriptRegistry, Storage,
```

**Step 2: Update the struct field and its doc comment**

Find:
```rust
/// An open Krillnotes workspace backed by a SQLite database.
///
/// `Workspace` is the primary interface for all document mutations. It combines
/// a [`Storage`] connection, a [`SchemaRegistry`] for note-type validation,
/// and an [`OperationLog`] for durable change history.
```
Replace with:
```rust
/// An open Krillnotes workspace backed by a SQLite database.
///
/// `Workspace` is the primary interface for all document mutations. It combines
/// a [`Storage`] connection, a [`ScriptRegistry`] for note-type validation and hooks,
/// and an [`OperationLog`] for durable change history.
```

Find:
```rust
    registry: SchemaRegistry,
```
Replace with:
```rust
    script_registry: ScriptRegistry,
```

**Step 3: Update `Workspace::create` — construction and field init**

Find (in `create`):
```rust
        let registry = SchemaRegistry::new()?;
```
Replace with:
```rust
        let script_registry = ScriptRegistry::new()?;
```

Find (schema lookup during root note creation in `create`):
```rust
            fields: registry.get_schema("TextNote")?.default_fields(),
```
Replace with:
```rust
            fields: script_registry.get_schema("TextNote")?.default_fields(),
```

Find (struct literal in `create`):
```rust
            registry,
```
Replace with:
```rust
            script_registry,
```

**Step 4: Update `Workspace::open` — same pattern**

Find (in `open`):
```rust
        let registry = SchemaRegistry::new()?;
```
Replace with:
```rust
        let script_registry = ScriptRegistry::new()?;
```

Find (struct literal in `open`):
```rust
            registry,
```
Replace with:
```rust
            script_registry,
```

**Step 5: Update `registry()` accessor method**

Find:
```rust
    /// Returns a reference to the schema registry for this workspace.
    pub fn registry(&self) -> &SchemaRegistry {
        &self.registry
    }
```
Replace with:
```rust
    /// Returns a reference to the script registry for this workspace.
    pub fn script_registry(&self) -> &ScriptRegistry {
        &self.script_registry
    }
```

**Step 6: Update all remaining `self.registry.*` call sites**

There are several in `workspace.rs`. Replace each:

| Old | New |
|-----|-----|
| `self.registry.get_schema(` | `self.script_registry.get_schema(` |
| `self.registry.list_types()` | `self.script_registry.list_types()` |
| `self.registry.run_on_save_hook(` | `self.script_registry.run_on_save_hook(` |

Use search-and-replace to catch all occurrences:
```bash
grep -n "self\.registry" /Users/careck/Source/Krillnotes/krillnotes-core/src/core/workspace.rs
```
Confirm every match is updated.

**Step 7: Run the test suite**

```bash
cargo test -p krillnotes-core --manifest-path /Users/careck/Source/Krillnotes/Cargo.toml 2>&1 | tail -15
```

Expected: compile errors only from `core/mod.rs` and `lib.rs` (still reference `SchemaRegistry`). No errors inside `workspace.rs`.

**Step 8: Commit**

```bash
cd /Users/careck/Source/Krillnotes && \
git add krillnotes-core/src/core/workspace.rs && \
git commit -m "$(cat <<'EOF'
refactor(core): rename Workspace::registry to script_registry: ScriptRegistry

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Update re-exports and Tauri command layer

**Files:**
- Modify: `krillnotes-core/src/core/mod.rs`
- Modify: `krillnotes-core/src/lib.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

Fix the remaining compile errors by swapping `SchemaRegistry` → `ScriptRegistry` in re-exports and adding `HookRegistry`.

**Step 1: Update `krillnotes-core/src/core/mod.rs`**

Find:
```rust
pub use scripting::{FieldDefinition, Schema, SchemaRegistry};
```
Replace with:
```rust
pub use scripting::{FieldDefinition, HookRegistry, Schema, ScriptRegistry};
```

**Step 2: Update `krillnotes-core/src/lib.rs`**

Find:
```rust
    scripting::{FieldDefinition, Schema, SchemaRegistry},
```
Replace with:
```rust
    scripting::{FieldDefinition, HookRegistry, Schema, ScriptRegistry},
```

**Step 3: Update Tauri `lib.rs` — `workspace.registry()` call**

File: `krillnotes-desktop/src-tauri/src/lib.rs`, line ~390.

Find:
```rust
    let schema = workspace.registry().get_schema(&node_type)
```
Replace with:
```rust
    let schema = workspace.script_registry().get_schema(&node_type)
```

**Step 4: Run the full test suite**

```bash
cargo test --manifest-path /Users/careck/Source/Krillnotes/Cargo.toml 2>&1 | tail -15
```

Expected: all tests pass, zero compile errors, zero regressions.

**Step 5: Commit**

```bash
cd /Users/careck/Source/Krillnotes && \
git add krillnotes-core/src/core/mod.rs \
        krillnotes-core/src/lib.rs \
        krillnotes-desktop/src-tauri/src/lib.rs && \
git commit -m "$(cat <<'EOF'
refactor(core): update re-exports and Tauri layer for ScriptRegistry rename

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Done

All five tasks complete. The scripting module is fully split:

- `scripting/schema.rs` — `Schema`, `FieldDefinition` (pub), `SchemaRegistry` (private, no Rhai dependency)
- `scripting/hooks.rs` — `HookRegistry` (pub), `HookEntry` (private), conversion helpers, `run_on_save_hook`
- `scripting/mod.rs` — `ScriptRegistry` (pub), engine ownership, host-fn registration, delegation methods

Adding a new hook event type (e.g. `on_create`) requires:
1. New collection + typed `run_on_create_hook` method in `HookRegistry`
2. Register `on_create` host function in `ScriptRegistry::new()`
3. Add typed `run_on_create_hook` delegation method on `ScriptRegistry`
