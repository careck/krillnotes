# Display Helpers Redesign Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the global-`run_context`-based `display_image`/`display_download_link` Rhai helpers with a UUID-first API and expose `get_attachments(note_id)` so scripts can safely iterate child notes.

**Architecture:** Add `attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>>` to `QueryContext` (same pre-build pattern as `children_by_id`, `notes_by_type`, etc.), populated via one `list_all_attachments()` call before each hook. Register `get_attachments(note_id)` to look this index up. Simplify `display_image(uuid, width, alt)` and `display_download_link(uuid, label)` to pure HTML generators that take a UUID `Dynamic` directly — no storage access, no `Arc` capture.

**Tech Stack:** Rust, Rhai scripting engine, rusqlite.

---

## Task 1: Add `attachments_by_note_id` to `QueryContext`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs:40-49` (struct definition)
- Modify: `krillnotes-core/src/core/scripting/mod.rs:2237-2243` (`make_empty_ctx` test helper)
- Modify: `krillnotes-core/src/core/workspace.rs:1053` (on_view context build)
- Modify: `krillnotes-core/src/core/workspace.rs:1112` (on_hover context build)
- Modify: `krillnotes-core/src/core/workspace.rs:1170` (tree action context build)

### Step 1: Add field to `QueryContext` struct

In `mod.rs` at line 48 (after `notes_by_link_target`), add:

```rust
/// Maps each note ID to its attachments, pre-built for O(1) script-time look-up.
pub attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>>,
```

The `AttachmentMeta` import is already present at line 17.

### Step 2: Update `make_empty_ctx()` in tests

In `mod.rs` at line 2237, add the new field to the struct literal:

```rust
fn make_empty_ctx() -> QueryContext {
    QueryContext {
        notes_by_id:              Default::default(),
        children_by_id:           Default::default(),
        notes_by_type:            Default::default(),
        notes_by_tag:             Default::default(),
        notes_by_link_target:     Default::default(),
        attachments_by_note_id:   Default::default(),
    }
}
```

### Step 3: Compile check (expect errors on the three workspace.rs build sites)

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding
cargo build -p krillnotes-core 2>&1 | grep "^error" | head -20
```

Expected: three "missing field `attachments_by_note_id`" errors in `workspace.rs`.

### Step 4: Populate `attachments_by_note_id` at all three `QueryContext` build sites

At each of the three lines in `workspace.rs` (1053, 1112, 1170), add the following
**before** the `let context = QueryContext { ... }` line:

```rust
let attachments_by_note_id: HashMap<String, Vec<crate::core::attachment::AttachmentMeta>> = {
    let mut map = HashMap::new();
    for att in self.list_all_attachments().unwrap_or_default() {
        map.entry(att.note_id.clone()).or_default().push(att);
    }
    map
};
```

Then add `attachments_by_note_id,` to each `QueryContext { ... }` struct literal.

### Step 5: Compile check — expect clean

```bash
cargo build -p krillnotes-core 2>&1 | grep "^error" | head -20
```

Expected: no errors.

### Step 6: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all existing tests pass.

### Step 7: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  add krillnotes-core/src/core/scripting/mod.rs \
      krillnotes-core/src/core/workspace.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  commit -m "feat: add attachments_by_note_id index to QueryContext"
```

---

## Task 2: Register `get_attachments(note_id)` Rhai function

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (add registration after `get_notes_with_link`, ~line 336)

### Step 1: Write the failing Rhai test

Add to the `#[cfg(test)]` section in `mod.rs` (near the other `test_on_view_*` tests):

