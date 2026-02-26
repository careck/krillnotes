# Zettelkasten Template — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add two small Rhai engine functions (`today()` and `tags` in on_view note_map), write a self-contained Zettelkasten template, and publish it as the second gallery entry on the website.

**Architecture:** Two core fixes in `krillnotes-core/src/core/scripting/mod.rs` unlock the template: (1) `today()` returns the current date string for auto-titling in on_save, (2) exposing `note.tags` in the on_view note_map enables related-note queries. The template itself (`templates/zettelkasten.rhai`) defines a Zettel note type and a Kasten container, mirroring the book_collection pattern. Website changes are in `krillnotes-website` and go directly on its main branch.

**Tech Stack:** Rust + Rhai scripting, Hugo static site

---

## Task 1: Create feature branch and worktree

**Step 1: Create worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/zettelkasten-template -b feat/zettelkasten-template
```

**Step 2: Verify**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/zettelkasten-template
```
Expected: worktree directory exists with repo contents.

**Step 3: All subsequent Krillnotes work happens inside this worktree path.**

```
/Users/careck/Source/Krillnotes/.worktrees/feat/zettelkasten-template/
```

---

## Task 2: Register `today()` in the Rhai engine

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

The Rhai engine has no date function. The Zettel `on_save` hook needs today's date to prefix the title. `chrono` is already a workspace dependency.

**Step 1: Write a failing test**

In `mod.rs`, inside the `#[cfg(test)]` block (near the other scripting tests), add:

```rust
#[test]
fn test_today_returns_yyyy_mm_dd() {
    use std::collections::HashMap;
    let mut registry = ScriptRegistry::new().unwrap();
    // Wrap today() in an on_save hook so we test it through the normal hook path
    registry.load_script(r#"
        schema("DateTest", #{
            fields: [#{ name: "dummy", type: "text", required: false }],
            on_save: |note| {
                note.title = today();
                note
            }
        });
    "#, "test").unwrap();

    let result = registry
        .run_on_save_hook("DateTest", "id1", "DateTest", "", &HashMap::new())
        .unwrap()
        .unwrap();
    let (title, _) = result;
    // Must be exactly 10 chars: YYYY-MM-DD
    assert_eq!(title.len(), 10, "expected YYYY-MM-DD (10 chars), got: {title}");
    assert_eq!(&title[4..5], "-", "missing year-month separator: {title}");
    assert_eq!(&title[7..8], "-", "missing month-day separator: {title}");
}
```

**Step 2: Run the test to confirm it fails**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/zettelkasten-template
cargo test -p krillnotes-core test_today_returns_yyyy_mm_dd 2>&1 | tail -10
```
Expected: FAIL — `today` is not a registered function.

**Step 3: Add the `today()` registration**

In `mod.rs`, add `use chrono::Local;` to the imports at the top of the file (alongside the existing `use` lines). Then, in `ScriptRegistry::new()`, just before the `Ok(Self { ... })` return (near line 454), add:

```rust
engine.register_fn("today", || Local::now().format("%Y-%m-%d").to_string());
```

**Step 4: Run the test to confirm it passes**

```bash
cargo test -p krillnotes-core test_today_returns_yyyy_mm_dd 2>&1 | tail -5
```
Expected: `test test_today_returns_yyyy_mm_dd ... ok`

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat(scripting): register today() in Rhai engine"
```

---

## Task 3: Expose `tags` in the on_view note_map

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` — `run_on_view_hook` (around line 628)

The on_view note_map currently only contains `id`, `node_type`, `title`, and `fields`. The Zettel `on_view` needs to read `note.tags` to query related notes. Notes in the QueryContext already expose `tags` (via `note_to_rhai_dynamic`); we just need to mirror that for the subject note.

**Step 1: Write a failing test**

In `mod.rs` tests, add:

```rust
#[test]
fn test_on_view_note_has_tags() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Tagged", #{
            fields: [],
            on_view: |note| {
                let t = note.tags;
                text(t.len().to_string())
            }
        });
    "#, "test").unwrap();

    let note = Note {
        id: "n1".to_string(), node_type: "Tagged".to_string(),
        title: "T".to_string(), parent_id: None, position: 0,
        created_at: 0, modified_at: 0, created_by: 0, modified_by: 0,
        fields: std::collections::HashMap::new(), is_expanded: false,
        tags: vec!["rust".to_string(), "notes".to_string()],
    };
    let ctx = QueryContext {
        notes_by_id: std::collections::HashMap::new(),
        children_by_id: std::collections::HashMap::new(),
        notes_by_type: std::collections::HashMap::new(),
        notes_by_tag: std::collections::HashMap::new(),
    };
    let html = registry.run_on_view_hook(&note, ctx).unwrap().unwrap();
    assert!(html.contains("2"), "expected tag count 2, got: {html}");
}
```

**Step 2: Run the test to confirm it fails**

```bash
cargo test -p krillnotes-core test_on_view_note_has_tags 2>&1 | tail -10
```
Expected: FAIL — `note.tags` is not accessible (property or index-access error).

**Step 3: Add `tags` to the on_view note_map**

In `run_on_view_hook` (the method on `ScriptRegistry` in `mod.rs`, around line 628), after the existing `note_map.insert("fields"...)` line, add:

```rust
let tags_array: rhai::Array = note.tags.iter()
    .map(|t| Dynamic::from(t.clone()))
    .collect();
