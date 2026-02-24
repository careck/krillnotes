# on_add_child Hook Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an `on_add_child` schema hook that fires when a note is created as a child of — or moved under — a note whose schema defines the hook; the hook can modify both the parent note and the new child note.

**Architecture:** Follow the exact pattern of `on_save` and `on_view`. Add a new `on_add_child_hooks` HashMap to `SchemaRegistry`, extract the hook FnPtr during script registration, expose a public `run_on_add_child_hook` wrapper on `ScriptRegistry`, then call it from `create_note()` and `move_note()` in `workspace.rs`. All modifications are applied via raw SQL UPDATEs within the existing transaction — no nested `on_save` call. Finally update the scripting guide in the website repo.

**Tech Stack:** Rust, Rhai scripting engine, SQLite via rusqlite, Hugo static site (Markdown docs).

---

### Task 1: Add `AddChildResult` and `run_on_add_child_hook` to `SchemaRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs`

**Step 1: Write a failing test in `scripting/mod.rs`**

In `krillnotes-core/src/core/scripting/mod.rs`, inside the existing `#[cfg(test)] mod tests` block (currently at line 348), add:

```rust
#[test]
fn test_on_add_child_hook_modifies_parent_and_child() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Folder", #{
            fields: [
                #{ name: "count", type: "number", required: false },
            ],
            on_add_child: |parent_note, child_note| {
                parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                parent_note.title = "Folder (" + parent_note.fields["count"].to_string() + ")";
                child_note.title = "Child from hook";
                #{ parent: parent_note, child: child_note }
            }
        });
        schema("Item", #{
            fields: [
                #{ name: "name", type: "text", required: false },
            ],
        });
    "#, "test").unwrap();

    let mut parent_fields = std::collections::HashMap::new();
    parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

    let mut child_fields = std::collections::HashMap::new();
    child_fields.insert("name".to_string(), FieldValue::Text("".to_string()));

    let result = registry
        .run_on_add_child_hook(
            "Folder",
            "parent-id", "Folder", "Folder", &parent_fields,
            "child-id",  "Item",   "Untitled", &child_fields,
        )
        .unwrap();

    let result = result.expect("hook should return a result");
    let (p_title, p_fields) = result.parent.expect("should have parent update");
    assert_eq!(p_title, "Folder (1)");
    assert_eq!(p_fields["count"], FieldValue::Number(1.0));

    let (c_title, _) = result.child.expect("should have child update");
    assert_eq!(c_title, "Child from hook");
}

#[test]
fn test_on_add_child_hook_absent_returns_none() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Plain", #{
            fields: [],
        });
    "#, "test").unwrap();

    let result = registry
        .run_on_add_child_hook(
            "Plain",
            "p-id", "Plain", "Title", &std::collections::HashMap::new(),
            "c-id", "Plain", "Child", &std::collections::HashMap::new(),
        )
        .unwrap();

    assert!(result.is_none(), "no hook registered should return None");
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_on_add_child 2>&1 | tail -20
```

Expected: FAIL — `run_on_add_child_hook` doesn't exist yet.

**Step 3: Add `AddChildResult` struct to `schema.rs`**

In `krillnotes-core/src/core/scripting/schema.rs`, add after the `HookEntry` struct (after line 16):

```rust
/// Returned by [`SchemaRegistry::run_on_add_child_hook`].
///
/// Each field is `Some` only when the hook returned modifications for that note.
pub(super) struct AddChildResult {
    pub(super) parent: Option<(String, HashMap<String, FieldValue>)>,
    pub(super) child:  Option<(String, HashMap<String, FieldValue>)>,
}
```

**Step 4: Add `on_add_child_hooks` field to `SchemaRegistry`**

In `schema.rs`, update the `SchemaRegistry` struct (currently lines 222–227):