```rust
#[test]
fn test_get_attachments_returns_array_of_maps() {
    use crate::core::attachment::AttachmentMeta;

    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("PhotoNote", #{
            fields: [],
            on_view: |note| {
                let atts = get_attachments(note.id);
                if atts.len() == 0 { return text("none"); }
                let first = atts[0];
                text(first.id + "|" + first.filename)
            }
        });
    "#, "test_script").unwrap();

    let note = crate::core::note::Note {
        id: "note-1".to_string(),
        node_type: "PhotoNote".to_string(),
        title: "Test".to_string(),
        parent_id: None,
        fields: Default::default(),
        tags: vec![],
        created_at: 0,
        updated_at: 0,
        position: 0,
    };

    let mut ctx = make_empty_ctx();
    ctx.attachments_by_note_id.insert(
        "note-1".to_string(),
        vec![AttachmentMeta {
            id: "att-uuid-1".to_string(),
            note_id: "note-1".to_string(),
            filename: "photo.png".to_string(),
            mime_type: Some("image/png".to_string()),
            size_bytes: 100,
            hash_sha256: "abc".to_string(),
            salt: "00".repeat(32),
            created_at: 0,
        }],
    );

    let html = registry.run_on_view_hook(&note, ctx).unwrap().unwrap();
    assert!(html.contains("att-uuid-1"), "got: {html}");
    assert!(html.contains("photo.png"), "got: {html}");
}
```

### Step 2: Run to verify it fails

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding
cargo test -p krillnotes-core test_get_attachments_returns_array 2>&1 | tail -20
```

Expected: FAIL — `get_attachments` not registered / returns empty.

### Step 3: Register `get_attachments` in `mod.rs`

After the `get_notes_with_link` registration block (~line 336), add:

```rust
// Register get_attachments(note_id) — returns attachment metadata for a note.
let qc_for_atts = Arc::clone(&query_context);
engine.register_fn("get_attachments", move |note_id: String| -> rhai::Array {
    let guard = qc_for_atts.lock().unwrap();
    guard.as_ref()
        .and_then(|ctx| ctx.attachments_by_note_id.get(&note_id).cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|att| {
            let mut m = rhai::Map::new();
            m.insert("id".into(),        Dynamic::from(att.id));
            m.insert("filename".into(),  Dynamic::from(att.filename));
            m.insert("mime_type".into(), att.mime_type
                .map(Dynamic::from)
                .unwrap_or(Dynamic::UNIT));
            m.insert("size_bytes".into(), Dynamic::from(att.size_bytes));
            Dynamic::from(m)
        })
        .collect()
});
```

### Step 4: Run test to verify it passes

```bash
cargo test -p krillnotes-core test_get_attachments_returns_array 2>&1 | tail -10
```

Expected: PASS.

### Step 5: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

### Step 6: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  add krillnotes-core/src/core/scripting/mod.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  commit -m "feat: expose get_attachments(note_id) to Rhai"
```

---

## Task 3: Simplify `make_display_image_html` and `make_download_link_html`

Drop `fields` and `attachments` parameters — these functions now take a resolved UUID directly.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs:523-582`

### Step 1: Replace `make_display_image_html`

Replace lines 523–551 with:

```rust
/// Renders an `<img data-kn-attach-id>` sentinel for a resolved UUID.
/// Returns a `kn-image-error` span when `uuid` is empty.
pub fn make_display_image_html(uuid: &str, width: i64, alt: &str) -> String {
    if uuid.is_empty() {
        return "<span class=\"kn-image-error\">No image set</span>".to_string();
    }
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
        "<img data-kn-attach-id=\"{}\"{}{} class=\"kn-image-embed\" />",
        uuid, width_attr, alt_attr
    )
}
```

### Step 2: Replace `make_download_link_html`

Replace lines 562–581 with:

```rust
/// Renders a `<a data-kn-download-id>` sentinel for a resolved UUID.
/// Returns a `kn-image-error` span when `uuid` is empty.
pub fn make_download_link_html(uuid: &str, label: &str) -> String {
    if uuid.is_empty() {
        return "<span class=\"kn-image-error\">No file set</span>".to_string();
    }
    format!(
        "<a data-kn-download-id=\"{}\" class=\"kn-download-link\">{}</a>",
        uuid,
        html_escape(label)
    )
}
```

### Step 3: Update the existing unit tests for these functions

The six tests starting at line 1091 test the old source-string API. Replace them:

```rust
// ── make_display_image_html tests ─────────────────────────────────────────

