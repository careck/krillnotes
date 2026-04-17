# Note Checkbox Feature — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a first-class `is_checked` boolean to the `Note` struct so that schemas with `show_checkbox: true` render an interactive checkbox in the tree view.

**Architecture:** `is_checked` lives on the `Note` struct as a peer of `title` — a global property stored in the `notes` DB table, not in `fields_json`. A new `SetChecked` operation variant tracks changes for CRDT sync. The `Schema` struct gains a `show_checkbox` flag parsed from Rhai scripts. TreeNode conditionally renders a checkbox when the schema flag is set. A built-in `TodoItem` system script demonstrates the feature.

**Tech Stack:** Rust (krillnotes-core), Tauri v2 commands, React 19 + Tailwind v4, Rhai scripting, SQLCipher.

---

## File Map

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `krillnotes-core/src/core/note.rs` | Add `is_checked: bool` to `Note` struct |
| Modify | `krillnotes-core/src/core/operation.rs` | Add `SetChecked` variant + wire into all match arms |
| Modify | `krillnotes-core/src/core/scripting/schema.rs` | Add `show_checkbox: bool` to `Schema`, parse from Rhai |
| Modify | `krillnotes-core/src/core/scripting/mod.rs` | Expose `note.is_checked` in Rhai note maps |
| Modify | `krillnotes-core/src/core/storage.rs` | DB migration: `ALTER TABLE notes ADD COLUMN is_checked` |
| Modify | `krillnotes-core/src/core/workspace/mod.rs` | Update `NoteRow` type alias, `map_note_row`, `note_from_row_tuple` |
| Modify | `krillnotes-core/src/core/workspace/notes.rs` | Add `set_note_checked()` method, update INSERT/SELECT queries |
| Modify | `krillnotes-core/src/core/workspace/sync.rs` | Handle `SetChecked` in `apply_remote_op`, update `CreateNote` INSERT |
| Modify | `krillnotes-core/src/core/export.rs` | Include `is_checked` in export INSERT |
| Modify | `krillnotes-core/src/core/workspace/tests.rs` | Tests for `set_note_checked` |
| Modify | `krillnotes-desktop/src-tauri/src/commands/notes.rs` | Add `set_note_checked` Tauri command |
| Modify | `krillnotes-desktop/src-tauri/src/lib.rs` | Register `set_note_checked` in `generate_handler!` |
| Modify | `krillnotes-desktop/src-tauri/src/commands/scripting.rs` | Add `show_checkbox` to `SchemaInfo` + `schema_to_info` |
| Modify | `krillnotes-desktop/src/types.ts` | Add `isChecked` to `Note`, `showCheckbox` to `SchemaInfo` |
| Modify | `krillnotes-desktop/src/components/TreeNode.tsx` | Render checkbox when `showCheckbox` is set |
| Modify | `krillnotes-desktop/src/components/WorkspaceView.tsx` | Add `handleToggleChecked` handler, pass to TreeNode |
| Create | `krillnotes-core/src/core/system_scripts/01_todo_item.schema.rhai` | Built-in TodoItem schema |
| Modify | `krillnotes-desktop/src/i18n/locales/*.json` (7 files) | i18n keys for checkbox UI |

---

## Task 1: Database Migration + Note Struct

**Files:**
- Modify: `krillnotes-core/src/core/note.rs:62-97`
- Modify: `krillnotes-core/src/core/storage.rs:430-445`

- [ ] **Step 1: Add `is_checked` to the `Note` struct**

In `krillnotes-core/src/core/note.rs`, add after `schema_version` (line 96):

```rust
    /// Whether this note's checkbox is ticked. Only meaningful when the
    /// schema sets `show_checkbox: true`; ignored otherwise.
    #[serde(default)]
    pub is_checked: bool,
```

The `#[serde(default)]` allows importing archives from before this field existed (same pattern as `tags`).

- [ ] **Step 2: Add the DB migration in `storage.rs`**

In `krillnotes-core/src/core/storage.rs`, after the `received_from_peer` migration block (after line 443), add:

```rust
        // Migration: add is_checked column to notes if absent.
        let is_checked_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_checked'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )?;
        if !is_checked_exists {
            conn.execute(
                "ALTER TABLE notes ADD COLUMN is_checked INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
```

