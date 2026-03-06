# Schema Extensions Phase 2 — Design Document

**Spec:** `docs/swarm/KrillNotes_Schema_Extensions_Spec_v0_4.docx` (Sections 9.1-9.5, 9.10-9.11, 5.2, 7.3, 7.5)
**Parent plan:** `docs/plans/2026-03-05-schema-extensions-v04-overview.md`
**Date:** 2026-03-06
**Branch base:** `master`

Phase 2 delivers: the constructing/reading split (two file categories), deferred
presentation binding (`register_view`, `register_hover`, `register_menu`),
tabbed view mode, and Script Manager category UI. This is a **clean break** —
`on_view`, `on_hover`, and `add_tree_action()` are removed entirely.

---

## 1. File Categories and Script Storage

### 1.1 Two Categories

| Category | File extension | `schema()` allowed | `register_*()` allowed |
|----------|---------------|-------------------|----------------------|
| Schema | `.schema.rhai` | Yes | Yes (but unusual) |
| Presentation | `.rhai` | No (hard error) | Yes |

A `.rhai` file can contain anything except `schema()` calls: library functions,
`register_view()`, `register_hover()`, `register_menu()`, or a mix of all three.

### 1.2 System Scripts

| Current | New schema file | New presentation file |
|---------|----------------|----------------------|
| `00_text_note.rhai` | `00_text_note.schema.rhai` | *(none)* |
| `01_contact.rhai` | `01_contact.schema.rhai` | `01_contact.rhai` |
| `02_task.rhai` | `02_task.schema.rhai` | *(none)* |
| `03_project.rhai` | `03_project.schema.rhai` | *(none)* |
| `05_recipe.rhai` | `05_recipe.schema.rhai` | *(none)* |
| `06_product.rhai` | `06_product.schema.rhai` | *(none)* |

Templates follow the same split. E.g. `zettelkasten.rhai` becomes
`zettelkasten.schema.rhai` + `zettelkasten.rhai`.

The `include_dir!` embed picks up both extensions from the same directory.

### 1.3 User Scripts (DB)

The `user_scripts` table gains a `category` column:

```sql
ALTER TABLE user_scripts ADD COLUMN category TEXT NOT NULL DEFAULT 'presentation';
```

Values: `"schema"` or `"presentation"`. Existing user scripts default to
`"presentation"`. Calling `schema()` from a `"presentation"` category script
raises a hard error.

### 1.4 Removed Functions

The following are removed entirely (no deprecation, no migration messages):

- `on_view` key inside `schema()` map
- `on_hover` key inside `schema()` map
- `add_tree_action()` top-level function

---

## 2. Two-Phase Loading and Deferred Binding

### 2.1 Loading Order

When a workspace opens, scripts execute in this order:

1. **Phase A -- Library/Presentation** (`.rhai` / category `"presentation"`):
   Sorted by `load_order`. These define utility functions and call
   `register_view()`, `register_hover()`, `register_menu()`. Registrations
   go into a deferred binding queue since schemas don't exist yet.

2. **Phase B -- Schema** (`.schema.rhai` / category `"schema"`):
   Sorted by `load_order`. These call `schema()` which registers types in
   the `SchemaRegistry`.

3. **Phase C -- Resolve bindings**: Iterate the deferred queue. For each
   entry, look up the target type in `SchemaRegistry`. If found, attach the
   binding. If not found, mark as unresolved and store a warning.

### 2.2 Rationale for Library-First

Library utility functions (e.g. `tag_list()`, `strip_markdown()`) must be
available when schema scripts run, since schemas may reference them in
`validate` closures or `on_save` hooks. This becomes critical when ontology
helper functions are introduced in future phases.

### 2.3 Deferred Binding Queue

```rust
pub struct DeferredBinding {
    pub kind: BindingKind,
    pub target_type: String,
    pub fn_ptr: rhai::FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
    pub display_first: bool,
    pub label: Option<String>,
    pub applies_to: Vec<String>,
}

pub enum BindingKind {
    View,
    Hover,
    Menu,
}
```

The queue lives on `SchemaRegistry` as `deferred_bindings: Vec<DeferredBinding>`.
After resolution, resolved bindings move into the appropriate storage maps.
Unresolved entries go into `warnings: Vec<ScriptWarning>`.

### 2.4 Resolved Storage

After resolution, `SchemaRegistry` holds:

| Storage | Type | Key |
|---------|------|-----|
| `view_registrations` | `HashMap<String, Vec<ViewRegistration>>` | schema name -> ordered views |
| `hover_registrations` | `HashMap<String, HookEntry>` | schema name -> single hover (last wins) |
| `menu_registrations` | `HashMap<String, Vec<MenuRegistration>>` | schema name -> menu actions |

These replace the current `on_view_hooks`, `on_hover_hooks`, and the
`HookRegistry` tree actions.

```rust
pub struct ViewRegistration {
    pub label: String,
    pub display_first: bool,
    pub fn_ptr: rhai::FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}

pub struct MenuRegistration {
    pub label: String,
    pub fn_ptr: rhai::FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}
```

