# Tree Action Mutations Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `create_note(parent_id, node_type)` and `update_note(note)` host functions to tree action closures, with all writes applied atomically in a single SQLite transaction after the closure returns.

**Architecture:** Tree action closures queue creates and updates into an `ActionTxContext` via a shared `Arc<Mutex<>>` (the same pattern as `query_context`). `get_children` and `get_note` also consult the in-memory cache in `ActionTxContext` so scripts can reference newly created notes within the same closure. After the closure succeeds, the workspace applies all queued operations in one transaction; on error, the queue is discarded (nothing written).

**Tech Stack:** Rust, Rhai scripting engine, rusqlite (SQLite), krillnotes-core crate.

---

### Task 1: Add action types to `hooks.rs`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/hooks.rs`

No tests needed — these are pure data types.

**Step 1: Add the types**

Add after the existing `TreeActionEntry` definition in `hooks.rs`:

```rust
/// Spec for a note to be created by a tree action.
#[derive(Debug, Clone)]
pub struct ActionCreate {
    pub id:        String,
    pub parent_id: String,
    pub node_type: String,
    pub title:     String,
    pub fields:    std::collections::HashMap<String, crate::core::note::FieldValue>,
}

/// Spec for a note to be updated by a tree action.
#[derive(Debug, Clone)]
pub struct ActionUpdate {
    pub note_id: String,
    pub title:   String,
    pub fields:  std::collections::HashMap<String, crate::core::note::FieldValue>,
}

/// Shared mutable context active during a tree action closure.
/// Host functions (`create_note`, `update_note`) queue operations here.
/// `get_children` / `get_note` also read from `note_cache` to see in-flight notes.
#[derive(Debug, Default)]
pub struct ActionTxContext {
    pub creates:    Vec<ActionCreate>,
    pub updates:    Vec<ActionUpdate>,
    /// Note maps (same Dynamic shape as `note_to_rhai_dynamic`) keyed by note ID.
    /// Populated by `create_note`; kept up-to-date by `update_note`.
    pub note_cache: std::collections::HashMap<String, rhai::Dynamic>,
}
```

**Step 2: Add `TreeActionResult` (return value from `invoke_tree_action_hook`)**

Add after `ActionTxContext`:

```rust
/// Return value from `invoke_tree_action_hook`.
#[derive(Debug, Default)]
pub struct TreeActionResult {
    /// If the closure returned an array of IDs, they are placed here (reorder path).
    pub reorder:  Option<Vec<String>>,
    pub creates:  Vec<ActionCreate>,
    pub updates:  Vec<ActionUpdate>,
}
```

**Step 3: Verify it compiles**

```bash
cargo build -p krillnotes-core 2>&1 | head -40
```

Expected: no new errors (types are unused for now).

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/scripting/hooks.rs
git commit -m "feat: add ActionTxContext and TreeActionResult types"
```

---

### Task 2: Register `create_note` host function

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs` (add `Clone`)
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Make `SchemaRegistry` cloneable**

In `schema.rs`, find the `SchemaRegistry` struct definition (around line 233) and add `#[derive(Clone)]`:

```rust
#[derive(Clone)]          // ADD THIS LINE
pub(super) struct SchemaRegistry {
    schemas:            Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks:      Arc<Mutex<HashMap<String, HookEntry>>>,
    on_view_hooks:      Arc<Mutex<HashMap<String, HookEntry>>>,
    on_add_child_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
}
```

This is a shallow clone (all fields are `Arc<Mutex<>>`), so the clone shares live data.

**Step 2: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the bottom of `scripting/mod.rs`:

```rust
#[test]
fn test_create_note_returns_note_map_with_defaults() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Task", #{
            fields: [
                #{ name: "status", type: "text", required: false },
            ]
        });
        add_tree_action("Make Task", ["Task"], |note| {
            let t = create_note(note.id, "Task");
            // Must return a map with id, node_type, title, fields
            assert(t.node_type == "Task", "node_type must be Task");
            assert(t.id != "", "id must not be empty");
            assert(t.fields.status == "", "status must default to empty string");
        });
    "#, "test").unwrap();

    let note = make_test_note("parent1", "Task");
    let ctx  = make_empty_ctx();
    let result = registry.invoke_tree_action_hook("Make Task", &note, ctx).unwrap();
    assert_eq!(result.creates.len(), 1, "one pending create expected");
    assert_eq!(result.creates[0].node_type, "Task");
    assert_eq!(result.creates[0].parent_id, "parent1");
}
```