```rust
#[derive(Debug)]
pub(super) struct SchemaRegistry {
    schemas:            Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks:      Arc<Mutex<HashMap<String, HookEntry>>>,
    on_view_hooks:      Arc<Mutex<HashMap<String, HookEntry>>>,
    on_add_child_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
}
```

**Step 5: Update `new()` in `schema.rs`**

Update the `new()` constructor (lines 230–236):

```rust
pub(super) fn new() -> Self {
    Self {
        schemas:            Arc::new(Mutex::new(HashMap::new())),
        on_save_hooks:      Arc::new(Mutex::new(HashMap::new())),
        on_view_hooks:      Arc::new(Mutex::new(HashMap::new())),
        on_add_child_hooks: Arc::new(Mutex::new(HashMap::new())),
    }
}
```

**Step 6: Add `run_on_add_child_hook` method to `SchemaRegistry`**

In `schema.rs`, add after the closing `}` of `run_on_save_hook` (after line 365):

```rust
/// Runs the `on_add_child` hook for the parent's schema, if registered.
///
/// Returns `None` when no hook is registered. On success, returns an
/// [`AddChildResult`] where each field is `Some` only when the hook returned
/// modifications for that note.
pub(super) fn run_on_add_child_hook(
    &self,
    engine: &Engine,
    parent_schema: &Schema,
    parent_id: &str,
    parent_type: &str,
    parent_title: &str,
    parent_fields: &HashMap<String, FieldValue>,
    child_schema: &Schema,
    child_id: &str,
    child_type: &str,
    child_title: &str,
    child_fields: &HashMap<String, FieldValue>,
) -> Result<Option<AddChildResult>> {
    let entry = {
        let hooks = self.on_add_child_hooks
            .lock()
            .map_err(|_| KrillnotesError::Scripting("on_add_child hook lock poisoned".to_string()))?;
        hooks.get(&parent_schema.name).cloned()
    };
    let entry = match entry {
        Some(e) => e,
        None => return Ok(None),
    };

    // Build parent note map
    let mut p_fields_map = Map::new();
    for (k, v) in parent_fields {
        p_fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
    }
    let mut parent_map = Map::new();
    parent_map.insert("id".into(),        Dynamic::from(parent_id.to_string()));
    parent_map.insert("node_type".into(), Dynamic::from(parent_type.to_string()));
    parent_map.insert("title".into(),     Dynamic::from(parent_title.to_string()));
    parent_map.insert("fields".into(),    Dynamic::from(p_fields_map));

    // Build child note map
    let mut c_fields_map = Map::new();
    for (k, v) in child_fields {
        c_fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
    }
    let mut child_map = Map::new();
    child_map.insert("id".into(),        Dynamic::from(child_id.to_string()));
    child_map.insert("node_type".into(), Dynamic::from(child_type.to_string()));
    child_map.insert("title".into(),     Dynamic::from(child_title.to_string()));
    child_map.insert("fields".into(),    Dynamic::from(c_fields_map));

    let result = entry
        .fn_ptr
        .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(parent_map), Dynamic::from(child_map)))
        .map_err(|e| KrillnotesError::Scripting(
            format!("on_add_child hook error in '{}': {e}", entry.script_name)
        ))?;

    // If the hook returned unit (no-op), treat as no modification
    if result.is_unit() {
        return Ok(Some(AddChildResult { parent: None, child: None }));
    }

    let result_map = result.try_cast::<Map>().ok_or_else(|| {
        KrillnotesError::Scripting(
            "on_add_child hook must return a map #{{ parent: ..., child: ... }} or ()".to_string()
        )
    })?;

    // Extract optional parent modifications
    let parent_update = if let Some(pm) = result_map.get("parent").and_then(|v| v.clone().try_cast::<Map>()) {
        let new_title = pm.get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result parent 'title' must be a string".to_string()))?;
        let new_fields_dyn = pm.get("fields")
            .and_then(|v| v.clone().try_cast::<Map>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result parent 'fields' must be a map".to_string()))?;
        let mut new_fields = HashMap::new();
        for field_def in &parent_schema.fields {
            let dyn_val = new_fields_dyn.get(field_def.name.as_str()).cloned().unwrap_or(Dynamic::UNIT);
            let fv = dynamic_to_field_value(dyn_val, &field_def.field_type)
                .map_err(|e| KrillnotesError::Scripting(format!("parent field '{}': {e}", field_def.name)))?;
            new_fields.insert(field_def.name.clone(), fv);
        }
        Some((new_title, new_fields))
    } else {
        None
    };

    // Extract optional child modifications
    let child_update = if let Some(cm) = result_map.get("child").and_then(|v| v.clone().try_cast::<Map>()) {
        let new_title = cm.get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result child 'title' must be a string".to_string()))?;
        let new_fields_dyn = cm.get("fields")
            .and_then(|v| v.clone().try_cast::<Map>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result child 'fields' must be a map".to_string()))?;
        let mut new_fields = HashMap::new();
        for field_def in &child_schema.fields {
            let dyn_val = new_fields_dyn.get(field_def.name.as_str()).cloned().unwrap_or(Dynamic::UNIT);
            let fv = dynamic_to_field_value(dyn_val, &field_def.field_type)
                .map_err(|e| KrillnotesError::Scripting(format!("child field '{}': {e}", field_def.name)))?;
            new_fields.insert(field_def.name.clone(), fv);
        }
        Some((new_title, new_fields))
    } else {
        None
    };

    Ok(Some(AddChildResult { parent: parent_update, child: child_update }))
}
```

