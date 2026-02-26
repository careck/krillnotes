# Remove Docs Hub Page Design

**Date:** 2026-02-26

## Goal

Simplify navigation by removing the `/docs/` hub page and promoting "Getting Started" to a direct top-level nav link.

## Motivation

There are only three doc pages (Getting Started, User Guide, Scripting). A hub page that just lists them adds a click with no value. Direct links are simpler.

## Changes

1. **`hugo.toml` menu** — remove the `Docs` entry, add `Getting Started` pointing to `/docs/getting-started/`.
   - New nav order: Features → Getting Started → User Guide → Scripting → Templates → GitHub

2. **Delete `content/docs/_index.md`** — the hub page at `/docs/` is no longer needed. Hugo will technically still generate a section page there but nothing will link to it.

3. **No URL changes** — all three content pages keep their existing URLs.

## Non-changes

- `list.html` template stays; harmless and potentially useful later.
- No redirects needed since no nav links pointed to `/docs/` from outside the site menu.