Add helpers at the top of the test module (or alongside other helpers if they already exist):

```rust
fn make_test_note(id: &str, node_type: &str) -> crate::Note {
    crate::Note {
        id: id.into(), title: "Test".into(),
        node_type: node_type.into(), parent_id: None,
        fields: Default::default(), position: 0,
        created_at: 0, modified_at: 0, created_by: 0, modified_by: 0,
        is_expanded: false,
    }
}

fn make_empty_ctx() -> QueryContext {
    QueryContext {
        notes_by_id:    Default::default(),
        children_by_id: Default::default(),
        notes_by_type:  Default::default(),
    }
}
```

**Step 3: Run the test, verify it fails**

```bash
cargo test -p krillnotes-core test_create_note_returns_note_map_with_defaults -- --nocapture 2>&1 | tail -20
```

Expected: `FAILED` — `create_note` is not registered yet.

**Step 4: Add `action_ctx` field to `ScriptRegistry`**

In `mod.rs`, add to the `ScriptRegistry` struct:

```rust
pub struct ScriptRegistry {
    engine: Engine,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
    current_loading_script_name: Arc<Mutex<Option<String>>>,
    schema_owners: Arc<Mutex<HashMap<String, String>>>,
    schema_registry: schema::SchemaRegistry,
    hook_registry: hooks::HookRegistry,
    query_context: Arc<Mutex<Option<QueryContext>>>,
    action_ctx: Arc<Mutex<Option<hooks::ActionTxContext>>>,   // ADD THIS
}
```

In `ScriptRegistry::new()`, initialize it and add to the returned struct:

```rust
let action_ctx: Arc<Mutex<Option<hooks::ActionTxContext>>> = Arc::new(Mutex::new(None));
```

And in the `Ok(ScriptRegistry { ... })` block at the end:

```rust
action_ctx,
```

**Step 5: Register `create_note` host function in `ScriptRegistry::new()`**

After the existing `get_notes_of_type` registration, add:

```rust
// create_note(parent_id, node_type) — available inside add_tree_action closures only.
let action_ctx_create = Arc::clone(&action_ctx);
let schema_reg_create = schema_registry.clone();
engine.register_fn(
    "create_note",
    move |parent_id: String, node_type: String|
        -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>>
    {
        // Guard: only callable within an active tree action.
        let mut ctx_guard = action_ctx_create.lock().unwrap();
        let ctx = ctx_guard.as_mut().ok_or_else(|| {
            rhai::EvalAltResult::ErrorRuntime(
                "create_note() called outside a tree action".into(),
                rhai::Position::NONE,
            )
        })?;

        // Look up schema to get default fields.
        let schema = schema_reg_create
            .get(&node_type)
            .map_err(|e| rhai::EvalAltResult::ErrorRuntime(
                format!("create_note: unknown schema {:?}: {e}", node_type).into(),
                rhai::Position::NONE,
            ))?;

        // Generate a new UUID.
        let id = uuid::Uuid::new_v4().to_string();

        // Build the note Dynamic map (same shape as note_to_rhai_dynamic).
        let fields = schema.default_fields();
        let mut fields_map = rhai::Map::new();
        for (k, v) in &fields {
            fields_map.insert(
                k.as_str().into(),
                crate::core::scripting::field_value_to_dynamic(v),
            );
        }
        let mut note_map = rhai::Map::new();
        note_map.insert("id".into(),        rhai::Dynamic::from(id.clone()));
        note_map.insert("node_type".into(), rhai::Dynamic::from(node_type.clone()));
        note_map.insert("title".into(),     rhai::Dynamic::from(String::new()));
        note_map.insert("fields".into(),    rhai::Dynamic::from(fields_map));
        let dyn_note = rhai::Dynamic::from_map(note_map);

        // Queue the create and populate the cache.
        ctx.note_cache.insert(id.clone(), dyn_note.clone());
        ctx.creates.push(hooks::ActionCreate {
            id,
            parent_id,
            node_type,
            title: String::new(),
            fields,
        });

        Ok(dyn_note)
    },
);
```

**Step 6: Run the test, verify it passes**

