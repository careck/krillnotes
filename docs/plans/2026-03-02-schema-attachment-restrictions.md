# Schema Attachment Restrictions Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `allow_attachments` and `attachment_types` schema-level options that control the note-level attachments panel.

**Architecture:** Two new fields on the `Schema` struct are parsed from Rhai, propagated through `SchemaInfo` to the frontend, which gates rendering of `AttachmentsSection` and passes MIME type filters down.

**Tech Stack:** Rust (krillnotes-core), Tauri (lib.rs), TypeScript/React (InfoPanel, AttachmentsSection)

---

### Task 1: Extend the `Schema` struct

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:59-71`

**Step 1: Add two fields to `Schema`**

In `schema.rs`, find the `Schema` struct (around line 59). Add after `allowed_children_types`:

```rust
/// When `true`, the note-level attachments panel is shown for this schema.
/// Defaults to `false` (opt-in).
pub allow_attachments: bool,
/// MIME types accepted by the note-level attachments panel; empty means all types are allowed.
/// Ignored when `allow_attachments` is `false`.
pub attachment_types: Vec<String>,
```

The struct now ends at line ~73.

**Step 2: Fix the constructor call at the end of `parse_from_rhai` (line 265)**

The `Ok(Schema { ... })` line at the bottom of `parse_from_rhai` needs two new fields. We'll add the parsing in Task 2, but first add placeholders to keep it compiling:

Change line 265:
```rust
Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort, allowed_parent_types, allowed_children_types })
```
To:
```rust
Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort, allowed_parent_types, allowed_children_types, allow_attachments: false, attachment_types: Vec::new() })
```

**Step 3: Verify it compiles**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-core && cargo build 2>&1 | head -20
```
Expected: no errors.

**Step 4: Commit**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-core/src/core/scripting/schema.rs
git commit -m "feat: add allow_attachments and attachment_types fields to Schema struct"
```

---

### Task 2: Parse `allow_attachments` and `attachment_types` from Rhai

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs` — `parse_from_rhai` function (around line 252-265)

**Step 1: Write a failing test**

In `krillnotes-core/src/core/scripting/mod.rs`, find the test block (around line 871). Add these two tests after `test_schema_title_flags_explicit_true` (line 1363):

```rust
#[test]
fn test_schema_allow_attachments_defaults_to_false() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("AttachTest", #{
            fields: [#{ name: "name", type: "text" }]
        });
    "#, "test").unwrap();
    let schema = registry.get_schema("AttachTest").unwrap();
    assert!(!schema.allow_attachments, "allow_attachments should default to false");
    assert!(schema.attachment_types.is_empty(), "attachment_types should default to empty");
}

#[test]
fn test_schema_allow_attachments_explicit_with_types() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("PhotoNote", #{
            allow_attachments: true,
            attachment_types: ["image/jpeg", "image/png"],
            fields: [#{ name: "caption", type: "text" }]
        });
    "#, "test").unwrap();
    let schema = registry.get_schema("PhotoNote").unwrap();
    assert!(schema.allow_attachments);
    assert_eq!(schema.attachment_types, vec!["image/jpeg", "image/png"]);
}
```

**Step 2: Run tests to confirm they pass (default is already false from Task 1)**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-core
cargo test test_schema_allow_attachments 2>&1
```
Expected: both tests PASS (since `allow_attachments: false` is hardcoded and `attachment_types` is empty).

Note: `test_schema_allow_attachments_explicit_with_types` will FAIL because the Rhai values aren't parsed yet.

**Step 3: Add parsing in `parse_from_rhai`**

In `schema.rs`, just before the final `Ok(Schema { ... })` line (around line 263), add:

```rust
let allow_attachments = def
    .get("allow_attachments")
    .and_then(|v| v.clone().try_cast::<bool>())
    .unwrap_or(false);

let mut attachment_types: Vec<String> = Vec::new();
if let Some(arr) = def
    .get("attachment_types")
    .and_then(|v| v.clone().try_cast::<rhai::Array>())
{
    for item in arr {
        let s = item.try_cast::<String>().ok_or_else(|| {
            KrillnotesError::Scripting("attachment_types array must contain only strings".into())
        })?;
        attachment_types.push(s);
    }
}
```

Also update the `Ok(Schema { ... })` constructor to use the parsed variables:
```rust
Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort, allowed_parent_types, allowed_children_types, allow_attachments, attachment_types })
```

**Step 4: Run both tests — expect PASS**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-core
cargo test test_schema_allow_attachments 2>&1
```
Expected: PASS for both tests.

**Step 5: Run full test suite**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-core
cargo test 2>&1 | tail -5
```
Expected: all tests pass (currently 235).

**Step 6: Commit**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: parse allow_attachments and attachment_types from Rhai schema definition"
```

---

### Task 3: Propagate through `SchemaInfo` in lib.rs

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:547-556` (SchemaInfo struct)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:581-590` (get_schema_fields)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:608-617` (get_all_schemas)

**Step 1: Add fields to `SchemaInfo`**

Find `struct SchemaInfo` (line ~547). Add two fields after `has_hover_hook`:

```rust
allow_attachments: bool,
attachment_types: Vec<String>,
```

**Step 2: Populate in `get_schema_fields`**

The `Ok(SchemaInfo { ... })` block (line ~581). Add:

```rust
allow_attachments: schema.allow_attachments,
attachment_types: schema.attachment_types,
```

**Step 3: Populate in `get_all_schemas`**

The `SchemaInfo { ... }` struct literal inside the loop (line ~608). Add the same two fields.

**Step 4: Build the Tauri crate**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop
cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
```
Expected: no errors.

