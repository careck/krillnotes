# Image Embedding Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `file` field type and `{{image: ...}}` markdown embedding with `display_image` / `display_download_link` Rhai helpers.

**Architecture:** Rust pre-processes `{{image: ...}}` blocks and Rhai display helpers into `<img data-kn-attach-id="UUID" />` / `<a data-kn-download-id="UUID" />` sentinel elements. The frontend post-processes those after DOMPurify sanitization, fetching decrypted bytes via the existing `get_attachment_data` Tauri command. The `file` field type stores an attachment UUID via the existing attachments table.

**Security note:** InfoPanel already renders server-generated HTML via `dangerouslySetInnerHTML` protected by DOMPurify. All new attributes (`data-kn-attach-id`, `data-kn-width`, `data-kn-download-id`) are explicitly added to DOMPurify's `ADD_ATTR` allowlist and never contain executable content. No raw user text is ever injected into HTML without `html_escape()`.

**Tech Stack:** Rust (`pulldown-cmark`, `rusqlite`, `serde_json`, `regex`), Rhai scripting engine, React/TypeScript, Tauri v2, DOMPurify.

---

## Task 1: `File` variant in `FieldValue` + `allowed_types` in `FieldDefinition`

**Files:**
- Modify: `krillnotes-core/src/core/note.rs:9-24`
- Modify: `krillnotes-core/src/core/scripting/schema.rs:28-51`
- Modify: `krillnotes-desktop/src/types.ts:28-34`

### Step 1: Add `File` variant to `FieldValue`

In `note.rs` at line 24 (end of the enum, before the closing `}`), add:

```rust
/// A reference to an attachment by UUID. `None` means "not set".
File(Option<String>),
```

The full enum after the change (lines 9-27):
```rust
pub enum FieldValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    Date(Option<NaiveDate>),
    Email(String),
    NoteLink(Option<String>),
    File(Option<String>),   // ← new
}
```

`serde_json` serialization is automatic via the `#[derive(Serialize, Deserialize)]` already on the enum — no other change needed in this file.

### Step 2: Add `allowed_types` to `FieldDefinition`

In `schema.rs`, after the existing `show_on_hover` field (line 50), add:

```rust
#[serde(default)]
pub allowed_types: Vec<String>,   // MIME types; empty = all allowed
```

### Step 3: Add `File` variant to TypeScript `FieldValue`

In `types.ts` at line 34, extend the union:

```typescript
export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean }
  | { Date: string | null }
  | { Email: string }
  | { NoteLink: string | null }
  | { File: string | null };   // ← new; null = not set
```

### Step 4: Write the failing test

Add to `krillnotes-core/src/core/note.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_value_file_roundtrip() {
        let v = FieldValue::File(Some("abc-123".to_string()));
        let json = serde_json::to_string(&v).unwrap();
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        match back {
            FieldValue::File(Some(id)) => assert_eq!(id, "abc-123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_field_value_file_none_roundtrip() {
        let v = FieldValue::File(None);
        let json = serde_json::to_string(&v).unwrap();
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, FieldValue::File(None)));
    }
}
```

### Step 5: Run test to verify it fails

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_field_value_file 2>&1 | tail -20
```

Expected: compile error — `File` variant not yet defined (if doing strict TDD) or PASSED if variant added first.

### Step 6: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all existing tests pass.

### Step 7: Commit

```bash
git add krillnotes-core/src/core/note.rs \
        krillnotes-core/src/core/scripting/schema.rs \
        krillnotes-desktop/src/types.ts
git commit -m "feat: add File FieldValue variant and allowed_types to FieldDefinition"
```

---

## Task 2: Attachment cleanup on `File` field value change in `update_note`

When a `File` field value changes (UUID replaced or cleared), the old attachment must be deleted so no orphaned `.enc` files accumulate.

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` — `update_note` function (~line 1793)

### Step 1: Understand the current `update_note` flow

Read `workspace.rs` lines 1793–1875. Notice:
- Line 1800: fetches `node_type`
- Line 1824–1825: serializes new `fields` to JSON
- Line 1827–1832: runs `UPDATE notes SET fields_json = ...`

We need to insert a cleanup step **between** reading the old fields and writing the new ones.

### Step 2: Write the failing test

Add to `krillnotes-core/src/core/workspace.rs` test section (follow existing test setup patterns):