```bash
cargo test -p krillnotes-core test_create_note_returns_note_map_with_defaults -- --nocapture 2>&1 | tail -10
```

Expected: `PASSED`.

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs \
        krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: register create_note host function for tree actions"
```

---

### Task 3: Register `update_note` host function

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_update_note_queues_update_for_existing_note() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Task", #{
            fields: [
                #{ name: "status", type: "text", required: false },
            ]
        });
        add_tree_action("Mark Done", ["Task"], |note| {
            note.fields.status = "Done";
            note.title = "Completed";
            update_note(note);
        });
    "#, "test").unwrap();

    let note = make_test_note("n1", "Task");
    let ctx  = make_empty_ctx();
    let result = registry.invoke_tree_action_hook("Mark Done", &note, ctx).unwrap();
    assert_eq!(result.updates.len(), 1);
    assert_eq!(result.updates[0].note_id, "n1");
    assert_eq!(result.updates[0].title, "Completed");
    assert_eq!(
        result.updates[0].fields.get("status"),
        Some(&crate::core::note::FieldValue::Text("Done".into())),
    );
}

#[test]
fn test_update_note_on_inflight_note_updates_create_spec() {
    // When update_note is called on a note that was just create_note'd in the
    // same action, the create spec is updated (no separate ActionUpdate entry).
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Task", #{
            fields: [#{ name: "status", type: "text", required: false }]
        });
        add_tree_action("New Task", ["Task"], |note| {
            let t = create_note(note.id, "Task");
            t.title = "My Task";
            t.fields.status = "Open";
            update_note(t);
        });
    "#, "test").unwrap();

    let note = make_test_note("parent1", "Task");
    let result = registry.invoke_tree_action_hook("New Task", &note, make_empty_ctx()).unwrap();

    assert_eq!(result.creates.len(), 1, "one create, not a separate update");
    assert_eq!(result.updates.len(), 0, "no separate update for inflight note");
    assert_eq!(result.creates[0].title, "My Task");
    assert_eq!(
        result.creates[0].fields.get("status"),
        Some(&crate::core::note::FieldValue::Text("Open".into())),
    );
}
```

**Step 2: Run, verify they fail**

```bash
cargo test -p krillnotes-core test_update_note -- --nocapture 2>&1 | tail -20
```

Expected: `FAILED`.

**Step 3: Register `update_note` in `ScriptRegistry::new()`**

After the `create_note` registration, add:

```rust
// update_note(note) — persists title/field changes; only in tree action closures.
let action_ctx_update = Arc::clone(&action_ctx);
let schema_reg_update = schema_registry.clone();
engine.register_fn(
    "update_note",
    move |note_map: rhai::Dynamic|
        -> std::result::Result<(), Box<rhai::EvalAltResult>>
    {
        let map = note_map.clone().try_cast::<rhai::Map>().ok_or_else(|| {
            rhai::EvalAltResult::ErrorRuntime(
                "update_note: argument must be a note map".into(),
                rhai::Position::NONE,
            )
        })?;

        let note_id = map.get("id")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| rhai::EvalAltResult::ErrorRuntime(
                "update_note: note map must have an `id` field".into(),
                rhai::Position::NONE,
            ))?;
        let node_type = map.get("node_type")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();
        let title = map.get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();
        let fields_dyn = map.get("fields")
            .and_then(|v| v.clone().try_cast::<rhai::Map>())
            .unwrap_or_default();

        // Convert Dynamic fields → FieldValue using schema.
        let schema = schema_reg_update.get(&node_type).map_err(|e| {
            rhai::EvalAltResult::ErrorRuntime(
                format!("update_note: unknown schema {:?}: {e}", node_type).into(),
                rhai::Position::NONE,
            )
        })?;
        let mut fields = std::collections::HashMap::new();
        for field_def in &schema.fields {
            let dyn_val = fields_dyn
                .get(field_def.name.as_str())
                .cloned()
                .unwrap_or(rhai::Dynamic::UNIT);
            let fv = crate::core::scripting::schema::dynamic_to_field_value(
                dyn_val, &field_def.field_type,
            ).map_err(|e| rhai::EvalAltResult::ErrorRuntime(
                format!("update_note field {:?}: {e}", field_def.name).into(),
                rhai::Position::NONE,
            ))?;
            fields.insert(field_def.name.clone(), fv);
        }

        let mut ctx_guard = action_ctx_update.lock().unwrap();
        let ctx = ctx_guard.as_mut().ok_or_else(|| {
            rhai::EvalAltResult::ErrorRuntime(
                "update_note() called outside a tree action".into(),
                rhai::Position::NONE,
            )
        })?;

        // Update the note_cache so get_children/get_note sees the new values.
        ctx.note_cache.insert(note_id.clone(), note_map.clone());

        // If the note is an in-flight create, update the create spec directly.
        if let Some(create) = ctx.creates.iter_mut().find(|c| c.id == note_id) {
            create.title  = title;
            create.fields = fields;
            return Ok(());
        }

        // Otherwise queue an update for a pre-existing DB note.
        // Replace any prior update for the same note.
        if let Some(existing) = ctx.updates.iter_mut().find(|u| u.note_id == note_id) {
            existing.title  = title;
            existing.fields = fields;
        } else {
            ctx.updates.push(hooks::ActionUpdate { note_id, title, fields });
        }

        Ok(())
    },
);
```

