# Design: `is_leaf` Schema Constraint

**Date:** 2026-03-10
**Status:** Approved

## Summary

Add an `is_leaf: bool` field to the `Schema` struct. When `true`, notes of that schema cannot have any children. The constraint is enforced in the Rust core and observed in the UI.

## Rhai API

New optional key on the schema map, defaulting to `false`:

```rhai
schema("MyType", #{
    is_leaf: true,   // default: false — children allowed
    fields: [...]
})
```

- `is_leaf: false` (default) — children are allowed, no restrictions
- `is_leaf: true` — no children allowed; all add-child paths are blocked

## Rust Core

- `Schema` struct gains `pub is_leaf: bool` (default `false`)
- Parsed from the Rhai map key `"is_leaf"` in `parse_from_rhai`
- Enforcement added to all four child-creation paths in `workspace.rs`:
  - `create_note` — when a `parent_id` is provided
  - `move_note` — when the destination parent changes
  - `deep_copy_note` — paste operations that place a note under a parent
  - Rhai `create_child_note` scripting function
- All blocked paths return `KrillnotesError::Validation("Cannot add children to a leaf note (<schema>)")`
- Existing children of notes whose schema gains `is_leaf: true` are unaffected (constraint is forward-only)

## Tauri DTO

`SchemaInfo` in `lib.rs` gains `is_leaf: bool`, serialised as `isLeaf` via the existing `#[serde(rename_all = "camelCase")]`.

## Frontend

- `SchemaInfo` in `types.ts` gains `isLeaf: boolean`
- **Context menu / add-child button** — disabled and greyed out when the selected note's schema has `isLeaf: true`
- **Drag-drop** — leaf notes reject being used as drop targets (blocked cursor shown)

## Files Affected

| File | Change |
|------|--------|
| `krillnotes-core/src/core/scripting/schema.rs` | Add `is_leaf` field + Rhai parsing |
| `krillnotes-core/src/core/scripting/mod.rs` | Update struct literals |
| `krillnotes-core/src/core/scripting/display_helpers.rs` | Update struct literals |
| `krillnotes-core/src/core/workspace.rs` | Enforcement in create/move/copy |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Add `isLeaf` to `SchemaInfo` DTO |
| `krillnotes-desktop/src/types.ts` | Add `isLeaf` to `SchemaInfo` interface |
| `krillnotes-desktop/src/components/TreeNode.tsx` | Disable add-child, block drop target |
| `krillnotes-desktop/src/components/TreeView.tsx` | Possibly observe for drag-drop |
| `SCRIPTING.md` | Document `is_leaf` option |
