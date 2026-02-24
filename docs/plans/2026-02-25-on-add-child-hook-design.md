# on_add_child Hook — Design

## Summary

Add a new schema hook `on_add_child` that fires when a note is added as a child to a note whose schema defines the hook. The hook receives both the parent note and the new child note, can modify either or both, and returns the modifications.

GitHub issue: [#4](https://github.com/careck/Krillnotes/issues/4)

---

## Rhai API

The hook is defined inside the parent's `schema()` call, alongside `on_save` and `on_view`:

```rhai
schema("ContactsFolder", #{
    fields: [
        #{ name: "child_count", type: "integer", can_view: true, can_edit: false },
    ],
    on_add_child: |parent_note, child_note| {
        parent_note.fields["child_count"] = parent_note.fields["child_count"] + 1;
        parent_note.title = "Contacts (" + parent_note.fields["child_count"].to_string() + ")";
        #{ parent: parent_note, child: child_note }
    }
});
```

**Signature:** `|parent_note, child_note| -> Map`

- Both arguments have the same shape as the `on_save` note map: `{ id, node_type, title, fields }`
- Returns a Rhai map with optional keys `parent` and/or `child`
- Missing keys mean "no change to that note"
- `parent_note.id` and `child_note.id` are read-only (not persisted if changed)

---

## Trigger conditions

The hook fires in two situations:

1. **Note creation** — a new note is created as a child of a parent whose schema has `on_add_child`
2. **Note move** — an existing note is moved (drag-and-drop) under a new parent whose schema has `on_add_child`

Root-level creation (no parent) never triggers the hook.

---

## Execution order

Both creation and move follow the same sequence:

1. Validate `allowed_parent_types` on the child schema → abort if not satisfied
2. Validate `allowed_children_types` on the parent schema → abort if not satisfied
3. *(Creation only)* Insert the child into DB with schema defaults
4. *(Move only)* Update the child's `parent_id` in DB
5. **Run `on_add_child` hook** (if registered for the parent's schema type)
6. If hook returns a modified `child`: update child fields/title in DB (direct update, no `on_save` triggered)
7. If hook returns a modified `parent`: update parent fields/title in DB (direct update, no `on_save` triggered)

The absence of a nested `on_save` call is intentional. Hook chaining is unpredictable and hard to debug; the `on_add_child` author is responsible for writing what they want stored.

---

## Error handling

Any Rhai runtime error in `on_add_child`:
- Aborts the entire operation (transaction rollback — the note is not created/moved)
- Surfaces to the user as an error dialog with script name and line number
- Consistent with how `on_save` errors are handled

---

## Implementation scope

All changes are in the core layer. No new Tauri commands or frontend changes required.

### Files to change

| File | Change |
|---|---|
| `krillnotes-core/src/core/scripting/schema.rs` | Add `on_add_child_hooks: HashMap<String, HookEntry>` to `SchemaRegistry`; add `run_on_add_child_hook()` method |
| `krillnotes-core/src/core/scripting/mod.rs` | Extract and register `on_add_child` FnPtr during `schema()` call registration |
| `krillnotes-core/src/core/workspace.rs` | Call hook in `create_note()` and in `move_note()` after validation, inside the DB transaction |
| `krillnotes-website/content/docs/scripting.md` | Document `on_add_child` hook: new section, update schema template, update built-in example |

---

## Scripting guide updates

- Section 1 (script structure): add `on_add_child` to the hooks list
- Section 2 (defining schemas): add `on_add_child` line to the schema template
- New section after `on_view`: `on_add_child hook` with signature, note map shape, return value, and a child-count example
- Section 4 (schema options): note that `allowed_parent_types`/`allowed_children_types` checks always run before the hook
- Section 10 (tips): add a "child count in parent title" pattern
- Built-in examples: extend `ContactsFolder` with an `on_add_child` example