#[test]
fn test_display_image_basic() {
    let html = make_display_image_html("uuid-5", 400, "Hero");
    assert!(html.contains("data-kn-attach-id=\"uuid-5\""));
    assert!(html.contains("data-kn-width=\"400\""));
    assert!(html.contains("alt=\"Hero\""));
}

#[test]
fn test_display_image_zero_width_omits_attr() {
    let html = make_display_image_html("uuid-6", 0, "");
    assert!(!html.contains("data-kn-width"));
}

#[test]
fn test_display_image_empty_uuid_shows_error() {
    let html = make_display_image_html("", 0, "");
    assert!(html.contains("kn-image-error"), "got: {html}");
}

// ── make_download_link_html tests ─────────────────────────────────────────

#[test]
fn test_display_download_link_with_label() {
    let html = make_download_link_html("uuid-7", "Download PDF");
    assert!(html.contains("data-kn-download-id=\"uuid-7\""));
    assert!(html.contains("Download PDF"));
}

#[test]
fn test_display_download_link_empty_uuid_shows_error() {
    let html = make_download_link_html("", "label");
    assert!(html.contains("kn-image-error"), "got: {html}");
}
```

Delete the three tests that no longer apply: `test_display_image_not_found_shows_error`, `test_display_download_link_empty_label_uses_filename`, `test_display_download_link_not_found_shows_error`.

### Step 4: Compile check (expect errors in mod.rs call sites)

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding
cargo build -p krillnotes-core 2>&1 | grep "^error" | head -20
```

Expected: errors at the two `display_image` / `display_download_link` call sites in `mod.rs`.
These will be fixed in Task 4.

### Step 5: Run just display_helpers tests to verify

```bash
cargo test -p krillnotes-core test_display 2>&1 | tail -10
```

Expected: the five new tests pass.

### Step 6: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  add krillnotes-core/src/core/scripting/display_helpers.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  commit -m "refactor: simplify make_display_image_html/make_download_link_html to take UUID directly"
```

---

## Task 4: Rewrite `display_image` and `display_download_link` Rhai closures

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs:504-532`

### Step 1: Write the failing Rhai integration tests

Add to the `#[cfg(test)]` section in `mod.rs`. Use `run_on_view_hook` as the harness
(the `engine` field is private):

```rust
#[test]
fn test_rhai_display_image_with_uuid_in_field() {
    use crate::core::note::{FieldValue, Note};

    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("PhotoNote", #{
            fields: [#{ name: "photo", type: "file", required: false }],
            on_view: |note| {
                display_image(note.fields["photo"], 300, "My alt")
            }
        });
    "#, "test_script").unwrap();

    let mut fields = std::collections::HashMap::new();
    fields.insert("photo".to_string(), FieldValue::File(Some("abc-uuid-123".to_string())));
    let note = Note {
        id: "n1".to_string(), node_type: "PhotoNote".to_string(),
        title: "T".to_string(), parent_id: None, fields, tags: vec![],
        created_at: 0, updated_at: 0, position: 0,
    };

    registry.set_run_context(note.clone(), vec![]);
    let html = registry.run_on_view_hook(&note, make_empty_ctx()).unwrap().unwrap();
    registry.clear_run_context();

    assert!(html.contains("data-kn-attach-id=\"abc-uuid-123\""), "got: {html}");
    assert!(html.contains("data-kn-width=\"300\""), "got: {html}");
}

#[test]
fn test_rhai_display_image_unset_field_shows_error() {
    use crate::core::note::{FieldValue, Note};

    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("PhotoNote", #{
            fields: [#{ name: "photo", type: "file", required: false }],
            on_view: |note| {
                display_image(note.fields["photo"], 0, "")
            }
        });
    "#, "test_script").unwrap();

    let mut fields = std::collections::HashMap::new();
    fields.insert("photo".to_string(), FieldValue::File(None));
    let note = Note {
        id: "n2".to_string(), node_type: "PhotoNote".to_string(),
        title: "T".to_string(), parent_id: None, fields, tags: vec![],
        created_at: 0, updated_at: 0, position: 0,
    };

    let html = registry.run_on_view_hook(&note, make_empty_ctx()).unwrap().unwrap();
    assert!(html.contains("kn-image-error"), "got: {html}");
}
```