Also expose `on_add_child_hooks` Arc as a getter (needed by `mod.rs` to register entries) — add next to the existing `on_save_hooks` getter or expose the arc directly via a method `on_add_child_hooks_arc()`:

```rust
pub(super) fn on_add_child_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
    Arc::clone(&self.on_add_child_hooks)
}
```

Check how `on_save_hooks` is exposed to `mod.rs` and follow the same pattern. If `mod.rs` directly accesses `schema_registry.on_save_hooks` as a field (since it's `pub(super)`), the same access works for `on_add_child_hooks`.

**Step 7: Run tests again**

```bash
cargo test -p krillnotes-core test_on_add_child 2>&1 | tail -20
```

Expected: Tests fail — `run_on_add_child_hook` still not exposed on `ScriptRegistry`.

**Step 8: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/scripting/schema.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: add AddChildResult and run_on_add_child_hook to SchemaRegistry"
```

---

### Task 2: Register hook extraction in `ScriptRegistry` and expose public wrapper

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Add `on_add_child_arc` inside `ScriptRegistry::new()`**

In `mod.rs`, inside `ScriptRegistry::new()` (where `on_save_arc` and `on_view_arc` are created — around lines 60–75), add:

```rust
let on_add_child_arc = schema_registry.on_add_child_hooks_arc(); // or direct field access
```

(Follow the exact pattern used for `on_save_arc`.)

**Step 2: Extract `on_add_child` FnPtr during schema registration**

In `mod.rs`, inside the `engine.register_fn("schema", ...)` closure (lines 75–105), after the existing `on_view` extraction block, add:

```rust
if let Some(fn_ptr) = def.get("on_add_child").and_then(|v| v.clone().try_cast::<FnPtr>()) {
    let ast = schema_ast_arc.lock().unwrap().clone()
        .ok_or_else(|| -> Box<EvalAltResult> {
            "schema() called outside of load_script".to_string().into()
        })?;
    let script_name = schema_name_arc.lock().unwrap()
        .clone()
        .unwrap_or_else(|| "<unknown>".to_string());
    on_add_child_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name });
}
```

**Step 3: Add public `AddChildResult` re-export and wrapper method**

First, add a public re-export at the top of `mod.rs` (with the other public types):

```rust
pub use schema::AddChildResult;
```

Then add the public wrapper method to `ScriptRegistry`'s `impl` block (after the existing `run_on_save_hook` wrapper):

```rust
/// Runs the `on_add_child` hook registered for `parent_schema_name`, if any.
pub fn run_on_add_child_hook(
    &self,
    parent_schema_name: &str,
    parent_id: &str,
    parent_type: &str,
    parent_title: &str,
    parent_fields: &HashMap<String, FieldValue>,
    child_schema_name: &str,
    child_id: &str,
    child_type: &str,
    child_title: &str,
    child_fields: &HashMap<String, FieldValue>,
) -> Result<Option<AddChildResult>> {
    let parent_schema = self.schema_registry.get(parent_schema_name)?;
    let child_schema  = self.schema_registry.get(child_schema_name)?;
    self.schema_registry.run_on_add_child_hook(
        &self.engine,
        &parent_schema,
        parent_id, parent_type, parent_title, parent_fields,
        &child_schema,
        child_id, child_type, child_title, child_fields,
    )
}
```

**Step 4: Run the tests**

```bash
cargo test -p krillnotes-core test_on_add_child 2>&1 | tail -20
```

Expected: Both tests PASS.

**Step 5: Run full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: All tests pass. Fix any compilation errors before proceeding.

**Step 6: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/scripting/mod.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: register on_add_child hook extraction and expose public wrapper"
```

