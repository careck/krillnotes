# Text Export Views — Design Spec

**Date:** 2026-04-29
**Status:** Draft

## Overview

Script-driven export of notes to CSV, JSON, and Markdown. Export views are registered per-schema in Rhai scripts (alongside existing `register_view` hooks), anchored on container/folder notes. The script callback drives traversal — iterating children, following links, filtering by tag — using the same query context available to `register_view` closures.

## Goals

- Let script authors define how notes of a schema type export to each format
- Reuse existing patterns: deferred bindings, `QueryContext`, `build_note_map()`
- Preview before save — user sees output before committing to file or clipboard
- Ship with book-collection example exports as documentation-by-example

## Non-Goals

- Binary exports (PDF, XLSX) — text formats only for v1
- Replacing the existing ZIP workspace export (`export.rs`) — that stays for backup/restore
- Multi-file export — each export produces a single file

---

## Script API

### Three Registration Functions

Each follows the `register_view` pattern: called during script load, captured as a deferred binding, resolved after all scripts are loaded.

```rhai
// CSV — returns Array of Maps (flat, uniform keys become column headers)
register_csv_export("BookCollection", "Book List", |note| {
    let books = get_children(note.id);
    let rows = [];
    for book in books {
        rows.push(#{
            title: book.title,
            author: book.fields["author"] ?? "",
            rating: book.fields["rating"] ?? ""
        });
    }
    rows
});

// JSON — returns Array of Maps (can be nested/deep)
register_json_export("BookCollection", "Book List", |note| {
    let books = get_children(note.id);
    let items = [];
    for book in books {
        items.push(#{
            title: book.title,
            author: book.fields["author"] ?? "",
            tags: book.tags,
            metadata: #{ created: book.created_at, schema: book.schema }
        });
    }
    items
});

// Markdown — returns a String
register_markdown_export("BookCollection", "Book List", |note| {
    let books = get_children(note.id);
    let md = "# " + note.title + "\n\n";
    for book in books {
        md += "## " + book.title + "\n";
        md += "- **Author:** " + (book.fields["author"] ?? "Unknown") + "\n\n";
    }
    md
});
```

### Callback Contract

| Format   | Receives        | Returns             | System Assembly                                      |
|----------|-----------------|---------------------|------------------------------------------------------|
| CSV      | `note` (map)    | `Array<Map>`        | Union all map keys → header row. Each map → data row. Values must be scalars (nested maps get JSON-stringified as fallback). |
| JSON     | `note` (map)    | `Array<Map>`        | Serialize to pretty-printed JSON array via serde_json |
| Markdown | `note` (map)    | `String`            | Pass through as-is                                   |

All callbacks have full access to the same query and traversal functions available to `register_view` closures: `get_children`, `get_note`, `get_notes_of_type`, `get_notes_for_tag`, `get_notes_with_link`, `get_attachments`, `schema_exists`, `get_schema_fields`.

### Helper: `note.to_map()`

Convenience function that flattens a note into a single-level map for easy CSV/JSON use:

- Includes: `id`, `title`, `schema`, `parent_id`, `position`, `created_at`, `modified_at`, `created_by`, `modified_by`, `is_checked`, `is_expanded`, `schema_version`
- `tags` joined as comma-separated string (unlike the raw note map where `tags` is an Array — `to_map()` flattens it for CSV compatibility)
- All entries from `fields` merged in (keys are field names, values are field values)

---

## Expanded Note Map in Rhai

`build_note_map()` currently exposes only `id`, `schema`, `title`, `fields`, `tags`, `is_checked`. This feature expands it to include **all** Note properties:

| Property         | Type     | Currently exposed | Adding |
|------------------|----------|:-----------------:|:------:|
| `id`             | String   | Yes               |        |
| `schema`         | String   | Yes               |        |
| `title`          | String   | Yes               |        |
| `fields`         | Map      | Yes               |        |
| `tags`           | Array    | Yes               |        |
| `is_checked`     | Bool     | Yes               |        |
| `parent_id`      | String   | No                | Yes    |
| `position`       | Float    | No                | Yes    |
| `created_at`     | String   | No                | Yes    |
| `modified_at`    | String   | No                | Yes    |
| `created_by`     | String   | No                | Yes    |
| `modified_by`    | String   | No                | Yes    |
| `is_expanded`    | Bool     | No                | Yes    |
| `schema_version` | Int      | No                | Yes    |

This benefits both export callbacks and existing `register_view` closures.

---

## Rust Core

### Registration (Deferred Bindings)

Three new `BindingKind` variants: `CsvExport`, `JsonExport`, `MarkdownExport`.

Three new functions registered with the Rhai engine in `engine.rs`:
- `register_csv_export(schema, label, callback)`
- `register_json_export(schema, label, callback)`
- `register_markdown_export(schema, label, callback)`

Each captures a `DeferredBinding` with the appropriate kind, target schema, label, FnPtr, and AST.

### Storage

```rust
pub enum ExportFormat {
    Csv,
    Json,
    Markdown,
}

pub struct ExportRegistration {
    pub label: String,
    pub format: ExportFormat,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}
```