note_map.insert("tags".into(), Dynamic::from(tags_array));
```

**Step 4: Run the test to confirm it passes**

```bash
cargo test -p krillnotes-core test_on_view_note_has_tags 2>&1 | tail -5
```
Expected: `test test_on_view_note_has_tags ... ok`

**Step 5: Run full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```
Expected: all tests pass.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat(scripting): expose note.tags in on_view hook"
```

---

## Task 4: Write `templates/zettelkasten.rhai` — Zettel schema

**Files:**
- Create: `templates/zettelkasten.rhai`

**Step 1: Create the file with the Zettel schema**

```rhai
// @name: Zettelkasten
// @description: An atomic note-taking system. Zettel notes are auto-titled with
// today's date and first words of the body. The Kasten folder shows recent notes
// and related-note links are surfaced via shared tags.
//
// Usage: Create a Kasten note, then add Zettel notes as children.
// Tip: assign native tags to each Zettel to enable related-note discovery.

schema("Zettel", #{
    title_can_edit: false,
    allowed_parent_types: ["Kasten"],
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ],
    on_save: |note| {
        let body = note.fields["body"] ?? "";
        let words = body.split(" ").filter(|w| w != "");
        let snippet = if words.len() == 0 {
            "Untitled"
        } else {
            let take = if words.len() > 6 { 6 } else { words.len() };
            let s = "";
            let i = 0;
            while i < take { s += words[i] + " "; i += 1; }
            s = s.trim();
            if words.len() > 6 { s + " \u{2026}" } else { s }
        };
        note.title = today() + " \u{2014} " + snippet;
        note
    },
    on_view: |note| {
        let body = note.fields["body"] ?? "";

        let body_block = if body != "" {
            text(body)
        } else {
            text("(no content)")
        };

        // Related notes via shared tags
        let tags = note.tags;
        if tags.len() == 0 {
            return body_block;
        }

        let related = get_notes_for_tag(tags).filter(|n| n.id != note.id);
        if related.len() == 0 {
            return body_block;
        }

        let rows = related.map(|n| [
            n.title,
            n.tags.reduce(|a, b| a + ", " + b) ?? ""
        ]);
        let related_section = section(
            "Related Notes (" + related.len() + ")",
            table(["Note", "Shared Tags"], rows)
        );

        stack([body_block, related_section])
    }
});
```

**Step 2: Verify it loads in the app**

Open the desktop app → Settings → Scripts → Import → select `templates/zettelkasten.rhai`.
Expected: script appears with no compile error.

---

## Task 5: Add Kasten schema and tree actions to `zettelkasten.rhai`

**Files:**
- Modify: `templates/zettelkasten.rhai` (append below Zettel schema)

**Step 1: Append the Kasten schema and tree actions**

```rhai
schema("Kasten", #{
    allowed_children_types: ["Zettel"],
    fields: [],
    on_view: |note| {
        let zettel = get_children(note.id);
        let count = zettel.len();

        if count == 0 {
            return text("No notes yet. Right-click to add a Zettel.");
        }

        // Collect all unique tags across all Zettel
        let all_tags = [];
        for z in zettel { for t in z.tags { all_tags += [t]; } }
        all_tags.sort();
        let unique_tags = [];
        for t in all_tags {
            if unique_tags.len() == 0 || unique_tags[unique_tags.len() - 1] != t {
                unique_tags += [t];
            }
        }

        let stats = count.to_string() + " Zettel \u{00b7} " + unique_tags.len().to_string() + " unique tags";

        // Sort children by title descending (YYYY-MM-DD prefix makes newest first)
        let sorted = zettel;
        sorted.sort_by(|a, b| a.title >= b.title);

        let recent = if sorted.len() > 10 { sorted.extract(0, 10) } else { sorted };
        let rows = recent.map(|z| [
            z.title,
            z.tags.reduce(|a, b| a + ", " + b) ?? "\u{2014}"
        ]);

        stack([
            text(stats),
            section("Recent Notes", table(["Note", "Tags"], rows))
        ])
    }
});

