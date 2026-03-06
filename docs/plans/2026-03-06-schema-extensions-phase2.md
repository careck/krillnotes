# Schema Extensions Phase 2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split scripts into two file categories (`.schema.rhai` / `.rhai`), replace `on_view`/`on_hover`/`add_tree_action` with deferred `register_view()`/`register_hover()`/`register_menu()` bindings, add tabbed view mode to InfoPanel, and update the Script Manager with category badges + creation flow.

**Architecture:** Two-phase script loading (presentation/library first, then schema) with a deferred binding queue that resolves after all scripts load. Frontend gets a new tabbed view mode where each `register_view()` call becomes a tab, with a built-in "Fields" tab always rightmost. Script Manager gains category selection, starter templates, and warning badges.

**Tech Stack:** Rust (rhai, rusqlite, serde), Tauri v2, React 19, TypeScript, Tailwind v4

**Design doc:** `docs/plans/2026-03-06-schema-extensions-phase2-design.md`

---

## Task 1: Add `category` to UserScript and DB migration

**Files:**
- Modify: `krillnotes-core/src/core/user_script.rs:14-23`
- Modify: `krillnotes-core/src/core/storage.rs:101` (inside `run_migrations`)
- Modify: `krillnotes-core/src/core/workspace.rs` (all SQL touching `user_scripts`)
- Test: `krillnotes-core/src/core/user_script.rs` (existing tests)

**Step 1: Write the failing test**

In `krillnotes-core/src/core/user_script.rs`, add a test:

```rust
#[test]
fn test_user_script_has_category_field() {
    let script = UserScript {
        id: "test".to_string(),
        name: "Test".to_string(),
        description: "".to_string(),
        source_code: "".to_string(),
        load_order: 0,
        enabled: true,
        created_at: 0,
        modified_at: 0,
        category: "schema".to_string(),
    };
    assert_eq!(script.category, "schema");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_user_script_has_category_field`
Expected: FAIL -- `category` field doesn't exist on `UserScript`

**Step 3: Add `category` field to `UserScript`**

In `krillnotes-core/src/core/user_script.rs:14-23`, add the field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_code: String,
    pub load_order: i32,
    pub enabled: bool,
    pub created_at: i64,
    pub modified_at: i64,
    pub category: String,  // "schema" or "presentation"
}
```

**Step 4: Add DB migration**

In `krillnotes-core/src/core/storage.rs`, at the end of `run_migrations()` (after the last migration block), add:

```rust
// Migration: add category column to user_scripts if absent.
let category_exists: bool = conn.query_row(
    "SELECT COUNT(*) FROM pragma_table_info('user_scripts') WHERE name='category'",
    [],
    |row| row.get::<_, i64>(0).map(|c| c > 0),
)?;
if !category_exists {
    conn.execute(
        "ALTER TABLE user_scripts ADD COLUMN category TEXT NOT NULL DEFAULT 'presentation'",
        [],
    )?;
}
```

**Step 5: Update all SQL queries that read/write user_scripts**

Search `workspace.rs` for all `user_scripts` SQL queries and add `category` to:
- `INSERT INTO user_scripts` -- include `category` column
- `SELECT ... FROM user_scripts` -- include `category` in the column list
- Row mapping (`row.get(...)`) -- extract `category` at the correct index

Also update `create_user_script()` to accept a `category` parameter and pass it through.
Default to `"presentation"` for backward compat.

Update the `schema.sql` DDL to include `category` in the `CREATE TABLE user_scripts` statement.

**Step 6: Fix all compilation errors**

All places constructing `UserScript` now need `category`. Fix each one.

**Step 7: Run tests to verify**

Run: `cargo test -p krillnotes-core`
Expected: All existing tests pass, new test passes.

**Step 8: Commit**

```
git add -A && git commit -m "feat: add category field to UserScript + DB migration"
```

---

## Task 2: Deferred binding types and storage on SchemaRegistry

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:17-23` (HookEntry), `391-430` (SchemaRegistry)
- Test: inline `#[cfg(test)]` in `schema.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_schema_registry_has_view_registrations() {
    let reg = SchemaRegistry::new();
    assert!(reg.get_views_for_type("TextNote").is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_schema_registry_has_view_registrations`
Expected: FAIL -- method `get_views_for_type` doesn't exist

**Step 3: Add new types and storage**

In `schema.rs`, add the new structs after `HookEntry`:

```rust
/// A registered custom view tab for a note type.
#[derive(Debug, Clone)]
pub struct ViewRegistration {
    pub label: String,
    pub display_first: bool,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}

/// A registered context menu action for note types.
#[derive(Debug, Clone)]
pub struct MenuRegistration {
    pub label: String,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}

/// A deferred binding queued during script loading, resolved after all scripts load.
#[derive(Debug, Clone)]
pub struct DeferredBinding {
    pub kind: BindingKind,
    pub target_type: String,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
    pub display_first: bool,
    pub label: Option<String>,
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum BindingKind {
    View,
    Hover,
    Menu,
}

/// A warning about an unresolved deferred binding.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptWarning {
    pub script_name: String,
    pub message: String,
}
```