- [ ] **Step 3: Update `schema.sql` DDL**

In `krillnotes-core/src/core/schema.sql`, add `is_checked INTEGER NOT NULL DEFAULT 0` after `schema_version` in the `CREATE TABLE notes` statement:

```sql
CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    schema TEXT NOT NULL,
    parent_id TEXT,
    position REAL NOT NULL DEFAULT 0.0,
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    created_by TEXT NOT NULL DEFAULT '',
    modified_by TEXT NOT NULL DEFAULT '',
    fields_json TEXT NOT NULL DEFAULT '{}',
    is_expanded INTEGER DEFAULT 1,
    schema_version INTEGER NOT NULL DEFAULT 1,
    is_checked INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_id) REFERENCES notes(id) ON DELETE CASCADE
);
```

- [ ] **Step 4: Update `NoteRow` type alias in `workspace/mod.rs`**

In `krillnotes-core/src/core/workspace/mod.rs`, the `NoteRow` type at line 1461 is a 13-element tuple. Add `bool` for `is_checked` as element 13 (before `Option<String>` for tags_csv), making it 14 elements:

```rust
type NoteRow = (String, String, String, Option<String>, f64, i64, i64, String, String, String, i64, u32, bool, Option<String>);
```

- [ ] **Step 5: Update `map_note_row` in `workspace/mod.rs`**

At line 1467, update to read the new column. The new column position is 12 (shifting `tags_csv` to 13):

```rust
fn map_note_row(row: &rusqlite::Row) -> rusqlite::Result<NoteRow> {
    Ok((
        row.get::<_, String>(0)?,           // id
        row.get::<_, String>(1)?,           // title
        row.get::<_, String>(2)?,           // schema
        row.get::<_, Option<String>>(3)?,   // parent_id
        row.get::<_, f64>(4)?,              // position
        row.get::<_, i64>(5)?,              // created_at
        row.get::<_, i64>(6)?,              // modified_at
        row.get::<_, String>(7).unwrap_or_default(),  // created_by
        row.get::<_, String>(8).unwrap_or_default(),  // modified_by
        row.get::<_, String>(9)?,           // fields_json
        row.get::<_, i64>(10)?,             // is_expanded
        row.get::<_, u32>(11)?,             // schema_version
        row.get::<_, bool>(12)?,            // is_checked
        row.get::<_, Option<String>>(13)?,  // tags_csv
    ))
}
```

- [ ] **Step 6: Update `note_from_row_tuple` in `workspace/mod.rs`**

Update the destructuring at line 1487 to include `is_checked`:

```rust
fn note_from_row_tuple(
    (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded_int, schema_version, is_checked, tags_csv): NoteRow,
) -> Result<Note> {
    let mut tags: Vec<String> = tags_csv
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    tags.sort();
    Ok(Note {
        id,
        title,
        schema,
        parent_id,
        position,
        created_at,
        modified_at,
        created_by,
        modified_by,
        fields: serde_json::from_str(&fields_json)?,
        is_expanded: is_expanded_int == 1,
        tags,
        schema_version,
        is_checked,
    })
}
```

- [ ] **Step 7: Update all SELECT queries that feed `map_note_row`**

Every query that selects into the 13-column tuple must now select 14 columns. Add `n.is_checked` after `n.schema_version` in these locations:

**`workspace/notes.rs`** — 4 queries:

1. `list_all_notes` (~line 819):
```sql
SELECT n.id, n.title, n.schema, n.parent_id, n.position,
       n.created_at, n.modified_at, n.created_by, n.modified_by,
       n.fields_json, n.is_expanded, n.schema_version,
       n.is_checked,
       GROUP_CONCAT(nt.tag, ',') AS tags_csv
```

2. `get_notes_for_tag` (~line 663):
```sql
SELECT n.id, n.title, n.schema, n.parent_id, n.position,
       n.created_at, n.modified_at, n.created_by, n.modified_by,
       n.fields_json, n.is_expanded, n.schema_version,
       n.is_checked,
       GROUP_CONCAT(nt2.tag, ',') AS tags_csv
```

