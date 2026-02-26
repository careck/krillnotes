# Note Linking — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `note_link` field type that stores a reference to another note by UUID, with a fast junction-table index, inline search picker, lazy frontend display, and safe deletion cleanup.

**Architecture:** `FieldValue::NoteLink(Option<String>)` stores the UUID in `fields_json` (source of truth). A `note_links(source_id, field_name, target_id)` junction table acts as a derived index for O(1) reverse lookups and deletion cleanup. The index is synced on every create/update and rebuilt from scratch on import.

**Tech Stack:** Rust (rusqlite, serde_json), Rhai scripting engine, Tauri v2, React/TypeScript

**Design doc:** `docs/plans/2026-02-26-note-linking-design.md`

---

### Task 1: DB Migration — `note_links` table

**Files:**
- Modify: `krillnotes-core/src/core/schema.sql`
- Modify: `krillnotes-core/src/core/storage.rs:95-143` (run_migrations)

**Step 1: Add to schema.sql** (for fresh databases)

Add after the `note_tags` block (around line 58):

```sql
CREATE TABLE IF NOT EXISTS note_links (
    source_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    field_name TEXT NOT NULL,
    target_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE RESTRICT,
    PRIMARY KEY (source_id, field_name)
);
CREATE INDEX IF NOT EXISTS idx_note_links_target ON note_links(target_id);
```

**Step 2: Add migration in storage.rs**

Follow the existing pattern in `run_migrations()` after the last migration block. Add:

```rust
// Migration: add note_links table
let note_links_exists: bool = conn
    .query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_links'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
    > 0;
if !note_links_exists {
    conn.execute_batch(
        "CREATE TABLE note_links (
            source_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
            field_name TEXT NOT NULL,
            target_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE RESTRICT,
            PRIMARY KEY (source_id, field_name)
        );
        CREATE INDEX idx_note_links_target ON note_links(target_id);",
    )?;
}
```

**Step 3: Write failing test**

In `krillnotes-core/src/core/storage.rs` tests (or nearby test module):

```rust
#[test]
fn test_note_links_table_exists_after_migration() {
    let ws = helpers::create_test_workspace();
    let conn = ws.connection();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_links'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
```