---

### Task 3: Invoke hook in `create_note()`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write a failing workspace test**

In `workspace.rs`, inside `#[cfg(test)] mod tests` (line 1600), add:

```rust
#[test]
fn test_on_add_child_hook_fires_on_create() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "").unwrap();

    // Load a script that defines Folder (with hook) and Item
    ws.script_registry_mut().load_script(r#"
        schema("Folder", #{
            fields: [
                #{ name: "count", type: "number", required: false },
            ],
            on_add_child: |parent_note, child_note| {
                parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                parent_note.title = "Folder (1)";
                #{ parent: parent_note, child: child_note }
            }
        });
        schema("Item", #{
            fields: [],
        });
    "#, "test").unwrap();

    // Create a root Folder note
    let root = ws.list_all_notes().unwrap()[0].clone();
    let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();

    // Create an Item under the Folder
    ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();

    // The hook should have updated the Folder's count and title
    let folder = ws.get_note(&folder_id).unwrap();
    assert_eq!(folder.title, "Folder (1)");
    assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
}
```

> **Note:** If `script_registry_mut()` doesn't exist, check how tests in `workspace.rs` currently load scripts (e.g., look at `test_on_save_hook_fires_on_update` or similar). Add a `pub(crate) fn script_registry_mut(&mut self) -> &mut ScriptRegistry` accessor to `Workspace` if needed.

**Step 2: Run the test to confirm it fails**

```bash
cargo test -p krillnotes-core test_on_add_child_hook_fires_on_create 2>&1 | tail -20
```

Expected: FAIL — hook not called yet.

**Step 3: Add the hook call to `create_note()`**

In `workspace.rs`, in the `create_note()` method, insert the following block **after** the DB insert (after line 337, before the `// Log operation` comment at line 339):

```rust
// Run on_add_child hook if the parent's schema defines one.
// Allowed-parent and allowed-children checks have already passed above.
if let Some(ref parent_id) = note.parent_id {
    let parent_note = self.get_note(parent_id)?;
    if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
        &parent_note.node_type,
        &parent_note.id, &parent_note.node_type, &parent_note.title, &parent_note.fields,
        &note.node_type,
        &note.id, &note.node_type, &note.title, &note.fields,
    )? {
        let now = chrono::Utc::now().timestamp();
        if let Some((new_title, new_fields)) = hook_result.child {
            let fields_json = serde_json::to_string(&new_fields)?;
            tx.execute(
                "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                rusqlite::params![new_title, fields_json, now, note.id],
            )?;
        }
        if let Some((new_title, new_fields)) = hook_result.parent {
            let fields_json = serde_json::to_string(&new_fields)?;
            tx.execute(
                "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                rusqlite::params![new_title, fields_json, now, parent_note.id],
            )?;
        }
    }
}
```

