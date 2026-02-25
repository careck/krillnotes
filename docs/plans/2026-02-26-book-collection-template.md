# Book Collection Template — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove Book from system scripts, create a self-contained `book_collection.rhai` template with a Library folder view and sort tree actions, and add a Templates gallery section to the website.

**Architecture:** A single Rhai script in `templates/` defines both Book and Library schemas. Library uses `on_view` to display books grouped by status with per-section columns, and `add_tree_action` for five sort options. The website repo gets a new Templates nav section with the gallery page and downloadable static files.

**Tech Stack:** Rhai scripting, Hugo static site

---

## Task 1: Create feature branch and worktree

**Step 1: Create worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/book-collection-template -b feat/book-collection-template
```

**Step 2: Verify**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/book-collection-template
```
Expected: worktree directory exists with the repo contents.

**Step 3: All subsequent Krillnotes work happens inside this worktree path.**

```
/Users/careck/Source/Krillnotes/.worktrees/feat/book-collection-template/
```

---

## Task 2: Remove Book from system scripts

**Files:**
- Delete: `krillnotes-core/src/system_scripts/04_book.rhai`

**Step 1: Delete the file**

```bash
rm krillnotes-core/src/system_scripts/04_book.rhai
```

**Step 2: Verify the app still compiles**

```bash
cargo build -p krillnotes-core 2>&1 | tail -5
```
Expected: `Finished` with no errors. The system scripts are loaded at runtime, not compiled in — so removing the file has no compile impact.

**Step 3: Commit**

```bash
git add -u krillnotes-core/src/system_scripts/04_book.rhai
git commit -m "chore: remove Book from system scripts (moved to template gallery)"
```

---

## Task 3: Create `templates/book_collection.rhai` — Book schema

**Files:**
- Create: `templates/book_collection.rhai`

**Step 1: Create the templates directory and write the Book schema**

```rhai
// @name: Book Collection
// @description: A personal library. Includes a Book note type and a Library folder
// with grouped on_view (Currently Reading / To Read / Read) and sort tree actions.
//
// Usage: Create a Library note, then add Book notes as children.

schema("Book", #{
    title_can_edit: false,
    allowed_parent_types: ["Library"],
    fields: [
        #{ name: "book_title",    type: "text",     required: true                   },
        #{ name: "author",        type: "text",     required: true                   },
        #{ name: "genre",         type: "text",     required: false                  },
        #{ name: "status",        type: "select",   required: false,
           options: ["To Read", "Reading", "Read"]                                   },
        #{ name: "rating",        type: "rating",   required: false, max: 5          },
        #{ name: "started",       type: "date",     required: false                  },
        #{ name: "finished",      type: "date",     required: false                  },
        #{ name: "notes",         type: "textarea", required: false                  },
        #{ name: "read_duration", type: "text",     required: false, can_edit: false },
    ],
    on_save: |note| {
        let title  = note.fields["book_title"];
        let author = note.fields["author"];

        note.title = if author != "" && title != "" {
            author + ": " + title
        } else if title != "" {
            title
        } else {
            "Untitled Book"
        };

        let started  = note.fields["started"];
        let finished = note.fields["finished"];
        note.fields["read_duration"] = if type_of(started) == "string" && started != ""
            && type_of(finished) == "string" && finished != "" {
            let s_parts = started.split("-");
            let f_parts = finished.split("-");
            let s_days = parse_int(s_parts[0]) * 365
                       + parse_int(s_parts[1]) * 30
                       + parse_int(s_parts[2]);
            let f_days = parse_int(f_parts[0]) * 365
                       + parse_int(f_parts[1]) * 30
                       + parse_int(f_parts[2]);
            let diff = f_days - s_days;
            if diff > 0 { diff.to_string() + " days" } else { "" }
        } else {
            ""
        };

        note
    }
});
```

**Step 2: Verify it compiles by running the desktop app and importing the script**

Open the app → Script Manager → Import → select `templates/book_collection.rhai`.
Expected: script appears in the list with no compile errors shown.

---

## Task 4: Add Library schema with `on_view` to `book_collection.rhai`

**Files:**
- Modify: `templates/book_collection.rhai` (append below the Book schema)

**Step 1: Append the Library schema**