---

## 3. Rhai Registration Functions

### 3.1 `register_view(target_type, label, closure)` / `register_view(target_type, label, options, closure)`

Registers a custom view tab for a note type. Queued as a deferred binding.

```rhai
// Simple form
register_view("Kasten", "Overview", |note| {
    let zettel = get_children(note.id);
    stack([text(zettel.len().to_string() + " notes")])
});

// With options
register_view("Kasten", "Overview", #{ display_first: true }, |note| {
    // ...
});
```

The closure receives the same note map shape as the former `on_view` and has
access to all query functions (`get_children`, `get_notes_for_tag`, etc.) and
display helpers (`text`, `table`, `section`, `stack`, `link_to`, `markdown`,
`field`).

### 3.2 `register_hover(target_type, closure)`

Registers a hover popup renderer. One per type (last registration wins).

```rhai
register_hover("Kasten", |note| {
    let kids = get_children(note.id);
    field("Notes", kids.len().to_string())
});
```

### 3.3 `register_menu(label, target_types, closure)`

Replaces `add_tree_action()`. Same closure semantics -- return an array of IDs
for reordering, or use `set_field()`/`create_child()`/`commit()` for mutations.

```rhai
register_menu("Sort by Date (Newest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title >= b.title);
    children.map(|c| c.id)
});
```

### 3.4 Enforcement

- `schema()` called from a `"presentation"` script -> hard error
- `register_view()` / `register_hover()` / `register_menu()` callable from
  any script (both categories)
- `on_view` / `on_hover` keys in `schema()` -> removed (hard error, unknown key)
- `add_tree_action()` -> removed (undefined function error)

---

## 4. Tabbed View Mode

### 4.1 Tab Layout

When viewing a note, InfoPanel renders tabs:

```
[ display_first views ] [ other views in registration order ] [ Fields ]
```

- Custom view tabs come from `register_view()` calls
- `display_first: true` tabs are pushed leftmost
- "Fields" tab is always present, always rightmost
- Shows the current field display/edit panel (what InfoPanel renders today)

### 4.2 Tab Behavior

- **Default selected tab:** leftmost tab
- **Tab persistence:** selection persists while the note is selected; switching
  notes resets to the default tab
- **Edit mode:** clicking "Edit" switches to the Fields tab. Saving or
  cancelling returns to the previously selected tab.
- **No views registered:** no tab bar shown. Fields panel renders as today.
  Zero UI change for types without custom views.

### 4.3 New Tauri Commands

#### `get_views_for_type(schema_name) -> Vec<ViewInfo>`

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewInfo {
    pub label: String,
    pub display_first: bool,
}
```

Called when selecting a note to determine which tabs to render.

#### `render_view(note_id, view_label) -> String`

Returns HTML from the view closure. Reuses the same hook execution path as
the former `run_on_view_hook`, but looks up the view by label.

### 4.4 Frontend Types

```typescript
interface ViewInfo {
    label: string;
    displayFirst: boolean;
}
```

---

## 5. Script Manager Changes

### 5.1 Category Badges

Each script in the list gets a colored badge:
- **Blue: "Schema"** -- for `.schema.rhai` / category `"schema"`
- **Amber: "Library"** -- for `.rhai` / category `"presentation"`

### 5.2 Create Script Flow

The "New Script" dialog gains a category selector (radio buttons):
"Schema" or "Library/Presentation". Sets the `category` column.

After choosing, the editor prefills with a starter template:

**Schema template:**
```rhai
// @name: MyType
// @description: Describe your note type here

schema("MyType", #{
    fields: [
        #{ name: "title_field", type: "text", required: true },
    ],
    on_save: |note| {
        commit();
    }
});
```

**Presentation template:**
```rhai
// @name: MyType Views
// @description: Views and actions for MyType

register_view("MyType", "Summary", |note| {
    text("Custom view for " + note.title)
});
```

### 5.3 Import from File

Single "Load from File..." button. Category is auto-detected from extension:
- `.schema.rhai` -> category `"schema"`
- `.rhai` (not `.schema.rhai`) -> category `"presentation"`

### 5.4 Unresolved Binding Warnings

Scripts with unresolved bindings show a warning icon. Tooltip reveals details.

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptWarning {
    pub script_name: String,
    pub message: String,
}
```

Command: `get_script_warnings() -> Vec<ScriptWarning>`

---

## 6. Script Migration

All system scripts and templates are migrated to the new split format.

### 6.1 Example: Zettelkasten

**Before (single file):**
```rhai
fn tag_list(tags) { ... }
fn strip_markdown(s) { ... }

schema("Zettel", #{
    fields: [...],
    on_save: |note| { ... },
    on_view: |note| { ... }
});

schema("Kasten", #{
    fields: [],
    on_hover: |note| { ... },
    on_view: |note| { ... }
});

add_tree_action("Sort by Date", ["Kasten"], |note| { ... });
```