```rust
#[test]
fn test_update_note_cleans_up_replaced_file_field() {
    let mut ws = create_test_workspace_with_password("").unwrap();
    let note = ws.create_note("MyType", None).unwrap();

    // Attach a first file and set it as the field value
    let meta = ws.attach_file(&note.id, "a.png", Some("image/png"), b"fake").unwrap();
    let mut fields = note.fields.clone();
    fields.insert("photo".to_string(), FieldValue::File(Some(meta.id.clone())));
    ws.update_note(&note.id, note.title.clone(), fields).unwrap();

    // Replace with a different file
    let meta2 = ws.attach_file(&note.id, "b.png", Some("image/png"), b"also_fake").unwrap();
    let updated = ws.get_note(&note.id).unwrap();
    let mut fields2 = updated.fields.clone();
    fields2.insert("photo".to_string(), FieldValue::File(Some(meta2.id.clone())));
    ws.update_note(&note.id, updated.title.clone(), fields2).unwrap();

    // The first attachment should be gone
    let result = ws.get_attachment_bytes(&meta.id);
    assert!(result.is_err(), "old attachment should have been deleted");
}
```

### Step 3: Run to verify it fails

```bash
cargo test -p krillnotes-core test_update_note_cleans_up 2>&1 | tail -20
```

Expected: FAIL — old attachment still exists.

### Step 4: Implement the cleanup

In `update_note`, after schema validation (~line 1821) and before the UPDATE SQL, add:

```rust
// Clean up replaced/cleared File field attachments
{
    let old_fields_json: String = self.storage.connection()
        .query_row(
            "SELECT fields_json FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
            |row| row.get(0),
        )
        .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
    let old_fields: HashMap<String, FieldValue> =
        serde_json::from_str(&old_fields_json).unwrap_or_default();

    for (key, old_val) in &old_fields {
        if let FieldValue::File(Some(old_uuid)) = old_val {
            let still_same = matches!(
                fields.get(key),
                Some(FieldValue::File(Some(u))) if u == old_uuid
            );
            if !still_same {
                let _ = self.delete_attachment(old_uuid); // best-effort
            }
        }
    }
}
```

### Step 5: Run test to verify it passes

```bash
cargo test -p krillnotes-core test_update_note_cleans_up 2>&1 | tail -10
```

### Step 6: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

### Step 7: Commit

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: delete replaced File field attachments in update_note"
```

---

## Task 3: `NoteRunContext` infrastructure in `ScriptRegistry`

Rhai closures for `display_image`, `display_download_link`, and `markdown` need access to the current note's fields and attachments list. The Rhai engine is created once (not per run), so we use a shared `Arc<Mutex<Option<NoteRunContext>>>` that the workspace populates before each script run.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

### Step 1: Add `NoteRunContext` struct and extend `ScriptRegistry`

Near the top of `scripting/mod.rs` (after imports), add:

```rust
use crate::core::attachment::AttachmentMeta;
use crate::core::note::{FieldValue, Note};
use std::sync::{Arc, Mutex};

/// Per-run context injected before executing a Rhai script.
pub struct NoteRunContext {
    pub note: Note,
    pub attachments: Vec<AttachmentMeta>,
}
```

In the `ScriptRegistry` struct definition, add:

```rust
pub run_context: Arc<Mutex<Option<NoteRunContext>>>,
```

### Step 2: Create the `Arc` in `ScriptRegistry::new()` and store it

Near the top of `new()` (before the engine registration block), add:

```rust
let run_context: Arc<Mutex<Option<NoteRunContext>>> = Arc::new(Mutex::new(None));
```

At the end of `new()` where the struct is returned, include `run_context`.

### Step 3: Add context helper methods

```rust
impl ScriptRegistry {
    pub fn set_run_context(&self, note: Note, attachments: Vec<AttachmentMeta>) {
        *self.run_context.lock().expect("run_context poisoned") =
            Some(NoteRunContext { note, attachments });
    }

    pub fn clear_run_context(&self) {
        *self.run_context.lock().expect("run_context poisoned") = None;
    }
}
```

### Step 4: Compile check

```bash
cargo build -p krillnotes-core 2>&1 | grep "^error" | head -20
```

Expected: clean compile.

### Step 5: Commit

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add NoteRunContext infrastructure to ScriptRegistry"
```

---

## Task 4: `resolve_attachment_source` shared helper

