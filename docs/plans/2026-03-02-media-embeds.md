# Media Embeds Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bare YouTube/Instagram URLs on their own line in `textarea` fields auto-render as click-to-play thumbnail cards in the InfoPanel view.

**Architecture:** A Rust preprocessor (`preprocess_media_embeds`) detects bare media URLs before markdown rendering and replaces them with sentinel `<div data-kn-embed-*>` elements. The frontend hydrates those sentinels into thumbnail cards that call `openUrl()` on click. An `embed_media(url)` Rhai helper also exposes the same sentinel generation for `on_view` scripts.

**Tech Stack:** Rust (`regex` crate already a dep), `pulldown-cmark` (existing), React/TypeScript, `@tauri-apps/plugin-opener` (`openUrl` — already imported in InfoPanel), DOMPurify (existing).

---

### Task 1: Create worktree + branch

**Step 1: Create feature branch and worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/media-embeds -b feat/media-embeds
```

Expected: `.worktrees/feat/media-embeds/` created, branch `feat/media-embeds` checked out there.

**Step 2: Confirm worktree is clean**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds status
```

Expected: `nothing to commit, working tree clean`

---

### Task 2: Rust — `make_media_embed_html()` helper + tests

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`

This is the single function that turns a URL string into a sentinel HTML div. Everything else calls this.

**Step 1: Write failing tests** — add to the `#[cfg(test)]` block at the bottom of `display_helpers.rs`:

```rust
// ── make_media_embed_html tests ──────────────────────────────────────────────

#[test]
fn test_embed_youtube_watch_url() {
    let html = make_media_embed_html("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    assert!(html.contains("data-kn-embed-type=\"youtube\""), "got: {html}");
    assert!(html.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {html}");
    assert!(html.contains("kn-media-embed"), "got: {html}");
}

#[test]
fn test_embed_youtube_short_url() {
    let html = make_media_embed_html("https://youtu.be/dQw4w9WgXcQ");
    assert!(html.contains("data-kn-embed-type=\"youtube\""), "got: {html}");
    assert!(html.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {html}");
}

#[test]
fn test_embed_youtube_watch_with_extra_params() {
    let html = make_media_embed_html("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s&list=PL123");
    assert!(html.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {html}");
}

#[test]
fn test_embed_instagram_post_url() {
    let html = make_media_embed_html("https://www.instagram.com/p/ABC123def/");
    assert!(html.contains("data-kn-embed-type=\"instagram\""), "got: {html}");
    assert!(html.contains("data-kn-embed-id=\"ABC123def\""), "got: {html}");
}

#[test]
fn test_embed_instagram_reel_url() {
    let html = make_media_embed_html("https://www.instagram.com/reel/XYZ789/");
    assert!(html.contains("data-kn-embed-type=\"instagram\""), "got: {html}");
    assert!(html.contains("data-kn-embed-id=\"XYZ789\""), "got: {html}");
}

#[test]
fn test_embed_unknown_url_returns_empty() {
    let html = make_media_embed_html("https://example.com/video");
    assert!(html.is_empty(), "got: {html}");
}

#[test]
fn test_embed_empty_string_returns_empty() {
    let html = make_media_embed_html("");
    assert!(html.is_empty(), "got: {html}");
}

#[test]
fn test_embed_url_is_html_escaped_in_output() {
    // URL with & must be escaped in attribute value
    let html = make_media_embed_html("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42");
    assert!(!html.contains("watch?v=dQw4w9WgXcQ&t=42"), "raw & must be escaped");
    assert!(html.contains("&amp;"), "got: {html}");
}
```

**Step 2: Run tests to confirm they fail**

```bash
cargo test -p krillnotes-core test_embed_ 2>&1 | tail -20
```

Expected: compile error — `make_media_embed_html` not defined.

**Step 3: Implement `make_media_embed_html`** — add after the `// ── Attachment-aware display helpers` section in `display_helpers.rs` (around line 505), before `is_field_empty`:

```rust
// ── Media embed helpers ───────────────────────────────────────────────────────

static YT_WATCH_RE: OnceLock<regex::Regex> = OnceLock::new();
static YT_SHORT_RE: OnceLock<regex::Regex> = OnceLock::new();
static IG_POST_RE:  OnceLock<regex::Regex> = OnceLock::new();
static IG_REEL_RE:  OnceLock<regex::Regex> = OnceLock::new();

/// Given a YouTube or Instagram URL, returns a sentinel `<div>` that the
/// frontend will hydrate into a click-to-play thumbnail card.
///
/// Returns an empty string for unrecognised URLs.
pub fn make_media_embed_html(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }

    let yt_watch = YT_WATCH_RE.get_or_init(|| {
        regex::Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/watch\?(?:[^&\s]*&)*v=([A-Za-z0-9_-]{11})")
            .expect("valid regex")
    });
    let yt_short = YT_SHORT_RE.get_or_init(|| {
        regex::Regex::new(r"(?:https?://)?youtu\.be/([A-Za-z0-9_-]{11})")
            .expect("valid regex")
    });
    let ig_post = IG_POST_RE.get_or_init(|| {
        regex::Regex::new(r"(?:https?://)?(?:www\.)?instagram\.com/p/([A-Za-z0-9_-]+)")
            .expect("valid regex")
    });
    let ig_reel = IG_REEL_RE.get_or_init(|| {
        regex::Regex::new(r"(?:https?://)?(?:www\.)?instagram\.com/reel/([A-Za-z0-9_-]+)")
            .expect("valid regex")
    });

    let (embed_type, id) =
        if let Some(caps) = yt_watch.captures(url) {
            ("youtube", caps[1].to_string())
        } else if let Some(caps) = yt_short.captures(url) {
            ("youtube", caps[1].to_string())
        } else if let Some(caps) = ig_post.captures(url) {
            ("instagram", caps[1].to_string())
        } else if let Some(caps) = ig_reel.captures(url) {
            ("instagram", caps[1].to_string())
        } else {
            return String::new();
        };

    format!(
        "<div class=\"kn-media-embed\" \
              data-kn-embed-type=\"{}\" \
              data-kn-embed-id=\"{}\" \
              data-kn-embed-url=\"{}\"></div>",
        embed_type,
        html_escape(&id),
        html_escape(url),
    )
}
```

**Step 4: Run tests — confirm they pass**

```bash
cargo test -p krillnotes-core test_embed_ 2>&1 | tail -20
```

Expected: `8 tests passed`

**Step 5: Commit**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds add krillnotes-core/src/core/scripting/display_helpers.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds commit -m "feat: add make_media_embed_html sentinel helper"
```

---

### Task 3: Rust — `preprocess_media_embeds()` + tests

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs`

**Step 1: Write failing tests** — add to the test block:

```rust
// ── preprocess_media_embeds tests ────────────────────────────────────────────

#[test]
fn test_preprocess_youtube_bare_line_replaced() {
    let input = "Check this out:\n\nhttps://www.youtube.com/watch?v=dQw4w9WgXcQ\n\nMore text.";
    let output = preprocess_media_embeds(input);
    assert!(output.contains("data-kn-embed-type=\"youtube\""), "got: {output}");
    assert!(output.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {output}");
    assert!(output.contains("Check this out:"), "surrounding text lost");
    assert!(output.contains("More text."), "surrounding text lost");
}

#[test]
fn test_preprocess_youtu_be_bare_line_replaced() {
    let input = "https://youtu.be/dQw4w9WgXcQ";
    let output = preprocess_media_embeds(input);
    assert!(output.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {output}");
}

#[test]
fn test_preprocess_instagram_post_bare_line_replaced() {
    let input = "https://www.instagram.com/p/ABC123/";
    let output = preprocess_media_embeds(input);
    assert!(output.contains("data-kn-embed-type=\"instagram\""), "got: {output}");
}

#[test]
fn test_preprocess_instagram_reel_bare_line_replaced() {
    let input = "https://www.instagram.com/reel/XYZ789/";
    let output = preprocess_media_embeds(input);
    assert!(output.contains("data-kn-embed-type=\"instagram\""), "got: {output}");
}

#[test]
fn test_preprocess_url_inline_in_sentence_not_replaced() {
    let input = "Watch https://www.youtube.com/watch?v=dQw4w9WgXcQ for fun.";
    let output = preprocess_media_embeds(input);
    assert!(!output.contains("kn-media-embed"), "inline URL must not embed");
    assert!(output.contains("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), "URL must be preserved");
}

#[test]
fn test_preprocess_non_media_url_unchanged() {
    let input = "https://example.com/video";
    let output = preprocess_media_embeds(input);
    assert_eq!(input, output);
}

#[test]
fn test_preprocess_url_with_leading_whitespace_replaced() {
    let input = "  https://www.youtube.com/watch?v=dQw4w9WgXcQ  ";
    let output = preprocess_media_embeds(input);
    assert!(output.contains("kn-media-embed"), "got: {output}");
}

#[test]
fn test_preprocess_multiple_embeds_in_text() {
    let input = "https://www.youtube.com/watch?v=dQw4w9WgXcQ\n\nhttps://www.instagram.com/p/ABC123/";
    let output = preprocess_media_embeds(input);
    assert!(output.contains("data-kn-embed-type=\"youtube\""), "got: {output}");
    assert!(output.contains("data-kn-embed-type=\"instagram\""), "got: {output}");
}
```

**Step 2: Run tests to confirm they fail**

```bash
cargo test -p krillnotes-core test_preprocess_ 2>&1 | tail -20
```

Expected: compile error — `preprocess_media_embeds` not defined.

**Step 3: Implement `preprocess_media_embeds`** — add directly below `make_media_embed_html` in `display_helpers.rs`:

```rust
static MEDIA_LINE_RE: OnceLock<regex::Regex> = OnceLock::new();

/// Pre-process bare media URLs that occupy their own line in markdown text.
///
/// Each matching line is replaced with a `<div data-kn-embed-*>` sentinel
/// that the frontend hydrates into a click-to-play thumbnail card.
///
/// A URL is considered "bare" when it is the only non-whitespace content on
/// its line. URLs embedded mid-sentence are left unchanged.
pub fn preprocess_media_embeds(text: &str) -> String {
    let re = MEDIA_LINE_RE.get_or_init(|| {
        regex::Regex::new(
            r"(?m)^[ \t]*(https?://(?:(?:www\.)?youtube\.com/watch\?[^\s]*|youtu\.be/[^\s]*|(?:www\.)?instagram\.com/(?:p|reel)/[^\s]*))[ \t]*$"
        ).expect("valid regex")
    });

    re.replace_all(text, |caps: &regex::Captures| {
        let url = caps[1].trim();
        let sentinel = make_media_embed_html(url);
        if sentinel.is_empty() {
            caps[0].to_string()  // unrecognised URL: leave unchanged
        } else {
            sentinel
        }
    }).into_owned()
}
```

**Step 4: Run all media tests**

```bash
cargo test -p krillnotes-core test_embed_ test_preprocess_ 2>&1 | tail -20
```

Expected: all pass.

**Step 5: Run full test suite to check for regressions**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 6: Commit**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds add krillnotes-core/src/core/scripting/display_helpers.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds commit -m "feat: add preprocess_media_embeds for bare URL detection"
```

---

### Task 4: Rust — Wire preprocessor into render pipeline

**Files:**
- Modify: `krillnotes-core/src/core/scripting/display_helpers.rs` (one edit)
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (one edit)

**Step 1: Wire into `format_field_value_html`** — in `display_helpers.rs`, find the `textarea` arm (around line 459):

```rust
// BEFORE:
(FieldValue::Text(s), "textarea") => {
    let preprocessed = if let Some((fields, attachments)) = image_context {
        preprocess_image_blocks(s, fields, attachments)
    } else {
        s.clone()
    };
    format!("<div class=\"kn-view-markdown\">{}</div>", render_markdown_to_html(&preprocessed))
}