3. `get_children` (~line 1166):
```sql
SELECT n.id, n.title, n.schema, n.parent_id, n.position,
       n.created_at, n.modified_at, n.created_by, n.modified_by,
       n.fields_json, n.is_expanded, n.schema_version,
       n.is_checked,
       GROUP_CONCAT(nt.tag, ',') AS tags_csv
```

4. `get_subtree` (~line 1925):
```sql
SELECT n.id, n.title, n.schema, n.parent_id, n.position,
       n.created_at, n.modified_at, n.created_by, n.modified_by,
       n.fields_json, n.is_expanded, n.schema_version,
       n.is_checked,
       GROUP_CONCAT(nt.tag, ',') AS tags_csv
```

Search for all usages: `grep -n "n.schema_version" krillnotes-core/src/core/workspace/` to find any others.

- [ ] **Step 8: Update `create_note` INSERT in `workspace/notes.rs`**

At line 154, add `is_checked` to the INSERT:

```rust
tx.execute(
    "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version, is_checked)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    rusqlite::params![
        note.id,
        note.title,
        note.schema,
        note.parent_id,
        note.position,
        note.created_at,
        note.modified_at,
        note.created_by,
        note.modified_by,
        serde_json::to_string(&note.fields)?,
        true,
        note.schema_version,
        note.is_checked,
    ],
)?;
```

Also update the `Note` construction at line 107 to include `is_checked: false`.

- [ ] **Step 9: Update root-note INSERTs in `workspace/mod.rs`**

There are two root-note INSERT statements in `workspace/mod.rs` (around lines 282 and 505 — the `create` and `open` paths). Add `is_checked` column and param to both:

```sql
INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version, is_checked)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
```

And add `root.is_checked` (or `false`) to the params, plus `is_checked: false` to the `Note` construction.

- [ ] **Step 10: Compile and verify**

Run: `cargo check -p krillnotes-core 2>&1 | head -40`

