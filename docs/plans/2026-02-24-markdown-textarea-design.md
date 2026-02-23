# Design: Unified Markdown Rendering for Textarea Fields

**Date:** 2026-02-24
**Status:** Approved

## Summary

Enable markdown rendering for all textarea fields in the default (no-hook) note view. Rendering happens exclusively on the Rust backend via a unified engine, keeping the frontend as a dumb HTML renderer. Rhai scripts continue to receive plain text when accessing field values; they can opt into markdown rendering by calling the new `markdown(text)` helper.

## Goals

- Textarea fields auto-render as CommonMark markdown in the default view
- Rhai scripts accessing `note.fields["field"]` always receive plain text
- New `markdown(text)` Rhai function for explicit markdown rendering in `on_view` hooks
- `fields()` and `field()` helpers remain plain-text (no auto-rendering change)
- Single render engine on the backend — no markdown library in the frontend

## Non-Goals

- Edit mode is unchanged (plain textarea)
- No per-field opt-in/opt-out configuration
- No WYSIWYG or split-pane editor

## Architecture

### Before

```
No on_view hook → InfoPanel.tsx renders FieldDisplay.tsx components (plain text)
on_view hook    → get_note_view → backend runs hook → InfoPanel.tsx renders HTML
```

### After

```
No on_view hook → get_note_view → render_default_view() → InfoPanel.tsx renders HTML
on_view hook    → get_note_view → backend runs hook    → InfoPanel.tsx renders HTML
```

`get_note_view` now always returns `String` (not `Option<String>`). The frontend always renders backend HTML in view mode.

## Backend Changes (`krillnotes-core`)

### New dependency

`pulldown-cmark` — CommonMark spec implementation, standard in the Rust ecosystem.

### New functions

**`render_markdown_to_html(text: &str) -> String`** (`display_helpers.rs`)
- Converts CommonMark markdown to HTML using `pulldown-cmark`
- Output is already structured HTML; caller is responsible for sanitization

**`markdown(text: String) -> String`** (Rhai host function)
- Registers `render_markdown_to_html` as a callable Rhai function
- For use in `on_view` hooks when explicit markdown rendering is desired
- Example: `markdown(note.fields["notes"])`

**`render_default_view(note: &Note, schema: Option<&Schema>) -> String`**
- Generates `kn-view-*` HTML for notes without an `on_view` hook
- Schema fields with `can_view = true`:
  - `textarea` type → rendered via `render_markdown_to_html`, wrapped in `kn-view-markdown`
  - All other types → HTML-escaped plain text, same structure as `field()` helper
- Fields in `note.fields` not present in schema → rendered in a "Legacy Fields" section as plain text
- Returns an empty string (or minimal placeholder) if no visible fields

### Modified: `run_view_hook` / `get_note_view` command

- Return type changes from `Option<String>` to `String`
- If an `on_view` hook is registered for the note type → run it (unchanged)
- If no hook → call `render_default_view` → return the result

## Frontend Changes (`krillnotes-desktop`)

### `InfoPanel.tsx`

1. **Always fetch view HTML** — remove the `if (info.hasViewHook)` gate around the `get_note_view` invocation; fetch unconditionally when a note is selected.
2. **`invoke<string>`** — change from `invoke<string | null>` since the backend always returns HTML.
3. **`handleEdit`** — remove `setCustomViewHtml(null)`; the HTML panel is hidden in edit mode by the existing `!isEditing` condition, so clearing is unnecessary. This also eliminates the need for a re-fetch on cancel.
4. **`handleSave`** — remove the `hasViewHook` gate on the post-save re-fetch; always re-fetch.

### CSS (`index.css` or equivalent)

Add `kn-view-markdown` scoped styles for markdown-rendered content:
- Headings (`h1`–`h4`): font size and weight
- Paragraphs: margin/spacing
- Lists (`ul`, `ol`): bullet/number and indent
- Code blocks (`pre`, `code`): monospace, background
- Blockquotes: left border, muted color
- Strong/em: bold/italic

### `FieldDisplay.tsx`

In view mode, the `!customViewHtml` branch in `InfoPanel.tsx` becomes dead code.
`FieldDisplay.tsx` is **not removed** — per project policy, unused code is flagged for the user to decide.

## Rhai API Addition

```rhai
// New: explicit markdown rendering in an on_view hook
on_view("JournalEntry", |note| {
    stack([
        field("Date", note.fields["date"]),
        markdown(note.fields["body"]),
    ])
});
```

`fields(note)` and `field(label, value)` continue to render plain escaped text.

## Security

- `render_markdown_to_html` output is included in the HTML returned by `get_note_view`
- DOMPurify on the frontend sanitizes all view HTML (existing behavior, unchanged)
- No change to the security model

## `hasViewHook` on `SchemaInfo`

`hasViewHook` remains on `SchemaInfo` as an informational field but is no longer used by the frontend to gate the `get_note_view` fetch. It can be removed in a future cleanup pass.