**After (two files):**

`zettelkasten.rhai` (presentation + library):
```rhai
fn tag_list(tags) { ... }
fn strip_markdown(s) { ... }

register_view("Zettel", "Content", #{ display_first: true }, |note| { ... });
register_view("Kasten", "Overview", #{ display_first: true }, |note| { ... });
register_hover("Kasten", |note| { ... });
register_menu("Sort by Date (Newest First)", ["Kasten"], |note| { ... });
register_menu("Sort by Date (Oldest First)", ["Kasten"], |note| { ... });
```

`zettelkasten.schema.rhai` (schemas only):
```rhai
schema("Zettel", #{
    title_can_edit: false,
    allowed_parent_types: ["Kasten"],
    fields: [
        #{ name: "body", type: "textarea", required: false, show_on_hover: true },
    ],
    on_save: |note| { ... }
});

schema("Kasten", #{
    allowed_children_types: ["Zettel"],
    fields: []
});
```

### 6.2 Contact (system script with on_view)

`01_contact.schema.rhai`: Both `ContactsFolder` and `Contact` schema definitions.
`01_contact.rhai`: `register_view("ContactsFolder", "Contacts", ...)` with the
table rendering that currently lives in the `on_view` closure.

---

## 7. Serde Boundary Rules

All Rust structs crossing the Rust->TS boundary MUST have
`#[serde(rename_all = "camelCase")]`. This includes:

- `ViewInfo` (label, displayFirst)
- `ScriptWarning` (scriptName, message)
- Any modified `SchemaInfo` fields
- `UserScript` if modified (category field)

Enum `rename_all` only renames variant names, NOT struct variant fields.
Always verify JSON keys match TS interfaces.

---

## 8. Files Changed (Summary)

### New files
- None (deferred binding logic added to existing `schema.rs`)

### Modified (Rust)
- `krillnotes-core/src/core/scripting/schema.rs` -- Remove `on_view_hooks`,
  `on_hover_hooks`. Add `DeferredBinding`, `BindingKind`, `ViewRegistration`,
  `MenuRegistration`, `ScriptWarning`. Add `view_registrations`,
  `hover_registrations`, `menu_registrations`, `deferred_bindings`, `warnings`.
  Add `resolve_bindings()` method.
- `krillnotes-core/src/core/scripting/mod.rs` -- Register `register_view()`,
  `register_hover()`, `register_menu()`. Remove `add_tree_action()` registration.
  Remove `on_view`/`on_hover` extraction from `schema()`. Update `run_on_view_hook`
  to use `view_registrations`. Add two-phase loading logic. Add `render_view()`
  (by label). Update `run_on_hover_hook` to use `hover_registrations`.
- `krillnotes-core/src/core/scripting/hooks.rs` -- Remove tree action storage
  (replaced by `menu_registrations` on `SchemaRegistry`).
- `krillnotes-core/src/core/workspace.rs` -- Update script loading to two-phase
  order. Update tree action invocation to use `menu_registrations`.
- `krillnotes-core/src/core/user_script.rs` -- Add `category` field to
  `UserScript` struct.
- `krillnotes-core/src/core/storage.rs` -- DB migration: add `category` column
  to `user_scripts`.
- `krillnotes-desktop/src-tauri/src/lib.rs` -- Add `get_views_for_type`,
  `render_view`, `get_script_warnings` Tauri commands.

### Modified (TypeScript)
- `krillnotes-desktop/src/types.ts` -- Add `ViewInfo`, `ScriptWarning`.
  Add `category` to `UserScript`. Update `SchemaInfo` if needed.
- `krillnotes-desktop/src/components/InfoPanel.tsx` -- Tabbed view mode,
  tab selection, edit-mode-switches-to-Fields behavior.
- `krillnotes-desktop/src/components/ScriptManagerDialog.tsx` -- Category
  badges, category selector in create flow, starter templates, warning icons,
  auto-detect on import.

### Modified (Scripts)
- All 6 system scripts in `krillnotes-core/src/system_scripts/`
- All 3 templates in `templates/`

---

## 9. Open Questions Resolved

| # | Question | Decision |
|---|----------|----------|
| 1 | User scripts split too? | Yes -- DB `category` column. Required for sync. |
| 2 | Backward compat for on_view/on_hover/add_tree_action | Clean break. Removed entirely. |
| 3 | Fields tab behavior | Always present, always rightmost. |
| 4 | `display_first` | Pushes view tab to leftmost position. |
| 5 | Unresolved bindings | Silent skip + Script Manager warning badge. |
| 6 | Loading order | `.rhai` first, `.schema.rhai` second, resolve third. |
| 7 | `.rhai` file contents | Anything except `schema()` calls. |
| 8 | Import category detection | Auto-detect from file extension. |
| 9 | New script templates | Prefilled starter code per category. |
| 10 | `register_menu()` scope | Replaces `add_tree_action()`. Same closure semantics. |