> **Careful:** `self.get_note()` uses `self.storage.connection()` (immutable borrow). But `tx` is a mutable borrow of the same connection. This is a borrow conflict in Rust. To work around it, fetch the parent note **before** opening the transaction, but after the validation checks. Specifically, move the parent note fetch to just before `let tx = ...` (line 310), storing it in a variable used both for the hook call and later inside the transaction.
>
> Concretely: after line 294 (end of `allowed_children_types` block), add:
> ```rust
> let hook_parent = if let Some(ref pid) = final_parent {
>     Some(self.get_note(pid)?)
> } else {
>     None
> };
> ```
> Then inside the transaction after insert, use `hook_parent` instead of calling `self.get_note()` again.

**Step 4: Run the test**

```bash
cargo test -p krillnotes-core test_on_add_child_hook_fires_on_create 2>&1 | tail -20
```

Expected: PASS.

**Step 5: Run full suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: All tests pass.

**Step 6: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/workspace.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: invoke on_add_child hook in create_note()"
```

---

### Task 4: Invoke hook in `move_note()`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write a failing test**

In `workspace.rs` tests, add:

```rust
#[test]
fn test_on_add_child_hook_fires_on_move() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "").unwrap();

    ws.script_registry_mut().load_script(r#"
        schema("Folder", #{
            fields: [
                #{ name: "count", type: "number", required: false },
            ],
            on_add_child: |parent_note, child_note| {
                parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                parent_note.title = "Folder (1)";
                #{ parent: parent_note, child: child_note }
            }
        });
        schema("Item", #{
            fields: [],
        });
    "#, "test").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    // Create a Folder and an Item as siblings under root
    let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
    let item_id   = ws.create_note(&root.id, AddPosition::AsChild, "Item").unwrap();

    // Move Item under Folder — hook should fire
    ws.move_note(&item_id, Some(&folder_id), 0).unwrap();

    let folder = ws.get_note(&folder_id).unwrap();
    assert_eq!(folder.title, "Folder (1)");
    assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
}
```

**Step 2: Run to confirm it fails**

```bash
cargo test -p krillnotes-core test_on_add_child_hook_fires_on_move 2>&1 | tail -20
```

Expected: FAIL.

**Step 3: Add the hook call to `move_note()`**

In `workspace.rs`, in `move_note()`, insert the following block **after** step 7 (after line 886, the `UPDATE notes SET parent_id` statement) and **before** the `// 8. Log a MoveNote operation` comment (line 888):

```rust
// Run on_add_child hook if the new parent's schema defines one.
if let Some(new_pid) = new_parent_id {
    // note_to_move was fetched before the transaction (line 827) — its
    // fields/title are unchanged by the move, so reuse it.
    // parent_note was fetched at line 848 — also safe to reuse.
    let parent_note = self.get_note(new_pid)?; // re-fetch for latest title/fields
    if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
        &parent_note.node_type,
        &parent_note.id, &parent_note.node_type, &parent_note.title, &parent_note.fields,
        &note_to_move.node_type,
        &note_to_move.id, &note_to_move.node_type, &note_to_move.title, &note_to_move.fields,
    )? {
        let hook_now = chrono::Utc::now().timestamp();
        if let Some((new_title, new_fields)) = hook_result.child {
            let fields_json = serde_json::to_string(&new_fields)?;
            tx.execute(
                "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                rusqlite::params![new_title, fields_json, hook_now, note_to_move.id],
            )?;
        }
        if let Some((new_title, new_fields)) = hook_result.parent {
            let fields_json = serde_json::to_string(&new_fields)?;
            tx.execute(
                "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                rusqlite::params![new_title, fields_json, hook_now, parent_note.id],
            )?;
        }
    }
}
```