This is the shared core used by `preprocess_image_blocks`, `display_image`, and `display_download_link`.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`

### Step 1: Write the failing tests

Add to the bottom of `display_helpers.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::attachment::AttachmentMeta;
    use crate::core::note::FieldValue;
    use std::collections::HashMap;

    fn make_meta(id: &str, filename: &str) -> AttachmentMeta {
        AttachmentMeta {
            id: id.to_string(),
            note_id: "note1".to_string(),
            filename: filename.to_string(),
            mime_type: Some("image/png".to_string()),
            size_bytes: 100,
            hash_sha256: "abc".to_string(),
            salt: "00".repeat(32),
            created_at: 0,
        }
    }

    #[test]
    fn test_resolve_by_filename() {
        let attachments = vec![make_meta("uuid-1", "photo.png")];
        let fields = HashMap::new();
        let result = resolve_attachment_source("attach:photo.png", &fields, &attachments);
        assert_eq!(result.map(|a| a.id.as_str()), Some("uuid-1"));
    }

    #[test]
    fn test_resolve_by_field() {
        let attachments = vec![make_meta("uuid-2", "cover.jpg")];
        let mut fields = HashMap::new();
        fields.insert("cover".to_string(), FieldValue::File(Some("uuid-2".to_string())));
        let result = resolve_attachment_source("field:cover", &fields, &attachments);
        assert_eq!(result.map(|a| a.id.as_str()), Some("uuid-2"));
    }

    #[test]
    fn test_resolve_missing_returns_none() {
        let attachments = vec![];
        let fields = HashMap::new();
        assert!(resolve_attachment_source("attach:missing.png", &fields, &attachments).is_none());
    }

    #[test]
    fn test_resolve_field_not_set_returns_none() {
        let mut fields = HashMap::new();
        fields.insert("cover".to_string(), FieldValue::File(None));
        assert!(resolve_attachment_source("field:cover", &fields, &[]).is_none());
    }
}
```

### Step 2: Run to verify it fails

```bash
cargo test -p krillnotes-core test_resolve 2>&1 | tail -20
```

Expected: compile error — function not yet defined.

### Step 3: Implement `resolve_attachment_source`

Add near the top of `display_helpers.rs` (after imports):

```rust
use crate::core::attachment::AttachmentMeta;
use crate::core::note::FieldValue;

/// Resolve an image/file source string to an AttachmentMeta.
///
/// Source formats:
///   "attach:<filename>" — search attachments by filename (first match)
///   "field:<fieldName>"  — read note.fields[fieldName] as FieldValue::File(uuid)
pub fn resolve_attachment_source<'a>(
    source: &str,
    fields: &HashMap<String, FieldValue>,
    attachments: &'a [AttachmentMeta],
) -> Option<&'a AttachmentMeta> {
    if let Some(filename) = source.strip_prefix("attach:") {
        attachments.iter().find(|a| a.filename == filename)
    } else if let Some(field_name) = source.strip_prefix("field:") {
        if let Some(FieldValue::File(Some(uuid))) = fields.get(field_name) {
            attachments.iter().find(|a| &a.id == uuid)
        } else {
            None
        }
    } else {
        None
    }
}
```

### Step 4: Run tests to verify they pass

```bash
cargo test -p krillnotes-core test_resolve 2>&1 | tail -10
```

### Step 5: Commit

```bash
git add krillnotes-core/src/core/scripting/display_helpers.rs
git commit -m "feat: add resolve_attachment_source helper"
```

---

## Task 5: `preprocess_image_blocks` for `{{image: ...}}` syntax

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`
- Maybe modify: `krillnotes-core/Cargo.toml` (add `regex` crate if absent)

### Step 1: Check for `regex` crate

```bash
grep "regex" krillnotes-core/Cargo.toml
```

If absent, add to `[dependencies]`:

```toml
regex = "1"
```

### Step 2: Write the failing tests

Add to the `tests` module in `display_helpers.rs`:

```rust
#[test]
fn test_preprocess_basic_attach() {
    let attachments = vec![make_meta("uuid-1", "photo.png")];
    let fields = HashMap::new();
    let result = preprocess_image_blocks(
        "Before {{image: attach:photo.png}} After",
        &fields,
        &attachments,
    );
    assert!(result.contains("data-kn-attach-id=\"uuid-1\""), "got: {result}");
    assert!(result.contains("Before"));
    assert!(result.contains("After"));
}

#[test]
fn test_preprocess_with_width_and_alt() {
    let attachments = vec![make_meta("uuid-2", "cover.jpg")];
    let fields = HashMap::new();
    let result = preprocess_image_blocks(
        "{{image: attach:cover.jpg, width: 300, alt: My cover}}",
        &fields,
        &attachments,
    );
    assert!(result.contains("data-kn-attach-id=\"uuid-2\""));
    assert!(result.contains("data-kn-width=\"300\""));
    assert!(result.contains("alt=\"My cover\""));
}

#[test]
fn test_preprocess_unresolvable_shows_error() {
    let fields = HashMap::new();
    let result = preprocess_image_blocks("{{image: attach:missing.png}}", &fields, &[]);
    assert!(result.contains("kn-image-error"), "got: {result}");
}

#[test]
fn test_preprocess_field_source() {
    let mut fields = HashMap::new();
    fields.insert("cover".to_string(), FieldValue::File(Some("uuid-3".to_string())));
    let attachments = vec![make_meta("uuid-3", "cover.jpg")];
    let result = preprocess_image_blocks("{{image: field:cover, width: 200}}", &fields, &attachments);
    assert!(result.contains("data-kn-attach-id=\"uuid-3\""));
    assert!(result.contains("data-kn-width=\"200\""));
}
```

### Step 3: Run to verify they fail

```bash
cargo test -p krillnotes-core test_preprocess 2>&1 | tail -20
```

### Step 4: Implement `preprocess_image_blocks` and helpers

Add to `display_helpers.rs`:

```rust
use regex::Regex;

/// Pre-process {{image: ...}} blocks in markdown text.
/// Each block is replaced with an <img data-kn-attach-id="UUID" ...> sentinel
/// for frontend hydration. No bytes are loaded here — the frontend fetches them.
pub fn preprocess_image_blocks(
    text: &str,
    fields: &HashMap<String, FieldValue>,
    attachments: &[AttachmentMeta],
) -> String {
    let re = Regex::new(r"\{\{image:\s*([^}]*)\}\}").expect("valid regex");

    re.replace_all(text, |caps: &regex::Captures| {
        let inner = caps[1].trim();
        let (source, opts) = parse_image_block(inner);

        match resolve_attachment_source(source, fields, attachments) {
            Some(meta) => {
                let width_attr = opts.get("width")
                    .map(|w| format!(" data-kn-width=\"{}\"", w))
                    .unwrap_or_default();
                let alt_attr = opts.get("alt")
                    .map(|a| format!(" alt=\"{}\"", html_escape(a)))
                    .unwrap_or_default();
                format!(
                    "<img data-kn-attach-id=\"{}\"{}{}  class=\"kn-image-embed\" />",
                    meta.id, width_attr, alt_attr
                )
            }
            None => format!(
                "<span class=\"kn-image-error\">Image not found: {}</span>",
                html_escape(source)
            ),
        }
    }).into_owned()
}

/// Parse "attach:photo.png, width: 200, alt: My caption"
/// into (source, {width: "200", alt: "My caption"}).
fn parse_image_block(inner: &str) -> (&str, HashMap<&str, &str>) {
    let mut parts = inner.splitn(2, ',');
    let source = parts.next().unwrap_or("").trim();
    let mut opts = HashMap::new();
    if let Some(rest) = parts.next() {
        for kv in rest.split(',') {
            if let Some((k, v)) = kv.split_once(':') {
                opts.insert(k.trim(), v.trim());
            }
        }
    }
    (source, opts)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}
```

### Step 5: Run tests to verify they pass

```bash
cargo test -p krillnotes-core test_preprocess 2>&1 | tail -10
```

### Step 6: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

### Step 7: Commit

```bash
git add krillnotes-core/src/core/scripting/display_helpers.rs \
        krillnotes-core/Cargo.toml
git commit -m "feat: add preprocess_image_blocks for {{image: ...}} syntax"
```

---

## Task 6: Update `markdown()` Rhai function to use run context