add_tree_action("Sort by Date (Newest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title >= b.title);
    children.map(|c| c.id)
});

add_tree_action("Sort by Date (Oldest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
```

**Step 2: Manually verify in the app**

- Create a Kasten note
- Add 3–5 Zettel children, write some body text in each
- Add native tags to some of them (tag cloud panel or InfoPanel)
- Click the Kasten note → Expected: stats line + recent notes table
- Click a Zettel note that has tags → Expected: body text + Related Notes section showing other Zettel with shared tags
- Right-click Kasten → Expected: two sort actions in the context menu

**Step 3: Commit**

```bash
git add templates/zettelkasten.rhai
git commit -m "feat: add zettelkasten template with Zettel on_save/on_view and Kasten overview"
```

---

## Task 6: Write unit tests for the Zettel on_save hook

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (test block)

The `on_save` hook calls `today()`, which makes it hard to assert the full title in tests (date changes daily). Assert that the title *starts with* a date prefix and *contains* the snippet.

**Step 1: Add tests**

```rust
#[test]
fn test_zettel_on_save_sets_date_title() {
    use std::collections::HashMap;
    use crate::FieldValue;
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("ZettelTest", #{
            title_can_edit: false,
            fields: [#{ name: "body", type: "textarea", required: false }],
            on_save: |note| {
                let body = note.fields["body"] ?? "";
                let words = body.split(" ").filter(|w| w != "");
                let snippet = if words.len() == 0 {
                    "Untitled"
                } else {
                    let take = if words.len() > 6 { 6 } else { words.len() };
                    let s = ""; let i = 0;
                    while i < take { s += words[i] + " "; i += 1; }
                    s = s.trim();
                    if words.len() > 6 { s + " \u{2026}" } else { s }
                };
                note.title = today() + " \u{2014} " + snippet;
                note
            }
        });
    "#, "test").unwrap();

    let mut fields = HashMap::new();
    fields.insert("body".to_string(),
        FieldValue::Text("Emergence is when simple rules produce complex behaviour".to_string()));

    let (title, _) = registry
        .run_on_save_hook("ZettelTest", "id1", "ZettelTest", "", &fields)
        .unwrap().unwrap();

    // Title must start with YYYY-MM-DD
    assert_eq!(&title[4..5], "-", "missing year-month separator: {title}");
    assert_eq!(&title[7..8], "-", "missing month-day separator: {title}");
    // Must contain the first 6 words
    assert!(title.contains("Emergence is when simple rules produce"),
        "snippet missing: {title}");
    // Body has 9 words; title must end with ellipsis (…)
    assert!(title.ends_with('\u{2026}'), "expected truncation ellipsis: {title}");
}

#[test]
fn test_zettel_on_save_empty_body_uses_untitled() {
    use std::collections::HashMap;
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("ZettelEmpty", #{
            title_can_edit: false,
            fields: [#{ name: "body", type: "textarea", required: false }],
            on_save: |note| {
                let body = note.fields["body"] ?? "";
                let words = body.split(" ").filter(|w| w != "");
                let snippet = if words.len() == 0 { "Untitled" } else { words[0] };
                note.title = today() + " \u{2014} " + snippet;
                note
            }
        });
    "#, "test").unwrap();

    let (title, _) = registry
        .run_on_save_hook("ZettelEmpty", "id2", "ZettelEmpty", "", &HashMap::new())
        .unwrap().unwrap();
    assert!(title.contains("Untitled"), "expected Untitled fallback: {title}");
}
```

**Step 2: Run the tests**

```bash
cargo test -p krillnotes-core test_zettel_on_save 2>&1 | tail -10
```
Expected: both tests pass.

**Step 3: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "test(scripting): add Zettel on_save hook unit tests"
```

---

## Task 7: Website — add Zettelkasten gallery page

**Working directory for all website tasks:** `/Users/careck/Source/krillnotes-website`

Website changes go directly on the website repo's main branch.

**Files:**
- Create: `content/templates/zettelkasten.md`

**Step 1: Write the gallery page**

```markdown
---
title: "Zettelkasten"
description: "An atomic note-taking system. Notes auto-title with today's date and related notes surface via shared tags."
screenshot: "/templates/zettelkasten.screenshot.png"
---

A Zettelkasten (German for *slip-box*) is a method for building knowledge through
small, atomic notes linked by shared ideas. This template brings that method to
Krillnotes: each **Zettel** note auto-titles itself with today's date and the first
few words of its content. Add native tags and the Zettel view shows you which other
notes share those tags.

![Zettelkasten template screenshot](/templates/zettelkasten.screenshot.png)

## Downloads

- [zettelkasten.rhai](/templates/zettelkasten.rhai) — import into Script Manager
- [zettelkasten.krillnotes.zip](/templates/zettelkasten.krillnotes.zip) — sample workspace

## How to use

1. Import `zettelkasten.rhai` in **Settings → Scripts → Import Script**
2. Create a new note and choose **Kasten** as the type — this is your slip-box
3. Add children and choose **Zettel** as the type for each
4. Write your idea in the **body** field and save — the title is set automatically
5. Add native tags to each Zettel (tag cloud panel or the InfoPanel)
6. Click any Zettel to see its body and a **Related Notes** table of notes sharing its tags
7. Right-click the Kasten to sort all notes by date

## How it works

### Zettel schema — `on_save` hook

When you save a Zettel, the `on_save` hook builds the title automatically using
the built-in `today()` function and the first six words of the body:

```rhai
on_save: |note| {
    let words = note.fields["body"].split(" ").filter(|w| w != "");
    let snippet = /* first 6 words, truncated with … if longer */;
    note.title = today() + " — " + snippet;
    note
}
```

The `YYYY-MM-DD` date prefix means titles sort chronologically as plain strings —
the tree sort actions need no special date parsing.

### Zettel schema — `on_view` hook

When you click a Zettel note, the `on_view` hook:

1. Renders the body text
2. Reads `note.tags` (the native tags assigned via the tag panel)
3. Calls `get_notes_for_tag(note.tags)` to find all notes with any matching tag
4. Filters out the current note itself
5. Displays a **Related Notes** table with title and shared tags columns

If the note has no tags, or no other notes share its tags, the related section is
omitted.

### Kasten schema — `on_view` hook

The Kasten overview shows a stats line (`N Zettel · K unique tags`) and a table of
the 10 most recent notes. "Most recent" is determined by sorting titles descending —
the `YYYY-MM-DD` prefix makes this correct without any date parsing.

### Sort tree actions

Two `add_tree_action` entries on Kasten let you reorder the tree by date from the
right-click menu: **Newest First** (descending title sort) and **Oldest First**
(ascending title sort).
```

**Step 2: Verify Hugo builds**

```bash
cd /Users/careck/Source/krillnotes-website && hugo server --buildDrafts 2>&1 | head -20
```
Expected: builds without errors. Open `http://localhost:1313/templates/zettelkasten/` and verify the page renders.

---

## Task 8: Website — add static download files

**Files:**
- Create: `static/templates/zettelkasten.rhai` — download copy
- Create: `static/templates/zettelkasten.krillnotes.zip` — sample workspace

**Step 1: Copy the script**

```bash
cp /Users/careck/Source/Krillnotes/.worktrees/feat/zettelkasten-template/templates/zettelkasten.rhai \
   /Users/careck/Source/krillnotes-website/static/templates/zettelkasten.rhai
```

**Step 2: Create the sample workspace**

1. Run the desktop app with `zettelkasten.rhai` loaded
2. Create a Kasten note named "My Zettelkasten"
3. Add 5 Zettel children with varied body text — mix subjects (e.g., philosophy, programming, nature)
4. Add native tags to each (e.g., `philosophy`, `emergence`, `systems`, `code`, `nature`) —
   make sure at least two notes share a tag so the Related Notes section appears
5. Export via **File → Export Workspace**
6. Save the result to `static/templates/zettelkasten.krillnotes.zip`

**Step 3: Take a screenshot**

Take a screenshot of the Kasten `on_view` showing the stats and recent notes table.
Save to: `static/templates/zettelkasten.screenshot.png`

**Step 4: Commit website changes**

```bash
cd /Users/careck/Source/krillnotes-website
git add content/templates/zettelkasten.md static/templates/
git commit -m "feat: add Zettelkasten template gallery page"
```

---

## Task 9: Open PR and finish

**Step 1: Open PR for Krillnotes changes**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/zettelkasten-template
gh pr create \
  --title "feat: zettelkasten template" \
  --body "Adds today() to Rhai engine, exposes note.tags in on_view hooks, and ships the Zettelkasten template (Zettel + Kasten schemas with auto-title, related-notes view, and date sort actions). See docs/plans/2026-02-26-zettelkasten-template-design.md."
```

**Step 2: After merge, remove the worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree remove .worktrees/feat/zettelkasten-template
```
