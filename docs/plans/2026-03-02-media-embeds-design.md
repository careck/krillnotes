# Design: Media Embeds in Textarea (issue #22)

**Date:** 2026-03-02
**Approach:** Option 2 — markdown-body rendering only (no new field types)

## Summary

Bare YouTube and Instagram URLs placed on their own line inside a `textarea` field auto-render as click-to-play thumbnail cards in the view panel. Clicking opens the URL in the system browser via `openUrl()`. An `embed_media(url)` Rhai helper is also registered so template `on_view` scripts can call it explicitly.

## Architecture

The implementation follows the existing `{{image: ...}}` sentinel pattern:

1. **Rust preprocessor** detects bare media URLs before markdown rendering.
2. **Sentinel HTML** (`<div data-kn-embed-*>`) passes through `pulldown-cmark` unchanged.
3. **Frontend hydration** in `InfoPanel.tsx` replaces sentinels with thumbnail UI.

## Rust — Preprocessing (`display_helpers.rs`)

New function `preprocess_media_embeds(text: &str) -> String`. Runs before `render_markdown_to_html()` in two callsites:
- `format_field_value_html()` — default view, `textarea` fields
- `markdown()` Rhai host function closure in `mod.rs`

A `OnceLock<Regex>` detects lines that contain only a media URL (optional surrounding whitespace):

**URL patterns:**
- `https://www.youtube.com/watch?v=VIDEO_ID` (with optional query params)
- `https://youtu.be/VIDEO_ID`
- `https://www.instagram.com/p/POST_ID/`
- `https://www.instagram.com/reel/REEL_ID/`

Each matching line is replaced with a sentinel HTML block:

```html
<div class="kn-media-embed"
     data-kn-embed-type="youtube"
     data-kn-embed-id="dQw4w9WgXcQ"
     data-kn-embed-url="https://youtube.com/watch?v=dQw4w9WgXcQ"></div>
```

`data-kn-embed-type` values: `"youtube"` or `"instagram"`.
`data-kn-embed-id` is the video/post ID (used for the YouTube thumbnail URL).

A helper `make_media_embed_html(url: &str) -> String` extracts the type and ID from a URL and returns the sentinel div (or empty string for unrecognised URLs).

## Rhai Helper — `embed_media(url)`

Registered in `mod.rs` alongside `markdown()`, `render_tags()`, etc. Calls `make_media_embed_html(url)` and returns the sentinel HTML string. Unrecognised URLs return an empty string (silent, no error span).

```rhai
on_view("VideoNote", |note| {
    stack([
        heading(note.title),
        embed_media(note.fields["youtube_url"] ?? ""),
    ])
});
```

## Frontend — Hydration (`InfoPanel.tsx`)

DOMPurify `ADD_ATTR` gets three new entries: `data-kn-embed-type`, `data-kn-embed-id`, `data-kn-embed-url`.

A `useEffect` (alongside the existing `data-kn-attach-id` image hydration) scans for `[data-kn-embed-type]` elements and replaces each sentinel:

**YouTube:**
```html
<div class="kn-media-thumbnail">
  <img src="https://img.youtube.com/vi/{id}/hqdefault.jpg" alt="Video thumbnail">
  <div class="kn-media-play-btn">▶</div>
</div>
```
Click → `openUrl(originalUrl)`.

**Instagram:**
```html
<div class="kn-media-card kn-media-card--instagram">
  <span class="kn-media-card-label">Open on Instagram ↗</span>
</div>
```
Click → `openUrl(originalUrl)`. No thumbnail (Instagram API requires auth).

## CSS (`globals.css`)

| Class | Description |
|---|---|
| `.kn-media-thumbnail` | Responsive container, max-width 480px, 16:9 aspect ratio, `cursor: pointer` |
| `.kn-media-play-btn` | Centered circle overlay, semi-transparent dark background, white ▶ |
| `.kn-media-card` | Rounded card, padding, `cursor: pointer` |
| `.kn-media-card--instagram` | Gradient background `#833ab4 → #fd1d1d → #fcb045`, white text |

## Out of Scope

- No thumbnail for Instagram (API gated without OAuth)
- No metadata fetching (video title, duration) — would require oEmbed
- No `HoverTooltip` support — embeds only in full InfoPanel view
- No support for other platforms (Vimeo, Twitter/X, TikTok) — future work
