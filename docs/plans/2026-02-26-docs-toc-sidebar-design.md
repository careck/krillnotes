# Design: Docs ToC Sidebar

**Date:** 2026-02-26
**Repo:** krillnotes-website
**Status:** Approved

## Problem

The scripting guide (and docs pages generally) are long enough that navigating them requires extensive scrolling. The manually-maintained numbered ToC at the top of `scripting.md` helps, but once you've scrolled past it there is no persistent navigation aid.

## Decision

Add a JS-generated sticky sidebar ToC to all docs pages, with active-section highlighting via `IntersectionObserver`. Remove the manual ToC list from `scripting.md`.

**Rejected alternative:** Hugo `.TableOfContents` — clean, zero-JS, but no active-section tracking. The JS approach was preferred for the scroll-position highlight.

## Layout

`single.html` is wrapped in a new `<div class="docs-page">` container that holds:
- `<nav class="docs-toc">` — left column (240px), generated at runtime
- `<article class="docs-layout">` — right column (1fr), existing content unchanged

CSS grid: `240px 1fr`, centered to max-width 1200px.

Responsive: below ~1100px viewport the sidebar column is hidden and the article fills the full width. No layout breakage on mobile.

## JS ToC generation (`static/js/main.js`)

On `DOMContentLoaded`:
1. Query all `h2` and `h3` inside `.docs-content`
2. Hugo already generates anchor `id`s for headings — no mutation needed
3. Build a `<ul>`: `h2` items at top level, `h3` items nested under the preceding `h2`
4. Insert into `.docs-toc`
5. If `.docs-toc` ends up empty (page has no headings), hide the element

## Active-section highlighting

An `IntersectionObserver` with `rootMargin: "-10% 0px -80% 0px"` watches each heading. When a heading enters the top band of the viewport, its sidebar link receives `.active`; the previous `.active` link loses it. This tracks the current section without polling.

## Styling (design tokens from `:root`)

- Sidebar uses existing `--ocean-*`, `--sand-*`, `--krill-*` tokens
- Active link: `--krill-primary` colour, slight left indicator bar
- Inactive links: `--sand-500`, no decoration
- `position: sticky; top: 5rem; max-height: calc(100vh - 6rem); overflow-y: auto`
- `h3` items are indented and slightly smaller than `h2` items
- A "On this page" label sits above the list

## Files touched

| File | Change |
|---|---|
| `layouts/_default/single.html` | Wrap existing article in `.docs-page`, add `<nav class="docs-toc">` |
| `static/css/style.css` | Add `.docs-page`, `.docs-toc`, `.docs-toc a`, `.docs-toc a.active`, responsive collapse |
| `static/js/main.js` | Add `buildDocsToc()` function called on DOMContentLoaded |
| `content/docs/scripting.md` | Remove manual ToC list and the `---` separator below it |
