# On-View Hook — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an `on_view` Rhai hook that returns custom HTML for a note's view panel, backed by a display-helper DSL and top-level query functions.

**Branch:** `feat/on-view-hook`
**Worktree:** `.worktrees/feat/on-view-hook/`

**Architecture:** A new `on_view` hook type in `HookRegistry` mirrors `on_save`. A `QueryContext` struct in `ScriptRegistry` is pre-built from all workspace notes before each hook call, enabling `get_children()` / `get_note()` / `get_notes_of_type()` as top-level Rhai functions. Pure Rust display helpers (`table`, `section`, `badge`, etc.) are registered on the engine and return `kn-view-*`-styled HTML strings. A new Tauri command `get_note_view` feeds the result to `InfoPanel`, which renders it via DOMPurify + `dangerouslySetInnerHTML`.

**Tech Stack:** Rust/Rhai (backend), Tauri v2 command (bridge), React/TypeScript + DOMPurify (frontend)

---

### Task 1: Extend `HookRegistry` with `on_view` support

**Files:**
- Modify: `krillnotes-core/src/core/scripting/hooks.rs`

Add `on_view_hooks` field to `HookRegistry`:
```rust
pub struct HookRegistry {
    on_save_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
    on_view_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,  // NEW
}
```

Update `new()` to initialise the new field.

Update `clear()` to also clear `on_view_hooks`.

Add new methods:
```rust
pub(super) fn on_view_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
    Arc::clone(&self.on_view_hooks)
}

pub fn has_view_hook(&self, schema_name: &str) -> bool {
    self.on_view_hooks.lock().unwrap().contains_key(schema_name)
}

pub fn run_on_view_hook(
    &self,
    engine: &Engine,
    note_map: rhai::Map,
) -> Result<Option<String>> {
    // 1. Determine schema name from note_map["node_type"]
    // 2. Clone entry out of mutex (release lock before call)
    // 3. Return Ok(None) if no hook registered
    // 4. Call: entry.fn_ptr.call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
    // 5. Cast result to String, return Ok(Some(html))
}
```

---

### Task 2: Create `display_helpers.rs`

**Files:**
- Create: `krillnotes-core/src/core/scripting/display_helpers.rs`

This module contains pure Rust functions that build HTML strings using `kn-view-*` CSS classes. All user-supplied content must be passed through `html_escape()`.

```rust
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}
```

Implement all helpers. Key signatures (Rhai Array = `rhai::Array`, Rhai Map = `rhai::Map`):

```rust
pub fn table(headers: rhai::Array, rows: rhai::Array) -> String { ... }
pub fn section(title: String, content: String) -> String { ... }
pub fn stack(items: rhai::Array) -> String { ... }
pub fn columns(items: rhai::Array) -> String { ... }
pub fn field_row(label: String, value: String) -> String { ... }
pub fn fields(note: rhai::Map) -> String { ... }  // humanises key names, renders all fields entries
pub fn heading(text: String) -> String { ... }
pub fn view_text(content: String) -> String { ... }  // exposed as "text" in Rhai
pub fn list(items: rhai::Array) -> String { ... }
pub fn badge(text: String) -> String { ... }
pub fn badge_colored(text: String, color: String) -> String { ... }
pub fn divider() -> String { ... }
```

`fields(note)` implementation: iterate `note["fields"]` map, skip empty string values, humanise keys (`"first_name"` → `"First Name"` by replacing `_` with space and title-casing), render each as `field_row`.

`badge_colored` color mapping (must use explicit strings so CSS is never generated dynamically):
- "red" → `kn-view-badge-red`
- "green" → `kn-view-badge-green`
- "blue" → `kn-view-badge-blue`
- "yellow" → `kn-view-badge-yellow`
- "gray" → `kn-view-badge-gray`
- "orange" → `kn-view-badge-orange`
- "purple" → `kn-view-badge-purple`
- any other → `kn-view-badge` (neutral)

---