Update `SchemaRegistry` to replace `on_view_hooks` and `on_hover_hooks` with the new storage:

```rust
pub(super) struct SchemaRegistry {
    schemas:              Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks:        Arc<Mutex<HashMap<String, HookEntry>>>,
    on_add_child_hooks:   Arc<Mutex<HashMap<String, HookEntry>>>,
    // New: replaces on_view_hooks and on_hover_hooks
    view_registrations:   Arc<Mutex<HashMap<String, Vec<ViewRegistration>>>>,
    hover_registrations:  Arc<Mutex<HashMap<String, HookEntry>>>,
    menu_registrations:   Arc<Mutex<HashMap<String, Vec<MenuRegistration>>>>,
    deferred_bindings:    Arc<Mutex<Vec<DeferredBinding>>>,
    warnings:             Arc<Mutex<Vec<ScriptWarning>>>,
}
```

Update `new()`, `clear()`, and add accessor methods:

```rust
pub(super) fn view_registrations_arc(&self) -> Arc<Mutex<HashMap<String, Vec<ViewRegistration>>>> {
    Arc::clone(&self.view_registrations)
}
pub(super) fn hover_registrations_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
    Arc::clone(&self.hover_registrations)
}
pub(super) fn menu_registrations_arc(&self) -> Arc<Mutex<HashMap<String, Vec<MenuRegistration>>>> {
    Arc::clone(&self.menu_registrations)
}
pub(super) fn deferred_bindings_arc(&self) -> Arc<Mutex<Vec<DeferredBinding>>> {
    Arc::clone(&self.deferred_bindings)
}

pub fn get_views_for_type(&self, schema_name: &str) -> Vec<ViewRegistration> {
    self.view_registrations.lock().unwrap()
        .get(schema_name).cloned().unwrap_or_default()
}

pub fn has_hover_registration(&self, schema_name: &str) -> bool {
    self.hover_registrations.lock().unwrap().contains_key(schema_name)
}

pub fn get_warnings(&self) -> Vec<ScriptWarning> {
    self.warnings.lock().unwrap().clone()
}

/// Returns a map of note_type -> [menu_label, ...] for all registered menu actions.
pub fn menu_action_map(&self) -> HashMap<String, Vec<String>> {
    let regs = self.menu_registrations.lock().unwrap();
    regs.iter()
        .map(|(k, v)| (k.clone(), v.iter().map(|r| r.label.clone()).collect()))
        .collect()
}
```

Remove the old `on_view_hooks`, `on_hover_hooks` fields and all their methods:
`on_view_hooks_arc()`, `on_hover_hooks_arc()`, `has_view_hook()`, `has_hover_hook()`,
`run_on_view_hook()`, `run_on_hover_hook()`.

**Step 4: Add `resolve_bindings()` method**

```rust
pub fn resolve_bindings(&self) {
    let mut bindings = self.deferred_bindings.lock().unwrap();
    let schemas = self.schemas.lock().unwrap();
    let mut views = self.view_registrations.lock().unwrap();
    let mut hovers = self.hover_registrations.lock().unwrap();
    let mut menus = self.menu_registrations.lock().unwrap();
    let mut warnings = self.warnings.lock().unwrap();

    for binding in bindings.drain(..) {
        match binding.kind {
            BindingKind::View => {
                if schemas.contains_key(&binding.target_type) {
                    let entry = ViewRegistration {
                        label: binding.label.unwrap_or_else(|| binding.target_type.clone()),
                        display_first: binding.display_first,
                        fn_ptr: binding.fn_ptr,
                        ast: binding.ast,
                        script_name: binding.script_name,
                    };
                    views.entry(binding.target_type).or_default().push(entry);
                } else {
                    warnings.push(ScriptWarning {
                        script_name: binding.script_name,
                        message: format!(
                            "register_view('{}', '{}') -- type not found",
                            binding.target_type,
                            binding.label.unwrap_or_default()
                        ),
                    });
                }
            }
            BindingKind::Hover => {
                if schemas.contains_key(&binding.target_type) {
                    let entry = HookEntry {
                        fn_ptr: binding.fn_ptr,
                        ast: binding.ast.as_ref().clone(),
                        script_name: binding.script_name,
                    };
                    hovers.insert(binding.target_type, entry);
                } else {
                    warnings.push(ScriptWarning {
                        script_name: binding.script_name,
                        message: format!(
                            "register_hover('{}') -- type not found",
                            binding.target_type
                        ),
                    });
                }
            }
            BindingKind::Menu => {
                for target_type in &binding.applies_to {
                    if schemas.contains_key(target_type) {
                        let entry = MenuRegistration {
                            label: binding.label.clone().unwrap_or_default(),
                            fn_ptr: binding.fn_ptr.clone(),
                            ast: Arc::clone(&binding.ast),
                            script_name: binding.script_name.clone(),
                        };
                        menus.entry(target_type.clone()).or_default().push(entry);
                    } else {
                        warnings.push(ScriptWarning {
                            script_name: binding.script_name.clone(),
                            message: format!(
                                "register_menu('{}', ['{}']) -- type not found",
                                binding.label.as_deref().unwrap_or(""),
                                target_type
                            ),
                        });
                    }
                }
            }
        }
    }
}
```