Expected: Compilation errors in `operation.rs` match arms (we haven't added the Operation variant yet) — that's fine, but no errors in `note.rs`, `storage.rs`, or `workspace/mod.rs`.

- [ ] **Step 11: Commit**

```bash
git add krillnotes-core/src/core/note.rs krillnotes-core/src/core/storage.rs krillnotes-core/src/core/schema.sql krillnotes-core/src/core/workspace/mod.rs krillnotes-core/src/core/workspace/notes.rs
git commit -m "feat(core): add is_checked column to Note struct and DB schema"
```

---

## Task 2: `SetChecked` Operation Variant

**Files:**
- Modify: `krillnotes-core/src/core/operation.rs:20-550`

- [ ] **Step 1: Add `SetChecked` variant to the `Operation` enum**

In `krillnotes-core/src/core/operation.rs`, after the `RegisterDevice` variant (line 368), before the closing `}` of the enum, add:

```rust
    /// The checked state of a note was toggled.
    SetChecked {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note whose checked state changed.
        note_id: String,
        /// New checked state.
        checked: bool,
        /// Public key (base64) of the identity that modified this note.
        modified_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
```

- [ ] **Step 2: Add `SetChecked` to the `operation_id()` match**

At line 394 (before `=> operation_id`), add:

```rust
            | Self::SetChecked { operation_id, .. }
```

- [ ] **Step 3: Add `SetChecked` to the `timestamp()` match**

At line 420 (before `=> *timestamp`), add:

```rust
            | Self::SetChecked { timestamp, .. }
```

- [ ] **Step 4: Add `SetChecked` to the `device_id()` match**

At line 446 (before `=> device_id`), add:

```rust
            | Self::SetChecked { device_id, .. }
```

- [ ] **Step 5: Add `SetChecked` to `author_key()`**

After `Self::RegisterDevice { identity_public_key, .. } => identity_public_key,` (line 474), add:

```rust
            Self::SetChecked { modified_by, .. } => modified_by,
```

- [ ] **Step 6: Add `SetChecked` to `set_author_key()`**

After `Self::RegisterDevice { identity_public_key, .. } => *identity_public_key = key,` (line 500), add:

```rust
            Self::SetChecked { modified_by, .. } => *modified_by = key,
```

- [ ] **Step 7: Add `SetChecked` to `set_signature()`**

At line 523 (before `=> *signature = sig`), add:

```rust
            | Self::SetChecked { signature, .. }
```

- [ ] **Step 8: Add `SetChecked` to `get_signature()`**

At line 547 (before `=> signature`), add:

```rust
            | Self::SetChecked { signature, .. }
```

- [ ] **Step 9: Compile and verify**

Run: `cargo check -p krillnotes-core 2>&1 | head -30`

Expected: Errors in `sync.rs` for unhandled match arm — that's expected and fixed in Task 4.

- [ ] **Step 10: Commit**

```bash
git add krillnotes-core/src/core/operation.rs
git commit -m "feat(core): add SetChecked operation variant"
```

---

## Task 3: `set_note_checked()` Workspace Method + Undo

**Files:**
- Modify: `krillnotes-core/src/core/workspace/notes.rs`
- Modify: `krillnotes-core/src/core/undo.rs` (if `NoteRestore` needs `old_is_checked`)

- [ ] **Step 1: Write the test**

In `krillnotes-core/src/core/workspace/tests.rs`, add:

```rust
#[test]
fn test_set_note_checked() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), test_gate(), None).unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    assert!(!root.is_checked, "Notes start unchecked");

    // Check it
    let updated = ws.set_note_checked(&root.id, true).unwrap();
    assert!(updated.is_checked);

    // Persist check
    let reloaded = ws.get_note(&root.id).unwrap();
    assert!(reloaded.is_checked);

    // Uncheck
    let updated = ws.set_note_checked(&root.id, false).unwrap();
    assert!(!updated.is_checked);
}
```

- [ ] **Step 2: Run test to confirm it fails**

Run: `cargo test -p krillnotes-core test_set_note_checked 2>&1 | tail -10`

Expected: FAIL — `set_note_checked` method not found.

- [ ] **Step 3: Implement `set_note_checked`**

In `krillnotes-core/src/core/workspace/notes.rs`, add the method to `impl Workspace` (after `toggle_note_expansion`):

```rust
    /// Sets the `is_checked` state of a note and logs a [`Operation::SetChecked`].
    pub fn set_note_checked(&mut self, note_id: &str, checked: bool) -> Result<Note> {
        let old_note = self.get_note(note_id)?;

        // Authorize — reuse UpdateNote-level permission.
        let auth_op = Operation::SetChecked {
            operation_id: String::new(),
            timestamp: HlcTimestamp { wall_ms: 0, counter: 0, node_id: 0 },
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            checked,
            modified_by: self.current_identity_pubkey.clone(),
            signature: String::new(),
        };
        self.authorize(&auth_op)?;

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let now = ts.wall_ms as i64;

        let tx = self.storage.connection_mut().transaction()?;
        tx.execute(
            "UPDATE notes SET is_checked = ?1, modified_at = ?2, modified_by = ?3 WHERE id = ?4",
            rusqlite::params![checked, now, self.current_identity_pubkey, note_id],
        )?;
        if tx.changes() == 0 {
            return Err(KrillnotesError::NoteNotFound(note_id.to_string()));
        }

        Self::save_hlc(&ts, &tx)?;
        let op_id = Uuid::new_v4().to_string();
        let mut op = Operation::SetChecked {
            operation_id: op_id.clone(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            checked,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;
        tx.commit()?;

        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::NoteRestore {
                note_id: note_id.to_string(),
                old_title: old_note.title,
                old_fields: old_note.fields,
                old_tags: old_note.tags,
            },
            propagate: true,
        });

        self.get_note(note_id)
    }
```

Note: We reuse `NoteRestore` for undo since it restores the full note state including `is_checked` (the undo path calls `update_note` which writes the full row). If `NoteRestore` doesn't restore `is_checked`, we'll add `old_is_checked: bool` to it — check in step 4.

- [ ] **Step 4: Verify undo path restores `is_checked`**

Check `workspace/undo.rs` for how `NoteRestore` is applied. If it calls `update_note` (which does a full row write), `is_checked` may get reset to `false` if `update_note` doesn't preserve it. Look at the `RetractInverse::NoteRestore` handler.

If the undo handler only writes `title`, `fields`, and `tags` back, we need to add `old_is_checked: bool` to `NoteRestore`:

In `krillnotes-core/src/core/undo.rs`, update:

```rust
NoteRestore {
    note_id: String,
    old_title: String,
    old_fields: BTreeMap<String, FieldValue>,
    old_tags: Vec<String>,
    old_is_checked: bool,
},
```

Then update every place that constructs `NoteRestore` to include `old_is_checked: old_note.is_checked`.

And update the undo apply handler to also restore `is_checked`:

```sql
UPDATE notes SET is_checked = ?N WHERE id = ?M
```

- [ ] **Step 5: Run test**

Run: `cargo test -p krillnotes-core test_set_note_checked 2>&1 | tail -10`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace/notes.rs krillnotes-core/src/core/workspace/tests.rs krillnotes-core/src/core/undo.rs
git commit -m "feat(core): add set_note_checked workspace method with undo support"
```

---

## Task 4: Sync + Export + Operation Dispatch

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs`
- Modify: `krillnotes-core/src/core/export.rs`

- [ ] **Step 1: Handle `SetChecked` in sync `apply_remote_op`**

In `krillnotes-core/src/core/workspace/sync.rs`, in the `match &op` block (around line 240), after the `Operation::SetTags` handler, add:

```rust
            Operation::SetChecked { note_id, checked, .. } => {
                let now_ms = ts.wall_ms as i64;
                tx.execute(
                    "UPDATE notes SET is_checked = ?1, modified_at = ?2 WHERE id = ?3",
                    rusqlite::params![checked, now_ms, note_id],
                )?;
            }
```

- [ ] **Step 2: Update sync `CreateNote` INSERT to include `is_checked`**

In the same file, the `CreateNote` handler at line 248 does `INSERT OR IGNORE INTO notes`. It currently inserts 12 columns with `is_expanded` hardcoded to `1`. Add `is_checked` column with `0`:

```rust
tx.execute(
    "INSERT OR IGNORE INTO notes \
     (id, title, schema, parent_id, position, created_at, modified_at, \
      created_by, modified_by, fields_json, is_expanded, schema_version, is_checked) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 1, 0)",
    rusqlite::params![
        note_id, title, schema, parent_id, position,
        now_ms, now_ms, created_by, created_by, fields_json,
    ],
)?;
```

- [ ] **Step 3: Add `SetChecked` to the `op_type_name` match in sync.rs**

In the `op_type_name` helper function (around line 458), add before the closing `}`:

```rust
            Operation::SetChecked { .. } => "SetChecked",
```

- [ ] **Step 4: Update export INSERT**

In `krillnotes-core/src/core/export.rs`, at line 448, update the INSERT to include `is_checked`:

```rust
tx.execute(
    "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, is_checked)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    rusqlite::params![
        note.id,
        note.title,
        note.schema,
        note.parent_id,
        note.position,
        note.created_at,
        note.modified_at,
        note.created_by,
        note.modified_by,
        fields_json,
        note.is_expanded,
        note.is_checked,
    ],
)?;
```

- [ ] **Step 5: Run all core tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -20`

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/export.rs
git commit -m "feat(core): handle SetChecked in sync and export paths"
```

---

## Task 5: Schema `show_checkbox` Flag

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:127-489`
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

- [ ] **Step 1: Add `show_checkbox` to `Schema` struct**

In `krillnotes-core/src/core/scripting/schema.rs`, add to the `Schema` struct (line 127-158), after `is_leaf: bool`:

```rust
    pub show_checkbox: bool,
```

- [ ] **Step 2: Parse `show_checkbox` from Rhai map**

In `parse_from_rhai` (around line 483), before the `is_leaf` parse:

```rust
        let show_checkbox = def
            .get("show_checkbox")
            .and_then(|v| v.clone().try_cast::<bool>())
            .unwrap_or(false);
```

- [ ] **Step 3: Include in `Schema` construction**

At line 488, add `show_checkbox` to the `Ok(Schema { ... })` return:

```rust
Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort, allowed_parent_schemas, allowed_children_schemas, allow_attachments, attachment_types, field_groups, ast: None, version, migrations, is_leaf, show_checkbox })
```

- [ ] **Step 4: Expose `is_checked` in Rhai note map**

In `krillnotes-core/src/core/scripting/mod.rs`, in `build_note_map` (around line 397, after `tags` insertion), add:

```rust
        note_map.insert("is_checked".into(), Dynamic::from(note.is_checked));