Stored in `SchemaRegistry` as:
```rust
pub export_registrations: HashMap<String, Vec<ExportRegistration>>
//                        schema_name → registrations
```

Resolved in `resolve_deferred_bindings()` alongside existing view and hook bindings. Deduplication: one export per `(schema, label, format)` tuple.

### Execution

New method: `Workspace::run_export(note_id, label, format) -> Result<String, KrillnotesError>`

1. Look up the note, find matching `ExportRegistration` by schema + label + format
2. Build `QueryContext` (same as for views)
3. Call the closure with the note map
4. Post-process by format:
   - **CSV:** Expect `Array<Map>`. Union all keys across all maps for the header row. Write each map as a row (missing keys → empty string). Validate values are scalars; nested maps/arrays get JSON-stringified as fallback.
   - **JSON:** Expect `Array<Map>`. Serialize to pretty-printed JSON via `serde_json::to_string_pretty`.
   - **Markdown:** Expect `String`. Return as-is.
5. Return the assembled content string.

New method: `Workspace::list_exports(note_id) -> Result<Vec<ExportInfo>, KrillnotesError>`

Returns available exports for a note based on its schema. `ExportInfo` contains `{ label, format }`.

### Helper Registration

Register `to_map` as a Rhai method on note maps in `engine.rs`. Flattens the note's core properties + schema fields into a single-level map.

---

## Tauri Commands

Two new commands in `lib.rs`:

```rust
pub async fn list_note_exports(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<ExportInfo>, String>

pub async fn run_note_export(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
    label: String,
    format: String,  // "csv" | "json" | "markdown"
) -> Result<String, String>
```

Both added to `tauri::generate_handler![]`.

---

## Frontend

### Context Menu

In `ContextMenu.tsx`, when right-clicking a note:

1. Call `list_note_exports(note_id)` to get available exports
2. If exports exist, show an **"Export"** submenu with entries like:
   - "Book List (CSV)"
   - "Book List (JSON)"
   - "Book List (Markdown)"
3. If no exports registered for that note's schema, the "Export" submenu does not appear

### Export Preview Dialog

New component: `ExportPreviewDialog.tsx`

**Triggered by:** clicking an export menu item

**Flow:**
1. Call `run_note_export(note_id, label, format)` to get content
2. Open the preview dialog showing:
   - Header: export label + format badge
   - Read-only text area with the content (no syntax highlighting needed — raw text for all formats including Markdown)
   - Two action buttons: **"Save to File"** / **"Copy to Clipboard"**
   - Close button

**Save to File:** Opens native save dialog (`@tauri-apps/plugin-dialog`) with suggested filename `{label}.{ext}` and file type filter. Writes content via `@tauri-apps/plugin-fs`.

**Copy to Clipboard:** Copies content to clipboard, shows success toast.

### i18n

All user-facing strings wired through `t()` across all 7 locales:
- "Export" submenu label
- Format names (CSV, JSON, Markdown)
- Dialog title, button labels
- Success/error toasts

---

## Example: Book Collection

Update `example-scripts/book-collection/book-collection.rhai` with all three export types as documentation-by-example:

```rhai
register_csv_export("BookCollection", "Book List", |note| {
    let books = get_children(note.id);
    let rows = [];
    for book in books {
        rows.push(#{
            title: book.title,
            author: book.fields["author"] ?? "",
            genre: book.fields["genre"] ?? "",
            rating: book.fields["rating"] ?? "",
            read_date: book.fields["read_date"] ?? ""
        });
    }
    rows
});

register_json_export("BookCollection", "Book List", |note| {
    let books = get_children(note.id);
    let items = [];
    for book in books {
        items.push(#{
            title: book.title,
            author: book.fields["author"] ?? "",
            genre: book.fields["genre"] ?? "",
            rating: book.fields["rating"] ?? "",
            tags: book.tags,
            created: book.created_at
        });
    }
    items
});

register_markdown_export("BookCollection", "Book List", |note| {
    let books = get_children(note.id);
    let md = "# " + note.title + "\n\n";
    for book in books {
        let stars = book.fields["rating"] ?? 0;
        md += "## " + book.title + "\n";
        md += "- **Author:** " + (book.fields["author"] ?? "Unknown") + "\n";
        md += "- **Genre:** " + (book.fields["genre"] ?? "") + "\n";
        md += "- **Rating:** " + stars + "/5\n\n";
    }
    md
});
```

The travel-planner example can receive similar treatment in a follow-up.

---

## Testing

### Rust (krillnotes-core)

- Registration: verify deferred bindings resolve for all three export types
- Execution: in-memory workspace with test schema + notes, run each format, assert output
- CSV: verify header union, row ordering, scalar fallback for nested values
- JSON: verify valid JSON output, nested structures preserved
- Markdown: verify string pass-through
- `to_map()`: verify all note fields present, fields merged correctly
- `build_note_map()`: verify new properties (`created_at`, `parent_id`, etc.) are exposed
- Edge cases: empty children, missing fields (defaults to empty), no exports registered

### Frontend

- Context menu: verify "Export" submenu appears only when exports are registered
- Preview dialog: verify content displays, save and copy buttons work
- i18n: verify all new keys present in all 7 locale files
