# Krillnotes Scripting Guide

Scripts in Krillnotes are written in [Rhai](https://rhai.rs), a small, fast scripting language
embedded in the application. Each script defines **schemas** (note types) and/or **presentation
logic** (views, hover tooltips, context-menu actions).

User scripts are managed through **View → Scripts**. The bundled system scripts
(TextNote, Contact, Task, Project, etc.) are always available and serve as working examples.

---

## Table of Contents

1. [Script structure](#1-script-structure)
2. [Defining schemas](#2-defining-schemas)
3. [Field types](#3-field-types)
4. [Schema options](#4-schema-options)
5. [`on_save` hook](#5-on_save-hook)
6. [Field validation](#6-field-validation)
7. [Field groups](#7-field-groups)
8. [`register_view`](#8-register_view)
9. [`register_hover`](#9-register_hover)
10. [`on_add_child` hook](#10-on_add_child-hook)
11. [`register_menu`](#11-register_menu)
12. [Schema versioning and migrations](#12-schema-versioning-and-migrations)
13. [Display helpers](#13-display-helpers)
14. [Query functions](#14-query-functions)
15. [Utility functions](#15-utility-functions)
16. [Introspection functions](#16-introspection-functions)
17. [Tips and patterns](#17-tips-and-patterns)
18. [Built-in script examples](#18-built-in-script-examples)

---

## 1. Script structure

Scripts are divided into two **categories**:

| Category | File extension | Allowed top-level calls |
|---|---|---|
| **Schema** | `.schema.rhai` | `schema()` and optionally `register_view/hover/menu()` |
| **Library/Presentation** | `.rhai` | `register_view()`, `register_hover()`, `register_menu()`, helper functions — **not** `schema()` |

Calling `schema()` from a `.rhai` (presentation) script is a hard error. Scripts in the
Script Manager carry a **category** setting — "Schema" or "Library" — chosen when the script
is created.

### Loading order

When a workspace opens, scripts run in four phases:

1. **Phase A — Presentation** (`.rhai` scripts by `load_order`): define helper functions and queue deferred `register_*` calls.
2. **Phase B — Schema** (`.schema.rhai` scripts by `load_order`): call `schema()` to register note types.
3. **Phase C — Resolve bindings**: match deferred `register_*` calls to registered schemas. Unresolved entries show a warning badge in the Script Manager.
4. **Phase D — Migrations**: for each schema, find notes with `schema_version < current version`, run `migrate` closures, and write back in one transaction per type.

Library-first ordering (Phase A before B) means **functions** defined in `.rhai` files are
available when schema scripts load and when hooks run.

> **Important:** top-level `const` and variable declarations in library scripts are **not**
> available inside hook closures. Only function definitions cross the boundary. If you need
> shared constants, wrap them in a function — see [§17 Tips and patterns](#17-tips-and-patterns).

### Minimal examples

**Schema script (`MyType.schema.rhai`):**

```rhai
// @name: MyType
// @description: My custom note type

schema("MyType", #{
    version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ],
    on_save: |note| {
        commit();
    }
});
```

**Presentation script (`MyType.rhai`):**

```rhai
// @name: MyType Views
// @description: Views and actions for MyType

register_view("MyType", "Overview", |note| {
    text(note.fields["body"] ?? "")
});
```

A script can contain any number of `schema()` or `register_*()` calls, provided it follows the
category rule. Keep related types together in a single file.

---

## 2. Defining schemas

```rhai
schema("TypeName", #{
    // --- required ---
    version: 1,

    // --- optional schema-level options ---
    title_can_view:         true,          // default: true
    title_can_edit:         true,          // default: true
    children_sort:          "asc",         // "asc" | "desc" | "none" (default)
    allowed_parent_types:   ["Folder"],    // default: [] (any parent allowed)
    allowed_children_types: ["Item"],      // default: [] (any child allowed)

    // --- required ---
    fields: [
        #{ name: "field_name", type: "text", required: true },
        // … more fields …
    ],

    // --- optional field groups ---
    field_groups: [
        #{ name: "Section title", fields: ["field_name"], visible: |note| true },
    ],

    // --- optional migrations ---
    migrate: #{
        // 2: |note| { … }
    },

    // --- optional hooks ---
    on_save:      |note| { /* … */ commit() },
    on_add_child: |parent_note, child_note| { /* … */ commit() },
});
```

The `version` key is **required**. Omitting it causes the script to fail loading with an error.

View rendering, hover tooltips, and context-menu actions are **not** defined inside `schema()`.
Use `register_view()`, `register_hover()`, and `register_menu()` in a presentation script instead.

### Schema name uniqueness

Schema names must be unique across all scripts. If two scripts register the same name the
**first to load wins** (scripts run in ascending `load_order`). The second script fails to load
and an error is shown in the Script Manager.

### Field definition

Each entry in `fields` is a map:

```rhai
#{
    name:          "my_field",   // required — snake_case string
    type:          "text",       // required — see Field types below
    required:      false,        // optional — default: false
    can_view:      true,         // optional — show in view mode (default: true)
    can_edit:      true,         // optional — show in edit mode (default: true)
    show_on_hover: false,        // optional — show in hover tooltip (default: false)
    options:       ["A", "B"],   // required for "select" fields
    max:           5,            // required for "rating" fields
    validate:      |v| (),       // optional — return an error string or ()
}
```

`can_edit: false` marks a derived/computed field — it can be written by an `on_save` hook
but users cannot change it directly.

---

## 3. Field types

| Type | Storage | Notes |
|---|---|---|
| `"text"` | String | Single-line text input |
| `"textarea"` | String | Multi-line text input; auto-rendered as markdown in view mode |
| `"number"` | Float | Numeric input |
| `"boolean"` | Bool | Checkbox |
| `"date"` | String (ISO `YYYY-MM-DD`) or `null` | Date picker |
| `"email"` | String | Email input with mailto link in view mode |
| `"select"` | String | Dropdown; requires `options: [...]` |
| `"rating"` | Float | Star rating; requires `max: N` (e.g. `max: 5`) |
| `"note_link"` | String (UUID) or `null` | Link to another note; optional `target_type` restricts the picker to notes of that schema type |
| `"file"` | String (UUID) or `null` | Attachment reference; optional `allowed_types` restricts the file picker to specific MIME types. In view mode images render as a thumbnail; other files show a paperclip icon and filename. |

### Reading field values in hooks

Inside a hook, fields are accessed via `note.fields["field_name"]` or `note.fields.field_name`.
The bracket syntax is safer when the field might not exist:

```rhai
let val = note.fields["notes"] ?? "";   // returns "" if the field is absent
```

Dates arrive as a string `"YYYY-MM-DD"` when set, or as the unit value `()` when empty:

```rhai
let d = note.fields["due_date"];
if type_of(d) == "string" && d != "" {
    // safe to use d as a string
}
```

`note_link` fields arrive as a UUID string when set, or `()` when empty:

```rhai
let linked_id = note.fields["linked_project"];
if linked_id != () {
    let target = get_note(linked_id);
    if target != () {
        field("Project", link_to(target))
    }
}
```

### `note_link` field options

| Option | Type | Description |
|---|---|---|
| `target_type` | String (optional) | If set, the note-picker in edit mode only shows notes of this schema type. |

### `file` field options

| Option | Type | Description |
|---|---|---|
| `allowed_types` | Array of strings (optional) | MIME type filters for the file picker (e.g. `["image/*", "application/pdf"]`). |

### Inline images in `textarea` markdown

`textarea` fields rendered as markdown support an inline image block syntax:

```
{{image: field:cover, width: 400, alt: My caption}}
{{image: attach:photo.png}}
```

The `field:` prefix reads the UUID from a `file` field. The `attach:` prefix finds an
attachment by filename. `width` and `alt` are optional.

---

## 4. Schema options

### `version: N` *(required)*

Declares the current data contract version. Must be an integer ≥ 1. All notes created or
saved with this schema will have their `schema_version` stamped with this value.

See [Schema versioning and migrations](#12-schema-versioning-and-migrations) for details.

### `title_can_edit: false`

Hides the title input in edit mode. Use this when the title is always derived by an
`on_save` hook (e.g. Contacts: `"Smith, Jane"`).

### `title_can_view: false`

Hides the title entirely in view mode. Rarely needed.

### `children_sort: "asc" | "desc"`

Automatically sorts child notes alphabetically by title when displayed in the tree.
Default is `"none"` (manual/insertion order).

### `allowed_parent_types: [...]`

Restricts which note types this type may be placed under. An empty array means no restriction.

```rhai
allowed_parent_types: ["ContactsFolder"],
```

### `allowed_children_types: [...]`

Restricts which note types may be placed inside this type.

```rhai
allowed_children_types: ["Contact"],
```

> **Validation order:** `allowed_parent_types` and `allowed_children_types` are always checked
> **before** any hook runs. If validation fails the operation is aborted and no hook fires.

### `field_groups: [...]`

See [Field groups](#7-field-groups).

### `migrate: #{ N: |note| { … } }`

See [Schema versioning and migrations](#12-schema-versioning-and-migrations).

---

## 5. `on_save` hook

The `on_save` hook runs every time a note is saved. It is defined as a key inside `schema()`.
Rather than mutating the note directly and returning it, the hook uses a **transactional API**:
call `set_field()` and `set_title()` to queue writes, optionally call `reject()` to signal
errors, then call `commit()` to apply everything atomically.

```rhai
schema("TypeName", #{
    version: 1,
    fields: [ /* … */ ],
    on_save: |note| {
        // Read fields directly from note.fields (read-only access)
        let name = note.fields["name"] ?? "";

        // Queue writes
        set_title(note.id, name);
        set_field(note.id, "summary", "Hello, " + name);

        // Apply all queued writes
        commit();
    }
});
```

### SaveTransaction functions

| Function | Description |
|---|---|
| `set_field(note_id, field_name, value)` | Queues a field write. Runs the field's `validate` closure immediately (hard error on failure). Read-your-writes: `note.fields` is updated in place. |
| `set_title(note_id, title)` | Queues a title write. Updates `note.title` in place. |
| `reject(message)` | Records a note-level error. Does **not** abort immediately — use `commit()` to trigger the abort. |
| `reject(field_name, message)` | Records a field-pinned error shown below the named field. |
| `commit()` | Runs required-field checks on all visible fields. If any `reject()` calls were made, aborts the save and surfaces all errors. Otherwise applies all queued writes atomically. **Always call `commit()` at the end of `on_save`.** |

The hook receives the note as a map for **field reading only**. All writes must go through
`set_field` or `set_title`. Both functions provide read-your-writes semantics — calling
`set_field` then reading `note.fields["that_field"]` gives back the queued value.

### The `note` map inside `on_save`

| Key | Type | Notes |
|---|---|---|
| `note.id` | String | — |
| `note.schema` | String | — |
| `note.title` | String | Updated by `set_title()` (read-your-writes) |
| `note.fields` | Map | Updated by `set_field()` (read-your-writes) |
| `note.tags` | Array of strings | Read-only |

### Example — derived title

```rhai
schema("Book", #{
    version: 1,
    fields: [
        #{ name: "book_title", type: "text", required: true },
        #{ name: "author",     type: "text", required: false },
    ],
    on_save: |note| {
        let title  = note.fields["book_title"] ?? "";
        let author = note.fields["author"] ?? "";
        let derived = if author != "" && title != "" { author + ": " + title }
                      else if title != "" { title }
                      else { "Untitled Book" };
        set_title(note.id, derived);
        commit();
    }
});
```

### Example — status badge

```rhai
schema("Task", #{
    version: 1,
    fields: [
        #{ name: "name",   type: "text",   required: true },
        #{ name: "status", type: "select", required: true,
           options: ["TODO", "WIP", "DONE"] },
    ],
    title_can_edit: false,
    on_save: |note| {
        let name   = note.fields["name"] ?? "";
        let status = note.fields["status"] ?? "";
        let symbol = if status == "DONE" { "✓" }
                     else if status == "WIP" { "→" }
                     else { " " };
        set_title(note.id, "[" + symbol + "] " + name);
        commit();
    }
});
```

### Example — reject on invalid input

```rhai
schema("Invoice", #{
    version: 1,
    fields: [
        #{ name: "amount", type: "number", required: true },
    ],
    on_save: |note| {
        if (note.fields["amount"] ?? 0.0) <= 0.0 {
            reject("amount", "Amount must be greater than zero");
        }
        commit();
    }
});
```

If `reject()` is called, `commit()` aborts and the error is shown to the user. The note is
not saved.

---

## 6. Field validation

Individual fields can declare a `validate` closure that returns an error string (on failure)
or `()` (on success):

```rhai
#{
    name: "email", type: "email", required: false,
    validate: |v| {
        if v == () || v == "" { return (); }  // empty is OK; required: true handles must-have
        if v.contains("@") { () }
        else { "Must be a valid email address" }
    }
}
```

Validation runs:

- **On blur** in the frontend — the error appears inline below the field.
- **Inside `set_field()`** — a failed `validate` closure is a hard error that aborts `on_save` immediately (before `commit()` runs).

The closure receives the raw field value — a string, number, boolean, or `()` (empty). Always
guard against `()` before type-specific operations unless the field is `required: true`.

---

## 7. Field groups

Field groups visually organise related fields under collapsible sections in the edit panel.
Define them via the `field_groups` key inside `schema()`:

```rhai
schema("Project", #{
    version: 1,
    fields: [
        #{ name: "name",         type: "text",     required: true },
        #{ name: "status",       type: "select",   required: true,
           options: ["Active", "On Hold", "Done"] },
        #{ name: "completed_at", type: "date",     required: false },
        #{ name: "notes",        type: "textarea", required: false },
    ],
    field_groups: [
        #{
            name:    "Completion details",
            fields:  ["completed_at", "notes"],
            visible: |note| note.fields["status"] == "Done",
        },
    ],
    on_save: |note| { commit(); }
});
```

### Group definition

| Key | Type | Required | Description |
|---|---|---|---|
| `name` | String | Yes | Header label shown above the group |
| `fields` | Array of strings | Yes | Field names to include in this group |
| `visible` | Closure `\|note\| → bool` | No | Returns `false` to hide the entire group |

Fields not listed in any group are shown ungrouped at the top of the edit panel.

The `visible` closure receives the current note map and is re-evaluated on every field value
change in the frontend, so groups can appear and disappear interactively.

---

## 8. `register_view`

`register_view` registers a named view tab for a note type. Call it from a presentation
script (`.rhai`). The view renders when the user selects that tab in the detail panel.

```rhai
// Simple form
register_view("TypeName", "Tab Label", |note| {
    text("Custom view for " + note.title)
});

// With options
register_view("TypeName", "Tab Label", #{ display_first: true }, |note| {
    stack([
        heading(note.title),
        text(note.fields["body"] ?? "")
    ])
});
```

### Parameters

| Parameter | Type | Description |
|---|---|---|
| `type` | String | The schema name to bind this view to |
| `label` | String | Tab label shown in the UI |
| `options` | Map (optional) | `#{ display_first: true }` pushes the tab to the leftmost position |
| `closure` | `\|note\| → String` | Returns HTML built with display helpers |

### Tab layout

```
[ display_first views ] [ other views in order ] [ Fields ]
```

- **No registered views** — no tab bar is shown; the detail panel renders as a plain field grid (unchanged from before).
- **Fields tab** — always present, always rightmost.
- **Edit mode** — clicking "Edit" switches to the Fields tab. Saving or cancelling returns to the previously active tab.

The closure has access to all [query functions](#14-query-functions) and
[display helpers](#13-display-helpers).

### Example — folder contact table

```rhai
register_view("ContactsFolder", "Contacts", #{ display_first: true }, |note| {
    let contacts = get_children(note.id);
    if contacts.len() == 0 {
        return text("No contacts yet. Add one via the context menu.");
    }
    let rows = contacts.map(|c| [
        link_to(c),
        c.fields["email"]  ?? "-",
        c.fields["phone"]  ?? "-",
    ]);
    let notes_val = note.fields["notes"] ?? "";
    let contacts_section = section(
        "Contacts (" + contacts.len() + ")",
        table(["Name", "Email", "Phone"], rows)
    );
    if notes_val == "" { contacts_section }
    else { stack([contacts_section, section("Notes", text(notes_val))]) }
});
```

### Unresolved bindings

If `register_view` references a type name that no script has registered, the binding is
marked *unresolved* and a warning badge appears next to the script in the Script Manager.

---

## 9. `register_hover`

`register_hover` registers a hover tooltip renderer for a note type. Call it from a
presentation script. One registration per type — last registration wins.

```rhai
register_hover("TypeName", |note| {
    field("Status", note.fields["status"] ?? "-")
});
```

### Parameters

| Parameter | Type | Description |
|---|---|---|
| `type` | String | The schema name |
| `closure` | `\|note\| → String` | Returns HTML shown in the tooltip |

The tooltip appears after ~600 ms of hover. Keep output brief — the tooltip has a fixed max
width and is not scrollable.

### Simple path — `show_on_hover: true`

For a quick single-field preview, mark the field with `show_on_hover: true` and skip the
hook entirely. No IPC round-trip is needed — the value is already in the frontend.

```rhai
schema("Note", #{
    version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false, show_on_hover: true },
    ],
    on_save: |note| { commit(); }
});
```

Multiple `show_on_hover` fields are all shown in definition order.

> **Priority:** A `register_hover` closure always takes precedence over `show_on_hover` flags.
> The flags are only used when no hover registration exists for the type.

---

## 10. `on_add_child` hook

The `on_add_child` hook runs whenever a note is created as a child — or moved via
drag-and-drop — under a note whose schema defines the hook. Both the parent and the child
are pre-seeded into the current SaveTransaction.

```rhai
schema("TypeName", #{
    version: 1,
    fields: [ /* … */ ],
    on_add_child: |parent_note, child_note| {
        // Modify parent and/or child via the SaveTransaction API
        set_field(parent_note.id, "child_count",
                  (parent_note.fields["child_count"] ?? 0.0) + 1.0);
        commit();
    }
});
```

Use `set_field`, `set_title`, and `reject`/`commit()` just like in `on_save`.
Both `parent_note` and `child_note` are available by ID.

### When it fires

| Operation | Fires? |
|---|---|
| Note created as a child | Yes |
| Note moved under a new parent (drag-and-drop) | Yes |
| Note created at root level (no parent) | No |

`allowed_parent_types` and `allowed_children_types` checks always run **before** the hook.
If either check fails, the operation is aborted and the hook never runs.

### Example — child count in parent title

```rhai
schema("ContactsFolder", #{
    version: 1,
    fields: [
        #{ name: "child_count", type: "number", can_view: true, can_edit: false },
    ],
    on_add_child: |parent_note, child_note| {
        let count = (parent_note.fields["child_count"] ?? 0.0) + 1.0;
        set_field(parent_note.id, "child_count", count);
        set_title(parent_note.id, "Contacts (" + count.to_int().to_string() + ")");
        commit();
    }
});
```

Note: this count only increases on add. It does not decrease when notes are deleted or moved
away. For a live accurate count use `register_view` with `get_children()` instead.

---

## 11. `register_menu`

`register_menu` registers a custom entry in the tree's right-click context menu. Call it
from a presentation script (`.rhai`).

```rhai
register_menu(label, target_types, callback)
```

| Parameter | Type | Description |
|---|---|---|
| `label` | String | Menu item text shown to the user |
| `target_types` | Array of Strings | Schema names for which the item appears |
| `callback` | Closure `\|note\| { … }` | Called when the user clicks the item |

The `note` argument has the same shape as in `on_save`. The closure can:

- Use [query functions](#14-query-functions) to read workspace state.
- Use SaveTransaction functions (`set_field`, `set_title`, `create_child`, `commit`) to write.
- **Return an array of note ID strings** to reorder child notes.

```rhai
register_menu("Sort Children A→Z", ["Folder"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
```

### Mutating notes from a menu action

Use `create_child(parent_id, type)` to create new notes and `set_field`/`set_title` to
modify them, then call `commit()`:

```rhai
register_menu("Create Sprint Template", ["TextNote"], |container| {
    let sprint = create_child(container.id, "TextNote");
    set_title(sprint.id, "Sprint 1");
    set_field(sprint.id, "body", "Sprint goals: TBD");

    let t1 = create_child(sprint.id, "Task");
    set_title(t1.id, "[ ] Define scope");
    set_field(t1.id, "name", "Define scope");
    set_field(t1.id, "status", "TODO");

    commit();
});
```

> **`on_save` is not invoked** for notes created via `create_child`. Schemas that derive their
> title from fields (such as `Task`) require the title to be set manually.

`create_child` is **only available in `register_menu` closures** and `on_add_child` hooks.
It is not available in `on_save` or view/hover closures.

---

## 12. Schema versioning and migrations

The `version` key in `schema()` declares the current data contract version. When you change a
schema's fields in a breaking way (renaming, splitting, or removing a field), bump the version
and add a `migrate` closure so existing notes are updated automatically.

```rhai
schema("Contact", #{
    version: 2,
    fields: [
        // "phone" renamed to "mobile" in v2
        #{ name: "first_name", type: "text", required: true },
        #{ name: "last_name",  type: "text", required: true },
        #{ name: "mobile",     type: "text", required: false },
    ],
    migrate: #{
        2: |note| {
            note.fields["mobile"] = note.fields["phone"];
            note.fields.remove("phone");
        }
    },
    on_save: |note| {
        set_title(note.id,
            (note.fields["last_name"] ?? "") + ", " + (note.fields["first_name"] ?? ""));
        commit();
    }
});
```

### How it works

When the workspace opens, **Phase D** runs after all scripts load:

1. For each registered schema, find all notes with `schema_version < current version`.
2. Chain migration closures in order (e.g. a note at v1 with a v3 schema runs the v2 closure then the v3 closure).
3. Write updated `title`, `fields`, and `schema_version` back in a single transaction per schema type.
4. Log one `UpdateSchema` operation recording how many notes were migrated.
5. Emit a `schema-migrated` event → a toast notification appears in the workspace.

### Migration closure contract

```rhai
migrate: #{
    2: |note| {
        // note.title — readable and writable
        // note.fields — mutable map of field values
        note.fields["mobile"] = note.fields["phone"];
        note.fields.remove("phone");
        // no return value; do NOT call set_field() or commit()
    }
}
```

The closure receives a map with `title` (String) and `fields` (Map). Mutate in place. Do
**not** call `set_field()` or `commit()` — migrations bypass the gated pipeline.

### Multi-version jump

```rhai
schema("Contact", #{
    version: 3,
    fields: [ /* … */ ],
    migrate: #{
        2: |note| {
            // v1 → v2: rename phone to mobile
            note.fields["mobile"] = note.fields["phone"];
            note.fields.remove("phone");
        },
        3: |note| {
            // v2 → v3: split name into first_name + last_name
            let parts = note.fields["name"].split(" ");
            note.fields["first_name"] = parts[0];
            note.fields["last_name"] = if parts.len() > 1 { parts[1] } else { "" };
            note.fields.remove("name");
        }
    },
    on_save: |note| { /* … */ commit(); }
});
```

A note at v1 runs closures 2 then 3. A note at v2 runs only closure 3.

### Rules

| Condition | Behaviour |
|---|---|
| `version` omitted | Hard error at load time — script fails to register |
| New version < registered version | Hard error — downgrade not allowed |
| New version == registered version | Allowed — hooks/fields can be updated freely |
| New version > registered version | Allowed — Phase D migration runs on next open |
| Migration closure fails | Entire batch for that schema type rolls back; error shown in Script Manager |

### When to bump the version

Only bump when the **stored data shape** changes in a way old data cannot satisfy the new
schema. Examples: renaming a field, splitting one field into two, changing a field's type.
Do **not** bump for: adding a new optional field, changing `on_save` logic, updating
`on_add_child`, or modifying view/hover/menu registrations.

---

## 13. Display helpers

All helpers return an HTML string. All user-supplied text is HTML-escaped automatically.
They are available in `register_view`, `register_hover`, and `on_add_child` closures.

### `text(content)`

Whitespace-preserving paragraph.

```rhai
text("Line one\nLine two")
```

### `markdown(text)`

Renders a string as **CommonMark markdown** and returns the resulting HTML.

```rhai
markdown(note.fields["notes"] ?? "")
```

In the default view (no registered view) `textarea` fields are already auto-rendered as
markdown. Use `markdown()` explicitly in `register_view` closures when you want markdown
alongside other helpers.

#### Inline image blocks in markdown

```
{{image: field:cover, width: 400, alt: My caption}}
{{image: attach:photo.png}}
```

| Parameter | Required | Description |
|---|---|---|
| first positional | Yes | `field:fieldName` reads the UUID from a `file` field; `attach:filename` finds by filename |
| `width` | No | Pixel width. Omit to use natural width. |
| `alt` | No | Alt text for accessibility. |

### `heading(text)`

A bold section heading.

```rhai
heading("Project Details")
```

### `field(label, value)`

A single key-value row with a muted label.

```rhai
field("Email", note.fields["email"] ?? "-")
```

### `fields(note)`

Renders all fields in the note as key-value rows, skipping empty values. Field key names
are humanised (`"first_name"` → `"First Name"`).

```rhai
fields(note)
```

### `table(headers, rows)`

A table with a header row. `headers` is an array of strings; `rows` is an array of arrays.

```rhai
let rows = contacts.map(|c| [c.title, c.fields["email"] ?? "-"]);
table(["Name", "Email"], rows)
```

### `section(title, content)`

Wraps content in a titled container with an uppercase small-caps label above.

```rhai
section("Notes", text(note.fields["notes"] ?? ""))
```

### `stack(items)`

Lays items out vertically with consistent spacing.

```rhai
stack([
    section("Overview", fields(note)),
    divider(),
    section("Tasks", list(tasks.map(|t| t.title)))
])
```

### `columns(items)`

Lays items out as equal-width columns side by side.

```rhai
columns([
    section("Left", text("...")),
    section("Right", text("..."))
])
```

### `list(items)`

A bullet list. Items are strings.

```rhai
list(tasks.map(|t| t.title))
```

### `badge(text)` / `badge(text, color)`

A pill badge. Supported colors: `"red"`, `"green"`, `"blue"`, `"yellow"`, `"gray"`,
`"orange"`, `"purple"`.

```rhai
badge("Active")
badge("High", "red")
badge("Done", "green")
```

### `render_tags(tags)`

Renders an array of tag strings as coloured pill badges.

```rhai
render_tags(note.tags)
```

### `stars(value)` / `stars(value, max)`

Renders a numeric rating as filled (★) and empty (☆) star characters. Default scale is 5.
Returns `"—"` for a zero or negative value.

```rhai
stars(note.fields["rating"] ?? 0)        // e.g. "★★★☆☆" for 3 out of 5
stars(note.fields["score"] ?? 0, 10)     // out of 10
```

### `display_image(uuid, width, alt)`

Embeds an attached image inline. The image is base64-encoded server-side and renders
synchronously.

```rhai
display_image(note.fields["cover"], 400, "Cover image")
```

### `display_download_link(uuid, label)`

Renders a clickable download link for an attachment.

```rhai
display_download_link(note.fields["document"], "Download PDF")
```

### `divider()`

A horizontal rule.

```rhai
divider()
```

### `link_to(note)`

Renders a clickable link that navigates to another note. Pushes the originating note onto
the back-navigation stack.

```rhai
let target = get_note(some_id);
if target != () { link_to(target) }
```

---

## 14. Query functions

Query functions are available inside `register_view`, `register_hover`, and `register_menu`
closures. They let you fetch related notes from the workspace without leaving the scripting layer.

### `get_children(note_id)`

Returns an array of direct child notes for the given ID.

```rhai
let items = get_children(note.id);
```

### `get_note(note_id)`

Returns a single note by ID, or `()` if not found.

```rhai
let parent = get_note(note.parent_id);
if parent != () {
    field("Parent", parent.title)
}
```

### `get_notes_of_type(type_name)`

Returns all notes in the workspace that match the given schema type.

```rhai
let all_tasks = get_notes_of_type("Task");
let open = all_tasks.filter(|t| t.fields["status"] != "DONE");
```

### `get_notes_for_tag(tags)`

Returns all notes that carry **any** of the given tags (OR semantics). Duplicates removed.

```rhai
let related = get_notes_for_tag(["rust", "notes"]);
// surface related notes in a view:
let related = get_notes_for_tag(note.tags).filter(|n| n.id != note.id);
```

Available in `register_view` and `register_menu` closures. **Not** available in `on_save`
or `on_add_child`.

### `get_notes_with_link(note_id)`

Returns all notes that have any `note_link` field pointing to the given note ID. Useful for
displaying backlinks.

```rhai
let tasks = get_notes_with_link(note.id);
section("Linked Tasks", table(["Task"], tasks.map(|t| [link_to(t)])))
```

Available in `register_view` and `register_menu` closures. **Not** available in `on_save`
or `on_add_child`.

### `get_attachments(note_id)`

Returns an array of attachment metadata maps for the given note ID.

```rhai
let files = get_attachments(note.id);
```

Each entry:

| Key | Type | Description |
|---|---|---|
| `id` | String (UUID) | Attachment ID |
| `filename` | String | Original filename |
| `mime_type` | String | MIME type |
| `size_bytes` | Integer | File size in bytes |

Available in `register_view`, `register_hover`, and `register_menu` closures.

### Note map shape

Each note returned by query functions:

| Key | Type |
|---|---|
| `note.id` | String |
| `note.schema` | String |
| `note.title` | String |
| `note.fields` | Map of field values |
| `note.tags` | Array of strings |

---

## 15. Utility functions

### `today()`

Returns today's date as a `"YYYY-MM-DD"` string.

```rhai
schema("Journal", #{
    version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ],
    on_save: |note| {
        let body  = note.fields["body"] ?? "";
        let first = body.split("\n")[0];
        set_title(note.id, today() + " — " + first);
        commit();
    }
});
```

---

## 16. Introspection functions

### `schema_exists(name)`

Returns `true` if a schema with the given name is currently registered.

```rhai
if schema_exists("Project") {
    // safe to reference Project notes
}
```

### `get_schema_fields(name)`

Returns an array of field-definition maps for the named schema.

```rhai
let defs = get_schema_fields("Task");
// defs[0].name, defs[0].type, defs[0].required, defs[0].can_view, defs[0].can_edit
```

---

## 17. Tips and patterns

### Null-coalescing with `??`

Field values may be absent when a note was created before the field was added to the schema.
Use `??` to provide a fallback:

```rhai
let phone = note.fields["phone"] ?? "-";
```

### Conditional sections

```rhai
let notes_val = note.fields["notes"] ?? "";
if notes_val == "" {
    contacts_section
} else {
    stack([contacts_section, section("Notes", text(notes_val))])
}
```

### Conditional badges based on a field value

```rhai
let status = note.fields["status"] ?? "";
let color  = if status == "DONE"    { "green" }
             else if status == "WIP" { "blue" }
             else                    { "gray" };
badge(status, color)
```

### Date arithmetic

Date fields are ISO strings (`"YYYY-MM-DD"`) when set. For simple day-difference calculations:

```rhai
let s_parts = started.split("-");
let f_parts = finished.split("-");
let s_days  = parse_int(s_parts[0]) * 365 + parse_int(s_parts[1]) * 30 + parse_int(s_parts[2]);
let f_days  = parse_int(f_parts[0]) * 365 + parse_int(f_parts[1]) * 30 + parse_int(f_parts[2]);
let diff    = f_days - s_days;
if diff > 0 { diff.to_string() + " days" } else { "" }
```

This is an approximation suitable for display (not calendar-accurate).

### Checking date field presence

Date fields are `()` (unit) when not set, not an empty string. Always check the type:

```rhai
let d = note.fields["due_date"];
let label = if type_of(d) == "string" && d != "" { d } else { "Not set" };
```

### `title_can_edit: false` + `on_save` title derivation

```rhai
schema("Contact", #{
    version: 1,
    title_can_edit: false,
    fields: [ /* … */ ],
    on_save: |note| {
        let last  = note.fields["last_name"]  ?? "";
        let first = note.fields["first_name"] ?? "";
        set_title(note.id, last + ", " + first);
        commit();
    }
});
```

### Folder / item pair

```rhai
schema("ProjectFolder", #{
    version: 1,
    allowed_children_types: ["Project"],
    fields: [],
    on_save: |note| { commit(); }
});

schema("Project", #{
    version: 1,
    allowed_parent_types: ["ProjectFolder"],
    fields: [ /* … */ ],
    on_save: |note| { commit(); }
});
```

### Sharing constants between library and schema scripts

Top-level `const` declarations in a library script are **not** visible inside schema hook
closures. Wrap shared values in a function instead — function definitions are fully available
across scripts:

```rhai
// ❌ will cause "Variable not found: PREFIXES" inside hooks
const PREFIXES = #{ mr: "Mr.", mrs: "Mrs.", dr: "Dr." };

// ✓ works — call prefixes() anywhere, including field definitions and hook closures
fn prefixes() {
    #{ mr: "Mr.", mrs: "Mrs.", dr: "Dr." }
}
```

Call it in a field `semantic` map, inside a hook, or anywhere else:

```rhai
// in a field definition (evaluated at schema-load time)
#{ name: "salutation", type: "select",
   semantic: #{ property: prefixes().mr } }

// inside an on_save hook
on_save: |note| {
    let p = prefixes();
    if note.fields["salutation"] == p.mr { /* … */ }
    commit();
}
```

### Avoiding accidental schema collisions

Schema names are checked for uniqueness across scripts at load time. The safest rule: **one
schema per script that defines it.** Do not copy `schema()` blocks between scripts.

---

## 18. Built-in script examples

The following scripts ship with Krillnotes and can be studied as complete examples.

### TextNote — minimal schema, no hooks

**`00_text_note.schema.rhai`:**

```rhai
schema("TextNote", #{
    version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ],
    on_save: |note| { commit(); }
});
```

### Task — derived title, status symbol

**`02_task.schema.rhai`:**

```rhai
schema("Task", #{
    version: 1,
    title_can_edit: false,
    fields: [
        #{ name: "name",           type: "text",     required: true  },
        #{ name: "status",         type: "select",   required: true,
           options: ["TODO", "WIP", "DONE"]                           },
        #{ name: "priority",       type: "select",   required: false,
           options: ["low", "medium", "high"]                         },
        #{ name: "due_date",       type: "date",     required: false  },
        #{ name: "assignee",       type: "text",     required: false  },
        #{ name: "notes",          type: "textarea", required: false  },
        #{ name: "priority_label", type: "text",     required: false, can_edit: false },
    ],
    on_save: |note| {
        let name   = note.fields["name"] ?? "";
        let status = note.fields["status"] ?? "";
        let symbol = if status == "DONE" { "✓" }
                     else if status == "WIP" { "→" }
                     else { " " };
        set_title(note.id, "[" + symbol + "] " + name);

        let priority = note.fields["priority"] ?? "";
        set_field(note.id, "priority_label",
            if priority == "high"        { "🔴 High" }
            else if priority == "medium" { "🟡 Medium" }
            else if priority == "low"    { "🟢 Low" }
            else                         { "" });
        commit();
    }
});
```

### Contacts — folder + card with custom table view

Two files: the schema definition and a presentation script for the folder view.

**`01_contact.schema.rhai`:**

```rhai
schema("ContactsFolder", #{
    version: 1,
    children_sort: "asc",
    allowed_children_types: ["Contact"],
    fields: [
        #{ name: "notes", type: "textarea", required: false },
    ],
    on_save: |note| { commit(); }
});

schema("Contact", #{
    version: 1,
    title_can_edit: false,
    allowed_parent_types: ["ContactsFolder"],
    fields: [
        #{ name: "first_name", type: "text",    required: true  },
        #{ name: "last_name",  type: "text",    required: true  },
        #{ name: "email",      type: "email",   required: false },
        #{ name: "phone",      type: "text",    required: false },
        #{ name: "mobile",     type: "text",    required: false },
        #{ name: "birthdate",  type: "date",    required: false },
        #{ name: "is_family",  type: "boolean", required: false },
    ],
    on_save: |note| {
        let last  = note.fields["last_name"]  ?? "";
        let first = note.fields["first_name"] ?? "";
        if last != "" || first != "" {
            set_title(note.id, last + ", " + first);
        }
        commit();
    }
});
```

**`01_contact.rhai`:**

```rhai
register_view("ContactsFolder", "Contacts", #{ display_first: true }, |note| {
    let contacts = get_children(note.id);
    if contacts.len() == 0 {
        return text("No contacts yet. Add a contact using the context menu.");
    }
    let rows = contacts.map(|c| [
        link_to(c),
        c.fields["email"]  ?? "-",
        c.fields["phone"]  ?? "-",
        c.fields["mobile"] ?? "-"
    ]);
    let contacts_section = section(
        "Contacts (" + contacts.len() + ")",
        table(["Name", "Email", "Phone", "Mobile"], rows)
    );
    let notes_val = note.fields["notes"] ?? "";
    if notes_val == "" { contacts_section }
    else { stack([contacts_section, section("Notes", text(notes_val))]) }
});
```

### Zettelkasten — atomic notes with today(), tags, hover preview, and related-note discovery

A two-file template. `Zettel` notes are auto-titled with today's date and the first six words
of the body. The body field uses `show_on_hover: true` so a preview appears on hover without
a hook. The `Kasten` folder shows recent notes and a live child count in hover.

**`zettelkasten.schema.rhai`:**

```rhai
schema("Zettel", #{
    version: 1,
    title_can_edit: false,
    allowed_parent_types: ["Kasten"],
    fields: [
        #{ name: "body", type: "textarea", required: false, show_on_hover: true },
    ],
    on_save: |note| {
        let body  = note.fields["body"] ?? "";
        let words = body.split(" ").filter(|w| w != "");
        let take  = if words.len() > 6 { 6 } else { words.len() };
        let snippet = if take == 0 { "Untitled" } else {
            let s = ""; let i = 0;
            while i < take { s += words[i] + " "; i += 1; }
            s = s.trim();
            if words.len() > 6 { s + " …" } else { s }
        };
        set_title(note.id, today() + " — " + snippet);
        commit();
    }
});

schema("Kasten", #{
    version: 1,
    allowed_children_types: ["Zettel"],
    fields: [],
    on_save: |note| { commit(); }
});
```

**`zettelkasten.rhai`:**

```rhai
fn tag_list(tags) {
    if tags.len() == 0 { return ""; }
    let s = tags[0];
    let i = 1;
    while i < tags.len() { s += ", " + tags[i]; i += 1; }
    s
}

register_view("Zettel", "Content", #{ display_first: true }, |note| {
    let body_block = markdown(note.fields["body"] ?? "");
    let tags = note.tags;
    if tags.len() == 0 { return body_block; }
    let related = get_notes_for_tag(tags).filter(|n| n.id != note.id);
    if related.len() == 0 { return body_block; }
    let rows = related.map(|n| [link_to(n), tag_list(n.tags)]);
    stack([body_block, section("Related Notes", table(["Note", "Tags"], rows))])
});

register_view("Kasten", "Notes", #{ display_first: true }, |note| {
    let zettel = get_children(note.id);
    if zettel.len() == 0 { return text("No notes yet."); }
    zettel.sort_by(|a, b| a.title >= b.title);
    let recent = if zettel.len() > 10 { zettel.extract(0, 10) } else { zettel };
    let rows = recent.map(|z| [link_to(z), tag_list(z.tags)]);
    section("Recent Notes", table(["Note", "Tags"], rows))
});

register_hover("Kasten", |note| {
    let kids = get_children(note.id);
    field("Notes", kids.len().to_string())
});

register_menu("Sort by Date (Newest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title >= b.title);
    children.map(|c| c.id)
});

register_menu("Sort by Date (Oldest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
```
