# Script Name in Hook Runtime Errors — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Include the name of the script that registered a hook in any runtime error message it produces, so users know exactly which script to open.

**Architecture:** `HookEntry` gains a `script_name: String` field. A new `current_loading_script_name: Arc<Mutex<Option<String>>>` on `ScriptRegistry` is set before each script evaluation and read by the `schema()` closure when it constructs a `HookEntry`. `load_script` gains a `name: &str` parameter, which all call sites must supply. Error format strings in `run_on_save_hook` and `run_on_view_hook` are updated to embed the name.

**Tech Stack:** Rust, Rhai (`rhai` crate), `std::sync::{Arc, Mutex}`.

---

### Task 1: Write the failing tests

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (append two tests inside `mod tests`)

Add these two tests to the bottom of the `mod tests` block (after line 1241, before the closing `}`):

```rust
#[test]
fn test_on_save_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(
        r#"
        schema("Boom", #{
            fields: [ #{ name: "x", type: "text" } ],
            on_save: |note| {
                let _ = note.fields["no_such_field"];
                note
            }
        });
        "#,
        "My Exploding Script",
    ).unwrap();

    let fields = HashMap::new();
    let err = registry
        .run_on_save_hook("Boom", "id-1", "Boom", "title", &fields)
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("My Exploding Script"),
        "error should include script name, got: {msg}"
    );
}

#[test]
fn test_on_view_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(
        r#"
        schema("BoomView", #{
            fields: [],
            on_view: |note| {
                let _ = note.fields["no_such_field"];
                text("x")
            }
        });
        "#,
        "My View Script",
    ).unwrap();

    use crate::Note;
    let note = Note {
        id: "n1".to_string(), node_type: "BoomView".to_string(),
        title: "T".to_string(), parent_id: None, position: 0,
        created_at: 0, modified_at: 0, created_by: 0, modified_by: 0,
        fields: HashMap::new(), is_expanded: false,
    };
    let ctx = QueryContext {
        notes_by_id: HashMap::new(),
        children_by_id: HashMap::new(),
        notes_by_type: HashMap::new(),
    };
    let err = registry.run_on_view_hook(&note, ctx).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("My View Script"),
        "error should include script name, got: {msg}"
    );
}
```

**Step 1: Add the two tests**

These tests won't compile yet because `load_script` doesn't accept a name argument. That's expected — this is the failing state.

---

### Task 2: Add `script_name` to `HookEntry` and update error messages

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs`

**Step 1: Extend `HookEntry`**

Change lines 10–15:

```rust
/// A stored hook entry: the Rhai closure and the AST it was defined in.
#[derive(Clone, Debug)]
pub(super) struct HookEntry {
    pub(super) fn_ptr: FnPtr,
    pub(super) ast: AST,
    pub(super) script_name: String,
}
```

**Step 2: Update `run_on_save_hook` error message**

Change line 335 from:
```rust
.map_err(|e| KrillnotesError::Scripting(format!("on_save hook error: {e}")))?;
```
to:
```rust
.map_err(|e| KrillnotesError::Scripting(format!("on_save hook error in '{}': {e}", entry.script_name)))?;
```

**Step 3: Update `run_on_view_hook` error message**

Change line 393 from:
```rust
.map_err(|e| KrillnotesError::Scripting(format!("on_view hook error: {e}")))?;
```
to:
```rust
.map_err(|e| KrillnotesError::Scripting(format!("on_view hook error in '{}': {e}", entry.script_name)))?;
```

The crate won't compile yet because `HookEntry` construction sites (in `mod.rs`) are missing `script_name`. That's expected.

---

### Task 3: Thread `script_name` through `ScriptRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Add `current_loading_script_name` to `ScriptRegistry` struct**

After the existing `current_loading_ast` field (line 52), add:

```rust
current_loading_script_name: Arc<Mutex<Option<String>>>,
```

**Step 2: Initialise it in `new()`**

After line 65 (`let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));`), add:

```rust
let current_loading_script_name: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
```

**Step 3: Capture it in the `schema()` closure**

After line 71 (`let schema_ast_arc = Arc::clone(&current_loading_ast);`), add:

```rust
let schema_name_arc = Arc::clone(&current_loading_script_name);
```

**Step 4: Read it when constructing `HookEntry` (on_save)**

Change the `on_save_arc.lock().unwrap().insert(...)` call (line 83) from:

```rust
on_save_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast });
```
to:
```rust
let script_name = schema_name_arc.lock().unwrap()
    .clone()
    .unwrap_or_else(|| "<unknown>".to_string());
on_save_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name });
```

**Step 5: Same for `on_view` construction**

Change the `on_view_arc.lock().unwrap().insert(...)` call (line 92) from:

