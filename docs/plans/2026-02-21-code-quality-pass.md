# Code Quality Pass Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Apply Rust coding guidelines and TypeScript improvements across the full stack before adding new features.

**Architecture:** Incremental, non-breaking changes organized into three tiers: correctness fixes first, then idiomatic Rust improvements, then TypeScript tightening. Each task is independently committable. No public API changes.

**Tech Stack:** Rust (Clippy, rusqlite), TypeScript/React (Tauri frontend)

---

### Task 1: Fix the failing test

The `test_text_note_schema_loaded` test asserts `field_type == "text"` but the TextNote schema was changed to `"textarea"` in commit `270287c`. The test was not updated.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs:238`

**Step 1: Run the failing test to confirm**

Run: `cargo test -p krillnotes-core test_text_note_schema_loaded 2>&1`
Expected: FAIL — `left: "textarea"  right: "text"`

**Step 2: Fix the assertion**

Change line 238 from:
```rust
assert_eq!(schema.fields[0].field_type, "text");
```
to:
```rust
assert_eq!(schema.fields[0].field_type, "textarea");
```

**Step 3: Run the test to verify it passes**

Run: `cargo test -p krillnotes-core test_text_note_schema_loaded 2>&1`
Expected: PASS

**Step 4: Run full test suite**

Run: `cargo test -p krillnotes-core 2>&1`
Expected: All 60 tests pass

**Step 5: Commit**

```
fix: update test_text_note_schema_loaded for textarea field type
```

---

### Task 2: Fix standard Clippy warnings in krillnotes-core

There is 1 standard Clippy warning in krillnotes-core.

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs:897-898`

**Step 1: Run Clippy to confirm warnings**

Run: `cargo clippy -p krillnotes-core 2>&1`
Expected: 1 warning — `collapsible_str_replace` on lines 897-898

**Step 2: Fix collapsible str replace**

In `workspace.rs`, the `humanize` function at line 895-909. Change:
```rust
fn humanize(filename: &str) -> String {
    filename
        .replace('-', " ")
        .replace('_', " ")
```
to:
```rust
fn humanize(filename: &str) -> String {
    filename
        .replace(['-', '_'], " ")
```

**Step 3: Run Clippy to verify clean**

Run: `cargo clippy -p krillnotes-core 2>&1`
Expected: 0 warnings

**Step 4: Run tests**

Run: `cargo test -p krillnotes-core 2>&1`
Expected: All pass

**Step 5: Commit**

```
fix: resolve Clippy warnings in krillnotes-core
```

---

### Task 3: Fix standard Clippy warnings in krillnotes-desktop

There are 11 standard Clippy warnings in the Tauri backend.

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Run Clippy to confirm**

Run: `cargo clippy -p krillnotes-desktop 2>&1`
Expected: 11 warnings (ptr_arg, needless_borrows, explicit_auto_deref)

**Step 2: Fix `ptr_arg` — change `&PathBuf` to `&Path`**

Add `use std::path::Path;` to imports (line 17 area), then change the two function signatures:

Line 51: `fn generate_unique_label(state: &AppState, path: &PathBuf)` → `fn generate_unique_label(state: &AppState, path: &Path)`

Line 71: `fn find_window_for_path(state: &AppState, path: &PathBuf)` → `fn find_window_for_path(state: &AppState, path: &Path)`

**Step 3: Fix `needless_borrows_for_generic_args`**

Line 107: `.title(&format!("Krillnotes - {}", label))` → `.title(format!("Krillnotes - {label}"))` (remove `&`, inline format arg)

**Step 4: Fix `explicit_auto_deref`**

Replace all `&*state` with `&state` — there are 9 instances on lines 185, 191, 196, 206, 227, 233, 238, 247, 258.

**Step 5: Run Clippy to verify clean**

Run: `cargo clippy -p krillnotes-desktop 2>&1`
Expected: 0 warnings

**Step 6: Run full workspace test suite**

Run: `cargo test --workspace 2>&1`
Expected: All pass

**Step 7: Commit**

```
fix: resolve Clippy warnings in krillnotes-desktop
```

---

### Task 4: Extract `note_from_row` helper to eliminate row-mapping duplication

