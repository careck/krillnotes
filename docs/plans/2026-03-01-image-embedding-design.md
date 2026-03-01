# Image Embedding Design

**Issue:** #41
**Date:** 2026-03-01
**Depends on:** File Attachments (#23, shipped PR #56)

## Summary

Two complementary features:

1. **`file` field type** — a new schema field type that stores a single file against a named field, backed by the existing encrypted attachment storage.
2. **Image embedding in markdown** — a `{{image: ...}}` custom block syntax for textarea fields and Rhai scripts, plus two Rhai display helpers (`display_image`, `display_download_link`) for use in layout functions.

## Approach

Rust pre-processes image references into `<img data-kn-attach-id="UUID" />` sentinel elements. The frontend post-processes those elements after DOMPurify sanitization, fetching decrypted bytes via the existing `get_attachment_data` Tauri command and replacing the `src` with a base64 data URL.

This keeps UI concerns (byte fetching, data URLs) in the frontend while the Rust core handles only data logic (parsing syntax, resolving references to UUIDs).

## Part 1 — `file` Field Type

### Schema

`FieldDefinition` gains a new optional field:

```rust
pub allowed_types: Vec<String>  // MIME types, e.g. ["image/png", "image/jpeg"]
                                 // empty = all types allowed
```

This follows the existing pattern of extension fields (`max` for rating, `options` for select).

### Storage

`FieldValue` gains a new variant:

```rust
File(Option<String>)  // stores an attachment UUID, or None
```

The attachment itself is stored in the existing `attachments` table and encrypted `.enc` files — no new storage infrastructure.

### Lifecycle

- **Set:** call `attach_file_bytes`, receive UUID, store as `FieldValue::File(Some(uuid))`
- **Replace:** read current UUID → `delete_attachment(old_uuid)` → attach new file → store new UUID
- **Clear:** `delete_attachment(uuid)` → store `FieldValue::File(None)`
- **Note deleted:** existing note deletion must also clean up any UUIDs held in `File` field values

### Validation

The MIME type of any file being attached to a `file` field is validated against `allowed_types` before storage — both on the frontend (`<input accept="...">`) and in the Rust backend before calling `attach_file`.

### TypeScript

```typescript
type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean }
  | { Date: string | null }
  | { Email: string }
  | { NoteLink: string | null }
  | { File: string | null }   // new
```

### UI

`FileField.tsx` — edit component:
- File picker button (respects `allowed_types` via `accept` attribute)
- Current file shown as filename + image thumbnail (for image MIME types) or file icon
- Clear button

View mode: filename + thumbnail for images, filename + icon for other types.

## Part 2 — `{{image: ...}}` Block Syntax

### Syntax

```
{{image: attach:diagram.png}}
{{image: field:cover, width: 400}}
{{image: attach:photo.jpg, width: 200, alt: My caption}}
```

- `attach:<filename>` — resolves by searching the note's attachments by filename (first match)
- `field:<fieldName>` — resolves by reading `note.fields[fieldName]` → `FieldValue::File(Some(uuid))`
- `width` — optional max-width in pixels (0 or omitted = no constraint)
- `alt` — optional alt text

### Rust Pre-processor

New function in `display_helpers.rs`:

```rust
pub fn preprocess_image_blocks(text: &str, workspace: &Workspace, note: &Note) -> String
```

Steps:
1. Regex-scan for `{{image: ...}}` blocks
2. Parse key-value content
3. Resolve source → attachment UUID (via workspace + note)
4. Emit `<img data-kn-attach-id="UUID" data-kn-width="200" alt="caption" class="kn-image-embed" />`
5. Unresolvable → emit `<span class="kn-image-error">Image not found: {source}</span>`

Called before `render_markdown_to_html`; emitted `<img>` tags pass through `pulldown-cmark` as raw inline HTML.

### Frontend Post-processing

`InfoPanel.tsx`:
- Add `data-kn-attach-id` and `data-kn-width` to DOMPurify's `ADD_ATTR`
- Add a `useEffect` that fires when `customViewHtml` changes:
  1. Query all `img[data-kn-attach-id]` in the container ref
  2. `Promise.all`: for each, call `invoke('get_attachment_data', { attachmentId })`, set `img.src = 'data:...;base64,...'` and `img.style.maxWidth`
  3. On error: replace element with `<span class="kn-image-error">Image not found</span>`

## Part 3 — Rhai Display Helpers

Two new functions registered in the Rhai engine with captured workspace + note context (same closure pattern as `link_to`, `render_tags`).

### `display_image`

```rhai
display_image("field:cover", 200, "Cover photo")
display_image("attach:diagram.png", 0, "")   // 0 = no width; "" = no alt
```

Returns the same `<img data-kn-attach-id="..." />` HTML as the `{{image: ...}}` pre-processor — shares the same UUID resolution logic.

### `display_download_link`

```rhai
display_download_link("field:document", "Download report")
display_download_link("attach:report.pdf", "")   // "" → falls back to filename
```

Returns:
```html
<a data-kn-download-id="UUID" class="kn-download-link">label or filename</a>
```

Frontend click handler in `InfoPanel` (which already handles `data-note-id`) also handles `data-kn-download-id`: fetches attachment bytes, creates a Blob URL, triggers a browser download.

### Registration

Both helpers are registered as closures in `scripting/mod.rs` at the same point where `markdown` is registered, capturing `Arc<Workspace>` and `note_id`.

## Shared Resolution Logic

Both the `{{image: ...}}` pre-processor and `display_image`/`display_download_link` need to resolve a source string to `(uuid, filename, mime_type)`. This is extracted into a shared helper:

```rust
fn resolve_attachment_source(
    source: &str,        // "attach:filename" or "field:fieldName"
    workspace: &Workspace,
    note: &Note,
) -> Option<AttachmentMeta>
```

## Export / Import

No changes needed. `FieldValue::File(uuid)` serializes as the UUID string. The attachment itself is already handled by the existing attachment export/import path. On import, `attach_file_with_id` preserves the original UUID so field references remain valid.

## Files Changed

| File | Change |
|------|--------|
| `krillnotes-core/src/core/note.rs` | Add `File(Option<String>)` to `FieldValue` |
| `krillnotes-core/src/core/scripting/schema.rs` | Add `allowed_types` to `FieldDefinition`; recognize `"file"` field type |
| `krillnotes-core/src/core/workspace.rs` | Handle `File` variant in note CRUD; clean up attachments on field value change and note deletion |
| `krillnotes-core/src/core/scripting/display_helpers.rs` | Add `preprocess_image_blocks`, `resolve_attachment_source`, `display_image`, `display_download_link` |
| `krillnotes-core/src/core/scripting/mod.rs` | Register `display_image`, `display_download_link`; update `markdown` registration to pass context |
| `krillnotes-desktop/src/types.ts` | Add `File` variant to `FieldValue` |
| `krillnotes-desktop/src/components/InfoPanel.tsx` | DOMPurify ADD_ATTR; useEffect for image hydration and download link handling |
| `krillnotes-desktop/src/components/fields/FileField.tsx` | New component |
| `krillnotes-desktop/src/components/NoteView.tsx` (or equivalent) | Render `FileField` for `"file"` type fields |
