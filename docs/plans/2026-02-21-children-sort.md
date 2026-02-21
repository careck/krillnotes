# Children Sort Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `children_sort` schema property so parent note types can declare how their children are sorted in the tree (alphabetically ascending, descending, or by manual position).

**Architecture:** The property follows the same pattern as `title_can_edit` — parsed from Rhai schema maps, stored in the `Schema` struct, exposed to the frontend via `SchemaInfo`. The frontend's `buildTree()` uses a schema lookup map to sort each parent's children by the correct mode. A new `get_all_schemas` Tauri command provides the lookup map.

**Tech Stack:** Rust (krillnotes-core), Tauri IPC, TypeScript/React (frontend)

---

### Task 1: Add `children_sort` to Rust Schema struct and parser

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:28-33` (Schema struct)
- Modify: `krillnotes-core/src/core/scripting/schema.rs:160-170` (parse_from_rhai)

**Step 1: Write the failing test**

Add to `krillnotes-core/src/core/scripting/mod.rs` (in the `tests` module, after the last test):

```rust
#[test]
fn test_children_sort_defaults_to_none() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("SortTest", #{
            fields: [#{ name: "x", type: "text" }]
        });
    "#).unwrap();
    let schema = registry.get_schema("SortTest").unwrap();
    assert_eq!(schema.children_sort, "none", "children_sort should default to 'none'");
}

#[test]
fn test_children_sort_explicit_asc() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("SortAsc", #{
            children_sort: "asc",
            fields: [#{ name: "x", type: "text" }]
        });
    "#).unwrap();
    let schema = registry.get_schema("SortAsc").unwrap();
    assert_eq!(schema.children_sort, "asc");
}

#[test]
fn test_children_sort_explicit_desc() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("SortDesc", #{
            children_sort: "desc",
            fields: [#{ name: "x", type: "text" }]
        });
    "#).unwrap();
    let schema = registry.get_schema("SortDesc").unwrap();
    assert_eq!(schema.children_sort, "desc");
}
```

**Step 2: Run tests to verify they fail**

Run: `cd /Users/careck/Source/Krillnotes && cargo test -p krillnotes-core test_children_sort`
Expected: FAIL — `Schema` has no field `children_sort`

**Step 3: Add `children_sort` field to Schema struct**

In `krillnotes-core/src/core/scripting/schema.rs`, add to the `Schema` struct (line 32, after `title_can_edit`):

```rust
pub children_sort: String,
```

**Step 4: Parse `children_sort` from Rhai map**

In `krillnotes-core/src/core/scripting/schema.rs`, after the `title_can_edit` parsing block (after line 168), add:

```rust
let children_sort = def
    .get("children_sort")
    .and_then(|v| v.clone().try_cast::<String>())
    .unwrap_or_else(|| "none".to_string());
```

And update the `Ok(Schema { ... })` return (line 170) to include `children_sort`:

```rust
Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort })
```

**Step 5: Fix existing test code that constructs Schema literals**

Several tests manually construct `Schema` structs. Add `children_sort: "none".to_string()` to each. These are at:
- `test_default_fields` (around line 263)
- `test_date_field_default` (around line 306)
- `test_email_field_default` (around line 326)

**Step 6: Run tests to verify they pass**

Run: `cd /Users/careck/Source/Krillnotes && cargo test -p krillnotes-core`
Expected: ALL PASS

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add children_sort to Schema struct with Rhai parsing"
```

---

### Task 2: Add `get_all_schemas` Tauri command

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:230-232` (SchemaRegistry::list)
- Modify: `krillnotes-core/src/core/scripting/mod.rs:160-162` (ScriptRegistry::list_types)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:370-408` (SchemaInfo + new command)

**Step 1: Add `all_schemas()` method to SchemaRegistry**

In `krillnotes-core/src/core/scripting/schema.rs`, add after the `list` method (after line 232):

```rust
pub(super) fn all(&self) -> HashMap<String, Schema> {
    self.schemas.lock().unwrap().clone()
}
```

**Step 2: Add `all_schemas()` method to ScriptRegistry**

In `krillnotes-core/src/core/scripting/mod.rs`, add after `list_types` (after line 162):

```rust
/// Returns all registered schemas keyed by name.
pub fn all_schemas(&self) -> HashMap<String, Schema> {
    self.schema_registry.all()
}
```

**Step 3: Update `SchemaInfo` in lib.rs to include `children_sort`**

In `krillnotes-desktop/src-tauri/src/lib.rs`, update the `SchemaInfo` struct (line 374-378):

```rust
struct SchemaInfo {
    fields: Vec<FieldDefinition>,
    title_can_view: bool,
    title_can_edit: bool,
    children_sort: String,
}
```

**Step 4: Update `get_schema_fields` to include `children_sort`**

In the existing `get_schema_fields` command (line 403-407), update the return:

```rust
Ok(SchemaInfo {
    fields: schema.fields,
    title_can_view: schema.title_can_view,
    title_can_edit: schema.title_can_edit,
    children_sort: schema.children_sort,
})
```

**Step 5: Add `get_all_schemas` Tauri command**

In `krillnotes-desktop/src-tauri/src/lib.rs`, add after the `get_schema_fields` command (after line 408):

```rust
/// Returns all schema infos keyed by node type name.
#[tauri::command]
fn get_all_schemas(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, SchemaInfo>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schemas = workspace.script_registry().all_schemas();
    let mut result = HashMap::new();
    for (name, schema) in schemas {
        result.insert(name, SchemaInfo {
            fields: schema.fields,
            title_can_view: schema.title_can_view,
            title_can_edit: schema.title_can_edit,
            children_sort: schema.children_sort,
        });
    }
    Ok(result)
}
```

