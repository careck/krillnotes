# Schema Attachment Restrictions — Design

**Issue:** #58
**Date:** 2026-03-02

## Summary

Add two schema-level options that control note-level (panel) attachments for a given note type:

- `allow_attachments: bool` — enables the attachments panel (default `false`)
- `attachment_types: [String]` — restricts accepted MIME types (default `[]` = all types allowed)

These options have no effect on field-level `file` fields, which are already configurable per-field via `allowed_types`.

## Approach

Extend the `Schema` struct with the two new fields and propagate them through the existing `SchemaInfo` Tauri response to the frontend. The frontend gates rendering of `AttachmentsSection` on `allowAttachments` and passes `attachmentTypes` as a MIME filter.

This mirrors existing schema-level boolean/array options (`title_can_view`, `allowed_parent_types`, etc.) and requires no new abstractions.

## Architecture

### Rust — `schema.rs`

```rust
pub struct Schema {
    // existing fields ...
    pub allow_attachments: bool,        // default: false
    pub attachment_types: Vec<String>,  // default: [] = all types
}
```

Parsed in `parse_from_rhai` from the Rhai schema map, using the same pattern as `title_can_view`.

### Rust — `SchemaInfo`

`SchemaInfo` (returned by `get_schema_fields`) gains:

```rust
pub allow_attachments: bool,
pub attachment_types: Vec<String>,
```

### Frontend — `types.ts`

```typescript
export interface SchemaInfo {
  // existing fields ...
  allowAttachments: boolean;
  attachmentTypes: string[];
}
```

### Frontend — `InfoPanel.tsx`

Gate `AttachmentsSection` on `schemaInfo.allowAttachments`:

```tsx
{schemaInfo?.allowAttachments && (
  <AttachmentsSection
    noteId={selectedNote?.id ?? null}
    allowedTypes={schemaInfo.attachmentTypes}
  />
)}
```

### Frontend — `AttachmentsSection.tsx`

Accept `allowedTypes: string[]` prop and apply it in:

1. **File picker** — passed as extension filters via `mimeToExtension` (already used in `FileField.tsx`)
2. **Drag-and-drop** — `file.type` checked against `allowedTypes`; mismatched files are rejected with a toast

## Rhai Template API

```rhai
schema("PhotoNote", #{
    allow_attachments: true,
    attachment_types: ["image/jpeg", "image/png", "image/gif"],
    fields: [
        #{ name: "caption", type: "text", required: false },
    ],
});
```

Omitting `allow_attachments` (or setting it to `false`) hides the attachments panel for that note type. Empty `attachment_types` with `allow_attachments: true` accepts all file types.

## Key Decisions

- **Default is `false`** — attachments are opt-in per schema type; all existing schemas that don't specify the option will have the panel hidden.
- **MIME types** — `attachment_types` uses MIME strings (e.g. `"image/jpeg"`), consistent with field-level `allowed_types`.
- **Client-side enforcement only** — MIME filtering is applied in the file picker filter and drag-and-drop handler; no server-side rejection (consistent with field-level file behavior).
- **No server-side changes** to `attach_file` / `attach_file_bytes` commands.