> **Note on visibility:** `dynamic_to_field_value` is currently `pub(super)` in `schema.rs`. If the compiler complains, either:
> - Change it to `pub(crate)`, or
> - Move the field conversion inline using the same match logic as `dynamic_to_field_value`.

**Step 4: Run tests, verify they pass**

```bash
cargo test -p krillnotes-core test_update_note -- --nocapture 2>&1 | tail -10
```

Expected: `PASSED`.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: register update_note host function for tree actions"
```

---

### Task 4: Update `get_children` and `get_note` to see the action cache

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn test_get_children_sees_inflight_creates() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Task", #{ fields: [] });
        add_tree_action("Verify Children", ["Task"], |note| {
            let t = create_note(note.id, "Task");
            let children = get_children(note.id);
            // The newly created note must appear in get_children
            let found = children.filter(|c| c.id == t.id);
            assert(found.len() == 1, "inflight note not visible in get_children");
        });
    "#, "test").unwrap();

    let note = make_test_note("parent1", "Task");
    // The action must complete without assertion error
    registry.invoke_tree_action_hook("Verify Children", &note, make_empty_ctx()).unwrap();
}

#[test]
fn test_get_note_sees_inflight_create() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Task", #{ fields: [] });
        add_tree_action("Verify get_note", ["Task"], |note| {
            let t = create_note(note.id, "Task");
            let fetched = get_note(t.id);
            assert(fetched != (), "inflight note not visible via get_note");
            assert(fetched.id == t.id, "wrong note returned");
        });
    "#, "test").unwrap();

    let note = make_test_note("parent1", "Task");
    registry.invoke_tree_action_hook("Verify get_note", &note, make_empty_ctx()).unwrap();
}
```

**Step 2: Run, verify they fail**

```bash
cargo test -p krillnotes-core "test_get_children_sees_inflight\|test_get_note_sees_inflight" -- --nocapture 2>&1 | tail -20
```

Expected: `FAILED` — assert fires because cache is not consulted yet.

**Step 3: Update `get_children` registration**

Find the existing `get_children` registration (around line 211 in `mod.rs`). It currently only uses `qc1`. Change it to also consult `action_ctx`:

```rust
let qc1           = Arc::clone(&query_context);
let action_ctx_gc = Arc::clone(&action_ctx);
engine.register_fn("get_children", move |id: String| -> rhai::Array {
    // Collect pre-existing children from the snapshot.
    let mut result: rhai::Array = {
        let guard = qc1.lock().unwrap();
        guard.as_ref()
            .and_then(|ctx| ctx.children_by_id.get(&id).cloned())
            .unwrap_or_default()
    };

    // Also include any in-flight creates with matching parent_id.
    if let Some(ctx) = action_ctx_gc.lock().unwrap().as_ref() {
        for (note_id, dyn_note) in &ctx.note_cache {
            // Extract parent_id from the Dynamic map.
            // In-flight notes don't have parent_id in the map, so check the
            // creates list directly.
            let _ = note_id; // suppress unused warning
            let _ = dyn_note;
        }
        // Use the creates list (it has parent_id) — build from there.
        for create in &ctx.creates {
            if create.parent_id == id {
                if let Some(dyn_note) = ctx.note_cache.get(&create.id) {
                    result.push(dyn_note.clone());
                }
            }
        }
    }

    result
});
```

