# Design: Tree Action Mutations (`create_note` / `update_note`)

**Date:** 2026-02-25
**Status:** Approved

## Summary

Extend tree action closures with two new host functions — `create_note` and `update_note` — so scripts can create new notes and modify existing ones, not just reorder children. All writes from a single action execute inside one SQLite transaction; any error rolls back the entire action.

## Motivation

The current `add_tree_action` API only supports one operation: returning an array of note IDs to reorder children. Modifications to note maps inside the closure are silently discarded. This prevents scripts from doing useful imperative work such as building note subtrees, stamping fields on children, or populating a project with default tasks.

## Approved Scripting API

Two new host functions are available inside `add_tree_action` closures:

```rhai
// create_note(parent_id, node_type) → note map with schema defaults
// update_note(note)                 → persists title/fields back to DB

add_tree_action("Create Sprint Template", ["Project"], |project| {
    let sprint = create_note(project.id, "Sprint");
    sprint.title = "Sprint 1";
    sprint.fields.status = "Planning";
    update_note(sprint);

    let task = create_note(sprint.id, "Task");   // child of the new sprint
    task.title = "Define goals";
    update_note(task);

    project.fields.status = "Active";
    update_note(project);
});
```

`get_children(id)` sees notes created earlier in the same closure because all writes share the same open transaction.

The existing reorder-by-return-value behaviour is preserved unchanged:

```rhai
add_tree_action("Sort Children A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)   // bare array → reorder, no change to this path
});
```

### Scope

`create_note` and `update_note` are available **only inside tree action closures**. They are not available in `on_save`, `on_add_child`, or `on_view`. Schema hooks are deliberately kept narrow (transform values, not spawn notes) to avoid re-entrancy and infinite-loop risks.

## Architecture

### Shared write context

`ScriptRegistry` gains a new field:

```rust
action_ctx: Arc<Mutex<Option<ActionTxContext>>>
```

This mirrors the existing `query_context: Arc<Mutex<Option<QueryContext>>>` pattern used for read-only query functions.

`ActionTxContext` holds:
- A reference/handle to the open DB connection (to issue INSERT/UPDATE SQL)
- A reference to the schema registry (for schema defaults when creating notes)
- A `Vec<Operation>` accumulating operation-log entries

### Transaction lifecycle in `Workspace::run_tree_action`

```
1. conn.execute("BEGIN TRANSACTION")
2. script_registry.set_action_context(conn_ref, schema_ref)
3. ScriptRegistry::invoke_tree_action_hook(...)   ← closure runs
     ├─ create_note() → grabs action_ctx, INSERT with schema defaults, appends op-log entry
     ├─ update_note() → grabs action_ctx, UPDATE title/fields, appends op-log entry
     └─ get_children() → when action_ctx active, queries live DB (sees in-flight INSERTs)
4. script_registry.clear_action_context()
5a. Ok  → flush op-log entries, COMMIT
5b. Err → ROLLBACK, return error to caller
```

### `get_children` behaviour change

When `action_ctx` is active, `get_children` issues a live SQL query rather than reading from the pre-loaded `QueryContext` snapshot. This ensures notes created by `create_note` earlier in the closure are visible to subsequent queries. When no action is active, the existing snapshot behaviour is unchanged.

### Backwards compatibility

The "return an array of IDs → reorder" path is unchanged. Reorder calls (`move_note`) continue to run after the main transaction commits, exactly as today.

### Operation log

`create_note` appends a `CreateNote` entry; `update_note` appends `UpdateField` entries — both into the `Vec<Operation>` inside `ActionTxContext`. These are written to the operations log in one batch before the `COMMIT`, matching the ordering convention used elsewhere.

## Decisions Not Made Here

- The exact Rust type used to share connection access with `'static` host functions (e.g. `Arc<Mutex<Connection>>`, raw pointer with lifetime guarantee, or a command channel) is left to the implementation plan.
- Whether `create_note` fires the parent's `on_add_child` hook: deferred — schema hooks are already suppressed within action context, so `on_add_child` is skipped for now. Can be added later as an opt-in parameter.