### Task 3: Extend `ScriptRegistry` with QueryContext and new registrations

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**3a — Add `display_helpers` module declaration at top of file:**
```rust
mod display_helpers;
```

**3b — Add `QueryContext` struct and field to `ScriptRegistry`:**
```rust
pub struct QueryContext {
    pub notes_by_id:    HashMap<String, rhai::Dynamic>,
    pub children_by_id: HashMap<String, Vec<rhai::Dynamic>>,
    pub notes_by_type:  HashMap<String, Vec<rhai::Dynamic>>,
}

pub struct ScriptRegistry {
    engine: Engine,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
    schema_registry: schema::SchemaRegistry,
    hook_registry: HookRegistry,
    query_context: Arc<Mutex<Option<QueryContext>>>,  // NEW
}
```

**3c — In `new()`, after existing registrations, add:**

Register `on_view()` host function (same pattern as `on_save()`):
```rust
let view_hooks_arc = hook_registry.on_view_hooks_arc();
let ast_arc2 = Arc::clone(&current_loading_ast);
engine.register_fn("on_view", move |name: String, fn_ptr: FnPtr| -> ... {
    let maybe_ast = ast_arc2.lock().unwrap().clone();
    let ast = maybe_ast.ok_or_else(|| -> Box<EvalAltResult> {
        "on_view called outside of load_script".to_string().into()
    })?;
    view_hooks_arc.lock().unwrap().insert(name, HookEntry { fn_ptr, ast });
    Ok(Dynamic::UNIT)
});
```

Register query functions (close over `Arc<Mutex<Option<QueryContext>>>`):
```rust
let qc = Arc::clone(&query_context);
engine.register_fn("get_children", move |id: String| -> rhai::Array {
    let guard = qc.lock().unwrap();
    guard.as_ref()
        .and_then(|ctx| ctx.children_by_id.get(&id).cloned())
        .unwrap_or_default()
});

let qc2 = Arc::clone(&query_context);
engine.register_fn("get_note", move |id: String| -> Dynamic {
    let guard = qc2.lock().unwrap();
    guard.as_ref()
        .and_then(|ctx| ctx.notes_by_id.get(&id).cloned())
        .unwrap_or(Dynamic::UNIT)
});

let qc3 = Arc::clone(&query_context);
engine.register_fn("get_notes_of_type", move |node_type: String| -> rhai::Array {
    let guard = qc3.lock().unwrap();
    guard.as_ref()
        .and_then(|ctx| ctx.notes_by_type.get(&node_type).cloned())
        .unwrap_or_default()
});
```

Register display helpers:
```rust
engine.register_fn("table",   display_helpers::table);
engine.register_fn("section", display_helpers::section);
engine.register_fn("stack",   display_helpers::stack);
engine.register_fn("columns", display_helpers::columns);
engine.register_fn("field",   display_helpers::field_row);
engine.register_fn("fields",  display_helpers::fields);
engine.register_fn("heading", display_helpers::heading);
engine.register_fn("text",    display_helpers::view_text);
engine.register_fn("list",    display_helpers::list);
engine.register_fn("badge",   display_helpers::badge);
engine.register_fn("badge",   display_helpers::badge_colored);
engine.register_fn("divider", display_helpers::divider);
```

Update `ScriptRegistry::new()` return to include `query_context: Arc::new(Mutex::new(None))`.

**3d — Add new public methods:**
```rust
pub fn has_view_hook(&self, schema_name: &str) -> bool {
    self.hook_registry.has_view_hook(schema_name)
}

pub fn run_on_view_hook(
    &self,
    note: &Note,
    context: QueryContext,
) -> Result<Option<String>> {
    // 1. Build note_map (same as on_save: id, node_type, title, fields)
    // 2. Store context in self.query_context
    // 3. Call self.hook_registry.run_on_view_hook(&self.engine, note_map)
    // 4. Clear self.query_context
    // 5. Return result
}
```

Also update `clear_all()` to clear `query_context` (set to `None`).

---