**Step 5: Commit**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: expose allow_attachments and attachment_types in SchemaInfo Tauri response"
```

---

### Task 4: Update the TypeScript `SchemaInfo` type

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:57-66`

**Step 1: Add fields to the interface**

Find `interface SchemaInfo` (line 57). Add after `hasHoverHook`:

```typescript
allowAttachments: boolean;
attachmentTypes: string[];
```

**Step 2: Verify TypeScript builds**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop
npm run build 2>&1 | tail -20
```
Expected: clean build (the new fields are optional from TypeScript's perspective because Tauri serializes them — if they're missing from existing callers, TypeScript will tell us).

**Step 3: Commit**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add allowAttachments and attachmentTypes to TypeScript SchemaInfo interface"
```

---

### Task 5: Gate `AttachmentsSection` in `InfoPanel.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx:572-573`

**Step 1: Update the render**

Find the `{/* Attachments */}` comment (line ~572). Replace:

```tsx
<AttachmentsSection noteId={selectedNote?.id ?? null} />
```

With:

```tsx
{schemaInfo?.allowAttachments && (
  <AttachmentsSection
    noteId={selectedNote?.id ?? null}
    allowedTypes={schemaInfo.attachmentTypes}
  />
)}
```

**Step 2: Verify TypeScript builds**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop
npm run build 2>&1 | tail -20
```
Expected: error about `allowedTypes` prop not existing on `AttachmentsSection` yet — we'll fix in Task 6.

**Step 3: Commit after Task 6 passes build**

Hold commit until Task 6 is done and build is clean.

---

### Task 6: Update `AttachmentsSection.tsx` to accept and apply `allowedTypes`

**Files:**
- Modify: `krillnotes-desktop/src/components/AttachmentsSection.tsx`

**Step 1: Add `mimeToExtension` helper and update props interface**

At the top of the file, after the imports, add the `mimeToExtension` helper (same implementation as in `FileField.tsx`):

```typescript
function mimeToExtension(mime: string): string {
  const sub = mime.split('/')[1] ?? mime;
  const clean = sub.split(';')[0].trim();
  const special: Record<string, string> = {
    'svg+xml': 'svg',
    'x-matroska': 'mkv',
    'vnd.openxmlformats-officedocument.wordprocessingml.document': 'docx',
    'vnd.openxmlformats-officedocument.spreadsheetml.sheet': 'xlsx',
    'vnd.openxmlformats-officedocument.presentationml.presentation': 'pptx',
    'x-m4v': 'mp4',
    'quicktime': 'mov',
  };
  return special[clean] ?? clean.replace(/\+.*$/, '').replace(/^x-/, '');
}
```

Update the props interface (line ~8):

```typescript
interface AttachmentsSectionProps {
  noteId: string | null;
  allowedTypes: string[];   // MIME types; empty = all allowed
}
```

Update the function signature (line ~22):

```typescript
export default function AttachmentsSection({ noteId, allowedTypes }: AttachmentsSectionProps) {
```

**Step 2: Apply filter in the file picker (`handleAdd`, line ~80)**

Replace the `openFilePicker` call:

```typescript
const handleAdd = async () => {
  if (!noteId) return;
  setError('');
  try {
    const filters = allowedTypes.length > 0
      ? [{ name: 'Allowed files', extensions: allowedTypes.flatMap(m => {
          const ext = mimeToExtension(m);
          return ext === 'jpeg' ? ['jpeg', 'jpg'] : [ext];
        }) }]
      : [];
    const selected = await openFilePicker({ multiple: true, filters });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    for (const filePath of paths) {
      await invoke('attach_file', { noteId, filePath });
    }
    await loadAttachments();
  } catch (e) {
    setError(`Failed to attach: ${e}`);
  }
};
```

**Step 3: Apply filter in drag-and-drop (`handleDrop`, line ~57)**

In the `for (const file of files)` loop, add a MIME check before processing:

```typescript
for (const file of files) {
  // Reject files with disallowed MIME types
  if (allowedTypes.length > 0 && !allowedTypes.includes(file.type)) {
    setError(`File type "${file.type || file.name}" is not allowed.`);
    continue;
  }
  try {
    // ... rest of existing code unchanged
```

**Step 4: Verify TypeScript builds clean**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop
npm run build 2>&1 | tail -20
```
Expected: no errors.

**Step 5: Commit Tasks 5 and 6 together**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-desktop/src/components/InfoPanel.tsx krillnotes-desktop/src/components/AttachmentsSection.tsx
git commit -m "feat: gate AttachmentsSection on schema allow_attachments; apply MIME type filter"
```

---

### Task 7: Verify and wrap up

**Step 1: Run full Rust test suite**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-core
cargo test 2>&1 | tail -5
```
Expected: all tests pass.

**Step 2: Run TypeScript build**

```bash
cd /Users/careck/Source/Krillnotes/krillnotes-desktop
npm run build 2>&1 | tail -10
```
Expected: clean build.

**Step 3: Invoke superpowers:finishing-a-development-branch to wrap up**
