# Book Collection Template — Design Document

**Date:** 2026-02-26
**Status:** Approved
**Scope:** Remove Book from system scripts; create a self-contained Book Collection template with a Library folder view and sorting tree actions; add a Templates gallery section to the website.

---

## Overview

The Book Collection is the first entry in the Krillnotes template gallery. It demonstrates the `on_view` hook and multiple `add_tree_action` sort mechanisms in a single, self-contained user script. Moving Book out of system scripts keeps the built-in set lean (generic primitives only) and establishes the template gallery as the home for purpose-built schemas.

---

## Repository Split

### Krillnotes repo
- Remove `krillnotes-core/src/system_scripts/04_book.rhai`
- Add `templates/book_collection.rhai` — authoritative script source

### Website repo (`krillnotes-website`)
- Add `content/templates/book-collection.md` — gallery page (screenshot, user guide, annotated script walkthrough)
- Add `static/templates/book_collection.rhai` — copy of the script for download
- Add `static/templates/book_collection.krillnotes` — sample workspace export with a few pre-populated books
- Add `static/templates/book_collection-screenshot.png` — screenshot of the Library on_view
- Update `hugo.toml` to add a Templates entry to the nav menu
- Add `content/templates/_index.md` — templates section landing page

---

## Script: `templates/book_collection.rhai`

### Book schema

Identical to the current `04_book.rhai` system script. Fields:

| Field | Type | Notes |
|---|---|---|
| `book_title` | `text` | required |
| `author` | `text` | required |
| `genre` | `text` | — |
| `status` | `select` | "To Read", "Reading", "Read" |
| `rating` | `rating` | max 5 |
| `started` | `date` | — |
| `finished` | `date` | — |
| `notes` | `textarea` | — |
| `read_duration` | `text` | view-only, derived |

`on_save`: derives title as `"Author: Book Title"` and computes `read_duration` from `started`/`finished`.

### Library schema

A folder note intended to contain Book notes as children.

**`on_view` hook** — three sections, each with tailored columns:

| Section | Columns | Condition |
|---|---|---|
| Currently Reading | Title, Author, Started | `status == "Reading"` |
| To Read | Title, Author, Genre | `status == "To Read"` |
| Read | Title, Author, Finished, Rating | `status == "Read"` |

Each section heading shows a count, e.g. "Read (12)". Books with no status or an unrecognised status fall into a fourth "Unsorted" section. Sections with zero books are omitted.

**Tree actions** (right-click on Library note):

| Action | Sort key | Order |
|---|---|---|
| Sort by Title | `title` | A → Z |
| Sort by Author | `author` field | A → Z |
| Sort by Genre | `genre` field | A → Z |
| Sort by Rating | `rating` field | High → Low |
| Sort by Date Read | `finished` field | Newest first |

---

## Website Gallery Page

The `content/templates/book-collection.md` page covers:
1. Screenshot of the Library `on_view`
2. Download buttons — `.rhai` script and `.krillnotes` sample export
3. User guide — how to create a Library note, add Book children, use sort actions
4. Annotated script walkthrough — explains `on_view`, `add_tree_action`, and the sort logic

---

## Out of Scope

- Tags and cross-note links (planned separately; would enable linking books to authors or themes)
- A "Wishlist" status or sub-collection (can be added later as a genre or status option)
- Automatic sync of the `.rhai` script between repos (manual copy is sufficient)