Replace the static `engine.register_fn("markdown", display_helpers::rhai_markdown)` with a closure that pre-processes image blocks when run context is available.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (line ~475)
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`
- Modify: `krillnotes-core/src/core/workspace.rs` (hook call sites)

### Step 1: Rename the existing `rhai_markdown` to `rhai_markdown_raw`

In `display_helpers.rs`, rename the function so it can be called from the closure:

```rust
pub fn rhai_markdown_raw(text: String) -> String {
    format!("<div class=\"kn-view-markdown\">{}</div>", render_markdown_to_html(&text))
}
```

Add an alias to avoid breaking any existing call sites:

```rust
pub fn rhai_markdown(text: String) -> String {
    rhai_markdown_raw(text)
}
```

### Step 2: Replace the Rhai `markdown` registration in `mod.rs`

Find line ~475:

```rust
engine.register_fn("markdown", display_helpers::rhai_markdown);
```

Replace with (using the `run_context` Arc created in Task 3):

```rust
let ctx_for_markdown = Arc::clone(&run_context);
engine.register_fn("markdown", move |text: String| -> String {
    let guard = ctx_for_markdown.lock().expect("run_context poisoned");
    let processed = if let Some(ref ctx) = *guard {
        display_helpers::preprocess_image_blocks(&text, &ctx.note.fields, &ctx.attachments)
    } else {
        text
    };
    drop(guard);
    display_helpers::rhai_markdown_raw(processed)
});
```

### Step 3: Find hook runner call sites and wrap with context

Search for where `on_view` and `on_hover` scripts are executed:

```bash
grep -rn "run_on_view\|run_on_hover\|on_view\|on_hover" \
  krillnotes-core/src/core/ | grep -v "\.rhai\|test\|//\|schema" | head -30
```

In each call site (likely in `workspace.rs`), wrap the script execution with context:

```rust
let attachments = self.get_attachments(&note.id).unwrap_or_default();
self.script_registry.set_run_context(note.clone(), attachments);
let result = self.script_registry.run_on_view_hook(/* ... */);
self.script_registry.clear_run_context();
result
```

If the hook runner lives inside `scripting/mod.rs` without workspace access, move the context setup to the workspace layer that calls it.

### Step 4: Compile check

```bash
cargo build -p krillnotes-core 2>&1 | grep "^error" | head -20
```

### Step 5: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

### Step 6: Commit

```bash
git add krillnotes-core/src/core/scripting/mod.rs \
        krillnotes-core/src/core/scripting/display_helpers.rs \
        krillnotes-core/src/core/workspace.rs
git commit -m "feat: markdown() Rhai helper now pre-processes {{image:}} blocks"
```

---

## Task 7: `display_image` and `display_download_link` Rhai helpers

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

### Step 1: Write the failing tests

Add to the `tests` module in `display_helpers.rs`:

```rust
#[test]
fn test_display_image_basic() {
    let attachments = vec![make_meta("uuid-5", "hero.jpg")];
    let fields = HashMap::new();
    let html = make_display_image_html("attach:hero.jpg", 400, "Hero", &fields, &attachments);
    assert!(html.contains("data-kn-attach-id=\"uuid-5\""));
    assert!(html.contains("data-kn-width=\"400\""));
    assert!(html.contains("alt=\"Hero\""));
}

#[test]
fn test_display_image_zero_width_omits_attr() {
    let attachments = vec![make_meta("uuid-6", "x.png")];
    let fields = HashMap::new();
    let html = make_display_image_html("attach:x.png", 0, "", &fields, &attachments);
    assert!(!html.contains("data-kn-width"));
}

#[test]
fn test_display_download_link_with_label() {
    let attachments = vec![make_meta("uuid-7", "report.pdf")];
    let fields = HashMap::new();
    let html = make_download_link_html("attach:report.pdf", "Download PDF", &fields, &attachments);
    assert!(html.contains("data-kn-download-id=\"uuid-7\""));
    assert!(html.contains("Download PDF"));
}

#[test]
fn test_display_download_link_empty_label_uses_filename() {
    let attachments = vec![make_meta("uuid-8", "data.csv")];
    let fields = HashMap::new();
    let html = make_download_link_html("attach:data.csv", "", &fields, &attachments);
    assert!(html.contains("data-kn-download-id=\"uuid-8\""));
    assert!(html.contains("data.csv"));
}
```

### Step 2: Run to verify they fail

```bash
cargo test -p krillnotes-core test_display_image test_display_download 2>&1 | tail -20
```

### Step 3: Implement the pure helper functions

```rust
pub fn make_display_image_html(
    source: &str,
    width: i64,
    alt: &str,
    fields: &HashMap<String, FieldValue>,
    attachments: &[AttachmentMeta],
) -> String {
    match resolve_attachment_source(source, fields, attachments) {
        Some(meta) => {
            let width_attr = if width > 0 {
                format!(" data-kn-width=\"{}\"", width)
            } else {
                String::new()
            };
            let alt_attr = if !alt.is_empty() {
                format!(" alt=\"{}\"", html_escape(alt))
            } else {
                String::new()
            };
            format!(
                "<img data-kn-attach-id=\"{}\"{}{}  class=\"kn-image-embed\" />",
                meta.id, width_attr, alt_attr
            )
        }
        None => format!(
            "<span class=\"kn-image-error\">Image not found: {}</span>",
            html_escape(source)
        ),
    }
}