The 11-column tuple extraction + `Note` construction pattern is copy-pasted in `get_note`, `list_all_notes`, and `get_children`. Extract a shared closure or helper.

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Add the helper function**

Add a private helper at the bottom of the `impl Workspace` block (before the closing `}` of the impl, above `fn humanize`):

```rust
/// Row-mapping closure for `rusqlite::Row` → raw tuple.
///
/// Returns the 11-column tuple that `note_from_row_tuple` converts into a `Note`.
/// Extracted to avoid duplicating column-index logic across every query.
fn map_note_row(row: &rusqlite::Row) -> rusqlite::Result<(String, String, String, Option<String>, i64, i64, i64, i64, i64, String, i64)> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, Option<String>>(3)?,
        row.get::<_, i64>(4)?,
        row.get::<_, i64>(5)?,
        row.get::<_, i64>(6)?,
        row.get::<_, i64>(7)?,
        row.get::<_, i64>(8)?,
        row.get::<_, String>(9)?,
        row.get::<_, i64>(10)?,
    ))
}

/// Converts a raw 11-column tuple into a [`Note`], parsing `fields_json`.
fn note_from_row_tuple(
    (id, title, node_type, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded_int): (String, String, String, Option<String>, i64, i64, i64, i64, i64, String, i64),
) -> Result<Note> {
    Ok(Note {
        id,
        title,
        node_type,
        parent_id,
        position: position as i32,
        created_at,
        modified_at,
        created_by,
        modified_by,
        fields: serde_json::from_str(&fields_json)?,
        is_expanded: is_expanded_int == 1,
    })
}
```

**Step 2: Refactor `get_note` (lines 170-209)**

Replace the body with:
```rust
pub fn get_note(&self, note_id: &str) -> Result<Note> {
    let row = self.connection().query_row(
        "SELECT id, title, node_type, parent_id, position,
                created_at, modified_at, created_by, modified_by,
                fields_json, is_expanded
         FROM notes WHERE id = ?",
        [note_id],
        map_note_row,
    )?;
    note_from_row_tuple(row)
}
```

**Step 3: Refactor `list_all_notes` (lines 406-452)**

Replace the body with:
```rust
pub fn list_all_notes(&self) -> Result<Vec<Note>> {
    let mut stmt = self.connection().prepare(
        "SELECT id, title, node_type, parent_id, position,
                created_at, modified_at, created_by, modified_by,
                fields_json, is_expanded
         FROM notes ORDER BY parent_id, position",
    )?;

    let rows = stmt
        .query_map([], map_note_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    rows.into_iter().map(note_from_row_tuple).collect()
}
```

**Step 4: Refactor `get_children` (lines 555-603)**

Replace the body with:
```rust
pub fn get_children(&self, parent_id: &str) -> Result<Vec<Note>> {
    let mut stmt = self.connection().prepare(
        "SELECT id, title, node_type, parent_id, position,
                created_at, modified_at, created_by, modified_by,
                fields_json, is_expanded
         FROM notes WHERE parent_id = ?1 ORDER BY position",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![parent_id], map_note_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    rows.into_iter().map(note_from_row_tuple).collect()
}
```

**Step 5: Run tests**

Run: `cargo test -p krillnotes-core 2>&1`
Expected: All pass (behavior is identical)

**Step 6: Run Clippy**

Run: `cargo clippy -p krillnotes-core 2>&1`
Expected: 0 warnings

**Step 7: Commit**

```
refactor: extract note_from_row helper to eliminate row-mapping duplication
```

---

### Task 5: Apply select pedantic Clippy fixes

Inline format args (`{var}` instead of `"{}", var`) and add `#[must_use]` to pure getters.

**Files:**
- Modify: `krillnotes-core/src/core/error.rs`
- Modify: `krillnotes-core/src/core/device.rs`
- Modify: `krillnotes-core/src/core/operation.rs`
- Modify: `krillnotes-core/src/core/scripting/schema.rs`
- Modify: `krillnotes-core/src/core/scripting/hooks.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Fix uninlined format args in error.rs**

Lines 48-54, change all `format!("...: {}", e)` to `format!("...: {e}")`:
```rust
Self::Database(e) => format!("Failed to save: {e}"),
Self::SchemaNotFound(name) => format!("Unknown note type: {name}"),
Self::Scripting(e) => format!("Script error: {e}"),
Self::Io(e) => format!("File error: {e}"),
Self::Json(e) => format!("Data format error: {e}"),
```

**Step 2: Add `#[must_use]` to `user_message()`**