### Step 2: Run to verify tests fail

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding
cargo test -p krillnotes-core test_rhai_display 2>&1 | tail -20
```

Expected: compile errors — old `(String, i64, String)` no longer matches new signature.

### Step 3: Replace the two closures in `mod.rs` (lines 504–532)

Delete the `ctx_for_display_image` block (lines 504–517) and the `ctx_for_download_link` block
(lines 519–532). Replace both with:

```rust
engine.register_fn("display_image", |uuid: Dynamic, width: i64, alt: String| -> String {
    match uuid.into_string() {
        Ok(id) if !id.is_empty() => display_helpers::make_display_image_html(&id, width, &alt),
        _ => "<span class=\"kn-image-error\">No image set</span>".to_string(),
    }
});

engine.register_fn("display_download_link", |uuid: Dynamic, label: String| -> String {
    match uuid.into_string() {
        Ok(id) if !id.is_empty() => display_helpers::make_download_link_html(&id, &label),
        _ => "<span class=\"kn-image-error\">No file set</span>".to_string(),
    }
});
```

### Step 4: Run tests to verify they pass

```bash
cargo test -p krillnotes-core test_rhai_display 2>&1 | tail -10
```

Expected: PASS.

### Step 5: Run full test suite

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all tests pass.

### Step 6: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  add krillnotes-core/src/core/scripting/mod.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  commit -m "feat: display_image/display_download_link take UUID directly; remove run_context dependency"
```

---

## Task 5: Update `photo_note.rhai` template

**Files:**
- Modify: `.worktrees/feat/image-embedding/templates/photo_note.rhai`

### Step 1: Overwrite with new API

```rhai
// @name: Photo Note
// @description: A note type with a photo field to test display_image and get_attachments.

schema("PhotoNote", #{
    fields: [
        #{ name: "photo",   type: "file",     required: false },
        #{ name: "caption", type: "text",     required: false },
        #{ name: "body",    type: "textarea", required: false },
    ],

    // on_view: full detail view in the InfoPanel.
    on_view: |note| {
        let caption = note.fields["caption"] ?? "";
        let body    = note.fields["body"]    ?? "";

        // note.fields["photo"] is the UUID string (or () when not set).
        let img = display_image(note.fields["photo"], 480, caption);

        let parts = [img];
        if caption != "" { parts += [text(caption)]; }
        if body != "" { parts += [section("Notes", markdown(body))]; }
        stack(parts)
    },

    // on_hover: compact thumbnail shown in tree hover tooltips.
    on_hover: |note| {
        display_image(note.fields["photo"], 200, note.title)
    },
});
```

### Step 2: Commit

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  add templates/photo_note.rhai
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding \
  commit -m "feat: update photo_note.rhai template to new display_image API"
```

---

## Task 6: Final verification

### Step 1: Run full Rust test suite

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/image-embedding
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all tests pass.

### Step 2: Run TypeScript build

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep -E "error TS" | head -20
```

Expected: clean.

---

## Notes

- **`resolve_attachment_source` stays** — still used by `preprocess_image_blocks` for the `{{image: field:xxx}}` / `{{image: attach:xxx}}` markdown syntax. The `field:` / `attach:` prefix parsing lives there, not in `display_image`.
- **`NoteRunContext` stays** — still used by `markdown()` to pre-process `{{image:}}` blocks. The `ctx_for_markdown` Arc clone and `set_run_context` / `clear_run_context` calls in `workspace.rs` are unchanged.
- **`Dynamic::into_string()`** consumes the value and returns `Result<String, Box<EvalAltResult>>`. Do not use `.to_string()` — for a unit `()` value it returns the string `"()"` rather than signalling "not set".
- **`size_bytes` type**: `AttachmentMeta.size_bytes` is `i64` — use `Dynamic::from(att.size_bytes)` directly.