**Step 4: Update `get_note` registration**

Find the existing `get_note` registration (around line 220). Update to also check the action cache:

```rust
let qc2           = Arc::clone(&query_context);
let action_ctx_gn = Arc::clone(&action_ctx);
engine.register_fn("get_note", move |id: String| -> rhai::Dynamic {
    // Check action cache first (in-flight notes).
    if let Some(ctx) = action_ctx_gn.lock().unwrap().as_ref() {
        if let Some(dyn_note) = ctx.note_cache.get(&id) {
            return dyn_note.clone();
        }
    }
    // Fall back to snapshot.
    let guard = qc2.lock().unwrap();
    guard.as_ref()
        .and_then(|ctx| ctx.notes_by_id.get(&id).cloned())
        .unwrap_or(rhai::Dynamic::UNIT)
});
```

**Step 5: Run tests, verify they pass**

```bash
cargo test -p krillnotes-core "test_get_children_sees_inflight\|test_get_note_sees_inflight" -- --nocapture 2>&1 | tail -10
```

Expected: `PASSED`.

**Step 6: Run all scripting tests to check for regressions**

```bash
cargo test -p krillnotes-core 2>&1 | tail -20
```

Expected: all existing tests still `PASSED`.

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: get_children and get_note consult action cache during tree actions"
```

---

### Task 5: Update `invoke_tree_action_hook` lifecycle and return type

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Change the return type**

Find `invoke_tree_action_hook` signature (around line 458). Change from:

```rust
pub fn invoke_tree_action_hook(
    &self,
    label: &str,
    note: &Note,
    context: QueryContext,
) -> Result<Option<Vec<String>>>
```

To:

```rust
pub fn invoke_tree_action_hook(
    &self,
    label: &str,
    note: &Note,
    context: QueryContext,
) -> Result<hooks::TreeActionResult>
```

**Step 2: Update the method body**

Replace the body with:

```rust
pub fn invoke_tree_action_hook(
    &self,
    label: &str,
    note: &Note,
    context: QueryContext,
) -> Result<hooks::TreeActionResult> {
    let entry = self.hook_registry.find_tree_action(label);
    let (fn_ptr, ast, script_name) = entry.ok_or_else(|| {
        KrillnotesError::Scripting(format!("unknown tree action: {label:?}"))
    })?;

    // Build note map (same shape as note_to_rhai_dynamic).
    let mut fields_map = rhai::Map::new();
    for (k, v) in &note.fields {
        fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
    }
    let mut note_map = rhai::Map::new();
    note_map.insert("id".into(),        rhai::Dynamic::from(note.id.clone()));
    note_map.insert("node_type".into(), rhai::Dynamic::from(note.node_type.clone()));
    note_map.insert("title".into(),     rhai::Dynamic::from(note.title.clone()));
    note_map.insert("fields".into(),    rhai::Dynamic::from(fields_map));

    // Activate the action context and query context.
    *self.action_ctx.lock().unwrap()    = Some(hooks::ActionTxContext::default());
    *self.query_context.lock().unwrap() = Some(context);

    let raw = fn_ptr
        .call::<rhai::Dynamic>(&self.engine, &ast, (rhai::Dynamic::from_map(note_map),))
        .map_err(|e| KrillnotesError::Scripting(
            format!("[{script_name}] tree action {label:?}: {e}")
        ));

    // Always clear both contexts, even on error.
    let action_result = self.action_ctx.lock().unwrap().take();
    *self.query_context.lock().unwrap() = None;

    let raw = raw?; // Propagate script error after clearing contexts.

    // Build the result from the action context.
    let (creates, updates) = match action_result {
        Some(ctx) => (ctx.creates, ctx.updates),
        None      => (Vec::new(), Vec::new()),
    };

    // If the closure returned an array of strings, that's the reorder path.
    let reorder = if let Some(arr) = raw.try_cast::<rhai::Array>() {
        let ids: Vec<String> = arr.into_iter()
            .filter_map(|v| v.try_cast::<String>())
            .collect();
        Some(ids)
    } else {
        None
    };

    Ok(hooks::TreeActionResult { reorder, creates, updates })
}
```

**Step 3: Fix the call site in `workspace.rs`**

`Workspace::run_tree_action` currently pattern-matches on `Option<Vec<String>>`. Change it to use the new `TreeActionResult`:

```rust
let result = self.script_registry.invoke_tree_action_hook(label, &note, context)?;

