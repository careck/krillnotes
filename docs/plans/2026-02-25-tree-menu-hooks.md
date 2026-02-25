# Tree Context Menu Hooks Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let Rhai scripts register custom entries in the tree right-click menu via `add_tree_action(label, types, callback)`.

**Architecture:** `HookRegistry` gains a `tree_actions` vec. `ScriptRegistry` registers the `add_tree_action` Rhai host function and exposes `tree_action_map()` / `invoke_tree_action_hook()` delegation methods. Two new Tauri commands let the frontend fetch the action map on load and invoke actions on click. If the callback returns an array of note IDs, the backend reorders those notes by position — enabling sort actions without write-access Rhai functions.

**Tech Stack:** Rust (rhai, rusqlite), TypeScript/React, Tauri v2

---

### Task 1: Add `TreeActionEntry` to `HookRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/hooks.rs`

**Step 1: Write the failing test**

At the bottom of `hooks.rs` (add a `#[cfg(test)]` block):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_tree_action_adds_entry() {
        let registry = HookRegistry::new();
        // We can't construct FnPtr/AST in a unit test without a full engine,
        // so just test the map method on an empty registry.
        let map = registry.tree_action_map();
        assert!(map.is_empty(), "fresh registry should have no tree actions");
    }

    #[test]
    fn test_clear_removes_tree_actions() {
        let registry = HookRegistry::new();
        registry.clear();
        let map = registry.tree_action_map();
        assert!(map.is_empty());
    }
}
```

**Step 2: Run tests to verify they fail**

```
cargo test -p krillnotes-core scripting::hooks::tests 2>&1 | head -30
```

Expected: compile error — `tree_action_map` and `clear` don't exist yet.

**Step 3: Implement**

Replace the entire contents of `hooks.rs`:

```rust
//! Hook registry for global / lifecycle hooks (tree menu actions, on_load, on_export, …).
//!
//! Schema-bound hooks (`on_save`, `on_view`, `on_add_child`) are managed by
//! [`SchemaRegistry`](super::schema::SchemaRegistry) and registered via the
//! `schema()` Rhai host function.

use rhai::{FnPtr, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A user-registered tree context-menu action.
pub struct TreeActionEntry {
    pub label:         String,
    pub allowed_types: Vec<String>,
    pub(super) fn_ptr: FnPtr,
    pub(super) ast:    AST,
}

impl std::fmt::Debug for TreeActionEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeActionEntry")
            .field("label", &self.label)
            .field("allowed_types", &self.allowed_types)
            .finish()
    }
}

/// Registry for global event hooks not tied to a specific schema.
///
/// Currently holds tree context-menu actions; on_load, on_export, and other
/// lifecycle hooks will be added here in future tasks.
///
/// Constructed only by `ScriptRegistry::new()`.
#[derive(Debug)]
pub struct HookRegistry {
    tree_actions: Arc<Mutex<Vec<TreeActionEntry>>>,
}

