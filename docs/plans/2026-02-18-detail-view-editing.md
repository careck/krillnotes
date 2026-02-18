# Phase 4: Detail View & Editing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add editing capabilities to detail panel with view/edit mode toggle, field editing, and note deletion

**Architecture:** View/edit mode separation in InfoPanel, hybrid field rendering (schema + legacy), explicit save workflow, smart delete with child handling strategies

**Tech Stack:** Rust (Tauri v2, rusqlite), TypeScript/React, existing Tauri commands pattern

---

## Task 1: Export FieldDefinition for Frontend

**Files:**
- Modify: `krillnotes-core/src/core/scripting.rs:7-11`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (imports)

**Step 1: Add serde derives to FieldDefinition**

In `krillnotes-core/src/core/scripting.rs`, update the FieldDefinition struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
}
```

**Step 2: Verify compilation**

Run: `cd krillnotes-core && cargo build`
Expected: Compiles successfully

**Step 3: Re-export FieldDefinition from core crate**

In `krillnotes-core/src/lib.rs`, add to exports:

```rust
pub use crate::core::scripting::FieldDefinition;
```

**Step 4: Verify compilation**

Run: `cd krillnotes-core && cargo build`
Expected: Compiles successfully

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting.rs krillnotes-core/src/lib.rs
git commit -m "feat(core): export FieldDefinition with serde support

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 2: Add get_schema_fields Command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add get_schema_fields command**

Add after existing Tauri commands in `krillnotes-desktop/src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn get_schema_fields(
    window: Window,
    state: State<'_, AppState>,
    node_type: String,
) -> Result<Vec<FieldDefinition>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schema = workspace.registry.get_schema(&node_type)
        .map_err(|e| e.to_string())?;

    Ok(schema.fields)
}
```

**Step 2: Add to invoke_handler**

In the `run()` function, add to `invoke_handler`:

```rust
.invoke_handler(tauri::generate_handler![
    greet,
    create_workspace,
    open_workspace,
    get_workspace_info,
    list_notes,
    get_node_types,
    create_note_with_type,
    toggle_note_expansion,
    set_selected_note,
    get_schema_fields,  // NEW
])
```

**Step 3: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add get_schema_fields command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 3: Add Workspace::update_note Method (with tests)

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write test for update_note**

Add to test module in `krillnotes-core/src/core/workspace.rs`:

```rust
#[test]
fn test_update_note() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Get the root note
    let notes = ws.list_all_notes().unwrap();
    let note_id = notes[0].id.clone();
    let original_modified = notes[0].modified_at;

    // Wait 1 second to ensure modified_at changes
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Update the note
    let new_title = "Updated Title".to_string();
    let mut new_fields = HashMap::new();
    new_fields.insert("body".to_string(), FieldValue::Text("Updated body".to_string()));

    let updated = ws.update_note(&note_id, new_title.clone(), new_fields.clone()).unwrap();

    // Verify changes
    assert_eq!(updated.title, new_title);
    assert_eq!(updated.fields.get("body"), Some(&FieldValue::Text("Updated body".to_string())));
    assert!(updated.modified_at > original_modified);
}