pub fn make_download_link_html(
    source: &str,
    label: &str,
    fields: &HashMap<String, FieldValue>,
    attachments: &[AttachmentMeta],
) -> String {
    match resolve_attachment_source(source, fields, attachments) {
        Some(meta) => {
            let display = if label.is_empty() { meta.filename.as_str() } else { label };
            format!(
                "<a data-kn-download-id=\"{}\" class=\"kn-download-link\">{}</a>",
                meta.id,
                html_escape(display)
            )
        }
        None => format!(
            "<span class=\"kn-image-error\">File not found: {}</span>",
            html_escape(source)
        ),
    }
}
```

### Step 4: Register in Rhai engine (`mod.rs`)

After the `markdown` closure registration, add:

```rust
let ctx_for_display_image = Arc::clone(&run_context);
engine.register_fn("display_image", move |source: String, width: i64, alt: String| -> String {
    let guard = ctx_for_display_image.lock().expect("run_context poisoned");
    if let Some(ref ctx) = *guard {
        let result = display_helpers::make_display_image_html(
            &source, width, &alt, &ctx.note.fields, &ctx.attachments
        );
        drop(guard);
        result
    } else {
        drop(guard);
        "<span class=\"kn-image-error\">No note context</span>".to_string()
    }
});

let ctx_for_download_link = Arc::clone(&run_context);
engine.register_fn("display_download_link", move |source: String, label: String| -> String {
    let guard = ctx_for_download_link.lock().expect("run_context poisoned");
    if let Some(ref ctx) = *guard {
        let result = display_helpers::make_download_link_html(
            &source, &label, &ctx.note.fields, &ctx.attachments
        );
        drop(guard);
        result
    } else {
        drop(guard);
        "<span class=\"kn-image-error\">No note context</span>".to_string()
    }
});
```

### Step 5: Run tests to verify they pass

```bash
cargo test -p krillnotes-core test_display_image test_display_download 2>&1 | tail -10
```

### Step 6: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

### Step 7: Commit

```bash
git add krillnotes-core/src/core/scripting/display_helpers.rs \
        krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add display_image and display_download_link Rhai helpers"
```

---

## Task 8: Frontend — `FileField` component and field dispatch

**Files:**
- Create: `krillnotes-desktop/src/components/FileField.tsx`
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx`
- Modify: `krillnotes-desktop/src/components/FieldEditor.tsx`

### Step 1: Read relevant files first

Read these files in full before writing any code:
- `krillnotes-desktop/src/components/FieldDisplay.tsx`
- `krillnotes-desktop/src/components/FieldEditor.tsx`
- `krillnotes-desktop/src/components/AttachmentsSection.tsx` (for `isImageMime` pattern)

### Step 2: Create `FileField.tsx`