Line 46, add `#[must_use]` attribute:
```rust
#[must_use]
pub fn user_message(&self) -> String {
```

**Step 3: Fix uninlined format args in device.rs**

Line 23: `format!("device-{:016x}", hash)` → `format!("device-{hash:016x}")`
Lines 28-31: `format!("Failed to get MAC address: {}", e)` → `format!("Failed to get MAC address: {e}")`

**Step 4: Add `#[must_use]` to Operation getters**

Lines 86 and 96 in operation.rs:
```rust
#[must_use]
pub fn operation_id(&self) -> &str {
```
```rust
#[must_use]
pub fn timestamp(&self) -> i64 {
```

**Step 5: Fix unnested or-patterns in schema.rs**

Line 52, change:
```rust
Some(FieldValue::Number(_)) | Some(FieldValue::Boolean(_)) => false,
```
to:
```rust
Some(FieldValue::Number(_) | FieldValue::Boolean(_)) => false,
```

**Step 6: Fix uninlined format args in schema.rs and hooks.rs**

In `schema.rs` line 57: `format!("Required field '{}' must not be empty", field_def.name)` → `format!("Required field '{}' must not be empty", field_def.name)` — keep this one as-is since `field_def.name` is a field access, not a plain variable.

In `hooks.rs` line 92: `format!("on_save hook error: {}", e)` → `format!("on_save hook error: {e}")`
In `hooks.rs` line 123: `format!("field '{}': {}", field_def.name, e)` — keep as-is (field access).
In `hooks.rs` line 189: `format!("invalid date '{}': {}", s, e)` → `format!("invalid date '{s}': {e}")`

**Step 7: Fix uninlined format args in lib.rs (desktop)**

Line 63: `format!("{}-{}", filename, counter)` → `format!("{filename}-{counter}")`
Line 89: `format!("Failed to focus: {}", e)` → `format!("Failed to focus: {e}")`
Line 110: `format!("Failed to create window: {}", e)` → `format!("Failed to create window: {e}")`
Line 193: `format!("Failed to create: {}", e)` → `format!("Failed to create: {e}")`
Line 198: `format!("Krillnotes - {}", label)` → `format!("Krillnotes - {label}")` (also line 241)
Line 235: `format!("Failed to open: {}", e)` → `format!("Failed to open: {e}")`

**Step 8: Run Clippy and tests**

Run: `cargo clippy --workspace 2>&1 && cargo test --workspace 2>&1`
Expected: 0 warnings, all tests pass

**Step 9: Commit**

```
refactor: apply pedantic Clippy fixes (inlined format args, must_use, or-patterns)
```

---

### Task 6: Add `device_id()` method to `Operation`

The `OperationLog` has `extract_device_id` that duplicates the same 4-arm match as `operation_id()` and `timestamp()`. Move it onto `Operation` itself.

**Files:**
- Modify: `krillnotes-core/src/core/operation.rs`
- Modify: `krillnotes-core/src/core/operation_log.rs`

**Step 1: Add `device_id()` to Operation**

In `operation.rs`, after the `timestamp()` method (line 103), add:
```rust
/// Returns the device identifier of the machine that created this operation.
#[must_use]
pub fn device_id(&self) -> &str {
    match self {
        Self::CreateNote { device_id, .. } => device_id,
        Self::UpdateField { device_id, .. } => device_id,
        Self::DeleteNote { device_id, .. } => device_id,
        Self::MoveNote { device_id, .. } => device_id,
    }
}
```

**Step 2: Remove `extract_device_id` from OperationLog and use `op.device_id()`**

In `operation_log.rs`, line 47: change `self.extract_device_id(op)` to `op.device_id()`.

Delete the `extract_device_id` method (lines 85-92).

**Step 3: Run tests**

Run: `cargo test -p krillnotes-core 2>&1`
Expected: All pass

**Step 4: Commit**

```
refactor: move device_id() accessor onto Operation, remove OperationLog duplicate
```

---

### Task 7: Tighten TypeScript `fieldType` to a union type

