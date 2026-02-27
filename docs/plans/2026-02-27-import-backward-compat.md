# Import Backward Compatibility Fix

**Date:** 2026-02-27
**Issue:** #26 — Can't import older export files
**Branch:** `fix/import-backward-compat`

## Problem

Importing archives exported before the tags feature (PR #15) fails because
`Note.tags: Vec<String>` has no `#[serde(default)]`. Serde requires the key to
be present in JSON; older `notes.json` files don't have it.

`workspace.json` being absent from older archives is **not a real bug** —
`import_workspace` never reads that file.

## Fix

### Step 1 — `krillnotes-core/src/core/note.rs`
Add `#[serde(default)]` to the `tags` field:

```rust
#[serde(default)]
pub tags: Vec<String>,
```

### Step 2 — Add regression tests in `export.rs`
- `test_import_notes_without_tags_field` — hand-craft a `notes.json` with no
  `"tags"` key and verify `import_workspace` succeeds, notes come back with
  empty tag lists.
- `test_import_archive_without_workspace_json` — zip with only `notes.json`,
  verify import succeeds (confirms workspace.json absence is already safe).

### Step 3 — Verify all existing tests still pass
```
cargo test -p krillnotes-core
```