```

Also check any other `note_map` construction sites in the same file (search for `note_map.insert("tags"`) and add `is_checked` there too.

- [ ] **Step 5: Compile and run core tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -20`

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat(core): add show_checkbox schema flag and expose is_checked in Rhai"
```

---

## Task 6: Tauri Command + Frontend Types

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/notes.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/scripting.rs:68-103`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src/types.ts`

- [ ] **Step 1: Add `set_note_checked` Tauri command**

In `krillnotes-desktop/src-tauri/src/commands/notes.rs`, after `toggle_note_expansion` (line 78), add:

```rust
#[tauri::command]
pub fn set_note_checked(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    checked: bool,
) -> std::result::Result<crate::Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;

    workspace.set_note_checked(&note_id, checked)
        .map_err(|e| { log::error!("set_note_checked failed: {e}"); e.to_string() })
}
```

- [ ] **Step 2: Register in `generate_handler!`**

In `krillnotes-desktop/src-tauri/src/lib.rs`, add `set_note_checked` to the `generate_handler!` macro invocation, near the other note commands.

- [ ] **Step 3: Add `show_checkbox` to `SchemaInfo`**

In `krillnotes-desktop/src-tauri/src/commands/scripting.rs`, add to the `SchemaInfo` struct (line 68-81):

```rust
    pub show_checkbox: bool,
