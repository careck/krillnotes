# Markdown Textarea Rendering Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Auto-render textarea fields as CommonMark markdown in the default note view, with a `markdown()` Rhai helper for explicit rendering in `on_view` hooks.

**Architecture:** The backend (`krillnotes-core`) always generates view HTML via `get_note_view`; a new `render_default_view()` function replaces the frontend `FieldDisplay.tsx` fallback for view mode. The frontend becomes a pure HTML renderer — it always calls `get_note_view` and always displays the result.

**Tech Stack:** `pulldown-cmark 0.12` (Rust, CommonMark + GFM strikethrough + tables), Rhai host function, React 19 / Tauri v2.

---

## File Map

| File | Change |
|------|--------|
| `krillnotes-core/Cargo.toml` | Add pulldown-cmark |
| `krillnotes-core/src/core/scripting/display_helpers.rs` | Add `render_markdown_to_html`, `rhai_markdown`, `field_row_html`, `format_field_value_html`, `render_default_view` |
| `krillnotes-core/src/core/scripting/mod.rs` | Register `markdown` Rhai fn, add `ScriptRegistry::render_default_view` method |
| `krillnotes-core/src/core/workspace.rs` | Change `run_view_hook` return to `Result<String>`, call `render_default_view` fallback |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Change `get_note_view` return from `Option<String>` to `String` |
| `krillnotes-desktop/src/components/InfoPanel.tsx` | Remove `hasViewHook` gate, always fetch HTML, fix `handleEdit`/`handleSave` |
| `krillnotes-desktop/src/styles/globals.css` | Add `kn-view-markdown` scoped styles |

---

### Task 1: Add `pulldown-cmark` dependency

**Files:**
- Modify: `krillnotes-core/Cargo.toml`

**Step 1: Add the dependency**

In `krillnotes-core/Cargo.toml`, add after the last `[dependencies]` entry:

```toml
pulldown-cmark = { version = "0.12", default-features = false, features = ["html"] }
```

**Step 2: Verify it compiles**

