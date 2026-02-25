# Design: Tree Context Menu Hooks

**Date:** 2026-02-25
**Issue:** #7
**Status:** Approved

## Summary

Scripts can register custom entries in the tree's right-click context menu via a new
top-level Rhai function `add_tree_action`. When the user clicks a custom item, the
registered callback receives the selected note and the tree refreshes automatically.

---

## Rhai API

```rhai
add_tree_action("Sort Children A→Z", ["Folder", "Project"], |note| {
    // note: same map shape as on_save / on_view
    // can call get_children(), update_note(), etc.
    // return value is ignored — tree refreshes unconditionally
});
```

Three arguments: label (String), allowed note types (Array of Strings), callback (FnPtr).

**Label uniqueness:** Labels must be unique per note type. If two scripts register the
same label for the same type, the first-registered entry wins and a load warning is
emitted — consistent with schema name collision policy.

### Example (added to `00_text_note.rhai`)

```rhai
add_tree_action("Sort Children A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    let sorted = children.sort_by(|a, b| a.title < b.title);
    let i = 0;
    for child in sorted {
        move_note(child.id, note.id, i);
        i += 1;
    }
});
```

---

## Two-Tier Hook Architecture (updated)

| Tier | Hooks | Owned by | Registered via |
|------|-------|----------|----------------|
| Schema-bound | `on_save`, `on_view`, `on_add_child` | `SchemaRegistry` | `schema()` call |
| Global / lifecycle | tree actions (this feature), `on_load`, `on_export` (future) | `HookRegistry` | standalone functions |

---

## Backend — `HookRegistry` (`hooks.rs`)

`HookRegistry` gains a new entry type and a `Vec` of registered actions:

```rust
pub struct TreeActionEntry {
    pub label:         String,
    pub allowed_types: Vec<String>,
    pub(crate) fn_ptr: FnPtr,
    pub(crate) ast:    AST,
}

pub struct HookRegistry {
    tree_actions: Arc<Mutex<Vec<TreeActionEntry>>>,
}
```

New public methods on `HookRegistry`:
- `register_tree_action(entry: TreeActionEntry)` — appends entry, warns on label collision
- `tree_action_map() -> HashMap<String, Vec<String>>` — type → labels (for frontend)
- `invoke_tree_action(engine, note_map, label) -> Result<()>` — finds entry by label, calls closure

---

## Backend — `ScriptRegistry` (`mod.rs`)

Registers `add_tree_action` as a Rhai host function during engine setup:

```rust
engine.register_fn("add_tree_action",
    move |label: String, types: rhai::Array, fn_ptr: FnPtr| { ... });
```

The closure uses the `current_loading_ast` arc (already available) to store a
`TreeActionEntry` in `HookRegistry`.

New delegation methods on `ScriptRegistry`:
- `tree_action_map() -> HashMap<String, Vec<String>>`
- `invoke_tree_action(note_id, label) -> Result<()>`

`clear_user_scripts()` also clears user-registered tree actions (keeping system ones).

---

## Backend — Tauri Commands (`lib.rs`)

Two new commands:

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `get_tree_action_map` | `window_label` | `HashMap<String, Vec<String>>` | Called on workspace open and after every script reload |
| `invoke_tree_action` | `window_label, note_id: String, label: String` | `Result<(), String>` | Called on menu item click |

`invoke_tree_action` loads the note from the DB, converts it to a Rhai map, calls the
closure, then emits the existing workspace-changed event so the frontend tree refreshes.

---

## Frontend

### WorkspaceView

New state: `treeActionMap: Record<string, string[]>`

- Fetched on workspace open alongside schema list
- Re-fetched after every script reload (same call sites that already re-fetch schemas)
- `handleTreeAction(noteId: string, label: string)` calls `invoke_tree_action`, surfaces
  errors via `StatusMessage`

### ContextMenu

New props:
```typescript
treeActions: string[]       // pre-filtered by right-clicked note's type
onTreeAction: (label: string) => void
```

Dynamic items render between the paste group and the Delete separator — only when
`treeActions.length > 0`:

```
Add Note
Edit
Copy Note
Paste as Child
Paste as Sibling
──────────────    ← only shown if treeActions.length > 0
Sort Children A→Z
Sort by Priority
──────────────
Delete
```

---

## Error Handling

| Situation | Behaviour |
|-----------|-----------|
| Rhai callback throws | `Err(String)` returned from Tauri command; displayed via `StatusMessage` |
| Label collision on load | Warning logged; first-registered entry wins |
| Action invoked with unknown label | `Err("unknown tree action: …")` |

---

## Files to Touch

| File | Change |
|------|--------|
| `krillnotes-core/src/core/scripting/hooks.rs` | Add `TreeActionEntry`, `tree_actions` vec, new methods |
| `krillnotes-core/src/core/scripting/mod.rs` | Register `add_tree_action` host fn; add delegation methods; clear on user reload |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Add `get_tree_action_map`, `invoke_tree_action` commands |
| `krillnotes-core/src/system_scripts/00_text_note.rhai` | Add example `add_tree_action` for sorting TextNote children |
| `krillnotes-desktop/src/components/WorkspaceView.tsx` | Add `treeActionMap` state, fetch calls, `handleTreeAction` |
| `krillnotes-desktop/src/components/ContextMenu.tsx` | Add `treeActions` + `onTreeAction` props; render dynamic items |
| `SCRIPTING.md` | Document `add_tree_action` |

## Out of Scope

- Dynamic labels evaluated per note (e.g. "Sort 5 children") — future enhancement
- Action groups / submenus
- Keyboard shortcuts for custom actions