**Step 5: Run tests**

Run: `cargo test -p krillnotes-core`
Expected: PASS (many compile errors to fix first -- update all references to removed methods)

**Step 6: Commit**

```
git add -A && git commit -m "feat: add deferred binding types and storage to SchemaRegistry"
```

---

## Task 3: Register `register_view/hover/menu` Rhai functions + remove old hooks

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs:166-270` (engine registration)
- Modify: `krillnotes-core/src/core/scripting/hooks.rs` (remove HookRegistry tree actions)
- Test: inline tests in `mod.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_register_view_queues_deferred_binding() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        register_view("TextNote", "Summary", |note| {
            text("hello")
        });
    "#, "test_views.rhai").unwrap();

    // Binding is deferred, not resolved yet
    let views = registry.schema_registry.get_views_for_type("TextNote");
    assert!(views.is_empty());

    // Now load a schema and resolve
    registry.load_script(r#"
        schema("TextNote", #{ fields: [] });
    "#, "text_note.schema.rhai").unwrap();

    registry.resolve_bindings();
    let views = registry.schema_registry.get_views_for_type("TextNote");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].label, "Summary");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_register_view_queues_deferred_binding`
Expected: FAIL -- `register_view` is not a registered Rhai function

**Step 3: Remove `add_tree_action` registration from mod.rs**

In `mod.rs`, remove the entire `add_tree_action` engine registration block (lines 166-195).

**Step 4: Remove `on_view` and `on_hover` extraction from `schema()` closure**

In `mod.rs` inside the `schema()` registration closure (around lines 242-266), remove the blocks that extract `on_view` and `on_hover` from the definition map. Also remove the `on_view_arc` and `on_hover_arc` variables captured by the closure.

**Step 5: Register `register_view()` Rhai function**

In the constructor, after the `schema()` registration, add:

```rust
// register_view(target_type, label, closure)
// register_view(target_type, label, options_map, closure)
let deferred_arc = schema_registry.deferred_bindings_arc();
let view_ast_arc = Arc::clone(&current_loading_ast);
let view_name_arc = Arc::clone(&current_loading_script_name);

// 3-arg form: register_view(type, label, closure)
let d1 = Arc::clone(&deferred_arc);
let a1 = Arc::clone(&view_ast_arc);
let n1 = Arc::clone(&view_name_arc);
engine.register_fn("register_view",
    move |target_type: String, label: String, fn_ptr: FnPtr|
    -> std::result::Result<Dynamic, Box<EvalAltResult>>
    {
        let ast = a1.lock().unwrap().clone()
            .ok_or_else(|| "register_view() called outside of load_script".to_string())?;
        let script_name = n1.lock().unwrap().clone().unwrap_or_default();
        d1.lock().unwrap().push(DeferredBinding {
            kind: BindingKind::View,
            target_type,
            fn_ptr,
            ast: Arc::new(ast),
            script_name,
            display_first: false,
            label: Some(label),
            applies_to: vec![],
        });
        Ok(Dynamic::UNIT)
    }
);