```bash
cargo build -p krillnotes-core
```
Expected: compiles with no errors (may show unused import warnings — that's fine until Task 2).

**Step 3: Commit**

```bash
git add krillnotes-core/Cargo.toml Cargo.lock
git commit -m "chore: add pulldown-cmark dependency for markdown rendering"
```

---

### Task 2: Add `render_markdown_to_html` and tests

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`

**Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the bottom of `display_helpers.rs`:

```rust
#[test]
fn test_render_markdown_bold() {
    let html = render_markdown_to_html("**bold text**");
    assert!(html.contains("<strong>bold text</strong>"));
}

#[test]
fn test_render_markdown_heading() {
    let html = render_markdown_to_html("# My Heading");
    assert!(html.contains("<h1>") && html.contains("My Heading"));
}

#[test]
fn test_render_markdown_list() {
    let html = render_markdown_to_html("- item one\n- item two");
    assert!(html.contains("item one") && html.contains("item two"));
}

#[test]
fn test_render_markdown_plain_text() {
    let html = render_markdown_to_html("just plain text");
    assert!(html.contains("just plain text"));
}

#[test]
fn test_render_markdown_empty() {
    let html = render_markdown_to_html("");
    assert!(html.is_empty() || html == "\n");
}
```

**Step 2: Run tests to confirm they fail**

```bash
cargo test -p krillnotes-core render_markdown -- --nocapture
```
Expected: compile error — `render_markdown_to_html` not yet defined.

**Step 3: Implement `render_markdown_to_html` and `rhai_markdown`**

Add at the top of `display_helpers.rs` after the existing `use rhai::{Array, Map};`:

```rust
use pulldown_cmark::{html as md_html, Options, Parser};
```

Add these two functions right after the `html_escape` function (before the structural helpers section):

```rust
// ── Markdown rendering ────────────────────────────────────────────────────────

/// Converts a CommonMark markdown string to an HTML string.
///
/// Enables strikethrough and tables (GFM extensions). The result is raw HTML —
/// the caller is responsible for XSS sanitisation (DOMPurify handles this on
/// the frontend for all view HTML).
pub fn render_markdown_to_html(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(text, options);
    let mut html_output = String::new();
    md_html::push_html(&mut html_output, parser);
    html_output
}

/// Rhai host function wrapper for `render_markdown_to_html`.
///
/// Registered as `markdown(text)` in the Rhai engine so `on_view` hooks can
/// explicitly render markdown:
///
/// ```rhai
/// on_view("Note", |note| {
///     markdown(note.fields["body"])
/// });
/// ```
pub fn rhai_markdown(text: String) -> String {
    render_markdown_to_html(&text)
}
```

**Step 4: Run tests to confirm they pass**

```bash
cargo test -p krillnotes-core render_markdown -- --nocapture
```
Expected: all 5 tests PASS.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/display_helpers.rs
git commit -m "feat: add render_markdown_to_html and rhai_markdown helper"
```

---

### Task 3: Register `markdown()` as a Rhai host function

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (around line 174, after the `divider`/`link_to` registrations)

**Step 1: Write the failing test**

Add to the test module in `mod.rs` (search for the existing `#[cfg(test)]` block):

```rust
#[test]
fn test_markdown_rhai_function_renders_bold() {
    let mut registry = ScriptRegistry::new().unwrap();
    let script = r#"
        let result = markdown("**hello**");
        result
    "#;
    let result = registry.engine.eval::<String>(script).unwrap();
    assert!(result.contains("<strong>hello</strong>"), "got: {result}");
}
```

**Step 2: Run test to confirm it fails**

```bash
cargo test -p krillnotes-core test_markdown_rhai_function -- --nocapture
```
Expected: FAIL with `Variable not found: markdown`.

**Step 3: Register the function**

In `mod.rs`, in `ScriptRegistry::new()`, add after `engine.register_fn("link_to", display_helpers::link_to);` (around line 174):

```rust
engine.register_fn("markdown", display_helpers::rhai_markdown);
```

**Step 4: Run test to confirm it passes**

```bash
cargo test -p krillnotes-core test_markdown_rhai_function -- --nocapture
```
Expected: PASS.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: register markdown() Rhai host function"
```

---

### Task 4: Add `render_default_view` to `display_helpers.rs`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`

**Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn test_render_default_view_textarea_renders_markdown() {
    use crate::{FieldValue, Note};
    use super::schema::{FieldDefinition, Schema};
    use std::collections::HashMap;

    let mut fields = HashMap::new();
    fields.insert("notes".into(), FieldValue::Text("**bold**".into()));

    let note = Note {
        id: "id1".into(), title: "Test".into(), node_type: "T".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0, fields, is_expanded: false,
    };
    let schema = Schema {
        name: "T".into(),
        fields: vec![FieldDefinition {
            name: "notes".into(), field_type: "textarea".into(),
            required: false, can_view: true, can_edit: true,
            options: vec![], max: 0,
        }],
        title_can_view: true, title_can_edit: true,
        children_sort: "none".into(),
        allowed_parent_types: vec![], allowed_children_types: vec![],
    };

    let html = render_default_view(&note, Some(&schema));
    assert!(html.contains("<strong>bold</strong>"), "expected rendered markdown, got: {html}");
    assert!(html.contains("kn-view-markdown"), "expected markdown wrapper class");
}

#[test]
fn test_render_default_view_text_field_html_escaped() {
    use crate::{FieldValue, Note};
    use super::schema::{FieldDefinition, Schema};
    use std::collections::HashMap;

    let mut fields = HashMap::new();
    fields.insert("name".into(), FieldValue::Text("<script>alert(1)</script>".into()));

    let note = Note {
        id: "id2".into(), title: "T".into(), node_type: "T".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0, fields, is_expanded: false,
    };
    let schema = Schema {
        name: "T".into(),
        fields: vec![FieldDefinition {
            name: "name".into(), field_type: "text".into(),
            required: false, can_view: true, can_edit: true,
            options: vec![], max: 0,
        }],
        title_can_view: true, title_can_edit: true,
        children_sort: "none".into(),
        allowed_parent_types: vec![], allowed_children_types: vec![],
    };

    let html = render_default_view(&note, Some(&schema));
    assert!(!html.contains("<script>"), "raw script tag must not appear");
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn test_render_default_view_skips_can_view_false() {
    use crate::{FieldValue, Note};
    use super::schema::{FieldDefinition, Schema};
    use std::collections::HashMap;

    let mut fields = HashMap::new();
    fields.insert("secret".into(), FieldValue::Text("hidden".into()));

    let note = Note {
        id: "id3".into(), title: "T".into(), node_type: "T".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0, fields, is_expanded: false,
    };
    let schema = Schema {
        name: "T".into(),
        fields: vec![FieldDefinition {
            name: "secret".into(), field_type: "text".into(),
            required: false, can_view: false, can_edit: true,
            options: vec![], max: 0,
        }],
        title_can_view: true, title_can_edit: true,
        children_sort: "none".into(),
        allowed_parent_types: vec![], allowed_children_types: vec![],
    };

    let html = render_default_view(&note, Some(&schema));
    assert!(!html.contains("hidden"), "can_view:false fields must not appear");
}

#[test]
fn test_render_default_view_legacy_fields_shown() {
    use crate::{FieldValue, Note};
    use super::schema::{FieldDefinition, Schema};
    use std::collections::HashMap;

    let mut fields = HashMap::new();
    fields.insert("known".into(), FieldValue::Text("hello".into()));
    fields.insert("old_field".into(), FieldValue::Text("legacy value".into()));

    let note = Note {
        id: "id4".into(), title: "T".into(), node_type: "T".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0, fields, is_expanded: false,
    };
    let schema = Schema {
        name: "T".into(),
        fields: vec![FieldDefinition {
            name: "known".into(), field_type: "text".into(),
            required: false, can_view: true, can_edit: true,
            options: vec![], max: 0,
        }],
        title_can_view: true, title_can_edit: true,
        children_sort: "none".into(),
        allowed_parent_types: vec![], allowed_children_types: vec![],
    };

    let html = render_default_view(&note, Some(&schema));
    assert!(html.contains("legacy value"), "legacy fields must be shown");
    assert!(html.contains("Legacy Fields"), "legacy section header must appear");
}
```

**Step 2: Run tests to confirm they fail**

```bash
cargo test -p krillnotes-core render_default_view -- --nocapture
```
Expected: compile error — `render_default_view` not defined.

**Step 3: Implement the helpers**

Add these new functions to `display_helpers.rs` after the existing `// ── Utilities` section (before the `#[cfg(test)]` block).

First, add to the top-level imports:
```rust
use crate::{FieldValue, Note};
use super::schema::Schema;
```

Then add the functions:

```rust
// ── Default view renderer ────────────────────────────────────────────────────

/// Like `field_row` but accepts pre-rendered HTML for the value (no escaping).
/// Used internally by `render_default_view` where the value may be markdown HTML.
fn field_row_html(label: &str, value_html: &str) -> String {
    format!(
        "<div class=\"kn-view-field-row\">\
           <span class=\"kn-view-field-label\">{}</span>\
           <div class=\"kn-view-field-value\">{}</div>\
         </div>",
        html_escape(label),
        value_html
    )
}

/// Formats a single field value as HTML, choosing between markdown rendering
/// (for `textarea`) and HTML-escaped plain text (for all other types).
fn format_field_value_html(value: &FieldValue, field_type: &str, max: i64) -> String {
    match (value, field_type) {
        (FieldValue::Text(s), "textarea") => {
            format!("<div class=\"kn-view-markdown\">{}</div>", render_markdown_to_html(s))
        }
        (FieldValue::Text(s), _) => {
            format!("<span>{}</span>", html_escape(s))
        }
        (FieldValue::Email(s), _) => {
            format!("<span>{}</span>", html_escape(s))
        }
        (FieldValue::Number(n), "rating") => {
            let rating = *n as i64;
            let max_stars = if max > 0 { max } else { 5 };
            let stars: String = (0..max_stars)
                .map(|i| if i < rating { '★' } else { '☆' })
                .collect();
            format!("<span>{stars}</span>")
        }
        (FieldValue::Number(n), _) => format!("<span>{n}</span>"),
        (FieldValue::Boolean(b), _) => {
            format!("<span>{}</span>", if *b { "Yes" } else { "No" })
        }
        (FieldValue::Date(Some(d)), _) => {
            format!("<span>{}</span>", d.format("%Y-%m-%d"))
        }
        (FieldValue::Date(None), _) => String::new(),
    }
}

/// Returns `true` if the field value is considered empty (and should be hidden).
fn is_field_empty(value: &FieldValue) -> bool {
    match value {
        FieldValue::Text(s) | FieldValue::Email(s) => s.is_empty(),
        FieldValue::Date(d) => d.is_none(),
        FieldValue::Number(_) | FieldValue::Boolean(_) => false,
    }
}

/// Generates a default HTML view for `note` when no `on_view` Rhai hook is
/// registered.
///
/// Schema fields are rendered in schema order, with `textarea` values converted
/// to CommonMark HTML and all other values HTML-escaped.
///
/// Fields present in `note.fields` but absent from the schema are appended in
/// a "Legacy Fields" section.
///
/// Accepts `None` for `schema` — in that case all fields are rendered as plain
/// text in sorted order.
pub fn render_default_view(note: &Note, schema: Option<&Schema>) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(schema) = schema {
        // Render schema-defined fields in declaration order.
        for field_def in &schema.fields {
            if !field_def.can_view {
                continue;
            }
            let Some(value) = note.fields.get(&field_def.name) else { continue };
            if is_field_empty(value) {
                continue;
            }
            let label = humanise_key(&field_def.name);
            let value_html =
                format_field_value_html(value, &field_def.field_type, field_def.max);
            if value_html.is_empty() {
                continue;
            }
            parts.push(field_row_html(&label, &value_html));
        }

        // Render any fields not in the schema as "legacy".
        let schema_names: std::collections::HashSet<&str> =
            schema.fields.iter().map(|f| f.name.as_str()).collect();
        let mut legacy: Vec<(&String, &FieldValue)> = note
            .fields
            .iter()
            .filter(|(k, _)| !schema_names.contains(k.as_str()))
            .collect();
        legacy.sort_by_key(|(k, _)| k.as_str());

        let mut legacy_parts: Vec<String> = Vec::new();
        for (key, value) in &legacy {
            if is_field_empty(value) {
                continue;
            }
            let label = humanise_key(key);
            let value_html = format_field_value_html(value, "text", 0);
            if !value_html.is_empty() {
                legacy_parts.push(field_row_html(&label, &value_html));
            }
        }
        if !legacy_parts.is_empty() {
            parts.push(format!(
                "<div class=\"kn-view-section\" style=\"margin-top:0.75rem\">\
                   <div class=\"kn-view-section-title\">Legacy Fields</div>\
                   {}\
                 </div>",
                legacy_parts.join("")
            ));
        }
    } else {
        // No schema — render all fields as plain text in sorted order.
        let mut all: Vec<(&String, &FieldValue)> = note.fields.iter().collect();
        all.sort_by_key(|(k, _)| k.as_str());
        for (key, value) in &all {
            if is_field_empty(value) {
                continue;
            }
            let label = humanise_key(key);
            let value_html = format_field_value_html(value, "text", 0);
            if !value_html.is_empty() {
                parts.push(field_row_html(&label, &value_html));
            }
        }
    }

    parts.join("")
}
```

**Step 4: Run tests to confirm they pass**

```bash
cargo test -p krillnotes-core render_default_view -- --nocapture
```
Expected: all 4 tests PASS.

**Step 5: Run the full test suite**

```bash
cargo test -p krillnotes-core
```
Expected: all tests PASS.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/display_helpers.rs
git commit -m "feat: add render_default_view for markdown textarea rendering"
```

---

### Task 5: Expose `render_default_view` via `ScriptRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

The `display_helpers` module is private to `scripting`, so `workspace.rs` cannot call `display_helpers::render_default_view` directly. Expose it as a `ScriptRegistry` method.

**Step 1: Write the failing test**

Add to `mod.rs` tests:

```rust
#[test]
fn test_script_registry_render_default_view_textarea_markdown() {
    use crate::{FieldValue, Note};
    use std::collections::HashMap;

    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Memo", #{
            fields: [
                #{ name: "body", type: "textarea", required: false }
            ]
        });
    "#).unwrap();

    let mut fields = HashMap::new();
    fields.insert("body".into(), FieldValue::Text("**important**".into()));
    let note = Note {
        id: "n1".into(), title: "Test".into(), node_type: "Memo".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0, fields, is_expanded: false,
    };

    let html = registry.render_default_view(&note);
    assert!(html.contains("<strong>important</strong>"), "got: {html}");
}
```

**Step 2: Run test to confirm it fails**

```bash
cargo test -p krillnotes-core test_script_registry_render_default_view -- --nocapture
```
Expected: compile error — method not found.

**Step 3: Add the method**

In `mod.rs`, add to the `impl ScriptRegistry` block (after the `has_view_hook` method, around line 282):

```rust
/// Renders a default HTML view for `note` using schema field type information.
///
/// Used when no `on_view` hook is registered for the note's type — the result
/// is sent to the frontend instead of falling back to `FieldDisplay.tsx`.
///
/// Textarea fields are rendered as CommonMark HTML; all other fields are
/// HTML-escaped plain text. Fields not in the schema appear in a legacy section.
pub fn render_default_view(&self, note: &Note) -> String {
    let schema = self.schema_registry.get(&note.node_type).ok();
    display_helpers::render_default_view(note, schema.as_ref())
}
```

**Step 4: Run test to confirm it passes**

```bash
cargo test -p krillnotes-core test_script_registry_render_default_view -- --nocapture
```
Expected: PASS.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: expose render_default_view on ScriptRegistry"
```

---

### Task 6: Update `run_view_hook` in `workspace.rs`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (around line 498)

The method currently returns `Result<Option<String>>` and short-circuits with `Ok(None)` when there is no hook. Change the return type to `Result<String>` and fall back to `render_default_view` when no hook is registered.

**Step 1: Write the failing test**

Add to the test module in `workspace.rs` (find the existing `#[cfg(test)]` block or add one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_workspace() -> (Workspace, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let ws = Workspace::create(db_path.to_str().unwrap(), "Test")
            .expect("workspace creation failed");
        (ws, dir)
    }

    #[test]
    fn test_run_view_hook_returns_html_without_hook() {
        let (mut ws, _dir) = make_workspace();

        // Load a schema with a textarea field but no on_view hook.
        ws.load_script(r#"
            schema("Memo", #{
                fields: [
                    #{ name: "body", type: "textarea", required: false }
                ]
            });
        "#).unwrap();

        let note_id = ws.create_note(None, "Memo", "My Memo", Default::default())
            .unwrap().id;
        ws.update_note(&note_id, "My Memo".into(), {
            let mut f = std::collections::HashMap::new();
            f.insert("body".into(), crate::FieldValue::Text("**hello**".into()));
            f
        }).unwrap();

        let html = ws.run_view_hook(&note_id).unwrap();
        assert!(!html.is_empty(), "default view must return non-empty HTML");
        assert!(html.contains("<strong>hello</strong>"),
            "textarea body should be markdown-rendered, got: {html}");
    }
}
```

**Note:** If `workspace.rs` already has a `#[cfg(test)]` block, append the test inside it rather than creating a new one. Check with `grep -n "#\[cfg(test)\]" krillnotes-core/src/core/workspace.rs`.

