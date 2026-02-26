# Note Linking — Design Document

**Date:** 2026-02-26
**Issue:** #14
**Status:** Approved

## Summary

Add a `note_link` field type that lets a note store a reference to another note by ID. Links are one-directional (originator → target). A junction table acts as a derived index for fast reverse lookups and deletion cleanup. The source of truth is always `fields_json`.

## Scope

- New `note_link` field type with optional `target_type` filter
- No query-based filtering of the picker (deferred to a future dynamic filtering feature)

## Schema Definition (Rhai)

```rhai
schema("Task", #{
    fields: [
        #{ name: "linked_project", field_type: "note_link", target_type: "Project" },
        #{ name: "blocked_by",     field_type: "note_link" },  // no type filter = any type
    ],
    ...
})
```

`target_type` is optional. If omitted, the picker searches across all note types.

## Data Layer

### Rust — `FieldValue` enum

New variant added to `FieldValue`:

```rust
NoteLink(Option<String>)  // None = not set, Some(uuid) = linked note ID
```

Serializes to JSON as `null` or `"uuid-string"` — consistent with the `Date` variant pattern.

### Rust — `FieldDefinition`

New optional field:

```rust
pub target_type: Option<String>,  // only meaningful for note_link fields
```

Defaults to `None`, ignored for all other field types.

### DB — `note_links` junction table

New migration:

```sql
CREATE TABLE note_links (
    source_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    field_name TEXT NOT NULL,
    target_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE RESTRICT,
    PRIMARY KEY (source_id, field_name)
);
CREATE INDEX idx_note_links_target ON note_links(target_id);
```

- PK `(source_id, field_name)` enforces one link per field per note
- Index on `target_id` makes reverse lookups fast
- `ON DELETE CASCADE` on `source_id`: if the originator note is deleted, its link rows are cleaned up automatically
- `ON DELETE RESTRICT` on `target_id`: prevents the DB from silently deleting a linked note; the application handles nulling out links explicitly before deletion

### Source of truth

`fields_json` is always the source of truth. The `note_links` table is a derived index — it can always be rebuilt from `fields_json`. This means:

- **Export:** no changes to the export format; `note_links` is not serialized
- **Import:** after all notes are restored from JSON, one rebuild pass scans all notes with `note_link` fields and repopulates the table

## Rhai / Scripting Layer

### New function: `get_notes_with_link(note_id)`

```rhai
// Returns an array of note maps that have any note_link field pointing to note_id
let backlinks = get_notes_with_link(note.id);
```

Returns the same note map format as `get_note()`. Works directly with `link_to()`:

```rhai
on_view("Project", |note| {
    let tasks = get_notes_with_link(note.id);
    if tasks.len() > 0 {
        heading("Linked tasks");
        table(["Task", "Status"], tasks.map(|t| [link_to(t), t.fields["status"] ?? "-"]))
    }
});
```

### Displaying a link field in `on_view`

No new functions needed — the existing pattern works:

```rhai
on_view("Task", |note| {
    let linked_id = note.fields["linked_project"];
    if linked_id != () {
        field("Project", link_to(get_note(linked_id)))
    }
});
```

## Frontend

### Edit mode — `NoteLinkEditor` component

Replaces the field input for `note_link` fields in `FieldEditor.tsx`. Behaviour:

- Shows the current linked note's title, or a placeholder ("Search for a note…") if unset
- Typing triggers a debounced `invoke('search_notes', { query, target_type })` call
- Search covers: title + all text-like fields (text, textarea, email, select) of candidate notes
- Results appear as an inline dropdown beneath the input
- Selecting a result saves the UUID and closes the dropdown
- A ✕ button clears the link (sets to `null`)

New Tauri command: `search_notes(query: String, target_type: Option<String>) → Vec<NoteSearchResult>` where `NoteSearchResult = { id: String, title: String }`.

### View mode — lazy title resolution

`FieldDisplay.tsx` gets a new branch for `{ NoteLink: string | null }`. On render:

- If `null`: display "—"
- If UUID: call `invoke('get_note', { id })`, then render a `kn-view-link` anchor identical to what `link_to()` produces — same navigation behaviour

### Deletion cleanup

Before deleting a note, the workspace (inside the existing delete transaction):

1. Queries `note_links` for all rows where `target_id = deleted_id`
2. For each row: loads the source note, sets that `note_link` field to `null` in `fields_json`, saves it, removes the `note_links` row
3. Proceeds with normal note deletion

Atomic — rolls back if any step fails.

## Summary Table

| Layer | Change |
|---|---|
| Rhai schema | `field_type: "note_link"`, optional `target_type` |
| Rust `FieldValue` | New `NoteLink(Option<String>)` variant |
| Rust `FieldDefinition` | New `target_type: Option<String>` |
| DB | New `note_links` junction table + migration |
| Tauri commands | `search_notes`, `rebuild_note_links_index` (used on import) |
| Rhai engine | `get_notes_with_link(id)` |
| Frontend edit | `NoteLinkEditor` with inline search dropdown |
| Frontend view | `FieldDisplay` lazy-resolves UUID → title via `get_note` |
| Deletion | Null out links before delete, inside transaction |
| Import | Rebuild `note_links` from `fields_json` post-restore |
