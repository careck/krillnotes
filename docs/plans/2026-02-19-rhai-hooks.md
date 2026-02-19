# Rhai Hooks Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a pre-save hook system to Rhai scripts so schemas can transform note data (e.g. auto-derive `title`) before it is written to SQLite.

**Architecture:** `SchemaRegistry` stores `FnPtr` closures alongside the compiled `AST` of the script that defined them. A new `on_save("SchemaName", |note| { ... })` host function is registered in the Rhai engine. `Workspace::update_note` queries the note's `node_type`, runs the hook if one exists, and saves the hook's output.

**Tech Stack:** Rust, Rhai 1.24 (`sync` feature), rusqlite, chrono

---

### Task 1: Add hook storage infrastructure to `SchemaRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting.rs`

This task wires up the data structures and host-function registration so that `on_save("Name", |note| { ... })` in a Rhai script stores the closure for later use. No calling logic yet — just storage.

**Step 1: Write the failing test**

Add to the `#[cfg(test)]` block in `scripting.rs`:

```rust
#[test]
fn test_hook_registered_via_on_save() {
    let mut registry = SchemaRegistry::new().unwrap();
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
    assert!(registry.has_hook("Widget"));
    assert!(!registry.has_hook("Missing"));
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p krillnotes-core test_hook_registered_via_on_save -- --nocapture
```

Expected: compile error — `has_hook` does not exist yet.

**Step 3: Implement**

In `scripting.rs`, add the following in this order:

**3a.** Expand the `use rhai` import at the top:

```rust
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
```

**3b.** Add `HookEntry` above `SchemaRegistry`:

```rust
/// A stored pre-save hook: the Rhai closure and the AST it was defined in.
#[derive(Clone)]
struct HookEntry {
    fn_ptr: FnPtr,
    ast: AST,
}
```

**3c.** Add two fields to `SchemaRegistry`:

```rust
pub struct SchemaRegistry {
    engine: Engine,
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
    hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
}
```

**3d.** In `SchemaRegistry::new()`, register `on_save` and wire the new fields.
Replace the entire `new()` body with:

```rust
pub fn new() -> Result<Self> {
    let mut engine = Engine::new();
    let schemas = Arc::new(Mutex::new(HashMap::new()));
    let hooks: Arc<Mutex<HashMap<String, HookEntry>>> = Arc::new(Mutex::new(HashMap::new()));
    let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));

    let schemas_clone = Arc::clone(&schemas);
    engine.register_fn("schema", move |name: String, def: Map| {
        let schema = Self::parse_schema(&name, &def).unwrap();
        schemas_clone.lock().unwrap().insert(name, schema);
    });

    let hooks_for_fn = Arc::clone(&hooks);
    let ast_for_fn = Arc::clone(&current_loading_ast);
    engine.register_fn("on_save", move |name: String, fn_ptr: FnPtr| {
        let ast = ast_for_fn
            .lock()
            .unwrap()
            .clone()
            .expect("on_save called outside of load_script");
        hooks_for_fn
            .lock()
            .unwrap()
            .insert(name, HookEntry { fn_ptr, ast });
    });

    let mut registry = Self {
        engine,
        schemas,
        hooks,
        current_loading_ast,
    };
    registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;
    registry.load_script(include_str!("../system_scripts/contact.rhai"))?;

    Ok(registry)
}
```

**3e.** Replace `load_script` so it compiles the AST and tracks it for hook registration:

```rust
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
```

**3f.** Add `has_hook` after `list_types`:

```rust
/// Returns `true` if a pre-save hook is registered for `schema_name`.
pub fn has_hook(&self, schema_name: &str) -> bool {
    self.hooks.lock().unwrap().contains_key(schema_name)
}
```

**Step 4: Run test to verify it passes**

```bash
cargo test -p krillnotes-core test_hook_registered_via_on_save -- --nocapture
```

Expected: PASS. Also run the full suite to check nothing regressed:

```bash
cargo test -p krillnotes-core
```

Expected: all existing tests still pass.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting.rs
git commit -m "feat(core): add hook storage and on_save registration to SchemaRegistry"
```

---

### Task 2: Add `FieldValue` ↔ Rhai `Dynamic` conversion helpers

**Files:**
- Modify: `krillnotes-core/src/core/scripting.rs`

Two private free functions that convert between `FieldValue` and Rhai's `Dynamic`. These will be used by `run_on_save_hook` in Task 3.

**Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block:

```rust
#[test]
fn test_field_value_to_dynamic_text() {
    let d = field_value_to_dynamic(&FieldValue::Text("hello".into()));
    assert_eq!(d.try_cast::<String>().unwrap(), "hello");
}