```

And in `schema_to_info` (line 83-103), add:

```rust
        show_checkbox: schema.show_checkbox,
```

- [ ] **Step 4: Update TypeScript types**

In `krillnotes-desktop/src/types.ts`, add to the `Note` interface (after `schemaVersion: number`):

```typescript
  isChecked: boolean;
```

Add to `SchemaInfo` interface (after `isLeaf: boolean`):

```typescript
  showCheckbox: boolean;
```

- [ ] **Step 5: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20`

Expected: No errors (or only pre-existing ones unrelated to this change).

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/notes.rs krillnotes-desktop/src-tauri/src/commands/scripting.rs krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src/types.ts
git commit -m "feat(desktop): add set_note_checked Tauri command and TS types"
```

---

## Task 7: Tree Checkbox Rendering

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`
- Modify: `krillnotes-desktop/src/i18n/locales/*.json` (7 files)

- [ ] **Step 1: Add i18n keys**

Add to all 7 locale files under the `"tree"` section:

**en.json:**
```json
"checkNote": "Mark as complete",
"uncheckNote": "Mark as incomplete"
```

**de.json:**
```json
"checkNote": "Als erledigt markieren",
"uncheckNote": "Als unerledigt markieren"
```

**es.json:**
```json
"checkNote": "Marcar como completado",
"uncheckNote": "Marcar como incompleto"
```

**fr.json:**
```json
"checkNote": "Marquer comme terminé",
"uncheckNote": "Marquer comme non terminé"
```

**ja.json:**
```json
"checkNote": "完了にする",
"uncheckNote": "未完了にする"
```

**ko.json:**
```json
"checkNote": "완료로 표시",
"uncheckNote": "미완료로 표시"
```

**zh.json:**
```json
"checkNote": "标记为完成",
"uncheckNote": "标记为未完成"
```

- [ ] **Step 2: Add `onToggleChecked` prop to TreeNode**

In `TreeNode.tsx`, add to the component's props interface:

```typescript
onToggleChecked: (noteId: string, checked: boolean) => void;
```

And accept it in the component's destructured props.

- [ ] **Step 3: Render checkbox in TreeNode**

In `TreeNode.tsx`, after the sharing indicators block (around line 260, before the title `<span>`), add a checkbox that only renders when the schema's `showCheckbox` is true:

```tsx
{schemas[node.note.schema]?.showCheckbox && (
  <input
    type="checkbox"
    checked={node.note.isChecked}
    onChange={(e) => {
      e.stopPropagation();
      onToggleChecked(node.note.id, e.target.checked);
    }}
    onClick={(e) => e.stopPropagation()}
    className="mr-1.5 h-3.5 w-3.5 rounded border-muted-foreground/50 accent-primary flex-shrink-0"
    aria-label={node.note.isChecked ? t('tree.uncheckNote') : t('tree.checkNote')}
  />
)}
```

Also add a strikethrough style to checked note titles by modifying the title `<span>`:

```tsx
<span className={`text-sm truncate flex-1 min-w-0 ${isGhost ? 'text-zinc-400 italic' : ''} ${node.note.isChecked && schemas[node.note.schema]?.showCheckbox ? 'line-through text-muted-foreground' : ''}`}>{node.note.title}</span>
```

- [ ] **Step 4: Pass `schemas` to TreeNode if not already passed**

Check if `schemas` (the `Record<string, SchemaInfo>`) is already available in TreeNode. If not, thread it from WorkspaceView → TreeView → TreeNode.

- [ ] **Step 5: Add handler in WorkspaceView**

In `WorkspaceView.tsx`, add a `handleToggleChecked` callback:

```typescript
const handleToggleChecked = useCallback(async (noteId: string, checked: boolean) => {
  try {
    const updated = await invoke<Note>('set_note_checked', { noteId, checked });
    setNotes(prev => prev.map(n => n.id === updated.id ? updated : n));
  } catch (err) {
    console.error('Failed to toggle checked:', err);
  }
}, []);
```

Pass `onToggleChecked={handleToggleChecked}` through to TreeView/TreeNode.

- [ ] **Step 6: Thread `onToggleChecked` through TreeView**

In `TreeView.tsx`, accept `onToggleChecked` in props and pass it to each `TreeNode`.

- [ ] **Step 7: Build and test in browser**

Run: `cd krillnotes-desktop && npm run tauri dev`

Test:
1. Create a user script with `show_checkbox: true` (or wait for Task 8's system script).
2. Create a note of that type.
3. Verify checkbox appears in tree.
4. Click checkbox — title should get strikethrough.
5. Reload — checkbox state persists.
6. Verify notes without `show_checkbox` don't show a checkbox.

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src/components/TreeNode.tsx krillnotes-desktop/src/components/TreeView.tsx krillnotes-desktop/src/components/WorkspaceView.tsx krillnotes-desktop/src/i18n/locales/
git commit -m "feat(desktop): render checkbox in tree for show_checkbox schemas"
```

---

## Task 8: TodoItem System Script

**Files:**
- Create: `krillnotes-core/src/core/system_scripts/01_todo_item.schema.rhai`

- [ ] **Step 1: Create the script**

```rhai
schema("TodoItem", #{
    version: 1,
    show_checkbox: true,
    is_leaf: true,
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});

register_view("TodoItem", "Details", #{ display_first: true }, |note| {
    let body = note.fields["body"] ?? "";
    if note.is_checked {
        text("✓ Done")
    } else if body != "" {
        markdown(body)
    } else {
        text("(empty)")
    }
});
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p krillnotes-core 2>&1 | tail -20`

Expected: All tests pass. The system script is auto-embedded via `include_dir!`.

- [ ] **Step 3: Build and test in browser**

Run: `cd krillnotes-desktop && npm run tauri dev`

Test:
1. Open workspace — "TodoItem" should appear in the Add Note dialog.
2. Create a TodoItem — it appears with a checkbox in the tree.
3. Toggle the checkbox — strikethrough on title, view shows "✓ Done".
4. Undo (Ctrl+Z) — checkbox reverts.
5. Create a TextNote — no checkbox.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/system_scripts/01_todo_item.schema.rhai
git commit -m "feat(core): add built-in TodoItem schema with checkbox support"
```

---

## Task 9: Final Verification

- [ ] **Step 1: Run full Rust test suite**

Run: `cargo test -p krillnotes-core 2>&1 | tail -20`

Expected: All tests pass.

- [ ] **Step 2: TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20`

Expected: No type errors.

- [ ] **Step 3: Full build**

Run: `cd krillnotes-desktop && npm update && npm run tauri build 2>&1 | tail -20`

Expected: Build succeeds.

- [ ] **Step 4: Manual testing checklist**

- [ ] TodoItem note shows checkbox in tree
- [ ] Clicking checkbox toggles `isChecked` and shows strikethrough
- [ ] Checkbox state persists after close/reopen
- [ ] TextNote (no `show_checkbox`) has no checkbox
- [ ] Undo reverts checkbox toggle
- [ ] Export/import preserves `is_checked` state
- [ ] Rhai script can read `note.is_checked`
- [ ] User scripts with `show_checkbox: true` also get the checkbox
