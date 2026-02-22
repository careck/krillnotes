# On-View Hook — Design

## Summary

Add an `on_view` hook to the Rhai scripting system that is called when a note is displayed in the view panel. The hook returns an HTML string built from safe display helper functions. This allows schema authors to create fully custom view layouts — for example, showing a ContactsFolder as a sortable table of child contacts rather than a simple field list.

## Motivation

Currently all notes render using the same generic field-list layout in `InfoPanel.tsx`. There is no way for a schema to customise how a note looks when viewed. The `on_save` hook already allows scripts to transform data at save time; `on_view` is the natural counterpart for display time. A comment in `02_task.rhai` already anticipated this: *"real-time urgency labels would require an on_view() hook, which is not yet implemented."*

## Agreed Design Decisions

### 1. Return format — Helper Function DSL
The hook returns a string built by composable Rust helper functions registered as Rhai host functions (`table`, `section`, `badge`, etc.). Each helper returns an HTML string so they can be nested freely. This is safer than raw HTML string building and simpler than a structured component tree.

### 2. Data access — note object + top-level query functions
The hook receives the note as its only parameter. Querying other notes (children, etc.) is done via top-level functions available to all scripts — the same pattern as `schema()` and `on_save()`:
- `get_children(note_id)` — direct children of a note
- `get_note(id)` — any note by ID
- `get_notes_of_type(schema_name)` — all notes of a given type

Query functions are backed by a `QueryContext` (pre-built index of all notes) that is populated by the workspace before calling the hook and cleared afterwards. This avoids unsafe DB access inside Rhai.

### 3. Invocation — backend Rhai, called on display
A new Tauri command `get_note_view(noteId)` is called by `InfoPanel` when a note is selected. It returns `Option<String>` (null if no hook is registered). The `SchemaInfo` response is extended with `has_view_hook: bool` so the frontend only calls `get_note_view` when needed.

### 4. Fallback
If no `on_view` hook is registered for a schema, the existing default field rendering is used unchanged.

### 5. Styling
A set of prefixed CSS classes (`kn-view-*`) are added to `globals.css` and used exclusively in the generated HTML. These reference existing CSS custom properties from the app theme, giving the custom view consistent styling in both light and dark modes.

### 6. HTML sanitisation
The returned HTML string is sanitised with DOMPurify before being rendered via `dangerouslySetInnerHTML` in React.

### 7. Note links (deferred)
`note_link(note)` as a clickable navigation helper is deferred to a future task that will also implement navigation history / back button. Without a back button, note links would strand the user.

## Display Helper Functions

All helpers are pure Rust functions registered on the Rhai engine. User-supplied data is HTML-escaped before insertion.

| Helper | Rhai signature | Purpose |
|--------|---------------|---------|
| `table(headers, rows)` | `([str], [[str]]) → str` | Table with thead/tbody |
| `section(title, content)` | `(str, str) → str` | Titled container |
| `stack(items)` | `([str]) → str` | Vertical flex stack |
| `columns(items)` | `([str]) → str` | Equal-width grid columns |
| `field(label, value)` | `(str, str) → str` | Single key-value row |
| `fields(note)` | `(note) → str` | All note fields as field rows |
| `heading(text)` | `(str) → str` | Section heading |
| `text(content)` | `(str) → str` | Whitespace-preserving paragraph |
| `list(items)` | `([str]) → str` | Bullet list |
| `badge(text)` | `(str) → str` | Neutral pill badge |
| `badge(text, color)` | `(str, str) → str` | Colored badge (red/green/blue/yellow/gray/orange/purple) |
| `divider()` | `() → str` | Horizontal rule |

## Example — ContactsFolder

```rhai
on_view("ContactsFolder", |note| {
    let contacts = get_children(note.id);
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

## Architecture Overview

```
on_view hook registered in Rhai script
    ↓
InfoPanel.tsx: note selected → invoke get_note_view(noteId)
    ↓
Tauri command → Workspace::run_view_hook(note_id)
    ↓
list_all_notes() → build QueryContext (indexed by id / parent / type)
    ↓
ScriptRegistry::run_on_view_hook(note_map, context)
    ↓
QueryContext stored in Arc<Mutex<>> for duration of hook call
    ↓
HookRegistry::run_on_view_hook(engine, note_map) → String
    ↓
Return HTML string → DOMPurify.sanitize() → dangerouslySetInnerHTML
```