**Step 6: Register the new command in the invoke handler**

In `krillnotes-desktop/src-tauri/src/lib.rs`, add `get_all_schemas` to the `tauri::generate_handler!` list (after `get_schema_fields` on line 680):

```rust
get_all_schemas,
```

**Step 7: Verify backend compiles**

Run: `cd /Users/careck/Source/Krillnotes && cargo build -p krillnotes-desktop`
Expected: Compiles successfully

**Step 8: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add get_all_schemas Tauri command with children_sort"
```

---

### Task 3: Update frontend types and tree building

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:46-50` (SchemaInfo interface)
- Modify: `krillnotes-desktop/src/utils/tree.ts:1-36` (buildTree function)

**Step 1: Add `childrenSort` to SchemaInfo TypeScript interface**

In `krillnotes-desktop/src/types.ts`, update the `SchemaInfo` interface (line 46-50):

```typescript
export interface SchemaInfo {
  fields: FieldDefinition[];
  titleCanView: boolean;
  titleCanEdit: boolean;
  childrenSort: 'asc' | 'desc' | 'none';
}
```

**Step 2: Update `buildTree` to accept schema sort config**

In `krillnotes-desktop/src/utils/tree.ts`, update the function signature and sorting logic. Replace the entire `buildTree` function:

```typescript
/**
 * Builds a tree structure from a flat array of notes.
 * Notes are expected to have parentId and position fields for hierarchy and ordering.
 * When sortConfig is provided, children are sorted according to their parent's schema:
 * - "asc": alphabetical by title (A→Z)
 * - "desc": reverse alphabetical by title (Z→A)
 * - "none" (default): by position (manual order)
 */
export function buildTree(
  notes: Note[],
  sortConfig?: Record<string, 'asc' | 'desc' | 'none'>
): TreeNode[] {
  // 1. Group children by parent_id
  const childrenMap = new Map<string | null, Note[]>();

  notes.forEach(note => {
    const parentId = note.parentId;
    if (!childrenMap.has(parentId)) {
      childrenMap.set(parentId, []);
    }
    childrenMap.get(parentId)!.push(note);
  });

  // 2. Recursive builder — sorts children based on parent's schema
  function buildNode(note: Note): TreeNode {
    const children = childrenMap.get(note.id) || [];
    const mode = sortConfig?.[note.nodeType] ?? 'none';
    if (mode === 'asc') {
      children.sort((a, b) => a.title.localeCompare(b.title));
    } else if (mode === 'desc') {
      children.sort((a, b) => b.title.localeCompare(a.title));
    } else {
      children.sort((a, b) => a.position - b.position);
    }
    return {
      note,
      children: children.map(buildNode)
    };
  }

  // 3. Sort root-level notes by position (roots have no parent schema)
  const roots = childrenMap.get(null) || [];
  roots.sort((a, b) => a.position - b.position);
  return roots.map(buildNode);
}
```

**Step 3: Verify TypeScript compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No type errors (buildTree callers pass zero or one arg, both valid)

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts krillnotes-desktop/src/utils/tree.ts
git commit -m "feat: update buildTree to sort children by schema's childrenSort"
```

---

### Task 4: Wire up schema fetching in WorkspaceView

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx:99-123` (loadNotes)

**Step 1: Fetch all schemas and pass sort config to buildTree**

In `krillnotes-desktop/src/components/WorkspaceView.tsx`, update the `loadNotes` function (around line 99-123):

Add import at the top of the file (with the existing type imports on line 11):

```typescript
import type { Note, TreeNode, WorkspaceInfo, DeleteResult, SchemaInfo } from '../types';
```

Update `loadNotes` to fetch schemas and pass sort config:

```typescript
const loadNotes = async (): Promise<Note[]> => {
  try {
    const [fetchedNotes, allSchemas] = await Promise.all([
      invoke<Note[]>('list_notes'),
      invoke<Record<string, SchemaInfo>>('get_all_schemas'),
    ]);
    setNotes(fetchedNotes);

    // Build sort config from schemas
    const sortConfig: Record<string, 'asc' | 'desc' | 'none'> = {};
    for (const [nodeType, schema] of Object.entries(allSchemas)) {
      sortConfig[nodeType] = schema.childrenSort;
    }

    const builtTree = buildTree(fetchedNotes, sortConfig);
    setTree(builtTree);

    if (!selectionInitialized.current) {
      selectionInitialized.current = true;
      if (workspaceInfo.selectedNoteId) {
        setSelectedNoteId(workspaceInfo.selectedNoteId);
      } else if (builtTree.length > 0) {
        const firstRootId = builtTree[0].note.id;
        setSelectedNoteId(firstRootId);
        await invoke('set_selected_note', { noteId: firstRootId });
      }
    }

    return fetchedNotes;
  } catch (err) {
    setError(`Failed to load notes: ${err}`);
    return [];
  }
};
```

**Step 2: Verify the full app compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No type errors

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: fetch all schemas and pass children_sort config to buildTree"
```

---

### Task 5: Full build verification

**Step 1: Run Rust tests**

Run: `cd /Users/careck/Source/Krillnotes && cargo test -p krillnotes-core`
Expected: ALL PASS

**Step 2: Build the full Tauri app**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Compiles successfully

**Step 3: Run TypeScript type check**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No errors