### Task 4: Add `run_view_hook` to `Workspace`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

Add a helper function (alongside existing `note_to_rhai_map` helpers) to convert a `Note` + all notes into a `QueryContext`. Then add the public method:

```rust
pub fn run_view_hook(&self, note_id: &str) -> Result<Option<String>> {
    let note = self.get_note(note_id)?;
    let all_notes = self.list_all_notes()?;

    // Build QueryContext
    let mut notes_by_id = HashMap::new();
    let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
    let mut notes_by_type: HashMap<String, Vec<Dynamic>> = HashMap::new();

    for n in &all_notes {
        let map = note_to_rhai_map(n);  // same conversion as used in on_save
        let dyn_map = Dynamic::from(map.clone());
        notes_by_id.insert(n.id.clone(), dyn_map.clone());
        if let Some(pid) = &n.parent_id {
            children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
        }
        notes_by_type.entry(n.node_type.clone()).or_default().push(dyn_map);
    }

    let context = QueryContext { notes_by_id, children_by_id, notes_by_type };
    self.script_registry.run_on_view_hook(&note, context)
}
```

Note: `note_to_rhai_map` may need to be factored out of `run_on_save_hook` if not already a standalone function. Check hooks.rs and extract if needed.

---

### Task 5: Update `lib.rs` — new Tauri command + `SchemaInfo`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**5a — Add `has_view_hook` to `SchemaInfo`:**
```rust
struct SchemaInfo {
    fields: Vec<FieldDefinition>,
    title_can_view: bool,
    title_can_edit: bool,
    children_sort: String,
    allowed_parent_types: Vec<String>,
    allowed_children_types: Vec<String>,
    has_view_hook: bool,  // NEW
}
```

Update both `get_schema_fields` and `get_all_schemas` to populate it:
```rust
has_view_hook: workspace.script_registry().has_view_hook(&node_type),
```

**5b — Add `get_note_view` command:**
```rust
#[tauri::command]
fn get_note_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Option<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_view_hook(&note_id).map_err(|e| e.to_string())
}
```

**5c — Register in `invoke_handler`:**
Add `get_note_view` to the handler list (after `get_all_schemas`).

---

### Task 6: Install DOMPurify

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/on-view-hook/krillnotes-desktop
npm install dompurify @types/dompurify
```

---

### Task 7: Update `types.ts`

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

Add to `SchemaInfo`:
```typescript
hasViewHook: boolean;
```

---

### Task 8: Add `kn-view-*` CSS classes to `globals.css`

**Files:**
- Modify: `krillnotes-desktop/src/styles/globals.css`

Add a clearly-marked section after existing styles:

```css
/* ── View hook display helpers (kn-view-*) ──────────────────────────────── */
.kn-view-table           { width: 100%; border-collapse: collapse; font-size: 0.875rem; }
.kn-view-th              { text-align: left; color: var(--color-muted-foreground); font-weight: 500;
                           padding: 0 1rem 0.5rem 0; border-bottom: 1px solid var(--color-border); }
.kn-view-td              { padding: 0.35rem 1rem 0.35rem 0; vertical-align: top; }
.kn-view-tr:hover .kn-view-td { background-color: var(--color-secondary); }
.kn-view-section         { margin-bottom: 1.25rem; }
.kn-view-section-title   { font-size: 0.7rem; font-weight: 600; text-transform: uppercase;
                           letter-spacing: 0.06em; color: var(--color-muted-foreground); margin-bottom: 0.5rem; }
.kn-view-stack           { display: flex; flex-direction: column; gap: 0.75rem; }
.kn-view-columns         { display: grid; gap: 1rem; }
.kn-view-field-row       { display: grid; grid-template-columns: auto 1fr; gap: 0 1.5rem; margin-bottom: 0.15rem; }
.kn-view-field-label     { font-size: 0.875rem; font-weight: 500; color: var(--color-muted-foreground);
                           white-space: nowrap; }
