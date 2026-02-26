# Note Anatomy Section Design

**Date:** 2026-02-26

## Goal

Add a `## Note anatomy` section to the scripting guide as an upfront orientation to the note data model, placed before `## 1. Script structure`.

## Content

### Placement

After the existing intro paragraphs and `---` separator, before `## 1. Script structure`.

### Text

```markdown
## Note anatomy

Every item in Krillnotes is a **note**. Notes form a tree: each note has exactly one parent (or is a root), and can have any number of children. The tree is how you build folders, projects, contact lists, and so on — by nesting compatible types inside each other.

Each note has two layers of data:

- **System fields** — always present: a unique `id`, a `node_type` (the schema name, e.g. `"Task"`), and a `title`.
- **Schema fields** — defined by the `fields: [...]` list in your schema. Accessed in hooks as `note.fields["field_name"]`.

Tags are a third, separate layer: assigned through the UI tag editor, not via schema fields.

The exact fields available in each hook:

| Field | `on_save` | `on_view` | `on_add_child` |
|---|---|---|---|
| `note.id` | ✓ | ✓ | ✓ |
| `note.node_type` | ✓ | ✓ | ✓ |
| `note.title` | ✓ writable | ✓ | ✓ |
| `note.fields` | ✓ writable | ✓ | ✓ |
| `note.tags` | — | ✓ | — |

---
```

## Verified Against Source

Fields confirmed against `krillnotes-core/src/core/scripting/`:
- `on_save` map: id, node_type, title, fields (schema.rs ~350)
- `on_view` map: id, node_type, title, fields, tags (mod.rs ~644)
- `on_add_child` map: id, node_type, title, fields — tags intentionally omitted (schema.rs ~461)
- `parent_id` is stored in the Note struct but NOT exposed to any hook maps

## Non-changes

- No renumbering of existing sections
- The existing per-hook note map tables in sections 5 and 6 stay as-is (they give writable/read-only detail; this section gives the overview)