**Step 2: Run test to confirm it fails**

```bash
cargo test -p krillnotes-core test_run_view_hook_returns_html -- --nocapture
```
Expected: compile error on return type mismatch, or test failure because `Ok(None)` is still returned.

**Step 3: Update `run_view_hook`**

In `workspace.rs`, change the `run_view_hook` method (currently lines 498–526):

```rust
/// Runs the `on_view` hook for the note's schema, falling back to a default
/// HTML view when no hook is registered.
///
/// The default view auto-renders `textarea` fields as CommonMark markdown.
///
/// # Errors
///
/// Returns [`KrillnotesError::Database`] if the note or any workspace note
/// cannot be fetched, or [`KrillnotesError::Scripting`] if the hook fails.
pub fn run_view_hook(&self, note_id: &str) -> Result<String> {
    let note = self.get_note(note_id)?;

    // No hook registered: generate the default view without fetching all notes.
    if !self.script_registry.has_view_hook(&note.node_type) {
        return Ok(self.script_registry.render_default_view(&note));
    }

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
    // run_on_view_hook now always returns Some(...) since we guarded with has_view_hook.
    Ok(self
        .script_registry
        .run_on_view_hook(&note, context)?
        .unwrap_or_default())
}
```

**Step 4: Run test to confirm it passes**