> **Borrow note:** `self.get_note()` borrows `self.storage` immutably, but `tx` is a mutable borrow of the same connection. Same issue as Task 3. Fetch `parent_note` **before** opening the transaction (before line 866) by storing it:
> ```rust
> let hook_new_parent = if let Some(pid) = new_parent_id {
>     Some(self.get_note(pid)?)
> } else {
>     None
> };
> ```
> Then inside the transaction use `hook_new_parent` instead of calling `self.get_note()`.

**Step 4: Run the test**

```bash
cargo test -p krillnotes-core test_on_add_child_hook_fires_on_move 2>&1 | tail -20
```

Expected: PASS.

**Step 5: Run full suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: All tests pass.

**Step 6: Commit**

```bash
git -C /Users/careck/Source/Krillnotes add krillnotes-core/src/core/workspace.rs
git -C /Users/careck/Source/Krillnotes commit -m "feat: invoke on_add_child hook in move_note()"
```

---

### Task 5: Update the scripting guide

**Files:**
- Modify: `krillnotes-website/content/docs/scripting.md`

This is a documentation-only task. No tests. Make all edits in one pass, then commit.

**Step 1: Update the table of contents**

Change:
```markdown
5. [on_save hook](#5-on_save-hook)
6. [on_view hook](#6-on_view-hook)
7. [Display helpers](#7-display-helpers)
8. [Query functions](#8-query-functions)
9. [Introspection functions](#9-introspection-functions)
10. [Tips and patterns](#10-tips-and-patterns)
11. [Built-in script examples](#11-built-in-script-examples)
```

To:
```markdown
5. [on_save hook](#5-on_save-hook)
6. [on_view hook](#6-on_view-hook)
7. [on_add_child hook](#7-on_add_child-hook)
8. [Display helpers](#8-display-helpers)
9. [Query functions](#9-query-functions)
10. [Introspection functions](#10-introspection-functions)
11. [Tips and patterns](#11-tips-and-patterns)
12. [Built-in script examples](#12-built-in-script-examples)
```

**Step 2: Update section 1 (script structure)**

In the table under "Top-level call", update the description row to mention all three hooks:

Change:
```
Hooks (`on_save`, `on_view`) are defined as keys directly inside the map passed to `schema()` — not as separate top-level calls.
```

To:
```
Hooks (`on_save`, `on_view`, `on_add_child`) are defined as keys directly inside the map passed to `schema()` — not as separate top-level calls.
```

**Step 3: Update section 2 (defining schemas)**

In the schema template, change the `// --- optional hooks ---` block from:
```rhai
    // --- optional hooks ---
    on_save: |note| { /* … */ note },
    on_view: |note| { /* … */ text("") },
```

To:
```rhai
    // --- optional hooks ---
    on_save:      |note| { /* … */ note },
    on_view:      |note| { /* … */ text("") },
    on_add_child: |parent_note, child_note| { /* … */ #{ parent: parent_note, child: child_note } },
```

**Step 4: Update section 4 (schema options)**

After the `allowed_children_types` block, add a note:

```markdown
> **Validation order:** `allowed_parent_types` and `allowed_children_types` are always checked **before** any hook runs. If validation fails the operation is aborted and no hook fires.
```

**Step 5: Add new section 7 — on_add_child hook**

Insert the following section between the closing `---` of section 6 (`on_view hook`) and the start of what is currently section 7 (`Display helpers`):

````markdown
---

## 7. on_add_child hook

The `on_add_child` hook runs whenever a note is created as a child — or moved via drag-and-drop — under a note whose schema defines the hook. It receives both the parent note and the child note, and can return modifications to either or both.

It is defined as an `on_add_child` key inside the parent's `schema()` call.

```rhai
schema("TypeName", #{
    fields: [ /* … */ ],
    on_add_child: |parent_note, child_note| {
        // modify parent_note and/or child_note
        #{ parent: parent_note, child: child_note }
    }
});
```

### Signature

`|parent_note, child_note| -> Map`

