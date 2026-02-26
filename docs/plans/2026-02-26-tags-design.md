# Tags Feature — Design Document

**Date:** 2026-02-26
**Issue:** #13 — Add tags to notes
**Status:** Approved

---

## Summary

Tags are a first-class note attribute — like `title`, they are always available on every note regardless of node type and require no schema definition. A note carries zero or more string tags. Tags are global across the workspace: every unique tag used in any note forms the workspace's tag vocabulary.

The feature touches every layer of the stack: database, Rust core, Tauri commands, frontend types, note editor, default view, scripting API, tag cloud panel, and export.

---

## Architecture Decisions

### Storage: separate `note_tags` junction table

Tags live in a dedicated junction table rather than in `fields_json` or a new JSON column. This keeps SQL queries clean, makes `get_notes_for_tag` a simple join with an index scan, and gives cascade deletes for free when a note is removed.

### Global tag list: derived, not stored

The workspace tag vocabulary is always computed as `SELECT DISTINCT tag FROM note_tags ORDER BY tag`. No separate metadata row is needed; the list stays consistent automatically.

### Tag colors: deterministic HSL hash

Pill hue = `(sum of UTF-8 char codes of the tag) % 360`. Rendered as `hsl(hue, 40%, 88%)` with dark text. Same tag always gets the same color — intentional, like a label system.

### Tag cloud collapse: drag-to-zero

The tag cloud panel uses the same horizontal drag handle pattern as the existing tree/content divider. Dragging to 0px collapses it. No toggle button needed.

### Search integration: tags concatenated into filter string

The backend appends tag strings to the title when applying the tree search filter, so the existing search bar picks up tags transparently. Tag cloud pill clicks set the search bar text to the tag name, triggering the same path.

---

## Database

### New table

```sql
CREATE TABLE IF NOT EXISTS note_tags (
  note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  tag     TEXT NOT NULL,
  PRIMARY KEY (note_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_note_tags_tag ON note_tags(tag);
```

### Migration

Added to `run_migrations()` in `krillnotes-core/src/core/storage.rs` alongside the existing `is_expanded` and `user_scripts` migrations. The migration checks for table absence before creating.

No changes to the `notes` table.

---

## Rust Core

### `Note` struct (`krillnotes-core/src/core/note.rs`)

```rust
pub struct Note {
    // ... existing fields unchanged ...
    pub tags: Vec<String>,  // new — empty vec for notes with no tags
}
```

Serializes as `"tags"` (camelCase via `serde(rename_all = "camelCase")`).

### Query changes (`workspace.rs`)

Every function that constructs a `Note` from a row — `get_note`, `list_all_notes`, `get_children`, `get_notes_for_tag` — must join `note_tags` and aggregate tags. Pattern:

```sql
SELECT n.*, GROUP_CONCAT(nt.tag, ',') AS tags_csv
FROM notes n
LEFT JOIN note_tags nt ON nt.note_id = n.id
WHERE ...
GROUP BY n.id
ORDER BY n.position
```

The `tags_csv` column is split on `,` and sorted to produce `Vec<String>`.

### New workspace methods

| Method | Signature | Behaviour |
|--------|-----------|-----------|
| `update_note_tags` | `(note_id: &str, tags: Vec<String>) -> Result<()>` | Deletes all rows for `note_id`, re-inserts the new set in a transaction. Normalises tags (lowercase, trim, deduplicate) before insert. |
| `get_all_tags` | `() -> Result<Vec<String>>` | `SELECT DISTINCT tag FROM note_tags ORDER BY tag`. |
| `get_notes_for_tag` | `(tags: Vec<String>) -> Result<Vec<Note>>` | Returns notes having ANY of the given tags (OR logic). Uses the same join pattern as `list_all_notes`. |

### Search filter

`list_all_notes` (or its search variant) concatenates `note.title + " " + note.tags.join(" ")` before applying the case-insensitive filter string. Tags become invisible to the user but searchable.

---

## Tauri Commands

Three new commands in `krillnotes-desktop/src-tauri/src/`:

| Command | Payload | Return |
|---------|---------|--------|
| `update_note_tags` | `{ note_id: String, tags: Vec<String> }` | `()` |
| `get_all_tags` | — | `Vec<String>` |
| `get_notes_for_tag` | `{ tags: Vec<String> }` | `Vec<Note>` |

---

## Frontend Types (`types.ts`)

```ts
export interface Note {
  // all existing fields unchanged
  tags: string[];   // new — always present, empty array when no tags
}
```

---

## Note Editor

### Placement

Tag editor appears between the title input and the schema fields in edit mode inside `InfoPanel.tsx`.

### Behaviour

- Existing tags render as pills. Each pill has an `×` button that removes the tag immediately from local state (saved on note save).
- A text input below the pills filters the workspace tag list (fetched once via `get_all_tags` when entering edit mode). Matching suggestions appear as a dropdown.
- `Tab` or clicking a suggestion selects the highlighted tag and adds it.
- `Enter` on unmatched text adds the typed string as a new tag (lowercased, trimmed).
- Tags are deduplicated in the UI: adding a duplicate is a no-op.
- On save, `update_note_tags` is called alongside the existing field update.

---

## Default Note View

In view mode, tags render above the schema fields as coloured pills. The colour is computed client-side with the same deterministic HSL formula:

```ts
function tagColor(tag: string): string {
  const hue = [...tag].reduce((acc, c) => acc + c.charCodeAt(0), 0) % 360;
  return `hsl(${hue}, 40%, 88%)`;
}
```

Pills are non-interactive in view mode (no click action — clicking a tag in the tree's tag cloud is the navigation entry point).

---

## Scripting API

### `render_tags(tags)` — display helper (`display_helpers.rs`)

Takes a Rhai `Array` of strings. Returns:

```html
<div class="kn-view-tags">
  <span class="kn-tag-pill" style="background:hsl(N,40%,88%)">tag-name</span>
  ...
</div>
```

Color computed server-side with the same HSL formula. Tags are HTML-escaped. Used in custom `on_view` hooks.

### `get_notes_for_tag(tags)` — query function (`scripting/mod.rs`)

Available inside `on_view` hook closures. Takes a Rhai Array of strings, calls `workspace.get_notes_for_tag(...)`, returns an array of note Dynamic maps (same shape as `get_note`). OR semantics: returns notes with any of the supplied tags.

```rhai
// Example usage in an on_view hook:
let related = get_notes_for_tag(["reading", "todo"]);
```

---

## Tag Cloud Panel (`WorkspaceView.tsx`)

### Layout

The tree column is split vertically: tree on top, tag cloud at bottom. A horizontal drag handle sits between them. The split uses the same resize-by-drag pattern as the existing left/right column divider.

State:
```ts
const [tagCloudHeight, setTagCloudHeight] = useState(120); // px
```

Dragging the handle adjusts `tagCloudHeight` (clamped: min `0`, max `400`). At `0`, the panel renders with `height: 0; overflow: hidden` — effectively hidden.

### Content

- Fetches `get_all_tags()` on mount and after every note save.
- Renders each tag as a pill with the deterministic HSL colour.
- Clicking a tag sets the search bar text to that tag name, which triggers the existing tree filter (tags participate in search, so matching notes surface immediately).

---

## Export (`export.rs`)

`export_workspace` writes a `workspace.json` file into the zip:

```json
{
  "version": 1,
  "tags": ["design", "reading", "todo"]
}
```

- `tags`: complete sorted list of distinct tags from `get_all_tags()`.
- The `notes.json` notes array already includes each note's `tags` field (since `Note` serialises `tags`), so tags are fully preserved on import without reading `workspace.json`.
- `workspace.json` is the canonical home for future workspace-level settings (as noted in the issue).

Import (`import_workspace`) reads each note's `tags` from `notes.json` and re-inserts the `note_tags` rows during the bulk insert transaction. `workspace.json` is not read during import in this phase.

---

## Out of Scope (this phase)

- Tag renaming or merging across the workspace
- Tag ordering (tags are stored and displayed sorted alphabetically)
- Import reading `workspace.json` (redundant since note tags come from `notes.json`)
- User-assigned tag colors