```tsx
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { AttachmentMeta, FieldValue } from '../types';
import { useEffect, useState } from 'react';
import { Paperclip, X } from 'lucide-react';

interface FileFieldProps {
  attachmentId: string | null;
  allowedTypes: string[];        // MIME types; empty = all
  isEditing: boolean;
  noteId: string;
  onValueChange: (newValue: FieldValue) => void;
}

function isImageMime(mime: string | null): boolean {
  return !!mime?.startsWith('image/');
}

export function FileField({
  attachmentId, allowedTypes, isEditing, noteId, onValueChange
}: FileFieldProps) {
  const [meta, setMeta] = useState<AttachmentMeta | null>(null);
  const [thumbSrc, setThumbSrc] = useState<string | null>(null);

  useEffect(() => {
    if (!attachmentId) { setMeta(null); setThumbSrc(null); return; }
    invoke<AttachmentMeta[]>('get_attachments', { noteId })
      .then(list => {
        const found = list.find(a => a.id === attachmentId) ?? null;
        setMeta(found);
        if (found && isImageMime(found.mimeType)) {
          invoke<string>('get_attachment_data', { attachmentId: found.id })
            .then(b64 => setThumbSrc(`data:${found.mimeType};base64,${b64}`));
        }
      });
  }, [attachmentId, noteId]);

  async function handlePick() {
    const filters = allowedTypes.length > 0
      ? [{ name: 'Allowed files', extensions: allowedTypes.map(m => m.split('/')[1]) }]
      : [];
    const selected = await open({ multiple: false, filters });
    if (!selected || typeof selected !== 'string') return;

    const filename = selected.split(/[/\\]/).pop() ?? 'file';
    const { readFile } = await import('@tauri-apps/plugin-fs');
    const bytes = Array.from(await readFile(selected));

    if (attachmentId) {
      await invoke('delete_attachment', { attachmentId });
    }

    const newMeta = await invoke<AttachmentMeta>('attach_file_bytes', {
      noteId, filename, data: bytes,
    });
    onValueChange({ File: newMeta.id });
  }

  async function handleClear() {
    if (attachmentId) {
      await invoke('delete_attachment', { attachmentId });
    }
    onValueChange({ File: null });
  }

  if (!isEditing) {
    if (!meta) return <span className="text-muted-foreground text-sm">—</span>;
    return (
      <div className="flex items-center gap-2">
        {thumbSrc
          ? <img src={thumbSrc} alt={meta.filename} className="w-10 h-10 object-cover rounded" />
          : <Paperclip className="w-4 h-4 text-muted-foreground" />}
        <span className="text-sm">{meta.filename}</span>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      {meta && (
        <div className="flex items-center gap-1 text-sm">
          {thumbSrc && (
            <img src={thumbSrc} alt={meta.filename} className="w-8 h-8 object-cover rounded" />
          )}
          <span>{meta.filename}</span>
          <button onClick={handleClear} className="text-muted-foreground hover:text-destructive">
            <X className="w-3 h-3" />
          </button>
        </div>
      )}
      <button onClick={handlePick} className="text-xs underline text-muted-foreground">
        {meta ? 'Replace…' : 'Choose file…'}
      </button>
    </div>
  );
}
```

### Step 3: Update `FieldDisplay.tsx` for `File` variant

In the `renderValue()` function, add a branch for `File` (after the `NoteLink` branch):

```tsx
else if ('File' in value) {
  return (
    <FileField
      attachmentId={value.File}
      allowedTypes={[]}
      isEditing={false}
      noteId={noteId}
      onValueChange={() => {}}
    />
  );
}
```

Check whether `noteId` is already a prop on `FieldDisplay`. If not, add it:
```tsx
interface FieldDisplayProps {
  // ... existing props
  noteId: string;
}
```

And update all call sites of `FieldDisplay` to pass `noteId`.

### Step 4: Update `FieldEditor.tsx` for `file` field type

In the field type dispatch (read the existing pattern first), add before the default case:

```tsx
if (fieldDef.fieldType === 'file') {
  const currentId = 'File' in value ? (value as { File: string | null }).File : null;
  return (
    <FileField
      attachmentId={currentId}
      allowedTypes={fieldDef.allowedTypes ?? []}
      isEditing={true}
      noteId={noteId}
      onValueChange={onChange}
    />
  );
}
```

### Step 5: TypeScript build check

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep -E "error TS" | head -20
```

Fix any type errors before proceeding.

### Step 6: Commit

```bash
git add krillnotes-desktop/src/components/FileField.tsx \
        krillnotes-desktop/src/components/FieldDisplay.tsx \
        krillnotes-desktop/src/components/FieldEditor.tsx