#[test]
fn test_update_note_not_found() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    let result = ws.update_note("nonexistent-id", "Title".to_string(), HashMap::new());
    assert!(result.is_err());
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p krillnotes-core workspace::tests::test_update_note`
Expected: FAIL with "method not found"

**Step 3: Implement update_note method**

Add to `impl Workspace` in `krillnotes-core/src/core/workspace.rs`:

```rust
pub fn update_note(
    &mut self,
    note_id: &str,
    title: String,
    fields: HashMap<String, FieldValue>,
) -> Result<Note> {
    let now = chrono::Utc::now().timestamp();

    // Serialize fields to JSON
    let fields_json = serde_json::to_string(&fields)?;

    // Update in database
    self.storage.connection().execute(
        "UPDATE notes SET title = ?1, fields = ?2, modified_at = ?3 WHERE id = ?4",
        params![title, fields_json, now, note_id],
    )?;

    // Verify note exists by fetching it
    let note = self.storage.connection().query_row(
        "SELECT id, title, node_type, parent_id, position, created_at, modified_at,
                created_by, modified_by, fields, is_expanded
         FROM notes WHERE id = ?1",
        params![note_id],
        |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let node_type: String = row.get(2)?;
            let parent_id: Option<String> = row.get(3)?;
            let position: i32 = row.get(4)?;
            let created_at: i64 = row.get(5)?;
            let modified_at: i64 = row.get(6)?;
            let created_by: i64 = row.get(7)?;
            let modified_by: i64 = row.get(8)?;
            let fields_json: String = row.get(9)?;
            let is_expanded: bool = row.get(10)?;

            let fields: HashMap<String, FieldValue> = serde_json::from_str(&fields_json)
                .unwrap_or_default();

            Ok(Note {
                id,
                title,
                node_type,
                parent_id,
                position,
                created_at,
                modified_at,
                created_by,
                modified_by,
                fields,
                is_expanded,
            })
        },
    )?;

    Ok(note)
}
```

**Step 4: Run test to verify pass**

Run: `cargo test -p krillnotes-core workspace::tests::test_update_note`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add Workspace::update_note method

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 4: Add update_note Tauri Command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add update_note command**

Add after get_schema_fields command:

```rust
#[tauri::command]
fn update_note(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
    title: String,
    fields: HashMap<String, FieldValue>,
) -> Result<Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    workspace.update_note(&note_id, title, fields)
        .map_err(|e| e.to_string())
}
```

**Step 2: Add HashMap import**

At top of file, ensure HashMap is imported:

```rust
use std::collections::HashMap;
```

**Step 3: Add to invoke_handler**

In the `run()` function, add to `invoke_handler`:

```rust
.invoke_handler(tauri::generate_handler![
    greet,
    create_workspace,
    open_workspace,
    get_workspace_info,
    list_notes,
    get_node_types,
    create_note_with_type,
    toggle_note_expansion,
    set_selected_note,
    get_schema_fields,
    update_note,  // NEW
])
```

**Step 4: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add update_note command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 5: Add Workspace::count_children Method (with tests)

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write test for count_children**

Add to test module:

```rust
#[test]
fn test_count_children() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Get root note
    let notes = ws.list_all_notes().unwrap();
    let root_id = notes[0].id.clone();

    // Initially has 0 children
    let count = ws.count_children(&root_id).unwrap();
    assert_eq!(count, 0);

    // Create 3 child notes
    ws.create_note_with_type(Some(root_id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    ws.create_note_with_type(Some(root_id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    ws.create_note_with_type(Some(root_id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();

    // Now has 3 children
    let count = ws.count_children(&root_id).unwrap();
    assert_eq!(count, 3);
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p krillnotes-core workspace::tests::test_count_children`
Expected: FAIL with "method not found"

**Step 3: Implement count_children method**

Add to `impl Workspace`:

```rust
pub fn count_children(&self, note_id: &str) -> Result<usize> {
    let count: i64 = self.storage.connection().query_row(
        "SELECT COUNT(*) FROM notes WHERE parent_id = ?1",
        params![note_id],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}
```

**Step 4: Run test to verify pass**

Run: `cargo test -p krillnotes-core workspace::tests::test_count_children`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add Workspace::count_children method

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 6: Add count_children Tauri Command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add count_children command**

Add after update_note command:

```rust
#[tauri::command]
fn count_children(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
) -> Result<usize, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    workspace.count_children(&note_id)
        .map_err(|e| e.to_string())
}
```

**Step 2: Add to invoke_handler**

```rust
.invoke_handler(tauri::generate_handler![
    greet,
    create_workspace,
    open_workspace,
    get_workspace_info,
    list_notes,
    get_node_types,
    create_note_with_type,
    toggle_note_expansion,
    set_selected_note,
    get_schema_fields,
    update_note,
    count_children,  // NEW
])
```

**Step 3: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add count_children command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 7: Add Delete Types (Strategy, Result)

**Files:**
- Modify: `krillnotes-core/src/lib.rs`
- Create: `krillnotes-core/src/core/delete.rs`

**Step 1: Create delete types module**

Create `krillnotes-core/src/core/delete.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DeleteStrategy {
    DeleteAll,
    PromoteChildren,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub deleted_count: usize,
    pub affected_ids: Vec<String>,
}
```

**Step 2: Add module to core/mod.rs**

In `krillnotes-core/src/core/mod.rs`, add:

```rust
pub mod delete;
```

**Step 3: Re-export from lib.rs**

In `krillnotes-core/src/lib.rs`, add to exports:

```rust
pub use crate::core::delete::{DeleteStrategy, DeleteResult};
```

**Step 4: Verify compilation**

Run: `cd krillnotes-core && cargo build`
Expected: Compiles successfully

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/delete.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/lib.rs
git commit -m "feat(core): add DeleteStrategy and DeleteResult types

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 8: Add Workspace::delete_note_recursive (with tests)

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write test for delete_note_recursive**

Add to test module:

```rust
#[test]
fn test_delete_note_recursive() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Get root note
    let root = ws.list_all_notes().unwrap()[0].clone();
    let root_id = root.id.clone();

    // Create tree: root -> child1 -> grandchild1
    //                   -> child2
    let child1 = ws.create_note_with_type(Some(root_id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    let child2 = ws.create_note_with_type(Some(root_id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    let grandchild1 = ws.create_note_with_type(Some(child1.id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();

    // Count: root + child1 + child2 + grandchild1 = 4 notes
    assert_eq!(ws.list_all_notes().unwrap().len(), 4);

    // Delete child1 (should delete child1 + grandchild1)
    let result = ws.delete_note_recursive(&child1.id).unwrap();
    assert_eq!(result.deleted_count, 2);
    assert!(result.affected_ids.contains(&child1.id));
    assert!(result.affected_ids.contains(&grandchild1.id));

    // Now only root + child2 remain
    let remaining = ws.list_all_notes().unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|n| n.id == root_id));
    assert!(remaining.iter().any(|n| n.id == child2.id));
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p krillnotes-core workspace::tests::test_delete_note_recursive`
Expected: FAIL with "method not found"

**Step 3: Add helper method get_children**

Add to `impl Workspace`:

```rust
fn get_children(&self, parent_id: &str) -> Result<Vec<Note>> {
    let mut stmt = self.storage.connection().prepare(
        "SELECT id, title, node_type, parent_id, position, created_at, modified_at,
                created_by, modified_by, fields, is_expanded
         FROM notes WHERE parent_id = ?1"
    )?;

    let notes = stmt.query_map(params![parent_id], |row| {
        let id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let node_type: String = row.get(2)?;
        let parent_id: Option<String> = row.get(3)?;
        let position: i32 = row.get(4)?;
        let created_at: i64 = row.get(5)?;
        let modified_at: i64 = row.get(6)?;
        let created_by: i64 = row.get(7)?;
        let modified_by: i64 = row.get(8)?;
        let fields_json: String = row.get(9)?;
        let is_expanded: bool = row.get(10)?;

        let fields: HashMap<String, FieldValue> = serde_json::from_str(&fields_json)
            .unwrap_or_default();

        Ok(Note {
            id,
            title,
            node_type,
            parent_id,
            position,
            created_at,
            modified_at,
            created_by,
            modified_by,
            fields,
            is_expanded,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(notes)
}
```

**Step 4: Implement delete_note_recursive**

Add to `impl Workspace`:

```rust
pub fn delete_note_recursive(&mut self, note_id: &str) -> Result<DeleteResult> {
    use crate::DeleteResult;

    let mut affected_ids = vec![note_id.to_string()];
    let children = self.get_children(note_id)?;

    // Recursively delete all children
    for child in children {
        let result = self.delete_note_recursive(&child.id)?;
        affected_ids.extend(result.affected_ids);
    }

    // Delete this note
    self.storage.connection().execute(
        "DELETE FROM notes WHERE id = ?1",
        params![note_id],
    )?;

    Ok(DeleteResult {
        deleted_count: affected_ids.len(),
        affected_ids,
    })
}
```

**Step 5: Run test to verify pass**

Run: `cargo test -p krillnotes-core workspace::tests::test_delete_note_recursive`
Expected: PASS

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add Workspace::delete_note_recursive

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 9: Add Workspace::delete_note_promote (with tests)

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write test for delete_note_promote**

Add to test module:

```rust
#[test]
fn test_delete_note_promote() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Get root note
    let root = ws.list_all_notes().unwrap()[0].clone();
    let root_id = root.id.clone();

    // Create tree: root -> middle -> child1
    //                              -> child2
    let middle = ws.create_note_with_type(Some(root_id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    let child1 = ws.create_note_with_type(Some(middle.id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    let child2 = ws.create_note_with_type(Some(middle.id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();

    // Count: 4 notes total
    assert_eq!(ws.list_all_notes().unwrap().len(), 4);

    // Delete middle (promote children)
    let result = ws.delete_note_promote(&middle.id).unwrap();
    assert_eq!(result.deleted_count, 1);
    assert_eq!(result.affected_ids, vec![middle.id.clone()]);

    // Now: root, child1, child2 (3 notes)
    let remaining = ws.list_all_notes().unwrap();
    assert_eq!(remaining.len(), 3);

    // Verify child1 and child2 now have root as parent
    let child1_updated = remaining.iter().find(|n| n.id == child1.id).unwrap();
    let child2_updated = remaining.iter().find(|n| n.id == child2.id).unwrap();
    assert_eq!(child1_updated.parent_id, Some(root_id.clone()));
    assert_eq!(child2_updated.parent_id, Some(root_id.clone()));
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p krillnotes-core workspace::tests::test_delete_note_promote`
Expected: FAIL with "method not found"

**Step 3: Add helper method get_note**

Add to `impl Workspace`:

```rust
fn get_note(&self, note_id: &str) -> Result<Note> {
    self.storage.connection().query_row(
        "SELECT id, title, node_type, parent_id, position, created_at, modified_at,
                created_by, modified_by, fields, is_expanded
         FROM notes WHERE id = ?1",
        params![note_id],
        |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let node_type: String = row.get(2)?;
            let parent_id: Option<String> = row.get(3)?;
            let position: i32 = row.get(4)?;
            let created_at: i64 = row.get(5)?;
            let modified_at: i64 = row.get(6)?;
            let created_by: i64 = row.get(7)?;
            let modified_by: i64 = row.get(8)?;
            let fields_json: String = row.get(9)?;
            let is_expanded: bool = row.get(10)?;

            let fields: HashMap<String, FieldValue> = serde_json::from_str(&fields_json)
                .unwrap_or_default();

            Ok(Note {
                id,
                title,
                node_type,
                parent_id,
                position,
                created_at,
                modified_at,
                created_by,
                modified_by,
                fields,
                is_expanded,
            })
        },
    ).map_err(|e| e.into())
}
```

**Step 4: Implement delete_note_promote**

Add to `impl Workspace`:

```rust
pub fn delete_note_promote(&mut self, note_id: &str) -> Result<DeleteResult> {
    use crate::DeleteResult;

    let note = self.get_note(note_id)?;

    // Update children to point to this note's parent (grandparent)
    self.storage.connection().execute(
        "UPDATE notes SET parent_id = ?1 WHERE parent_id = ?2",
        params![note.parent_id, note_id],
    )?;

    // Delete this note
    self.storage.connection().execute(
        "DELETE FROM notes WHERE id = ?1",
        params![note_id],
    )?;

    Ok(DeleteResult {
        deleted_count: 1,
        affected_ids: vec![note_id.to_string()],
    })
}
```

**Step 5: Run test to verify pass**

Run: `cargo test -p krillnotes-core workspace::tests::test_delete_note_promote`
Expected: PASS

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add Workspace::delete_note_promote

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 10: Add Workspace::delete_note Public Method

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Write test for delete_note**

Add to test module:

```rust
#[test]
fn test_delete_note_with_strategy() {
    use crate::DeleteStrategy;

    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let child = ws.create_note_with_type(Some(root.id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();

    // Test DeleteAll strategy
    let result = ws.delete_note(&child.id, DeleteStrategy::DeleteAll).unwrap();
    assert_eq!(result.deleted_count, 1);

    // Create new child for PromoteChildren test
    let child2 = ws.create_note_with_type(Some(root.id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();
    let grandchild = ws.create_note_with_type(Some(child2.id.clone()), AddPosition::AsChild, "TextNote".to_string()).unwrap();

    let result = ws.delete_note(&child2.id, DeleteStrategy::PromoteChildren).unwrap();
    assert_eq!(result.deleted_count, 1);

    // Verify grandchild promoted
    let notes = ws.list_all_notes().unwrap();
    let gc = notes.iter().find(|n| n.id == grandchild.id).unwrap();
    assert_eq!(gc.parent_id, Some(root.id));
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p krillnotes-core workspace::tests::test_delete_note_with_strategy`
Expected: FAIL with "method not found"

**Step 3: Implement delete_note**

Add to `impl Workspace`:

```rust
pub fn delete_note(
    &mut self,
    note_id: &str,
    strategy: crate::DeleteStrategy,
) -> Result<crate::DeleteResult> {
    match strategy {
        crate::DeleteStrategy::DeleteAll => self.delete_note_recursive(note_id),
        crate::DeleteStrategy::PromoteChildren => self.delete_note_promote(note_id),
    }
}
```

**Step 4: Run test to verify pass**

Run: `cargo test -p krillnotes-core workspace::tests::test_delete_note_with_strategy`
Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat(core): add Workspace::delete_note with strategy

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 11: Add delete_note Tauri Command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add delete_note command**

Add after count_children command:

```rust
#[tauri::command]
fn delete_note(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
    strategy: DeleteStrategy,
) -> Result<DeleteResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    workspace.delete_note(&note_id, strategy)
        .map_err(|e| e.to_string())
}
```

**Step 2: Add imports**

At top of file:

```rust
use krillnotes_core::{DeleteStrategy, DeleteResult};
```

**Step 3: Add to invoke_handler**

```rust
.invoke_handler(tauri::generate_handler![
    greet,
    create_workspace,
    open_workspace,
    get_workspace_info,
    list_notes,
    get_node_types,
    create_note_with_type,
    toggle_note_expansion,
    set_selected_note,
    get_schema_fields,
    update_note,
    count_children,
    delete_note,  // NEW
])
```

**Step 4: Verify compilation**

Run: `cd krillnotes-desktop/src-tauri && cargo build`
Expected: Compiles successfully

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add delete_note command

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 12: Add Frontend Types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Add new types to types.ts**

Append to `krillnotes-desktop/src/types.ts`:

```typescript
export interface FieldDefinition {
  name: string;
  fieldType: string;  // "text" | "number" | "boolean"
  required: boolean;
}

export enum DeleteStrategy {
  DeleteAll = "DeleteAll",
  PromoteChildren = "PromoteChildren",
}

export interface DeleteResult {
  deletedCount: number;
  affectedIds: string[];
}
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(frontend): add FieldDefinition, DeleteStrategy, DeleteResult types

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 13: Create FieldDisplay Component

**Files:**
- Create: `krillnotes-desktop/src/components/FieldDisplay.tsx`

**Step 1: Create FieldDisplay component**

Create `krillnotes-desktop/src/components/FieldDisplay.tsx`:

```typescript
import type { FieldValue } from '../types';

interface FieldDisplayProps {
  fieldName: string;
  fieldType: string;
  value: FieldValue;
}

function FieldDisplay({ fieldName, fieldType, value }: FieldDisplayProps) {
  const renderValue = () => {
    if ('Text' in value) {
      return (
        <p className="whitespace-pre-wrap break-words">
          {value.Text || <span className="text-muted-foreground italic">(empty)</span>}
        </p>
      );
    } else if ('Number' in value) {
      return <p>{value.Number}</p>;
    } else if ('Boolean' in value) {
      return (
        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={value.Boolean}
            disabled
            className="rounded"
          />
          <span>{value.Boolean ? 'Yes' : 'No'}</span>
        </div>
      );
    }
    return <span className="text-muted-foreground italic">(unknown type)</span>;
  };

  return (
    <div className="mb-4">
      <label className="block text-sm font-medium text-muted-foreground mb-1">
        {fieldName}
      </label>
      <div className="text-foreground">
        {renderValue()}
      </div>
    </div>
  );
}

export default FieldDisplay;
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldDisplay.tsx
git commit -m "feat(frontend): create FieldDisplay component

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 14: Create FieldEditor Component

**Files:**
- Create: `krillnotes-desktop/src/components/FieldEditor.tsx`

**Step 1: Create FieldEditor component**

Create `krillnotes-desktop/src/components/FieldEditor.tsx`:

```typescript
import type { FieldValue } from '../types';

interface FieldEditorProps {
  fieldName: string;
  fieldType: string;
  value: FieldValue;
  required: boolean;
  onChange: (value: FieldValue) => void;
}

function FieldEditor({ fieldName, fieldType, value, required, onChange }: FieldEditorProps) {
  const renderEditor = () => {
    if ('Text' in value) {
      return (
        <textarea
          value={value.Text}
          onChange={(e) => onChange({ Text: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md min-h-[100px] resize-y"
          required={required}
        />
      );
    } else if ('Number' in value) {
      return (
        <input
          type="number"
          value={value.Number}
          onChange={(e) => onChange({ Number: parseFloat(e.target.value) || 0 })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    } else if ('Boolean' in value) {
      return (
        <input
          type="checkbox"
          checked={value.Boolean}
          onChange={(e) => onChange({ Boolean: e.target.checked })}
          className="rounded"
        />
      );
    }
    return <span className="text-red-500">Unknown field type</span>;
  };

  return (
    <div className="mb-4">
      <label className="block text-sm font-medium mb-1">
        {fieldName}
        {required && <span className="text-red-500 ml-1">*</span>}
      </label>
      {renderEditor()}
    </div>
  );
}

export default FieldEditor;
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldEditor.tsx
git commit -m "feat(frontend): create FieldEditor component

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 15: Modify InfoPanel - Add View Mode Fields

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Read current InfoPanel**

Run: `cat krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 2: Replace InfoPanel with view mode field rendering**

Replace entire contents of `krillnotes-desktop/src/components/InfoPanel.tsx`:

```typescript
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, FieldDefinition } from '../types';
import FieldDisplay from './FieldDisplay';

interface InfoPanelProps {
  selectedNote: Note | null;
}

function InfoPanel({ selectedNote }: InfoPanelProps) {
  const [schemaFields, setSchemaFields] = useState<FieldDefinition[]>([]);

  useEffect(() => {
    if (!selectedNote) {
      setSchemaFields([]);
      return;
    }

    // Fetch schema fields for this note type
    invoke<FieldDefinition[]>('get_schema_fields', { nodeType: selectedNote.nodeType })
      .then(fields => setSchemaFields(fields))
      .catch(err => {
        console.error('Failed to fetch schema fields:', err);
        setSchemaFields([]);
      });
  }, [selectedNote?.nodeType]);

  if (!selectedNote) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Select a note to view details
      </div>
    );
  }

  const formatTimestamp = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleString();
  };

  // Identify legacy fields (in note.fields but not in schema)
  const schemaFieldNames = new Set(schemaFields.map(f => f.name));
  const legacyFieldNames = Object.keys(selectedNote.fields).filter(
    name => !schemaFieldNames.has(name)
  );

  return (
    <div className="p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-4xl font-bold">{selectedNote.title}</h1>
        <div className="flex gap-2">
          <button className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90">
            Edit
          </button>
          <button className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600">
            Delete
          </button>
        </div>
      </div>

      {/* Fields Section */}
      <div className="mb-6">
        <h2 className="text-xl font-semibold mb-4">Fields</h2>

        {/* Schema fields */}
        {schemaFields.map(field => (
          <FieldDisplay
            key={field.name}
            fieldName={field.name}
            fieldType={field.fieldType}
            value={selectedNote.fields[field.name] || { Text: '' }}
          />
        ))}

        {/* Legacy fields */}
        {legacyFieldNames.length > 0 && (
          <>
            <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
              Legacy Fields
            </h3>
            {legacyFieldNames.map(name => (
              <FieldDisplay
                key={name}
                fieldName={`${name} (legacy)`}
                fieldType="text"
                value={selectedNote.fields[name]}
              />
            ))}
          </>
        )}

        {schemaFields.length === 0 && legacyFieldNames.length === 0 && (
          <p className="text-muted-foreground italic">No fields</p>
        )}
      </div>

      {/* Metadata Section */}
      <div className="bg-secondary p-6 rounded-lg space-y-4">
        <div>
          <p className="text-sm text-muted-foreground">Type</p>
          <p className="text-lg">{selectedNote.nodeType}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">Created</p>
          <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">Modified</p>
          <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">ID</p>
          <p className="text-xs font-mono">{selectedNote.id}</p>
        </div>
      </div>
    </div>
  );
}

export default InfoPanel;
```

**Step 3: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat(frontend): add field display to InfoPanel view mode

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 16: Add Edit Mode to InfoPanel

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Add edit mode state and logic**

Replace the InfoPanel component with full edit mode support:

```typescript
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, FieldDefinition, FieldValue } from '../types';
import FieldDisplay from './FieldDisplay';
import FieldEditor from './FieldEditor';

interface InfoPanelProps {
  selectedNote: Note | null;
  onNoteUpdated: () => void;
}

function InfoPanel({ selectedNote, onNoteUpdated }: InfoPanelProps) {
  const [schemaFields, setSchemaFields] = useState<FieldDefinition[]>([]);
  const [isEditing, setIsEditing] = useState(false);
  const [editedTitle, setEditedTitle] = useState('');
  const [editedFields, setEditedFields] = useState<Record<string, FieldValue>>({});
  const [isDirty, setIsDirty] = useState(false);

  useEffect(() => {
    if (!selectedNote) {
      setSchemaFields([]);
      setIsEditing(false);
      return;
    }

    // Fetch schema fields
    invoke<FieldDefinition[]>('get_schema_fields', { nodeType: selectedNote.nodeType })
      .then(fields => setSchemaFields(fields))
      .catch(err => {
        console.error('Failed to fetch schema fields:', err);
        setSchemaFields([]);
      });
  }, [selectedNote?.id]);

  useEffect(() => {
    // Reset edit state when selected note changes
    if (selectedNote) {
      setEditedTitle(selectedNote.title);
      setEditedFields({ ...selectedNote.fields });
      setIsDirty(false);
    }
  }, [selectedNote?.id]);

  const handleEdit = () => {
    setIsEditing(true);
  };

  const handleCancel = () => {
    if (isDirty) {
      if (!confirm('Discard changes?')) {
        return;
      }
    }
    setIsEditing(false);
    setEditedTitle(selectedNote!.title);
    setEditedFields({ ...selectedNote!.fields });
    setIsDirty(false);
  };

  const handleSave = async () => {
    if (!selectedNote) return;

    try {
      await invoke('update_note', {
        noteId: selectedNote.id,
        title: editedTitle,
        fields: editedFields,
      });
      setIsEditing(false);
      setIsDirty(false);
      onNoteUpdated();
    } catch (err) {
      alert(`Failed to save: ${err}`);
    }
  };

  const handleFieldChange = (fieldName: string, value: FieldValue) => {
    setEditedFields(prev => ({ ...prev, [fieldName]: value }));
    setIsDirty(true);
  };

  if (!selectedNote) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Select a note to view details
      </div>
    );
  }

  const formatTimestamp = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleString();
  };

  // Identify schema and legacy fields
  const schemaFieldNames = new Set(schemaFields.map(f => f.name));
  const allFieldNames = Object.keys(selectedNote.fields);
  const legacyFieldNames = allFieldNames.filter(name => !schemaFieldNames.has(name));

  return (
    <div className={`p-6 ${isEditing ? 'border-2 border-primary rounded-lg' : ''}`}>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        {isEditing ? (
          <input
            type="text"
            value={editedTitle}
            onChange={(e) => {
              setEditedTitle(e.target.value);
              setIsDirty(true);
            }}
            className="text-4xl font-bold bg-background border border-border rounded-md px-2 py-1 flex-1"
          />
        ) : (
          <h1 className="text-4xl font-bold">{selectedNote.title}</h1>
        )}
        <div className="flex gap-2 ml-4">
          {isEditing ? (
            <>
              <button
                onClick={handleSave}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                Save
              </button>
              <button
                onClick={handleCancel}
                className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              <button
                onClick={handleEdit}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                Edit
              </button>
              <button className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600">
                Delete
              </button>
            </>
          )}
        </div>
      </div>

      {/* Fields Section */}
      <div className="mb-6">
        <h2 className="text-xl font-semibold mb-4">Fields</h2>

        {/* Schema fields */}
        {schemaFields.map(field => (
          isEditing ? (
            <FieldEditor
              key={field.name}
              fieldName={field.name}
              fieldType={field.fieldType}
              value={editedFields[field.name] || { Text: '' }}
              required={field.required}
              onChange={(value) => handleFieldChange(field.name, value)}
            />
          ) : (
            <FieldDisplay
              key={field.name}
              fieldName={field.name}
              fieldType={field.fieldType}
              value={selectedNote.fields[field.name] || { Text: '' }}
            />
          )
        ))}

        {/* Legacy fields */}
        {legacyFieldNames.length > 0 && (
          <>
            <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
              Legacy Fields
            </h3>
            {legacyFieldNames.map(name => (
              isEditing ? (
                <FieldEditor
                  key={name}
                  fieldName={`${name} (legacy)`}
                  fieldType="text"
                  value={editedFields[name]}
                  required={false}
                  onChange={(value) => handleFieldChange(name, value)}
                />
              ) : (
                <FieldDisplay
                  key={name}
                  fieldName={`${name} (legacy)`}
                  fieldType="text"
                  value={selectedNote.fields[name]}
                />
              )
            ))}
          </>
        )}

        {schemaFields.length === 0 && legacyFieldNames.length === 0 && (
          <p className="text-muted-foreground italic">No fields</p>
        )}
      </div>

      {/* Metadata Section */}
      <div className="bg-secondary p-6 rounded-lg space-y-4">
        <div>
          <p className="text-sm text-muted-foreground">Type</p>
          <p className="text-lg">{selectedNote.nodeType}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">Created</p>
          <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">Modified</p>
          <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">ID</p>
          <p className="text-xs font-mono">{selectedNote.id}</p>
        </div>
      </div>
    </div>
  );
}

export default InfoPanel;
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat(frontend): add edit mode to InfoPanel

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 17: Create DeleteConfirmDialog Component

**Files:**
- Create: `krillnotes-desktop/src/components/DeleteConfirmDialog.tsx`

**Step 1: Create DeleteConfirmDialog component**

Create `krillnotes-desktop/src/components/DeleteConfirmDialog.tsx`:

```typescript
import { useState } from 'react';
import { DeleteStrategy } from '../types';

interface DeleteConfirmDialogProps {
  noteTitle: string;
  childCount: number;
  onConfirm: (strategy: DeleteStrategy) => void;
  onCancel: () => void;
}

function DeleteConfirmDialog({ noteTitle, childCount, onConfirm, onCancel }: DeleteConfirmDialogProps) {
  const [strategy, setStrategy] = useState<DeleteStrategy>(DeleteStrategy.DeleteAll);

  const handleConfirm = () => {
    onConfirm(strategy);
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg p-6 max-w-md w-full">
        <h2 className="text-xl font-bold mb-4">Delete Note</h2>

        {childCount === 0 ? (
          <p className="mb-6">
            Are you sure you want to delete <strong>{noteTitle}</strong>?
          </p>
        ) : (
          <>
            <p className="mb-4">
              Delete <strong>{noteTitle}</strong>? This note has <strong>{childCount}</strong> {childCount === 1 ? 'child' : 'children'}.
            </p>
            <div className="space-y-3 mb-6">
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="radio"
                  name="deleteStrategy"
                  checked={strategy === DeleteStrategy.DeleteAll}
                  onChange={() => setStrategy(DeleteStrategy.DeleteAll)}
                  className="mt-1"
                />
                <div>
                  <div className="font-medium">Delete this note and all descendants</div>
                  <div className="text-sm text-muted-foreground">
                    ({childCount + 1} notes total)
                  </div>
                </div>
              </label>
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="radio"
                  name="deleteStrategy"
                  checked={strategy === DeleteStrategy.PromoteChildren}
                  onChange={() => setStrategy(DeleteStrategy.PromoteChildren)}
                  className="mt-1"
                />
                <div>
                  <div className="font-medium">Delete this note and promote children</div>
                  <div className="text-sm text-muted-foreground">
                    Children will be moved to parent level
                  </div>
                </div>
              </label>
            </div>
          </>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
          >
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  );
}

export default DeleteConfirmDialog;
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/DeleteConfirmDialog.tsx
git commit -m "feat(frontend): create DeleteConfirmDialog component

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 18: Wire Up Delete in InfoPanel

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Add delete dialog state and handler**

Update InfoPanel to include delete functionality. Add these changes:

Add imports at top:
```typescript
import DeleteConfirmDialog from './DeleteConfirmDialog';
import type { DeleteStrategy, DeleteResult } from '../types';
```

Add state after other useState declarations:
```typescript
const [showDeleteDialog, setShowDeleteDialog] = useState(false);
const [childCount, setChildCount] = useState(0);
```

Add delete handler functions before the return statement:
```typescript
const handleDeleteClick = async () => {
  if (!selectedNote) return;

  try {
    const count = await invoke<number>('count_children', { noteId: selectedNote.id });
    setChildCount(count);
    setShowDeleteDialog(true);
  } catch (err) {
    alert(`Failed to check children: ${err}`);
  }
};

const handleDeleteConfirm = async (strategy: DeleteStrategy) => {
  if (!selectedNote) return;

  try {
    await invoke<DeleteResult>('delete_note', {
      noteId: selectedNote.id,
      strategy,
    });
    setShowDeleteDialog(false);
    onNoteUpdated();
  } catch (err) {
    alert(`Failed to delete: ${err}`);
    setShowDeleteDialog(false);
  }
};

const handleDeleteCancel = () => {
  setShowDeleteDialog(false);
};
```

Update Delete button onClick:
```typescript
<button
  onClick={handleDeleteClick}
  className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
>
  Delete
</button>
```

Add dialog at end of return statement (before final closing `</div>`):
```typescript
{showDeleteDialog && (
  <DeleteConfirmDialog
    noteTitle={selectedNote.title}
    childCount={childCount}
    onConfirm={handleDeleteConfirm}
    onCancel={handleDeleteCancel}
  />
)}
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat(frontend): wire up delete functionality in InfoPanel

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 19: Update WorkspaceView for Note Updates

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Read current WorkspaceView**

Run: `cat krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 2: Add refresh handler**

In WorkspaceView, update the InfoPanel usage:

Add refreshNotes function:
```typescript
const refreshNotes = () => {
  loadNotes();
};
```

Update InfoPanel component call to pass callback:
```typescript
<InfoPanel
  selectedNote={selectedNote}
  onNoteUpdated={refreshNotes}
/>
```

Also update InfoPanel import to match new signature if needed.

**Step 3: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat(frontend): wire WorkspaceView to refresh on updates

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Task 20: Add Auto-Select After Delete

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Update refreshNotes to handle deleted note**

Modify the refreshNotes function to auto-select next note if current is deleted:

```typescript
const refreshNotes = () => {
  const currentId = selectedNote?.id;
  loadNotes().then(() => {
    // If current note no longer exists, auto-select next available
    if (currentId && !notes.some(n => n.id === currentId)) {
      // Try to select next sibling, prev sibling, parent, or first note
      if (notes.length > 0) {
        setSelectedNote(notes[0]);
        invoke('set_selected_note', { noteId: notes[0].id });
      } else {
        setSelectedNote(null);
      }
    }
  });
};
```

**Step 2: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat(frontend): auto-select next note after delete

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

---

## Summary

**Phase 4 Implementation Complete!** 🎉

**Backend Delivered:**
- ✅ FieldDefinition exported with serde support
- ✅ get_schema_fields command
- ✅ update_note method and command
- ✅ count_children method and command
- ✅ delete_note with strategies (DeleteAll, PromoteChildren)
- ✅ All backend unit tests passing

**Frontend Delivered:**
- ✅ TypeScript types (FieldDefinition, DeleteStrategy, DeleteResult)
- ✅ FieldDisplay component (read-only field rendering)
- ✅ FieldEditor component (editable field inputs)
- ✅ InfoPanel view/edit mode toggle
- ✅ Hybrid field rendering (schema + legacy)
- ✅ DeleteConfirmDialog with child strategy selection
- ✅ WorkspaceView refresh and auto-select logic

**Lines of Code:** ~1200 (estimated)

**Tasks Completed:** 20

**Next Phase:** TBD (possibly markdown support, batch operations, or keyboard shortcuts)