#[test]
fn test_field_value_to_dynamic_number() {
    let d = field_value_to_dynamic(&FieldValue::Number(3.14));
    assert!((d.try_cast::<f64>().unwrap() - 3.14).abs() < f64::EPSILON);
}

#[test]
fn test_field_value_to_dynamic_boolean() {
    let d = field_value_to_dynamic(&FieldValue::Boolean(true));
    assert!(d.try_cast::<bool>().unwrap());
}

#[test]
fn test_field_value_to_dynamic_date_none() {
    let d = field_value_to_dynamic(&FieldValue::Date(None));
    assert!(d.is_unit());
}

#[test]
fn test_field_value_to_dynamic_date_some() {
    use chrono::NaiveDate;
    let date = NaiveDate::from_ymd_opt(2026, 2, 19).unwrap();
    let d = field_value_to_dynamic(&FieldValue::Date(Some(date)));
    assert_eq!(d.try_cast::<String>().unwrap(), "2026-02-19");
}

#[test]
fn test_field_value_to_dynamic_email() {
    let d = field_value_to_dynamic(&FieldValue::Email("a@b.com".into()));
    assert_eq!(d.try_cast::<String>().unwrap(), "a@b.com");
}

#[test]
fn test_dynamic_to_field_value_text() {
    let fv = dynamic_to_field_value(Dynamic::from("hi".to_string()), "text").unwrap();
    assert_eq!(fv, FieldValue::Text("hi".into()));
}

#[test]
fn test_dynamic_to_field_value_number() {
    let fv = dynamic_to_field_value(Dynamic::from(2.0_f64), "number").unwrap();
    assert_eq!(fv, FieldValue::Number(2.0));
}

#[test]
fn test_dynamic_to_field_value_boolean() {
    let fv = dynamic_to_field_value(Dynamic::from(false), "boolean").unwrap();
    assert_eq!(fv, FieldValue::Boolean(false));
}

#[test]
fn test_dynamic_to_field_value_date_none() {
    let fv = dynamic_to_field_value(Dynamic::UNIT, "date").unwrap();
    assert_eq!(fv, FieldValue::Date(None));
}

#[test]
fn test_dynamic_to_field_value_date_some() {
    use chrono::NaiveDate;
    let d = Dynamic::from("2026-02-19".to_string());
    let fv = dynamic_to_field_value(d, "date").unwrap();
    assert_eq!(fv, FieldValue::Date(Some(NaiveDate::from_ymd_opt(2026, 2, 19).unwrap())));
}