// 4-arg form: register_view(type, label, options, closure)
let d2 = Arc::clone(&deferred_arc);
let a2 = Arc::clone(&view_ast_arc);
let n2 = Arc::clone(&view_name_arc);
engine.register_fn("register_view",
    move |target_type: String, label: String, options: rhai::Map, fn_ptr: FnPtr|
    -> std::result::Result<Dynamic, Box<EvalAltResult>>
    {
        let ast = a2.lock().unwrap().clone()
            .ok_or_else(|| "register_view() called outside of load_script".to_string())?;
        let script_name = n2.lock().unwrap().clone().unwrap_or_default();
        let display_first = options.get("display_first")
            .and_then(|v| v.as_bool().ok())
            .unwrap_or(false);
        d2.lock().unwrap().push(DeferredBinding {
            kind: BindingKind::View,
            target_type,
            fn_ptr,
            ast: Arc::new(ast),
            script_name,
            display_first,
            label: Some(label),
            applies_to: vec![],
        });
        Ok(Dynamic::UNIT)
    }
);
```

**Step 6: Register `register_hover()` Rhai function**

```rust
// register_hover(target_type, closure)
let d3 = Arc::clone(&deferred_arc);
let a3 = Arc::clone(&view_ast_arc);
let n3 = Arc::clone(&view_name_arc);
engine.register_fn("register_hover",
    move |target_type: String, fn_ptr: FnPtr|
    -> std::result::Result<Dynamic, Box<EvalAltResult>>
    {
        let ast = a3.lock().unwrap().clone()
            .ok_or_else(|| "register_hover() called outside of load_script".to_string())?;
        let script_name = n3.lock().unwrap().clone().unwrap_or_default();
        d3.lock().unwrap().push(DeferredBinding {
            kind: BindingKind::Hover,
            target_type,
            fn_ptr,
            ast: Arc::new(ast),
            script_name,
            display_first: false,
            label: None,
            applies_to: vec![],
        });
        Ok(Dynamic::UNIT)
    }
);
```

**Step 7: Register `register_menu()` Rhai function**

```rust
// register_menu(label, target_types, closure)
let d4 = Arc::clone(&deferred_arc);
let a4 = Arc::clone(&view_ast_arc);
let n4 = Arc::clone(&view_name_arc);
engine.register_fn("register_menu",
    move |label: String, types: rhai::Array, fn_ptr: FnPtr|
    -> std::result::Result<Dynamic, Box<EvalAltResult>>
    {
        let ast = a4.lock().unwrap().clone()
            .ok_or_else(|| "register_menu() called outside of load_script".to_string())?;
        let script_name = n4.lock().unwrap().clone().unwrap_or_default();
        let applies_to: Vec<String> = types.into_iter()
            .filter_map(|v| v.into_string().ok())
            .collect();
        d4.lock().unwrap().push(DeferredBinding {
            kind: BindingKind::Menu,
            target_type: String::new(),
            fn_ptr,
            ast: Arc::new(ast),
            script_name,
            display_first: false,
            label: Some(label),
            applies_to,
        });
        Ok(Dynamic::UNIT)
    }
);
```

**Step 8: Remove HookRegistry tree action storage**

In `hooks.rs`, remove the tree action fields and methods from `HookRegistry`:
`TreeActionEntry`, `register_tree_action()`, `find_tree_action()`, `tree_action_map()`.
Keep the `HookRegistry` struct if it has other uses, or remove it entirely if tree actions
were its only purpose.

Remove `hook_registry` from `ScriptRegistry` if now empty.

**Step 9: Update `run_on_view_hook` and `run_on_hover_hook` on ScriptRegistry**

Replace `run_on_view_hook()` to use `view_registrations`. Add a new
`run_view()` method that takes a view label and renders the specific view.

Similarly update `run_on_hover_hook()` to use `hover_registrations`.

**Step 10: Add `resolve_bindings()` method to ScriptRegistry**

```rust
pub fn resolve_bindings(&self) {
    self.schema_registry.resolve_bindings();
}
```

**Step 11: Update `invoke_tree_action_hook` to use menu_registrations**

Replace the hook_registry lookup with a menu_registrations lookup.
The `find_menu_action()` method looks up in `menu_registrations` by label.

**Step 12: Run all tests and fix compilation errors**

Run: `cargo test -p krillnotes-core`
Expected: Many existing tests will break because they use `on_view`/`on_hover` in `schema()` calls and `add_tree_action()`. These tests need to be migrated to use `register_view()`/`register_hover()`/`register_menu()`. Fix each one.

**Step 13: Commit**

```
git add -A && git commit -m "feat: register_view/hover/menu Rhai functions + remove old hooks"
```

---

## Task 4: Two-phase script loading in Workspace

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (`Workspace::create` lines 162-186, `Workspace::open` lines 376-382)
- Test: workspace integration tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_two_phase_loading_presentation_before_schema() {
    // Create workspace with a presentation script (lower load_order)
    // and a schema script (higher load_order).
    // The presentation script calls register_view() for a type
    // defined in the schema script. Verify the view resolves.
    let mut ws = Workspace::create_in_memory("").unwrap();

    // Insert a presentation script first (load_order 0)
    ws.create_user_script_with_category(
        r#"// @name: Views
register_view("TestType", "Summary", |note| { text("hello") });
"#,
        "presentation",
    ).unwrap();

    // Insert a schema script second (load_order 1)
    ws.create_user_script_with_category(
        r#"// @name: TestType
schema("TestType", #{ fields: [] });
"#,
        "schema",
    ).unwrap();

    // Reload scripts -- presentation loads first, schema second, then resolve
    ws.reload_all_scripts().unwrap();

    let views = ws.get_views_for_type("TestType");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].label, "Summary");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_two_phase_loading_presentation_before_schema`
Expected: FAIL -- methods don't exist yet

**Step 3: Implement two-phase loading**

In `workspace.rs`, modify the script loading logic (in `create()`, `open()`, and
`reload_all_scripts()`). The pattern:

```rust
fn load_scripts_two_phase(&mut self, scripts: &[UserScript]) -> Vec<ScriptError> {
    let mut errors = vec![];

    // Phase A: load presentation scripts first
    let presentation: Vec<_> = scripts.iter()
        .filter(|s| s.enabled && s.category == "presentation")
        .collect();
    for script in &presentation {
        if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
            errors.push(ScriptError { name: script.name.clone(), message: e.to_string() });
        }
    }

    // Phase B: load schema scripts
    let schemas: Vec<_> = scripts.iter()
        .filter(|s| s.enabled && s.category == "schema")
        .collect();
    for script in &schemas {
        if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
            errors.push(ScriptError { name: script.name.clone(), message: e.to_string() });
        }
    }

    // Phase C: resolve deferred bindings
    self.script_registry.resolve_bindings();

    errors
}
```

For system (starter) scripts embedded via `include_dir!`, apply the same sorting:
load `.rhai` files first (by prefix), then `.schema.rhai` files (by prefix).

**Step 4: Add `create_user_script_with_category()` method**

```rust
pub fn create_user_script_with_category(
    &mut self,
    source_code: &str,
    category: &str,
) -> Result<(UserScript, Vec<ScriptError>)> {
    // Same as create_user_script but passes category to INSERT
}
```

**Step 5: Add `get_views_for_type()` and `get_script_warnings()` to Workspace**

```rust
pub fn get_views_for_type(&self, schema_name: &str) -> Vec<ViewRegistration> {
    self.script_registry.get_views_for_type(schema_name)
}

pub fn get_script_warnings(&self) -> Vec<ScriptWarning> {
    self.script_registry.get_script_warnings()
}
```

**Step 6: Update `tree_action_map()` to use menu_registrations**

```rust
pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
    self.script_registry.menu_action_map()
}
```

**Step 7: Enforce `schema()` only in schema-category scripts**

In `mod.rs`, the `schema()` closure needs to know the current script's category.
Add a `current_loading_category: Arc<Mutex<Option<String>>>` field to `ScriptRegistry`.
Set it before each `load_script()` call. Inside the `schema()` closure, check it:

```rust
let cat = schema_cat_arc.lock().unwrap();
if cat.as_deref() == Some("presentation") {
    return Err("schema() can only be called from schema-category scripts".into());
}
```

For system scripts, determine category from the file extension:
- Ends with `.schema.rhai` -> `"schema"`
- Otherwise -> `"presentation"`

**Step 8: Run tests**

Run: `cargo test -p krillnotes-core`
Expected: PASS

**Step 9: Commit**

```
git add -A && git commit -m "feat: two-phase script loading with presentation-first order"
```

---

## Task 5: Migrate all system scripts and templates

**Files:**
- Rename+modify: all 6 files in `krillnotes-core/src/system_scripts/`
- Rename+modify: all 3 files in `templates/`
- New: presentation files where needed

**Step 1: Migrate `00_text_note.rhai`**

Rename to `00_text_note.schema.rhai`. Move `add_tree_action` calls to `00_text_note.rhai`:

`00_text_note.schema.rhai`:
```rhai
// @name: Text Note
// @description: A simple text note with a body field.

schema("TextNote", #{
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});
```

`00_text_note.rhai`:
```rhai
// @name: Text Note Actions
// @description: Tree actions for TextNote.

register_menu("Sort Children A-Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

register_menu("Add Child Note", ["TextNote"], |note| {
    let child = create_child(note.id, "TextNote");
    set_title(child.id, "New note");
    commit();
});
```

**Note:** The "Add Child Note" action currently uses the old `create_note`/`update_note` API.
It needs to be migrated to `create_child()`/`set_title()`/`commit()`.

**Step 2: Migrate `01_contact.rhai`**

`01_contact.schema.rhai`: Both `ContactsFolder` and `Contact` schema definitions (without `on_view`).

`01_contact.rhai`: `register_view("ContactsFolder", "Contacts", #{ display_first: true }, |note| { ... })` with the table rendering.

**Step 3: Migrate `02_task.rhai`**

Rename to `02_task.schema.rhai`. No presentation file needed (no on_view/on_hover).

**Step 4: Migrate `03_project.rhai`**

Rename to `03_project.schema.rhai`. No presentation file needed.

**Step 5: Migrate `05_recipe.rhai`**

Rename to `05_recipe.schema.rhai`. No presentation file needed.

**Step 6: Migrate `06_product.rhai`**

Rename to `06_product.schema.rhai`. No presentation file needed.

**Step 7: Migrate `templates/book_collection.rhai`**

Split into `book_collection.schema.rhai` + `book_collection.rhai`.

`book_collection.schema.rhai`: `Book` and `Library` schema definitions (without `on_view`).