Run: `cargo test -p krillnotes-core test_note_links_table_exists -- --nocapture`
Expected: FAIL (table doesn't exist yet)

**Step 4: Implement** — apply the schema.sql and storage.rs changes above.

**Step 5: Run test** — Expected: PASS

**Step 6: Commit**
```bash
git add krillnotes-core/src/core/schema.sql krillnotes-core/src/core/storage.rs
git commit -m "feat: add note_links junction table migration"
```

---

### Task 2: `FieldValue::NoteLink` variant + `FieldDefinition::target_type`

**Files:**
- Modify: `krillnotes-core/src/core/note.rs:9-21` (FieldValue enum)
- Modify: `krillnotes-core/src/core/scripting/schema.rs:31-43` (FieldDefinition struct)

**Step 1: Write failing test**

```rust
#[test]
fn test_note_link_field_value_serializes_to_null() {
    let v = FieldValue::NoteLink(None);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "null");
}

#[test]
fn test_note_link_field_value_serializes_to_string() {
    let v = FieldValue::NoteLink(Some("abc-123".into()));
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""abc-123""#);
}

#[test]
fn test_note_link_field_value_deserializes_from_null() {
    let v: FieldValue = serde_json::from_str("null").unwrap();
    // Can't directly assert variant without PartialEq on Option<String> path,
    // but we can check via serialization round-trip:
    assert_eq!(serde_json::to_string(&v).unwrap(), "null");
}
```

Run: `cargo test -p krillnotes-core test_note_link_field_value -- --nocapture`
Expected: FAIL (variant doesn't exist)

**Step 2: Add variant to FieldValue enum** in `note.rs` after the `Email` variant:

```rust
/// A reference to another note by UUID. `None` = not set, `Some(uuid)` = linked note ID.
/// Serializes as JSON `null` or `"uuid-string"` — same pattern as `Date`.
#[serde(with = "note_link_serde")]
NoteLink(Option<String>),
```

Add the serde module at the bottom of `note.rs`:

```rust
mod note_link_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &Option<String>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.serialize(s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(d)
    }
}
```

**Note:** The `Date` variant already uses a custom serde helper — follow the same pattern. The `NoteLink` variant serializes `None` as `null` and `Some(s)` as the string. Check how `Date` is handled and mirror it if a simpler approach works (e.g. `#[serde(skip_serializing_if)]`).

**Step 3: Add `target_type` to `FieldDefinition`** in `schema.rs`:

```rust
/// Optional schema type filter for `note_link` fields.
/// If set, the link picker only shows notes of this type.
#[serde(default)]
pub target_type: Option<String>,
```

**Step 4: Run tests** — Expected: PASS

**Step 5: Commit**
```bash
git add krillnotes-core/src/core/note.rs krillnotes-core/src/core/scripting/schema.rs
git commit -m "feat: add FieldValue::NoteLink variant and FieldDefinition::target_type"
```

---

### Task 3: Schema parsing — handle `note_link` field type

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:101-110` (`default_fields`)
- Modify: `krillnotes-core/src/core/scripting/schema.rs:556-627` (`dynamic_to_field_value`)
- Modify: `krillnotes-core/src/core/scripting/schema.rs` — `parse_from_rhai` function

**Step 1: Write failing test**

```rust
#[test]
fn test_default_field_for_note_link_is_none() {
    let field = FieldDefinition {
        name: "linked".into(),
        field_type: "note_link".into(),
        required: false,
        can_view: true,
        can_edit: true,
        options: vec![],
        max: 0,
        target_type: None,
    };
    let schema_def = SchemaDefinition {
        name: "Test".into(),
        fields: vec![field],
        ..Default::default()
    };
    let defaults = schema_def.default_fields();
    assert!(matches!(defaults["linked"], FieldValue::NoteLink(None)));
}
```

Run: `cargo test -p krillnotes-core test_default_field_for_note_link -- --nocapture`
Expected: FAIL

**Step 2: Update `default_fields()`**

In the `match field_def.field_type.as_str()` block, add:

```rust
"note_link" => FieldValue::NoteLink(None),
```

**Step 3: Update `dynamic_to_field_value()`**

Find the match block that converts Rhai Dynamic values. Add a branch for `"note_link"`:

```rust
"note_link" => {
    // In Rhai, a note_link field is a string UUID or null/()
    if dynamic.is_unit() {
        FieldValue::NoteLink(None)
    } else {
        let s = dynamic.into_string()
            .map_err(|_| SchemaError::FieldTypeMismatch("note_link".into()))?;
        if s.is_empty() {
            FieldValue::NoteLink(None)
        } else {
            FieldValue::NoteLink(Some(s))
        }
    }
}
```

**Step 4: Parse `target_type` in `parse_from_rhai`**

In the function that builds `FieldDefinition` from a Rhai map, extract `target_type`:

```rust
let target_type = field_map
    .get("target_type")
    .and_then(|v| v.clone().into_string().ok())
    .filter(|s| !s.is_empty());
```

Then include it in the `FieldDefinition { ... }` struct literal.

**Step 5: Run tests** — Expected: PASS

**Step 6: Verify schema parsing with an integration test**

```rust
#[test]
fn test_parse_note_link_field_from_rhai() {
    let script = r#"
        schema("Task", #{
            fields: [
                #{ name: "linked_project", field_type: "note_link", target_type: "Project" }
            ]
        })
    "#;
    let schemas = parse_schemas_from_script(script).unwrap();
    let field = &schemas[0].fields[0];
    assert_eq!(field.field_type, "note_link");
    assert_eq!(field.target_type, Some("Project".to_string()));
}
```

**Step 7: Commit**
```bash
git add krillnotes-core/src/core/scripting/schema.rs
git commit -m "feat: handle note_link in schema parsing and default fields"
```

---

### Task 4: `sync_note_links` — maintain junction table on write

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (add helper method)

This method is called after any note write (create or update) to keep `note_links` in sync.

**Step 1: Write failing test**

```rust
#[test]
fn test_sync_note_links_inserts_row() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let target = ws.create_note_with_type("Task", None).unwrap();
    let source = ws.create_note_with_type("Task", None).unwrap();

    let mut fields = HashMap::new();
    fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
    ws.update_note(&source.id, source.title.clone(), fields).unwrap();

    let conn = ws.connection();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM note_links WHERE source_id = ?1 AND target_id = ?2",
        [&source.id, &target.id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_sync_note_links_removes_row_when_cleared() {
    // set up link, then update with NoteLink(None), verify row is gone
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let target = ws.create_note_with_type("Task", None).unwrap();
    let source = ws.create_note_with_type("Task", None).unwrap();

    // Set link
    let mut fields = HashMap::new();
    fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
    ws.update_note(&source.id, source.title.clone(), fields).unwrap();

    // Clear link
    let mut fields2 = HashMap::new();
    fields2.insert("link".into(), FieldValue::NoteLink(None));
    ws.update_note(&source.id, source.title.clone(), fields2).unwrap();

    let conn = ws.connection();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM note_links WHERE source_id = ?1",
        [&source.id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0);
}
```

Run: `cargo test -p krillnotes-core test_sync_note_links -- --nocapture`
Expected: FAIL

**Step 2: Implement `sync_note_links`**

Add as a private method on `Workspace`:

```rust
fn sync_note_links(&self, note_id: &str, fields: &HashMap<String, FieldValue>) -> Result<()> {
    let conn = self.connection();
    // Clear existing rows for this source note (full replace approach)
    conn.execute("DELETE FROM note_links WHERE source_id = ?1", [note_id])?;
    // Insert rows for any non-null NoteLink fields
    for (field_name, value) in fields {
        if let FieldValue::NoteLink(Some(target_id)) = value {
            conn.execute(
                "INSERT OR REPLACE INTO note_links (source_id, field_name, target_id)
                 VALUES (?1, ?2, ?3)",
                [note_id, field_name.as_str(), target_id.as_str()],
            )?;
        }
    }
    Ok(())
}
```

**Step 3: Call `sync_note_links` from `update_note`**

In `update_note` (around line 1480, after the UPDATE SQL), add:

```rust
self.sync_note_links(note_id, &fields)?;
```

**Step 4: Call `sync_note_links` from `create_note_with_type`** (or whatever the note creation method is called). Look for where `INSERT INTO notes` happens and call `sync_note_links` with the new note's id and its initial fields after the insert.

**Step 5: Run tests** — Expected: PASS

**Step 6: Commit**
```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: add sync_note_links to maintain junction table on note write"
```

---

### Task 5: `clear_links_to` — deletion cleanup

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

When a note is about to be deleted, this method finds all notes that link to it, nulls out those fields in `fields_json`, and removes the `note_links` rows — all within a transaction.

**Step 1: Write failing test**

```rust
#[test]
fn test_clear_links_to_nulls_field_in_source_note() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let target = ws.create_note_with_type("Task", None).unwrap();
    let source = ws.create_note_with_type("Task", None).unwrap();

    // Establish link
    let mut fields = HashMap::new();
    fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
    ws.update_note(&source.id, source.title.clone(), fields).unwrap();

    // Clear links pointing to target
    ws.clear_links_to(&target.id).unwrap();

    // source note's "link" field should now be NoteLink(None)
    let updated_source = ws.get_note(&source.id).unwrap();
    assert!(matches!(
        updated_source.fields.get("link").unwrap(),
        FieldValue::NoteLink(None)
    ));

    // note_links row should be gone
    let conn = ws.connection();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM note_links WHERE target_id = ?1",
        [&target.id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0);
}
```

Run: `cargo test -p krillnotes-core test_clear_links_to -- --nocapture`
Expected: FAIL

**Step 2: Implement `clear_links_to`**

Add as a public method on `Workspace` (public so the delete functions can call it):

```rust
pub fn clear_links_to(&mut self, target_id: &str) -> Result<()> {
    // Find all notes that link to target_id
    let links: Vec<(String, String)> = {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT source_id, field_name FROM note_links WHERE target_id = ?1",
        )?;
        stmt.query_map([target_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };

    for (source_id, field_name) in &links {
        // Load fields_json, null the link field, write back
        let fields_json: String = {
            let conn = self.connection();
            conn.query_row(
                "SELECT fields_json FROM notes WHERE id = ?1",
                [source_id],
                |row| row.get(0),
            )?
        };
        let mut json_val: serde_json::Value = serde_json::from_str(&fields_json)?;
        if let Some(obj) = json_val.as_object_mut() {
            obj.insert(field_name.clone(), serde_json::Value::Null);
        }
        let updated_json = serde_json::to_string(&json_val)?;
        self.connection().execute(
            "UPDATE notes SET fields_json = ?1, modified_at = ?2 WHERE id = ?3",
            rusqlite::params![updated_json, crate::core::now(), source_id],
        )?;
    }

    // Remove all note_links rows pointing to target_id
    self.connection().execute(
        "DELETE FROM note_links WHERE target_id = ?1",
        [target_id],
    )?;

    Ok(())
}
```

**Note on transactions:** The deletion methods use `self.storage.connection_mut()` to get a transaction. You may need to refactor `clear_links_to` to accept a `&Transaction` parameter so it runs inside the existing deletion transaction. Check how `delete_note_recursive` opens its transaction and thread the connection through accordingly.

**Step 3: Run tests** — Expected: PASS

**Step 4: Commit**
```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: add clear_links_to for deletion cleanup"
```

---

### Task 6: Wire deletion cleanup into both delete strategies

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs:1257` (`delete_note_recursive`)
- Modify: `krillnotes-core/src/core/workspace.rs:1332` (`delete_note_promote`)

**Step 1: Write failing test**

```rust
#[test]
fn test_delete_note_nulls_links_in_other_notes() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let target = ws.create_note_with_type("Task", None).unwrap();
    let source = ws.create_note_with_type("Task", None).unwrap();

    let mut fields = HashMap::new();
    fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
    ws.update_note(&source.id, source.title.clone(), fields).unwrap();

    // Delete target
    ws.delete_note(&target.id, DeleteStrategy::DeleteAll).unwrap();

    // source's link field should be null
    let updated_source = ws.get_note(&source.id).unwrap();
    assert!(matches!(
        updated_source.fields.get("link").unwrap(),
        FieldValue::NoteLink(None)
    ));
}

#[test]
fn test_delete_note_recursive_clears_links_for_entire_subtree() {
    // Create parent → child subtree; another note links to the child.
    // Deleting parent (recursive) should null the link.
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let parent = ws.create_note_with_type("Task", None).unwrap();
    let child = ws.create_note_with_type("Task", Some(parent.id.clone())).unwrap();
    let observer = ws.create_note_with_type("Task", None).unwrap();

    let mut fields = HashMap::new();
    fields.insert("link".into(), FieldValue::NoteLink(Some(child.id.clone())));
    ws.update_note(&observer.id, observer.title.clone(), fields).unwrap();

    ws.delete_note(&parent.id, DeleteStrategy::DeleteAll).unwrap();

    let updated_observer = ws.get_note(&observer.id).unwrap();
    assert!(matches!(
        updated_observer.fields.get("link").unwrap(),
        FieldValue::NoteLink(None)
    ));
}
```

Run: `cargo test -p krillnotes-core test_delete_note_nulls_links -- --nocapture`
Expected: FAIL

**Step 2: Add to `delete_note_recursive`**

At the beginning of `delete_note_recursive`, before the recursive deletion logic, collect all IDs in the subtree that will be deleted, then clear links to each:

```rust
// Gather all IDs in the subtree
let all_ids = self.collect_subtree_ids(note_id)?;

// Clear incoming links for every note in the subtree
for id in &all_ids {
    self.clear_links_to(id)?;
}
```

You'll need a helper `collect_subtree_ids(note_id)` that returns `Vec<String>` of the note and all descendants using a recursive query or loop.

**Step 3: Add to `delete_note_promote`**

At the beginning, before the promote logic:

```rust
self.clear_links_to(note_id)?;
```

**Step 4: Run tests** — Expected: PASS

**Step 5: Commit**
```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: clear incoming note links before deletion"
```

---

### Task 7: `get_notes_with_link` — workspace method + Rhai registration

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (Rhai engine registration)

**Step 1: Write failing test**

```rust
#[test]
fn test_get_notes_with_link_returns_linking_notes() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let target = ws.create_note_with_type("Task", None).unwrap();
    let source1 = ws.create_note_with_type("Task", None).unwrap();
    let source2 = ws.create_note_with_type("Task", None).unwrap();

    for source in [&source1, &source2] {
        let mut fields = HashMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();
    }

    let results = ws.get_notes_with_link(&target.id).unwrap();
    assert_eq!(results.len(), 2);
    let result_ids: Vec<&str> = results.iter().map(|n| n.id.as_str()).collect();
    assert!(result_ids.contains(&source1.id.as_str()));
    assert!(result_ids.contains(&source2.id.as_str()));
}

#[test]
fn test_get_notes_with_link_returns_empty_for_unlinked_note() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let note = ws.create_note_with_type("Task", None).unwrap();
    let results = ws.get_notes_with_link(&note.id).unwrap();
    assert!(results.is_empty());
}
```

Run: `cargo test -p krillnotes-core test_get_notes_with_link -- --nocapture`
Expected: FAIL

**Step 2: Implement `get_notes_with_link`** in workspace.rs:

```rust
pub fn get_notes_with_link(&self, target_id: &str) -> Result<Vec<Note>> {
    let conn = self.connection();
    let mut stmt = conn.prepare(
        "SELECT n.id FROM note_links nl
         JOIN notes n ON n.id = nl.source_id
         WHERE nl.target_id = ?1",
    )?;
    let source_ids: Vec<String> = stmt
        .query_map([target_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;

    let mut notes = Vec::new();
    for id in source_ids {
        notes.push(self.get_note(&id)?);
    }
    Ok(notes)
}
```

**Step 3: Register in Rhai engine** in `scripting/mod.rs`

Find where `get_notes_of_type` or similar functions are registered on the `QueryContext`. Add `get_notes_with_link` following the same pattern:

```rust
engine.register_fn("get_notes_with_link", move |note_id: &str| -> Array {
    ctx.get_notes_with_link(note_id)
        .unwrap_or_default()
        .into_iter()
        .map(|n| note_to_dynamic(&n))
        .collect()
});
```

Look at how `get_notes_of_type` is registered and mirror it exactly.

**Step 4: Run tests** — Expected: PASS

**Step 5: Commit**
```bash
git add krillnotes-core/src/core/workspace.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add get_notes_with_link workspace method and Rhai function"
```

---

### Task 8: `search_notes` — workspace method + Tauri command

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Write failing test**

```rust
#[test]
fn test_search_notes_matches_title() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [] })"#
    );
    let mut n = ws.create_note_with_type("Task", None).unwrap();
    ws.update_note(&n.id, "Fix login bug".into(), HashMap::new()).unwrap();

    let results = ws.search_notes("login", None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, n.id);
}

#[test]
fn test_search_notes_filters_by_target_type() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"
        schema("Task", #{ fields: [] })
        schema("Note", #{ fields: [] })
        "#
    );
    let task = ws.create_note_with_type("Task", None).unwrap();
    ws.update_note(&task.id, "login task".into(), HashMap::new()).unwrap();
    let note = ws.create_note_with_type("Note", None).unwrap();
    ws.update_note(&note.id, "login note".into(), HashMap::new()).unwrap();

    let results = ws.search_notes("login", Some("Task")).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, task.id);
}

#[test]
fn test_search_notes_matches_text_fields() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Contact", #{ fields: [#{ name: "email", field_type: "email" }] })"#
    );
    let c = ws.create_note_with_type("Contact", None).unwrap();
    let mut fields = HashMap::new();
    fields.insert("email".into(), FieldValue::Email("alice@example.com".into()));
    ws.update_note(&c.id, "Alice".into(), fields).unwrap();

    let results = ws.search_notes("alice@example", None).unwrap();
    assert_eq!(results.len(), 1);
}
```

Run: `cargo test -p krillnotes-core test_search_notes -- --nocapture`
Expected: FAIL

**Step 2: Add `NoteSearchResult` struct** to workspace.rs (or note.rs):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSearchResult {
    pub id: String,
    pub title: String,
}
```

**Step 3: Implement `search_notes`** in workspace.rs:

```rust
pub fn search_notes(
    &self,
    query: &str,
    target_type: Option<&str>,
) -> Result<Vec<NoteSearchResult>> {
    let query_lower = query.to_lowercase();
    if query_lower.is_empty() {
        return Ok(vec![]);
    }

    let all_notes = self.get_all_notes()?; // use list_notes or equivalent

    let results = all_notes
        .into_iter()
        .filter(|n| {
            if let Some(t) = target_type {
                n.node_type == t
            } else {
                true
            }
        })
        .filter(|n| {
            if n.title.to_lowercase().contains(&query_lower) {
                return true;
            }
            for value in n.fields.values() {
                let text = match value {
                    FieldValue::Text(s) | FieldValue::Email(s) => Some(s.as_str()),
                    _ => None,
                };
                if let Some(t) = text {
                    if t.to_lowercase().contains(&query_lower) {
                        return true;
                    }
                }
            }
            false
        })
        .map(|n| NoteSearchResult { id: n.id, title: n.title })
        .collect();

    Ok(results)
}
```

**Note:** Look for an existing `get_all_notes` or `list_notes` method that returns `Vec<Note>`. If the existing `list_notes` returns a tree structure, find the underlying flat query.

**Step 4: Add Tauri command** in `lib.rs`:

```rust
#[tauri::command]
fn search_notes(
    window: tauri::Window,
    state: State<'_, AppState>,
    query: String,
    target_type: Option<String>,
) -> Result<Vec<NoteSearchResult>> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .map_err(|e| KrillnotesError::Custom(e.to_string()))?;
    let workspace = workspaces.get(label)  // read-only, no mut needed
        .ok_or(KrillnotesError::Custom("Workspace not found".to_string()))?;
    workspace.search_notes(&query, target_type.as_deref())
        .map_err(|e| e.into())
}
```

Register `search_notes` in the `.invoke_handler(tauri::generate_handler![...])` list.

**Step 5: Run tests** — Expected: PASS

**Step 6: Commit**
```bash
git add krillnotes-core/src/core/workspace.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add search_notes workspace method and Tauri command"
```

---

### Task 9: `rebuild_note_links_index` + import hook

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`
- Modify: `krillnotes-core/src/core/export.rs:303` (`import_workspace`)

**Step 1: Write failing test**

```rust
#[test]
fn test_rebuild_note_links_index_repopulates_from_fields_json() {
    let mut ws = helpers::create_test_workspace_with_schema(
        r#"schema("Task", #{ fields: [#{ name: "link", field_type: "note_link" }] })"#
    );
    let target = ws.create_note_with_type("Task", None).unwrap();
    let source = ws.create_note_with_type("Task", None).unwrap();
    let mut fields = HashMap::new();
    fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
    ws.update_note(&source.id, source.title.clone(), fields).unwrap();

    // Manually wipe the index
    ws.connection().execute("DELETE FROM note_links", []).unwrap();

    // Rebuild
    ws.rebuild_note_links_index().unwrap();

    let conn = ws.connection();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM note_links WHERE source_id = ?1 AND target_id = ?2",
        [&source.id, &target.id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
```

Run: `cargo test -p krillnotes-core test_rebuild_note_links -- --nocapture`
Expected: FAIL

**Step 2: Implement `rebuild_note_links_index`** in workspace.rs:

```rust
pub fn rebuild_note_links_index(&self) -> Result<()> {
    let all_notes = self.get_all_notes()?;
    let conn = self.connection();
    conn.execute("DELETE FROM note_links", [])?;
    for note in &all_notes {
        for (field_name, value) in &note.fields {
            if let FieldValue::NoteLink(Some(target_id)) = value {
                // Only insert if target note exists (skip dangling refs from corrupted data)
                let exists: bool = conn
                    .query_row(
                        "SELECT COUNT(*) FROM notes WHERE id = ?1",
                        [target_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap_or(0)
                    > 0;
                if exists {
                    conn.execute(
                        "INSERT OR REPLACE INTO note_links (source_id, field_name, target_id)
                         VALUES (?1, ?2, ?3)",
                        [&note.id, field_name, target_id],
                    )?;
                }
            }
        }
    }
    Ok(())
}
```

**Step 3: Call from `import_workspace`** in export.rs

After all notes have been restored and the workspace is opened, call:

```rust
workspace.rebuild_note_links_index()?;
```

Find the spot near the end of `import_workspace` where notes are restored. This is a "build the index" step that runs once after all data is in place.

**Step 4: Run tests** — Expected: PASS

**Step 5: Commit**
```bash
git add krillnotes-core/src/core/workspace.rs krillnotes-core/src/core/export.rs
git commit -m "feat: add rebuild_note_links_index and call it on import"
```

---

### Task 10: TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:28-45`

**Step 1: Update `FieldValue` union**

```typescript
export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean }
  | { Date: string | null }
  | { Email: string }
  | { NoteLink: string | null };  // ADD THIS — null = not set, string = linked note UUID
```

**Step 2: Update `FieldType`**

```typescript
export type FieldType =
  | 'text' | 'textarea' | 'number' | 'boolean'
  | 'date' | 'email' | 'select' | 'rating'
  | 'note_link';  // ADD THIS
```

**Step 3: Update `FieldDefinition`**

```typescript
export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
  options: string[];
  max: number;
  targetType?: string;   // ADD THIS — only meaningful for note_link fields
}
```

**Step 4: Add `NoteSearchResult` type**

```typescript
export interface NoteSearchResult {
  id: string;
  title: string;
}
```

**Step 5: Build to verify no TypeScript errors**

Run: `cd krillnotes-desktop && npm run build`
Expected: no TS errors (there will be TS errors in FieldDisplay/FieldEditor where `FieldValue` switches are exhaustive — fix them in the next tasks)

**Step 6: Commit**
```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add NoteLink to TypeScript FieldValue and FieldType"
```

---

### Task 11: `NoteLinkEditor` component

**Files:**
- Create: `krillnotes-desktop/src/components/NoteLinkEditor.tsx`

**Step 1: Create the component**

```tsx
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { NoteSearchResult } from '../types';

interface Props {
  value: string | null;          // current linked note UUID, or null
  targetType?: string;           // optional schema type filter
  onChange: (id: string | null) => void;
}

export function NoteLinkEditor({ value, targetType, onChange }: Props) {
  const [displayTitle, setDisplayTitle] = useState<string>('');
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<NoteSearchResult[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Resolve UUID → title on mount / when value changes
  useEffect(() => {
    if (!value) {
      setDisplayTitle('');
      return;
    }
    invoke<{ id: string; title: string }>('get_note', { noteId: value })
      .then(n => setDisplayTitle(n.title))
      .catch(() => setDisplayTitle('(deleted)'));
  }, [value]);

  function handleInput(e: React.ChangeEvent<HTMLInputElement>) {
    const q = e.target.value;
    setQuery(q);
    setIsOpen(true);

    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!q.trim()) {
      setResults([]);
      return;
    }
    debounceRef.current = setTimeout(async () => {
      try {
        const r = await invoke<NoteSearchResult[]>('search_notes', {
          query: q,
          targetType: targetType ?? null,
        });
        setResults(r);
      } catch {
        setResults([]);
      }
    }, 300);
  }

  function handleSelect(result: NoteSearchResult) {
    onChange(result.id);
    setQuery('');
    setResults([]);
    setIsOpen(false);
  }

  function handleClear(e: React.MouseEvent) {
    e.stopPropagation();
    onChange(null);
    setQuery('');
    setResults([]);
    setIsOpen(false);
  }

  // Show the query if the user is typing, otherwise show the resolved title
  const inputValue = isOpen ? query : displayTitle;

  return (
    <div style={{ position: 'relative' }}>
      <div style={{ display: 'flex', gap: 4 }}>
        <input
          type="text"
          value={inputValue}
          placeholder="Search for a note…"
          onChange={handleInput}
          onFocus={() => setIsOpen(true)}
          onBlur={() => setTimeout(() => setIsOpen(false), 150)}
          style={{ flex: 1 }}
        />
        {value && (
          <button type="button" onClick={handleClear} title="Clear link">
            ✕
          </button>
        )}
      </div>
      {isOpen && results.length > 0 && (
        <ul
          style={{
            position: 'absolute',
            top: '100%',
            left: 0,
            right: 0,
            zIndex: 100,
            background: 'var(--bg-panel)',
            border: '1px solid var(--border)',
            borderRadius: 4,
            padding: 0,
            margin: 0,
            listStyle: 'none',
            maxHeight: 200,
            overflowY: 'auto',
          }}
        >
          {results.map(r => (
            <li
              key={r.id}
              onMouseDown={() => handleSelect(r)}
              style={{ padding: '6px 10px', cursor: 'pointer' }}
            >
              {r.title}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
```

**Note on styling:** Use CSS variables from the existing theme (`var(--bg-panel)`, `var(--border)` etc.). Check what variables are already used in `FieldEditor.tsx` and use the same ones for consistency.

**Step 2: Build**

Run: `cd krillnotes-desktop && npm run build`
Expected: no errors (component is unused yet)

**Step 3: Commit**
```bash
git add krillnotes-desktop/src/components/NoteLinkEditor.tsx
git commit -m "feat: add NoteLinkEditor inline search component"
```

---

### Task 12: `FieldEditor` — add `note_link` branch

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldEditor.tsx:15-115`

**Step 1: Add branch**

In the switch/if-else block that renders editor inputs (before the default text case), add:

```tsx
if (fieldType === 'note_link') {
  const currentId = value && 'NoteLink' in value ? value.NoteLink : null;
  return (
    <NoteLinkEditor
      value={currentId}
      targetType={/* pass targetType from props */}
      onChange={(id) => onChange({ NoteLink: id })}
    />
  );
}
```

**Step 2: Pass `targetType` through props**

`FieldEditor` needs to receive `targetType?: string` as a prop. Add it to the interface and thread it through from `InfoPanel.tsx` (line 409):

In `InfoPanel.tsx`:
```tsx
<FieldEditor
  ...existing props...
  targetType={field.targetType}
/>
```

**Step 3: Update `defaultValueForFieldType`**

Find the function in `InfoPanel.tsx` (or wherever it lives) that returns default FieldValue for a given type. Add:

```typescript
case 'note_link':
  return { NoteLink: null };
```

**Step 4: Build**

Run: `cd krillnotes-desktop && npm run build`
Expected: no errors

**Step 5: Commit**
```bash
git add krillnotes-desktop/src/components/FieldEditor.tsx krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: add note_link field editing via NoteLinkEditor"
```

---

### Task 13: `FieldDisplay` — lazy title resolution for `note_link`

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx:13-43`

**Step 1: Add lazy-resolution logic**

`FieldDisplay` is currently a pure/synchronous component. For `NoteLink` it needs async resolution. Use `useState` + `useEffect`:

```tsx
// At the top of FieldDisplay component (or as a sub-component):
function NoteLinkDisplay({ noteId }: { noteId: string }) {
  const [title, setTitle] = useState<string | null>(null);

  useEffect(() => {
    invoke<{ id: string; title: string }>('get_note', { noteId })
      .then(n => setTitle(n.title))
      .catch(() => setTitle('(deleted)'));
  }, [noteId]);

  if (title === null) return <span>…</span>;

  return (
    <a
      className="kn-view-link"
      data-note-id={noteId}
      style={{ cursor: 'pointer', textDecoration: 'underline' }}
      onClick={(e) => e.preventDefault()}
    >
      {title}
    </a>
  );
}
```

**Step 2: Add NoteLink branch to `FieldDisplay`**

In the existing type dispatch block, add before the default:

```tsx
if (fieldType === 'note_link') {
  if (!value || !('NoteLink' in value) || value.NoteLink === null) {
    return <span>—</span>;
  }
  return <NoteLinkDisplay noteId={value.NoteLink} />;
}
```

**Note:** The `kn-view-link` anchor is already handled by `InfoPanel.tsx`'s click handler that intercepts `data-note-id` attributes. The `onClick` preventDefault in the component prevents browser default but the parent's event delegation handles navigation. Check `InfoPanel.tsx` around lines 428-434 to confirm the existing click delegation pattern.

**Step 3: Build**

Run: `cd krillnotes-desktop && npm run build`
Expected: no errors

**Step 4: Run all Rust tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

**Step 5: Commit**
```bash
git add krillnotes-desktop/src/components/FieldDisplay.tsx
git commit -m "feat: add NoteLink display with lazy title resolution"
```

---

### Task 14: Update SCRIPTING.md documentation

**Files:**
- Modify: `SCRIPTING.md`

**Step 1: Add `note_link` to the field types table**

Find the field types table and add a row:

```
| `note_link` | Link to another note (UUID) | `target_type` (optional): schema type filter | Yes |
```

**Step 2: Add `get_notes_with_link` to the Rhai functions section**

Find the scripting functions reference and add:

```markdown
#### `get_notes_with_link(note_id)`
Returns an array of notes that have any `note_link` field pointing to the given note ID. Useful for displaying backlinks in an `on_view` hook.

```rhai
on_view("Project", |note| {
    let tasks = get_notes_with_link(note.id);
    if tasks.len() > 0 {
        heading("Linked here");
        list(tasks.map(|t| link_to(t)))
    }
})
```

**Step 3: Add a schema example**

```rhai
schema("Task", #{
    fields: [
        // Link to any note of type "Project"
        #{ name: "linked_project", field_type: "note_link", target_type: "Project" },
        // Link to any note (no type filter)
        #{ name: "blocked_by", field_type: "note_link" },
    ]
})
```

**Step 4: Commit**
```bash
git add SCRIPTING.md
git commit -m "docs: document note_link field type and get_notes_with_link in SCRIPTING.md"
```

---

### Task 15: Manual end-to-end verification

**Checklist:**
1. Open Krillnotes, create a workspace with this schema:
   ```rhai
   schema("Task", #{
       fields: [
           #{ name: "linked_project", field_type: "note_link", target_type: "Project" },
       ]
   })
   schema("Project", #{ fields: [] })
   ```
2. Create two Project notes ("Project Alpha", "Project Beta")
3. Create a Task note — in edit mode, click the `linked_project` field, type "alpha" — verify dropdown shows "Project Alpha" only (not "Project Beta" or Task notes)
4. Select "Project Alpha" — verify field shows "Project Alpha" as a clickable link in view mode
5. Click the link — verify it navigates to "Project Alpha"
6. Edit the Task again, click ✕ on the field — verify field clears to "—" in view mode
7. Re-link to "Project Alpha", then delete "Project Alpha"
8. Verify the Task's `linked_project` field is now "—" (null)
9. In a Project's `on_view` hook, use `get_notes_with_link(note.id)` — verify it returns the linked tasks
10. Export the workspace, create fresh workspace, import — verify links are restored correctly

---

## Final Commit Count

Approximate: 14-15 commits, one per task — each small, reviewable, and independently testable.