impl HookRegistry {
    pub(super) fn new() -> Self {
        Self {
            tree_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Arc clone of tree_actions for use in Rhai host-function closures.
    pub(super) fn tree_actions_arc(&self) -> Arc<Mutex<Vec<TreeActionEntry>>> {
        Arc::clone(&self.tree_actions)
    }

    /// Appends a new tree action. Logs a warning if a duplicate label+type
    /// combination already exists (first-registered wins).
    pub(super) fn register_tree_action(&self, entry: TreeActionEntry) {
        let mut actions = self.tree_actions.lock().unwrap();
        // Warn on duplicate label per allowed type.
        for ty in &entry.allowed_types {
            if actions.iter().any(|a| &a.label == &entry.label && a.allowed_types.contains(ty)) {
                eprintln!(
                    "[krillnotes] tree action label {:?} already registered for type {ty:?}; \
                     first-registered entry wins",
                    entry.label
                );
            }
        }
        actions.push(entry);
    }

    /// Returns a map of `note_type → [action_label, …]` for every registered action.
    pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
        let actions = self.tree_actions.lock().unwrap();
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for entry in actions.iter() {
            for ty in &entry.allowed_types {
                map.entry(ty.clone()).or_default().push(entry.label.clone());
            }
        }
        map
    }

    /// Removes all registered tree actions so scripts can be reloaded.
    pub(super) fn clear(&self) {
        self.tree_actions.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_tree_action_adds_entry() {
        let registry = HookRegistry::new();
        let map = registry.tree_action_map();
        assert!(map.is_empty(), "fresh registry should have no tree actions");
    }

    #[test]
    fn test_clear_removes_tree_actions() {
        let registry = HookRegistry::new();
        registry.clear();
        let map = registry.tree_action_map();
        assert!(map.is_empty());
    }
}
```

**Step 4: Run tests**

```
cargo test -p krillnotes-core scripting::hooks::tests 2>&1 | tail -10
```

Expected: `test result: ok. 2 passed`

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/hooks.rs
git commit -m "feat: add TreeActionEntry and tree_action_map to HookRegistry"
```

---

### Task 2: Wire `add_tree_action` into `ScriptRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Context:** `ScriptRegistry::new()` registers all Rhai host functions. The `on_add_child` registration at line ~132 is the best pattern to follow. `clear_all()` at line ~406 needs to also call `hook_registry.clear()`.

**Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `mod.rs`:

```rust
// ── tree actions ─────────────────────────────────────────────────────────────

#[test]
fn test_add_tree_action_registers_entry() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        add_tree_action("Sort Children", ["TextNote"], |note| { () });
    "#, "test_script").unwrap();
    let map = registry.tree_action_map();
    assert_eq!(map.get("TextNote"), Some(&vec!["Sort Children".to_string()]));
}

#[test]
fn test_tree_action_map_empty_before_load() {
    let registry = ScriptRegistry::new().unwrap();
    assert!(registry.tree_action_map().is_empty());
}

#[test]
fn test_clear_all_removes_tree_actions() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        add_tree_action("Do Thing", ["TextNote"], |note| { () });
    "#, "test_script").unwrap();
    registry.clear_all();
    assert!(registry.tree_action_map().is_empty());
}

#[test]
fn test_invoke_tree_action_hook_calls_callback() {
    use crate::FieldValue;
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("TextNote", #{ fields: [] });
        add_tree_action("Noop", ["TextNote"], |note| { () });
    "#, "test_script").unwrap();
    let note = crate::Note {
        id: "n1".into(), title: "Hello".into(),
        node_type: "TextNote".into(), parent_id: None,
        fields: std::collections::HashMap::new(), position: 0,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
    };
    let result = registry.invoke_tree_action_hook("Noop", &note, ctx).unwrap();
    assert!(result.is_none(), "callback returning () should yield None");
}

#[test]
fn test_invoke_tree_action_returns_id_vec_when_callback_returns_array() {
    use crate::FieldValue;
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("TextNote", #{ fields: [] });
        add_tree_action("Sort", ["TextNote"], |note| { ["id-b", "id-a"] });
    "#, "test_script").unwrap();
    let note = crate::Note {
        id: "p1".into(), title: "Parent".into(),
        node_type: "TextNote".into(), parent_id: None,
        fields: std::collections::HashMap::new(), position: 0,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
    };
    let result = registry.invoke_tree_action_hook("Sort", &note, ctx).unwrap();
    assert_eq!(result, Some(vec!["id-b".to_string(), "id-a".to_string()]));
}

#[test]
fn test_invoke_tree_action_unknown_label_errors() {
    let registry = ScriptRegistry::new().unwrap();
    let note = crate::Note {
        id: "n1".into(), title: "T".into(),
        node_type: "TextNote".into(), parent_id: None,
        fields: std::collections::HashMap::new(), position: 0,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
    };
    let err = registry.invoke_tree_action_hook("No Such Action", &note, ctx).unwrap_err();
    assert!(err.to_string().contains("unknown tree action"), "got: {err}");
}

#[test]
fn test_invoke_tree_action_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("TextNote", #{ fields: [] });
        add_tree_action("Boom", ["TextNote"], |note| { throw "intentional"; });
    "#, "my_script").unwrap();
    let note = crate::Note {
        id: "n1".into(), title: "T".into(),
        node_type: "TextNote".into(), parent_id: None,
        fields: std::collections::HashMap::new(), position: 0,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
    };
    let err = registry.invoke_tree_action_hook("Boom", &note, ctx).unwrap_err();
    assert!(err.to_string().contains("my_script"), "error should include script name, got: {err}");
}
```

**Step 2: Run to verify tests fail**

```
cargo test -p krillnotes-core test_add_tree_action 2>&1 | head -20
cargo test -p krillnotes-core test_invoke_tree_action 2>&1 | head -20
```

Expected: compile errors — `hook_registry`, `tree_action_map`, `invoke_tree_action_hook` don't exist.

**Step 3: Implement**

In `mod.rs`, make these changes:

**3a. Add `hook_registry` field to `ScriptRegistry` struct** (after `schema_registry`):

```rust
pub struct ScriptRegistry {
    engine: Engine,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
    current_loading_script_name: Arc<Mutex<Option<String>>>,
    schema_owners: Arc<Mutex<HashMap<String, String>>>,
    schema_registry: schema::SchemaRegistry,
    hook_registry: hooks::HookRegistry,   // ← ADD
    query_context: Arc<Mutex<Option<QueryContext>>>,
}
```

**3b. In `ScriptRegistry::new()`, construct `hook_registry` and register `add_tree_action`**

After line that creates `schema_registry` (around line 81):

```rust
let hook_registry = hooks::HookRegistry::new();
let tree_actions_arc = hook_registry.tree_actions_arc();
let add_tree_name_arc = Arc::clone(&current_loading_script_name);
let add_tree_ast_arc  = Arc::clone(&current_loading_ast);
engine.register_fn("add_tree_action",
    move |label: String, types: rhai::Array, fn_ptr: FnPtr|
    -> std::result::Result<Dynamic, Box<EvalAltResult>>
    {
        let ast = add_tree_ast_arc.lock().unwrap().clone()
            .ok_or_else(|| -> Box<EvalAltResult> {
                "add_tree_action() called outside of load_script".to_string().into()
            })?;
        let script_name = add_tree_name_arc.lock().unwrap()
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string());
        let allowed_types: Vec<String> = types
            .into_iter()
            .filter_map(|v| v.try_cast::<String>())
            .collect();
        let entry = hooks::TreeActionEntry {
            label,
            allowed_types,
            fn_ptr,
            ast,
            script_name,
        };
        tree_actions_arc.lock().unwrap().push(entry);
        Ok(Dynamic::UNIT)
    }
);
```

Note: add `script_name: String` field to `TreeActionEntry` in `hooks.rs` as well (used for error messages).

**3c. In the `ScriptRegistry` struct initializer**, add `hook_registry` after `schema_registry`:

```rust
Ok(Self {
    engine,
    current_loading_ast,
    current_loading_script_name,
    schema_owners,
    schema_registry,
    hook_registry,   // ← ADD
    query_context,
})
```

**3d. Add delegation methods** after `clear_all()`:

```rust
/// Returns a map of `note_type → [action_label, …]` for every registered tree action.
pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
    self.hook_registry.tree_action_map()
}

/// Runs the tree action registered under `label`, passing `note` to the callback.
///
/// Returns `Ok(Some(ids))` if the callback returns an array of strings (a reorder request).
/// Returns `Ok(None)` if the callback returns any other value.
/// Returns `Err(...)` if the callback throws a Rhai error.
pub fn invoke_tree_action_hook(
    &self,
    label: &str,
    note: &Note,
    context: QueryContext,
) -> Result<Option<Vec<String>>> {
    let entry = {
        let actions = self.hook_registry.tree_actions_arc().lock().unwrap();
        actions.iter()
            .find(|a| a.label == label)
            .map(|a| (a.fn_ptr.clone(), a.ast.clone(), a.script_name.clone()))
    };

    let (fn_ptr, ast, script_name) = entry.ok_or_else(|| {
        KrillnotesError::Scripting(format!("unknown tree action: {label:?}"))
    })?;

    // Build note map — same shape as on_save / on_view.
    let mut fields_map = rhai::Map::new();
    for (k, v) in &note.fields {
        fields_map.insert(k.as_str().into(), schema::field_value_to_dynamic(v));
    }
    let mut note_map = rhai::Map::new();
    note_map.insert("id".into(),        Dynamic::from(note.id.clone()));
    note_map.insert("node_type".into(), Dynamic::from(note.node_type.clone()));
    note_map.insert("title".into(),     Dynamic::from(note.title.clone()));
    note_map.insert("fields".into(),    Dynamic::from(fields_map));

    // Install query context, run, then clear.
    *self.query_context.lock().unwrap() = Some(context);
    let raw = fn_ptr
        .call::<Dynamic>(&self.engine, &ast, (Dynamic::from_map(note_map),))
        .map_err(|e| KrillnotesError::Scripting(
            format!("[{script_name}] tree action {label:?}: {e}")
        ));
    *self.query_context.lock().unwrap() = None;
    let raw = raw?;

    // If callback returns an Array of Strings, treat as reorder request.
    if let Some(arr) = raw.try_cast::<rhai::Array>() {
        let ids: Vec<String> = arr.into_iter()
            .filter_map(|v| v.try_cast::<String>())
            .collect();
        return Ok(Some(ids));
    }

    Ok(None)
}
```

**3e. Update `clear_all()`** to also clear tree actions:

```rust
pub fn clear_all(&self) {
    self.schema_registry.clear();
    self.schema_owners.lock().unwrap().clear();
    self.hook_registry.clear();          // ← ADD
    *self.query_context.lock().unwrap() = None;
}
```

**Step 4: Add `script_name` to `TreeActionEntry` in hooks.rs**

In `hooks.rs`, add the field and update `register_tree_action`:

```rust
pub struct TreeActionEntry {
    pub label:           String,
    pub allowed_types:   Vec<String>,
    pub(super) script_name: String,    // ← ADD
    pub(super) fn_ptr:   FnPtr,
    pub(super) ast:      AST,
}
```

Also update `register_tree_action` warning to use `entry.script_name`.

**Step 5: Run tests**

```
cargo test -p krillnotes-core 2>&1 | tail -15
```

Expected: all tests pass (including the 6 new tree action tests).

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/hooks.rs \
        krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: register add_tree_action Rhai host function in ScriptRegistry"
```

---

### Task 3: Add `run_tree_action` to `Workspace` and two Tauri commands

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Write failing tests for the workspace method**

Add to `workspace.rs` tests:

```rust
#[test]
fn test_run_tree_action_reorders_children() {
    let mut ws = open_test_workspace();
    // Create parent + two children
    let parent = ws.create_note(None, "TextNote", "Parent", 0).unwrap();
    let child_a = ws.create_note(Some(&parent.id), "TextNote", "B Note", 0).unwrap();
    let child_b = ws.create_note(Some(&parent.id), "TextNote", "A Note", 1).unwrap();

    // Load a script that sorts children alphabetically
    ws.create_user_script(r#"
// @name: SortTest
add_tree_action("Sort A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
    "#).unwrap();

    ws.run_tree_action(&parent.id, "Sort A→Z").unwrap();

    let kids = ws.get_children(&parent.id).unwrap();
    assert_eq!(kids[0].title, "A Note");
    assert_eq!(kids[1].title, "B Note");
}
```

**Step 2: Run to verify failure**

```
cargo test -p krillnotes-core test_run_tree_action 2>&1 | head -20
```

Expected: compile error — `run_tree_action` doesn't exist.

**Step 3: Add `run_tree_action` to workspace.rs**

Find `run_view_hook` (around line 689) and add after it:

```rust
/// Runs the tree action named `label` on the note identified by `note_id`.
///
/// Builds a full `QueryContext` (same as `run_view_hook`), calls the registered
/// callback, and — if the callback returns an array of note IDs — reorders
/// those notes by calling `move_note` in the given order.
///
/// # Errors
///
/// Returns an error if the note or any workspace note cannot be fetched, if
/// no action is registered under `label`, or if the callback throws.
pub fn run_tree_action(&mut self, note_id: &str, label: &str) -> Result<()> {
    let note = self.get_note(note_id)?;
    let all_notes = self.list_all_notes()?;

    let mut notes_by_id: std::collections::HashMap<String, Dynamic> =
        std::collections::HashMap::new();
    let mut children_by_id: std::collections::HashMap<String, Vec<Dynamic>> =
        std::collections::HashMap::new();
    let mut notes_by_type: std::collections::HashMap<String, Vec<Dynamic>> =
        std::collections::HashMap::new();
    for n in &all_notes {
        let dyn_map = note_to_rhai_dynamic(n);
        notes_by_id.insert(n.id.clone(), dyn_map.clone());
        if let Some(pid) = &n.parent_id {
            children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
        }
        notes_by_type.entry(n.node_type.clone()).or_default().push(dyn_map);
    }
    let context = QueryContext { notes_by_id, children_by_id, notes_by_type };

    let reorder = self.script_registry.invoke_tree_action_hook(label, &note, context)?;

    if let Some(ids) = reorder {
        for (position, id) in ids.iter().enumerate() {
            self.move_note(id, note.parent_id.as_deref(), position as i32)?;
        }
    }

    Ok(())
}

/// Returns a map of `note_type → [action_label, …]` from the script registry.
pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
    self.script_registry.tree_action_map()
}
```

Also add the necessary import at the top of the file if not present:
```rust
use std::collections::HashMap;
use rhai::Dynamic;
```

**Step 4: Run workspace tests**

```
cargo test -p krillnotes-core test_run_tree_action 2>&1 | tail -10
```

Expected: test passes.

**Step 5: Add two Tauri commands to lib.rs**

After `get_all_schemas` (around line 498), add:

```rust
/// Returns a map of `note_type → [action_label, …]` for all registered tree actions.
#[tauri::command]
fn get_tree_action_map(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, Vec<String>>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.tree_action_map())
}

/// Runs the tree action `label` on `note_id` and refreshes the workspace.
#[tauri::command]
fn invoke_tree_action(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    label: String,
) -> std::result::Result<(), String> {
    let window_label = window.label().to_string();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(&window_label).ok_or("No workspace open")?;
    workspace.run_tree_action(&note_id, &label)
        .map_err(|e| e.to_string())
}
```

**Register both commands** in the `invoke_handler!` list (find `get_all_schemas` in the handler list and add the two new commands next to it).

**Step 6: Build**

```
cargo build -p krillnotes-desktop 2>&1 | grep -E "^error" | head -20
```

Expected: clean build.

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs \
        krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add run_tree_action to Workspace and Tauri commands"
```

---

### Task 4: Add example sort hook to `00_text_note.rhai`

**Files:**
- Modify: `krillnotes-core/src/system_scripts/00_text_note.rhai`

**Step 1: Check that the Rhai sort works in an integration test**

Check the workspace test from Task 3 passes:

```
cargo test -p krillnotes-core test_run_tree_action 2>&1 | tail -5
```

Expected: still passing.

**Step 2: Add the example action**

Append to `00_text_note.rhai`:

```rhai
// Example tree action: sort a TextNote's children alphabetically by title.
// Right-click any TextNote in the tree and choose "Sort Children A→Z".
add_tree_action("Sort Children A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
```

**Step 3: Run all core tests**

```
cargo test -p krillnotes-core 2>&1 | tail -10
```

Expected: all pass (the `load_text_note` helper test loads this script, so it should still compile).

**Step 4: Commit**

```bash
git add krillnotes-core/src/system_scripts/00_text_note.rhai
git commit -m "feat: add Sort Children A→Z example tree action to TextNote script"
```

---

### Task 5: Frontend — fetch `treeActionMap` in `WorkspaceView`

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Context:** `loadNotes` (around line 114) already calls `Promise.all([list_notes, get_all_schemas])`. Extend it to also call `get_tree_action_map`. The `treeActionMap` state is `Record<string, string[]>`.

**Step 1: Add state and update `loadNotes`**

After the existing `const [schemas, setSchemas] = useState(...)` state declaration, add:

```typescript
const [treeActionMap, setTreeActionMap] = useState<Record<string, string[]>>({});
```

In `loadNotes`, extend `Promise.all`:

```typescript
const [fetchedNotes, allSchemas, actionMap] = await Promise.all([
    invoke<Note[]>('list_notes'),
    invoke<Record<string, SchemaInfo>>('get_all_schemas'),
    invoke<Record<string, string[]>>('get_tree_action_map'),
]);
setNotes(fetchedNotes);
setSchemas(allSchemas);
setTreeActionMap(actionMap);
```

**Step 2: Add `handleTreeAction`**

After `copyNote` and `pasteNote`, add:

```typescript
const handleTreeAction = useCallback(async (noteId: string, label: string) => {
    try {
        await invoke('invoke_tree_action', { noteId, label });
        await loadNotes();
    } catch (err) {
        setError(`Tree action failed: ${err}`);
    }
}, []);
```

**Step 3: Pass props to `ContextMenu`**

Find the `<ContextMenu ... />` render in WorkspaceView and add:

```tsx
treeActions={treeActionMap[contextMenu.noteType] ?? []}
onTreeAction={(label) => handleTreeAction(contextMenu.noteId, label)}
```

Note: `contextMenu` state needs to store `noteType` as well as `noteId`. Update `contextMenu` state type to include `noteType: string`, and update `handleContextMenu` to also capture `note.node_type`.

**Step 4: Build (TypeScript check)**

```
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: errors about missing `treeActions`/`onTreeAction` props on `ContextMenu` (those come in Task 6).

**Step 5: Commit** (after Task 6 makes it compile cleanly)

Defer commit to after Task 6.

---

### Task 6: Frontend — add dynamic items to `ContextMenu`

**Files:**
- Modify: `krillnotes-desktop/src/components/ContextMenu.tsx`

**Step 1: Add new props**

Add to `ContextMenuProps`:

```typescript
treeActions: string[];
onTreeAction: (label: string) => void;
```

**Step 2: Render dynamic items**

In the JSX, between the "Paste as Sibling" button and the `<div className="border-t ...">` separator before Delete, add:

```tsx
{treeActions.length > 0 && (
    <>
        <div className="border-t border-secondary my-1" />
        {treeActions.map((label) => (
            <button
                key={label}
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-secondary"
                onClick={() => { onTreeAction(label); onClose(); }}
            >
                {label}
            </button>
        ))}
    </>
)}
```

**Step 3: Type check**

```
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: clean (no errors).

**Step 4: Full build**

```
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

Expected: clean build.

**Step 5: Commit both Task 5 and Task 6 changes**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx \
        krillnotes-desktop/src/components/ContextMenu.tsx
git commit -m "feat: show and invoke tree actions in context menu"
```

---

### Task 7: Update `SCRIPTING.md`

**Files:**
- Modify: `SCRIPTING.md`

**Step 1: Add `add_tree_action` to the table of contents and content**

Find the Table of Contents (section 1 is "Script structure"). Add a new section after the existing `on_add_child` hook section:

In the TOC, add:
```markdown
8. [add_tree_action](#8-add_tree_action)
```

Add new section before "Display helpers":

```markdown
## 8. add_tree_action

`add_tree_action` registers a custom entry in the tree's right-click context menu.

```rhai
add_tree_action(label, allowed_types, callback)
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `label` | String | Menu item text shown to the user |
| `allowed_types` | Array of Strings | Schema names for which the item appears |
| `callback` | Closure `\|note\| { … }` | Called when the user clicks the item |

The `note` argument has the same shape as in `on_save` — `id`, `node_type`, `title`, and `fields`.

The callback can use query functions (`get_children`, `get_note`, etc.) to read workspace state. If it returns an array of note ID strings, the backend reorders those notes in the given order. Any other return value is ignored. The tree refreshes automatically after the callback completes.

**Example — sort children alphabetically:**

```rhai
add_tree_action("Sort Children A→Z", ["Folder"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
```

**Label uniqueness:** Labels must be unique per note type. If two scripts register the same label for the same type, the first-registered entry wins and a warning is printed.
```

Renumber all subsequent sections (Display helpers → 9, Query functions → 10, etc.) and update the TOC accordingly.

**Step 2: Commit**

```bash
git add SCRIPTING.md
git commit -m "docs: document add_tree_action in SCRIPTING.md"
```

---

### Task 8: Manual smoke test

**Step 1: Run the app**

```
cd krillnotes-desktop && npm run tauri dev
```

**Step 2: Open a workspace with TextNote notes**

1. Open or create a workspace.
2. Create a TextNote with two or more TextNote children in arbitrary order.

**Step 3: Verify context menu**

1. Right-click the parent TextNote.
2. Confirm "Sort Children A→Z" appears in the menu above the Delete item.

**Step 4: Invoke the action**

1. Click "Sort Children A→Z".
2. Confirm children reorder alphabetically.
3. Confirm no error banner appears.

**Step 5: Verify non-TextNote nodes don't show the action**

1. Right-click a non-TextNote node (e.g. a Task or Contact).
2. Confirm "Sort Children A→Z" does **not** appear.

**Step 6: Commit nothing** — smoke test only.

---

### Task 9: Open PR

```bash
git push -u origin feat/tree-menu-hooks
gh pr create \
  --title "feat: tree context menu hooks (issue #7)" \
  --body "Closes #7

Adds \`add_tree_action(label, types, callback)\` Rhai API for registering custom tree context menu entries. Includes a Sort Children A→Z example on TextNote."
```