`book_collection.rhai`:
```rhai
register_view("Library", "Collection", #{ display_first: true }, |note| {
    // ... existing on_view code ...
});

register_menu("Sort by Title (A-Z)", ["Library"], |note| { ... });
register_menu("Sort by Author (A-Z)", ["Library"], |note| { ... });
register_menu("Sort by Genre (A-Z)", ["Library"], |note| { ... });
register_menu("Sort by Rating (High-Low)", ["Library"], |note| { ... });
register_menu("Sort by Date Read (Newest First)", ["Library"], |note| { ... });
```

**Step 8: Migrate `templates/zettelkasten.rhai`**

Split into `zettelkasten.schema.rhai` + `zettelkasten.rhai`.

`zettelkasten.schema.rhai`: `Zettel` and `Kasten` schema definitions (without `on_view`/`on_hover`).

`zettelkasten.rhai`:
```rhai
fn tag_list(tags) { ... }  // library functions stay here
fn strip_markdown(s) { ... }

register_view("Zettel", "Content", #{ display_first: true }, |note| { ... });
register_view("Kasten", "Overview", #{ display_first: true }, |note| { ... });
register_hover("Kasten", |note| { ... });
register_menu("Sort by Date (Newest First)", ["Kasten"], |note| { ... });
register_menu("Sort by Date (Oldest First)", ["Kasten"], |note| { ... });
```

**Step 9: Migrate `templates/photo_note.rhai`**

Split into `photo_note.schema.rhai` + `photo_note.rhai`.

`photo_note.schema.rhai`: `PhotoNote` schema (without `on_view`/`on_hover`).

`photo_note.rhai`:
```rhai
register_view("PhotoNote", "Photo", #{ display_first: true }, |note| {
    // existing on_view code
});

register_hover("PhotoNote", |note| {
    display_image(note.fields["photo"], 200, note.title)
});
```

**Step 10: Run tests**

Run: `cargo test -p krillnotes-core`
Expected: PASS -- all scripts load in two-phase order, bindings resolve.

**Step 11: Commit**

```
git add -A && git commit -m "feat: migrate all system scripts and templates to split format"
```

---

## Task 6: New Tauri commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add `get_views_for_type` command**

```rust
#[tauri::command]
fn get_views_for_type(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
) -> std::result::Result<Vec<ViewInfo>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    let views = workspace.get_views_for_type(&schema_name);
    Ok(views.iter().map(|v| ViewInfo {
        label: v.label.clone(),
        display_first: v.display_first,
    }).collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewInfo {
    label: String,
    display_first: bool,
}
```

**Step 2: Add `render_view` command**

```rust
#[tauri::command]
fn render_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    view_label: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.render_view(&note_id, &view_label).map_err(|e| e.to_string())
}
```

**Step 3: Add `get_script_warnings` command**

```rust
#[tauri::command]
fn get_script_warnings(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<ScriptWarning>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.get_script_warnings())
}
```

**Step 4: Update `get_all_schemas` to use new registration checks**

Replace `has_view_hook`/`has_hover_hook` with `has_views`/`has_hover` that check
the new registrations. Update `SchemaInfo` struct and `schema_to_info()` accordingly.

**Step 5: Update `get_note_view` to render the first/default view**

The existing `get_note_view` command should render the default view (first `display_first`
view, or first registered view). Keeps backward compatibility with frontend until the
tabbed UI is wired. Falls back to default view if no registered views.

**Step 6: Update `create_user_script` to accept category**

```rust
#[tauri::command]
fn create_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    source_code: String,
    category: String,
) -> std::result::Result<ScriptMutationResult<UserScript>, String> {
    // ... pass category through
}
```

**Step 7: Register all new commands in `generate_handler!`**

Add `get_views_for_type`, `render_view`, `get_script_warnings` to the handler macro.

**Step 8: Compile and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: PASS

**Step 9: Commit**

```
git add -A && git commit -m "feat: Tauri commands for views, warnings, and category"
```

---

## Task 7: TypeScript types and InfoPanel tabbed view mode

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Update TypeScript types**

In `types.ts`, add:

```typescript
export interface ViewInfo {
    label: string;
    displayFirst: boolean;
}

export interface ScriptWarning {
    scriptName: string;
    message: string;
}
```

Update `UserScript`:
```typescript
export interface UserScript {
    // ... existing fields ...
    category: string;  // "schema" | "presentation"
}
```

Update `SchemaInfo` -- replace `hasViewHook`/`hasHoverHook` with `hasViews`/`hasHover`.

**Step 2: Add tab state to InfoPanel**

In `InfoPanel.tsx`, add state:

```typescript
const [views, setViews] = useState<ViewInfo[]>([]);
const [activeTab, setActiveTab] = useState<string>("fields");
const [viewHtml, setViewHtml] = useState<Record<string, string>>({});
const [previousTab, setPreviousTab] = useState<string | null>(null);
```

**Step 3: Fetch views when note is selected**