The `fieldType` property is typed as `string` but only 6 values are valid. Tighten it.

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`
- Modify: `krillnotes-desktop/src/components/FieldEditor.tsx`
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx`

**Step 1: Add the union type and update FieldDefinition**

In `types.ts`, add a type alias and update the interface:

Before `FieldDefinition` interface (around line 33), add:
```typescript
export type FieldType = 'text' | 'textarea' | 'number' | 'boolean' | 'date' | 'email';
```

Change line 36 from:
```typescript
fieldType: string;  // "text" | "number" | "boolean" | "date" | "email"
```
to:
```typescript
fieldType: FieldType;
```

**Step 2: Update FieldEditor to use the type**

In `FieldEditor.tsx` line 5, change:
```typescript
fieldType: string;
```
to:
```typescript
fieldType: FieldType;
```

Add the import at line 1:
```typescript
import type { FieldValue, FieldType } from '../types';
```

**Step 3: Verify the build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit 2>&1`
Expected: No type errors

**Step 4: Commit**

```
refactor(ts): tighten fieldType from string to union type
```

---

### Task 8: Add aria-label to tree expand/collapse button

The expand/collapse button in TreeNode uses icon-only text (▼/▶) without an accessible name.

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx:29-38`

**Step 1: Add aria-label and aria-expanded**

Change the button (lines 29-38) from:
```tsx
<button
  tabIndex={-1}
  onClick={(e) => {
    e.stopPropagation();
    onToggleExpand(node.note.id);
  }}
  className="mr-1 text-muted-foreground hover:text-foreground"
>
  {isExpanded ? '▼' : '▶'}
</button>
```
to:
```tsx
<button
  tabIndex={-1}
  onClick={(e) => {
    e.stopPropagation();
    onToggleExpand(node.note.id);
  }}
  className="mr-1 text-muted-foreground hover:text-foreground"
  aria-label={isExpanded ? 'Collapse' : 'Expand'}
  aria-expanded={isExpanded}
>
  {isExpanded ? '▼' : '▶'}
</button>
```

**Step 2: Verify the build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit 2>&1`
Expected: No type errors

**Step 3: Commit**

```
fix(a11y): add aria-label to tree expand/collapse button
```

---

### Task 9: Delete unused WorkspaceInfo component

The component at `WorkspaceInfo.tsx` is never imported anywhere. The `WorkspaceInfo` *type* in `types.ts` is used throughout — that stays. Only the component file is dead code. User approved removal.

**Files:**
- Delete: `krillnotes-desktop/src/components/WorkspaceInfo.tsx`

**Step 1: Verify the component is unused**

Search for imports of this component file. The only references to "WorkspaceInfo" in the codebase should be to the *type* from `types.ts`, not this component.

**Step 2: Delete the file**

```bash
rm krillnotes-desktop/src/components/WorkspaceInfo.tsx
```

**Step 3: Verify the build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit 2>&1`
Expected: No type errors

**Step 4: Commit**

```
chore: remove unused WorkspaceInfo component
```

---

### Task 10: Remove stale guideline comment

Line 3 of `workspace.rs` has a manual compliance stamp: `// Rust guideline compliant 2026-02-19 (updated: add delete_note strategy dispatcher)`. Line 44 of `delete.rs` has a similar one. These are noise — the git log serves this purpose.

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs:3`
- Modify: `krillnotes-core/src/core/delete.rs:44`

**Step 1: Remove the comments**

In `workspace.rs`, delete line 3:
```rust
// Rust guideline compliant 2026-02-19 (updated: add delete_note strategy dispatcher)
```

In `delete.rs`, delete line 44:
```rust
// Rust guideline compliant 2026-02-19
```

**Step 2: Run tests**

Run: `cargo test --workspace 2>&1`
Expected: All pass

**Step 3: Commit**

```
chore: remove stale guideline compliance comments
```

---

### Task 11: Final verification

**Step 1: Run full Clippy check**

Run: `cargo clippy --workspace 2>&1`
Expected: 0 warnings

**Step 2: Run full test suite**

Run: `cargo test --workspace 2>&1`
Expected: All tests pass

**Step 3: Verify TypeScript build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit 2>&1`
Expected: No errors

**Step 4: Run git diff --stat to review all changes**

Verify the changes match the plan scope — no unintended modifications.