```rust
on_view_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast });
```
to:
```rust
let script_name = schema_name_arc.lock().unwrap()
    .clone()
    .unwrap_or_else(|| "<unknown>".to_string());
on_view_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name });
```

**Step 6: Include `current_loading_script_name` in the `Ok(Self { ... })` return**

At the end of `new()` (line 174), add `current_loading_script_name` to the struct literal:

```rust
Ok(Self {
    engine,
    current_loading_ast,
    current_loading_script_name,
    schema_registry,
    query_context,
})
```

**Step 7: Change `load_script` signature to accept a name**

Change line 205:
```rust
pub fn load_script(&mut self, script: &str) -> Result<()> {
```
to:
```rust
pub fn load_script(&mut self, script: &str, name: &str) -> Result<()> {
```

**Step 8: Set and clear `current_loading_script_name` in `load_script`**

Immediately after setting `current_loading_ast` (line 213), add:
```rust
*self.current_loading_script_name.lock().unwrap() = Some(name.to_string());
```

Immediately after clearing `current_loading_ast` (line 222), add:
```rust
*self.current_loading_script_name.lock().unwrap() = None;
```

The full updated `load_script` body should look like:

```rust
pub fn load_script(&mut self, script: &str, name: &str) -> Result<()> {
    let ast = self
        .engine
        .compile(script)
        .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;

    *self.current_loading_ast.lock().unwrap() = Some(ast.clone());
    *self.current_loading_script_name.lock().unwrap() = Some(name.to_string());

    let result = self
        .engine
        .eval_ast::<()>(&ast)
        .map_err(|e| KrillnotesError::Scripting(e.to_string()));

    *self.current_loading_ast.lock().unwrap() = None;
    *self.current_loading_script_name.lock().unwrap() = None;

    result
}
```

The crate won't compile yet because every `load_script` call site is missing the new `name` argument. That's expected.

---

### Task 4: Update all `load_script` call sites

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (5 sites)
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (all test call sites)

#### workspace.rs call sites

| Line | Old call | New call |
|------|----------|----------|
| 105 | `script_registry.load_script(&script.source_code)` | `script_registry.load_script(&script.source_code, &script.name)` |
| 201 | `ws.script_registry.load_script(&script.source_code)` | `ws.script_registry.load_script(&script.source_code, &script.name)` |
| 1131 | `self.script_registry.load_script(source_code)` | `self.script_registry.load_script(source_code, &fm.name)` |
| 1184 | `self.script_registry.load_script(source_code)` | `self.script_registry.load_script(source_code, &fm.name)` |
| 1348 | `self.script_registry.load_script(&script.source_code)` | `self.script_registry.load_script(&script.source_code, &script.name)` |

#### mod.rs test call sites

**Update the `load_text_note` helper** (line 342):
```rust
fn load_text_note(registry: &mut ScriptRegistry) {
    registry.load_script(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/system_scripts/00_text_note.rhai"
    )), "TextNote").expect("TextNote starter script should load");
}
```

**For all inline `r#"..."#` test call sites**, add `"test"` as the second argument. The pattern is:

```rust
// Before
registry.load_script(r#" ... "#).unwrap();
// After
registry.load_script(r#" ... "#, "test").unwrap();
```

Apply this to every `load_script` call that uses an inline string. There are approximately 35 such calls spread across the test functions.

**For `include_str!` calls to real system scripts**, pass a descriptive name:
- `01_contact.rhai` → `"Contact"`
- `04_book.rhai` → `"Book"`
- `starter_scripts()` loop at line 947 → `&starter.filename`

**Step 1: Apply all workspace.rs changes described in the table above**

**Step 2: Apply all mod.rs test changes described above**

Run: `cargo build -p krillnotes-core`

Expected: build succeeds with no errors.

---

### Task 5: Run tests and commit

**Step 1: Run all tests**

```bash
cargo test -p krillnotes-core
```

Expected: all tests pass, including the two new ones added in Task 1.

**Step 2: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs \
        krillnotes-core/src/core/scripting/mod.rs \
        krillnotes-core/src/core/workspace.rs
git commit -m "feat: include script name in on_save/on_view runtime error messages"
```

**Step 3: Mark TODO as done**

In `TODO.md`, change the first unchecked item from:

```
[ ] When a hook (on_save, on_view) throws a runtime error, the error popup should include the name of the script it came from, so the user knows where to look. The line number is already correct.
```

to:

```
✅ DONE! When a hook (on_save, on_view) throws a runtime error, the error popup should include the name of the script it came from, so the user knows where to look. The line number is already correct.
```

Commit:
```bash
git add TODO.md
git commit -m "chore: mark script runtime error script name task as done"
```