```typescript
useEffect(() => {
    if (selectedNote && schema) {
        invoke<ViewInfo[]>('get_views_for_type', { schemaName: selectedNote.nodeType })
            .then(v => {
                setViews(v);
                // Set default tab: first display_first, or first view, or "fields"
                const sorted = [...v].sort((a, b) =>
                    (b.displayFirst ? 1 : 0) - (a.displayFirst ? 1 : 0)
                );
                setActiveTab(sorted.length > 0 ? sorted[0].label : "fields");
            });
    }
}, [selectedNote?.id, schema]);
```

**Step 4: Render tab bar**

Only render if views exist (no tab bar for types without custom views):

```tsx
{views.length > 0 && (
    <div className="flex border-b border-border">
        {[...views]
            .sort((a, b) => (b.displayFirst ? 1 : 0) - (a.displayFirst ? 1 : 0))
            .map(v => (
                <button
                    key={v.label}
                    className={`px-3 py-1.5 text-sm ${activeTab === v.label
                        ? 'border-b-2 border-primary font-medium'
                        : 'text-muted-foreground'}`}
                    onClick={() => setActiveTab(v.label)}
                >
                    {v.label}
                </button>
            ))
        }
        <button
            className={`px-3 py-1.5 text-sm ${activeTab === 'fields'
                ? 'border-b-2 border-primary font-medium'
                : 'text-muted-foreground'}`}
            onClick={() => setActiveTab('fields')}
        >
            {t('info_panel.fields', 'Fields')}
        </button>
    </div>
)}
```

**Step 5: Fetch and render view HTML per tab**

When a custom view tab is active, fetch its HTML:

```typescript
useEffect(() => {
    if (activeTab !== 'fields' && selectedNote && !isEditing) {
        invoke<string>('render_view', {
            noteId: selectedNote.id,
            viewLabel: activeTab,
        }).then(html => {
            setViewHtml(prev => ({ ...prev, [activeTab]: html }));
        });
    }
}, [activeTab, selectedNote?.id, isEditing]);
```

Render the active tab's content:
- If `activeTab === 'fields'` -> render the existing fields panel
- Otherwise -> render the view HTML in a sanitized container