```bash
cargo test -p krillnotes-core test_run_view_hook_returns_html -- --nocapture
```
Expected: PASS.

**Step 5: Run the full test suite**

```bash
cargo test -p krillnotes-core
```
Expected: all tests PASS. (If existing tests assert `Ok(None)` from `run_view_hook`, update them to expect `Ok(String)` — search for `run_view_hook` in test files.)

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: run_view_hook returns default HTML for notes without on_view hook"
```

---

### Task 7: Update the `get_note_view` Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (around line 469)

**Step 1: Locate the command**

Find the `get_note_view` function at approximately line 469 of `lib.rs`.

**Step 2: Update the return type and doc comment**

Change from:
```rust
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

To:
```rust
/// Returns the HTML view for a note.
///
/// When an `on_view` Rhai hook is registered for the note's schema the hook
/// generates the HTML; otherwise a default view is generated, with `textarea`
/// fields rendered as CommonMark markdown.
///
/// # Errors
///
/// Returns an error string if no workspace is open or if the hook fails.
#[tauri::command]
fn get_note_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_view_hook(&note_id).map_err(|e| e.to_string())
}
```

**Step 3: Build the Tauri backend**

```bash
cargo build -p krillnotes-desktop-lib 2>&1 | head -40
```
(Or run `cargo build` from inside `krillnotes-desktop/src-tauri/`.)