```rhai
schema("Library", #{
    allowed_children_types: ["Book"],
    fields: [],
    on_view: |note| {
        let books = get_children(note.id);
        if books.len() == 0 {
            return text("No books yet. Right-click to add a Book.");
        }

        let reading = books.filter(|b| (b.fields["status"] ?? "") == "Reading");
        let to_read = books.filter(|b| (b.fields["status"] ?? "") == "To Read");
        let read    = books.filter(|b| (b.fields["status"] ?? "") == "Read");
        let unsorted = books.filter(|b| {
            let s = b.fields["status"] ?? "";
            s != "Reading" && s != "To Read" && s != "Read"
        });

        let sections = [];

        if reading.len() > 0 {
            let rows = reading.map(|b| [
                b.title,
                b.fields["author"] ?? "-",
                b.fields["started"] ?? "-"
            ]);
            sections += [section(
                "Currently Reading (" + reading.len() + ")",
                table(["Title", "Author", "Started"], rows)
            )];
        }

        if to_read.len() > 0 {
            let rows = to_read.map(|b| [
                b.title,
                b.fields["author"] ?? "-",
                b.fields["genre"] ?? "-"
            ]);
            sections += [section(
                "To Read (" + to_read.len() + ")",
                table(["Title", "Author", "Genre"], rows)
            )];
        }

        if read.len() > 0 {
            let rows = read.map(|b| {
                let r = b.fields["rating"] ?? 0;
                let stars = "";
                let i = 0;
                while i < r    { stars += "★"; i += 1; }
                while i < 5    { stars += "☆"; i += 1; }
                [
                    b.title,
                    b.fields["author"]   ?? "-",
                    b.fields["finished"] ?? "-",
                    if r > 0 { stars } else { "—" }
                ]
            });
            sections += [section(
                "Read (" + read.len() + ")",
                table(["Title", "Author", "Finished", "Rating"], rows)
            )];
        }

        if unsorted.len() > 0 {
            let rows = unsorted.map(|b| [
                b.title,
                b.fields["author"] ?? "-"
            ]);
            sections += [section(
                "Unsorted (" + unsorted.len() + ")",
                table(["Title", "Author"], rows)
            )];
        }

        stack(sections)
    }
});
```

**Step 2: Manually verify in the app**

- Create a Library note
- Add several Book children with different status values ("Reading", "To Read", "Read", and one with no status)
- Click the Library note in view mode
- Expected: correct sections appear, only non-empty sections show, star rating renders for "Read" books

---

## Task 5: Add sort tree actions to `book_collection.rhai`

**Files:**
- Modify: `templates/book_collection.rhai` (append below the Library schema)

**Step 1: Append all five tree actions**

```rhai
add_tree_action("Sort by Title (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

add_tree_action("Sort by Author (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["author"] ?? "") <= (b.fields["author"] ?? "")
    );
    children.map(|c| c.id)
});

add_tree_action("Sort by Genre (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["genre"] ?? "") <= (b.fields["genre"] ?? "")
    );
    children.map(|c| c.id)
});

add_tree_action("Sort by Rating (High→Low)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["rating"] ?? 0) >= (b.fields["rating"] ?? 0)
    );
    children.map(|c| c.id)
});

add_tree_action("Sort by Date Read (Newest First)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["finished"] ?? "") >= (b.fields["finished"] ?? "")
    );
    children.map(|c| c.id)
});
```

**Step 2: Manually verify in the app**

- Right-click the Library note in the tree
- Expected: five sort actions appear in the context menu
- Test "Sort by Author" — books should reorder by author name A→Z
- Test "Sort by Rating" — highest-rated books should move to the top
- Test "Sort by Date Read" — books with the most recent `finished` date should appear first; books with no `finished` date sink to the bottom (empty string sorts before any date)

**Step 3: Commit**

```bash
git add templates/book_collection.rhai
git commit -m "feat: add book_collection template with Library on_view and sort actions"
```

---

## Task 6: Website — add Templates nav section

**Working directory for all website tasks:** `/Users/careck/Source/krillnotes-website`

> Note: website changes go directly on the main/master branch of the website repo unless it has its own branch convention.

**Files:**
- Modify: `hugo.toml`
- Create: `content/templates/_index.md`

**Step 1: Add Templates to the nav in `hugo.toml`**

Add after the Scripting entry (weight 4), before GitHub (weight 5):

```toml
[[menu.main]]
  name = 'Templates'
  url = '/templates/'
  weight = 5
[[menu.main]]
  name = 'GitHub'
  url = 'https://github.com/careck/krillnotes'
  weight = 6
```

**Step 2: Create the templates section index**

`content/templates/_index.md`:

```markdown
---
title: "Templates"
description: "Ready-to-use Krillnotes templates. Download and import to get started."
---

Downloadable templates for common use cases. Each template includes a Rhai script
to import into the Script Manager and an optional sample workspace to explore.
```

**Step 3: Verify Hugo builds**

```bash
cd /Users/careck/Source/krillnotes-website && hugo server --buildDrafts 2>&1 | head -20
```
Expected: site builds without errors, Templates appears in the nav.

---

## Task 7: Website — add Book Collection gallery page

**Files:**
- Create: `content/templates/book-collection.md`

**Step 1: Write the gallery page**

```markdown
---
title: "Book Collection"
description: "Track your reading list, current books, and past reads with star ratings."
---

A personal library template. Create a **Library** note as the root, then add
**Book** notes as children. The Library view groups your books by reading status
and lets you sort them in several ways from the right-click menu.

## Downloads

- [book_collection.rhai](/templates/book_collection.rhai) — import into Script Manager
- [book_collection.krillnotes](/templates/book_collection.krillnotes) — sample workspace

## How to use

1. Import `book_collection.rhai` in **Settings → Scripts → Import Script**
2. Create a new note and choose **Library** as the type
3. Add children and choose **Book** as the type for each
4. Fill in the book details — title, author, genre, and reading status
5. Click the Library note to see your books grouped by status
6. Right-click the Library note to sort by title, author, genre, rating, or date read

## How it works

### Book schema — `on_save` hook

The Book schema derives two fields automatically when you save:

- **Title** is computed as `"Author: Book Title"` so books sort correctly by author
  when using the tree sort actions.
- **Read duration** is derived from the `started` and `finished` dates as `"N days"`.

```rhai
on_save: |note| {
    note.title = note.fields["author"] + ": " + note.fields["book_title"];
    // ... read_duration calculation
    note
}
```

### Library schema — `on_view` hook

The Library view calls `get_children` to fetch all Book notes, then partitions them
by `status` and builds a section for each group using the `section`, `table`, and
`stack` display helpers:

- **Currently Reading** — title, author, started date
- **To Read** — title, author, genre
- **Read** — title, author, finished date, star rating

Empty sections are omitted. Books without a status appear in an Unsorted section.

### Sort tree actions

Five `add_tree_action` entries add sort options to the Library right-click menu.
ISO date strings sort correctly as plain strings, so "Sort by Date Read" works
without any date parsing — books without a finished date sink to the bottom.
```

**Step 2: Verify page renders**

```bash
hugo server --buildDrafts
```
Open `http://localhost:1313/templates/book-collection/` in a browser.
Expected: page renders with correct nav highlight on Templates.

---

## Task 8: Website — add static download files

**Files:**
- Create: `static/templates/book_collection.rhai` — copy of the script
- Create: `static/templates/book_collection.krillnotes` — sample export (see note)

**Step 1: Copy the script from the Krillnotes repo**

```bash
cp /Users/careck/Source/Krillnotes/.worktrees/feat/book-collection-template/templates/book_collection.rhai \
   /Users/careck/Source/krillnotes-website/static/templates/book_collection.rhai
```

**Step 2: Create the sample export**

The `.krillnotes` export file is created by:
1. Running the desktop app with the `book_collection.rhai` script loaded
2. Creating a Library note with 5–8 sample books covering all three statuses and a range of ratings
3. Using **File → Export Workspace** (or the export menu action) to export
4. Saving the result to `static/templates/book_collection.krillnotes`

> If export is not yet available in the current build, skip this file for now and add a note to the gallery page that the sample export is coming soon.

**Step 3: Add the screenshot**

Take a screenshot of the Library `on_view` with sample books and save it to:
`static/templates/book_collection-screenshot.png`

Then add it to the gallery page in `content/templates/book-collection.md` below the title:
```markdown
![Book Collection Library view](/templates/book_collection-screenshot.png)
```

**Step 4: Commit website changes**

```bash
cd /Users/careck/Source/krillnotes-website
git add hugo.toml content/templates/ static/templates/
git commit -m "feat: add Book Collection template gallery page"
```

---

## Task 9: Merge and finish

**Step 1: Open PR for the Krillnotes changes**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/book-collection-template
gh pr create --title "feat: book collection template" \
  --body "Moves Book out of system scripts and adds the Book Collection template with Library on_view and sort tree actions. See docs/plans/2026-02-26-book-collection-template-design.md."
```

**Step 2: After merge, remove the worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree remove .worktrees/feat/book-collection-template
```