git commit -m "feat: FileField component for file field type"
```

---

## Task 9: Frontend — image hydration and download link handling in `InfoPanel`

All HTML rendered in InfoPanel (whether from default view or Rhai scripts) is already sanitized by DOMPurify. We extend it to allow our sentinel attributes and post-process them client-side.

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

### Step 1: Read `InfoPanel.tsx` in full before editing

Note the locations of: DOMPurify.sanitize call (~line 349), click handler (~lines 353-366), customViewHtml useEffect.

### Step 2: Update DOMPurify allowlist

On line ~349, update `ADD_ATTR` to include the new sentinel attributes:

```tsx
DOMPurify.sanitize(customViewHtml, {
  ADD_ATTR: ['data-note-id', 'data-kn-attach-id', 'data-kn-width', 'data-kn-download-id'],
})
```

### Step 3: Add a `useRef` on the rendered HTML container

Add near the top of the component:

```tsx
const viewHtmlRef = useRef<HTMLDivElement>(null);
```

Add `ref={viewHtmlRef}` to the `<div>` that has `dangerouslySetInnerHTML`.

### Step 4: Add `useEffect` for image hydration

After the existing `useEffect` calls, add:

```tsx
useEffect(() => {
  const container = viewHtmlRef.current;
  if (!container || !customViewHtml) return;

  const imgs = Array.from(
    container.querySelectorAll<HTMLImageElement>('img[data-kn-attach-id]')
  );
  Promise.all(
    imgs.map(async (img) => {
      const attachmentId = img.getAttribute('data-kn-attach-id')!;
      const widthAttr = img.getAttribute('data-kn-width');
      try {
        const b64 = await invoke<string>('get_attachment_data', { attachmentId });
        // Find MIME type from the loaded attachments metadata if available,
        // otherwise fall back to a generic type (browser still renders images)
        img.src = `data:image/octet-stream;base64,${b64}`;
        if (widthAttr && parseInt(widthAttr) > 0) {
          img.style.maxWidth = `${widthAttr}px`;
        }
        img.removeAttribute('data-kn-attach-id');
      } catch {
        const span = document.createElement('span');
        span.className = 'kn-image-error';
        span.textContent = 'Image not found';
        img.replaceWith(span);
      }
    })
  );
}, [customViewHtml]);
```

### Step 5: Handle download link clicks

In the existing click handler (lines ~353-366), add a branch **before** the note link check:

```tsx
const downloadLink = target.closest('[data-kn-download-id]');
if (downloadLink) {
  e.preventDefault();
  const attachmentId = downloadLink.getAttribute('data-kn-download-id')!;
  const filename = downloadLink.textContent?.trim() ?? 'download';
  invoke<string>('get_attachment_data', { attachmentId })
    .then(b64 => {
      const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
      const blob = new Blob([bytes]);
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = filename;
      a.click();
      URL.revokeObjectURL(url);
    })
    .catch(err => alert(String(err)));
  return;
}
```

### Step 6: TypeScript build check

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep -E "error TS" | head -20
```

### Step 7: Commit

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: hydrate image embeds and handle download links in InfoPanel"
```

---

## Task 10: Manual end-to-end testing

No automated tests cover the full Tauri integration. Verify manually:

1. **File field — basic:**
   - Edit a note type schema to add a `file` field (type: `"file"`).
   - In a note of that type, click "Choose file…" and pick an image.
   - Verify thumbnail appears in both edit and view mode.
   - Click "Replace…" with a different file — verify old `.enc` file is removed from `<workspace>/attachments/`.

2. **File field — allowed_types:**
   - Set `allowed_types: ["image/png"]` in the schema.
   - Try picking a `.pdf` — file picker should filter it out.

3. **`{{image: ...}}` in textarea:**
   - Attach an image via AttachmentsSection.
   - In a textarea field, type: `{{image: attach:yourfilename.png, width: 300}}`
   - Switch to view mode — image renders at max 300px.

4. **`{{image: field:xxx}}` in textarea:**
   - Set a `file` field to an image, then embed `{{image: field:photo}}` in a textarea.
   - View mode — image renders.

5. **`display_image` and `display_download_link` in Rhai:**
   - Write an `on_view` script using:
     ```rhai
     display_image("field:photo", 200, "My photo")
     display_download_link("attach:report.pdf", "Download report")
     ```
   - Verify image renders and download triggers a file save.

6. **Export/import round-trip:**
   - Export workspace to `.krillnotes`, re-import into a fresh workspace.
   - Verify file fields and embedded images still resolve correctly.

### Final commit

```bash
git add -A
git commit -m "feat: image embedding — file field type, {{image:}} syntax, display helpers (#41)"
```

---

## Implementation Notes

- **`parse_image_block` edge case:** Alt text containing a comma (e.g., `alt: Hello, world`) will be truncated by `split(',')`. Acceptable for v1 — a smarter parser is a follow-up.
- **MIME type in data URL (Task 9):** `data:image/octet-stream` works for display. To use the correct MIME type, call `get_attachments(noteId)` and match by ID. This can be a follow-up improvement.
- **`@tauri-apps/plugin-fs` in FileField:** Check `src-tauri/capabilities/default.json` for `"plugin:fs|read-files"` or similar. If absent, add it.
- **Rhai lock ordering:** Always `drop(guard)` before calling any function that might re-acquire `run_context`. The current structure (drop before calling helper) is correct.
- **`allowed_types` in schema editor UI:** If a schema editor exists in the frontend, add an input for `allowedTypes` on `file` fields. If schema is edited only via `.rhai` files, no frontend change is needed.