- `parent_note` — the note whose schema defines this hook (same map shape as `on_save`)
- `child_note` — the new child (on creation: has schema default fields; on move: has existing data)
- **Return value:** a Rhai map with optional `parent` and/or `child` keys. Only present keys are persisted. Returning `()` is a no-op for both notes.

### The note map

Both arguments have the same shape:

| Key | Type | Writable |
|---|---|---|
| `note.id` | String | No (ignored if changed) |
| `note.node_type` | String | No (ignored if changed) |
| `note.title` | String | Yes |
| `note.fields` | Map | Yes (individual keys) |

### When it fires

| Operation | Fires? |
|---|---|
| Note created as a child | Yes |
| Note moved under a new parent | Yes |
| Note created at root level (no parent) | No |

### Validation order

`allowed_parent_types` and `allowed_children_types` checks run **before** the hook. If either check fails the operation is aborted and the hook never runs.

### Error handling

Any runtime error in the hook aborts the entire operation (the note is not created or moved) and shows an error with the script name and line number.

### Example — child count in parent title

```rhai
schema("ContactsFolder", #{
    fields: [
        #{ name: "child_count", type: "number", can_view: true, can_edit: false },
    ],
    on_add_child: |parent_note, child_note| {
        let count = (parent_note.fields["child_count"] ?? 0.0) + 1.0;
        parent_note.fields["child_count"] = count;
        parent_note.title = "Contacts (" + count.to_int().to_string() + ")";
        #{ parent: parent_note, child: child_note }
    }
});
```

### Example — no modification needed

Return `()` or an empty map to leave both notes unchanged:

```rhai
on_add_child: |parent_note, child_note| {
    // side-effect only (e.g. external call), no note changes
    ()
}
```
````

**Step 6: Renumber existing sections 7–11 → 8–12**

Rename all the headings and their anchor IDs:
- `## 7. Display helpers` → `## 8. Display helpers`
- `## 8. Query functions` → `## 9. Query functions`
- `## 9. Introspection functions` → `## 10. Introspection functions`
- `## 10. Tips and patterns` → `## 11. Tips and patterns`
- `## 11. Built-in script examples` → `## 12. Built-in script examples`

**Step 7: Update section 11 tips (now section 11 after renumber)**

Add a new "Child count with on_add_child" tip after the existing "Folder / item pair" tip:

````markdown
### Child count with `on_add_child`

Track how many children a container note has using a derived field updated by the hook:

```rhai
schema("ProjectFolder", #{
    fields: [
        #{ name: "item_count", type: "number", can_view: true, can_edit: false },
    ],
    allowed_children_types: ["Project"],
    on_add_child: |parent_note, child_note| {
        let count = (parent_note.fields["item_count"] ?? 0.0) + 1.0;
        parent_note.fields["item_count"] = count;
        parent_note.title = "Projects (" + count.to_int().to_string() + ")";
        #{ parent: parent_note, child: child_note }
    }
});
```

Note: this count only increases (add) — it does not decrease when notes are deleted or moved away. To maintain accurate counts, use `on_view` to compute the count live from `get_children()` instead.
````

**Step 8: Commit**

```bash
git -C /Users/careck/Source/krillnotes-website add content/docs/scripting.md
git -C /Users/careck/Source/krillnotes-website commit -m "docs: document on_add_child hook"
```

---

### Task 6: Build verification

**Step 1: Build the desktop app**

```bash
cargo build -p krillnotes-core -p krillnotes-desktop 2>&1 | tail -30
```

Expected: Compiles without errors or warnings (warnings are acceptable but errors are not).

**Step 2: Run the full test suite one final time**

```bash
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: All tests pass.

**Step 3: Commit (if any fixup needed)**

If step 1 or 2 required any fixes, commit them:

```bash
git -C /Users/careck/Source/Krillnotes add -p
git -C /Users/careck/Source/Krillnotes commit -m "fix: compilation issues after on_add_child hook"
```
