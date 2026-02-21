# Children Sort Design

## Summary

Add a `children_sort` schema property that controls how a note's children are sorted in the tree. Set on the **parent's** schema (the container decides the order). Three modes: `"asc"` (A-Z by title), `"desc"` (Z-A by title), `"none"` (manual position order, the default).

## Motivation

Some note types act as containers (e.g., a "Contacts" folder). Their children should appear alphabetically rather than in insertion order. This should be declarative via the schema, just like `title_can_edit`.

## Design

### Schema Property

- **Name:** `children_sort`
- **Type:** String enum: `"none"` | `"asc"` | `"desc"`
- **Default:** `"none"` (preserves current position-based behavior)
- **Set on:** The parent note's schema

### Rhai Usage

```rhai
schema("ContactsFolder", #{
    children_sort: "asc",
    fields: []
});
```

### Changes by Layer

#### 1. Rust Schema Struct (`schema.rs`)

Add `children_sort: String` to the `Schema` struct. Parse from Rhai map with default `"none"`.

#### 2. Tauri IPC (`lib.rs`)

Add `children_sort: String` to `SchemaInfo`. Expose it alongside `title_can_view` / `title_can_edit`.

#### 3. Frontend Types (`types.ts`)

Add `childrenSort: "asc" | "desc" | "none"` to `SchemaInfo` interface.

#### 4. Tree Building (`tree.ts`)

Modify `buildTree()` to accept a schema lookup (map of `nodeType` to sort config). After grouping children by parent, look up the parent note's `nodeType`, get its `childrenSort`, and sort accordingly:

- `"none"` -> sort by `position` (current behavior)
- `"asc"` -> sort by `title` ascending (locale-aware `localeCompare`)
- `"desc"` -> sort by `title` descending (locale-aware `localeCompare`)

#### 5. Schema Fetching

The frontend needs access to all schemas' `childrenSort` values when building the tree. Options:
- Fetch all schemas upfront (a new Tauri command returning `Map<nodeType, SchemaInfo>`)
- Or pass the already-fetched per-note schema info through

The simplest approach: add a new Tauri command `get_all_schemas()` that returns the full map, called once on workspace load alongside `list_notes`.

### Non-Goals

- Per-note overrides (individual notes overriding their schema's sort)
- Sorting by fields other than title
- Sort stability guarantees beyond what JS `Array.sort` provides