// Reorder path is unchanged.
if let Some(ids) = result.reorder {
    for (position, id) in ids.iter().enumerate() {
        self.move_note(id, Some(note_id), position as i32)?;
    }
}
```

(Pending creates/updates will be handled in Task 6.)

**Step 4: Verify it compiles and all tests pass**

```bash
cargo test -p krillnotes-core 2>&1 | tail -20
```

Expected: all tests pass, including the reorder tests.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs \
        krillnotes-core/src/core/workspace.rs
git commit -m "feat: update invoke_tree_action_hook to return TreeActionResult"
```

---

### Task 6: Apply pending ops in `Workspace::run_tree_action`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write integration tests**

Find the `#[cfg(test)]` module at the bottom of `workspace.rs`. If a test helper for creating a temp workspace exists, use it. Otherwise add one:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_workspace() -> Workspace {
        let file = NamedTempFile::new().expect("tmp file");
        let path = file.path().to_str().unwrap();
        Workspace::create(path, "test-ws", None).expect("create workspace")
    }

    // ... existing tests ...

    #[test]
    fn test_tree_action_create_note_writes_to_db() {
        let mut ws = temp_workspace();
        // Load a script that creates a child note via create_note().
        ws.save_script("create_action", r#"
            schema("Folder", #{ fields: [] });
            schema("Item",   #{ fields: [#{ name: "tag", type: "text", required: false }] });
            add_tree_action("Add Item", ["Folder"], |folder| {
                let item = create_note(folder.id, "Item");
                item.title = "My Item";
                item.fields.tag = "hello";
                update_note(item);
            });
        "#).expect("save script");
        ws.reload_scripts().expect("reload");

        // Create a Folder note to act on.
        let folder_id = ws.create_note(&ws.root_id.clone(), AddPosition::AsChild, "Folder")
            .expect("create folder");

        // Run the tree action.
        ws.run_tree_action(&folder_id, "Add Item").expect("run action");

        // Verify the new note exists in the DB.
        let children = ws.get_children(&folder_id).expect("get children");
        assert_eq!(children.len(), 1, "one child expected");
        assert_eq!(children[0].title, "My Item");
        assert_eq!(
            children[0].fields.get("tag"),
            Some(&FieldValue::Text("hello".into())),
        );
    }

    #[test]
    fn test_tree_action_update_note_writes_to_db() {
        let mut ws = temp_workspace();
        ws.save_script("update_action", r#"
            schema("Task", #{ fields: [#{ name: "status", type: "text", required: false }] });
            add_tree_action("Mark Done", ["Task"], |note| {
                note.fields.status = "Done";
                note.title = "Completed";
                update_note(note);
            });
        "#).expect("save script");
        ws.reload_scripts().expect("reload");

        let task_id = ws.create_note(&ws.root_id.clone(), AddPosition::AsChild, "Task")
            .expect("create task");

        ws.run_tree_action(&task_id, "Mark Done").expect("run action");

        let updated = ws.get_note(&task_id).expect("get note");
        assert_eq!(updated.title, "Completed");
        assert_eq!(
            updated.fields.get("status"),
            Some(&FieldValue::Text("Done".into())),
        );
    }

    #[test]
    fn test_tree_action_nested_create_builds_subtree() {
        let mut ws = temp_workspace();
        ws.save_script("subtree_action", r#"
            schema("Project", #{ fields: [] });
            schema("Sprint",  #{ fields: [] });
            schema("Task",    #{ fields: [] });
            add_tree_action("Create Template", ["Project"], |project| {
                let sprint = create_note(project.id, "Sprint");
                sprint.title = "Sprint 1";
                update_note(sprint);

                let task = create_note(sprint.id, "Task");
                task.title = "Define goals";
                update_note(task);
            });
        "#).expect("save script");
        ws.reload_scripts().expect("reload");

        let project_id = ws.create_note(&ws.root_id.clone(), AddPosition::AsChild, "Project")
            .expect("create project");

        ws.run_tree_action(&project_id, "Create Template").expect("run action");

        let sprints = ws.get_children(&project_id).expect("get project children");
        assert_eq!(sprints.len(), 1);
        assert_eq!(sprints[0].title, "Sprint 1");

        let tasks = ws.get_children(&sprints[0].id).expect("get sprint children");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Define goals");
    }

    #[test]
    fn test_tree_action_error_rolls_back_all_writes() {
        let mut ws = temp_workspace();
        ws.save_script("failing_action", r#"
            schema("Folder", #{ fields: [] });
            add_tree_action("Fail Mid", ["Folder"], |folder| {
                let item = create_note(folder.id, "Folder");
                item.title = "Should Not Exist";
                update_note(item);
                throw "intentional error";
            });
        "#).expect("save script");
        ws.reload_scripts().expect("reload");

        let folder_id = ws.create_note(&ws.root_id.clone(), AddPosition::AsChild, "Folder")
            .expect("create folder");

        let result = ws.run_tree_action(&folder_id, "Fail Mid");
        assert!(result.is_err(), "action should fail");

        let children = ws.get_children(&folder_id).expect("get children");
        assert_eq!(children.len(), 0, "rollback: no children should exist");
    }
}
```

**Step 2: Run tests, verify they fail**

```bash
cargo test -p krillnotes-core "test_tree_action_create\|test_tree_action_update\|test_tree_action_nested\|test_tree_action_error" -- --nocapture 2>&1 | tail -30
```

Expected: `FAILED` — creates/updates are queued but not yet applied.

**Step 3: Update `Workspace::run_tree_action` to apply pending ops**

Replace the current `run_tree_action` body (lines 742-768) with:

```rust
pub fn run_tree_action(&mut self, note_id: &str, label: &str) -> Result<()> {
    let note      = self.get_note(note_id)?;
    let all_notes = self.list_all_notes()?;

    let mut notes_by_id:    HashMap<String, Dynamic> = HashMap::new();
    let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
    let mut notes_by_type:  HashMap<String, Vec<Dynamic>> = HashMap::new();
    for n in &all_notes {
        let dyn_map = note_to_rhai_dynamic(n);
        notes_by_id.insert(n.id.clone(), dyn_map.clone());
        if let Some(pid) = &n.parent_id {
            children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
        }
        notes_by_type.entry(n.node_type.clone()).or_default().push(dyn_map);
    }
    let context = QueryContext { notes_by_id, children_by_id, notes_by_type };

    let result = self.script_registry.invoke_tree_action_hook(label, &note, context)?;

    // Apply pending creates and updates in a single transaction.
    if !result.creates.is_empty() || !result.updates.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let tx  = self.storage.connection_mut().transaction()?;

        for create in &result.creates {
            // Position: append after the last existing child of this parent.
            let position: i32 = tx.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM notes WHERE parent_id = ?1",
                rusqlite::params![create.parent_id],
                |row| row.get(0),
            )?;
            let fields_json = serde_json::to_string(&create.fields)?;
            tx.execute(
                "INSERT INTO notes \
                 (id, title, node_type, parent_id, position, \
                  created_at, modified_at, created_by, modified_by, \
                  fields_json, is_expanded) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    create.id,
                    create.title,
                    create.node_type,
                    create.parent_id,
                    position,
                    now, now,
                    self.current_user_id, self.current_user_id,
                    fields_json,
                    true,
                ],
            )?;
            // Log the creation.
            let op = crate::core::operations::Operation::CreateNote {
                note_id:    create.id.clone(),
                note_title: create.title.clone(),
                node_type:  create.node_type.clone(),
            };
            self.operation_log.log(&tx, &op)?;
        }

        for update in &result.updates {
            let fields_json = serde_json::to_string(&update.fields)?;
            tx.execute(
                "UPDATE notes \
                 SET title = ?1, fields_json = ?2, modified_at = ?3, modified_by = ?4 \
                 WHERE id = ?5",
                rusqlite::params![
                    update.title,
                    fields_json,
                    now,
                    self.current_user_id,
                    update.note_id,
                ],
            )?;
            // Log the update.
            let op = crate::core::operations::Operation::UpdateField {
                note_id:    update.note_id.clone(),
                note_title: update.title.clone(),
                field_key:  "_tree_action".into(),
                old_value:  String::new(),
                new_value:  String::new(),
            };
            self.operation_log.log(&tx, &op)?;
        }

        tx.commit()?;
    }

    // Reorder path (unchanged from before).
    if let Some(ids) = result.reorder {
        for (position, id) in ids.iter().enumerate() {
            self.move_note(id, Some(note_id), position as i32)?;
        }
    }

    Ok(())
}
```

> **Note on Operation types:** Check the exact variants and fields of `Operation` in `operations.rs`. The `CreateNote` and `UpdateField` variants above may need adjusting to match the actual enum definition. The `_tree_action` sentinel for `field_key` avoids over-engineering the log for now; it can be replaced with per-field entries later.

**Step 4: Run tests, verify they pass**

```bash
cargo test -p krillnotes-core "test_tree_action" -- --nocapture 2>&1 | tail -20
```

Expected: all four tests `PASSED`.

**Step 5: Run the full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -20
```

Expected: all tests pass.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: apply tree action creates/updates atomically in run_tree_action"
```

---

### Task 7: Documentation and example script

**Files:**
- Modify: `SCRIPTING.md`
- Modify: `krillnotes-core/src/system_scripts/00_text_note.rhai`

**Step 1: Add `create_note` and `update_note` docs to `SCRIPTING.md`**

Find the `add_tree_action` section (around line 410). After the existing description and example, add:

````markdown
#### Mutating notes from a tree action

Tree action closures have access to two additional functions for writing to the
workspace:

**`create_note(parent_id, node_type)`** — creates a new note of the given type
under the specified parent and returns a note map with schema defaults. The note
is not in the database until the action completes; all writes are applied
atomically. If the closure throws an error, nothing is written.

**`update_note(note)`** — persists title and field changes on a note map back to
the database. Works on any note — both the action target and notes returned by
`get_children()` or `create_note()`.

Because all writes share a pending transaction, **`get_children()` and
`get_note()` will see notes created earlier in the same closure**, allowing
scripts to build subtrees:

```rhai
add_tree_action("Create Sprint Template", ["Project"], |project| {
    let sprint = create_note(project.id, "Sprint");
    sprint.title = "Sprint 1";
    sprint.fields.status = "Planning";
    update_note(sprint);

    // sprint.id is already known — get_children will find it too
    let task = create_note(sprint.id, "Task");
    task.title = "Define goals";
    update_note(task);

    // Update the project itself
    project.fields.status = "Active";
    update_note(project);
});
```

> `create_note` and `update_note` are **only available inside `add_tree_action`
> closures**. They are not available in `on_save`, `on_add_child`, or `on_view`.
````

**Step 2: Update the TextNote system script example**

Open `krillnotes-core/src/system_scripts/00_text_note.rhai` and add a second example action that demonstrates `create_note`:

```rhai
// Example tree action: sort a TextNote's children alphabetically by title.
add_tree_action("Sort Children A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

// Example tree action: add a new child TextNote with a placeholder title.
add_tree_action("Add Child Note", ["TextNote"], |note| {
    let child = create_note(note.id, "TextNote");
    child.title = "New note";
    update_note(child);
});
```

**Step 3: Build to make sure the script compiles (it is embedded at compile time)**

```bash
cargo build -p krillnotes-core 2>&1 | grep -E "error|warning" | head -20
```

Expected: no errors.

**Step 4: Run all tests one final time**

```bash
cargo test -p krillnotes-core 2>&1 | tail -10
```

Expected: all pass.

**Step 5: Commit**

```bash
git add SCRIPTING.md \
        krillnotes-core/src/system_scripts/00_text_note.rhai
git commit -m "docs: document create_note/update_note in SCRIPTING.md; add example to TextNote"
```

---

## Summary

| Task | What it does |
|------|-------------|
| 1 | `ActionTxContext`, `ActionCreate`, `ActionUpdate`, `TreeActionResult` types |
| 2 | `create_note` host function — queues creates, populates cache |
| 3 | `update_note` host function — updates cache and queues updates (or patches in-flight creates) |
| 4 | `get_children` / `get_note` consult action cache during a live action |
| 5 | `invoke_tree_action_hook` manages context lifecycle, returns `TreeActionResult` |
| 6 | `run_tree_action` applies all creates/updates atomically; rolls back on error |
| 7 | Docs + example script |