Expected: compiles with no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: get_note_view always returns HTML (removes Option wrapper)"
```

---

### Task 8: Update `InfoPanel.tsx` to always fetch and render backend HTML

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

There are four changes in this file:

**Change A — Always fetch view HTML (remove `hasViewHook` gate)**

Find (around line 94):
```tsx
        // Fetch custom view HTML if the schema has an on_view hook.
        if (info.hasViewHook) {
          invoke<string | null>('get_note_view', { noteId: selectedNote.id })
            .then(html => setCustomViewHtml(html ?? null))
            .catch(() => setCustomViewHtml(null));
        } else {
          setCustomViewHtml(null);
        }
```

Replace with:
```tsx
        // Always fetch the view HTML; the backend generates a default view
        // for notes without an on_view hook (textarea fields render as markdown).
        invoke<string>('get_note_view', { noteId: selectedNote.id })
          .then(html => setCustomViewHtml(html))
          .catch(() => setCustomViewHtml(null));
```

**Change B — `handleEdit`: remove the `setCustomViewHtml(null)` clear**

Find (around line 165):
```tsx
  const handleEdit = () => {
    setCustomViewHtml(null); // clear while editing; will re-fetch on return to view
    setIsEditing(true);
  };
```

Replace with:
```tsx
  const handleEdit = () => {
    // No need to clear customViewHtml — the HTML panel is hidden in edit mode
    // by the !isEditing condition, so the old HTML stays ready for when the
    // user cancels without saving.
    setIsEditing(true);
  };
