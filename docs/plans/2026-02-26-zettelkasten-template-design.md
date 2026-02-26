# Zettelkasten Template — Design Document

**Date:** 2026-02-26
**Status:** Approved
**Scope:** Small core fix to expose tags in `on_view` hooks; self-contained Zettelkasten template with auto-titled Zettel notes, related-notes view, and a Kasten folder overview; website gallery page.

---

## Overview

The Zettelkasten template is the second entry in the Krillnotes template gallery. It demonstrates `on_save` auto-titling, `on_view` with cross-note tag queries, and date-based tree sort actions. A small one-line core fix is required to expose the current note's tags inside `on_view` hooks.

---

## Core Fix

**File:** `krillnotes-core/src/core/scripting/mod.rs` — `run_on_view_hook`

Add `tags` to the note_map built before calling the view hook, mirroring what `note_to_rhai_dynamic` already does for QueryContext notes:

```rust
let tags_array: rhai::Array = note.tags.iter()
    .map(|t| Dynamic::from(t.clone()))
    .collect();
note_map.insert("tags".into(), Dynamic::from(tags_array));
```

This makes `note.tags` readable from any `on_view` Rhai hook.

---

## Repository Changes

### Krillnotes repo
- `krillnotes-core/src/core/scripting/mod.rs` — expose tags in on_view note_map
- `templates/zettelkasten.rhai` — new template script

### Website repo (`krillnotes-website`)
- `content/templates/zettelkasten.md` — gallery page
- `static/templates/zettelkasten.rhai` — download copy of the script
- `static/templates/zettelkasten.krillnotes.zip` — sample workspace with ~5 pre-populated Zettel notes

---

## Script: `templates/zettelkasten.rhai`

### Zettel schema

| Field | Type | Notes |
|---|---|---|
| `body` | `textarea` | The note content |

`title_can_edit: false`, `allowed_parent_types: ["Kasten"]`

**`on_save`:** Title = `YYYY-MM-DD — ` + first 6 words of body (appends `…` if body has more). Falls back to `YYYY-MM-DD — Untitled` when body is empty. Date is taken from `today()`.

**`on_view`:** Renders the body as a text block, then a **Related Notes** section. Calls `get_notes_for_tag(note.tags)`, filters out the note itself, and presents a table of title + tags for each related note. Section is omitted when no tags are set or no related notes exist.

### Kasten schema

No fields. `allowed_children_types: ["Zettel"]`

**`on_view`:** Stats line (`N Zettel · K unique tags`), then a table of the 10 most recent Zettel notes (title + tags columns). "Most recent" = top 10 after sorting children by title descending (ISO date prefix makes this correct automatically).

### Tree actions on Kasten

| Action | Sort key | Order |
|---|---|---|
| Sort by Date (Newest First) | title | Descending |
| Sort by Date (Oldest First) | title | Ascending |

---

## Website Gallery Page

`content/templates/zettelkasten.md` covers:

1. Screenshot of the Kasten `on_view`
2. Download links — `.rhai` script and `.krillnotes` sample workspace
3. User guide — create a Kasten note, add Zettel children, use native tags, sort actions
4. Script walkthrough — explains `on_save` auto-titling, `on_view` related-notes query, and how the date prefix makes sort work without a date field

---

## Out of Scope

- Markdown rendering in body (future enhancement)
- Cross-Zettel link creation from the UI
- Automatic tag suggestions