// AFTER:
(FieldValue::Text(s), "textarea") => {
    let after_images = if let Some((fields, attachments)) = image_context {
        preprocess_image_blocks(s, fields, attachments)
    } else {
        s.clone()
    };
    let preprocessed = preprocess_media_embeds(&after_images);
    format!("<div class=\"kn-view-markdown\">{}</div>", render_markdown_to_html(&preprocessed))
}
```

**Step 2: Wire into the `markdown()` Rhai closure** — in `mod.rs`, find the `markdown` closure (around line 513):

```rust
// BEFORE:
engine.register_fn("markdown", move |text: String| -> String {
    let guard = ctx_for_markdown.lock().expect("run_context poisoned");
    let maybe_context = guard.as_ref().map(|ctx| (ctx.note.fields.clone(), ctx.attachments.clone()));
    drop(guard);
    let processed = if let Some((fields, attachments)) = maybe_context {
        display_helpers::preprocess_image_blocks(&text, &fields, &attachments)
    } else {
        text
    };
    display_helpers::rhai_markdown_raw(processed)
});

// AFTER:
engine.register_fn("markdown", move |text: String| -> String {
    let guard = ctx_for_markdown.lock().expect("run_context poisoned");
    let maybe_context = guard.as_ref().map(|ctx| (ctx.note.fields.clone(), ctx.attachments.clone()));
    drop(guard);
    let after_images = if let Some((fields, attachments)) = maybe_context {
        display_helpers::preprocess_image_blocks(&text, &fields, &attachments)
    } else {
        text
    };
    let processed = display_helpers::preprocess_media_embeds(&after_images);
    display_helpers::rhai_markdown_raw(processed)
});
```

**Step 3: Add a wiring integration test** — in the test block of `display_helpers.rs`:

```rust
#[test]
fn test_render_default_view_textarea_bare_url_becomes_sentinel() {
    use crate::{FieldValue, FieldDefinition, Note, Schema};
    use std::collections::HashMap;

    let mut fields = HashMap::new();
    fields.insert(
        "body".into(),
        FieldValue::Text("Watch this:\n\nhttps://www.youtube.com/watch?v=dQw4w9WgXcQ\n\nDone.".into()),
    );
    let note = Note {
        id: "id-embed".into(), title: "T".into(), node_type: "T".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0, fields, is_expanded: false, tags: vec![],
    };
    let schema = Schema {
        name: "T".into(),
        fields: vec![FieldDefinition {
            name: "body".into(), field_type: "textarea".into(),
            required: false, can_view: true, can_edit: true,
            options: vec![], max: 0, target_type: None, show_on_hover: false, allowed_types: vec![],
        }],
        title_can_view: true, title_can_edit: true,
        children_sort: "none".into(),
        allowed_parent_types: vec![], allowed_children_types: vec![],
        allow_attachments: false, attachment_types: vec![],
    };

    let html = render_default_view(&note, Some(&schema), &HashMap::new(), &[]);
    assert!(html.contains("data-kn-embed-type=\"youtube\""), "sentinel must appear, got: {html}");
    assert!(html.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {html}");
    assert!(html.contains("Watch this:"), "surrounding text lost");
}
```

**Step 4: Run all tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all pass (including new integration test).

**Step 5: Commit**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds add \
    krillnotes-core/src/core/scripting/display_helpers.rs \
    krillnotes-core/src/core/scripting/mod.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds commit -m "feat: wire media embed preprocessor into render pipeline"
```

---

### Task 5: Rust — Register `embed_media()` Rhai helper

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Register the function** — in `mod.rs`, add after the `link_to` registration (around line 512):

```rust
engine.register_fn("embed_media", |url: String| -> String {
    display_helpers::make_media_embed_html(&url)
});
```

**Step 2: Add a Rhai-level test** — find how other Rhai tests in `mod.rs` construct a test engine (search for `fn test_script_registry` or `register_display_helpers_for_test`) and match that pattern exactly. Add:

```rust
#[test]
fn test_embed_media_rhai_function_youtube() {
    // Use whichever test engine setup pattern the existing Rhai tests use.
    // Look for tests like test_markdown_rhai_function_renders_bold for the pattern.
    let result: String = engine
        .eval_expression::<String>(
            r#"embed_media("https://www.youtube.com/watch?v=dQw4w9WgXcQ")"#,
        )
        .expect("eval failed");
    assert!(result.contains("data-kn-embed-type=\"youtube\""), "got: {result}");
    assert!(result.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {result}");
}

#[test]
fn test_embed_media_rhai_function_unknown_returns_empty() {
    let result: String = engine
        .eval_expression::<String>(r#"embed_media("https://example.com")"#)
        .expect("eval failed");
    assert!(result.is_empty(), "got: {result}");
}
```

**Step 3: Run Rhai tests**

```bash
cargo test -p krillnotes-core test_embed_media_rhai 2>&1 | tail -10
```

**Step 4: Run full suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 5: Commit**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds add krillnotes-core/src/core/scripting/mod.rs
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds commit -m "feat: register embed_media() Rhai display helper"
```

---

### Task 6: Frontend — DOMPurify + hydration useEffect

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Expand DOMPurify's ADD_ATTR** — find the HTML sanitization call (line ~388). It has `ADD_ATTR: ['data-note-id', 'data-kn-attach-id', ...]`. Add three new attributes:

```
// BEFORE:
ADD_ATTR: ['data-note-id', 'data-kn-attach-id', 'data-kn-width', 'data-kn-download-id']

// AFTER:
ADD_ATTR: ['data-note-id', 'data-kn-attach-id', 'data-kn-width', 'data-kn-download-id',
           'data-kn-embed-type', 'data-kn-embed-id', 'data-kn-embed-url']
```

**Step 2: Add the media hydration `useEffect`** — insert directly after the existing image hydration `useEffect` block (ends around line 189). The new effect watches `customViewHtml` and replaces `[data-kn-embed-type]` sentinels:

```tsx
// Hydrate [data-kn-embed-type] sentinels into click-to-play media cards
useEffect(() => {
  const container = viewHtmlRef.current;
  if (!container || !customViewHtml) return;

  const sentinels = Array.from(
    container.querySelectorAll<HTMLElement>('[data-kn-embed-type]')
  );

  sentinels.forEach((el) => {
    const type = el.getAttribute('data-kn-embed-type');
    const id   = el.getAttribute('data-kn-embed-id') ?? '';
    const url  = el.getAttribute('data-kn-embed-url') ?? '';

    const card = document.createElement('div');

    if (type === 'youtube' && id) {
      card.className = 'kn-media-thumbnail';
      const img = document.createElement('img');
      img.src = `https://img.youtube.com/vi/${id}/hqdefault.jpg`;
      img.alt = 'Video thumbnail';
      const play = document.createElement('div');
      play.className = 'kn-media-play-btn';
      play.textContent = '▶';
      card.appendChild(img);
      card.appendChild(play);
    } else if (type === 'instagram') {
      card.className = 'kn-media-card kn-media-card--instagram';
      const label = document.createElement('span');
      label.className = 'kn-media-card-label';
      label.textContent = 'Open on Instagram ↗';
      card.appendChild(label);
    } else {
      return; // unknown type — leave sentinel in place
    }

    card.addEventListener('click', () => { openUrl(url); });
    el.replaceWith(card);
  });
}, [customViewHtml]);
```

**Step 3: TypeScript build check**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds/krillnotes-desktop && npx tsc --noEmit 2>&1 | head -30
```

Expected: no errors.

**Step 4: Commit**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds add krillnotes-desktop/src/components/InfoPanel.tsx
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds commit -m "feat: hydrate media embed sentinels into thumbnail cards in InfoPanel"
```

---

### Task 7: CSS — Media card styles

**Files:**
- Modify: `krillnotes-desktop/src/styles/globals.css`

**Step 1: Add styles** — append to the end of `globals.css`:

```css
/* ── Media embed cards ─────────────────────────────────────────────────────── */

.kn-media-thumbnail {
  position: relative;
  display: block;
  max-width: 480px;
  width: 100%;
  aspect-ratio: 16 / 9;
  cursor: pointer;
  border-radius: 8px;
  overflow: hidden;
  margin: 8px 0;
}

.kn-media-thumbnail img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.kn-media-play-btn {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 56px;
  height: 56px;
  background: rgba(0, 0, 0, 0.65);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
  font-size: 20px;
  pointer-events: none;
}

.kn-media-card {
  display: inline-flex;
  align-items: center;
  padding: 12px 16px;
  border-radius: 8px;
  cursor: pointer;
  margin: 8px 0;
  max-width: 480px;
  width: 100%;
  box-sizing: border-box;
}

.kn-media-card--instagram {
  background: linear-gradient(45deg, #833ab4, #fd1d1d, #fcb045);
  color: white;
}

.kn-media-card-label {
  font-size: 14px;
  font-weight: 500;
}
```

**Step 2: TypeScript build check**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds/krillnotes-desktop && npx tsc --noEmit 2>&1 | head -10
```

**Step 3: Commit**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds add krillnotes-desktop/src/styles/globals.css
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds commit -m "feat: add CSS for media embed thumbnail and Instagram card"
```

---

### Task 8: Manual smoke test + PR

**Step 1: Build and run the app**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/media-embeds/krillnotes-desktop && npm run tauri dev
```

**Step 2: Manual test checklist**

- Open or create a note with a `textarea` field
- Paste `https://www.youtube.com/watch?v=dQw4w9WgXcQ` alone on a line, save and view → thumbnail with play button appears
- Click thumbnail → browser opens YouTube URL
- Paste `https://youtu.be/dQw4w9WgXcQ` alone on a line → same result
- Paste `https://www.instagram.com/p/ANYID/` alone → Instagram gradient card appears; click opens browser
- Paste `https://www.instagram.com/reel/ANYID/` alone → same as above
- Paste a YouTube URL mid-sentence (e.g. `Watch https://...  here`) → no embed; URL appears as plain text
- Paste a non-media URL alone on a line → no embed; URL appears as plain text
- In a Rhai `on_view` script, call `embed_media("https://www.youtube.com/watch?v=dQw4w9WgXcQ")` → thumbnail renders

**Step 3: Run full Rust test suite one final time**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all pass.

**Step 4: Push branch and open PR**

```bash
git -C /Users/careck/Source/Krillnotes push -u github-https feat/media-embeds
gh pr create --repo careck/krillnotes \
  --base master \
  --title "feat: media embeds in textarea (closes #22)" \
  --body "$(cat <<'EOF'
## Summary
- Bare YouTube/Instagram URLs on their own line in textarea fields auto-render as click-to-play thumbnail cards
- YouTube: shows img.youtube.com thumbnail with play button overlay; click opens in browser
- Instagram: shows gradient card with link label; click opens in browser (no thumbnail - API requires auth)
- embed_media(url) Rhai helper registered for use in on_view scripts
- No iframe, no third-party requests until clicked

## Test plan
- [ ] YouTube watch URL bare on its own line - thumbnail appears, click opens browser
- [ ] YouTube youtu.be short URL - same
- [ ] Instagram /p/ URL - gradient card, click opens browser
- [ ] Instagram /reel/ URL - same
- [ ] YouTube URL inline in sentence - no embed, plain text
- [ ] Non-media URL alone on a line - no embed
- [ ] embed_media() from Rhai on_view script - thumbnail renders in InfoPanel
- [ ] All Rust tests pass
EOF
)"
```