```

**Change C — `handleSave`: always re-fetch (remove `hasViewHook` gate)**

Find (around line 199):
```tsx
      // Re-fetch custom view HTML after save (on_save may have changed field values).
      if (schemaInfo.hasViewHook) {
        invoke<string | null>('get_note_view', { noteId: selectedNote.id })
          .then(html => setCustomViewHtml(html ?? null))
          .catch(() => setCustomViewHtml(null));
      }
```

Replace with:
```tsx
      // Re-fetch view HTML after save — on_save may have changed field values.
      invoke<string>('get_note_view', { noteId: selectedNote.id })
        .then(html => setCustomViewHtml(html))
        .catch(() => setCustomViewHtml(null));
```

**Step 1: Apply all three changes above.**

**Step 2: Build the frontend**

```bash
cd krillnotes-desktop && npm run build
```
Expected: TypeScript compiles with no errors.

**Step 3: Note about `FieldDisplay.tsx` in view mode**

After this change, the `!customViewHtml` branch at line 331 (the `FieldDisplay.tsx` grid) is only reachable during the brief moment between note selection and the `get_note_view` response arriving (loading state). `FieldDisplay.tsx` is **not removed** — per the project's code removal policy, flag it and let the user decide:

> `FieldDisplay.tsx` is no longer used in steady-state view mode (only as a loading-flash fallback). Consider removing the `dl.grid` block at line 331 and its legacy-field equivalent in a future cleanup, or keeping them as a loading skeleton.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: InfoPanel always fetches backend HTML for view mode"
```

---

### Task 9: Add `kn-view-markdown` CSS styles

**Files:**
- Modify: `krillnotes-desktop/src/styles/globals.css`

**Step 1: Add the styles**

Append to `globals.css` after the `.kn-view-back` section:

```css
/* Markdown-rendered textarea content ────────────────────────────────────── */
/* Scoped under .kn-view-markdown so generated element styles (h1, p, ul…)  */
/* do not bleed into surrounding UI.                                          */

.kn-view-markdown                  { font-size: 0.875rem; line-height: 1.6; }
.kn-view-markdown p                { margin: 0 0 0.5rem 0; }
.kn-view-markdown p:last-child     { margin-bottom: 0; }
.kn-view-markdown h1               { font-size: 1.2rem; font-weight: 700; margin: 0.75rem 0 0.3rem; }
.kn-view-markdown h2               { font-size: 1.05rem; font-weight: 600; margin: 0.65rem 0 0.25rem; }
.kn-view-markdown h3               { font-size: 0.95rem; font-weight: 600; margin: 0.5rem 0 0.2rem; }
.kn-view-markdown h4,
.kn-view-markdown h5,
.kn-view-markdown h6               { font-size: 0.875rem; font-weight: 600; margin: 0.4rem 0 0.15rem; }
.kn-view-markdown ul               { list-style-type: disc; padding-left: 1.25rem; margin: 0.25rem 0; }
.kn-view-markdown ol               { list-style-type: decimal; padding-left: 1.25rem; margin: 0.25rem 0; }
.kn-view-markdown li               { margin-bottom: 0.15rem; }
.kn-view-markdown li > p           { margin: 0; }
.kn-view-markdown pre              { background: var(--color-secondary);
                                     border-radius: var(--radius-sm);
                                     padding: 0.5rem 0.75rem; font-size: 0.8rem;
                                     overflow-x: auto; margin: 0.4rem 0; }
.kn-view-markdown code             { font-family: ui-monospace, SFMono-Regular, monospace;
                                     font-size: 0.85em;
                                     background: var(--color-secondary);
                                     border-radius: var(--radius-sm);
                                     padding: 0.1rem 0.3rem; }
.kn-view-markdown pre code         { background: transparent; padding: 0; font-size: inherit; }
.kn-view-markdown blockquote       { border-left: 3px solid var(--color-border);
                                     margin: 0.4rem 0; padding: 0.1rem 0.75rem;
                                     color: var(--color-muted-foreground); }
.kn-view-markdown strong           { font-weight: 600; }
.kn-view-markdown em               { font-style: italic; }
.kn-view-markdown del              { text-decoration: line-through; }
.kn-view-markdown hr               { border: none;
                                     border-top: 1px solid var(--color-border);
                                     margin: 0.75rem 0; }
.kn-view-markdown a                { color: var(--color-primary); text-decoration: underline; }
.kn-view-markdown table            { width: 100%; border-collapse: collapse;
                                     margin: 0.4rem 0; font-size: 0.875rem; }
.kn-view-markdown th               { text-align: left; font-weight: 500;
                                     color: var(--color-muted-foreground);
                                     padding: 0 0.75rem 0.35rem 0;
                                     border-bottom: 1px solid var(--color-border); }
.kn-view-markdown td               { padding: 0.25rem 0.75rem 0.25rem 0;
                                     vertical-align: top; }
```

**Step 2: Build to confirm CSS compiles**

```bash
cd krillnotes-desktop && npm run build
```
Expected: build succeeds.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/styles/globals.css
git commit -m "feat: add kn-view-markdown CSS styles for rendered textarea content"
```

---

### Task 10: End-to-end smoke test and branch finish

**Step 1: Run the full Rust test suite**

```bash
cargo test -p krillnotes-core
```
Expected: all tests PASS.

**Step 2: Run the Tauri dev server**

```bash
cd krillnotes-desktop && npm run tauri dev
```

**Manual verification checklist:**
1. Open or create a workspace
2. Create a note of any type that has a `textarea` field
3. Edit the note, type `**bold** and *italic*` in the textarea, save
4. Without an `on_view` hook: verify the view panel shows rendered bold/italic text (not raw markdown syntax)
5. In a Rhai script: add `on_view("YourType", |note| { markdown(note.fields["your_field"]) });` — verify it renders markdown
6. In a Rhai script: verify `note.fields["your_field"]` inside `on_save` is still raw text (not HTML)
7. Verify the `fields(note)` helper in an `on_view` hook still renders plain text (not markdown)

**Step 3: Mark TODO item as done**

In `TODO.md`, change:
```
[ ] enable markdown rendering for all textarea fields. The default view should automatically render the value as markdown, however when accessing the value via the API in a rhai script, the value should be returned as plain text. Add a markdown render view command for rhai scripting.
```
to:
```
[x] enable markdown rendering for all textarea fields. The default view should automatically render the value as markdown, however when accessing the value via the API in a rhai script, the value should be returned as plain text. Add a markdown render view command for rhai scripting.
```

**Step 4: Final commit**

```bash
git add TODO.md
git commit -m "chore: mark markdown textarea feature as complete in TODO"
```

**Step 5: Use superpowers:finishing-a-development-branch to integrate the work.**