**Note on HTML rendering:** The existing `get_note_view` already uses an HTML container
for Rhai-generated views. The same sanitization pattern applies here -- the HTML comes
from user-controlled Rhai scripts (same trust model as today's on_view hooks).

**Step 6: Edit mode switches to Fields tab**

When entering edit mode, save the current tab and switch to Fields:

```typescript
const handleEdit = () => {
    setPreviousTab(activeTab);
    setActiveTab('fields');
    setIsEditing(true);
};

const handleSaveOrCancel = () => {
    setIsEditing(false);
    if (previousTab) {
        setActiveTab(previousTab);
        setPreviousTab(null);
    }
};
```

**Step 7: Run TypeScript check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

**Step 8: Commit**

```
git add -A && git commit -m "feat: tabbed view mode in InfoPanel"
```

---

## Task 8: Script Manager category UI

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`
- Modify: `krillnotes-desktop/src/i18n/locales/en.json` (and other locales)

**Step 1: Add category badges to script list**

In the script list rendering, add a colored badge next to each script name:

```tsx
<span className={`text-xs px-1.5 py-0.5 rounded ${
    script.category === 'schema'
        ? 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300'
        : 'bg-amber-100 text-amber-700 dark:bg-amber-900 dark:text-amber-300'
}`}>
    {script.category === 'schema' ? t('scripts.schema') : t('scripts.library')}
</span>
```

**Step 2: Add category selector to "New Script" dialog**

Before the editor opens, show radio buttons for category selection:

```tsx
const [newScriptCategory, setNewScriptCategory] = useState<string>('presentation');
```

Add radio buttons in the "new script" flow (before entering editor view).

**Step 3: Prefill starter templates**

When category is selected and editor opens, prefill with the appropriate template:

```typescript
const SCHEMA_TEMPLATE = `// @name: MyType
// @description: Describe your note type here

schema("MyType", #{
    fields: [
        #{ name: "title_field", type: "text", required: true },
    ],
    on_save: |note| {
        commit();
    }
});
`;

const PRESENTATION_TEMPLATE = `// @name: MyType Views
// @description: Views and actions for MyType

register_view("MyType", "Summary", |note| {
    text("Custom view for " + note.title)
});
`;
```

Use `newScriptCategory === 'schema' ? SCHEMA_TEMPLATE : PRESENTATION_TEMPLATE` when opening the editor.

**Step 4: Pass category to `create_user_script`**

Update the `handleSave` to pass category:

```typescript
await invoke('create_user_script', {
    sourceCode: code,
    category: newScriptCategory,
});
```

**Step 5: Auto-detect category on file import**

In `handleImportFromFile()`, detect category from the file extension:

```typescript
const isSchema = path.endsWith('.schema.rhai');
const category = isSchema ? 'schema' : 'presentation';
// Pass category when creating the script
```

**Step 6: Warning icons for unresolved bindings**

Fetch warnings and display icons:

```typescript
const [warnings, setWarnings] = useState<ScriptWarning[]>([]);

useEffect(() => {
    invoke<ScriptWarning[]>('get_script_warnings').then(setWarnings);
}, [/* after script list loads */]);

// In script list item:
const scriptWarnings = warnings.filter(w => w.scriptName === script.name);
{scriptWarnings.length > 0 && (
    <span title={scriptWarnings.map(w => w.message).join('\n')}
          className="text-amber-500 cursor-help">
        !
    </span>
)}
```

**Step 7: Add i18n keys**

In `en.json` and other locale files, add:
```json
"scripts": {
    "schema": "Schema",
    "library": "Library",
    "category": "Category",
    "category_schema": "Schema (defines a note type)",
    "category_library": "Library / Presentation"
}
```

**Step 8: Run TypeScript check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

**Step 9: Commit**

```
git add -A && git commit -m "feat: Script Manager category badges, templates, and warnings"
```

---

## Task 9: Update ContextMenu and WorkspaceView for menu_registrations

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`
- Modify: `krillnotes-desktop/src/components/ContextMenu.tsx`

**Step 1: Verify tree action map still works**

The `get_tree_action_map` Tauri command now reads from `menu_registrations` instead
of `HookRegistry`. The frontend API is unchanged -- it still returns
`Record<string, string[]>`. Verify `WorkspaceView.tsx` still calls
`get_tree_action_map` and passes actions to `ContextMenu`. No frontend changes
should be needed for this.

**Step 2: Test manually**

Run: `cd krillnotes-desktop && npm run tauri dev`

Verify:
- Right-click a TextNote -> "Sort Children A-Z" and "Add Child Note" appear
- Right-click a Library -> all 5 sort actions appear
- Right-click a Kasten -> both sort actions appear
- Custom views render in tabs
- Fields tab is always present
- Edit mode switches to Fields tab

**Step 3: Commit (if any fixes needed)**

```
git add -A && git commit -m "fix: context menu integration with menu_registrations"
```

---

## Task 10: Update existing tests

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (test module)

**Step 1: Migrate all test scripts**

Every test that currently uses `on_view: |note| { ... }` or `on_hover: |note| { ... }`
inside `schema()`, or uses `add_tree_action()`, must be updated to use the new
`register_view()`/`register_hover()`/`register_menu()` + two-phase loading pattern.

Pattern for migrating a test:

```rust
// OLD:
registry.load_script(r#"
    schema("Test", #{
        fields: [],
        on_view: |note| { "html" }
    });
"#, "test").unwrap();

// NEW:
registry.load_script(r#"
    register_view("Test", "Default", |note| { "html" });
"#, "test_views.rhai").unwrap();

registry.load_script(r#"
    schema("Test", #{ fields: [] });
"#, "test.schema.rhai").unwrap();

registry.resolve_bindings();
```

**Step 2: Add new tests for phase 2 features**

Add tests for:
- `register_view` with `display_first: true`
- `register_hover` (last one wins)
- `register_menu` with multiple target types
- Unresolved binding produces warning
- `schema()` from presentation-category script fails
- Two-phase loading order (presentation before schema)
- Mixed script with library functions + `register_view`
- Multiple views for the same type

**Step 3: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: ALL PASS

**Step 4: Run TypeScript check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

**Step 5: Commit**

```
git add -A && git commit -m "test: migrate all tests to register_view/hover/menu pattern"
```

---

## Task 11: Final integration test and cleanup

**Step 1: Full build**

Run: `cd krillnotes-desktop && npm update && npm run tauri dev`

**Step 2: Manual verification checklist**

- [ ] Create a workspace, verify all system scripts load without errors
- [ ] Open a TextNote -- no tabs shown, Fields panel renders
- [ ] Open a Kasten (zettelkasten) -- "Overview" tab shown with `display_first`, Fields tab last
- [ ] Click "Overview" tab -- renders the zettel list view
- [ ] Click "Fields" tab -- shows empty fields
- [ ] Click "Edit" -- switches to Fields tab
- [ ] Cancel edit -- returns to previous tab
- [ ] Open a PhotoNote -- "Photo" tab with image, Fields tab last
- [ ] Hover a Kasten in tree -- hover popup shows note count
- [ ] Right-click TextNote -> Sort/Add actions appear
- [ ] Right-click Library -> 5 sort actions appear
- [ ] Script Manager -> scripts show blue/amber badges
- [ ] Create new Schema script -> prefilled template
- [ ] Create new Library script -> prefilled template
- [ ] Import `.schema.rhai` file -> category auto-detected as Schema

**Step 3: Cleanup**

Remove any dead code, unused imports, or temporary comments.

**Step 4: Final commit**

```
git add -A && git commit -m "chore: cleanup and final integration verification"
```
