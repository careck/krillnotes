# Design: Field & Title Visibility Flags

**Date:** 2026-02-20
**Status:** Approved

## Problem

All schema fields are unconditionally shown in both view and edit mode. There is no way to:

- Hide a field in view mode (e.g. internal/helper fields only relevant during editing)
- Hide a field in edit mode (e.g. computed/display-only fields)
- Hide the note title input in edit mode for notes whose title is auto-calculated by `on_save`

The Contact note is the motivating example: its title is computed from `first_name` + `last_name` by the `on_save` hook, so the title input in edit mode is misleading — any manual entry would be overwritten on save.

## Design

### Approach: Flat flags (Approach A)

Optional boolean flags on each field and at the schema level. All flags default to `true`, preserving current behavior for schemas that omit them.

### Rhai Schema Syntax

```rhai
schema("Contact", #{
    title_can_view: true,   // default — show title in view mode
    title_can_edit: false,  // hide title input in edit mode
    fields: [
        #{ name: "first_name", type: "text", required: true },
        #{ name: "last_name",  type: "text", required: true },
        // can_view / can_edit default to true when omitted
        #{ name: "helper",   type: "text", can_view: false },  // hidden in view
        #{ name: "computed", type: "text", can_edit: false },  // hidden in edit
    ]
});
```

### Rust Changes

**`FieldDefinition`** (`krillnotes-core/src/core/scripting/schema.rs`):

```rust
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub can_view: bool,   // new, default: true
    pub can_edit: bool,   // new, default: true
}
```

**`Schema`**:

```rust
pub struct Schema {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
    pub title_can_view: bool,  // new, default: true
    pub title_can_edit: bool,  // new, default: true
}
```

The Rhai map parsing reads these optional keys with `.get()` falling back to `true`.

### Frontend Changes

`FieldDefinition` and `Schema` TypeScript types updated to include the new flags.

In `InfoPanel.tsx`, conditional rendering:

- **View mode**: only render fields where `can_view === true`
- **Edit mode**: only render fields where `can_edit === true`
- **Title in view mode**: only render if `schema.title_can_view === true` (or no schema)
- **Title in edit mode**: only render title `<input>` if `schema.title_can_edit === true` (or no schema)

No new components required.

### Contact Schema Update

`contact.rhai` updated to add `title_can_edit: false` at the schema level.

## Defaults

| Context         | Flag              | Default |
|-----------------|-------------------|---------|
| Field           | `can_view`        | `true`  |
| Field           | `can_edit`        | `true`  |
| Schema (title)  | `title_can_view`  | `true`  |
| Schema (title)  | `title_can_edit`  | `true`  |

## Files Affected

- `krillnotes-core/src/core/scripting/schema.rs` — struct definitions + Rhai parsing
- `krillnotes-core/src/system_scripts/contact.rhai` — add `title_can_edit: false`
- `krillnotes-desktop/src/lib/types.ts` (or equivalent) — TypeScript type updates
- `krillnotes-desktop/src/components/InfoPanel.tsx` — conditional rendering logic