#[test]
fn test_dynamic_to_field_value_email() {
    let fv = dynamic_to_field_value(Dynamic::from("x@y.com".to_string()), "email").unwrap();
    assert_eq!(fv, FieldValue::Email("x@y.com".into()));
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_field_value_to_dynamic -- --nocapture
cargo test -p krillnotes-core test_dynamic_to_field_value -- --nocapture
```

Expected: compile error — functions do not exist yet.

**Step 3: Implement**

Add these two functions as private free functions directly above the `#[cfg(test)]` block in `scripting.rs`. Also add `use chrono::NaiveDate;` at the top of the file.

```rust
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

**Step 4: Run tests to verify they pass**

```bash
cargo test -p krillnotes-core test_field_value_to_dynamic -- --nocapture
cargo test -p krillnotes-core test_dynamic_to_field_value -- --nocapture
cargo test -p krillnotes-core
```

Expected: all 12 new tests PASS, no regressions.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting.rs
git commit -m "feat(core): add FieldValue <-> Dynamic conversion helpers"
```

---

### Task 3: Implement `run_on_save_hook`

**Files:**
- Modify: `krillnotes-core/src/core/scripting.rs`

The public method that scripts call after compilation. It builds a Rhai note map, invokes the stored closure, and parses the result back to Rust types.

**Step 1: Write the failing test**

Add to the `#[cfg(test)]` block:

```rust
#[test]
fn test_run_on_save_hook_sets_title() {
    let mut registry = SchemaRegistry::new().unwrap();
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
    let registry = SchemaRegistry::new().unwrap();
    let fields = HashMap::new();
    let result = registry
        .run_on_save_hook("TextNote", "id-1", "TextNote", "title", &fields)
        .unwrap();
    assert!(result.is_none());
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_run_on_save_hook -- --nocapture
```

Expected: compile error — `run_on_save_hook` does not exist yet.

**Step 3: Implement**

Add this method to the `impl SchemaRegistry` block, after `has_hook`:

```rust
/// Runs the pre-save hook registered for `schema_name`, if any.
///
/// Builds a Rhai note map from the provided values, calls the stored closure,
/// and parses the returned map back to Rust types.
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
    // Clone the entry out of the mutex so the lock is not held during the call.
    let entry = {
        let hooks = self.hooks.lock().unwrap();
        hooks.get(schema_name).cloned()
    };
    let entry = match entry {
        Some(e) => e,
        None => return Ok(None),
    };

    let schema = self.get_schema(schema_name)?;

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
        .call::<Dynamic>(&self.engine, &entry.ast, (Dynamic::from(note_map),))
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
    // Fields present in the schema but absent from the hook result fall back
    // to the original value.
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
```

**Step 4: Run tests to verify they pass**

```bash
cargo test -p krillnotes-core test_run_on_save_hook -- --nocapture
cargo test -p krillnotes-core
```

Expected: both new tests PASS, no regressions.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting.rs
git commit -m "feat(core): implement run_on_save_hook on SchemaRegistry"
```

---

### Task 4: Add `on_save` hook to `contact.rhai`

**Files:**
- Modify: `krillnotes-core/src/system_scripts/contact.rhai`

**Step 1: Write the failing test**

Add to the `#[cfg(test)]` block in `scripting.rs`:

```rust
#[test]
fn test_contact_on_save_hook_derives_title() {
    let registry = SchemaRegistry::new().unwrap();
    assert!(registry.has_hook("Contact"), "Contact schema should have an on_save hook");

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
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p krillnotes-core test_contact_on_save_hook -- --nocapture
```

Expected: FAIL — `has_hook("Contact")` returns false.

**Step 3: Add the hook to `contact.rhai`**

Append after the closing `});` of the `schema(...)` call:

```rhai
on_save("Contact", |note| {
    let last  = note.fields["last_name"];
    let first = note.fields["first_name"];
    if last != "" || first != "" {
        note.title = last + ", " + first;
    }
    note
});
```

The guard (`if last != "" || first != ""`) prevents a bare `", "` title when both name fields are empty on a freshly created contact.

**Step 4: Run test to verify it passes**

```bash
cargo test -p krillnotes-core test_contact_on_save_hook -- --nocapture
cargo test -p krillnotes-core
```

Expected: PASS, no regressions.

**Step 5: Commit**

```bash
git add krillnotes-core/src/system_scripts/contact.rhai \
        krillnotes-core/src/core/scripting.rs
git commit -m "feat(core): add on_save hook to Contact schema to derive title from name fields"
```

---

### Task 5: Wire the hook into `Workspace::update_note`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write the failing test**

Add to the `#[cfg(test)]` block in `workspace.rs`:

```rust
#[test]
fn test_update_contact_derives_title_from_hook() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Create a root note to act as parent
    let notes = ws.list_notes().unwrap();
    let root_id = notes[0].id.clone();

    let contact_id = ws
        .create_note(&root_id, crate::AddPosition::AsChild, "Contact")
        .unwrap();

    let mut fields = HashMap::new();
    fields.insert("first_name".to_string(), FieldValue::Text("Alice".to_string()));
    fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
    fields.insert("last_name".to_string(), FieldValue::Text("Walker".to_string()));
    fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
    fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
    fields.insert("email".to_string(), FieldValue::Email("".to_string()));
    fields.insert("birthdate".to_string(), FieldValue::Date(None));
    fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
    fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
    fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
    fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));

    let updated = ws
        .update_note(&contact_id, "ignored title".to_string(), fields)
        .unwrap();

    assert_eq!(updated.title, "Walker, Alice");
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p krillnotes-core test_update_contact_derives_title -- --nocapture
```

Expected: FAIL — `updated.title` is `"ignored title"` instead of `"Walker, Alice"`.

**Step 3: Implement**

In `workspace.rs`, at the start of `update_note`, add the node-type lookup and hook call. Insert the following block **before** the `let now = ...` line:

```rust
// Look up this note's schema so the pre-save hook can be dispatched.
let node_type: String = self
    .storage
    .connection()
    .query_row(
        "SELECT node_type FROM notes WHERE id = ?1",
        rusqlite::params![note_id],
        |row| row.get(0),
    )
    .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

// Run the pre-save hook. If a hook is registered it may modify title and fields.
let (title, fields) =
    match self
        .registry
        .run_on_save_hook(&node_type, note_id, &node_type, &title, &fields)?
    {
        Some((new_title, new_fields)) => (new_title, new_fields),
        None => (title, fields),
    };
```

**Step 4: Run test to verify it passes**

```bash
cargo test -p krillnotes-core test_update_contact_derives_title -- --nocapture
cargo test -p krillnotes-core
```

Expected: PASS, no regressions.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): call on_save hook in update_note before writing to database"
```

---

## Done

All five tasks complete. The hooks system is live:
- Rhai scripts call `on_save("SchemaName", |note| { ...; note })` to register a pre-save transform
- `SchemaRegistry` stores the closure + its compiled AST
- `Workspace::update_note` dispatches the hook before any SQL write
- `Contact` derives its title automatically from `last_name` and `first_name`