.kn-view-field-value     { font-size: 0.875rem; color: var(--color-foreground); }
.kn-view-heading         { font-size: 1.1rem; font-weight: 600; margin-bottom: 0.4rem; }
.kn-view-text            { white-space: pre-wrap; word-break: break-words; font-size: 0.875rem; }
.kn-view-list            { list-style-type: disc; padding-left: 1.25rem; font-size: 0.875rem; }
.kn-view-list li         { margin-bottom: 0.2rem; }
.kn-view-divider         { border: none; border-top: 1px solid var(--color-border); margin: 1rem 0; }
.kn-view-badge           { display: inline-flex; align-items: center; padding: 0.1rem 0.5rem;
                           border-radius: 9999px; font-size: 0.7rem; font-weight: 500;
                           background-color: var(--color-secondary); color: var(--color-secondary-foreground); }
.kn-view-badge-red       { background-color: #fee2e2; color: #b91c1c; }
.kn-view-badge-green     { background-color: #dcfce7; color: #15803d; }
.kn-view-badge-blue      { background-color: #dbeafe; color: #1d4ed8; }
.kn-view-badge-yellow    { background-color: #fef9c3; color: #a16207; }
.kn-view-badge-gray      { background-color: #f3f4f6; color: #374151; }
.kn-view-badge-orange    { background-color: #ffedd5; color: #c2410c; }
.kn-view-badge-purple    { background-color: #f3e8ff; color: #7e22ce; }
```

---

### Task 9: Update `InfoPanel.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**9a — Import DOMPurify at top:**
```typescript
import DOMPurify from 'dompurify';
```

**9b — Add state:**
```typescript
const [customViewHtml, setCustomViewHtml] = useState<string | null>(null);
```

**9c — In the `useEffect` that fetches schema, after `setSchemaInfo(info)`:**
```typescript
if (info.hasViewHook) {
    invoke<string | null>('get_note_view', { noteId: selectedNote.id })
        .then(html => setCustomViewHtml(html ?? null))
        .catch(() => setCustomViewHtml(null));
} else {
    setCustomViewHtml(null);
}
```

Also reset `customViewHtml` to `null` when entering edit mode (in the `setIsEditing(true)` call or `useEffect` on `requestEditMode`).

**9d — In view-mode JSX, wrap the field rendering:**
```tsx
{!isEditing && (
    customViewHtml ? (
        <div
            dangerouslySetInnerHTML={{
                __html: DOMPurify.sanitize(customViewHtml)
            }}
        />
    ) : (
        /* existing <dl> grid field rendering */
    )
)}
```

---

### Task 10: Add `on_view` hook to `01_contact.rhai`

**Files:**
- Modify: `krillnotes-core/src/system_scripts/01_contact.rhai`

Append after the existing `on_save` hook:

```rhai
on_view("ContactsFolder", |note| {
    let contacts = get_children(note.id);
    if contacts.len() == 0 {
        return text("No contacts yet.");
    }
    let rows = contacts.map(|c| [
        c.title,
        c.fields.email  ?? "-",
        c.fields.phone  ?? "-",
        c.fields.mobile ?? "-"
    ]);
    section(
        "Contacts (" + contacts.len() + ")",
        table(["Name", "Email", "Phone", "Mobile"], rows)
    )
});
```

---

### Verification

1. `cd .worktrees/feat/on-view-hook && cargo test -p krillnotes-core` — all tests pass
2. `cd .worktrees/feat/on-view-hook/krillnotes-desktop && npm run build` — TypeScript and CSS compile cleanly
3. `cargo tauri dev` from worktree — open a workspace with a ContactsFolder containing at least 2 contacts
4. Select the ContactsFolder → view panel shows the custom table instead of the default field list
5. Select a Contact child → view panel shows default field rendering (fallback path)
6. Add/edit a contact, return to ContactsFolder → table reflects latest data
7. Select any TextNote → default rendering unchanged
8. Open Script Management, disable the ContactsFolder's script, reload → view falls back to default
